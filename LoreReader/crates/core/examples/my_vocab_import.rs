//! Build "my vocabulary" entries from plain paragraphs, with the same normalisation and
//! n-grams as LoreReader's 「我的論文」 (no PDF, no model).
//!
//! cargo run -p lr-core --example my_vocab_import -- out.json <folder>…
//!
//! Each `<folder>` holds `paragraphs.jsonl`: one paragraph per line,
//! `{"section": "…", "units": [{"text": "…"}, …]}` (or `{"text": "…"}`). Sections named
//! References / Bibliography / Acknowledgments are skipped. The entries' source is
//! `import:<folder name>`; an existing `out.json` keeps its other sources.

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: my_vocab_import <out.json> <folder>…");
        std::process::exit(2);
    }
    let out = std::path::Path::new(&a[0]);
    let mut v = lr_core::myvocab::MyVocab::load(out);
    for dir in &a[1..] {
        let dir = std::path::Path::new(dir);
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let text = std::fs::read_to_string(dir.join("paragraphs.jsonl")).expect("paragraphs.jsonl");
        let mut paras = vec![];
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let p: serde_json::Value = serde_json::from_str(line).expect("json line");
            let section = p["section"].as_str().unwrap_or("").to_lowercase();
            if ["reference", "bibliograph", "acknowledg"].iter().any(|s| section.contains(s)) {
                continue;
            }
            let t = match p["units"].as_array() {
                Some(us) => us.iter().filter_map(|u| u["text"].as_str()).collect::<Vec<_>>().join(" "),
                None => p["text"].as_str().unwrap_or("").to_string(),
            };
            if !t.trim().is_empty() {
                paras.push(t);
            }
        }
        let words = lr_core::myvocab::vocabulary_of_paragraphs(&paras, &[]);
        let multi = words.keys().filter(|k| k.contains(' ')).count();
        eprintln!("import:{name}: {} paragraphs → {} entries ({} single words, {multi} multi-word)", paras.len(), words.len(), words.len() - multi);
        v.set_source(&format!("import:{name}"), Some(words));
    }
    v.save(out).expect("write");
    let multi = v.entries.iter().filter(|e| e.key.contains(' ')).count();
    eprintln!("{}: {} entries ({} single words, {multi} multi-word)", out.display(), v.entries.len(), v.entries.len() - multi);
}
