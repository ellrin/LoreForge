//! Custom voices (design §7, 自訂聲音): Qwen3-TTS through lore-voice's `qwen` module.
//!
//! | method | path | what |
//! |---|---|---|
//! | GET | /api/tts | `{ status: { ready, design, missing_bytes, why }, job }` (job: the download or the candidates being made) |
//! | POST | /api/tts/download | install what is missing into the shared model folder (`job.kind = "download"`, progress in `job`) |
//! | POST | /api/tts/cancel | stop the job |
//! | GET/PUT | /api/characters/{id}/voice | the character's `voice/voice.toml` + which clips exist / `{ mode?, pitch?, taiwan?, han?, clone_text? }` |
//! | POST | /api/characters/{id}/voice/candidates | `{ kind: "design" \| "accent", description?, apple_voice?, line? }`: makes [`CANDIDATES`] takes (`job.candidates` fills as they come) |
//! | GET | /api/characters/{id}/voice/candidates/{n} | candidate `n` (wav) |
//! | POST | /api/characters/{id}/voice/pick | `{ n }`: candidate `n` becomes the character's voice |
//! | GET | /api/characters/{id}/voice/clip/{name} | `chosen` or `reference` (wav) |
//! | PUT/DELETE | /api/characters/{id}/voice/reference | the author's recording (`?consent=1&text=…`, body = the audio) / remove it |
//!
//! 試聽 and replies go through `/api/voice/speak` and `/api/chat` with [`charvoice::template`].
//! Candidates live in `<cache>/LoreEcho/voice/<id>/` (wipeable); only a picked one is kept.

use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lore_voice::qwen::{self, CustomVoice, DesignJob, SynthJob, Tts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Shared;
use crate::charvoice::{self, Chosen};

/// Takes per 產生.
pub const CANDIDATES: u64 = 4;

/// A job body: its state to fill in, the cancel flag.
type Work = Box<dyn FnOnce(Arc<Mutex<Job>>, Arc<AtomicBool>) -> Result<(), String> + Send>;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Job {
    /// "" (none yet) | "download" | "candidates".
    pub kind: String,
    pub running: bool,
    pub phase: String,
    pub done: u64,
    pub total: u64,
    pub error: String,
    /// candidates: whose, how they were made, and the takes made so far.
    pub character: String,
    /// candidates: `design` | `accent`.
    pub source: String,
    #[serde(skip)]
    pub chosen: Chosen,
    pub candidates: Vec<u64>,
}

/// The custom-voice engine of the app (none in tests unless set) and its one job at a time.
pub struct Engine {
    tts: Mutex<Option<Arc<dyn Tts>>>,
    cache: Mutex<PathBuf>,
    job: Arc<Mutex<Job>>,
    cancel: Mutex<Arc<AtomicBool>>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine { tts: Mutex::new(None), cache: Mutex::new(std::env::temp_dir().join("LoreEcho-voice")), job: Arc::default(), cancel: Mutex::new(Arc::default()) }
    }
}

impl Engine {
    /// Use `tts`; candidates go under `cache`.
    pub fn set(&self, tts: Arc<dyn Tts>, cache: PathBuf) {
        *self.tts.lock().unwrap() = Some(tts);
        *self.cache.lock().unwrap() = cache;
    }

    pub fn get(&self) -> Option<Arc<dyn Tts>> {
        self.tts.lock().unwrap().clone()
    }

    fn candidates_dir(&self, id: &str) -> PathBuf {
        self.cache.lock().unwrap().join(id)
    }

    pub fn job(&self) -> Job {
        self.job.lock().unwrap().clone()
    }

