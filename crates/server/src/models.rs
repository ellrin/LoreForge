//! Local models: catalog, system check, downloads, and the llama.cpp runtime.
//!
//! The catalog is data (models/catalog.json): a new model is a new entry.
//! Files are resolved at download time through the Hugging Face API by
//! suffix, and llama.cpp comes from its latest GitHub release, so neither
//! exact build names nor download links are hard-coded.
//!
//! Nothing downloads without the user first seeing the plan (file names,
//! sizes, sources) and confirming it.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::sync::Mutex;

const CATALOG: &str = include_str!("../../../models/catalog.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub default: String,
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub variant: String,
    #[serde(default)]
    pub summary: String,
    pub repo: String,
    pub file_suffix: String,
    #[serde(default)]
    pub mmproj_suffix: Option<String>,
    pub approx_gb: f32,
    pub min_memory_gb: u32,
    #[serde(default = "default_ctx")]
    pub context: u32,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub vision: bool,
    /// Refusal removed (abliterated). Shown with a tag, never recommended.
    #[serde(default)]
    pub uncensored: bool,
}

fn default_ctx() -> u32 {
    32768
}

/// Bundled catalog, or a user override in the app data folder.
pub fn catalog() -> Catalog {
    let user = data_dir().join("models/catalog.json");
    std::fs::read_to_string(user)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::from_str(CATALOG).expect("bundled catalog is valid"))
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer")
}

/// Shared store for model weights and the llama.cpp runtime, so every project
/// on this machine uses one copy: `LOCAL_LLM_HOME`, then `local_llm_home` in
/// config.toml (apps opened from Finder do not see shell variables).
pub fn llm_home() -> Option<PathBuf> {
    std::env::var_os("LOCAL_LLM_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::read_config().local_llm_home)
}

/// Model files, laid out as `<owner>/<repo>/<file>` like `hf download --local-dir`,
/// so models fetched outside the app are found too.
fn models_dir() -> PathBuf {
    llm_home().map(|h| h.join("gguf")).unwrap_or_else(|| data_dir().join("models"))
}
fn runtime_dir() -> PathBuf {
    llm_home().map(|h| h.join("runtimes/llama.cpp")).unwrap_or_else(|| data_dir().join("llama.cpp"))
}
/// The app's own records (installed manifests, catalog override) stay private to the app.
fn manifests_dir() -> PathBuf {
    data_dir().join("models")
}

// ── System check ────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub memory_gb: f32,
    /// NVIDIA GPU name and memory, when nvidia-smi is available.
    pub gpu: Option<String>,
    pub gpu_memory_gb: Option<f32>,
    /// On Apple silicon the GPU uses system memory.
    pub unified_memory: bool,
}

pub async fn system_info() -> SystemInfo {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let memory_gb = sys.total_memory() as f32 / 1024f32.powi(3);
    let (mut gpu, mut gpu_memory_gb) = (None, None);
    if let Ok(Ok(out)) = tokio::time::timeout(
        Duration::from_secs(5),
        crate::proc::cmd("nvidia-smi").args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]).output(),
    )
    .await
    {
        if out.status.success() {
            let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string();
            if let Some((name, mib)) = line.rsplit_once(',') {
                gpu = Some(name.trim().to_string());
                gpu_memory_gb = mib.trim().parse::<f32>().ok().map(|m| m / 1024.0);
            }
        }
    }
    let unified_memory = std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64";
    SystemInfo { os: std::env::consts::OS, arch: std::env::consts::ARCH, memory_gb, gpu, gpu_memory_gb, unified_memory }
}

/// The default when this machine can hold it, else the most demanding model
/// that fits: `min_memory_gb` tracks the model's tier better than file size
/// (a 4B at Q8 is a bigger file than a 12B at Q4). Uncensored models are
/// never recommended.
pub fn recommend(cat: &Catalog, sys: &SystemInfo) -> Option<String> {
    // Unified memory and discrete GPUs both count; take the larger pool.
    let pool = sys.memory_gb.max(sys.gpu_memory_gb.unwrap_or(0.0));
    let fits: Vec<&ModelEntry> = cat.models.iter().filter(|m| !m.uncensored && pool + 0.5 >= m.min_memory_gb as f32).collect();
    fits.iter()
        .find(|m| m.id == cat.default)
        .or_else(|| fits.iter().max_by(|a, b| a.min_memory_gb.cmp(&b.min_memory_gb).then(a.approx_gb.total_cmp(&b.approx_gb))))
        .map(|m| m.id.clone())
}

