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
use lore_llm::local::Layout;
use lore_llm::shared_runtime::{LocalStatus, Priority, SharedRuntime};
use lore_llm::router::{ApiAccess, CLAUDE_SELECTED, ExternalServer, LOCAL_CURRENT, LocalAccess, ProviderFactory, StandardFactory, SubscriptionAccess};
use lore_llm::secrets::{Account, KeySource, SharedKeys};
use lore_llm::{LlmRequest, Route, RouteOptions, Router, TaskTable};

const TASKS: &str = include_str!("../tasks.toml");
/// Keychain service: the name the first versions stored the key under.
pub const SERVICE: &str = "PaperComposer";
/// Sampling temperature for local engines (what L-Paper always used;
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
    pub(crate) fn model_ref(self) -> String {
        match self {
            EngineChoice::Local => format!("local:{LOCAL_CURRENT}"),
            EngineChoice::Subscription => format!("claude-code:{CLAUDE_SELECTED}"),
            EngineChoice::Api => format!("claude-api:{CLAUDE_SELECTED}"),
        }
    }

    /// A request's `engine` word; empty, `auto` or anything else = automatic.
    pub(crate) fn parse(engine: Option<&str>) -> Option<EngineChoice> {
        engine.filter(|e| !e.is_empty() && *e != "auto").and_then(|e| serde_json::from_value(serde_json::json!(e)).ok())
    }
}

/// `claude:<alias>`, `local:<catalog id>` or `agy:<model id>`, the id not empty.
fn named_model(m: &str) -> bool {
    ["claude:", "local:", "agy:"].iter().any(|p| m.strip_prefix(p).is_some_and(|id| !id.trim().is_empty()))
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
    pub keys: SharedKeys,
    pub cli: Cli,
    /// Claude Code login status, refreshed at most once a minute.
    pub cc: Arc<Cache>,
    /// The machine's shared llama-server (lore-llm `shared_runtime`; `None` in tests).
    pub local_rt: Option<Arc<SharedRuntime>>,
    /// Where the shared model choice is kept (`private/shared/engine.toml` `[engine] model`).
    shared: RwLock<Option<lore_home::Home>>,
    /// Where models and llama.cpp live (LOCAL_LLM_HOME, or the app's own folders).
    pub layout: Layout,
    /// An external server from config.toml or the environment.
    external: RwLock<EngineConfig>,
    /// Claude model id picked in settings.
    model: RwLock<String>,
    /// PAPERCOMPOSER_CLAUDE_BASE_URL (a local stub in tests).
    base_url: String,
    /// `PAPERCOMPOSER_LLM_MOCK` (the isolated example's `--mock`, the skeleton tests): every
    /// call answered from a file of scripted replies; never the network, the CLI or a local model.
    pub mock: Option<Arc<Scripted>>,
}

/// Scripted replies per task id, in order; the last one repeats. The file is JSON:
/// `{ "pc-draft-skeleton": [ {…}, … ], "pc-draft-fill": [ … ] }` (a reply may be a JSON value or a
/// string). The tasks asked, in order, are kept for tests.
pub struct Scripted {
    replies: std::sync::Mutex<std::collections::BTreeMap<String, std::collections::VecDeque<String>>>,
    pub asked: std::sync::Mutex<Vec<(String, String)>>,
    /// `"_delay_ms": { "pc-draft-fill": 300 }`: a reply that takes time (to test edits made meanwhile).
    delays: std::collections::BTreeMap<String, u64>,
}

