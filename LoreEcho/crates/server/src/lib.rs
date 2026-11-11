//! le-server: LoreEcho's local server (127.0.0.1 only). Serves `web/`, the shared theme and
//! shell, the shared 模型 settings (lore-engines) and the API below. Handlers are plain async
//! fns over [`Shared`]; the desktop app runs this server in-process and listens to
//! [`DesktopEvent`]s (window size, global shortcut) through a [`DesktopHook`].
//!
//! | method | path | what |
//! |---|---|---|
//! | GET | /api/health | `{ ok, app, character, desktop }` |
//! | GET | /theme.css, /theme.js, /prefs.css | the shared theme (lore-home), the author's fonts |
//! | GET/PUT | /api/theme | the theme every app shows (`private/shared/ui.toml`) |
//! | GET | /icons.js, /base.css, /shell.js | the shared shell (lore-shell) |
//! | GET/PUT | /api/engines, GET /api/links/ports, POST /api/links/open | lore-engines (the 模型 tab) |
//! | GET/PUT | /api/window | `{ x?, y?, expanded }`: the bubble's place, panel open or not (`state.toml`) |
//! | GET/PUT | /api/settings | `{ character?, shortcut }` (`settings.toml`) |
//! | GET/POST | /api/characters | GET `{ characters: [{ id, name, voice, language, avatars, full, figure }], current, states, chains }` (`full` = states with a full-body picture; `chains` = fallback order per state); POST `{ name }` = a new character, a copy of the example |
//! | POST | /api/characters/import | a zip (body) → a new character; GET /api/characters/{id}/export = that zip |
//! | GET/PUT | /api/characters/{id} | `{ id, name, persona, voice, voice_rate, language, theme, profile, background_limit }` / `{ name?, persona?, voice?, voice_rate?, theme_loop?, theme_volume?, profile? }` |
//! | GET/PUT/DELETE | /api/characters/{id}/theme | the theme song (GET with byte ranges; PUT = mp3 m4a aac wav ogg opus flac, ≤ 30 MB) |
//! | GET/PUT/DELETE | /api/characters/{id}/{avatar\|full}/{state} | the picture (GET follows the state's fallback chain, `full` then the portrait); PUT = the picture bytes (png, jpg, webp, svg, ≤ 10 MB); DELETE = empty the slot |
//! | POST | /api/chat | `{ messages, character?, engine?, kb?, speak? }` → the reply as server-sent events ([`chat`]) |
//! | POST | /api/kb/recall | `{ id, outcome }` → a review outcome through KnowledgeBase's API while it runs ([`kbctx`]) |
//! | POST | /api/actions/{note,cards}/… | 功能列表: drafts (tasks `le-note`, `le-cards`) and the confirmed saves to PaperComposer / KnowledgeBase ([`actions`]) |
//! | … | /api/voice… | listening and speaking ([`voice`]) |

pub mod actions;
pub mod api;
pub mod characters;
pub mod chat;
pub mod config;
pub mod kbctx;
pub mod llm;
pub mod store;
pub mod voice;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

pub use config::Config;

/// What the page asked the desktop window to do.
#[derive(Debug, Clone, PartialEq)]
pub enum DesktopEvent {
    /// Open the panel (true) or go back to the bubble (false).
    Expanded(bool),
    /// The page dragged the open window's edge: the new size (already inside the limits).
    Size(f64, f64),
    /// The global shortcut changed ("" = none).
    Shortcut(String),
}

pub type DesktopHook = Arc<dyn Fn(DesktopEvent) + Send + Sync>;

pub struct AppState {
    pub home: lore_home::Home,
    /// `private/LoreEcho` (`None`: no LoreForge folder, nothing is kept).
    pub dir: Option<PathBuf>,
    pub web: PathBuf,
    pub llm: llm::Llm,
    /// The desktop app, when there is one.
    pub hook: Option<DesktopHook>,
    /// The author's KnowledgeBase, read-only ([`kbctx`]).
    pub kb: kbctx::Kb,
    /// PaperComposer, for 整理成筆記 / 記下靈感 ([`actions`]).
    pub pc: actions::Pc,
    /// Listening and speaking (lore-voice; unavailable in tests unless a fake is given).
    pub voice: lore_voice::Voice,
    /// One writer of the two settings files at a time.
    pub(crate) write: Mutex<()>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    /// `home`'s `private/LoreEcho`, seeded from `private-example/LoreEcho` the first time.
    pub fn new(home: lore_home::Home, web: PathBuf, llm: llm::Llm, hook: Option<DesktopHook>) -> Shared {
        AppState::with_voice(home, web, llm, hook, lore_voice::Voice::unavailable("這裡沒有語音"))
    }

