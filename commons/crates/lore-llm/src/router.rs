//! Task routing (design §9): every LLM job is a task declared in
//! `tasks.toml` with what it needs; [`Router::provider_for`] picks the first
//! model (user override, default, fallback) that is available and satisfies
//! the needs, and says clearly why when none is (offline, not signed in, not
//! downloaded, cannot read images…).
//!
//! The router does not know how to build engines: a [`ProviderFactory`] does.
//! [`StandardFactory`] covers Claude (API key / subscription) and local
//! llama-server; tests use [`MockFactory`].
//!
//! Two refs mean "what the user set up in the app" rather than a catalog
//! entry: `local:current` ([`LOCAL_CURRENT`]: the external server, or the
//! shared default model of `engine.toml`, acquired from the machine's one
//! llama-server, [`crate::shared_runtime`]) and `claude:selected`
//! ([`CLAUDE_SELECTED`]: [`StandardFactory::selected_claude`]). A per-call
//! engine choice goes through [`Router::provider_for_with`] and
//! [`RouteOptions`] (`strict`: that engine or an error, never a fallback).

use crate::catalog::{Catalog, CloudModel};
use crate::claude_api::{ClaudeApi, ClaudeApiConfig};
use crate::claude_code::{Cache, ClaudeCode, Cli};
use crate::local::{self, Layout};
use crate::openai::{OpenAiCompat, OpenAiCompatConfig};
use crate::provider::LlmProvider;
use crate::shared::{self, Alternative, EngineKind, EngineSettings, RouteMode, TaskClass};
use crate::shared_runtime::{Priority, SharedLocal, SharedRuntime};
use crate::types::{LlmRequest, Needs};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

const BUNDLED: &str = include_str!("../tasks.toml");

/// `local:current`: the external server when the app has one, else the shared default
/// local model (`engine.toml` `[engine] model`; `vision_model` for a task that reads images).
pub const LOCAL_CURRENT: &str = "current";
/// `claude:selected` (also `claude-api:selected`, `claude-code:selected`):
/// the Claude model the app's settings picked ([`StandardFactory::selected_claude`]).
pub const CLAUDE_SELECTED: &str = "selected";
/// Context assumed for a selected Claude model that is not in the catalog.
const UNKNOWN_CLOUD_CONTEXT: u32 = 200_000;

// ── model refs ──────────────────────────────────────────

/// Which way into Claude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeVia {
    /// Whatever the app has set up (subscription first when preferred).
    Auto,
    Api,
    Subscription,
}

/// A model reference as written in tasks.toml: `claude:sonnet`,
/// `claude-api:…`, `claude-code:…`, `local:<catalog id or prefix>`, `mock:<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRef {
    Claude { model: String, via: ClaudeVia },
    Local(String),
    Mock(String),
}

impl ModelRef {
    pub fn parse(s: &str) -> Result<ModelRef, String> {
        let (kind, name) = s.trim().split_once(':').ok_or_else(|| format!("模型設定「{s}」少了種類（例如 claude:sonnet、local:qwen3.5-9b-q8）"))?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(format!("模型設定「{s}」少了模型名稱"));
        }
        Ok(match kind.trim() {
            "claude" => ModelRef::Claude { model: name, via: ClaudeVia::Auto },
            "claude-api" => ModelRef::Claude { model: name, via: ClaudeVia::Api },
            "claude-code" | "claude-subscription" => ModelRef::Claude { model: name, via: ClaudeVia::Subscription },
            "local" => ModelRef::Local(name),
            "mock" => ModelRef::Mock(name),
            other => return Err(format!("不認得的模型種類「{other}」")),
        })
    }

    /// Cloud models need the internet; local ones and mocks do not.
    pub fn needs_network(&self) -> bool {
        matches!(self, ModelRef::Claude { .. })
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelRef::Claude { model, via: ClaudeVia::Auto } => write!(f, "claude:{model}"),
            ModelRef::Claude { model, via: ClaudeVia::Api } => write!(f, "claude-api:{model}"),
            ModelRef::Claude { model, via: ClaudeVia::Subscription } => write!(f, "claude-code:{model}"),
            ModelRef::Local(m) => write!(f, "local:{m}"),
            ModelRef::Mock(m) => write!(f, "mock:{m}"),
        }
    }
}

// ── tasks.toml ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSpec {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub needs: Needs,
    pub default: String,
    #[serde(default)]
    pub fallback: Option<String>,
    /// `Some(false)`: disabled offline. `None`: runs offline when a candidate is local.
    #[serde(default)]
    pub offline: Option<bool>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// `create` | `tidy` | `ask`: which engine `auto` tries first once shared settings
    /// are installed ([`crate::shared`]). Unset = the task's own order.
    #[serde(default)]
    pub class: Option<TaskClass>,
    /// Local-model priority; unset = the class default ([`Priority::for_class`]). The
    /// router passes the effective one (a call's [`RouteOptions::priority`] first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
}

/// A tasks.toml file: task definitions and user overrides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskTable {
    #[serde(default)]
    pub task: BTreeMap<String, TaskSpec>,
    /// Task id → model ref chosen by the user (the per-task model picker).
    #[serde(default, rename = "override")]
    pub overrides: BTreeMap<String, String>,
}

impl TaskTable {
    /// The shared sample (tasks of design §9 and the current apps).
    pub fn bundled() -> TaskTable {
        TaskTable::parse(BUNDLED).expect("lore-llm tasks.toml is valid")
    }

    pub fn parse(s: &str) -> Result<TaskTable, String> {
        let t: TaskTable = toml::from_str(s).map_err(|e| format!("tasks.toml：{e}"))?;
        t.check()?;
        Ok(t)
    }

    /// Every model ref parses and every override names a known task.
    pub fn check(&self) -> Result<(), String> {
        for (id, t) in &self.task {
            ModelRef::parse(&t.default).map_err(|e| format!("[task.{id}] default：{e}"))?;
            if let Some(f) = &t.fallback {
                ModelRef::parse(f).map_err(|e| format!("[task.{id}] fallback：{e}"))?;
            }
        }
        for (id, r) in &self.overrides {
            ModelRef::parse(r).map_err(|e| format!("[override] {id}：{e}"))?;
        }
        Ok(())
    }

    /// Layer `other` (an app's or the user's file) on top: its tasks replace
    /// same-named ones, its overrides win.
    pub fn merge(mut self, other: TaskTable) -> TaskTable {
        self.task.extend(other.task);
        self.overrides.extend(other.overrides);
        self
    }

    /// [`plan`]'s candidates from this table, with the installed shared settings (what a
    /// [`Router`] over this table would try; for callers that walk candidates themselves).
    pub fn candidates_for(&self, task: &str, opts: &RouteOptions) -> Vec<String> {
        let Some(t) = self.task.get(task) else { return vec![] };
        plan(task, t, self.overrides.get(task).cloned(), opts, shared::installed().as_ref()).candidates
    }

    /// `base` plus the user's file when it exists and parses (a broken user
    /// file is reported, not silently ignored).
    pub fn load(base: TaskTable, user_file: Option<&Path>) -> Result<TaskTable, String> {
        match user_file.filter(|p| p.exists()) {
            Some(p) => {
                let s = std::fs::read_to_string(p).map_err(|e| format!("{}：{e}", p.display()))?;
                Ok(base.merge(TaskTable::parse(&s)?))
            }
            None => Ok(base),
        }
    }
}

