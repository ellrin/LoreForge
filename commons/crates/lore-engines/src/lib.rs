//! lore-engines — the settings behind the shell's 模型 tab, the same in every app.
//!
//! ```ignore
//! // in the app's router (any state type):
//! .merge(lore_engines::router(lore_engines::Engines::new(home.clone(), "PaperComposer", TASKS)))
//! ```
//!
//! - `GET /api/engines` → the local model, the Claude connection, the three task classes
//!   with their routes, this app's tasks (class, last model used) and per-task overrides
//!   (docs/shell.md F).
//! - `PUT /api/engines` with the parts to change: `{ local: { model }, claude: { via, model },
//!   routes: { create: "cloud" }, overrides: { "<task>": "local" } }` → the full GET body;
//!   a bad value → 400 `{ "error": "一行中文" }` and nothing is written.
//! - The local model is the machine's one shared llama-server (lore-llm `shared_runtime`,
//!   docs/local-models.md): `local.status` = the words every app shows (「已載入 …」,
//!   「等待 X 用完模型」…), `local.loaded`, `local.users` / `local.waiting` (who uses it, who
//!   waits), from the registry in the cache folder. PUT `{ local: { model } }` only writes the
//!   shared default model id (`engine.toml` `[engine] model`); the model loads when a task needs it.
//! - Memory of the local model: `local.idle_minutes` (GET; PUT `{ local: { idle_minutes: 15 } }`,
//!   0 = never; kept in `engine.toml` `[local]`, shared by every app) and `local.can_release`
//!   (no other app holds a lease); PUT `{ local: { release: true } }` stops the server now, or
//!   400 「X 正在用」.
//! - `google` in the GET body = Google models through the Antigravity CLI (`agy`, for 語句潤飾 only):
//!   `{ available, models: [{id, label}], checked, new }` from the last `agy models` listing kept
//!   in the cache folder (`lore_llm::agy`); `checked` = unix seconds or null, `new` = ids that were
//!   not in the listing before. `POST /api/engines/refresh` lists again now and returns the GET body
//!   (plus `google.error` when the listing failed); a listing older than 24 hours refreshes itself
//!   in the background, GET never waits for it.
//! - `claude.models` = the catalog's models first (order, labels, aliases unchanged), then every model
//!   the API lists that the catalog lacks (`GET /v1/models`, `lore_llm::claude_models`, only with an
//!   API key; the subscription cannot list), each `{ id (full), label, vision, new }`; `new` = not in
//!   the previous listing. Also `claude.listed` (a listing is kept), `claude.checked` (unix seconds or
//!   null), `claude.can_list` (an API key exists). The refresh button and the daily refresh cover both
//!   lists; `claude.error` beside `google.error`; one failing never hides the other. A listed id is
//!   a valid `PUT` model and routes as `claude:<id>`.
//! - `GET /api/links/ports` → `{ "sc": 51234, … }`: the desktop apps running now (lore-home `ports`).
//! - `POST /api/links/open { url, background? }` (`background`: `open -g -a <App>` without the link, the app stays behind) → hands a `lore-(pc|sc|lr|kb)://` or
//!   `http://127.0.0.1…` URL to the OS opener (204); anything else → 400.
//!
//! Everything is kept in `private/shared/engine.toml` through lore-home (never a key:
//! keys are in the keychain, [`lore_llm::secrets::SharedKeys`]). Mounting also installs
//! the file as the process's routing ([`lore_llm::shared::install`]): every lore-llm
//! router of the app then follows the class routes and overrides, read on each call.

use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use lore_home::Home;
use lore_llm::catalog::Catalog;
use lore_llm::agy::{self, CliLister, ModelLister, Stored};
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::claude_models::{self, ClaudeLister, HttpLister};
use lore_llm::local::Layout;
use lore_llm::secrets::{Account, SharedKeys};
use lore_llm::shared::{self, EngineSettings, TaskClass};
use lore_llm::shared_runtime::{LocalStatus, SharedRuntime, app_label};
use lore_llm::{OpenAiCompat, OpenAiCompatConfig, TaskTable};
use serde_json::{Value, json};
use std::sync::Arc;

