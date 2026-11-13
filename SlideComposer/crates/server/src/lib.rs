//! sc-server: axum on 127.0.0.1:7880. Serves `web/` and the JSON API.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/info | workspace, styles, decks, tools (pandoc, Chrome) |
//! | GET | /api/styles/{id} | style pack (tags, palette, emphasis, components, arrows, ghost) |
//! | POST | /api/decks | `{name, style}` new deck from the style template |
//! | POST | /api/decks/import?name=&style= | body = .pptx bytes, or JSON `{path, name, style}` |
//! | GET | /api/decks/{name} | deck + layout backdrops + style + undo state |
//! | POST | /api/decks/{name}/ops | `{ops, label}` → changed slides (validated, undoable) |
//! | POST | /api/decks/{name}/undo, /redo | |
//! | POST | /api/decks/{name}/reveal | `{slide, spec}` progressive reveal (one transaction) |
//! | POST | /api/decks/{name}/new-slide | `{after?, layout?}` |
//! | POST | /api/decks/{name}/component | `{name, x, y, n, slide}` → object (not inserted) |
//! | POST | /api/decks/{name}/save | write deck.json (also done after every change) |
//! | POST | /api/decks/{name}/export | write exports/<name>.pptx (free name) |
//! | GET | /api/decks/{name}/pptx | export bytes (download) |
//! | POST | /api/decks/{name}/pdf | headless Chrome prints /print.html → exports/<name>.pdf |
//! | GET | /api/decks/{name}/asset?id= | image bytes (`pkg:` / `file:`) |
//! | POST | /api/decks/{name}/assets?ext= | body = image bytes → `{asset}` |
//! | POST | /api/decks/{name}/latex | `{omml}` → `{latex}` (cached in the deck folder) |
//! | POST | /api/math | `{latex, display?, size?}` → `{omml}` (pandoc) |
//! | POST | /api/files/open | `{path}` → `{name}`: open a .pptx where it lives, bound to it |
//! | GET | /api/decks/{name}/file | the bound file's save state |
//! | POST | /api/decks/{name}/file/write | `{force?}` write the deck back to its file (autosave) |
//! | POST | /api/decks/{name}/file/reload | replace the deck with the file as it is on disk |
//! | POST | /api/decks/{name}/file/save-as | `{path}` bind to a new file and write it |
//! | POST | /api/shell/dialog | `{kind: open\|save, name?}` → `{path}` native dialog (desktop app only, else 501) |

mod export_open;
pub mod jobs;
pub mod kbterms;
pub mod llm;
mod flow; // web/figflow.js: 流程圖 (lore-flow editor, one picture that carries its spec)
mod plot; // web/figplot.js: 繪圖 (lore-plot editor, a PNG picture that carries its spec)
mod notation;
mod reader;
pub mod deckpolish;
pub mod deckvoice;
pub mod deckstyle;
pub mod todeck;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sc_core::{Session, Workspace};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::services::ServeDir;
use lore_canvas::reveal::{RevealSpec, reveal};
use lore_canvas::{Applied, Object, Op, Slide};

pub struct AppState {
    pub ws: Workspace,
    pub web: PathBuf,
    pub port: Mutex<u16>,
    pub sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
    pub pandoc: lore_math::Pandoc,
    /// LoreForge private folders (read-only here: font preferences from private/shared/fonts.toml).
    pub home: lore_home::Home,
    /// Fonts this machine has (system + private/shared/fonts/), scanned once.
    pub fonts: std::sync::OnceLock<lore_fonts::Installed>,
    /// Native open/save dialogs, installed by the desktop shell (none in browser mode).
    pub dialog: std::sync::OnceLock<DialogHook>,
    /// Equation fallback pictures already rendered: `<deck>/<key>/<object hash>` → PNG,
    /// so autosaves do not run Chrome again for unchanged equations.
    pub fallback_cache: Mutex<HashMap<String, Vec<u8>>>,
    /// Model engines (論文報告); tests set it before the first request (`llm::Llm::with_factory`).
    pub llm: std::sync::OnceLock<llm::Llm>,
}

/// A native file dialog request from the page (`/api/shell/dialog`).
#[derive(Debug, Clone, Deserialize)]
pub struct DialogReq {
    /// `open` (pick a .pptx) or `save` (choose where a .pptx goes).
    pub kind: String,
    /// Suggested file name for `save`.
    #[serde(default)]
    pub name: Option<String>,
}

/// Shows the dialog and returns the chosen path (blocking; runs off the async runtime).
pub type DialogHook = Arc<dyn Fn(DialogReq) -> Option<PathBuf> + Send + Sync>;

/// The author's private font folder (`private/shared/fonts/`), when there is one.
fn fonts_dir(home: &lore_home::Home) -> Option<PathBuf> {
    home.shared_file("fonts").filter(|p| p.is_dir())
}

fn installed_fonts(s: &AppState) -> &lore_fonts::Installed {
    s.fonts.get_or_init(|| lore_fonts::Installed::scan(fonts_dir(&s.home).as_deref()))
}

