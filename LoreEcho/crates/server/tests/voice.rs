//! Voice, theme song and 角色設定 through the API: a temporary LoreForge folder (the real
//! private-example/LoreEcho copied in), lore-voice's fake backend (no audio), mock engines.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use le_server::llm::Llm;
use le_server::{AppState, Shared};
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use lore_voice::fake::{Call, FakeBackend, FakeControl};
use serde_json::{Value, json};
use tower::ServiceExt;

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

struct App {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    shared: Shared,
    fake: FakeControl,
}

impl App {
    fn new(llm: Llm) -> App {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
        std::fs::create_dir_all(root.join("private")).unwrap();
        let (backend, fake) = FakeBackend::instant();
        let shared = AppState::with_voice(lore_home::Home::at(&root), root.join("web"), llm, None, lore_voice::Voice::new(Box::new(backend)));
        App { _tmp: tmp, root, shared, fake }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("private/LoreEcho")
    }

    async fn raw(&self, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let res = le_server::router_isolated(self.shared.clone()).oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn json(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let (s, _, b) = self.raw(req.body(body).unwrap()).await;
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn bytes(&self, method: &str, uri: &str, bytes: Vec<u8>) -> StatusCode {
        self.raw(Request::builder().method(method).uri(uri).body(Body::from(bytes)).unwrap()).await.0
    }

    fn spoken(&self) -> Vec<lore_voice::SpeakRequest> {
        self.fake.calls().into_iter().filter_map(|c| if let Call::Speak(_, r) = c { Some(r) } else { None }).collect()
    }
}

/// A tiny "mp3" (an ID3 header and padding): enough for the format check.
fn mp3() -> Vec<u8> {
    let mut v = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    v.resize(4096, 0x55);
    v
}

#[tokio::test]
async fn voice_status_voices_listen_and_speak() {
    let app = App::new(Llm::none());
    let (s, v) = app.json("GET", "/api/voice", None).await;
    assert_eq!((s, v["available"].as_bool(), v["engine"].as_str()), (StatusCode::OK, Some(true), Some("fake")));

    // the character's language (zh-TW) first, better quality first
    let (_, v) = app.json("GET", "/api/voice/voices", None).await;
    let ids: Vec<&str> = v["voices"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["fake.zh-TW.d", "fake.zh-TW.c", "com.apple.eloquence.zh-TW.Eddy", "fake.zh-CN.b", "fake.en-US.a"]);

    assert_eq!(app.json("POST", "/api/voice/listen", Some(json!({ "continuous": true }))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.json("POST", "/api/voice/listen/stop", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.fake.calls()[..2], [Call::Listen(lore_voice::ListenOptions { language: "zh-TW".into(), continuous: true }), Call::StopListening]);

    // 試聽 with the character's voice, or the one given
    let (_, d) = app.json("PUT", "/api/characters/echo", Some(json!({ "voice": "fake.zh-TW.c", "voice_rate": 1.5, "voice_pitch": 0.8 }))).await;
    assert_eq!(d["voice_pitch"], 0.8);
    let (s, v) = app.json("POST", "/api/voice/speak", Some(json!({ "text": "你好。" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["id"].as_u64().is_some());
    app.json("POST", "/api/voice/speak", Some(json!({ "text": "試聽", "voice": "fake.zh-TW.d", "rate": 0.8, "pitch": 1.3 }))).await;
    let said = app.spoken();
    assert_eq!((said[0].voice.as_str(), said[0].rate, said[0].pitch, said[0].language.as_str()), ("fake.zh-TW.c", 1.5, 0.8, "zh-TW"));
    assert_eq!((said[1].voice.as_str(), said[1].rate, said[1].pitch), ("fake.zh-TW.d", 0.8, 1.3));
    // the open window warms the character's voice
    assert_eq!(app.json("POST", "/api/voice/warm", Some(json!({}))).await.0, StatusCode::NO_CONTENT);
    assert!(app.fake.calls().contains(&Call::Warm("fake.zh-TW.c".into())));
    assert_eq!(app.json("POST", "/api/voice/speak", Some(json!({ "text": " ** " }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(app.json("POST", "/api/voice/stop", None).await.0, StatusCode::NO_CONTENT);
    assert!(app.fake.calls().contains(&Call::StopSpeaking));

    // System Settings is never opened from tests (not the real engine); unknown pages refused
    let (s, v) = app.json("POST", "/api/voice/open-settings", Some(json!({ "what": "microphone" }))).await;
    assert_eq!((s, v["opened"].as_bool()), (StatusCode::OK, Some(false)));
    assert!(v["url"].as_str().unwrap().contains("Privacy_Microphone"));
    assert_eq!(app.json("POST", "/api/voice/open-settings", Some(json!({ "what": "/bin/sh" }))).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn without_an_engine_the_routes_say_why() {
    let tmp = tempfile::tempdir().unwrap();
    let shared = AppState::new(lore_home::Home::at(tmp.path()), tmp.path().join("web"), Llm::none(), None);
    let res = le_server::router_isolated(shared.clone()).oneshot(Request::get("/api/voice").body(Body::empty()).unwrap()).await.unwrap();
    let v: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["available"], false);
    assert!(v["why"].as_str().is_some_and(|w| !w.is_empty()));
    let req = Request::post("/api/voice/listen").header("content-type", "application/json").body(Body::from("{}")).unwrap();
    assert_eq!(le_server::router_isolated(shared).oneshot(req).await.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn events_stream_the_voice_as_it_happens() {
    let app = App::new(Llm::none());
    let res = le_server::router_isolated(app.shared.clone()).oneshot(Request::get("/api/voice/events").body(Body::empty()).unwrap()).await.unwrap();
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = res.into_body();
    let mut text = String::new();
    let mut next = async |text: &mut String| {
        let f = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.expect("an event in time").unwrap().unwrap();
        text.push_str(&String::from_utf8_lossy(&f.into_data().unwrap()));
    };
    next(&mut text).await;
    assert!(text.starts_with("event: hello"), "{text}");
    app.fake.emit(lore_voice::Event::Partial { text: "你好".into() });
    while !text.contains("event: partial") {
        next(&mut text).await;
    }
    assert!(text.contains(r#""text":"你好""#), "{text}");
}

#[tokio::test]
async fn a_reply_is_read_aloud_sentence_by_sentence() {
    let reply = "我在。今天想從哪裡開始聊？可以先說說卡住的地方";
    let mock = Arc::new(MockProvider::new(vec![reply.into(), reply.into()]).named("mock:local"));
    let app = App::new(Llm::with_factory(MockFactory::default().with("local:current", mock)));
    let body = json!({ "messages": [{ "role": "user", "text": "嗨" }], "speak": true });
    let (s, _) = app.json("POST", "/api/chat", Some(body)).await;
    assert_eq!(s, StatusCode::OK);
    let said: Vec<String> = app.spoken().into_iter().map(|r| r.text).collect();
    assert_eq!(said, vec!["我在。", "今天想從哪裡開始聊？", "可以先說說卡住的地方"]);
    // without speak: silent
    app.json("POST", "/api/chat", Some(json!({ "messages": [{ "role": "user", "text": "嗨" }] }))).await;
    assert_eq!(app.spoken().len(), 3);
}

#[tokio::test]
async fn the_reply_stream_says_where_each_spoken_sentence_is() {
    let reply = "好的，**重點**是這個。\n---\n再來一句";
    let mock = Arc::new(MockProvider::new(vec![reply.into()]).named("mock:local"));
    let app = App::new(Llm::with_factory(MockFactory::default().with("local:current", mock)));
    let body = json!({ "messages": [{ "role": "user", "text": "嗨" }], "speak": true });
    let req = Request::post("/api/chat").header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
    let (_, _, b) = app.raw(req).await;
    let text = String::from_utf8(b).unwrap();
    let says: Vec<Value> = text.split("\n\n").filter(|e| e.starts_with("event: say")).map(|e| serde_json::from_str(e.split("data: ").nth(1).unwrap()).unwrap()).collect();
    let spoken: Vec<String> = app.spoken().into_iter().map(|r| r.text).collect();
    assert_eq!(spoken, vec!["好的，重點是這個。", "再來一句"]);
    // spans in UTF-16 units of the reply: 0..12, the unspoken "---" line skipped, then the rest
    let n = |s: &str| s.encode_utf16().count() as u64;
    assert_eq!(says.len(), 2, "{text}");
    assert_eq!((says[0]["start"].as_u64(), says[0]["end"].as_u64(), says[0]["spoken"].as_u64()), (Some(0), Some(n("好的，**重點**是這個。\n")), Some(n("好的，重點是這個。"))));
    assert_eq!((says[1]["start"].as_u64(), says[1]["end"].as_u64()), (Some(n("好的，**重點**是這個。\n---\n")), Some(n(reply))));
    assert!(says[0]["id"].as_u64().unwrap() < says[1]["id"].as_u64().unwrap());
}

#[tokio::test]
async fn personal_voice_states() {
    use lore_voice::PersonalVoice::*;
    let app = App::new(Llm::none());
    // the fake's default: not on this Mac
    assert_eq!(app.json("GET", "/api/voice/personal", None).await.1["status"], "unsupported");
    // never asked: GET asks nothing; 使用個人聲音 asks, the author allows, none made yet
    app.fake.personal(NotDetermined, Authorized, false);
    assert_eq!(app.json("GET", "/api/voice/personal", None).await.1["status"], "not_determined");
    let (s, v) = app.json("POST", "/api/voice/personal", None).await;
    assert_eq!((s, v), (StatusCode::OK, json!({ "status": "authorized", "voices": 0 })));
    // one made: listed first in the picker
    app.fake.personal(Authorized, Authorized, true);
    assert_eq!(app.json("GET", "/api/voice/personal", None).await.1, json!({ "status": "authorized", "voices": 1 }));
    let (_, v) = app.json("GET", "/api/voice/voices", None).await;
    assert_eq!((v["voices"][0]["id"].as_str(), v["voices"][0]["personal"].as_bool()), (Some("fake.personal.me"), Some(true)));
    // refused: asking again changes nothing (the system never asks twice)
    app.fake.personal(Denied, Authorized, true);
    assert_eq!(app.json("POST", "/api/voice/personal", None).await.1["status"], "denied");
    let (_, v) = app.json("GET", "/api/voice/voices", None).await;
    assert!(v["voices"].as_array().unwrap().iter().all(|x| x["personal"] == false));
    let (_, v) = app.json("POST", "/api/voice/open-settings", Some(json!({ "what": "personal-voice" }))).await;
    assert!(v["url"].as_str().unwrap().contains("PersonalVoice"));
}

#[tokio::test]
async fn a_character_with_an_old_custom_voice_speaks_with_its_system_voice() {
    let app = App::new(Llm::none());
    let v = app.dir().join("characters/echo/voice");
    std::fs::create_dir_all(&v).unwrap();
    std::fs::write(v.join("voice.toml"), "mode = \"design\"\n[chosen]\nkind = \"design\"\ntext = \"你好\"\n").unwrap();
    std::fs::write(v.join("chosen.wav"), lore_voice::fake::wav_bytes(&[0; 2400], 24000)).unwrap();
    app.json("PUT", "/api/characters/echo", Some(json!({ "voice": "fake.zh-TW.d" }))).await;
    let (s, _) = app.json("POST", "/api/voice/speak", Some(json!({ "text": "你好。" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(app.spoken()[0].voice, "fake.zh-TW.d");
    // and the folder is not exported any more
    let (_, _, zip) = app.raw(Request::get("/api/characters/echo/export").body(Body::empty()).unwrap()).await;
    let z = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    assert!(z.file_names().all(|n| !n.contains("/voice/")));
}

#[tokio::test]
async fn theme_song_upload_play_and_remove() {
    let app = App::new(Llm::none());
    let (_, list) = app.json("GET", "/api/characters", None).await;
    assert!(list["characters"][0]["theme"].is_null(), "the example has no song");
    assert_eq!(app.json("GET", "/api/characters/echo/theme", None).await.0, StatusCode::NOT_FOUND);

    assert_eq!(app.bytes("PUT", "/api/characters/echo/theme", b"\x89PNG\r\n\x1a\n not a song".to_vec()).await, StatusCode::BAD_REQUEST);
    assert_eq!(app.bytes("PUT", "/api/characters/echo/theme", mp3()).await, StatusCode::NO_CONTENT);
    assert!(app.dir().join("characters/echo/theme.mp3").is_file());
    let (_, list) = app.json("GET", "/api/characters", None).await;
    assert_eq!(list["characters"][0]["theme"], json!({ "loop": true, "volume": 0.5 }));
    app.json("PUT", "/api/characters/echo", Some(json!({ "theme_loop": false, "theme_volume": 2.0 }))).await;
    let (_, d) = app.json("GET", "/api/characters/echo", None).await;
    assert_eq!(d["theme"], json!({ "loop": false, "volume": 1.0 }));

    // played with byte ranges
    let (s, h, b) = app.raw(Request::get("/api/characters/echo/theme").header("range", "bytes=0-9").body(Body::empty()).unwrap()).await;
    assert_eq!((s, h["content-type"].to_str().unwrap(), b.len()), (StatusCode::PARTIAL_CONTENT, "audio/mpeg", 10));
    // a new format replaces the old file
    let mut flac = b"fLaC".to_vec();
    flac.resize(64, 0);
    assert_eq!(app.bytes("PUT", "/api/characters/echo/theme", flac).await, StatusCode::NO_CONTENT);
    assert!(!app.dir().join("characters/echo/theme.mp3").exists() && app.dir().join("characters/echo/theme.flac").is_file());
    assert_eq!(app.json("DELETE", "/api/characters/echo/theme", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.json("DELETE", "/api/characters/echo/theme", None).await.0, StatusCode::NOT_FOUND);
    assert!(!app.root.join("private-example/LoreEcho/characters/echo/theme.flac").exists());
}

#[tokio::test]
async fn profile_and_song_survive_export_and_import() {
    let app = App::new(Llm::none());
    let profile = json!({
        "background": "在舊書店長大，\n喜歡替書找主人。",
        "speech_style": "慢慢說",
        "catchphrases": ["原來如此", "嗯哼"],
        "call_user": "ellrin",
        "relationship": "讀書夥伴",
        "likes": "下雨天",
        "dislikes": "",
        "examples": [{ "user": "你是誰？", "reply": "替書找主人的人。" }],
    });
    let (s, d) = app.json("PUT", "/api/characters/echo", Some(json!({ "profile": profile, "voice": "fake.zh-TW.d" }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["profile"]["catchphrases"], json!(["原來如此", "嗯哼"]));
    assert_eq!(d["background_limit"], le_server::chat::BACKGROUND_LIMIT);
    app.bytes("PUT", "/api/characters/echo/theme", mp3()).await;

    let (s, _, zip) = app.raw(Request::get("/api/characters/echo/export").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, b) = app.raw(Request::post("/api/characters/import").body(Body::from(zip)).unwrap()).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&b).unwrap()["id"].as_str().unwrap().to_string();
    let (_, imp) = app.json("GET", &format!("/api/characters/{id}"), None).await;
    assert_eq!(imp["profile"], d["profile"]);
    assert_eq!(imp["voice"], "fake.zh-TW.d");
    assert_eq!(std::fs::read(app.dir().join(format!("characters/{id}/theme.mp3"))).unwrap(), mp3());

    // a song that is not a song makes the import fail and leaves nothing
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    use std::io::Write;
    for (name, bytes) in [("bad/character.toml", &b"name = \"x\"\n"[..]), ("bad/theme.mp3", &b"hello, not audio"[..])] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    let bad = z.finish().unwrap().into_inner();
    assert_eq!(app.bytes("POST", "/api/characters/import", bad).await, StatusCode::BAD_REQUEST);
    assert!(!app.dir().join("characters/bad").exists());

    // the profile reaches the model: the examples as earlier turns, the background in the system prompt
    let mock = Arc::new(MockProvider::new(vec!["嗯哼。".into()]).named("mock:local"));
    let chat = App::new(Llm::with_factory(MockFactory::default().with("local:current", mock.clone())));
    chat.json("PUT", "/api/characters/echo", Some(json!({ "profile": profile }))).await;
    chat.json("POST", "/api/chat", Some(json!({ "messages": [{ "role": "user", "text": "為什麼你叫回聲？" }] }))).await;
    let seen = mock.seen.lock().unwrap();
    assert!(seen[0].system.contains("身分與背景：在舊書店長大， 喜歡替書找主人。"), "{}", seen[0].system);
    let texts: Vec<String> = seen[0].messages.iter().map(|m| m.text()).collect();
    assert_eq!(texts, vec!["你是誰？", "替書找主人的人。", "為什麼你叫回聲？"]);
}
