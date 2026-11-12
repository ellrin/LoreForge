//! The engine choices every app shares (`private/shared/engine.toml`, read and written by
//! lore-home; served by `lore-engines` at `/api/engines`): the Claude connection, a
//! route per task class, and per-task routes.
//!
//! | class | 名稱 | `auto` |
//! |---|---|---|
//! | `create` | 創作與審稿 | Claude, then local |
//! | `tidy` | 整理與格式 | local, then Claude |
//! | `ask` | 問答 | local, then Claude |
//!
//! `cloud` / `local` = that engine only; when it cannot run, the router's error carries a
//! one-click [`Alternative`] (「改用本機」／「改用 Claude」) for that one call.
//!
//! The settings reach every [`Router`](crate::Router) of the process through [`install`]
//! (the app's `lore-engines` mount does it), so apps that build a router per call need no
//! change. Until something is installed, routing is exactly the tasks' own order.
//! [`Router::with_settings`](crate::Router::with_settings) fixes them for one router (tests).
//!
//! The process also remembers which model each task last ran on ([`last_used`]).

use crate::router::ClaudeVia;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

/// What kind of work a task is; decides which engine `auto` tries first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskClass {
    /// 創作與審稿: Claude first.
    Create,
    /// 整理與格式: local first.
    Tidy,
    /// 問答: local when it is there.
    Ask,
}

impl TaskClass {
    pub const ALL: [TaskClass; 3] = [TaskClass::Create, TaskClass::Tidy, TaskClass::Ask];

    pub fn id(self) -> &'static str {
        match self {
            TaskClass::Create => "create",
            TaskClass::Tidy => "tidy",
            TaskClass::Ask => "ask",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TaskClass::Create => "創作與審稿",
            TaskClass::Tidy => "整理與格式",
            TaskClass::Ask => "問答",
        }
    }

    pub fn parse(s: &str) -> Option<TaskClass> {
        TaskClass::ALL.into_iter().find(|c| c.id() == s.trim())
    }

    /// The order of `auto`.
    pub fn auto_order(self) -> [EngineKind; 2] {
        match self {
            TaskClass::Create => [EngineKind::Claude, EngineKind::Local],
            TaskClass::Tidy | TaskClass::Ask => [EngineKind::Local, EngineKind::Claude],
        }
    }
}

/// How a class or a task picks its engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    /// The class order; the second engine when the first cannot run.
    #[default]
    Auto,
    /// Claude only.
    Cloud,
    /// The local model only.
    Local,
}

impl RouteMode {
    pub fn id(self) -> &'static str {
        match self {
            RouteMode::Auto => "auto",
            RouteMode::Cloud => "cloud",
            RouteMode::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<RouteMode> {
        [RouteMode::Auto, RouteMode::Cloud, RouteMode::Local].into_iter().find(|m| m.id() == s.trim())
    }

    /// The one engine kind this mode allows (`None` = both).
    pub fn only(self) -> Option<EngineKind> {
        match self {
            RouteMode::Auto => None,
            RouteMode::Cloud => Some(EngineKind::Claude),
            RouteMode::Local => Some(EngineKind::Local),
        }
    }
}

/// Claude or the local model (a mock counts as local).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    Claude,
    Local,
}

impl EngineKind {
    pub fn other(self) -> EngineKind {
        match self {
            EngineKind::Claude => EngineKind::Local,
            EngineKind::Local => EngineKind::Claude,
        }
    }

    /// The button that runs one call on this engine instead.
    pub fn switch_label(self) -> &'static str {
        match self {
            EngineKind::Claude => "改用 Claude",
            EngineKind::Local => "改用本機",
        }
    }

    /// The kind of a model ref (`claude…:` = Claude, anything else local).
    pub fn of(model_ref: &str) -> EngineKind {
        if model_ref.trim_start().starts_with("claude") { EngineKind::Claude } else { EngineKind::Local }
    }
}

