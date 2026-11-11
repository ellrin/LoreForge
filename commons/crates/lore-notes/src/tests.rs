use super::*;
use lore_notation::{Entry, Kind as NKind};

fn at(s: &str) -> chrono::DateTime<chrono::FixedOffset> {
    chrono::DateTime::parse_from_rfc3339(s).unwrap()
}

fn registry() -> Notation {
    let mut w = Entry::new("w", NKind::Symbol);
    w.latex = Some("\\boldsymbol{w}".into());
    w.macro_name = Some("vw".into());
    w.meaning = Some("weight vector".into());
    let mut eq = Entry::new("loss", NKind::Equation);
    eq.latex = Some("L = \\sum_i \\ell_i".into());
    eq.label = Some("eq:loss".into());
    Notation { entries: vec![w, eq], ..Default::default() }
}

fn new(title: &str, body: &str) -> NewNote {
    NewNote { title: title.into(), body: body.into(), ..Default::default() }
}

#[test]
fn round_trip() {
    let n = Note {
        id: "2026-11-12-loss".into(),
        title: "Loss weights: why".into(),
        kind: Kind::Idea,
        source: "loreecho".into(),
        created: "2026-11-12T10:04:00+08:00".into(),
        updated: Some("2026-11-12T11:00:00+08:00".into()),
        kb: vec!["kb-1".into(), "kb-2".into()],
        tags: vec!["loss".into(), "training".into()],
        extra: [("lang".to_string(), "en".to_string())].into(),
        body: "We weight \\(\\vw\\) by \\ref{eq:loss}.\n\n% a comment\n\\begin{equation}\n  x = 1\n\\end{equation}".into(),
        modified: 0,
    };
    let text = to_tex(&n);
    assert!(text.starts_with("% ---\n% source: loreecho\n% created: 2026-11-12T10:04:00+08:00\n"));
    assert!(text.contains("% kind: idea\n% kb: kb-1, kb-2\n% tags: loss, training\n% title: Loss weights: why\n"));
    let back = parse("2026-11-12-loss", &text);
    assert_eq!(back, n);
    assert_eq!(to_tex(&back), text);
}

#[test]
fn hand_written_file_without_front_matter() {
    let n = parse("scratch", "Just text with $x$.\n");
    assert_eq!(n.title, "scratch");
    assert_eq!(n.kind, Kind::Note);
    assert_eq!(n.body, "Just text with $x$.");
    // an opened but never closed block is body, not front matter
    let n = parse("x", "% ---\n% title: a\nbody\n");
    assert_eq!(n.body, "% ---\n% title: a\nbody");
}

#[test]
fn slugs() {
    assert_eq!(slug("Loss weights: why?"), "loss-weights-why");
    assert_eq!(slug("損失函數的權重"), "損失函數的權重");
    assert_eq!(slug("  --- "), "");
    assert_eq!(slug(&"a".repeat(80)).chars().count(), 40);
    assert!(valid_id("2026-11-12-a"));
    for bad in ["", "../x", "a/b", ".trash", "a\\b", "a..b"] {
        assert!(!valid_id(bad), "{bad}");
    }
}

#[test]
fn unique_names_and_trash() {
    let d = tempfile::tempdir().unwrap();
    let notes = Notes::in_workspace(d.path());
    let t = at("2026-11-12T09:00:00+08:00");
    let a = notes.create_at(new("Same", "first"), &registry(), t).unwrap();
    let b = notes.create_at(new("Same", "second"), &registry(), t).unwrap();
    let c = notes.create_at(new("Same", "third"), &registry(), t).unwrap();
    assert_eq!(a.id, "2026-11-12-same");
    assert_eq!(b.id, "2026-11-12-same-2");
    assert_eq!(c.id, "2026-11-12-same-3");
    assert_eq!(notes.get(&a.id).unwrap().body, "first");
    // no title: from the body; nothing usable: the kind
    let i = notes.create_at(NewNote { body: "$x$".into(), kind: Kind::Idea, ..Default::default() }, &registry(), t).unwrap();
    assert_eq!(i.id, "2026-11-12-x");
    assert!(matches!(notes.create_at(new("", "  "), &registry(), t), Err(Error::Empty)));

    let to = notes.delete(&a.id).unwrap();
    assert_eq!(to, d.path().join("notes/.trash/2026-11-12-same.tex"));
    let again = notes.create_at(new("Same", "fourth"), &registry(), t).unwrap();
    assert_eq!(again.id, "2026-11-12-same");
    let to2 = notes.delete(&again.id).unwrap();
    assert_eq!(to2, d.path().join("notes/.trash/2026-11-12-same-2.tex"));
    assert!(matches!(notes.get("2026-11-12-same"), Err(Error::NotFound(_))));
    assert!(matches!(notes.delete("../x"), Err(Error::BadId(_))));
    // the trash is not listed
    assert_eq!(notes.list().len(), 3);
}

