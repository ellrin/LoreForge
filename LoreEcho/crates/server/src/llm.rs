//! Model calls through lore-llm: the chat is the task `le-chat` (`tasks.toml`, class `ask`:
//! the local model first, Claude when it is not connected; the shared 模型 tab can change
//! that per class or per task). The API key comes from the shared LoreForge keychain item or
//! ANTHROPIC_API_KEY, never a file.
//!
//! The local engine (`local:current`): `LOREECHO_LLM_URL` (+ `LOREECHO_LLM_MODEL`), else the
//! model all LoreForge apps share (`private/shared/engine.toml` `[engine] model`, re-read on
//! every call) on the machine's one llama-server: lore-llm `shared_runtime` acquires it per
//! request and starts it when nobody has (docs/local-models.md).

use std::sync::Arc;

use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::router::{ApiAccess, ExternalServer, LocalAccess, ProviderFactory, RouteError, StandardFactory, SubscriptionAccess};
use lore_llm::secrets::{Account, SharedKeys};
use lore_llm::shared_runtime::SharedRuntime;
use lore_llm::{Route, RouteOptions, Router, TaskTable};
use serde_json::Value;

pub const TASKS: &str = include_str!("../tasks.toml");
/// The keychain service read before the shared one's old names.
pub const SERVICE: &str = "LoreEcho";
pub const TASK_CHAT: &str = "le-chat";
/// Short name in the shared local-model registry (and `ports.json`).
pub const APP: &str = "le";

/// At quit: this app's hold on the shared local model goes; the server stops when no other
/// app uses it.
pub fn quit() {
    SharedRuntime::shutdown_process();
}

