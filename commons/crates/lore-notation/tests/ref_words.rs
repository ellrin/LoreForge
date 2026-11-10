//! A journal's reference words (`CheckOptions::ref_words`) in the checks.

use lore_notation::*;

fn check(text: &str, words: Option<RefWords>) -> Report {
    let opt = CheckOptions { ref_words: words, ..Default::default() };
    check_document(Document::from_str("main.tex", text), &Notation::default(), &opt)
}

const REFS: &str = r#"\begin{document}
\section{Intro}
\begin{figure}
\caption{Setup.}
\label{fig:a}
\end{figure}
\begin{table}
\caption{Numbers.}
\label{tab:a}
\end{table}
\begin{equation}
x = 1 \label{eq:a}
\end{equation}
The setup is in Figure~\ref{fig:a} and the numbers in Table~\ref{tab:a}.
Figure~\ref{fig:a} shows more, and Fig.~\ref{fig:a} again.
The value follows from Eq.~\eqref{eq:a}.
\end{document}
"#;

fn messages(r: &Report) -> Vec<String> {
    r.by_code("ref_word_style").iter().map(|i| i.message.clone()).collect()
}

#[test]
fn reference_words_follow_the_journal() {
    // no journal words: only mixed mid-sentence forms are noted (as before)
    let plain = check(REFS, None);
    assert!(plain.by_code("ref_word_style").is_empty());
    assert_eq!(plain.by_code("ref_prefix_style").len(), 1);

    // the journal writes "Fig." everywhere: both "Figure" uses are flagged
    let fig = check(REFS, Some(RefWords { fig: Some("Fig.".into()), ..Default::default() }));
    let issues = fig.by_code("ref_word_style");
    assert_eq!(issues.len(), 2, "{:?}", messages(&fig));
    assert!(issues.iter().all(|i| i.subject.as_deref() == Some("Figure")));
    assert!(fig.by_code("ref_prefix_style").is_empty(), "the journal's rule replaces the mixed-forms note");

    // "Figure" at the start of a sentence, "Fig." inside: only the mid-sentence "Figure" is wrong
    let ss = check(REFS, Some(RefWords { fig: Some("Fig.".into()), sentence_start: [("fig".to_string(), "Figure".to_string())].into(), ..Default::default() }));
    let m = messages(&ss);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(m[0].contains("句中"), "{}", m[0]);

    // the journal writes only \eqref: "Eq.~\eqref" is flagged; tables are not configured
    let eq = check(REFS, Some(RefWords { eq: Some(String::new()), ..Default::default() }));
    let issues = eq.by_code("ref_word_style");
    assert_eq!(issues.len(), 1, "{:?}", messages(&eq));
    assert_eq!(issues[0].subject.as_deref(), Some("Eq."));
}

#[test]
fn cleveref_journals_take_cref_without_a_word() {
    let text = REFS.replace("Figure~\\ref{fig:a} shows more, and Fig.~\\ref{fig:a} again.", "\\Cref{fig:a} shows more, and \\cref{fig:a} again, unlike Fig. 2.");
    let cl = || Some(RefWords { mode: Some("cleveref".into()), ..Default::default() });
    let c = check(&text, cl());
    let m = messages(&c);
    // "Figure~\ref", "Table~\ref", "Eq.~\eqref": words before plain refs; \Cref and \cref pass
    assert!(!m.is_empty() && m.iter().all(|m| m.contains("cleveref")), "{m:?}");
    // typed numbers are still caught next to \cref
    assert!(c.by_code("hardcoded_reference").iter().any(|i| i.subject.as_deref() == Some("Fig. 2")));
    // \cref counts as a reference
    assert!(c.by_code("label_unreferenced").is_empty());
    // a word in front of \cref prints the name twice
    let twice = check(&text.replace("\\cref{fig:a} again", "Fig.~\\cref{fig:a} again"), cl());
    assert!(messages(&twice).iter().any(|m| m.contains("兩次")), "{:?}", messages(&twice));
    // all-\cref text has nothing to say
    let clean = "\\begin{document}\n\\begin{figure}\n\\caption{A.}\n\\label{fig:a}\n\\end{figure}\n\\Cref{fig:a} shows it, as in \\cref{fig:a}.\n\\end{document}\n";
    assert!(check(clean, cl()).by_code("ref_word_style").is_empty());
}

#[test]
fn unreferenced_hint_uses_the_journal_word() {
    let text = "\\begin{document}\n\\begin{figure}\n\\caption{A.}\n\\label{fig:a}\n\\end{figure}\nNothing.\n\\end{document}\n";
    let d = check(text, None);
    assert!(d.by_code("label_unreferenced")[0].message.contains("Fig.~\\ref{fig:a}"));
    let j = check(text, Some(RefWords { fig: Some("Figure".into()), ..Default::default() }));
    assert!(j.by_code("label_unreferenced")[0].message.contains("Figure~\\ref{fig:a}"));
}
