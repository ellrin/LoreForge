//! JSON API (design §4.7). Every write names its space; learning state is always the
//! current member's.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/health | status, member, assistant, `reader_url` (LoreReader, for `pdf` sources) |
//! | GET | /api/spaces | mounted spaces |
//! | GET | /api/registry?space= | kinds, link types, tags |
//! | GET | /api/nodes?q=&kind=&tag=&domain=&state=&space=&since=&until=&date_field=&sort=&limit=&offset= | card list |
//! | POST | /api/nodes | create `{space, via?, initial_state?: learning\|known, …fields}` |
//! | GET | /api/nodes/{id} | card + learning state + backlinks |
//! | PUT | /api/nodes/{id} | edit `{revision?, …fields}` (409 on stale revision) |
//! | DELETE | /api/nodes/{id} | move to the space's trash |
//! | POST | /api/nodes/{id}/restore | undo the delete (latest trashed copy) |
//! | POST | /api/nodes/{id}/share | `{to_space}` explicit copy |
//! | POST | /api/nodes/{id}/encounter | `{via?}` |
//! | POST | /api/scan | `{text, spaces?, candidates?, record?, via?}` |
//! | GET | /api/recall/due?limit= | due checks |
//! | POST | /api/recall/{id} | `{outcome: remembered|forgot|unfamiliar, via?}` |
//! | GET | /api/recent?days=&limit= | created / edited lately |
//! | GET | /api/graph?center=&depth=&tags=&space=&limit=&full=&missing= | graph (`full=true`: every card; `missing=true`: placeholders for unresolved `[[links]]`) |
//! | GET | /api/timeline?days=&space= | per day created / edited |
//! | POST | /api/assets?space=&ext= | raw bytes → `{asset, ext, url}` |
//! | GET | /api/assets/{space}/{hex.ext} | the file |
//! | POST | /api/draft | `{term, context, kind?, lang?}` → card draft (mock until lore-llm) |
//! | POST | /api/reindex | rebuild the index from the files |
//! | GET | /api/stats | today's numbers (+ notification times for NeonDeck) |
//! | GET/PUT | /api/reminders | e-mail / notification settings, state, password status |
//! | PUT | /api/reminders/password | `{password}` → keychain, or `credentials.toml` when no keychain is usable (empty removes); returns `{saved, password}` |
//! | GET | /api/reminders/preview?format=html\|text | today's digest, not sent |
//! | POST | /api/reminders/test-email | send today's digest now (does not count as the daily one) |
//! | POST | /api/reminders/test-notify | show a notification with osascript |
//! | POST | /api/reminders/claim | `{by?}` take the current notification slot (NeonDeck) |

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

use crate::reminders::{self, ReminderSettings};
use crate::{Shared, digest};

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