// ── factories ───────────────────────────────────────────

/// Builds engines for model refs. `Err` = why that model is unavailable now,
/// in the user's words. Implementations should check `task.needs` before
/// doing anything expensive (starting a local server).
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    async fn make(&self, model: &ModelRef, task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String>;

    /// Could `model` run `task` now, without starting anything? Decides whether a
    /// one-click [`Alternative`] is offered. Default: yes.
    async fn check(&self, _model: &ModelRef, _task: &TaskSpec) -> Result<(), String> {
        Ok(())
    }
}

/// Fixed providers by ref string (`"mock:a"`, `"claude:sonnet"`…), for tests.
#[derive(Default, Clone)]
pub struct MockFactory(pub BTreeMap<String, Arc<dyn LlmProvider>>);

impl MockFactory {
    pub fn with(mut self, r: &str, p: Arc<dyn LlmProvider>) -> Self {
        self.0.insert(r.to_string(), p);
        self
    }
}

#[async_trait]
impl<F: ProviderFactory + ?Sized> ProviderFactory for Arc<F> {
    async fn make(&self, model: &ModelRef, task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String> {
        (**self).make(model, task).await
    }
    async fn check(&self, model: &ModelRef, task: &TaskSpec) -> Result<(), String> {
        (**self).check(model, task).await
    }
}

#[async_trait]
impl<F: ProviderFactory + ?Sized> ProviderFactory for &F {
    async fn make(&self, model: &ModelRef, task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String> {
        (**self).make(model, task).await
    }
    async fn check(&self, model: &ModelRef, task: &TaskSpec) -> Result<(), String> {
        (**self).check(model, task).await
    }
}

#[async_trait]
impl ProviderFactory for MockFactory {
    async fn make(&self, model: &ModelRef, _task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String> {
        self.0.get(&model.to_string()).cloned().ok_or_else(|| "沒有設定".to_string())
    }
    async fn check(&self, model: &ModelRef, _task: &TaskSpec) -> Result<(), String> {
        self.0.get(&model.to_string()).map(|_| ()).ok_or_else(|| "沒有設定".to_string())
    }
}

// ── router ──────────────────────────────────────────────

/// A candidate that was passed over, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skip {
    pub model: String,
    pub reason: String,
}

/// The chosen engine for one task.
#[derive(Clone)]
pub struct Route {
    pub task: String,
    pub model: ModelRef,
    pub provider: Arc<dyn LlmProvider>,
    /// Not the first choice (override or default).
    pub fallback_used: bool,
    /// Earlier candidates and why they were skipped (for a UI note).
    pub skipped: Vec<Skip>,
    pub effort: Option<String>,
    pub max_tokens: Option<u32>,
    /// After a `cloud` / `local` route or a strict choice: the other engine, for a call
    /// that fails once it runs (not checked; [`RouteError`]'s alternative is).
    pub alternative: Option<Alternative>,
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("task", &self.task)
            .field("model", &self.model)
            .field("provider", &self.provider.id())
            .field("fallback_used", &self.fallback_used)
            .field("skipped", &self.skipped)
            .finish()
    }
}

impl Route {
    /// The task's effort and max_tokens, unless the request sets its own.
    pub fn apply(&self, mut req: LlmRequest) -> LlmRequest {
        if req.effort.is_none() {
            req.effort = self.effort.clone();
        }
        if let Some(n) = self.max_tokens
            && req.max_tokens == LlmRequest::DEFAULT_MAX_TOKENS
        {
            req.max_tokens = n;
        }
        req
    }
}

/// Per-call routing choices (nothing shared is changed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteOptions {
    /// A model ref for this call only (an engine switch in the UI), tried
    /// before the user's saved override and the task's own order.
    pub override_ref: Option<String>,
    /// Only `override_ref` is tried: an explicit engine choice fails with
    /// the reason instead of silently falling back. Without `override_ref`
    /// it has no effect.
    pub strict: bool,
    /// How urgent a local-model call is (someone waits on screen: `Interactive`);
    /// `None` = the task's, else its class default.
    pub priority: Option<Priority>,
}

impl RouteOptions {
    /// This engine for this call, or an error (no fallback).
    pub fn only(model_ref: impl Into<String>) -> RouteOptions {
        RouteOptions { override_ref: Some(model_ref.into()), strict: true, priority: None }
    }
    /// This engine first, then the task's usual candidates.
    pub fn prefer(model_ref: impl Into<String>) -> RouteOptions {
        RouteOptions { override_ref: Some(model_ref.into()), strict: false, priority: None }
    }
    /// The same, at this local-model priority.
    pub fn with_priority(mut self, p: Priority) -> RouteOptions {
        self.priority = Some(p);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("沒有這個 LLM 任務：{0}")]
    UnknownTask(String),
    #[error("「{label}」需要網路，現在離線，暫時無法使用")]
    NeedsNetwork { task: String, label: String },
    #[error("「{label}」目前沒有可用的模型：{}", list(.skipped))]
    Unavailable {
        task: String,
        label: String,
        skipped: Vec<Skip>,
        /// The other engine, when only one kind was allowed (a `cloud` / `local` route
        /// or a strict per-call choice) and the other one could run.
        alternative: Option<Box<Alternative>>,
    },
}

fn list(s: &[Skip]) -> String {
    s.iter().map(|k| format!("{}（{}）", k.model, k.reason)).collect::<Vec<_>>().join("；")
}

impl RouteError {
    /// The first reason a candidate was passed over (for a strict choice:
    /// why that engine cannot run), else the whole message.
    pub fn reason(&self) -> String {
        match self {
            RouteError::Unavailable { skipped, .. } if !skipped.is_empty() => skipped[0].reason.clone(),
            other => other.to_string(),
        }
    }

    /// The one-click alternative (「改用本機」／「改用 Claude」), when there is one.
    pub fn alternative(&self) -> Option<&Alternative> {
        match self {
            RouteError::Unavailable { alternative, .. } => alternative.as_deref(),
            _ => None,
        }
    }

    /// The JSON body for the UI: `{ "error": "…", "alternative": { engine, label, model_ref } }`
    /// (`alternative` only when there is one). `reason` = [`RouteError::reason`] instead of
    /// the whole message (an explicit choice).
    pub fn payload(&self, reason: bool) -> serde_json::Value {
        let mut v = serde_json::json!({ "error": if reason { self.reason() } else { self.to_string() } });
        if let Some(a) = self.alternative() {
            v["alternative"] = serde_json::to_value(a).unwrap_or_default();
        }
        v
    }
}

/// The candidates of one call, and what restricts them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub candidates: Vec<String>,
    /// Only this kind may run: a `cloud` / `local` route, or a strict per-call choice.
    pub only: Option<EngineKind>,
    /// The restriction comes from a route (not from a per-call choice).
    pub by_route: bool,
    /// Refs of the other kind, best first (for the alternative).
    pub others: Vec<String>,
    pub mode: RouteMode,
}

