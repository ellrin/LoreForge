//! The know-how hub over HTTP (`docs/brain-hub.md`): lore-know opened on the author's personal
//! space and shown by L-Brain's hub page. Handlers are plain fns over `Shared`, like `api.rs`.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/hub/health | `{ok, hub}` |
//! | GET | /api/hub/stats | `{stats, cards}` (the dashboard; knowledge cards are the first family) |
//! | GET | /api/hub/categories | `{categories, formats, sorts, facets}` |
//! | POST | /api/hub/categories | `{label, format, section?, group?}` a hand-made category |
//! | PUT/DELETE | /api/hub/categories/{id} | rename / steps; remove (409 while it has items) |
//! | GET | /api/hub/items?category=&family=&q=&sort=&desc=&level=&entity=&source=&tag=facet:tag&limit=&offset= | rows |
//! | POST | /api/hub/items | add an item (201) |
//! | GET/PUT/DELETE | /api/hub/items/{id} | one item with what it contains and what uses it; change (409 on a stale revision); to the trash |
//! | POST | /api/hub/items/{id}/restore | the latest trashed copy back |
//! | GET | /api/hub/thumb/{id} | a figure's drawing (SVG) or a reference's picture |
//! | GET/POST | /api/hub/proposals | what an AI or a tool proposes; nothing is written until applied |
//! | GET | /api/hub/proposals/{id} | the proposal and every change as the author would see it |
//! | POST | /api/hub/proposals/{id}/apply | `{chosen?: [n]}` |
//! | POST | /api/hub/proposals/{id}/undo | `{proposal, not_undone}` |
//! | POST | /api/hub/proposals/{id}/dismiss | |
//! | POST | /api/hub/import?own=1 | an outside application's batch (JSON) |
//! | GET | /api/hub/export?category=&source= | a category as a batch file |
//! | GET | /api/hub/sources | outside applications and where their items are |
//! | GET/POST | /api/hub/make/{id}, POST …/draft | making a 待製作 reference in the flow editor (`figmake.rs`) |
//! | * | /flow/… | the shared flow editor (`lore_flow::web::router`), its model calls through L-Brain's routing |
//!
//! Every error is a status and `{ "error": "<one line, Traditional Chinese>" }`; no absolute path
//! is ever put into a response.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Mutex, MutexGuard};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Datelike, Duration, NaiveDate};
use lore_know::index::{Query as ItemQuery, SORTS};
use lore_know::intake::{self, Batch};
use lore_know::proposal;
use lore_know::stats::{Family, Part};
use lore_know::{Category, Change, Item, KnowError, Problem, Sink, Source, facet, format, reader, split_id};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;

/// The folders the manifests name, by their private folder name (`Native::app_dir`).
const APP_FOLDERS: [&str; 4] = ["PaperComposer", "SlideComposer", "LoreReader", "KnowledgeBase"];

/// Most rows one list answers with.
const MAX_LIMIT: usize = 1000;
const DEFAULT_LIMIT: usize = 200;

/// Everything the hub routes need beside the knowledge base.
pub struct HubState {
    /// `None`: no personal space, every `/api/hub/*` answers 503.
    pub hub: Option<lore_know::hub::Hub>,
    /// Private folder name → the folder (for a reference's picture).
    pub app_dirs: BTreeMap<String, PathBuf>,
    /// The shared flow editor mounted at `/flow`.
    pub flow: lore_flow::web::Flow,
}

impl HubState {
    /// No hub (tests that do not touch it): the flow editor has no private folder either.
    pub fn none() -> HubState {
        HubState { hub: None, app_dirs: BTreeMap::new(), flow: lore_flow::web::Flow::bare() }
    }

    /// The author's own: `private/KnowledgeBase/hub` with the apps' folders that exist. Without a
    /// personal space (`private/KnowledgeBase` is missing) there is no hub, and nothing is created.
    pub fn from_home(home: &lore_home::Home) -> HubState {
        let personal = home.app_dir("KnowledgeBase").is_some_and(|d| d.is_dir());
        let mut app_dirs = BTreeMap::new();
        for name in APP_FOLDERS {
            if let Some(d) = home.app_dir(name).filter(|d| d.is_dir()) {
                app_dirs.insert(name.to_string(), d);
            }
        }
        if let Some(d) = home.shared_dir().filter(|d| d.is_dir()) {
            app_dirs.insert("shared".to_string(), d);
        }
        let flow_dirs: Vec<PathBuf> = [home.shared_dir(), home.app_dir("SlideComposer")]
            .into_iter()
            .flatten()
            .map(|d| d.join("flow"))
            .filter(|d| d.is_dir())
            .collect();
        let hub = match reader::hub_dir(home).filter(|_| personal) {
            Some(dir) => match lore_know::hub::Hub::open(dir, &reader::builtin_manifests(), &app_dirs, &flow_dirs) {
                Ok(h) => Some(h),
                Err(e) => {
                    eprintln!("kb-server: the hub did not open: {e}");
                    None
                }
            },
            None => None,
        };
        HubState { hub, app_dirs, flow: lore_flow::web::Flow::new(home.clone()) }
    }

