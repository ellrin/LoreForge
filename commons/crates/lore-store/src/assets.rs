//! Content-addressed assets: `sha256` of the stored bytes, kept as
//! `<root>/ab/abcd….ext` (first two hex digits as a folder).
//!
//! - [`Backend::Icloud`]: personal spaces; `<root>` is an iCloud Drive folder. When macOS
//!   has evicted a file it leaves a placeholder `.<name>.icloud`; reading then runs
//!   `brctl download <path>` through the injected [`CommandRunner`] and waits for the
//!   file. **Not tested on real hardware** (tests use a fake runner and temp folders).
//! - [`Backend::Git`]: household spaces; `<root>` is a folder inside the shared repo.
//!   Raster images are re-encoded to webp, scaled to `max_dim`, and must fit `max_bytes`.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::write_atomic;
use crate::runner::CommandRunner;
use crate::{Result, StoreError};

/// Extensions accepted as assets.
pub const ALLOWED_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "pdf"];

/// A stored asset: `{"asset": "sha256:<hex>", "ext": "png"}` in node JSON.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetRef {
    /// `sha256:<64 hex>`
    pub asset: String,
    pub ext: String,
}

impl AssetRef {
    pub fn new(hex: &str, ext: &str) -> AssetRef {
        AssetRef { asset: format!("sha256:{hex}"), ext: ext.to_string() }
    }

    /// Parse `sha256:<hex>` (or a bare hex digest) with an extension.
    pub fn parse(asset: &str, ext: &str) -> Result<AssetRef> {
        let hex = asset.strip_prefix("sha256:").unwrap_or(asset);
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
            return Err(StoreError::Invalid(format!("asset id `{asset}`")));
        }
        let ext = normalize_ext(ext)?;
        Ok(AssetRef::new(hex, &ext))
    }

    pub fn hex(&self) -> &str {
        self.asset.strip_prefix("sha256:").unwrap_or(&self.asset)
    }

    /// `ab/abcd….ext`
    pub fn rel_path(&self) -> String {
        let h = self.hex();
        format!("{}/{}.{}", &h[..2], h, self.ext)
    }

    /// `abcd….ext` (used in URLs).
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.hex(), self.ext)
    }
}

pub fn normalize_ext(ext: &str) -> Result<String> {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    let e = if e == "jpeg" { "jpg".to_string() } else { e };
    if ALLOWED_EXT.contains(&e.as_str()) { Ok(e) } else { Err(StoreError::Invalid(format!("asset type `.{ext}` is not allowed"))) }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Icloud,
    Git { max_bytes: u64, max_dim: u32 },
}

pub struct AssetStore {
    root: PathBuf,
    backend: Backend,
    runner: Arc<dyn CommandRunner>,
    /// How often and how long to wait for `brctl download` to bring a file back.
    poll: (u32, Duration),
}

impl std::fmt::Debug for AssetStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetStore").field("root", &self.root).field("backend", &self.backend).finish()
    }
}

impl AssetStore {
    pub fn new(root: PathBuf, backend: Backend, runner: Arc<dyn CommandRunner>) -> AssetStore {
        AssetStore { root, backend, runner, poll: (40, Duration::from_millis(250)) }
    }