/// Opens a checked URL with the OS (replaced in tests).
pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// What the routes need from the app.
#[derive(Clone)]
pub struct Engines {
    pub home: Home,
    /// The app's tasks (`tasks.toml`): the ones `tasks` / `overrides` list and accept.
    pub tasks: TaskTable,
    pub catalog: Catalog,
    /// This process's side of the shared llama-server (the local state, 釋放, idle checks);
    /// `None` in tests and isolated servers.
    pub shared: Option<Arc<SharedRuntime>>,
    /// Where downloaded models are (`LOCAL_LLM_HOME` by default), for the model list.
    pub layout: Option<Layout>,
    /// The app's own `[engine] url` (wins over the shared one, as in the app's routing).
    pub external: Option<(String, Option<String>)>,
    /// List abliterated models (L-Paper yes, household apps no).
    pub include_uncensored: bool,
    pub cli: Cli,
    pub cc: Arc<Cache>,
    pub keys: SharedKeys,
    /// The Claude model the app picked before the shared one (shown until one is chosen).
    pub app_claude_model: Option<String>,
    pub opener: Opener,
    /// `POST /api/links/open {url, background: true}`: start the app without bringing it to the
    /// front (a page that only needs the app's server, e.g. 做成簡報 listing L-Slide's bases).
    pub starter: Opener,
    /// Ping the local server for its state (off in tests).
    pub probe: bool,
    /// [`router`] installs the shared file as the process routing (off: the settings page
    /// works, routing stays the tasks' own order; for app tests that must not depend on it).
    pub install_routing: bool,
    /// The runtime ports file `GET /api/links/ports` reads (default: the OS cache dir's).
    pub ports_file: Option<std::path::PathBuf>,
    /// Whether an API key exists, read once off the async threads (a keychain read can
    /// wait on a password prompt; it must never stall the server). Cleared by PUT.
    api_known: Arc<std::sync::Mutex<Option<bool>>>,
    /// `agy models` (a fake in tests) and where its last answer is kept (the cache folder).
    pub agy_lister: Arc<dyn ModelLister>,
    pub agy_file: Option<std::path::PathBuf>,
    agy_busy: Arc<std::sync::atomic::AtomicBool>,
    /// Unix seconds of the last listing attempt, so a failing CLI is not retried at every GET.
    agy_tried: Arc<std::sync::atomic::AtomicU64>,
    /// `GET /v1/models` (a fake in tests), where its last answer is kept, like the Google list.
    pub claude_lister: Arc<dyn ClaudeLister>,
    pub claude_file: Option<std::path::PathBuf>,
    claude_busy: Arc<std::sync::atomic::AtomicBool>,
    claude_tried: Arc<std::sync::atomic::AtomicU64>,
}

/// A listing is refreshed by itself when older than this (a day).
pub const AGY_MAX_AGE: u64 = 24 * 3600;
/// After a failed automatic listing, wait this long before trying again.
const AGY_RETRY: u64 = 600;

/// No CLI at all (isolated engines).
struct NoAgy;

#[async_trait::async_trait]
impl ModelLister for NoAgy {
    fn installed(&self) -> bool {
        false
    }
    async fn list(&self) -> Result<Vec<agy::ModelInfo>, String> {
        Err("找不到 Antigravity CLI（agy）".into())
    }
}

impl Engines {
    /// `app` = the app's old keychain service ("PaperComposer"…), `tasks_toml` = its own
    /// tasks file (a file that does not parse lists no tasks, with a note on stderr).
    pub fn new(home: Home, app: &str, tasks_toml: &str) -> Engines {
        let tasks = TaskTable::parse(tasks_toml).unwrap_or_else(|e| {
            eprintln!("lore-engines: {e}");
            TaskTable::default()
        });
        let layout = lore_llm::local::home_or(None, home.local_home().as_deref()).map(|h| Layout::shared(&h));
        Engines {
            home,
            tasks,
            catalog: Catalog::bundled(),
            shared: None,
            layout,
            external: None,
            include_uncensored: false,
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            keys: SharedKeys::for_app(app, None),
            app_claude_model: None,
            opener: Arc::new(os_open),
            starter: Arc::new(os_start),
            probe: true,
            install_routing: true,
            ports_file: lore_home::ports::default_path(),
            api_known: Arc::new(std::sync::Mutex::new(None)),
            agy_lister: Arc::new(CliLister::default()),
            agy_file: agy::default_path(),
            agy_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            agy_tried: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            claude_lister: Arc::new(HttpLister::default()),
            claude_file: claude_models::default_path(),
            claude_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            claude_tried: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// The Claude model lister and the file that keeps its answer (tests).
    pub fn claude_list(mut self, lister: Arc<dyn ClaudeLister>, file: Option<std::path::PathBuf>) -> Self {
        self.claude_lister = lister;
        self.claude_file = file;
        self
    }

    fn claude_stored(&self) -> Option<Stored> {
        self.claude_file.as_deref().and_then(Stored::load)
    }

    /// The API key, read off the async threads (a keychain read can wait on a prompt).
    async fn api_key(&self) -> Option<String> {
        let keys = self.keys.clone();
        tokio::task::spawn_blocking(move || keys.get(None, &Account::Single).map(|(k, _)| k)).await.ok().flatten()
    }

    /// List the Claude models now and keep the answer. Without an API key there is nothing to
    /// do (`Ok`, the catalog stays the list). One listing at a time.
    pub async fn refresh_claude(&self) -> Result<(), String> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.claude_busy.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
            return Ok(());
        }
        self.claude_tried.store(agy::now(), SeqCst);
        let r = async {
            let Some(key) = self.api_key().await else { return Ok(()) };
            let fresh = self.claude_lister.list(&key).await?;
            let file = self.claude_file.as_deref().ok_or("沒有可存放的快取資料夾")?;
            claude_models::after(Stored::load(file).as_ref(), fresh, agy::now()).save(file).map_err(|e| format!("無法儲存模型清單：{e}"))
        }
        .await;
        self.claude_busy.store(false, SeqCst);
        r
    }

