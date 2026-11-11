//! Custom voices with Qwen3-TTS (mlx-audio, Apple Silicon), next to the Apple engines.
//!
//! - [`Tts`]: what an app needs from the engine: [`Tts::status`], [`Tts::install`] (the 下載
//!   button: size, progress, cancel), [`Tts::synth`] (one sentence in the voice of a reference
//!   clip) and [`Tts::design`] (a clip from a description, for picking a voice). [`Qwen`] is the
//!   real one; [`crate::fake::FakeTts`] the tests'.
//! - [`Mixed`]: a [`Backend`] that listens and speaks with the Apple helper and, for a
//!   [`SpeakRequest`] with a [`CustomVoice`], synthesises the sentence with the [`Tts`] and plays
//!   the file through the helper ([`Player`]: rate and pitch). The next sentence is synthesised
//!   while the one before plays. Not installed or failing → the Apple voice of the request
//!   (`voice`), with one [`Event::Notice`].
//!
//! Files: everything lives in the shared model folder (`<LOCAL_LLM_HOME>/tts`, see
//! `docs/local-models.md`), installed only by [`Tts::install`]; versions in `tts.json`. The worker
//! is a Python child (`python/lore_tts.py`, JSON lines), started on first use, one model in
//! memory, stopped after [`IDLE`] without requests or when the app ends (its stdin closes).
//! It is LoreEcho's alone and small (≈ 3–4 GB while speaking), so it is not one of the shared
//! llama-server's leases: it sits next to the language model (docs/local-models.md §7).
//!
//! The real engine is **not tested on real hardware by `cargo test`**; the flow is (fakes).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Backend, ErrorKind, Event, ListenOptions, Sink, SpeakRequest, VoiceInfo};

/// A voice made from a reference clip (24 kHz mono wav) and what is said in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomVoice {
    pub audio: PathBuf,
    pub text: String,
    /// Write words with their Taiwan readings ([`crate::readings`]).
    #[serde(default)]
    pub taiwan: bool,
    /// 和 as hàn (with `taiwan`).
    #[serde(default)]
    pub han: bool,
}

/// One sentence to synthesise into `out` (wav).
#[derive(Debug, Clone, PartialEq)]
pub struct SynthJob {
    pub text: String,
    /// BCP 47 (`zh-TW`).
    pub language: String,
    pub voice: CustomVoice,
    /// The same seed and text give the same take (candidates differ by seed).
    pub seed: Option<u64>,
    pub out: PathBuf,
}

/// A clip from a description (voice design) into `out` (wav).
#[derive(Debug, Clone, PartialEq)]
pub struct DesignJob {
    pub text: String,
    pub language: String,
    pub description: String,
    pub seed: u64,
    pub out: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TtsStatus {
    /// Python, mlx-audio and the speak model are in place.
    pub ready: bool,
    /// The design model too.
    pub design: bool,
    /// Bytes still to download for everything (0 = all there).
    pub missing_bytes: u64,
    /// Why it cannot be installed here (no model folder, not Apple Silicon); empty when it can.
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Progress {
    /// 下載 uv · Python · 套件 · 模型.
    pub phase: String,
    pub done: u64,
    pub total: u64,
}

pub type OnProgress<'a> = &'a (dyn Fn(Progress) + Sync);

pub trait Tts: Send + Sync + 'static {
    fn status(&self) -> TtsStatus;
    /// Install what is missing (blocking). `cancel` stops it (the error is then `"cancelled"`).
    fn install(&self, progress: OnProgress, cancel: &AtomicBool) -> Result<(), String>;
    /// Blocking; the sentence is in `job.out` when it returns Ok.
    fn synth(&self, job: &SynthJob) -> Result<(), String>;
    fn design(&self, job: &DesignJob) -> Result<(), String>;
    /// Free the memory now (the next call starts it again).
    fn stop(&self) {}
}

/// Plays synthesised sentences: `play` queues a file after the ones before it and reports
/// `SpeakStarted`/`SpeakFinished` with `id` through the backend's sink; `stop_speaking` drops them.
pub trait Player: Backend {
    fn play(&self, id: u64, path: &Path, rate: f64, pitch: f64) -> Result<(), String>;
}

/// The worker stops after this long without a request.
pub const IDLE: Duration = Duration::from_secs(10 * 60);

/// Language name the model takes for a BCP 47 tag.
pub fn lang_code(language: &str) -> &'static str {
    match language.split('-').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "zh" => "chinese",
        "en" => "english",
        "ja" => "japanese",
        "ko" => "korean",
        _ => "auto",
    }
}