/// `claude:selected` → the shared Claude model, `claude:` → the shared way in (not for
/// tasks that need images: the subscription cannot read them, the API key may).
fn rewrite(r: &str, s: &EngineSettings, spec: &TaskSpec) -> String {
    match ModelRef::parse(r) {
        Ok(ModelRef::Claude { model, via }) => {
            let model = match &s.claude_model {
                Some(m) if model == CLAUDE_SELECTED => m.clone(),
                _ => model,
            };
            let via = match s.claude_via {
                Some(v) if via == ClaudeVia::Auto && !spec.needs.vision => v,
                _ => via,
            };
            ModelRef::Claude { model, via }.to_string()
        }
        _ => r.to_string(),
    }
}

fn push(out: &mut Vec<String>, r: String) {
    if !out.contains(&r) {
        out.push(r);
    }
}

/// Which refs to try for `task`, in order.
///
/// - A strict per-call choice: that ref only.
/// - No shared settings installed: per-call ref, saved override, default, fallback.
/// - Shared settings: the mode is the task's override, else its class route, else `auto`.
///   The per-call ref and the saved override come first, then default and fallback; a
///   task with a class (or a `cloud` / `local` mode) gets `claude:selected` /
///   `local:current` when it names no model of that kind. `auto` orders them by the
///   class ([`TaskClass::auto_order`]); `cloud` / `local` keep only that kind.
pub fn plan(task: &str, spec: &TaskSpec, saved: Option<String>, opts: &RouteOptions, settings: Option<&EngineSettings>) -> Plan {
    let rw = |r: &str| settings.map_or_else(|| r.to_string(), |s| rewrite(r, s, spec));
    // every ref the task could use, for the alternative
    let mut pool: Vec<String> = Vec::new();
    for r in [saved.clone(), Some(spec.default.clone()), spec.fallback.clone(), Some(format!("claude:{CLAUDE_SELECTED}")), Some(format!("local:{LOCAL_CURRENT}"))]
        .into_iter()
        .flatten()
    {
        push(&mut pool, rw(&r));
    }
    let of_kind = |v: &[String], k: EngineKind| v.iter().filter(|r| EngineKind::of(r) == k).cloned().collect::<Vec<_>>();
    if opts.strict
        && let Some(r) = &opts.override_ref
    {
        let r = rw(r);
        let k = EngineKind::of(&r);
        return Plan { candidates: vec![r], only: Some(k), by_route: false, others: of_kind(&pool, k.other()), mode: RouteMode::Auto };
    }
    let mut head = Vec::new();
    for r in [opts.override_ref.clone(), saved].into_iter().flatten() {
        push(&mut head, rw(&r));
    }
    let mut rest = Vec::new();
    for r in [Some(spec.default.clone()), spec.fallback.clone()].into_iter().flatten() {
        let r = rw(&r);
        if !head.contains(&r) {
            push(&mut rest, r);
        }
    }
    let Some(s) = settings else {
        head.extend(rest);
        return Plan { candidates: head, ..Default::default() };
    };
    let mode = s.mode_for(task, spec.class);
    if spec.class.is_some() || mode != RouteMode::Auto {
        for (k, fill) in [(EngineKind::Claude, format!("claude:{CLAUDE_SELECTED}")), (EngineKind::Local, format!("local:{LOCAL_CURRENT}"))] {
            if !head.iter().chain(&rest).any(|r| EngineKind::of(r) == k) {
                let r = rw(&fill);
                if !head.contains(&r) {
                    push(&mut rest, r);
                }
            }
        }
    }
    if let (Some(c), RouteMode::Auto) = (spec.class, mode) {
        let order = c.auto_order();
        rest.sort_by_key(|r| order.iter().position(|k| *k == EngineKind::of(r)));
    }
    head.extend(rest);
    match mode.only() {
        None => Plan { candidates: head, mode, ..Default::default() },
        Some(k) => Plan { candidates: of_kind(&head, k), only: Some(k), by_route: true, others: of_kind(&head, k.other()), mode },
    }
}

pub struct Router<F: ProviderFactory> {
    tasks: BTreeMap<String, TaskSpec>,
    overrides: RwLock<BTreeMap<String, String>>,
    online: AtomicBool,
    /// Fixed shared settings; `None` = whatever [`shared::install`] provides, read per route.
    settings: Option<EngineSettings>,
    pub factory: F,
}

impl<F: ProviderFactory> Router<F> {
    pub fn new(table: TaskTable, factory: F) -> Router<F> {
        Router { tasks: table.task, overrides: RwLock::new(table.overrides), online: AtomicBool::new(true), settings: None, factory }
    }

    /// Route with these shared settings instead of the installed ones (tests, previews).
    pub fn with_settings(mut self, s: EngineSettings) -> Self {
        self.settings = Some(s);
        self
    }

    /// The shared settings this router follows now (`None` = the tasks' own order).
    pub fn settings(&self) -> Option<EngineSettings> {
        self.settings.clone().or_else(shared::installed)
    }

    /// [`plan`] for one call of `task` (`None` for an unknown task).
    pub fn plan(&self, task: &str, opts: &RouteOptions) -> Option<Plan> {
        let t = self.tasks.get(task)?;
        Some(plan(task, t, self.override_for(task), opts, self.settings().as_ref()))
    }

    pub fn task(&self, id: &str) -> Option<&TaskSpec> {
        self.tasks.get(id)
    }

    pub fn tasks(&self) -> impl Iterator<Item = (&String, &TaskSpec)> {
        self.tasks.iter()
    }

