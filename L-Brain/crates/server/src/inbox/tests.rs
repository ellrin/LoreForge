//! Inbox routes and worker over fakes: an in-memory box, a fake processor, a mock model.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kb_core::{FixedClock, KbOptions};
use lore_know::Item;
use lore_know::inbox::{Context, Counts, Entry, Made, Processor, Request as Req, state};
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;
use crate::hub::HubState;
use crate::reminders::Reminders;
use crate::{AppState, Config};

const MANIFEST: &str = r#"
[[category]]
id = "t.own"
label = "自己的"
group = "寫作"
format = "rule"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "own.csv"

[[category]]
id = "t.words"
label = "用詞"
format = "wording"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "words.csv"

[[category]]
id = "t.layouts"
label = "版型檔"
format = "layout"
[category.native]
kind = "files"
app_dir = "App"
path = "layouts"
"#;

// ── fakes ────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeBox {
    entries: Mutex<Vec<Entry>>,
}

impl FakeBox {
    fn change(&self, id: &str, f: impl FnOnce(&mut Entry)) -> lore_know::Result<Entry> {
        let mut all = self.entries.lock().unwrap();
        let e = all.iter_mut().find(|e| e.id == id).ok_or_else(|| lore_know::KnowError::NotFound(id.into()))?;
        f(e);
        Ok(e.clone())
    }
}

impl Box_ for FakeBox {
    fn add(&self, category: &str, text: &str, own: bool, from: ib::From) -> lore_know::Result<Vec<Entry>> {
        let mut all = self.entries.lock().unwrap();
        if all.iter().any(|e| e.text == text && e.category == category && e.state != state::SKIPPED) {
            return Ok(vec![]);
        }
        let e = Entry { id: format!("i{:04}", all.len() + 1), category: category.into(), text: text.into(), own, from, state: state::WAITING.into(), ..Default::default() };
        all.push(e.clone());
        Ok(vec![e])
    }
    fn get(&self, id: &str) -> Option<Entry> {
        self.entries.lock().unwrap().iter().find(|e| e.id == id).cloned()
    }
    fn list(&self, states: &[&str]) -> Vec<Entry> {
        self.entries.lock().unwrap().iter().filter(|e| states.contains(&e.state.as_str())).cloned().collect()
    }
    fn counts(&self) -> Counts {
        let all = self.entries.lock().unwrap();
        let n = |s: &str| all.iter().filter(|e| e.state == s).count();
        Counts { waiting: n(state::WAITING), working: n(state::WORKING), ready: n(state::READY), failed: n(state::FAILED) }
    }
    fn next(&self) -> lore_know::Result<Option<Entry>> {
        let mut all = self.entries.lock().unwrap();
        Ok(all.iter_mut().find(|e| e.state == state::WAITING).map(|e| {
            e.state = state::WORKING.into();
            e.clone()
        }))
    }
    fn ready(&self, id: &str, made: Made, engine: &str) -> lore_know::Result<Entry> {
        self.change(id, |e| {
            e.state = state::READY.into();
            e.result = Some(made.item);
            e.similar = made.similar;
            e.engine = engine.into();
        })
    }
    fn failed(&self, id: &str, error: &str) -> lore_know::Result<Entry> {
        self.change(id, |e| {
            e.state = state::FAILED.into();
            e.error = error.into();
        })
    }
    fn recover(&self) -> lore_know::Result<usize> {
        let mut n = 0;
        for e in self.entries.lock().unwrap().iter_mut().filter(|e| e.state == state::WORKING) {
            e.state = state::WAITING.into();
            n += 1;
        }
        Ok(n)
    }
    fn accept(&self, sink: &mut dyn Sink, id: &str, edited: Option<Item>) -> lore_know::Result<(Entry, Item)> {
        let e = self.get(id).ok_or_else(|| lore_know::KnowError::NotFound(id.into()))?;
        let item = edited.or(e.result.clone()).ok_or_else(|| lore_know::KnowError::Invalid("no result".into()))?;
        let stored = sink.put(item)?;
        let entry = self.change(id, |e| {
            e.state = state::ADDED.into();
            e.added = stored.id.clone();
        })?;
        Ok((entry, stored))
    }
    fn skip(&self, id: &str) -> lore_know::Result<Entry> {
        self.change(id, |e| e.state = state::SKIPPED.into())
    }
    fn retry(&self, id: &str) -> lore_know::Result<Entry> {
        self.change(id, |e| {
            e.state = state::WAITING.into();
            e.error.clear();
        })
    }
}

