//! Units: folders catalogued as one record each and never hashed file by file.
//!
//! - **project** — a folder with markers: `.git` (folder or file), `main.tex`,
//!   a `.tex` together with a `.bib`, `Cargo.toml`, `package.json`,
//!   `*.xcodeproj`, `pyproject.toml`. Its inside is never read except
//!   `.git/config` (remote URLs, credentials stripped), `.git/logs/HEAD`
//!   (timestamps only; names and e-mail addresses are never kept) and the root
//!   commit (`git rev-list --max-parents=0 HEAD`, read-only, no optional locks).
//! - **package** — a macOS package folder by extension ([`PACKAGE_EXTS`]:
//!   `.app`, `.framework`, `.bundle`, `.photoslibrary`, …).
//! - **dataset** — a folder holding more than [`DATASET_MIN_FILES`] files of
//!   which at least [`DATASET_MIN_SHARE`] share one extension.
//!
//! Packages and datasets are counted with `symlink_metadata` only (number of
//! files, bytes); no file in them is opened.
//!
//! Ids:
//! - git project with a commit: `proj-git-<root commit sha>`; two clones of
//!   the same repository are one record with two `seen_at` paths;
//! - otherwise `proj-` / `pkg-` / `data-` + the first 16 hex of
//!   sha256(resolved absolute path), plus a `fingerprint`: projects = marker
//!   types + sorted top-level names; packages = name + file count + bytes;
//!   datasets = main extension + file count + bytes + sorted top-level names.
//!   On a rescan, a unit at a new path whose fingerprint matches a stored
//!   record of the same kind whose every path has vanished is merged into that
//!   record (a moved folder keeps its record).

use crate::CatalogError;
use crate::record::{SeenAt, TimePoint, axis};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Inside a project nothing is read; these names are listed for the record.
pub const NEVER_READ: [&str; 4] = ["node_modules", "target", "build", ".git"];

/// Folder extensions Finder shows as one item (macOS packages).
pub const PACKAGE_EXTS: [&str; 27] = [
    "app", "appex", "framework", "bundle", "plugin", "kext", "prefpane", "saver", "qlgenerator", "mdimporter", "xpc", "photoslibrary", "musiclibrary",
    "imovielibrary", "fcpbundle", "logicx", "band", "xcarchive", "playground", "rtfd", "pages", "key", "numbers", "pkg", "mpkg", "sparsebundle", "lpdf",
];

/// A dataset has more files than this…
pub const DATASET_MIN_FILES: u64 = 500;
/// …and at least this share of them has one extension.
pub const DATASET_MIN_SHARE: f64 = 0.9;

pub const KIND_PROJECT: &str = "project";
pub const KIND_PACKAGE: &str = "package";
pub const KIND_DATASET: &str = "dataset";

/// The catalog subfolder of each unit kind.
pub fn dir_name(kind: &str) -> &'static str {
    match kind {
        KIND_PACKAGE => "packages",
        KIND_DATASET => "datasets",
        _ => "projects",
    }
}

/// Markers found among a folder's entry names (empty = not a project).
pub fn markers(entries: &[(String, bool)]) -> Vec<String> {
    let mut m = Vec::new();
    let has = |n: &str| entries.iter().any(|(e, _)| e == n);
    if has(".git") {
        m.push("git".into());
    }
    let tex_bib = entries.iter().any(|(e, d)| !d && e.ends_with(".tex")) && entries.iter().any(|(e, d)| !d && e.ends_with(".bib"));
    if has("main.tex") || tex_bib {
        m.push("latex".into());
    }
    if has("Cargo.toml") {
        m.push("cargo".into());
    }
    if has("package.json") {
        m.push("npm".into());
    }
    if entries.iter().any(|(e, d)| *d && e.ends_with(".xcodeproj")) {
        m.push("xcode".into());
    }
    if has("pyproject.toml") {
        m.push("python".into());
    }
    m
}

/// Is this folder name a macOS package?
pub fn is_package(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, e)| !stem.is_empty() && PACKAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Files and bytes below a folder, by `symlink_metadata` only (no file is
/// opened; symlinks, placeholders and dataless entries are not followed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tally {
    pub files: u64,
    pub bytes: u64,
    /// lower-case extension ("" = none) → files.
    pub exts: BTreeMap<String, u64>,
}

