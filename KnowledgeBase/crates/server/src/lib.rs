//! kb-server: axum on 127.0.0.1:7700, the only writer of the knowledge base.
//! Serves `web/` and the JSON API (design §4.7). Other local apps (PaperComposer,
//! SlideComposer, JELearner, NeonDeck's WKWebView with a `file://` / `null` origin)
//! call it over CORS. NeonDeck serves its desktop from a `deck://` scheme, so that origin is
//! allowed too. `reminders` + `digest`: recall notifications and the daily e-mail (§4.5).

pub mod api;
pub mod config;
pub mod digest;
pub mod llm;
pub mod notation;
pub mod reminders;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use kb_core::{KbOptions, KnowledgeBase};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

pub use config::Config;

pub struct AppState {
    pub kb: Mutex<KnowledgeBase>,
    pub web: PathBuf,
    pub rem: reminders::Reminders,
    /// LoreForge private folders: `/prefs.css` from private/shared/fonts.toml, and the
    /// theme all apps share (private/shared/ui.toml, the only thing written there).
    pub home: lore_home::Home,
    /// Model calls (card drafts).
    pub llm: llm::Llm,
}

impl AppState {
    pub fn new(kb: KnowledgeBase, web: PathBuf, rem: reminders::Reminders) -> Shared {
        Self::with_home(kb, web, rem, lore_home::Home::discover(&config::repo_dir()))
    }

    /// With the LoreForge folder named (tests use a temporary one).
    pub fn with_home(kb: KnowledgeBase, web: PathBuf, rem: reminders::Reminders, home: lore_home::Home) -> Shared {
        let mut llm = llm::Llm::new(reminders::app_dir().as_deref());
        llm.share_engine(home.clone());
        Arc::new(AppState { kb: Mutex::new(kb), web, rem, home, llm })
    }

    /// Like [`AppState::new`] with the given model access (tests use mock engines).
    pub fn with_llm(kb: KnowledgeBase, web: PathBuf, rem: reminders::Reminders, llm: llm::Llm) -> Shared {
        let home = lore_home::Home::discover(&config::repo_dir());
        Arc::new(AppState { kb: Mutex::new(kb), web, rem, home, llm })
    }
}

pub type Shared = Arc<AppState>;

/// Open the knowledge base described by `config`.
pub fn open(config: &Config, opts: KbOptions) -> Result<KnowledgeBase, String> {
    let mut spaces = lore_store::Spaces::new();
    for root in &config.spaces {
        spaces.mount(root).map_err(|e| format!("{}: {e}", root.display()))?;
    }
    if spaces.is_empty() {
        return Err("no space mounted (set KB_SPACES or --space)".into());
    }
    KnowledgeBase::open(spaces, &config.member, KbOptions { intervals: config.intervals.clone(), ..opts }).map_err(|e| e.to_string())
}

/// Origins allowed to call the API: this machine's apps only.
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
    lore_engines::Engines::new(shared.home.clone(), "KnowledgeBase", include_str!("../tasks.toml"))
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
    // `no-cache` = revalidate every time (a 304 when unchanged), so the desktop WebView picks
    // up new web/ files after an update instead of showing its cached copy.
    let web = Router::new()
        .fallback_service(ServeDir::new(&shared.web).append_index_html_on_directories(true))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache")));
    // The shared kit (icons.js, base.css, shell.js): compiled in from commons/ui.
    let mut kit = Router::new();
    for (path, mime, body) in lore_shell::FILES {
        kit = kit.route(path, axum::routing::get(move || async move { ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body) }));
    }
    api::routes().merge(notation::routes()).with_state(shared).merge(lore_engines::router(engines)).merge(kit).fallback_service(web).layer(cors)
}

/// Bind 127.0.0.1:`port` (0 = any free port), start the reminder scheduler and return the
/// server future.
pub async fn bind(config: Config, kb: KnowledgeBase) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await?;
    let addr = listener.local_addr()?;
    let rem = reminders::Reminders::system(config.app_dir.clone(), format!("http://127.0.0.1:{}", addr.port()));
    let shared = AppState::new(kb, config.web.clone(), rem);
    reminders::spawn_scheduler(shared.clone());
    let app = router(shared);
    Ok((addr, async move { axum::serve(listener, app).await }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        for ok in ["null", "file://", "deck://desktop", "http://127.0.0.1:7878", "http://localhost:7979", "tauri://localhost", "https://tauri.localhost"] {
            assert!(allowed_origin(&HeaderValue::from_static(ok)), "{ok}");
        }
        for bad in ["https://example.com", "http://127.0.0.1.evil.com", "http://localhost.evil.com:80"] {
            assert!(!allowed_origin(&HeaderValue::from_static(bad)), "{bad}");
        }
    }
}