fn font_table(s: &AppState) -> lore_fonts::Table {
    lore_fonts::Table::with_user(&s.home.prefs().fonts.fallback)
}

/// Every font name a deck uses (run looks, paragraph ends, table cells…).
fn deck_font_names(deck: &serde_json::Value) -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, x) in m {
                    if matches!(k.as_str(), "font" | "ea_font" | "cs_font") {
                        if let Some(f) = x.as_str() {
                            out.push(f.to_string());
                        }
                    } else {
                        walk(x, out);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    walk(deck, &mut out);
    out
}

pub type Shared = Arc<AppState>;

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}
impl From<sc_core::Error> for ApiError {
    fn from(e: sc_core::Error) -> Self {
        let code = match e {
            sc_core::Error::NotFound(_) => StatusCode::NOT_FOUND,
            sc_core::Error::Invalid(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(code, e.to_string())
    }
}
impl From<lore_canvas::OpError> for ApiError {
    fn from(e: lore_canvas::OpError) -> Self {
        ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
    }
}
fn bad(m: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, m.into())
}

type R = Result<Json<Value>, ApiError>;

pub fn repo_dir() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    p.canonicalize().unwrap_or(p)
}

/// The workspace both the browser mode and the desktop app use:
/// `SLIDECOMPOSER_WORKSPACE`, else LoreForge `private/SlideComposer`
/// (created from `private-example/SlideComposer` the first time).
pub fn default_workspace() -> Workspace {
    let repo = repo_dir();
    let home = lore_home::Home::discover(&repo);
    let default = match home.seed_app("SlideComposer") {
        Ok(Some(d)) => d,
        Ok(None) => repo.join("../private/SlideComposer"),
        Err(e) => {
            eprintln!("sc-server: private/SlideComposer: {e}");
            repo.join("../private/SlideComposer")
        }
    };
    Workspace::from_env(&default)
}

pub fn routes() -> Router<Shared> {
    // the shared shell: /icons.js /base.css /shell.js (commons/ui, compiled in)
    let mut shell = Router::new();
    for (path, mime, body) in lore_shell::FILES {
        shell = shell.route(path, get(move || async move { ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body) }));
    }
    Router::new()
        .merge(shell)
        .route("/theme.css", get(theme_css))
        .route("/theme.js", get(theme_js))
        .route("/api/theme", get(theme_get).put(theme_put))
        .route("/notation-panel.js", get(notation_panel_js))
        .route("/api/decks/{name}/notation", get(notation_get).put(notation_put))
        .route("/api/decks/{name}/arrangements", get(arrangements_get))
        .route("/api/arrangements", post(arrangement_save))
        .route("/api/decks/{name}/notation/extract", post(notation_extract))
        .route("/prefs.css", get(prefs_css))
        .route("/prefs.js", get(prefs_js))
        .route("/fonts/{file}", get(font_file))
        .route("/api/styles/{id}", get(style))
        .route("/api/styles/{id}/preview/{n}", get(style_preview))
        .route("/api/styles/{id}/sample", get(style_sample))
        .route("/api/styles/{id}/asset", get(style_asset))
        .route("/api/decks", post(new_deck))
        .route("/api/decks/import", post(import).layer(DefaultBodyLimit::max(300 * 1024 * 1024)))
        .route("/api/decks/{name}", get(open))
        .route("/api/decks/{name}/ops", post(ops))
        .route("/api/decks/{name}/polish", get(deckpolish::get_report).post(deckpolish::run))
        .route("/api/decks/{name}/polish/changes", post(deckpolish::changes))
        .route("/api/decks/{name}/polish/{item}", post(deckpolish::item))
        .route("/api/decks/{name}/undo", post(undo))
        .route("/api/decks/{name}/redo", post(redo))
        .route("/api/decks/{name}/reveal", post(do_reveal))
        .route("/api/decks/{name}/new-slide", post(new_slide))
        .route("/api/decks/{name}/component", post(component))
        .route("/api/decks/{name}/save", post(save))
        .route("/api/decks/{name}/export", post(export))
        .route("/api/decks/{name}/pptx", get(pptx))
        .route("/api/decks/{name}/pdf", post(pdf))
        .route("/api/decks/{name}/asset", get(asset))
        .route("/api/decks/{name}/assets", post(add_asset).layer(DefaultBodyLimit::max(60 * 1024 * 1024)))
        .route("/api/decks/{name}/latex", post(latex))
        .route("/api/math", post(math))
        .route("/api/files/open", post(open_file))
        .route("/api/decks/{name}/file", get(file_get))
        .route("/api/decks/{name}/file/write", post(file_write))
        .route("/api/decks/{name}/file/reload", post(file_reload))
        .route("/api/decks/{name}/file/save-as", post(file_save_as))
        .route("/api/shell/dialog", post(shell_dialog))
        .route("/api/export/open", post(export_open::export_open))
        .route("/api/decks/{name}/learn", axum::routing::put(kbterms::understood))
        .merge(reader::routes())
        .merge(todeck::routes().layer(local_cors()))
        .route("/api/info", get(info).layer(local_cors()))
}

