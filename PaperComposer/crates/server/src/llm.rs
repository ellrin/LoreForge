//! Writing engines, through lore-llm.
//!
//! Every model call is a task from `tasks.toml` (compose, soft check,
//! decompose, outline, caption, reference search and notes, citation
//! extraction). The engine switch in the UI picks one of three engines:
//!
//! - local: an OpenAI-compatible server — the `[engine] url` in config.toml
//!   (or PAPERCOMPOSER_LLM_URL), or the llama-server started for a downloaded
//!   model. Replies are rewritten in Taiwan usage (local models write
//!   mainland vocabulary even when asked for Traditional Chinese).
//! - subscription: Claude Code on the user's claude.ai login.
//! - api: the Anthropic API with the key from the OS keychain.
//!
//! With no choice the task's own order applies (local first, then Claude);
//! a choice is that engine or an error, never a silent fallback.
//! `local:current` and `claude:selected` in tasks.toml are lore-llm's refs for
//! "the local engine that is connected" and "the Claude model picked in
//! settings". Local engines write at temperature 0.2 (Claude takes no
//! sampling parameters). The API key never touches a file: it comes from ANTHROPIC_API_KEY or the
//! keychain (service `PaperComposer`, one key for the app).

use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::local::{Layout, LocalRuntime};
use lore_llm::router::{ApiAccess, CLAUDE_SELECTED, ExternalServer, LOCAL_CURRENT, LocalAccess, StandardFactory, SubscriptionAccess};
use lore_llm::secrets::{Account, KeySource, KeyStore};
use lore_llm::{LlmRequest, Route, RouteOptions, Router, TaskTable};

const TASKS: &str = include_str!("../tasks.toml");
/// Keychain service: the name the first versions stored the key under.
pub const SERVICE: &str = "PaperComposer";
/// Sampling temperature for local engines (what PaperComposer always used;
/// lore-llm leaves it out for Claude, whose API rejects it).
pub const LOCAL_TEMPERATURE: f32 = 0.2;

/// Which engine writes. None = automatic: the task's order (local first).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    Local,
    /// Claude Code CLI on the claude.ai subscription.
    Subscription,
    /// Anthropic API with a key (billed per use).
    #[serde(alias = "claude")]
    Api,
}

impl EngineChoice {
    /// The model ref the choice puts in front of a task's candidates.
    fn model_ref(self) -> String {
        match self {
            EngineChoice::Local => format!("local:{LOCAL_CURRENT}"),
            EngineChoice::Subscription => format!("claude-code:{CLAUDE_SELECTED}"),
            EngineChoice::Api => format!("claude-api:{CLAUDE_SELECTED}"),
        }
    }
}

/// `[engine]` in config.toml: an OpenAI-compatible server someone else runs
/// (llama.cpp server, Ollama, LM Studio).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EngineConfig {
    /// Base URL ending in /v1, e.g. http://127.0.0.1:8080/v1
    pub url: Option<String>,
    pub model: Option<String>,
}

impl EngineConfig {
    /// Environment overrides the config file.
    pub fn resolve(file: Option<EngineConfig>) -> EngineConfig {
        let file = file.unwrap_or_default();
        EngineConfig {
            url: std::env::var("PAPERCOMPOSER_LLM_URL").ok().or(file.url),
            model: std::env::var("PAPERCOMPOSER_LLM_MODEL").ok().or(file.model),
        }
    }
}

#[derive(Serialize)]
pub struct EngineStatus {
    pub connected: bool,
    pub name: String,
}

#[derive(Serialize)]
pub struct ClaudeStatus {
    pub configured: bool,
    pub source: Option<KeySource>,
    /// The key with all but the last four characters hidden.
    pub masked: Option<String>,
    pub model: String,
    pub models: Vec<String>,
}

