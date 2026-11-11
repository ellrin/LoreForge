//! Model calls through lore-llm, wired like LoreReader's: the card draft is the task
//! `kb-summarize-card` (`tasks.toml`: `local:current`, then `claude:selected`). The API key
//! comes from the OS keychain (service `KnowledgeBase`) or ANTHROPIC_API_KEY, never a file.
//! Local: the own `[engine] url` / `KB_LLM_URL` when set, else the model all apps share
//! (`private/shared/engine.toml` `[engine] model`) on the machine's one llama-server
//! (lore-llm `shared_runtime`, docs/local-models.md).

use std::path::Path;
use std::sync::Arc;

use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::shared_runtime::SharedRuntime;
use lore_llm::router::{ApiAccess, ExternalServer, LocalAccess, ProviderFactory, RouteError, RouteOptions, StandardFactory, SubscriptionAccess};
use lore_llm::secrets::{Account, SharedKeys};
use lore_llm::{LlmRequest, Router, TaskTable};
use serde::Deserialize;

const TASKS: &str = include_str!("../tasks.toml");
pub const SERVICE: &str = "KnowledgeBase";
pub const TASK_DRAFT: &str = "kb-summarize-card";
/// Short name in the shared local-model registry (and `ports.json`).
pub const APP: &str = "kb";

