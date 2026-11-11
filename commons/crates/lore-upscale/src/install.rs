//! Where the tool lives and the one-time download.

use crate::run::{CancelToken, UpscaleError};
use crate::{BINARY, RUNTIME_DIR};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The official release (Real-ESRGAN-ncnn-vulkan 20220424, macOS universal binary).
pub const DEFAULT_URL: &str = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-macos.zip";
/// SHA-256 of that zip (51 817 124 bytes), checked before anything is unpacked.
pub const DEFAULT_SHA256: &str = "e0ad05580abfeb25f8d8fb55aaf7bedf552c375b5b4d9bd3c8d59764d2cc333a";

const LICENSE_BSD: &str = include_str!("../assets/LICENSE-Real-ESRGAN.txt");
const LICENSE_MIT: &str = include_str!("../assets/LICENSE-Real-ESRGAN-ncnn-vulkan.txt");

/// What to download and the checksum it must have.
#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub sha256: String,
}

impl Source {
    /// The pinned official release.
    pub fn official() -> Source {
        Source { url: DEFAULT_URL.into(), sha256: DEFAULT_SHA256.into() }
    }
}

/// A download's state for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Bytes received, and the total when the server said.
    Downloading { done: u64, total: Option<u64> },
    Verifying,
    Installing,
}

/// The tool's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub dir: PathBuf,
}

impl Install {
    /// `<home>/runtimes/realesrgan` in the shared model folder.
    pub fn in_home(home: &Path) -> Install {
        Install { dir: home.join(RUNTIME_DIR) }
    }

    /// Any folder with the binary and `models/` (a tool already unpacked somewhere).
    pub fn from_dir(dir: impl Into<PathBuf>) -> Install {
        Install { dir: dir.into() }
    }

    /// The shared model folder: `LOCAL_LLM_HOME`, else `[local] home` of `engine.toml`
    /// (found from `start`, like every app does).
    pub fn discover(start: &Path) -> Option<Install> {
        let home = std::env::var_os("LOCAL_LLM_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| lore_home::Home::discover(start).local_home())?;
        Some(Install::in_home(&home))
    }

    pub fn binary(&self) -> PathBuf {
        self.dir.join(BINARY)
    }

    pub fn models_dir(&self) -> PathBuf {
        self.dir.join("models")
    }

    /// The binary's path when it and the files of network `model_name` exist, else
    /// [`UpscaleError::NotInstalled`] naming what is missing.
    pub fn check(&self, model_name: &str) -> Result<PathBuf, UpscaleError> {
        let bin = self.binary();
        let mut missing = Vec::new();
        if !bin.is_file() {
            missing.push(BINARY.to_string());
        }
        for ext in ["param", "bin"] {
            let f = format!("models/{model_name}.{ext}");
            if !self.dir.join(&f).is_file() {
                missing.push(f);
            }
        }
        if missing.is_empty() { Ok(bin) } else { Err(UpscaleError::NotInstalled { dir: self.dir.clone(), missing: missing.join("、") }) }
    }

    /// Everything the models need is there.
    pub fn is_installed(&self) -> bool {
        ["realesrgan-x4plus-anime", "realesrgan-x4plus", "realesr-animevideov3-x2", "realesr-animevideov3-x3", "realesr-animevideov3-x4"].iter().all(|n| self.check(n).is_ok())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download `source`, verify the checksum, unpack the binary and models, remove the
/// quarantine attribute, chmod, record the licences. Meant to be called when the author
/// presses 下載. Replaces an existing install only after everything checked out; a failure
/// or cancel leaves the old one (if any) and no partial folder.
pub async fn download(install: &Install, source: &Source, cancel: &CancelToken, mut on_progress: impl FnMut(Progress)) -> Result<(), UpscaleError> {
    let parent = install.dir.parent().ok_or_else(|| UpscaleError::Download("資料夾位置不對".into()))?;
    std::fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".realesrgan-download-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let out = fetch_and_unpack(install, source, cancel, &mut on_progress, &stage).await;
    let _ = std::fs::remove_dir_all(&stage);
    out
}

async fn fetch_and_unpack(install: &Install, source: &Source, cancel: &CancelToken, on_progress: &mut impl FnMut(Progress), stage: &Path) -> Result<(), UpscaleError> {
    let resp = reqwest::get(&source.url).await.map_err(|e| UpscaleError::Download(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(UpscaleError::Download(format!("HTTP {}", resp.status())));
    }
    let total = resp.content_length();
    let zip_path = stage.join("download.zip");
    let mut file = std::fs::File::create(&zip_path)?;
    let (mut hasher, mut done) = (Sha256::new(), 0u64);
    let mut stream = resp.bytes_stream();
    on_progress(Progress::Downloading { done, total });
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(UpscaleError::Cancelled);
        }
        let chunk = chunk.map_err(|e| UpscaleError::Download(e.to_string()))?;
        hasher.update(&chunk);
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        on_progress(Progress::Downloading { done, total });
    }
    file.flush()?;
    drop(file);
    on_progress(Progress::Verifying);
    let actual = hex(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(source.sha256.trim()) {
        return Err(UpscaleError::Checksum { expected: source.sha256.trim().to_lowercase(), actual });
    }
    on_progress(Progress::Installing);
    let unpacked = stage.join("unpacked");
    let (zp, up, src) = (zip_path.clone(), unpacked.clone(), source.clone());
    tokio::task::spawn_blocking(move || unpack(&zp, &up, &src)).await.map_err(|e| UpscaleError::Download(e.to_string()))??;
    if install.dir.exists() {
        std::fs::remove_dir_all(&install.dir)?;
    }
    std::fs::rename(&unpacked, &install.dir)?;
    Ok(())
}

/// The binary and `models/*` out of the zip (never anything else, never outside `dir`),
/// then quarantine, chmod, licences, `install.json`.
fn unpack(zip_path: &Path, dir: &Path, source: &Source) -> Result<(), UpscaleError> {
    let bad = |e: zip::result::ZipError| UpscaleError::Download(format!("壓縮檔打不開：{e}"));
    let mut zip = zip::ZipArchive::new(std::fs::File::open(zip_path)?).map_err(bad)?;
    std::fs::create_dir_all(dir.join("models"))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(bad)?;
        let Some(rel) = f.enclosed_name() else { continue };
        let want = rel.as_path() == Path::new(BINARY) || (rel.parent() == Some(Path::new("models")) && f.is_file());
        if !want || !f.is_file() {
            continue;
        }
        let mut out = std::fs::File::create(dir.join(&rel))?;
        std::io::copy(&mut f, &mut out)?;
    }
    let bin = dir.join(BINARY);
    if !bin.is_file() {
        return Err(UpscaleError::Download(format!("壓縮檔裡沒有 {BINARY}")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
    }
    // Downloaded files carry the quarantine attribute; Gatekeeper would refuse the binary.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(dir).output();
    }
    std::fs::write(dir.join("LICENSE-Real-ESRGAN.txt"), LICENSE_BSD)?;
    std::fs::write(dir.join("LICENSE-Real-ESRGAN-ncnn-vulkan.txt"), LICENSE_MIT)?;
    let info = serde_json::json!({
        "name": "Real-ESRGAN ncnn-vulkan",
        "release": "20220424",
        "url": source.url,
        "sha256": source.sha256.trim().to_lowercase(),
        "license": "BSD-3-Clause (Real-ESRGAN); MIT (ncnn-vulkan wrapper); see LICENSE-*.txt",
    });
    std::fs::write(dir.join("install.json"), serde_json::to_vec_pretty(&info).unwrap_or_default())?;
    Ok(())
}
