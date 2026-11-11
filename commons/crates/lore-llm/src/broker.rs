//! The client side of the shared model broker (`lore-modeld`, design companion §8).
//!
//! One small background process per machine owns the local llama-server; every app
//! sends its local-model requests there (the same OpenAI-compatible interface) with two
//! headers, and the broker decides who goes first:
//!
//! - [`APP_HEADER`] `X-Lore-App: pc|sc|lr|kb|…` — who asks (any short name).
//! - [`PRIORITY_HEADER`] `X-Lore-Priority: interactive|background` — missing = background.
//!
//! The broker records `{port, pid}` in `<cache>/modeld.json` ([`state_path`], the same
//! cache folder as lore-home's `ports.json`; `LOREFORGE_CACHE` moves it). [`find`] reads
//! it and checks that the process answers `GET /api/status`; [`ensure`] starts the broker
//! when none runs (binary from [`binary`]). The router uses it through
//! [`BrokerAccess`] in `LocalAccess.broker`: `local:<model>` then becomes an
//! [`OpenAiCompat`] pointed at the broker with the two headers. [`focus`] tells the broker
//! which app's window is in front.

use crate::openai::{OpenAiCompat, OpenAiCompatConfig};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `<cache>/modeld.json`: `{ "port": 51234, "pid": 4321 }` of the running broker.
pub const FILE: &str = "modeld.json";
/// Request header naming the app (`pc`, `sc`, `lr`, `kb`, …).
pub const APP_HEADER: &str = "X-Lore-App";
/// Request header with the [`Priority`].
pub const PRIORITY_HEADER: &str = "X-Lore-Priority";
/// What `GET /api/status` answers in `service`.
pub const SERVICE: &str = "lore-modeld";
/// Environment variable naming the broker binary.
pub const BINARY_ENV: &str = "LORE_MODELD";

/// How urgent a request is. Interactive: someone is waiting for it (chat, voice).
/// Background: batch work (tidying, translation, batch review).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
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

    /// The header value; anything but `interactive` (or nothing) is background.
    pub fn from_header(v: Option<&str>) -> Priority {
        match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("interactive") => Priority::Interactive,
            _ => Priority::Background,
        }
    }
}

/// `<cache>/modeld.json` (`LOREFORGE_CACHE` moves the folder).
pub fn state_path() -> Option<PathBuf> {
    lore_home::ports::cache_dir().map(|d| d.join(FILE))
}

/// `(port, pid)` from the state file; `None` when missing or broken.
pub fn read_state(path: &Path) -> Option<(u16, u32)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let port = u16::try_from(v.get("port")?.as_u64()?).ok().filter(|p| *p != 0)?;
    let pid = u32::try_from(v.get("pid")?.as_u64()?).ok()?;
    Some((port, pid))
}

/// Write the state file (atomically).
pub fn write_state(path: &Path, port: u16, pid: u32) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{pid}"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&json!({ "port": port, "pid": pid }))?)?;
    std::fs::rename(&tmp, path)
}

fn http(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder().timeout(timeout).build().expect("http client")
}

/// `GET <base>/api/status` (`base` = `http://127.0.0.1:<port>`).
pub async fn status(base: &str) -> Result<Value, String> {
    let r = http(Duration::from_secs(3))
        .get(format!("{}/api/status", base.trim_end_matches('/')))
        .send()
        .await
        .map_err(|e| format!("模型管家沒有回應：{e}"))?;
    if !r.status().is_success() {
        return Err(format!("模型管家回應 {}", r.status()));
    }
    r.json().await.map_err(|e| format!("模型管家的回應看不懂：{e}"))
}

/// The broker named by the state file at `path`, when its process lives and answers.
pub async fn find_at(path: &Path) -> Option<String> {
    let (port, pid) = read_state(path)?;
    if !lore_home::ports::pid_alive(pid) {
        return None;
    }
    let base = format!("http://127.0.0.1:{port}");
    let st = status(&base).await.ok()?;
    (st["service"] == SERVICE).then_some(base)
}

/// The running broker (`http://127.0.0.1:<port>`), if any.
pub async fn find() -> Option<String> {
    find_at(&state_path()?).await
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "lore-modeld.exe" } else { "lore-modeld" }
}

/// The broker binary: `LORE_MODELD`, else next to the current executable (or its
/// parent folder, for test binaries in `target/*/deps`), else on `PATH`.
pub fn binary() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os(BINARY_ENV).filter(|v| !v.is_empty()).map(PathBuf::from) {
        return if p.is_file() { Ok(p) } else { Err(format!("{BINARY_ENV} 指到的 {} 不存在", p.display())) };
    }
    let near = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = near {
        if d.file_name().is_some_and(|n| n == "deps")
            && let Some(up) = d.parent()
        {
            dirs.push(up.to_path_buf());
        }
        dirs.insert(0, d);
    }
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    dirs.into_iter().map(|d| d.join(exe_name())).find(|p| p.is_file()).ok_or_else(|| {
        format!("找不到 lore-modeld（共用模型管家）：請設定 {BINARY_ENV}，或把 lore-modeld 放在 App 旁邊或 PATH 裡（commons 裡 cargo build -p lore-modeld）")
    })
}