    /// Tests and demos: a hub in `dir` (hub records in `dir/hub`, app data in `dir/app`, flow
    /// drawings in `dir/flow`) from one synthetic manifest; never the real folders.
    pub fn isolated(dir: &FsPath, manifest: &str) -> Result<HubState, KnowError> {
        let app = dir.join("app");
        std::fs::create_dir_all(&app)?;
        let app_dirs = BTreeMap::from([("App".to_string(), app)]);
        let flow = dir.join("flow");
        std::fs::create_dir_all(&flow)?;
        let hub = lore_know::hub::Hub::open(dir.join("hub"), &[manifest], &app_dirs, std::slice::from_ref(&flow))?;
        Ok(HubState { hub: Some(hub), app_dirs, flow: lore_flow::web::Flow::bare().with_dir(flow) })
    }
}

// ── routes ───────────────────────────────────────────────────────────────────

pub fn routes(shared: &Shared) -> Router<Shared> {
    let big = || DefaultBodyLimit::max(20 * 1024 * 1024);
    Router::new()
        .route("/api/hub/health", get(health))
        .route("/api/hub/stats", get(stats))
        .route("/api/hub/categories", get(categories).post(category_add))
        .route("/api/hub/categories/{id}", axum::routing::put(category_change).delete(category_remove))
        .route("/api/hub/items", get(items).post(item_add))
        .route("/api/hub/items/{id}", get(item_view).put(item_change).delete(item_remove))
        .route("/api/hub/items/{id}/restore", post(item_restore))
        .route("/api/hub/thumb/{id}", get(thumb))
        .route("/api/hub/proposals", get(proposals).post(proposal_add))
        .route("/api/hub/proposals/{id}", get(proposal_view))
        .route("/api/hub/tree", get(tree_get).put(tree_put))
        .route("/api/hub/proposals/{id}/apply", post(proposal_apply))
        .route("/api/hub/proposals/{id}/picture", post(proposal_picture).layer(big()))
        .route("/api/hub/proposals/{id}/undo", post(proposal_undo))
        .route("/api/hub/proposals/{id}/dismiss", post(proposal_dismiss))
        .route("/api/hub/import", post(import).layer(big()))
        .route("/api/hub/export", get(export))
        .route("/api/hub/sources", get(sources))
        .merge(crate::review::routes())
        // the shared flow editor, with this app's model routing (`figmake::FlowLlm`)
        .merge(lore_flow::web::router::<Shared>(shared.flow.clone().with_llm(std::sync::Arc::new(crate::figmake::FlowLlm(shared.llm.clone())))))
}

// ── errors ───────────────────────────────────────────────────────────────────

pub struct HubError {
    status: StatusCode,
    message: String,
    /// A stale write: the revision now stored.
    revision: Option<u64>,
    problems: Vec<Problem>,
}

impl HubError {
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn new(status: StatusCode, message: impl AsRef<str>) -> HubError {
        HubError { status, message: one_line(message.as_ref()), revision: None, problems: vec![] }
    }
}

/// One line, and no absolute path: a word that looks like one (`/Users/…`, `C:\…`) is cut to its
/// last component.
fn one_line(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.split(' ').map(scrub_word).collect::<Vec<_>>().join(" ")
}

fn scrub_word(w: &str) -> String {
    let trimmed = w.trim_matches(|c: char| matches!(c, '"' | '\'' | '「' | '」' | '(' | ')' | ',' | '，' | '。' | ':' | '：'));
    let abs = (trimmed.starts_with('/') && trimmed[1..].contains('/')) || (trimmed.len() > 2 && trimmed.as_bytes()[1] == b':' && trimmed.contains('\\'));
    if !abs {
        return w.to_string();
    }
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    w.replacen(trimmed, last, 1)
}

impl From<KnowError> for HubError {
    fn from(e: KnowError) -> HubError {
        match e {
            KnowError::NotFound(_) => HubError::new(StatusCode::NOT_FOUND, e.to_string()),
            KnowError::Invalid(_) => HubError::new(StatusCode::BAD_REQUEST, e.to_string()),
            KnowError::Stale { have, .. } => HubError { revision: Some(have), ..HubError::new(StatusCode::CONFLICT, e.to_string()) },
            KnowError::ReadOnly(_) => HubError::new(StatusCode::FORBIDDEN, e.to_string()),
            KnowError::Io(ref io) => {
                eprintln!("kb-server hub: {io}");
                HubError::new(StatusCode::INTERNAL_SERVER_ERROR, "讀寫資料時出了問題，沒有完成。")
            }
            KnowError::Other(_) => HubError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}

impl IntoResponse for HubError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message });
        if let Some(r) = self.revision {
            body["revision"] = json!(r);
        }
        if !self.problems.is_empty() {
            body["problems"] = json!(self.problems);
        }
        (self.status, Json(body)).into_response()
    }
}