impl Tally {
    pub fn of(dir: &Path) -> Tally {
        let mut t = Tally::default();
        t.add(dir);
        t
    }

    fn add(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.filter_map(|e| e.ok()) {
            let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if md.file_type().is_symlink() || crate::scan::is_dataless(&md) || crate::scan::is_placeholder_name(&name) || name == ".DS_Store" {
                continue;
            }
            if md.is_dir() {
                self.add(&e.path());
            } else if md.is_file() {
                self.files += 1;
                self.bytes += md.len();
                let ext = name.rsplit_once('.').filter(|(s, _)| !s.is_empty()).map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
                *self.exts.entry(ext).or_default() += 1;
            }
        }
    }

    /// (extension, share) of the most common extension.
    pub fn main_ext(&self) -> Option<(String, f64)> {
        let (e, n) = self.exts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
        Some((e.clone(), *n as f64 / self.files.max(1) as f64))
    }

    pub fn is_dataset(&self) -> bool {
        self.files > DATASET_MIN_FILES && self.main_ext().is_some_and(|(_, s)| s >= DATASET_MIN_SHARE)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitRecord {
    pub id: String,
    /// `project`, `package` or `dataset`.
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
    /// Remote URLs from `.git/config`, credentials stripped. Own vs cloned
    /// is decided later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<String>,
    /// Root commit of a git project (also its id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_commit: Option<String>,
    /// Matching key for moved folders (path-based ids only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Packages and datasets: file count, bytes, main extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_ext: Option<String>,
    #[serde(default)]
    pub seen_at: Vec<SeenAt>,
    #[serde(default)]
    pub times: Vec<TimePoint>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Kept for callers of the first version.
pub type ProjectRecord = UnitRecord;

fn path_hash(root: &Path) -> String {
    crate::hash::sha256_bytes(root.to_string_lossy().as_bytes())[..16].to_string()
}

pub fn project_id(root: &Path) -> String {
    format!("proj-{}", path_hash(root))
}

fn fingerprint(parts: &[String]) -> String {
    crate::hash::sha256_bytes(parts.join("\n").as_bytes())[..16].to_string()
}

fn top_names(entries: &[(String, bool)]) -> Vec<String> {
    let mut n: Vec<String> = entries.iter().map(|(e, _)| e.clone()).filter(|e| e != ".DS_Store").collect();
    n.sort();
    n
}

fn base(kind: &str, id: String, root: &Path, md: &std::fs::Metadata, now: &str) -> UnitRecord {
    UnitRecord {
        id,
        kind: kind.into(),
        name: root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        markers: vec![],
        remotes: vec![],
        root_commit: None,
        fingerprint: None,
        files: None,
        bytes: None,
        main_ext: None,
        seen_at: vec![SeenAt { path: root.to_string_lossy().to_string(), first_seen: now.into(), last_seen: now.into() }],
        times: crate::times::fs_times(md),
        extra: Map::new(),
    }
}

/// The record of the project at `root` (`entries` = its top-level names).
pub fn inspect(root: &Path, markers: Vec<String>, entries: &[(String, bool)], md: &std::fs::Metadata, now: &str) -> UnitRecord {
    let mut r = base(KIND_PROJECT, project_id(root), root, md, now);
    let git = root.join(".git");
    if git.is_dir() {
        if let Ok(s) = read_small(&git.join("config"), 256 << 10) {
            r.remotes = remote_urls(&s);
        }
        if let Ok(s) = read_small(&git.join("logs").join("HEAD"), 16 << 20) {
            let ts = log_timestamps(&s);
            if let (Some(a), Some(b)) = (ts.iter().min(), ts.iter().max()) {
                for (ax, t) in [(axis::GIT_FIRST, *a), (axis::GIT_LAST, *b)] {
                    if let Some(t) = Utc.timestamp_opt(t, 0).single() {
                        r.times.push(TimePoint::new(ax, t, "git:logs/HEAD"));
                    }
                }
            }
        }
    }
    if git.exists() {
        r.root_commit = root_commit(root);
    }
    match &r.root_commit {
        Some(c) => r.id = format!("proj-git-{c}"),
        None => {
            let mut parts = vec![KIND_PROJECT.to_string()];
            let mut m = markers.clone();
            m.sort();
            parts.push(m.join(","));
            parts.extend(top_names(entries));
            r.fingerprint = Some(fingerprint(&parts));
        }
    }
    r.markers = markers;
    r
}

/// A package folder: one record, counted but not opened.
pub fn inspect_package(root: &Path, md: &std::fs::Metadata, now: &str) -> UnitRecord {
    let mut r = base(KIND_PACKAGE, format!("pkg-{}", path_hash(root)), root, md, now);
    let t = Tally::of(root);
    let ext = root.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    r.fingerprint = Some(fingerprint(&[KIND_PACKAGE.into(), r.name.clone(), t.files.to_string(), t.bytes.to_string()]));
    r.markers = vec![ext];
    r.files = Some(t.files);
    r.bytes = Some(t.bytes);
    r
}

/// A dataset folder (caller checked [`Tally::is_dataset`]).
pub fn inspect_dataset(root: &Path, t: &Tally, entries: &[(String, bool)], md: &std::fs::Metadata, now: &str) -> UnitRecord {
    let mut r = base(KIND_DATASET, format!("data-{}", path_hash(root)), root, md, now);
    let ext = t.main_ext().map(|(e, _)| e).unwrap_or_default();
    let mut parts = vec![KIND_DATASET.to_string(), ext.clone(), t.files.to_string(), t.bytes.to_string()];
    parts.extend(top_names(entries));
    r.fingerprint = Some(fingerprint(&parts));
    r.files = Some(t.files);
    r.bytes = Some(t.bytes);
    r.main_ext = Some(ext);
    r
}

/// The root commit (lexically smallest when there are several), read-only:
/// `GIT_OPTIONAL_LOCKS=0`, no prompts, no config from the scanned repo's hooks.
pub fn root_commit(root: &Path) -> Option<String> {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| l.len() >= 40 && l.chars().all(|c| c.is_ascii_hexdigit())).min().map(str::to_string)
}

fn read_small(p: &Path, max: u64) -> std::io::Result<String> {
    use std::io::Read;
    let md = std::fs::symlink_metadata(p)?;
    if !md.is_file() {
        return Err(std::io::Error::other("not a file"));
    }
    let mut s = Vec::new();
    std::fs::File::open(p)?.take(max).read_to_end(&mut s)?;
    Ok(String::from_utf8_lossy(&s).to_string())
}

/// `url = …` lines of `[remote "…"]` sections, credentials stripped.
pub fn remote_urls(config: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_remote = false;
    for l in config.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_remote = l.starts_with("[remote ");
            continue;
        }
        if in_remote && let Some((k, v)) = l.split_once('=') && k.trim() == "url" {
            let u = strip_credentials(v.trim());
            if !out.contains(&u) {
                out.push(u);
            }
        }
    }
    out
}

