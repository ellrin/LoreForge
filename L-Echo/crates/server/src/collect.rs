//! 收件 (docs/brain-hub.md §16): the author reads English sentences aloud to L-Echo, or selects
//! them in another Lore app, one after another. L-Echo does no thinking here: each text is handed
//! to L-Brain's inbox at once (`POST /api/hub/inbox`) and the page learns how many wait, so he can
//! keep talking. Nothing is kept by L-Echo except the two remembered choices below.
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/collect | `{ text, via?: "voice"\|"typed", category?, own? }` → `{ added, sentences, counts, category, label }` |
//! | POST | /api/collect/selection | `{ category?, own? }`: the selection held from another app goes in (`via = "selection"`) → same answer; none → 409 |
//! | GET | /api/collect | `{ category, label, own, targets: [{id, label, group}], counts, brain }` |
//! | PUT | /api/collect | `{ category?, own? }` saved under `[collect]` of settings.toml → same as GET |
//!
//! L-Brain closed = 503 and NOTHING is kept anywhere else (no queue, no file): the author sees at
//! once that it did not go through and can say it again. GET still answers (`brain: false`).

#![allow(clippy::result_large_err)]

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Shared, store};

/// The longest text handed over in one go (characters): a sentence or two, not a document.
const MAX_CHARS: usize = 4000;
const CLOSED: &str = "L-Brain 沒有開著，這句沒有送出，請再說一次";
/// The inbox takes these categories for now (the format of `pc.patterns`).
const INBOX_FORMAT: &str = "pattern";

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/collect", get(status).put(choose).post(collect_text)).route("/api/collect/selection", post(collect_selection))
}

fn bad(status: StatusCode, msg: impl AsRef<str>) -> Response {
    let line = msg.as_ref().lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    (status, Json(json!({ "error": line }))).into_response()
}

fn closed() -> Response {
    bad(StatusCode::SERVICE_UNAVAILABLE, CLOSED)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).no_proxy().build().unwrap_or_default()
}

async fn brain(s: &Shared) -> Result<String, Response> {
    s.kb.server().await.ok_or_else(closed)
}

async fn read(r: Result<reqwest::Response, reqwest::Error>) -> Result<Value, Response> {
    let r = r.map_err(|_| closed())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(v);
    }
    let line = v["error"].as_str().and_then(|e| e.lines().map(str::trim).find(|l| !l.is_empty())).map(str::to_string).unwrap_or_else(|| status.to_string());
    Err(bad(StatusCode::BAD_GATEWAY, format!("L-Brain 沒有接受：{line}")))
}

fn zero() -> Value {
    json!({ "waiting": 0, "working": 0, "ready": 0, "failed": 0 })
}

fn counts_of(v: &Value) -> Value {
    if v.is_object() { v.clone() } else { zero() }
}

fn choices(s: &Shared) -> store::Collect {
    s.dir.as_deref().map(store::collect).unwrap_or_default()
}

/// The categories the inbox can take: writable ones of the pattern format.
fn targets_of(cats: &Value) -> Vec<Value> {
    cats["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["writable"].as_bool() == Some(true) && c["format"].as_str() == Some(INBOX_FORMAT))
        .map(|c| json!({ "id": c["id"], "label": c["label"], "group": c["group"] }))
        .collect()
}

fn label_of(cats: &Value, id: &str) -> String {
    cats["categories"].as_array().and_then(|l| l.iter().find(|c| c["id"] == id)).and_then(|c| c["label"].as_str().map(str::to_string)).unwrap_or_else(|| id.to_string())
}

// ── sending ──

struct Hand<'a> {
    text: &'a str,
    via: &'a str,
    app: &'a str,
    doc: &'a str,
    category: Option<String>,
    own: Option<bool>,
}