pub(crate) type R = Result<Json<Value>, HubError>;

pub(crate) fn ok(v: impl serde::Serialize) -> R {
    Ok(Json(serde_json::to_value(v).map_err(|_| HubError::new(StatusCode::INTERNAL_SERVER_ERROR, "回覆組不起來。"))?))
}

fn no_hub() -> HubError {
    HubError::new(StatusCode::SERVICE_UNAVAILABLE, "個人空間還沒有設定，知識庫整合暫時不能用。")
}

pub(crate) type Guard<'a> = MutexGuard<'a, lore_know::hub::Hub>;

pub(crate) fn hub(s: &Shared) -> Result<Guard<'_>, HubError> {
    let m: &Mutex<_> = s.hub.as_ref().ok_or_else(no_hub)?;
    Ok(m.lock().unwrap_or_else(|p| p.into_inner()))
}

pub(crate) fn not_found(what: &str) -> HubError {
    HubError::new(StatusCode::NOT_FOUND, format!("找不到：{what}"))
}

// ── rows ─────────────────────────────────────────────────────────────────────

/// What a list row carries beyond the item: its one line and its picture.
pub(crate) fn row(h: &lore_know::hub::Hub, item: &Item) -> Value {
    let line = h.formats.get(&item.format).map(|f| f.line(item)).unwrap_or_default();
    let pictured = item.format == format::FIGURE && (item.entity || !item.field("image").is_empty());
    let thumb = if pictured { format!("/api/hub/thumb/{}", item.id) } else { String::new() };
    let mut v = serde_json::to_value(item).unwrap_or(Value::Null);
    v["line"] = json!(line);
    v["thumb"] = json!(thumb);
    v
}

fn rows(h: &lore_know::hub::Hub, items: &[Item]) -> Vec<Value> {
    items.iter().map(|i| row(h, i)).collect()
}

// ── health and stats ─────────────────────────────────────────────────────────

async fn health(State(s): State<Shared>) -> R {
    ok(json!({ "ok": true, "hub": s.hub.is_some() }))
}

/// The 12 Mondays ending with this week's, oldest first.
fn twelve_mondays(today: NaiveDate) -> Vec<NaiveDate> {
    let this = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    (0..12).rev().map(|n| this - Duration::weeks(n)).collect()
}

fn day_of(stamp: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(stamp.get(..10)?, "%Y-%m-%d").ok()
}

/// The knowledge cards (kb-core's own data) as a family of the dashboard, and the numbers the
/// cover shows beside it.
fn cards_family(s: &Shared, mondays: &[NaiveDate], today: NaiveDate) -> Result<(Family, Value), HubError> {
    let kb = s.kb.lock().unwrap_or_else(|p| p.into_inner());
    let all = kb.list(&kb_core::index::Filter { limit: Some(5000), ..Default::default() }).map_err(|e| HubError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let due = kb.due(5000).map(|d| d.len()).unwrap_or(0);
    let mut weeks = vec![0usize; mondays.len()];
    let (mut added_30d, mut known, mut learning, mut unfamiliar) = (0, 0, 0, 0);
    // kind id → (label, count), labels from the card's own space
    let mut kinds: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for n in &all {
        match n.state.as_deref() {
            Some("known") => known += 1,
            Some("learning") => learning += 1,
            // marked unfamiliar, gone stale, or not started: the cards to meet again
            _ => unfamiliar += 1,
        }
        let label = kb.registry(&n.space).ok().and_then(|r| r.kinds.iter().find(|k| k.id == n.kind)).map(|k| k.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| n.kind.clone());
        kinds.entry(n.kind.clone()).or_insert((label, 0)).1 += 1;
        if let Some(d) = day_of(&n.created_at) {
            if (today - d).num_days() < 30 && d <= today {
                added_30d += 1;
            }
            if let Some(first) = mondays.first() {
                let at = (d - *first).num_days();
                if at >= 0 && (at / 7) < weeks.len() as i64 {
                    weeks[(at / 7) as usize] += 1;
                }
            }
        }
    }
    let mut parts: Vec<Part> = kinds.into_values().map(|(label, count)| Part { category: String::new(), label, count }).collect();
    parts.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));
    let family = Family {
        id: "kb/cards".into(),
        label: "知識卡".into(),
        app: "kb".into(),
        categories: vec![],
        count: all.len(),
        parts,
        weeks,
        added_30d,
        used_30d: 0,
        gap: String::new(),
        independent: false,
    };
    let cards = json!({ "total": all.len(), "due": due, "known": known, "learning": learning, "unfamiliar": unfamiliar });
    Ok((family, cards))
}