    /// The app's connectivity check feeds this (default: online).
    pub fn set_online(&self, online: bool) {
        self.online.store(online, Ordering::Relaxed);
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// The user's pick for a task (`None` = back to the default). Persisting
    /// it in the user's tasks.toml is the app's job.
    pub fn set_override(&self, task: &str, model: Option<&str>) -> Result<(), String> {
        if !self.tasks.contains_key(task) {
            return Err(format!("沒有這個 LLM 任務：{task}"));
        }
        let mut o = self.overrides.write().unwrap_or_else(|e| e.into_inner());
        match model {
            Some(m) => {
                ModelRef::parse(m)?;
                o.insert(task.into(), m.into());
            }
            None => {
                o.remove(task);
            }
        }
        Ok(())
    }

    pub fn override_for(&self, task: &str) -> Option<String> {
        self.overrides.read().unwrap_or_else(|e| e.into_inner()).get(task).cloned()
    }

    /// Candidates in order: override, default, fallback (no repeats).
    pub fn candidates(&self, task: &str) -> Vec<String> {
        self.candidates_with(task, &RouteOptions::default())
    }

    /// Candidates for one call: the call's ref, then (unless strict) the
    /// saved override, default, fallback (no repeats).
    /// With shared settings installed, the class route and task overrides apply ([`plan`]).
    pub fn candidates_with(&self, task: &str, opts: &RouteOptions) -> Vec<String> {
        self.plan(task, opts).map(|p| p.candidates).unwrap_or_default()
    }

    /// The other engine of a restricted plan, when it could run now.
    async fn alternative(&self, plan: &Plan, spec: &TaskSpec, online: bool) -> Option<Alternative> {
        plan.only?;
        for raw in &plan.others {
            let Ok(r) = ModelRef::parse(raw) else { continue };
            if !online && r.needs_network() {
                continue;
            }
            if self.factory.check(&r, spec).await.is_ok() {
                return Some(Alternative::new(EngineKind::of(raw), raw.clone()));
            }
        }
        None
    }

    /// The engine for a task, or a clear reason why there is none.
    pub async fn provider_for(&self, task: &str) -> Result<Route, RouteError> {
        self.provider_for_with(task, &RouteOptions::default()).await
    }

    /// [`Router::provider_for`] with per-call choices: an engine picked for
    /// this call (`override_ref`) and whether it may fall back (`strict`).
    /// With `strict`, a failure is `RouteError::Unavailable` with one skip
    /// ([`RouteError::reason`] says why) or `NeedsNetwork` for a cloud
    /// choice while offline.
    pub async fn provider_for_with(&self, task: &str, opts: &RouteOptions) -> Result<Route, RouteError> {
        let spec = self.tasks.get(task).ok_or_else(|| RouteError::UnknownTask(task.into()))?;
        let mut eff = spec.clone();
        eff.priority = Some(opts.priority.or(spec.priority).unwrap_or_else(|| Priority::for_class(spec.class)));
        let spec = &eff;
        let label = spec.label.clone().unwrap_or_else(|| task.to_string());
        let online = self.is_online();
        let plan = plan(task, spec, self.override_for(task), opts, self.settings().as_ref());
        let refs: Vec<(String, Result<ModelRef, String>)> = plan.candidates.iter().map(|c| (c.clone(), ModelRef::parse(c))).collect();
        let any_local = refs.iter().any(|(_, r)| r.as_ref().is_ok_and(|r| !r.needs_network()));
        if !online && (spec.offline == Some(false) || !any_local) {
            // `cloud` while offline: say so, with the local model one click away.
            if plan.by_route && spec.offline != Some(false) {
                let skipped = plan.candidates.iter().map(|c| Skip { model: c.clone(), reason: "離線".into() }).collect();
                let alternative = self.alternative(&plan, spec, online).await;
                return Err(RouteError::Unavailable { task: task.into(), label, skipped, alternative: alternative.map(Box::new) });
            }
            return Err(RouteError::NeedsNetwork { task: task.into(), label });
        }
        let mut skipped = Vec::new();
        for (i, (raw, parsed)) in refs.into_iter().enumerate() {
            let skip = |reason: String| Skip { model: raw.clone(), reason };
            let r = match parsed {
                Ok(r) => r,
                Err(e) => {
                    skipped.push(skip(e));
                    continue;
                }
            };
            if !online && r.needs_network() {
                skipped.push(skip("離線".into()));
                continue;
            }
            let p = match self.factory.make(&r, spec).await {
                Ok(p) => p,
                Err(why) => {
                    skipped.push(skip(why));
                    continue;
                }
            };
            let caps = p.capabilities();
            if let Some(why) = caps.shortfall(&spec.needs) {
                skipped.push(skip(why));
                continue;
            }
            if !online && caps.needs_network {
                skipped.push(skip("離線".into()));
                continue;
            }
            shared::record_used(task, &p.id());
            let alternative = plan.only.and_then(|_| plan.others.first()).map(|o| Alternative::new(EngineKind::of(o), o.clone()));
            return Ok(Route {
                task: task.into(),
                model: r,
                provider: p,
                fallback_used: i > 0,
                skipped,
                effort: spec.effort.clone(),
                max_tokens: spec.max_tokens,
                alternative,
            });
        }
        let alternative = self.alternative(&plan, spec, online).await;
        Err(RouteError::Unavailable { task: task.into(), label, skipped, alternative: alternative.map(Box::new) })
    }
}

// ── the standard factory ────────────────────────────────

/// Claude through an API key.
#[derive(Debug, Clone)]
pub struct ApiAccess {
    pub base_url: String,
    pub api_key: String,
    /// Server-side refusal fallbacks (skipped automatically for Haiku).
    pub fallbacks: bool,
}

/// Claude through the user's subscription (Claude Code).
#[derive(Clone)]
pub struct SubscriptionAccess {
    pub cli: Cli,
    pub cache: Arc<Cache>,
    /// Allow WebSearch for tasks that need the web.
    pub allow_web: bool,
    /// Temp-folder prefix (the app's name).
    pub temp_prefix: String,
}

/// A llama-server someone else runs (`[llama] url`).
#[derive(Debug, Clone)]
pub struct ExternalServer {
    /// Base URL ending in `/v1`.
    pub url: String,
    /// The catalog id it serves; `None` = whatever is asked.
    pub model: Option<String>,
}

/// Local models: an external server, or the machine's shared llama-server.
#[derive(Clone, Default)]
pub struct LocalAccess {
    /// Shared folder; `None` = `LOCAL_LLM_HOME`.
    pub home: Option<PathBuf>,
    /// The app's own folders instead of the shared layout of `home`
    /// (wins over `home` when set).
    pub layout: Option<Layout>,
    /// An app's own server (its config or environment, never `private/`); wins when set.
    pub external: Option<ExternalServer>,
    /// The machine's shared llama-server (this process's side of the protocol).
    pub shared: Option<Arc<SharedRuntime>>,
    /// `local:current`: `engine.toml` `[engine] model`.
    pub default_model: Option<String>,
    /// `engine.toml` `[engine] vision_model` (for a task that reads images).
    pub vision_model: Option<String>,
    /// Rewrite Chinese replies in Taiwan usage.
    pub zh_tw: bool,
    /// Allow abliterated models (PaperComposer yes, household apps no).
    pub include_uncensored: bool,
}

impl LocalAccess {
    /// The external server as `local:current` (any model name; the catalog
    /// supplies vision/context when it knows the name, `local` when there is
    /// no name). `Err` without one (the shared server is [`StandardFactory::make`]'s).
    pub async fn current(&self, catalog: &Catalog) -> Result<OpenAiCompat, String> {
        let mut cfg = if let Some(ext) = &self.external {
            let m = ext.model.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "local".into());
            let mut c = OpenAiCompatConfig::llama_server(ext.url.clone(), m.clone());
            if let Some(entry) = catalog.get(&m) {
                c.vision = entry.vision;
                c.context = entry.context_or_default();
            }
            c
        } else {
            return Err("沒有連接本機模型".into());
        };
        cfg.zh_tw = self.zh_tw;
        Ok(OpenAiCompat::new(cfg))
    }
}

/// Builds Claude and local engines from the app's settings and the catalog.
pub struct StandardFactory {
    pub catalog: Catalog,
    /// `claude:` refs try the subscription before the API key (JELearner
    /// `[engine] kind = auto`).
    pub prefer_subscription: bool,
    pub api: Option<ApiAccess>,
    pub subscription: Option<SubscriptionAccess>,
    pub local: Option<LocalAccess>,
    /// The Claude model picked in the app's settings, for `claude:selected`
    /// (an alias or id; `None` = the catalog's `cloud.default`). An id that
    /// is not in the catalog is used as is, with a 200k context.
    pub selected_claude: Option<String>,
}

impl StandardFactory {
    pub fn new(catalog: Catalog) -> StandardFactory {
        StandardFactory { catalog, prefer_subscription: true, api: None, subscription: None, local: None, selected_claude: None }
    }

    /// The Claude model `claude:<model>` means: `selected` → the settings'
    /// pick, otherwise a catalog alias or id.
    pub fn claude_model(&self, model: &str) -> Result<CloudModel, String> {
        if model == CLAUDE_SELECTED {
            let pick = self.selected_claude.as_deref().map(str::trim).filter(|m| !m.is_empty()).unwrap_or(&self.catalog.cloud.default);
            return Ok(self.catalog.cloud_model(pick).cloned().unwrap_or_else(|| CloudModel {
                id: pick.to_string(),
                summary: String::new(),
                context: UNKNOWN_CLOUD_CONTEXT,
            }));
        }
        self.catalog.cloud_model(model).cloned().ok_or_else(|| format!("模型目錄裡沒有 Claude 模型「{model}」"))
    }