impl Scripted {
    pub fn parse(json: &str) -> Result<Scripted, String> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("mock 檔：{e}"))?;
        let obj = v.as_object().ok_or("mock 檔要是物件：{ 任務: [回覆…] }")?;
        let mut replies = std::collections::BTreeMap::new();
        let delays = obj.get("_delay_ms").and_then(|d| d.as_object()).map(|d| d.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()).unwrap_or_default();
        for (task, list) in obj.iter().filter(|(k, _)| !k.starts_with('_')) {
            let list = match list {
                serde_json::Value::Array(a) => a.clone(),
                other => vec![other.clone()],
            };
            let q: std::collections::VecDeque<String> = list.into_iter().map(|r| match r {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            }).collect();
            replies.insert(task.clone(), q);
        }
        Ok(Scripted { replies: std::sync::Mutex::new(replies), asked: std::sync::Mutex::new(vec![]), delays })
    }

    pub fn load(path: &std::path::Path) -> Result<Scripted, String> {
        Scripted::parse(&std::fs::read_to_string(path).map_err(|e| format!("{}：{e}", path.display()))?)
    }

    /// The reply for `task` after its scripted delay.
    pub async fn answer(&self, task: &str, prompt: &Prompt) -> Result<String, String> {
        if let Some(ms) = self.delays.get(task) {
            tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
        }
        self.reply(task, prompt)
    }

    /// The next reply for `task` (the last one again when only one is left).
    pub fn reply(&self, task: &str, prompt: &Prompt) -> Result<String, String> {
        if let Ok(mut a) = self.asked.lock() {
            a.push((task.to_string(), prompt.user.clone()));
        }
        let mut r = self.replies.lock().map_err(|_| "mock".to_string())?;
        let q = r.get_mut(task).ok_or_else(|| format!("mock 沒有 {task} 的回覆"))?;
        match q.len() {
            0 => Err(format!("mock 沒有 {task} 的回覆")),
            1 => Ok(q[0].clone()),
            _ => Ok(q.pop_front().unwrap_or_default()),
        }
    }
}

impl Llm {
    pub fn new(catalog: Catalog, tasks: TaskTable, local_rt: Option<Arc<SharedRuntime>>, layout: Layout, external: EngineConfig, model: Option<String>) -> Llm {
        let model = model.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| catalog.cloud.default.clone());
        Llm {
            catalog,
            tasks,
            keys: SharedKeys::for_app(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            local_rt,
            shared: RwLock::new(None),
            layout,
            external: RwLock::new(external),
            model: RwLock::new(model),
            base_url: std::env::var("PAPERCOMPOSER_CLAUDE_BASE_URL").unwrap_or_else(|_| lore_llm::claude_api::DEFAULT_BASE_URL.into()),
            mock: std::env::var_os("PAPERCOMPOSER_LLM_MOCK").map(|p| Arc::new(Scripted::load(std::path::Path::new(&p)).unwrap_or_else(|e| panic!("{e}")))),
        }
    }

    /// The app's tasks over lore-llm's shared ones, then the user's file.
    pub fn task_table(user_file: Option<&std::path::Path>) -> Result<TaskTable, String> {
        let base = TaskTable::bundled().merge(TaskTable::parse(TASKS)?).merge(lore_plot::propose::task_table()).merge(lore_flow::llm::task_table()).merge(lore_polish::task_table()).merge(lore_cards::search::task_table());
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

    /// Read the shared model choice from `home` (re-read on every call).
    pub fn share_engine(&self, home: lore_home::Home) {
        *self.shared.write().unwrap_or_else(|e| e.into_inner()) = Some(home);
    }

    /// `engine.toml` `[engine]`: the shared default model ids.
    pub fn shared_engine(&self) -> lore_home::engine::SharedEngine {
        self.shared.read().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|h| h.engine()).unwrap_or_default()
    }

    /// A local engine to ask: an external server, or a chosen model on the shared llama-server.
    pub async fn local_configured(&self) -> bool {
        self.external().is_some() || (self.local_rt.is_some() && self.shared_engine().model.is_some())
    }

    pub async fn ready(&self, e: EngineChoice) -> bool {
        if self.mock.is_some() {
            return true;
        }
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
        let e = self.shared_engine();
        LocalAccess {
            layout: Some(self.layout.clone()),
            external: self.external(),
            shared: self.local_rt.clone(),
            default_model: e.model,
            fallback_home: crate::loreforge_home().local_home(),
            vision_model: e.vision_model,
            zh_tw: true,
            include_uncensored: true,
            ..Default::default()
        }
    }

