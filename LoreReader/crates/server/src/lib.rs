//! lr-server: axum on 127.0.0.1:7890. Serves `web/` and the JSON API. Handlers are plain
//! async fns over `Shared` so they can become Tauri commands later.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /open?sha=&page= | open a paper at a page (KnowledgeBase links); "not in this machine's library" otherwise |
//! | GET | /api/info | library, exports, workspace, KB url, engines, desktop |
//! | GET | /api/papers | papers (paper.json) + whether the PDF is in this library |
//! | POST | /api/papers/import?name= | body = PDF bytes, or JSON `{path}` → copied into the library |
//! | GET/PUT | /api/papers/{sha} | meta / `{title}` |
//! | GET | /api/papers/{sha}/pdf | the PDF |
//! | GET/POST | /api/papers/{sha}/annotations | list / add `{page, kind (highlight / note / text), color, quote, rects, note}` |
//! | PUT/DELETE | /api/papers/{sha}/annotations/{id} | `{color?, note?, rects?}` / remove |
//! | GET/POST | /api/papers/{sha}/quotes | citable sentences / add `{text, page, note?}` |
//! | DELETE | /api/papers/{sha}/quotes/{id} | |
//! | GET | /api/quotes | citable sentences of every paper (`sha256`, `paper_title`, `doi`) |
//! | GET/POST/DELETE | /api/papers/{sha}/chat | conversation / `{text, attachments, paragraph?, chip?}` → reply (task `lr-chat`; a paragraph brings its context) / clear |
//! | POST | /api/compose | `{pages: [{sha, page}], title?}` → new PDF in the library + exports |
//! | POST | /api/translate | `{text, context?}` (task `lr-translate`) |
//! | POST | /api/papers/{sha}/read | the page's pdf.js text (all pages, items with fonts and positions) → noun list |
//! | GET | /api/papers/{sha}/nouns | the noun list (inventory × KB × ignore list × cached categories) |
//! | POST | /api/papers/{sha}/nouns/classify | `{again?}` → one batched `lr-classify-terms` call |
//! | GET/POST | /api/papers/{sha}/symbols | the symbol table / the author adds `{latex, meaning?…}` |
//! | PUT | /api/papers/{sha}/symbols/{id} | `{latex?, meaning?, meaning_en?, grade?, note?, by?, evidence?, page?}` (author edits win) |
//! | POST | /api/papers/{sha}/symbols/rebuild | detect again (after a glyph-table change) |
//! | GET | /api/papers/{sha}/paragraphs | paragraphs with chips, cached analyses, 懂了 flags, run status |
//! | POST | /api/papers/{sha}/paragraphs/run | `{again?}` → restart the background `lr-paragraph` run |
//! | POST | /api/papers/{sha}/paragraphs/{pid}/understood | `{on}` → 懂了 (reading.json; separate from KB state) |
//! | GET/PUT | /api/glyphs | the author's glyph → LaTeX override (`glyphs.toml`) |
//! | POST/DELETE | /api/ignore | `{term}` → add to / remove from `ignore-words.txt` (「這不是術語」) |
//! | POST | /api/papers/{sha}/mine | `{on, pages?}` → 「我的論文」 flag (paper.json) and the rebuilt my-vocab.json |
//! | GET/PUT | /api/reader | `{field}`: the reader's field and level for term classification (`reading.toml`) |
//! | GET/PUT | /api/models | per-feature engine (the engine chips): `{task, model: auto\|local\|cloud\|haiku\|sonnet\|opus}` |
//! | POST | /api/papers/{sha}/symbols/{id}/global | 加入全域: the row into the global 對照表 (`shared/notation.json`) |
//! | GET | /api/notation/global | the global 對照表 (`shared/notation.json`; empty when missing) |
//! | GET | /icons.js, /base.css, /shell.js, /makedeck.js, /makebook.js | the shared shell (`lore_shell::FILES`); /notation-panel.js the shared 對照表 component |
//! | GET | /api/kb/status | is the KB server up, member, spaces |
//! | POST | /api/kb/open | start the KnowledgeBase app (macOS, local KB only; never when isolated) |
//! | POST | /api/kb/cards | `{sha, page, title, summary, initial_state, kind?, context?, space?}` → KB `POST /api/nodes` |
//! | POST | /api/kb/draft | `{term, context, sha?, page?}` → KB `POST /api/draft` (with `page` and the paper's title as `source`) |
//! | GET | /api/kb/nodes/{id} | a card (summary for the term popover) |
//! | GET | /api/papers/{sha}/export | notes export state: `dirty`, `last` {path, name, at, exists}, default folder/name |
//! | POST | /api/papers/{sha}/export | `{path}` / `{last: true}` (overwrite the last export) / `{choose: true}` (desktop save panel) → PDF with the notes written to that file |
//! | POST | /api/papers/{sha}/export.pdf | the PDF with the notes as a download (browser) |
//! | POST | /api/papers/{sha}/export/decline | "不儲存" on close: not dirty until the notes change |
//! | GET | /api/dirty | every paper whose notes changed since their last export (not the close path: the shell and the page ask about the open paper only, via `…/export`) |
//! | POST | /api/print | `{sha}` → `{native}`: the desktop app opens its print dialog |
//! | POST | /api/print/ready?id= | print.html is rendered (desktop: show the dialog) |

pub mod kb;
pub mod llm;
pub mod reading;

use std::net::SocketAddr;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lore_llm::{LlmRequest, Message};
use lr_core::library::valid_sha;
use lr_core::annotate;
use lr_core::library::git_work_tree;
use lr_core::papers::{Annotation, AnnotationPatch, Attachment, ChatMessage, Derived, PaperMeta, hash_annotations};
use lr_core::pdfops::{self, Pick};
use lr_core::{Config, Library, Papers};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::services::ServeDir;

/// The desktop app's print bridge: open print.html in a window, then show the
/// native print dialog once the page says it is rendered.
pub trait Printer: Send + Sync {
    fn open(&self, url: &str, id: &str) -> Result<(), String>;
    fn ready(&self, id: &str) -> Result<(), String>;
}

/// The desktop app's save panel: the path the author chose, `None` when cancelled.
/// Called from a blocking thread.
pub trait Saver: Send + Sync {
    fn choose(&self, dir: &std::path::Path, name: &str) -> Result<Option<PathBuf>, String>;
}

