//! API tests: a temporary LoreForge folder (the real private-example/LoreEcho copied in),
//! mock engines, no keychain, no `claude` CLI, no network.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use le_server::characters::BUILTIN;
use le_server::chat::BREVITY_RULE;
use le_server::llm::Llm;
use le_server::{AppState, DesktopEvent, DesktopHook, Shared};
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use serde_json::{Value, json};
use tower::ServiceExt;

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/LoreEcho")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// Every file under `dir` with its bytes (to show nothing was written).
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.push((e.path(), std::fs::read(e.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

struct App {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    shared: Shared,
    events: Arc<Mutex<Vec<DesktopEvent>>>,
}

impl App {
    fn new(llm: Llm) -> App {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        copy_dir(&example(), &root.join("private-example/LoreEcho"));
        std::fs::create_dir_all(root.join("private")).unwrap();
        let events = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let hook: DesktopHook = Arc::new(move |e| ev.lock().unwrap().push(e));
        let shared = AppState::new(lore_home::Home::at(&root), root.join("web"), llm, Some(hook));
        App { _tmp: tmp, root, shared, events }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("private/LoreEcho")
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Vec<u8>, String) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let res = le_server::router_isolated(self.shared.clone()).oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let ct = res.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        (status, bytes, ct)
    }

    async fn json(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (s, b, _) = self.call(method, uri, body).await;
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }
}

/// `(event, data)` pairs of a server-sent-events body.
fn sse(body: &[u8]) -> Vec<(String, Value)> {
    let text = String::from_utf8_lossy(body);
    text.split("\n\n")
        .filter_map(|block| {
            let mut name = String::new();
            let mut data = String::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    name = v.trim().into();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push_str(v.strip_prefix(' ').unwrap_or(v));
                }
            }
            (!name.is_empty()).then(|| (name, serde_json::from_str(&data).unwrap_or(Value::Null)))
        })
        .collect()
}

#[tokio::test]
async fn seeding_copies_the_example_and_never_writes_into_it() {
    let app = App::new(Llm::none());
    let before = snapshot(&app.root.join("private-example"));
    assert!(app.dir().join("characters/echo/character.toml").is_file());
    assert!(app.dir().join("characters/echo/avatar/idle.svg").is_file());
    // writes land in private/LoreEcho only
    app.json("PUT", "/api/window", Some(json!({ "x": 10.0, "y": 20.0, "expanded": true }))).await;
    app.json("PUT", "/api/settings", Some(json!({ "character": "echo", "shortcut": "Alt+E" }))).await;
    assert_eq!(snapshot(&app.root.join("private-example")), before);
    assert!(app.dir().join("state.toml").is_file() && app.dir().join("settings.toml").is_file());
    // a second start does not overwrite the private copy
    std::fs::write(app.dir().join("characters/echo/character.toml"), "name = \"改過\"\n").unwrap();
    let again = AppState::new(lore_home::Home::at(&app.root), app.root.join("web"), Llm::none(), None);
    assert_eq!(again.character(None).name, "改過");
}