    /// Where local models live: `LocalAccess.layout`, else the shared layout
    /// of its `home` (or `LOCAL_LLM_HOME`).
    pub fn local_layout(&self) -> Option<Layout> {
        let la = self.local.as_ref()?;
        la.layout.clone().or_else(|| local::home(la.home.as_deref()).map(|h| Layout::shared(&h)))
    }

    /// The external server as `local:current` ([`LocalAccess::current`]). `Err` when there is none.
    pub async fn current_local(&self) -> Result<OpenAiCompat, String> {
        self.local.as_ref().ok_or("沒有設定本機模型")?.current(&self.catalog).await
    }

    /// The catalog model `local:current` means for `task` on the shared server.
    fn current_model(&self, task: &TaskSpec) -> Result<&crate::catalog::Model, String> {
        let la = self.local.as_ref().ok_or("沒有設定本機模型")?;
        let layout = self.local_layout();
        crate::shared_runtime::current_model(
            &self.catalog,
            layout.as_ref(),
            la.default_model.as_deref(),
            la.vision_model.as_deref(),
            task.needs.vision,
            la.include_uncensored,
        )
    }

    async fn claude(&self, model: &str, via: ClaudeVia, task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String> {
        let cm = self.claude_model(model)?;
        let effort = task.effort.clone().or_else(|| self.catalog.cloud.effort.clone());
        let order: &[ClaudeVia] = match via {
            ClaudeVia::Api => &[ClaudeVia::Api],
            ClaudeVia::Subscription => &[ClaudeVia::Subscription],
            ClaudeVia::Auto if self.prefer_subscription => &[ClaudeVia::Subscription, ClaudeVia::Api],
            ClaudeVia::Auto => &[ClaudeVia::Api, ClaudeVia::Subscription],
        };
        let mut why = Vec::new();
        for v in order {
            match v {
                ClaudeVia::Subscription => {
                    let Some(s) = &self.subscription else {
                        why.push("沒有設定 Claude 訂閱".to_string());
                        continue;
                    };
                    let mut p = ClaudeCode::new(s.cli.clone(), cm.id.clone(), effort.clone());
                    p.allow_web = s.allow_web;
                    p.context = cm.context;
                    p.temp_prefix = s.temp_prefix.clone();
                    if let Some(short) = p.capabilities().shortfall(&task.needs) {
                        why.push(format!("Claude 訂閱{short}"));
                        continue;
                    }
                    let st = s.cache.get(&s.cli, false).await;
                    if !st.installed {
                        why.push("找不到 Claude Code".into());
                    } else if !st.ready {
                        why.push("Claude 訂閱尚未登入".into());
                    } else {
                        return Ok(Arc::new(p));
                    }
                }
                ClaudeVia::Api => {
                    let Some(a) = &self.api else {
                        why.push("沒有 Claude API key".to_string());
                        continue;
                    };
                    let mut cfg = ClaudeApiConfig::new(cm.id.clone());
                    cfg.base_url = a.base_url.clone();
                    cfg.effort = effort.clone();
                    cfg.fallbacks = a.fallbacks;
                    cfg.context = cm.context;
                    return Ok(Arc::new(ClaudeApi::new(cfg, a.api_key.clone())));
                }
                ClaudeVia::Auto => unreachable!(),
            }
        }
        Err(why.join("、"))
    }

    async fn local(&self, r: &str, task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String> {
        let la = self.local.as_ref().ok_or("沒有設定本機模型")?;
        if r == LOCAL_CURRENT && (la.external.is_some() || la.shared.is_none()) {
            return Ok(Arc::new(self.current_local().await?));
        }
        let layout = self.local_layout();
        let on_disk = |m: &crate::catalog::Model| layout.as_ref().is_some_and(|l| l.find_model(m).is_some());
        let m = if r == LOCAL_CURRENT {
            self.current_model(task)?
        } else {
            self.catalog.resolve_local(r, on_disk).ok_or_else(|| format!("模型目錄裡沒有本機模型「{r}」"))?
        };
        if m.uncensored && !la.include_uncensored {
            return Err("這個 App 不使用移除拒答的模型".into());
        }
        if task.needs.vision && !m.vision {
            return Err(format!("{} 不能看圖", m.name));
        }
        if let Some(c) = task.needs.context
            && m.context_or_default() < c
        {
            return Err(format!("{} 的上下文只有 {}", m.name, m.context_or_default()));
        }
        if let Some(ext) = &la.external {
            if let Some(serving) = &ext.model
                && serving != &m.id
            {
                return Err(format!("外部 llama-server 跑的是 {serving}，不是 {}", m.id));
            }
            let mut c = OpenAiCompatConfig::llama_server(ext.url.clone(), m.id.clone());
            c.vision = m.vision;
            c.context = m.context_or_default();
            c.zh_tw = la.zh_tw;
            return Ok(Arc::new(OpenAiCompat::new(c)));
        }
        // the shared server: files checked now when the folder is known, the model acquired
        // per request (without the folder this app can use a server another app started)
        let rt = la.shared.clone().ok_or("沒有設定本機 llama-server")?;
        if let Some(why) = layout.as_ref().and_then(|l| l.missing(m)) {
            return Err(why);
        }
        let vision = m.vision && layout.as_ref().is_none_or(|l| l.find_mmproj(m).is_some());
        Ok(Arc::new(SharedLocal {
            rt,
            model: m.clone(),
            vision,
            layout,
            priority: task.priority.unwrap_or_else(|| Priority::for_class(task.class)),
            zh_tw: la.zh_tw,
        }))
    }
}

#[async_trait]
impl ProviderFactory for StandardFactory {
    async fn make(&self, model: &ModelRef, task: &TaskSpec) -> Result<Arc<dyn LlmProvider>, String> {
        match model {
            ModelRef::Claude { model, via } => self.claude(model, *via, task).await,
            ModelRef::Local(r) => self.local(r, task).await,
            ModelRef::Mock(_) => Err("mock 只在測試裡使用".into()),
        }
    }

    /// Claude: the same checks as [`make`](ProviderFactory::make) (nothing is started).
    /// Local: the external server, or the model (for `local:current`, the shared default)
    /// is on disk with llama.cpp; nothing is started.
    async fn check(&self, model: &ModelRef, task: &TaskSpec) -> Result<(), String> {
        match model {
            ModelRef::Claude { model, via } => self.claude(model, *via, task).await.map(|_| ()),
            ModelRef::Local(r) => {
                let la = self.local.as_ref().ok_or("沒有設定本機模型")?;
                if r == LOCAL_CURRENT && (la.external.is_some() || la.shared.is_none()) {
                    return self.current_local().await.map(|_| ());
                }
                let layout = self.local_layout();
                let on_disk = |m: &crate::catalog::Model| layout.as_ref().is_some_and(|l| l.find_model(m).is_some());
                let m = if r == LOCAL_CURRENT {
                    self.current_model(task)?
                } else {
                    self.catalog.resolve_local(r, on_disk).ok_or_else(|| format!("模型目錄裡沒有本機模型「{r}」"))?
                };
                match (&la.external, &layout) {
                    (Some(ext), _) => match &ext.model {
                        Some(serving) if serving != &m.id => Err(format!("外部 llama-server 跑的是 {serving}，不是 {}", m.id)),
                        _ => Ok(()),
                    },
                    (None, l) if la.shared.is_some() => l.as_ref().and_then(|l| l.missing(m)).map_or(Ok(()), Err),
                    _ => Err("沒有設定本機 llama-server".into()),
                }
            }
            ModelRef::Mock(_) => Err("mock 只在測試裡使用".into()),
        }
    }
}

/// A quick connectivity probe for [`Router::set_online`]: can a TCP
/// connection be opened to `host:port` within `timeout`?
pub async fn probe_online(host_port: &str, timeout: std::time::Duration) -> bool {
    tokio::time::timeout(timeout, tokio::net::TcpStream::connect(host_port)).await.is_ok_and(|r| r.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProvider;
    use crate::types::Capabilities;

    fn caps(vision: bool, web: bool, net: bool, context: u32) -> Capabilities {
        Capabilities { json: true, vision, web, streaming: true, context, needs_network: net }
    }

    fn mock(name: &str, c: Capabilities) -> Arc<dyn LlmProvider> {
        Arc::new(MockProvider::new(vec![format!("from {name}")]).named(name).with_capabilities(c))
    }

    const TASKS: &str = r#"
[task.card]
label = "整理成知識卡"
needs = { json = true }
default = "claude:sonnet"
fallback = "local:qwen"
offline = true

[task.crop]
needs = { vision = true, json = true }
default = "claude:sonnet"
fallback = "local:gemma"

[task.web]
label = "搜尋文獻"
needs = { web = true }
default = "claude:sonnet"
offline = false

[task.long]
needs = { context = 32000 }
default = "local:small"
effort = "low"
max_tokens = 9000
"#;

    fn router(f: MockFactory) -> Router<MockFactory> {
        Router::new(TaskTable::parse(TASKS).unwrap(), f)
    }

    #[tokio::test]
    async fn default_first_then_fallback_with_reasons() {
        let f = MockFactory::default()
            .with("claude:sonnet", mock("claude", caps(true, true, true, 1_000_000)))
            .with("local:qwen", mock("qwen", caps(false, false, false, 32768)));
        let r = router(f);
        let route = r.provider_for("card").await.unwrap();
        assert_eq!(route.provider.id(), "claude");
        assert!(!route.fallback_used);

        r.set_online(false);
        let route = r.provider_for("card").await.unwrap();
        assert_eq!(route.provider.id(), "qwen");
        assert!(route.fallback_used);
        assert_eq!(route.skipped, vec![Skip { model: "claude:sonnet".into(), reason: "離線".into() }]);
    }

    #[tokio::test]
    async fn offline_tasks_explain_themselves() {
        let f = MockFactory::default().with("claude:sonnet", mock("claude", caps(true, true, true, 1_000_000)));
        let r = router(f);
        r.set_online(false);
        let e = r.provider_for("web").await.unwrap_err();
        assert_eq!(e, RouteError::NeedsNetwork { task: "web".into(), label: "搜尋文獻".into() });
        assert!(e.to_string().contains("需要網路"));
        assert!(matches!(r.provider_for("nope").await, Err(RouteError::UnknownTask(_))));
    }

    #[tokio::test]
    async fn needs_are_checked_against_capabilities() {
        // Only a text-only Claude (like the subscription) and a vision-less local model.
        let f = MockFactory::default()
            .with("claude:sonnet", mock("sub", caps(false, false, true, 1_000_000)))
            .with("local:gemma", mock("gemma", caps(false, false, false, 32768)));
        let r = router(f);
        let e = r.provider_for("crop").await.unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("claude:sonnet（不支援看圖）") && msg.contains("local:gemma（不支援看圖）"), "{msg}");

        let f = MockFactory::default().with("local:small", mock("small", caps(false, false, false, 16384)));
        let e = router(f).provider_for("long").await.unwrap_err();
        assert!(e.to_string().contains("32000"), "{e}");
    }

    #[tokio::test]
    async fn overrides_come_first_and_can_be_cleared() {
        let f = MockFactory::default()
            .with("claude:sonnet", mock("claude", caps(true, true, true, 1_000_000)))
            .with("claude:haiku", mock("haiku", caps(true, true, true, 1_000_000)))
            .with("local:qwen", mock("qwen", caps(false, false, false, 32768)));
        let r = router(f);
        r.set_override("card", Some("claude:haiku")).unwrap();
        assert_eq!(r.candidates("card"), vec!["claude:haiku", "claude:sonnet", "local:qwen"]);
        assert_eq!(r.provider_for("card").await.unwrap().provider.id(), "haiku");
        assert!(r.set_override("card", Some("bogus")).is_err());
        assert!(r.set_override("missing", Some("claude:haiku")).is_err());
        r.set_override("card", None).unwrap();
        assert_eq!(r.provider_for("card").await.unwrap().provider.id(), "claude");
    }

    #[tokio::test]
    async fn route_applies_task_effort_and_max_tokens() {
        let f = MockFactory::default().with("local:small", mock("small", caps(false, false, false, 65536)));
        let route = router(f).provider_for("long").await.unwrap();
        let req = route.apply(LlmRequest::prompt("s", "u"));
        assert_eq!(req.effort.as_deref(), Some("low"));
        assert_eq!(req.max_tokens, 9000);
        assert_eq!(route.provider.complete(&req).await.unwrap(), "from small");
        let own = route.apply(LlmRequest::prompt("s", "u").effort("high").max_tokens(100));
        assert_eq!((own.effort.as_deref(), own.max_tokens), (Some("high"), 100));
    }

    #[test]
    fn model_refs_round_trip() {
        for s in ["claude:sonnet", "claude-api:claude-opus-5-5", "claude-code:haiku", "local:gemma-4-12b", "mock:x"] {
            assert_eq!(ModelRef::parse(s).unwrap().to_string(), s);
        }
        assert_eq!(ModelRef::parse("claude-subscription:haiku").unwrap().to_string(), "claude-code:haiku");
        assert!(ModelRef::parse("sonnet").is_err() && ModelRef::parse("gpt:4").is_err() && ModelRef::parse("local:").is_err());
    }

    #[test]
    fn bundled_tasks_cover_the_design_and_parse() {
        let t = TaskTable::bundled();
        for id in ["kb-summarize-card", "sc-slide-content", "sc-crop-modules", "research-web", "pc-compose-paragraph", "kb-extract-terms", "reader-chat"] {
            assert!(t.task.contains_key(id), "{id}");
        }
        assert_eq!(t.task["research-web"].offline, Some(false));
        assert!(t.task["sc-crop-modules"].needs.vision);
        // Every local ref in the sample names a catalog brain.
        let cat = Catalog::bundled();
        for (id, spec) in &t.task {
            for r in [Some(&spec.default), spec.fallback.as_ref()].into_iter().flatten() {
                match ModelRef::parse(r).unwrap() {
                    ModelRef::Local(m) => assert!(cat.resolve_local(&m, |_| false).is_some(), "{id}: {r}"),
                    ModelRef::Claude { model, .. } => assert!(cat.cloud_model(&model).is_some(), "{id}: {r}"),
                    ModelRef::Mock(_) => panic!("no mocks in the sample"),
                }
            }
        }
    }

    #[test]
    fn user_file_overrides_and_adds() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tasks.toml");
        std::fs::write(&p, "[override]\nkb-summarize-card = \"claude:haiku\"\n\n[task.my-task]\ndefault = \"local:qwen3.5-4b-q8\"\n").unwrap();
        let t = TaskTable::load(TaskTable::bundled(), Some(&p)).unwrap();
        assert_eq!(t.overrides["kb-summarize-card"], "claude:haiku");
        assert!(t.task.contains_key("my-task") && t.task.contains_key("research-web"));
        std::fs::write(&p, "[override]\nx = \"nonsense\"\n").unwrap();
        assert!(TaskTable::load(TaskTable::bundled(), Some(&p)).is_err(), "broken user file is reported");
        assert!(TaskTable::load(TaskTable::bundled(), Some(&dir.path().join("none.toml"))).is_ok());
    }

    // ── StandardFactory (no network: fake CLI, mock HTTP, temp folders) ──

    fn spec(needs: Needs) -> TaskSpec {
        TaskSpec { label: None, needs, default: "claude:sonnet".into(), fallback: None, offline: None, effort: None, max_tokens: None, class: None, priority: None }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn standard_factory_prefers_a_signed_in_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let bin = crate::claude_code::fake::write(dir.path());
        let cli = Cli { bin: Some(bin), config_dir: Some(dir.path().join("ellrin")) };
        let mut f = StandardFactory::new(Catalog::bundled());
        f.subscription = Some(SubscriptionAccess { cli: cli.clone(), cache: Arc::new(Cache::default()), allow_web: false, temp_prefix: "test".into() });
        f.api = Some(ApiAccess { base_url: "http://127.0.0.1:9".into(), api_key: "test-key".into(), fallbacks: true });
        let sonnet = ModelRef::parse("claude:sonnet").unwrap();

        // Not signed in yet: the API key is used.
        let p = f.make(&sonnet, &spec(Needs::default())).await.unwrap();
        assert_eq!(p.id(), "claude:claude-sonnet-5-5");

        crate::claude_code::fake::sign_in(&dir.path().join("ellrin"), "ellrin@example.com");
        f.subscription.as_mut().unwrap().cache = Arc::new(Cache::default());
        let p = f.make(&sonnet, &spec(Needs::default())).await.unwrap();
        assert_eq!(p.id(), "claude-subscription:claude-sonnet-5-5");
        // Vision is not offered by the subscription: the API takes it.
        let p = f.make(&sonnet, &spec(Needs { vision: true, ..Default::default() })).await.unwrap();
        assert_eq!(p.id(), "claude:claude-sonnet-5-5");
        // Subscription only and vision: a clear reason.
        let only = ModelRef::parse("claude-code:sonnet").unwrap();
        let e = f.make(&only, &spec(Needs { vision: true, ..Default::default() })).await.err().unwrap();
        assert!(e.contains("看圖"), "{e}");
        f.api = None;
        f.subscription = None;
        let e = f.make(&sonnet, &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains("訂閱") && e.contains("API key"), "{e}");
        assert!(f.make(&ModelRef::parse("claude:gpt").unwrap(), &spec(Needs::default())).await.err().unwrap().contains("gpt"));
    }

    #[tokio::test]
    async fn standard_factory_local_reasons_and_external_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = StandardFactory::new(Catalog::bundled());
        let q = ModelRef::parse("local:qwen3.5-9b-q8").unwrap();
        assert!(f.make(&q, &spec(Needs::default())).await.err().unwrap().contains("本機"));

        f.local = Some(LocalAccess {
            home: Some(dir.path().to_path_buf()),
            external: None,
            shared: Some(SharedRuntime::new("t", dir.path().join("reg"), Default::default())),
            zh_tw: true,
            include_uncensored: false,
            ..Default::default()
        });
        let e = f.make(&q, &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains("llama.cpp"), "nothing downloaded: {e}");
        let e = f.make(&ModelRef::parse("local:gemma-4-12b-abl-q4").unwrap(), &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains("移除拒答"), "{e}");
        let e = f.make(&ModelRef::parse("local:qwen3.8-27b-q4").unwrap(), &spec(Needs { context: Some(32000), ..Default::default() })).await.err().unwrap();
        assert!(e.contains("16384"), "{e}");

        // An external llama-server: no files needed, the catalog supplies capabilities.
        f.local.as_mut().unwrap().external = Some(ExternalServer { url: "http://127.0.0.1:9/v1".into(), model: Some("qwen3.5-9b-q8".into()) });
        let p = f.make(&q, &spec(Needs { vision: true, ..Default::default() })).await.unwrap();
        assert_eq!(p.id(), "llama-server:qwen3.5-9b-q8");
        assert!(p.capabilities().vision && !p.capabilities().needs_network);
        let e = f.make(&ModelRef::parse("local:gemma-4-12b-q8").unwrap(), &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains("外部"), "{e}");
    }

