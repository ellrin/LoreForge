//! The synthetic sample paper in `examples/sample` exercises every check.

use std::collections::BTreeSet;
use std::path::PathBuf;
use ym_notation::*;

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sample")
}

fn registry() -> Notation {
    Notation::load(&sample().join("notation.json")).unwrap()
}

#[test]
fn every_check_fires_on_the_sample() {
    let doc = Document::load(&sample()).unwrap();
    assert_eq!(doc.files, vec!["main.tex", "sections/introduction.tex", "sections/method.tex", "sections/results.tex"], "generated notation.tex is skipped");
    let r = check_document(doc, &registry(), &CheckOptions::default());
    for code in [
        "undefined_symbol",
        "unused_definition",
        "unregistered_symbol",
        "unused_entry",
        "symbol_multiple_meanings",
        "meaning_multiple_symbols",
        "used_before_definition",
        "equation_undefined_variables",
        "abbrev_never_expanded",
        "abbrev_used_before_expansion",
        "abbrev_expanded_twice",
        "abbrev_abstract",
        "spelling_variants",
        "label_unreferenced",
        "ref_undefined",
        "label_duplicate",
        "hardcoded_reference",
        "caption_style",
        "ref_prefix_style",
        "registry_invalid",
    ] {
        assert!(!r.by_code(code).is_empty(), "{code} missing: {:#?}", r.summary);
    }
    // the caption uses \vw before the text defines it, with a jump location
    let early = r.by_code("used_before_definition");
    let w = early.iter().find(|i| i.subject.as_deref() == Some("\\boldsymbol{w}")).unwrap();
    assert_eq!(w.locations[0].file, "sections/method.tex");
    assert_eq!(w.locations[0].context, "caption");
    assert_eq!(w.related[0].line, 10);
    assert!(w.locations[0].sentence >= 1);
    // \speed (registry macro) counts as u: "widget speed" is u and v
    let same = r.by_code("meaning_multiple_symbols");
    assert!(same.iter().any(|i| i.message.contains("$u$") && i.message.contains("$v$")));
    // u is registered (through its macro), so it is not "unregistered"
    assert!(!r.by_code("unregistered_symbol").iter().any(|i| i.subject.as_deref() == Some("u")));
    // \speed is not "already defined": notation.tex is generated
    assert!(!r.by_code("registry_invalid").iter().any(|i| i.entry.as_deref() == Some("speed")));
    // JSON report round-trips through serde
    let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
    assert!(v["issues"].as_array().unwrap().len() == r.issues.len());
}

#[test]
fn changing_the_table_changes_the_macros_and_the_scan() {
    let mut reg = registry();
    let before = notation_tex(&reg, &BTreeSet::new());
    assert!(before.contains("\\newcommand{\\speed}{\\ensuremath{u}}"));
    reg.get_mut("speed").unwrap().latex = Some("\\nu".into());
    let after = notation_tex(&reg, &BTreeSet::new());
    assert!(after.contains("\\newcommand{\\speed}{\\ensuremath{\\nu}}"));
    // the text says \speed: it now reads as \nu
    let r = check_document(Document::load(&sample()).unwrap(), &reg, &CheckOptions::default());
    assert!(r.symbols.iter().any(|s| s.key == "\\nu" && s.entry.as_deref() == Some("speed")));
    assert!(!r.symbols.iter().any(|s| s.key == "u"));
    assert_eq!(katex_macros(&reg)["\\speed"], "\\nu");
}

#[test]
fn global_profile_import_and_merge() {
    let doc = registry();
    let mut global = Notation::default();
    let mut theta = Entry::new("theta", Kind::Symbol);
    theta.latex = Some("\\theta".into());
    theta.meaning = Some("模型參數".into());
    let mut w = Entry::new("weights", Kind::Symbol);
    w.latex = Some("\\bm{w}".into()); // same symbol as the document's \boldsymbol{w}
    w.meaning = Some("別的意思".into());
    global.entries = vec![theta, w];
    let merged = Notation::merged(&global, &doc);
    assert!(merged.iter().any(|m| m.entry.id == "theta" && m.origin == Origin::Global));
    assert!(!merged.iter().any(|m| m.entry.id == "weights"), "document wins for the same symbol");
    let mut d2 = doc.clone();
    let rep = d2.import(&global, None, Conflict::KeepDocument);
    assert_eq!(rep.added, vec!["theta"]);
    assert_eq!(rep.kept, vec!["weights"]);
    assert_eq!(d2.get("w").unwrap().meaning.as_deref(), Some("權重向量"));
    let mut d3 = doc.clone();
    let rep = d3.import(&global, Some(&["weights"]), Conflict::TakeGlobal);
    assert_eq!(rep.replaced, vec!["weights"]);
    // JSON keeps the field name "macro"
    let json = doc.to_json();
    assert!(json.contains("\"macro\": \"speed\""));
    assert_eq!(Notation::from_json(&json).unwrap(), doc);
}