// ── the manifest (tts.json) ────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub uv: UvSpec,
    pub python: String,
    pub packages: Vec<String>,
    pub runtime_bytes: u64,
    pub models: Vec<ModelSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UvSpec {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelSpec {
    /// `speak` | `design`.
    pub role: String,
    pub repo: String,
    pub revision: String,
    pub files: Vec<(String, u64)>,
}

impl ModelSpec {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.1).sum()
    }
}

pub fn manifest() -> Manifest {
    serde_json::from_str(include_str!("../tts.json")).expect("tts.json")
}

const SCRIPT: &str = include_str!("../python/lore_tts.py");

/// Where things are under `<home>/tts`.
#[derive(Debug, Clone)]
pub struct Layout {
    pub dir: PathBuf,
}

impl Layout {
    pub fn new(home: &Path) -> Layout {
        Layout { dir: home.join("tts") }
    }
    fn uv(&self) -> PathBuf {
        self.dir.join("uv/uv")
    }
    pub fn python(&self) -> PathBuf {
        self.dir.join("venv/bin/python")
    }
    /// Written by a finished runtime install: the packages it installed.
    fn marker(&self) -> PathBuf {
        self.dir.join("venv/.lore-installed")
    }
    pub fn model(&self, m: &ModelSpec) -> PathBuf {
        self.dir.join("models").join(m.repo.rsplit('/').next().unwrap_or(&m.repo))
    }
    fn script(&self) -> PathBuf {
        self.dir.join("lore_tts.py")
    }
    fn runtime_ok(&self, man: &Manifest) -> bool {
        std::fs::read_to_string(self.marker()).is_ok_and(|s| s.trim() == man.packages.join(" ")) && self.python().is_file()
    }
    /// Bytes of `m` not yet in place.
    fn model_missing(&self, m: &ModelSpec) -> u64 {
        let d = self.model(m);
        m.files.iter().filter(|(f, size)| std::fs::metadata(d.join(f)).map(|x| x.len()).ok() != Some(*size)).map(|f| f.1).sum()
    }
}

// ── the real engine ────────────────────────────────────────────────

/// Qwen3-TTS through mlx-audio in `<home>/tts` (`home` = the shared model folder).
pub struct Qwen {
    home: Option<PathBuf>,
    man: Manifest,
    worker: Arc<Mutex<Option<Worker>>>,
    last: Arc<Mutex<Instant>>,
    watching: AtomicBool,
}

impl Qwen {
    pub fn new(home: Option<PathBuf>) -> Qwen {
        Qwen { home, man: manifest(), worker: Arc::default(), last: Arc::new(Mutex::new(Instant::now())), watching: AtomicBool::new(false) }
    }

    fn layout(&self) -> Result<Layout, String> {
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return Err("自訂聲音需要 Apple Silicon 的 Mac".into());
        }
        self.home.as_deref().map(Layout::new).ok_or_else(|| "不知道模型資料夾（LOCAL_LLM_HOME）；先在 PaperComposer 用一次本機模型".into())
    }

    fn spec(&self, role: &str) -> &ModelSpec {
        self.man.models.iter().find(|m| m.role == role).expect("tts.json role")
    }

    /// The worker's answer to `cmd` (starting it when needed).
    fn call(&self, cmd: Value, timeout: Duration) -> Result<Value, String> {
        let l = self.layout()?;
        if !self.status().ready {
            return Err("自訂聲音還沒下載".into());
        }
        *self.last.lock().unwrap() = Instant::now();
        self.watch_idle();
        let mut g = self.worker.lock().unwrap();
        if g.as_mut().is_none_or(|w| !w.alive()) {
            *g = Some(Worker::spawn(&l.python(), &l.script(), &l.dir)?);
        }
        let r = g.as_mut().unwrap().ask(cmd, timeout);
        if r.is_err() {
            *g = None; // a stuck or dead worker is started again next time
        }
        *self.last.lock().unwrap() = Instant::now();
        r
    }

    fn watch_idle(&self) {
        if self.watching.swap(true, Ordering::SeqCst) {
            return;
        }
        let (worker, last) = (Arc::downgrade(&self.worker), self.last.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let Some(w) = worker.upgrade() else { return };
                if last.lock().unwrap().elapsed() > IDLE
                    && let Ok(mut g) = w.try_lock()
                {
                    *g = None;
                }
            }
        });
    }
}