    /// The `agy models` lister and the file that keeps its answer (tests).
    pub fn agy(mut self, lister: Arc<dyn ModelLister>, file: Option<std::path::PathBuf>) -> Self {
        self.agy_lister = lister;
        self.agy_file = file;
        self
    }

    fn stored(&self) -> Option<Stored> {
        self.agy_file.as_deref().and_then(Stored::load)
    }

    /// Run `agy models` now and keep the answer. One listing at a time (a second caller gets
    /// the stored one back untouched).
    pub async fn refresh_google(&self) -> Result<(), String> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.agy_busy.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
            return Ok(());
        }
        self.agy_tried.store(agy::now(), SeqCst);
        let r = async {
            let fresh = self.agy_lister.list().await?;
            let file = self.agy_file.as_deref().ok_or("沒有可存放的快取資料夾")?;
            Stored::after(Stored::load(file).as_ref(), fresh, agy::now()).save(file).map_err(|e| format!("無法儲存模型清單：{e}"))
        }
        .await;
        self.agy_busy.store(false, SeqCst);
        r
    }

    /// The `google` part of the GET body; starts a background listing when the stored one is
    /// missing or older than [`AGY_MAX_AGE`].
    async fn google(&self) -> Value {
        use std::sync::atomic::Ordering::SeqCst;
        let installed = self.agy_lister.installed();
        let stored = self.stored();
        let now = agy::now();
        let due = stored.as_ref().is_none_or(|s| s.stale(now, AGY_MAX_AGE)) && now.saturating_sub(self.agy_tried.load(SeqCst)) >= AGY_RETRY;
        if installed && due && !self.agy_busy.load(SeqCst) {
            let me = self.clone();
            tokio::spawn(async move {
                let _ = me.refresh_google().await;
            });
        }
        let s = stored.unwrap_or_default();
        json!({
            "available": installed && !s.models.is_empty(),
            "models": s.models,
            "checked": (s.checked > 0).then_some(s.checked),
            "new": s.new,
        })
    }

    /// The shared llama-server, as this process sees it. Its idle check (every app, the
    /// shared `[local] idle_minutes`) starts here, or on the first request when there is no
    /// async runtime yet.
    pub fn shared(mut self, rt: Arc<SharedRuntime>) -> Self {
        self.shared = Some(rt);
        self.watch();
        self
    }

    /// The app's own model folders (L-Paper before `LOCAL_LLM_HOME`).
    pub fn layout(mut self, layout: Layout) -> Self {
        self.layout = Some(layout);
        self
    }

    fn watch(&self) {
        if let Some(rt) = &self.shared {
            let home = self.home.clone();
            rt.watch_idle(Arc::new(move || idle_duration(idle_minutes(&home))));
        }
    }

    /// The app's `[engine]` (an external server), when it has one.
    pub fn external(mut self, url: Option<String>, model: Option<String>) -> Self {
        self.external = url.filter(|u| !u.trim().is_empty()).map(|u| (u, model));
        self
    }

    /// The app's Claude CLI and its status cache (share them so a sign-in shows at once).
    pub fn cli(mut self, cli: Cli, cc: Arc<Cache>) -> Self {
        self.cli = cli;
        self.cc = cc;
        self
    }

    pub fn claude_model(mut self, m: Option<String>) -> Self {
        self.app_claude_model = m.filter(|m| !m.trim().is_empty());
        self
    }

    /// More tasks to list (L-Slide: lore-flow's).
    pub fn with_tasks(mut self, more: TaskTable) -> Self {
        self.tasks = std::mem::take(&mut self.tasks).merge(more);
        self
    }

    /// Never the keychain, the environment key, the `claude` CLI, the model folder, the
    /// network or the OS opener (tests).
    pub fn isolated(mut self) -> Self {
        self.keys.isolate();
        self.cli = Cli { bin: Some(std::env::temp_dir().join("lore-engines-no-claude")), config_dir: None };
        self.layout = None;
        self.shared = None;
        self.probe = false;
        self.ports_file = None;
        self.agy_lister = Arc::new(NoAgy);
        self.agy_file = None;
        self.claude_file = None;
        self.opener = Arc::new(|_| Err("isolated".into()));
        self.starter = Arc::new(|_| Err("isolated".into()));
        self
    }

    /// The shared file as lore-llm's routing settings.
    pub fn settings(&self) -> EngineSettings {
        settings_of(&self.home)
    }

    fn label(&self, id: &str) -> String {
        self.catalog.get(id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string())
    }

    async fn local(&self) -> Value {
        let chosen = self.home.engine().and_then(|e| e.model);
        let ext = self.external.clone();
        let mut models: Vec<Value> = match &self.layout {
            Some(l) => l
                .installed(&self.catalog, self.include_uncensored)
                .into_iter()
                .filter(|m| m.is_brain())
                .map(|m| json!({ "id": m.id, "label": m.name, "vision": m.vision }))
                .collect(),
            None => vec![],
        };
        let snap = match &self.shared {
            Some(rt) if ext.is_none() => Some(rt.snapshot().await),
            _ => None,
        };
        let me = self.shared.as_ref().map_or(0, |r| r.pid);
        let loaded = snap.as_ref().and_then(|s| s.registry.server.clone());
        let (state, url, model, vision) = match (&ext, &snap) {
            (Some((url, model)), _) => {
                let up = self.probe && OpenAiCompat::new(OpenAiCompatConfig::llama_server(url.clone(), model.clone().unwrap_or_else(|| "local".into()))).ping().await;
                (if up { "ready" } else { "off" }, Some(url.clone()), model.clone(), None)
            }
            (None, Some(s)) => match (&s.status, &loaded) {
                (LocalStatus::Loading { .. } | LocalStatus::Switching { .. }, Some(sv)) => ("starting", None, chosen.clone().or(Some(sv.model.clone())), None),
                (_, Some(sv)) => ("ready", Some(sv.url.clone()), chosen.clone().or(Some(sv.model.clone())), Some(sv.vision).filter(|_| chosen.as_deref().is_none_or(|c| c == sv.model))),
                (_, None) if models.is_empty() => ("missing", None, chosen.clone(), None),
                (_, None) => ("off", None, chosen.clone(), None),
            },
            (None, None) if models.is_empty() => ("missing", None, chosen.clone(), None),
            (None, None) => ("off", None, chosen.clone(), None),
        };
        let vision = vision.unwrap_or_else(|| model.as_deref().and_then(|m| self.catalog.get(m)).is_some_and(|m| m.vision));
        if let Some(m) = &model
            && !models.iter().any(|x| x["id"] == m.as_str())
        {
            models.insert(0, json!({ "id": m, "label": self.label(m), "vision": vision }));
        }
        let status = match (&ext, &snap) {
            (Some(_), _) => None,
            (None, Some(s)) => Some(match &s.status {
                LocalStatus::Off => "未載入 · 用到時自動載入".to_string(),
                LocalStatus::Loaded { .. } => {
                    // every app holding the server (this one too), not only the others
                    let mut apps: Vec<String> = s.registry.leases.iter().map(|l| app_label(&l.app)).collect();
                    apps.sort();
                    apps.dedup();
                    let name = loaded.as_ref().map(|sv| self.label(&sv.model)).unwrap_or_default();
                    if apps.is_empty() { format!("已載入 {name}") } else { format!("已載入 · {} 正在用", apps.join("、")) }
                }
                other => other.text(&self.catalog),
            }),
            (None, None) => Some("未載入 · 用到時自動載入".to_string()),
        };
        let users: Vec<String> = snap.as_ref().map(|s| s.users(me).iter().map(|a| app_label(a)).collect()).unwrap_or_default();
        let waiting: Vec<Value> = snap
            .as_ref()
            .map(|s| s.registry.wanting.iter().map(|w| json!({ "app": app_label(&w.app), "model": w.model, "label": self.label(&w.model) })).collect())
            .unwrap_or_default();
        let can_release = snap.as_ref().is_some_and(|s| s.can_release);
        json!({
            "state": state,
            "status": status,
            "url": url,
            "model": model,
            "label": model.as_deref().map(|m| self.label(m)),
            "loaded": loaded.as_ref().map(|s| s.model.clone()),
            "vision": vision,
            "models": models,
            "users": users,
            "waiting": waiting,
            "idle_minutes": idle_minutes(&self.home),
            "can_release": can_release,
            "release_blocked": (!can_release && loaded.is_some()).then(|| users.first().map(|u| format!("{u} 正在用"))).flatten(),
        })
    }

    /// `opus` for `claude-opus-5-5`, else the id.
    fn alias(&self, id: &str) -> String {
        let c = &self.catalog.cloud;
        c.aliases.iter().find(|(_, v)| v.as_str() == id).map(|(k, _)| k.clone()).unwrap_or_else(|| id.to_string())
    }

    async fn claude(&self, s: &EngineSettings) -> Value {
        let st = self.cc.get(&self.cli, false).await;
        let known = *self.api_known.lock().unwrap_or_else(|e| e.into_inner());
        let api = match known {
            Some(b) => b,
            None => {
                let keys = self.keys.clone();
                let b = tokio::task::spawn_blocking(move || keys.get(None, &Account::Single).is_some()).await.unwrap_or(false);
                *self.api_known.lock().unwrap_or_else(|e| e.into_inner()) = Some(b);
                b
            }
        };
        let via = match s.claude_via {
            Some(lore_llm::router::ClaudeVia::Api) => "api",
            Some(_) => "subscription",
            None if !st.ready && api => "api",
            None => "subscription",
        };
        let state = match via {
            "api" if api => "ready",
            "subscription" if st.ready => "ready",
            "subscription" if !st.installed => "missing",
            _ => "off",
        };
        let pick = s.claude_model.clone().or_else(|| self.app_claude_model.clone()).unwrap_or_else(|| self.catalog.cloud.default.clone());
        let model = self.catalog.cloud_model(&pick).map(|m| self.alias(&m.id)).unwrap_or(pick);
        let mut models: Vec<Value> = self
            .catalog
            .cloud
            .models
            .iter()
            .map(|m| {
                let id = self.alias(&m.id);
                let label = m.summary.split('・').next().filter(|l| !l.is_empty()).map(String::from).unwrap_or_else(|| id.clone());
                json!({ "id": id, "label": label, "vision": true, "new": false })
            })
            .collect();
        // what the API lists beyond the catalog (nothing is ever removed)
        let stored = self.claude_stored();
        let now = agy::now();
        if let Some(st) = &stored {
            for m in st.models.iter().filter(|m| !self.catalog.cloud.models.iter().any(|c| c.id == m.id) && !self.catalog.cloud.aliases.contains_key(&m.id)) {
                models.push(json!({ "id": m.id, "label": m.label, "vision": true, "new": st.new.contains(&m.id) }));
            }
        }
        let due = api && stored.as_ref().is_none_or(|s| s.stale(now, AGY_MAX_AGE)) && now.saturating_sub(self.claude_tried.load(std::sync::atomic::Ordering::SeqCst)) >= AGY_RETRY;
        if due && self.claude_file.is_some() && !self.claude_busy.load(std::sync::atomic::Ordering::SeqCst) {
            let me = self.clone();
            tokio::spawn(async move {
                let _ = me.refresh_claude().await;
            });
        }
        json!({
            "state": state,
            "via": via,
            "vias": { "subscription": st.ready, "api": api },
            "model": model,
            // the subscription (Claude Code) cannot send images here; only the API route with a key can
            "vision": via == "api" && api,
            "models": models,
            "listed": stored.is_some(),
            "checked": stored.as_ref().filter(|s| s.checked > 0).map(|s| s.checked),
            "can_list": api,
        })
    }

    /// The `GET /api/engines` body.
    pub async fn state(&self) -> Value {
        self.watch(); // armed here too when the router was built outside a runtime
        let s = self.settings();
        let classes: Vec<Value> = TaskClass::ALL
            .iter()
            .map(|c| json!({ "id": c.id(), "label": c.label(), "route": s.route(*c).id(), "auto": c.auto_order().map(|k| if k == shared::EngineKind::Claude { "claude" } else { "local" }) }))
            .collect();
        let tasks: Vec<Value> = self
            .tasks
            .task
            .iter()
            .map(|(id, t)| {
                let last = shared::last_used_at(id);
                json!({
                    "id": id, "label": t.label.clone().unwrap_or_else(|| id.clone()), "class": t.class.map(TaskClass::id),
                    // what the 模型 tab needs to offer a model per step: its own order, and whether a text-only model may answer
                    "default": t.default, "fallback": t.fallback, "agy": t.agy && !t.needs.vision && !t.needs.web, "vision": t.needs.vision,
                    "purpose": t.purpose_id(),
                    "last": last.as_ref().map(|l| l.0.clone()),
                    "last_label": last.as_ref().and_then(|l| self.catalog.engine_name(&l.0)),
                    "last_at": last.as_ref().map(|l| l.1),
                })
            })
            .collect();
        let overrides: serde_json::Map<String, Value> =
            self.tasks.task.keys().map(|id| (id.clone(), Value::from(s.models.get(id).cloned().unwrap_or_else(|| s.overrides.get(id).map_or("auto", |m| m.id()).to_string())))).collect();
        json!({
            "local": self.local().await,
            "claude": self.claude(&s).await,
            "google": self.google().await,
            "classes": classes,
            "tasks": tasks,
            "overrides": overrides,
            // 用途: the same six in every Lore app; `model` = the one model its steps use ("" = each
            // step's own order), `agy` = whether a text-only Antigravity model may be named for it
            "purposes": shared::PURPOSES.iter().map(|(id, label)| json!({
                "id": id, "label": label, "model": s.purposes.get(*id).cloned().unwrap_or_default(), "agy": !matches!(*id, "see" | "web"),
                "steps": self.tasks.task.values().filter(|t| t.purpose_id() == *id).count(),
            })).collect::<Vec<_>>(),
        })
    }

    /// Apply a `PUT /api/engines` body (all checked before anything is written).
    pub async fn update(&self, body: &Value) -> Result<(), String> {
        self.watch();
        let obj = body.as_object().ok_or("要送 JSON 物件")?;
        let str_of = |v: &Value, what: &str| v.as_str().map(str::trim).map(String::from).ok_or(format!("{what}要是文字"));
        // local
        let mut pick = None;
        let mut idle = None;
        let mut release = None;
        if let Some(l) = obj.get("local").filter(|v| !v.is_null()) {
            if let Some(v) = l.get("idle_minutes").filter(|v| !v.is_null()) {
                let n = v.as_u64().filter(|n| *n <= MAX_IDLE_MINUTES).ok_or(format!("閒置釋放要是 0–{MAX_IDLE_MINUTES} 的整數分鐘"))?;
                idle = Some(n);
            }
            if l.get("release").and_then(Value::as_bool) == Some(true) {
                release = Some(self.shared.clone().ok_or("沒有本機模型可釋放")?);
            }
        }
        if let Some(l) = obj.get("local").filter(|v| !v.is_null())
            && let Some(m) = l.get("model").filter(|v| !v.is_null())
        {
            let m = str_of(m, "本機模型")?;
            let model = self.catalog.get(&m).filter(|x| x.is_brain()).ok_or(format!("模型目錄裡沒有本機模型「{m}」"))?;
            if model.uncensored && !self.include_uncensored {
                return Err("這個工具不使用移除拒答的模型".into());
            }
            if let Some(why) = self.layout.as_ref().and_then(|l| l.missing(model)) {
                return Err(why);
            }
            pick = Some(m);
        }
        // claude
        let mut claude = None;
        if let Some(c) = obj.get("claude").filter(|v| !v.is_null()) {
            let via = match c.get("via").filter(|v| !v.is_null()) {
                Some(v) => {
                    let v = str_of(v, "連線方式")?;
                    if !lore_home::engine::VIAS.contains(&v.as_str()) {
                        return Err(format!("不認得的連線方式「{v}」"));
                    }
                    Some(v)
                }
                None => None,
            };
            let model = match c.get("model").filter(|v| !v.is_null()) {
                Some(v) => {
                    let m = str_of(v, "Claude 模型")?;
                    if self.catalog.cloud_model(&m).is_none() && !self.claude_stored().is_some_and(|s| s.models.iter().any(|x| x.id == m)) {
                        return Err(format!("模型目錄裡沒有 Claude 模型「{m}」"));
                    }
                    Some(m)
                }
                None => None,
            };
            claude = Some((via, model));
        }
        // routes
        let mut routes = Vec::new();
        if let Some(r) = obj.get("routes").filter(|v| !v.is_null()) {
            for (class, mode) in r.as_object().ok_or("routes 要是物件")? {
                let mode = str_of(mode, "走向")?;
                if TaskClass::parse(class).is_none() {
                    return Err(format!("不認得的任務類別「{class}」"));
                }
                if lore_llm::RouteMode::parse(&mode).is_none() {
                    return Err(format!("不認得的走向「{mode}」"));
                }
                routes.push((class.clone(), mode));
            }
        }
        // purposes: a model for every step of that purpose, or "auto" / "" to clear it
        let mut purposes = Vec::new();
        if let Some(o) = obj.get("purposes").filter(|v| !v.is_null()) {
            for (id, model) in o.as_object().ok_or("purposes 要是物件")? {
                let Some((_, label)) = shared::PURPOSES.iter().find(|(p, _)| p == id) else { return Err(format!("沒有這個用途「{id}」")) };
                let model = if model.is_null() { String::new() } else { str_of(model, "模型")? };
                let model = model.trim().to_string();
                if model.is_empty() || model == "auto" {
                    purposes.push((id.clone(), None));
                } else if !shared::is_model_ref(&model) {
                    return Err(format!("不認得的模型「{model}」"));
                } else if model.starts_with("agy:") && matches!(id.as_str(), "see" | "web") {
                    return Err(format!("「{label}」不能用 Antigravity 的模型（它們只收文字，不能看圖也不能上網）"));
                } else {
                    purposes.push((id.clone(), Some(model)));
                }
            }
        }
        // overrides
        let mut overrides = Vec::new();
        if let Some(o) = obj.get("overrides").filter(|v| !v.is_null()) {
            for (task, mode) in o.as_object().ok_or("overrides 要是物件")? {
                if !self.tasks.task.contains_key(task) {
                    return Err(format!("沒有這個任務「{task}」"));
                }
                let mode = if mode.is_null() { "auto".to_string() } else { str_of(mode, "走向")? };
                if shared::is_model_ref(&mode) {
                    // a named model for this task: Antigravity's are text only and need the task's leave
                    let spec = &self.tasks.task[task];
                    if mode.starts_with("agy:") && (!spec.agy || spec.needs.vision || spec.needs.web) {
                        return Err(format!("「{}」不能用 Antigravity 的模型（要看圖、要上網，或沒有開放）", spec.label.clone().unwrap_or_else(|| task.clone())));
                    }
                } else if lore_llm::RouteMode::parse(&mode).is_none() {
                    return Err(format!("不認得的走向「{mode}」"));
                }
                overrides.push((task.clone(), mode));
            }
        }
        let io = |e: std::io::Error| format!("無法儲存設定：{e}");
        if let Some(n) = idle {
            set_idle_minutes(&self.home, n).map_err(io)?;
        }
        if let Some(m) = pick {
            // only the shared default id; the model loads when a task needs it
            self.home.set_engine_model(Some(&m)).map_err(io)?;
        }
        if let Some(rt) = release {
            rt.release().await?;
        }
        if let Some((via, model)) = claude {
            self.home.set_claude(via.as_deref(), model.as_deref()).map_err(io)?;
        }
        for (c, m) in routes {
            self.home.set_route(&c, &m).map_err(io)?;
        }
        for (t, m) in overrides {
            self.home.set_override(&t, Some(&m)).map_err(io)?;
        }
        for (p, m) in purposes {
            self.home.set_purpose(&p, m.as_deref()).map_err(io)?;
        }
        Ok(())
    }
}