pub struct AppState {
    pub config: Config,
    /// The library, or why it cannot be used (e.g. inside a git work tree).
    pub library: Result<Library, String>,
    pub papers: Papers,
    /// `private/LoreReader` (or a temp dir in tests).
    pub workspace: PathBuf,
    pub web: PathBuf,
    pub home: lore_home::Home,
    /// `private/shared` (cross-app personal data: profile.toml, my-vocab.json); a folder
    /// inside the workspace in isolated tests.
    pub shared: PathBuf,
    pub kb: kb::Kb,
    pub llm: llm::Llm,
    pub port: Mutex<u16>,
    /// Serialises metadata writes (read-modify-write of the JSON files).
    pub(crate) write: Mutex<()>,
    /// The text of each opened paper as the page sent it (memory only).
    pub texts: Mutex<HashMap<String, Arc<lr_core::text::PaperText>>>,
    /// Its paragraphs with chips (memory only; analyses are in paragraphs.json).
    pub paras: Mutex<HashMap<String, Arc<Vec<lr_core::paragraphs::Paragraph>>>>,
    /// The background paragraph analysis of each paper.
    pub runs: Mutex<HashMap<String, reading::RunStatus>>,
    pub printer: Mutex<Option<Arc<dyn Printer>>>,
    pub saver: Mutex<Option<Arc<dyn Saver>>>,
    /// Tests and the isolated example: never start other apps.
    pub isolated: bool,
}

pub type Shared = Arc<AppState>;

pub struct ApiError(StatusCode, Value);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}
impl From<lr_core::Error> for ApiError {
    fn from(e: lr_core::Error) -> Self {
        let code = match e {
            lr_core::Error::NotFound(_) => StatusCode::NOT_FOUND,
            lr_core::Error::Invalid(_) | lr_core::Error::Pdf(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(code, json!({ "error": e.to_string() }))
    }
}
impl From<kb::KbError> for ApiError {
    fn from(e: kb::KbError) -> Self {
        ApiError(if e.down { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::BAD_GATEWAY }, json!({ "error": e.message, "kb_down": e.down }))
    }
}
fn bad(m: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, json!({ "error": m.into() }))
}

type R = Result<Json<Value>, ApiError>;

/// This app's folder (`LoreForge/LoreReader`).
pub fn repo_dir() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    p.canonicalize().unwrap_or(p)
}

/// `private/LoreReader` (created from `private-example/LoreReader` the first time), or
/// `LOREREADER_WORKSPACE`.
pub fn default_workspace() -> PathBuf {
    if let Some(w) = std::env::var_os("LOREREADER_WORKSPACE").filter(|v| !v.is_empty()) {
        return PathBuf::from(w);
    }
    let repo = repo_dir();
    match lore_home::Home::discover(&repo).seed_app("LoreReader") {
        Ok(Some(d)) => d,
        Ok(None) => repo.join("../private/LoreReader"),
        Err(e) => {
            eprintln!("lr-server: private/LoreReader: {e}");
            repo.join("../private/LoreReader")
        }
    }
}

/// State for `config` with metadata in `workspace`. `isolated`: no keychain, no
/// environment key, no `claude` CLI (tests).
pub fn state(config: Config, workspace: PathBuf, web: PathBuf, isolated: bool) -> Shared {
    let library = Library::open(&config.library, &config.exports).map_err(|e| e.to_string());
    if let Err(e) = &library {
        eprintln!("lr-server: {e}");
    }
    let mut llm = llm::Llm::new(&config.engine, config.claude_model.clone(), config.app_dir.as_ref().map(|d| d.join("tasks.toml")).as_deref());
    if isolated {
        llm.isolate();
    }
    // isolated: the shared folder (and so the shared theme) lives inside the test's workspace
    let mut home = lore_home::Home::discover(&repo_dir());
    if isolated {
        home.private = Some(workspace.clone());
    }
    let shared = home.shared_dir().unwrap_or_else(|| workspace.join("../shared"));
    llm.share_engine(home.clone());
    Arc::new(AppState {
        kb: kb::Kb::new(&config.kb_url),
        llm,
        config,
        library,
        papers: Papers::new(workspace.join("papers")),
        workspace,
        web,
        shared,
        home,
        port: Mutex::new(0),
        write: Mutex::new(()),
        texts: Mutex::new(HashMap::new()),
        paras: Mutex::new(HashMap::new()),
        runs: Mutex::new(HashMap::new()),
        printer: Mutex::new(None),
        saver: Mutex::new(None),
        isolated,
    })
}