impl Tts for Qwen {
    fn status(&self) -> TtsStatus {
        let l = match self.layout() {
            Ok(l) => l,
            Err(why) => return TtsStatus { why, ..Default::default() },
        };
        let runtime = l.runtime_ok(&self.man);
        let (speak, design) = (l.model_missing(self.spec("speak")), l.model_missing(self.spec("design")));
        let missing = if runtime { 0 } else { self.man.runtime_bytes } + speak + design;
        TtsStatus { ready: runtime && speak == 0, design: runtime && speak == 0 && design == 0, missing_bytes: missing, why: String::new() }
    }

    fn install(&self, progress: OnProgress, cancel: &AtomicBool) -> Result<(), String> {
        let l = self.layout()?;
        install(&l, &self.man, progress, cancel)
    }

    fn synth(&self, job: &SynthJob) -> Result<(), String> {
        let text = if job.voice.taiwan { crate::readings::taiwan(&job.text, crate::readings::Options { han: job.voice.han }) } else { job.text.clone() };
        let model = self.layout()?.model(self.spec("speak"));
        self.call(
            json!({ "cmd": "synth", "model": model, "text": text, "lang": lang_code(&job.language), "ref_audio": job.voice.audio, "ref_text": job.voice.text, "seed": job.seed, "out": job.out }),
            Duration::from_secs(120),
        )
        .map(|_| ())
    }

    fn design(&self, job: &DesignJob) -> Result<(), String> {
        if !self.status().design {
            return Err("聲音設計模型還沒下載".into());
        }
        let model = self.layout()?.model(self.spec("design"));
        self.call(
            json!({ "cmd": "design", "model": model, "text": job.text, "lang": lang_code(&job.language), "instruct": job.description, "seed": job.seed, "out": job.out }),
            Duration::from_secs(300),
        )
        .map(|_| ())
    }

    fn stop(&self) {
        *self.worker.lock().unwrap() = None;
    }
}

/// The Python child: JSON lines; killed on drop.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    req: u64,
}

