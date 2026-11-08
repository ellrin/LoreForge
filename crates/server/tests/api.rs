//! API tests over temp spaces (offline; nothing touches iCloud or the real private space).

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kb_core::{FixedClock, KbOptions};
use kb_server::{AppState, Config};
use serde_json::{Value, json};
use tower::ServiceExt;

fn setup() -> (tempfile::TempDir, axum::Router, Arc<FixedClock>) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("personal");
    std::fs::create_dir_all(p.join("nodes")).unwrap();
    std::fs::write(
        p.join("space.toml"),
        format!("id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n[assets]\nbackend = \"icloud\"\nroot = \"{}\"\n", dir.path().join("icloud").display()),
    )
    .unwrap();
    let h = dir.path().join("household");
    std::fs::create_dir_all(h.join("nodes")).unwrap();
    std::fs::write(h.join("space.toml"), "id = \"home\"\nname = \"家\"\nkind = \"household\"\nmembers = [\"ellrin\", \"daidai\"]\n").unwrap();
    let config = Config { spaces: vec![p, h], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default() };
    let clock = Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00"));
    let kb = kb_server::open(&config, KbOptions { clock: clock.clone(), ..Default::default() }).unwrap();
    let app = kb_server::router(Arc::new(AppState { kb: Mutex::new(kb), web: config.web.clone() }));
    (dir, app, clock)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn card_lifecycle_scan_recall() {
    let (_d, app, clock) = setup();
    let (s, v) = call(&app, "GET", "/api/spaces", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["spaces"].as_array().unwrap().len(), 2);

    // A write without a space is refused.
    let (s, _) = call(&app, "POST", "/api/nodes", Some(json!({ "title": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = call(&app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": "Dice coefficient", "aliases": ["DSC"], "tags": ["metric"] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["node"]["id"].as_str().unwrap().to_string();

    let (s, v) = call(&app, "POST", "/api/scan", Some(json!({ "text": "Our DSC was 0.91.", "candidates": true }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["spans"][0]["node_id"], id);
    assert_eq!(v["spans"][0]["state"], "learning");
    assert_eq!((v["spans"][0]["start"].as_u64(), v["spans"][0]["end"].as_u64()), (Some(4), Some(7)));

    let (s, v) = call(&app, "PUT", &format!("/api/nodes/{id}"), Some(json!({ "revision": 1, "summary": "重疊指標" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["node"]["revision"], 2);
    let (s, _) = call(&app, "PUT", &format!("/api/nodes/{id}"), Some(json!({ "revision": 1, "summary": "x" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    clock.advance_days(1);
    let (_, v) = call(&app, "GET", "/api/recall/due", None).await;
    assert_eq!(v["nodes"][0]["id"], id);
    let (s, v) = call(&app, "POST", &format!("/api/recall/{id}"), Some(json!({ "outcome": "remembered", "via": "test" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["progress"]["streak"], 1);
    clock.advance_days(3);
    let (_, v) = call(&app, "POST", &format!("/api/recall/{id}"), Some(json!({ "outcome": "remembered" }))).await;
    assert_eq!(v["progress"]["state"], "known");
    let (_, v) = call(&app, "GET", "/api/nodes?state=known", None).await;
    assert_eq!(v["nodes"].as_array().unwrap().len(), 1);

    let (_, v) = call(&app, "GET", "/api/timeline?days=30", None).await;
    assert!(!v["days"].as_array().unwrap().is_empty());
    let (_, v) = call(&app, "GET", "/api/graph?tags=true", None).await;
    assert_eq!(v["nodes"].as_array().unwrap().len(), 2);
    let (_, v) = call(&app, "GET", "/api/registry?space=me", None).await;
    assert!(v["kinds"].as_array().unwrap().iter().any(|k| k["id"] == "concept"));
    let (_, v) = call(&app, "POST", "/api/draft", Some(json!({ "term": "IoU", "context": "IoU is intersection over union." }))).await;
    assert_eq!(v["draft"]["title"], "IoU");

    // Share into the household, then trash the personal card.
    let (s, v) = call(&app, "POST", &format!("/api/nodes/{id}/share"), Some(json!({ "to_space": "home" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["node"]["id"], id);
    let (s, _) = call(&app, "DELETE", &format!("/api/nodes/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "GET", &format!("/api/nodes/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assets_roundtrip_and_cors() {
    let (_d, app, _) = setup();
    let req = Request::builder().method("POST").uri("/api/assets?space=me&ext=svg").body(Body::from("<svg xmlns=\"http://www.w3.org/2000/svg\"/>")).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let url = v["url"].as_str().unwrap().to_string();
    let res = app.clone().oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/svg+xml");

    // NeonDeck's WKWebView (file:// page) sends `Origin: null`.
    let req = Request::builder().method("OPTIONS").uri("/api/recall/due").header("origin", "null").header("access-control-request-method", "GET").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.headers().get("access-control-allow-origin").unwrap(), "null");
    let req = Request::builder().uri("/api/health").header("origin", "https://example.com").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(res.headers().get("access-control-allow-origin").is_none());
}

#[test]
fn workspace_template_opens() {
    let dir = tempfile::tempdir().unwrap();
    let tpl = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workspace-template");
    ym_store::copy_missing(&tpl, &dir.path().join("kb")).unwrap();
    let config = Config { spaces: vec![dir.path().join("kb")], member: "ellrin".into(), port: 0, web: dir.path().into(), intervals: Default::default() };
    let kb = kb_server::open(&config, KbOptions::default()).unwrap();
    let reg = kb.registry("personal").unwrap();
    assert!(reg.has_link_type("vocab-of") && reg.has_kind("equation"));
    assert!(reg.kinds.iter().find(|k| k.id == "paper").unwrap().fields.iter().any(|f| f.name == "doi"));
    assert_eq!(kb.tags("personal").unwrap().domains.len(), 3);
}