    /// Start `work` as the job unless one is running.
    fn start(&self, kind: &str, character: &str, work: impl FnOnce(Arc<Mutex<Job>>, Arc<AtomicBool>) -> Result<(), String> + Send + 'static) -> Result<(), String> {
        {
            let mut j = self.job.lock().unwrap();
            if j.running {
                return Err(if j.kind == "download" { "正在下載" } else { "正在產生聲音" }.into());
            }
            *j = Job { kind: kind.into(), running: true, character: character.into(), ..Default::default() };
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().unwrap() = cancel.clone();
        let job = self.job.clone();
        std::thread::spawn(move || {
            let r = work(job.clone(), cancel);
            let mut j = job.lock().unwrap();
            j.running = false;
            if let Err(e) = r {
                j.error = if e == "cancelled" { "已取消".into() } else { e };
            }
        });
        Ok(())
    }

    pub fn cancel(&self) {
        self.cancel.lock().unwrap().store(true, Ordering::SeqCst);
    }
}

/// The shared model folder: `LOCAL_LLM_HOME`, else `engine.toml [local] home`.
pub fn model_home(home: &lore_home::Home) -> Option<PathBuf> {
    lore_llm::local::home_or(None, home.local_home().as_deref())
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/tts", get(status))
        .route("/api/tts/download", post(download))
        .route("/api/tts/cancel", post(cancel))
        .route("/api/characters/{id}/voice", get(voice_get).put(voice_put))
        .route("/api/characters/{id}/voice/candidates", post(candidates))
        .route("/api/characters/{id}/voice/candidates/{n}", get(candidate_get))
        .route("/api/characters/{id}/voice/pick", post(pick))
        .route("/api/characters/{id}/voice/clip/{name}", get(clip_get))
        .route("/api/characters/{id}/voice/reference", put(reference_put).delete(reference_delete).layer(DefaultBodyLimit::max(32 << 20)))
}

fn bad(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn tts_status(s: &Shared) -> qwen::TtsStatus {
    match s.tts.get() {
        Some(t) => t.status(),
        None => qwen::TtsStatus { why: "這裡沒有自訂聲音".into(), ..Default::default() },
    }
}

async fn status(State(s): State<Shared>) -> Json<Value> {
    let st = tokio::task::spawn_blocking({
        let s = s.clone();
        move || tts_status(&s)
    })
    .await
    .unwrap_or_default();
    Json(json!({ "status": st, "job": s.tts.job() }))
}

async fn download(State(s): State<Shared>) -> Response {
    let Some(t) = s.tts.get() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "這裡沒有自訂聲音") };
    let why = t.status().why;
    if !why.is_empty() {
        return bad(StatusCode::SERVICE_UNAVAILABLE, why);
    }
    let r = s.tts.start("download", "", move |job, cancel| {
        t.install(
            &|p| {
                let mut j = job.lock().unwrap();
                (j.phase, j.done, j.total) = (p.phase, p.done, p.total);
            },
            &cancel,
        )
    });
    match r {
        Ok(()) => (StatusCode::ACCEPTED, Json(s.tts.job())).into_response(),
        Err(e) => bad(StatusCode::CONFLICT, e),
    }
}

async fn cancel(State(s): State<Shared>) -> StatusCode {
    s.tts.cancel();
    StatusCode::NO_CONTENT
}

fn app_dir(s: &Shared) -> Result<PathBuf, Response> {
    s.dir.clone().ok_or_else(|| bad(StatusCode::SERVICE_UNAVAILABLE, "沒有 LoreForge 資料夾，無法儲存"))
}

async fn voice_get(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    if let Err(e) = crate::characters::load(&dir, &id) {
        return bad(StatusCode::NOT_FOUND, e);
    }
    Json(charvoice::view(&dir, &id)).into_response()
}

async fn voice_put(State(s): State<Shared>, Path(id): Path<String>, body: Option<Json<charvoice::Patch>>) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    let Some(Json(p)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { mode?, pitch?, taiwan?, han?, clone_text? }") };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match charvoice::patch(&dir, &id, &p) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CandidatesBody {
    kind: String,
    description: String,
    apple_voice: String,
    line: String,
}

/// What the takes say (short, everyday, no word of the Taiwan reading table).
fn test_line(name: &str) -> String {
    format!("你好，我是{name}。今天想先從哪一段開始？我們慢慢來就好。")
}

