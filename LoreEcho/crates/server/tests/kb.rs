//! KnowledgeBase in the chat (kbctx): a synthetic space (`tests/fixtures/kb`) copied into a temp
//! dir, mock engines, a mock KnowledgeBase server on 127.0.0.1 for the review outcomes.

use std::path::{Path, PathBuf};
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

const FOURIER: &str = "nAAAAAAAAAAAAAAAAAAAAAAAAA1";
const NYQUIST: &str = "nAAAAAAAAAAAAAAAAAAAAAAAAA3";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.path().is_dir() { copy_dir(&e.path(), &to.join(e.file_name())) } else { std::fs::copy(e.path(), to.join(e.file_name())).unwrap(); }
    }
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() { stack.push(e.path()) } else { out.push((e.path(), std::fs::read(e.path()).unwrap())) }
        }
    }
    out.sort();
    out
}

struct App {
    _tmp: tempfile::TempDir,
    kb: PathBuf,
    shared: Shared,
    mock: Arc<MockProvider>,
}

impl App {
    fn new(replies: &[&str], url: Option<String>) -> App {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        copy_dir(&manifest.join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
        std::fs::create_dir_all(root.join("private")).unwrap();
        let kb = root.join("kb");
        copy_dir(&manifest.join("tests/fixtures/kb"), &kb);
        let mock = Arc::new(MockProvider::new(replies.iter().map(|s| s.to_string()).collect()).named("mock:local"));
        let shared = AppState::new(lore_home::Home::at(&root), root.join("web"), Llm::with_factory(MockFactory::default().with("local:current", mock.clone())), None);
        shared.kb.pin(Pin { spaces: vec![kb.clone()], member: "ellrin".into(), url });
        App { _tmp: tmp, kb, shared, mock }
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Vec<u8>) {
        let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        let res = le_server::router_isolated(self.shared.clone()).oneshot(req).await.unwrap();
        (res.status(), res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn chat(&self, body: Value) -> Vec<(String, Value)> {
        let (s, b) = self.post("/api/chat", body).await;
        assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        sse(&b)
    }

    fn system(&self, i: usize) -> String {
        self.mock.seen.lock().unwrap()[i].system.clone()
    }
}

fn sse(body: &[u8]) -> Vec<(String, Value)> {
    String::from_utf8_lossy(body)
        .split("\n\n")
        .filter_map(|block| {
            let (mut name, mut data) = (String::new(), String::new());
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event:") { name = v.trim().into() } else if let Some(v) = line.strip_prefix("data:") { data.push_str(v.strip_prefix(' ').unwrap_or(v)) }
            }
            (!name.is_empty()).then(|| (name, serde_json::from_str(&data).unwrap_or(Value::Null)))
        })
        .collect()
}

fn cards_of(events: &[(String, Value)]) -> Value {
    events.iter().find(|(n, _)| n == "cards").map(|(_, d)| d.clone()).expect("a cards event")
}

fn titles(cards: &Value) -> Vec<String> {
    cards["cards"].as_array().unwrap().iter().map(|c| c["title"].as_str().unwrap().to_string()).collect()
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "text": text })
}

#[tokio::test]
async fn cited_cards_come_before_start_and_go_into_the_prompt() {
    let app = App::new(&["你的卡〈傅立葉轉換〉說它把訊號拆成頻率。"], None);
    let before = snapshot(&app.kb);
    let ev = app.chat(json!({ "messages": [user("傅立葉轉換跟取樣有什麼關係？")] })).await;
    let names: Vec<&str> = ev.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(&names[..3], ["search", "cards", "start"], "{names:?}");
    let c = cards_of(&ev);
    assert_eq!(c["intent"], "");
    let t = titles(&c);
    assert_eq!(t[0], "傅立葉轉換");
    assert_eq!(c["cards"][0]["state"], "known");
    assert_eq!(c["cards"][0]["source"], "Signals, a primer");
    let sys = app.system(0);
    assert!(sys.contains("他自己的筆記") && sys.contains("〈傅立葉轉換〉（concept，已懂）") && sys.contains("〔來源：Signals, a primer〕"), "{sys}");
    assert!(sys.contains("你的卡"), "say when a card is used");
    assert_eq!(snapshot(&app.kb), before, "the space is only read");
}