impl From<lore_store::StoreError> for ApiError {
    fn from(e: lore_store::StoreError) -> ApiError {
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
        .route("/prefs.css", get(prefs_css))
        .route("/theme.css", get(theme_css))
        .route("/theme.js", get(theme_js))
        .route("/api/theme", get(theme_get).put(theme_put))
        .route("/api/health", get(health))
        .route("/api/spaces", get(spaces))
        .route("/api/registry", get(registry))
        .route("/api/nodes", get(list).post(create))
        .route("/api/nodes/{id}", get(view).put(update).delete(trash))
        .route("/api/nodes/{id}/restore", post(restore))
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
        .route("/api/stats", get(stats))
        .route("/api/reminders", get(reminders_get).put(reminders_put))
        .route("/api/reminders/password", axum::routing::put(reminders_password))
        .route("/api/reminders/preview", get(reminders_preview))
        .route("/api/reminders/test-email", post(reminders_test_email))
        .route("/api/reminders/test-notify", post(reminders_test_notify))
        .route("/api/reminders/claim", post(reminders_claim))
}

macro_rules! kb {
    ($s:expr) => {
        $s.kb.lock().unwrap_or_else(|p| p.into_inner())
    };
}

async fn health(State(s): State<Shared>) -> R {
    let kb = kb!(s);
    ok(json!({ "ok": true, "member": kb.member(), "now": kb.now().to_rfc3339(), "assistant": kb.assistant_name(), "intervals": kb.intervals(), "reader_url": crate::config::reader_url() }))
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
    /// The member's state for the new card: `learning` (default) or `known`
    /// (a term already mastered, e.g. added from LoreReader).
    initial_state: Option<kb_core::State>,
    #[serde(flatten)]
    fields: NodeFields,
}

async fn create(State(s): State<Shared>, Json(b): Json<CreateBody>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let space = b.space.filter(|s| !s.is_empty()).ok_or_else(|| bad("`space` is required: every write names its space"))?;
    let mut kb = kb!(s);
    let node = kb.create_as(&space, b.fields, b.via.as_deref().or(Some("kb")), b.initial_state.unwrap_or(kb_core::State::Learning))?;
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

async fn restore(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let mut kb = kb!(s);
    let (space, node) = kb.restore(&id)?;
    ok(json!({ "space": space, "node": node }))
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
    /// Every card (up to [`GRAPH_MAX`]) instead of the newest `limit` (default 400).
    full: Option<bool>,
    /// Placeholder nodes (`kind = "missing"`) for unresolved `[[links]]`.
    missing: Option<bool>,
}

/// Most cards one graph response carries.
const GRAPH_MAX: usize = 20_000;

async fn graph(State(s): State<Shared>, Query(q): Query<GraphQ>) -> R {
    let kb = kb!(s);
    let limit = if q.full.unwrap_or(false) { GRAPH_MAX } else { q.limit.unwrap_or(400).min(GRAPH_MAX) };
    let gq = kb_core::GraphQuery {
        center: q.center.filter(|c| !c.is_empty()),
        depth: q.depth.unwrap_or(2).min(6),
        with_tags: q.tags.unwrap_or(false),
        space: q.space.filter(|c| !c.is_empty()),
        limit,
        missing: q.missing.unwrap_or(false),
    };
    ok(kb.graph_with(&gq)?)
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
    let r = lore_store::AssetRef::parse(hex, ext)?;
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

/// A card draft from a quoted source through the `kb-summarize-card` task. Nothing is saved.
/// Body: `{term, context, kind?, lang?, page?, source?, engine?: "local"|"claude"}`.
/// Engine trouble is a 503 with a one-line `error`; the caller keeps the source text.
async fn draft(State(s): State<Shared>, Json(body): Json<serde_json::Map<String, Value>>) -> R {
    let engine = body.get("engine").and_then(Value::as_str).map(str::to_string);
    let req: DraftRequest = serde_json::from_value(Value::Object(body)).map_err(|e| bad(e.to_string()))?;
    let p = kb_core::draft_prompt(&req)?;
    let llm_req = lore_llm::LlmRequest::prompt(p.system.clone(), p.user.clone()).max_tokens(1024).json(p.schema.clone());
    let a = s.llm.complete_json(crate::llm::TASK_DRAFT, llm_req, engine.as_deref()).await.map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, e, "assistant"))?;
    let draft = kb_core::parse_draft(&req, &p.quote, &a.value).map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string(), "assistant"))?;
    ok(json!({ "draft": draft, "assistant": crate::llm::engine_label(&a.engine), "engine": a.engine }))
}

async fn reindex(State(s): State<Shared>) -> R {
    let mut kb = kb!(s);
    ok(kb.rebuild()?)
}

// ── Recall reminders (design §4.5) ─────────────────────

async fn stats(State(s): State<Shared>) -> R {
    let settings = s.rem.settings();
    let kb = kb!(s);
    let st = reminders::stats(&kb)?;
    ok(json!({ "now": kb.now().to_rfc3339(), "stats": st, "notify": { "times": settings.notify.times } }))
}

fn reminders_view(s: &Shared) -> Value {
    let settings = s.rem.settings();
    let pw = s.rem.password();
    json!({
        "settings": settings,
        "state": s.rem.state(),
        "password": { "set": pw.is_some(), "source": pw.map(|(_, src)| src) },
        "settings_file": s.rem.settings_path(),
    })
}