/// The other engine, offered after a `cloud` / `local` route (or an explicit per-call
/// choice) could not run: the UI shows `label` and runs that one call with
/// `RouteOptions::only(model_ref)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alternative {
    pub engine: EngineKind,
    /// 「改用本機」 / 「改用 Claude」.
    pub label: String,
    pub model_ref: String,
}

impl Alternative {
    pub fn new(engine: EngineKind, model_ref: impl Into<String>) -> Alternative {
        Alternative { engine, label: engine.switch_label().into(), model_ref: model_ref.into() }
    }
}

/// The shared choices, typed. `Default` = nothing chosen (every route `auto`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineSettings {
    /// `claude:` refs go this way only (`None` = the app's preference).
    pub claude_via: Option<ClaudeVia>,
    /// `claude:selected` means this model (`None` = the app's own pick).
    pub claude_model: Option<String>,
    pub routes: BTreeMap<TaskClass, RouteMode>,
    /// Task id → mode; wins over the task's class.
    pub overrides: BTreeMap<String, RouteMode>,
}

impl EngineSettings {
    /// From the strings of the shared file; unknown values are left out (= `auto` / unset).
    pub fn from_strings<'a>(
        via: Option<&str>,
        model: Option<&str>,
        routes: impl IntoIterator<Item = (&'a String, &'a String)>,
        overrides: impl IntoIterator<Item = (&'a String, &'a String)>,
    ) -> EngineSettings {
        EngineSettings {
            claude_via: match via.map(str::trim) {
                Some("subscription") => Some(ClaudeVia::Subscription),
                Some("api") => Some(ClaudeVia::Api),
                _ => None,
            },
            claude_model: model.map(str::trim).filter(|m| !m.is_empty()).map(String::from),
            routes: routes.into_iter().filter_map(|(c, m)| Some((TaskClass::parse(c)?, RouteMode::parse(m)?))).collect(),
            overrides: overrides.into_iter().filter_map(|(t, m)| Some((t.clone(), RouteMode::parse(m)?))).collect(),
        }
    }

    pub fn route(&self, class: TaskClass) -> RouteMode {
        self.routes.get(&class).copied().unwrap_or_default()
    }

    /// Task override, else the class route, else `auto`.
    pub fn mode_for(&self, task: &str, class: Option<TaskClass>) -> RouteMode {
        if let Some(m) = self.overrides.get(task).filter(|m| **m != RouteMode::Auto) {
            return *m;
        }
        class.map(|c| self.route(c)).unwrap_or_default()
    }
}

/// Where the process reads the shared settings from (called on every route).
pub type Source = Arc<dyn Fn() -> EngineSettings + Send + Sync>;

static SOURCE: RwLock<Option<Source>> = RwLock::new(None);

/// Make every router of this process follow `source` (the app's mount does this once).
pub fn install(source: Source) {
    *SOURCE.write().unwrap_or_else(|e| e.into_inner()) = Some(source);
}

/// Back to the tasks' own order.
pub fn uninstall() {
    *SOURCE.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed settings, read now (`None` = nothing installed).
pub fn installed() -> Option<EngineSettings> {
    let s = SOURCE.read().unwrap_or_else(|e| e.into_inner()).clone();
    s.map(|f| f())
}

/// Tests that install settings, or route classed tasks without fixed settings, take this.
#[cfg(test)]
pub(crate) static TEST_GLOBAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

static LAST: Mutex<BTreeMap<String, (String, u64)>> = Mutex::new(BTreeMap::new());

/// `llama-server:qwen…` → `local:qwen…`, `claude-subscription:claude-…` → `claude:claude-…`.
pub fn used_ref(provider_id: &str) -> String {
    match provider_id.split_once(':') {
        Some(("llama-server", m)) => format!("local:{m}"),
        Some(("claude" | "claude-subscription", m)) => format!("claude:{m}"),
        _ => provider_id.to_string(),
    }
}

/// Remember that `task` ran on `provider_id` (the router does this for every route).
pub fn record_used(task: &str, provider_id: &str) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    LAST.lock().unwrap_or_else(|e| e.into_inner()).insert(task.to_string(), (used_ref(provider_id), now));
}

