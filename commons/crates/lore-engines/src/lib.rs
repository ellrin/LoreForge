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
//! - Memory of the local model: `local.idle_minutes` (GET; PUT `{ local: { idle_minutes: 15 } }`,
//!   0 = never; kept in `engine.toml` `[local]`, shared by every app) and `local.can_release`;
//!   PUT `{ local: { release: true } }` stops the app's own llama-server now. Only the app that
//!   runs the server ([`Engines::runtime`], PaperComposer) releases; the others get a 400.
//! - `GET /api/links/ports` → `{ "sc": 51234, … }`: the desktop apps running now (lore-home `ports`).
//! - `POST /api/links/open { url }` → hands a `lore-(pc|sc|lr|kb)://` or
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
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::local::{Layout, LocalRuntime, Phase};
use lore_llm::secrets::{Account, SharedKeys};
use lore_llm::shared::{self, EngineSettings, TaskClass};
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
    /// The llama-server the app runs itself, if any (PaperComposer): its phase is the
    /// local state, and `PUT local.model` starts that model.
    pub runtime: Option<Arc<LocalRuntime>>,
    /// Where downloaded models are (`LOCAL_LLM_HOME` by default), for the model list.
    pub layout: Option<Layout>,
    /// The app's own `[engine] url` (wins over the shared one, as in the app's routing).
    pub external: Option<(String, Option<String>)>,
    /// List abliterated models (PaperComposer yes, household apps no).
    pub include_uncensored: bool,
    pub cli: Cli,
    pub cc: Arc<Cache>,
    pub keys: SharedKeys,
    /// The Claude model the app picked before the shared one (shown until one is chosen).
    pub app_claude_model: Option<String>,
    pub opener: Opener,
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
}