/// Pages of the other Lore apps (127.0.0.1 / localhost on any port, the desktop webviews) may
/// call the routes with this layer: PaperComposer's 做成簡報 lists the bases and starts a deck here.
fn local_cors() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|o, _| local_origin(o.to_str().unwrap_or(""))))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::CONTENT_TYPE])
}

pub fn local_origin(o: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:"].iter().any(|p| o.strip_prefix(p).is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())))
        || o == "tauri://localhost"
        || o == "http://tauri.localhost"
}

/// The LoreForge themes (commons/ui/theme.css, the same file in every app),
/// compiled in so the bundled app serves it too.
async fn theme_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::CSS)
}

/// commons/ui/theme.js with the shared choice in front (first paint has the theme).
async fn theme_js(State(s): State<Shared>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::script(&s.home))
}

/// The theme every LoreForge app shows (`private/shared/ui.toml`); null = none chosen yet.
async fn theme_get(State(s): State<Shared>) -> Json<Value> {
    Json(json!({ "theme": s.home.theme() }))
}

async fn theme_put(State(s): State<Shared>, Json(b): Json<Value>) -> R {
    let id = b["theme"].as_str().unwrap_or_default();
    match s.home.set_theme(id) {
        Ok(t) => Ok(Json(json!({ "theme": t }))),
        Err(e) => Err(bad(e.to_string())),
    }
}

/// 版型: the author's named arrangements and this deck's page areas per kind.
async fn arrangements_get(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    Ok(Json(json!({
        "presets": s.ws.arrangements(),
        "pages": s.ws.page_areas(sess.editor.style.as_ref(), &sess.source),
        "defaults": sess.editor.deck.defaults,
    })))
}

async fn arrangement_save(State(s): State<Shared>, Json(v): Json<Value>) -> R {
    let id = s.ws.save_arrangement(&v)?;
    Ok(Json(json!({ "id": id })))
}

/// The shared notation-panel Web Component (commons/ui), compiled in.
async fn notation_panel_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], include_str!("../../../../commons/ui/notation-panel/notation-panel.js"))
}

fn global_notation(s: &Shared) -> Option<std::path::PathBuf> {
    s.home.shared_dir().map(|d| d.join("notation.json")).filter(|p| p.is_file())
}

/// 對照表: merged registry, check report on the slides, line map for jumps.
async fn notation_get(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let (glob, doc) = notation::load(&sess.dir, global_notation(&s).as_deref());
    Ok(Json(notation::view(&sess.editor.deck, &sess.latex, &glob, &doc)))
}

/// Save the document file (from the panel), then the fresh view.
async fn notation_put(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<Value>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let doc = notation::from_panel(&b).map_err(bad)?;
    doc.save(&sess.dir.join("notation.json")).map_err(bad)?;
    let (glob, doc) = notation::load(&sess.dir, global_notation(&s).as_deref());
    Ok(Json(notation::view(&sess.editor.deck, &sess.latex, &glob, &doc)))
}

/// Auto-extract symbols / equations / abbreviations from the slides into the document file.
async fn notation_extract(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let (glob, mut doc) = notation::load(&sess.dir, global_notation(&s).as_deref());
    let added = notation::extract_into(&sess.editor.deck, &sess.latex, &mut doc);
    if added > 0 {
        doc.save(&sess.dir.join("notation.json")).map_err(bad)?;
    }
    let mut v = notation::view(&sess.editor.deck, &sess.latex, &glob, &doc);
    v["added"] = json!(added);
    Ok(Json(v))
}

/// Interface fonts from `private/shared/fonts.toml` (CSS variables; linked after styles.css).
/// …plus `@font-face` rules for the font files in private/shared/fonts/.
async fn prefs_css(State(s): State<Shared>) -> impl IntoResponse {
    let css = s.home.prefs().fonts.css() + &lore_fonts::font_faces_css(installed_fonts(&s), "/fonts/");
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], css)
}

