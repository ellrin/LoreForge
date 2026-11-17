//! 寫書 (docs/book.md): an L-Reader paper → course notes in a book (an L-Paper document with
//! `kind = "book"`), as a background job on lore-jobs: `source → terms → plan → research → review →
//! write → check` (`pipeline.rs`). Checkpoints live in the cache tier
//! (`<OS cache>/PaperComposer/book/<job>/`, `PAPERCOMPOSER_CACHE` overrides); the book itself is a
//! document where the author keeps it. Everything that writes LaTeX goes through the paper's own
//! paths (`floats.rs`, sections, references, the notation registry), so the book is edited exactly
//! like a paper afterwards.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/info | `{app: "pc", books}` |
//! | POST | /api/book | a `Setup` (`setup.rs`; the older `{source: {kind: "lr", sha}, …}` too) → `{job, …job object}`; no starting point → 400 |
//! | POST | /api/book/setup | a `Setup`, or nothing (the defaults) → `{setup: id}`, kept in memory for an hour |
//! | GET · PUT | /api/book/setup/{id} | the setup (`new` = the defaults), each source with its `label` · save the panel's changes |
//! | GET | /api/book/options | `{books: [{id, title}], folder, formats: [{id, label, pages}], papers: [{sha, title, year}]}` |
//! | GET | /api/book/cards | `?q=` → `{cards: [{id, title, state}]}`: cards that are not `known`, at most 30 |
//! | GET | /api/book/jobs | `?unfinished=1` → `{jobs}` |
//! | GET | /api/book/jobs/{id} | `?after=<seq>` long poll (25 s) → job object |
//! | GET | /api/book/jobs/{id}/events | same as `?after=` |
//! | GET | /api/book/jobs/{id}/stages/{stage} | a checkpoint |
//! | POST | /api/book/jobs/{id}/review | `{chapters: [{id, title, depth, include, terms?}], make_cards?}` or `{skip: true, make_cards?}` |
//! | POST | /api/book/jobs/{id}/params | `{length?, lang?, page?, folder?}`: length or language → the plan again; `folder` = where a new book goes (absolute; the last one used is the default) |
//! | POST | /api/book/jobs/{id}/stop · /resume | `{engine?}` on resume |
//! | GET · POST | /api/book/jobs/{id}/imports | `{pending}` · hand the waiting PDFs to L-Reader again → `{imported, pending, failed}` |
//! | GET | /api/book/jobs/{id}/open | the finished book's id in opened.toml → `{id, path}` |
//! | DELETE | /api/book/jobs/{id} | the job's cache folder |
//!
//! `/api/book*`, `/api/books` and `/api/info` answer 127.0.0.1 / localhost origins (CORS), like
//! L-Slide's `/api/todeck*`, so L-Reader's 寫 review card can call them. Errors: status +
//! `{error}` in one line of Chinese.

pub mod cards;
pub mod model;
pub mod pipeline;
pub mod plan;
pub mod research;
pub mod setup;
pub mod source;
pub mod tools;
pub mod write;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_jobs::{Job, Jobs, status};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;

/// Everything the job reads and talks to. The server builds it from its own state; tests build one
/// over temp folders with mocks.
pub struct Deps {
    /// The cache tier root of this feature (jobs in `jobs/`).
    pub cache: PathBuf,
    /// L-Paper's workspace (references library, book template).
    pub workspace: Option<PathBuf>,
    /// The global notation registry (read when used, never written).
    pub global_file: Option<PathBuf>,
    pub reader: Option<lore_learn::reader::Reader>,
    pub reader_url: String,
    pub kb_spaces: Vec<PathBuf>,
    pub member: String,
    pub filters: lore_words::Filters,
    pub model: Arc<dyn model::BookModel>,
    pub http: Arc<dyn lore_research::Http>,
    pub research: lore_research::Config,
    /// Web search allowed.
    pub web: bool,
    pub extract: Arc<dyn source::Extract>,
    pub tools: Arc<dyn tools::Tools>,
    /// lore-plot's styles (what `/plot/api/styles` lists).
    pub plot_styles: Value,
    pub books_file: PathBuf,
    /// The other Lore apps' addresses (`kb` L-Brain, `ls` L-Shelf). `None`: the live ports
    /// (ports.json) when asked; `Some` (tests, the isolated example): only these, never a running app.
    pub apps: Option<std::collections::BTreeMap<String, String>>,
}

