//! Model calls through lore-llm: the 論文報告 deck (task `sc-paper-deck`, this crate's
//! `tasks.toml`) and the module boxes of an architecture figure (lore-llm's
//! `sc-crop-modules`). Engines come from the shared settings (`private/shared/engine.toml`,
//! the 模型 tab) and the shared keychain item; never a key file. Tests answer through
//! [`Llm::with_factory`] (mock providers).

use std::sync::Arc;

use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::router::{ApiAccess, LocalAccess, ProviderFactory, RouteError, RouteOptions, StandardFactory, SubscriptionAccess};
use lore_llm::shared_runtime::SharedRuntime;
use lore_llm::secrets::{Account, SharedKeys};
use lore_llm::{LlmRequest, Route, Router, TaskTable};

pub const TASKS: &str = include_str!("../tasks.toml");
pub const SERVICE: &str = "SlideComposer";
/// Short name in the shared local-model registry (and `ports.json`).
pub const APP: &str = "sc";

/// At quit: this app's hold on the shared local model goes; the server stops when no other
/// app uses it.
pub fn quit() {
    SharedRuntime::shutdown_process();
}
pub const TASK_PAPER_DECK: &str = "sc-paper-deck";
/// 微調's 建議 (todeck/suggest.rs).
pub const TASK_OUTLINE_REVIEW: &str = "sc-outline-review";
const TEMPERATURE: f32 = 0.3;

/// What a model answered, and with which engine.
pub struct Answer {
    pub value: serde_json::Value,
    /// Provider id (`llama-server:…`, `claude-code:…`, `claude-api:…`, `mock…`).
    pub engine: String,
}

pub struct Llm {
    home: lore_home::Home,
    catalog: Catalog,
    tasks: TaskTable,
    keys: SharedKeys,
    cli: Cli,
    cc: Arc<Cache>,
    /// The machine's shared llama-server (`None`: no local engine).
    shared: Option<Arc<SharedRuntime>>,
    no_local: bool,
    fake: Option<Arc<dyn ProviderFactory>>,
}

/// This app's tasks (its own, lore-flow's, lore-plot's and lore-polish's), as the 模型 tab lists them.
pub fn app_tasks() -> TaskTable {
    TaskTable::parse(TASKS)
        .expect("bundled tasks.toml parses")
        .merge(TaskTable::parse(include_str!("../../../../commons/crates/lore-flow/tasks.toml")).expect("lore-flow tasks parse"))
        .merge(lore_plot::assist::task_table())
        .merge(lore_polish::task_table())
        .merge(lore_cards::search::task_table())
}

