//! The read-only walk.
//!
//! Entries are visited in name order with `symlink_metadata` (symlinks are
//! never followed, only counted). Before any file is opened it is checked for
//! being an iCloud placeholder (`.<name>.icloud`) or dataless (`SF_DATALESS`):
//! opening those would download them. Projects (marker files), macOS packages
//! (`.app`, …) and datasets (> 500 files, ≥ 90 % one extension) become one
//! record each and are not walked file by file (see [`crate::project`]);
//! everything else is walked, `build` / `target` / `node_modules` included. A scan root that cannot be read stops the scan; anything below it
//! that cannot be read is skipped and listed.

use crate::hash::{CacheEntry, FileKey, HashCache};
use crate::project::{self, Tally, UnitRecord};
use crate::record::{Download, FileRecord, Observation, TimePoint};
use crate::report::{self, Report};
use crate::store::{Catalog, Guard, resolve};
use crate::{CatalogError, times};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub roots: Vec<PathBuf>,
    /// Where records and reports go (guarded, see [`Guard`]).
    pub catalog_dir: PathBuf,
    /// Where the hash cache goes (cache tier).
    pub cache_dir: PathBuf,
    pub guard: Guard,
}

/// Why something was not catalogued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Permission denied or a read error.
    Unreadable,
    /// iCloud placeholder (`*.icloud`) or dataless file: not downloaded.
    Placeholder,
    Symlink,
    /// Sockets, FIFOs, devices.
    Special,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: SkipReason,
}

/// One file seen in this scan.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub name: String,
    /// The scan root it was found under, and its first folder below that root.
    pub root: String,
    pub top: Option<String>,
    pub size: u64,
    pub sha256: String,
    pub kind: String,
    pub times: Vec<TimePoint>,
    pub download: Download,
}

#[derive(Debug, Default)]
pub struct Walk {
    pub files: Vec<Seen>,
    /// Projects, packages and datasets (one record each, not walked file by file).
    pub units: Vec<UnitRecord>,
    pub skipped: Vec<Skipped>,
    pub ds_store: u64,
    pub hashed_files: u64,
    pub hashed_bytes: u64,
    pub cached_files: u64,
    /// The scan's start time (RFC 3339): first_seen / last_seen of everything.
    pub now: String,
}

#[derive(Debug)]
pub struct ScanOutcome {
    pub report: Report,
    pub report_md: PathBuf,
    pub report_json: PathBuf,
    pub catalog: PathBuf,
}

/// Scan `cfg.roots`, merge records into the catalog, write the report.
pub fn scan(cfg: &ScanConfig) -> Result<ScanOutcome, CatalogError> {
    let started = chrono::Utc::now();
    let now = started.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut roots = Vec::new();
    for r in &cfg.roots {
        let c = r.canonicalize().map_err(|e| CatalogError::Root { path: r.clone(), source: e })?;
        std::fs::read_dir(&c).map_err(|e| CatalogError::Root { path: r.clone(), source: e })?;
        roots.push(c);
    }
    let catalog = Catalog::open(&cfg.catalog_dir, &cfg.guard)?;
    // Never write the catalog into a scanned folder (or scan the catalog).
    for r in &roots {
        if catalog.dir.starts_with(r) || r.starts_with(&catalog.dir) || resolve(&cfg.cache_dir).starts_with(r) {
            return Err(CatalogError::Guard(format!("目錄或快取不能放在要掃描的資料夾 {} 裡", r.display())));
        }
    }
    let mut cache = HashCache::load(&cfg.cache_dir);
    let mut w = Walk { now: now.clone(), ..Walk::default() };
    for r in &roots {
        walk_dir(r, r, None, &mut cache, &mut w);
    }

    // Records: one per content.
    let mut by_sha: BTreeMap<&str, Vec<&Seen>> = BTreeMap::new();
    for s in &w.files {
        by_sha.entry(s.sha256.as_str()).or_default().push(s);
    }
    let files_dir = catalog.files_dir();
    for (sha, seen) in &by_sha {
        let first = seen[0];
        let mut rec = FileRecord::load(&files_dir, sha).unwrap_or_else(|| FileRecord::new(sha, first.size, &first.kind));
        rec.size = first.size;
        rec.kind.clone_from(&first.kind);
        for s in seen {
            rec.observe(&Observation { path: s.path.clone(), name: s.name.clone(), times: s.times.clone(), download: s.download.clone() }, &now);
        }
        rec.save(&files_dir)?;
        // Small facts from inside (page count, first-pages text, image size), once per content.
        if matches!(first.kind.as_str(), "pdf" | "image") {
            let dir = crate::extract::Extract::dir(&cfg.cache_dir);
            if crate::extract::Extract::load(&dir, sha).is_none() {
                crate::extract::extract(Path::new(&first.path), &first.kind, first.size).save(&dir, sha)?;
            }
        }
    }
    for kind in [project::KIND_PROJECT, project::KIND_PACKAGE, project::KIND_DATASET] {
        let units: Vec<UnitRecord> = w.units.iter().filter(|u| u.kind == kind).cloned().collect();
        project::save_units(&catalog.dir.join(project::dir_name(kind)), &units, &now)?;
    }

    let seen_paths: HashSet<String> = w.files.iter().map(|s| s.path.clone()).collect();
    cache.prune(&roots, &seen_paths);
    std::fs::create_dir_all(&cfg.cache_dir).map_err(|e| CatalogError::io(&cfg.cache_dir, e))?;
    cache.save()?;

    let rep = report::build(&roots, &w, started, chrono::Utc::now());
    let (md, json) = report::write(&catalog.reports_dir(), &rep)?;
    Ok(ScanOutcome { report: rep, report_md: md, report_json: json, catalog: catalog.dir })
}

