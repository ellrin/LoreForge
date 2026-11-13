//! 寫書 (docs/book.md): a LoreReader paper → course notes in a book (a PaperComposer document with
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
//! | POST | /api/book | `{source: {kind: "lr", sha}, book: "new" \| id, length, lang, mode: "review" \| "direct", extras?, folder?, page?, engine?}` → `{job, …job object}` |
//! | GET | /api/book/jobs | `?unfinished=1` → `{jobs}` |
//! | GET | /api/book/jobs/{id} | `?after=<seq>` long poll (25 s) → job object |
//! | GET | /api/book/jobs/{id}/events | same as `?after=` |
//! | GET | /api/book/jobs/{id}/stages/{stage} | a checkpoint |
//! | POST | /api/book/jobs/{id}/review | `{chapters: [{id, title, depth, include, terms?}]}` or `{skip: true}` |
//! | POST | /api/book/jobs/{id}/params | `{length?, lang?, page?}`: length or language → the plan again |
//! | POST | /api/book/jobs/{id}/stop · /resume | `{engine?}` on resume |
//! | GET · POST | /api/book/jobs/{id}/imports | `{pending}` · hand the waiting PDFs to LoreReader again → `{imported, pending, failed}` |
//! | GET | /api/book/jobs/{id}/open | the finished book's id in opened.toml → `{id, path}` |
//! | DELETE | /api/book/jobs/{id} | the job's cache folder |
//!
//! `/api/book*`, `/api/books` and `/api/info` answer 127.0.0.1 / localhost origins (CORS), like
//! SlideComposer's `/api/todeck*`, so LoreReader's 寫 review card can call them. Errors: status +
//! `{error}` in one line of Chinese.

pub mod model;
pub mod pipeline;
pub mod plan;
pub mod research;
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
    /// PaperComposer's workspace (references library, book template).
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
}

impl Deps {
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
        let reader_url = std::env::var("LOREREADER_URL").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| format!("http://127.0.0.1:{lr_port}"));
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
            web: true,
            extract: Arc::new(source::DocsrcExtract),
            tools: Arc::new(tools::RealTools),
            plot_styles: lore_plot::style::list(&home)["styles"].clone(),
            books_file: crate::books::registry_file(),
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
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
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

#[derive(Debug, Deserialize)]
struct SourceRef {
    kind: String,
    #[serde(default)]
    sha: String,
}

#[derive(Debug, Deserialize)]
struct MakeBody {
    source: SourceRef,
    #[serde(default)]
    book: Option<String>,
    #[serde(default)]
    length: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    extras: Option<source::Extras>,
    #[serde(default)]
    folder: Option<PathBuf>,
    #[serde(default)]
    page: Option<String>,
    #[serde(default)]
    engine: Option<String>,
}

const LENGTHS: &[&str] = &["short", "standard", "long"];
const LANGS: &[&str] = &["en-zh", "en", "zh"];

fn view_with_id(js: &Jobs, job: &Job) -> Value {
    let mut v = js.view(job);
    v["job"] = job.id.clone().into();
    v
}

async fn create(State(s): State<Shared>, Json(b): Json<MakeBody>) -> Response {
    if b.source.kind != "lr" || !lore_learn::reader::valid_sha(&b.source.sha) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "不認得的來源");
    }
    let length = b.length.unwrap_or_else(|| "standard".into());
    let lang = b.lang.unwrap_or_else(|| "en-zh".into());
    let page = b.page.unwrap_or_else(|| "b5".into());
    if !LENGTHS.contains(&length.as_str()) || !LANGS.contains(&lang.as_str()) || !pc_core::book::PAGES.contains(&page.as_str()) {
        return err(StatusCode::BAD_REQUEST, "不認得的篇幅、語言或頁面大小");
    }
    let book = b.book.filter(|x| !x.is_empty()).unwrap_or_else(|| "new".into());
    let (book_path, folder) = if book == "new" {
        let folder = b.folder.filter(|f| f.is_absolute()).or_else(crate::books::next_folder);
        if folder.is_none() {
            return err(StatusCode::BAD_REQUEST, "請選擇存放的資料夾");
        }
        (None, folder)
    } else {
        match crate::opened::registry().get(&book).map(|e| e.path.clone()) {
            Some(p) if pc_core::open::is_paper(&p) => (Some(p), None),
            _ => return err(StatusCode::NOT_FOUND, "找不到這本書"),
        }
    };
    let params = pipeline::Params { sha: b.source.sha, book, book_path, folder, length, lang, page, engine: b.engine.filter(|e| matches!(e.as_str(), "local" | "subscription" | "api" | "claude")), extras: b.extras.unwrap_or_default() };
    let js = jobs(&s).await;
    let direct = b.mode.as_deref() == Some("direct");
    match serde_json::to_value(params).map_err(|e| e.to_string()).and_then(|p| js.create(p, direct, "").map_err(|e| e.to_string())) {
        Ok(job) => Json(view_with_id(&js, &job)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("無法建立工作：{e}")),
    }
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
    if b["skip"] == true || b["skipped"] == true {
        b = json!({ "skipped": true });
    } else if b.get("chapters").is_some_and(|c| !c.is_array()) {
        return err(StatusCode::BAD_REQUEST, "chapters 要是陣列");
    } else if b["chapters"].as_array().is_some_and(|c| !c.iter().any(|r| r["include"] != false)) {
        return err(StatusCode::BAD_REQUEST, "至少要留一章");
    }
    if job.is_running() || job.status() != status::REVIEW {
        return err(StatusCode::CONFLICT, "這個工作沒有在等微調");
    }
    match js.submit(&job, "review", b) {
        Ok(()) => Json(js.view(&job)).into_response(),
        Err(m) => err(StatusCode::CONFLICT, m),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ParamsBody {
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
    let changed = |k: &str, v: &Option<String>| v.as_deref().is_some_and(|v| p[k].as_str() != Some(v));
    let redo = changed("length", &b.length) || changed("lang", &b.lang);
    js.set_params(&job, |p| {
        for (k, v) in [("length", &b.length), ("lang", &b.lang), ("page", &b.page)] {
            if let Some(v) = v {
                p[k] = v.clone().into();
            }
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

/// The page opened LoreReader: hand it the waiting PDFs (from this server; LoreReader has no CORS).
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
    r.notes.retain(|x| !x.starts_with("LoreReader 沒有開啟"));
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