// ── Download plan ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedFile {
    pub kind: String, // "model" | "mmproj" | "runtime"
    pub name: String,
    pub url: String,
    pub size: u64,
    pub source: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub model: ModelEntry,
    pub files: Vec<PlannedFile>,
    pub total_to_download: u64,
}

fn hf_base() -> String {
    std::env::var("PAPERCOMPOSER_HF_BASE").unwrap_or_else(|_| "https://huggingface.co".into())
}
fn gh_api() -> String {
    std::env::var("PAPERCOMPOSER_GH_API").unwrap_or_else(|_| "https://api.github.com".into())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("PaperComposer")
        .connect_timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

#[derive(Deserialize)]
struct HfEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    lfs: Option<HfLfs>,
}
#[derive(Deserialize)]
struct HfLfs {
    size: u64,
}

async fn hf_find(repo: &str, suffix: &str) -> Result<PlannedFile, String> {
    let url = format!("{}/api/models/{repo}/tree/main?recursive=true", hf_base());
    let list: Vec<HfEntry> = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("無法連線到 Hugging Face：{e}"))?
        .error_for_status()
        .map_err(|e| format!("Hugging Face 找不到 {repo}：{e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let want = suffix.to_lowercase();
    let mut hits: Vec<&HfEntry> = list.iter().filter(|e| e.kind == "file" && e.path.to_lowercase().ends_with(&want)).collect();
    hits.sort_by_key(|e| e.path.len()); // shortest = the plain variant, not a split part
    let e = hits.first().ok_or_else(|| format!("{repo} 裡沒有以 {suffix} 結尾的檔案"))?;
    // Per-repo folders: different repos ship files with the same name (mmproj-BF16.gguf).
    let name = format!("{repo}/{}", e.path);
    Ok(PlannedFile {
        kind: String::new(),
        installed: models_dir().join(&name).exists(),
        url: format!("{}/{repo}/resolve/main/{}", hf_base(), e.path),
        size: e.lfs.as_ref().map(|l| l.size).unwrap_or(e.size),
        source: format!("huggingface.co/{repo}"),
        name,
    })
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// Asset name fragments to try, best first, for this platform.
fn runtime_patterns(sys: &SystemInfo) -> Vec<&'static str> {
    match (sys.os, sys.arch) {
        // Vulkan runs on NVIDIA, AMD and Intel GPUs without a separate CUDA runtime.
        ("windows", "x86_64") => vec!["bin-win-vulkan-x64", "bin-win-cpu-x64"],
        ("windows", "aarch64") => vec!["bin-win-cpu-arm64"],
        ("macos", "aarch64") => vec!["bin-macos-arm64"],
        ("macos", _) => vec!["bin-macos-x64"],
        ("linux", "x86_64") => vec!["bin-ubuntu-vulkan-x64", "bin-ubuntu-x64"],
        ("linux", "aarch64") => vec!["bin-ubuntu-arm64"],
        _ => vec![],
    }
}

pub fn runtime_binary() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    find_file(&runtime_dir(), exe)
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if e.file_name() == name {
            return Some(p);
        }
    }
    None
}