#[tokio::test]
async fn a_draw_is_chosen_in_code_due_first() {
    let app = App::new(&["抽到〈取樣定理〉和〈熵〉。"], None);
    let ev = app.chat(json!({ "messages": [user("從知識庫抽兩個出來")] })).await;
    let c = cards_of(&ev);
    assert_eq!(c["intent"], "draw");
    let t = titles(&c);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0], "取樣定理", "the due card comes first");
    assert_eq!(c["cards"][0]["due"], true);
    let sys = app.system(0);
    assert!(sys.contains("抽 2 張") && sys.contains("不要換") && sys.contains(&format!("〈{}〉", t[1])), "{sys}");

    let ev = app.chat(json!({ "messages": [user("which cards about frequency do I have")] })).await;
    let c = cards_of(&ev);
    assert_eq!(c["intent"], "list");
}

#[tokio::test]
async fn quiz_then_answer_then_the_author_grades_himself() {
    let app = App::new(&["取樣頻率至少要多少才夠？", "對，要大於最高頻率的兩倍。"], None);
    let ev = app.chat(json!({ "messages": [user("考我")] })).await;
    let c = cards_of(&ev);
    assert_eq!((c["intent"].as_str(), c["quiz"].as_str()), (Some("quiz"), Some(NYQUIST)), "the due card");
    assert!(app.system(0).contains("不要說出答案"));
    let ev = app
        .chat(json!({ "kb": { "quiz": NYQUIST }, "messages": [user("考我"), { "role": "assistant", "text": "取樣頻率至少要多少才夠？" }, user("最高頻率的兩倍以上")] }))
        .await;
    let c = cards_of(&ev);
    assert_eq!((c["intent"].as_str(), c["grade"].as_str()), (Some("answer"), Some(NYQUIST)));
    let sys = app.system(1);
    assert!(sys.contains("比較") && sys.contains("大於訊號最高頻率的兩倍"), "{sys}");
    // KnowledgeBase is not running: one line, nothing written
    let before = snapshot(&app.kb);
    let (s, b) = app.post("/api/kb/recall", json!({ "id": NYQUIST, "outcome": "remembered" })).await;
    assert_eq!(s, StatusCode::OK);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["error"], "KnowledgeBase 沒開，沒有記下");
    assert_eq!(snapshot(&app.kb), before);
    let (s, _) = app.post("/api/kb/recall", json!({ "id": "../x", "outcome": "remembered" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// A stand-in for kb-server's `POST /api/recall/{id}`: records what it was told.
async fn mock_kb() -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let s2 = seen.clone();
    let app = axum::Router::new().route(
        "/api/recall/{id}",
        axum::routing::post(move |axum::extract::Path(id): axum::extract::Path<String>, axum::Json(b): axum::Json<Value>| {
            let s2 = s2.clone();
            async move {
                s2.lock().unwrap().push((id, b));
                axum::Json(json!({ "progress": { "state": "learning", "streak": 1 }, "step": { "advanced": true, "became_known": false } }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, seen)
}

#[tokio::test]
async fn outcomes_go_through_knowledgebase_s_api_when_it_runs() {
    let (url, seen) = mock_kb().await;
    let app = App::new(&["記下了。"], Some(url));
    let (s, b) = app.post("/api/kb/recall", json!({ "id": NYQUIST, "outcome": "forgot" })).await;
    assert_eq!(s, StatusCode::OK);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(seen.lock().unwrap()[0], (NYQUIST.to_string(), json!({ "outcome": "forgot", "via": "loreecho" })));
    // 「記住這個」: the last reply's cards as remembered
    let ev = app.chat(json!({ "kb": { "cited": [FOURIER] }, "messages": [user("記住這個")] })).await;
    let c = cards_of(&ev);
    assert_eq!(c["intent"], "remember");
    assert_eq!(c["recorded"], json!([FOURIER]));
    assert_eq!(seen.lock().unwrap()[1].1["outcome"], "remembered");
    assert!(app.system(0).contains("記成「記得」"));
}

#[tokio::test]
async fn remember_without_knowledgebase_running_says_so() {
    let app = App::new(&["KnowledgeBase 沒開。"], None);
    let ev = app.chat(json!({ "kb": { "cited": [FOURIER] }, "messages": [user("記住這個")] })).await;
    let c = cards_of(&ev);
    assert_eq!(c["notice"], "KnowledgeBase 沒開，沒有記下");
    assert!(c.get("recorded").is_none());
}

#[tokio::test]
async fn no_knowledgebase_no_search() {
    let app = App::new(&["好。"], None);
    app.shared.kb.pin(Pin { spaces: vec![], member: "ellrin".into(), url: None });
    let ev = app.chat(json!({ "messages": [user("傅立葉轉換是什麼")] })).await;
    assert_eq!(ev[0].0, "start", "nothing to read: the stream starts as before");
    let ev = app.chat(json!({ "messages": [user("抽兩張卡")] })).await;
    assert_eq!(cards_of(&ev)["available"], false);
    assert!(app.system(1).contains("找不到 KnowledgeBase"));
}