/// Hand one text to L-Brain's inbox and answer for the page.
async fn hand_over(s: &Shared, h: Hand<'_>) -> Response {
    let text = h.text.trim();
    if text.is_empty() {
        return bad(StatusCode::BAD_REQUEST, "沒有文字可以送");
    }
    if text.chars().count() > MAX_CHARS {
        return bad(StatusCode::PAYLOAD_TOO_LARGE, format!("文字太長了（上限 {MAX_CHARS} 字）"));
    }
    let saved = choices(s);
    let category = h.category.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).unwrap_or(saved.category);
    let own = h.own.unwrap_or(saved.own);
    let base = match brain(s).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    // The author (2026-11-24): a sentence he hands over is one he likes, which is how he will
    // write; its wording is always kept as the example, whoever wrote it first. The flag stays in
    // the contract but is no longer a choice.
    let _ = own;
    let body = json!({ "category": category, "text": text, "own": true, "from": { "via": h.via, "app": h.app, "doc": h.doc } });
    let v = match read(client().post(format!("{base}/api/hub/inbox")).json(&body).send().await).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let added = v["added"].as_array().cloned().unwrap_or_default();
    let sentences: Vec<Value> = added.iter().map(|e| e["text"].clone()).collect();
    // the label is a courtesy: a failure here never undoes the hand-over
    let label = match read(client().get(format!("{base}/api/hub/categories")).send().await).await {
        Ok(c) => label_of(&c, &category),
        Err(_) => category.clone(),
    };
    Json(json!({ "added": added.len(), "sentences": sentences, "counts": counts_of(&v["counts"]), "category": category, "label": label })).into_response()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CollectBody {
    text: String,
    via: Option<String>,
    category: Option<String>,
    own: Option<bool>,
}