impl Deps {
    /// Where app `id` (`kb`, `ls`) answers now, if it runs.
    pub fn app_url(&self, id: &str) -> Option<String> {
        match &self.apps {
            Some(m) => m.get(id).cloned(),
            None => lore_home::ports::live().get(id).map(|p| format!("http://127.0.0.1:{p}")),
        }
    }

    pub fn global(&self) -> lore_notation::Notation {
        self.global_file.as_ref().and_then(|p| lore_notation::Notation::load(p).ok()).unwrap_or_default()
    }

    /// The real ones for this server.
    pub(crate) fn for_server(s: &Shared, workspace: Option<PathBuf>) -> Deps {
        let home = crate::loreforge_home();
        let cache = std::env::var_os("PAPERCOMPOSER_CACHE").map(PathBuf::from).unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer")).join("book");
        let reader = lore_learn::reader::Reader::discover(&home);
        let filters = home.app_dir("LoreReader").map(|d| lore_words::Filters::with_file(&d.join("ignore-words.txt"))).unwrap_or_default();
        let lr_port = lore_home::ports::live().get("lr").copied().unwrap_or(7890);
        // the isolated example's scripted replies: no web, no running app is ever asked
        let isolated = std::env::var_os("PAPERCOMPOSER_LLM_MOCK").is_some();
        let reader_url = if isolated { "http://127.0.0.1:9".to_string() } else { std::env::var("LOREREADER_URL").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| format!("http://127.0.0.1:{lr_port}")) };
        let cfg = lore_research::Config::default();
        Deps {
            cache,
            global_file: workspace.as_deref().map(crate::notation::global_path),
            workspace,
            reader,
            reader_url,
            kb_spaces: lore_learn::space_dirs(&home),
            member: lore_learn::member(&home),
            filters,
            model: Arc::new(model::LlmModel(s.llm.clone())),
            http: Arc::new(lore_research::http::ReqwestHttp::new(&cfg.user_agent, cfg.timeout)),
            research: cfg,
            web: !isolated,
            extract: Arc::new(source::DocsrcExtract),
            tools: Arc::new(tools::RealTools),
            plot_styles: lore_plot::style::list(&home)["styles"].clone(),
            books_file: crate::books::registry_file(),
            // the isolated example never asks the author's running L-Brain or L-Shelf
            apps: (isolated || std::env::var_os("PAPERCOMPOSER_NO_LIVE_APPS").is_some()).then(Default::default),
        }
    }
}

static TEST_DEPS: OnceLock<Arc<Deps>> = OnceLock::new();

/// Tests of the HTTP routes: the job runs on these instead of the server's own.
pub fn use_deps(d: Arc<Deps>) {
    let _ = TEST_DEPS.set(d);
}

/// The jobs of this server (opened once; jobs running when the app ended are `interrupted`).
pub(crate) async fn jobs(s: &Shared) -> Arc<Jobs> {
    static JOBS: OnceLock<Arc<Jobs>> = OnceLock::new();
    if let Some(j) = JOBS.get() {
        return j.clone();
    }
    let deps = match TEST_DEPS.get() {
        Some(d) => d.clone(),
        None => {
            let ws = s.loaded.read().await.workspace.clone();
            Arc::new(Deps::for_server(s, ws))
        }
    };
    JOBS.get_or_init(|| Jobs::open(deps.cache.join("jobs"), Arc::new(pipeline::BookJob { deps: deps.clone() }))).clone()
}

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/info", get(info))
        .route("/api/book", post(create))
        .route("/api/book/setup", post(setup_new))
        .route("/api/book/setup/{id}", get(setup_get).put(setup_put))
        .route("/api/book/options", get(options))
        .route("/api/book/cards", get(cards))
        .route("/api/book/jobs", get(list).post(create))
        .route("/api/book/jobs/{id}", get(one).delete(remove))
        .route("/api/book/jobs/{id}/events", get(one))
        .route("/api/book/jobs/{id}/stages/{stage}", get(stage_output))
        .route("/api/book/jobs/{id}/stop", post(stop))
        .route("/api/book/jobs/{id}/resume", post(resume))
        .route("/api/book/jobs/{id}/review", post(review).put(review))
        .route("/api/book/jobs/{id}/params", post(change_params))
        .route("/api/book/jobs/{id}/imports", get(imports).post(retry_imports))
        .route("/api/book/jobs/{id}/open", get(open_book))
        .layer(local_cors())
}