pub fn routes() -> Router<Shared> {
    shell_files(Router::new())
        .route("/prefs.css", get(prefs_css))
        .route("/prefs.js", get(prefs_js))
        .route("/theme.css", get(theme_css))
        .route("/theme.js", get(theme_js))
        .route("/notation-panel.js", get(notation_panel_js))
        .route("/api/theme", get(theme_get).put(theme_put))
        .route("/open", get(open))
        .route("/api/info", get(info))
        .route("/api/papers", get(list_papers))
        .route("/api/papers/import", post(import).layer(DefaultBodyLimit::max(400 * 1024 * 1024)))
        .route("/api/papers/{sha}", get(paper).put(rename))
        .route("/api/papers/{sha}/view", put(set_view))
        .route("/api/view", get(view_get).put(view_put))
        .route("/api/papers/{sha}/pdf", get(pdf))
        .route("/api/papers/{sha}/annotations", get(annotations).post(add_annotation))
        .route("/api/papers/{sha}/annotations/{id}", put(update_annotation).delete(delete_annotation))
        .route("/api/papers/{sha}/quotes", get(quotes).post(add_quote))
        .route("/api/papers/{sha}/quotes/{id}", axum::routing::delete(delete_quote))
        .route("/api/quotes", get(all_quotes))
        .route("/api/papers/{sha}/chat", get(chat).post(send_chat).delete(clear_chat))
        .route("/api/compose", post(compose))
        .route("/api/translate", post(translate))
        .route("/api/papers/{sha}/read", post(reading::read_paper).layer(DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/papers/{sha}/nouns", get(reading::nouns_get))
        .route("/api/papers/{sha}/nouns/classify", post(reading::classify))
        .route("/api/ignore", post(reading::ignore_add).delete(reading::ignore_remove))
        .route("/api/papers/{sha}/symbols", get(reading::symbols_get).post(reading::symbol_add))
        .route("/api/papers/{sha}/symbols/rebuild", post(reading::symbols_rebuild))
        .route("/api/papers/{sha}/symbols/{id}", put(reading::symbol_put))
        .route("/api/papers/{sha}/symbols/{id}/global", post(reading::symbol_to_global))
        .route("/api/notation/global", get(reading::global_notation_get))
        .route("/api/glyphs", get(reading::glyphs_get).put(reading::glyphs_put))
        .route("/api/papers/{sha}/paragraphs", get(reading::paragraphs_get))
        .route("/api/papers/{sha}/paragraphs/run", post(reading::paragraphs_run))
        .route("/api/papers/{sha}/paragraphs/{pid}/understood", post(reading::understood))
        .route("/api/models", get(reading::models_get).put(reading::models_put))
        .route("/api/reader", get(reading::reader_get).put(reading::reader_put))
        .route("/api/papers/{sha}/mine", post(reading::mine).layer(DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/kb/status", get(kb_status))
        .route("/api/kb/open", post(kb_open))
        .route("/api/kb/cards", post(kb_card))
        .route("/api/kb/draft", post(kb_draft))
        .route("/api/kb/nodes/{id}", get(kb_node))
        .route("/api/papers/{sha}/export", get(export_state).post(export))
        .route("/api/papers/{sha}/export.pdf", post(export_download))
        .route("/api/papers/{sha}/export/decline", post(export_decline))
        .route("/api/dirty", get(dirty))
        .route("/api/print", post(print))
        .route("/api/print/ready", post(print_ready))
}

/// The shared shell (commons/ui icons.js, base.css, shell.js) through lore-shell.
fn shell_files(mut r: Router<Shared>) -> Router<Shared> {
    for (path, mime, body) in lore_shell::FILES {
        r = r.route(path, get(move || async move { ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body) }));
    }
    r
}

/// The shared notation-panel Web Component (commons/ui), compiled in (the 對照表).
async fn notation_panel_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], include_str!("../../../../commons/ui/notation-panel/notation-panel.js"))
}

pub fn router(shared: Shared) -> Router {
    // static files: always revalidate (no build step; the desktop WebView keeps old copies otherwise)
    let web = tower::ServiceBuilder::new()
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-cache")))
        .service(ServeDir::new(&shared.web).append_index_html_on_directories(true));
    routes().merge(lore_engines::router(engines(&shared))).with_state(shared).fallback_service(web)
}

/// The shell's 模型 tab (`GET/PUT /api/engines`) and `POST /api/links/open`, shared by every
/// app (lore-engines). Isolated: no keychain, CLI or opener, and the process routing stays
/// the tasks' own order.
fn engines(s: &AppState) -> lore_engines::Engines {
    let e = lore_engines::Engines::new(s.home.clone(), "LoreReader", llm::TASKS)
        .external(s.config.engine.url.clone(), s.config.engine.model.clone())
        .claude_model(s.config.claude_model.clone())
        .cli(s.llm.cli.clone(), s.llm.cc.clone());
    if !s.isolated {
        return match &s.llm.local_rt {
            Some(rt) => e.shared(rt.clone()), // 本機 status, 釋放, this app's share of the idle checks
            None => e,
        };
    }
    let mut e = e.isolated();
    e.install_routing = false;
    e
}

pub async fn bind(shared: Shared, port: u16) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    *shared.port.lock().unwrap() = addr.port();
    let app = router(shared);
    Ok((addr, async move { axum::serve(listener, app).await }))
}

fn lib(s: &AppState) -> Result<&Library, ApiError> {
    s.library.as_ref().map_err(|e| ApiError(StatusCode::CONFLICT, json!({ "error": e, "library_error": true })))
}

fn sha_ok(sha: &str) -> Result<(), ApiError> {
    if valid_sha(sha) { Ok(()) } else { Err(bad("不是 sha256")) }
}

// ── theme / fonts ────────────────────────────────────────────────────────

/// Interface fonts from `private/shared/fonts.toml` (CSS variables; linked after styles.css).
async fn prefs_css(State(s): State<Shared>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], s.home.prefs().fonts.css())
}

/// `window.LORE_PREFS`: the shared theme id (null = none chosen yet).
async fn prefs_js(State(s): State<Shared>) -> impl IntoResponse {
    let body = format!("window.LORE_PREFS = {};\n", json!({ "theme": s.home.theme() }));
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], body)
}

/// The LoreForge themes (commons/ui/theme.css, the same file in every app), compiled in.
async fn theme_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::CSS)
}

/// commons/ui/theme.js with the shared choice in front (first paint has the theme).
async fn theme_js(State(s): State<Shared>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::script(&s.home))
}

/// The theme every LoreForge app shows (`private/shared/ui.toml`).
async fn theme_get(State(s): State<Shared>) -> Json<Value> {
    Json(json!({ "theme": s.home.theme() }))
}

async fn theme_put(State(s): State<Shared>, Json(b): Json<Value>) -> Result<Json<Value>, ApiError> {
    let t = s.home.set_theme(b["theme"].as_str().unwrap_or_default()).map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "theme": t })))
}

// ── open from KnowledgeBase ──────────────────────────────────────────────

#[derive(Deserialize)]
struct OpenQ {
    sha: Option<String>,
    page: Option<u32>,
}

