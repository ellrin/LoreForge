//! The punctuation, sentence and paragraph guard for every path that writes English prose
//! (lore-notation `punct`): drafts, the abstract, the writing panel, 我的風格 / 用字,
//! revisions and 回填. Off unless the workspace's `style/checks.toml` has `[punctuation]`
//! (the author's choice; the public default is neutral). With it, the sentence limits come
//! from the profile's rules.csv (`max_words`, `max_commas` rows of scope `sentence`) and the
//! paragraph ranges from `[paragraph_words.<Section>]` of the same checks.toml. The
//! document's own hyphenated terms (its references' titles and keys, its notation, its
//! experiment record) are allowed; the workspace guard (no paper) has only the whitelist.

use lore_notation::punct::{self, Guard, ParagraphRange};
use pc_core::profile::Rule;
use std::collections::BTreeMap;
use std::path::Path;

/// The checks file's raw text (empty when missing).
fn checks_raw(ws: &Path) -> String {
    std::fs::read_to_string(ws.join(pc_core::draft::STYLE_DIR).join(pc_core::stylecheck::FILE)).unwrap_or_default()
}

/// A sentence rule's threshold from rules.csv (`max_words`, `max_commas`).
pub fn sentence_limit(rules: &[Rule], check: &str) -> Option<usize> {
    rules.iter().find(|r| r.check == check && r.scope == "sentence").and_then(|r| r.threshold.trim().parse().ok())
}

/// The guard for a paper (or, with `paper` None, for the workspace: the writing panel).
pub fn load(ws: &Path, paper: Option<&Path>, rules: &[Rule]) -> Guard {
    let Some(mut rule) = punct::Rule::from_toml(&checks_raw(ws)) else { return Guard::default() };
    rule.max_words = sentence_limit(rules, "max_words");
    rule.max_commas = sentence_limit(rules, "max_commas");
    // the paper's own references (titles and keys, not the whole library or a sample .bib),
    // its notation and its experiment record
    let mut own = String::new();
    if let Some(p) = paper {
        for r in pc_core::draft::material::refs(ws, p) {
            own += &format!("\n{} {}", r.key, r.title);
        }
        own += &serde_json::to_string(&crate::notation::load(ws, p)).unwrap_or_default();
        // the experiment record's names (datasets, tools, settings)
        for e in std::fs::read_dir(pc_core::draft::session::draft_dir(p).join(pc_core::draft::session::RECORDS_DIR)).into_iter().flatten().flatten() {
            own += &std::fs::read_to_string(e.path()).unwrap_or_default();
        }
    }
    Guard { rule, exempt: punct::hyphenated_terms(&own) }
}

/// The paragraph ranges by section (`Introduction`, `Methods`, …); empty when not set.
pub fn paragraph_ranges(ws: &Path) -> BTreeMap<String, ParagraphRange> {
    if punct::Rule::from_toml(&checks_raw(ws)).is_none() {
        return BTreeMap::new();
    }
    punct::paragraph_ranges(&checks_raw(ws))
}

/// The outline's paragraph sizes: how much each paragraph should carry, by section.
pub fn outline_sizes(r: &BTreeMap<String, ParagraphRange>) -> String {
    if r.is_empty() {
        return String::new();
    }
    let mut s = "\n## Paragraph sizes (words, by section)\nGroup the points so that each paragraph would take this many words: merge \
points that make a thin paragraph, split a paragraph that carries several turns. Vary the sizes inside the range; never \
three neighbouring paragraphs of about the same size.\n".to_string();
    for (sec, x) in r {
        s += &format!("- {sec}: {}–{} words", x.target.0, x.target.1);
        if let Some((a, b)) = x.sentences {
            s += &format!(", {a}–{b} sentences");
        }
        s += "\n";
    }
    s
}

/// What is still wrong, as problems shown with the paragraph.
pub fn flags(v: &[punct::Violation]) -> Vec<String> {
    v.iter().map(|x| format!("標點／句長：{}", x.describe())).collect()
}