/// Pages of the other Lore apps (127.0.0.1 / localhost on any port, the desktop webviews).
pub fn local_cors() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|o, _| local_origin(o.to_str().unwrap_or(""))))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::PUT])
        .allow_headers([axum::http::header::CONTENT_TYPE])
}

pub fn local_origin(o: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:"].iter().any(|p| o.strip_prefix(p).is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())))
        || o == "tauri://localhost"
        || o == "http://tauri.localhost"
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

async fn info(State(s): State<Shared>) -> Response {
    let b = crate::books::listing(&s).await;
    Json(json!({ "app": "pc", "books": b["books"], "folder": b["folder"] })).into_response()
}

const LENGTHS: &[&str] = &["short", "standard", "long"];
const LANGS: &[&str] = &["en-zh", "en", "zh"];

fn view_with_id(js: &Jobs, job: &Job) -> Value {
    let mut v = js.view(job);
    v["job"] = job.id.clone().into();
    v
}

async fn create(State(s): State<Shared>, body: axum::body::Bytes) -> Response {
    let v: Value = if body.iter().all(u8::is_ascii_whitespace) { json!({}) } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_REQUEST, format!("看不懂的設定：{e}")),
        }
    };
    let mut b = match setup::Setup::from_body(v) {
        Ok(b) => b,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    b.tidy();
    if !b.has_start() {
        return err(StatusCode::BAD_REQUEST, "沒有起點：選論文、檔案、資料夾，或打關鍵字、選知識卡");
    }
    if let Err(e) = b.check_sources() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, e);
    }
    if !LENGTHS.contains(&b.length.as_str()) || !LANGS.contains(&b.lang.as_str()) || !setup::FORMATS.contains(&b.format.as_str()) || (b.format == "notes" && !pc_core::book::PAGES.contains(&b.page.as_str())) {
        return err(StatusCode::BAD_REQUEST, "不認得的篇幅、語言、格式或頁面大小");
    }
    let (book_path, folder) = if b.book == "new" {
        let folder = b.folder.clone().filter(|f| f.is_absolute()).or_else(crate::books::next_folder);
        if folder.is_none() {
            return err(StatusCode::BAD_REQUEST, "請選擇存放的資料夾");
        }
        (None, folder)
    } else {
        match crate::opened::registry().get(&b.book).map(|e| e.path.clone()) {
            Some(p) if pc_core::open::is_paper(&p) => (Some(p), None),
            _ => return err(StatusCode::NOT_FOUND, "找不到這本書"),
        }
    };
    let params = pipeline::Params::from_setup(&b, book_path, folder);
    let title = params.starts().first().map(|x| x.label()).filter(|t| !t.is_empty()).unwrap_or_else(|| b.keywords.join(", "));
    let js = jobs(&s).await;
    let direct = b.mode == "direct";
    match serde_json::to_value(params).map_err(|e| e.to_string()).and_then(|p| js.create(p, direct, &title).map_err(|e| e.to_string())) {
        Ok(job) => Json(view_with_id(&js, &job)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("無法建立工作：{e}")),
    }
}

// ── 開始面板 ─────────────────────────────────────────────────────────────────

/// `POST /api/book/setup`: keep a setup (an empty body = the defaults) → `{setup: id}`.
async fn setup_new(body: axum::body::Bytes) -> Response {
    let v: Value = if body.iter().all(u8::is_ascii_whitespace) { json!({}) } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_REQUEST, format!("看不懂的設定：{e}")),
        }
    };
    let mut st = match setup::Setup::from_body(v) {
        Ok(x) => x,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    st.tidy();
    if st.folder.is_none() {
        st.folder = crate::books::next_folder();
    }
    Json(json!({ "setup": setup::put_new(st) })).into_response()
}