async fn stats(State(s): State<Shared>) -> R {
    let today = s.kb.lock().unwrap_or_else(|p| p.into_inner()).now().date_naive();
    // the hub and the cards are read one after the other, never both locked
    let mut st = hub(&s)?.stats(today)?;
    let mondays = twelve_mondays(today);
    if st.week_starts.is_empty() {
        st.week_starts = mondays.iter().map(|d| d.to_string()).collect();
    }
    let (family, cards) = cards_family(&s, &mondays, today)?;
    st.total += family.count;
    st.families.insert(0, family);
    let inbox = crate::inbox::counts(&s);
    ok(json!({ "stats": st, "cards": cards, "inbox": inbox }))
}

// ── categories ───────────────────────────────────────────────────────────────

fn category_json(c: &Category, count: usize) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    v["count"] = json!(count);
    v["connected"] = json!(c.connected());
    v
}

fn count_of(h: &lore_know::hub::Hub, id: &str) -> Result<usize, HubError> {
    Ok(h.counts()?.into_iter().find(|c| c.category == id).map(|c| c.total).unwrap_or(0))
}

async fn categories(State(s): State<Shared>) -> R {
    let h = hub(&s)?;
    let counts: HashMap<String, usize> = h.counts()?.into_iter().map(|c| (c.category, c.total)).collect();
    let cats: Vec<Value> = h.store.registry().all().iter().map(|c| category_json(c, counts.get(&c.id).copied().unwrap_or(0))).collect();
    let formats: Vec<_> = h.formats.values().map(|f| f.def()).collect();
    let sorts: Vec<[&str; 2]> = SORTS.iter().map(|(a, b)| [*a, *b]).collect();
    ok(json!({ "categories": cats, "formats": formats, "sorts": sorts, "facets": [facet::DOMAIN, facet::KIND, facet::USE] }))
}

#[derive(Deserialize)]
struct NewCategory {
    label: String,
    format: String,
    section: Option<String>,
    group: Option<String>,
}

async fn category_add(State(s): State<Shared>, Json(b): Json<NewCategory>) -> Result<(StatusCode, Json<Value>), HubError> {
    let mut h = hub(&s)?;
    let c = h.store.registry_mut().add_manual(&b.label, &b.format, b.section.as_deref().unwrap_or(""), b.group.as_deref().unwrap_or(""))?;
    h.reindex()?;
    Ok((StatusCode::CREATED, Json(json!({ "category": category_json(&c, 0) }))))
}

#[derive(Deserialize)]
struct CategoryChange {
    label: Option<String>,
    tasks: Option<Vec<String>>,
}

async fn category_change(State(s): State<Shared>, Path(id): Path<String>, Json(b): Json<CategoryChange>) -> R {
    let mut h = hub(&s)?;
    if h.store.registry().get(&id).is_none() {
        return Err(not_found(&format!("類別 {id}")));
    }
    if let Some(label) = &b.label {
        h.store.registry_mut().rename(&id, label)?;
    }
    if let Some(tasks) = b.tasks {
        h.store.registry_mut().set_tasks(&id, tasks)?;
    }
    h.reindex()?;
    let c = h.store.registry().get(&id).cloned().ok_or_else(|| not_found(&format!("類別 {id}")))?;
    let n = count_of(&h, &id)?;
    ok(json!({ "category": category_json(&c, n) }))
}

async fn category_remove(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let mut h = hub(&s)?;
    let c = h.store.registry().get(&id).cloned().ok_or_else(|| not_found(&format!("類別 {id}")))?;
    let n = count_of(&h, &id)?;
    if n > 0 && c.origin != "lore" {
        return Err(HubError::new(StatusCode::CONFLICT, format!("「{}」還有 {n} 條內容，要先清空才能移除", c.label)));
    }
    let gone = h.store.registry_mut().remove(&id, n > 0)?;
    h.reindex()?;
    ok(json!({ "removed": category_json(&gone, 0) }))
}

// ── items ────────────────────────────────────────────────────────────────────

/// The `family` of a category: `<app>/<group>`, or the category id when it has no group.
pub(crate) fn family_of(c: &Category) -> String {
    if c.group.is_empty() { c.id.clone() } else { format!("{}/{}", c.app, c.group) }
}

fn flag(s: &str) -> bool {
    matches!(s, "1" | "true" | "yes" | "on")
}

fn number(v: &str, what: &str) -> Result<usize, HubError> {
    v.trim().parse::<usize>().map_err(|_| HubError::new(StatusCode::BAD_REQUEST, format!("「{what}」要是數字")))
}