/// `https://user:token@host/x` → `https://host/x`.
pub fn strip_credentials(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://")
        && let Some((auth, host)) = rest.split_once('@')
        && !auth.contains('/')
    {
        return format!("{scheme}://{host}");
    }
    url.to_string()
}

/// The host of a remote or download URL (`git@host:x/y`, `ssh://git@host:22/x`,
/// `https://host/x`, `blob:https://host/…`).
pub fn url_host(url: &str) -> Option<String> {
    let u = url.trim();
    let u = u.strip_prefix("blob:").unwrap_or(u);
    let rest = match u.split_once("://") {
        Some((_, r)) => r,
        None if u.contains('@') && u.contains(':') => u.split_once('@')?.1.split(':').next()?,
        None => return None,
    };
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = if host.starts_with('[') { host } else { host.split(':').next()? };
    let host = host.trim().to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Unix timestamps of a reflog (`old new Name <mail> 1700000000 +0000\tmsg`).
pub fn log_timestamps(log: &str) -> Vec<i64> {
    log.lines().filter_map(|l| l.split('\t').next()?.rsplit_once('>')?.1.split_whitespace().next()?.parse().ok()).collect()
}

impl UnitRecord {
    /// Merge a fresh inspection into the stored record: first_seen and
    /// unknown fields kept, paths added, remotes united, counts refreshed,
    /// git_first / git_last = earliest / latest over every clone.
    pub fn merge_from(&mut self, fresh: UnitRecord, now: &str) {
        if !fresh.markers.is_empty() {
            self.markers = fresh.markers;
        }
        for r in fresh.remotes {
            if !self.remotes.contains(&r) {
                self.remotes.push(r);
            }
        }
        self.name = fresh.name;
        self.root_commit = fresh.root_commit.or(self.root_commit.take());
        self.fingerprint = fresh.fingerprint.or(self.fingerprint.take());
        self.files = fresh.files.or(self.files);
        self.bytes = fresh.bytes.or(self.bytes);
        self.main_ext = fresh.main_ext.or(self.main_ext.take());
        for s in fresh.seen_at {
            match self.seen_at.iter_mut().find(|o| o.path == s.path) {
                Some(o) => o.last_seen = now.into(),
                None => self.seen_at.push(s),
            }
        }
        let git: Vec<TimePoint> = self.times.iter().chain(fresh.times.iter()).filter(|t| t.axis == axis::GIT_FIRST || t.axis == axis::GIT_LAST).cloned().collect();
        self.times.retain(|t| t.axis != axis::GIT_FIRST && t.axis != axis::GIT_LAST);
        for t in fresh.times {
            if t.axis != axis::GIT_FIRST && t.axis != axis::GIT_LAST && !self.times.contains(&t) {
                self.times.push(t);
            }
        }
        if let Some(first) = git.iter().filter(|t| t.axis == axis::GIT_FIRST).min_by(|a, b| a.value.cmp(&b.value)) {
            self.times.push(first.clone());
        }
        if let Some(last) = git.iter().filter(|t| t.axis == axis::GIT_LAST).max_by(|a, b| a.value.cmp(&b.value)) {
            self.times.push(last.clone());
        }
    }

    pub fn load(dir: &Path, id: &str) -> Option<UnitRecord> {
        serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).ok()?).ok()
    }

    /// Every stored record in `dir`.
    pub fn load_all(dir: &Path) -> Vec<UnitRecord> {
        let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
        let mut v: Vec<UnitRecord> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn save(&self, dir: &Path) -> Result<(), CatalogError> {
        let p = dir.join(format!("{}.json", self.id));
        let mut b = serde_json::to_vec_pretty(self).map_err(|e| CatalogError::io(&p, std::io::Error::other(e)))?;
        b.push(b'\n');
        crate::store::write_atomic(&p, &b)
    }
}