/// Longest idle time that can be set (a day).
pub const MAX_IDLE_MINUTES: u64 = 1440;

fn engine_toml(home: &Home) -> Option<std::path::PathBuf> {
    home.shared_dir().map(|d| d.join(lore_home::engine::FILE))
}

/// `[local] idle_minutes` of the shared engine file (default
/// [`lore_llm::local::DEFAULT_IDLE_MINUTES`]; 0 = never release).
pub fn idle_minutes(home: &Home) -> u64 {
    let t: Option<toml::Table> = engine_toml(home).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.parse().ok());
    t.as_ref()
        .and_then(|t| t.get("local"))
        .and_then(|l| l.get("idle_minutes"))
        .and_then(toml::Value::as_integer)
        .and_then(|n| u64::try_from(n).ok())
        .map(|n| n.min(MAX_IDLE_MINUTES))
        .unwrap_or(lore_llm::local::DEFAULT_IDLE_MINUTES)
}

/// Minutes → the runtime's idle time (0 = never).
pub fn idle_duration(minutes: u64) -> Option<std::time::Duration> {
    (minutes > 0).then(|| std::time::Duration::from_secs(minutes * 60))
}

/// Write `[local] idle_minutes` (the file's other tables and keys stay; atomic).
pub fn set_idle_minutes(home: &Home, minutes: u64) -> std::io::Result<()> {
    let path = engine_toml(home).ok_or_else(|| std::io::Error::other("no shared settings folder"))?;
    let mut t: toml::Table = std::fs::read_to_string(&path).ok().and_then(|s| s.parse().ok()).unwrap_or_default();
    let e = t.entry("local").or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !e.is_table() {
        *e = toml::Value::Table(toml::Table::new());
    }
    e.as_table_mut().expect("a table").insert("idle_minutes".into(), toml::Value::Integer(minutes.min(MAX_IDLE_MINUTES) as i64));
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no shared settings folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}.idle.tmp", lore_home::engine::FILE, std::process::id()));
    std::fs::write(&tmp, t.to_string())?;
    std::fs::rename(&tmp, &path)
}