async fn items(State(s): State<Shared>, Query(pairs): Query<Vec<(String, String)>>) -> R {
    let (mut category, mut family, mut desc) = (None::<String>, None::<String>, None::<bool>);
    let mut q = ItemQuery::default();
    let (mut limit, mut offset) = (DEFAULT_LIMIT, 0usize);
    for (k, v) in &pairs {
        let v = v.trim();
        match k.as_str() {
            "category" if !v.is_empty() => category = Some(v.to_string()),
            "family" if !v.is_empty() => family = Some(v.to_string()),
            "q" => q.text = v.to_string(),
            "sort" => q.sort = v.to_string(),
            "desc" if !v.is_empty() => desc = Some(flag(v)),
            "level" if !v.is_empty() => q.level = Some(v.to_string()),
            "entity" if !v.is_empty() => q.entity = Some(flag(v)),
            "source" if !v.is_empty() => q.source = Some(v.to_string()),
            "tag" => {
                let (f, t) = v.split_once(':').ok_or_else(|| HubError::new(StatusCode::BAD_REQUEST, "標籤要寫成「面向:標籤」"))?;
                q.tags.entry(f.trim().to_string()).or_default().push(t.trim().to_string());
            }
            "limit" if !v.is_empty() => limit = number(v, "limit")?,
            "offset" if !v.is_empty() => offset = number(v, "offset")?,
            _ => {}
        }
    }
    let limit = limit.clamp(1, MAX_LIMIT);
    // the "newest first" sorts run newest first unless the caller says otherwise
    q.desc = desc.unwrap_or(matches!(q.sort.as_str(), "created" | "updated" | "used" | "used_at"));

    let h = hub(&s)?;
    // the categories the list is limited to; None = every category
    let mut only: Option<Vec<String>> = None;
    if let Some(f) = &family {
        let ids: Vec<String> = h.store.registry().all().iter().filter(|c| family_of(c) == *f || c.id == *f).map(|c| c.id.clone()).collect();
        only = Some(ids);
    }
    if let Some(c) = &category {
        let ok_here = only.as_ref().is_none_or(|o| o.contains(c));
        only = Some(if ok_here { vec![c.clone()] } else { vec![] });
    }
    let (list, total) = match only.as_deref() {
        Some([]) => (vec![], 0),
        Some([one]) => {
            q.category = Some(one.clone());
            q.limit = limit;
            q.offset = offset;
            h.query(&q)?
        }
        Some(many) => {
            // several categories: the index orders everything that matches, the page is cut here
            let apps: Vec<&str> = many.iter().filter_map(|id| h.store.registry().get(id)).map(|c| c.app.as_str()).collect();
            if let Some(first) = apps.first().filter(|a| apps.iter().all(|b| b == *a)) {
                q.app = Some(first.to_string());
            }
            let (ids, _) = h.index.query(&q)?;
            let ids: Vec<String> = ids.into_iter().filter(|id| split_id(id).is_some_and(|(c, _)| many.iter().any(|m| m == c))).collect();
            let total = ids.len();
            let mut page = vec![];
            for id in ids.iter().skip(offset).take(limit) {
                if let Some(i) = h.store.get(id)? {
                    page.push(i);
                }
            }
            (page, total)
        }
        None => {
            q.limit = limit;
            q.offset = offset;
            h.query(&q)?
        }
    };
    let tag_scope = only.as_deref().and_then(|o| if let [one] = o { Some(one.as_str()) } else { None });
    let tags = h.index.tag_counts(tag_scope)?;
    ok(json!({ "items": rows(&h, &list), "total": total, "tags": tags }))
}

/// The category an item id belongs to, and whether it takes writes through the hub.
fn writable_category(h: &lore_know::hub::Hub, id_or_category: &str) -> Result<Category, HubError> {
    let cat_id = split_id(id_or_category).map(|(c, _)| c).unwrap_or(id_or_category);
    let c = h.store.registry().get(cat_id).cloned().ok_or_else(|| not_found(&format!("類別 {cat_id}")))?;
    if !c.writable {
        return Err(HubError::new(StatusCode::FORBIDDEN, format!("「{}」只能看和加標籤，不能在這裡改", c.label)));
    }
    Ok(c)
}

pub(crate) fn invalid(h: &lore_know::hub::Hub, item: &Item) -> Result<(), HubError> {
    let f = h.formats.get(&item.format).ok_or_else(|| HubError::new(StatusCode::BAD_REQUEST, format!("沒有「{}」這個格式", item.format)))?;
    let problems = f.validate(item);
    if let Some(p) = problems.first() {
        let msg = if p.field == "title" { p.message.clone() } else { format!("「{}」{}", p.at, p.message) };
        return Err(HubError { problems, ..HubError::new(StatusCode::BAD_REQUEST, msg) });
    }
    Ok(())
}

/// Fit an item sent by the page to its category: the category's format; every format but the
/// figure is drawable by definition.
fn fit(c: &Category, mut item: Item) -> Result<Item, HubError> {
    if item.format.is_empty() {
        item.format = c.format.clone();
    }
    if item.format != c.format {
        return Err(HubError::new(StatusCode::BAD_REQUEST, format!("「{}」收的是 {}，不是 {}", c.label, c.format, item.format)));
    }
    item.category = c.id.clone();
    if item.format != format::FIGURE {
        item.entity = true;
    }
    Ok(item)
}