/// A font file from the author's private font folder (never anything else).
async fn font_file(State(s): State<Shared>, Path(file): Path<String>) -> Response {
    let Some(dir) = fonts_dir(&s.home).filter(|_| lore_fonts::safe_font_file(&file)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(dir.join(&file)) {
        Ok(bytes) => {
            let ct = match file.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
                Some("woff2") => "font/woff2",
                Some("woff") => "font/woff",
                Some("otf") | Some("otc") => "font/otf",
                _ => "font/ttf",
            };
            ([(header::CONTENT_TYPE, ct)], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `window.SC_PREFS`: font fallbacks for fonts named in decks (`[fallback]` in
/// private/shared/fonts.toml), loaded before render.js. Licensed or company fonts
/// are named only there, never in this repository.
async fn prefs_js(State(s): State<Shared>) -> impl IntoResponse {
    let fonts = s.home.prefs().fonts;
    let body = format!("window.SC_PREFS = {};\n", json!({ "fonts": { "fallback": fonts.fallback, "stacks": font_table(&s).stacks() } }));
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], body)
}

/// The settings behind the 模型 tab: the shared engine file, with this app's tasks (lore-flow's).
fn engines(shared: &Shared) -> lore_engines::Engines {
    lore_engines::Engines::new(shared.home.clone(), "SlideComposer", llm::TASKS)
        .with_tasks(lore_llm::TaskTable::parse(include_str!("../../../../commons/crates/lore-flow/tasks.toml")).unwrap_or_default())
        .with_tasks(lore_plot::assist::task_table())
}

pub fn router(shared: Shared) -> Router {
    let mut e = engines(&shared);
    if let Some(rt) = lore_llm::shared_runtime::SharedRuntime::process(llm::APP) {
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
    // static files: always revalidate (no build step, edits show up on reload)
    let web = tower::ServiceBuilder::new()
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        ))
        .service(ServeDir::new(&shared.web).append_index_html_on_directories(true));
    let plots = plot::plot_router(&shared);
    let flows = flow::flow_router(&shared);
    routes().merge(plot::routes()).merge(flow::routes()).with_state(shared).merge(plots).merge(flows).merge(lore_engines::router(engines)).fallback_service(web)
}

pub fn state(ws: Workspace, web: PathBuf) -> Shared {
    state_at(ws, web, lore_home::Home::discover(&repo_dir()))
}

/// With the LoreForge folder named (tests use a temporary one).
pub fn state_at(ws: Workspace, web: PathBuf, home: lore_home::Home) -> Shared {
    Arc::new(AppState {
        ws,
        web,
        port: Mutex::new(0),
        sessions: Mutex::new(HashMap::new()),
        pandoc: lore_math::Pandoc::default(),
        home,
        fonts: std::sync::OnceLock::new(),
        dialog: std::sync::OnceLock::new(),
        fallback_cache: Mutex::new(HashMap::new()),
        llm: std::sync::OnceLock::new(),
    })
}

pub async fn bind(shared: Shared, port: u16) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    *shared.port.lock().unwrap() = addr.port();
    let app = router(shared);
    Ok((addr, async move { axum::serve(listener, app).await }))
}

fn session(s: &Shared, name: &str) -> Result<Arc<Mutex<Session>>, ApiError> {
    let mut map = s.sessions.lock().unwrap();
    if let Some(x) = map.get(name) {
        return Ok(x.clone());
    }
    let sess = Arc::new(Mutex::new(s.ws.open(name)?));
    map.insert(name.to_string(), sess.clone());
    Ok(sess)
}

// ---------------------------------------------------------------------------

async fn info(State(s): State<Shared>) -> R {
    Ok(Json(json!({
        "workspace": s.ws.root,
        "decks_dir": s.ws.decks,
        "styles": s.ws.style_ids(),
        "style_list": style_list(&s.ws),
        "decks": s.ws.deck_names(),
        "pandoc": s.pandoc.available(),
        "chrome": sc_core::find_chrome().is_some(),
        "shell": if s.dialog.get().is_some() { "desktop" } else { "browser" },
    })))
}

/// The styles for the start page: id, display name and how many preview pictures
/// (`styles/<id>/preview/slide-<n>.png`, from the style_preview example) it has.
fn style_list(ws: &Workspace) -> Vec<Value> {
    ws.style_ids()
        .into_iter()
        .map(|id| {
            let name = ws.style(&id).map(|st| st.pack.pack.name.clone()).unwrap_or_default();
            let dir = ws.styles_dir().join(&id).join("preview");
            let previews = (1..).take_while(|n| dir.join(format!("slide-{n}.png")).is_file()).count();
            json!({ "id": id, "name": if name.is_empty() { id.clone() } else { name }, "previews": previews })
        })
        .collect()
}

/// One preview picture of a style (`slide-<n>.png` only).
async fn style_preview(State(s): State<Shared>, Path((id, n)): Path<(String, u32)>) -> Result<Response, ApiError> {
    s.ws.style(&id)?;
    let f = s.ws.styles_dir().join(&id).join("preview").join(format!("slide-{n}.png"));
    let bytes = std::fs::read(&f).map_err(|_| ApiError(StatusCode::NOT_FOUND, "no preview".into()))?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "no-cache")], bytes).into_response())
}

/// A style's cover and content page as a small deck, rendered by the client as a preview.
async fn style_sample(State(s): State<Shared>, Path(id): Path<String>) -> R {
    Ok(Json(serde_json::to_value(s.ws.style_sample(&id)?).unwrap()))
}

async fn style_asset(State(s): State<Shared>, Path(id): Path<String>, Query(q): Query<AssetQ>) -> Result<Response, ApiError> {
    s.ws.style(&id)?;
    let bytes = s.ws.style_asset(&id, &q.id).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no asset".into()))?;
    let ext = q.id.rsplit('.').next().unwrap_or("").to_lowercase();
    let mime = match ext.as_str() { "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "svg" => "image/svg+xml", "emf" => "image/x-emf", _ => "application/octet-stream" };
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "max-age=600")], bytes).into_response())
}

async fn style(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let st = s.ws.style(&id)?;
    Ok(Json(serde_json::to_value(&st.pack).unwrap()))
}

#[derive(Deserialize)]
struct NewDeck {
    name: String,
    style: String,
}

