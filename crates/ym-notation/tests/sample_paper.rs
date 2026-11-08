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