impl Engines {
    /// `app` = the app's old keychain service ("PaperComposer"…), `tasks_toml` = its own
    /// tasks file (a file that does not parse lists no tasks, with a note on stderr).
    pub fn new(home: Home, app: &str, tasks_toml: &str) -> Engines {
        let tasks = TaskTable::parse(tasks_toml).unwrap_or_else(|e| {
            eprintln!("lore-engines: {e}");
            TaskTable::default()
        });
        Engines {
            home,
            tasks,
            catalog: Catalog::bundled(),
            runtime: None,
            layout: lore_llm::local::home(None).map(|h| Layout::shared(&h)),
            external: None,
            include_uncensored: false,
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            keys: SharedKeys::for_app(app, None),
            app_claude_model: None,
            opener: Arc::new(os_open),
            probe: true,
            install_routing: true,
            ports_file: lore_home::ports::default_path(),
            api_known: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// The app's own llama-server and model folders. The runtime follows the shared idle
    /// setting (`[local] idle_minutes`) and its idle watcher is started.
    pub fn runtime(mut self, rt: Arc<LocalRuntime>, layout: Layout) -> Self {
        let home = self.home.clone();
        rt.idle_from(Arc::new(move || idle_duration(idle_minutes(&home))));
        rt.watch_idle();
        self.runtime = Some(rt);
        self.layout = Some(layout);
        self
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

    /// More tasks to list (SlideComposer: lore-flow's).
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
        self.probe = false;
        self.ports_file = None;
        self.opener = Arc::new(|_| Err("isolated".into()));
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
        let shared = self.home.engine();
        let ext = self.external.clone().or_else(|| shared.and_then(|e| Some((e.url?, e.model))));
        let mut models: Vec<Value> = match &self.layout {
            Some(l) => l
                .installed(&self.catalog, self.include_uncensored)
                .into_iter()
                .filter(|m| m.is_brain())
                .map(|m| json!({ "id": m.id, "label": m.name, "vision": m.vision }))
                .collect(),
            None => vec![],
        };
        let phase = self.runtime.as_ref().map(|r| r.phase()).unwrap_or(Phase::Idle);
        let (state, url, model, vision) = match phase {
            Phase::Starting { model } => ("starting", None, Some(model), None),
            Phase::Ready { model, url } => {
                let vision = self.runtime.as_ref().unwrap().current_info().await.map(|i| i.vision);
                ("ready", Some(url), Some(model), vision)
            }
            Phase::Error { .. } => ("error", None, None, None),
            Phase::Idle => match ext {
                Some((url, model)) => {
                    let up = self.probe && OpenAiCompat::new(OpenAiCompatConfig::llama_server(url.clone(), model.clone().unwrap_or_else(|| "local".into()))).ping().await;
                    (if up { "ready" } else { "off" }, Some(url), model, None)
                }
                None if models.is_empty() => ("missing", None, None, None),
                None => ("off", None, None, None),
            },
        };
        let vision = vision.unwrap_or_else(|| model.as_deref().and_then(|m| self.catalog.get(m)).is_some_and(|m| m.vision));
        if let Some(m) = &model
            && !models.iter().any(|x| x["id"] == m.as_str())
        {
            models.insert(0, json!({ "id": m, "label": self.label(m), "vision": vision }));
        }
        json!({
            "state": state,
            "url": url,
            "model": model,
            "label": model.as_deref().map(|m| self.label(m)),
            "vision": vision,
            "models": models,
            "idle_minutes": idle_minutes(&self.home),
            "can_release": self.runtime.is_some(),
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
        let models: Vec<Value> = self
            .catalog
            .cloud
            .models
            .iter()
            .map(|m| {
                let id = self.alias(&m.id);
                let label = m.summary.split('・').next().filter(|l| !l.is_empty()).map(String::from).unwrap_or_else(|| id.clone());
                json!({ "id": id, "label": label, "vision": true })
            })
            .collect();
        json!({
            "state": state,
            "via": via,
            "vias": { "subscription": st.ready, "api": api },
            "model": model,
            // the subscription (Claude Code) cannot read images here; the API can
            "vision": api,
            "models": models,
        })
    }

    /// The `GET /api/engines` body.
    pub async fn state(&self) -> Value {
        if let Some(rt) = &self.runtime {
            rt.watch_idle(); // armed here too when the router was built outside a runtime
        }
        let s = self.settings();
        let classes: Vec<Value> = TaskClass::ALL
            .iter()
            .map(|c| json!({ "id": c.id(), "label": c.label(), "route": s.route(*c).id(), "auto": c.auto_order().map(|k| if k == shared::EngineKind::Claude { "claude" } else { "local" }) }))
            .collect();
        let tasks: Vec<Value> = self
            .tasks
            .task
            .iter()
            .map(|(id, t)| json!({ "id": id, "label": t.label.clone().unwrap_or_else(|| id.clone()), "class": t.class.map(TaskClass::id), "last": shared::last_used(id) }))
            .collect();
        let overrides: serde_json::Map<String, Value> =
            self.tasks.task.keys().map(|id| (id.clone(), Value::from(s.overrides.get(id).map_or("auto", |m| m.id())))).collect();
        json!({
            "local": self.local().await,
            "claude": self.claude(&s).await,
            "classes": classes,
            "tasks": tasks,
            "overrides": overrides,
        })
    }

    /// Apply a `PUT /api/engines` body (all checked before anything is written).
    pub async fn update(&self, body: &Value) -> Result<(), String> {
        if let Some(rt) = &self.runtime {
            rt.watch_idle();
        }
        let obj = body.as_object().ok_or("要送 JSON 物件")?;
        let str_of = |v: &Value, what: &str| v.as_str().map(str::trim).map(String::from).ok_or(format!("{what}要是文字"));
        // local
        let mut start = None;
        let mut idle = None;
        let mut release = None;
        if let Some(l) = obj.get("local").filter(|v| !v.is_null()) {
            if let Some(v) = l.get("idle_minutes").filter(|v| !v.is_null()) {
                let n = v.as_u64().filter(|n| *n <= MAX_IDLE_MINUTES).ok_or(format!("閒置釋放要是 0–{MAX_IDLE_MINUTES} 的整數分鐘"))?;
                idle = Some(n);
            }
            if l.get("release").and_then(Value::as_bool) == Some(true) {
                release = Some(self.runtime.clone().ok_or("本機模型請在 PaperComposer 釋放")?);
            }
        }
        if let Some(l) = obj.get("local").filter(|v| !v.is_null())
            && let Some(m) = l.get("model").filter(|v| !v.is_null())
        {
            {
                let m = str_of(m, "本機模型")?;
                let current = self.home.engine().and_then(|e| e.model);
                let running = match &self.runtime {
                    Some(r) => r.current_info().await.map(|i| i.model_id),
                    None => None,
                };
                if running.as_deref() != Some(m.as_str()) && current.as_deref() != Some(m.as_str()) {
                    let (Some(rt), Some(layout)) = (&self.runtime, &self.layout) else {
                        return Err("本機模型請在 PaperComposer 切換".into());
                    };
                    let model = self.catalog.get(&m).filter(|x| x.is_brain()).ok_or(format!("模型目錄裡沒有本機模型「{m}」"))?;
                    if model.uncensored && !self.include_uncensored {
                        return Err("這個工具不使用移除拒答的模型".into());
                    }
                    if let Some(why) = layout.missing(model) {
                        return Err(why);
                    }
                    start = Some((rt.clone(), layout.clone(), model.clone()));
                }
            }
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
                    if self.catalog.cloud_model(&m).is_none() {
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
        // overrides
        let mut overrides = Vec::new();
        if let Some(o) = obj.get("overrides").filter(|v| !v.is_null()) {
            for (task, mode) in o.as_object().ok_or("overrides 要是物件")? {
                if !self.tasks.task.contains_key(task) {
                    return Err(format!("沒有這個任務「{task}」"));
                }
                let mode = if mode.is_null() { "auto".to_string() } else { str_of(mode, "走向")? };
                if lore_llm::RouteMode::parse(&mode).is_none() {
                    return Err(format!("不認得的走向「{mode}」"));
                }
                overrides.push((task.clone(), mode));
            }
        }
        let io = |e: std::io::Error| format!("無法儲存設定：{e}");
        if let Some(n) = idle {
            set_idle_minutes(&self.home, n).map_err(io)?;
        }
        if let Some(rt) = release {
            rt.release().await;
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
        if let Some((rt, layout, model)) = start {
            // the app's phase hook publishes the server in [engine] once it is ready
            tokio::spawn(async move {
                if let Err(e) = rt.ensure_in(&layout, &model).await {
                    eprintln!("lore-engines: {e}");
                }
            });
            tokio::task::yield_now().await;
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
    EngineSettings::from_strings(s.claude.via.as_deref(), s.claude.model.as_deref(), &s.route, &s.overrides)
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
    let e = Arc::new(e);
    let (g, p, o, q) = (e.clone(), e.clone(), e.clone(), e);
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
                    match (e.opener)(&url) {
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

#[cfg(test)]
mod tests;