/// At quit: this app's hold on the shared local model goes; the server stops when no other
/// app uses it.
pub fn quit() {
    SharedRuntime::shutdown_process();
}
const LOCAL_TEMPERATURE: f32 = 0.2;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct EngineConfig {
    url: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct File {
    claude_model: Option<String>,
    engine: EngineConfig,
}

/// What a model answered, and with which engine.
pub struct Answer {
    pub value: serde_json::Value,
    /// Provider id (`llama-server:…`, `claude-code:…`, `claude-api:…`).
    pub engine: String,
}

pub struct Llm {
    catalog: Catalog,
    tasks: TaskTable,
    keys: SharedKeys,
    cli: Cli,
    cc: Arc<Cache>,
    /// The machine's shared llama-server (`None`: tests).
    pub local_rt: Option<Arc<SharedRuntime>>,
    /// Where the shared model choice is kept (`private/shared/engine.toml`).
    shared: Option<lore_home::Home>,
    external: Option<ExternalServer>,
    model: String,
    /// Tests: these providers instead of the real engines.
    fake: Option<Arc<dyn ProviderFactory>>,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

impl Llm {
    /// Engines from `<app dir>/config.toml` and the environment (`KB_LLM_URL`, `KB_LLM_MODEL`).
    pub fn new(app_dir: Option<&Path>) -> Llm {
        let file: File = app_dir
            .map(|d| d.join("config.toml"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        let catalog = Catalog::bundled();
        let user = app_dir.map(|d| d.join("tasks.toml"));
        let tasks = Llm::task_table(user.as_deref()).unwrap_or_else(|e| {
            eprintln!("kb-server: tasks.toml: {e} (using the built-in tasks)");
            Llm::task_table(None).expect("built-in tasks")
        });
        let model = file.claude_model.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| catalog.cloud.default.clone());
        let url = env("KB_LLM_URL").or(file.engine.url);
        Llm {
            catalog,
            tasks,
            keys: SharedKeys::for_app(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            local_rt: SharedRuntime::process(APP),
            shared: None,
            external: url.map(|url| ExternalServer { url, model: env("KB_LLM_MODEL").or(file.engine.model) }),
            model,
            fake: None,
        }
    }

    /// No engine at all, never the real keychain or `claude` CLI (tests).
    pub fn none() -> Llm {
        let mut l = Llm::new(None);
        l.external = None;
        l.local_rt = None;
        l.keys.isolate();
        l.cli = Cli { bin: Some(std::env::temp_dir().join("kb-no-such-claude")), config_dir: None };
        l
    }

    /// Tests: answer through `f` (mock providers).
    pub fn with_factory(f: impl ProviderFactory + 'static) -> Llm {
        let mut l = Llm::none();
        l.fake = Some(Arc::new(f));
        l
    }

    pub fn task_table(user_file: Option<&Path>) -> Result<TaskTable, String> {
        let base = TaskTable::bundled().merge(TaskTable::parse(TASKS)?);
        TaskTable::load(base, user_file.filter(|p| p.is_file()))
    }

    /// Use the shared model choice of `home` (re-read on every call).
    pub fn share_engine(&mut self, home: lore_home::Home) {
        self.shared = Some(home);
    }

    fn factory(&self) -> StandardFactory {
        let mut f = StandardFactory::new(self.catalog.clone());
        f.prefer_subscription = true;
        f.selected_claude = Some(self.model.clone());
        let base = std::env::var("KNOWLEDGEBASE_CLAUDE_BASE_URL").unwrap_or_else(|_| lore_llm::claude_api::DEFAULT_BASE_URL.into());
        f.api = self.keys.get(None, &Account::Single).map(|(k, _)| ApiAccess { base_url: base, api_key: k, fallbacks: true });
        f.subscription = Some(SubscriptionAccess { cli: self.cli.clone(), cache: self.cc.clone(), allow_web: false, temp_prefix: "knowledgebase-claude".into() });
        let e = self.shared.as_ref().and_then(|h| h.engine()).unwrap_or_default();
        f.local = Some(LocalAccess {
            external: self.external.clone(),
            shared: self.local_rt.clone(),
            default_model: e.model,
            fallback_home: self.shared.as_ref().and_then(|h| h.local_home()),
            vision_model: e.vision_model,
            zh_tw: true,
            ..Default::default()
        });
        f
    }

    /// Run a JSON request as `task`. `engine`: `"local"` / `"claude"` pins that engine
    /// (no fallback); anything else = the task's order. Errors are one line for the UI;
    /// the second part is the JSON body (with the one-click `alternative` when there is one).
    pub async fn complete_json(&self, task: &str, req: LlmRequest, engine: Option<&str>) -> Result<Answer, (String, serde_json::Value)> {
        let opts = match engine {
            Some("local") => RouteOptions::only("local:current"),
            Some("claude") => RouteOptions::only("claude:selected"),
            _ => RouteOptions::default(),
        }
        // the 起草摘要 button waits for it: interactive for a local model
        .with_priority(lore_llm::shared_runtime::Priority::Interactive);
        let route = match &self.fake {
            Some(f) => Router::new(self.tasks.clone(), f.clone()).provider_for_with(task, &opts).await,
            None => Router::new(self.tasks.clone(), self.factory()).provider_for_with(task, &opts).await,
        }
        .map_err(|e| (unavailable(&e), alt_payload(&e)))?;
        let req = route.apply(req.temperature(LOCAL_TEMPERATURE));
        let value = lore_llm::complete_json(route.provider.as_ref(), &req).await.map_err(|e| {
            let m = first_line(&format!("模型沒有回答：{e}"));
            (m.clone(), serde_json::json!({ "error": m }))
        })?;
        Ok(Answer { value, engine: route.provider.id().to_string() })
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// The body for the page: `{ error, alternative? }` (the kit toast shows 「改用本機／改用 Claude」).
fn alt_payload(e: &RouteError) -> serde_json::Value {
    let mut v = e.payload(true);
    v["error"] = unavailable(e).into();
    v
}

/// "沒有可用的模型，無法起草：local:current（沒有連接本機模型）；…" as one line.
fn unavailable(e: &RouteError) -> String {
    match e {
        RouteError::Unavailable { skipped, .. } if !skipped.is_empty() => {
            let why: Vec<String> = skipped.iter().map(|k| format!("{}（{}）", k.model, k.reason)).collect();
            first_line(&format!("沒有可用的模型，無法起草：{}", why.join("；")))
        }
        other => first_line(&other.reason()),
    }
}

/// A few characters for the toast: 「本機」 or 「Claude」.
pub fn engine_label(id: &str) -> String {
    if id.starts_with("claude") {
        "Claude".into()
    } else if id.starts_with("mock") {
        "mock".into()
    } else {
        "本機".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_order_is_local_then_claude() {
        let t = Llm::task_table(None).unwrap();
        assert_eq!(t.task[TASK_DRAFT].default, "local:current");
        assert_eq!(t.task[TASK_DRAFT].fallback.as_deref(), Some("claude:selected"));
    }

    #[tokio::test]
    async fn nothing_configured_is_one_line() {
        let e = Llm::none().complete_json(TASK_DRAFT, LlmRequest::prompt("s", "u"), None).await.err().unwrap().0;
        assert!(!e.is_empty() && !e.contains('\n'), "{e}");
    }

    #[test]
    fn labels() {
        assert_eq!(engine_label("llama-server:qwen"), "本機");
        assert_eq!(engine_label("claude-code:haiku"), "Claude");
    }
}
