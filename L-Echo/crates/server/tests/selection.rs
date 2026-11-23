//! Text selected in another app: request → GET (kind) → apply / refusal / proxies → end, against a
//! fake app on 127.0.0.1 that records what it is sent.

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use le_server::llm::Llm;
use le_server::{AppState, Shared};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(s: &Shared, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(if method == "GET" { Body::empty() } else { Body::from(body.to_string()) }).unwrap();
    let res = le_server::router_isolated(s.clone()).oneshot(req).await.unwrap();
    let st = res.status();
    let b = res.into_body().collect().await.unwrap().to_bytes();
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

type Log = Arc<Mutex<Vec<(String, Value)>>>;

/// A fake asking app: records every call as (path?query, body); `/api/lore/selected` answers `refuse`'s status when set.
async fn fake_app(refuse: Arc<Mutex<Option<u16>>>) -> (u16, Log) {
    let log: Log = Arc::default();
    let rec = |log: &Log, path: &str, v: Value| log.lock().unwrap().push((path.to_string(), v));
    let (l1, l2, l3, l4, l5) = (log.clone(), log.clone(), log.clone(), log.clone(), log.clone());
    let app = Router::new()
        .route("/api/lore/selected", post(move |Json(v): Json<Value>| {
            let (l, r) = (l1.clone(), refuse.clone());
            async move {
                rec(&l, "/api/lore/selected", v.clone());
                match *r.lock().unwrap() {
                    Some(code) if v.get("end").is_none() => (StatusCode::from_u16(code).unwrap(), Json(json!({ "error": "選取已經改過" }))),
                    _ => (StatusCode::OK, Json(json!({ "ok": true, "message": "已套用" }))),
                }
            }
        }))
        .route("/api/polish/applied", post(move |Json(v): Json<Value>| {
            let l = l2.clone();
            async move {
                rec(&l, "/api/polish/applied", v);
                Json(json!({ "ok": true }))
            }
        }))
        .route("/api/polish", post(move |q: axum::extract::RawQuery, Json(v): Json<Value>| {
            let l = l3.clone();
            async move {
                rec(&l, &format!("/api/polish?{}", q.0.unwrap_or_default()), v.clone());
                (StatusCode::CREATED, Json(json!({ "id": "p1", "got": v })))
            }
        }))
        .route("/api/polish/history", get(move |q: axum::extract::RawQuery| {
            let l = l4.clone();
            async move {
                rec(&l, &format!("/api/polish/history?{}", q.0.unwrap_or_default()), Value::Null);
                Json(json!([{ "id": "old" }]))
            }
        }))
        .route("/api/lore/card", post(move |Json(v): Json<Value>| {
            let l = l5.clone();
            async move {
                rec(&l, "/api/lore/card", v);
                (StatusCode::CONFLICT, Json(json!({ "error": "已經有這張卡", "id": "n1" })))
            }
        }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (port, log)
}

fn request(id: &str, port: u16, text: &str) -> Value {
    json!({ "request": id, "app": "sc", "port": port, "doc": "deck-a", "source": "slide 3", "text": text, "prefix": "前", "suffix": "後", "context": "整段", "scope": "bullet", "keep": [], "language": "zh" })
}

fn paths(log: &Log) -> Vec<String> {
    log.lock().unwrap().iter().map(|(p, _)| p.clone()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn selection_request_apply_refuse_proxies_and_end() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let refuse: Arc<Mutex<Option<u16>>> = Arc::default();
    let (port, log) = fake_app(refuse.clone()).await;

    // nothing open: GET closed, apply and every proxy 409
    assert_eq!(call(&s, "GET", "/api/selection", Value::Null).await.1, json!({ "open": false }));
    assert_eq!(call(&s, "POST", "/api/selection/apply", json!({ "after": "x" })).await.0, StatusCode::CONFLICT);
    for (m, p) in [("POST", "/api/selection/polish"), ("POST", "/api/selection/polish/applied"), ("POST", "/api/selection/polish/like"), ("GET", "/api/selection/polish/history"), ("GET", "/api/selection/polish/presets"), ("POST", "/api/selection/lore/card")] {
        let (c, v) = call(&s, m, p, json!({})).await;
        assert_eq!((c, v["error"].as_str()), (StatusCode::CONFLICT, Some("沒有選取的文字")), "{p}");
    }

    // a request: kept, shown with its kind
    let (c, v) = call(&s, "POST", "/api/selection", request("r1", port, "注意力機制")).await;
    assert_eq!((c, v), (StatusCode::OK, json!({ "ok": true })));
    let v = call(&s, "GET", "/api/selection", Value::Null).await.1;
    assert_eq!(v, json!({ "open": true, "request": "r1", "app": "sc", "doc": "deck-a", "source": "slide 3", "text": "注意力機制", "scope": "bullet", "language": "zh", "kind": "term", "prefix": "前", "suffix": "後", "context": "整段", "keep": [] }));

    // apply with a polish id: the app hears the result, then the proposal was used
    let (c, v) = call(&s, "POST", "/api/selection/apply", json!({ "after": "注意力", "polish": "p1" })).await;
    assert_eq!((c, v), (StatusCode::OK, json!({ "ok": true, "message": "已套用" })));
    {
        let l = log.lock().unwrap();
        assert_eq!(l.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["/api/lore/selected", "/api/polish/applied"]);
        assert_eq!(l[0].1, json!({ "request": "r1", "after": "注意力", "polish": "p1" }));
        assert_eq!(l[1].1, json!({ "doc": "deck-a", "id": "p1" }));
    }
    // without a polish id: no /api/polish/applied
    log.lock().unwrap().clear();
    assert_eq!(call(&s, "POST", "/api/selection/apply", json!({ "after": "注意力" })).await.0, StatusCode::OK);
    assert_eq!(paths(&log), vec!["/api/lore/selected"]);

    // the app refuses: its status and words pass through, nothing marked used
    log.lock().unwrap().clear();
    *refuse.lock().unwrap() = Some(409);
    let (c, v) = call(&s, "POST", "/api/selection/apply", json!({ "after": "再改", "polish": "p2" })).await;
    assert_eq!((c, v["error"].as_str()), (StatusCode::CONFLICT, Some("選取已經改過")));
    assert_eq!(paths(&log), vec!["/api/lore/selected"]);
    *refuse.lock().unwrap() = None;
    assert_eq!(call(&s, "POST", "/api/selection/apply", json!({ "after": "  " })).await.0, StatusCode::BAD_REQUEST);

    // proxies: body and query as they came, the app's status and JSON as they are
    log.lock().unwrap().clear();
    let (c, v) = call(&s, "POST", "/api/selection/polish?x=1", json!({ "doc": "deck-a", "text": "注意力機制", "instruction": "簡潔" })).await;
    assert_eq!((c, v), (StatusCode::CREATED, json!({ "id": "p1", "got": { "doc": "deck-a", "text": "注意力機制", "instruction": "簡潔" } })));
    let (c, v) = call(&s, "GET", "/api/selection/polish/history?doc=deck-a&text=%E6%B3%A8", Value::Null).await;
    assert_eq!((c, v), (StatusCode::OK, json!([{ "id": "old" }])));
    let (c, v) = call(&s, "POST", "/api/selection/lore/card", json!({ "title": "注意力機制", "source": "deck-a" })).await;
    assert_eq!((c, v), (StatusCode::CONFLICT, json!({ "error": "已經有這張卡", "id": "n1" })));
    {
        let l = log.lock().unwrap();
        assert_eq!(l[0].0, "/api/polish?x=1");
        assert_eq!(l[1].0, "/api/polish/history?doc=deck-a&text=%E6%B3%A8");
        assert_eq!(l[2], ("/api/lore/card".to_string(), json!({ "title": "注意力機制", "source": "deck-a" })));
    }
    // a route the fake app lacks: its 404 comes back
    assert_eq!(call(&s, "GET", "/api/selection/polish/presets", Value::Null).await.0, StatusCode::NOT_FOUND);

    // end: the app is told, the state is gone
    log.lock().unwrap().clear();
    assert_eq!(call(&s, "POST", "/api/selection/end", json!({})).await.0, StatusCode::OK);
    assert_eq!(log.lock().unwrap().clone(), vec![("/api/lore/selected".to_string(), json!({ "request": "r1", "end": true }))]);
    assert_eq!(call(&s, "GET", "/api/selection", Value::Null).await.1, json!({ "open": false }));
    assert_eq!(call(&s, "POST", "/api/selection/polish", json!({})).await.0, StatusCode::CONFLICT);
    // nothing written
    let written: Vec<_> = walk(&tmp.path().join("private"));
    assert!(written.iter().all(|p| p.contains("/characters/") || p.ends_with("README.md")), "{written:?}");
}

fn walk(d: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        if e.path().is_dir() { out.extend(walk(&e.path())) } else { out.push(e.path().display().to_string()) }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn selection_a_new_request_ends_the_old_one() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let (first, first_log) = fake_app(Arc::default()).await;
    let (second, second_log) = fake_app(Arc::default()).await;
    assert_eq!(call(&s, "POST", "/api/selection", request("r1", first, "softmax")).await.0, StatusCode::OK);
    assert_eq!(call(&s, "POST", "/api/selection", request("r2", second, "This method works well on every dataset we tried.")).await.0, StatusCode::OK);
    let mut heard = vec![];
    for _ in 0..200 {
        heard = first_log.lock().unwrap().clone();
        if !heard.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(heard, vec![("/api/lore/selected".to_string(), json!({ "request": "r1", "end": true }))]);
    assert!(second_log.lock().unwrap().is_empty());
    let v = call(&s, "GET", "/api/selection", Value::Null).await.1;
    assert_eq!((v["request"].as_str(), v["kind"].as_str()), (Some("r2"), Some("passage")));
}

#[tokio::test]
async fn selection_bad_requests_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let s = state(tmp.path());
    let mut bad = request("r1", 7000, "一句");
    bad["app"] = json!("../x");
    assert_eq!(call(&s, "POST", "/api/selection", bad).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/selection", request("r1", 0, "一句")).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/selection", request("r1", 7000, " ")).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "POST", "/api/selection", request("r1", 7000, &"字".repeat(lore_shell::selection::MAX_TEXT + 1))).await.0, StatusCode::BAD_REQUEST);
    let mut long = request("r1", 7000, "一句");
    long["context"] = json!("字".repeat(lore_shell::selection::MAX_CONTEXT + 1));
    assert_eq!(call(&s, "POST", "/api/selection", long).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&s, "GET", "/api/selection", Value::Null).await.1, json!({ "open": false }));
}
