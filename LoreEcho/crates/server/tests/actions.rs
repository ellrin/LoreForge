//! 功能列表 (actions.rs): drafts with mock engines, saves online (mock PaperComposer and
//! KnowledgeBase servers on 127.0.0.1) and offline (files in a temp LoreForge folder).

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

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.path().is_dir() { copy_dir(&e.path(), &to.join(e.file_name())) } else { std::fs::copy(e.path(), to.join(e.file_name())).unwrap(); }
    }
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_dir() { stack.push(e.path()) } else { out.push(e.path()) }
        }
    }
    out.sort();
    out
}

struct App {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    kb: PathBuf,
    shared: Shared,
    mock: Arc<MockProvider>,
}

impl App {
    /// `replies` = the mock model's answers (none: no engine at all).
    fn new(replies: &[&str], kb_url: Option<String>, pc_url: Option<String>) -> App {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        copy_dir(&manifest.join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
        std::fs::create_dir_all(root.join("private")).unwrap();
        let kb = root.join("kb");
        copy_dir(&manifest.join("tests/fixtures/kb"), &kb);
        let mock = Arc::new(MockProvider::new(replies.iter().map(|s| s.to_string()).collect()).named("mock:local"));
        let factory = if replies.is_empty() { MockFactory::default() } else { MockFactory::default().with("local:current", mock.clone()) };
        let shared = AppState::new(lore_home::Home::at(&root), root.join("web"), Llm::with_factory(factory), None);
        shared.kb.pin(Pin { spaces: vec![kb.clone()], member: "ellrin".into(), url: kb_url });
        shared.pc.pin(pc_url);
        App { _tmp: tmp, root, kb, shared, mock }
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        let res = le_server::router_isolated(self.shared.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    fn notes(&self) -> PathBuf {
        self.root.join("private/PaperComposer/notes")
    }
}

fn talk() -> Value {
    json!([
        { "role": "user", "text": "傅立葉轉換和取樣有什麼關係？" },
        { "role": "assistant", "text": "取樣頻率要超過最高頻率的兩倍，這是 Nyquist 的結論。" },
    ])
}

/// A mock PaperComposer or KnowledgeBase: records the bodies, answers `reply` with `status`.
async fn peer(path: &'static str, status: StatusCode, reply: Value) -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let s2 = seen.clone();
    let app = axum::Router::new().route(
        path,
        axum::routing::get(|| async { axum::Json(json!({ "notes": [] })) }).post(move |axum::Json(b): axum::Json<Value>| {
            let seen = s2.clone();
            let reply = reply.clone();
            async move {
                seen.lock().unwrap().push(b);
                (status, axum::Json(reply))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, seen)
}

// ── 整理成筆記 ──

#[tokio::test]
async fn note_draft_runs_le_note_with_the_conversation_and_checks_the_latex() {
    let reply = json!({ "title": "取樣定理", "body": "\\section*{問題}\n如 Table 1 所示。", "tags": ["訊號"], "kb": [FOURIER, "nZZZZZZZZZZZZZZZZZZZZZZZZZZ"] }).to_string();
    let a = App::new(&[&reply], None, None);
    let (s, v) = a.post("/api/actions/note/draft", json!({ "messages": talk() })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["title"], "取樣定理");
    assert_eq!(v["kb"], json!([FOURIER]), "only cards that were offered");
    assert_eq!(v["kb_titles"], json!(["傅立葉轉換"]));
    assert!(v["findings"].as_array().unwrap().iter().any(|f| f["code"] == "hardcoded_reference"), "{v}");
    assert_eq!(v["blocking"], false);
    let req = a.mock.seen.lock().unwrap()[0].clone();
    assert!(req.json_schema.is_some());
    assert!(req.system.contains("\\section*{問題}") && req.system.contains(FOURIER), "the existing cards are offered");
    assert!(req.messages[0].text().contains("我：傅立葉轉換和取樣有什麼關係？"));
    assert!(!a.notes().exists(), "a draft writes nothing");
}

#[tokio::test]
async fn note_draft_without_a_model_offers_the_other_engine() {
    let a = App::new(&[], None, None);
    let (s, v) = a.post("/api/actions/note/draft", json!({ "messages": talk() })).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(v["error"].as_str().is_some_and(|e| !e.contains('\n')));
    let (s, _) = a.post("/api/actions/note/draft", json!({ "messages": [] })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn note_save_offline_writes_the_contract_file() {
    let a = App::new(&[], None, None);
    let (s, v) = a.post("/api/actions/note/save", json!({ "title": "取樣定理", "body": "\\section*{結論}\n取樣頻率 $f_s > 2 f_{\\max}$。", "kind": "note", "tags": ["訊號", "取樣"], "kb": [FOURIER] })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["via"], "file");
    let path = PathBuf::from(v["path"].as_str().unwrap());
    assert!(path.starts_with(a.notes()));
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "% ---");
    assert_eq!(lines[1], "% source: loreecho");
    assert!(lines[2].starts_with("% created: 20"));
    assert_eq!(&lines[3..8], &["% kind: note", &format!("% kb: {FOURIER}"), "% tags: 訊號, 取樣", "% title: 取樣定理", "% ---"]);
    assert!(text.contains("$f_s > 2 f_{\\max}$"));
    // the same title again: a new file
    let (_, v2) = a.post("/api/actions/note/save", json!({ "title": "取樣定理", "body": "x", "kind": "note" })).await;
    assert_eq!(v2["id"].as_str().unwrap(), format!("{}-2", v["id"].as_str().unwrap()));
}

#[tokio::test]
async fn note_save_offline_refuses_blocking_findings() {
    let a = App::new(&[], None, None);
    let (s, v) = a.post("/api/actions/note/save", json!({ "title": "t", "body": "見 \\ref{eq:none}。", "kind": "note" })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(v["findings"].as_array().unwrap().iter().any(|f| f["code"] == "ref_undefined"));
    assert!(files(&a.notes()).is_empty(), "nothing written");
}

#[tokio::test]
async fn idea_uses_no_model_and_is_kind_idea() {
    let a = App::new(&[], None, None); // no engine: an idea never asks one
    let (s, v) = a.post("/api/actions/note/save", json!({ "body": "用 50% 的資料試試 #2", "kind": "idea" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let text = std::fs::read_to_string(v["path"].as_str().unwrap()).unwrap();
    assert!(text.contains("% kind: idea\n") && text.contains("% title: 用 50% 的資料試試 #2\n"), "{text}");
    assert!(text.ends_with("用 50\\% 的資料試試 \\#2\n"), "{text}");
    assert!(a.mock.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn note_save_online_goes_through_papercomposer() {
    let (url, seen) = peer("/api/notes", StatusCode::CREATED, json!({ "id": "2026-11-12-x", "path": "/notes/2026-11-12-x.tex", "findings": [] })).await;
    let a = App::new(&[], None, Some(url));
    let (s, v) = a.post("/api/actions/note/save", json!({ "title": "x", "body": "內容", "kind": "note", "tags": ["a"], "kb": [FOURIER] })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["via"].as_str(), v["id"].as_str()), (Some("pc"), Some("2026-11-12-x")));
    let b = seen.lock().unwrap()[0].clone();
    assert_eq!(b["source"], "loreecho");
    assert_eq!(b["kind"], "note");
    assert_eq!(b["kb"], json!([FOURIER]));
    assert!(!a.notes().exists(), "PaperComposer wrote it, not LoreEcho");
}

#[tokio::test]
async fn note_save_online_shows_papercomposers_refusal_and_writes_nothing() {
    let (url, _) = peer("/api/notes", StatusCode::UNPROCESSABLE_ENTITY, json!({ "error": "x", "findings": [{ "code": "ref_undefined", "severity": "error", "message": "m" }] })).await;
    let a = App::new(&[], None, Some(url));
    let (s, v) = a.post("/api/actions/note/save", json!({ "title": "x", "body": "內容", "kind": "note" })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v["findings"][0]["code"], "ref_undefined");
    assert!(!a.notes().exists());
}

#[tokio::test]
async fn check_while_editing() {
    let a = App::new(&[], None, None);
    let (s, v) = a.post("/api/actions/note/check", json!({ "body": "見 \\ref{x}" })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["blocking"], true);
}

// ── 做成知識卡 ──

fn cards_reply() -> String {
    json!({ "cards": [
        { "title": "量化誤差", "summary": "類比值變成數位值時的誤差。", "body": "$f_s > 2f_{\\max}$", "tags": ["訊號"], "links": [FOURIER, "nZZZZZZZZZZZZZZZZZZZZZZZZZZ"] },
        { "title": "傅立葉轉換", "summary": "已經有的卡", "links": [] },
        { "title": "Fourier transform", "summary": "已經有的卡（別名）" },
        { "title": "  ", "summary": "沒有名字" },
        { "title": "混疊", "summary": "取樣不足時高頻變成低頻。" }
    ] })
    .to_string()
}

#[tokio::test]
async fn cards_draft_keeps_new_named_cards_with_known_links() {
    let a = App::new(&[&cards_reply()], None, None);
    let (s, v) = a.post("/api/actions/cards/draft", json!({ "messages": talk() })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let cards = v["cards"].as_array().unwrap();
    let titles: Vec<&str> = cards.iter().map(|c| c["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["量化誤差", "混疊"], "nameless and existing cards are dropped");
    assert_eq!(cards[0]["links"], json!([{ "to": FOURIER, "rel": "related", "title": "傅立葉轉換" }]));
    assert!(a.mock.seen.lock().unwrap()[0].system.contains(FOURIER));
    assert!(!a.kb.join("pending").exists(), "a draft writes nothing");
}

#[tokio::test]
async fn cards_save_offline_writes_pending_only() {
    let a = App::new(&[], None, None);
    let before: Vec<PathBuf> = files(&a.kb);
    let (s, v) = a.post("/api/actions/cards/save", json!({ "topic": "傅立葉轉換和取樣有什麼關係？", "cards": [
        { "title": "取樣定理", "summary": "s", "body": "b", "tags": ["訊號"], "links": [{ "to": FOURIER, "rel": "related", "title": "傅立葉轉換" }] },
        { "title": "" }
    ] })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["via"], "file");
    let ids = v["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    let id = ids[0].as_str().unwrap();
    assert!(id.starts_with('p'), "{id}");
    let card: Value = serde_json::from_slice(&std::fs::read(a.kb.join(format!("pending/{id}.json"))).unwrap()).unwrap();
    assert_eq!(card["title"], "取樣定理");
    assert_eq!(card["kind"], "concept");
    assert_eq!(card["source"], "LoreEcho：傅立葉轉換和取樣有什麼關係？");
    assert_eq!(card["links"], json!([{ "to": FOURIER, "rel": "related" }]));
    assert_eq!(card["from_conversation"], "傅立葉轉換和取樣有什麼關係？");
    assert!(card["created"].as_str().is_some());
    let listed = kb_core::pending::list(&lore_store::Space::open(&a.kb).unwrap());
    assert_eq!(listed.len(), 1, "KnowledgeBase reads it back");
    let after: Vec<PathBuf> = files(&a.kb).into_iter().filter(|p| !p.starts_with(a.kb.join("pending"))).collect();
    assert_eq!(after, before, "nothing outside pending/ changes");
    let (s, _) = a.post("/api/actions/cards/save", json!({ "cards": [{ "title": " " }] })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cards_save_online_goes_through_knowledgebase() {
    let (url, seen) = peer("/api/pending", StatusCode::OK, json!({ "ids": ["nPENDING"] })).await;
    let a = App::new(&[], Some(url), None);
    let (s, v) = a.post("/api/actions/cards/save", json!({ "cards": [{ "title": "混疊", "summary": "s", "links": [] }] })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["via"], "kb");
    assert_eq!(v["ids"], json!(["nPENDING"]));
    let b = seen.lock().unwrap()[0].clone();
    assert_eq!(b["cards"][0]["title"], "混疊");
    assert_eq!(b["cards"][0]["source"], "LoreEcho");
    assert!(!a.kb.join("pending").exists());
}

#[tokio::test]
async fn cards_save_online_error_is_not_a_silent_file() {
    let (url, _) = peer("/api/pending", StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "壞了" })).await;
    let a = App::new(&[], Some(url), None);
    let (s, v) = a.post("/api/actions/cards/save", json!({ "cards": [{ "title": "混疊" }] })).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert!(v["error"].as_str().unwrap().contains("壞了"));
    assert!(!a.kb.join("pending").exists());
}

#[test]
fn note_and_cards_tasks_are_tidy() {
    let t = Llm::task_table(None).unwrap();
    for id in [le_server::actions::TASK_NOTE, le_server::actions::TASK_CARDS] {
        let task = &t.task[id];
        assert_eq!(task.class, Some(lore_llm::TaskClass::Tidy), "{id}");
        assert_eq!(task.default, "local:current");
        assert_eq!(task.fallback.as_deref(), Some("claude:selected"));
    }
}
