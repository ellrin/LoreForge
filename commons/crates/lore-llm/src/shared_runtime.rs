//! Local models shared by every Lore app in-process (docs/local-models.md).
//!
//! No app owns the model and there is no broker process: every app follows the same
//! protocol over one small registry in the cache folder,
//! `<cache>/LoreForge/local-model.json` (`LOREFORGE_CACHE` moves it), read and written only
//! under `flock` on `local-model.lock` (the OS drops the lock when its holder dies):
//!
//! ```json
//! { "server":  { "model": "qwen3.5-9b-q8", "url": "http://127.0.0.1:51234/v1", "pid": 4321,
//!                "started_by": { "app": "pc", "pid": 1200 }, "since": 1794363123,
//!                "started_at": "Wed Nov 11 20:15:02 2026", "vision": false,
//!                "loading": true, "from": "gemma-4-12b-q8" },
//!   "leases":  [ { "app": "le", "pid": 1500, "since": 1794363200, "last_used": 1794363300,
//!                  "priority": "interactive", "active": 0 } ],
//!   "wanting": [ { "app": "pc", "pid": 1200, "model": "gemma-4-31b-q4",
//!                  "priority": "background", "since": 1794363310 } ],
//!   "last_activity": 1794363300 }
//! ```
//!
//! `loading` (only while the server loads its weights) and `from` (the model it replaced)
//! are left out when not set; `active` = requests of that process running now.
//! Times are Unix seconds; `started_at` is `ps -o lstart=` of the server's pid.
//!
//! - **One model at a time.** [`SharedRuntime::acquire`] hands back the loaded server when it
//!   runs the model asked for; starts one when none runs (any app can: the binary and the
//!   weights come from a [`Layout`]); otherwise the caller waits in `wanting` until the model
//!   may change: no request running (leases' `active`, the server's `/slots`), the model has
//!   been loaded for [`Timing::min_residency`], and for a background caller nobody used it in
//!   the last [`Timing::hold`]. Interactive callers go before background ones, first come
//!   first served within a level; a background caller waiting [`Timing::promote_after`]
//!   counts as interactive (the order of the retired broker's `sched.rs`, without focus).
//! - **Pruning** on every locked read: leases and wanters whose pid is gone are dropped; the
//!   server stays only while its pid runs `llama-server` with the recorded start time and
//!   `/health` answers (or it is still loading). Nothing is ever signalled without that
//!   check. A broken file is read as empty and the server named in it is put back when it
//!   still answers.
//! - **Image tools** (`tools` in the file): the upscaler ([`SharedRuntime::begin_tool_blocking`])
//!   waits for the running requests to end, then registers; while it runs, requests for the
//!   local model wait and no idle release happens. The loaded model stays in memory (the tool
//!   is small); only GPU time is yielded.
//! - **Release**: a request's [`Ticket`] updates `last_used`; [`SharedRuntime::check_idle`]
//!   (every app, every [`Timing::idle_tick`]) stops a server unused for the idle time; the
//!   last lease holder to quit ([`SharedRuntime::shutdown`]) stops it.
//! - The server is started detached (`setsid`), its output in `<cache>/LoreForge/llama-server.log`
//!   (emptied at each start), so an app quitting never takes it down by accident.
//!
//! [`SharedLocal`] is the router's provider for `local:` models: it acquires per request,
//! and acquires once more when the server died under it. [`LocalStatus`] +
//! [`LocalStatus::text`] are the words every app shows (section 4 of the design).
//!
//! macOS (and other Unix) only. Tested with a fake llama-server (`lore-llm-testkit`) and real
//! child processes; a real llama-server is **not tested on real hardware** here.

use crate::catalog::{Catalog, Model};
use crate::error::LlmError;
use crate::local::{Layout, server_args};
use crate::openai::{OpenAiCompat, OpenAiCompatConfig};
use crate::provider::{LlmProvider, OnEvent};
use crate::shared::TaskClass;
use crate::types::{Capabilities, LlmRequest, LlmResponse};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The registry file in the cache folder.
pub const REGISTRY: &str = "local-model.json";
/// The lock file next to it (`flock`).
pub const LOCK: &str = "local-model.lock";
/// llama-server's output, emptied at each start.
pub const LOG: &str = "llama-server.log";
/// The executable name a registered server's pid must run.
pub const SERVER_EXE: &str = "llama-server";

// ── priority ────────────────────────────────────────────

/// How urgent a local request is. Interactive: someone is looking at the screen for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Interactive,
    #[default]
    Background,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Interactive => "interactive",
            Priority::Background => "background",
        }
    }

    pub fn parse(s: &str) -> Option<Priority> {
        match s.trim() {
            "interactive" => Some(Priority::Interactive),
            "background" => Some(Priority::Background),
            _ => None,
        }
    }

    /// The class default: `ask` (and a task without a class) interactive, `create` / `tidy`
    /// background. A caller with someone waiting on screen passes `interactive` itself.
    pub fn for_class(class: Option<TaskClass>) -> Priority {
        match class {
            Some(TaskClass::Create | TaskClass::Tidy) => Priority::Background,
            Some(TaskClass::Ask) | None => Priority::Interactive,
        }
    }
}

// ── timing ──────────────────────────────────────────────

/// The protocol's times (the defaults are the design's; tests shorten them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timing {
    /// A model stays at least this long after it was loaded.
    pub min_residency: Duration,
    /// A lease used this recently keeps the model for background wanters.
    pub hold: Duration,
    /// A background wanter waiting this long counts as interactive.
    pub promote_after: Duration,
    /// How often each app checks for an idle server.
    pub idle_tick: Duration,
    /// Longest a start may take (a large model loads for a while).
    pub start_wait: Duration,
    /// SIGTERM, then SIGKILL after this long.
    pub stop_grace: Duration,
    /// Waiters re-check after this long, growing to `poll_max`.
    pub poll_min: Duration,
    pub poll_max: Duration,
    /// `/health` and `/slots` answer within this.
    pub probe_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            min_residency: Duration::from_secs(60),
            hold: Duration::from_secs(120),
            promote_after: Duration::from_secs(120),
            idle_tick: Duration::from_secs(30),
            start_wait: Duration::from_secs(300),
            stop_grace: Duration::from_secs(5),
            poll_min: Duration::from_millis(500),
            poll_max: Duration::from_secs(2),
            probe_timeout: Duration::from_millis(1500),
        }
    }
}

