//! The author's notation registry (lore-notation) for generated text: what
//! the drafting and revision prompts are told (macros, labels,
//! abbreviations), and the registry's checks run on what they write.
//!
//! The registry is `profile/notation.json` (the author's habits) merged with
//! `papers/<p>/notation.json` (this paper); the paper file wins.

use lore_notation::{check_document, CheckOptions, Document, Entry, Issue, Kind, Notation};
use std::path::Path;

/// Findings in generated text that must be fixed before it is kept.
const BLOCKING: &[&str] = &["hardcoded_reference", "ref_undefined"];
/// Findings worth showing the author next to the generated text.
const SHOWN: &[&str] = &[
    "hardcoded_reference",
    "ref_undefined",
    "abbrev_never_expanded",
    "abbrev_used_before_expansion",
    "undefined_symbol",
    "used_before_definition",
    "symbol_multiple_meanings",
];

pub fn load(workspace: &Path, paper: &Path) -> Notation {
    let global = Notation::load(&workspace.join("profile/notation.json")).unwrap_or_default();
    let doc = Notation::load(&paper.join("notation.json")).unwrap_or_default();
    Notation::effective(&global, &doc)
}

fn kind_entries(n: &Notation, k: Kind) -> Vec<&Entry> {
    n.entries.iter().filter(|e| e.kind() == k).collect()
}

/// Prompt text: the rules, then the registry (cut to a sensible size).
pub fn describe(n: &Notation) -> String {
    let mut s = String::from(
        "\n## The author's notation registry — follow it in every piece of LaTeX\n\
- Write a registered symbol with its macro when it has one (\\vw, not \\boldsymbol{w}), else with its registered LaTeX.\n\
- Refer to figures, tables, equations and sections only with \\ref{label} (Table~\\ref{tab:x}, Fig.~\\ref{fig:y}, \
Eq.~\\eqref{eq:z}); never type a number such as \"Table 1\" or \"Section 3\".\n\
- Expand an abbreviation at its first use in the body unless the registry marks it conventional.\n\
- A symbol the registry does not have must be defined in the sentence where it first appears; prefer an existing one.\n\
- Every piece of math, even a single symbol or an assignment such as $\\lambda = 0.5$, goes inside $…$.\n",
    );
    let line = |e: &Entry| -> String {
        let what = e.meaning_en.as_deref().or(e.meaning.as_deref()).unwrap_or("");
        match (e.macro_name(), e.latex.as_deref()) {
            (Some(m), Some(l)) => format!("\\{m} = {l} — {what}"),
            (Some(m), None) => format!("\\{m} — {what}"),
            (None, Some(l)) => format!("{l} — {what}"),
            (None, None) => what.to_string(),
        }
    };
    let mut add = |title: &str, rows: Vec<String>| {
        if !rows.is_empty() {
            s += &format!("{title}:\n");
            for r in rows.into_iter().take(80) {
                s += &format!("- {r}\n");
            }
        }
    };
    add("Symbols", kind_entries(n, Kind::Symbol).into_iter().map(line).collect());
    add("Operators", kind_entries(n, Kind::Operator).into_iter().map(line).collect());
    add(
        "Equations (label — body)",
        kind_entries(n, Kind::Equation).into_iter().filter_map(|e| Some(format!("{} — {}", e.label.as_deref()?, e.latex.as_deref().unwrap_or("")))).collect(),
    );
    add(
        "Figures and tables (label — caption)",
        n.entries
            .iter()
            .filter(|e| matches!(e.kind(), Kind::Figure | Kind::Table))
            .filter_map(|e| Some(format!("{} — {}", e.label.as_deref()?, e.caption.as_deref().unwrap_or(""))))
            .collect(),
    );
    add(
        "Abbreviations (short — long)",
        kind_entries(n, Kind::Abbrev)
            .into_iter()
            .filter_map(|e| Some(format!("{} — {}{}", e.short.as_deref()?, e.long.as_deref().unwrap_or("?"), e.macro_name().map(|m| format!(" (macro \\{m})")).unwrap_or_default())))
            .collect(),
    );
    if !n.conventional.is_empty() {
        s += &format!("Conventional (no definition or expansion needed): {}\n", n.conventional.join(", "));
    }
    s
}

/// What the registry's checks say about `text`, read in the context of the
/// paper (`main.tex` and its sections) plus `others` (generated text not in
/// the paper yet). Only findings located in `text` are returned.
pub struct Checked {
    pub blocking: Vec<String>,
    pub shown: Vec<String>,
    /// LaTeX of symbols the registry does not have.
    pub new_symbols: Vec<String>,
}

const FRAGMENT: &str = "sections/zz-generated.tex";