async fn new_deck(State(s): State<Shared>, Json(b): Json<NewDeck>) -> R {
    let name = s.ws.new_deck(&b.name, &b.style)?;
    Ok(Json(json!({ "name": name })))
}

#[derive(Deserialize, Default)]
struct ImportQ {
    name: Option<String>,
    style: Option<String>,
    path: Option<String>,
}

async fn import(State(s): State<Shared>, Query(q): Query<ImportQ>, headers: HeaderMap, body: Bytes) -> R {
    let json_body = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let (bytes, q) = if json_body {
        let b: ImportQ = serde_json::from_slice(&body).map_err(|e| bad(e.to_string()))?;
        let path = b.path.clone().ok_or_else(|| bad("缺少 path"))?;
        (std::fs::read(&path).map_err(|e| bad(format!("無法讀取 {path}：{e}")))?, b)
    } else {
        (body.to_vec(), q)
    };
    let name = q.name.clone().or_else(|| {
        q.path.as_deref().and_then(|p| std::path::Path::new(p).file_stem()).map(|s| s.to_string_lossy().to_string())
    });
    let name = name.unwrap_or_else(|| "imported".into());
    let style = q.style.clone().or_else(|| s.ws.style_ids().into_iter().next());
    let name = s.ws.import_pptx(&name, &bytes, style.as_deref())?;
    Ok(Json(json!({ "name": name })))
}

fn status(sess: &Session) -> Value {
    json!({
        "rev": sess.editor.rev,
        "undo": sess.editor.can_undo(),
        "redo": sess.editor.can_redo(),
    })
}

async fn open(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let style = sess.editor.style.as_ref().map(|st| serde_json::to_value(&st.pack).unwrap());
    // What each font the deck names becomes on this machine.
    let deck = serde_json::to_value(&sess.editor.deck).unwrap_or_default();
    let mut names = deck_font_names(&deck);
    if let Some(st) = &style {
        names.extend(deck_font_names(st));
    }
    let table = font_table(&s);
    let fonts = lore_fonts::report(&table, installed_fonts(&s), names);
    let stacks: serde_json::Map<String, serde_json::Value> = fonts.iter().map(|r| (r.name.clone(), json!(r.chain))).collect();
    Ok(Json(json!({
        "name": sess.name,
        "deck": deck,
        "backdrops": sess.backdrops,
        "style": style,
        "status": status(&sess),
        "file": sess.file_status(),
        "fonts": fonts,
        "font_stacks": stacks,
    })))
}

fn changed(sess: &mut Session, a: Applied) -> R {
    sess.save()?;
    let slides: Vec<&Slide> = a.slides.iter().filter_map(|u| sess.editor.deck.slide(u)).collect();
    let order: Vec<&str> = sess.editor.deck.slides.iter().map(|s| s.uid.as_str()).collect();
    Ok(Json(json!({
        "label": a.label,
        "slides": slides,
        "order": order,
        "defaults": sess.editor.deck.defaults,
        "status": status(sess),
        "file": sess.file_status(),
    })))
}

#[derive(Deserialize)]
struct OpsBody {
    ops: Vec<Op>,
    #[serde(default)]
    label: String,
}

async fn ops(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<OpsBody>) -> R {
    let sess = session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    let a = sess.editor.apply(b.ops, &b.label)?;
    changed(&mut sess, a)
}

async fn undo(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    match sess.editor.undo() {
        Some(a) => changed(&mut sess, a),
        None => Err(bad("沒有可復原的動作")),
    }
}

async fn redo(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    match sess.editor.redo() {
        Some(a) => changed(&mut sess, a),
        None => Err(bad("沒有可重做的動作")),
    }
}

#[derive(Deserialize)]
struct RevealBody {
    slide: String,
    spec: RevealSpec,
}

async fn do_reveal(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<RevealBody>) -> R {
    let sess = session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    let idx = sess.editor.deck.slide_index(&b.slide).ok_or_else(|| bad("找不到投影片"))?;
    let slide = sess.editor.deck.slides[idx].clone();
    let rules = sess.editor.style.as_ref().map(|st| st.pack.ghost.clone()).unwrap_or(lore_ooxml::style::GhostRules {
        picture_alpha: 0.2,
        line_alpha: 0.2,
        fill_alpha: 0.3,
        text_color: "A6A6A6".into(),
        agenda_bold_current: true,
    });
    let grey = rules.text_color.clone();
    let ghost = move |l: &lore_canvas::Look| lore_canvas::Look { color: grey.clone(), ..l.clone() };
    let mut deck = sess.editor.deck.clone();
    let out = reveal(&slide, &b.spec, &rules, &ghost, &mut || deck.new_uid());
    if out.len() < 2 {
        return Err(bad("至少要兩個步驟"));
    }
    let mut ops = vec![Op::DeleteSlide { slide: b.slide.clone() }];
    for (i, sl) in out.into_iter().enumerate() {
        ops.push(Op::InsertSlide { index: idx + i, slide: sl });
    }
    let a = sess.editor.apply(ops, "漸進揭露")?;
    changed(&mut sess, a)
}

