//! Local models: system check, downloads, and the llama.cpp runtime.
//!
//! The catalog is ym-llm's shared `catalog.json` (a new model is a new
//! entry); a user may replace it with `models/catalog.json` in the app data
//! folder, in the same shape. Files are resolved at download time through the
//! Hugging Face API by suffix, and llama.cpp comes from its latest GitHub
//! release, so neither exact build names nor download links are hard-coded.
//! Starting and stopping llama-server is ym-llm's `LocalRuntime`.
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
use tokio::sync::Mutex;
use ym_llm::catalog::{Catalog, Machine, Model};
use ym_llm::local::{self, Layout, LocalRuntime, Phase};

/// The shared catalog, or a user override in the app data folder. Without
/// an override, the descriptions are PaperComposer's own
/// (`model_summaries.json`): the shared ones are worded for every app.
pub fn catalog() -> Catalog {
    let user = data_dir().join("models/catalog.json");
    let mut c = Catalog::load(Some(&user));
    if !user.is_file() {
        apply_summaries(&mut c);
    }
    c
}

/// Replace the descriptions of the models `model_summaries.json` names.
pub fn apply_summaries(c: &mut Catalog) {
    #[derive(Deserialize)]
    struct Summaries {
        summaries: HashMap<String, String>,
    }
    let Ok(s) = serde_json::from_str::<Summaries>(include_str!("model_summaries.json")) else { return };
    for m in &mut c.models {
        if let Some(t) = s.summaries.get(&m.id) {
            m.summary = t.clone();
        }
    }
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer")
}

/// Shared store for model weights and the llama.cpp runtime, so every project
/// on this machine uses one copy: `LOCAL_LLM_HOME`, then `local_llm_home` in
/// config.toml (apps opened from Finder do not see shell variables).
pub fn llm_home() -> Option<PathBuf> {
    local::home(crate::read_config().local_llm_home.as_deref())
}

/// Where models and llama.cpp live: the shared folder (`gguf/`,
/// `runtimes/llama.cpp/`), else the app's data folder as the first versions
/// laid it out (`models/`, `llama.cpp/`). Weights are always
/// `<owner>/<repo>/<file>` like `hf download --local-dir`.
pub fn layout() -> Layout {
    match llm_home() {
        Some(h) => Layout::shared(&h),
        None => Layout::new(data_dir().join("models"), data_dir().join("llama.cpp")),
    }
}

fn models_dir() -> PathBuf {
    layout().gguf
}
fn runtime_dir() -> PathBuf {
    layout().runtime
}

/// Catalog brains already in the model folder (downloaded here, by another
/// project, or with `hf download`).
pub fn installed(cat: &Catalog) -> Vec<String> {
    layout().installed(cat, true).into_iter().map(|m| m.id.clone()).collect()
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
    // Unified memory and discrete GPUs both count; ym-llm takes the larger pool.
    ym_llm::catalog::recommend(cat, &Machine { memory_gb: sys.memory_gb as f64, gpu_memory_gb: sys.gpu_memory_gb.map(f64::from) })
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
    pub model: Model,
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
    layout().find_runtime()
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

pub async fn plan(cat: &Catalog, id: &str) -> Result<Plan, String> {
    let model = cat.brains(true).find(|m| m.id == id).cloned().ok_or("目錄裡沒有這個模型")?;
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
    /// The llama-server, one at a time: the app restores the last model at
    /// launch, and a pick from the menu meanwhile would otherwise load two
    /// large models at once and overflow GPU memory.
    pub runtime: Arc<LocalRuntime>,
    /// Downloading, or why a download or start failed; `None` = the
    /// runtime's own state.
    note: Mutex<Option<String>>,
}

impl Manager {
    /// "idle" | "downloading" | "starting" | "ready" | "error: …"
    pub async fn phase(&self) -> String {
        if let Some(n) = self.note.lock().await.clone() {
            return n;
        }
        match self.runtime.phase() {
            Phase::Idle => "idle".into(),
            Phase::Starting { .. } => "starting".into(),
            Phase::Ready { .. } => "ready".into(),
            Phase::Error { message } => format!("error: {message}"),
        }
    }

    pub async fn note(&self, n: Option<String>) {
        *self.note.lock().await = n;
    }
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
    mgr.note(Some("downloading".into())).await;
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
            mgr.note(Some(format!("error: {e}"))).await;
            return Err(e);
        }
        if f.kind == "runtime" {
            if let Err(e) = extract(&dest, &runtime_dir()) {
                mgr.note(Some(format!("error: {e}"))).await;
                return Err(e);
            }
            let _ = std::fs::remove_file(&dest);
        }
    }
    mgr.note(None).await;
    Ok(())
}

