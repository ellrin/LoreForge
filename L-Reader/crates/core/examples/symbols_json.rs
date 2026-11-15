//! Run the symbol detection on a page-text JSON (the body of `POST /api/papers/{sha}/read`,
//! e.g. dumped from the reader page) and print the runs, equations and the table.
//! With `--tex source.tex`, compare the detected symbols with what lore-notation finds
//! in the paper's LaTeX source (precision / recall) and list the differences.
//!
//! cargo run -p lr-core --example symbols_json -- paper.json [--out table.json] [--tex paper.tex]

use std::collections::BTreeSet;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = a.first() else {
        eprintln!("usage: symbols_json <paper.json> [--out table.json] [--tex paper.tex]");
        std::process::exit(2);
    };
    let opt = |name: &str| a.iter().position(|x| x == name).and_then(|i| a.get(i + 1));
    let paper: lr_core::text::PaperText = serde_json::from_slice(&std::fs::read(input).expect("read")).expect("json");
    // as the server does: margin line numbers blanked before any analysis
    let mut paper = paper.normalise();
    lr_core::text::blank_line_numbers(&mut paper);
    let det = lr_core::symbols::detect(&paper, &lr_core::symbols::Glyphs::default());
    for s in &det.spans {
        println!("p{:<3} {:<6} {}", s.page, s.eq.map(|e| format!("eq{e}")).unwrap_or_default(), s.latex);
    }
    for q in &det.equations {
        println!("EQ {} p{} ({:?}) {}", q.id, q.page, q.number, q.latex);
    }
    let t = lr_core::symbols::build(&paper, &det, "local");
    for e in &t.entries {
        println!("{:<10} {:>3}× {:<28} {}", e.grade, e.occurrences.len(), e.entry.latex.clone().unwrap_or_default(), e.entry.meaning_en.clone().unwrap_or_default());
    }
    if let Some(out) = opt("--out") {
        std::fs::write(out, serde_json::to_vec_pretty(&t).unwrap()).expect("write");
    }
    if let Some(tex) = opt("--tex") {
        let truth = lr_core::symbols::source_keys(&std::fs::read_to_string(tex).expect("tex"));
        let found = lr_core::symbols::detected_keys(&det);
        let tp: BTreeSet<_> = found.intersection(&truth).collect();
        let fp: Vec<_> = found.difference(&truth).collect();
        let fn_: Vec<_> = truth.difference(&found).collect();
        println!("\nsymbols: truth {} found {} tp {} precision {:.3} recall {:.3}", truth.len(), found.len(), tp.len(), tp.len() as f64 / found.len().max(1) as f64, tp.len() as f64 / truth.len().max(1) as f64);
        println!("false positives ({}): {:?}", fp.len(), fp);
        println!("missed ({}): {:?}", fn_.len(), fn_);
        let nums: Vec<_> = det.equations.iter().filter_map(|q| q.number.clone()).collect();
        println!("numbered equations found ({}): {:?}", nums.len(), nums);
    }
}