async fn runtime_asset(sys: &SystemInfo) -> Result<PlannedFile, String> {
    // Binaries ship with the per-build releases (b12345, marked prerelease);
    // the versioned "latest" release (v0.9.0) carries none, so take the
    // newest release that has a build for this platform.
    let releases: Vec<Release> = http()
        .get(format!("{}/repos/ggml-org/llama.cpp/releases?per_page=20", gh_api()))
        .send()
        .await
        .map_err(|e| format!("無法連線到 GitHub：{e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    pick_runtime(&releases, sys).ok_or_else(|| format!("llama.cpp 最近的版本都沒有適合 {}/{} 的執行檔", sys.os, sys.arch))
}

fn pick_runtime(releases: &[Release], sys: &SystemInfo) -> Option<PlannedFile> {
    let pats = runtime_patterns(sys);
    releases.iter().find_map(|rel| {
        pats.iter().find_map(|pat| {
            let a = rel.assets.iter().find(|a| a.name.contains(pat) && (a.name.ends_with(".zip") || a.name.ends_with(".tar.gz")))?;
            Some(PlannedFile {
                kind: "runtime".into(),
                name: a.name.clone(),
                url: a.browser_download_url.clone(),
                size: a.size,
                source: format!("github.com/ggml-org/llama.cpp（{}）", rel.tag_name),
                installed: false,
            })
        })
    })
}

pub async fn plan(id: &str) -> Result<Plan, String> {
    let cat = catalog();
    let model = cat.models.iter().find(|m| m.id == id).cloned().ok_or("目錄裡沒有這個模型")?;
    let mut files = Vec::new();
    let mut f = hf_find(&model.repo, &model.file_suffix).await?;
    f.kind = "model".into();
    files.push(f);
    if let Some(mm) = &model.mmproj_suffix {
        let mut f = hf_find(&model.repo, mm).await?;
        f.kind = "mmproj".into();
        files.push(f);
    }
    if runtime_binary().is_none() {
        files.push(runtime_asset(&system_info().await).await?);
    }
    let total_to_download = files.iter().filter(|f| !f.installed).map(|f| f.size).sum();
    Ok(Plan { model, files, total_to_download })
}

// ── Downloads ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Default)]
pub struct Progress {
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub state: String, // "queued" | "downloading" | "done" | "error"
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Manager {
    pub progress: Mutex<HashMap<String, Progress>>,
    pub runtime: Mutex<Option<Running>>,
    /// "idle" | "downloading" | "starting" | "ready" | "error: …"
    pub phase: Mutex<String>,
    /// One start at a time: the app restores the last model at launch, and a
    /// pick from the menu meanwhile would load two large models at once,
    /// overflow GPU memory, and time out.
    pub starting: Mutex<()>,
}

pub struct Running {
    pub child: Child,
    pub model_id: String,
    pub url: String,
}

async fn download(mgr: &Manager, f: &PlannedFile, dest: &Path) -> Result<(), String> {
    let set = |state: &str, done: u64, err: Option<String>| {
        let name = f.name.clone();
        let total = f.size;
        let state = state.to_string();
        async move {
            mgr.progress.lock().await.insert(name.clone(), Progress { name, done, total, state, error: err });
        }
    };
    set("downloading", 0, None).await;
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let part = dest.with_extension("part");
    let resp = http().get(&f.url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let mut out = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut done = 0u64;
    let mut last = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        out.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        if done - last > 4 << 20 {
            last = done;
            set("downloading", done, None).await;
        }
    }
    out.flush().await.map_err(|e| e.to_string())?;
    drop(out);
    tokio::fs::rename(&part, dest).await.map_err(|e| e.to_string())?;
    set("done", done, None).await;
    Ok(())
}

fn extract(archive: &Path, into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let name = archive.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        zip::ZipArchive::new(f).and_then(|mut z| z.extract(into)).map_err(|e| e.to_string())?;
    } else {
        let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        tar::Archive::new(flate2::read::GzDecoder::new(f)).unpack(into).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    if let Some(bin) = runtime_binary() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Download everything in the plan, then start the model. Runs in the background.
pub async fn install(mgr: Arc<Manager>, plan: Plan) -> Result<(), String> {
    *mgr.phase.lock().await = "downloading".into();
    std::fs::create_dir_all(models_dir()).map_err(|e| e.to_string())?;
    for f in &plan.files {
        mgr.progress.lock().await.insert(
            f.name.clone(),
            Progress { name: f.name.clone(), total: f.size, state: if f.installed { "done" } else { "queued" }.into(), done: if f.installed { f.size } else { 0 }, error: None },
        );
    }
    for f in plan.files.iter().filter(|f| !f.installed) {
        let dest = if f.kind == "runtime" { data_dir().join(&f.name) } else { models_dir().join(&f.name) };
        if let Err(e) = download(&mgr, f, &dest).await {
            let mut p = mgr.progress.lock().await;
            if let Some(x) = p.get_mut(&f.name) {
                x.state = "error".into();
                x.error = Some(e.clone());
            }
            *mgr.phase.lock().await = format!("error: {e}");
            return Err(e);
        }
        if f.kind == "runtime" {
            extract(&dest, &runtime_dir())?;
            let _ = std::fs::remove_file(&dest);
        }
    }
    // Remember which files belong to this model.
    let manifest = InstalledModel {
        id: plan.model.id.clone(),
        model_file: plan.files.iter().find(|f| f.kind == "model").map(|f| f.name.clone()).unwrap_or_default(),
        mmproj_file: plan.files.iter().find(|f| f.kind == "mmproj").map(|f| f.name.clone()),
        context: plan.model.context,
    };
    std::fs::create_dir_all(manifests_dir()).map_err(|e| e.to_string())?;
    let path = manifests_dir().join(format!("{}.json", plan.model.id));
    std::fs::write(path, serde_json::to_string_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledModel {
    pub id: String,
    pub model_file: String,
    pub mmproj_file: Option<String>,
    pub context: u32,
}

pub fn installed() -> Vec<InstalledModel> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(manifests_dir()).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "json") && e.file_name() != "catalog.json" {
            if let Some(m) = std::fs::read_to_string(e.path()).ok().and_then(|s| serde_json::from_str::<InstalledModel>(&s).ok()) {
                if models_dir().join(&m.model_file).exists() {
                    out.push(m);
                }
            }
        }
    }
    // Catalog models already in the shared store (downloaded by another
    // project or by `hf download`) count as installed without a manifest.
    for m in catalog().models {
        if out.iter().any(|o| o.id == m.id) {
            continue;
        }
        if let Some(model_file) = find_local(&m.repo, &m.file_suffix) {
            let mmproj_file = m.mmproj_suffix.as_deref().and_then(|s| find_local(&m.repo, s));
            out.push(InstalledModel { id: m.id, model_file, mmproj_file, context: m.context });
        }
    }
    out
}

/// `<repo>/<path>` of a complete local file in the repo folder ending with `suffix`.
fn find_local(repo: &str, suffix: &str) -> Option<String> {
    let want = suffix.to_lowercase();
    let root = models_dir().join(repo);
    let mut hits = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
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
    hits.sort_by_key(|p| p.as_os_str().len()); // same rule as hf_find
    let rel = hits.first()?.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
    Some(format!("{repo}/{rel}"))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(8089)
}

/// Start llama-server for an installed model; returns its /v1 base URL once it answers.
pub async fn start(mgr: &Manager, id: &str) -> Result<String, String> {
    let m = installed().into_iter().find(|m| m.id == id).ok_or("這個模型尚未下載")?;
    let bin = runtime_binary().ok_or("尚未安裝 llama.cpp")?;
    let _one_at_a_time = mgr.starting.lock().await;
    stop(mgr).await;
    *mgr.phase.lock().await = "starting".into();
    let port = free_port();
    let mut cmd = crate::proc::cmd(&bin);
    cmd.arg("-m")
        .arg(models_dir().join(&m.model_file))
        // No thinking: Gemma 4 and Qwen reason for a thousand tokens or more
        // before answering, which at ~10 tokens/s on a laptop turns a
        // paragraph into minutes. Every task here asks for a fixed JSON shape.
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-c", &m.context.to_string(), "-ngl", "999", "--jinja", "--reasoning-budget", "0"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(mm) = &m.mmproj_file {
        if models_dir().join(mm).exists() {
            cmd.arg("--mmproj").arg(models_dir().join(mm));
        }
    }
    if let Some(dir) = bin.parent() {
        cmd.current_dir(dir); // shared libraries ship next to the binary
    }
    let mut child = cmd.spawn().map_err(|e| format!("無法啟動 llama.cpp：{e}"))?;
    let url = format!("http://127.0.0.1:{port}/v1");
    // Loading a 12B model can take a while on first start.
    let client = http();
    for _ in 0..240 {
        if let Ok(Some(status)) = child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
            *mgr.phase.lock().await = format!("error: llama.cpp 結束（{status}）{tail}");
            return Err(format!("llama.cpp 啟動失敗：{tail}"));
        }
        if client.get(format!("{url}/models")).timeout(Duration::from_secs(2)).send().await.is_ok_and(|r| r.status().is_success()) {
            *mgr.runtime.lock().await = Some(Running { child, model_id: m.id.clone(), url: url.clone() });
            *mgr.phase.lock().await = "ready".into();
            return Ok(url);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    *mgr.phase.lock().await = "error: 模型啟動逾時".into();
    Err("模型啟動逾時".into())
}

static MANAGER: std::sync::OnceLock<Arc<Manager>> = std::sync::OnceLock::new();

pub fn register(mgr: Arc<Manager>) {
    let _ = MANAGER.set(mgr);
}

/// Synchronous stop for app exit: the desktop shell calls this so the model
/// server never outlives the window.
pub fn kill_runtime() {
    if let Some(mgr) = MANAGER.get() {
        if let Ok(mut r) = mgr.runtime.try_lock() {
            if let Some(run) = r.as_mut() {
                let _ = run.child.start_kill();
            }
        }
    }
}

pub async fn stop(mgr: &Manager) {
    if let Some(mut r) = mgr.runtime.lock().await.take() {
        let _ = r.child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_parses_and_default_exists() {
        let c: Catalog = serde_json::from_str(CATALOG).unwrap();
        assert!(c.models.iter().any(|m| m.id == c.default));
    }

    #[test]
    fn recommends_by_memory() {
        let c: Catalog = serde_json::from_str(CATALOG).unwrap();
        let sys = |mem: f32| SystemInfo { os: "windows", arch: "x86_64", memory_gb: mem, gpu: None, gpu_memory_gb: None, unified_memory: false };
        assert_eq!(recommend(&c, &sys(32.0)).as_deref(), Some("gemma-4-12b-q8"));
        assert_eq!(recommend(&c, &sys(16.0)).as_deref(), Some("gemma-4-12b-q4"));
        assert_eq!(recommend(&c, &sys(12.0)).as_deref(), Some("qwen3.5-4b-q8"));
        assert_eq!(recommend(&c, &sys(8.0)), None);
    }

    #[test]
    fn uncensored_models_are_never_recommended() {
        let c: Catalog = serde_json::from_str(CATALOG).unwrap();
        assert!(c.models.iter().any(|m| m.uncensored));
        let only = Catalog { version: 1, default: String::new(), models: c.models.into_iter().filter(|m| m.uncensored).collect() };
        let sys = SystemInfo { os: "macos", arch: "aarch64", memory_gb: 128.0, gpu: None, gpu_memory_gb: None, unified_memory: true };
        assert_eq!(recommend(&only, &sys), None);
    }

    #[test]
    fn runtime_skips_releases_without_binaries() {
        let asset = |n: &str| Asset { name: n.into(), size: 1, browser_download_url: format!("https://x/{n}") };
        let releases = vec![
            Release { tag_name: "v0.9.0".into(), assets: vec![asset("nightly-tag.txt")] },
            Release { tag_name: "b12182".into(), assets: vec![asset("llama-b12182-bin-macos-x64.tar.gz"), asset("llama-b12182-bin-macos-arm64.tar.gz")] },
        ];
        let mac = SystemInfo { os: "macos", arch: "aarch64", memory_gb: 32.0, gpu: None, gpu_memory_gb: None, unified_memory: true };
        let f = pick_runtime(&releases, &mac).unwrap();
        assert_eq!(f.name, "llama-b12182-bin-macos-arm64.tar.gz");
        assert!(f.source.contains("b12182"));
        let other = SystemInfo { os: "plan9", ..mac };
        assert!(pick_runtime(&releases, &other).is_none());
    }
}
