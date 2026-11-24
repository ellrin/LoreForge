//! One or two pages of a deck from another app: request → GET → proxies (edit, revise, apply,
//! undo, image) → end, a replaced request, a selection and a page request ending each other,
//! against a fake app on 127.0.0.1 that records what it is sent.

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, Uri, header};
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use le_server::llm::Llm;
use le_server::{AppState, Shared};
use serde_json::{Value, json};
use tower::ServiceExt;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake";

async fn raw(s: &Shared, method: &str, uri: &str, body: Value) -> (StatusCode, String, Bytes) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(if method == "GET" { Body::empty() } else { Body::from(body.to_string()) }).unwrap();
    let res = le_server::router_isolated(s.clone()).oneshot(req).await.unwrap();
    let st = res.status();
    let ct = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    (st, ct, res.into_body().collect().await.unwrap().to_bytes())
}

async fn call(s: &Shared, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let (st, _, b) = raw(s, method, uri, body).await;
    (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn state(root: &Path) -> Shared {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ex = manifest.join("../../../private-example/LoreEcho");
    let to = root.join("private-example/LoreEcho");
    fn copy(a: &Path, b: &Path) {
        std::fs::create_dir_all(b).unwrap();
        for e in std::fs::read_dir(a).unwrap().flatten() {
            if e.path().is_dir() { copy(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
        }
    }
    copy(&ex, &to);
    std::fs::create_dir_all(root.join("private")).unwrap();
    AppState::new(lore_home::Home::at(root), root.join("web"), Llm::none(), None)
}

/// (method, path?query as received, body as JSON or Null)
type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

/// A fake asking app: records every call; `apply` answers `apply_status` when set.
async fn fake_app(apply_status: Arc<Mutex<Option<u16>>>) -> (u16, Log) {
    let log: Log = Arc::default();
    let l = log.clone();
    let app = Router::new().fallback(move |m: Method, uri: Uri, b: Bytes| {
        let (l, st) = (l.clone(), apply_status.clone());
        async move {
            let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
            let pq = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
            l.lock().unwrap().push((m.to_string(), pq, v.clone()));
            let p = uri.path();
            if p.contains("/image/") {
                return ([(header::CONTENT_TYPE, "image/png")], PNG).into_response();
            }
            if p.ends_with("/page-edit") {
                return (StatusCode::CREATED, axum::Json(json!({ "id": "e1", "got": v }))).into_response();
            }
            if p.ends_with("/apply") {
                if let Some(code) = *st.lock().unwrap() {
                    return (StatusCode::from_u16(code).unwrap(), axum::Json(json!({ "error": "頁面已經改過" }))).into_response();
                }
                return axum::Json(json!({ "ok": true, "message": "已套用" })).into_response();
            }
            axum::Json(json!({ "ok": true, "got": v })).into_response()
        }
    });
    let lis = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = lis.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(lis, app).await.unwrap() });
    (port, log)
}

fn request(id: &str, port: u16, deck: &str, n: usize) -> Value {
    let slides: Vec<Value> = (0..n).map(|i| json!({ "id": format!("s{i}"), "n": i + 3, "title": format!("第 {} 頁", i + 3) })).collect();
    json!({ "request": id, "app": "sc", "port": port, "deck": deck, "source": "A synthetic deck", "slides": slides })
}

fn selection(id: &str, port: u16) -> Value {
    json!({ "request": id, "app": "sc", "port": port, "doc": "deck-a", "source": "slide 3", "text": "一句", "scope": "bullet" })
}

async fn heard(log: &Log, n: usize) -> Vec<(String, String, Value)> {
    for _ in 0..200 {
        let l = log.lock().unwrap().clone();
        if l.len() >= n {
            return l;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    log.lock().unwrap().clone()
}

const PROXIES: [(&str, &str); 5] = [("POST", "/api/page/edit"), ("POST", "/api/page/edit/e1/revise"), ("POST", "/api/page/edit/e1/apply"), ("POST", "/api/page/edit/e1/undo"), ("GET", "/api/page/edit/e1/image/before-0")];

#[tokio::test(flavor = "multi_thread")]
async fn page_request_proxies_and_end() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let apply: Arc<Mutex<Option<u16>>> = Arc::default();
    let (port, log) = fake_app(apply.clone()).await;

    // nothing open: GET closed, every proxy 409
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1, json!({ "open": false }));
    for (m, p) in PROXIES {
        let (c, v) = call(&s, m, p, json!({})).await;
        assert_eq!((c, v["error"].as_str()), (StatusCode::CONFLICT, Some("沒有交給 L-Echo 的頁面")), "{p}");
    }

    // a request: kept, shown
    let (c, v) = call(&s, "POST", "/api/page", request("r1", port, "deck a", 2)).await;
    assert_eq!((c, v), (StatusCode::OK, json!({ "ok": true })));
    let v = call(&s, "GET", "/api/page", Value::Null).await.1;
    assert_eq!(v, json!({ "open": true, "request": "r1", "app": "sc", "deck": "deck a", "source": "A synthetic deck", "slides": [{ "id": "s0", "n": 3, "title": "第 3 頁" }, { "id": "s1", "n": 4, "title": "第 4 頁" }] }));

    // the edit: the deck name encoded in the path, the slide ids added when absent, the app's status and JSON as they are
    let (c, v) = call(&s, "POST", "/api/page/edit?x=1", json!({ "instruction": "把這頁拆成兩頁", "engine": "claude" })).await;
    assert_eq!(c, StatusCode::CREATED);
    assert_eq!(v, json!({ "id": "e1", "got": { "instruction": "把這頁拆成兩頁", "engine": "claude", "slides": ["s0", "s1"] } }));
    // slides the page sent are kept as they are
    call(&s, "POST", "/api/page/edit", json!({ "instruction": "改備忘稿", "slides": ["s1"] })).await;
    // revise, apply, undo: body unchanged
    assert_eq!(call(&s, "POST", "/api/page/edit/e1/revise", json!({ "instruction": "短一點" })).await, (StatusCode::OK, json!({ "ok": true, "got": { "instruction": "短一點" } })));
    assert_eq!(call(&s, "POST", "/api/page/edit/e1/apply", json!({})).await, (StatusCode::OK, json!({ "ok": true, "message": "已套用" })));
    assert_eq!(call(&s, "POST", "/api/page/edit/e1/undo", json!({})).await.0, StatusCode::OK);
    // the image: bytes and content type passed through
    let (c, ct, b) = raw(&s, "GET", "/api/page/edit/e1/image/after-1", Value::Null).await;
    assert_eq!((c, ct.as_str(), &b[..]), (StatusCode::OK, "image/png", PNG));
    {
        let l = log.lock().unwrap();
        let got: Vec<(&str, &str)> = l.iter().map(|(m, p, _)| (m.as_str(), p.as_str())).collect();
        assert_eq!(got, vec![
            ("POST", "/api/decks/deck%20a/page-edit?x=1"),
            ("POST", "/api/decks/deck%20a/page-edit"),
            ("POST", "/api/decks/deck%20a/page-edit/e1/revise"),
            ("POST", "/api/decks/deck%20a/page-edit/e1/apply"),
            ("POST", "/api/decks/deck%20a/page-edit/e1/undo"),
            ("GET", "/api/decks/deck%20a/page-edit/e1/image/after-1"),
        ]);
        assert_eq!(l[1].2, json!({ "instruction": "改備忘稿", "slides": ["s1"] }));
    }
    // an edit body that is not a JSON object: 400, nothing sent
    log.lock().unwrap().clear();
    assert_eq!(call(&s, "POST", "/api/page/edit", json!("x")).await.0, StatusCode::BAD_REQUEST);
    assert!(log.lock().unwrap().is_empty());

    // the app refuses an apply: its 409 passes through, the request stays
    *apply.lock().unwrap() = Some(409);
    let (c, v) = call(&s, "POST", "/api/page/edit/e1/apply", json!({})).await;
    assert_eq!((c, v["error"].as_str()), (StatusCode::CONFLICT, Some("頁面已經改過")));
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1["open"], json!(true));

    // end: the app is told, the state is gone
    log.lock().unwrap().clear();
    assert_eq!(call(&s, "POST", "/api/page/end", json!({})).await.0, StatusCode::OK);
    assert_eq!(log.lock().unwrap().clone(), vec![("POST".to_string(), "/api/lore/paged".to_string(), json!({ "request": "r1", "end": true }))]);
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1, json!({ "open": false }));
    assert_eq!(call(&s, "POST", "/api/page/edit", json!({})).await.0, StatusCode::CONFLICT);

    // a 410 from the app's apply ends the request here too
    call(&s, "POST", "/api/page", request("r2", port, "deck a", 1)).await;
    *apply.lock().unwrap() = Some(410);
    assert_eq!(call(&s, "POST", "/api/page/edit/e1/apply", json!({})).await.0, StatusCode::GONE);
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1, json!({ "open": false }));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_a_new_request_ends_the_old_one() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let (first, first_log) = fake_app(Arc::default()).await;
    let (second, second_log) = fake_app(Arc::default()).await;
    assert_eq!(call(&s, "POST", "/api/page", request("r1", first, "deck-a", 1)).await.0, StatusCode::OK);
    assert_eq!(call(&s, "POST", "/api/page", request("r2", second, "deck-b", 2)).await.0, StatusCode::OK);
    assert_eq!(heard(&first_log, 1).await, vec![("POST".to_string(), "/api/lore/paged".to_string(), json!({ "request": "r1", "end": true }))]);
    assert!(second_log.lock().unwrap().is_empty());
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1["request"], json!("r2"));
    // proxies now go to the second app
    call(&s, "POST", "/api/page/edit/e9/undo", json!({})).await;
    assert_eq!(second_log.lock().unwrap()[0].1, "/api/decks/deck-b/page-edit/e9/undo");
}