/// Dataless (content evicted to iCloud; opening it downloads it).
pub fn is_dataless(md: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        const SF_DATALESS: u32 = 0x4000_0000;
        std::os::macos::fs::MetadataExt::st_flags(md) & SF_DATALESS != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = md;
        false
    }
}

/// iCloud placeholder name (`.<name>.icloud`).
pub fn is_placeholder_name(name: &str) -> bool {
    name.ends_with(".icloud")
}

fn walk_dir(dir: &Path, root: &Path, top: Option<&str>, cache: &mut HashCache, w: &mut Walk) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            w.skipped.push(Skipped { path: dir.to_string_lossy().into(), reason: SkipReason::Unreadable });
            return;
        }
    };
    let mut entries: Vec<(String, PathBuf, Option<std::fs::Metadata>)> =
        rd.filter_map(|e| e.ok()).map(|e| (e.file_name().to_string_lossy().to_string(), e.path(), std::fs::symlink_metadata(e.path()).ok())).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    if dir != root {
        let names: Vec<(String, bool)> = entries.iter().map(|(n, _, m)| (n.clone(), m.as_ref().is_some_and(|m| m.is_dir()))).collect();
        let markers = project::markers(&names);
        if !markers.is_empty() {
            if let Ok(md) = std::fs::symlink_metadata(dir) {
                let p = project::inspect(dir, markers, &names, &md, &w.now);
                w.units.push(p);
            }
            return;
        }
        let tally = Tally::of(dir);
        if tally.is_dataset() {
            if let Ok(md) = std::fs::symlink_metadata(dir) {
                let d = project::inspect_dataset(dir, &tally, &names, &md, &w.now);
                w.units.push(d);
            }
            return;
        }
    }

    for (name, path, md) in entries {
        let p = path.to_string_lossy().to_string();
        if name == ".DS_Store" {
            w.ds_store += 1;
            continue;
        }
        let Some(md) = md else {
            w.skipped.push(Skipped { path: p, reason: SkipReason::Unreadable });
            continue;
        };
        let ft = md.file_type();
        if ft.is_symlink() {
            w.skipped.push(Skipped { path: p, reason: SkipReason::Symlink });
            continue;
        }
        if is_placeholder_name(&name) || is_dataless(&md) {
            w.skipped.push(Skipped { path: p, reason: SkipReason::Placeholder });
            continue;
        }
        let child_top = top.map(str::to_string).or_else(|| ft.is_dir().then(|| name.clone()));
        if ft.is_dir() {
            if project::is_package(&name) {
                let u = project::inspect_package(&path, &md, &w.now);
                w.units.push(u);
                continue;
            }
            walk_dir(&path, root, child_top.as_deref(), cache, w);
            continue;
        }
        if !ft.is_file() {
            w.skipped.push(Skipped { path: p, reason: SkipReason::Special });
            continue;
        }
        if let Some(s) = file(&path, &name, &md, cache, w) {
            w.files.push(Seen { root: root.to_string_lossy().into(), top: child_top, ..s });
        } else {
            w.skipped.push(Skipped { path: p, reason: SkipReason::Unreadable });
        }
    }
}

