//! shelf-server — LoreShelf's local server (127.0.0.1 only).
//!
//! Serves `web/` (no-cache), the shared LoreForge theme (`/theme.css`,
//! `/theme.js`, `/api/theme`) and shell kit (`/icons.js`, `/base.css`,
//! `/shell.js`), and a read-only JSON API over the local catalog:
//!
//! - `GET /api/health`
//! - `GET /api/files?limit=` — items without paths (files, projects, packages, datasets)
//! - `GET /api/files/{id}` — the item, its paths (local display) and a short description
//! - `GET /api/neighbors/{id}?k=12&layers=2` — similar items with each factor's share
//! - `POST /api/open/{id}` — open in the Lore app for its type (`openwith`: `open -a <App>`), else the default app (`open`; an `.app` is revealed, never launched)
//! - `POST /api/reveal/{id}` — show in Finder (`open -R`)
//! - `GET /api/thumb/{id}?s=512` — QuickLook PNG (cached, LRU); 404 = use the kind icon
//! - `GET /api/timeline` — buckets 本週 / 上週 / 本月 / 上個月 / seasons / years
//! - `GET /api/search?q=&cat=&limit=` — guided search over names, first-pages text, categories, hosts, folders, dates; with facets
//! - `POST /api/echo/open {right?, bottom?, height?}` — bring LoreEcho up (`open -g -a LoreEcho`) and seat its window in that corner
//! - `POST /api/act {focus}` + `GET /api/events?after=` — another local app asks the page to turn to a file (long poll)
//! - `GET /api/search?loose=1&q=<sentence>` — files a spoken sentence is probably about (LoreEcho)
//! - `GET /api/categories` — type (exact) and use (rules / the author's choice) categories with counts
//! - `PUT /api/files/{id}/categories` — the author's use categories (local git commit)
//! - `GET /api/nearest?from=&cat=` — the closest item that passes the filter
//! - `GET /api/default-focus?cat=` — where the view starts (the document last opened or modified, whichever is later, else newest PDF; never images, videos or sensitive items)
//! - `GET /api/datasets/{id}/items?offset=&limit=` and `GET /api/thumb/ds/{id}/{index}` — a dataset's members
//!
//! `cat=a,b` (OR within a group, AND across groups) filters files, neighbours,
//! nearest and the timeline. Items in 證件 carry `sensitive: true`.
//!
//! Nothing leaves the machine; nothing in the catalog or the scanned folders
//! is written (the theme choice goes to `private/shared/ui.toml` like every app).
//! The index is rebuilt when the catalog's latest report changes.

pub mod cards;
pub mod classify;
pub mod dataset;
pub mod filing;
pub mod filing_ai;
pub mod fulltext;
pub mod index;
pub mod kbtags;
pub mod llm;
pub mod openwith;
pub mod search;
pub mod thumbs;
pub mod timeline;

use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, body::Bytes};
use index::Index;
use serde_json::{Value, json};
use lore_catalog::category::Filter;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

/// Default port; the next free one is used when it is taken.
pub const DEFAULT_PORT: u16 = 7900;
/// Name in `ports.json`.
pub const PORT_NAME: &str = "ls";
/// The app the "open LoreEcho with the shelf" setting brings up.
pub const ECHO_APP: &str = "LoreEcho";

/// Runs the OS opener (injected so tests never open anything).
pub trait Runner: Send + Sync {
    /// Start and do not wait (opening a file).
    fn run(&self, program: &str, args: &[String]) -> Result<(), String>;
    /// Run to the end, killed after `timeout` (QuickLook thumbnails).
    fn run_wait(&self, program: &str, args: &[String], timeout: std::time::Duration) -> Result<(), String>;
    /// `/Applications/<app>.app` is there (`openwith`: else the system default opens the file).
    fn installed(&self, app: &str) -> bool {
        openwith::installed(app)
    }
}

/// The real opener: `open` on macOS, `xdg-open` elsewhere (no reveal there).
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn run_wait(&self, program: &str, args: &[String], timeout: std::time::Duration) -> Result<(), String> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let end = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(st) if st.success() => return Ok(()),
                Some(st) => return Err(format!("{program}: {st}")),
                None if std::time::Instant::now() >= end => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{program}: timed out"));
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }
}