async fn reminders_get(State(s): State<Shared>) -> R {
    let s2 = s.clone();
    ok(tokio::task::spawn_blocking(move || reminders_view(&s2)).await.map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal"))?)
}

async fn reminders_put(State(s): State<Shared>, Json(mut b): Json<ReminderSettings>) -> R {
    b.validate().map_err(bad)?;
    s.rem.save_settings(&b).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e, "internal"))?;
    reminders_get(State(s)).await
}

#[derive(Deserialize)]
struct PasswordBody {
    password: String,
}

async fn reminders_password(State(s): State<Shared>, Json(b): Json<PasswordBody>) -> R {
    let s2 = s.clone();
    let src = tokio::task::spawn_blocking(move || s2.rem.set_password(&b.password))
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal"))?
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e, "internal"))?;
    let s3 = s.clone();
    let pw = tokio::task::spawn_blocking(move || s3.rem.password().map(|(_, src)| src)).await.ok().flatten();
    ok(json!({ "saved": src, "password": { "set": pw.is_some(), "source": pw } }))
}

#[derive(Deserialize)]
struct PreviewQ {
    format: Option<String>,
}

async fn reminders_preview(State(s): State<Shared>, Query(q): Query<PreviewQ>) -> Result<Response, ApiError> {
    let settings = s.rem.settings();
    let d = {
        let kb = kb!(s);
        digest::build(&kb, &settings.email, &s.rem.base_url)?
    };
    Ok(match q.format.as_deref() {
        Some("text") => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], format!("Subject: {}\n\n{}", d.subject, d.text)).into_response(),
        Some("json") => Json(json!({ "subject": d.subject, "text": d.text, "html": d.html, "due": d.due_total, "recent": d.recent.len() })).into_response(),
        _ => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], d.html).into_response(),
    })
}

async fn reminders_test_email(State(s): State<Shared>) -> R {
    let settings = s.rem.settings();
    let _busy = s.rem.busy.lock().await;
    let d = {
        let kb = kb!(s);
        digest::build(&kb, &settings.email, &s.rem.base_url)?
    };
    reminders::send_with_keychain(&s.rem, &settings.email, &d).await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e, "smtp"))?;
    ok(json!({ "sent": true, "to": settings.email.to, "subject": d.subject }))
}

async fn reminders_test_notify(State(s): State<Shared>) -> R {
    let (due, title, body) = {
        let kb = kb!(s);
        reminders::notify_text(&kb)?
    };
    let (title, body) = if due > 0 { (title, body) } else { ("KnowledgeBase 通知測試".to_string(), "目前沒有到期的卡片".to_string()) };
    s.rem.notify(&title, &body).map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e, "notify"))?;
    ok(json!({ "shown": true, "title": title, "body": body }))
}

#[derive(Deserialize, Default)]
struct ClaimBody {
    by: Option<String>,
}

async fn reminders_claim(State(s): State<Shared>, body: Bytes) -> R {
    // Lenient: an empty body (or a `sendBeacon` without content type) is fine.
    let by = serde_json::from_slice::<ClaimBody>(&body).ok().and_then(|b| b.by).filter(|b| !b.is_empty()).unwrap_or_else(|| "neondeck".into());
    let _busy = s.rem.busy.lock().await;
    let kb = kb!(s);
    ok(reminders::claim(&s.rem, &kb, &by)?)
}

/// The user's interface fonts (`private/shared/fonts.toml`) as CSS variables; linked after styles.css.
async fn prefs_css(State(s): State<Shared>) -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], s.home.prefs().fonts.css())
}

/// The LoreForge themes (commons/ui/theme.css, the same file in every app), compiled in.
async fn theme_css() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], lore_home::theme::CSS)
}

/// commons/ui/theme.js with the shared choice in front (first paint has the theme).
async fn theme_js(State(s): State<Shared>) -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], lore_home::theme::script(&s.home))
}

/// The theme every LoreForge app shows (`private/shared/ui.toml`); null = none chosen yet.
async fn theme_get(State(s): State<Shared>) -> Json<Value> {
    Json(json!({ "theme": s.home.theme() }))
}

async fn theme_put(State(s): State<Shared>, Json(b): Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match s.home.set_theme(b["theme"].as_str().unwrap_or_default()) {
        Ok(t) => Json(json!({ "theme": t })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}