/// `private/shared/engine.toml` as lore-llm's routing settings.
pub fn settings_of(home: &Home) -> EngineSettings {
    let s = home.engine_settings();
    EngineSettings::from_strings(s.claude.via.as_deref(), s.claude.model.as_deref(), &s.route, &s.overrides).with_purposes(&s.purpose)
}

/// Make every lore-llm router of this process follow `home`'s shared file.
pub fn install(home: &Home) {
    let home = home.clone();
    shared::install(Arc::new(move || settings_of(&home)));
}

fn bad(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.into() }))).into_response()
}

/// The routes, for any app state `S`. Also [`install`]s the shared file as the routing.
pub fn router<S: Clone + Send + Sync + 'static>(e: Engines) -> axum::Router<S> {
    if e.install_routing {
        install(&e.home);
    }
    if e.shared.is_some() {
        // the server's URL is runtime data now: an old `[engine] url` goes
        let _ = e.home.forget_engine_url();
    }
    let e = Arc::new(e);
    let (g, p, o, r, q) = (e.clone(), e.clone(), e.clone(), e.clone(), e);
    axum::Router::new()
        .route(
            "/api/engines",
            get(move || {
                let e = g.clone();
                async move { Json(e.state().await) }
            })
            .put(move |body: Option<Json<Value>>| {
                let e = p.clone();
                async move {
                    let Some(Json(body)) = body else { return bad("要送 JSON 物件") };
                    // a key may have been saved meanwhile (the app's own setup flow)
                    *e.api_known.lock().unwrap_or_else(|x| x.into_inner()) = None;
                    match e.update(&body).await {
                        Ok(()) => Json(e.state().await).into_response(),
                        Err(m) => bad(m),
                    }
                }
            }),
        )
        .route(
            "/api/engines/refresh",
            post(move || {
                let e = r.clone();
                async move {
                    let (g, c) = tokio::join!(e.refresh_google(), e.refresh_claude());
                    let mut body = e.state().await;
                    if let Err(m) = g {
                        body["google"]["error"] = Value::from(m);
                    }
                    if let Err(m) = c {
                        body["claude"]["error"] = Value::from(m);
                    }
                    Json(body)
                }
            }),
        )
        .route(
            "/api/links/ports",
            get(move || {
                let e = q.clone();
                async move {
                    let live = match e.ports_file.clone() {
                        Some(f) => tokio::task::spawn_blocking(move || lore_home::ports::live_at(&f)).await.unwrap_or_default(),
                        None => Default::default(),
                    };
                    Json(live)
                }
            }),
        )
        .route(
            "/api/links/open",
            post(move |body: Option<Json<Value>>| {
                let e = o.clone();
                async move {
                    let url = body.as_ref().and_then(|Json(b)| b.get("url")).and_then(Value::as_str).unwrap_or("").trim().to_string();
                    if !link_allowed(&url) {
                        return bad("只能開啟 LoreForge 的連結");
                    }
                    let background = body.as_ref().and_then(|Json(b)| b.get("background")).and_then(Value::as_bool).unwrap_or(false);
                    match if background { (e.starter)(&url) } else { (e.opener)(&url) } {
                        Ok(()) => StatusCode::NO_CONTENT.into_response(),
                        Err(m) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": m }))).into_response(),
                    }
                }
            }),
        )
}