    #[tokio::test]
    async fn bundled_tasks_route_with_a_standard_factory() {
        let _g = crate::shared::TEST_GLOBAL.lock().await;
        let mut f = StandardFactory::new(Catalog::bundled());
        f.prefer_subscription = false;
        f.api = Some(ApiAccess { base_url: "http://127.0.0.1:9".into(), api_key: "k".into(), fallbacks: true });
        f.local = Some(LocalAccess {
            home: None,
            external: Some(ExternalServer { url: "http://127.0.0.1:9/v1".into(), model: None }),
            zh_tw: false,
            include_uncensored: false,
            ..Default::default()
        });
        let r = Router::new(TaskTable::bundled(), f);
        assert_eq!(r.provider_for("sc-crop-modules").await.unwrap().provider.id(), "claude:claude-sonnet-5-5");
        assert_eq!(r.provider_for("kb-summarize-card").await.unwrap().provider.id(), "llama-server:qwen3.5-9b-q8");
        r.set_online(false);
        let route = r.provider_for("sc-slide-content").await.unwrap();
        assert_eq!(route.provider.id(), "llama-server:gemma-4-12b-q8");
        assert!(route.fallback_used);
        assert!(matches!(r.provider_for("research-web").await, Err(RouteError::NeedsNetwork { .. })));
        let jl = r.provider_for("jl-realtime").await.unwrap();
        assert_eq!(jl.effort.as_deref(), Some("low"));
    }