async fn candidates(State(s): State<Shared>, Path(id): Path<String>, body: Option<Json<CandidatesBody>>) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { kind, description? }") };
    let c = match crate::characters::load(&dir, &id) {
        Ok(c) => c,
        Err(e) => return bad(StatusCode::NOT_FOUND, e),
    };
    let Some(t) = s.tts.get() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "這裡沒有自訂聲音") };
    let st = t.status();
    let line = Some(b.line.trim()).filter(|l| !l.is_empty()).map(str::to_string).unwrap_or_else(|| test_line(&c.name));
    if line.chars().count() > 120 {
        return bad(StatusCode::BAD_REQUEST, "試聽句太長（最多 120 字）");
    }
    let taiwan = charvoice::read(&dir, &id).taiwan;
    let out = s.tts.candidates_dir(&id);
    let language = c.language.clone();
    let seed0 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() % 1_000_000).unwrap_or(1) * 10;
    let work: Work = match b.kind.as_str() {
        "design" => {
            if !st.design {
                return bad(StatusCode::SERVICE_UNAVAILABLE, "自訂聲音還沒下載");
            }
            let d = b.description.trim().to_string();
            if d.is_empty() || d.chars().count() > 300 {
                return bad(StatusCode::BAD_REQUEST, "描述要 1–300 字");
            }
            let description = if taiwan && !d.contains("台灣") && !d.contains("臺灣") { format!("{d}，{}", charvoice::TAIWAN_HINT) } else { d.clone() };
            let chosen = Chosen { kind: "design".into(), description: d, text: line.clone(), seed: seed0, ..Default::default() };
            Box::new(move |job, cancel| {
                fresh(&out)?;
                {
                    let mut j = job.lock().unwrap();
                    j.source = chosen.kind.clone();
                    j.chosen = chosen;
                }
                for n in 1..=CANDIDATES {
                    step(&job, &cancel, n)?;
                    t.design(&DesignJob { text: line.clone(), language: language.clone(), description: description.clone(), seed: seed0 + n, out: out.join(format!("{n}.wav")) })?;
                    done(&job, n);
                }
                Ok(())
            })
        }
        "accent" => {
            if !st.ready {
                return bad(StatusCode::SERVICE_UNAVAILABLE, "自訂聲音還沒下載");
            }
            let voice = s.voice.clone();
            let apple = b.apple_voice.trim().to_string();
            let chosen = Chosen { kind: "accent".into(), apple_voice: apple.clone(), text: line.clone(), seed: seed0, ..Default::default() };
            Box::new(move |job, cancel| {
                fresh(&out)?;
                {
                    let mut j = job.lock().unwrap();
                    j.source = chosen.kind.clone();
                    j.chosen = chosen;
                }
                job.lock().unwrap().phase = "口音參考".into();
                let reference = accent_reference(&voice, &apple, &out)?;
                for n in 1..=CANDIDATES {
                    step(&job, &cancel, n)?;
                    let v = CustomVoice { audio: reference.clone(), text: qwen::ACCENT_PASSAGE.into(), taiwan, han: false };
                    t.synth(&SynthJob { text: line.clone(), language: language.clone(), voice: v, seed: Some(seed0 + n), out: out.join(format!("{n}.wav")) })?;
                    done(&job, n);
                }
                Ok(())
            })
        }
        _ => return bad(StatusCode::BAD_REQUEST, "kind = design | accent"),
    };
    match s.tts.start("candidates", &id, work) {
        Ok(()) => (StatusCode::ACCEPTED, Json(s.tts.job())).into_response(),
        Err(e) => bad(StatusCode::CONFLICT, e),
    }
}

fn fresh(dir: &FsPath) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())
}

fn step(job: &Mutex<Job>, cancel: &AtomicBool, n: u64) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }
    let mut j = job.lock().unwrap();
    (j.phase, j.done, j.total) = (format!("{n}/{CANDIDATES}"), n - 1, CANDIDATES);
    Ok(())
}

fn done(job: &Mutex<Job>, n: u64) {
    let mut j = job.lock().unwrap();
    j.candidates.push(n);
    j.done = n;
}

