//! Local models in the shared folder and the llama-server lifecycle
//! (JELearner `local_llm.rs` + PaperComposer `models.rs`).
//!
//! Every app on the machine keeps model weights and llama.cpp in one folder,
//! `LOCAL_LLM_HOME` (or a configured path, since an app opened from Finder
//! does not see shell variables):
//!
//! ```text
//! <home>/gguf/<owner>/<repo>/<file>     what `hf download --local-dir` writes
//! <home>/runtimes/llama.cpp/…/llama-server
//! ```
//!
//! An app may keep its own folders instead ([`Layout::new`]; PaperComposer
//! without `LOCAL_LLM_HOME` uses `<data>/models` and `<data>/llama.cpp`).
//!
//! Nothing here downloads (PaperComposer's setup or `hf download` fill the
//! folder). [`LocalRuntime`] starts one llama-server at a time for one
//! catalog model and stops it on request or drop. No thinking
//! (`--reasoning-budget 0`): every task wants a fixed JSON shape quickly.
//!
//! Real llama-server runs are `#[ignore]` tests; **not tested on real
//! hardware** from this crate (JELearner measured Qwen3.5 4B on an M6 32 GB).

use crate::catalog::{Catalog, Model};
use crate::openai::{OpenAiCompat, OpenAiCompatConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::Mutex;

/// The shared folder: the configured one, else `LOCAL_LLM_HOME`.
pub fn home(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("LOCAL_LLM_HOME").filter(|v| !v.is_empty()).map(PathBuf::from))
}

pub fn gguf_dir(home: &Path) -> PathBuf {
    home.join("gguf")
}

pub fn runtime_dir(home: &Path) -> PathBuf {
    home.join("runtimes").join("llama.cpp")
}

/// Files under `dir` (not in `.cache`, where hf keeps partial downloads)
/// whose path ends with `suffix`, shortest first (the plain file, not a split
/// part or an MTP head in a subfolder).
fn find_under(dir: &Path, suffix: &str) -> Option<PathBuf> {
    let want = suffix.to_lowercase();
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if e.file_name() != ".cache" {
                    stack.push(p);
                }
            } else if p.to_string_lossy().to_lowercase().ends_with(&want) {
                hits.push(p);
            }
        }
    }
    hits.sort_by_key(|p| p.as_os_str().len());
    hits.into_iter().next()
}

/// Where model weights and llama.cpp live. The shared layout is
/// [`Layout::shared`]; an app with its own folder from before
/// `LOCAL_LLM_HOME` (PaperComposer: `<data>/models`, `<data>/llama.cpp`)
/// passes its folders with [`Layout::new`]. Weights are always laid out as
/// `<gguf>/<owner>/<repo>/<file>` (what `hf download --local-dir` writes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Model weights: `<gguf>/<owner>/<repo>/<file>`.
    pub gguf: PathBuf,
    /// llama.cpp: `llama-server` anywhere below this folder.
    pub runtime: PathBuf,
}

impl Layout {
    /// The shared folder: `<home>/gguf`, `<home>/runtimes/llama.cpp`.
    pub fn shared(home: &Path) -> Layout {
        Layout { gguf: gguf_dir(home), runtime: runtime_dir(home) }
    }

    /// Any two folders (an app's own layout).
    pub fn new(gguf: impl Into<PathBuf>, runtime: impl Into<PathBuf>) -> Layout {
        Layout { gguf: gguf.into(), runtime: runtime.into() }
    }

    /// The model's weights, when downloaded.
    pub fn find_model(&self, m: &Model) -> Option<PathBuf> {
        if m.repo.is_empty() || m.file_suffix.is_empty() {
            return None;
        }
        find_under(&self.gguf.join(&m.repo), &m.file_suffix)
    }

    /// The vision projector next to the weights, when the catalog names one.
    pub fn find_mmproj(&self, m: &Model) -> Option<PathBuf> {
        find_under(&self.gguf.join(&m.repo), m.mmproj_suffix.as_deref()?)
    }