impl Worker {
    fn spawn(python: &Path, script: &Path, dir: &Path) -> Result<Worker, String> {
        // the script that matches this build (it may have changed since the install)
        if std::fs::read_to_string(script).ok().as_deref() != Some(SCRIPT) {
            std::fs::write(script, SCRIPT).map_err(|e| format!("{}: {e}", script.display()))?;
        }
        let mut child = Command::new(python)
            .arg("-I")
            .arg(script)
            .current_dir(dir)
            .env("HF_HUB_OFFLINE", "1")
            .env("HF_HOME", dir.join("hf"))
            .env("TOKENIZERS_PARALLELISM", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(dir.join("worker.log")).map(Stdio::from).unwrap_or_else(|_| Stdio::null())) // what Python says, for when it does not start
            .spawn()
            .map_err(|e| format!("無法啟動自訂聲音：{e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        let w = Worker { child, stdin, rx, req: 0 };
        // first line: {"ready":true} once mlx-audio's imports are done
        match w.rx.recv_timeout(Duration::from_secs(120)) {
            Ok(v) if v["ready"] == true => Ok(w),
            _ => Err(format!("自訂聲音沒有啟動（Python 環境可能壞了，請重新下載；記錄在 {}）", dir.join("worker.log").display())),
        }
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn ask(&mut self, mut cmd: Value, timeout: Duration) -> Result<Value, String> {
        self.req += 1;
        cmd["req"] = json!(self.req);
        writeln!(self.stdin, "{cmd}").and_then(|_| self.stdin.flush()).map_err(|e| format!("自訂聲音沒有回應：{e}"))?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let v = self.rx.recv_timeout(left).map_err(|_| "自訂聲音沒有回應".to_string())?;
            if v["req"].as_u64() != Some(self.req) {
                continue; // an answer to an earlier request that timed out
            }
            return if v["ok"] == true { Ok(v) } else { Err(v["error"].as_str().unwrap_or("失敗").to_string()) };
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── installing ─────────────────────────────────────────────────────

const CANCELLED: &str = "cancelled";

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) { Err(CANCELLED.into()) } else { Ok(()) }
}

/// Run `cmd` until it ends, killing it on cancel.
fn run(mut cmd: Command, cancel: &AtomicBool, what: &str) -> Result<(), String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("{what}：{e}"))?;
    let mut err = child.stderr.take();
    let tail = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = err.as_mut() {
            let _ = std::io::Read::read_to_string(e, &mut s);
        }
        s
    });
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CANCELLED.into());
        }
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            let log = tail.join().unwrap_or_default();
            return if st.success() {
                Ok(())
            } else {
                let last: Vec<&str> = log.lines().rev().take(3).collect();
                Err(format!("{what}失敗：{}", last.into_iter().rev().collect::<Vec<_>>().join(" / ")))
            };
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `url` → `to` with curl (resumes a `.part`), `size` checked; progress as bytes arrive.
fn fetch(url: &str, to: &Path, size: u64, cancel: &AtomicBool, on: &dyn Fn(u64)) -> Result<(), String> {
    if std::fs::metadata(to).map(|m| m.len()).ok() == Some(size) {
        on(size);
        return Ok(());
    }
    std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = to.with_file_name(format!("{}.part", to.file_name().unwrap().to_string_lossy()));
    if std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) > size {
        let _ = std::fs::remove_file(&part);
    }
    let mut child = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-C", "-", "-o"])
        .arg(&part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl：{e}"))?;
    loop {
        on(std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0));
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CANCELLED.into()); // the .part stays: the next 下載 goes on from there
        }
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            if !st.success() {
                let mut e = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut s, &mut e);
                }
                return Err(format!("下載失敗：{}", e.trim()));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if got != size {
        let _ = std::fs::remove_file(&part);
        return Err(format!("下載的檔案大小不對（{got} ≠ {size}）"));
    }
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

fn sha256(path: &Path) -> Result<String, String> {
    let out = Command::new("/usr/bin/shasum").args(["-a", "256"]).arg(path).output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string())
}