pub struct Llm {
    pub catalog: Catalog,
    tasks: TaskTable,
    pub keys: KeyStore,
    pub cli: Cli,
    /// Claude Code login status, refreshed at most once a minute.
    pub cc: Arc<Cache>,
    /// The llama-server for a downloaded model (one at a time).
    pub runtime: Arc<LocalRuntime>,
    /// Where models and llama.cpp live (LOCAL_LLM_HOME, or the app's own folders).
    pub layout: Layout,
    /// An external server from config.toml or the environment.
    external: RwLock<EngineConfig>,
    /// Claude model id picked in settings.
    model: RwLock<String>,
    /// PAPERCOMPOSER_CLAUDE_BASE_URL (a local stub in tests).
    base_url: String,
}

impl Llm {
    pub fn new(catalog: Catalog, tasks: TaskTable, runtime: Arc<LocalRuntime>, layout: Layout, external: EngineConfig, model: Option<String>) -> Llm {
        let model = model.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| catalog.cloud.default.clone());
        Llm {
            catalog,
            tasks,
            keys: KeyStore::anthropic(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            runtime,
            layout,
            external: RwLock::new(external),
            model: RwLock::new(model),
            base_url: std::env::var("PAPERCOMPOSER_CLAUDE_BASE_URL").unwrap_or_else(|_| lore_llm::claude_api::DEFAULT_BASE_URL.into()),
        }
    }

    /// The app's tasks over lore-llm's shared ones, then the user's file.
    pub fn task_table(user_file: Option<&std::path::Path>) -> Result<TaskTable, String> {
        let base = TaskTable::bundled().merge(TaskTable::parse(TASKS)?);
        TaskTable::load(base, user_file)
    }