#[derive(Deserialize)]
struct NewSlideBody {
    after: Option<String>,
    layout: Option<String>,
    /// Page kind (`cover`, `content`, `end`): its layout from the style.
    #[serde(default)]
    kind: Option<String>,
    /// Arrangement the slide is laid out with, and the objects the editor placed for it.
    #[serde(default)]
    arrangement: Option<String>,
    #[serde(default)]
    objects: Vec<Object>,
}

async fn new_slide(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<NewSlideBody>) -> R {
    let sess = session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    let kind = b.kind.clone().unwrap_or_else(|| "content".into());
    let layout = b.layout.clone().or_else(|| {
        sess.editor.style.as_ref().and_then(|st| st.pack.layouts.get(&kind).cloned())
    }).or_else(|| sess.source.layouts.first().map(|l| l.name().to_string()));
    // the deck's own file lacks that layout (an imported deck): the page after which it goes decides
    let after_layout = b.after.as_deref().and_then(|u| sess.editor.deck.slide(u)).and_then(|s| s.layout.clone());
    let slide_layout = match &layout {
        Some(l) if sess.source.layout_by_name(l).is_none() => after_layout.or(layout.clone()),
        _ => layout.clone(),
    };
    let uid = sess.editor.deck.new_uid();
    let mut slide = Slide::new(uid, slide_layout.clone());
    if let Some(l) = &layout {
        slide.objects = s.ws.new_slide_objects(sess.editor.style.as_ref(), &sess.source, l, &sess.dir)?;
    }
    // a recipe brings its own title: drop the stand-in title box (decks without a title placeholder)
    if b.objects.iter().any(|o| o.slot.as_deref().is_some_and(|s| s.starts_with("title"))) {
        slide.objects.retain(|o| o.slot.as_deref() != Some("title"));
    }
    // the arrangement's slots, after the title and starters, with fresh ids
    let mut next = slide.objects.iter().map(|o| o.id).max().unwrap_or(1) + 1;
    for mut o in b.objects {
        o.id = next;
        next += 1;
        slide.objects.push(o);
    }
    slide.arrangement = b.arrangement.clone();
    let index = b
        .after
        .as_deref()
        .and_then(|u| sess.editor.deck.slide_index(u))
        .map(|i| i + 1)
        .unwrap_or(sess.editor.deck.slides.len());
    let a = sess.editor.apply(vec![Op::InsertSlide { index, slide }], "新增投影片")?;
    changed(&mut sess, a)
}

#[derive(Deserialize)]
struct CompBody {
    name: String,
    slide: String,
    x: i64,
    y: i64,
    #[serde(default = "one")]
    n: usize,
}
fn one() -> usize {
    1
}

async fn component(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<CompBody>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let st = sess.editor.style.as_ref().ok_or_else(|| bad("這份簡報沒有風格包"))?;
    let id = sess.editor.deck.slide(&b.slide).map(|s| s.max_id() + 1).unwrap_or(2);
    let o = lore_canvas::components::instantiate(st, &b.name, b.x, b.y, b.n, id).ok_or_else(|| bad(format!("風格包沒有元件 {}", b.name)))?;
    Ok(Json(serde_json::to_value(o).unwrap()))
}

async fn save(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    sess.save()?;
    Ok(Json(json!({ "saved": true })))
}

