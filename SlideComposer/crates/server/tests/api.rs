//! API tests on a synthetic deck (ym-ooxml's test fixture), offline.

#[path = "../../../../commons/crates/ym-ooxml/tests/common/mod.rs"]
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const STYLE: &str = r#"
[pack]
id = "test"
name = "Test"
slide_cx = 9144000
slide_cy = 5143500
[tags.title]
font = "Calibri"
size = 24.0
bold = true
[tags.body]
font = "Calibri"
size = 14.0
[palette.colors]
black = "000000"
red = "FF0000"
[emphasis.key]
color = "FF0000"
bold = true
[ghost]
picture_alpha = 0.2
line_alpha = 0.2
text_color = "A6A6A6"
"#;

fn setup() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("styles/test")).unwrap();
    std::fs::write(dir.path().join("styles/test/style.toml"), STYLE).unwrap();
    let ws = sc_core::Workspace::new(dir.path(), None);
    let web = sc_server::repo_dir().join("web");
    let app = sc_server::router(sc_server::state(ws, web));
    (dir, app)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>, raw: Option<Vec<u8>>) -> (StatusCode, Value, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = if let Some(b) = body {
        req = req.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&b).unwrap())
    } else if let Some(r) = raw {
        Body::from(r)
    } else {
        Body::empty()
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (st, v, bytes)
}

#[tokio::test]
async fn import_edit_undo_reveal_export() {
    let (dir, app) = setup();
    let (st, v, _) = call(&app, "POST", "/api/decks/import?name=demo&style=test", None, Some(common::synthetic_deck())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "demo");
    let (_, d, _) = call(&app, "GET", "/api/decks/demo", None, None).await;
    assert_eq!(d["deck"]["slides"].as_array().unwrap().len(), 2);
    let s2 = d["deck"]["slides"][1]["uid"].as_str().unwrap().to_string();

    // unedited export keeps every part
    let (_, _, bytes) = call(&app, "GET", "/api/decks/demo/pptx", None, None).await;
    let back = ym_ooxml::Deck::from_bytes(&bytes).unwrap();
    let orig = ym_ooxml::Deck::from_bytes(&common::synthetic_deck()).unwrap();
    for p in &orig.pkg.parts {
        assert!(back.pkg.get(&p.name) == Some(p.data.as_slice()), "{}", p.name);
    }

    // rule: unknown tag rejected (422), nothing applied
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/ops", Some(json!({"ops": [{"op": "set_obj", "slide": s2, "id": 3, "path": "/text/paras/0/runs/0/tag", "value": "giant"}], "label": "x"})), None).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    // move the picture, then undo / redo
    let pic = d["deck"]["slides"][1]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 5).unwrap().clone();
    let mut f = pic["frame"].clone();
    f["x"] = json!(f["x"].as_i64().unwrap() + 100000);
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/ops", Some(json!({"ops": [{"op": "set_obj", "slide": s2, "id": 5, "path": "/frame", "value": f}], "label": "移動"})), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"]["undo"], "移動");
    let (_, v, _) = call(&app, "POST", "/api/decks/demo/undo", Some(json!({})), None).await;
    assert_eq!(v["status"]["redo"], "移動");
    let (_, v, _) = call(&app, "POST", "/api/decks/demo/redo", Some(json!({})), None).await;
    let moved = v["slides"][0]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 5).unwrap()["frame"]["x"].clone();
    assert_eq!(moved, f["x"]);

    // progressive reveal: 2 steps -> 3 slides
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/reveal", Some(json!({"slide": s2, "spec": {"mode": "ghost", "steps": [[3], [5]]}})), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["order"].as_array().unwrap().len(), 3);

    // picture asset is served from the source package
    let (st, _, b) = call(&app, "GET", "/api/decks/demo/asset?id=pkg:ppt/media/image2.png", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b, b"\x89PNG-figure");

    // export writes a new file, never overwriting
    let (_, a, _) = call(&app, "POST", "/api/decks/demo/export", Some(json!({})), None).await;
    let (_, b2, _) = call(&app, "POST", "/api/decks/demo/export", Some(json!({})), None).await;
    assert_ne!(a["path"], b2["path"]);
    let out = ym_ooxml::Deck::open(a["path"].as_str().unwrap()).unwrap();
    assert_eq!(out.slides.len(), 3);

    // deck.json on disk is the source of truth: a fresh server sees the edits
    let ws = sc_core::Workspace::new(dir.path(), None);
    let sess = ws.open("demo").unwrap();
    assert_eq!(sess.deck().slides.len(), 3);
}

#[tokio::test]
async fn new_deck_needs_a_template_and_names_are_checked() {
    let (_dir, app) = setup();
    let (st, _, _) = call(&app, "POST", "/api/decks", Some(json!({"name": "../evil", "style": "test"})), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v, _) = call(&app, "GET", "/api/info", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["styles"][0], "test");
}