// ── the registry file ───────────────────────────────────

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// An app process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Who {
    pub app: String,
    pub pid: u32,
}

/// The one running llama-server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub model: String,
    /// Base URL ending in `/v1`.
    pub url: String,
    pub pid: u32,
    pub started_by: Who,
    /// When it was started (Unix seconds).
    pub since: u64,
    /// `ps -o lstart=` of `pid` (checked before any signal).
    pub started_at: String,
    /// Started with a vision projector.
    pub vision: bool,
    /// Still loading its weights.
    #[serde(default, skip_serializing_if = "is_false")]
    pub loading: bool,
    /// The model it replaced (a switch rather than a first load).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// An app process that uses the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub app: String,
    pub pid: u32,
    pub since: u64,
    pub last_used: u64,
    pub priority: Priority,
    /// Requests of this process running now.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub active: u32,
}

/// An app process waiting for another model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Want {
    pub app: String,
    pub pid: u32,
    pub model: String,
    pub priority: Priority,
    pub since: u64,
}

/// `local-model.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub server: Option<Server>,
    #[serde(default)]
    pub leases: Vec<Lease>,
    #[serde(default)]
    pub wanting: Vec<Want>,
    /// Image tools (upscaler, ...) running now; while any does, local-model requests wait
    /// ([`SharedRuntime::begin_tool_blocking`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolUse>,
    #[serde(default)]
    pub last_activity: u64,
}

/// An app process running an image tool (a small GPU job next to the loaded model): the
/// language model yields for as long as it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUse {
    pub app: String,
    pub pid: u32,
    /// What runs (`upscale`).
    pub tool: String,
    pub since: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn secs(d: Duration) -> u64 {
    d.as_secs_f64().ceil() as u64
}

impl Registry {
    fn effective(&self, w: &Want, now: u64, t: &Timing) -> Priority {
        if w.priority == Priority::Background && now.saturating_sub(w.since) >= secs(t.promote_after) {
            Priority::Interactive
        } else {
            w.priority
        }
    }

    /// Wanters of a model other than the loaded one, in the order they may change it.
    fn line(&self, now: u64, t: &Timing) -> Vec<&Want> {
        let loaded = self.server.as_ref().map(|s| s.model.as_str());
        let mut v: Vec<&Want> = self.wanting.iter().filter(|w| Some(w.model.as_str()) != loaded).collect();
        v.sort_by_key(|w| (self.effective(w, now, t) != Priority::Interactive, w.since, w.pid));
        v
    }

    /// The lease that used the server last (other than `except`), for 「等待 X 用完模型」.
    fn user(&self, except: u32) -> Option<&Lease> {
        self.leases.iter().filter(|l| l.pid != except).max_by_key(|l| (l.active, l.last_used))
    }
}

// ── processes and HTTP (blocking, short) ────────────────

fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// `(lstart, comm)` of a pid from `ps` (lstart with single spaces, comm a full path).
fn ps_info(pid: u32) -> Option<(String, String)> {
    let out = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "lstart=", "-o", "comm="]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let toks: Vec<&str> = line.split_whitespace().collect();
    if toks.len() < 6 {
        return None;
    }
    // lstart is five words: "Wed Nov 11 20:15:02 2026"; comm is the rest (a path may hold spaces)
    Some((toks[..5].join(" "), toks[5..].join(" ")))
}

