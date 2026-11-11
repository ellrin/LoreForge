//! 論文報告: a paper (a PaperComposer LaTeX folder or a LoreReader PDF) becomes a
//! paper-report deck, as a server-side job (`crate::jobs`) of eight stages
//! (docs/paper-deck.md): `source → notation → terms → profile → outline → review → slides → polish`.
//! Read-only on the paper; the deck is a normal deck.
//!
//! - `source` (`source.rs`, lore-docsrc): PaperComposer → `latex::extract_dir` (sections,
//!   figure files, table grids, equations with labels, bibliography) and the paper's own
//!   `notation.json`; LoreReader → `pdf::extract` on the library PDF (crops cached in this
//!   workspace's `cache/todeck/<sha>/`) and LoreReader's `symbols.json`. The pictures are kept
//!   in the job folder (`img/`).
//! - `notation`: the entries the deck registers (the paper's ids) and the registry macros.
//! - `terms` / `profile` / `polish`: other owners' functions (`terms.rs`, `profile.rs`,
//!   `polish.rs`; signatures documented there).
//! - `outline` (lore-llm task `sc-paper-deck`, class `create`, streamed): the slides in the
//!   author's paper-report order (design §7.1) referring to the material by id, then the module
//!   boxes of the main architecture figure (`sc-crop-modules`, vision, optional).
//! - `review`: the author's edits and answers (skipped with 直接產生).
//! - `slides` (`build.rs`): page kinds from the base, the author's academic arrangements, the
//!   style's tags, native tables, OMML equations (registry macros expanded), `Ref:` lines, the
//!   big picture expanded by `reveal`; the paper's notation joins the deck's `notation.json`.
//!
//! Routes: docs/paper-deck.md "API". `/api/todeck*` answer 127.0.0.1 / localhost origins (CORS).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::jobs::{self, Job, Jobs};
use crate::llm::Llm;
use crate::reader::Reader;
use crate::{AppState, R, Shared, bad};

pub mod build;
pub mod outline;
pub mod plan;
pub mod polish;
pub mod profile;
pub mod source;
pub mod stages;
pub mod terms;

pub use build::*;
pub use plan::*;
pub use source::*;
pub use stages::{Params, Source, apply_review};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/todeck/papers", get(papers))
        .route("/api/todeck", post(make))
        .route("/api/todeck/jobs", get(list).post(create))
        .route("/api/todeck/jobs/{id}", get(one).delete(remove))
        .route("/api/todeck/jobs/{id}/events", get(events))
        .route("/api/todeck/jobs/{id}/stages/{stage}", get(stage_output))
        .route("/api/todeck/jobs/{id}/img/{file}", get(image))
        .route("/api/todeck/jobs/{id}/stop", post(stop))
        .route("/api/todeck/jobs/{id}/cancel", post(stop))
        .route("/api/todeck/jobs/{id}/resume", post(resume))
        .route("/api/todeck/jobs/{id}/review", post(review).put(review))
}

pub(crate) fn llm(s: &AppState) -> &Llm {
    s.llm.get_or_init(|| Llm::new(s.home.clone()))
}

/// The jobs of this workspace (`cache/todeck/`), read from disk the first time: jobs that
/// were running when the app ended are `interrupted`.
pub fn jobs(s: &Shared) -> Arc<Jobs> {
    type Reg = Mutex<HashMap<PathBuf, (Weak<AppState>, Arc<Jobs>)>>;
    static REG: OnceLock<Reg> = OnceLock::new();
    let root = s.ws.root.join("cache/todeck");
    let mut reg = REG.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((w, j)) = reg.get(&root)
        && w.upgrade().is_some_and(|x| Arc::ptr_eq(&x, s))
    {
        return j.clone();
    }
    let j = Jobs::open(root.clone(), Arc::new(stages::PaperDeck::new(Arc::downgrade(s))));
    reg.insert(root, (Arc::downgrade(s), j.clone()));
    j
}

