//! Check a paper and write the report (and optionally a draft registry).
//!
//! ```sh
//! cargo run -p lore-notation --example check -- <paper folder or .tex> \
//!     [--notation notation.json] [--global profile/notation.json] \
//!     [--out report.json] [--extract draft.json] [--tex out-dir]
//! ```
//! The demo data of `ui/notation-panel/demo/` is made with
//! `--notation examples/sample/notation.json` on `examples/sample`.

use std::path::PathBuf;
use lore_notation::*;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path: Option<PathBuf> = None;
    let (mut notation, mut global, mut out, mut extract, mut tex) = (None, None, None, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--notation" => notation = args.next().map(PathBuf::from),
            "--global" => global = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--extract" => extract = args.next().map(PathBuf::from),
            "--tex" => tex = args.next().map(PathBuf::from),
            _ => path = Some(PathBuf::from(a)),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: check <paper> [--notation n.json] [--global g.json] [--out report.json] [--extract draft.json] [--tex dir]");
        std::process::exit(2);
    };
    let doc = Document::load(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let load = |p: &Option<PathBuf>| p.as_ref().map(|p| Notation::load(p).expect("notation.json")).unwrap_or_default();
    let reg = Notation::effective(&load(&global), &load(&notation));
    if let Some(p) = &extract {
        let d = extract_document(doc.clone());
        write_atomic(p, &serde_json::to_string_pretty(&d).unwrap()).expect("write draft");
        eprintln!("draft: {} entries → {}", d.notation.entries.len(), p.display());
    }
    if let Some(dir) = &tex {
        let existing = scan::document_macros(&doc.text).into_keys().collect();
        let p = write_notation_tex(dir, &reg, &existing).expect("write notation.tex");
        eprintln!("macros → {}", p.display());
    }
    let report = check_document(doc, &reg, &CheckOptions::default());
    for i in &report.issues {
        let at = i.locations.first().map(|l| format!("{}:{}", l.file, l.line)).unwrap_or_default();
        eprintln!("[{:?}] {} {} — {}", i.severity, i.code, at, i.message);
    }
    eprintln!("{} issues: {:?}", report.issues.len(), report.summary);
    match &out {
        Some(p) => write_atomic(p, &report.to_json()).expect("write report"),
        None => println!("{}", report.to_json()),
    }
}
