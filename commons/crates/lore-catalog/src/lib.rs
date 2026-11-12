//! lore-catalog — a read-only file catalog (LoreShelf wave 1).
//!
//! Walks folders the author names and records every file by its content
//! (sha256): names and paths it was seen at, time axes with their sources
//! (file system, download xattrs, document dates), download URLs, duplicates.
//! Folders with project markers (`.git/`, `main.tex`, `.tex` + `.bib`,
//! `Cargo.toml`, `package.json`, `*.xcodeproj`, `pyproject.toml`) are one
//! record each and are not descended into.
//!
//! Rules:
//! - never modifies, moves, renames or writes into a scanned folder; never
//!   follows symlinks; never opens an iCloud placeholder (`*.icloud`) or a
//!   dataless file (that would download it); skips `.DS_Store`;
//! - the catalog lives only on this computer ([`store::Guard`]); hashes are
//!   cached in the cache tier and can be wiped;
//! - knows nothing about any app. Directories are passed in explicitly; only
//!   [`store::default_catalog_dir`] / [`store::default_cache_dir`] read the
//!   environment.

pub mod category;
pub mod extract;
pub mod hash;
pub mod project;
pub mod record;
pub mod report;
pub mod scan;
pub mod store;
pub mod times;

pub use record::{Download, FileRecord, SeenAt, TimePoint};
pub use report::Report;
pub use scan::{ScanConfig, ScanOutcome, scan};
pub use store::{Catalog, Guard, LOCAL_ONLY_MARKER};

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The catalog location is not allowed (cloud, repo, remote) or git failed.
    #[error("{0}")]
    Guard(String),
    /// A scan root cannot be read (missing, or macOS privacy settings deny it).
    #[error("無法讀取 {path}：{source}")]
    Root { path: PathBuf, source: std::io::Error },
    #[error("{path}：{source}")]
    Io { path: PathBuf, source: std::io::Error },
}

impl CatalogError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> CatalogError {
        CatalogError::Io { path: path.to_path_buf(), source }
    }
}

#[cfg(test)]
mod tests;
