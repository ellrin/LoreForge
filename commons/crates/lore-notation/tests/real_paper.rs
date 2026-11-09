//! Acceptance run on a real paper that must never enter the repository.
//!
//! ```sh
//! NOTATION_SAMPLE=/path/to/main.tex \
//! NOTATION_EXPECT=/path/to/expect.json \
//! cargo test -p lore-notation --test real_paper -- --ignored --nocapture
//! ```
//!
//! `NOTATION_EXPECT` (optional) is a JSON list of findings a human review
//! made: `[{"name": "...", "code": "used_before_definition", "subject": "G",
//! "contains": ["圖表說明"]}]`. A finding is found when an issue has that
//! code, that exact subject (if given; a grouped issue counts for each of its
//! `subjects`) and every `contains` string in its subject, message or details. The test prints the recall table and fails
//! below `NOTATION_MIN_RECALL` (default 0.8). Both files stay outside the repo.

use serde::Deserialize;
use std::path::PathBuf;
use lore_notation::*;

#[derive(Deserialize)]
struct Expect {
    name: String,
    code: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    contains: Vec<String>,
}

#[test]
#[ignore]
fn real_paper_recall() {
    let Ok(path) = std::env::var("NOTATION_SAMPLE") else {
        eprintln!("NOTATION_SAMPLE not set; skipped");
        return;
    };
    let doc = Document::load(&PathBuf::from(path)).expect("load paper");
    let report = check_document(doc, &Notation::default(), &CheckOptions::default());
    eprintln!("{} issues: {:?}", report.issues.len(), report.summary);
    let Ok(exp) = std::env::var("NOTATION_EXPECT") else {
        for i in &report.issues {
            eprintln!("  {} {}", i.code, i.message);
        }
        return;
    };
    let expect: Vec<Expect> = serde_json::from_str(&std::fs::read_to_string(exp).expect("read expect")).expect("expect.json");
    let mut used = vec![false; report.issues.len()];
    let mut found = 0;
    eprintln!("\n| finding | result |\n|---|---|");
    for e in &expect {
        let hit = report.issues.iter().position(|i| {
            let hay = format!("{} {} {}", i.subject.clone().unwrap_or_default(), i.message, i.details.join(" "));
            // a grouped issue (one caption, several symbols) covers each of its `subjects`
            let subject_ok = e.subject.as_ref().is_none_or(|s| i.subject.as_deref() == Some(s.as_str()) || i.subjects.contains(s));
            i.code == e.code && subject_ok && e.contains.iter().all(|c| hay.contains(c.as_str()))
        });
        if let Some(k) = hit {
            used[k] = true;
            found += 1;
        }
        eprintln!("| {} | {} |", e.name, if hit.is_some() { "found" } else { "MISSED" });
    }
    let recall = found as f64 / expect.len().max(1) as f64;
    eprintln!("\nrecall {found}/{} = {recall:.2}", expect.len());
    let warn = report.issues.iter().filter(|i| i.severity != Severity::Info).count();
    eprintln!("{} issues: {warn} warnings/errors, {} info", report.issues.len(), report.issues.len() - warn);
    eprintln!("\nnot in the review list (warnings and errors):");
    for (k, i) in report.issues.iter().enumerate() {
        if !used[k] && i.severity != Severity::Info {
            eprintln!("  {} — {}", i.code, i.message);
        }
    }
    let min: f64 = std::env::var("NOTATION_MIN_RECALL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.8);
    assert!(recall >= min, "recall {recall:.2} < {min}");
}