async fn open(State(s): State<Shared>, Query(q): Query<OpenQ>) -> Response {
    let sha = q.sha.unwrap_or_default().to_ascii_lowercase();
    let page = q.page.unwrap_or(1).max(1);
    if s.library.as_ref().is_ok_and(|l| l.has(&sha)) {
        return Redirect::to(&format!("/?paper={sha}&page={page}")).into_response();
    }
    // The original is gone but a LoreReader export of it (same pages) was opened since.
    if valid_sha(&sha)
        && let Some(m) = s.papers.list().unwrap_or_default().into_iter().find(|m| m.source_sha.as_deref() == Some(sha.as_str()) && s.library.as_ref().is_ok_and(|l| l.has(&m.sha256)))
    {
        return Redirect::to(&format!("/?paper={}&page={page}", m.sha256)).into_response();
    }
    let known = valid_sha(&sha).then(|| s.papers.meta(&sha).ok().flatten()).flatten();
    let what = known.map(|m| format!("《{}》", html_escape(&m.title))).unwrap_or_else(|| "這份 PDF".into());
    let body = format!(
        "<!doctype html><html lang=\"zh-Hant\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>LoreReader</title>\
<link rel=\"stylesheet\" href=\"/theme.css\"><link rel=\"stylesheet\" href=\"/base.css\"><link rel=\"stylesheet\" href=\"/styles.css\"><script src=\"/theme.js\"></script><body class=\"notice-page\"><main class=\"notice\"><h1>不在這台電腦的書庫裡</h1>\
<p>{what}（sha256 <code>{}</code>，第 {page} 頁）不在這台電腦的 LoreReader 書庫中。</p>\
<p>在這台電腦用 LoreReader 開啟同一個 PDF 檔，它就會加入書庫，這個連結之後就能直接開到那一頁。</p>\
<p><a class=\"btn\" href=\"/\">回到書庫</a></p></main></body></html>",
        html_escape(&sha)
    );
    (StatusCode::NOT_FOUND, Html(body)).into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ── library / papers ─────────────────────────────────────────────────────

async fn info(State(s): State<Shared>) -> R {
    Ok(Json(json!({
        "library": s.config.library,
        "exports": s.config.exports,
        "library_error": s.library.as_ref().err(),
        "workspace": s.workspace,
        "kb_url": s.kb.base,
        "local_engine": s.llm.has_local(),
        "desktop": s.printer.lock().unwrap().is_some(),
        "save_dialog": s.saver.lock().unwrap().is_some(),
        "colors": lr_core::papers::COLORS,
    })))
}

fn paper_view(s: &AppState, m: &PaperMeta) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
    v["present"] = s.library.as_ref().is_ok_and(|l| l.has(&m.sha256)).into();
    // the mode the page opens in: the paper's own, else the global default
    let own = m.extra.get("view").and_then(|x| x.as_str()).filter(|x| lr_core::papers::VIEWS.contains(x));
    v["view_mode"] = own.map(str::to_string).unwrap_or_else(|| default_view(&s.workspace)).into();
    v
}

async fn list_papers(State(s): State<Shared>) -> R {
    let list: Vec<Value> = s.papers.list()?.iter().map(|m| paper_view(&s, m)).collect();
    Ok(Json(json!({ "papers": list })))
}

/// Add bytes to the library and register them (paper.json only the first time).
fn add_paper(s: &AppState, bytes: &[u8], original_name: &str, derived: Option<Derived>, title: Option<String>) -> Result<(PaperMeta, bool), ApiError> {
    let l = lib(s)?;
    let pages = pdfops::page_count(bytes).map_err(|e| bad(format!("讀不了這份 PDF：{e}")))?;
    let (sha, new) = l.add(bytes)?;
    let stem = original_name.trim_end_matches(".pdf").trim_end_matches(".PDF");
    // Title: given (a composed PDF), the PDF's own /Title unless it is a placeholder, else the
    // file name until the page sends its text and the headline of page 1 is used.
    let (title, title_source) = match (title, pdfops::info_title(bytes).filter(|t| !pdfops::is_junk_title(t, stem))) {
        (Some(t), _) => (t, "author"),
        (None, Some(t)) => (t, "pdf"),
        (None, None) => (if stem.is_empty() { "未命名".into() } else { stem.to_string() }, "file"),
    };
    let meta = PaperMeta {
        schema: lr_core::papers::PAPER_SCHEMA.into(),
        sha256: sha,
        title,
        original_name: original_name.to_string(),
        doi: pdfops::doi_of(bytes),
        pages,
        added_at: lr_core::now(),
        derived_from: derived,
        original_path: None,
        source_sha: None,
        extra: [("title_source".to_string(), Value::from(title_source))].into_iter().collect(),
    };
    let _g = s.write.lock().unwrap();
    Ok((s.papers.register(meta)?, new))
}

#[derive(Deserialize)]
struct ImportQ {
    name: Option<String>,
}

async fn import(State(s): State<Shared>, Query(q): Query<ImportQ>, headers: HeaderMap, body: Bytes) -> R {
    let is_json = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("application/json"));
    let (bytes, name, from) = if is_json {
        let v: Value = serde_json::from_slice(&body).map_err(|e| bad(e.to_string()))?;
        let path = PathBuf::from(v["path"].as_str().ok_or_else(|| bad("需要 path"))?.trim());
        let bytes = std::fs::read(&path).map_err(|e| bad(format!("{}：{e}", path.display())))?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        (bytes, name, Some(path).filter(|p| p.is_absolute()))
    } else {
        (body.to_vec(), q.name.unwrap_or_default(), None)
    };
    let name = std::path::Path::new(&name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "paper.pdf".into());
    // A PDF that LoreReader saved with notes: back to its paper, notes restored.
    let emb = annotate::read(&bytes).unwrap_or_default();
    if !emb.is_empty() {
        return import_export(&s, &bytes, &name, from.as_deref(), emb);
    }
    let (mut meta, new) = add_paper(&s, &bytes, &name, None, None)?;
    if let Some(p) = from {
        let _g = s.write.lock().unwrap();
        meta = s.papers.update_meta(&meta.sha256, |m| m.original_path = Some(p.to_string_lossy().to_string()))?;
    }
    Ok(Json(json!({ "paper": paper_view(&s, &meta), "new": new })))
}

/// Import a PDF that carries LoreReader's notes (`/LoreReaderSource` and/or `lr:` annotations).
///
/// Identity: the paper it was exported from when that PDF is in the library (KB links and
/// quotes keep their sha); else a paper already recorded with the same `source_sha`; else
/// the file **without** LoreReader's annotations (they are drawn from annotations.json, not
/// twice) becomes a new paper with `source_sha` = the marker. The notes are merged by id
/// and the file is remembered as that paper's last export (so "儲存" overwrites it).
fn import_export(s: &AppState, bytes: &[u8], name: &str, from: Option<&std::path::Path>, emb: annotate::Embedded) -> R {
    let l = lib(s)?;
    let own = lr_core::library::sha256(bytes);
    let src = emb.source.clone().filter(|x| *x != own);
    let found = src.as_ref().and_then(|src| {
        if l.has(src) {
            return Some(src.clone());
        }
        s.papers.list().ok()?.into_iter().find(|m| m.source_sha.as_deref() == Some(src.as_str()) && l.has(&m.sha256)).map(|m| m.sha256)
    });
    let plain_name = name.replace(" (筆記)", "").replace("（筆記）", "");
    let (meta, new) = match found {
        Some(sha) => match s.papers.meta(&sha)? {
            Some(m) => (m, false),
            None => add_paper(s, &l.read(&sha)?, &plain_name, None, None)?,
        },
        None => {
            let stripped = annotate::strip(bytes)?;
            let (m, new) = add_paper(s, &stripped, &plain_name, None, None)?;
            let _g = s.write.lock().unwrap();
            let m = s.papers.update_meta(&m.sha256, |x| {
                if x.source_sha.is_none() {
                    x.source_sha = src.clone();
                }
            })?;
            (m, new)
        }
    };
    let _g = s.write.lock().unwrap();
    let restored = s.papers.merge_annotations(&meta.sha256, emb.annotations.clone())?;
    let path = from.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_string());
    if path.is_some() || s.papers.export_state(&meta.sha256)?.is_none() {
        s.papers.set_exported(&meta.sha256, &hash_annotations(&emb.annotations), path.as_deref())?;
    }
    Ok(Json(json!({ "paper": paper_view(s, &meta), "new": new, "restored": restored, "from_export": true })))
}

