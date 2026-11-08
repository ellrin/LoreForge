//! ym-notation — the notation registry (對照表) shared by PaperComposer and
//! SlideComposer: `notation.json`, macro generation (`notation.tex`), LaTeX
//! scanning, consistency checks and auto-extract. Pure Rust, no network.
//!
//! ```no_run
//! use ym_notation::*;
//! let doc = Document::load(std::path::Path::new("papers/demo")).unwrap();
//! let reg = Notation::load(std::path::Path::new("papers/demo/notation.json")).unwrap();
//! let report = check_document(doc, &reg, &CheckOptions::default());
//! println!("{}", report.to_json());
//! ```

pub mod abbrev;
pub mod check;
pub mod extract;
pub mod macros;
pub mod math;
pub mod model;
pub mod scan;
pub mod source;
pub mod symbols;

pub use check::{check, CheckOptions, Issue, Report, Severity};
pub use extract::{draft, refine, Draft, MockAssistant, NotationAssistant, Suggestion};
pub use macros::{insert_text, katex_macros, notation_tex, write_notation_tex, InsertMode};
pub use model::{Conflict, Entry, Kind, MergedEntry, Notation, Origin};
pub use scan::{Scan, ScanOptions};
pub use source::{Document, Loc};

/// Scan with the registry's macros expanded, then run every check.
pub fn check_document(doc: Document, notation: &Notation, opt: &CheckOptions) -> Report {
    let scan = scan_document(doc, notation, opt);
    check::check(&scan, notation, opt)
}

/// Scan a document; registry macros (`\ribmask`) count as what they stand for.
pub fn scan_document(doc: Document, notation: &Notation, opt: &CheckOptions) -> Scan {
    Scan::new(doc, ScanOptions { index_letters: opt.index_letters.clone(), extra_macros: macros::scan_macros(notation) })
}

/// Draft registry from a document (heuristics only).
pub fn extract_document(doc: Document) -> Draft {
    let scan = Scan::new(doc, ScanOptions::default());
    let syms = symbols::analyze(&scan, &Default::default());
    extract::draft(&scan, &syms)
}

/// Write a file atomically (temp file in the same folder, then rename).
pub fn write_atomic(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("file"), std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}
