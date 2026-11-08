//! JSON API (design §4.7). Every write names its space; learning state is always the
//! current member's.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/health | status, member, assistant |
//! | GET | /api/spaces | mounted spaces |
//! | GET | /api/registry?space= | kinds, link types, tags |
//! | GET | /api/nodes?q=&kind=&tag=&domain=&state=&space=&since=&until=&date_field=&sort=&limit=&offset= | card list |
//! | POST | /api/nodes | create `{space, via?, …fields}` |
//! | GET | /api/nodes/{id} | card + learning state + backlinks |
//! | PUT | /api/nodes/{id} | edit `{revision?, …fields}` (409 on stale revision) |
//! | DELETE | /api/nodes/{id} | move to the space's trash |
//! | POST | /api/nodes/{id}/share | `{to_space}` explicit copy |
//! | POST | /api/nodes/{id}/encounter | `{via?}` |
//! | POST | /api/scan | `{text, spaces?, candidates?, record?, via?}` |
//! | GET | /api/recall/due?limit= | due checks |
//! | POST | /api/recall/{id} | `{outcome: remembered|forgot|unfamiliar, via?}` |
//! | GET | /api/recent?days=&limit= | created / edited lately |
//! | GET | /api/graph?center=&depth=&tags=&space=&limit= | graph |
//! | GET | /api/timeline?days=&space= | per day created / edited |
//! | POST | /api/assets?space=&ext= | raw bytes → `{asset, ext, url}` |
//! | GET | /api/assets/{space}/{hex.ext} | the file |
//! | POST | /api/draft | `{term, context, kind?, lang?}` → card draft (mock until ym-llm) |
//! | POST | /api/reindex | rebuild the index from the files |

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kb_core::index::Filter;
use kb_core::{DraftRequest, KbError, NodeFields, Outcome, ScanOptions};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;

pub struct ApiError(StatusCode, String, &'static str);

impl From<KbError> for ApiError {
    fn from(e: KbError) -> ApiError {
        let cat = e.category();
        let status = match cat {
            "not_found" => StatusCode::NOT_FOUND,
            "forbidden" => StatusCode::FORBIDDEN,
            "invalid" => StatusCode::BAD_REQUEST,
            "conflict" => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string(), cat)
    }
}

impl From<ym_store::StoreError> for ApiError {
    fn from(e: ym_store::StoreError) -> ApiError {
        KbError::from(e).into()
    }
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into(), "invalid")
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "category": self.2 }))).into_response()
    }
}

type R = Result<Json<Value>, ApiError>;