async fn item_add(State(s): State<Shared>, Json(item): Json<Item>) -> Result<(StatusCode, Json<Value>), HubError> {
    if !item.id.is_empty() {
        return Err(HubError::new(StatusCode::BAD_REQUEST, "新的一條不能先有代號"));
    }
    let mut h = hub(&s)?;
    let c = writable_category(&h, &item.category)?;
    let mut item = fit(&c, item)?;
    if item.source.kind.is_empty() {
        item.source = Source::author();
    }
    item.tidy();
    invalid(&h, &item)?;
    let stored = h.put(item)?;
    Ok((StatusCode::CREATED, Json(json!({ "item": row(&h, &stored) }))))
}

async fn item_view(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let h = hub(&s)?;
    let item = h.store.get(&id)?.ok_or_else(|| not_found(&id))?;
    let mut contains = vec![];
    for c in &item.contains {
        if let Some(i) = h.store.get(c)? {
            contains.push(i);
        }
    }
    // what is made of this one: only figures hold other items
    let mut used_by = vec![];
    for c in h.store.registry().all().iter().filter(|c| c.format == format::FIGURE) {
        for i in h.store.list(&c.id)? {
            if i.contains.contains(&id) {
                used_by.push(i);
            }
        }
    }
    let problems = h.formats.get(&item.format).map(|f| f.validate(&item)).unwrap_or_default();
    ok(json!({ "item": row(&h, &item), "contains": rows(&h, &contains), "used_by": rows(&h, &used_by), "problems": problems }))
}

async fn item_change(State(s): State<Shared>, Path(id): Path<String>, Json(item): Json<Item>) -> R {
    if split_id(&id).is_none() {
        return Err(HubError::new(StatusCode::BAD_REQUEST, format!("代號不對：{id}")));
    }
    let mut h = hub(&s)?;
    let c = writable_category(&h, &id)?;
    let mut item = fit(&c, Item { id: id.clone(), ..item })?;
    if item.source.kind.is_empty() {
        item.source = Source::author();
    }
    item.tidy();
    invalid(&h, &item)?;
    let stored = h.put(item)?;
    ok(json!({ "item": row(&h, &stored) }))
}

async fn item_remove(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let mut h = hub(&s)?;
    writable_category(&h, &id)?;
    let gone = h.remove(&id)?;
    ok(json!({ "removed": row(&h, &gone) }))
}

async fn item_restore(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let mut h = hub(&s)?;
    writable_category(&h, &id)?;
    let back = h.restore(&id)?;
    ok(json!({ "item": row(&h, &back) }))
}

// ── thumbnails ───────────────────────────────────────────────────────────────

const PICTURE_TYPES: [(&str, &str); 4] = [("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("svg", "image/svg+xml")];

/// The picture file of a reference item, only if it is a plain file name of a known picture type
/// inside the item's own folder (a link out of the folder is refused). `(bytes, content type)`.
pub(crate) fn reference_picture(h: &lore_know::hub::Hub, app_dirs: &BTreeMap<String, PathBuf>, item: &Item) -> Option<(Vec<u8>, &'static str)> {
    let cat = h.store.registry().get(&item.category)?;
    if cat.native.kind != "dir" {
        return None;
    }
    let (_, local) = split_id(&item.id)?;
    if local.contains("..") {
        return None;
    }
    let folder = app_dirs.get(&cat.native.app_dir)?.join(&cat.native.path).join(local);
    picture_in(&folder, item.field("image"))
}

fn picture_in(folder: &FsPath, name: &str) -> Option<(Vec<u8>, &'static str)> {
    let plain = !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..";
    if !plain {
        return None;
    }
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let mime = PICTURE_TYPES.iter().find(|(e, _)| *e == ext)?.1;
    let root = folder.canonicalize().ok()?;
    let file = root.join(name).canonicalize().ok()?;
    if !file.starts_with(&root) || !file.is_file() {
        return None;
    }
    Some((std::fs::read(file).ok()?, mime))
}

async fn thumb(State(s): State<Shared>, Path(id): Path<String>) -> Result<Response, HubError> {
    let h = hub(&s)?;
    let item = h.store.get(&id)?.ok_or_else(|| not_found(&id))?;
    if item.format != format::FIGURE {
        return Err(not_found("縮圖"));
    }
    let (body, mime): (Vec<u8>, &str) = if item.entity {
        (h.thumbnail(&id).map_err(|e| HubError::new(StatusCode::NOT_FOUND, e.to_string()))?.into_bytes(), "image/svg+xml")
    } else {
        reference_picture(&h, &s.hub_dirs, &item).ok_or_else(|| not_found("縮圖"))?
    };
    let mut res = Response::new(Body::from(body));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(if mime == "image/svg+xml" { "image/svg+xml; charset=utf-8" } else { mime_static(mime) }));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(res)
}

fn mime_static(m: &str) -> &'static str {
    PICTURE_TYPES.iter().find(|(_, t)| *t == m).map(|(_, t)| *t).unwrap_or("application/octet-stream")
}