    // ── per-call choices, app-level refs ──

    #[tokio::test]
    async fn per_call_choices_are_strict_or_preferred_without_touching_overrides() {
        let f = MockFactory::default()
            .with("claude:sonnet", mock("claude", caps(true, true, true, 1_000_000)))
            .with("claude:haiku", mock("haiku", caps(true, true, true, 1_000_000)))
            .with("local:qwen", mock("qwen", caps(false, false, false, 32768)));
        let r = router(f);
        r.set_override("card", Some("claude:haiku")).unwrap();

        // Strict: that engine, or why not — never the task's own order.
        let route = r.provider_for_with("card", &RouteOptions::only("local:qwen")).await.unwrap();
        assert_eq!((route.provider.id().as_str(), route.fallback_used), ("qwen", false));
        let e = r.provider_for_with("card", &RouteOptions::only("local:gemma")).await.unwrap_err();
        assert!(matches!(&e, RouteError::Unavailable { skipped, .. } if skipped.len() == 1), "{e:?}");
        assert_eq!(e.reason(), "沒有設定");
        let e = r.provider_for_with("crop", &RouteOptions::only("local:qwen")).await.unwrap_err();
        assert!(e.reason().contains("看圖"), "{e}");
        assert_eq!(r.candidates_with("card", &RouteOptions::only("local:qwen")), vec!["local:qwen"]);

        // Preferred: the call's pick first, then override, default, fallback.
        let opts = RouteOptions::prefer("local:gemma");
        assert_eq!(r.candidates_with("card", &opts), vec!["local:gemma", "claude:haiku", "claude:sonnet", "local:qwen"]);
        let route = r.provider_for_with("card", &opts).await.unwrap();
        assert_eq!(route.provider.id(), "haiku");
        assert!(route.fallback_used);

        // The saved override is untouched; strict cloud choices offline say so.
        assert_eq!(r.override_for("card").as_deref(), Some("claude:haiku"));
        r.set_online(false);
        assert!(matches!(r.provider_for_with("card", &RouteOptions::only("claude:sonnet")).await, Err(RouteError::NeedsNetwork { .. })));
        assert_eq!(r.provider_for_with("card", &RouteOptions::default()).await.unwrap().provider.id(), "qwen");
    }