/// How to start the broker when none runs.
#[derive(Debug, Clone)]
pub struct Launch {
    /// `None`: [`binary`].
    pub binary: Option<PathBuf>,
    /// Extra arguments (`--gguf <dir> --runtime <dir>` for an app's own model folders,
    /// `--home <LoreForge folder>`).
    pub args: Vec<String>,
    /// How long to wait for a started broker to answer.
    pub wait: Duration,
    /// `None`: [`state_path`].
    pub state: Option<PathBuf>,
}

impl Default for Launch {
    fn default() -> Self {
        Launch { binary: None, args: Vec::new(), wait: Duration::from_secs(15), state: None }
    }
}

impl Launch {
    fn state(&self) -> Result<PathBuf, String> {
        self.state.clone().or_else(state_path).ok_or_else(|| "找不到快取資料夾（模型管家的 modeld.json）".to_string())
    }

    fn binary(&self) -> Result<PathBuf, String> {
        match &self.binary {
            Some(b) if b.is_file() => Ok(b.clone()),
            Some(b) => Err(format!("找不到 lore-modeld：{}", b.display())),
            None => binary(),
        }
    }
}

/// The running broker, started (detached, so it outlives this app) when none runs.
pub async fn ensure(launch: &Launch) -> Result<String, String> {
    let state = launch.state()?;
    if let Some(base) = find_at(&state).await {
        return Ok(base);
    }
    let bin = launch.binary()?;
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(&launch.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // not part of the app's group: survives its quit and its ^C
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0200); // no window, own process group
    }
    let mut child = cmd.spawn().map_err(|e| format!("無法啟動 lore-modeld（{}）：{e}", bin.display()))?;
    let deadline = tokio::time::Instant::now() + launch.wait;
    loop {
        if let Some(base) = find_at(&state).await {
            // reap it when it ends, so it does not linger as a zombie of this app
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(base);
        }
        if let Ok(Some(st)) = child.try_wait()
            && !st.success()
        {
            return Err(format!("lore-modeld 啟動失敗（{st}）"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("lore-modeld 啟動逾時".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Tell the broker that `app`'s window is in front now (`None`: no LoreForge window is).
pub async fn focus(base: &str, app: Option<&str>) -> Result<(), String> {
    let r = http(Duration::from_secs(3))
        .post(format!("{}/api/focus", base.trim_end_matches('/')))
        .json(&json!({ "app": app }))
        .send()
        .await
        .map_err(|e| format!("模型管家沒有回應：{e}"))?;
    if r.status().is_success() { Ok(()) } else { Err(format!("模型管家回應 {}", r.status())) }
}

/// [`focus`] on the running broker; nothing happens when none runs (apps call it on
/// every focus change, which must not start the broker).
pub async fn focus_running(app: Option<&str>) {
    if let Some(base) = find().await {
        let _ = focus(&base, app).await;
    }
}

/// Route local models through the broker (`LocalAccess.broker`).
#[derive(Debug, Clone)]
pub struct BrokerAccess {
    /// The app's short name ([`APP_HEADER`]).
    pub app: String,
    pub priority: Priority,
    /// A fixed broker URL (`http://127.0.0.1:<port>`); `None`: find or start it.
    pub url: Option<String>,
    pub launch: Launch,
}

impl BrokerAccess {
    pub fn new(app: impl Into<String>, priority: Priority) -> BrokerAccess {
        BrokerAccess { app: app.into(), priority, url: None, launch: Launch::default() }
    }

    /// The broker's base URL: the fixed one, else the running one, else a started one.
    pub async fn base_url(&self) -> Result<String, String> {
        match &self.url {
            Some(u) => Ok(u.trim_end_matches('/').to_string()),
            None => ensure(&self.launch).await,
        }
    }

    /// The running broker's URL without starting one.
    pub async fn running_url(&self) -> Option<String> {
        match &self.url {
            Some(u) => Some(u.trim_end_matches('/').to_string()),
            None => find_at(&self.launch.state().ok()?).await,
        }
    }

    /// Could a request get through: a fixed URL, a running broker or a binary to start.
    pub async fn available(&self) -> Result<(), String> {
        if self.running_url().await.is_some() {
            return Ok(());
        }
        self.launch.binary().map(|_| ())
    }

    /// The two headers of every request.
    pub fn headers(&self) -> Vec<(String, String)> {
        vec![(APP_HEADER.into(), self.app.clone()), (PRIORITY_HEADER.into(), self.priority.as_str().into())]
    }

    /// An OpenAI-compatible config for `model` (a catalog id) through the broker at `base`.
    pub fn config(&self, base: &str, model: &str) -> OpenAiCompatConfig {
        let mut c = OpenAiCompatConfig::llama_server(format!("{}/v1", base.trim_end_matches('/')), model);
        c.label = "lore-modeld".into();
        c.headers = self.headers();
        c
    }

    /// [`BrokerAccess::config`] as a provider.
    pub fn provider(&self, base: &str, model: &str) -> OpenAiCompat {
        OpenAiCompat::new(self.config(base, model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::LlmProvider;
    use crate::types::LlmRequest;
    use axum::{Json, Router, routing::get, routing::post};
    use std::sync::{Arc, Mutex};

    async fn serve(app: Router) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[test]
    fn priority_header_values() {
        assert_eq!(Priority::from_header(Some("interactive")), Priority::Interactive);
        assert_eq!(Priority::from_header(Some(" Interactive ")), Priority::Interactive);
        assert_eq!(Priority::from_header(Some("background")), Priority::Background);
        assert_eq!(Priority::from_header(Some("urgent")), Priority::Background, "unknown = background");
        assert_eq!(Priority::from_header(None), Priority::Background, "missing = background");
        assert_eq!(serde_json::to_value(Priority::Interactive).unwrap(), "interactive");
    }

    #[test]
    fn state_file_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sub").join(FILE);
        assert!(read_state(&f).is_none());
        write_state(&f, 51234, 42).unwrap();
        assert_eq!(read_state(&f), Some((51234, 42)));
        std::fs::write(&f, "{\"port\": 0, \"pid\": 1}").unwrap();
        assert!(read_state(&f).is_none());
        std::fs::write(&f, "nope").unwrap();
        assert!(read_state(&f).is_none());
    }

    /// The client finds a live broker that answers, ignores a dead pid or another service.
    #[tokio::test]
    async fn find_needs_a_live_pid_and_an_answer() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(FILE);
        let good = serve(Router::new().route("/api/status", get(|| async { Json(json!({ "service": SERVICE })) }))).await;
        let other = serve(Router::new().route("/api/status", get(|| async { Json(json!({ "service": "else" })) }))).await;
        let port = |u: &str| u.rsplit(':').next().unwrap().parse::<u16>().unwrap();
        write_state(&f, port(&good), std::process::id()).unwrap();
        assert_eq!(find_at(&f).await.as_deref(), Some(good.as_str()));
        write_state(&f, port(&good), u32::MAX - 1).unwrap();
        assert!(find_at(&f).await.is_none(), "dead pid");
        write_state(&f, port(&other), std::process::id()).unwrap();
        assert!(find_at(&f).await.is_none(), "not the broker");
        // Launch with a missing binary: a clear error, nothing started.
        let l = Launch { binary: Some(d.path().join("nope")), state: Some(f.clone()), ..Launch::default() };
        let e = ensure(&l).await.unwrap_err();
        assert!(e.contains("找不到 lore-modeld"), "{e}");
        // ... but a running broker is found without a binary.
        write_state(&f, port(&good), std::process::id()).unwrap();
        assert_eq!(ensure(&l).await.unwrap(), good);
    }

    /// Requests through the broker carry the app and priority headers; focus posts the app.
    #[tokio::test]
    async fn requests_carry_the_headers_and_focus_posts_the_app() {
        let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
        let s1 = seen.clone();
        let s2 = seen.clone();
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                post(move |h: axum::http::HeaderMap, Json(b): Json<Value>| {
                    let s = s1.clone();
                    async move {
                        let v = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
                        s.lock().unwrap().push(json!({ "app": v("x-lore-app"), "priority": v("x-lore-priority"), "model": b["model"] }));
                        Json(json!({"choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]}))
                    }
                }),
            )
            .route(
                "/api/focus",
                post(move |Json(b): Json<Value>| {
                    let s = s2.clone();
                    async move {
                        s.lock().unwrap().push(json!({ "focus": b["app"] }));
                        Json(json!({}))
                    }
                }),
            );
        let base = serve(app).await;
        let mut b = BrokerAccess::new("pc", Priority::Interactive);
        b.url = Some(format!("{base}/"));
        assert_eq!(b.base_url().await.unwrap(), base);
        assert!(b.available().await.is_ok());
        let p = b.provider(&base, "qwen3.5-9b-q8");
        assert_eq!(p.id(), "lore-modeld:qwen3.5-9b-q8");
        assert_eq!(p.complete(&LlmRequest::prompt("s", "u")).await.unwrap(), "ok");
        b.priority = Priority::Background;
        b.app = "kb".into();
        b.provider(&base, "gemma-4-12b-q8").complete(&LlmRequest::prompt("s", "u")).await.unwrap();
        focus(&base, Some("sc")).await.unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                json!({ "app": "pc", "priority": "interactive", "model": "qwen3.5-9b-q8" }),
                json!({ "app": "kb", "priority": "background", "model": "gemma-4-12b-q8" }),
                json!({ "focus": "sc" }),
            ]
        );
    }
}