async fn paper(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    let m = s.papers.meta(&sha)?.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, json!({ "error": "沒有這份論文的資料" })))?;
    Ok(Json(json!({ "paper": paper_view(&s, &m) })))
}

#[derive(Deserialize)]
struct RenameBody {
    title: String,
}

async fn rename(State(s): State<Shared>, Path(sha): Path<String>, Json(b): Json<RenameBody>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    let m = s.papers.set_title(&sha, &b.title)?;
    Ok(Json(json!({ "paper": paper_view(&s, &m) })))
}

#[derive(Deserialize)]
struct ViewBody {
    view: Option<String>,
}

/// Remember the reading mode of one paper (`null` = forget it, the default applies).
async fn set_view(State(s): State<Shared>, Path(sha): Path<String>, Json(b): Json<ViewBody>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    let m = s.papers.set_view(&sha, b.view.as_deref())?;
    drop(_g);
    Ok(Json(json!({ "paper": paper_view(&s, &m) })))
}

fn view_file(ws: &std::path::Path) -> PathBuf {
    ws.join("view.json")
}

fn load_view(ws: &std::path::Path) -> serde_json::Map<String, Value> {
    std::fs::read(view_file(ws)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// The reading mode for papers without their own (`private/LoreReader/view.json` `default`).
fn default_view(ws: &std::path::Path) -> String {
    load_view(ws).get("default").and_then(|v| v.as_str()).filter(|v| lr_core::papers::VIEWS.contains(v)).unwrap_or("scroll").to_string()
}

async fn view_get(State(s): State<Shared>) -> R {
    Ok(Json(json!({ "default": default_view(&s.workspace), "views": lr_core::papers::VIEWS })))
}

#[derive(Deserialize)]
struct ViewDefaultBody {
    default: String,
}

async fn view_put(State(s): State<Shared>, Json(b): Json<ViewDefaultBody>) -> R {
    if !lr_core::papers::VIEWS.contains(&b.default.as_str()) {
        return Err(bad(format!("閱讀方式只能是 {}", lr_core::papers::VIEWS.join("、"))));
    }
    let _g = s.write.lock().unwrap();
    let mut v = load_view(&s.workspace);
    v.insert("schema".into(), "lorereader.view/1".into());
    v.insert("default".into(), b.default.into());
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(v)).unwrap_or_default();
    bytes.push(b'\n');
    lr_core::write_atomic(&view_file(&s.workspace), &bytes).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    drop(_g);
    view_get(State(s)).await
}

async fn pdf(State(s): State<Shared>, Path(sha): Path<String>) -> Result<Response, ApiError> {
    sha_ok(&sha)?;
    let bytes = lib(&s)?.read(&sha)?;
    Ok(([(header::CONTENT_TYPE, "application/pdf"), (header::CACHE_CONTROL, "private, max-age=31536000, immutable")], bytes).into_response())
}

async fn annotations(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    Ok(Json(json!({ "annotations": s.papers.annotations(&sha)? })))
}

async fn add_annotation(State(s): State<Shared>, Path(sha): Path<String>, Json(a): Json<Annotation>) -> Result<(StatusCode, Json<Value>), ApiError> {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    let a = s.papers.add_annotation(&sha, a)?;
    Ok((StatusCode::CREATED, Json(json!({ "annotation": a }))))
}

async fn update_annotation(State(s): State<Shared>, Path((sha, id)): Path<(String, String)>, Json(p): Json<AnnotationPatch>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    Ok(Json(json!({ "annotation": s.papers.update_annotation(&sha, &id, p)? })))
}

async fn delete_annotation(State(s): State<Shared>, Path((sha, id)): Path<(String, String)>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    s.papers.delete_annotation(&sha, &id)?;
    Ok(Json(json!({ "deleted": id })))
}

async fn quotes(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    Ok(Json(json!({ "quotes": s.papers.quotes(&sha)? })))
}

#[derive(Deserialize)]
struct QuoteBody {
    text: String,
    page: u32,
    #[serde(default)]
    note: String,
}

async fn add_quote(State(s): State<Shared>, Path(sha): Path<String>, Json(b): Json<QuoteBody>) -> Result<(StatusCode, Json<Value>), ApiError> {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    let q = s.papers.add_quote(&sha, &b.text, b.page, &b.note)?;
    Ok((StatusCode::CREATED, Json(json!({ "quote": q }))))
}

async fn delete_quote(State(s): State<Shared>, Path((sha, id)): Path<(String, String)>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    s.papers.delete_quote(&sha, &id)?;
    Ok(Json(json!({ "deleted": id })))
}

async fn all_quotes(State(s): State<Shared>) -> R {
    Ok(Json(json!({ "schema": lr_core::papers::QUOTES_SCHEMA, "quotes": s.papers.all_quotes()? })))
}

// ── page operations ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PageRef {
    sha: String,
    page: u32,
}

#[derive(Deserialize)]
struct ComposeBody {
    pages: Vec<PageRef>,
    title: Option<String>,
    /// What the UI did (`delete`, `duplicate`, `reorder`, `insert`, `merge`, `edit`).
    op: Option<String>,
}