/// Fallback pictures for the equation boxes export will write (new or
/// edited): each box is rendered by headless Chrome from `object.html`.
/// Runs without holding the session lock (the page reads the deck).
/// Returns the pictures and, when some could not be made, why.
async fn equation_fallbacks(s: &Shared, name: &str) -> (HashMap<String, Vec<u8>>, Option<String>) {
    let (mut cached, need) = cached_fallbacks(s, name);
    if need.is_empty() {
        return (cached, None);
    }
    let Some(chrome) = sc_core::find_chrome() else {
        return (HashMap::new(), Some(format!("沒有找到 Chrome：{} 個新公式沒有備用圖片（Keynote、Google 簡報會看不到）", need.len())));
    };
    let port = *s.port.lock().unwrap();
    let name = name.to_string();
    let need_keys: Vec<(String, String)> = need.iter().map(|(_, _, _, c)| c.clone()).collect();
    let need: Vec<(String, u32, lore_canvas::Frame)> = need.into_iter().map(|(sl, id, f, _)| (sl, id, f)).collect();
    let (fresh, warn) = tokio::task::spawn_blocking(move || {
        let mut out = HashMap::new();
        let mut failed = 0;
        let mut why: Option<String> = None;
        let dir = std::env::temp_dir().join(format!("sc-fallback-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for (slide, id, f) in need {
            // After a timeout Chrome is likely to hang again: skip the rest
            // instead of waiting 30 s per box.
            if why.as_deref().is_some_and(|w| w.contains("超過")) {
                failed += 1;
                continue;
            }
            let w = ((f.w as f64) / 9525.0).ceil().max(1.0) as u32;
            let h = ((f.h as f64) / 9525.0).ceil().max(1.0) as u32;
            let url = format!("http://127.0.0.1:{port}/object.html?deck={}&slide={}&obj={id}", urlenc(&name), urlenc(&slide));
            let png = dir.join(format!("{slide}-{id}.png"));
            let _ = std::fs::remove_file(&png);
            match sc_core::screenshot_png(&chrome, &url, w, h, &png).and_then(|_| Ok(std::fs::read(&png)?)) {
                Ok(b) => {
                    out.insert(lore_canvas::pptx_out::equation_fallback_key(&slide, id), b);
                }
                Err(e) => {
                    failed += 1;
                    why.get_or_insert(e.to_string());
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        let warn = (failed > 0).then(|| {
            format!("{failed} 個公式的備用圖片沒有產生成功（{}）；這些公式在 Keynote、Google 簡報會看不到", why.unwrap_or_default())
        });
        (out, warn)
    })
    .await
    .unwrap_or_else(|_| (HashMap::new(), Some("備用圖片產生失敗".into())));
    let mut cache = s.fallback_cache.lock().unwrap();
    for (key, ck) in need_keys {
        if let Some(b) = fresh.get(&key) {
            cache.insert(ck, b.clone());
            cached.insert(key, b.clone());
        }
    }
    (cached, warn)
}

/// Fallback pictures already rendered for this deck's equation boxes (by
/// content), and the boxes still needing one: (slide, id, frame, (key, cache key)).
#[allow(clippy::type_complexity)]
fn cached_fallbacks(s: &Shared, name: &str) -> (HashMap<String, Vec<u8>>, Vec<(String, u32, lore_canvas::Frame, (String, String))>) {
    let Ok(sess) = session(s, name) else { return (HashMap::new(), vec![]) };
    let sess = sess.lock().unwrap();
    let deck = &sess.editor.deck;
    let cache = s.fallback_cache.lock().unwrap();
    let mut have = HashMap::new();
    let mut need = vec![];
    for (slide, id, f) in lore_canvas::pptx_out::equations_needing_fallback(deck) {
        let key = lore_canvas::pptx_out::equation_fallback_key(&slide, id);
        let obj = deck.slide(&slide).and_then(|sl| sl.find(id)).and_then(|o| serde_json::to_vec(o).ok()).unwrap_or_default();
        let ck = format!("{name}/{key}/{:016x}", lore_canvas::fnv(&obj));
        match cache.get(&ck) {
            Some(b) => {
                have.insert(key, b.clone());
            }
            None => need.push((slide, id, f, (key, ck))),
        }
    }
    (have, need)
}

/// Write every open deck with unsaved changes back to its file (the desktop
/// app calls this when it quits, so the autosave delay never loses edits).
/// Only already-rendered equation fallbacks are used; files changed elsewhere are left alone.
pub fn flush_files(s: &Shared) {
    let sessions: Vec<(String, Arc<Mutex<Session>>)> = s.sessions.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (name, sess) in sessions {
        if !sess.lock().unwrap().pending() {
            continue;
        }
        let (fallbacks, _) = cached_fallbacks(s, &name);
        if let Err(e) = sess.lock().unwrap().write_back(&fallbacks, false) {
            eprintln!("save {name}: {e}");
        }
    }
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

async fn export(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let (fallbacks, warning) = equation_fallbacks(&s, &name).await;
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let p = sess.export_pptx_with(&fallbacks)?;
    Ok(Json(json!({ "path": p, "fallbacks": fallbacks.len(), "warning": warning })))
}

async fn pptx(State(s): State<Shared>, Path(name): Path<String>) -> Result<Response, ApiError> {
    let (fallbacks, _) = equation_fallbacks(&s, &name).await;
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let bytes = sess.export_bytes_with(&fallbacks)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.pptx\"", name)),
        ],
        bytes,
    )
        .into_response())
}

async fn pdf(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let chrome = sc_core::find_chrome().ok_or_else(|| bad("找不到 Chrome／Chromium，無法輸出 PDF（TODO：v2 改用 Typst）"))?;
    let dir = {
        let sess = session(&s, &name)?;
        let sess = sess.lock().unwrap();
        sess.dir.join("exports")
    };
    std::fs::create_dir_all(&dir).map_err(|e| bad(e.to_string()))?;
    let out = sc_core::free_path(&dir, &name, "pdf");
    let port = *s.port.lock().unwrap();
    let url = format!("http://127.0.0.1:{port}/print.html?deck={name}");
    let o2 = out.clone();
    tokio::task::spawn_blocking(move || sc_core::print_pdf(&chrome, &url, &o2))
        .await
        .map_err(|e| bad(e.to_string()))??;
    Ok(Json(json!({ "path": out })))
}

#[derive(Deserialize)]
struct AssetQ {
    id: String,
}

async fn asset(State(s): State<Shared>, Path(name): Path<String>, Query(q): Query<AssetQ>) -> Result<Response, ApiError> {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let (bytes, mime) = sess.asset(&q.id).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no asset".into()))?;
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "max-age=3600")], bytes).into_response())
}

#[derive(Deserialize)]
struct ExtQ {
    ext: String,
}