    /// `llama-server` below the runtime folder.
    pub fn find_runtime(&self) -> Option<PathBuf> {
        let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
        let mut stack = vec![self.runtime.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if e.file_name() == exe {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Brains of the catalog that are on disk.
    pub fn installed<'a>(&self, cat: &'a Catalog, include_uncensored: bool) -> Vec<&'a Model> {
        cat.brains(include_uncensored).filter(|m| self.find_model(m).is_some()).collect()
    }

    /// Why a model cannot start, in the user's words.
    pub fn missing(&self, m: &Model) -> Option<String> {
        if self.find_runtime().is_none() {
            return Some(format!(
                "還沒有 llama.cpp：{} 裡找不到 llama-server。請用 PaperComposer 的「AI 與同步」安裝一次，或把 llama.cpp 解壓到這個資料夾",
                self.runtime.display()
            ));
        }
        if self.find_model(m).is_none() {
            return Some(format!(
                "還沒下載 {} {}：hf download {} --include \"*{}\" --local-dir \"{}\"",
                m.name,
                m.variant,
                m.repo,
                m.file_suffix,
                self.gguf.join(&m.repo).display()
            ));
        }
        None
    }
}

/// The model's weights in the shared folder, when downloaded.
pub fn find_model(home: &Path, m: &Model) -> Option<PathBuf> {
    Layout::shared(home).find_model(m)
}

/// The vision projector next to the weights, when the catalog names one.
pub fn find_mmproj(home: &Path, m: &Model) -> Option<PathBuf> {
    Layout::shared(home).find_mmproj(m)
}

/// `llama-server` in the shared runtimes folder.
pub fn find_runtime(home: &Path) -> Option<PathBuf> {
    Layout::shared(home).find_runtime()
}

/// Brains of the catalog that are on disk.
pub fn installed<'a>(home: &Path, cat: &'a Catalog, include_uncensored: bool) -> Vec<&'a Model> {
    Layout::shared(home).installed(cat, include_uncensored)
}

/// Why a model cannot start, in the user's words.
pub fn missing(home: &Path, m: &Model) -> Option<String> {
    Layout::shared(home).missing(m)
}

/// llama-server arguments for one model (no thinking, all layers on the GPU,
/// the catalog's context, the projector when there is one).
pub fn server_args(weights: &Path, mmproj: Option<&Path>, port: u16, context: u32) -> Vec<String> {
    let mut a: Vec<String> = vec!["-m".into(), weights.display().to_string()];
    a.extend(
        ["--host", "127.0.0.1", "--port", &port.to_string(), "-c", &context.to_string(), "-ngl", "999", "--jinja", "--reasoning-budget", "0"]
            .map(String::from),
    );
    if let Some(mm) = mmproj {
        a.extend(["--mmproj".into(), mm.display().to_string()]);
    }
    a
}

/// A running llama-server.
#[derive(Debug)]
pub struct Running {
    pub child: Child,
    pub model_id: String,
    /// Base URL ending in `/v1`.
    pub url: String,
    /// Started with a vision projector.
    pub vision: bool,
    pub context: u32,
}

impl Running {
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(8089)
}

/// Start llama-server for `m` and wait until `/v1/models` answers (a large
/// model takes a while to load the first time).
pub async fn start(home: &Path, m: &Model, wait: Duration) -> Result<Running, String> {
    start_in(&Layout::shared(home), m, wait).await
}

/// [`start`] with any [`Layout`].
pub async fn start_in(layout: &Layout, m: &Model, wait: Duration) -> Result<Running, String> {
    if let Some(why) = layout.missing(m) {
        return Err(why);
    }
    let bin = layout.find_runtime().expect("checked");
    let weights = layout.find_model(m).expect("checked");
    let mmproj = layout.find_mmproj(m);
    let port = free_port();
    let context = m.context_or_default();
    let mut cmd = tokio::process::Command::new(&bin);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.args(server_args(&weights, mmproj.as_deref(), port, context))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = bin.parent() {
        cmd.current_dir(dir); // its shared libraries sit next to it
    }
    let mut child = cmd.spawn().map_err(|e| format!("無法啟動 llama-server：{e}"))?;
    let url = format!("http://127.0.0.1:{port}/v1");
    let probe = OpenAiCompat::new(OpenAiCompatConfig::llama_server(url.clone(), m.id.clone()));
    let deadline = tokio::time::Instant::now() + wait;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            let tail: Vec<&str> = err.lines().rev().take(3).collect();
            return Err(format!("llama-server 結束了（{status}）：{}", tail.into_iter().rev().collect::<Vec<_>>().join(" ")));
        }
        if probe.ping().await {
            return Ok(Running { child, model_id: m.id.clone(), url, vision: mmproj.is_some(), context });
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = child.kill().await;
    Err(format!("{} 啟動逾時", m.name))
}

