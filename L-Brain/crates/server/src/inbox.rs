//! 收件 over HTTP and its background worker (docs/brain-hub.md §16).
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/hub/inbox | `{category, text, own?, from?}` → 201 `{added, counts}` |
//! | GET | /api/hub/inbox?state=ready,failed | `{entries: [Row], counts}` |
//! | POST | /api/hub/inbox/{id}/accept | `{item?}` → `{entry, item}` |
//! | POST | /api/hub/inbox/accept | `{ids}` → `{accepted, refused}` |
//! | POST | /api/hub/inbox/{id}/skip, /retry | `{entry}` |
//! | GET | /api/hub/inbox/counts | `Counts` |
//!
//! Every error is a status and `{ "error": "<one line, Traditional Chinese>" }`. All the logic sits
//! behind [`Box_`] (the inbox store, lore-know package I1) and `Processor` (I2), so it is tested
//! with fakes.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_know::hub::Hub;
use lore_know::inbox::{self as ib, Context, Counts, Entry, Made, Processor, state};
use lore_know::index::Query as ItemQuery;
use lore_know::{Category, Item, Sink, reader};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;
use crate::hub::{HubError, family_of, hub, invalid};

/// The task the worker runs (`tasks.toml`).
pub const TASK: &str = "kb-inbox-pattern";
/// Most existing items of a category given to a processor as examples.
const EXISTING: usize = 30;
/// Rest between two rounds of the worker.
const REST: std::time::Duration = std::time::Duration::from_secs(3);

/// The inbox store, exactly the methods of lore-know's `Inbox` (`inbox_store.rs`, package I1).
pub trait Box_: Send + Sync {
    fn add(&self, category: &str, text: &str, own: bool, from: ib::From) -> lore_know::Result<Vec<Entry>>;
    fn get(&self, id: &str) -> Option<Entry>;
    fn list(&self, states: &[&str]) -> Vec<Entry>;
    fn counts(&self) -> Counts;
    fn next(&self) -> lore_know::Result<Option<Entry>>;
    fn ready(&self, id: &str, made: Made, engine: &str) -> lore_know::Result<Entry>;
    fn failed(&self, id: &str, error: &str) -> lore_know::Result<Entry>;
    fn recover(&self) -> lore_know::Result<usize>;
    fn accept(&self, sink: &mut dyn Sink, id: &str, edited: Option<Item>) -> lore_know::Result<(Entry, Item)>;
    fn skip(&self, id: &str) -> lore_know::Result<Entry>;
    fn retry(&self, id: &str) -> lore_know::Result<Entry>;
}

/// Chooses the existing items of a category given to the processor (at most [`EXISTING`]).
pub type Picker = Arc<dyn Fn(&Hub, &Category, &Entry) -> Vec<Item> + Send + Sync>;

/// What the routes and the worker need.
pub struct InboxState {
    pub store: Arc<dyn Box_>,
    /// One per format the inbox can make.
    pub processors: Vec<Arc<dyn Processor>>,
    pub picker: Picker,
}

/// The default picker: the most recently changed items of the category.
pub fn recent(h: &Hub, c: &Category, _entry: &Entry) -> Vec<Item> {
    let q = ItemQuery { category: Some(c.id.clone()), sort: "updated".into(), desc: true, limit: EXISTING, ..Default::default() };
    h.query(&q).map(|(items, _)| items).unwrap_or_default()
}