    #[tokio::test]
    async fn shared_factories_work_through_arc_and_references() {
        let f = Arc::new(MockFactory::default().with("claude:sonnet", mock("claude", caps(true, true, true, 1_000_000))));
        let r = Router::new(TaskTable::parse(TASKS).unwrap(), f.clone());
        assert_eq!(r.provider_for("web").await.unwrap().provider.id(), "claude");
        let by_ref = Router::new(TaskTable::parse(TASKS).unwrap(), &*f);
        assert_eq!(by_ref.provider_for("web").await.unwrap().provider.id(), "claude");
        let dynamic: Arc<dyn ProviderFactory> = f;
        assert!(Router::new(TaskTable::parse(TASKS).unwrap(), dynamic).provider_for("web").await.is_ok());
    }

    #[tokio::test]
    async fn claude_selected_follows_the_settings_pick() {
        let mut f = StandardFactory::new(Catalog::bundled());
        f.prefer_subscription = false;
        f.api = Some(ApiAccess { base_url: "http://127.0.0.1:9".into(), api_key: "test-key".into(), fallbacks: true });
        let sel = ModelRef::parse("claude-api:selected").unwrap();
        let p = f.make(&sel, &spec(Needs::default())).await.unwrap();
        assert_eq!(p.id(), format!("claude:{}", Catalog::bundled().cloud.default), "unset: the catalog default");
        f.selected_claude = Some("haiku".into());
        assert_eq!(f.make(&sel, &spec(Needs::default())).await.unwrap().id(), "claude:claude-haiku-5-5", "aliases resolve");
        // A model the catalog does not list yet (picked in settings) is used as is.
        f.selected_claude = Some("claude-next-test".into());
        let p = f.make(&sel, &spec(Needs::default())).await.unwrap();
        assert_eq!(p.id(), "claude:claude-next-test");
        assert_eq!(p.capabilities().context, 200_000);
        // Other names still have to be in the catalog.
        assert!(f.make(&ModelRef::parse("claude:claude-next-test").unwrap(), &spec(Needs::default())).await.is_err());
    }

    #[tokio::test]
    async fn local_current_is_the_external_server_or_the_shared_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = StandardFactory::new(Catalog::bundled());
        let cur = ModelRef::parse("local:current").unwrap();
        f.local = Some(LocalAccess { zh_tw: true, ..Default::default() });
        assert!(f.make(&cur, &spec(Needs::default())).await.err().unwrap().contains("沒有連接本機模型"));

        // An external server with a name the catalog does not know (Ollama, LM Studio).
        f.local.as_mut().unwrap().external = Some(ExternalServer { url: "http://127.0.0.1:9/v1".into(), model: Some("my-model:7b".into()) });
        let p = f.make(&cur, &spec(Needs::default())).await.unwrap();
        assert_eq!(p.id(), "llama-server:my-model:7b");
        assert!(!p.capabilities().vision);
        // No name: "local". A catalog name: the catalog's capabilities.
        f.local.as_mut().unwrap().external = Some(ExternalServer { url: "http://127.0.0.1:9/v1".into(), model: None });
        assert_eq!(f.current_local().await.unwrap().id(), "llama-server:local");
        f.local.as_mut().unwrap().external = Some(ExternalServer { url: "http://127.0.0.1:9/v1".into(), model: Some("qwen3.5-9b-q8".into()) });
        assert!(f.current_local().await.unwrap().capabilities().vision);
        assert!(f.current_local().await.unwrap().config().zh_tw);

        // No external server: the shared default model, acquired per request (nothing starts here).
        let home = dir.path();
        let la = f.local.as_mut().unwrap();
        la.external = None;
        la.home = Some(home.to_path_buf());
        la.shared = Some(SharedRuntime::new("t", home.join("reg"), Default::default()));
        assert!(f.make(&cur, &spec(Needs::default())).await.err().unwrap().contains("還沒選"), "no model chosen");
        f.local.as_mut().unwrap().default_model = Some("gemma-4-12b-q8".into());
        let e = f.make(&cur, &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains("llama.cpp"), "nothing on disk: {e}");
        let m = Catalog::bundled().get("gemma-4-12b-q8").unwrap().clone();
        for p in [home.join("runtimes/llama.cpp/llama-server"), home.join("gguf").join(&m.repo).join(&m.file_suffix)] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        assert!(f.check(&cur, &spec(Needs::default())).await.is_ok());
        let p = f.make(&cur, &spec(Needs::default())).await.unwrap();
        assert_eq!(p.id(), "llama-server:gemma-4-12b-q8");
        assert_eq!(p.capabilities().context, m.context_or_default());
        assert!(!p.capabilities().vision, "no projector on disk");
        assert!(!home.join("reg").join(crate::shared_runtime::REGISTRY).exists(), "routing starts and registers nothing");
        let e = f.make(&cur, &spec(Needs { vision: true, ..Default::default() })).await.err().unwrap();
        assert!(e.contains("不能看圖"), "{e}");
    }

    #[tokio::test]
    async fn an_app_layout_replaces_the_shared_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-9b-q8").unwrap().clone();
        let mut f = StandardFactory::new(cat);
        f.local = Some(LocalAccess {
            home: Some(data.join("unused")),
            layout: Some(Layout::new(data.join("models"), data.join("llama.cpp"))),
            shared: Some(SharedRuntime::new("t", data.join("reg"), Default::default())),
            ..Default::default()
        });
        assert_eq!(f.local_layout().unwrap().gguf, data.join("models"));
        let e = f.make(&ModelRef::parse("local:qwen3.5-9b-q8").unwrap(), &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains(&data.join("llama.cpp").display().to_string()), "{e}");
        std::fs::create_dir_all(data.join("llama.cpp")).unwrap();
        std::fs::write(data.join("llama.cpp/llama-server"), "").unwrap();
        let e = f.make(&ModelRef::parse("local:qwen3.5-9b-q8").unwrap(), &spec(Needs::default())).await.err().unwrap();
        assert!(e.contains("hf download") && e.contains(&data.join("models").join(&m.repo).display().to_string()), "{e}");
    }
}