/// What `task` last ran on in this process (`local:<model>` / `claude:<model id>`).
pub fn last_used(task: &str) -> Option<String> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).get(task).map(|(r, _)| r.clone())
}

/// [`last_used`] with the Unix time (seconds) it happened.
pub fn last_used_at(task: &str) -> Option<(String, u64)> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).get(task).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProvider;
    use crate::provider::LlmProvider;
    use crate::router::{MockFactory, RouteError, RouteOptions, Router, TaskTable};
    use crate::types::Capabilities;

    const TASKS: &str = r#"
[task.c]
label = "寫段落"
default = "local:qwen"
fallback = "claude:sonnet"
class = "create"

[task.t]
default = "claude:sonnet"
fallback = "local:qwen"
class = "tidy"

[task.a]
default = "claude:sonnet"
class = "ask"

[task.plain]
default = "local:qwen"
fallback = "claude:sonnet"

[task.see]
needs = { vision = true }
default = "claude:selected"
class = "create"
"#;

    fn mock(name: &str, net: bool) -> Arc<dyn LlmProvider> {
        let caps = Capabilities { json: true, vision: true, web: false, streaming: true, context: 100_000, needs_network: net };
        Arc::new(MockProvider::new(vec![format!("from {name}")]).named(name).with_capabilities(caps))
    }

    /// Claude and/or the local model, under every ref the tasks use.
    fn factory(claude: bool, local: bool) -> MockFactory {
        let mut f = MockFactory::default();
        if claude {
            for r in ["claude:sonnet", "claude:selected", "claude:opus", "claude-code:sonnet", "claude-code:selected"] {
                f = f.with(r, mock("claude", true));
            }
        }
        if local {
            for r in ["local:qwen", "local:current"] {
                f = f.with(r, mock("local", false));
            }
        }
        f
    }

    fn router(claude: bool, local: bool, s: EngineSettings) -> Router<MockFactory> {
        Router::new(TaskTable::parse(TASKS).unwrap(), factory(claude, local)).with_settings(s)
    }

    fn routes(class: TaskClass, mode: RouteMode) -> EngineSettings {
        EngineSettings { routes: [(class, mode)].into(), ..Default::default() }
    }

    fn name(k: EngineKind) -> &'static str {
        match k {
            EngineKind::Claude => "claude",
            EngineKind::Local => "local",
        }
    }

    #[tokio::test]
    async fn every_class_route_and_availability() {
        for (task, class) in [("c", TaskClass::Create), ("t", TaskClass::Tidy), ("a", TaskClass::Ask)] {
            for mode in [RouteMode::Auto, RouteMode::Cloud, RouteMode::Local] {
                for (claude, local) in [(true, true), (true, false), (false, true), (false, false)] {
                    let case = format!("{task} {} claude={claude} local={local}", mode.id());
                    let up = |k: EngineKind| match k {
                        EngineKind::Claude => claude,
                        EngineKind::Local => local,
                    };
                    let r = router(claude, local, routes(class, mode)).provider_for(task).await;
                    let allowed: Vec<EngineKind> = match mode.only() {
                        None => class.auto_order().to_vec(),
                        Some(k) => vec![k],
                    };
                    match allowed.iter().find(|k| up(**k)) {
                        Some(k) => {
                            let route = r.unwrap_or_else(|e| panic!("{case}: {e}"));
                            assert_eq!(route.provider.id(), name(*k), "{case}");
                            assert_eq!(route.fallback_used, *k != allowed[0], "{case}");
                            assert_eq!(route.alternative.as_ref().map(|a| a.engine), mode.only().map(EngineKind::other), "{case}");
                        }
                        None => {
                            let e = r.err().unwrap_or_else(|| panic!("{case}: no error"));
                            let alt = e.alternative().map(|a| a.engine);
                            match mode.only() {
                                Some(k) if up(k.other()) => {
                                    assert_eq!(alt, Some(k.other()), "{case}");
                                    let a = e.alternative().unwrap();
                                    assert_eq!(a.label, k.other().switch_label());
                                    // the alternative runs that one call
                                    let once = router(claude, local, routes(class, mode)).provider_for_with(task, &RouteOptions::only(&a.model_ref)).await.unwrap();
                                    assert_eq!(once.provider.id(), name(k.other()), "{case}");
                                }
                                _ => assert_eq!(alt, None, "{case}"),
                            }
                        }
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn without_settings_the_task_order_is_kept() {
        let _g = TEST_GLOBAL.lock().await;
        let r = Router::new(TaskTable::parse(TASKS).unwrap(), factory(true, true));
        assert_eq!(r.candidates("c"), vec!["local:qwen", "claude:sonnet"], "class ignored, nothing filled");
        assert_eq!(r.candidates("a"), vec!["claude:sonnet"]);
        assert_eq!(r.provider_for("c").await.unwrap().provider.id(), "local");
        // with default settings the class order applies and a missing kind is filled
        let r = r.with_settings(EngineSettings::default());
        assert_eq!(r.candidates("c"), vec!["claude:sonnet", "local:qwen"]);
        assert_eq!(r.candidates("a"), vec!["local:current", "claude:sonnet"]);
        assert_eq!(r.candidates("plain"), vec!["local:qwen", "claude:sonnet"], "no class: the task order");
    }

    #[tokio::test]
    async fn task_override_beats_class_and_per_call_choices_beat_both() {
        let mut s = routes(TaskClass::Create, RouteMode::Cloud);
        s.overrides.insert("c".into(), RouteMode::Local);
        s.overrides.insert("plain".into(), RouteMode::Cloud);
        let r = router(true, true, s.clone());
        assert_eq!(r.candidates("c"), vec!["local:qwen"]);
        assert_eq!(r.provider_for("c").await.unwrap().provider.id(), "local");
        assert_eq!(r.candidates("plain"), vec!["claude:sonnet"], "an override restricts a task without a class");
        // auto as an override = follow the class
        s.overrides.insert("c".into(), RouteMode::Auto);
        assert_eq!(router(true, true, s.clone()).candidates("c"), vec!["claude:sonnet"]);

        // a strict per-call choice ignores routes; its failure offers the other engine
        let r = router(true, false, s.clone());
        let e = r.provider_for_with("c", &RouteOptions::only("local:qwen")).await.unwrap_err();
        assert_eq!(e.alternative().map(|a| a.model_ref.as_str()), Some("claude:sonnet"));
        let route = r.provider_for_with("t", &RouteOptions::only("claude:opus")).await.unwrap();
        assert_eq!(route.provider.id(), "claude");
        // a preferred per-call ref and a saved override come first, then the class order
        let r = router(true, true, EngineSettings::default());
        assert_eq!(r.candidates_with("t", &RouteOptions::prefer("claude:opus")), vec!["claude:opus", "local:qwen", "claude:sonnet"]);
        r.set_override("t", Some("claude:opus")).unwrap();
        assert_eq!(r.candidates("t"), vec!["claude:opus", "local:qwen", "claude:sonnet"]);
        // a `local` route drops a saved Claude override
        let r = router(true, true, routes(TaskClass::Tidy, RouteMode::Local));
        r.set_override("t", Some("claude:opus")).unwrap();
        assert_eq!(r.candidates("t"), vec!["local:qwen"]);
    }

    #[tokio::test]
    async fn cloud_offline_offers_the_local_model() {
        let r = router(true, true, routes(TaskClass::Create, RouteMode::Cloud));
        r.set_online(false);
        let e = r.provider_for("c").await.unwrap_err();
        assert!(matches!(&e, RouteError::Unavailable { skipped, .. } if skipped[0].reason == "離線"), "{e:?}");
        assert_eq!(e.alternative().unwrap().engine, EngineKind::Local);
        let p = e.payload(false);
        assert_eq!(p["alternative"]["label"], "改用本機");
        assert_eq!(p["alternative"]["model_ref"], "local:qwen");
        assert!(p["error"].as_str().unwrap().contains("寫段落"));
        // offline and nothing local: no alternative
        let r = router(true, false, routes(TaskClass::Create, RouteMode::Cloud));
        r.set_online(false);
        assert!(r.provider_for("c").await.unwrap_err().alternative().is_none());
        // `local` never offers Claude while offline
        let r = router(true, false, routes(TaskClass::Tidy, RouteMode::Local));
        r.set_online(false);
        assert!(r.provider_for("t").await.unwrap_err().alternative().is_none());
        assert!(!RouteError::UnknownTask("x".into()).payload(true).as_object().unwrap().contains_key("alternative"));
    }

    #[tokio::test]
    async fn the_shared_claude_choice_rewrites_claude_refs() {
        let s = EngineSettings { claude_via: Some(ClaudeVia::Subscription), claude_model: Some("opus".into()), ..Default::default() };
        let r = router(true, true, s);
        assert_eq!(r.candidates("c"), vec!["claude-code:sonnet", "local:qwen"], "a task's own model stays");
        assert_eq!(r.candidates("a"), vec!["local:current", "claude-code:sonnet"]);
        assert_eq!(r.candidates("see"), vec!["claude-code:opus", "local:current"], "selected = the shared model and way in, images too");
        assert_eq!(r.candidates_with("t", &RouteOptions::only("claude-api:selected")), vec!["claude-api:opus"]);
    }

    #[tokio::test]
    async fn installed_settings_reach_routers_and_tables() {
        let _g = TEST_GLOBAL.lock().await;
        let t = TaskTable::parse("[task.only-in-this-test]\ndefault = \"claude:sonnet\"\nfallback = \"local:qwen\"\nclass = \"create\"\n").unwrap();
        install(Arc::new(|| EngineSettings { overrides: [("only-in-this-test".to_string(), RouteMode::Local)].into(), ..Default::default() }));
        assert_eq!(t.candidates_for("only-in-this-test", &RouteOptions::default()), vec!["local:qwen"]);
        let r = Router::new(t.clone(), factory(true, true));
        let route = r.provider_for("only-in-this-test").await.unwrap();
        assert_eq!(route.provider.id(), "local");
        assert_eq!(last_used("only-in-this-test").as_deref(), Some("local"));
        uninstall();
        assert_eq!(t.candidates_for("only-in-this-test", &RouteOptions::default()), vec!["claude:sonnet", "local:qwen"]);
    }

    #[test]
    fn names_and_parsing() {
        assert_eq!(used_ref("llama-server:qwen3.5-9b-q8"), "local:qwen3.5-9b-q8");
        assert_eq!(used_ref("claude-subscription:claude-sonnet-5-5"), "claude:claude-sonnet-5-5");
        assert_eq!(used_ref("claude:claude-opus-5-5"), "claude:claude-opus-5-5");
        record_used("names-test", "llama-server:x");
        assert_eq!(last_used("names-test").as_deref(), Some("local:x"));
        let routes = [("create".to_string(), "cloud".to_string()), ("bogus".to_string(), "local".to_string()), ("tidy".to_string(), "sky".to_string())];
        let over = [("pc-x".to_string(), "local".to_string())];
        let s = EngineSettings::from_strings(Some("api"), Some(" sonnet "), routes.iter().map(|(a, b)| (a, b)), over.iter().map(|(a, b)| (a, b)));
        assert_eq!(s.claude_via, Some(ClaudeVia::Api));
        assert_eq!(s.claude_model.as_deref(), Some("sonnet"));
        assert_eq!(s.routes, [(TaskClass::Create, RouteMode::Cloud)].into());
        assert_eq!(s.mode_for("pc-x", Some(TaskClass::Create)), RouteMode::Local);
        assert_eq!(s.mode_for("pc-y", Some(TaskClass::Create)), RouteMode::Cloud);
        assert_eq!(s.mode_for("pc-y", Some(TaskClass::Ask)), RouteMode::Auto);
        assert_eq!(EngineKind::of("claude-code:haiku"), EngineKind::Claude);
        assert_eq!(EngineKind::of("mock:x"), EngineKind::Local);
        assert!(TaskTable::parse("[task.x]\ndefault = \"local:q\"\nclass = \"draw\"\n").is_err(), "an unknown class is an error");
    }
}
