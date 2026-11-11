//! Custom voices through the API: a temporary LoreForge folder (private-example/LoreEcho copied
//! in), lore-voice's fake backend and fake Qwen3-TTS (no audio, no Python, no network).

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
use lore_voice::fake::{Call, FakeBackend, FakeControl, FakeTts};
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
    tts: FakeTts,
}

impl App {
    fn new(installed: bool) -> App {
        let reply = "好啊。我們從定義開始";
        let mock = Arc::new(MockProvider::new(vec![reply.into(), reply.into()]).named("mock:local"));
        let llm = Llm::with_factory(MockFactory::default().with("local:current", mock));
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
        std::fs::create_dir_all(root.join("private")).unwrap();
        let (backend, fake) = FakeBackend::instant();
        fake.synth_seconds(10.0);
        let tts = FakeTts::new(installed);
        let voice = lore_voice::Voice::new(Box::new(lore_voice::qwen::Mixed::new(backend, Arc::new(tts.clone()), root.join("cache/say"))));
        let shared = AppState::with_voice(lore_home::Home::at(&root), root.join("web"), llm, None, voice);
        shared.tts.set(Arc::new(tts.clone()), root.join("cache/candidates"));
        App { _tmp: tmp, root, shared, fake, tts }
    }

    fn voice_dir(&self) -> PathBuf {
        self.root.join("private/LoreEcho/characters/echo/voice")
    }