#[tokio::test(flavor = "multi_thread")]
async fn page_and_selection_end_each_other() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let (port, log) = fake_app(Arc::default()).await;

    // a page request, then a selection: the page request ends
    call(&s, "POST", "/api/page", request("p1", port, "deck-a", 1)).await;
    assert_eq!(call(&s, "POST", "/api/selection", selection("t1", port)).await.0, StatusCode::OK);
    assert_eq!(heard(&log, 1).await, vec![("POST".to_string(), "/api/lore/paged".to_string(), json!({ "request": "p1", "end": true }))]);
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1, json!({ "open": false }));
    assert_eq!(call(&s, "GET", "/api/selection", Value::Null).await.1["request"], json!("t1"));

    // a selection, then a page request: the selection ends
    log.lock().unwrap().clear();
    call(&s, "POST", "/api/page", request("p2", port, "deck-a", 1)).await;
    assert_eq!(heard(&log, 1).await, vec![("POST".to_string(), "/api/lore/selected".to_string(), json!({ "request": "t1", "end": true }))]);
    assert_eq!(call(&s, "GET", "/api/selection", Value::Null).await.1, json!({ "open": false }));
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1["request"], json!("p2"));
}

#[tokio::test]
async fn page_bad_requests_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let mut bad = request("r1", 7000, "d", 1);
    bad["app"] = json!("../x");
    assert_eq!(call(&s, "POST", "/api/page", bad).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/page", request("r1", 0, "d", 1)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/page", request("r1", 7000, "d", 0)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/page", request("r1", 7000, "d", 3)).await.0, StatusCode::BAD_REQUEST, "at most two slides");
    assert_eq!(call(&s, "POST", "/api/page", request(" ", 7000, "d", 1)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/page", request("r1", 7000, " ", 1)).await.0, StatusCode::BAD_REQUEST);
    let mut empty_id = request("r1", 7000, "d", 1);
    empty_id["slides"][0]["id"] = json!("");
    assert_eq!(call(&s, "POST", "/api/page", empty_id).await.0, StatusCode::BAD_REQUEST);
    let mut long = request("r1", 7000, "d", 1);
    long["slides"][0]["title"] = json!("字".repeat(lore_shell::pageedit::MAX_NAME + 1));
    assert_eq!(call(&s, "POST", "/api/page", long).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "GET", "/api/page", Value::Null).await.1, json!({ "open": false }));
}