fn install(l: &Layout, man: &Manifest, progress: OnProgress, cancel: &AtomicBool) -> Result<(), String> {
    std::fs::create_dir_all(&l.dir).map_err(|e| format!("{}: {e}", l.dir.display()))?;
    let models: Vec<&ModelSpec> = man.models.iter().filter(|m| l.model_missing(m) > 0).collect();
    let runtime = !l.runtime_ok(man);
    let total = if runtime { man.runtime_bytes } else { 0 } + models.iter().map(|m| l.model_missing(m)).sum::<u64>();
    let base = std::sync::atomic::AtomicU64::new(0);
    let report = |phase: &str, extra: u64| progress(Progress { phase: phase.into(), done: (base.load(Ordering::SeqCst) + extra).min(total), total });
    if runtime {
        // uv (hash-checked) → Python → mlx-audio; a cancelled or failed step removes the half-made env
        let tgz = l.dir.join("uv.tar.gz");
        fetch(&man.uv.url, &tgz, man.uv.bytes, cancel, &|n| report("下載 uv", n))?;
        if sha256(&tgz)? != man.uv.sha256 {
            let _ = std::fs::remove_file(&tgz);
            return Err("uv 的檢查碼不對，已刪除".into());
        }
        let uv_dir = l.dir.join("uv");
        let _ = std::fs::remove_dir_all(&uv_dir);
        std::fs::create_dir_all(&uv_dir).map_err(|e| e.to_string())?;
        let mut tar = Command::new("/usr/bin/tar");
        tar.arg("xzf").arg(&tgz).arg("-C").arg(&uv_dir).args(["--strip-components", "1"]);
        run(tar, cancel, "解開 uv ")?;
        let _ = std::fs::remove_file(&tgz);
        base.fetch_add(man.uv.bytes, Ordering::SeqCst);
        let uv = |args: &[&str]| {
            let mut c = Command::new(l.uv());
            c.args(args)
                .current_dir(&l.dir)
                .env("UV_PYTHON_INSTALL_DIR", l.dir.join("python"))
                .env("UV_CACHE_DIR", l.dir.join("cache"))
                .env("UV_LINK_MODE", "copy")
                .env("UV_NO_CONFIG", "1")
                .env_remove("VIRTUAL_ENV");
            c
        };
        let venv = l.dir.join("venv");
        let undo = || {
            let _ = std::fs::remove_dir_all(&venv);
        };
        undo();
        report("Python", 0);
        if let Err(e) = run(uv(&["venv", "--python", &man.python, "--python-preference", "only-managed", "venv"]), cancel, "Python ") {
            undo();
            return Err(e);
        }
        base.fetch_add(man.runtime_bytes / 10, Ordering::SeqCst);
        report("套件", 0);
        let mut args = vec!["pip", "install", "--python", "venv/bin/python"];
        args.extend(man.packages.iter().map(String::as_str));
        if let Err(e) = run(uv(&args), cancel, "安裝套件") {
            undo();
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(l.dir.join("cache"));
        std::fs::write(l.marker(), man.packages.join(" ")).map_err(|e| e.to_string())?;
        base.store(man.runtime_bytes, Ordering::SeqCst);
        report("套件", 0);
    }
    for m in models {
        let d = l.model(m);
        for (f, size) in &m.files {
            check(cancel)?;
            let url = format!("https://huggingface.co/{}/resolve/{}/{}", m.repo, m.revision, f);
            let to = d.join(f);
            let had = std::fs::metadata(&to).map(|x| x.len()).ok() == Some(*size);
            fetch(&url, &to, *size, cancel, &|n| if !had { report("模型", n) })?;
            if !had {
                base.fetch_add(*size, Ordering::SeqCst);
            }
        }
    }
    report("完成", 0);
    Ok(())
}

// ── the backend ────────────────────────────────────────────────────

struct Job {
    epoch: u64,
    id: u64,
    r: SpeakRequest,
    voice: CustomVoice,
}

/// Apple listening and voices + custom voices through a [`Tts`] (see the module notes).
pub struct Mixed<P: Player> {
    apple: Arc<P>,
    tts: Arc<dyn Tts>,
    cache: PathBuf,
    epoch: Arc<AtomicU64>,
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    sink: Arc<Mutex<Option<Sink>>>,
    /// The fallback notice is said once until a custom voice works again.
    noticed: Arc<AtomicBool>,
}

impl<P: Player> Mixed<P> {
    /// `cache`: where sentence files go (removed when played).
    pub fn new(apple: P, tts: Arc<dyn Tts>, cache: PathBuf) -> Mixed<P> {
        Mixed { apple: Arc::new(apple), tts, cache, epoch: Arc::default(), jobs: Mutex::new(None), sink: Arc::default(), noticed: Arc::default() }
    }

    fn file(cache: &Path, id: u64) -> PathBuf {
        cache.join(format!("say-{}-{id}.wav", std::process::id()))
    }

    fn worker(&self) -> mpsc::Sender<Job> {
        let mut g = self.jobs.lock().unwrap();
        if let Some(tx) = g.as_ref() {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel::<Job>();
        let (apple, tts, cache, epoch, sink, noticed) = (self.apple.clone(), self.tts.clone(), self.cache.clone(), self.epoch.clone(), self.sink.clone(), self.noticed.clone());
        std::thread::spawn(move || {
            for j in rx {
                if j.epoch != epoch.load(Ordering::SeqCst) {
                    continue; // stopped while waiting
                }
                let out = Mixed::<P>::file(&cache, j.id);
                let made = std::fs::create_dir_all(&cache).map_err(|e| e.to_string()).and_then(|_| tts.synth(&SynthJob { text: j.r.text.clone(), language: j.r.language.clone(), voice: j.voice.clone(), seed: None, out: out.clone() }));
                if j.epoch != epoch.load(Ordering::SeqCst) {
                    let _ = std::fs::remove_file(&out);
                    continue;
                }
                let played = match made {
                    Ok(()) => {
                        noticed.store(false, Ordering::SeqCst);
                        apple.play(j.id, &out, j.r.rate, j.r.pitch)
                    }
                    Err(e) => Err(e),
                };
                if let Err(e) = played {
                    let _ = std::fs::remove_file(&out);
                    fallback(&*apple, &sink, &noticed, j.id, &j.r, &format!("自訂聲音失敗（{e}），先用系統聲音"));
                }
            }
        });
        *g = Some(tx.clone());
        tx
    }
}

/// Speak `r` with the Apple voice instead, saying why once.
fn fallback<P: Player + ?Sized>(apple: &P, sink: &Mutex<Option<Sink>>, noticed: &AtomicBool, id: u64, r: &SpeakRequest, why: &str) {
    if !noticed.swap(true, Ordering::SeqCst)
        && let Some(s) = sink.lock().unwrap().clone()
    {
        s(Event::Notice { message: why.into() });
    }
    let plain = SpeakRequest { custom: None, ..r.clone() };
    if let Err(e) = apple.speak(id, &plain)
        && let Some(s) = sink.lock().unwrap().clone()
    {
        s(Event::Error { kind: ErrorKind::Failed, message: e });
        s(Event::SpeakFinished { id, cancelled: true });
    }
}

impl<P: Player> Backend for Mixed<P> {
    fn start(&self, sink: Sink) {
        *self.sink.lock().unwrap() = Some(sink.clone());
        // a sentence file is removed once it has been played
        let cache = self.cache.clone();
        self.apple.start(Arc::new(move |e: Event| {
            if let Event::SpeakFinished { id, .. } = &e {
                let _ = std::fs::remove_file(Mixed::<P>::file(&cache, *id));
            }
            sink(e)
        }));
    }

    fn name(&self) -> &str {
        self.apple.name()
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        self.apple.voices()
    }

    fn listen(&self, o: &ListenOptions) -> Result<(), String> {
        self.apple.listen(o)
    }

    fn stop_listening(&self) {
        self.apple.stop_listening()
    }

    fn synth_file(&self, r: &SpeakRequest, path: &Path) -> Result<(), String> {
        self.apple.synth_file(r, path)
    }

    fn speak(&self, id: u64, r: &SpeakRequest) -> Result<(), String> {
        let Some(voice) = r.custom.clone() else { return self.apple.speak(id, r) };
        if !self.tts.status().ready {
            fallback(&*self.apple, &self.sink, &self.noticed, id, r, "自訂聲音還沒下載，先用系統聲音");
            return Ok(());
        }
        let job = Job { epoch: self.epoch.load(Ordering::SeqCst), id, r: r.clone(), voice };
        self.worker().send(job).map_err(|_| "自訂聲音停了".to_string())
    }

    fn stop_speaking(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.apple.stop_speaking();
    }
}

// ── audio files ────────────────────────────────────────────────────

/// A 16-bit mono PCM wav.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// The length of a PCM wav in seconds (`None`: not a wav we can read).
pub fn wav_seconds(b: &[u8]) -> Option<f64> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut i, mut byte_rate) = (12usize, 0u32);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        let body = i + 8;
        if id == b"fmt " && body + 12 <= b.len() {
            byte_rate = u32::from_le_bytes(b[body + 8..body + 12].try_into().ok()?);
        } else if id == b"data" {
            let len = len.min(b.len() - body);
            return (byte_rate > 0).then(|| len as f64 / byte_rate as f64);
        }
        i = body.checked_add(len)?.checked_add(len & 1)?;
    }
    None
}