fn ok(v: impl serde::Serialize) -> R {
    Ok(Json(serde_json::to_value(v).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal"))?))
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/spaces", get(spaces))
        .route("/api/registry", get(registry))
        .route("/api/nodes", get(list).post(create))
        .route("/api/nodes/{id}", get(view).put(update).delete(trash))
        .route("/api/nodes/{id}/share", post(share))
        .route("/api/nodes/{id}/encounter", post(encounter))
        .route("/api/scan", post(scan))
        .route("/api/recall/due", get(due))
        .route("/api/recall/{id}", post(recall))
        .route("/api/recent", get(recent))
        .route("/api/graph", get(graph))
        .route("/api/timeline", get(timeline))
        .route("/api/assets", post(put_asset).layer(DefaultBodyLimit::max(30 * 1024 * 1024)))
        .route("/api/assets/{space}/{file}", get(get_asset))
        .route("/api/draft", post(draft))
        .route("/api/reindex", post(reindex))
}

macro_rules! kb {
    ($s:expr) => {
        $s.kb.lock().unwrap_or_else(|p| p.into_inner())
    };
}

async fn health(State(s): State<Shared>) -> R {
    let kb = kb!(s);
    ok(json!({ "ok": true, "member": kb.member(), "now": kb.now().to_rfc3339(), "assistant": kb.assistant_name(), "intervals": kb.intervals() }))
}

async fn spaces(State(s): State<Shared>) -> R {
    let kb = kb!(s);
    ok(json!({ "member": kb.member(), "spaces": kb.spaces_info()? }))
}

#[derive(Deserialize)]
struct SpaceQ {
    space: Option<String>,
}

async fn registry(State(s): State<Shared>, Query(q): Query<SpaceQ>) -> R {
    let kb = kb!(s);
    let space = match q.space {
        Some(sp) => sp,
        None => kb.spaces().default_for(kb.member()).map(|s| s.id().to_string()).ok_or_else(|| bad("no space"))?,
    };
    let reg = kb.registry(&space)?;
    let tags = kb.tags(&space)?;
    let counts: Vec<Value> = kb.tag_counts(None)?.into_iter().map(|(t, n)| json!({ "tag": t, "count": n })).collect();
    ok(json!({ "space": space, "kinds": reg.kinds, "link_types": reg.link_types, "domains": tags.domains, "tags": tags.tags, "tag_counts": counts }))
}

async fn list(State(s): State<Shared>, Query(f): Query<Filter>) -> R {
    let kb = kb!(s);
    ok(json!({ "nodes": kb.list(&f)? }))
}

#[derive(Deserialize)]
struct CreateBody {
    space: Option<String>,
    via: Option<String>,
    #[serde(flatten)]
    fields: NodeFields,
}

async fn create(State(s): State<Shared>, Json(b): Json<CreateBody>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let space = b.space.filter(|s| !s.is_empty()).ok_or_else(|| bad("`space` is required: every write names its space"))?;
    let mut kb = kb!(s);
    let node = kb.create(&space, b.fields, b.via.as_deref().or(Some("kb")))?;
    let summary = kb.summary(&node.id)?;
    Ok((StatusCode::CREATED, Json(json!({ "space": space, "node": node, "summary": summary }))))
}

async fn view(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let kb = kb!(s);
    ok(kb.view(&id)?)
}

#[derive(Deserialize)]
struct UpdateBody {
    revision: Option<u64>,
    #[serde(flatten)]
    fields: NodeFields,
}

async fn update(State(s): State<Shared>, Path(id): Path<String>, Json(b): Json<UpdateBody>) -> R {
    let mut kb = kb!(s);
    let node = kb.update(&id, b.fields, b.revision)?;
    ok(json!({ "node": node }))
}

async fn trash(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let mut kb = kb!(s);
    let to = kb.trash(&id)?;
    ok(json!({ "trashed": id, "file": to.file_name().map(|n| n.to_string_lossy().to_string()) }))
}

#[derive(Deserialize)]
struct ShareBody {
    to_space: String,
}

async fn share(State(s): State<Shared>, Path(id): Path<String>, Json(b): Json<ShareBody>) -> R {
    let mut kb = kb!(s);
    let copy = kb.share(&id, &b.to_space)?;
    ok(json!({ "node": copy, "space": b.to_space }))
}

#[derive(Deserialize, Default)]
struct ViaBody {
    via: Option<String>,
}

async fn encounter(State(s): State<Shared>, Path(id): Path<String>, body: Option<Json<ViaBody>>) -> R {
    let mut kb = kb!(s);
    let via = body.and_then(|b| b.0.via);
    ok(json!({ "progress": kb.encounter(&id, via.as_deref())? }))
}

#[derive(Deserialize)]
struct ScanBody {
    text: String,
    #[serde(flatten)]
    opts: ScanOptions,
}

async fn scan(State(s): State<Shared>, Json(b): Json<ScanBody>) -> R {
    let mut kb = kb!(s);
    ok(kb.scan(&b.text, &b.opts)?)
}

#[derive(Deserialize)]
struct LimitQ {
    limit: Option<u32>,
    days: Option<i64>,
}

async fn due(State(s): State<Shared>, Query(q): Query<LimitQ>) -> R {
    let kb = kb!(s);
    ok(json!({ "now": kb.now().to_rfc3339(), "nodes": kb.due(q.limit.unwrap_or(50))? }))
}

#[derive(Deserialize)]
struct RecallBody {
    outcome: Outcome,
    via: Option<String>,
}

async fn recall(State(s): State<Shared>, Path(id): Path<String>, Json(b): Json<RecallBody>) -> R {
    let mut kb = kb!(s);
    ok(kb.recall(&id, b.outcome, b.via.as_deref().or(Some("kb")))?)
}

async fn recent(State(s): State<Shared>, Query(q): Query<LimitQ>) -> R {
    let kb = kb!(s);
    ok(json!({ "nodes": kb.recent(q.days.unwrap_or(7), q.limit.unwrap_or(50))? }))
}

#[derive(Deserialize)]
struct GraphQ {
    center: Option<String>,
    depth: Option<u32>,
    tags: Option<bool>,
    space: Option<String>,
    limit: Option<usize>,
}

async fn graph(State(s): State<Shared>, Query(q): Query<GraphQ>) -> R {
    let kb = kb!(s);
    ok(kb.graph(q.center.as_deref().filter(|c| !c.is_empty()), q.depth.unwrap_or(2), q.tags.unwrap_or(false), q.space.as_deref().filter(|c| !c.is_empty()), q.limit.unwrap_or(400))?)
}

#[derive(Deserialize)]
struct TimelineQ {
    days: Option<i64>,
    space: Option<String>,
}

async fn timeline(State(s): State<Shared>, Query(q): Query<TimelineQ>) -> R {
    let kb = kb!(s);
    ok(json!({ "days": kb.timeline(q.days.unwrap_or(30), q.space.as_deref().filter(|c| !c.is_empty()))? }))
}

#[derive(Deserialize)]
struct AssetQ {
    space: String,
    ext: String,
}

async fn put_asset(State(s): State<Shared>, Query(q): Query<AssetQ>, body: Bytes) -> R {
    if body.is_empty() {
        return Err(bad("empty upload"));
    }
    let kb = kb!(s);
    let r = kb.put_asset(&q.space, &body, &q.ext)?;
    let url = format!("/api/assets/{}/{}", q.space, r.file_name());
    ok(json!({ "asset": r.asset, "ext": r.ext, "url": url }))
}

async fn get_asset(State(s): State<Shared>, Path((space, file)): Path<(String, String)>) -> Result<Response, ApiError> {
    let (hex, ext) = file.rsplit_once('.').ok_or_else(|| bad("asset file name"))?;
    let r = ym_store::AssetRef::parse(hex, ext)?;
    let bytes = {
        let kb = kb!(s);
        kb.get_asset(&space, &r)?
    };
    let ct = match r.ext.as_str() {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    };
    Ok(([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "private, max-age=31536000, immutable")], bytes).into_response())
}

async fn draft(State(s): State<Shared>, Json(req): Json<DraftRequest>) -> R {
    let kb = kb!(s);
    ok(json!({ "draft": kb.draft(&req)?, "assistant": kb.assistant_name() }))
}

async fn reindex(State(s): State<Shared>) -> R {
    let mut kb = kb!(s);
    ok(kb.rebuild()?)
}