fn file(path: &Path, name: &str, md: &std::fs::Metadata, cache: &mut HashCache, w: &mut Walk) -> Option<Seen> {
    let key = FileKey::of(md);
    let ext_kind = kind_from_ext(name);
    let entry = match cache.get(path, key) {
        Some(e) => {
            w.cached_files += 1;
            e.clone()
        }
        None => {
            let (sha256, head) = crate::hash::sha256_file(path).ok()?;
            w.hashed_files += 1;
            w.hashed_bytes += key.size;
            let sniffed = if ext_kind.is_none() { Some(kind_from_magic(&head).to_string()) } else { None };
            let kind = ext_kind.or(sniffed.as_deref()).unwrap_or("other");
            let doc_times = match kind {
                "pdf" => times::pdf_dates(path, key.size),
                _ if is_ooxml(name) => times::ooxml_dates(path),
                _ => vec![],
            };
            let e = CacheEntry { key, sha256, sniffed, doc_times };
            cache.put(path, e.clone());
            e
        }
    };
    let kind = ext_kind.map(str::to_string).or(entry.sniffed.clone()).unwrap_or_else(|| "other".into());
    let mut t = times::fs_times(md);
    let (xt, download) = times::xattr_info(path);
    t.extend(xt);
    t.extend(entry.doc_times.iter().cloned());
    Some(Seen {
        path: path.to_string_lossy().into(),
        name: name.into(),
        root: String::new(),
        top: None,
        size: key.size,
        sha256: entry.sha256,
        kind,
        times: t,
        download,
    })
}

fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_ooxml(name: &str) -> bool {
    matches!(ext(name).as_str(), "docx" | "docm" | "pptx" | "pptm" | "xlsx" | "xlsm" | "dotx" | "potx" | "xltx")
}

/// Kind from the extension (`None` = unknown, sniff the content).
pub fn kind_from_ext(name: &str) -> Option<&'static str> {
    Some(match ext(name).as_str() {
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp" | "svg" | "dng" | "cr2" | "nef" | "arw" | "ico" => "image",
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" => "video",
        "mp3" | "m4a" | "wav" | "aac" | "flac" | "ogg" | "aif" | "aiff" | "opus" => "audio",
        "docx" | "docm" | "doc" | "dotx" | "odt" | "rtf" | "pages" => "document",
        "pptx" | "pptm" | "ppt" | "potx" | "key" | "odp" => "slides",
        "xlsx" | "xlsm" | "xls" | "xltx" | "csv" | "tsv" | "numbers" | "ods" => "spreadsheet",
        "txt" | "md" | "markdown" => "text",
        "tex" | "bib" | "cls" | "sty" | "bst" => "latex",
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => "archive",
        "dmg" | "pkg" | "iso" | "exe" | "msi" | "appimage" | "deb" | "rpm" => "installer",
        "epub" | "mobi" | "azw3" | "djvu" => "ebook",
        "json" | "xml" | "yaml" | "yml" | "toml" | "html" | "htm" | "ics" | "vcf" | "eml" | "sqlite" | "db" => "data",
        "py" | "rs" | "js" | "ts" | "c" | "cc" | "cpp" | "h" | "hpp" | "swift" | "java" | "m" | "ipynb" | "sh" | "r" | "jl" | "go" | "css" | "lua" => "code",
        "ttf" | "otf" | "woff" | "woff2" | "ttc" => "font",
        _ => return None,
    })
}

/// Kind from the first bytes, for files without a known extension.
pub fn kind_from_magic(head: &[u8]) -> &'static str {
    if head.starts_with(b"%PDF") {
        "pdf"
    } else if head.starts_with(b"\x89PNG") || head.starts_with(b"\xff\xd8\xff") || head.starts_with(b"GIF8") {
        "image"
    } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"\x1f\x8b") || head.starts_with(b"7z\xbc\xaf") {
        "archive"
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        "video"
    } else {
        "other"
    }
}