/// Makes a `rule` from the reply's `title`; the reply `{"refuse": true}` is turned down.
#[derive(Default)]
struct FakeProcessor {
    seen: Mutex<Vec<(Value, Vec<String>)>>,
}

impl Processor for FakeProcessor {
    fn format(&self) -> &'static str {
        "rule"
    }
    fn request(&self, entry: &Entry, cx: &Context) -> Req {
        self.seen.lock().unwrap().push((cx.tree.clone(), cx.existing.iter().map(|i| i.title.clone()).collect()));
        Req { system: "Cut the sentence.".into(), user: entry.text.clone(), schema: json!({ "type": "object" }) }
    }
    fn read(&self, _entry: &Entry, _cx: &Context, reply: &Value) -> Result<Made, String> {
        if reply["refuse"].as_bool() == Some(true) {
            return Err("這句話切不出句型。".into());
        }
        let title = reply["title"].as_str().unwrap_or("untitled");
        let mut item = Item::new("t.own", "rule", title, lore_know::Source::author());
        for (k, v) in [("type", "soft"), ("scope", "abstract"), ("severity", "warn"), ("message", title)] {
            item.fields.insert(k.into(), json!(v));
        }
        Ok(Made { item, similar: vec![] })
    }
}

// ── harness ──────────────────────────────────────────────────────────────────

struct Env {
    _dir: tempfile::TempDir,
    shared: Shared,
    app: axum::Router,
    store: Arc<FakeBox>,
    proc_: Arc<FakeProcessor>,
}