async fn collect_text(State(s): State<Shared>, body: Option<Json<CollectBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { text }") };
    let via = b.via.as_deref().unwrap_or("voice");
    if via != "voice" && via != "typed" {
        return bad(StatusCode::BAD_REQUEST, "via 只能是 voice 或 typed");
    }
    hand_over(&s, Hand { text: &b.text, via, app: "le", doc: "", category: b.category, own: b.own }).await
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SelectionBody {
    category: Option<String>,
    own: Option<bool>,
}

/// A document name that is really a path (a folder separator, `~`, a drive letter) is never sent.
fn is_path_like(doc: &str) -> bool {
    let d = doc.trim();
    d.contains('/') || d.contains('\\') || d.starts_with('~') || d.as_bytes().get(1) == Some(&b':')
}

async fn collect_selection(State(s): State<Shared>, body: Option<Json<SelectionBody>>) -> Response {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let Some(sel) = s.selection.current() else { return bad(StatusCode::CONFLICT, "沒有選取的文字") };
    let doc = if is_path_like(&sel.doc) { "" } else { sel.doc.trim() };
    // a sentence selected in a read-only document (a PDF in L-Reader) is someone else's
    let own = if sel.readonly { Some(false) } else { b.own };
    hand_over(&s, Hand { text: &sel.text, via: "selection", app: &sel.app, doc, category: b.category, own }).await
}

// ── the remembered choices and the running line ──

async fn answer(s: &Shared) -> Response {
    let c = choices(s);
    let (up, targets, counts, label) = match s.kb.server().await {
        None => (false, vec![], zero(), c.category.clone()),
        Some(base) => {
            let cats = read(client().get(format!("{base}/api/hub/categories")).send().await).await.unwrap_or(Value::Null);
            let counts = read(client().get(format!("{base}/api/hub/inbox/counts")).send().await).await.map(|v| counts_of(&v)).unwrap_or_else(|_| zero());
            (true, targets_of(&cats), counts, label_of(&cats, &c.category))
        }
    };
    Json(json!({ "category": c.category, "label": label, "own": c.own, "targets": targets, "counts": counts, "brain": up })).into_response()
}

async fn status(State(s): State<Shared>) -> Response {
    answer(&s).await
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ChooseBody {
    category: Option<String>,
    own: Option<bool>,
}

async fn choose(State(s): State<Shared>, body: Option<Json<ChooseBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { category, own }") };
    let Some(dir) = s.dir.clone() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "沒有 LoreForge 資料夾，記不住設定") };
    let r = {
        let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
        store::set_collect(&dir, b.category.as_deref(), b.own)
    };
    match r {
        Ok(_) => answer(&s).await,
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kbctx::Pin;
    use crate::llm::Llm;
    use crate::{AppState, Shared};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use lore_llm::router::MockFactory;
    use std::path::Path as FsPath;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    fn copy_dir(from: &FsPath, to: &FsPath) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            if e.path().is_dir() { copy_dir(&e.path(), &to.join(e.file_name())) } else { std::fs::copy(e.path(), to.join(e.file_name())).unwrap(); }
        }
    }

    // ── a fake L-Brain ──

    #[derive(Default)]
    struct Fake {
        /// `POST /api/hub/inbox` bodies
        posted: Mutex<Vec<Value>>,
    }

    fn categories() -> Value {
        json!({ "categories": [
            { "id": "pc.patterns", "label": "句型", "group": "論文寫作", "format": "pattern", "writable": true },
            { "id": "pc.more", "label": "更多句型", "group": "論文寫作", "format": "pattern", "writable": true },
            { "id": "pc.templates", "label": "論文模板", "group": "論文寫作", "format": "dir", "writable": false },
            { "id": "sc.flow", "label": "流程圖", "group": "簡報", "format": "figure", "writable": true } ],
            "formats": [], "sorts": [], "facets": [] })
    }

    async fn fake_brain() -> (String, Arc<Fake>) {
        let f = Arc::new(Fake::default());
        let a = f.clone();
        let app = axum::Router::new()
            .route(
                "/api/hub/inbox",
                axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                    let f = a.clone();
                    async move {
                        let mut posted = f.posted.lock().unwrap();
                        posted.push(body.clone());
                        let n = posted.len();
                        let entry = json!({ "id": format!("i{n:03}"), "category": body["category"], "text": body["text"], "own": body["own"], "from": body["from"], "state": "waiting" });
                        (StatusCode::CREATED, axum::Json(json!({ "added": [entry], "counts": { "waiting": n, "working": 0, "ready": 0, "failed": 0 } })))
                    }
                }),
            )
            .route("/api/hub/inbox/counts", axum::routing::get(|| async { axum::Json(json!({ "waiting": 3, "working": 1, "ready": 2, "failed": 0 })) }))
            .route("/api/hub/categories", axum::routing::get(|| async { axum::Json(categories()) }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (url, f)
    }

    // ── the app under test ──

    struct App {
        shared: Shared,
    }

    fn app_in(root: &FsPath, brain: Option<String>) -> App {
        let manifest = FsPath::new(env!("CARGO_MANIFEST_DIR"));
        if !root.join("private-example/LoreEcho").exists() {
            copy_dir(&manifest.join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
            std::fs::create_dir_all(root.join("private")).unwrap();
        }
        let shared = AppState::new(lore_home::Home::at(root), root.join("web"), Llm::with_factory(MockFactory::default()), None);
        shared.kb.pin(Pin { spaces: vec![], member: "ellrin".into(), url: brain });
        App { shared }
    }

    /// An app on a fresh temp folder; the folder lives as long as the guard.
    fn app(brain: Option<String>) -> (App, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (app_in(tmp.path(), brain), tmp)
    }

    async fn call(a: &App, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(Body::from(body.map(|b| b.to_string()).unwrap_or_default())).unwrap();
        let res = crate::router_isolated(a.shared.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn post(a: &App, uri: &str, body: Value) -> (StatusCode, Value) {
        call(a, "POST", uri, Some(body)).await
    }

    /// Another Lore app hands L-Echo a selection (the app's own port is never called in these tests).
    async fn select(a: &App, doc: &str, text: &str, readonly: bool) {
        let body = json!({ "request": "r1", "app": "lr", "doc": doc, "text": text, "readonly": readonly });
        let (s, v) = post(a, "/api/selection", body).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    const S1: &str = "We propose a method that reduces the cost of training.";

    // ── step 1 ──

    #[tokio::test]
    async fn spoken_words_go_to_lbrains_inbox_at_once() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        let (s, v) = post(&a, "/api/collect", json!({ "text": format!("  {S1}  ") })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["added"], 1);
        assert_eq!(v["sentences"], json!([S1]));
        assert_eq!(v["counts"]["waiting"], 1);
        assert_eq!((v["category"].as_str(), v["label"].as_str()), (Some("pc.patterns"), Some("句型")));
        let posted = f.posted.lock().unwrap().clone();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0]["text"], S1);
        assert_eq!(posted[0]["from"], json!({ "via": "voice", "app": "le", "doc": "" }));
        println!("POST /api/collect → {v}\n  to L-Brain: {}", posted[0]);
        // typed is a way in too; anything else is not
        let (s, _) = post(&a, "/api/collect", json!({ "text": S1, "via": "typed" })).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(f.posted.lock().unwrap()[1]["from"]["via"], "typed");
        let (s, _) = post(&a, "/api/collect", json!({ "text": S1, "via": "telepathy" })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_remembered_category_and_own_flag_are_used() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        let (s, _) = call(&a, "PUT", "/api/collect", Some(json!({ "category": "pc.more", "own": true }))).await;
        assert_eq!(s, StatusCode::OK);
        let (_, v) = post(&a, "/api/collect", json!({ "text": S1 })).await;
        assert_eq!((v["category"].as_str(), v["label"].as_str()), (Some("pc.more"), Some("更多句型")));
        // a request's own choice wins for that request only
        let (_, v) = post(&a, "/api/collect", json!({ "text": S1, "category": "pc.patterns", "own": false })).await;
        assert_eq!(v["category"], "pc.patterns");
        let (_, v) = post(&a, "/api/collect", json!({ "text": S1 })).await;
        assert_eq!(v["category"], "pc.more");
        let posted = f.posted.lock().unwrap().clone();
        assert_eq!((posted[0]["category"].as_str(), posted[0]["own"].as_bool()), (Some("pc.more"), Some(true)));
        assert_eq!((posted[1]["category"].as_str(), posted[1]["own"].as_bool()), (Some("pc.patterns"), Some(true)));
        assert_eq!(posted[2]["own"], true);
        // never chosen: the pattern category, not his own
        let (url2, f2) = fake_brain().await;
        let (b, _t2) = app(Some(url2));
        post(&b, "/api/collect", json!({ "text": S1 })).await;
        let p = f2.posted.lock().unwrap().clone();
        assert_eq!((p[0]["category"].as_str(), p[0]["own"].as_bool()), (Some("pc.patterns"), Some(true)));
    }

    #[tokio::test]
    async fn lbrain_closed_is_a_503_and_nothing_is_kept() {
        let (a, _t) = app(None);
        let dir = a.shared.dir.clone().unwrap();
        let listing = |d: &FsPath| {
            let mut v: Vec<String> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            v.sort();
            v
        };
        let before = listing(&dir);
        let (s, v) = post(&a, "/api/collect", json!({ "text": S1 })).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert!(v["error"].as_str().unwrap().contains("L-Brain 沒有開著") && !v["error"].as_str().unwrap().contains('\n'), "{v}");
        // an address nobody listens on is the same
        let (b, _t2) = app(Some("http://127.0.0.1:9".into()));
        let (s, _) = post(&b, "/api/collect", json!({ "text": S1 })).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        // nothing was written anywhere in L-Echo's folder (no queue, no settings)
        assert_eq!(listing(&dir), before);
    }

    #[tokio::test]
    async fn empty_and_oversized_texts_are_refused() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        for t in ["", "   \n "] {
            let (s, _) = post(&a, "/api/collect", json!({ "text": t })).await;
            assert_eq!(s, StatusCode::BAD_REQUEST);
        }
        let (s, v) = post(&a, "/api/collect", json!({ "text": "a".repeat(4001) })).await;
        assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!v["error"].as_str().unwrap().contains('\n'));
        let (s, _) = post(&a, "/api/collect", json!({ "text": "b".repeat(4000) })).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(f.posted.lock().unwrap().len(), 1, "refused texts never reach L-Brain");
    }

    // ── step 2 ──

    #[tokio::test]
    async fn the_current_selection_goes_to_the_inbox_with_where_it_came_from() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        select(&a, "Notes on Sampling", S1, false).await;
        let (s, v) = post(&a, "/api/collect/selection", json!({})).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!((v["added"].as_u64(), v["sentences"].clone()), (Some(1), json!([S1])));
        let posted = f.posted.lock().unwrap().clone();
        assert_eq!(posted[0]["from"], json!({ "via": "selection", "app": "lr", "doc": "Notes on Sampling" }));
        println!("POST /api/collect/selection → {v}\n  to L-Brain: {}", posted[0]);
    }

    #[tokio::test]
    async fn a_read_only_selection_keeps_its_wording_too() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        call(&a, "PUT", "/api/collect", Some(json!({ "own": true }))).await;
        select(&a, "A Paper", S1, true).await;
        post(&a, "/api/collect/selection", json!({ "own": true })).await;
        // a selection that can be changed is his, as remembered
        select(&a, "My Draft", S1, false).await;
        post(&a, "/api/collect/selection", json!({})).await;
        let p = f.posted.lock().unwrap().clone();
        assert_eq!(p[0]["own"], true, "what he hands over keeps its wording, also from a read-only document");
        assert_eq!(p[1]["own"], true);
    }

    #[tokio::test]
    async fn no_selection_is_a_409() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        let (s, v) = post(&a, "/api/collect/selection", json!({})).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "沒有選取的文字");
        assert!(f.posted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_path_is_never_sent_as_the_documents_title() {
        let (url, f) = fake_brain().await;
        let (a, _t) = app(Some(url));
        for doc in ["/home/ellrin/papers/x.pdf", "~/papers/x.pdf", "C:\\Users\\ellrin\\x.pdf", "papers/x.pdf"] {
            select(&a, doc, S1, true).await;
            let (s, _) = post(&a, "/api/collect/selection", json!({})).await;
            assert_eq!(s, StatusCode::OK);
        }
        let p = f.posted.lock().unwrap().clone();
        assert!(p.iter().all(|b| b["from"]["doc"] == ""), "{p:?}");
    }

    // ── step 3 ──

    #[tokio::test]
    async fn the_choices_are_remembered_across_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let a = app_in(tmp.path(), None);
        let dir = a.shared.dir.clone().unwrap();
        std::fs::write(dir.join("settings.toml"), "later = 1\n[collect]\nkept = \"yes\"\n").unwrap();
        let (s, v) = call(&a, "PUT", "/api/collect", Some(json!({ "category": "pc.more", "own": true }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let b = app_in(tmp.path(), None);
        let (s, v) = call(&b, "GET", "/api/collect", None).await;
        assert_eq!((s, v["category"].as_str(), v["own"].as_bool()), (StatusCode::OK, Some("pc.more"), Some(true)), "{v}");
        let t = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(t.contains("later = 1") && t.contains("kept = \"yes\"") && t.contains("pc.more"), "{t}");
        // a partial change keeps the other choice; nonsense is refused
        let (_, v) = call(&b, "PUT", "/api/collect", Some(json!({ "own": false }))).await;
        assert_eq!((v["category"].as_str(), v["own"].as_bool()), (Some("pc.more"), Some(false)));
        let (s, _) = call(&b, "PUT", "/api/collect", Some(json!({ "category": "a b\n" }))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_page_learns_the_targets_and_the_counts() {
        let (url, _) = fake_brain().await;
        let (a, _t) = app(Some(url));
        let (s, v) = call(&a, "GET", "/api/collect", None).await;
        assert_eq!(s, StatusCode::OK);
        let ids: Vec<&str> = v["targets"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["pc.patterns", "pc.more"], "only writable pattern categories");
        assert_eq!(v["targets"][0], json!({ "id": "pc.patterns", "label": "句型", "group": "論文寫作" }));
        assert_eq!((v["brain"].clone(), v["counts"].clone(), v["label"].clone()), (json!(true), json!({ "waiting": 3, "working": 1, "ready": 2, "failed": 0 }), json!("句型")));
        println!("GET /api/collect → {v}");
        let (_, v) = call(&a, "PUT", "/api/collect", Some(json!({ "category": "pc.more" }))).await;
        println!("PUT /api/collect → {v}");
        assert_eq!(v["label"], "更多句型");
    }

    #[tokio::test]
    async fn lbrain_closed_still_answers_the_remembered_choice() {
        let (a, _t) = app(None);
        let (s, _) = call(&a, "PUT", "/api/collect", Some(json!({ "category": "pc.more", "own": true }))).await;
        assert_eq!(s, StatusCode::OK);
        let (s, v) = call(&a, "GET", "/api/collect", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!((v["brain"].clone(), v["targets"].clone(), v["category"].clone(), v["own"].clone()), (json!(false), json!([]), json!("pc.more"), json!(true)));
        assert_eq!(v["counts"], json!({ "waiting": 0, "working": 0, "ready": 0, "failed": 0 }));
    }
}