/// lore-know's inbox store behind the routes' trait (its methods already take `&self`).
impl Box_ for lore_know::inbox_store::Inbox {
    fn add(&self, category: &str, text: &str, _own: bool, from: lore_know::inbox::From) -> lore_know::Result<Vec<Entry>> {
        // The author (2026-11-24): what he hands over is how he will write, so its wording is
        // always kept as the example; the flag is no longer a choice.
        lore_know::inbox_store::Inbox::add(self, category, text, true, from)
    }
    fn get(&self, id: &str) -> Option<Entry> {
        lore_know::inbox_store::Inbox::get(self, id).ok()
    }
    fn list(&self, states: &[&str]) -> Vec<Entry> {
        lore_know::inbox_store::Inbox::list(self, states)
    }
    fn counts(&self) -> Counts {
        lore_know::inbox_store::Inbox::counts(self)
    }
    fn next(&self) -> lore_know::Result<Option<Entry>> {
        lore_know::inbox_store::Inbox::next(self)
    }
    fn ready(&self, id: &str, made: Made, engine: &str) -> lore_know::Result<Entry> {
        lore_know::inbox_store::Inbox::ready(self, id, made, engine)
    }
    fn failed(&self, id: &str, error: &str) -> lore_know::Result<Entry> {
        lore_know::inbox_store::Inbox::failed(self, id, error)
    }
    fn recover(&self) -> lore_know::Result<usize> {
        lore_know::inbox_store::Inbox::recover(self)
    }
    fn accept(&self, sink: &mut dyn Sink, id: &str, edited: Option<Item>) -> lore_know::Result<(Entry, Item)> {
        lore_know::inbox_store::Inbox::accept(self, sink, id, edited)
    }
    fn skip(&self, id: &str) -> lore_know::Result<Entry> {
        lore_know::inbox_store::Inbox::skip(self, id)
    }
    fn retry(&self, id: &str) -> lore_know::Result<Entry> {
        lore_know::inbox_store::Inbox::retry(self, id)
    }
}

/// The patterns shown to the model for a sentence: the ones closest to it (lore-know's
/// `examples_for`), from all of the category.
pub fn closest(h: &Hub, c: &Category, entry: &Entry) -> Vec<Item> {
    let q = ItemQuery { category: Some(c.id.clone()), sort: "updated".into(), desc: true, limit: 2000, ..Default::default() };
    let all = h.query(&q).map(|(items, _)| items).unwrap_or_default();
    lore_know::process_pattern::examples_for(&entry.text, &all, EXISTING)
}

/// The inbox of the running app: `<hub>/inbox`, sentences into patterns. `None` without a hub.
pub fn real(home: &lore_home::Home) -> Option<InboxState> {
    let dir = lore_know::reader::hub_dir(home)?.join("inbox");
    let store = Arc::new(lore_know::inbox_store::Inbox::open(dir).ok()?);
    Some(InboxState { store, processors: vec![Arc::new(lore_know::process_pattern::PatternProcessor)], picker: Arc::new(closest) })
}

fn off() -> HubError {
    HubError::new(StatusCode::SERVICE_UNAVAILABLE, "收件還沒有開啟。")
}

fn inbox(s: &Shared) -> Result<&InboxState, HubError> {
    s.inbox.as_ref().ok_or_else(off)
}

/// The counts for the stats line (zeros when the inbox is off).
pub fn counts(s: &Shared) -> Counts {
    s.inbox.as_ref().map(|i| i.store.counts()).unwrap_or_default()
}

type R = Result<Json<Value>, HubError>;

fn ok(v: impl serde::Serialize) -> R {
    Ok(Json(serde_json::to_value(v).map_err(|_| HubError::new(StatusCode::INTERNAL_SERVER_ERROR, "回覆組不起來。"))?))
}

fn find(s: &InboxState, id: &str) -> Result<Entry, HubError> {
    s.store.get(id).ok_or_else(|| HubError::new(StatusCode::NOT_FOUND, "找不到這一筆收件。"))
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/hub/inbox", get(list).post(add))
        .route("/api/hub/inbox/counts", get(counts_route))
        .route("/api/hub/inbox/accept", post(accept_many))
        .route("/api/hub/inbox/{id}/accept", post(accept_one))
        .route("/api/hub/inbox/{id}/skip", post(skip))
        .route("/api/hub/inbox/{id}/retry", post(retry))
}

// ── add ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Add {
    category: String,
    text: String,
    #[serde(default)]
    own: bool,
    #[serde(default)]
    from: ib::From,
}

async fn add(State(s): State<Shared>, Json(b): Json<Add>) -> Result<(StatusCode, Json<Value>), HubError> {
    let i = inbox(&s)?;
    if b.text.trim().is_empty() {
        return Err(HubError::new(StatusCode::BAD_REQUEST, "沒有收到文字。"));
    }
    {
        let h = hub(&s)?;
        let c = h.store.registry().get(&b.category).cloned().ok_or_else(|| HubError::new(StatusCode::BAD_REQUEST, format!("沒有「{}」這個類別。", b.category)))?;
        if !c.writable {
            return Err(HubError::new(StatusCode::FORBIDDEN, format!("「{}」只能看和加標籤，不能收件。", c.label)));
        }
        if !i.processors.iter().any(|p| p.format() == c.format) {
            return Err(HubError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("「{}」還不能收件整理。", c.label)));
        }
    }
    let added = i.store.add(&b.category, &b.text, b.own, b.from).map_err(HubError::from)?;
    Ok((StatusCode::CREATED, Json(json!({ "added": added, "counts": i.store.counts() }))))
}

