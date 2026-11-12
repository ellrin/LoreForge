//! Thumbnails through macOS QuickLook (`qlmanage -t -s <size> -o <tmp> <path>`),
//! run by the injected [`Runner`](crate::Runner) with a timeout, at most
//! [`MAX_PARALLEL`] at a time. Cached in `<cache>/LoreForge/shelf/thumbs/<id>-<size>.png`
//! (a `.none` marker when QuickLook gave nothing), oldest-used removed beyond
//! [`CACHE_MAX_BYTES`]. Never for app bundles, datasets, symlinks or dataless
//! (iCloud-evicted) files — QuickLook would download those. A project's
//! thumbnail is its `main.pdf` (the compiled `main.tex`) when present.

use crate::index::Entry;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CACHE_MAX_BYTES: u64 = 200 << 20;
pub const TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_PARALLEL: usize = 2;
pub const DEFAULT_SIZE: u32 = 512;
/// Bundles that are programs: never handed to QuickLook.
pub const NEVER: [&str; 8] = ["app", "appex", "framework", "bundle", "plugin", "kext", "xpc", "prefpane"];

pub fn cache_file(dir: &Path, id: &str, size: u32) -> PathBuf {
    dir.join(format!("{id}-{size}.png"))
}

fn none_file(dir: &Path, id: &str, size: u32) -> PathBuf {
    dir.join(format!("{id}-{size}.none"))
}

/// What to give QuickLook for this entry (`None` = no thumbnail).
pub fn source(e: &Entry) -> Option<PathBuf> {
    let existing = e.paths.iter().map(PathBuf::from).find(|p| std::fs::symlink_metadata(p).is_ok())?;
    let target = match e.item.kind.as_str() {
        "dataset" => return None,
        "package" if NEVER.contains(&e.item.ext.as_str()) => return None,
        "project" => {
            let pdf = existing.join("main.pdf");
            if existing.join("main.tex").exists() && pdf.is_file() { pdf } else { return None }
        }
        _ => existing,
    };
    safe_file(&target).then_some(target)
}

/// A regular file QuickLook may read without downloading anything.
pub fn safe_file(p: &Path) -> bool {
    let Ok(md) = std::fs::symlink_metadata(p) else { return false };
    md.is_file()
        && !lore_catalog::scan::is_dataless(&md)
        && !p.file_name().is_some_and(|n| lore_catalog::scan::is_placeholder_name(&n.to_string_lossy()))
}

/// Cache a finished thumbnail (`None` = remember there is none).
pub fn store(dir: &Path, id: &str, size: u32, png: Option<&[u8]>) {
    let _ = std::fs::create_dir_all(dir);
    match png {
        Some(b) => {
            let _ = lore_catalog::store::write_atomic(&cache_file(dir, id, size), b);
        }
        None => {
            let _ = std::fs::write(none_file(dir, id, size), b"");
        }
    }
    prune(dir, CACHE_MAX_BYTES);
}

/// A cached thumbnail (refreshing its use time), `Some(None)` = known to have
/// none, `None` = not tried yet.
pub fn cached(dir: &Path, id: &str, size: u32) -> Option<Option<Vec<u8>>> {
    let f = cache_file(dir, id, size);
    if let Ok(b) = std::fs::read(&f) {
        if let Ok(h) = std::fs::File::options().write(true).open(&f) {
            let _ = h.set_modified(std::time::SystemTime::now());
        }
        return Some(Some(b));
    }
    none_file(dir, id, size).exists().then_some(None)
}

/// Run QuickLook for `src` and cache the result. Blocking.
pub fn make(runner: &dyn crate::Runner, dir: &Path, id: &str, size: u32, src: &Path) -> Option<Vec<u8>> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&tmp).ok()?;
    let args = vec!["-t".to_string(), "-s".into(), size.to_string(), "-o".into(), tmp.to_string_lossy().into(), src.to_string_lossy().into()];
    let ok = runner.run_wait("qlmanage", &args, TIMEOUT).is_ok();
    let png = if ok {
        std::fs::read_dir(&tmp).ok().and_then(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "png")))
    } else {
        None
    };
    let out = match png.and_then(|p| std::fs::read(&p).ok()).filter(|b| b.starts_with(b"\x89PNG")) {
        Some(b) => {
            let _ = lore_catalog::store::write_atomic(&cache_file(dir, id, size), &b);
            Some(b)
        }
        None => {
            let _ = std::fs::write(none_file(dir, id, size), b"");
            None
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    prune(dir, CACHE_MAX_BYTES);
    out
}

/// Remove the least recently used thumbnails until the folder fits `max`.
pub fn prune(dir: &Path, max: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, p) in files {
        if total <= max {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}
