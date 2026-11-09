//! kb-server: axum on 127.0.0.1:7700, the only writer of the knowledge base.
//! Serves `web/` and the JSON API (design §4.7). Other local apps (PaperComposer,
//! SlideComposer, JELearner, NeonDeck's WKWebView with a `file://` / `null` origin)
//! call it over CORS. NeonDeck serves its desktop from a `deck://` scheme, so that origin is
//! allowed too. `reminders` + `digest`: recall notifications and the daily e-mail (§4.5).

pub mod api;
pub mod config;
pub mod digest;
pub mod reminders;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use kb_core::{KbOptions, KnowledgeBase};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

pub use config::Config;

pub struct AppState {
    pub kb: Mutex<KnowledgeBase>,
    pub web: PathBuf,
    pub rem: reminders::Reminders,
    /// LoreForge private folders (read-only here: `/prefs.css` from private/shared/fonts.toml).
    pub home: lore_home::Home,
}

impl AppState {
    pub fn new(kb: KnowledgeBase, web: PathBuf, rem: reminders::Reminders) -> Shared {
        let home = lore_home::Home::discover(&config::repo_dir());
        Arc::new(AppState { kb: Mutex::new(kb), web, rem, home })
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

pub fn router(shared: Shared) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|o, _| allowed_origin(o)))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE]);
    let web = ServeDir::new(&shared.web).append_index_html_on_directories(true);
    api::routes().with_state(shared).fallback_service(web).layer(cors)
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
