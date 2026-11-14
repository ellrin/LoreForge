//! 修正知識卡 (fix.rs): a draft with a mock Claude (web search asked for), the save through a mock
//! KnowledgeBase (`PUT /api/nodes/{id}` with the revision), a conflict, and nothing at all while
//! KnowledgeBase is closed.

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use le_server::kbctx::Pin;
use le_server::llm::Llm;
use le_server::{AppState, Shared};
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use serde_json::{Value, json};
use tower::ServiceExt;

const DICE: &str = "nAAAAAAAAAAAAAAAAAAAAAAAAA1";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.path().is_dir() { copy_dir(&e.path(), &to.join(e.file_name())) } else { std::fs::copy(e.path(), to.join(e.file_name())).unwrap(); }
    }
}

struct App {
    _tmp: tempfile::TempDir,
    kb: std::path::PathBuf,
    shared: Shared,
    mock: Arc<MockProvider>,
}

fn app(replies: &[&str], kb_url: Option<String>) -> App {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
    std::fs::create_dir_all(root.join("private")).unwrap();
    let kb = root.join("kb");
    copy_dir(&manifest.join("tests/fixtures/kb"), &kb);
    let mock = Arc::new(MockProvider::new(replies.iter().map(|s| s.to_string()).collect()).named("mock:claude"));
    let factory = MockFactory::default().with("claude:haiku", mock.clone());
    let shared = AppState::new(lore_home::Home::at(&root), root.join("web"), Llm::with_factory(factory), None);
    shared.kb.pin(Pin { spaces: vec![kb.clone()], member: "ellrin".into(), url: kb_url });
    App { _tmp: tmp, kb, shared, mock }
}

async fn post(a: &App, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
    let res = le_server::router_isolated(a.shared.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// A mock KnowledgeBase: `GET /api/nodes/{id}` = the card; `PUT` recorded, answered with `put_status`.
async fn kb(put_status: StatusCode) -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let s2 = seen.clone();
    let card = json!({ "space": "personal", "node": { "id": DICE, "revision": 3, "title": "Dice", "aliases": ["F1"], "summary": "重疊指標", "body": "$2|A\\cap B|/(|A|\\cup|B|)$", "sources": [{ "type": "paper", "ref": "x" }] } });
    let app = axum::Router::new().route(
        "/api/nodes/{id}",
        axum::routing::get(move || {
            let c = card.clone();
            async move { axum::Json(c) }
        })
        .put(move |axum::Json(b): axum::Json<Value>| {
            let seen = s2.clone();
            async move {
                seen.lock().unwrap().push(b.clone());
                (put_status, axum::Json(if put_status.is_success() { json!({ "node": { "id": DICE, "revision": 4, "title": b["title"] } }) } else { json!({ "error": "conflict" }) }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, seen)
}

fn talk() -> Value {
    json!([
        { "role": "user", "text": "Dice 是什麼？" },
        { "role": "assistant", "text": "你的卡〈Dice〉說…" },
        { "role": "user", "text": "這張卡的分母寫錯了，應該是 |A|+|B|" },
    ])
}

#[tokio::test]
async fn a_draft_checks_the_card_on_the_web_and_shows_old_and_new() {
    let (url, _) = kb(StatusCode::OK).await;
    let reply = json!({ "title": "Dice", "summary": "", "body": "$2|A\\cap B|/(|A|+|B|)$", "changes": [{ "what": "分母改成 |A|+|B|", "why": "定義", "url": "https://en.wikipedia.org/wiki/Dice-S%C3%B8rensen_coefficient" }], "sources": [{ "url": "https://en.wikipedia.org/wiki/Dice-S%C3%B8rensen_coefficient", "title": "Wikipedia" }] }).to_string();
    let a = app(&[&reply], Some(url));
    let (s, v) = post(&a, "/api/actions/fix/draft", json!({ "messages": talk(), "card": DICE })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["card"]["revision"], 3);
    assert_eq!(v["card"]["body"], "$2|A\\cap B|/(|A|\\cup|B|)$");
    assert_eq!(v["fix"]["body"], "$2|A\\cap B|/(|A|+|B|)$");
    assert_eq!(v["fix"]["summary"], "重疊指標", "an empty field keeps the card's own");
    assert_eq!(v["changes"].as_array().unwrap().len(), 1);
    let req = a.mock.seen.lock().unwrap()[0].clone();
    assert!(req.web_search, "Haiku searches the web");
    let u = req.messages[0].text();
    assert!(u.contains("分母寫錯了") && u.contains("\\cup") && !u.contains("你的卡〈Dice〉說"), "only his words and the card: {u}");
}

#[tokio::test]
async fn applying_goes_through_knowledgebase_with_the_revision() {
    let (url, seen) = kb(StatusCode::OK).await;
    let a = app(&[], Some(url));
    let (s, v) = post(&a, "/api/actions/fix/save", json!({ "id": DICE, "revision": 3, "title": " Dice ", "summary": "重疊指標", "body": "新", "sources": [{ "url": "https://a.org/dice", "title": "A" }] })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let put = seen.lock().unwrap()[0].clone();
    assert_eq!((put["revision"].as_u64(), put["title"].as_str(), put["body"].as_str()), (Some(3), Some("Dice"), Some("新")));
    assert_eq!(put["sources"].as_array().unwrap().len(), 2, "the card's own source kept, the web page added");
    assert_eq!(put["sources"][1]["ref"], "https://a.org/dice");
}

#[tokio::test]
async fn a_card_changed_meanwhile_is_not_overwritten() {
    let (url, _) = kb(StatusCode::CONFLICT).await;
    let a = app(&[], Some(url));
    let (s, v) = post(&a, "/api/actions/fix/save", json!({ "id": DICE, "revision": 2, "title": "Dice", "body": "新" })).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("改過"), "{v}");
}

#[tokio::test]
async fn nothing_happens_while_knowledgebase_is_closed() {
    let a = app(&["{}"], None);
    let before = std::fs::read_dir(a.kb.join("nodes")).map(|d| d.count()).unwrap_or(0);
    let (s, v) = post(&a, "/api/actions/fix/draft", json!({ "messages": talk(), "card": DICE })).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert!(a.mock.seen.lock().unwrap().is_empty(), "no model call");
    let (s, _) = post(&a, "/api/actions/fix/save", json!({ "id": DICE, "revision": 3, "title": "Dice", "body": "新" })).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(std::fs::read_dir(a.kb.join("nodes")).map(|d| d.count()).unwrap_or(0), before, "no file written");
}

#[tokio::test]
async fn which_card_and_what_is_wrong_are_required() {
    let a = app(&[], None);
    assert_eq!(post(&a, "/api/actions/fix/draft", json!({ "messages": talk(), "card": "../x" })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(post(&a, "/api/actions/fix/draft", json!({ "messages": [{ "role": "assistant", "text": "嗨" }], "card": DICE })).await.0, StatusCode::BAD_REQUEST);
}