async fn papers(State(s): State<Shared>) -> R {
    let home = s.home.clone();
    let (pc, lr) = tokio::task::spawn_blocking(move || {
        let pc = pc_papers(&home);
        let lr: Vec<Value> = Reader::discover(&home)
            .map(|r| r.list().iter().map(|m| json!({ "sha": m["sha256"], "title": crate::reader::title_of(m), "pdf": r.pdf(m["sha256"].as_str().unwrap_or("")).is_some() })).collect())
            .unwrap_or_default();
        (pc, lr)
    })
    .await
    .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "pc": pc, "lr": lr })))
}

// ── Jobs over HTTP ───────────────────────────────────────────────────────────

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct MakeBody {
    pub source: Source,
    #[serde(default)]
    pub length: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    /// `review` (先看大綱) or `direct` (直接產生): answers `{job}` at once. Without it the old
    /// one-shot behaviour: a direct job, answered when it is done.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub direct: Option<bool>,
}

/// A new job from a request body (the base is checked now).
fn new_job(s: &Shared, b: &MakeBody, direct: bool) -> Result<Arc<Job>, Response> {
    let style = match &b.style {
        Some(st) if !st.is_empty() => st.clone(),
        _ => s.ws.style_ids().into_iter().next().ok_or_else(|| err(StatusCode::BAD_REQUEST, "沒有底版"))?,
    };
    s.ws.style(&style).map_err(|e| crate::ApiError::from(e).into_response())?;
    if !matches!(b.source.kind.as_str(), "pc" | "lr") {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "不認得的來源"));
    }
    let params = Params {
        source: b.source.clone(),
        length: b.length.clone().unwrap_or_else(|| "standard".into()),
        lang: b.lang.clone().unwrap_or_else(|| "zh".into()),
        style,
        engine: b.engine.clone().filter(|e| e == "local" || e == "claude"),
    };
    let title = match (&b.source.kind[..], &b.source.path) {
        ("pc", Some(p)) => std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        _ => String::new(),
    };
    let params = serde_json::to_value(params).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    jobs(s).create(params, direct, &title).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("無法建立工作：{e}")))
}

/// The job object plus `job` (its id), as `POST /api/todeck` with a mode answers.
fn view_with_id(s: &Shared, job: &Job) -> Value {
    let mut v = jobs(s).view(job);
    v["job"] = job.id.clone().into();
    v
}

async fn create(State(s): State<Shared>, Json(b): Json<MakeBody>) -> Response {
    let direct = b.direct.unwrap_or(false) || b.mode.as_deref() == Some("direct");
    match new_job(&s, &b, direct) {
        Ok(job) => (StatusCode::CREATED, Json(view_with_id(&s, &job))).into_response(),
        Err(r) => r,
    }
}