#[tokio::test]
async fn health_theme_shell_and_engines() {
    let app = App::new(Llm::none());
    let (s, h) = app.json("GET", "/api/health", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["app"], "LoreEcho");
    assert_eq!(h["character"], "echo");
    assert_eq!(h["desktop"], true);
    let (s, _, ct) = app.call("GET", "/theme.css", None).await;
    assert!(s.is_success() && ct.starts_with("text/css"));
    for f in ["/icons.js", "/base.css", "/shell.js", "/theme.js", "/prefs.css"] {
        assert_eq!(app.call("GET", f, None).await.0, StatusCode::OK, "{f}");
    }
    let (s, t) = app.json("PUT", "/api/theme", Some(json!({ "theme": "giftbox" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t["theme"], "giftbox");
    assert_eq!(app.json("GET", "/api/theme", None).await.1["theme"], "giftbox");
    assert_eq!(app.json("PUT", "/api/theme", Some(json!({ "theme": "no-such-theme" }))).await.0, StatusCode::BAD_REQUEST);
    let (s, e) = app.json("GET", "/api/engines", None).await;
    assert_eq!(s, StatusCode::OK);
    let tasks = e["tasks"].as_array().unwrap();
    assert!(tasks.iter().any(|t| t["id"] == "le-chat" && t["class"] == "ask"), "{e}");
}

#[tokio::test]
async fn window_state_round_trip() {
    let app = App::new(Llm::none());
    assert_eq!(app.json("GET", "/api/window", None).await.1, json!({ "expanded": false }));
    let (s, w) = app.json("PUT", "/api/window", Some(json!({ "x": 1600.5, "y": 940.0 }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(w, json!({ "x": 1600.5, "y": 940.0, "expanded": false }));
    assert!(app.events.lock().unwrap().is_empty(), "moving tells the desktop nothing");
    let (_, w) = app.json("PUT", "/api/window", Some(json!({ "expanded": true }))).await;
    assert_eq!(w, json!({ "x": 1600.5, "y": 940.0, "expanded": true }));
    assert_eq!(*app.events.lock().unwrap(), vec![DesktopEvent::Expanded(true)]);
    // read back from the file by a fresh state
    let again = AppState::new(lore_home::Home::at(&app.root), app.root.join("web"), Llm::none(), None);
    assert_eq!(serde_json::to_value(again.window()).unwrap(), w);
    let file = std::fs::read_to_string(app.dir().join("state.toml")).unwrap();
    assert!(file.contains("[window]") && file.contains("expanded = true"), "{file}");
    // nonsense is refused and changes nothing
    assert_eq!(app.json("PUT", "/api/window", Some(json!({ "x": 1e12 }))).await.0, StatusCode::BAD_REQUEST);
    assert!(app.json("PUT", "/api/window", Some(json!({ "width": 3 }))).await.0.is_client_error());
    assert_eq!(app.json("GET", "/api/window", None).await.1, w);
}

#[tokio::test]
async fn settings_check_character_and_shortcut() {
    let app = App::new(Llm::none());
    assert_eq!(app.json("GET", "/api/settings", None).await.1, json!({ "shortcut": "Alt+Space" }));
    assert_eq!(app.json("PUT", "/api/settings", Some(json!({ "character": "nobody" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(app.json("PUT", "/api/settings", Some(json!({ "character": "../echo" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(app.json("PUT", "/api/settings", Some(json!({ "shortcut": "E" }))).await.0, StatusCode::BAD_REQUEST);
    let (s, st) = app.json("PUT", "/api/settings", Some(json!({ "shortcut": "CmdOrCtrl+Shift+E" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(st["shortcut"], "CmdOrCtrl+Shift+E");
    assert_eq!(*app.events.lock().unwrap(), vec![DesktopEvent::Shortcut("CmdOrCtrl+Shift+E".into())]);
}

#[tokio::test]
async fn characters_and_avatar_fallback() {
    let app = App::new(Llm::none());
    // a second character with only an idle picture, and a broken one
    let two = app.dir().join("characters/two");
    std::fs::create_dir_all(two.join("avatar")).unwrap();
    std::fs::write(two.join("character.toml"), "name = \"二號\"\npersona = \"你很安靜。\"\n").unwrap();
    std::fs::write(two.join("avatar/idle.png"), b"\x89PNG fake").unwrap();
    let broken = app.dir().join("characters/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("character.toml"), "name = [").unwrap();

    let (s, c) = app.json("GET", "/api/characters", None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = c["characters"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["echo", "two"]);
    assert_eq!(c["current"], "echo");
    let echo = &c["characters"][0];
    assert_eq!(echo["name"], "回聲");
    assert_eq!(echo["language"], "zh-TW");
    assert_eq!(echo["avatars"], json!(["idle", "listening", "thinking", "speaking"]));
    assert!(echo.get("persona").is_none(), "the persona stays on the server");
    assert_eq!(c["characters"][1]["avatars"], json!(["idle"]));

    let (s, b, ct) = app.call("GET", "/api/characters/echo/avatar/thinking", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ct, "image/svg+xml");
    assert_eq!(b, std::fs::read(app.dir().join("characters/echo/avatar/thinking.svg")).unwrap());
    // a missing state is idle's picture
    let (s, b, ct) = app.call("GET", "/api/characters/two/avatar/speaking", None).await;
    assert_eq!((s, ct.as_str()), (StatusCode::OK, "image/png"));
    assert_eq!(b, b"\x89PNG fake");
    for bad in ["/api/characters/two/avatar/dancing", "/api/characters/nobody/avatar/idle", "/api/characters/..%2Fecho/avatar/idle", "/api/characters/broken/avatar/idle"] {
        assert_eq!(app.call("GET", bad, None).await.0, StatusCode::NOT_FOUND, "{bad}");
    }

    // the choice is kept; a chosen character that disappears falls back to the first
    app.json("PUT", "/api/settings", Some(json!({ "character": "two" }))).await;
    assert_eq!(app.json("GET", "/api/characters", None).await.1["current"], "two");
    std::fs::remove_dir_all(&two).unwrap();
    assert_eq!(app.json("GET", "/api/characters", None).await.1["current"], "echo");

    // no characters at all: the built-in one
    std::fs::remove_dir_all(app.dir().join("characters")).unwrap();
    let c = app.json("GET", "/api/characters", None).await.1;
    assert_eq!(c["current"], BUILTIN);
    assert_eq!(c["characters"][0]["avatars"], json!([]));
}

#[tokio::test]
async fn chat_streams_through_the_router_with_persona_and_brevity_rule() {
    let reply = "我在。今天想從哪裡開始聊？可以先說說卡住的地方。";
    let mock = Arc::new(MockProvider::new(vec![reply.into()]).named("mock:local"));
    let app = App::new(Llm::with_factory(MockFactory::default().with("local:current", mock.clone())));
    let body = json!({ "character": "echo", "messages": [
        { "role": "user", "text": "嗨" },
        { "role": "assistant", "text": "嗨，我在。" },
        { "role": "user", "text": "幫我想想傅立葉轉換" },
    ]});
    let (s, b, ct) = app.call("POST", "/api/chat", Some(body)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(ct.starts_with("text/event-stream"), "{ct}");
    let events = sse(&b);
    assert_eq!(events.first().unwrap().0, "start");
    assert_eq!(events[0].1["engine"], "mock:local");
    let deltas: Vec<&str> = events.iter().filter(|(n, _)| n == "delta").map(|(_, d)| d["text"].as_str().unwrap()).collect();
    assert!(deltas.len() > 1, "the reply arrives in pieces: {deltas:?}");
    assert_eq!(deltas.concat(), reply);
    let (last, done) = events.last().unwrap();
    assert_eq!(last, "done");
    assert_eq!(done["text"], reply);

    let seen = mock.seen.lock().unwrap();
    let req = &seen[0];
    let persona = "你是使用者的學習夥伴";
    assert!(req.system.contains(persona), "{}", req.system);
    assert!(req.system.contains("回聲") && req.system.contains(BREVITY_RULE), "{}", req.system);
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[2].text(), "幫我想想傅立葉轉換");
    assert_eq!(req.max_tokens, 1200, "the task's max_tokens");
    // the server keeps nothing
    let files: Vec<String> = snapshot(&app.dir()).into_iter().map(|(p, _)| p.display().to_string()).collect();
    assert!(files.iter().all(|f| f.contains("/characters/") || f.ends_with("README.md")), "{files:?}");
}

#[tokio::test]
async fn chat_uses_the_chosen_character_and_reports_problems() {
    let mock = Arc::new(MockProvider::new(vec!["好。".into()]).named("mock:local"));
    let app = App::new(Llm::with_factory(MockFactory::default().with("local:current", mock.clone())));
    let two = app.dir().join("characters/two");
    std::fs::create_dir_all(&two).unwrap();
    std::fs::write(two.join("character.toml"), "name = \"二號\"\npersona = \"你是一位嚴格的數學老師。\"\n").unwrap();
    app.json("PUT", "/api/settings", Some(json!({ "character": "two" }))).await;
    let (s, _, _) = app.call("POST", "/api/chat", Some(json!({ "messages": [{ "role": "user", "text": "考我" }] }))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(mock.seen.lock().unwrap()[0].system.starts_with("你是一位嚴格的數學老師。"));

    // bad requests
    for body in [json!({ "messages": [] }), json!({ "messages": [{ "role": "user", "text": "a" }, { "role": "assistant", "text": "b" }] }), json!({ "messages": [{ "role": "robot", "text": "a" }] })] {
        let (s, e) = app.json("POST", "/api/chat", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
        assert!(e["error"].is_string());
    }
    // pinned to Claude, which the mocks do not have: 503 with one line
    let (s, e) = app.json("POST", "/api/chat", Some(json!({ "engine": "claude", "messages": [{ "role": "user", "text": "a" }] }))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!e["error"].as_str().unwrap().contains('\n'));
}

#[tokio::test]
async fn chat_without_any_engine_is_503() {
    let app = App::new(Llm::none());
    let (s, e) = app.json("POST", "/api/chat", Some(json!({ "messages": [{ "role": "user", "text": "嗨" }] }))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    let m = e["error"].as_str().unwrap();
    assert!(!m.is_empty() && !m.contains('\n'), "{m}");
}

#[tokio::test]
async fn a_failing_engine_ends_the_stream_with_an_error() {
    // no queued reply: the mock fails once the call runs
    let mock = Arc::new(MockProvider::new(vec![]).named("mock:local"));
    let app = App::new(Llm::with_factory(MockFactory::default().with("local:current", mock)));
    let (s, b, _) = app.call("POST", "/api/chat", Some(json!({ "messages": [{ "role": "user", "text": "嗨" }] }))).await;
    assert_eq!(s, StatusCode::OK);
    let events = sse(&b);
    assert_eq!(events.last().unwrap().0, "error", "{events:?}");
    assert!(events.iter().all(|(n, _)| n != "done"));
}
