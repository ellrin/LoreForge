//! 語句潤飾 in a deck over HTTP (lore-polish's `/api/polish*`), offline: a blank synthetic deck,
//! a mock model, a temporary LoreForge folder. The equation and the line break the page sends
//! locked must come back; the log lands in the deck's folder; 👍 only in the temp home.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&b).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn polish_a_run_with_an_equation_over_http() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    lore_home::copy_missing(&sc_server::repo_dir().join("../private-example/SlideComposer"), &root.join("sc")).unwrap();
    let ws = sc_core::Workspace::new(root.join("sc"), None);
    let name = ws.new_deck("talk", "blank").unwrap();
    let dir = ws.decks.join(&name);
    std::fs::write(dir.join("notation.json"), r#"{"version":1,"entries":[{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"\\vw","meaning":"weights"}]}"#).unwrap();
    let home = lore_home::Home::at(root.join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let replies = [r#"{"rewrite":"Weights ⟦3⟧ fall as ⟦1⟧ shrinks.⟦2⟧Fast."}"#, r#"{"rewrite":"Weights fall fast."}"#];
    let model = Arc::new(MockProvider::new(replies.iter().map(|s| s.to_string()).collect()));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, MockFactory::default().with("claude:selected", model))).is_ok());
    let app = sc_server::router_isolated(shared.clone());

    // the page: equation runs as $…$, a line break inside the paragraph as \n (kept)
    let text = "When $\\lambda$ shrinks, the weights\n\\vw drop quickly.";
    let body = json!({ "doc": name, "text": text, "instruction": "shorter", "keep": ["\n"] });
    let (st, p) = call(&app, "POST", "/api/polish", Some(body.clone())).await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(p["ok"], json!(true), "{p}");
    assert_eq!(p["after"], json!("Weights \\vw fall as $\\lambda$ shrinks.\nFast."));
    // dropped: the equation, the line break and the registered macro are named
    let (_, p2) = call(&app, "POST", "/api/polish", Some(body)).await;
    assert_eq!(p2["ok"], json!(false));
    assert_eq!(p2["problems"].as_array().unwrap().len(), 3, "{p2}");
    // the log is the deck's, one line per proposal
    let log = std::fs::read_to_string(dir.join("polish-log.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert!(!dir.join("polish.json").exists(), "排版用字's report is another file");
    let id = p["id"].as_str().unwrap();
    assert_eq!(call(&app, "POST", "/api/polish/applied", Some(json!({ "doc": name, "id": id }))).await.1["applied"], json!(true));
    assert_eq!(call(&app, "POST", "/api/polish/like", Some(json!({ "doc": name, "id": id }))).await.1["liked"], json!(true));
    let liked = std::fs::read_to_string(root.join("home/private/shared/polish/liked.jsonl")).unwrap();
    assert!(liked.contains("\"app\":\"SlideComposer\""), "{liked}");
    let (_, h) = call(&app, "GET", &format!("/api/polish/history?doc={name}&text=Weights%20fall"), None).await;
    assert!(h["items"].as_array().unwrap().is_empty());
    // no voice in tests: the dialog hides the mic
    assert_eq!(call(&app, "GET", "/api/voice", None).await.1["available"], json!(false));
    assert_eq!(call(&app, "POST", "/api/polish", Some(json!({ "doc": "nope", "text": "x", "instruction": "y" }))).await.0, StatusCode::NOT_FOUND);
}
