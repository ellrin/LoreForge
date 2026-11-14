//! 搜尋檔案 for another app: request → candidates from a mock LoreShelf (filtered) → pick → the
//! asking app (a real `lore_shell::findfile` router with a recording callback) → end.

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use le_server::llm::Llm;
use le_server::{AppState, Shared};
use axum::Json;
use axum::routing::get;
use lore_shell::findfile::{Finds, OnPick, Pick};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn serve(app: axum::Router) -> (String, u16) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{a}"), a.port())
}

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

#[tokio::test(flavor = "multi_thread")]
async fn find_request_candidates_pick_and_end() {
    let tmp = tempfile::tempdir().unwrap();
    let files = tmp.path().join("exp");
    std::fs::create_dir_all(&files).unwrap();
    for n in ["run-1.csv", "run-2.md", "slides.png"] {
        std::fs::write(files.join(n), "epoch,loss\n1,0.5\n").unwrap();
    }
    // a mock LoreShelf
    let dir = files.clone();
    let shelf = axum::Router::new()
        .route("/api/search", get(|| async {
            Json(json!({ "items": [
                { "item": { "id": "a", "name": "run-1.csv", "kind": "table", "times": { "modified": "2026-11-01T00:00:00Z" } } },
                { "item": { "id": "p", "name": "slides.png", "kind": "image", "times": {} } },
                { "item": { "id": "b", "name": "run-2.md", "kind": "text", "times": {} } },
            ] }))
        }))
        .route("/api/files/{id}", get(move |axum::extract::Path(id): axum::extract::Path<String>| {
            let dir = dir.clone();
            async move {
                let name = match id.as_str() { "a" => "run-1.csv", "b" => "run-2.md", _ => "slides.png" };
                Json(json!({ "id": id, "paths": [dir.join(name).to_string_lossy()] }))
            }
        }));
    let (shelf_url, _) = serve(shelf).await;

    // the asking app: the shared receive route with a recording callback
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let g2 = got.clone();
    let on_pick: OnPick = Arc::new(move |p: Pick| {
        let g = g2.clone();
        Box::pin(async move {
            g.lock().unwrap().push(format!("{} {}", p.purpose, p.path.file_name().unwrap().to_string_lossy()));
            Ok(format!("已加入{}", p.purpose))
        })
    });
    let finds = Finds::new("pc", on_pick).with_ask(Arc::new(|_: &lore_shell::findfile::FindRequest| Ok(())));
    let app: axum::Router = lore_shell::findfile::router(finds);
    let (app_url, app_port) = serve(app).await;

    // the app asks (its page → its server): the request id, then LoreEcho is told
    let req: Value = reqwest::Client::new().post(format!("{app_url}/api/lore/find")).json(&json!({ "purpose": "實驗紀錄", "exts": ["md", "csv"], "context": {} })).send().await.unwrap().json().await.unwrap();
    let id = req["request"].as_str().unwrap().to_string();

    let s = state(tmp.path());
    s.find.pin_shelf(&shelf_url);
    assert_eq!(call(&s, "GET", "/api/find/search?q=loss", Value::Null).await.0, StatusCode::GONE, "no find mode yet");
    let (c, _) = call(&s, "POST", "/api/find", json!({ "request": id, "app": "pc", "purpose": "實驗紀錄", "exts": ["md", "csv"], "port": app_port })).await;
    assert_eq!(c, StatusCode::OK);
    let (_, v) = call(&s, "GET", "/api/find", Value::Null).await;
    assert_eq!((v["find"]["purpose"].as_str(), v["seq"].as_u64()), (Some("實驗紀錄"), Some(1)));

    // candidates: the png is left out, the folder shown
    let (c, v) = call(&s, "GET", "/api/find/search?q=loss%20curve", Value::Null).await;
    assert_eq!(c, StatusCode::OK, "{v}");
    let names: Vec<&str> = v["items"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["run-1.csv", "run-2.md"]);
    assert!(v["items"][0]["folder"].as_str().unwrap().ends_with("exp"));

    // two picks, one refused extension
    let (c, v) = call(&s, "POST", "/api/find/pick", json!({ "id": "a" })).await;
    assert_eq!((c, v["name"].as_str(), v["message"].as_str()), (StatusCode::OK, Some("run-1.csv"), Some("已加入實驗紀錄")));
    assert_eq!(call(&s, "POST", "/api/find/pick", json!({ "id": "b" })).await.0, StatusCode::OK);
    assert_eq!(call(&s, "POST", "/api/find/pick", json!({ "id": "p" })).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(*got.lock().unwrap(), vec!["實驗紀錄 run-1.csv".to_string(), "實驗紀錄 run-2.md".into()]);
    let st: Value = reqwest::get(format!("{app_url}/api/lore/find/{id}")).await.unwrap().json().await.unwrap();
    assert_eq!(st["added"], json!(["run-1.csv", "run-2.md"]));

    // 好了: find mode ends, the app hears it
    assert_eq!(call(&s, "POST", "/api/find/end", Value::Null).await.0, StatusCode::OK);
    assert!(call(&s, "GET", "/api/find", Value::Null).await.1["find"].is_null());
    let st: Value = reqwest::get(format!("{app_url}/api/lore/find/{id}")).await.unwrap().json().await.unwrap();
    assert_eq!(st["open"], json!(false));
}