// ── list ─────────────────────────────────────────────────────────────────────

/// An entry as the pages list it: the entry, the result's line, the titles of what it resembles.
fn row(h: &Hub, e: &Entry) -> Value {
    let line = e.result.as_ref().and_then(|r| h.formats.get(&r.format).map(|f| f.line(r))).unwrap_or_default();
    let similar: Vec<Value> = e
        .similar
        .iter()
        .filter_map(|id| h.store.get(id).ok().flatten().map(|i| json!({ "id": i.id, "title": i.title })))
        .collect();
    let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
    v["line"] = json!(line);
    v["similar_items"] = json!(similar);
    v
}

async fn list(State(s): State<Shared>, Query(q): Query<std::collections::HashMap<String, String>>) -> R {
    let i = inbox(&s)?;
    let wanted = q.get("state").map(String::as_str).filter(|v| !v.trim().is_empty()).unwrap_or("ready,failed,waiting,working");
    let states: Vec<&str> = wanted.split(',').map(str::trim).filter(|x| !x.is_empty()).collect();
    let entries = i.store.list(&states);
    let h = hub(&s)?;
    let rows: Vec<Value> = entries.iter().map(|e| row(&h, e)).collect();
    ok(json!({ "entries": rows, "counts": i.store.counts() }))
}

async fn counts_route(State(s): State<Shared>) -> R {
    ok(inbox(&s)?.store.counts())
}

// ── accept, skip, retry ──────────────────────────────────────────────────────

/// The hub as a [`Sink`], so that what the author adds also reaches the index.
struct HubSink<'a>(&'a mut Hub);

impl Sink for HubSink<'_> {
    fn get(&self, id: &str) -> lore_know::Result<Option<Item>> {
        self.0.store.get(id)
    }
    fn list(&self, category: &str) -> lore_know::Result<Vec<Item>> {
        self.0.store.list(category)
    }
    fn put(&mut self, item: Item) -> lore_know::Result<Item> {
        self.0.put(item)
    }
    fn remove(&mut self, id: &str) -> lore_know::Result<Item> {
        self.0.remove(id)
    }
    fn restore(&mut self, id: &str) -> lore_know::Result<Item> {
        self.0.restore(id)
    }
}