pub struct Llm {
    catalog: Catalog,
    tasks: TaskTable,
    keys: SharedKeys,
    cli: Cli,
    cc: Arc<Cache>,
    /// `LOREECHO_LLM_URL`, when set.
    external: Option<ExternalServer>,
    /// The machine's shared llama-server (`None`: tests).
    pub local_rt: Option<Arc<SharedRuntime>>,
    /// Where the shared engine is kept (`private/shared/engine.toml`).
    shared: Option<lore_home::Home>,
    base_url: String,
    /// Tests: these providers instead of the real engines.
    fake: Option<Arc<dyn ProviderFactory>>,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

impl Llm {
    /// The real engines: the shared local engine of `home`, Claude through the subscription
    /// or the shared key. `user_tasks` = an optional `tasks.toml` with an `[override]`.
    pub fn new(home: Option<lore_home::Home>, user_tasks: Option<&std::path::Path>) -> Llm {
        let tasks = Llm::task_table(user_tasks).unwrap_or_else(|e| {
            eprintln!("le-server: tasks.toml: {e} (using the built-in tasks)");
            Llm::task_table(None).expect("built-in tasks")
        });
        Llm {
            catalog: Catalog::bundled(),
            tasks,
            keys: SharedKeys::for_app(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            external: env("LOREECHO_LLM_URL").map(|url| ExternalServer { url, model: env("LOREECHO_LLM_MODEL") }),
            local_rt: home.as_ref().and_then(|_| SharedRuntime::process(APP)),
            shared: home,
            base_url: env("LOREECHO_CLAUDE_BASE_URL").unwrap_or_else(|| lore_llm::claude_api::DEFAULT_BASE_URL.into()),
            fake: None,
        }
    }

    /// No engine at all: never the real keychain, environment key, `claude` CLI or a local
    /// server (tests).
    pub fn none() -> Llm {
        let mut l = Llm::new(None, None);
        l.external = None;
        l.shared = None;
        l.local_rt = None;
        l.keys.isolate();
        l.cli = Cli { bin: Some(std::env::temp_dir().join("le-no-such-claude")), config_dir: None };
        l
    }

    /// Tests: answer through `f` (mock providers).
    pub fn with_factory(f: impl ProviderFactory + 'static) -> Llm {
        let mut l = Llm::none();
        l.fake = Some(Arc::new(f));
        l
    }

    /// The app's tasks over lore-llm's shared ones, then the user's file.
    pub fn task_table(user_file: Option<&std::path::Path>) -> Result<TaskTable, String> {
        let base = TaskTable::bundled().merge(TaskTable::parse(TASKS)?);
        TaskTable::load(base, user_file.filter(|p| p.is_file()))
    }

    /// The own server: `LOREECHO_LLM_URL`, when set.
    fn external(&self) -> Option<ExternalServer> {
        self.external.clone()
    }

    fn factory(&self) -> StandardFactory {
        let mut f = StandardFactory::new(self.catalog.clone());
        f.prefer_subscription = true;
        f.api = self.keys.get(None, &Account::Single).map(|(k, _)| ApiAccess { base_url: self.base_url.clone(), api_key: k, fallbacks: true });
        f.subscription = Some(SubscriptionAccess { cli: self.cli.clone(), cache: self.cc.clone(), allow_web: false, temp_prefix: "loreecho-claude".into() });
        let e = self.shared.as_ref().and_then(|h| h.engine()).unwrap_or_default();
        f.local = Some(LocalAccess {
            external: self.external(),
            shared: self.local_rt.clone(),
            default_model: e.model,
            fallback_home: self.shared.as_ref().and_then(|h| h.local_home()),
            vision_model: e.vision_model,
            zh_tw: true,
            ..Default::default()
        });
        f
    }

    /// The engine for `task`. `engine`: `"local"` / `"claude"` pins that engine (the
    /// one-click alternative after a failure); anything else = the task's order and the
    /// shared routes. `Err` = the JSON body for the page (`{ error, alternative? }`).
    pub async fn route(&self, task: &str, engine: Option<&str>) -> Result<Route, Value> {
        let (opts, strict) = match engine {
            Some("local") => (RouteOptions::only("local:current"), true),
            Some("claude") => (RouteOptions::only("claude:selected"), true),
            _ => (RouteOptions::default(), false),
        };
        let r = match &self.fake {
            Some(f) => Router::new(self.tasks.clone(), f.clone()).provider_for_with(task, &opts).await,
            None => Router::new(self.tasks.clone(), self.factory()).provider_for_with(task, &opts).await,
        };
        r.map_err(|e| payload(&e, strict))
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// `{ error, alternative? }` with a one-line error.
fn payload(e: &RouteError, strict: bool) -> Value {
    let mut v = e.payload(strict);
    let msg = match e {
        RouteError::Unavailable { skipped, .. } if !strict && !skipped.is_empty() => {
            let why: Vec<String> = skipped.iter().map(|k| format!("{}（{}）", k.model, k.reason)).collect();
            format!("沒有可用的模型：{}", why.join("；"))
        }
        _ => v["error"].as_str().unwrap_or("模型呼叫失敗").to_string(),
    };
    v["error"] = first_line(&msg).into();
    v
}

/// A few characters for the page: 「本機」 or 「Claude」.
pub fn engine_label(id: &str) -> &'static str {
    if id.starts_with("claude") {
        "Claude"
    } else if id.starts_with("mock") {
        "mock"
    } else {
        "本機"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_task_is_ask_local_then_claude() {
        let t = Llm::task_table(None).unwrap();
        let chat = &t.task[TASK_CHAT];
        assert_eq!(chat.default, "local:current");
        assert_eq!(chat.fallback.as_deref(), Some("claude:selected"));
        assert_eq!(chat.class, Some(lore_llm::TaskClass::Ask));
    }

    #[tokio::test]
    async fn nothing_configured_is_one_line() {
        let e = Llm::none().route(TASK_CHAT, None).await.err().unwrap();
        let m = e["error"].as_str().unwrap();
        assert!(!m.is_empty() && !m.contains('\n'), "{m}");
    }

    #[test]
    fn labels() {
        assert_eq!(engine_label("llama-server:qwen"), "本機");
        assert_eq!(engine_label("claude-code:haiku"), "Claude");
    }
}