async fn add_asset(State(s): State<Shared>, Path(name): Path<String>, Query(q): Query<ExtQ>, body: Bytes) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    let id = sess.add_asset(&body, &q.ext)?;
    Ok(Json(json!({ "asset": id })))
}

#[derive(Deserialize)]
struct LatexBody {
    omml: Vec<String>,
}

async fn latex(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<LatexBody>) -> R {
    let sess = session(&s, &name)?;
    let pandoc = s.pandoc.clone();
    let res = tokio::task::spawn_blocking(move || {
        let mut sess = sess.lock().unwrap();
        b.omml
            .iter()
            .map(|m| sess.latex_for(m, &pandoc).map_err(|e| e.to_string()))
            .map(|r| match r {
                Ok(l) => json!({ "latex": l }),
                Err(e) => json!({ "error": e }),
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "results": res })))
}

#[derive(Deserialize)]
struct MathBody {
    latex: String,
    #[serde(default)]
    display: bool,
    #[serde(default = "size14")]
    size: f64,
}
fn size14() -> f64 {
    14.0
}

async fn math(State(s): State<Shared>, Json(b): Json<MathBody>) -> R {
    let pandoc = s.pandoc.clone();
    let omml = tokio::task::spawn_blocking(move || sc_core::latex_to_omml(&pandoc, &b.latex, b.display, b.size))
        .await
        .map_err(|e| bad(e.to_string()))??;
    Ok(Json(json!({ "omml": omml })))
}

// ── Decks bound to a .pptx where it lives ────────────────────────────────────

#[derive(Deserialize)]
struct PathBody {
    path: String,
}

/// 開啟… / Finder: open a .pptx where it lives; the deck is bound to it.
async fn open_file(State(s): State<Shared>, Json(b): Json<PathBody>) -> R {
    let ws = s.ws.clone();
    let name = tokio::task::spawn_blocking(move || ws.open_path(std::path::Path::new(&b.path)))
        .await
        .map_err(|e| bad(e.to_string()))??;
    Ok(Json(json!({ "name": name })))
}

async fn file_get(State(s): State<Shared>, Path(name): Path<String>) -> R {
    let sess = session(&s, &name)?;
    let sess = sess.lock().unwrap();
    Ok(Json(json!({ "file": sess.file_status() })))
}

#[derive(Deserialize, Default)]
struct WriteBody {
    #[serde(default)]
    force: bool,
}

/// Autosave: write the deck back to its file (after the page's debounce).
/// A file changed on disk is not written (`file.state` = `changed` / `missing`)
/// unless `force` (保留我的版本，覆蓋).
async fn file_write(State(s): State<Shared>, Path(name): Path<String>, body: Option<Json<WriteBody>>) -> R {
    let force = body.map(|b| b.force).unwrap_or(false);
    let (fallbacks, warning) = equation_fallbacks(&s, &name).await;
    let sess = session(&s, &name)?;
    let res = tokio::task::spawn_blocking(move || {
        let mut sess = sess.lock().unwrap();
        let w = sess.write_back(&fallbacks, force)?;
        Ok::<_, sc_core::Error>((w, sess.file_status()))
    })
    .await
    .map_err(|e| bad(e.to_string()))??;
    Ok(Json(json!({ "written": res.0 == sc_core::Wrote::Written, "file": res.1, "warning": warning })))
}

/// 重新載入: the deck becomes the file as it is on disk (editor state from the file).
async fn file_reload(State(s): State<Shared>, Path(name): Path<String>) -> R {
    // hold the session while the folder is rewritten, then drop it: the next request reopens it
    let sess = session(&s, &name)?;
    {
        let _guard = sess.lock().unwrap();
        s.ws.reload_file(&name)?;
    }
    s.sessions.lock().unwrap().remove(&name);
    Ok(Json(json!({ "name": name })))
}

/// 另存新檔: bind the deck to `path` (from the native save dialog) and write it there.
async fn file_save_as(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<PathBody>) -> R {
    let path = PathBuf::from(&b.path);
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("pptx")) {
        return Err(bad("檔名要以 .pptx 結尾"));
    }
    let (fallbacks, warning) = equation_fallbacks(&s, &name).await;
    let sess = session(&s, &name)?;
    let file = tokio::task::spawn_blocking(move || {
        let mut sess = sess.lock().unwrap();
        sess.save_as(&path, &fallbacks)?;
        Ok::<_, sc_core::Error>(sess.file_status())
    })
    .await
    .map_err(|e| bad(e.to_string()))??;
    Ok(Json(json!({ "file": file, "warning": warning })))
}

/// Native open/save dialog through the desktop shell; 501 in browser mode
/// (the page then uses a file input / a download instead).
async fn shell_dialog(State(s): State<Shared>, Json(b): Json<DialogReq>) -> R {
    let Some(hook) = s.dialog.get().cloned() else {
        return Err(ApiError(StatusCode::NOT_IMPLEMENTED, "瀏覽器模式沒有檔案對話框".into()));
    };
    let path = tokio::task::spawn_blocking(move || hook(b)).await.map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "path": path })))
}