/// `lore-pc://`, `lore-sc://`, `lore-lr://`, `lore-kb://` links and `http://127.0.0.1`
/// pages (any port) only; no spaces or control characters.
pub fn link_allowed(url: &str) -> bool {
    if url.is_empty() || url.len() > 4096 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    if ["lore-pc://", "lore-sc://", "lore-lr://", "lore-kb://"].iter().any(|p| url.starts_with(p)) {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://127.0.0.1") else { return false };
    let rest = match rest.strip_prefix(':') {
        Some(r) => {
            let digits = r.chars().take_while(char::is_ascii_digit).count();
            if digits == 0 || digits > 5 {
                return false;
            }
            &r[digits..]
        }
        None => rest,
    };
    rest.is_empty() || rest.starts_with(['/', '?', '#'])
}

/// Hand `url` to the OS (`open` / `xdg-open` / the Windows URL handler); no shell.
pub fn os_open(url: &str) -> Result<(), String> {
    let mut c = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let mut child = c.arg(url).stdin(std::process::Stdio::null()).spawn().map_err(|e| format!("無法開啟連結：{e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Start the app of a `lore-<id>://` link without bringing it to the front: `open -g -a <App>
/// --args --background` with no URL (each app focuses its window when a link arrives; with
/// `--background` it starts with its window hidden until a link or the Dock icon shows it). The link's page is
/// not opened (only the app's server is needed). Not macOS: [`os_open`].
pub fn os_start(url: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return os_open(url);
    }
    let id = url.strip_prefix("lore-").and_then(|r| r.split_once("://")).map(|(id, _)| id).ok_or("只能在背景開啟 Lore App")?;
    let name = lore_home::ports::app_name(id).to_string();
    let mut child = std::process::Command::new("open").args(["-g", "-a", &name, "--args", "--background"]).stdin(std::process::Stdio::null()).spawn().map_err(|e| format!("無法開啟 {name}：{e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests;