#[test]
fn draft_from_the_sample() {
    let d = extract_document(Document::load(&sample()).unwrap());
    let n = &d.notation;
    assert!(n.find_latex("\\boldsymbol{w}").is_some_and(|e| e.meaning_en.as_deref() == Some("weight vector of the scorer")));
    assert!(n.entries.iter().any(|e| e.kind() == Kind::Equation && e.label.as_deref() == Some("eq:score")));
    assert!(n.entries.iter().any(|e| e.kind() == Kind::Table && e.label.as_deref() == Some("tab:main")));
    assert!(n.entries.iter().any(|e| e.short.as_deref() == Some("GR") && e.long.as_deref() == Some("gear ratio")));
}

fn check_text(text: &str, reg: &Notation) -> Report {
    check_document(Document::from_str("main.tex", text), reg, &CheckOptions::default())
}

const GROUPED: &str = r#"\begin{document}
\section{Intro}
We use a widget scoring network (WSN). Artificial widgets are often studied with AI.
The DCWSN variant and the Spin-NET tool help. A non-widget sample is rare.
\begin{figure}
\caption{The scorer $q$ reads $\boldsymbol{k}$ and a nonwidget sample.}
\label{fig:a}
\end{figure}
Fig.~\ref{fig:a} shows the setup. A non-widget case is common. Another non-widget case follows.
Then more words follow here. And still more words come next. The setup has many parts.
Let $q$ denote the scorer and $\boldsymbol{k}$ denote the key vector.
\begin{equation}
\label{eq:z}
y = q + \beta \boldsymbol{k}
\end{equation}
where $y$ is the output.
\begin{table}
\caption{Results.}
\begin{tabular}{cc}
Method & $\Delta$WSN \\
\end{tabular}
\end{table}
\end{document}
"#;

#[test]
fn findings_are_grouped_and_deduplicated() {
    let r = check_text(GROUPED, &Notation::default());
    // one caption, two symbols used before the text defines them → one issue
    let early = r.by_code("used_before_definition");
    assert_eq!(early.len(), 1, "{early:#?}");
    assert_eq!(early[0].subjects, vec!["q".to_string(), "\\boldsymbol{k}".to_string()]);
    assert!(early[0].message.contains("圖表說明") && early[0].locations.iter().all(|l| l.context == "caption"));
    // beta: one undefined_symbol issue that mentions the equation, no second equation issue
    assert!(r.by_code("equation_undefined_variables").is_empty(), "{:#?}", r.by_code("equation_undefined_variables"));
    let beta = r.by_code("undefined_symbol").into_iter().find(|i| i.subject.as_deref() == Some("\\beta")).unwrap();
    assert!(beta.details.iter().any(|d| d.contains("eq:z")) && beta.message.contains("denotes"));
    // a symbol only in a table header is a hint, not a warning
    let delta = r.by_code("undefined_symbol").into_iter().find(|i| i.subject.as_deref() == Some("\\Delta")).unwrap();
    assert_eq!(delta.severity, Severity::Info);
    // AI is well known; DCWSN (WSN + prefix) and Spin-NET look like names → hints
    let never = r.by_code("abbrev_never_expanded");
    assert!(!never.iter().any(|i| i.subject.as_deref() == Some("AI")));
    for name in ["DCWSN", "Spin-NET"] {
        let i = never.iter().find(|i| i.subject.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name}: {never:#?}"));
        assert_eq!(i.severity, Severity::Info, "{name}");
    }
    // spelling variants: counts, the fix, and the caption use tagged as caption
    let v = &r.by_code("spelling_variants")[0];
    assert!(v.message.contains("non-widget 3 次、nonwidget 1 次") && v.message.contains("統一寫成「non-widget」"), "{}", v.message);
    assert_eq!(v.locations[0].context, "caption");
    // an unreferenced equation label is only a hint
    let eq = r.by_code("label_unreferenced");
    assert!(eq.iter().all(|i| i.severity == Severity::Info) && !eq.is_empty());
}

#[test]
fn conventional_and_ignored_lists_silence_findings() {
    let mut reg = Notation::from_json(r#"{"version":1,"entries":[],"conventional":["DCWSN","\\Delta"],"ignored":[{"code":"undefined_symbol","subject":"\\beta"},{"code":"label_unreferenced"}]}"#).unwrap();
    let r = check_text(GROUPED, &reg);
    assert!(!r.by_code("abbrev_never_expanded").iter().any(|i| i.subject.as_deref() == Some("DCWSN")));
    assert!(!r.by_code("undefined_symbol").iter().any(|i| matches!(i.subject.as_deref(), Some("\\Delta" | "\\beta"))));
    assert!(r.by_code("label_unreferenced").is_empty());
    // the lists survive a round trip and merge with the global file
    let global = Notation::from_json(r#"{"version":1,"conventional":["Spin-NET"]}"#).unwrap();
    reg = Notation::from_json(&reg.to_json()).unwrap();
    let eff = Notation::effective(&global, &reg);
    assert!(eff.is_conventional("Spin-NET") && eff.is_conventional("DCWSN") && eff.is_ignored("label_unreferenced", Some("eq:z")));
    let r = check_text(GROUPED, &eff);
    assert!(!r.by_code("abbrev_never_expanded").iter().any(|i| i.subject.as_deref() == Some("Spin-NET")));
    // a registry without the lists writes none
    assert!(!Notation::default().to_json().contains("ignored"));
}