async fn compose(State(s): State<Shared>, Json(b): Json<ComposeBody>) -> R {
    let l = lib(&s)?;
    let mut shas: Vec<String> = Vec::new();
    for p in &b.pages {
        sha_ok(&p.sha)?;
        if !shas.contains(&p.sha) {
            shas.push(p.sha.clone());
        }
    }
    let sources: Vec<Vec<u8>> = shas.iter().map(|sha| l.read(sha)).collect::<Result<_, _>>()?;
    let refs: Vec<&[u8]> = sources.iter().map(Vec::as_slice).collect();
    let picks: Vec<Pick> = b.pages.iter().map(|p| Pick { src: shas.iter().position(|x| *x == p.sha).unwrap_or(0), page: p.page }).collect();
    let bytes = pdfops::compose(&refs, &picks)?;
    let first = s.papers.meta(&shas[0]).ok().flatten();
    let title = b.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| format!("{}（編輯）", first.as_ref().map(|m| m.title.as_str()).unwrap_or("PDF")));
    let path = l.export(&bytes, &title)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let derived = Derived { op: b.op.unwrap_or_else(|| "compose".into()), from: shas };
    let (meta, _) = add_paper(&s, &bytes, &name, Some(derived), Some(title))?;
    Ok(Json(json!({ "paper": paper_view(&s, &meta), "export": path })))
}

// ── save with notes ──────────────────────────────────────────────────────

/// Folder and file name the save panel starts with: next to the file it was opened from
/// (if known and still there), else ~/Documents; `<original name> (筆記).pdf`.
fn default_target(m: Option<&PaperMeta>) -> (PathBuf, String) {
    let dir = m
        .and_then(|m| m.original_path.as_deref())
        .and_then(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf))
        .filter(|d| d.is_dir())
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir);
    let stem = m
        .map(|m| {
            let n = m.original_name.trim();
            let n = n.strip_suffix(".pdf").or_else(|| n.strip_suffix(".PDF")).unwrap_or(n);
            if n.is_empty() || m.derived_from.is_some() { m.title.clone() } else { n.to_string() }
        })
        .unwrap_or_default();
    let stem = stem.replace(" (筆記)", "");
    (dir, format!("{} (筆記).pdf", lr_core::safe_stem(&stem)))
}

fn export_view(s: &AppState, sha: &str) -> Result<Value, ApiError> {
    let m = s.papers.meta(sha)?;
    let st = s.papers.export_state(sha)?;
    let (dir, name) = default_target(m.as_ref());
    let last = st.as_ref().and_then(|st| st.path.as_ref().map(|p| (p, st))).map(|(p, st)| {
        let pb = std::path::Path::new(p);
        json!({ "path": p, "name": pb.file_name().map(|n| n.to_string_lossy().to_string()), "at": st.at, "exists": pb.is_file() })
    });
    Ok(json!({
        "dirty": s.papers.is_dirty(sha)?,
        "count": s.papers.annotations(sha)?.len(),
        "last": last,
        "default_dir": dir,
        "default_name": name,
        "save_dialog": s.saver.lock().unwrap().is_some(),
    }))
}

async fn export_state(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    Ok(Json(export_view(&s, &sha)?))
}

/// The paper's PDF with its notes written in, and the annotations hash it captured.
fn build_export(s: &AppState, sha: &str) -> Result<(Vec<u8>, String, PaperMeta), ApiError> {
    let l = lib(s)?;
    let meta = s.papers.meta(sha)?.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, json!({ "error": "沒有這份論文的資料" })))?;
    let src = l.read(sha)?;
    let (list, hash) = {
        let _g = s.write.lock().unwrap();
        let list = s.papers.annotations(sha)?;
        let h = hash_annotations(&list);
        (list, h)
    };
    let root = meta.source_sha.clone().unwrap_or_else(|| sha.to_string());
    Ok((annotate::write(&src, &list, &root)?, hash, meta))
}

/// Write the export to `path` (atomically). Refused: inside the library or a git work
/// tree, the file the paper was opened from, or an existing file LoreReader did not save.
fn write_export(s: &AppState, sha: &str, path: PathBuf) -> Result<PathBuf, ApiError> {
    let l = lib(s)?;
    if !path.is_absolute() {
        return Err(bad("需要完整路徑"));
    }
    let path = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) { path } else { PathBuf::from(format!("{}.pdf", path.display())) };
    let file = path.file_name().ok_or_else(|| bad("需要檔名"))?.to_owned();
    let parent = path.parent().filter(|p| p.is_dir()).ok_or_else(|| bad(format!("資料夾不存在：{}", path.display())))?;
    let target = parent.canonicalize().map_err(|e| bad(e.to_string()))?.join(file);
    let real = target.canonicalize().unwrap_or_else(|_| target.clone());
    let lib_root = l.root.canonicalize().unwrap_or_else(|_| l.root.clone());
    if real.starts_with(&lib_root) {
        return Err(bad("不能存進書庫資料夾"));
    }
    if let Some(repo) = git_work_tree(&target) {
        return Err(bad(format!("{} 在 git 工作目錄 {} 裡；PDF 不能放進 repo", target.display(), repo.display())));
    }
    let meta = s.papers.meta(sha)?;
    let original = meta.as_ref().and_then(|m| m.original_path.as_deref()).map(|p| std::path::Path::new(p).canonicalize().unwrap_or_else(|_| PathBuf::from(p)));
    if original.as_ref() == Some(&real) {
        return Err(bad("不會覆寫原本的 PDF，請換一個檔名"));
    }
    if real.exists() {
        let ours = std::fs::read(&real).ok().and_then(|b| annotate::read(&b).ok()).is_some_and(|e| e.source.is_some());
        if !ours {
            return Err(bad("這個檔案不是 LoreReader 存的，不會覆寫；請換一個檔名"));
        }
    }
    let (bytes, hash, _) = build_export(s, sha)?;
    lr_core::write_atomic(&real, &bytes).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("{}：{e}", real.display()) })))?;
    let _g = s.write.lock().unwrap();
    s.papers.set_exported(sha, &hash, Some(&real.to_string_lossy()))?;
    Ok(real)
}

#[derive(Deserialize, Default)]
struct ExportBody {
    path: Option<String>,
    #[serde(default)]
    last: bool,
    #[serde(default)]
    choose: bool,
}