pub struct AppState {
    pub home: lore_home::Home,
    pub web: PathBuf,
    pub catalog: PathBuf,
    pub runner: Arc<dyn Runner>,
    /// QuickLook thumbnail cache (`<cache>/LoreForge/shelf/thumbs`).
    pub thumbs: PathBuf,
    /// Extract cache filled by the scan (`<cache>/LoreForge/shelf/extract`).
    pub extract: PathBuf,
    members: Mutex<HashMap<String, Arc<Vec<dataset::Member>>>>,
    ql: tokio::sync::Semaphore,
    cache: Mutex<Option<(Option<PathBuf>, Arc<Index>)>>,
    /// The whole-text index, kept across an edit's rebuild (same entries); its key = `cache`'s.
    ft: Mutex<Option<(Option<PathBuf>, Arc<fulltext::FullIndex>)>>,
    /// What another app asked the page to do (`POST /api/act`): a counter and the last request.
    acts: tokio::sync::watch::Sender<(u64, Value)>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    /// `cache` = the shelf cache folder (`<cache>/LoreForge/shelf`).
    pub fn new(home: lore_home::Home, web: PathBuf, catalog: PathBuf, cache: PathBuf, runner: Arc<dyn Runner>) -> Shared {
        Arc::new(AppState {
            home,
            web,
            catalog,
            runner,
            thumbs: cache.join("thumbs"),
            extract: lore_catalog::extract::Extract::dir(&cache),
            members: Mutex::new(HashMap::new()),
            ql: tokio::sync::Semaphore::new(thumbs::MAX_PARALLEL),
            cache: Mutex::new(None),
            ft: Mutex::new(None),
            acts: tokio::sync::watch::channel((0, Value::Null)).0,
        })
    }

    /// Forget the index (after the author's edit).
    pub fn invalidate(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// A dataset's members, listed once per server run.
    pub fn members(&self, id: &str, root: &std::path::Path) -> Arc<Vec<dataset::Member>> {
        let mut m = self.members.lock().unwrap_or_else(|e| e.into_inner());
        m.entry(id.to_string()).or_insert_with(|| Arc::new(dataset::members(root))).clone()
    }

    /// The index, rebuilt when a new scan report appeared.
    pub fn index(&self) -> Arc<Index> {
        // a new scan report, or a finished `loreshelf extract`, makes a new index
        let ft_stamp = self.extract.parent().map(|c| lore_catalog::fulltext::FullText::dir(c).join(lore_catalog::fulltext::STAMP)).and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        let stamp = lore_catalog::Catalog::latest_report(&self.catalog).map(|p| match ft_stamp {
            Some(t) => p.join(format!("{:?}", t)),
            None => p,
        });
        // new labels (`filing-ai.json`) or topic merges (`filing.json`) too
        let labels_stamp: Vec<_> = [filing_ai::FILE, filing::SETTINGS].iter().map(|f| std::fs::metadata(self.catalog.join(f)).and_then(|m| m.modified()).ok()).collect();
        let stamp = stamp.map(|p| p.join(format!("{:?}", labels_stamp)));
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((s, idx)) = c.as_ref()
            && *s == stamp
        {
            return idx.clone();
        }
        // an edit (a category) keeps the entries and their order: the whole-text index stays
        let mut ftc = self.ft.lock().unwrap_or_else(|e| e.into_inner());
        let reuse = ftc.as_ref().filter(|(s, _)| *s == stamp).map(|(_, f)| f.clone());
        let idx = Arc::new(Index::load_with(&self.catalog, &self.extract, reuse));
        *ftc = Some((stamp.clone(), idx.ft.clone()));
        *c = Some((stamp, idx.clone()));
        idx
    }
}

/// `LoreShelf/` (this crate is `LoreShelf/crates/server`).
pub fn app_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    d.canonicalize().unwrap_or(d)
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

pub fn router(shared: Shared) -> Router {
    let api = Router::new()
        .route("/theme.css", get(theme_css))
        .route("/theme.js", get(theme_js))
        .route("/api/theme", get(theme_get).put(theme_put))
        .route("/api/health", get(health))
        .route("/api/files", get(files))
        .route("/api/files/{id}", get(file))
        .route("/api/neighbors/{id}", get(neighbors))
        .route("/api/open/{id}", post(open))
        .route("/api/reveal/{id}", post(reveal))
        .route("/api/thumb/{id}", get(thumb))
        .route("/api/thumb/ds/{id}/{index}", get(member_thumb))
        .route("/api/timeline", get(timeline))
        .route("/api/categories", get(categories))
        .route("/api/files/{id}/categories", axum::routing::put(set_categories))
        .route("/api/nearest", get(nearest))
        .route("/api/search", get(search_h))
        .route("/api/echo/open", post(echo_open))
        .route("/api/act", post(act))
        .route("/api/events", get(events))
        .route("/api/default-focus", get(default_focus))
        .route("/api/datasets/{id}/items", get(dataset_items))
        .with_state(shared.clone());
    let web = Router::new()
        .fallback_service(ServeDir::new(&shared.web).append_index_html_on_directories(true))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache")));
    let mut kit = Router::new();
    for (path, mime, body) in lore_shell::FILES {
        kit = kit.route(path, get(move || async move { ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body) }));
    }
    api.merge(kit).fallback_service(web)
}

