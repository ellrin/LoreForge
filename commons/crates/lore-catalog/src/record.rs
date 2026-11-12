//! One record per content: `files/<sha256>.json`.
//!
//! The same bytes at several paths are one record (duplicates). A rescan
//! merges into the existing record: names and paths are added, `first_seen`
//! is kept and `last_seen` moves on, times are added (deduplicated by axis,
//! value and source), and fields this version does not know are kept.

use crate::CatalogError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

/// Time axes (`TimePoint::axis`).
pub mod axis {
    /// File system birth time.
    pub const FS_CREATED: &str = "fs_created";
    pub const FS_MODIFIED: &str = "fs_modified";
    /// When it was downloaded (quarantine xattr).
    pub const DOWNLOADED: &str = "downloaded";
    /// Last opened (`com.apple.lastuseddate#PS`, kMDItemLastUsedDate).
    pub const LAST_OPENED: &str = "last_opened";
    /// Dates written inside the document (PDF Info, OOXML core.xml).
    pub const DOC_CREATED: &str = "doc_created";
    pub const DOC_MODIFIED: &str = "doc_modified";
    /// Projects: first and last entry of `.git/logs/HEAD`.
    pub const GIT_FIRST: &str = "git_first";
    pub const GIT_LAST: &str = "git_last";

    /// The axes of a file record, in report order.
    pub const FILE_AXES: [&str; 6] = [FS_CREATED, FS_MODIFIED, DOWNLOADED, LAST_OPENED, DOC_CREATED, DOC_MODIFIED];
}

/// One time on one axis, with where it came from (`fs`,
/// `xattr:com.apple.quarantine`, `xattr:com.apple.lastuseddate#PS`,
/// `pdf:Info`, `ooxml:docProps/core.xml`, `git:logs/HEAD`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimePoint {
    pub axis: String,
    /// RFC 3339, UTC.
    pub value: String,
    pub source: String,
}

impl TimePoint {
    pub fn new(axis: &str, t: chrono::DateTime<chrono::Utc>, source: &str) -> TimePoint {
        TimePoint { axis: axis.into(), value: t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true), source: source.into() }
    }
}

/// Where a download came from (`kMDItemWhereFroms`, quarantine agent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    /// The app that downloaded it (quarantine agent name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl Download {
    pub fn is_empty(&self) -> bool {
        self.urls.is_empty() && self.referrer.is_none() && self.agent.is_none()
    }

    fn merge(&mut self, o: &Download) {
        for u in &o.urls {
            if !self.urls.contains(u) {
                self.urls.push(u.clone());
            }
        }
        if o.referrer.is_some() {
            self.referrer.clone_from(&o.referrer);
        }
        if o.agent.is_some() {
            self.agent.clone_from(&o.agent);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenAt {
    pub path: String,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRecord {
    pub sha256: String,
    pub size: u64,
    pub kind: String,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub seen_at: Vec<SeenAt>,
    #[serde(default)]
    pub times: Vec<TimePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<Download>,
    /// Fields written by later versions or by hand; kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What one scan saw of one content at one path.
#[derive(Debug, Clone)]
pub struct Observation {
    pub path: String,
    pub name: String,
    pub times: Vec<TimePoint>,
    pub download: Download,
}

impl FileRecord {
    pub fn new(sha256: &str, size: u64, kind: &str) -> FileRecord {
        FileRecord { sha256: sha256.into(), size, kind: kind.into(), names: vec![], seen_at: vec![], times: vec![], download: None, extra: Map::new() }
    }

    /// Fold one observation in; `now` = the scan's start time (RFC 3339).
    pub fn observe(&mut self, o: &Observation, now: &str) {
        if !self.names.contains(&o.name) {
            self.names.push(o.name.clone());
        }
        match self.seen_at.iter_mut().find(|s| s.path == o.path) {
            Some(s) => s.last_seen = now.into(),
            None => self.seen_at.push(SeenAt { path: o.path.clone(), first_seen: now.into(), last_seen: now.into() }),
        }
        for t in &o.times {
            if !self.times.contains(t) {
                self.times.push(t.clone());
            }
        }
        if !o.download.is_empty() {
            self.download.get_or_insert_with(Download::default).merge(&o.download);
        }
    }

    pub fn has_axis(&self, axis: &str) -> bool {
        self.times.iter().any(|t| t.axis == axis)
    }

    pub fn path(dir: &Path, sha: &str) -> std::path::PathBuf {
        dir.join(format!("{sha}.json"))
    }

    /// The existing record, or `None` (an unreadable record is replaced — it
    /// is rebuilt from the files anyway).
    pub fn load(dir: &Path, sha: &str) -> Option<FileRecord> {
        let b = std::fs::read(Self::path(dir, sha)).ok()?;
        serde_json::from_slice(&b).ok()
    }

    pub fn save(&self, dir: &Path) -> Result<(), CatalogError> {
        let p = Self::path(dir, &self.sha256);
        let mut b = serde_json::to_vec_pretty(self).map_err(|e| CatalogError::io(&p, std::io::Error::other(e)))?;
        b.push(b'\n');
        crate::store::write_atomic(&p, &b)
    }
}