    /// Status light for the local engine.
    pub async fn local_status(&self) -> EngineStatus {
        let label = |id: &str| self.catalog.get(id).map(|m| format!("{} {}", m.name, m.variant)).unwrap_or_else(|| id.to_string());
        if let Some(ext) = self.external() {
            let connected = match self.local_access().current(&self.catalog).await {
                Ok(p) => p.ping().await,
                Err(_) => false,
            };
            return EngineStatus { connected, name: ext.model.unwrap_or_else(|| "本機模型".into()) };
        }
        let chosen = self.shared_engine().model;
        let Some(rt) = &self.local_rt else { return EngineStatus { connected: false, name: "未連接模型".into() } };
        let snap = rt.snapshot().await;
        match (&snap.status, chosen) {
            // loaded (by any app): ready to answer
            (LocalStatus::Loaded { model }, _) => EngineStatus { connected: true, name: label(model) },
            // chosen, loads on the first request (or waits / loads now): the same words as every app
            (st, Some(m)) => EngineStatus { connected: false, name: format!("{} · {}", label(&m), st.text(&self.catalog)) },
            (_, None) => EngineStatus { connected: false, name: "未連接模型".into() },
        }
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
    /// Every call here answers an open request (a generate button, a draft step), so a local
    /// model is asked at interactive priority.
    pub async fn route(&self, task: &str, choice: Option<EngineChoice>) -> Result<Route, String> {
        // scripted replies only: a feature without a script never reaches a real engine
        if self.mock.is_some() {
            return Err(format!("假模型模式：{task} 沒有腳本"));
        }
        let opts = choice.map(|c| RouteOptions::only(c.model_ref())).unwrap_or_default().with_priority(Priority::Interactive);
        let router = Router::new(self.tasks.clone(), self.factory());
        router.provider_for_with(task, &opts).await.map_err(|e| route_error(&e, choice.is_some()))
    }

    /// The engine for a task from a request's `engine` word (`local`, `subscription`, `api` /
    /// `claude`; empty or `auto` = the task's order). 語句潤飾 alone may also name its model
    /// (`claude:<alias>`, `local:<catalog id>`, `agy:<model id>`: L-Echo's model menu): that
    /// model or an error, never another one; every other task ignores a name.
    pub async fn route_engine(&self, task: &str, engine: Option<&str>) -> Result<Route, String> {
        if self.mock.is_some() {
            return Err(format!("假模型模式：{task} 沒有腳本"));
        }
        self.route_engine_with(self.factory(), task, engine).await
    }

    async fn route_engine_with<F: ProviderFactory>(&self, factory: F, task: &str, engine: Option<&str>) -> Result<Route, String> {
        let named = engine.filter(|m| task == lore_polish::TASK && named_model(m));
        let choice = if named.is_some() { None } else { EngineChoice::parse(engine) };
        let opts = match (named, choice) {
            (Some(m), _) => RouteOptions::only(m),
            (None, Some(c)) => RouteOptions::only(c.model_ref()),
            (None, None) => RouteOptions::default(),
        }
        .with_priority(Priority::Interactive);
        Router::new(self.tasks.clone(), factory).provider_for_with(task, &opts).await.map_err(|e| route_error(&e, named.is_some() || choice.is_some()))
    }

    /// Run `task` on one exact model ref ("claude:haiku", "local:current"),
    /// no fallback: the draft window's model picker.
    pub async fn complete_on(&self, task: &str, model_ref: &str, prompt: &Prompt, extra: Option<&str>, schema: Option<serde_json::Value>) -> Result<String, String> {
        if let Some(m) = &self.mock {
            return m.answer(task, prompt).await;
        }
        let router = Router::new(self.tasks.clone(), self.factory());
        let route = router.provider_for_with(task, &RouteOptions::only(model_ref).with_priority(Priority::Interactive)).await.map_err(|e| route_error(&e, true))?;
        route.provider.complete(&request(&route, prompt, extra, schema)).await.map_err(|e| e.to_string())
    }

    /// Run one prompt as `task`. `extra` is appended to the user prompt (the
    /// retry feedback); `schema` asks for JSON of that shape.
    pub async fn complete(&self, task: &str, choice: Option<EngineChoice>, prompt: &Prompt, extra: Option<&str>, schema: Option<serde_json::Value>) -> Result<String, String> {
        if let Some(m) = &self.mock {
            return m.answer(task, prompt).await;
        }
        let route = self.route(task, choice).await?;
        route.provider.complete(&request(&route, prompt, extra, schema)).await.map_err(|e| e.to_string())
    }
}

/// A routing failure as the error text the UI gets: the message, or — when the other
/// engine could run (「改用本機」／「改用 Claude」) — `RouteError::payload` as JSON, which
/// the page's api() reads back into the message and that one-click choice.
pub fn route_error(e: &lore_llm::RouteError, reason: bool) -> String {
    if e.alternative().is_some() { e.payload(reason).to_string() } else if reason { e.reason() } else { e.to_string() }
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

    /// The candidates a task gets with the shared settings installed (any class routes).
    fn first_candidates(task: &str) -> Vec<String> {
        let t = Llm::task_table(None).unwrap();
        let spec = t.task.get(task).unwrap();
        let s = lore_llm::shared::EngineSettings::default();
        lore_llm::router::plan(task, spec, None, &Default::default(), Some(&s)).candidates
    }

    /// 排骨架 lays out the whole paper: Claude only (no class: the local model is never a candidate).
    #[test]
    fn skeleton_plan_never_routes_to_the_local_model() {
        assert_eq!(first_candidates("pc-draft-skeleton"), vec!["claude:opus".to_string()]);
    }

    /// 暫存's repair: Sonnet first, the local model after it.
    #[test]
    fn skeleton_repair_routes_claude_first() {
        assert_eq!(first_candidates("pc-draft-skeleton-repair"), vec!["claude:sonnet".to_string(), "local:current".to_string()]);
    }

    /// 補肉: Haiku first (class `create`; `ask` would put the local model first), then local.
    #[test]
    fn skeleton_fill_routes_claude_first() {
        assert_eq!(first_candidates("pc-draft-fill"), vec!["claude:haiku".to_string(), "local:current".to_string()]);
        let t = Llm::task_table(None).unwrap();
        let mut spec = t.task.get("pc-draft-fill").unwrap().clone();
        spec.class = Some(lore_llm::shared::TaskClass::Ask);
        let other = lore_llm::router::plan("pc-draft-fill", &spec, None, &Default::default(), Some(&lore_llm::shared::EngineSettings::default())).candidates;
        assert_eq!(other.first().map(String::as_str), Some("local:current"), "the proof bites: `ask` would have put local first");
    }

    #[test]
    fn scripted_replies_per_task_in_order_the_last_repeats() {
        let m = Scripted::parse(r#"{ "a": [{"x":1}, "two"], "b": "only" }"#).unwrap();
        let p = Prompt { system: String::new(), user: "u".into() };
        assert_eq!(m.reply("a", &p).unwrap(), r#"{"x":1}"#);
        assert_eq!(m.reply("a", &p).unwrap(), "two");
        assert_eq!(m.reply("a", &p).unwrap(), "two");
        assert_eq!(m.reply("b", &p).unwrap(), "only");
        assert!(m.reply("c", &p).unwrap_err().contains("c"));
        assert_eq!(m.asked.lock().unwrap().len(), 5);
    }

    /// 深度討論 is cloud only: with the shared settings installed (any class routes), the local
    /// model is never a candidate (a class on the task let `ask` put it first: 70 s+ replies).
    #[test]
    fn discuss_never_routes_to_the_local_model() {
        let t = Llm::task_table(None).unwrap();
        let spec = t.task.get("pc-draft-discuss").unwrap();
        let s = lore_llm::shared::EngineSettings::default();
        let p = lore_llm::router::plan("pc-draft-discuss", spec, None, &Default::default(), Some(&s));
        assert_eq!(p.candidates, vec!["claude:sonnet".to_string()]);
    }
    use std::path::PathBuf;
    use lore_llm::{OpenAiCompat, OpenAiCompatConfig};
    use lore_llm::secrets::KeyStore;

    fn llm(external: Option<String>) -> Arc<Llm> {
        let tasks = Llm::task_table(None).unwrap();
        let home = std::env::temp_dir().join(format!("pc-llm-{}", std::process::id()));
        let mut l = Llm::new(Catalog::bundled(), tasks, None, Layout::shared(&home), EngineConfig { url: external, model: None }, Some("claude-haiku-5-5".into()));
        l.keys.isolate(); // never the real keychain in tests
        l.cli = Cli { bin: Some(std::env::temp_dir().join("pc-no-such-claude")), config_dir: None };
        Arc::new(l)
    }

    #[test]
    fn app_tasks_parse_and_cover_every_use() {
        let t = Llm::task_table(None).unwrap();
        for id in ["pc-compose-paragraph", "pc-soft-check", "pc-decompose", "pc-outline", "pc-caption", "pc-thesis-check", "pc-ref-queries", "pc-ref-rank", "pc-ref-notes", "pc-cite-extract", "pc-engine-test", "pc-book-plan", "pc-book-suggest", "pc-book-chapter"] {
            assert!(t.task.contains_key(id), "{id}");
        }
        // Claude first, then the local model.
        for id in ["pc-outline", "pc-ref-queries", "pc-ref-rank", "pc-cite-extract", "pc-book-plan", "pc-book-suggest", "pc-book-chapter"] {
            assert_eq!(t.task[id].default, "claude:selected");
            assert_eq!(t.task[id].fallback.as_deref(), Some("local:current"));
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
        // Claude first: with no Claude set up, the task order's local fallback runs.
        assert_eq!(l.route("pc-outline", None).await.unwrap().provider.id(), "llama-server:local");
    }

    #[tokio::test]
    async fn only_sentence_polish_can_name_its_model() {
        use lore_llm::MockProvider;
        use lore_llm::router::MockFactory;
        let mock = |name: &str| -> Arc<dyn lore_llm::LlmProvider> { Arc::new(MockProvider::new(vec![]).named(name)) };
        let f = MockFactory::default()
            .with("claude:selected", mock("mock:selected"))
            .with("claude-code:selected", mock("mock:subscription"))
            .with("local:current", mock("mock:current"))
            .with("local:qwen3.5-9b-q8", mock("mock:qwen"))
            .with("claude:haiku", mock("mock:haiku"))
            .with("agy:m-1", mock("mock:google"));
        let l = llm(None);
        let polish = lore_polish::TASK;
        assert!(Llm::task_table(None).unwrap().task[polish].agy, "the polish task may name agy: models");
        let id = |r: Result<Route, String>| r.unwrap().provider.id();
        // as before: the task's order, and the engine switch's words
        let auto = id(l.route_engine_with(&f, polish, None).await);
        assert_eq!(auto, "mock:selected");
        assert_eq!(id(l.route_engine_with(&f, polish, Some("auto")).await), auto);
        assert_eq!(id(l.route_engine_with(&f, polish, Some("local")).await), "mock:current");
        assert_eq!(id(l.route_engine_with(&f, polish, Some("subscription")).await), "mock:subscription");
        // a named model: that one
        assert_eq!(id(l.route_engine_with(&f, polish, Some("agy:m-1")).await), "mock:google");
        assert_eq!(id(l.route_engine_with(&f, polish, Some("claude:haiku")).await), "mock:haiku");
        assert_eq!(id(l.route_engine_with(&f, polish, Some("local:qwen3.5-9b-q8")).await), "mock:qwen");
        // a model that is not there: an error in one line, never another model
        let e = l.route_engine_with(&f, polish, Some("claude:sonnet")).await.err().unwrap();
        assert!(!e.is_empty() && !e.contains('\n'), "{e}");
        assert!(l.route_engine_with(&f, polish, Some("agy:m-2")).await.is_err());
        // half a name is no name
        assert_eq!(id(l.route_engine_with(&f, polish, Some("claude:")).await), auto);
        // every other task ignores a named model, Google included
        let outline = id(l.route_engine_with(&f, "pc-outline", None).await);
        for name in ["agy:m-1", "claude:haiku", "local:qwen3.5-9b-q8"] {
            assert_eq!(id(l.route_engine_with(&f, "pc-outline", Some(name)).await), outline, "{name}");
        }
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
        inner.keys = SharedKeys { main: ks, old: vec![] };
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