pub fn check(paper: &Path, n: &Notation, others: &[String], text: &str) -> Checked {
    let mut main = std::fs::read_to_string(paper.join("main.tex")).unwrap_or_else(|_| "\\documentclass{article}\n\\begin{document}\n\\end{document}\n".into());
    let mut files: Vec<(String, String)> = vec![];
    for f in pc_core::paper::section_files(paper) {
        if let Ok(t) = std::fs::read_to_string(paper.join("sections").join(&f)) {
            files.push((format!("sections/{f}"), t));
        }
    }
    let mut extra = String::new();
    for (i, o) in others.iter().enumerate() {
        let name = format!("sections/zz-other-{i}.tex");
        extra += &format!("\\input{{{}}}\n", name.trim_end_matches(".tex"));
        files.push((name, o.clone()));
    }
    extra += &format!("\\input{{{}}}\n", FRAGMENT.trim_end_matches(".tex"));
    files.push((FRAGMENT.into(), text.to_string()));
    match main.rfind("\\end{document}") {
        Some(i) => main.insert_str(i, &extra),
        None => main += &extra,
    }
    let mut all = vec![("main.tex".to_string(), main)];
    all.extend(files);
    let report = check_document(Document::from_files(&all), n, &CheckOptions::default());
    let here = |i: &&Issue| i.locations.iter().any(|l| l.file == FRAGMENT);
    let msg = |i: &Issue| i.message.clone();
    let blocking = report.issues.iter().filter(here).filter(|i| BLOCKING.contains(&i.code.as_str())).map(msg).collect();
    let shown = report.issues.iter().filter(here).filter(|i| SHOWN.contains(&i.code.as_str())).map(msg).collect();
    let mut new_symbols: Vec<String> = report
        .issues
        .iter()
        .filter(here)
        .filter(|i| i.code == "unregistered_symbol")
        .flat_map(|i| if i.subjects.is_empty() { i.subject.clone().into_iter().collect() } else { i.subjects.clone() })
        .collect();
    new_symbols.sort();
    new_symbols.dedup();
    Checked { blocking, shown, new_symbols }
}

/// Add symbols to the paper's own registry, tagged `suggested` for review.
/// Returns how many were new.
pub fn register(paper: &Path, latex: &[String]) -> Result<usize, String> {
    let path = paper.join("notation.json");
    let mut n = Notation::load(&path)?;
    let mut added = 0;
    for l in latex.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if n.find_latex(l).is_some() {
            continue;
        }
        let mut e = Entry::new(&n.free_id("s"), Kind::Symbol);
        e.latex = Some(l.to_string());
        e.tags = vec!["suggested".into()];
        n.upsert(e);
        added += 1;
    }
    if added > 0 {
        n.save(&path)?;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paper() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pc-notation-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(d.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        std::fs::write(
            d.join("sections/results.tex"),
            "\\section{Results}\nThe weight $\\vw$ scales each frame.\n\n\\begin{table}\n\\caption{Main comparison.}\n\\label{tab:main}\n\\begin{tabular}{l}\nx\n\\end{tabular}\n\\end{table}\n",
        )
        .unwrap();
        std::fs::write(
            d.join("notation.json"),
            r#"{"version":1,"entries":[{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"vw","meaning_en":"frame weight"},{"id":"t1","kind":"table","label":"tab:main","caption":"Main comparison"},{"id":"cv","kind":"abbrev","short":"CV","long":"cross-validation"}]}"#,
        )
        .unwrap();
        d
    }

    #[test]
    fn prompts_carry_the_rules_and_the_registry() {
        let p = paper();
        let n = load(&p, &p);
        let d = describe(&n);
        assert!(d.contains("\\vw = \\boldsymbol{w} — frame weight"));
        assert!(d.contains("tab:main — Main comparison") && d.contains("CV — cross-validation"));
        assert!(d.contains("never type a number"));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn typed_references_block_and_new_symbols_are_found() {
        let p = paper();
        let n = load(&p, &p);
        let bad = check(&p, &n, &[], "Table 1 shows that $\\vw$ and the new margin $\\gamma$ matter.\n");
        assert!(!bad.blocking.is_empty(), "a typed 'Table 1' must be fixed");
        let good = check(&p, &n, &[], "Table~\\ref{tab:main} shows that $\\vw$ matters.\n");
        assert!(good.blocking.is_empty(), "{:?}", good.blocking);
        assert_eq!(register(&p, &bad.new_symbols).unwrap(), bad.new_symbols.len());
        let reg = Notation::load(&p.join("notation.json")).unwrap();
        for s in &bad.new_symbols {
            assert!(reg.find_latex(s).is_some_and(|e| e.has_tag("suggested")));
        }
        assert_eq!(register(&p, &bad.new_symbols).unwrap(), 0, "registered once");
        let _ = std::fs::remove_dir_all(&p);
    }
}