async fn export(State(s): State<Shared>, Path(sha): Path<String>, Json(b): Json<ExportBody>) -> R {
    sha_ok(&sha)?;
    lib(&s)?;
    let target = if b.last {
        let st = s.papers.export_state(&sha)?;
        PathBuf::from(st.and_then(|st| st.path).ok_or_else(|| bad("還沒有存過含筆記的 PDF"))?)
    } else if let Some(p) = b.path.filter(|p| !p.trim().is_empty()) {
        PathBuf::from(p.trim())
    } else if b.choose {
        let saver = s.saver.lock().unwrap().clone();
        let Some(sv) = saver else { return Ok(Json(json!({ "native": false }))) };
        let (dir, name) = default_target(s.papers.meta(&sha)?.as_ref());
        let picked = tokio::task::spawn_blocking(move || sv.choose(&dir, &name))
            .await
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })))?;
        match picked {
            Some(p) => p,
            None => return Ok(Json(json!({ "cancelled": true }))),
        }
    } else {
        return Err(bad("需要 path"));
    };
    let out = write_export(&s, &sha, target)?;
    let name = out.file_name().map(|n| n.to_string_lossy().to_string());
    Ok(Json(json!({ "path": out, "name": name, "export": export_view(&s, &sha)? })))
}

fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Browser fallback: the PDF with notes as a download (recorded as exported, no path).
async fn export_download(State(s): State<Shared>, Path(sha): Path<String>) -> Result<Response, ApiError> {
    sha_ok(&sha)?;
    let (bytes, hash, meta) = build_export(&s, &sha)?;
    let (_, name) = default_target(Some(&meta));
    {
        let _g = s.write.lock().unwrap();
        s.papers.set_exported(&sha, &hash, None)?;
    }
    let disp = format!("attachment; filename=\"notes.pdf\"; filename*=UTF-8''{}", pct(&name));
    Ok(([(header::CONTENT_TYPE, "application/pdf".to_string()), (header::CONTENT_DISPOSITION, disp)], bytes).into_response())
}

async fn export_decline(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    s.papers.set_declined(&sha)?;
    Ok(Json(json!({ "declined": true })))
}

/// Papers in this library whose notes changed since they were last saved into a PDF.
async fn dirty(State(s): State<Shared>) -> R {
    let l = lib(&s)?;
    let mut out = vec![];
    for m in s.papers.list()? {
        if l.has(&m.sha256) && s.papers.is_dirty(&m.sha256)? {
            let (dir, name) = default_target(Some(&m));
            let last = s.papers.export_state(&m.sha256)?.and_then(|st| st.path).filter(|p| std::path::Path::new(p).is_file());
            out.push(json!({ "sha256": m.sha256, "title": m.title, "last_path": last, "default_dir": dir, "default_name": name }));
        }
    }
    Ok(Json(json!({ "papers": out })))
}

// ── models ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TranslateBody {
    text: String,
    #[serde(default)]
    context: String,
}

/// A model call that did not run: lore-llm's payload (with `alternative` when the other
/// engine could answer) plus `llm_unavailable`.
pub(crate) fn llm_error(e: llm::LlmFail) -> ApiError {
    let mut v = e.0.clone();
    v["error"] = json!(format!("沒有可用的模型：{e}"));
    v["llm_unavailable"] = json!(true);
    ApiError(StatusCode::SERVICE_UNAVAILABLE, v)
}

async fn translate(State(s): State<Shared>, Json(b): Json<TranslateBody>) -> R {
    let text = b.text.trim();
    if text.is_empty() {
        return Err(bad("沒有選取文字"));
    }
    let mut user = format!("選取：{text}");
    if !b.context.trim().is_empty() && b.context.trim() != text {
        user += &format!("\n\n上下文：{}", b.context.trim());
    }
    let choice = reading::model_choice(&s.workspace, "lr-translate");
    let a = s.llm.complete_on("lr-translate", choice.as_deref(), LlmRequest::prompt(llm::TRANSLATE_SYSTEM, user).max_tokens(800)).await.map_err(llm_error)?;
    Ok(Json(json!({ "translation": a.text, "engine": a.engine, "fallback_used": a.fallback_used })))
}

async fn chat(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    Ok(Json(json!({ "messages": s.papers.chat(&sha)? })))
}

#[derive(Deserialize)]
struct ChatBody {
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachments: Vec<Attachment>,
    /// File the exchange under this paragraph and send its context.
    #[serde(default)]
    paragraph: Option<String>,
    /// Run this question on this engine once (`local`, `haiku` ...) instead of the model
    /// menu's choice: 「改用地端」 after Claude was not available.
    #[serde(default)]
    model: Option<String>,
    /// The stuck point: `{kind: term|symbol|equation, id}`.
    #[serde(default)]
    chip: Option<reading::ChipRef>,
}

fn with_attachments(text: &str, att: &[Attachment]) -> String {
    let mut out = String::new();
    for a in att {
        out += &format!("〔引文，p. {}〕「{}」\n", a.page, a.text.trim());
        if !a.context.trim().is_empty() && a.context.trim() != a.text.trim() {
            out += &format!("所在段落：{}\n", a.context.trim());
        }
        out += "\n";
    }
    out + text
}

async fn send_chat(State(s): State<Shared>, Path(sha): Path<String>, Json(b): Json<ChatBody>) -> R {
    sha_ok(&sha)?;
    if b.text.trim().is_empty() && b.attachments.is_empty() && b.paragraph.is_none() {
        return Err(bad("沒有內容"));
    }
    let title = s.papers.meta(&sha)?.map(|m| m.title).unwrap_or_default();
    let (context, chip) = match &b.paragraph {
        Some(pid) => {
            let (c, chip) = reading::chat_context(&s, &sha, &title, pid, b.chip.as_ref()).await?;
            (Some(c), chip)
        }
        None => (None, None),
    };
    let history = s.papers.chat(&sha)?;
    let compose = |text: &str, att: &[Attachment], ctx: Option<&str>| -> String {
        match ctx {
            Some(c) => format!("{c}\n〔問題〕{}", if text.trim().is_empty() { reading::DEFAULT_QUESTION } else { text }),
            None => with_attachments(text, att),
        }
    };
    let mut msgs: Vec<Message> = history
        .iter()
        .rev()
        .take(20)
        .rev()
        .map(|m| if m.role == "assistant" { Message::assistant(m.text.clone()) } else { Message::user(compose(&m.text, &m.attachments, m.context.as_deref())) })
        .collect();
    msgs.push(Message::user(compose(&b.text, &b.attachments, context.as_deref())));
    let text = if b.text.trim().is_empty() && b.paragraph.is_some() { reading::DEFAULT_QUESTION.to_string() } else { b.text.clone() };
    let user = ChatMessage { role: "user".into(), text, attachments: b.attachments.clone(), at: lr_core::now(), engine: None, paragraph: b.paragraph.clone(), chip: chip.clone(), context };
    let choice = match b.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) if llm::CHOICES.contains(&m) => Some(m.to_string()),
        Some(_) => return Err(bad("模型只能是 auto、local、cloud、haiku、sonnet 或 opus")),
        None => reading::model_choice(&s.workspace, "lr-chat"),
    };
    let a = s.llm.complete_on("lr-chat", choice.as_deref(), LlmRequest::chat(llm::chat_system(&title), msgs).max_tokens(2000)).await.map_err(llm_error)?;
    let reply = ChatMessage { role: "assistant".into(), text: a.text, attachments: vec![], at: lr_core::now(), engine: Some(a.engine), paragraph: b.paragraph.clone(), chip, context: None };
    {
        let _g = s.write.lock().unwrap();
        s.papers.push_chat(&sha, &[user, reply.clone()])?;
    }
    Ok(Json(json!({ "reply": reply })))
}