/// Where the local engine is, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Starting { model: String },
    Ready { model: String, url: String },
    Error { message: String },
}

/// One llama-server at a time (two large models at once would overflow GPU
/// memory): [`LocalRuntime::ensure`] reuses the running one when it is the
/// right model, otherwise stops it and starts the new one. Starts are
/// serialized, so a model restored at launch and a pick from a menu do not
/// race. The server is killed when the runtime is dropped.
pub struct LocalRuntime {
    running: Mutex<Option<Running>>,
    phase: std::sync::Mutex<Phase>,
    starting: Mutex<()>,
    pub wait: Duration,
}

impl Default for LocalRuntime {
    fn default() -> Self {
        LocalRuntime {
            running: Mutex::new(None),
            phase: std::sync::Mutex::new(Phase::Idle),
            starting: Mutex::new(()),
            wait: Duration::from_secs(180),
        }
    }
}

impl LocalRuntime {
    pub fn new() -> LocalRuntime {
        LocalRuntime::default()
    }

    pub fn phase(&self) -> Phase {
        self.phase.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, p: Phase) {
        *self.phase.lock().unwrap_or_else(|e| e.into_inner()) = p;
    }

    /// The running server, if alive (for `local:current`).
    pub async fn current_info(&self) -> Option<ServerInfo> {
        let (model_id, url, vision, context) = self.current().await?;
        Some(ServerInfo { model_id, url, vision, context })
    }

    /// `(model id, url, vision, context)` of the running server, if alive.
    pub async fn current(&self) -> Option<(String, String, bool, u32)> {
        let mut r = self.running.lock().await;
        let alive = r.as_mut().map(Running::alive)?;
        if !alive {
            *r = None;
            self.set(Phase::Idle);
            return None;
        }
        r.as_ref().map(|run| (run.model_id.clone(), run.url.clone(), run.vision, run.context))
    }

    /// The `/v1` URL of a server running `m`, starting it if needed.
    pub async fn ensure(&self, home: &Path, m: &Model) -> Result<ServerInfo, String> {
        self.ensure_in(&Layout::shared(home), m).await
    }

    /// [`LocalRuntime::ensure`] with any [`Layout`].
    pub async fn ensure_in(&self, layout: &Layout, m: &Model) -> Result<ServerInfo, String> {
        let _one = self.starting.lock().await;
        if let Some((id, url, vision, context)) = self.current().await
            && id == m.id
        {
            return Ok(ServerInfo { model_id: id, url, vision, context });
        }
        self.stop().await;
        self.set(Phase::Starting { model: m.id.clone() });
        match start_in(layout, m, self.wait).await {
            Ok(run) => {
                let out = ServerInfo { model_id: run.model_id.clone(), url: run.url.clone(), vision: run.vision, context: run.context };
                self.set(Phase::Ready { model: run.model_id.clone(), url: run.url.clone() });
                *self.running.lock().await = Some(run);
                Ok(out)
            }
            Err(e) => {
                self.set(Phase::Error { message: e.clone() });
                Err(e)
            }
        }
    }

    /// Tests: treat an already running process as the server for `model_id`.
    #[cfg(test)]
    pub(crate) async fn adopt(&self, run: Running) {
        self.set(Phase::Ready { model: run.model_id.clone(), url: run.url.clone() });
        *self.running.lock().await = Some(run);
    }

    pub async fn stop(&self) {
        if let Some(mut r) = self.running.lock().await.take() {
            let _ = r.child.kill().await;
        }
        self.set(Phase::Idle);
    }

    /// Synchronous stop for app exit (window closing, signal handler).
    pub fn kill_now(&self) {
        if let Ok(mut r) = self.running.try_lock()
            && let Some(run) = r.as_mut()
        {
            let _ = run.child.start_kill();
        }
    }
}