/// Start llama-server for a downloaded model; returns its /v1 base URL once it answers.
pub async fn start(mgr: &Manager, cat: &Catalog, id: &str) -> Result<String, String> {
    let m = cat.brains(true).find(|m| m.id == id).ok_or("目錄裡沒有這個模型")?;
    mgr.note(None).await;
    mgr.runtime.ensure_in(&layout(), m).await.map(|r| r.url)
}

static MANAGER: std::sync::OnceLock<Arc<Manager>> = std::sync::OnceLock::new();

pub fn register(mgr: Arc<Manager>) {
    let _ = MANAGER.set(mgr);
}

/// Synchronous stop for app exit: the desktop shell calls this so the model
/// server never outlives the window.
pub fn kill_runtime() {
    if let Some(mgr) = MANAGER.get() {
        mgr.runtime.kill_now();
    }
}

pub async fn stop(mgr: &Manager) {
    mgr.runtime.stop().await;
    mgr.note(None).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_composer_describes_models_in_its_own_words() {
        let mut c = Catalog::bundled();
        apply_summaries(&mut c);
        for m in c.brains(true) {
            assert!(!m.summary.contains("字詞") && !m.summary.contains("對話"), "{}: {}", m.id, m.summary);
        }
        // every id in the map is still in the shared catalog
        let s: serde_json::Value = serde_json::from_str(include_str!("model_summaries.json")).unwrap();
        for id in s["summaries"].as_object().unwrap().keys() {
            assert!(c.get(id).is_some(), "{id} is not in the shared catalog");
        }
    }

    #[test]
    fn shared_catalog_has_the_default_brain() {
        let c = Catalog::bundled();
        assert!(c.brains(true).any(|m| m.id == c.defaults.brain));
        assert!(c.brains(true).any(|m| m.uncensored), "PaperComposer lists abliterated models too");
    }

    #[test]
    fn recommends_by_memory() {
        let c = Catalog::bundled();
        let sys = |mem: f32| SystemInfo { os: "windows", arch: "x86_64", memory_gb: mem, gpu: None, gpu_memory_gb: None, unified_memory: false };
        assert_eq!(recommend(&c, &sys(32.0)).as_deref(), Some("gemma-4-12b-q8"));
        assert_eq!(recommend(&c, &sys(16.0)).as_deref(), Some("gemma-4-12b-q4"));
        assert_eq!(recommend(&c, &sys(12.0)).as_deref(), Some("qwen3.5-4b-q8"));
        assert_eq!(recommend(&c, &sys(8.0)), None);
        // A discrete GPU counts too.
        let gpu = SystemInfo { gpu_memory_gb: Some(24.0), ..sys(8.0) };
        assert_eq!(recommend(&c, &gpu).as_deref(), Some("gemma-4-12b-q8"));
    }

    #[test]
    fn uncensored_models_are_never_recommended() {
        let mut only = Catalog::bundled();
        only.models.retain(|m| m.uncensored);
        let sys = SystemInfo { os: "macos", arch: "aarch64", memory_gb: 128.0, gpu: None, gpu_memory_gb: None, unified_memory: true };
        assert_eq!(recommend(&only, &sys), None);
    }

    #[tokio::test]
    async fn phase_reads_downloads_then_the_runtime() {
        let m = Manager::default();
        assert_eq!(m.phase().await, "idle");
        m.note(Some("downloading".into())).await;
        assert_eq!(m.phase().await, "downloading");
        stop(&m).await;
        assert_eq!(m.phase().await, "idle");
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