/// The pid runs llama-server and started when the registry says.
fn verified(pid: u32, started_at: &str) -> bool {
    let Some((lstart, comm)) = ps_info(pid) else { return false };
    let base = Path::new(&comm).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(comm);
    base == SERVER_EXE && lstart == started_at.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn signal(pid: u32, sig: libc::c_int) {
    // SAFETY: only ever called on a pid just verified as our llama-server.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

/// Stop a registered server: SIGTERM, SIGKILL after the grace time. Only when its pid is
/// verified; otherwise nothing is signalled (the caller just forgets the registration).
fn stop_server(s: &Server, grace: Duration) {
    if !verified(s.pid, &s.started_at) {
        return;
    }
    signal(s.pid, libc::SIGTERM);
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !pid_alive(s.pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if verified(s.pid, &s.started_at) {
        signal(s.pid, libc::SIGKILL);
        let deadline = Instant::now() + Duration::from_secs(2);
        while pid_alive(s.pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A plain HTTP/1.0 GET to `127.0.0.1`: `(status, body)`.
fn http_get(v1_url: &str, path: &str, timeout: Duration) -> Option<(u16, String)> {
    use std::net::ToSocketAddrs;
    let base = v1_url.trim_end_matches('/').trim_end_matches("/v1");
    let host = base.strip_prefix("http://")?;
    let addr = host.to_socket_addrs().ok()?.next()?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    s.set_write_timeout(Some(timeout)).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").ok()?;
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let status = text.split_whitespace().nth(1)?.parse().ok()?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Some((status, body))
}

/// `/health`: `Some(true)` ready, `Some(false)` answering but loading, `None` no answer.
fn health(url: &str, t: &Timing) -> Option<bool> {
    http_get(url, "/health", t.probe_timeout).map(|(st, _)| st == 200)
}

/// A slot is processing (`/slots`; no answer = not known to be busy).
fn slots_busy(url: &str, t: &Timing) -> bool {
    let Some((200, body)) = http_get(url, "/slots", t.probe_timeout) else { return false };
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().any(|s| s["is_processing"].as_bool().unwrap_or(false))))
        .unwrap_or(false)
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).ok()
}

/// The pid listening on a local port (`lsof`), for a broken registry.
fn listener_pid(port: u16) -> Option<u32> {
    let out = std::process::Command::new("lsof").args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok()
}

/// Put back the server a broken registry named, when it still answers and its pid checks out.
fn recover(text: &str, t: &Timing) -> Option<Server> {
    let at = text.find("http://127.0.0.1:")?;
    let rest = &text[at..];
    let end = rest.find(|c: char| c == '"' || c.is_whitespace()).unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches('/').to_string();
    let url = if url.ends_with("/v1") { url } else { format!("{url}/v1") };
    let port: u16 = url.trim_start_matches("http://127.0.0.1:").trim_end_matches("/v1").parse().ok()?;
    if health(&url, t) != Some(true) {
        return None;
    }
    let pid = listener_pid(port)?;
    let (started_at, comm) = ps_info(pid)?;
    if Path::new(&comm).file_name()? != SERVER_EXE {
        return None;
    }
    let (_, body) = http_get(&url, "/v1/models", t.probe_timeout)?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let model = v["data"][0]["id"].as_str()?.to_string();
    Some(Server {
        model,
        url,
        pid,
        started_by: Who { app: String::new(), pid: 0 },
        since: now(),
        started_at,
        vision: false,
        loading: false,
        from: None,
    })
}

// ── the runtime ─────────────────────────────────────────

/// See [`SharedRuntime::watch_idle`]: the idle time now (`None`: never release).
pub type IdleSource = Arc<dyn Fn() -> Option<Duration> + Send + Sync>;

/// One app process's side of the protocol.
pub struct SharedRuntime {
    /// Short app name (`pc`, `sc`, `lr`, `kb`, `le`).
    pub app: String,
    pub pid: u32,
    /// The registry folder.
    pub dir: PathBuf,
    pub timing: Timing,
    watching: AtomicBool,
    idle: std::sync::Mutex<Option<IdleSource>>,
}

/// A request's hold on the server: its URL; dropping it records the use.
pub struct Ticket {
    rt: Arc<SharedRuntime>,
    pub url: String,
    pub model: String,
    pub vision: bool,
    done: bool,
}

impl Ticket {
    /// The request is over (also on drop).
    pub fn finish(mut self) {
        self.close();
    }
    fn close(&mut self) {
        if !self.done {
            self.done = true;
            self.rt.finished();
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.close();
    }
}

/// An image tool's hold on the runtime; dropping it lets the language model back.
pub struct ToolTicket {
    rt: Arc<SharedRuntime>,
    tool: String,
}

impl Drop for ToolTicket {
    fn drop(&mut self) {
        let _ = self.rt.locked(false, |reg| {
            reg.tools.retain(|u| !(u.pid == self.rt.pid && u.tool == self.tool));
            reg.last_activity = now();
        });
    }
}

/// What this process sees, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum LocalStatus {
    /// No server.
    Off,
    /// Loading a model into empty memory.
    Loading { model: String, secs: u64 },
    /// Replacing one model with another.
    Switching { model: String, from: String, secs: u64 },
    /// This app waits for another model while `app` uses the loaded one.
    Waiting { app: String, model: String },
    /// A model is loaded and ready.
    Loaded { model: String },
}

/// Display name of a short app name.
pub fn app_label(app: &str) -> String {
    match app {
        "pc" => "PaperComposer",
        "sc" => "SlideComposer",
        "lr" => "LoreReader",
        "kb" => "KnowledgeBase",
        "le" => "LoreEcho",
        "" => "另一個 App",
        other => other,
    }
    .to_string()
}

impl LocalStatus {
    /// The words every app shows (design §4): 載入模型中 · N 秒, 等待 <app> 用完模型,
    /// 切換模型中 · N 秒, 已載入 <模型>, 未載入.
    pub fn text(&self, catalog: &Catalog) -> String {
        let name = |id: &str| catalog.get(id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string());
        match self {
            LocalStatus::Off => "未載入".into(),
            LocalStatus::Loading { secs, .. } => format!("載入模型中 · {secs} 秒"),
            LocalStatus::Switching { secs, .. } => format!("切換模型中 · {secs} 秒"),
            LocalStatus::Waiting { app, .. } => format!("等待 {} 用完模型", app_label(app)),
            LocalStatus::Loaded { model } => format!("已載入 {}", name(model)),
        }
    }
}

/// The registry as one app sees it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub registry: Registry,
    pub status: LocalStatus,
    /// The 釋放 button may stop the server (no other app holds a live lease).
    pub can_release: bool,
}

impl Snapshot {
    /// Apps (short names) that hold a lease, other than `pid`.
    pub fn users(&self, except: u32) -> Vec<String> {
        let mut v: Vec<String> = self.registry.leases.iter().filter(|l| l.pid != except).map(|l| l.app.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// What `pid` sees in `reg` at `n`.
fn status_of(reg: &Registry, pid: u32, n: u64) -> LocalStatus {
    let mine = reg.wanting.iter().find(|w| w.pid == pid);
    if let (Some(w), Some(u)) = (mine, reg.tools.first()) {
        return LocalStatus::Waiting { app: u.app.clone(), model: w.model.clone() };
    }
    match (&reg.server, mine) {
        (Some(s), Some(w)) if !s.loading && s.model != w.model => {
            let app = reg.user(pid).map(|l| l.app.clone()).unwrap_or_else(|| s.started_by.app.clone());
            LocalStatus::Waiting { app, model: w.model.clone() }
        }
        (None, _) => LocalStatus::Off,
        (Some(s), _) if s.loading => match &s.from {
            Some(from) => LocalStatus::Switching { model: s.model.clone(), from: from.clone(), secs: n.saturating_sub(s.since) },
            None => LocalStatus::Loading { model: s.model.clone(), secs: n.saturating_sub(s.since) },
        },
        (Some(s), _) => LocalStatus::Loaded { model: s.model.clone() },
    }
}

enum Step {
    Ready { url: String, model: String, vision: bool },
    /// Started a server: wait for it to answer.
    Started { url: String, pid: u32, started_at: String },
    Wait,
}

static PROCESS: OnceLock<Arc<SharedRuntime>> = OnceLock::new();

impl SharedRuntime {
    /// A runtime over the registry in `dir` (tests: a temp folder).
    pub fn new(app: impl Into<String>, dir: impl Into<PathBuf>, timing: Timing) -> Arc<SharedRuntime> {
        Arc::new(SharedRuntime { app: app.into(), pid: std::process::id(), dir: dir.into(), timing, watching: AtomicBool::new(false), idle: std::sync::Mutex::new(None) })
    }

    /// This process's runtime over the real cache folder (`<cache>/LoreForge`,
    /// `LOREFORGE_CACHE`); the first call names the app. `None` without a cache folder.
    pub fn process(app: &str) -> Option<Arc<SharedRuntime>> {
        if let Some(rt) = PROCESS.get() {
            return Some(rt.clone());
        }
        let dir = lore_home::ports::cache_dir()?;
        Some(PROCESS.get_or_init(|| SharedRuntime::new(app, dir, Timing::default())).clone())
    }

    /// The process runtime, when one was made.
    pub fn current_process() -> Option<Arc<SharedRuntime>> {
        PROCESS.get().cloned()
    }

    /// At quit: this process's lease goes, and the server stops when it was the last user.
    /// Synchronous (window close and signal handlers cannot wait on async work).
    pub fn shutdown_process() {
        if let Some(rt) = PROCESS.get() {
            rt.shutdown();
        }
    }

    pub fn registry_path(&self) -> PathBuf {
        self.dir.join(REGISTRY)
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir.join(LOG)
    }

    /// Run `f` on the registry under the lock (pruned first when `prune`); written back
    /// when changed. Blocking.
    fn locked<T>(&self, prune: bool, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
        std::fs::create_dir_all(&self.dir)?;
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.dir.join(LOCK))?;
        lock.lock()?;
        let path = self.registry_path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (mut reg, broken) = if text.trim().is_empty() {
            (Registry::default(), false)
        } else {
            match serde_json::from_str::<Registry>(&text) {
                Ok(r) => (r, false),
                Err(_) => (Registry::default(), true),
            }
        };
        let before = if broken { None } else { Some(reg.clone()) };
        if broken {
            eprintln!("lore-llm: {} was broken, read as empty", path.display());
            reg.server = recover(&text, &self.timing);
        }
        if prune {
            self.prune(&mut reg);
        }
        let out = f(&mut reg);
        if before.as_ref() != Some(&reg) {
            let tmp = self.dir.join(format!(".{REGISTRY}.{}.tmp", self.pid));
            std::fs::write(&tmp, serde_json::to_vec_pretty(&reg)?)?;
            std::fs::rename(&tmp, &path)?;
        }
        drop(lock); // unlocks
        Ok(out)
    }

    /// Drop dead leases and wanters, and a server that fails its checks (no signal).
    fn prune(&self, reg: &mut Registry) {
        reg.leases.retain(|l| pid_alive(l.pid));
        reg.wanting.retain(|w| pid_alive(w.pid));
        reg.tools.retain(|u| pid_alive(u.pid));
        let t = &self.timing;
        if let Some(s) = reg.server.as_mut() {
            let ok = verified(s.pid, &s.started_at)
                && match health(&s.url, t) {
                    Some(true) => {
                        s.loading = false;
                        true
                    }
                    Some(false) => true,
                    None => s.loading && now() < s.since + secs(t.start_wait),
                };
            if !ok {
                eprintln!("lore-llm: llama-server {} (pid {}) is gone, registration dropped", s.model, s.pid);
                reg.server = None;
            }
        }
    }

    /// Launch llama-server for `m` detached; the registration while it loads.
    fn launch(&self, layout: &Layout, m: &Model, from: Option<String>) -> Result<Server, String> {
        use std::os::unix::process::CommandExt;
        if let Some(why) = layout.missing(m) {
            return Err(why);
        }
        let bin = layout.find_runtime().ok_or("找不到 llama-server")?;
        let weights = layout.find_model(m).ok_or("找不到模型檔")?;
        let mmproj = layout.find_mmproj(m);
        let port = free_port().ok_or("找不到可用的連接埠")?;
        let mut args = server_args(&weights, mmproj.as_deref(), port, m.context_or_default());
        args.extend(["--alias".to_string(), m.id.clone()]);
        let log = std::fs::File::create(self.log_path()).map_err(|e| format!("無法寫入 {}：{e}", self.log_path().display()))?;
        let log2 = log.try_clone().map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(&bin);
        cmd.args(&args).stdin(std::process::Stdio::null()).stdout(log).stderr(log2);
        if let Some(dir) = bin.parent() {
            cmd.current_dir(dir); // its shared libraries sit next to it
        }
        // SAFETY: setsid is async-signal-safe; the server leaves the app's session and
        // process group, so quitting the app never takes it along.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("無法啟動 llama-server：{e}"))?;
        let pid = child.id();
        // reap it when it ends (a zombie would still pass `kill -0`)
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let mut started_at = None;
        for _ in 0..50 {
            if let Some((lstart, _)) = ps_info(pid) {
                started_at = Some(lstart);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let started_at = started_at.ok_or("llama-server 一啟動就結束了")?;
        Ok(Server {
            model: m.id.clone(),
            url: format!("http://127.0.0.1:{port}/v1"),
            pid,
            started_by: Who { app: self.app.clone(), pid: self.pid },
            since: now(),
            started_at,
            vision: mmproj.is_some(),
            loading: true,
            from,
        })
    }

    fn lease_mut<'a>(&self, reg: &'a mut Registry, priority: Priority) -> &'a mut Lease {
        let t = now();
        if let Some(i) = reg.leases.iter().position(|l| l.pid == self.pid) {
            let l = &mut reg.leases[i];
            if priority == Priority::Interactive {
                l.priority = priority;
            }
            return l;
        }
        reg.leases.push(Lease { app: self.app.clone(), pid: self.pid, since: t, last_used: t, priority, active: 0 });
        reg.leases.last_mut().expect("just pushed")
    }

    /// One locked look: take the server, start one, or wait.
    fn step(&self, layout: Option<&Layout>, m: &Model, priority: Priority) -> Result<Step, String> {
        let t = &self.timing;
        let res = self.locked(true, |reg| -> Result<Step, String> {
            let n = now();
            let blocked = !reg.tools.is_empty(); // an image tool runs: the model yields
            match reg.server.as_ref() {
                Some(s) if !blocked && s.model == m.id && !s.loading => {
                    let (url, vision) = (s.url.clone(), s.vision);
                    reg.wanting.retain(|w| !(w.pid == self.pid && w.model == m.id));
                    let l = self.lease_mut(reg, priority);
                    l.active += 1;
                    l.last_used = n;
                    reg.last_activity = n;
                    return Ok(Step::Ready { url, model: m.id.clone(), vision });
                }
                _ => {}
            }
            match reg.wanting.iter_mut().find(|w| w.pid == self.pid && w.model == m.id) {
                Some(w) => {
                    if priority == Priority::Interactive {
                        w.priority = priority;
                    }
                }
                None => reg.wanting.push(Want { app: self.app.clone(), pid: self.pid, model: m.id.clone(), priority, since: n }),
            }
            if blocked {
                return Ok(Step::Wait);
            }
            let Some(first) = reg.line(n, t).first().map(|w| (w.pid, w.model.clone(), reg.effective(w, n, t))) else {
                return Ok(Step::Wait); // our model is loading
            };
            if (first.0, first.1.as_str()) != (self.pid, m.id.as_str()) {
                return Ok(Step::Wait);
            }
            // our turn to start a server: only with the model folder (an app without one can
            // use a server another app started, never start one)
            let Some(layout) = layout else {
                reg.wanting.retain(|w| !(w.pid == self.pid && w.model == m.id));
                return Err("這個 App 不知道模型資料夾（LOCAL_LLM_HOME），無法啟動本機模型；請先在 PaperComposer 用一次本機模型".to_string());
            };
            let from = match reg.server.clone() {
                None => None,
                Some(s) => {
                    if s.loading || reg.leases.iter().any(|l| l.active > 0) || n.saturating_sub(s.since) < secs(t.min_residency) {
                        return Ok(Step::Wait);
                    }
                    if first.2 == Priority::Background && reg.leases.iter().any(|l| l.pid != self.pid && n.saturating_sub(l.last_used) < secs(t.hold)) {
                        return Ok(Step::Wait);
                    }
                    if slots_busy(&s.url, t) {
                        reg.last_activity = n;
                        return Ok(Step::Wait);
                    }
                    stop_server(&s, t.stop_grace);
                    reg.server = None;
                    Some(s.model)
                }
            };
            match self.launch(layout, m, from) {
                Ok(s) => {
                    let step = Step::Started { url: s.url.clone(), pid: s.pid, started_at: s.started_at.clone() };
                    reg.server = Some(s);
                    reg.last_activity = n;
                    Ok(step)
                }
                Err(e) => {
                    reg.wanting.retain(|w| !(w.pid == self.pid && w.model == m.id));
                    Err(e)
                }
            }
        });
        res.map_err(|e| format!("無法讀寫 {}：{e}", self.registry_path().display()))?
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(self.log_path()).unwrap_or_default();
        let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).rev().take(3).collect();
        tail.into_iter().rev().collect::<Vec<_>>().join(" ")
    }

    /// Wait for a server this process started; the ticket once it answers.
    fn await_start(self: &Arc<Self>, m: &Model, priority: Priority, url: &str, pid: u32, started_at: &str) -> Result<Ticket, String> {
        let deadline = Instant::now() + self.timing.start_wait;
        loop {
            let up = health(url, &self.timing) == Some(true);
            let dead = !up && !pid_alive(pid);
            if up || dead || Instant::now() >= deadline {
                let forget = |reg: &mut Registry| {
                    if reg.server.as_ref().is_some_and(|s| s.pid == pid) {
                        reg.server = None;
                    }
                    reg.wanting.retain(|w| !(w.pid == self.pid && w.model == m.id));
                };
                if dead {
                    let _ = self.locked(false, forget);
                    return Err(format!("llama-server 結束了：{}", self.log_tail()));
                }
                if !up {
                    let _ = self.locked(false, |reg| {
                        if let Some(s) = reg.server.as_ref().filter(|s| s.pid == pid) {
                            stop_server(s, self.timing.stop_grace);
                        }
                        forget(reg);
                    });
                    return Err(format!("{} 啟動逾時", m.name));
                }
                let ok = self
                    .locked(false, |reg| {
                        let n = now();
                        let Some(s) = reg.server.as_mut().filter(|s| s.pid == pid && s.started_at == started_at) else { return None };
                        s.loading = false;
                        let vision = s.vision;
                        reg.wanting.retain(|w| !(w.pid == self.pid && w.model == m.id));
                        let l = self.lease_mut(reg, priority);
                        l.active += 1;
                        l.last_used = n;
                        reg.last_activity = n;
                        Some(vision)
                    })
                    .map_err(|e| e.to_string())?;
                return match ok {
                    Some(vision) => Ok(Ticket { rt: self.clone(), url: url.to_string(), model: m.id.clone(), vision, done: false }),
                    None => Err("llama-server 啟動後就不見了".into()),
                };
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Blocking [`SharedRuntime::acquire`].
    /// `layout`: where llama-server and the weights are (`None`: this app can only use a
    /// server another app started).
    pub fn acquire_blocking(self: &Arc<Self>, layout: Option<&Layout>, m: &Model, priority: Priority) -> Result<Ticket, String> {
        let mut pause = self.timing.poll_min;
        loop {
            match self.step(layout, m, priority)? {
                Step::Ready { url, model, vision } => return Ok(Ticket { rt: self.clone(), url, model, vision, done: false }),
                Step::Started { url, pid, started_at } => return self.await_start(m, priority, &url, pid, &started_at),
                Step::Wait => {
                    std::thread::sleep(pause);
                    pause = (pause + pause / 2).min(self.timing.poll_max);
                }
            }
        }
    }

    /// The server running `m` for one request: the loaded one, one started now, or after
    /// waiting for the model to change (docs/local-models.md §3). Keep the [`Ticket`] while
    /// the request runs.
    pub async fn acquire(self: &Arc<Self>, layout: Option<&Layout>, m: &Model, priority: Priority) -> Result<Ticket, String> {
        self.rearm();
        let (rt, layout, m) = (self.clone(), layout.cloned(), m.clone());
        tokio::task::spawn_blocking(move || rt.acquire_blocking(layout.as_ref(), &m, priority)).await.map_err(|e| e.to_string())?
    }

    /// An image tool (the upscaler) takes the runtime: waits until no model request runs and
    /// no other tool holds it, then registers; until the [`ToolTicket`] drops, requests for
    /// the local model wait (the loaded model stays in memory, the tool is small). `cancel`
    /// is polled while waiting. Blocking.
    pub fn begin_tool_blocking(self: &Arc<Self>, tool: &str, cancel: &dyn Fn() -> bool) -> Result<ToolTicket, String> {
        let t = &self.timing;
        let mut pause = t.poll_min;
        loop {
            if cancel() {
                return Err("已取消".into());
            }
            let got = self
                .locked(true, |reg| {
                    let n = now();
                    if !reg.tools.is_empty() {
                        return false;
                    }
                    if let Some(s) = reg.server.as_ref() {
                        if s.loading || reg.leases.iter().any(|l| l.active > 0) {
                            return false;
                        }
                        if slots_busy(&s.url, t) {
                            reg.last_activity = n;
                            return false;
                        }
                    }
                    reg.tools.push(ToolUse { app: self.app.clone(), pid: self.pid, tool: tool.to_string(), since: n });
                    reg.last_activity = n;
                    true
                })
                .map_err(|e| format!("無法讀寫 {}：{e}", self.registry_path().display()))?;
            if got {
                return Ok(ToolTicket { rt: self.clone(), tool: tool.to_string() });
            }
            std::thread::sleep(pause);
            pause = (pause + pause / 2).min(t.poll_max);
        }
    }

    /// A request ended: `active` down, `last_used` and `last_activity` now.
    fn finished(&self) {
        let _ = self.locked(false, |reg| {
            let n = now();
            if let Some(l) = reg.leases.iter_mut().find(|l| l.pid == self.pid) {
                l.active = l.active.saturating_sub(1);
                l.last_used = n;
            }
            reg.last_activity = n;
        });
    }

    /// The registry and this app's status, pruned. Blocking.
    pub fn snapshot_blocking(&self) -> Snapshot {
        let reg = self.locked(true, |reg| reg.clone()).unwrap_or_default();
        let status = status_of(&reg, self.pid, now());
        let can_release = reg.server.is_some() && !reg.leases.iter().any(|l| l.pid != self.pid);
        Snapshot { registry: reg, status, can_release }
    }

    pub async fn snapshot(self: &Arc<Self>) -> Snapshot {
        let rt = self.clone();
        tokio::task::spawn_blocking(move || rt.snapshot_blocking()).await.unwrap_or(Snapshot {
            registry: Registry::default(),
            status: LocalStatus::Off,
            can_release: false,
        })
    }

    /// Stop the server when nothing runs on it and it has been unused for `idle` (any app
    /// does this; also an orphan whose apps all died). Returns whether it stopped one.
    pub fn check_idle_blocking(&self, idle: Option<Duration>) -> bool {
        let Some(idle) = idle else { return false };
        let t = &self.timing;
        self.locked(true, |reg| {
            let n = now();
            let Some(s) = reg.server.clone() else { return false };
            if s.loading || !reg.tools.is_empty() || reg.leases.iter().any(|l| l.active > 0) || !reg.wanting.is_empty() {
                return false;
            }
            if slots_busy(&s.url, t) {
                reg.last_activity = n;
                return false;
            }
            let last = reg.last_activity.max(s.since);
            if n.saturating_sub(last) < secs(idle) {
                return false;
            }
            eprintln!("lore-llm: local model {} unused for {} s, released", s.model, n.saturating_sub(last));
            stop_server(&s, t.stop_grace);
            reg.server = None;
            true
        })
        .unwrap_or(false)
    }

    pub async fn check_idle(self: &Arc<Self>, idle: Option<Duration>) -> bool {
        let rt = self.clone();
        tokio::task::spawn_blocking(move || rt.check_idle_blocking(idle)).await.unwrap_or(false)
    }

    /// Check for an idle server now and every [`Timing::idle_tick`] (once per runtime; needs
    /// a tokio runtime, else `false`).
    /// The source is kept: when there is no runtime yet, the next [`SharedRuntime::acquire`]
    /// starts the watcher.
    pub fn watch_idle(self: &Arc<Self>, idle: IdleSource) -> bool {
        *self.idle.lock().unwrap_or_else(|e| e.into_inner()) = Some(idle.clone());
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return false };
        if self.watching.swap(true, Ordering::SeqCst) {
            return true;
        }
        let rt = self.clone();
        handle.spawn(async move {
            loop {
                rt.check_idle(idle()).await;
                tokio::time::sleep(rt.timing.idle_tick).await;
            }
        });
        true
    }

    /// Start the idle watcher when a source was given before a runtime existed.
    fn rearm(self: &Arc<Self>) {
        if self.watching.load(Ordering::SeqCst) {
            return;
        }
        let src = self.idle.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(src) = src {
            self.watch_idle(src);
        }
    }

    /// The 釋放 button: stop the server now, unless another app holds a lease (then why).
    pub fn release_blocking(&self) -> Result<(), String> {
        let t = &self.timing;
        self.locked(true, |reg| {
            if let Some(l) = reg.leases.iter().find(|l| l.pid != self.pid) {
                return Err(format!("{} 正在用", app_label(&l.app)));
            }
            if let Some(s) = reg.server.take() {
                stop_server(&s, t.stop_grace);
            }
            reg.leases.retain(|l| l.pid != self.pid);
            Ok(())
        })
        .map_err(|e| e.to_string())?
    }

    pub async fn release(self: &Arc<Self>) -> Result<(), String> {
        let rt = self.clone();
        tokio::task::spawn_blocking(move || rt.release_blocking()).await.map_err(|e| e.to_string())?
    }

    /// At quit: drop this process's lease and wants; stop the server when this process
    /// used or started it and no other app holds a lease or waits.
    pub fn shutdown(&self) {
        let t = &self.timing;
        let _ = self.locked(true, |reg| {
            let mine = reg.leases.iter().any(|l| l.pid == self.pid) || reg.server.as_ref().is_some_and(|s| s.started_by.pid == self.pid);
            reg.leases.retain(|l| l.pid != self.pid);
            reg.wanting.retain(|w| w.pid != self.pid);
            reg.tools.retain(|u| u.pid != self.pid);
            if mine && reg.leases.is_empty() && reg.wanting.is_empty() {
                if let Some(s) = reg.server.take() {
                    stop_server(&s, t.stop_grace);
                }
            }
        });
    }
}

// ── the router's provider ───────────────────────────────

/// A `local:` model through the shared server: acquired per request.
pub struct SharedLocal {
    pub rt: Arc<SharedRuntime>,
    /// Where llama-server and the weights are (`None`: only a server another app started).
    pub layout: Option<Layout>,
    pub model: Model,
    pub priority: Priority,
    /// Rewrite Chinese replies in Taiwan usage.
    pub zh_tw: bool,
    /// The model reads images here (a projector is on disk).
    pub vision: bool,
}

impl SharedLocal {
    fn engine(&self, url: &str) -> OpenAiCompat {
        let mut c = OpenAiCompatConfig::llama_server(url.to_string(), self.model.id.clone());
        c.vision = self.vision;
        c.context = self.model.context_or_default();
        c.zh_tw = self.zh_tw;
        OpenAiCompat::new(c)
    }

    async fn ticket(&self) -> Result<Ticket, LlmError> {
        self.rt.acquire(self.layout.as_ref(), &self.model, self.priority).await.map_err(|e| LlmError::Api("llama-server".into(), 503, e))
    }
}

#[async_trait]
impl LlmProvider for SharedLocal {
    fn id(&self) -> String {
        format!("llama-server:{}", self.model.id)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { json: true, vision: self.vision, web: false, streaming: true, context: self.model.context_or_default(), needs_network: false }
    }

    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let ticket = self.ticket().await?;
        match self.engine(&ticket.url).send(req).await {
            // the server died under the request: once more (it is started again)
            Err(LlmError::Network(..)) => {
                drop(ticket);
                let ticket = self.ticket().await?;
                self.engine(&ticket.url).send(req).await
            }
            other => other,
        }
    }

    async fn stream(&self, req: &LlmRequest, on: OnEvent<'_>) -> Result<LlmResponse, LlmError> {
        let ticket = self.ticket().await?;
        let said = AtomicBool::new(false);
        let watch = |e| {
            said.store(true, Ordering::Relaxed);
            on(e)
        };
        match self.engine(&ticket.url).stream(req, &watch).await {
            Err(LlmError::Network(..)) if !said.load(Ordering::Relaxed) => {
                drop(ticket);
                let ticket = self.ticket().await?;
                self.engine(&ticket.url).stream(req, on).await
            }
            other => other,
        }
    }
}

/// The local model `local:current` means for a task: the shared default (`model`), or for
/// a task that reads images when it cannot, `vision_model`, else the smallest installed
/// local model that reads images. `Err` = why there is none.
pub fn current_model<'a>(
    catalog: &'a Catalog,
    layout: Option<&Layout>,
    default: Option<&str>,
    vision_model: Option<&str>,
    needs_vision: bool,
    include_uncensored: bool,
) -> Result<&'a Model, String> {
    let sees = |m: &Model| m.vision && layout.is_none_or(|l| l.find_mmproj(m).is_some());
    let installed = |m: &Model| layout.is_some_and(|l| l.find_model(m).is_some() && l.find_runtime().is_some());
    let pick = default.map(str::trim).filter(|d| !d.is_empty()).ok_or("還沒選本機模型（齒輪 → 模型）")?;
    let m = catalog.resolve_local(pick, |m| layout.is_some_and(|l| l.find_model(m).is_some())).ok_or_else(|| format!("模型目錄裡沒有本機模型「{pick}」"))?;
    if !needs_vision || sees(m) {
        return Ok(m);
    }
    if let Some(v) = vision_model.map(str::trim).filter(|v| !v.is_empty())
        && let Some(vm) = catalog.get(v).filter(|x| x.is_brain())
        && sees(vm)
    {
        return Ok(vm);
    }
    let mut seeing: Vec<&Model> = catalog.brains(include_uncensored).filter(|x| sees(x) && installed(x)).collect();
    seeing.sort_by(|a, b| a.approx_gb.total_cmp(&b.approx_gb));
    seeing.into_iter().next().ok_or_else(|| format!("{} 不能看圖，本機也沒有能看圖的模型", m.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(pid: u32, model: &str, p: Priority, since: u64) -> Want {
        Want { app: format!("a{pid}"), pid, model: model.into(), priority: p, since }
    }

    /// The order of the retired broker's scheduler: interactive first, then first come;
    /// a background wanter is promoted after `promote_after`; the loaded model is no line.
    #[test]
    fn interactive_first_then_first_come_and_promotion() {
        let t = Timing { promote_after: Duration::from_secs(100), ..Timing::default() };
        let mut r = Registry {
            wanting: vec![want(1, "b", Priority::Background, 1000), want(2, "c", Priority::Interactive, 1050), want(3, "d", Priority::Interactive, 1040)],
            ..Default::default()
        };
        let pids = |r: &Registry, n| r.line(n, &t).iter().map(|w| w.pid).collect::<Vec<_>>();
        assert_eq!(pids(&r, 1060), vec![3, 2, 1]);
        assert_eq!(pids(&r, 1100), vec![1, 3, 2], "promoted after waiting 100 s, then oldest first");
        r.server = Some(Server {
            model: "d".into(),
            url: String::new(),
            pid: 9,
            started_by: Who { app: "x".into(), pid: 9 },
            since: 0,
            started_at: String::new(),
            vision: false,
            loading: false,
            from: None,
        });
        assert_eq!(pids(&r, 1060), vec![2, 1], "a wanter of the loaded model is not in line");
    }

    #[test]
    fn status_follows_the_registry() {
        let server = |loading, from: Option<&str>| Server {
            model: "a".into(),
            url: String::new(),
            pid: 9,
            started_by: Who { app: "pc".into(), pid: 8 },
            since: 100,
            started_at: String::new(),
            vision: false,
            loading,
            from: from.map(String::from),
        };
        let mut r = Registry::default();
        assert_eq!(status_of(&r, 1, 0), LocalStatus::Off);
        r.server = Some(server(true, None));
        assert_eq!(status_of(&r, 1, 107), LocalStatus::Loading { model: "a".into(), secs: 7 });
        r.server = Some(server(true, Some("b")));
        assert!(matches!(status_of(&r, 1, 103), LocalStatus::Switching { secs: 3, .. }));
        r.server = Some(server(false, None));
        assert_eq!(status_of(&r, 1, 0), LocalStatus::Loaded { model: "a".into() });
        r.wanting.push(want(1, "b", Priority::Interactive, 0));
        assert_eq!(status_of(&r, 1, 0), LocalStatus::Waiting { app: "pc".into(), model: "b".into() }, "nobody else: the starter");
        r.leases.push(Lease { app: "le".into(), pid: 5, since: 0, last_used: 50, priority: Priority::Interactive, active: 1 });
        r.leases.push(Lease { app: "sc".into(), pid: 6, since: 0, last_used: 90, priority: Priority::Background, active: 0 });
        assert_eq!(status_of(&r, 1, 0), LocalStatus::Waiting { app: "le".into(), model: "b".into() }, "the one running a request");
        assert_eq!(status_of(&r, 2, 0), LocalStatus::Loaded { model: "a".into() }, "another app does not wait");
    }

    #[test]
    fn status_words_are_the_same_everywhere() {
        let c = Catalog::bundled();
        assert_eq!(LocalStatus::Off.text(&c), "未載入");
        assert_eq!(LocalStatus::Loading { model: "qwen3.5-4b-q8".into(), secs: 12 }.text(&c), "載入模型中 · 12 秒");
        assert_eq!(LocalStatus::Switching { model: "a".into(), from: "b".into(), secs: 3 }.text(&c), "切換模型中 · 3 秒");
        assert_eq!(LocalStatus::Waiting { app: "pc".into(), model: "a".into() }.text(&c), "等待 PaperComposer 用完模型");
        assert_eq!(LocalStatus::Loaded { model: "qwen3.5-4b-q8".into() }.text(&c), "已載入 Qwen3.5 4B");
    }

    #[test]
    fn class_defaults_and_registry_format() {
        assert_eq!(Priority::for_class(Some(TaskClass::Ask)), Priority::Interactive);
        assert_eq!(Priority::for_class(Some(TaskClass::Create)), Priority::Background);
        assert_eq!(Priority::for_class(Some(TaskClass::Tidy)), Priority::Background);
        let r: Registry = serde_json::from_str(
            r#"{"server":{"model":"m","url":"http://127.0.0.1:1/v1","pid":4,"started_by":{"app":"pc","pid":3},"since":5,"started_at":"Wed Nov 11 20:15:02 2026","vision":false},
               "leases":[{"app":"le","pid":3,"since":1,"last_used":2,"priority":"interactive"}],"wanting":[],"last_activity":2}"#,
        )
        .unwrap();
        assert!(!r.server.as_ref().unwrap().loading && r.leases[0].active == 0);
        let back = serde_json::to_string(&r).unwrap();
        assert!(!back.contains("\"loading\"") && !back.contains("\"active\"") && !back.contains("\"from\""), "{back}");
    }

    /// A pid that is not a llama-server (this test) is never verified, so never signalled.
    #[test]
    fn only_a_verified_llama_server_is_signalled() {
        let me = std::process::id();
        let (lstart, comm) = ps_info(me).expect("ps answers for this process");
        assert_eq!(lstart.split_whitespace().count(), 5, "{lstart}");
        assert!(!comm.is_empty());
        assert!(!verified(me, &lstart), "not llama-server");
        assert!(!verified(me, "Thu Jan  1 00:00:00 1970"));
        assert!(pid_alive(me) && !pid_alive(0));
    }

    #[test]
    fn current_model_picks_the_default_or_a_vision_model() {
        let dir = tempfile::tempdir().unwrap();
        let l = Layout::shared(dir.path());
        let c = Catalog::bundled();
        let put = |p: PathBuf| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        };
        put(dir.path().join("runtimes/llama.cpp/llama-server"));
        let q = c.get("qwen3.5-4b-q8").unwrap();
        put(dir.path().join("gguf").join(&q.repo).join(&q.file_suffix));
        assert!(current_model(&c, Some(&l), None, None, false, false).unwrap_err().contains("還沒選"));
        assert_eq!(current_model(&c, Some(&l), Some("qwen3.5-4b-q8"), None, false, false).unwrap().id, "qwen3.5-4b-q8");
        let e = current_model(&c, Some(&l), Some("qwen3.5-4b-q8"), None, true, false).unwrap_err();
        assert!(e.contains("不能看圖"), "no projector on disk: {e}");
        put(dir.path().join("gguf").join(&q.repo).join(q.mmproj_suffix.as_deref().unwrap()));
        assert_eq!(current_model(&c, Some(&l), Some("qwen3.5-4b-q8"), None, true, false).unwrap().id, "qwen3.5-4b-q8", "it sees now");
    }

    /// An image tool holds the runtime alone; a second one waits (and can be cancelled);
    /// the language model's acquire waits while it runs; dropping the ticket frees it.
    #[test]
    fn image_tool_takes_the_slot() {
        let dir = std::env::temp_dir().join(format!("lore-llm-tool-{}", std::process::id()));
        let rt = SharedRuntime::new("sc", &dir, Timing { poll_min: Duration::from_millis(10), poll_max: Duration::from_millis(20), ..Timing::default() });
        let t1 = rt.begin_tool_blocking("upscale", &|| false).unwrap();
        assert_eq!(rt.snapshot_blocking().registry.tools.len(), 1);
        let cancelled = AtomicBool::new(false);
        let waiter = std::thread::scope(|sc| {
            let h = sc.spawn(|| rt.begin_tool_blocking("upscale", &|| cancelled.load(Ordering::SeqCst)).err());
            std::thread::sleep(Duration::from_millis(80));
            cancelled.store(true, Ordering::SeqCst);
            h.join().unwrap()
        });
        assert_eq!(waiter.as_deref(), Some("已取消"));
        let m = Catalog::bundled().get("qwen3.5-4b-q8").unwrap().clone();
        assert!(matches!(rt.step(None, &m, Priority::Interactive), Ok(Step::Wait)), "the model yields");
        assert!(matches!(rt.snapshot_blocking().status, LocalStatus::Waiting { .. }));
        drop(t1);
        assert!(rt.snapshot_blocking().registry.tools.is_empty());
        assert!(rt.begin_tool_blocking("upscale", &|| false).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