/// What [`LocalRuntime::ensure`] hands back (the child stays inside the runtime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub model_id: String,
    pub url: String,
    pub vision: bool,
    pub context: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn finds_files_in_the_hf_layout() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").expect("Qwen 4B in the catalog");
        assert!(find_model(home, m).is_none());
        assert!(missing(home, m).unwrap().contains("llama.cpp"));

        touch(&home.join("runtimes/llama.cpp/llama-b1/llama-server"));
        let repo = home.join("gguf").join(&m.repo);
        // A partial download in hf's .cache does not count; the plain file wins over an MTP head.
        touch(&repo.join(".cache/huggingface/download/Qwen3.5-4B-Q8_0.gguf"));
        assert!(missing(home, m).unwrap().contains("hf download"));
        touch(&repo.join("MTP/mtp-Qwen3.5-4B-Q8_0.gguf"));
        touch(&repo.join("Qwen3.5-4B-Q8_0.gguf"));
        touch(&repo.join("mmproj-BF16.gguf"));
        assert_eq!(find_model(home, m).unwrap(), repo.join("Qwen3.5-4B-Q8_0.gguf"));
        assert_eq!(find_mmproj(home, m).unwrap(), repo.join("mmproj-BF16.gguf"));
        assert_eq!(find_runtime(home).unwrap(), home.join("runtimes/llama.cpp/llama-b1/llama-server"));
        assert!(missing(home, m).is_none());
        assert_eq!(installed(home, &cat, false).iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["qwen3.5-4b-q8"]);
    }

    #[test]
    fn an_app_layout_finds_files_in_its_own_folders() {
        // PaperComposer before LOCAL_LLM_HOME: <data>/models/<owner>/<repo>, <data>/llama.cpp.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let l = Layout::new(data.join("models"), data.join("llama.cpp"));
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").unwrap();
        let e = l.missing(m).unwrap();
        assert!(e.contains("llama.cpp"), "{e}");
        touch(&data.join("llama.cpp/build/bin/llama-server"));
        touch(&data.join("models").join(&m.repo).join("Qwen3.5-4B-Q8_0.gguf"));
        assert!(l.missing(m).is_none());
        assert_eq!(l.find_runtime().unwrap(), data.join("llama.cpp/build/bin/llama-server"));
        assert_eq!(l.installed(&cat, true).len(), 1);
        // The shared layout over the same folder sees none of it.
        assert!(find_model(data, m).is_none() && find_runtime(data).is_none());
        assert_eq!(Layout::shared(data), Layout::new(data.join("gguf"), data.join("runtimes/llama.cpp")));
    }

    #[test]
    fn server_args_turn_thinking_off() {
        let a = server_args(Path::new("/m.gguf"), Some(Path::new("/mm.gguf")), 9000, 32768);
        let has = |k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(has("--reasoning-budget", "0") && has("-c", "32768") && has("--port", "9000") && has("--mmproj", "/mm.gguf"));
        assert!(has("--host", "127.0.0.1"), "never listens beyond this machine");
    }

    /// A llama-server that dies at once: the error carries its last words.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_crashing_server_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").unwrap();
        touch(&home.join("gguf").join(&m.repo).join("Qwen3.5-4B-Q8_0.gguf"));
        let bin = home.join("runtimes/llama.cpp/llama-server");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "#!/bin/sh\necho 'error: out of memory' >&2\nexit 3\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rt = LocalRuntime { wait: Duration::from_secs(10), ..LocalRuntime::default() };
        let e = rt.ensure(home, m).await.unwrap_err();
        assert!(e.contains("out of memory"), "{e}");
        assert!(matches!(rt.phase(), Phase::Error { .. }));
        assert!(rt.current().await.is_none());
    }

    /// Real hardware: llama-server and Qwen3.5 4B from the shared folder, one
    /// JSON answer. Run: `cargo test -p lore-llm real_local -- --ignored`.
    #[tokio::test]
    #[ignore = "needs LOCAL_LLM_HOME with llama.cpp and Qwen3.5 4B"]
    async fn real_local_model_answers() {
        use crate::provider::LlmProvider;
        use crate::types::LlmRequest;
        let home = home(None).expect("LOCAL_LLM_HOME");
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").unwrap();
        let rt = LocalRuntime::new();
        let run = rt.ensure(&home, m).await.unwrap();
        let p = OpenAiCompat::new(OpenAiCompatConfig::llama_server(run.url.clone(), m.id.clone()));
        let schema = serde_json::json!({"type":"object","properties":{"word":{"type":"string"}},"required":["word"]});
        let req = LlmRequest::prompt("Reply with JSON only.", "Give one English word that means 'happy'.").max_tokens(200).json(schema);
        let v = crate::json::complete_json(&p, &req).await.unwrap();
        assert!(v["word"].as_str().is_some_and(|w| !w.is_empty()));
        let _ = p.id();
        rt.stop().await;
    }
}