/// Store this scan's units of one kind: same id → merged (clones of one
/// repo, rescans); a new path-based id whose fingerprint matches a stored
/// record whose paths have all vanished → merged into that record (moved).
pub fn save_units(dir: &Path, fresh: &[UnitRecord], now: &str) -> Result<(), CatalogError> {
    let mut stored: BTreeMap<String, UnitRecord> = UnitRecord::load_all(dir).into_iter().map(|r| (r.id.clone(), r)).collect();
    let fresh_paths: std::collections::HashSet<&str> = fresh.iter().flat_map(|u| u.seen_at.iter().map(|s| s.path.as_str())).collect();
    let mut touched = Vec::new();
    for u in fresh {
        let mut target = u.id.clone();
        // A path already recorded on another record (a folder moved earlier).
        if !stored.contains_key(&target)
            && u.root_commit.is_none()
            && let Some(path) = u.seen_at.first().map(|s| s.path.as_str())
            && let Some(old) = stored.values().find(|o| o.kind == u.kind && o.root_commit.is_none() && o.seen_at.iter().any(|s| s.path == path))
        {
            target = old.id.clone();
        }
        if !stored.contains_key(&target)
            && u.root_commit.is_none()
            && let Some(fp) = &u.fingerprint
            && let Some(old) = stored.values().find(|o| {
                o.kind == u.kind
                    && o.fingerprint.as_ref() == Some(fp)
                    && !o.seen_at.is_empty()
                    && o.seen_at.iter().all(|s| !fresh_paths.contains(s.path.as_str()) && std::fs::symlink_metadata(&s.path).is_err())
            })
        {
            target = old.id.clone();
        }
        match stored.get_mut(&target) {
            Some(old) => old.merge_from(u.clone(), now),
            None => {
                stored.insert(target.clone(), u.clone());
            }
        }
        touched.push(target);
    }
    touched.sort();
    touched.dedup();
    for id in touched {
        stored[&id].save(dir)?;
    }
    Ok(())
}