    /// Change how long reads wait for an iCloud download (`attempts` × `interval`).
    pub fn with_poll(mut self, attempts: u32, interval: Duration) -> AssetStore {
        self.poll = (attempts.max(1), interval);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn path(&self, r: &AssetRef) -> PathBuf {
        self.root.join(r.rel_path())
    }

    fn placeholder(&self, r: &AssetRef) -> PathBuf {
        self.root.join(&r.hex()[..2]).join(format!(".{}.icloud", r.file_name()))
    }

    /// The file is here or iCloud knows it (placeholder).
    pub fn exists(&self, r: &AssetRef) -> bool {
        self.path(r).is_file() || (self.backend == Backend::Icloud && self.placeholder(r).is_file())
    }

    /// Store bytes; returns the reference to put in a node. Storing the same content
    /// twice is a no-op. Household (git) images come back as webp.
    pub fn put(&self, bytes: &[u8], ext: &str) -> Result<AssetRef> {
        let ext = normalize_ext(ext)?;
        let (bytes, ext) = match self.backend {
            Backend::Icloud => (bytes.to_vec(), ext),
            Backend::Git { max_bytes, max_dim } => prepare_for_git(bytes, &ext, max_bytes, max_dim)?,
        };
        let r = AssetRef::new(&sha256_hex(&bytes), &ext);
        if !self.exists(&r) {
            write_atomic(&self.path(&r), &bytes)?;
        }
        Ok(r)
    }

    /// Make sure the file is on this disk (iCloud may have evicted it) and return its path.
    pub fn ensure_local(&self, r: &AssetRef) -> Result<PathBuf> {
        let path = self.path(r);
        if path.is_file() {
            return Ok(path);
        }
        if self.backend == Backend::Icloud && self.placeholder(r).is_file() {
            let p = path.to_string_lossy().to_string();
            let out = self.runner.run("brctl", &["download", &p])?;
            if out.status != 0 {
                return Err(StoreError::Invalid(format!("brctl download failed: {}", out.stderr.trim())));
            }
            for i in 0..self.poll.0 {
                if path.is_file() {
                    return Ok(path);
                }
                if i + 1 < self.poll.0 {
                    std::thread::sleep(self.poll.1);
                }
            }
            return Err(StoreError::NotFound(format!("{} (still downloading from iCloud)", r.file_name())));
        }
        Err(StoreError::NotFound(r.file_name()))
    }

    pub fn get(&self, r: &AssetRef) -> Result<Vec<u8>> {
        let path = self.ensure_local(r)?;
        let bytes = std::fs::read(path)?;
        if sha256_hex(&bytes) != r.hex() {
            return Err(StoreError::Invalid(format!("{} does not match its hash", r.file_name())));
        }
        Ok(bytes)
    }

    /// Copy an asset from another store (used by an explicit share). The new reference
    /// may differ (household re-encodes to webp).
    pub fn import_from(&self, other: &AssetStore, r: &AssetRef) -> Result<AssetRef> {
        let bytes = other.get(r)?;
        self.put(&bytes, &r.ext)
    }
}

fn prepare_for_git(bytes: &[u8], ext: &str, max_bytes: u64, max_dim: u32) -> Result<(Vec<u8>, String)> {
    let raster = matches!(ext, "png" | "jpg" | "gif" | "webp");
    if !raster {
        if bytes.len() as u64 > max_bytes {
            return Err(StoreError::Invalid(format!("asset is {} bytes; the household limit is {max_bytes}", bytes.len())));
        }
        return Ok((bytes.to_vec(), ext.to_string()));
    }
    let img = image::load_from_memory(bytes).map_err(|e| StoreError::Image(e.to_string()))?;
    let mut dim = max_dim.max(16);
    loop {
        let scaled = if img.width() > dim || img.height() > dim { img.resize(dim, dim, image::imageops::FilterType::Lanczos3) } else { img.clone() };
        let mut out = Cursor::new(Vec::new());
        // Lossless only: `image` has no lossy webp encoder. Photos may need several
        // down-scaling rounds to fit the cap.
        scaled.to_rgba8().write_to(&mut out, image::ImageFormat::WebP).map_err(|e| StoreError::Image(e.to_string()))?;
        let out = out.into_inner();
        if out.len() as u64 <= max_bytes {
            return Ok((out, "webp".to_string()));
        }
        let longest = scaled.width().max(scaled.height());
        if longest <= 128 {
            return Err(StoreError::Invalid(format!("image does not fit the household limit of {max_bytes} bytes")));
        }
        dim = (longest as f32 * 0.75) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::CommandOutput;
    use std::sync::Mutex;

    /// Fake `brctl`: records calls; on `download` writes the file iCloud would bring back.
    struct FakeBrctl {
        calls: Mutex<Vec<String>>,
        content: Vec<u8>,
    }
    impl CommandRunner for FakeBrctl {
        fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput> {
            self.calls.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            if program == "brctl" && args.first() == Some(&"download") {
                let p = PathBuf::from(args[1]);
                let ph = p.with_file_name(format!(".{}.icloud", p.file_name().unwrap().to_string_lossy()));
                std::fs::write(&p, &self.content)?;
                let _ = std::fs::remove_file(ph);
            }
            Ok(CommandOutput::default())
        }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 7 % 256) as u8, (y * 13 % 256) as u8, ((x ^ y) % 256) as u8, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn icloud_put_get_and_layout() {
        let d = tempfile::tempdir().unwrap();
        let runner = Arc::new(FakeBrctl { calls: Mutex::new(vec![]), content: vec![] });
        let s = AssetStore::new(d.path().to_path_buf(), Backend::Icloud, runner.clone());
        let r = s.put(b"hello", "PNG").unwrap();
        let hex = sha256_hex(b"hello");
        assert_eq!(r.asset, format!("sha256:{hex}"));
        assert_eq!(r.ext, "png");
        assert!(d.path().join(&hex[..2]).join(format!("{hex}.png")).is_file());
        assert_eq!(s.get(&r).unwrap(), b"hello");
        assert_eq!(s.put(b"hello", "png").unwrap(), r, "same content, same ref");
        assert!(runner.calls.lock().unwrap().is_empty());
        assert!(s.put(b"x", "exe").is_err());
    }

    #[test]
    fn icloud_placeholder_triggers_brctl() {
        let d = tempfile::tempdir().unwrap();
        let content = b"evicted image".to_vec();
        let hex = sha256_hex(&content);
        let r = AssetRef::new(&hex, "png");
        let runner = Arc::new(FakeBrctl { calls: Mutex::new(vec![]), content: content.clone() });
        let s = AssetStore::new(d.path().to_path_buf(), Backend::Icloud, runner.clone()).with_poll(3, Duration::from_millis(1));
        std::fs::create_dir_all(d.path().join(&hex[..2])).unwrap();
        std::fs::write(d.path().join(&hex[..2]).join(format!(".{hex}.png.icloud")), b"plist").unwrap();
        assert!(s.exists(&r));
        assert_eq!(s.get(&r).unwrap(), content);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("brctl download "));
        assert!(calls[0].ends_with(&format!("{hex}.png")));
    }

    #[test]
    fn missing_asset_is_not_found() {
        let d = tempfile::tempdir().unwrap();
        let s = AssetStore::new(d.path().to_path_buf(), Backend::Icloud, Arc::new(crate::SystemRunner));
        let r = AssetRef::new(&sha256_hex(b"nope"), "png");
        assert!(matches!(s.get(&r), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn git_backend_reencodes_webp_and_caps() {
        let d = tempfile::tempdir().unwrap();
        let s = AssetStore::new(d.path().to_path_buf(), Backend::Git { max_bytes: 200_000, max_dim: 64 }, Arc::new(crate::SystemRunner));
        let r = s.put(&png(200, 100), "png").unwrap();
        assert_eq!(r.ext, "webp");
        let bytes = s.get(&r).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (64, 32));
        // Non-images above the cap are refused.
        let small = AssetStore::new(d.path().to_path_buf(), Backend::Git { max_bytes: 10, max_dim: 64 }, Arc::new(crate::SystemRunner));
        assert!(small.put(&[0u8; 100], "pdf").is_err());
        // Import from a personal store re-encodes.
        let p = AssetStore::new(d.path().join("icloud"), Backend::Icloud, Arc::new(crate::SystemRunner));
        let pr = p.put(&png(20, 20), "png").unwrap();
        let hr = s.import_from(&p, &pr).unwrap();
        assert_eq!(hr.ext, "webp");
        assert!(s.exists(&hr));
    }

    #[test]
    fn parse_refs() {
        let hex = sha256_hex(b"a");
        assert!(AssetRef::parse(&format!("sha256:{hex}"), "png").is_ok());
        assert!(AssetRef::parse("sha256:../../etc", "png").is_err());
        assert_eq!(AssetRef::parse(&hex, ".JPEG").unwrap().ext, "jpg");
    }
}