fn accept(s: &Shared, id: &str, edited: Option<Item>) -> Result<(Entry, Item), HubError> {
    let i = inbox(s)?;
    let e = find(i, id)?;
    if e.state != state::READY {
        return Err(HubError::new(StatusCode::CONFLICT, "這一筆還沒有整理好，或已經處理過了。"));
    }
    let mut h = hub(s)?;
    if let Some(item) = &edited {
        if item.category != e.category {
            return Err(HubError::new(StatusCode::BAD_REQUEST, "改過的內容要留在原來的類別。"));
        }
        invalid(&h, item)?;
    }
    i.store.accept(&mut HubSink(&mut h), id, edited).map_err(HubError::from)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct One {
    item: Option<Item>,
}

async fn accept_one(State(s): State<Shared>, Path(id): Path<String>, body: axum::body::Bytes) -> R {
    // the body is optional: `{}` or nothing accepts as it is
    let one: One = if body.iter().all(u8::is_ascii_whitespace) { One::default() } else { serde_json::from_slice(&body).map_err(|_| HubError::new(StatusCode::BAD_REQUEST, "改過的內容讀不懂。"))? };
    let (entry, item) = accept(&s, &id, one.item)?;
    ok(json!({ "entry": entry, "item": item }))
}

#[derive(Deserialize)]
struct Many {
    ids: Vec<String>,
}

async fn accept_many(State(s): State<Shared>, Json(b): Json<Many>) -> R {
    inbox(&s)?;
    let (mut accepted, mut refused) = (0, vec![]);
    for id in &b.ids {
        match accept(&s, id, None) {
            Ok(_) => accepted += 1,
            Err(e) => refused.push(json!({ "id": id, "error": e.message() })),
        }
    }
    ok(json!({ "accepted": accepted, "refused": refused }))
}

async fn skip(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let i = inbox(&s)?;
    find(i, &id)?;
    ok(json!({ "entry": i.store.skip(&id).map_err(HubError::from)? }))
}

async fn retry(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let i = inbox(&s)?;
    let e = find(i, &id)?;
    if e.state != state::FAILED {
        return Err(HubError::new(StatusCode::CONFLICT, "只有沒做成的才能重試。"));
    }
    ok(json!({ "entry": i.store.retry(&id).map_err(HubError::from)? }))
}

// ── the worker ───────────────────────────────────────────────────────────────

/// What the processor is told: the family's tree and a few existing items.
fn prepare(s: &Shared, i: &InboxState, e: &Entry) -> Result<(Arc<dyn Processor>, Context), String> {
    let m = s.hub.as_ref().ok_or("個人空間還沒有設定，這一筆沒有整理。")?;
    let h = m.lock().unwrap_or_else(|p| p.into_inner());
    let c = h.store.registry().get(&e.category).cloned().ok_or("這個類別已經不存在了。")?;
    let p = i.processors.iter().find(|p| p.format() == c.format).cloned().ok_or("這個類別還不能收件整理。")?;
    let tree = reader::tree(h.store.hub(), &family_of(&c)).unwrap_or(Value::Null);
    let mut existing = (i.picker)(&h, &c, e);
    existing.truncate(EXISTING);
    Ok((p, Context { tree, existing }))
}

fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// Work on the next waiting entry; `true` when there was one (done or failed).
pub async fn work_once(s: &Shared) -> bool {
    let Some(i) = s.inbox.as_ref() else { return false };
    let e = match i.store.next() {
        Ok(Some(e)) => e,
        Ok(None) => return false,
        Err(err) => {
            eprintln!("kb-server inbox: {err}");
            return false;
        }
    };
    let fail = |why: &str| {
        if let Err(err) = i.store.failed(&e.id, &one_line(why)) {
            eprintln!("kb-server inbox: {err}");
        }
        true
    };
    let (p, cx) = match prepare(s, i, &e) {
        Ok(x) => x,
        Err(why) => return fail(&why),
    };
    let req = p.request(&e, &cx);
    // nothing may hold the queue for ever: a model that says nothing for five minutes is a
    // failure the author can retry, and the next sentence gets its turn
    let call = s.llm.complete_task(TASK, &req.system, &req.user, req.schema.clone());
    let answer = match tokio::time::timeout(std::time::Duration::from_secs(300), call).await {
        Ok(Ok(a)) => a,
        Ok(Err(why)) => return fail(&why),
        Err(_) => return fail("模型五分鐘沒有回答，先跳過這一句；可以再試一次"),
    };
    let (made, engine) = match p.read(&e, &cx, &answer.value) {
        Ok(made) => (made, answer.engine),
        Err(first) => {
            // one more round with what was wrong (too many holes, a place the tree lacks, a
            // rewritten sentence): a model usually fixes it when told; a second refusal is the failure
            let user = format!("{}\n\nYour previous reply was refused by the checker: {first}\nPrevious reply: {}\nReply again, fixing exactly that and keeping every other rule.", req.user, answer.value);
            let call = s.llm.complete_task(TASK, &req.system, &user, req.schema.clone());
            let again = match tokio::time::timeout(std::time::Duration::from_secs(300), call).await {
                Ok(Ok(a)) => a,
                _ => return fail(&first),
            };
            match p.read(&e, &cx, &again.value) {
                Ok(made) => (made, again.engine),
                Err(why) => return fail(&why),
            }
        }
    };
    if let Err(err) = i.store.ready(&e.id, made, &engine) {
        eprintln!("kb-server inbox: {err}");
    }
    true
}

/// At start: entries left `working` by a stop go back to waiting; then entries are worked on one
/// at a time, a rest of a few seconds when nothing waits.
pub fn spawn_worker(s: Shared) {
    if s.inbox.is_none() {
        return;
    }
    tokio::spawn(async move {
        if let Some(i) = &s.inbox
            && let Err(e) = i.store.recover()
        {
            eprintln!("kb-server inbox: {e}");
        }
        loop {
            while work_once(&s).await {}
            tokio::time::sleep(REST).await;
        }
    });
}

#[cfg(test)]
mod tests;