/// The Apple Taiwan voice (`apple`, or the best zh-TW one) reading [`qwen::ACCENT_PASSAGE`], as
/// a 24 kHz wav in `dir`.
fn accent_reference(voice: &lore_voice::Voice, apple: &str, dir: &FsPath) -> Result<PathBuf, String> {
    let id = if apple.is_empty() {
        let mut v = voice.voices()?;
        lore_voice::sort_voices(&mut v, "zh-TW");
        v.into_iter().find(|x| x.language == "zh-TW").map(|x| x.id).ok_or("這台 Mac 沒有台灣中文的系統聲音")?
    } else {
        apple.to_string()
    };
    let raw = dir.join("accent.caf");
    voice.synth_file(&lore_voice::SpeakRequest { text: qwen::ACCENT_PASSAGE.into(), voice: id, rate: 1.0, language: "zh-TW".into(), ..Default::default() }, &raw)?;
    let wav = dir.join("accent.wav");
    let head = std::fs::read(&raw).map_err(|e| e.to_string())?;
    if qwen::wav_seconds(&head).is_some() {
        std::fs::rename(&raw, &wav).map_err(|e| e.to_string())?; // already a wav (the fake)
    } else {
        qwen::to_wav24(&raw, &wav)?;
        let _ = std::fs::remove_file(&raw);
    }
    Ok(wav)
}

fn wav(path: PathBuf) -> Response {
    match std::fs::read(&path) {
        Ok(b) => ([(header::CONTENT_TYPE, "audio/wav"), (header::CACHE_CONTROL, "no-store")], b).into_response(),
        Err(_) => bad(StatusCode::NOT_FOUND, "沒有這段聲音"),
    }
}

async fn candidate_get(State(s): State<Shared>, Path((id, n)): Path<(String, String)>) -> Response {
    let n: u64 = n.trim_end_matches(".wav").parse().unwrap_or(0);
    if !crate::characters::valid_id(&id) || !(1..=CANDIDATES).contains(&n) {
        return bad(StatusCode::NOT_FOUND, "沒有這個候選");
    }
    wav(s.tts.candidates_dir(&id).join(format!("{n}.wav")))
}

async fn clip_get(State(s): State<Shared>, Path((id, name)): Path<(String, String)>) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    let file = match name.as_str() {
        "chosen" => charvoice::CHOSEN,
        "reference" => charvoice::REFERENCE,
        _ => return bad(StatusCode::NOT_FOUND, "沒有這段聲音"),
    };
    if !crate::characters::valid_id(&id) {
        return bad(StatusCode::NOT_FOUND, "沒有這段聲音");
    }
    wav(charvoice::folder(&dir, &id).join(file))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PickBody {
    n: u64,
}

async fn pick(State(s): State<Shared>, Path(id): Path<String>, body: Option<Json<PickBody>>) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    let n = body.map(|Json(b)| b.n).unwrap_or(0);
    let j = s.tts.job();
    if j.kind != "candidates" || j.character != id || !j.candidates.contains(&n) {
        return bad(StatusCode::BAD_REQUEST, "沒有這個候選，請重新產生");
    }
    // the job keeps the base seed; take n was made with base + n
    let chosen = Chosen { seed: j.chosen.seed + n, ..j.chosen.clone() };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match charvoice::pick(&dir, &id, &s.tts.candidates_dir(&id).join(format!("{n}.wav")), chosen) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RefQuery {
    consent: Option<String>,
    text: Option<String>,
}

async fn reference_put(State(s): State<Shared>, Path(id): Path<String>, Query(q): Query<RefQuery>, body: Bytes) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    let consent = matches!(q.consent.as_deref(), Some("1" | "true" | "yes"));
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match charvoice::save_reference(&dir, &id, &body, consent, q.text.as_deref()) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

async fn reference_delete(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    let dir = match app_dir(&s) { Ok(d) => d, Err(r) => return r };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match charvoice::remove_reference(&dir, &id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(StatusCode::NOT_FOUND, e),
    }
}