fn setup(llm: crate::llm::Llm, with_inbox: bool) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let hubdir = dir.path().join("hubdata");
    std::fs::create_dir_all(hubdir.join("app")).unwrap();
    std::fs::write(hubdir.join("app/words.csv"), "avoid,prefer,note\n").unwrap();
    let p = dir.path().join("personal");
    std::fs::create_dir_all(p.join("nodes")).unwrap();
    std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default(), app_dir: None };
    let kb = crate::open(&config, KbOptions { clock: Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00")), ..Default::default() }).unwrap();
    let rem = Reminders::isolated(&dir.path().join("rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let state = HubState::isolated(&hubdir, MANIFEST).unwrap();
    let mut shared = AppState::with_hub(kb, config.web.clone(), rem, llm, state);
    let store = Arc::new(FakeBox::default());
    let proc_ = Arc::new(FakeProcessor::default());
    if with_inbox {
        let picker: Picker = Arc::new(recent);
        Arc::get_mut(&mut shared).unwrap().inbox = Some(InboxState { store: store.clone(), processors: vec![proc_.clone()], picker });
    }
    let app = crate::router_isolated(shared.clone());
    Env { _dir: dir, shared, app, store, proc_ }
}

fn mock(reply: Value) -> crate::llm::Llm {
    crate::llm::Llm::with_factory(MockFactory::default().with("local:current", Arc::new(MockProvider::new(vec![reply.to_string()]).named("mock:local"))))
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let s = res.status();
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn hand_over(e: &Env, text: &str) -> Value {
    let (s, v) = call(&e.app, "POST", "/api/hub/inbox", Some(json!({ "category": "t.own", "text": text, "own": true, "from": { "via": "voice" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

async fn queued(e: &Env, text: &str) -> String {
    hand_over(e, text).await["added"][0]["id"].as_str().unwrap().to_string()
}

/// A hand-over worked to `ready` by the worker.
async fn ready_entry(e: &Env, text: &str) -> String {
    let id = queued(e, text).await;
    assert!(work_once(&e.shared).await);
    assert_eq!(e.store.get(&id).unwrap().state, state::READY);
    id
}

// ── step 1: routes ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_hand_over_is_queued_and_counted() {
    let e = setup(crate::llm::Llm::none(), true);
    let v = hand_over(&e, "The model is trained on a small set.").await;
    println!("POST /api/hub/inbox -> {v}");
    assert_eq!(v["added"][0]["state"], "waiting");
    assert_eq!(v["added"][0]["from"]["via"], "voice");
    assert_eq!(v["counts"], json!({ "waiting": 1, "working": 0, "ready": 0, "failed": 0 }));
    // the same sentence again: nothing new, still 201
    let v = hand_over(&e, "The model is trained on a small set.").await;
    assert_eq!(v["added"], json!([]));
    assert_eq!(v["counts"]["waiting"], 1);
    let (s, v) = call(&e.app, "GET", "/api/hub/inbox/counts", None).await;
    println!("GET /api/hub/inbox/counts -> {v}");
    assert_eq!((s, v["waiting"].as_u64()), (StatusCode::OK, Some(1)));
    let (s, _) = call(&e.app, "POST", "/api/hub/inbox", Some(json!({ "category": "t.own", "text": "  " }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // an inbox that is off
    let off = setup(crate::llm::Llm::none(), false);
    let (s, v) = call(&off.app, "GET", "/api/hub/inbox/counts", None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn an_unknown_or_read_only_category_is_refused() {
    let e = setup(crate::llm::Llm::none(), true);
    for (cat, want) in [("t.nothing", StatusCode::BAD_REQUEST), ("t.layouts", StatusCode::FORBIDDEN), ("t.words", StatusCode::UNPROCESSABLE_ENTITY)] {
        let (s, v) = call(&e.app, "POST", "/api/hub/inbox", Some(json!({ "category": cat, "text": "Some words here." }))).await;
        assert_eq!(s, want, "{cat} {v}");
        let msg = v["error"].as_str().unwrap();
        assert!(!msg.is_empty() && !msg.contains('\n'));
    }
    assert_eq!(e.store.counts(), Counts::default());
}

#[tokio::test]
async fn ready_entries_list_with_their_line_and_similar_titles() {
    let e = setup(mock(json!({ "title": "Pattern A" })), true);
    let (s, old) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": "t.own", "title": "Old one", "fields": { "type": "soft", "scope": "abstract", "severity": "warn", "message": "Old one" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{old}");
    let old_id = old["item"]["id"].as_str().unwrap().to_string();
    let id = ready_entry(&e, "Results are shown in the table.").await;
    e.store.change(&id, |en| en.similar = vec![old_id.clone(), "t.own:gone".into()]).unwrap();
    queued(&e, "A second sentence waits.").await;
    let (s, v) = call(&e.app, "GET", "/api/hub/inbox?state=ready,failed", None).await;
    println!("GET /api/hub/inbox?state=ready,failed -> {v}");
    assert_eq!(s, StatusCode::OK);
    let rows = v["entries"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0]["id"].as_str(), rows[0]["state"].as_str()), (Some(id.as_str()), Some("ready")));
    assert!(!rows[0]["line"].as_str().unwrap().is_empty());
    assert_eq!(rows[0]["similar_items"], json!([{ "id": old_id, "title": "Old one" }]));
    assert_eq!(v["counts"]["waiting"], 1);
    // the default lists the waiting one too
    let (_, v) = call(&e.app, "GET", "/api/hub/inbox", None).await;
    assert_eq!(v["entries"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn accept_writes_through_the_hub_and_marks_the_entry() {
    let e = setup(mock(json!({ "title": "Pattern A" })), true);
    let id = ready_entry(&e, "Results are shown in the table.").await;
    // not ready -> 409, unknown -> 404
    let w = queued(&e, "Another one waits here.").await;
    let (s, _) = call(&e.app, "POST", &format!("/api/hub/inbox/{w}/accept"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, v) = call(&e.app, "POST", "/api/hub/inbox/i9999/accept", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    // an edited item
    let mut edited = e.store.get(&id).unwrap().result.unwrap();
    edited.title = "Pattern A, edited".into();
    edited.fields.insert("message".into(), json!("Pattern A, edited"));
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/inbox/{id}/accept"), Some(json!({ "item": edited }))).await;
    println!("POST /api/hub/inbox/{{id}}/accept -> {v}");
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["entry"]["state"], "added");
    let item_id = v["item"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["entry"]["added"], json!(item_id));
    // the hub (and its index) has it
    let (_, list) = call(&e.app, "GET", "/api/hub/items?category=t.own", None).await;
    assert!(list["items"].as_array().unwrap().iter().any(|i| i["id"] == json!(item_id) && i["title"] == "Pattern A, edited"), "{list}");
    let (s, _) = call(&e.app, "POST", &format!("/api/hub/inbox/{id}/accept"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn many_can_be_accepted_at_once() {
    let e = setup(mock(json!({ "title": "Pattern" })), true);
    let a = ready_entry(&e, "First sentence is here.").await;
    let b = ready_entry(&e, "Second sentence is here.").await;
    let w = queued(&e, "Third still waits.").await;
    let (s, v) = call(&e.app, "POST", "/api/hub/inbox/accept", Some(json!({ "ids": [a, b, w] }))).await;
    println!("POST /api/hub/inbox/accept -> {v}");
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["accepted"], 2);
    assert_eq!(v["refused"][0]["id"], json!(w));
    assert!(v["refused"][0]["error"].as_str().is_some_and(|m| !m.is_empty()));
}

#[tokio::test]
async fn skip_and_retry_answer_the_entry() {
    let e = setup(crate::llm::Llm::none(), true);
    let a = queued(&e, "Skip this sentence now.").await;
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/inbox/{a}/skip"), None).await;
    println!("POST /api/hub/inbox/{{id}}/skip -> {v}");
    assert_eq!((s, v["entry"]["state"].as_str()), (StatusCode::OK, Some("skipped")));
    let b = queued(&e, "Retry this sentence later.").await;
    let (s, _) = call(&e.app, "POST", &format!("/api/hub/inbox/{b}/retry"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    e.store.failed(&b, "沒做成。").unwrap();
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/inbox/{b}/retry"), None).await;
    println!("POST /api/hub/inbox/{{id}}/retry -> {v}");
    assert_eq!((s, v["entry"]["state"].as_str(), v["entry"]["error"].as_str()), (StatusCode::OK, Some("waiting"), Some("")));
    let (s, _) = call(&e.app, "POST", "/api/hub/inbox/i9999/skip", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// ── step 2: the worker ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_waiting_entry_becomes_ready_with_the_models_result() {
    let e = setup(mock(json!({ "title": "Pattern A" })), true);
    let id = queued(&e, "Results are shown in the table.").await;
    assert!(work_once(&e.shared).await);
    let got = e.store.get(&id).unwrap();
    assert_eq!(got.state, state::READY);
    assert_eq!(got.result.unwrap().title, "Pattern A");
    assert_eq!(got.engine, "mock:local");
    assert!(!work_once(&e.shared).await, "nothing waits any more");
}

#[tokio::test]
async fn a_reply_the_processor_refuses_is_a_failure_with_its_reason() {
    let e = setup(mock(json!({ "refuse": true })), true);
    let id = queued(&e, "Results are shown in the table.").await;
    assert!(work_once(&e.shared).await);
    let got = e.store.get(&id).unwrap();
    assert_eq!((got.state.as_str(), got.error.as_str()), (state::FAILED, "這句話切不出句型。"));
}

#[tokio::test]
async fn no_engine_is_a_failure_that_can_be_retried() {
    let e = setup(crate::llm::Llm::none(), true);
    let id = queued(&e, "Results are shown in the table.").await;
    assert!(work_once(&e.shared).await);
    let got = e.store.get(&id).unwrap();
    assert_eq!(got.state, state::FAILED);
    assert!(!got.error.is_empty() && !got.error.contains('\n'), "{}", got.error);
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/inbox/{id}/retry"), None).await;
    assert_eq!((s, v["entry"]["state"].as_str()), (StatusCode::OK, Some("waiting")));
    // the worker takes it again
    assert!(work_once(&e.shared).await);
    assert_eq!(e.store.get(&id).unwrap().state, state::FAILED);
}

#[tokio::test]
async fn the_worker_gives_the_tree_and_examples_to_the_processor() {
    let e = setup(mock(json!({ "title": "Pattern A" })), true);
    for t in ["Old one", "Older one"] {
        let (s, _) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": "t.own", "title": t, "fields": { "type": "soft", "scope": "abstract", "severity": "warn", "message": t } }))).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    // the family of t.own (group 寫作, app `t`) is `t/寫作`
    let tree = json!({ "levels": ["段落"], "tree": [{ "label": "intro", "children": [] }] });
    let (s, v) = call(&e.app, "PUT", "/api/hub/tree?family=t/%E5%AF%AB%E4%BD%9C", Some(tree.clone())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    queued(&e, "Results are shown in the table.").await;
    assert!(work_once(&e.shared).await);
    let seen = e.proc_.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, tree);
    let mut titles = seen[0].1.clone();
    titles.sort();
    assert_eq!(titles, ["Old one", "Older one"]);
}

// ── step 3: stats ────────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_carry_the_inbox_counts() {
    let e = setup(crate::llm::Llm::none(), true);
    queued(&e, "One sentence waits here.").await;
    let (s, v) = call(&e.app, "GET", "/api/hub/stats", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["inbox"], json!({ "waiting": 1, "working": 0, "ready": 0, "failed": 0 }));
    assert!(v["stats"].is_object() && v["cards"].is_object());
    let off = setup(crate::llm::Llm::none(), false);
    let (_, v) = call(&off.app, "GET", "/api/hub/stats", None).await;
    assert_eq!(v["inbox"], json!({ "waiting": 0, "working": 0, "ready": 0, "failed": 0 }));
}
