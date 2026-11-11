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
    assert!(app.dir().join("characters/echo/full/idle.svg").is_file());
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
async fn window_size_is_kept_inside_the_limits_and_reaches_the_desktop() {
    let app = App::new(Llm::none());
    let (s, w) = app.json("PUT", "/api/window", Some(json!({ "w": 700.0, "h": 5000.0 }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((w["w"].clone(), w["h"].clone()), (json!(700.0), json!(1000.0)), "clamped to 420..1000");
    let (_, w) = app.json("PUT", "/api/window", Some(json!({ "w": 100.0 }))).await;
    assert_eq!((w["w"].clone(), w["h"].clone()), (json!(420.0), json!(1000.0)));
    assert_eq!(*app.events.lock().unwrap(), vec![DesktopEvent::Size(700.0, 1000.0), DesktopEvent::Size(420.0, 1000.0)]);
    assert_eq!(app.json("GET", "/api/window", None).await.1, w);
}

#[tokio::test]
async fn settings_check_character_and_shortcut() {
    let app = App::new(Llm::none());
    assert_eq!(app.json("GET", "/api/settings", None).await.1, json!({ "shortcut": "Alt+Space", "talk": "hold", "auto_send": true, "read_aloud": true }));
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
    let all = json!(["idle", "listening", "thinking", "speaking", "happy", "unsure", "surprised", "waiting", "searching", "noting"]);
    assert_eq!(echo["avatars"], all);
    assert!(echo.get("persona").is_none(), "the persona stays on the server");
    assert_eq!(echo["full"], all);
    assert_eq!(echo["figure"], json!({ "scale": 1.35, "offset_y": 0.0 }), "the example stands like a VN sprite");
    assert_eq!(c["characters"][1]["figure"], json!({ "scale": 1.0, "offset_y": 0.0 }), "defaults");
    assert_eq!(c["characters"][1]["avatars"], json!(["idle"]));
    assert_eq!(c["characters"][1]["full"], json!([]), "two has no full body");

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
    assert_eq!(c["characters"][0]["full"], json!([]));
    assert_eq!(app.call("GET", "/api/characters/default/full/idle", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn full_body_fallback_chain() {
    let app = App::new(Llm::none());
    let chars = app.dir().join("characters");
    let echo = chars.join("echo");
    // the example: one full body per state, all SVG
    for state in le_server::characters::STATES {
        let (s, b, ct) = app.call("GET", &format!("/api/characters/echo/full/{state}"), None).await;
        assert_eq!((s, ct.as_str()), (StatusCode::OK, "image/svg+xml"), "{state}");
        assert_eq!(b, std::fs::read(echo.join(format!("full/{state}.svg"))).unwrap(), "{state}");
    }
    // only a full idle (png) and portraits: other states use full/idle, not the portrait
    let three = chars.join("three");
    std::fs::create_dir_all(three.join("full")).unwrap();
    std::fs::create_dir_all(three.join("avatar")).unwrap();
    std::fs::write(three.join("character.toml"), "name = \"三號\"\n").unwrap();
    std::fs::write(three.join("full/idle.png"), b"\x89PNG full idle").unwrap();
    std::fs::write(three.join("avatar/speaking.png"), b"\x89PNG face speaking").unwrap();
    let (s, b, ct) = app.call("GET", "/api/characters/three/full/speaking", None).await;
    assert_eq!((s, ct.as_str(), b.as_slice()), (StatusCode::OK, "image/png", &b"\x89PNG full idle"[..]));
    // portraits only: the portrait of that state, then the portrait idle
    let two = chars.join("two");
    std::fs::create_dir_all(two.join("avatar")).unwrap();
    std::fs::write(two.join("character.toml"), "name = \"二號\"\n").unwrap();
    std::fs::write(two.join("avatar/idle.png"), b"\x89PNG face idle").unwrap();
    std::fs::write(two.join("avatar/thinking.webp"), b"RIFF face thinking").unwrap();
    let (s, b, ct) = app.call("GET", "/api/characters/two/full/thinking", None).await;
    assert_eq!((s, ct.as_str(), b.as_slice()), (StatusCode::OK, "image/webp", &b"RIFF face thinking"[..]));
    let (s, b, _) = app.call("GET", "/api/characters/two/full/speaking", None).await;
    assert_eq!((s, b.as_slice()), (StatusCode::OK, &b"\x89PNG face idle"[..]));
    let c = app.json("GET", "/api/characters", None).await.1;
    let by_id = |id: &str| c["characters"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap().clone();
    assert_eq!(by_id("three")["full"], json!(["idle"]));
    assert_eq!(by_id("three")["avatars"], json!(["speaking"]));
    assert_eq!(by_id("two")["full"], json!([]));
    // no pictures at all, bad state, bad id: 404
    let four = chars.join("four");
    std::fs::create_dir_all(&four).unwrap();
    std::fs::write(four.join("character.toml"), "name = \"四號\"\n").unwrap();
    for bad in ["/api/characters/four/full/idle", "/api/characters/echo/full/dancing", "/api/characters/nobody/full/idle", "/api/characters/..%2Fecho/full/idle", "/api/characters/echo/full/..%2Favatar%2Fidle"] {
        assert_eq!(app.call("GET", bad, None).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
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

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n fake";

async fn put_raw(app: &App, uri: &str, method: &str, bytes: Vec<u8>) -> (StatusCode, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri).body(Body::from(bytes)).unwrap();
    let res = le_server::router_isolated(app.shared.clone()).oneshot(req).await.unwrap();
    (res.status(), res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[tokio::test]
async fn the_new_states_follow_their_fallback_chain() {
    use le_server::characters::chain;
    assert_eq!(chain("searching"), ["searching", "thinking", "idle"]);
    assert_eq!(chain("noting"), ["noting", "speaking", "idle"]);
    assert_eq!(chain("waiting"), ["waiting", "thinking", "idle"]);
    assert_eq!(chain("happy"), ["happy", "idle"]);
    assert_eq!(chain("idle"), ["idle"]);

    let app = App::new(Llm::none());
    let x = app.dir().join("characters/x");
    std::fs::create_dir_all(x.join("avatar")).unwrap();
    std::fs::write(x.join("character.toml"), "name = \"x\"\n").unwrap();
    std::fs::write(x.join("avatar/idle.png"), b"\x89PNG idle").unwrap();
    std::fs::write(x.join("avatar/thinking.png"), b"\x89PNG thinking").unwrap();
    std::fs::write(x.join("avatar/speaking.png"), b"\x89PNG speaking").unwrap();
    for (state, want) in [("searching", &b"\x89PNG thinking"[..]), ("waiting", b"\x89PNG thinking"), ("noting", b"\x89PNG speaking"), ("surprised", b"\x89PNG idle")] {
        assert_eq!(app.call("GET", &format!("/api/characters/x/avatar/{state}"), None).await.1, want, "{state}");
    }
    // an own picture wins
    std::fs::write(x.join("avatar/searching.webp"), b"RIFF\0\0\0\0WEBPsearch").unwrap();
    assert_eq!(app.call("GET", "/api/characters/x/avatar/searching", None).await.1, b"RIFF\0\0\0\0WEBPsearch");
    // the page gets the chains
    let c = app.json("GET", "/api/characters", None).await.1;
    assert_eq!(c["chains"]["noting"], json!(["noting", "speaking", "idle"]));
    assert_eq!(c["states"].as_array().unwrap().len(), 10);
}

#[tokio::test]
async fn character_save_load_and_pictures() {
    let app = App::new(Llm::none());
    let (s, d) = app.json("PUT", "/api/characters/echo", Some(json!({ "name": "小回", "persona": "你很安靜。", "voice": "v1" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(d["name"], "小回");
    let d = app.json("GET", "/api/characters/echo", None).await.1;
    assert_eq!((d["persona"].as_str(), d["voice"].as_str()), (Some("你很安靜。"), Some("v1")));
    let c = app.json("GET", "/api/characters", None).await.1;
    assert_eq!(c["characters"][0]["figure"]["scale"], 1.35, "figure_scale survives a save");
    assert_eq!(app.json("PUT", "/api/characters/echo", Some(json!({ "bogus": 1 }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(app.json("PUT", "/api/characters/nobody", Some(json!({ "name": "x" }))).await.0, StatusCode::BAD_REQUEST);

    // a picture replaces the SVG of its slot, is served, then removed (the slot falls back)
    let echo = app.dir().join("characters/echo");
    assert!(echo.join("full/happy.svg").is_file());
    assert_eq!(put_raw(&app, "/api/characters/echo/full/happy", "PUT", PNG.to_vec()).await.0, StatusCode::NO_CONTENT);
    assert!(echo.join("full/happy.png").is_file() && !echo.join("full/happy.svg").exists());
    let (s, b, ct) = app.call("GET", "/api/characters/echo/full/happy", None).await;
    assert_eq!((s, ct.as_str(), b.as_slice()), (StatusCode::OK, "image/png", PNG));
    assert_eq!(app.call("DELETE", "/api/characters/echo/full/happy", None).await.0, StatusCode::NO_CONTENT);
    let (_, b, ct) = app.call("GET", "/api/characters/echo/full/happy", None).await;
    assert_eq!((ct.as_str(), b), ("image/svg+xml", std::fs::read(echo.join("full/idle.svg")).unwrap()));
    assert_eq!(app.call("DELETE", "/api/characters/echo/full/happy", None).await.0, StatusCode::NOT_FOUND);

    // refused: other formats, code in SVG, bad slots, empty
    for (uri, body) in [
        ("/api/characters/echo/avatar/happy", b"GIF89a".to_vec()),
        ("/api/characters/echo/avatar/happy", b"<svg xmlns='x'><script>1</script></svg>".to_vec()),
        ("/api/characters/echo/avatar/happy", b"<svg xmlns='x' onload='x()'/>".to_vec()),
        ("/api/characters/echo/avatar/dancing", PNG.to_vec()),
        ("/api/characters/echo/body/happy", PNG.to_vec()),
        ("/api/characters/echo/avatar/happy", vec![]),
    ] {
        assert_eq!(put_raw(&app, uri, "PUT", body).await.0, StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(put_raw(&app, "/api/characters/echo/avatar/happy", "PUT", b"<?xml version='1.0'?><svg viewBox='0 0 1 1'/>".to_vec()).await.0, StatusCode::NO_CONTENT);
    assert_eq!(put_raw(&app, "/api/characters/echo/avatar/unsure", "PUT", vec![0xFF, 0xD8, 0xFF, 0xE0, 1]).await.0, StatusCode::NO_CONTENT);
    assert!(echo.join("avatar/unsure.jpg").is_file() && !echo.join("avatar/unsure.svg").exists());
    assert_eq!(put_raw(&app, "/api/characters/echo/avatar/noting", "PUT", b"RIFF\0\0\0\0WEBPVP8 ".to_vec()).await.0, StatusCode::NO_CONTENT);
    assert!(echo.join("avatar/noting.webp").is_file());
}

#[tokio::test]
async fn new_character_copies_the_example_and_the_zip_round_trips() {
    let app = App::new(Llm::none());
    let chars = app.dir().join("characters");
    let (s, d) = app.json("POST", "/api/characters", Some(json!({ "name": "小明" }))).await;
    assert_eq!((s, d["name"].as_str()), (StatusCode::CREATED, Some("小明")));
    let id = d["id"].as_str().unwrap().to_string();
    assert_eq!(id, "char");
    assert!(chars.join(&id).join("full/noting.svg").is_file(), "a copy of the example, pictures included");
    assert_eq!(app.json("POST", "/api/characters", Some(json!({ "name": "Ada Lin" }))).await.1["id"], "ada-lin");
    assert_eq!(app.json("POST", "/api/characters", Some(json!({ "name": "  " }))).await.0, StatusCode::BAD_REQUEST);
    assert!(!app.root.join("private-example/LoreEcho/characters/char").exists(), "nothing written into the example");

    // customise, export, import: the import is a new character, nothing overwritten
    put_raw(&app, &format!("/api/characters/{id}/avatar/happy"), "PUT", PNG.to_vec()).await;
    app.json("PUT", &format!("/api/characters/{id}"), Some(json!({ "persona": "你愛笑。" }))).await;
    let (s, zip, ct) = app.call("GET", &format!("/api/characters/{id}/export"), None).await;
    assert_eq!((s, ct.as_str()), (StatusCode::OK, "application/zip"));
    let (s, b) = put_raw(&app, "/api/characters/import", "POST", zip.clone()).await;
    assert_eq!(s, StatusCode::CREATED);
    let imp: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(imp["id"], "char-2", "the folder name is kept, a taken one gets -2");
    assert_eq!((imp["name"].as_str(), imp["persona"].as_str()), (Some("小明"), Some("你愛笑。")));
    let (a, b2) = (chars.join(&id), chars.join("char-2"));
    for rel in ["character.toml", "avatar/happy.png", "avatar/idle.svg", "full/noting.svg", "full/idle.svg"] {
        assert_eq!(std::fs::read(a.join(rel)).unwrap(), std::fs::read(b2.join(rel)).unwrap(), "{rel}");
    }
    assert_eq!(put_raw(&app, "/api/characters/import", "POST", zip).await.0, StatusCode::CREATED);
    assert!(chars.join("char-3").is_dir());

    // not a zip / no character.toml / a picture that is not a picture
    assert_eq!(put_raw(&app, "/api/characters/import", "POST", b"nope".to_vec()).await.0, StatusCode::BAD_REQUEST);
    let bad = zip_of(&[("a/avatar/idle.png", b"hello")]);
    assert_eq!(put_raw(&app, "/api/characters/import", "POST", bad).await.0, StatusCode::BAD_REQUEST);
    let bad = zip_of(&[("a/character.toml", b"name = \"a\"\n"), ("a/avatar/idle.png", b"hello")]);
    assert_eq!(put_raw(&app, "/api/characters/import", "POST", bad).await.0, StatusCode::BAD_REQUEST);
    assert!(!chars.join("a").exists(), "a failed import leaves nothing behind");
    // zip paths cannot reach outside the characters folder
    let sneaky = zip_of(&[("../../evil/character.toml", b"name = \"e\"\n"), ("../outside/avatar/idle.png", PNG)]);
    let (s, b) = put_raw(&app, "/api/characters/import", "POST", sneaky).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "entries with .. are skipped: {}", String::from_utf8_lossy(&b));
    assert!(!app.root.join("private/evil").exists() && !app.root.join("private/outside").exists() && !chars.join("evil").exists());
}