async fn clear_chat(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    s.papers.clear_chat(&sha)?;
    Ok(Json(json!({ "cleared": true })))
}

// ── KnowledgeBase ────────────────────────────────────────────────────────

async fn kb_status(State(s): State<Shared>) -> Json<Value> {
    match s.kb.get("/api/spaces").await {
        Ok(v) => Json(json!({ "ok": true, "url": s.kb.base, "member": v["member"], "spaces": v["spaces"] })),
        Err(e) => Json(json!({ "ok": false, "url": s.kb.base, "down": e.down, "error": e.message })),
    }
}

/// `POST /api/kb/open`: start the KnowledgeBase app (macOS `open -a KnowledgeBase`) when
/// its server is this machine's and not up. The page then waits for `/api/kb/status`.
async fn kb_open(State(s): State<Shared>) -> R {
    let local = ["http://127.0.0.1", "http://localhost"].iter().any(|p| s.kb.base.starts_with(p));
    if s.isolated || !local || !cfg!(target_os = "macos") {
        return Ok(Json(json!({ "opened": false })));
    }
    let ok = std::process::Command::new("open").args(["-g", "-a", "KnowledgeBase"]).status().map(|st| st.success()).unwrap_or(false);
    if !ok {
        return Err(ApiError(StatusCode::NOT_FOUND, json!({ "error": "找不到 KnowledgeBase App" })));
    }
    Ok(Json(json!({ "opened": true })))
}

#[derive(Deserialize)]
struct CardBody {
    sha: String,
    page: u32,
    title: String,
    #[serde(default)]
    summary: String,
    kind: Option<String>,
    /// learning (default) | known
    #[serde(default)]
    initial_state: String,
    context: Option<String>,
    space: Option<String>,
}

/// Create a KB card — only on the author's explicit action.
async fn kb_card(State(s): State<Shared>, Json(b): Json<CardBody>) -> Result<(StatusCode, Json<Value>), ApiError> {
    sha_ok(&b.sha)?;
    if b.title.trim().is_empty() {
        return Err(bad("請輸入標題"));
    }
    if !matches!(b.initial_state.as_str(), "" | "learning" | "known") {
        return Err(bad("初始狀態只能是「學習中」或「已掌握」"));
    }
    let paper_title = s.papers.meta(&b.sha)?.map(|m| m.title).unwrap_or_default();
    let space = match b.space.filter(|x| !x.is_empty()) {
        Some(x) => x,
        None => s.kb.default_space().await?,
    };
    let body = kb::card_body(&kb::CardRequest {
        space: &space,
        title: &b.title,
        summary: &b.summary,
        kind: b.kind.as_deref(),
        initial_state: &b.initial_state,
        sha: &b.sha,
        page: b.page,
        paper_title: &paper_title,
        context: b.context.as_deref(),
    });
    let v = s.kb.post("/api/nodes", &body, Duration::from_secs(10)).await?;
    Ok((StatusCode::CREATED, Json(json!({ "node": v["node"], "summary": v["summary"], "space": space }))))
}

async fn kb_draft(State(s): State<Shared>, Json(b): Json<Value>) -> R {
    // the paper's title and the page go with the quote, so the draft can say where it is from
    let source = b["sha"].as_str().and_then(|sha| s.papers.meta(sha).ok().flatten()).map(|m| m.title);
    let body = json!({ "term": b["term"], "context": b["context"], "lang": "zh-Hant", "page": b["page"], "source": source });
    Ok(Json(s.kb.post("/api/draft", &body, Duration::from_secs(120)).await?))
}

async fn kb_node(State(s): State<Shared>, Path(id): Path<String>) -> R {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(bad("id"));
    }
    Ok(Json(s.kb.get(&format!("/api/nodes/{id}")).await?))
}

// ── print ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PrintBody {
    sha: String,
}

/// Desktop: open print.html in a window (the native dialog follows when it is ready).
/// Browser: `{native: false}` and the page opens print.html itself.
async fn print(State(s): State<Shared>, Json(b): Json<PrintBody>) -> R {
    sha_ok(&b.sha)?;
    let printer = s.printer.lock().unwrap().clone();
    let Some(p) = printer else { return Ok(Json(json!({ "native": false }))) };
    let id = lr_core::new_id("p");
    let port = *s.port.lock().unwrap();
    let url = format!("http://127.0.0.1:{port}/print.html?sha={}&job={id}", b.sha);
    p.open(&url, &id).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })))?;
    Ok(Json(json!({ "native": true, "job": id })))
}

#[derive(Deserialize)]
struct ReadyQ {
    id: String,
}

async fn print_ready(State(s): State<Shared>, Query(q): Query<ReadyQ>) -> R {
    let printer = s.printer.lock().unwrap().clone();
    match printer {
        Some(p) => p.ready(&q.id).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })))?,
        None => return Ok(Json(json!({ "native": false }))),
    }
    Ok(Json(json!({ "native": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_carry_the_page() {
        let a = Attachment { text: "x".into(), page: 2, context: "a x b".into() };
        assert!(with_attachments("why?", &[a]).contains("p. 2"));
    }
}