/// The paper's text broken by the guard, for 審稿 (warnings; the author's own text too).
pub fn audit(g: &Guard, paper: &Path) -> Vec<(String, String)> {
    if !g.active() {
        return vec![];
    }
    let mut out = vec![];
    for s in pc_core::open::sections(paper) {
        for (k, p) in s.paragraphs.iter().enumerate() {
            // floats, headings and other blocks are not prose
            let t = p.trim_start();
            if t.starts_with("\\begin{") || t.starts_with("\\section") || t.starts_with("\\subsection") || t.starts_with('%') {
                continue;
            }
            // one finding per paragraph, every violation in it
            let v = g.check(p);
            if !v.is_empty() {
                let what: Vec<String> = v.iter().map(|x| x.describe()).collect();
                out.push((format!("{} ¶{}", s.file, k + 1), format!("{} 處：{}", v.len(), what.join("；"))));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_without_the_authors_table_and_reads_his_limits() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ws.path().join("style")).unwrap();
        std::fs::create_dir_all(ws.path().join("refs")).unwrap();
        assert!(!load(ws.path(), None, &[]).active(), "no checks.toml: off");
        std::fs::write(ws.path().join("style/checks.toml"), "wordy = [\"a number of\"]\n").unwrap();
        assert!(!load(ws.path(), None, &[]).active(), "no [punctuation]: off");
        std::fs::write(
            ws.path().join("style/checks.toml"),
            "wordy = [\"a number of\"]\n[punctuation]\nallowed = [\".\", \",\"]\nhyphen_whitelist = [\"X-ray\"]\n[paragraph_words.Methods]\ntarget = [55, 120]\nbounds = [35, 170]\n",
        )
        .unwrap();
        std::fs::write(ws.path().join("refs/library.bib"), "@article{a, title = {Self-Supervised Chest Images}}\n@article{b, title = {Data-Driven Everything}}\n").unwrap();
        let paper = ws.path().join("papers/p");
        std::fs::create_dir_all(paper.join("sections")).unwrap();
        std::fs::write(paper.join("main.tex"), "\\documentclass{article}\n\\begin{document}\nAs in~\\cite{a}.\n\\end{document}\n").unwrap();
        let mut rules = vec![];
        for (id, check, t) in [("R002", "max_words", "35"), ("R217", "max_commas", "3")] {
            rules.push(Rule { id: id.into(), kind: pc_core::profile::RuleKind::Hard, scope: "sentence".into(), check: check.into(), threshold: t.into(), severity: "warning".into(), message: String::new() });
        }
        let g = load(ws.path(), Some(&paper), &rules);
        assert!(g.active());
        assert_eq!((g.rule.max_words, g.rule.max_commas), (Some(35), Some(3)));
        assert!(g.check("A self-supervised X-ray model.").is_empty(), "the paper's own reference and the whitelist");
        assert_eq!(g.check("A data-driven model: fast.").len(), 2, "a library entry the paper does not cite exempts nothing");
        assert_eq!(load(ws.path(), None, &rules).check("A self-supervised model.").len(), 1, "no paper: the whitelist only");
        assert_eq!(paragraph_ranges(ws.path())["Methods"].target, (55, 120));
        assert!(outline_sizes(&paragraph_ranges(ws.path())).contains("- Methods: 55–120 words"));
        assert!(outline_sizes(&BTreeMap::new()).is_empty());
        // the checks' own lists still load beside the new tables
        let (c, problems) = pc_core::stylecheck::StyleChecks::load(&ws.path().join("style/checks.toml"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.list("wordy").len(), 1);
    }

    fn author_guard() -> Guard {
        let rule = punct::Rule { allowed: vec!['.', ','], hyphen_whitelist: vec!["X-ray".into()], max_words: Some(35), max_commas: Some(3) };
        Guard { rule, exempt: punct::hyphenated_terms("Self-Supervised Chest Images") }
    }

    /// The model reply every path is tested with: a colon, an em dash, a semicolon, a compound.
    const BAD: &str = "The model is data-driven: it learns from labels — not rules; the X-ray encoder is self-supervised.";

    #[test]
    fn the_audit_groups_by_paragraph_and_skips_floats() {
        let ws = tempfile::tempdir().unwrap();
        let p = ws.path().join("p");
        std::fs::create_dir_all(p.join("sections")).unwrap();
        std::fs::write(p.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        std::fs::write(
            p.join("sections/results.tex"),
            "\\section{Results}\nOne: two; three — four.\n\n\\begin{figure}\n\\caption{A: b. C; d.}\n\\end{figure}\n\nClean text here.\n",
        )
        .unwrap();
        let a = audit(&author_guard(), &p);
        assert_eq!(a.len(), 1, "{a:?}");
        assert!(a[0].0.ends_with("¶1") && a[0].1.starts_with("3 處"), "{a:?}");
        assert!(audit(&Guard::default(), &p).is_empty());
    }

    #[test]
    fn every_generation_path_runs_through_the_guard() {
        let g = author_guard();
        let named = |t: &str| punct::repair_note(&g.check(t));
        // 1. section drafts and the abstract: the reply's sentences, joined, checked; the repair names each
        let reply = format!(r#"{{"sentences":[{{"text":"{BAD}","phrase_id":null}}]}}"#);
        let d = pc_core::compose::parse_draft(&reply).unwrap();
        let text = d.sentences.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join(" ");
        let note = named(&text);
        for want in ["colon", "dash \"—\"", "semicolon", "hyphenated \"data-driven\""] {
            assert!(note.contains(want), "{want} in {note}");
        }
        assert!(g.check(&text).iter().all(|v| v.found != "X-ray" && v.found != "self-supervised"), "whitelist and reference terms");
        let (fixed, still) = g.finish(&text);
        assert_eq!(fixed, "The model is data driven, it learns from labels, not rules. The X-ray encoder is self-supervised.");
        assert!(still.is_empty());
        // 2. the writing panel: the same, sentence by sentence
        let mut d2 = pc_core::compose::parse_draft(&reply).unwrap();
        assert_eq!(crate::guard_draft(&g, &mut d2, false).len(), 4);
        assert!(crate::guard_draft(&g, &mut d2, true).is_empty() && !d2.sentences[0].text.contains(':'));
        // 3. revisions and 回填 rewrites: mechanical fixes, what is left is said
        let long = format!("{} {}.", BAD, vec!["and more"; 20].join(" "));
        let (t, still) = g.finish(&long);
        assert!(!t.contains(':') && !t.contains(';') && !t.contains('—'));
        assert!(flags(&still).iter().any(|f| f.contains("words (at most 35)")), "a long sentence is flagged, never hidden");
        // 4. drafted captions (wizard, 繪圖 自動) and the flow caption
        assert_eq!(g.fix("Dice per site: internal vs. external — a data-driven view."), "Dice per site, internal vs. external, a data driven view.");
        // maths and citations stay as they are
        assert!(g.check("We minimise $L: x \\to y$ as in~\\cite{lee-2020}.").is_empty());
    }
}