    pub fn model(&self) -> String {
        self.model.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_model(&self, m: String) {
        *self.model.write().unwrap_or_else(|e| e.into_inner()) = m;
    }

    pub fn cloud_models(&self) -> Vec<String> {
        self.catalog.cloud.models.iter().map(|m| m.id.clone()).collect()
    }

    pub fn set_external(&self, e: EngineConfig) {
        *self.external.write().unwrap_or_else(|e| e.into_inner()) = e;
    }

    fn external(&self) -> Option<ExternalServer> {
        let e = self.external.read().unwrap_or_else(|e| e.into_inner());
        e.url.clone().map(|url| ExternalServer { url, model: e.model.clone() })
    }

    pub fn key(&self) -> Option<(String, KeySource)> {
        self.keys.get(None, &Account::Single)
    }

    /// Store (or with an empty string, remove) the key in the OS keychain.
    pub fn save_key(&self, k: &str) -> Result<(), String> {
        self.keys.save(k, &Account::Single).map(|_| ())
    }

    pub fn claude_status(&self) -> ClaudeStatus {
        let k = self.key();
        ClaudeStatus {
            configured: k.is_some(),
            masked: k.as_ref().map(|(k, _)| lore_llm::secrets::mask(k)),
            source: k.map(|(_, s)| s),
            model: self.model(),
            models: self.cloud_models(),
        }
    }

    /// A local engine is connected: an external server, or a running model.
    pub async fn local_configured(&self) -> bool {
        self.external().is_some() || self.runtime.current().await.is_some()
    }

    pub async fn ready(&self, e: EngineChoice) -> bool {
        match e {
            EngineChoice::Local => self.local_configured().await,
            EngineChoice::Subscription => self.cc.get(&self.cli, false).await.ready,
            EngineChoice::Api => self.key().is_some(),
        }
    }

    /// Local engines: the external server or the runtime (`local:current`),
    /// catalog models from the model folder; Taiwan usage, abliterated
    /// models allowed.
    fn local_access(&self) -> LocalAccess {
        LocalAccess {
            layout: Some(self.layout.clone()),
            external: self.external(),
            runtime: Some(self.runtime.clone()),
            zh_tw: true,
            include_uncensored: true,
            ..Default::default()
        }
    }

    /// Status light for the local engine.
    pub async fn local_status(&self) -> EngineStatus {
        let name = match (self.external(), self.runtime.current_info().await) {
            (Some(ext), _) => ext.model.unwrap_or_else(|| "本機模型".into()),
            (None, Some(run)) => self.catalog.get(&run.model_id).map(|m| format!("{} {}", m.name, m.variant)).unwrap_or(run.model_id),
            (None, None) => return EngineStatus { connected: false, name: "未連接模型".into() },
        };
        let connected = match self.local_access().current(&self.catalog).await {
            Ok(p) => p.ping().await,
            Err(_) => false,
        };
        EngineStatus { connected, name }
    }

    /// Engines for the current settings.
    fn factory(&self) -> StandardFactory {
        let mut f = StandardFactory::new(self.catalog.clone());
        f.prefer_subscription = true;
        f.selected_claude = Some(self.model());
        f.api = self.key().map(|(k, _)| ApiAccess { base_url: self.base_url.clone(), api_key: k, fallbacks: true });
        f.subscription = Some(SubscriptionAccess { cli: self.cli.clone(), cache: self.cc.clone(), allow_web: false, temp_prefix: "papercomposer-claude".into() });
        f.local = Some(self.local_access());
        f
    }

    /// The engine for a task. A choice from the engine switch is that
    /// engine or nothing (the reason); without one the task's order decides.
    pub async fn route(&self, task: &str, choice: Option<EngineChoice>) -> Result<Route, String> {
        let opts = choice.map(|c| RouteOptions::only(c.model_ref())).unwrap_or_default();
        let router = Router::new(self.tasks.clone(), self.factory());
        router.provider_for_with(task, &opts).await.map_err(|e| if choice.is_some() { e.reason() } else { e.to_string() })
    }

    /// Run `task` on one exact model ref ("claude:haiku", "local:current"),
    /// no fallback: the draft window's model picker.
    pub async fn complete_on(&self, task: &str, model_ref: &str, prompt: &Prompt, extra: Option<&str>, schema: Option<serde_json::Value>) -> Result<String, String> {
        let router = Router::new(self.tasks.clone(), self.factory());
        let route = router.provider_for_with(task, &RouteOptions::only(model_ref)).await.map_err(|e| e.reason())?;
        route.provider.complete(&request(&route, prompt, extra, schema)).await.map_err(|e| e.to_string())
    }

    /// Run one prompt as `task`. `extra` is appended to the user prompt (the
    /// retry feedback); `schema` asks for JSON of that shape.
    pub async fn complete(&self, task: &str, choice: Option<EngineChoice>, prompt: &Prompt, extra: Option<&str>, schema: Option<serde_json::Value>) -> Result<String, String> {
        let route = self.route(task, choice).await?;
        route.provider.complete(&request(&route, prompt, extra, schema)).await.map_err(|e| e.to_string())
    }
}

/// The request for a prompt: max 4096 tokens unless the task says otherwise,
/// temperature 0.2 on local engines.
pub fn request(route: &Route, prompt: &Prompt, extra: Option<&str>, schema: Option<serde_json::Value>) -> LlmRequest {
    let mut user = prompt.user.clone();
    if let Some(e) = extra {
        user += "\n";
        user += e;
    }
    let mut req = LlmRequest::prompt(prompt.system.clone(), user).temperature(LOCAL_TEMPERATURE);
    if let Some(s) = schema {
        req = req.json(s);
    }
    route.apply(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use lore_llm::{OpenAiCompat, OpenAiCompatConfig};

    fn llm(external: Option<String>) -> Arc<Llm> {
        let tasks = Llm::task_table(None).unwrap();
        let home = std::env::temp_dir().join(format!("pc-llm-{}", std::process::id()));
        let mut l = Llm::new(Catalog::bundled(), tasks, Arc::new(LocalRuntime::new()), Layout::shared(&home), EngineConfig { url: external, model: None }, Some("claude-haiku-5-5".into()));
        l.keys.use_keychain = false; // never the real keychain in tests
        l.keys.env_var = None;
        l.cli = Cli { bin: Some(std::env::temp_dir().join("pc-no-such-claude")), config_dir: None };
        Arc::new(l)
    }

    #[test]
    fn app_tasks_parse_and_cover_every_use() {
        let t = Llm::task_table(None).unwrap();
        for id in ["pc-compose-paragraph", "pc-soft-check", "pc-decompose", "pc-outline", "pc-caption", "pc-ref-queries", "pc-ref-rank", "pc-ref-notes", "pc-cite-extract", "pc-engine-test"] {
            assert!(t.task.contains_key(id), "{id}");
        }
        // Claude-only tasks have no local candidate.
        for id in ["pc-outline", "pc-ref-queries", "pc-ref-rank", "pc-cite-extract"] {
            assert_eq!(t.task[id].default, "claude:selected");
            assert!(t.task[id].fallback.is_none());
        }
        assert_eq!(t.task["pc-compose-paragraph"].default, "local:current", "the app's order: local first");
    }

    #[tokio::test]
    async fn choices_map_to_strict_overrides() {
        // Nothing set up: no engine at all, and a choice says why.
        let l = llm(None);
        assert!(!l.ready(EngineChoice::Local).await && !l.ready(EngineChoice::Api).await);
        let e = l.route("pc-compose-paragraph", Some(EngineChoice::Local)).await.unwrap_err();
        assert!(e.contains("本機"), "{e}");
        assert!(l.route("pc-compose-paragraph", None).await.is_err());

        // An external server is the local engine; auto picks it first.
        let l = llm(Some("http://127.0.0.1:9/v1".into()));
        assert!(l.ready(EngineChoice::Local).await);
        let r = l.route("pc-compose-paragraph", None).await.unwrap();
        assert_eq!(r.provider.id(), "llama-server:local");
        assert!(!r.fallback_used);
        // Asking for the API without a key does not fall back to local.
        let e = l.route("pc-compose-paragraph", Some(EngineChoice::Api)).await.unwrap_err();
        assert!(e.contains("API key"), "{e}");
        // Claude-only tasks never run locally.
        assert!(l.route("pc-outline", None).await.is_err());
    }

    #[tokio::test]
    async fn api_choice_uses_the_model_from_settings() {
        let l = llm(Some("http://127.0.0.1:9/v1".into()));
        let dir = tempfile_dir("pc-llm-key");
        let mut ks = KeyStore::anthropic("pc-test", Some(dir.clone()));
        ks.use_keychain = false;
        ks.env_var = None;
        ks.save("test-key", &Account::Single).unwrap();
        let mut inner = Arc::try_unwrap(l).ok().unwrap();
        inner.keys = ks;
        let l = Arc::new(inner);
        let r = l.route("pc-outline", Some(EngineChoice::Api)).await.unwrap();
        assert_eq!(r.provider.id(), "claude:claude-haiku-5-5");
        let r = l.route("pc-caption", None).await.unwrap();
        assert_eq!(r.provider.id(), "llama-server:local", "auto: local before Claude");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_requests_carry_the_schema() {
        let l = llm(Some("http://127.0.0.1:9/v1".into()));
        let route = l.route("pc-soft-check", Some(EngineChoice::Local)).await.unwrap();
        let schema = pc_core::softcheck::reply_schema();
        let req = request(&route, &Prompt { system: "s".into(), user: "u".into() }, Some("again"), Some(schema.clone()));
        assert_eq!(req.json_schema.as_ref(), Some(&schema));
        assert_eq!(req.messages[0].text(), "u\nagain");
        let body = OpenAiCompat::new(OpenAiCompatConfig::llama_server("http://x/v1", "local")).body(&req);
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
        assert!((body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6, "local engines write at 0.2");
        // Claude gets the same request without sampling parameters.
        let claude = lore_llm::ClaudeApi::new(lore_llm::ClaudeApiConfig::new("claude-haiku-5-5"), "test-key").body(&req);
        assert!(claude.get("temperature").is_none());
    }

    fn tempfile_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