impl Llm {
    pub fn new(home: lore_home::Home) -> Llm {
        Llm {
            home,
            catalog: Catalog::bundled(),
            tasks: TaskTable::bundled().merge(app_tasks()),
            keys: SharedKeys::for_app(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            shared: SharedRuntime::process(APP),
            no_local: false,
            fake: None,
        }
    }

    /// No engine at all: never the keychain, the `claude` CLI or a local server (tests).
    pub fn none(home: lore_home::Home) -> Llm {
        let mut l = Llm::new(home);
        l.keys.isolate();
        l.cli = Cli { bin: Some(std::env::temp_dir().join("sc-no-such-claude")), config_dir: None };
        l.no_local = true;
        l.shared = None;
        l
    }

    /// Tests: answer through `f` (mock providers).
    pub fn with_factory(home: lore_home::Home, f: impl ProviderFactory + 'static) -> Llm {
        let mut l = Llm::none(home);
        l.fake = Some(Arc::new(f));
        l
    }

    fn factory(&self, api_key: Option<String>) -> StandardFactory {
        let settings = self.home.engine_settings();
        let mut f = StandardFactory::new(self.catalog.clone());
        f.prefer_subscription = true;
        f.selected_claude = settings.claude.model.clone();
        let base = std::env::var("SLIDECOMPOSER_CLAUDE_BASE_URL").unwrap_or_else(|_| lore_llm::claude_api::DEFAULT_BASE_URL.into());
        f.api = api_key.map(|k| ApiAccess { base_url: base, api_key: k, fallbacks: true });
        f.subscription = Some(SubscriptionAccess { cli: self.cli.clone(), cache: self.cc.clone(), allow_web: false, temp_prefix: "slidecomposer-claude".into() });
        if !self.no_local {
            // the shared default model of engine.toml, acquired from the machine's one llama-server
            let e = settings.engine.clone().unwrap_or_default();
            f.local = Some(LocalAccess { shared: self.shared.clone(), default_model: e.model, vision_model: e.vision_model, fallback_home: self.home.local_home(), zh_tw: true, ..Default::default() });
        }
        f
    }

    /// The provider for `task`. `engine`: `"local"` / `"claude"` pins that engine (no fallback).
    /// 語句潤飾 alone may also name its model (`claude:<alias>`, `local:<catalog id>`,
    /// `agy:<model id>`: L-Echo's model menu): that model or an error, never another one.
    pub async fn route(&self, task: &str, engine: Option<&str>) -> Result<Route, RouteError> {
        let opts = match engine {
            Some("local") => RouteOptions::only("local:current"),
            Some("claude") => RouteOptions::only("claude:selected"),
            Some(m) if task == lore_polish::TASK && named_model(m) => RouteOptions::only(m),
            _ => RouteOptions::default(),
        }
        // someone watches the result (the 論文報告 progress, a button): interactive for a local model
        .with_priority(lore_llm::shared_runtime::Priority::Interactive);
        match &self.fake {
            Some(f) => Router::new(self.tasks.clone(), f.clone()).provider_for_with(task, &opts).await,
            None => {
                // a keychain read can wait on a password prompt: off the async threads
                let keys = self.keys.clone();
                let key = tokio::task::spawn_blocking(move || keys.get(None, &Account::Single).map(|(k, _)| k)).await.ok().flatten();
                Router::new(self.tasks.clone(), self.factory(key)).provider_for_with(task, &opts).await
            }
        }
    }

    /// Run a JSON request as `task`. Errors: one line for the UI and the JSON body (with the
    /// one-click `alternative` the kit's toast offers as 改用本機／改用 Claude).
    pub async fn complete_json(&self, task: &str, req: LlmRequest, engine: Option<&str>) -> Result<Answer, (String, serde_json::Value)> {
        let route = self.route(task, engine).await.map_err(|e| (unavailable(&e), alt_payload(&e)))?;
        let req = route.apply(req.temperature(TEMPERATURE));
        let value = lore_llm::complete_json(route.provider.as_ref(), &req).await.map_err(|e| {
            let m = first_line(&format!("模型沒有回答：{e}"));
            (m.clone(), serde_json::json!({ "error": m }))
        })?;
        let id = route.provider.id();
        lore_llm::shared::record_used(task, &id);
        Ok(Answer { value, engine: id })
    }
}

/// `claude:<alias>`, `local:<catalog id>` or `agy:<model id>`, the id not empty.
fn named_model(m: &str) -> bool {
    ["claude:", "local:", "agy:"].iter().any(|p| m.strip_prefix(p).is_some_and(|id| !id.trim().is_empty()))
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn alt_payload(e: &RouteError) -> serde_json::Value {
    let mut v = e.payload(true);
    v["error"] = unavailable(e).into();
    v
}

fn unavailable(e: &RouteError) -> String {
    match e {
        RouteError::Unavailable { skipped, .. } if !skipped.is_empty() => {
            let why: Vec<String> = skipped.iter().map(|k| format!("{}（{}）", k.model, k.reason)).collect();
            first_line(&format!("沒有可用的模型：{}", why.join("；")))
        }
        other => first_line(&other.reason()),
    }
}

/// 「本機」 or 「Claude」 for the toast.
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
    fn paper_deck_tasks_are_opus_only_never_the_local_model() {
        // 論文報告 must be right: Opus, then Sonnet; no class, so the shared routing can never put the
        // local model first (the two vision tasks once ran on it: no module boxes, misread equations)
        let t = TaskTable::bundled().merge(app_tasks());
        for id in [TASK_PAPER_DECK, TASK_OUTLINE_REVIEW, crate::todeck::eqread::TASK, crate::todeck::figparts::TASK, crate::todeck::extract::TASK_LOCATE, crate::todeck::extract::TASK_CHECK, lore_docsrc::boxer::TASK_ID] {
            let spec = &t.task[id];
            assert_eq!((spec.default.as_str(), spec.fallback.as_deref(), spec.class), ("claude:opus", Some("claude:sonnet"), None), "{id}");
        }
        assert!(t.task.contains_key("sc-flow-graph"));
        let polish = &t.task[lore_polish::TASK];
        assert_eq!((polish.class, polish.default.as_str()), (Some(lore_llm::TaskClass::Create), "claude:selected"));
    }

    #[tokio::test]
    async fn only_sentence_polish_can_name_its_model() {
        use lore_llm::MockProvider;
        use lore_llm::router::MockFactory;
        let mock = |name: &str| -> Arc<dyn lore_llm::LlmProvider> { Arc::new(MockProvider::new(vec![]).named(name)) };
        let dir = tempfile::tempdir().unwrap();
        let f = MockFactory::default()
            .with("claude:selected", mock("mock:selected"))
            .with("local:current", mock("mock:current"))
            .with("local:qwen3.5-9b-q8", mock("mock:qwen"))
            .with("claude:haiku", mock("mock:haiku"))
            .with("claude:opus", mock("mock:opus"))
            .with("agy:m-1", mock("mock:google"));
        let llm = Llm::with_factory(lore_home::Home::at(dir.path()), f);
        let id = |r: Route| r.provider.id();
        let polish = lore_polish::TASK;
        // as before: the task's order, and the two engine words
        let auto = id(llm.route(polish, None).await.unwrap());
        assert!(auto == "mock:selected" || auto == "mock:current", "{auto}");
        assert_eq!(id(llm.route(polish, Some("local")).await.unwrap()), "mock:current");
        assert_eq!(id(llm.route(polish, Some("claude")).await.unwrap()), "mock:selected");
        // a named model: that one
        assert_eq!(id(llm.route(polish, Some("agy:m-1")).await.unwrap()), "mock:google");
        assert_eq!(id(llm.route(polish, Some("claude:haiku")).await.unwrap()), "mock:haiku");
        assert_eq!(id(llm.route(polish, Some("local:qwen3.5-9b-q8")).await.unwrap()), "mock:qwen");
        // a model that is not there: an error in one line, never another model
        let e = llm.route(polish, Some("claude:sonnet")).await.err().unwrap();
        let m = unavailable(&e);
        assert!(!m.is_empty() && !m.contains('\n'), "{m}");
        assert!(llm.route(polish, Some("agy:m-2")).await.is_err());
        // half a name is no name
        assert_eq!(id(llm.route(polish, Some("claude:")).await.unwrap()), auto);
        // every other task ignores a named model, Google included
        for name in ["agy:m-1", "claude:haiku", "local:qwen3.5-9b-q8"] {
            assert_eq!(id(llm.route(TASK_PAPER_DECK, Some(name)).await.unwrap()), "mock:opus", "{name}");
        }
    }

    #[tokio::test]
    async fn nothing_configured_is_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let e = Llm::none(lore_home::Home::at(dir.path())).complete_json(TASK_PAPER_DECK, LlmRequest::prompt("s", "u"), None).await.err().unwrap().0;
        assert!(!e.is_empty() && !e.contains('\n'), "{e}");
    }
}