#[test]
fn list_newest_first_and_update() {
    let d = tempfile::tempdir().unwrap();
    let notes = Notes::new(d.path().join("notes"));
    assert!(notes.list().is_empty());
    let old = notes.create_at(new("Old", "a"), &registry(), at("2026-11-10T09:00:00+08:00")).unwrap();
    let mid = notes.create_at(NewNote { title: "Idea".into(), body: "b".into(), kind: Kind::Idea, source: Some("loreecho".into()), tags: vec!["x, y".into()], ..Default::default() }, &registry(), at("2026-11-11T09:00:00+08:00")).unwrap();
    std::fs::write(d.path().join("notes/readme.md"), "not a note").unwrap();
    let ids: Vec<String> = notes.list().into_iter().map(|n| n.id).collect();
    assert_eq!(ids, vec![mid.id.clone(), old.id.clone()]);
    let m = notes.get(&mid.id).unwrap();
    assert_eq!((m.kind, m.source.as_str(), m.tags.clone()), (Kind::Idea, "loreecho", vec!["x y".to_string()]));

    let s = notes.update_at(&old.id, Edit { title: Some("Renamed".into()), body: Some("new body".into()), ..Default::default() }, &registry(), at("2026-11-12T09:00:00+08:00")).unwrap();
    assert_eq!(s.id, old.id, "the id stays");
    let n = notes.get(&old.id).unwrap();
    assert_eq!((n.title.as_str(), n.body.as_str(), n.source.as_str()), ("Renamed", "new body", "manual"));
    assert_eq!(n.created, "2026-11-10T09:00:00+08:00");
    assert_eq!(n.updated.as_deref(), Some("2026-11-12T09:00:00+08:00"));
}

#[test]
fn lore_notation_blocks() {
    let d = tempfile::tempdir().unwrap();
    let notes = Notes::new(d.path().join("notes"));
    let t = at("2026-11-12T09:00:00+08:00");
    let reg = registry();
    // a typed reference number
    let Err(Error::Blocked(f)) = notes.create_at(new("Typed", "As Table 1 shows, the loss drops."), &reg, t) else { panic!("not blocked") };
    assert!(f.iter().any(|x| x.code == "hardcoded_reference" && x.blocking && x.severity == "error" && x.line == Some(1)), "{f:?}");
    assert!(!f.iter().any(|x| x.code == "unused_entry"), "registry-wide findings are not the note's");
    // a label defined twice
    let Err(Error::Blocked(f)) = notes.create_at(new("Dup", "\\begin{equation}a\\label{eq:a}\\end{equation}\n\\begin{equation}b\\label{eq:a}\\end{equation}\nSee \\eqref{eq:a}."), &reg, t) else { panic!() };
    assert!(f.iter().any(|x| x.code == "label_duplicate"));
    // structure
    for bad in ["Unclosed {brace", "Math $x without end", "\\begin{equation} x", "\\documentclass{article}"] {
        let r = notes.create_at(new("S", bad), &reg, t);
        assert!(matches!(r, Err(Error::Blocked(ref f)) if f.iter().any(|x| x.code == "note_structure")), "{bad}: {r:?}");
    }
    assert!(notes.list().is_empty(), "nothing written");

    // a \ref to a label the registry knows is fine, a registered macro is used as is;
    // a label nobody defines blocks (PaperComposer's rule for written text)
    let ok = notes.create_at(new("Fine", "By \\eqref{eq:loss}, \\(\\vw\\) shrinks. A 100\\% \\{ok\\} rate."), &reg, t).unwrap();
    assert!(!ok.findings.iter().any(|f| f.blocking), "{:?}", ok.findings);
    assert!(!ok.findings.iter().any(|f| f.subject.as_deref() == Some("eq:loss")));
    let Err(Error::Blocked(f)) = notes.create_at(new("Unknown", "See \\ref{fig:arch}."), &reg, t) else { panic!("not blocked") };
    assert!(f.iter().any(|x| x.code == "ref_undefined" && x.blocking));
    // a label the note defines itself
    assert!(notes.create_at(new("Own", "\\begin{equation}x\\label{eq:own}\\end{equation}\nBy \\eqref{eq:own}."), &reg, t).is_ok());

    // an update runs the same check and leaves the file as it was
    let before = std::fs::read_to_string(&ok.path).unwrap();
    let r = notes.update_at(&ok.id, Edit { body: Some("Fig. 2 shows it.".into()), ..Default::default() }, &reg, t);
    assert!(matches!(r, Err(Error::Blocked(_))));
    assert_eq!(std::fs::read_to_string(&ok.path).unwrap(), before);
}

#[test]
fn check_is_not_vacuous() {
    // the scanner sees a fragment (no \begin{document}) as the body
    let f = check("Eq. (3) gives it.", &Notation::default());
    assert!(f.iter().any(|x| x.code == "hardcoded_reference"), "{f:?}");
    assert!(check("Plain text with $\\alpha$ and \\ref{eq:loss}.", &registry()).iter().all(|f| !f.blocking));
}