/// `GET /api/book/setup/{id}` (`new` = the defaults).
async fn setup_get(AxPath(id): AxPath<String>) -> Response {
    if id == "new" {
        let st = setup::Setup { folder: crate::books::next_folder(), ..Default::default() };
        return Json(st.view()).into_response();
    }
    match setup::get(&id) {
        Some(st) => Json(st.view()).into_response(),
        None => err(StatusCode::NOT_FOUND, "開始面板的設定已經過期"),
    }
}

/// `PUT /api/book/setup/{id}`: the panel saves what the author changed.
async fn setup_put(AxPath(id): AxPath<String>, Json(v): Json<Value>) -> Response {
    let mut st = match setup::Setup::from_body(v) {
        Ok(x) => x,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    st.tidy();
    if !setup::replace(&id, st.clone()) {
        return err(StatusCode::NOT_FOUND, "開始面板的設定已經過期");
    }
    Json(st.view()).into_response()
}

/// `GET /api/book/options`: what the panel offers.
async fn options(State(s): State<Shared>) -> Response {
    let b = crate::books::listing(&s).await;
    let books: Vec<Value> = b["books"].as_array().into_iter().flatten().map(|x| json!({ "id": x["id"], "title": x["title"] })).collect();
    let deps = jobs_deps(&s).await;
    let d = deps.clone();
    let papers = tokio::task::spawn_blocking(move || reader_papers(d.reader.as_ref())).await.unwrap_or_default();
    Json(json!({ "books": books, "folder": b["folder"], "formats": formats(), "papers": papers })).into_response()
}

/// The formats a book can have (`pages`: the page sizes it offers; review papers are A4).
pub fn formats() -> Value {
    json!([
        { "id": "notes", "label": "講義", "pages": pc_core::book::PAGES },
        { "id": "review1", "label": "Review（單欄）", "pages": ["a4"] },
        { "id": "review2", "label": "Review（雙欄）", "pages": ["a4"] },
    ])
}

/// L-Reader's papers, newest first: `{sha, title, year}`.
pub fn reader_papers(r: Option<&lore_learn::reader::Reader>) -> Vec<Value> {
    let Some(r) = r else { return vec![] };
    let mut list = r.list();
    list.sort_by(|a, b| b["added_at"].as_str().unwrap_or("").cmp(a["added_at"].as_str().unwrap_or("")));
    list.iter()
        .filter_map(|p| {
            let sha = p["sha256"].as_str().filter(|x| lore_learn::reader::valid_sha(x))?;
            let title = p["title"].as_str().map(str::trim).filter(|t| !t.is_empty())?;
            let year = p["year"].as_i64().map(|y| y.to_string()).or_else(|| p["year"].as_str().map(String::from)).unwrap_or_default();
            Some(json!({ "sha": sha, "title": title, "year": year }))
        })
        .collect()
}

/// `GET /api/book/cards?q=`: cards the member does not know yet (`known` left out), at most 30.
async fn cards(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let deps = jobs_deps(&s).await;
    let q = q.get("q").cloned().unwrap_or_default();
    let out = tokio::task::spawn_blocking(move || unknown_cards(&deps.kb_spaces, &deps.member, &q, 30)).await.unwrap_or_default();
    Json(json!({ "cards": out })).into_response()
}

/// Cards whose title or an alias contains `q` (all when empty), not `known` for `member`.
pub fn unknown_cards(spaces: &[PathBuf], member: &str, q: &str, max: usize) -> Vec<Value> {
    let q = q.trim().to_lowercase();
    let names = lore_learn::load_names(spaces, member);
    let mut seen: Vec<&str> = vec![];
    let mut out: Vec<(String, Value)> = vec![];
    for n in &names {
        if n.state == lore_learn::Familiar::Known || seen.contains(&n.card.as_str()) {
            continue;
        }
        if !q.is_empty() && !names.iter().any(|m| m.card == n.card && m.name.to_lowercase().contains(&q)) {
            continue;
        }
        seen.push(&n.card);
        let state = if n.state == lore_learn::Familiar::Learning { "learning" } else { "unknown" };
        out.push((n.title.to_lowercase(), json!({ "id": n.card, "title": n.title, "state": state })));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().take(max).map(|(_, v)| v).collect()
}

async fn find(s: &Shared, id: &str) -> Result<(Arc<Jobs>, Arc<Job>), Response> {
    let js = jobs(s).await;
    if !lore_jobs::valid_id(id) {
        return Err(err(StatusCode::NOT_FOUND, "沒有這個工作"));
    }
    let job = js.get(id).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個工作"))?;
    Ok((js, job))
}

async fn list(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let unfinished = q.get("unfinished").is_some_and(|v| v == "1" || v == "true");
    let js = jobs(&s).await;
    let out: Vec<Value> = js
        .list()
        .iter()
        .filter(|j| !unfinished || matches!(j.status().as_str(), status::INTERRUPTED | status::STOPPED | status::FAILED | status::REVIEW))
        .map(|j| js.view(j))
        .collect();
    Json(json!({ "jobs": out })).into_response()
}

/// The job object; with `?after=<seq>` it waits (at most 25 s) until the job moves past it.
async fn one(State(s): State<Shared>, AxPath(id): AxPath<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Some(after) = q.get("after").or_else(|| q.get("since")).and_then(|v| v.parse::<u64>().ok()) {
        job.changed_after(after, Duration::from_secs(25)).await;
    }
    Json(js.view(&job)).into_response()
}

async fn stage_output(State(s): State<Shared>, AxPath((id, stage)): AxPath<(String, String)>) -> Response {
    let (_, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !pipeline::STAGES.iter().any(|st| st.id == stage) {
        return err(StatusCode::NOT_FOUND, "沒有這個階段");
    }
    match job.output(&stage) {
        Some(v) => Json(v).into_response(),
        None => err(StatusCode::NOT_FOUND, "這個階段還沒完成"),
    }
}

async fn stop(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    js.stop(&job).await;
    Json(js.view(&job)).into_response()
}

#[derive(Debug, Default, Deserialize)]
struct ResumeBody {
    #[serde(default)]
    engine: Option<String>,
}

async fn resume(State(s): State<Shared>, AxPath(id): AxPath<String>, body: Option<Json<ResumeBody>>) -> Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if job.is_running() {
        return err(StatusCode::CONFLICT, "工作正在進行");
    }
    if let Some(e) = body.and_then(|Json(b)| b.engine).filter(|e| matches!(e.as_str(), "local" | "subscription" | "api" | "claude")) {
        js.set_params(&job, |p| p["engine"] = e.into());
    }
    js.start(&job);
    Json(js.view(&job)).into_response()
}

async fn review(State(s): State<Shared>, AxPath(id): AxPath<String>, body: Option<Json<Value>>) -> Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let mut b = body.map(|Json(v)| v).unwrap_or(json!({}));
    if !b.is_object() {
        return err(StatusCode::BAD_REQUEST, "要送 JSON 物件");
    }
    let make_cards = b["make_cards"].as_bool();
    if b["skip"] == true || b["skipped"] == true {
        b = json!({ "skipped": true });
        if let Some(m) = make_cards {
            b["make_cards"] = m.into();
        }
    } else if b.get("chapters").is_some_and(|c| !c.is_array()) {
        return err(StatusCode::BAD_REQUEST, "chapters 要是陣列");
    } else if b["chapters"].as_array().is_some_and(|c| !c.iter().any(|r| r["include"] != false)) {
        return err(StatusCode::BAD_REQUEST, "至少要留一章");
    }
    if job.is_running() || job.status() != status::REVIEW {
        return err(StatusCode::CONFLICT, "這個工作沒有在等微調");
    }
    if let Some(m) = make_cards {
        js.set_params(&job, |p| p["make_cards"] = m.into());
    }
    match js.submit(&job, "review", b) {
        Ok(()) => Json(js.view(&job)).into_response(),
        Err(m) => err(StatusCode::CONFLICT, m),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ParamsBody {
    /// A new book's folder (absolute), chosen in the review window.
    #[serde(default)]
    folder: Option<PathBuf>,
    #[serde(default)]
    length: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    page: Option<String>,
}

/// 篇幅／語言 redo the plan (the job comes back to the review); the page size only changes what
/// the write stage makes.
async fn change_params(State(s): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<ParamsBody>) -> Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if job.is_running() || job.status() != status::REVIEW {
        return err(StatusCode::CONFLICT, "這個工作沒有在等微調");
    }
    if b.length.as_deref().is_some_and(|l| !LENGTHS.contains(&l)) || b.lang.as_deref().is_some_and(|l| !LANGS.contains(&l)) || b.page.as_deref().is_some_and(|p| !pc_core::book::PAGES.contains(&p)) {
        return err(StatusCode::BAD_REQUEST, "不認得的篇幅、語言或頁面大小");
    }
    let p = job.meta().params;
    if let Some(f) = &b.folder {
        if p["book"].as_str() != Some("new") {
            return err(StatusCode::BAD_REQUEST, "加進現有的書：存放的資料夾就是那本書");
        }
        if !f.is_absolute() {
            return err(StatusCode::BAD_REQUEST, "資料夾要是完整路徑");
        }
    }
    let changed = |k: &str, v: &Option<String>| v.as_deref().is_some_and(|v| p[k].as_str() != Some(v));
    let redo = changed("length", &b.length) || changed("lang", &b.lang);
    js.set_params(&job, |p| {
        for (k, v) in [("length", &b.length), ("lang", &b.lang), ("page", &b.page)] {
            if let Some(v) = v {
                p[k] = v.clone().into();
            }
        }
        if let Some(f) = &b.folder {
            p["folder"] = f.display().to_string().into();
        }
    });
    if redo && let Err(m) = js.redo_from(&job, "plan") {
        return err(StatusCode::CONFLICT, m);
    }
    Json(js.view(&job)).into_response()
}

async fn imports(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    let (_, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let r: research::Research = job.output("research").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    Json(json!({ "pending": r.pending_imports })).into_response()
}

/// The page opened L-Reader: hand it the waiting PDFs (from this server; L-Reader has no CORS).
async fn retry_imports(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    let (_, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Some(mut r) = job.output("research").and_then(|v| serde_json::from_value::<research::Research>(v).ok()) else {
        return err(StatusCode::NOT_FOUND, "還沒有找資料");
    };
    let deps = jobs_deps(&s).await;
    let (n, pending, failed) = research::retry_imports(&*deps.http, &deps.reader_url, &r.pending_imports).await;
    r.imported += n;
    r.pending_imports = pending.clone();
    r.notes.retain(|x| !x.starts_with("L-Reader 沒有開啟"));
    if let Ok(v) = serde_json::to_value(&r) {
        let counts = json!({ "imported": r.imported, "pending": pending.len() });
        let _ = job.replace_output("research", v, counts);
    }
    Json(json!({ "imported": n, "pending": pending, "failed": failed })).into_response()
}

async fn jobs_deps(s: &Shared) -> Arc<Deps> {
    match TEST_DEPS.get() {
        Some(d) => d.clone(),
        None => {
            static D: OnceLock<Arc<Deps>> = OnceLock::new();
            if let Some(d) = D.get() {
                return d.clone();
            }
            let ws = s.loaded.read().await.workspace.clone();
            D.get_or_init(|| Arc::new(Deps::for_server(s, ws))).clone()
        }
    }
}

/// The finished book, registered in opened.toml (the id the page opens).
async fn open_book(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    let (_, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Some(book) = job.output("write").and_then(|v| v["book"].as_str().map(PathBuf::from)) else {
        return err(StatusCode::NOT_FOUND, "書還沒寫好");
    };
    if !pc_core::open::is_paper(&book) {
        return err(StatusCode::NOT_FOUND, "找不到這本書");
    }
    match crate::opened::register(&s, &book).await {
        Ok(oid) => Json(json!({ "id": oid, "path": book })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn remove(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    match js.delete(&job) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(m) => err(StatusCode::CONFLICT, m),
    }
}