/// `POST /api/todeck`: with `mode`, a job (`{job, …job object}`); without, the old one-shot
/// call — a direct job, answered with its result once done (or its error).
async fn make(State(s): State<Shared>, Json(b): Json<MakeBody>) -> Response {
    if let Some(mode) = b.mode.clone() {
        return match new_job(&s, &b, mode == "direct") {
            Ok(job) => Json(view_with_id(&s, &job)).into_response(),
            Err(r) => r,
        };
    }
    let job = match new_job(&s, &b, true) {
        Ok(j) => j,
        Err(r) => return r,
    };
    job.settled().await;
    let v = jobs(&s).view(&job);
    match v["state"].as_str() {
        Some(jobs::status::DONE) => Json(v["result"].clone()).into_response(),
        _ => {
            let mut f = v["failure"].clone();
            let code = f["status"].as_u64().and_then(|c| StatusCode::from_u16(c as u16).ok()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            if let Some(o) = f.as_object_mut() {
                o.remove("status");
                o.remove("stage");
            }
            if f.is_null() {
                f = json!({ "error": "沒有完成" });
            }
            (code, Json(f)).into_response()
        }
    }
}

fn find(s: &Shared, id: &str) -> Result<Arc<Job>, Response> {
    if !jobs::valid_id(id) {
        return Err(err(StatusCode::NOT_FOUND, "沒有這個工作"));
    }
    jobs(s).get(id).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個工作"))
}

async fn list(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let unfinished = q.get("unfinished").is_some_and(|v| v == "1" || v == "true");
    let js = jobs(&s);
    let out: Vec<Value> = js
        .list()
        .iter()
        .filter(|j| !unfinished || matches!(j.status().as_str(), jobs::status::INTERRUPTED | jobs::status::STOPPED | jobs::status::FAILED | jobs::status::REVIEW))
        .map(|j| js.view(j))
        .collect();
    Json(json!({ "jobs": out })).into_response()
}

async fn one(State(s): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    match find(&s, &id) {
        Ok(j) => Json(jobs(&s).view(&j)).into_response(),
        Err(r) => r,
    }
}

/// Long-poll: answers when the job's `seq` passes `after` (or after 25 s).
async fn events(State(s): State<Shared>, UrlPath(id): UrlPath<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    let after = q.get("after").or_else(|| q.get("since")).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    job.changed_after(after, Duration::from_secs(25)).await;
    Json(jobs(&s).view(&job)).into_response()
}

async fn stage_output(State(s): State<Shared>, UrlPath((id, stage)): UrlPath<(String, String)>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    if !stages::STAGES.iter().any(|st| st.id == stage) {
        return err(StatusCode::NOT_FOUND, "沒有這個階段");
    }
    match job.output(&stage) {
        Some(v) => Json(v).into_response(),
        None => err(StatusCode::NOT_FOUND, "這個階段還沒完成"),
    }
}

async fn image(State(s): State<Shared>, UrlPath((id, file)): UrlPath<(String, String)>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    let ok = !file.is_empty() && !file.starts_with('.') && file.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    let ctype = match file.rsplit('.').next() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "",
    };
    if !ok || ctype.is_empty() {
        return err(StatusCode::NOT_FOUND, "沒有這張圖");
    }
    match std::fs::read(job.dir.join("img").join(&file)) {
        Ok(b) => ([(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "max-age=3600")], b).into_response(),
        Err(_) => err(StatusCode::NOT_FOUND, "沒有這張圖"),
    }
}

async fn stop(State(s): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    let js = jobs(&s);
    js.stop(&job).await;
    Json(js.view(&job)).into_response()
}

#[derive(Debug, Default, Deserialize)]
struct ResumeBody {
    #[serde(default)]
    engine: Option<String>,
}

async fn resume(State(s): State<Shared>, UrlPath(id): UrlPath<String>, body: Option<Json<ResumeBody>>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    let js = jobs(&s);
    if job.is_running() {
        return err(StatusCode::CONFLICT, "工作正在進行");
    }
    if let Some(e) = body.and_then(|Json(b)| b.engine).filter(|e| e == "local" || e == "claude") {
        js.set_params(&job, |p| p["engine"] = e.into());
    }
    js.start(&job);
    Json(js.view(&job)).into_response()
}

async fn review(State(s): State<Shared>, UrlPath(id): UrlPath<String>, body: Option<Json<Value>>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    let mut b = body.map(|Json(v)| v).unwrap_or(json!({}));
    if !b.is_object() {
        return err(StatusCode::BAD_REQUEST, "要送 JSON 物件");
    }
    if b["skip"] == true || b["skipped"] == true {
        b = json!({ "skipped": true });
    }
    if b.get("outline").is_some_and(|o| !o.is_array()) {
        return err(StatusCode::BAD_REQUEST, "outline 要是陣列");
    }
    let js = jobs(&s);
    match js.submit(&job, "review", b) {
        Ok(()) => Json(js.view(&job)).into_response(),
        Err(m) => err(StatusCode::CONFLICT, m),
    }
}

async fn remove(State(s): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    let job = match find(&s, &id) {
        Ok(j) => j,
        Err(r) => return r,
    };
    match jobs(&s).delete(&job) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(m) => err(StatusCode::CONFLICT, m),
    }
}