// ── proposals ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StateQ {
    state: Option<String>,
}

async fn proposals(State(s): State<Shared>, Query(q): Query<StateQ>) -> R {
    let h = hub(&s)?;
    let state = q.state.as_deref().filter(|s| !s.is_empty());
    ok(json!({ "proposals": h.proposals.list(state), "open": h.proposals.count_open() }))
}

async fn proposal_view(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let h = hub(&s)?;
    let p = h.proposals.get(&id)?;
    let shown = proposal::show(&h.store, &p)?;
    ok(json!({ "proposal": p, "shown": shown }))
}

#[derive(Deserialize)]
struct NewProposal {
    #[serde(default)]
    by: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    changes: Vec<Change>,
}

/// How L-Echo and tools propose: the batch is kept; nothing is written to the items.
async fn proposal_add(State(s): State<Shared>, Json(b): Json<NewProposal>) -> Result<(StatusCode, Json<Value>), HubError> {
    let h = hub(&s)?;
    let p = h.proposals.create(&b.by, &b.title, b.changes)?;
    Ok((StatusCode::CREATED, Json(json!({ "proposal": p }))))
}

#[derive(Deserialize, Default)]
struct ApplyBody {
    chosen: Option<Vec<usize>>,
}

async fn proposal_apply(State(s): State<Shared>, Path(id): Path<String>, body: axum::body::Bytes) -> R {
    // no body (or `{}`) = apply every change
    let given: ApplyBody = if body.iter().all(u8::is_ascii_whitespace) {
        ApplyBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| HubError::new(StatusCode::BAD_REQUEST, "要套用哪幾條，寫成 {\"chosen\": [0, 2]}"))?
    };
    let mut h = hub(&s)?;
    let chosen = given.chosen;
    let p = h.apply_proposal(&id, chosen.as_deref())?;
    place_picture(&mut h, &s.hub_dirs, &p);
    ok(json!({ "proposal": p }))
}

// A family's tree (段落 → 小節 → 使用時機 for the writing rows): the built-in one until the
// author changes it; items are placed on it by their `place` tags (paths, the first the main one).

async fn tree_get(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> R {
    let h = hub(&s)?;
    let family = q.get("family").map(String::as_str).unwrap_or("");
    ok(json!({ "family": family, "tree": reader::tree(h.store.hub(), family) }))
}

async fn tree_put(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>, Json(body): Json<Value>) -> R {
    let h = hub(&s)?;
    let family = q.get("family").filter(|f| !f.trim().is_empty()).ok_or_else(|| HubError::new(StatusCode::BAD_REQUEST, "要說是哪一類的樹"))?;
    reader::save_tree(h.store.hub(), family, &body)?;
    ok(json!({ "family": family, "tree": reader::tree(h.store.hub(), family) }))
}

// A proposal may come with one picture (L-Echo's split of a figure): it waits beside the
// proposal and, when the figure's reference is approved, becomes that reference's picture.

#[derive(Deserialize)]
struct PictureBody {
    /// The file name the proposal's figure item carries in `fields.image`.
    name: String,
    /// `data:image/png;base64,…` (PNG or JPEG, at most 10 MB).
    data_url: String,
}

fn picture_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

fn waiting_picture(hub_dir: &FsPath, id: &str) -> Option<(PathBuf, String, &'static str)> {
    let plain = !id.is_empty() && !id.contains(['/', '\\', ':']) && !id.starts_with('.');
    if !plain {
        return None;
    }
    let dir = hub_dir.join("proposals");
    let name = std::fs::read_to_string(dir.join(format!("{id}.pic.txt"))).ok()?;
    ["png", "jpg"].into_iter().find_map(|ext| {
        let f = dir.join(format!("{id}.pic.{ext}"));
        f.is_file().then(|| (f, name.trim().to_string(), ext))
    })
}

async fn proposal_picture(State(s): State<Shared>, Path(id): Path<String>, Json(b): Json<PictureBody>) -> R {
    use base64::Engine;
    let h = hub(&s)?;
    h.proposals.get(&id)?;
    let data = b.data_url.split_once("base64,").map(|x| x.1).unwrap_or(&b.data_url);
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| HubError::new(StatusCode::BAD_REQUEST, "這張圖讀不了"))?;
    let ext = picture_ext(&bytes).ok_or_else(|| HubError::new(StatusCode::BAD_REQUEST, "只收 PNG 或 JPEG"))?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(HubError::new(StatusCode::PAYLOAD_TOO_LARGE, "圖片超過 10 MB"));
    }
    let dir = h.store.hub().join("proposals");
    std::fs::create_dir_all(&dir).map_err(KnowError::from)?;
    std::fs::write(dir.join(format!("{id}.pic.{ext}")), bytes).map_err(KnowError::from)?;
    std::fs::write(dir.join(format!("{id}.pic.txt")), b.name.trim()).map_err(KnowError::from)?;
    ok(json!({ "kept": true }))
}

