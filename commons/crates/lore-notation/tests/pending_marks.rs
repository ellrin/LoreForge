//! 待補 marks: the checker raises nothing because of them, and a draft with
//! marks compiles (pdflatex, when installed) with the marks visible.

use lore_notation::pending::{self, MarkKind};
use lore_notation::*;

const MAIN: &str = "\\documentclass{article}\n\\usepackage{amsmath}\n__PRE__\\begin{document}\n\
\\begin{abstract}\nThe weighted loss raises Dice to \\pending{P1}{Dice on the external set}, \
consistent with the internal results\\pendingdep{P1}.\n\\end{abstract}\n\
\\section{Results}\\label{sec:res}\n\
On the internal set the weighted loss reached a Dice of 0.72 (Table~\\ref{tab:main}). \
The external set gives \\pending{P1}{Dice on the external set}, which keeps the ranking\\pendingdep{P1}.\n\n\
\\pendingpara{P2}{Compare the two sites; the per-site table goes after Table~\\ref{tab:main}.}\n\n\
\\begin{table}[t]\n\\centering\n\\caption{Dice per method; external values \\pending{P1}{pending}.}\\label{tab:main}\n\
\\begin{tabular}{lcc}\nMethod & Internal & External \\\\\nBaseline & 0.69 & \\pendingcell{P1}{Baseline / External} \\\\\n\
Weighted & 0.72 & \\pendingcell{P1}{Weighted / External} \\\\\n\\end{tabular}\n\\end{table}\n\n\
\\section{Conclusion}\nWeighting keeps its gain on a second site\\pendingdep{P1}.\n\\end{document}\n";

fn codes(text: &str) -> Vec<String> {
    let doc = Document::from_files(&[("main.tex".to_string(), text.to_string())]);
    let mut c: Vec<String> = check_document(doc, &Notation::default(), &CheckOptions::default()).issues.iter().map(|i| format!("{} {:?}", i.code, i.subject)).collect();
    c.sort();
    c
}

#[test]
fn marks_add_no_findings_and_their_names_are_reserved() {
    let marked = MAIN.replace("__PRE__", "\\input{pending}\n");
    // the same text with every mark filled by a plain number
    let mut plain = marked.clone();
    for id in ["P1", "P2"] {
        plain = pending::fill(&plain, id, &|_| Some("0.70".into()));
    }
    plain = plain.replace("\\pendingpara{P2}{Compare the two sites; the per-site table goes after Table~\\ref{tab:main}.}", "Compare the two sites; the per-site table goes after Table~\\ref{tab:main}.");
    assert!(pending::marks(&plain).is_empty(), "{plain}");
    assert_eq!(codes(&marked), codes(&plain), "a mark is not an abbreviation, a symbol or a typed reference");
    assert!(!codes(&marked).iter().any(|c| c.contains("P1") || c.contains("P2")));
    for m in pending::MACROS {
        assert!(macros::check_macro_name(m, &Default::default()).is_err(), "{m} is reserved");
    }
    // what a mark names is a note for the author, not the paper's text: no abbreviation finding
    let auc = "\\documentclass{article}\n\\begin{document}\nThe model reaches \\pending{P3}{external AUC and ECE}.\n\\end{document}\n";
    assert!(codes(auc).is_empty(), "{:?}", codes(auc));
    let kinds: Vec<MarkKind> = pending::marks(&marked).iter().map(|m| m.kind).collect();
    assert_eq!(kinds.iter().filter(|k| **k == MarkKind::Dep).count(), 3, "abstract, results and conclusion carry their dependency");
}

#[test]
fn a_generated_pending_file_is_not_part_of_the_text() {
    let d = std::env::temp_dir().join(format!("lore-pending-doc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.tex"), MAIN.replace("__PRE__", "\\input{pending}\n")).unwrap();
    pending::write_tex(&d).unwrap();
    let doc = Document::load(&d).unwrap();
    assert_eq!(doc.files, vec!["main.tex"]);
    let _ = std::fs::remove_dir_all(&d);
}

/// Compiles the sample twice (no colour package; xcolor with options loaded by
/// the paper) and reads the PDF's text back. Skipped without pdflatex.
#[test]
fn a_draft_with_marks_compiles_and_shows_them() {
    let ok = |c: &str, v: &str| std::process::Command::new(c).arg(v).output().is_ok_and(|o| o.status.success());
    if !ok("pdflatex", "--version") || !ok("pdftotext", "-v") {
        eprintln!("pdflatex or pdftotext missing: skipped");
        return;
    }
    for (tag, pre) in [("plain", "\\input{pending}\n"), ("xcolor", "\\usepackage[table,dvipsnames]{xcolor}\n\\input{pending}\n")] {
        let d = std::env::temp_dir().join(format!("lore-pending-tex-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("main.tex"), MAIN.replace("__PRE__", pre)).unwrap();
        pending::write_tex(&d).unwrap();
        let out = std::process::Command::new("pdflatex").args(["-interaction=nonstopmode", "-halt-on-error", "main.tex"]).current_dir(&d).output().unwrap();
        assert!(out.status.success(), "{tag}: {}", String::from_utf8_lossy(&out.stdout));
        let log = std::fs::read_to_string(d.join("main.log")).unwrap();
        assert!(!log.contains("Undefined control sequence"), "{tag}");
        let txt = std::process::Command::new("pdftotext").args(["main.pdf", "-"]).current_dir(&d).output().unwrap();
        let txt = String::from_utf8_lossy(&txt.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(txt.contains("P1: Dice on the external set"), "{tag}: {txt}");
        assert!(txt.contains("PENDING P2"), "{tag}: {txt}");
        assert!(txt.contains("[P1]"), "{tag}: {txt}");
        // LORE_PENDING_KEEP=<dir>: keep the PDFs to look at
        if let Some(keep) = std::env::var_os("LORE_PENDING_KEEP") {
            let _ = std::fs::copy(d.join("main.pdf"), std::path::Path::new(&keep).join(format!("pending-{tag}.pdf")));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