/// Any audio file macOS can read → 24 kHz mono 16-bit wav (what the model wants), with
/// `/usr/bin/afconvert`.
pub fn to_wav24(from: &Path, to: &Path) -> Result<(), String> {
    let out = Command::new("/usr/bin/afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@24000", "-c", "1"])
        .arg(from)
        .arg(to)
        .output()
        .map_err(|e| format!("無法轉換音檔（afconvert）：{e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("這個音檔讀不了：{}", String::from_utf8_lossy(&out.stderr).trim())) }
}

/// What the Apple Taiwan voice reads to make an accent reference (≈ 12 s; a longer clip makes every
/// sentence slower. Taiwan words: 便利商店, 腳踏車, 禮拜六, 鮮奶, 飯糰, 跟).
pub const ACCENT_PASSAGE: &str = "欸，你吃飽了沒？我剛剛去巷口的便利商店買了鮮奶跟飯糰。禮拜六沒下雨的話，我們騎腳踏車去河濱公園走走，好不好？";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Voice;
    use crate::fake::{Call, FakeBackend, FakeTts};

    fn custom() -> CustomVoice {
        CustomVoice { audio: PathBuf::from("/x/ref.wav"), text: "參考".into(), taiwan: false, han: false }
    }

    fn wait(mut ok: impl FnMut() -> bool) {
        for _ in 0..300 {
            if ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out");
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<Event> {
        let mut v = vec![];
        while let Ok(e) = rx.try_recv() {
            v.push(e);
        }
        v
    }

    fn tempdir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("lore-voice-qwen-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_reply_in_a_custom_voice_is_synthesised_and_played_in_order() {
        let tmp = tempdir();
        let (fake, ctl) = FakeBackend::instant();
        let tts = FakeTts::new(true);
        let v = Voice::new(Box::new(Mixed::new(fake, Arc::new(tts.clone()), tmp.clone())));
        let mut rx = v.subscribe();
        let mut sp = v.speaker_from(SpeakRequest { voice: "apple.fallback".into(), rate: 1.2, pitch: 9.0, language: "zh-TW".into(), custom: Some(custom()), ..Default::default() });
        sp.push("第一句。第二");
        sp.push("句！第三");
        sp.finish();
        wait(|| !v.speaking());
        let texts: Vec<String> = tts.synths().into_iter().map(|j| j.text).collect();
        assert_eq!(texts, ["第一句。", "第二句！", "第三"]);
        assert!(tts.synths().iter().all(|j| j.language == "zh-TW" && j.voice == custom()));
        let plays: Vec<(u64, f64, f64)> = ctl.calls().into_iter().filter_map(|c| if let Call::Play(id, _, r, p) = c { Some((id, r, p)) } else { None }).collect();
        assert_eq!(plays.iter().map(|p| p.0).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(plays.iter().all(|p| p.1 == 1.2 && p.2 == crate::PITCH.1), "rate kept, pitch clamped");
        assert!(!ctl.calls().iter().any(|c| matches!(c, Call::Speak(..))), "no system voice");
        let ev = drain(&mut rx);
        assert_eq!(ev.first(), Some(&Event::Speaking { on: true }));
        assert_eq!(ev.last(), Some(&Event::Speaking { on: false }));
        assert!(!ev.iter().any(|e| matches!(e, Event::Notice { .. } | Event::Error { .. })));
        // played files are removed
        wait(|| std::fs::read_dir(&tmp).map(|d| d.count() == 0).unwrap_or(true));
    }

    #[test]
    fn not_installed_falls_back_to_the_system_voice_with_one_notice() {
        let (fake, ctl) = FakeBackend::instant();
        let tts = FakeTts::new(false);
        let v = Voice::new(Box::new(Mixed::new(fake, Arc::new(tts.clone()), tempdir())));
        let mut rx = v.subscribe();
        let mut sp = v.speaker_from(SpeakRequest { voice: "apple.meijia".into(), language: "zh-TW".into(), custom: Some(custom()), ..Default::default() });
        sp.push("一。二。");
        sp.finish();
        let spoken: Vec<SpeakRequest> = ctl.calls().into_iter().filter_map(|c| if let Call::Speak(_, r) = c { Some(r) } else { None }).collect();
        assert_eq!(spoken.len(), 2);
        assert!(spoken.iter().all(|r| r.voice == "apple.meijia" && r.custom.is_none()));
        assert!(tts.synths().is_empty());
        let notices = drain(&mut rx).into_iter().filter(|e| matches!(e, Event::Notice { .. })).count();
        assert_eq!(notices, 1);
        assert!(!v.speaking());
    }

    #[test]
    fn a_failing_engine_falls_back_per_sentence() {
        let (fake, ctl) = FakeBackend::instant();
        let tts = FakeTts::new(true);
        tts.fail(true);
        let v = Voice::new(Box::new(Mixed::new(fake, Arc::new(tts.clone()), tempdir())));
        v.speak(&SpeakRequest { text: "一。".into(), custom: Some(custom()), ..Default::default() }).unwrap();
        wait(|| !v.speaking());
        assert!(ctl.calls().iter().any(|c| matches!(c, Call::Speak(1, r) if r.text == "一。" && r.custom.is_none())));
    }

    #[test]
    fn stopping_drops_sentences_still_being_made() {
        let (fake, ctl) = FakeBackend::new(); // nothing finishes by itself
        let tts = FakeTts::new(true);
        tts.delay(50);
        let v = Voice::new(Box::new(Mixed::new(fake, Arc::new(tts.clone()), tempdir())));
        for t in ["一。", "二。", "三。"] {
            v.speak(&SpeakRequest { text: t.into(), custom: Some(custom()), ..Default::default() }).unwrap();
        }
        v.stop_speaking();
        assert!(!v.speaking());
        std::thread::sleep(Duration::from_millis(250));
        assert!(tts.synths().len() <= 1, "at most the one already started");
        assert!(!ctl.calls().iter().any(|c| matches!(c, Call::Play(..))), "nothing played after the stop");
        assert!(ctl.calls().contains(&Call::StopSpeaking));
    }

    #[test]
    fn plain_requests_and_listening_go_to_the_apple_side() {
        let (fake, ctl) = FakeBackend::instant();
        let v = Voice::new(Box::new(Mixed::new(fake, Arc::new(FakeTts::new(true)), tempdir())));
        v.speak(&SpeakRequest { text: "嗨".into(), ..Default::default() }).unwrap();
        v.listen(&ListenOptions { language: "zh-TW".into(), continuous: false }).unwrap();
        assert!(matches!(ctl.calls()[0], Call::Speak(..)));
        assert!(matches!(ctl.calls()[1], Call::Listen(..)));
        assert_eq!(v.voices().unwrap().len(), crate::fake::voices().len());
    }

    #[test]
    fn wav_helpers_and_language_names() {
        let w = wav_bytes(&vec![0; 24000 * 3], 24000);
        assert_eq!(wav_seconds(&w), Some(3.0));
        assert_eq!(wav_seconds(b"RIFF....WAVE"), None);
        assert_eq!(wav_seconds(b"not a wav at all"), None);
        assert_eq!(lang_code("zh-TW"), "chinese");
        assert_eq!(lang_code("en-US"), "english");
        assert_eq!(lang_code("fr"), "auto");
    }

    #[test]
    fn the_manifest_names_both_models_with_sizes_and_nothing_is_installed_by_itself() {
        let m = manifest();
        assert!(m.uv.sha256.len() == 64 && m.uv.url.contains(&m.uv.version));
        for role in ["speak", "design"] {
            let s = m.models.iter().find(|x| x.role == role).unwrap();
            assert!(s.bytes() > 1_000_000_000 && s.revision.len() == 40, "{role}");
        }
        let tmp = tempdir();
        let q = Qwen::new(Some(tmp.clone()));
        let st = q.status();
        let here = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        if here {
            assert!(!st.ready && st.missing_bytes > 5_000_000_000 && st.why.is_empty());
            assert!(Qwen::new(None).status().why.contains("模型資料夾"));
        } else {
            assert!(!st.why.is_empty());
        }
        assert!(q.synth(&SynthJob { text: "x".into(), language: "zh-TW".into(), voice: custom(), seed: None, out: tmp.join("o.wav") }).is_err());
        assert!(!tmp.join("tts").exists(), "status and a refused synth create nothing");
    }
}