impl From<String> for crate::ApiError {
    fn from(e: String) -> Self {
        crate::ApiError(StatusCode::UNPROCESSABLE_ENTITY, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn typed_references_are_dropped() {
        assert_eq!(strip_typed_refs("As shown in Fig. 3, the model wins"), "As shown in, the model wins");
        assert_eq!(strip_typed_refs("Results (Table 2) are better"), "Results are better");
        assert_eq!(strip_typed_refs("Minimise Eq. (1) jointly"), "Minimise jointly");
        assert_eq!(strip_typed_refs("A figure of merit"), "A figure of merit");
    }

    #[test]
    fn macros_expand_until_stable() {
        let m: BTreeMap<String, String> = [("\\vx".to_string(), "\\mathbf{x}".to_string()), ("\\loss".to_string(), "\\mathcal{L}(\\vx)".to_string())].into();
        assert_eq!(expand_macros("\\loss = \\vxy + \\vx", &m), "\\mathcal{L}(\\mathbf{x}) = \\vxy + \\mathbf{x}");
    }

    #[test]
    fn jpeg_size_reads_sof() {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0, 0xFF, 0xC0, 0x00, 0x11, 8, 0x00, 0x20, 0x00, 0x40, 3];
        b.extend([0u8; 16]);
        assert_eq!(jpeg_size(&b), Some((64, 32)));
    }

    #[test]
    fn short_refs() {
        let f: BTreeMap<String, String> = [("author", "Ellrin, A. and Daidai, B. and Third, C."), ("title", "A {Synthetic} Study"), ("journal", "Journal of Tests"), ("year", "2025")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(source::short_ref(&f, ""), "Ellrin et al., A Synthetic Study, Journal of Tests, 2025");
    }

    #[test]
    fn review_edits_the_plan() {
        let m = Material {
            title: "T".into(),
            figures: vec![Fig { id: "fig1".into(), ..Default::default() }, Fig { id: "fig2".into(), ..Default::default() }],
            tables: vec![Fig { id: "tab1".into(), ..Default::default() }],
            ..Default::default()
        };
        let ps = |role: &str, title: &str| PlanSlide { role: role.into(), title: title.into(), ..Default::default() };
        let mut overview = ps("overview", "Big picture");
        overview.figure = "fig1".into();
        let mut results = ps("results", "Results");
        results.table = "tab1".into();
        results.takeaway = "Ours wins".into();
        let plan = Plan { slides: vec![ps("cover", "T"), overview, results, ps("opinion", "Some opinions"), ps("thanks", "Thanks")], ..Default::default() };
        let rv = json!({
            "outline": [
                { "id": "s1", "kind": "cover", "title": "T", "figs": [], "point": "" },
                { "id": "s3", "kind": "results", "title": "Main results", "figs": ["tab1", "fig2"], "point": "Ours wins by far" },
                { "id": "s2", "kind": "overview", "title": "Big picture", "figs": ["fig1"], "point": "" },
                { "id": "s4", "kind": "opinion", "title": "Some opinions", "figs": [], "point": "" },
                { "id": "s5", "kind": "thanks", "title": "Thanks", "figs": [], "point": "" }
            ],
            "answers": { "mainFig": "fig2", "opinion": false }
        });
        let out = apply_review(plan.clone(), &rv, &m);
        let roles: Vec<&str> = out.slides.iter().map(|s| s.role.as_str()).collect();
        assert_eq!(roles, ["cover", "results", "overview", "thanks"]);
        assert_eq!(out.slides[1].title, "Main results");
        assert_eq!(out.slides[1].takeaway, "Ours wins by far");
        assert_eq!((out.slides[1].figure.as_str(), out.slides[1].table.as_str()), ("fig2", "tab1"));
        assert_eq!(out.slides[2].figure, "fig2", "主架構圖 answer");
        assert_eq!(apply_review(plan.clone(), &json!({ "skipped": true }), &m).slides.len(), 5);
    }
}
