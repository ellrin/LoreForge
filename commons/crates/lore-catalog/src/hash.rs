//! Streaming sha256 and the incremental hash cache.
//!
//! The cache maps a path to what was learned from reading the file the last
//! time (sha256, kind sniffed from the first bytes, document dates), valid
//! while (size, mtime in ns, inode) are unchanged; a rescan of an unchanged
//! folder opens no file. It lives in the cache tier
//! (`<cache>/LoreForge/shelf/hash-cache.json`) and can be deleted any time.

use crate::CatalogError;
use crate::record::TimePoint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const CACHE_FILE: &str = "hash-cache.json";
/// Bytes kept from the start of a file for kind sniffing.
pub const HEAD_LEN: usize = 16;

/// sha256 (lower-case hex) of a file, read in 1 MiB blocks, and its first bytes.
pub fn sha256_file(path: &Path) -> std::io::Result<(String, Vec<u8>)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut head = Vec::new();
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if head.len() < HEAD_LEN {
            let take = (HEAD_LEN - head.len()).min(n);
            head.extend_from_slice(&buf[..take]);
        }
        h.update(&buf[..n]);
    }
    Ok((hex::encode(h.finalize()), head))
}

pub fn sha256_bytes(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// The identity of a file's state on disk: when it changes, re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileKey {
    pub size: u64,
    pub mtime_ns: i64,
    pub inode: u64,
}

impl FileKey {
    pub fn of(md: &std::fs::Metadata) -> FileKey {
        let mtime_ns = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(md);
        #[cfg(not(unix))]
        let inode = 0;
        FileKey { size: md.len(), mtime_ns, inode }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheEntry {
    #[serde(flatten)]
    pub key: FileKey,
    pub sha256: String,
    /// Kind sniffed from the first bytes (only when the extension says nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sniffed: Option<String>,
    /// Dates read from inside the file (PDF Info, OOXML core.xml).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_times: Vec<TimePoint>,
}

/// path → entry, loaded once per scan and written back atomically.
#[derive(Debug, Default)]
pub struct HashCache {
    pub file: Option<PathBuf>,
    pub entries: BTreeMap<String, CacheEntry>,
}

impl HashCache {
    /// Load `<dir>/hash-cache.json`; a missing or unreadable cache is empty.
    pub fn load(dir: &Path) -> HashCache {
        let file = dir.join(CACHE_FILE);
        let entries = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        HashCache { file: Some(file), entries }
    }

    pub fn get(&self, path: &Path, key: FileKey) -> Option<&CacheEntry> {
        self.entries.get(&path.to_string_lossy().to_string()).filter(|e| e.key == key)
    }

    pub fn put(&mut self, path: &Path, entry: CacheEntry) {
        self.entries.insert(path.to_string_lossy().to_string(), entry);
    }

    /// Drop entries under `roots` that are not in `seen` (deleted or moved files).
    pub fn prune(&mut self, roots: &[PathBuf], seen: &std::collections::HashSet<String>) {
        self.entries.retain(|p, _| seen.contains(p) || !roots.iter().any(|r| Path::new(p).starts_with(r)));
    }

    pub fn save(&self) -> Result<(), CatalogError> {
        let Some(file) = &self.file else { return Ok(()) };
        let bytes = serde_json::to_vec(&self.entries).map_err(|e| CatalogError::io(file, std::io::Error::other(e)))?;
        crate::store::write_atomic(file, &bytes)
    }
}
