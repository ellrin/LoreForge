//! lr-core — LoreReader's core, no I/O beyond the library and the private workspace.
//!
//! - [`library`]: the PDF library. Always **outside** any git work tree (default
//!   `~/Documents/LoreReader/library`); PDFs are stored by sha256 (`ab/abcd….pdf`) and
//!   never modified. Exports (`Save as`) go to a separate folder with never-overwrite names.
//! - [`config`]: where the library, exports and the KnowledgeBase server are
//!   (env > `config.toml` in the app's settings folder > defaults).
//! - [`papers`]: per-paper metadata in the private workspace
//!   (`private/LoreReader/papers/<sha256>/`): `paper.json`, `annotations.json`,
//!   `quotes.json` (citable sentences), `chat.json`. Writes are atomic.
//! - [`pdfops`]: non-destructive page operations (delete, duplicate, reorder, insert
//!   from another PDF, merge) — all one [`pdfops::compose`] call that builds a new PDF.
//! - [`text`]: the paper's text as pdf.js gives it (items with fonts and positions;
//!   UTF-16 offsets shared with the page).
//! - [`nouns`]: the noun list (track A): word inventory, KB n-gram matching,
//!   normalisation, word filters and the batched classification.
//! - [`symbols`]: variables and equations, step 1 (no model): math runs from pdf.js
//!   items (fonts, sizes, baselines, accents), glyph → LaTeX, display equations, the
//!   regex definition finder and the per-paper symbol table (lore-notation fields).
//! - [`synth`]: synthetic text PDFs for tests and demos (never a real paper).

pub mod config;
pub mod library;
pub mod papers;
pub mod pdfops;
pub mod symbols;
pub mod synth;
pub mod nouns;
pub mod text;

pub use config::Config;
pub use library::Library;
pub use papers::Papers;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("找不到：{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("PDF：{0}")]
    Pdf(String),
}

impl From<lopdf::Error> for Error {
    fn from(e: lopdf::Error) -> Error {
        Error::Pdf(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::Invalid(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Write `bytes` to `path` through a temp file in the same folder + rename.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `dir/stem.ext`, or `stem-2.ext`, `stem-3.ext`… — a name that does not exist yet.
pub fn free_path(dir: &std::path::Path, stem: &str, ext: &str) -> std::path::PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem}-{n}.{ext}") };
        let p = dir.join(name);
        if p.symlink_metadata().is_err() {
            return p;
        }
        n += 1;
    }
}

/// A file-name-safe stem (no path separators or control characters, at most 80 chars).
pub fn safe_stem(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .chars()
        .take(80)
        .collect();
    if t.trim().is_empty() { "paper".into() } else { t.trim().to_string() }
}

/// Now as RFC 3339 with the local offset.
pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// A short unique id with a prefix (`a…` annotations, `q…` quotes).
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = chrono::Utc::now().timestamp_millis();
    format!("{prefix}{ms:x}{:03x}", N.fetch_add(1, Ordering::Relaxed) & 0xfff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_names_and_stems() {
        let d = tempfile::tempdir().unwrap();
        let a = free_path(d.path(), "x", "pdf");
        std::fs::write(&a, b"1").unwrap();
        let b = free_path(d.path(), "x", "pdf");
        assert_eq!(b.file_name().unwrap(), "x-2.pdf");
        assert_eq!(safe_stem("../a/b:c"), "_a_b_c");
        assert_eq!(safe_stem("  "), "paper");
        assert_ne!(new_id("q"), new_id("q"));
    }
}