/// Bind 127.0.0.1 (`port`, else any free port) and register in ports.json as `ls`.
pub async fn bind(shared: Shared, port: u16) -> std::io::Result<(std::net::SocketAddr, impl std::future::Future<Output = std::io::Result<()>>)> {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    let addr = listener.local_addr()?;
    lore_home::ports::register(PORT_NAME, addr.port());
    let app = router(shared);
    Ok((addr, async move { axum::serve(listener, app).await }))
}

async fn theme_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::CSS)
}

async fn theme_js(State(s): State<Shared>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::script(&s.home))
}

async fn theme_get(State(s): State<Shared>) -> Json<Value> {
    Json(json!({ "theme": s.home.theme() }))
}

async fn theme_put(State(s): State<Shared>, Json(b): Json<Value>) -> Response {
    match s.home.set_theme(b["theme"].as_str().unwrap_or_default()) {
        Ok(t) => Json(json!({ "theme": t })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

async fn health(State(s): State<Shared>) -> Json<Value> {
    let idx = s.index();
    Json(json!({ "ok": true, "app": "LoreShelf", "build": build_stamp(), "items": idx.entries.len(), "kinds": idx.kinds() }))
}

#[derive(serde::Deserialize)]
struct ListQ {
    limit: Option<usize>,
    cat: Option<String>,
}

async fn files(State(s): State<Shared>, Query(q): Query<ListQ>) -> Json<Value> {
    let idx = s.index();
    Json(json!(idx.list(q.limit.unwrap_or(10_000), &Filter::parse(q.cat.as_deref()))))
}

async fn categories(State(s): State<Shared>) -> Json<Value> {
    Json(json!(s.index().categories()))
}

/// Body `{"categories": ["paper", …]}` (use categories; `[]` = none, replaces the rules).
async fn set_categories(State(s): State<Shared>, UrlPath(id): UrlPath<String>, Json(b): Json<Value>) -> Response {
    if s.index().get(&id).is_none() {
        return err(StatusCode::NOT_FOUND, "找不到這個項目");
    }
    let Some(list) = b["categories"].as_array() else { return err(StatusCode::BAD_REQUEST, "需要 categories 陣列") };
    let mut ids: Vec<String> = Vec::new();
    for v in list {
        let Some(c) = v.as_str().filter(|c| lore_catalog::category::group(c) == Some(lore_catalog::category::GROUP_USE)) else {
            return err(StatusCode::BAD_REQUEST, "未知的用途分類");
        };
        if !ids.iter().any(|x| x == c) {
            ids.push(c.to_string());
        }
    }
    let cat = lore_catalog::Catalog { dir: s.catalog.clone() };
    let (id2, ids2) = (id.clone(), ids.clone());
    match tokio::task::spawn_blocking(move || cat.record_edit(&id2, "categories", json!(ids2))).await {
        Ok(Ok(commit)) => {
            s.invalidate();
            Json(json!({ "id": id, "categories": ids, "commit": commit })).into_response()
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct NearQ {
    from: String,
    cat: Option<String>,
}

async fn nearest(State(s): State<Shared>, Query(q): Query<NearQ>) -> Response {
    let idx = s.index();
    let f = Filter::parse(q.cat.as_deref());
    match idx.nearest(&q.from, &f) {
        Some((item, score, parts)) => Json(json!({ "item": item, "score": (score * 1000.0).round() / 1000.0, "parts": parts })).into_response(),
        None => err(StatusCode::NOT_FOUND, "沒有符合的項目"),
    }
}

/// Which build this is: the program file's modification time. The desktop app only joins a
/// server of its own build (an older copy still running would lack what the page now asks for).
pub fn build_stamp() -> u64 {
    static STAMP: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *STAMP.get_or_init(|| {
        std::env::current_exe().and_then(std::fs::metadata).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
    })
}

/// `{ "focus": id }` from another local app (LoreEcho): the page turns the globe to that file.
async fn act(State(s): State<Shared>, Json(b): Json<Value>) -> Response {
    let Some(id) = b["focus"].as_str() else { return err(StatusCode::BAD_REQUEST, "需要 focus") };
    if s.index().get(id).is_none() {
        return err(StatusCode::NOT_FOUND, "找不到這個項目");
    }
    s.acts.send_modify(|v| *v = (v.0 + 1, json!({ "focus": id })));
    Json(json!({ "ok": true })).into_response()
}

#[derive(serde::Deserialize)]
struct EventsQ {
    after: Option<u64>,
}

/// How long `GET /api/events` waits for the next request before answering "nothing yet".
const EVENTS_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

/// Long poll: the next [`act`] request after number `after` (`{ seq, act }`; `act: null` after
/// [`EVENTS_WAIT`]). The page asks again with the `seq` it got; nothing runs while it waits.
async fn events(State(s): State<Shared>, Query(q): Query<EventsQ>) -> Json<Value> {
    let mut rx = s.acts.subscribe();
    let now = rx.borrow_and_update().clone();
    let Some(after) = q.after else { return Json(json!({ "seq": now.0, "act": null })) };
    if now.0 > after {
        return Json(json!({ "seq": now.0, "act": now.1 }));
    }
    match tokio::time::timeout(EVENTS_WAIT, rx.changed()).await {
        Ok(Ok(())) => {
            let v = rx.borrow().clone();
            Json(json!({ "seq": v.0, "act": v.1 }))
        }
        _ => Json(json!({ "seq": now.0, "act": null })),
    }
}

#[derive(serde::Deserialize)]
struct SearchQ {
    q: Option<String>,
    cat: Option<String>,
    limit: Option<usize>,
    /// The query is a spoken sentence (LoreEcho): [`search::loose`].
    loose: Option<String>,
}

/// Guided search (`search.rs`): `{ total, items: [{ item, via, snippet }], facets }`.
async fn search_h(State(s): State<Shared>, Query(q): Query<SearchQ>) -> Json<Value> {
    let idx = s.index();
    let f = Filter::parse(q.cat.as_deref());
    if q.loose.is_some() {
        let found = search::loose(&idx, q.q.as_deref().unwrap_or_default(), &f, q.limit.unwrap_or(5));
        let items: Vec<Value> = found.iter().map(|x| json!({ "item": x.item, "via": x.via, "score": (x.score * 10.0).round() / 10.0, "at": x.at })).collect();
        return Json(json!({ "total": items.len(), "items": items, "facets": [] }));
    }
    let (found, total, facets) = search::search(&idx, q.q.as_deref().unwrap_or_default(), &f, q.limit.unwrap_or(30));
    let items: Vec<Value> = found.iter().map(|x| json!({ "item": x.item, "via": x.via, "snippet": x.snippet, "at": x.at })).collect();
    Json(json!({ "total": total, "items": items, "facets": facets }))
}

/// LoreEcho's open window next to the shelf: the narrowest that still shows the character
/// (its page drops the figure at 520 px and below), this tall.
pub const ECHO_W: f64 = 540.0;
pub const ECHO_H: f64 = 560.0;
/// LoreEcho's bubble (its saved place is the bubble's; the open window shares its bottom-right corner).
const ECHO_BUBBLE: f64 = 64.0;
/// How long to wait for LoreEcho's server after launching it.
const ECHO_WAIT: std::time::Duration = std::time::Duration::from_secs(12);

/// Bring LoreEcho up next to the shelf (the 開啟時同步開啟 LoreEcho setting), without taking the
/// front. With `{ "right", "bottom" }` (screen points of the corner it should sit in, `"height"`
/// optional) its window is then opened there at its narrowest, through its own
/// `PUT /api/window` (ports.json `le`).
async fn echo_open(State(s): State<Shared>, body: Bytes) -> Response {
    if !cfg!(target_os = "macos") {
        return err(StatusCode::NOT_IMPLEMENTED, "只在 macOS");
    }
    let b: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let r = run(&s, vec!["-g".into(), "-a".into(), ECHO_APP.into()], "opened");
    if let (Some(right), Some(bottom)) = (b["right"].as_f64(), b["bottom"].as_f64()) {
        let patch = echo_place(right, bottom, b["height"].as_f64());
        std::thread::spawn(move || {
            let until = std::time::Instant::now() + ECHO_WAIT;
            loop {
                if let Some(port) = lore_home::ports::live().get("le").copied() {
                    if let Err(e) = put_json(port, "/api/window", &patch) {
                        eprintln!("loreshelf: LoreEcho window: {e}");
                    }
                    return;
                }
                if std::time::Instant::now() > until {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        });
    }
    r
}

/// LoreEcho's window patch for a corner at (`right`, `bottom`).
pub fn echo_place(right: f64, bottom: f64, height: Option<f64>) -> Value {
    json!({ "x": right - ECHO_BUBBLE, "y": bottom - ECHO_BUBBLE, "w": ECHO_W, "h": height.unwrap_or(ECHO_H).clamp(420.0, 1000.0), "expanded": true })
}

/// A small `PUT` with a JSON body to another Lore app on this machine.
fn put_json(port: u16, path: &str, body: &Value) -> Result<(), String> {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let body = body.to_string();
    let req = format!("PUT {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let line = out.lines().next().unwrap_or_default().to_string();
    if line.contains(" 200") { Ok(()) } else { Err(line) }
}

#[derive(serde::Deserialize)]
struct PageQ {
    offset: Option<usize>,
    limit: Option<usize>,
}

async fn dataset_items(State(s): State<Shared>, UrlPath(id): UrlPath<String>, Query(q): Query<PageQ>) -> Response {
    let idx = s.index();
    let Some(e) = idx.get(&id).filter(|e| e.item.kind == "dataset") else { return err(StatusCode::NOT_FOUND, "找不到這個資料集") };
    let Some(root) = dataset::root_of(&e.paths) else { return err(StatusCode::GONE, "資料夾已不在記錄的位置") };
    let s2 = s.clone();
    let m = tokio::task::spawn_blocking(move || s2.members(&id, &root)).await.unwrap_or_default();
    let (offset, limit) = (q.offset.unwrap_or(0), q.limit.unwrap_or(60).clamp(1, 500));
    let items: Vec<Value> = m.iter().enumerate().skip(offset).take(limit).map(|(i, x)| json!({ "index": i, "name": x.name, "rel": x.rel, "size": x.size })).collect();
    Json(json!({ "total": m.len(), "offset": offset, "items": items })).into_response()
}

async fn file(State(s): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    let idx = s.index();
    let Some(e) = idx.get(&id) else { return err(StatusCode::NOT_FOUND, "找不到這個項目") };
    let mut v = json!(e.item);
    v["paths"] = json!(e.paths);
    v["description"] = json!(e.description);
    v["category_hits"] = json!(e.hits);
    v["mentions"] = json!(e.mentions.iter().map(|&i| idx.entries[i].item.id.clone()).collect::<Vec<_>>());
    if e.item.kind == "dataset"
        && let Some(root) = dataset::root_of(&e.paths)
    {
        let s2 = s.clone();
        let id2 = id.clone();
        let sum = tokio::task::spawn_blocking(move || {
            let m = s2.members(&id2, &root);
            dataset::summary(&root, &m)
        })
        .await
        .ok();
        v["dataset"] = json!(sum);
    }
    Json(v).into_response()
}

#[derive(serde::Deserialize)]
struct NeighQ {
    k: Option<usize>,
    layers: Option<u32>,
    cat: Option<String>,
}

async fn neighbors(State(s): State<Shared>, UrlPath(id): UrlPath<String>, Query(q): Query<NeighQ>) -> Response {
    let idx = s.index();
    let k = q.k.unwrap_or(12).clamp(1, 64);
    let layers = q.layers.unwrap_or(2).clamp(1, 3);
    let f = Filter::parse(q.cat.as_deref());
    match tokio::task::spawn_blocking(move || idx.neighbors(&id, k, layers, &f)).await {
        Ok(Some(v)) => Json(json!(v)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "找不到這個項目"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// The first recorded path that still exists.
fn existing_path(s: &AppState, id: &str) -> Result<(String, String), (StatusCode, &'static str)> {
    let idx = s.index();
    let e = idx.get(id).ok_or((StatusCode::NOT_FOUND, "找不到這個項目"))?;
    let p = e.paths.iter().find(|p| std::fs::symlink_metadata(p).is_ok()).ok_or((StatusCode::GONE, "檔案已不在記錄的位置"))?;
    Ok((p.clone(), e.item.ext.clone()))
}

fn opener() -> &'static str {
    if cfg!(target_os = "macos") { "open" } else { "xdg-open" }
}

async fn open(State(s): State<Shared>, UrlPath(id): UrlPath<String>, _b: Bytes) -> Response {
    let (path, ext) = match existing_path(&s, &id) {
        Ok(x) => x,
        Err((code, msg)) => return err(code, msg),
    };
    // A downloaded app is never launched from here: show it instead.
    if ext == "app" {
        return run(&s, vec!["-R".into(), path], "revealed");
    }
    // PDF → LoreReader, .pptx → SlideComposer, .docx / .tex / LaTeX project → PaperComposer (`openwith`)
    let kind = s.index().get(&id).map(|e| e.item.kind.clone()).unwrap_or_default();
    let (args, _app) = openwith::args(&kind, &ext, std::path::Path::new(&path), &|a| s.runner.installed(a));
    run(&s, args, "opened")
}

async fn reveal(State(s): State<Shared>, UrlPath(id): UrlPath<String>, _b: Bytes) -> Response {
    let path = match existing_path(&s, &id) {
        Ok((p, _)) => p,
        Err((code, msg)) => return err(code, msg),
    };
    if !cfg!(target_os = "macos") {
        let dir = std::path::Path::new(&path).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or(path);
        return run(&s, vec![dir], "revealed");
    }
    run(&s, vec!["-R".into(), path], "revealed")
}

#[derive(serde::Deserialize)]
struct ThumbQ {
    s: Option<u32>,
}

async fn thumb(State(st): State<Shared>, UrlPath(id): UrlPath<String>, Query(q): Query<ThumbQ>) -> Response {
    let size = q.s.unwrap_or(thumbs::DEFAULT_SIZE).clamp(32, 1024);
    let png = |b: Vec<u8>| ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "private, max-age=3600")], b).into_response();
    let none = || err(StatusCode::NOT_FOUND, "沒有縮圖");
    let idx = st.index();
    let Some(e) = idx.get(&id) else { return err(StatusCode::NOT_FOUND, "找不到這個項目") };
    match thumbs::cached(&st.thumbs, &id, size) {
        Some(Some(b)) => return png(b),
        Some(None) => return none(),
        None => {}
    }
    if e.item.kind == "dataset" {
        let Some(root) = dataset::root_of(&e.paths) else { return none() };
        let Ok(_permit) = st.ql.acquire().await else { return none() };
        let st2 = st.clone();
        return match tokio::task::spawn_blocking(move || mosaic(&st2, &id, &root, size)).await {
            Ok(Some(b)) => png(b),
            _ => none(),
        };
    }
    let Some(src) = thumbs::source(e) else { return none() };
    let Ok(_permit) = st.ql.acquire().await else { return none() };
    // Another request may have made it while this one waited.
    if let Some(Some(b)) = thumbs::cached(&st.thumbs, &id, size) {
        return png(b);
    }
    let (runner, dir, id2) = (st.runner.clone(), st.thumbs.clone(), id.clone());
    match tokio::task::spawn_blocking(move || thumbs::make(runner.as_ref(), &dir, &id2, size, &src)).await {
        Ok(Some(b)) => png(b),
        _ => none(),
    }
}

/// One member's thumbnail (cached as `<id>-m<index>`). Blocking.
fn member_png(st: &AppState, id: &str, root: &std::path::Path, members: &[dataset::Member], index: usize, size: u32) -> Option<Vec<u8>> {
    let key = format!("{id}-m{index}");
    if let Some(c) = thumbs::cached(&st.thumbs, &key, size) {
        return c;
    }
    let p = root.join(&members.get(index)?.rel);
    if !thumbs::safe_file(&p) {
        return None;
    }
    thumbs::make(st.runner.as_ref(), &st.thumbs, &key, size, &p)
}

/// The 2×2 mosaic of evenly spaced member images. Blocking.
fn mosaic(st: &AppState, id: &str, root: &std::path::Path, size: u32) -> Option<Vec<u8>> {
    if let Some(Some(b)) = thumbs::cached(&st.thumbs, id, size) {
        return Some(b);
    }
    let m = st.members(id, root);
    let tiles: Vec<Vec<u8>> = dataset::spread_images(&m, 4).into_iter().filter_map(|i| member_png(st, id, root, &m, i, (size / 2).max(32))).collect();
    let out = dataset::mosaic(&tiles, size);
    thumbs::store(&st.thumbs, id, size, out.as_deref());
    out
}

async fn member_thumb(State(st): State<Shared>, UrlPath((id, index)): UrlPath<(String, usize)>, Query(q): Query<ThumbQ>) -> Response {
    let size = q.s.unwrap_or(256).clamp(32, 1024);
    let idx = st.index();
    let Some(e) = idx.get(&id).filter(|e| e.item.kind == "dataset") else { return err(StatusCode::NOT_FOUND, "找不到這個資料集") };
    let Some(root) = dataset::root_of(&e.paths) else { return err(StatusCode::NOT_FOUND, "沒有縮圖") };
    let Ok(_permit) = st.ql.acquire().await else { return err(StatusCode::NOT_FOUND, "沒有縮圖") };
    let st2 = st.clone();
    let r = tokio::task::spawn_blocking(move || {
        let m = st2.members(&id, &root);
        member_png(&st2, &id, &root, &m, index, size)
    })
    .await;
    match r {
        Ok(Some(b)) => ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "private, max-age=3600")], b).into_response(),
        _ => err(StatusCode::NOT_FOUND, "沒有縮圖"),
    }
}

#[derive(serde::Deserialize)]
struct CatQ {
    cat: Option<String>,
}

/// `{id}` of the item the view starts on (`null` when nothing fits).
async fn default_focus(State(s): State<Shared>, Query(q): Query<CatQ>) -> Json<Value> {
    let idx = s.index();
    Json(json!({ "id": idx.default_focus(&Filter::parse(q.cat.as_deref())).map(|i| i.id.clone()) }))
}

async fn timeline(State(s): State<Shared>, Query(q): Query<CatQ>) -> Json<Value> {
    let now = chrono::Local::now().fixed_offset();
    Json(json!(timeline::buckets(&s.index(), now, &Filter::parse(q.cat.as_deref()))))
}

fn run(s: &AppState, args: Vec<String>, what: &str) -> Response {
    match s.runner.run(opener(), &args) {
        Ok(()) => Json(json!({ what: true })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

#[cfg(test)]
mod tests;
