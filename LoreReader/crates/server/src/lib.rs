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
//! | GET/POST | /api/papers/{sha}/annotations | list / add `{page, kind, color, quote, rects, note}` |
//! | PUT/DELETE | /api/papers/{sha}/annotations/{id} | `{color?, note?}` / remove |
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
//! | GET/PUT | /api/models | the model menu: `{task, model: auto\|local\|haiku\|sonnet\|opus}` |
//! | GET | /api/kb/status | is the KB server up, member, spaces |
//! | POST | /api/kb/cards | `{sha, page, title, summary, initial_state, kind?, context?, space?}` → KB `POST /api/nodes` |
//! | POST | /api/kb/draft | `{term, context}` → KB `POST /api/draft` |
//! | GET | /api/kb/nodes/{id} | a card (summary for the term popover) |
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
use lr_core::papers::{Annotation, AnnotationPatch, Attachment, ChatMessage, Derived, PaperMeta};
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
    let shared = if isolated { workspace.join("shared") } else { lore_home::Home::discover(&repo_dir()).shared_dir().unwrap_or_else(|| workspace.join("../shared")) };
    Arc::new(AppState {
        kb: kb::Kb::new(&config.kb_url),
        llm,
        config,
        library,
        papers: Papers::new(workspace.join("papers")),
        workspace,
        web,
        shared,
        home: lore_home::Home::discover(&repo_dir()),
        port: Mutex::new(0),
        write: Mutex::new(()),
        texts: Mutex::new(HashMap::new()),
        paras: Mutex::new(HashMap::new()),
        runs: Mutex::new(HashMap::new()),
        printer: Mutex::new(None),
    })
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/prefs.css", get(prefs_css))
        .route("/prefs.js", get(prefs_js))
        .route("/open", get(open))
        .route("/api/info", get(info))
        .route("/api/papers", get(list_papers))
        .route("/api/papers/import", post(import).layer(DefaultBodyLimit::max(400 * 1024 * 1024)))
        .route("/api/papers/{sha}", get(paper).put(rename))
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
        .route("/api/glyphs", get(reading::glyphs_get).put(reading::glyphs_put))
        .route("/api/papers/{sha}/paragraphs", get(reading::paragraphs_get))
        .route("/api/papers/{sha}/paragraphs/run", post(reading::paragraphs_run))
        .route("/api/papers/{sha}/paragraphs/{pid}/understood", post(reading::understood))
        .route("/api/models", get(reading::models_get).put(reading::models_put))
        .route("/api/reader", get(reading::reader_get).put(reading::reader_put))
        .route("/api/papers/{sha}/mine", post(reading::mine).layer(DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/kb/status", get(kb_status))
        .route("/api/kb/cards", post(kb_card))
        .route("/api/kb/draft", post(kb_draft))
        .route("/api/kb/nodes/{id}", get(kb_node))
        .route("/api/print", post(print))
        .route("/api/print/ready", post(print_ready))
}

pub fn router(shared: Shared) -> Router {
    // static files: always revalidate (no build step; the desktop WebView keeps old copies otherwise)
    let web = tower::ServiceBuilder::new()
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-cache")))
        .service(ServeDir::new(&shared.web).append_index_html_on_directories(true));
    routes().with_state(shared).fallback_service(web)
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

/// `window.LORE_PREFS`. `theme` is null until the shared, synced theme preference
/// exists in lore-home; the page then prefers it over its own localStorage value.
async fn prefs_js() -> impl IntoResponse {
    let body = format!("window.LORE_PREFS = {};\n", json!({ "theme": null }));
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], body)
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
    let known = valid_sha(&sha).then(|| s.papers.meta(&sha).ok().flatten()).flatten();
    let what = known.map(|m| format!("《{}》", html_escape(&m.title))).unwrap_or_else(|| "這份 PDF".into());
    let body = format!(
        "<!doctype html><html lang=\"zh-Hant\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>LoreReader</title>\
<link rel=\"stylesheet\" href=\"/styles.css\"><script src=\"/prefs.js\"></script><script src=\"/theme.js\"></script><body class=\"notice-page\"><main class=\"notice\"><h1>不在這台電腦的書庫裡</h1>\
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
        "colors": lr_core::papers::COLORS,
    })))
}

fn paper_view(s: &AppState, m: &PaperMeta) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
    v["present"] = s.library.as_ref().is_ok_and(|l| l.has(&m.sha256)).into();
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
    let meta = PaperMeta {
        schema: lr_core::papers::PAPER_SCHEMA.into(),
        sha256: sha,
        title: title.or_else(|| pdfops::info_title(bytes)).unwrap_or_else(|| if stem.is_empty() { "未命名".into() } else { stem.to_string() }),
        original_name: original_name.to_string(),
        doi: pdfops::doi_of(bytes),
        pages,
        added_at: lr_core::now(),
        derived_from: derived,
        extra: Default::default(),
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
    let (bytes, name) = if is_json {
        let v: Value = serde_json::from_slice(&body).map_err(|e| bad(e.to_string()))?;
        let path = PathBuf::from(v["path"].as_str().ok_or_else(|| bad("需要 path"))?.trim());
        let bytes = std::fs::read(&path).map_err(|e| bad(format!("{}：{e}", path.display())))?;
        (bytes, path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
    } else {
        (body.to_vec(), q.name.unwrap_or_default())
    };
    let name = std::path::Path::new(&name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "paper.pdf".into());
    let (meta, new) = add_paper(&s, &bytes, &name, None, None)?;
    Ok(Json(json!({ "paper": paper_view(&s, &meta), "new": new })))
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

// ── models ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TranslateBody {
    text: String,
    #[serde(default)]
    context: String,
}

pub(crate) fn llm_error(e: String) -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": format!("沒有可用的模型：{e}"), "llm_unavailable": true }))
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
    let choice = reading::model_choice(&s.workspace, "lr-chat");
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
    let body = json!({ "term": b["term"], "context": b["context"], "lang": "zh-Hant" });
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