    /// [`AppState::new`] with a voice (the system's, or a fake in tests).
    pub fn with_voice(home: lore_home::Home, web: PathBuf, llm: llm::Llm, hook: Option<DesktopHook>, voice: lore_voice::Voice) -> Shared {
        let dir = match home.seed_app(config::APP) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("le-server: private/{}: {e}", config::APP);
                None
            }
        };
        Arc::new(AppState { home, dir, web, llm, hook, kb: kbctx::Kb::default(), pc: actions::Pc::default(), voice, write: Mutex::new(()) })
    }

    /// The real engines (`private/LoreEcho/tasks.toml` may override the task's model) and the
    /// system's voice.
    pub fn system(home: lore_home::Home, web: PathBuf, hook: Option<DesktopHook>) -> Shared {
        let user_tasks = home.app_dir(config::APP).map(|d| d.join("tasks.toml"));
        let llm = llm::Llm::new(Some(home.clone()), user_tasks.as_deref());
        let s = AppState::with_voice(home, web, llm, hook, lore_voice::Voice::system());
        s.kb.use_system();
        s.pc.use_system();
        s
    }

    pub(crate) fn tell(&self, e: DesktopEvent) {
        if let Some(h) = &self.hook {
            h(e);
        }
    }
}

/// Origins allowed to call the API: this machine's pages only.
pub fn allowed_origin(origin: &HeaderValue) -> bool {
    let Ok(o) = origin.to_str() else { return false };
    if o == "null" || o.starts_with("file://") || o == "tauri://localhost" || o.starts_with("deck://") {
        return true;
    }
    let local_host = |rest: &str| {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        host == "127.0.0.1" || host == "localhost" || host == "[::1]" || host == "tauri.localhost"
    };
    o.strip_prefix("http://").is_some_and(local_host) || o.strip_prefix("https://").is_some_and(local_host)
}

/// The settings behind the 模型 tab: the shared engine file, with this app's tasks.
fn engines(shared: &Shared) -> lore_engines::Engines {
    lore_engines::Engines::new(shared.home.clone(), config::APP, llm::TASKS)
}

pub fn router(shared: Shared) -> Router {
    let mut e = engines(&shared);
    if let Some(rt) = shared.llm.local_rt.clone() {
        e = e.shared(rt); // 本機 status, 釋放, and this app's share of the idle checks
    }
    router_with(shared, e)
}

/// Tests: never the keychain, the `claude` CLI, the network, the OS opener or the process routing.
pub fn router_isolated(shared: Shared) -> Router {
    let mut e = engines(&shared).isolated();
    e.install_routing = false;
    router_with(shared, e)
}

fn router_with(shared: Shared, engines: lore_engines::Engines) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|o, _| allowed_origin(o)))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE]);
    // `no-cache`: the desktop WebView revalidates and picks up new web/ files.
    let web = Router::new()
        .fallback_service(ServeDir::new(&shared.web).append_index_html_on_directories(true))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")));
    let mut kit = Router::new();
    for (path, mime, body) in lore_shell::FILES {
        kit = kit.route(path, axum::routing::get(move || async move { ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body) }));
    }
    api::routes().merge(voice::routes()).merge(actions::routes()).route("/api/kb/recall", axum::routing::post(kbctx::recall_post)).with_state(shared).merge(lore_engines::router(engines)).merge(kit).fallback_service(web).layer(cors)
}

/// Bind 127.0.0.1:`config.port` (0 = any free port); the address, the state and the server.
pub async fn bind(config: Config, hook: Option<DesktopHook>) -> std::io::Result<(SocketAddr, Shared, impl std::future::Future<Output = std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await?;
    let addr = listener.local_addr()?;
    let shared = AppState::system(config.home.clone(), config.web.clone(), hook);
    let app = router(shared.clone());
    Ok((addr, shared, async move { axum::serve(listener, app).await }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        for ok in ["null", "http://127.0.0.1:7860", "http://localhost:7979", "tauri://localhost"] {
            assert!(allowed_origin(&HeaderValue::from_static(ok)), "{ok}");
        }
        for bad in ["https://example.com", "http://127.0.0.1.evil.com"] {
            assert!(!allowed_origin(&HeaderValue::from_static(bad)), "{bad}");
        }
    }
}
