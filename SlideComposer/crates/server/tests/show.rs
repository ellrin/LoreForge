//! The show's 選頁, offline: the browser logic passes under node; the permissions SlideComposer
//! declares (the voice's, never the camera).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let ws = sc_core::Workspace::new(dir.path(), None);
    let home = lore_home::Home::at(dir.path().join("lf"));
    let app = sc_server::router_isolated(sc_server::state_at(ws, sc_server::repo_dir().join("web"), home));
    (dir, app)
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let res = app.clone().oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
    let st = res.status();
    (st, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

#[test]
fn picker_logic_under_node() {
    let script = sc_server::repo_dir().join("web/tests/show.test.js");
    match std::process::Command::new("node").arg(&script).output() {
        Ok(o) => assert!(o.status.success(), "node show.test.js:\n{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
        Err(_) => eprintln!("node not installed: show.test.js skipped"),
    }
}

#[tokio::test]
async fn permissions_declared_for_voice_only() {
    let (_d, app) = app();
    let (st, body) = get(&app, "/api/permissions").await;
    assert_eq!(st, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    let rows = v["permissions"].as_array().unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["microphone", "speech", "personal_voice"]);
    assert!(rows.iter().all(|r| r["status"] == "unsupported"), "no helpers in tests: nothing to ask");
}