/// After an apply: the waiting picture goes into the folder of the approved reference that names
/// it (`ref.<ext>`, what the references' reader shows), and the index is rebuilt. Silent when
/// there is none, or when the reference was not approved (the picture then keeps waiting).
fn place_picture(h: &mut lore_know::hub::Hub, app_dirs: &BTreeMap<String, PathBuf>, p: &lore_know::Proposal) {
    let Some((file, name, ext)) = waiting_picture(h.store.hub(), &p.id) else { return };
    for c in p.changes.iter().filter(|c| c.op == "add" && c.result == "applied") {
        let Some(it) = c.item.as_ref().filter(|i| i.format == format::FIGURE && i.field("image") == name) else { continue };
        // the change keeps the temporary id: the stored item is found by its title in its category
        let Ok(items) = h.store.list(&it.category) else { continue };
        let Some(stored) = items.iter().find(|x| x.title == it.title && !x.entity) else { continue };
        let (Some(cat), Some((_, local))) = (h.store.registry().get(&stored.category).cloned(), split_id(&stored.id)) else { continue };
        if cat.native.kind != "dir" || local.contains("..") {
            continue;
        }
        let Some(folder) = app_dirs.get(&cat.native.app_dir).map(|d| d.join(&cat.native.path).join(local)) else { continue };
        if std::fs::create_dir_all(&folder).is_ok() && std::fs::copy(&file, folder.join(format!("ref.{ext}"))).is_ok() {
            let _ = std::fs::remove_file(&file);
            let _ = std::fs::remove_file(file.with_file_name(format!("{}.pic.txt", p.id)));
            let _ = h.reindex();
        }
        return;
    }
}

async fn proposal_undo(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let mut h = hub(&s)?;
    let not_undone = h.undo_proposal(&id)?;
    let p = h.proposals.get(&id)?;
    ok(json!({ "proposal": p, "not_undone": not_undone }))
}

async fn proposal_dismiss(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let h = hub(&s)?;
    let p = h.proposals.dismiss(&id)?;
    ok(json!({ "proposal": p }))
}

// ── outside applications ─────────────────────────────────────────────────────

/// `own=1`: the author says the file is his own (an export of a Lore or hand-made category, or a
/// file he wrote). Such a file has no source and imports as his, never replacing anything; without
/// the flag a file with no source is refused, so a forgotten `source` cannot pass as his own.
/// (`Hub::import` has no such flag: the empty `source` is how it knows.)
async fn import(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>, body: String) -> R {
    let mut batch: Batch = intake::parse(&body)?;
    let own = q.get("own").is_some_and(|v| flag(v));
    if own {
        batch.source = String::new();
    } else if batch.source.trim().is_empty() {
        return Err(HubError::new(StatusCode::BAD_REQUEST, "這個檔案沒有寫來源；如果是自己的檔案，請標明是自己的"));
    }
    let report = hub(&s)?.import(&batch, own)?;
    ok(json!({ "category": report.category, "added": report.added, "replaced": report.replaced, "removed": report.removed, "problems": report.problems }))
}

fn parse_source(s: &str) -> Result<Source, HubError> {
    match s.split_once(':') {
        None if s == "author" => Ok(Source::author()),
        Some(("lore", app)) if !app.is_empty() => Ok(Source::lore(app)),
        Some(("external", name)) if !name.trim().is_empty() => Ok(Source::external(name)),
        _ => Err(HubError::new(StatusCode::BAD_REQUEST, "來源要寫成 author、lore:<程式> 或 external:<名字>")),
    }
}

async fn export(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Result<Response, HubError> {
    let category = q.get("category").map(|c| c.trim()).filter(|c| !c.is_empty()).ok_or_else(|| HubError::new(StatusCode::BAD_REQUEST, "要說明匯出哪一類"))?;
    let source = match q.get("source").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(s) => Some(parse_source(s)?),
        None => None,
    };
    let batch = hub(&s)?.export(category, source.as_ref())?;
    let name = format!("{}.json", lore_know::safe_local(category));
    let mut res = Response::new(Body::from(intake::to_text(&batch)));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
    res.headers_mut().insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).unwrap_or(HeaderValue::from_static("attachment")));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

async fn sources(State(s): State<Shared>) -> R {
    let h = hub(&s)?;
    let list = intake::sources(&h.store, h.store.registry().all())?;
    let out: Vec<Value> = list
        .into_iter()
        .map(|i| json!({ "name": i.name, "sets": i.sets.into_iter().map(|(category, count, aligned)| json!({ "category": category, "count": count, "aligned": aligned })).collect::<Vec<_>>() }))
        .collect();
    ok(out)
}

#[cfg(test)]
mod tests;