    async fn raw(&self, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let res = le_server::router_isolated(self.shared.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        (status, res.into_body().collect().await.unwrap().to_bytes().to_vec())
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
        let (s, b) = self.raw(req.body(body).unwrap()).await;
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn job_done(&self) -> Value {
        for _ in 0..200 {
            let (_, v) = self.json("GET", "/api/tts", None).await;
            if v["job"]["running"] == false {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job did not end");
    }

    fn plays(&self) -> usize {
        self.fake.calls().iter().filter(|c| matches!(c, Call::Play(..))).count()
    }

    fn spoken(&self) -> Vec<lore_voice::SpeakRequest> {
        self.fake.calls().into_iter().filter_map(|c| if let Call::Speak(_, r) = c { Some(r) } else { None }).collect()
    }
}

fn wav(secs: f64) -> Vec<u8> {
    lore_voice::qwen::wav_bytes(&vec![0i16; (24000.0 * secs) as usize], 24000)
}

async fn wait_quiet(app: &App) {
    for _ in 0..300 {
        if !app.shared.voice.speaking() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("still speaking");
}

#[tokio::test]
async fn not_installed_shows_the_download_and_speaks_with_the_system_voice() {
    let app = App::new(false);
    let (_, v) = app.json("GET", "/api/tts", None).await;
    assert_eq!(v["status"]["ready"], false);
    assert!(v["status"]["missing_bytes"].as_u64().unwrap() > 0);
    // a character that already has a picked voice (e.g. imported) still talks
    std::fs::create_dir_all(app.voice_dir()).unwrap();
    std::fs::write(app.voice_dir().join("chosen.wav"), wav(2.0)).unwrap();
    std::fs::write(app.voice_dir().join("voice.toml"), "mode = \"design\"\n[chosen]\nkind = \"design\"\ntext = \"你好。\"\n").unwrap();
    let (s, _) = app.json("POST", "/api/voice/speak", Some(json!({ "text": "試聽一下。" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(app.spoken().len(), 1, "the system voice");
    assert!(app.spoken()[0].custom.is_none());
    assert_eq!(app.plays(), 0);
    // candidates need the download
    let (s, _) = app.json("POST", "/api/characters/echo/voice/candidates", Some(json!({ "kind": "design", "description": "低沉" }))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    // 下載
    let (s, _) = app.json("POST", "/api/tts/download", None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let v = app.job_done().await;
    assert_eq!(v["job"]["error"], "");
    assert_eq!(v["status"]["ready"], true);
    assert_eq!(app.tts.installs(), 1);
}

#[tokio::test]
async fn design_candidates_pick_then_replies_speak_in_that_voice() {
    let app = App::new(true);
    let (s, _) = app.json("POST", "/api/characters/echo/voice/candidates", Some(json!({ "kind": "design", "description": "三十多歲男性，嗓音低沉" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let v = app.job_done().await;
    assert_eq!(v["job"]["candidates"], json!([1, 2, 3, 4]));
    let designs = app.tts.designs();
    assert_eq!(designs.len(), 4);
    assert!(designs.iter().all(|d| d.description.contains("台灣口音")), "the Taiwan hint is added");
    let seeds: std::collections::BTreeSet<u64> = designs.iter().map(|d| d.seed).collect();
    assert_eq!(seeds.len(), 4, "four different takes");
    // each take can be heard
    let (s, b) = app.raw(Request::get("/api/characters/echo/voice/candidates/3").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert!(lore_voice::qwen::wav_seconds(&b).is_some());
    // pick 3
    let (s, v) = app.json("POST", "/api/characters/echo/voice/pick", Some(json!({ "n": 3 }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "design");
    assert_eq!(v["speaking_with"], "chosen.wav");
    assert_eq!(v["chosen"]["seed"], designs[2].seed);
    assert_eq!(v["chosen"]["description"], "三十多歲男性，嗓音低沉", "the author's words are kept as written");
    // fine-tune
    let (_, v) = app.json("PUT", "/api/characters/echo/voice", Some(json!({ "pitch": -2.5 }))).await;
    assert_eq!(v["pitch"], -2.5);
    // a reply read aloud: every sentence synthesised from chosen.wav and played with the pitch
    let (s, _) = app.json("POST", "/api/chat", Some(json!({ "messages": [{ "role": "user", "text": "嗨" }], "speak": true }))).await;
    assert_eq!(s, StatusCode::OK);
    wait_quiet(&app).await;
    let synths = app.tts.synths();
    assert_eq!(synths.iter().map(|j| j.text.as_str()).collect::<Vec<_>>(), ["好啊。", "我們從定義開始"]);
    assert!(synths.iter().all(|j| j.voice.audio.ends_with("chosen.wav") && j.voice.text.starts_with("你好，我是")));
    assert!(app.fake.calls().iter().any(|c| matches!(c, Call::Play(_, _, _, p) if *p == -2.5)));
    assert!(app.spoken().is_empty(), "no system voice");
    // a bad pick
    let (s, _) = app.json("POST", "/api/characters/echo/voice/pick", Some(json!({ "n": 9 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accent_candidates_clone_the_apple_taiwan_voice() {
    let app = App::new(true);
    let (s, _) = app.json("POST", "/api/characters/echo/voice/candidates", Some(json!({ "kind": "accent" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let v = app.job_done().await;
    assert_eq!(v["job"]["error"], "", "{v}");
    assert_eq!(v["job"]["candidates"], json!([1, 2, 3, 4]));
    // the best zh-TW system voice of the fake read the passage
    let read: Vec<String> = app.fake.calls().into_iter().filter_map(|c| if let Call::SynthFile(r) = c { Some(r.voice) } else { None }).collect();
    assert_eq!(read, ["fake.zh-TW.d"]);
    let synths = app.tts.synths();
    assert_eq!(synths.len(), 4);
    assert!(synths.iter().all(|j| j.voice.text == lore_voice::qwen::ACCENT_PASSAGE && j.voice.taiwan && j.seed.is_some()));
    let (_, v) = app.json("POST", "/api/characters/echo/voice/pick", Some(json!({ "n": 1 }))).await;
    assert_eq!(v["mode"], "accent");
    assert_eq!(v["chosen"]["kind"], "accent");
}

#[tokio::test]
async fn a_recording_needs_consent_and_stays_out_of_exports_by_default() {
    let app = App::new(true);
    let put = |q: &str, b: Vec<u8>| Request::put(format!("/api/characters/echo/voice/reference{q}")).body(Body::from(b)).unwrap();
    let (s, b) = app.raw(put("", wav(5.0))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&b).contains("有權使用"));
    assert!(!app.voice_dir().join("reference.wav").exists());
    let (s, _) = app.raw(put("?consent=1&text=%E4%B8%80%E4%BA%8C%E4%B8%89", wav(1.0))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "too short");
    let (s, b) = app.raw(put("?consent=1&text=%E4%B8%80%E4%BA%8C%E4%B8%89", wav(6.0))).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let (_, v) = app.json("GET", "/api/characters/echo/voice", None).await;
    assert_eq!((v["mode"].as_str(), v["speaking_with"].as_str(), v["clone"]["text"].as_str()), (Some("clone"), Some("reference.wav"), Some("一二三")));
    // 試聽 uses it
    app.json("POST", "/api/voice/speak", Some(json!({ "text": "試聽。" }))).await;
    wait_quiet(&app).await;
    assert!(app.tts.synths().last().unwrap().voice.audio.ends_with("reference.wav"));
    // export: left out unless asked
    let zip_names = |b: &[u8]| {
        let z = zip::ZipArchive::new(std::io::Cursor::new(b.to_vec())).unwrap();
        z.file_names().map(String::from).collect::<Vec<_>>()
    };
    let (_, b) = app.raw(Request::get("/api/characters/echo/export").body(Body::empty()).unwrap()).await;
    let names = zip_names(&b);
    assert!(names.contains(&"echo/voice/voice.toml".to_string()) && !names.iter().any(|n| n.ends_with("reference.wav")), "{names:?}");
    let (_, b) = app.raw(Request::get("/api/characters/echo/export?voice_ref=1").body(Body::empty()).unwrap()).await;
    assert!(zip_names(&b).contains(&"echo/voice/reference.wav".to_string()));
    // imported: the recording comes without consent
    let (s, d) = app.raw(Request::post("/api/characters/import").body(Body::from(b)).unwrap()).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&d).unwrap()["id"].as_str().unwrap().to_string();
    let (_, v) = app.json("GET", &format!("/api/characters/{id}/voice"), None).await;
    assert_eq!((v["has_reference"].as_bool(), v["clone"]["consent"].as_bool(), v["speaking_with"].as_str()), (Some(true), Some(false), Some("")));
    // removing it
    let (s, v) = app.json("DELETE", "/api/characters/echo/voice/reference", None).await;
    assert_eq!((s, v["mode"].as_str()), (StatusCode::OK, Some("system")));
}

#[tokio::test]
async fn one_job_at_a_time_and_cancel() {
    let app = App::new(true);
    app.tts.fail(true);
    app.json("POST", "/api/characters/echo/voice/candidates", Some(json!({ "kind": "design", "description": "清亮" }))).await;
    let v = app.job_done().await;
    assert!(v["job"]["error"].as_str().unwrap().contains("failed"));
    let (s, _) = app.json("POST", "/api/characters/echo/voice/candidates", Some(json!({ "kind": "design", "description": "" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = app.json("POST", "/api/characters/echo/voice/candidates", Some(json!({ "kind": "sing" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = app.json("POST", "/api/tts/cancel", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}
