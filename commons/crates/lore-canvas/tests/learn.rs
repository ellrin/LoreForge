//! The editor-only 「不熟」 emphasis (`learn`): accepted by the style rules, never in an export.

#[path = "../../lore-ooxml/tests/common/mod.rs"]
mod common;

use lore_canvas::style::{LEARN_EM, has_learn, strip_learn};
use lore_canvas::*;
use lore_ooxml::Deck as ODeck;
use serde_json::json;

const STYLE: &str = r#"
[pack]
id = "test"
name = "Test"
slide_cx = 9144000
slide_cy = 5143500
[tags.title]
font = "Calibri"
size = 24.0
bold = true
color = "000000"
[tags.body]
font = "Calibri"
size = 14.0
color = "000000"
[tags.caption]
font = "Calibri"
size = 12.0
color = "000000"
[palette.colors]
black = "000000"
red = "FF0000"
grey = "A6A6A6"
[emphasis.key]
color = "FF0000"
bold = true
[ghost]
picture_alpha = 0.2
line_alpha = 0.2
fill_alpha = 0.3
text_color = "A6A6A6"
"#;

fn style() -> StyleSystem {
    StyleSystem::new(lore_ooxml::style::StylePack::from_toml(STYLE).unwrap())
}

fn load() -> (ODeck, Deck) {
    let od = ODeck::from_bytes(&common::synthetic_deck()).unwrap();
    let cd = pptx_in::import(&od, Some(&style()), Some("test".into()));
    (od, cd)
}

fn no_assets(_: &str) -> Option<Vec<u8>> {
    None
}

/// Ops marking every plain text run (no emphasis yet) of top-level text, shapes and tables.
fn mark_all(deck: &Deck) -> Vec<Op> {
    let mut ops = vec![];
    for s in &deck.slides {
        for o in &s.objects {
            let mut tb = |prefix: String, t: &TextBody| {
                for (pi, p) in t.paras.iter().enumerate() {
                    for (ri, r) in p.runs.iter().enumerate() {
                        if r.kind == "text" && r.em.is_none() && !r.text.trim().is_empty() {
                            ops.push(Op::SetObj { slide: s.uid.clone(), id: o.id, path: format!("{prefix}/paras/{pi}/runs/{ri}/em"), value: json!(LEARN_EM) });
                        }
                    }
                }
            };
            match &o.kind {
                Kind::Text(x) | Kind::Shape(x) => {
                    if let Some(t) = &x.text {
                        tb("/text".into(), t);
                    }
                }
                Kind::Table(t) => {
                    for (ri, row) in t.rows.iter().enumerate() {
                        for (ci, c) in row.cells.iter().enumerate() {
                            tb(format!("/rows/{ri}/cells/{ci}/text"), &c.text);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    ops
}

#[test]
fn style_rules_accept_learn_and_keep_the_look() {
    let (_, cd) = load();
    let st = style();
    let mut ed = Editor::new(cd, Some(st.clone()));
    let s = ed.deck.slides[1].uid.clone();
    ed.apply(vec![Op::SetObj { slide: s.clone(), id: 3, path: "/text/paras/0/runs/0/em".into(), value: json!("learn") }], "不熟")
        .unwrap();
    // an undefined emphasis is still refused
    let e = ed
        .apply(vec![Op::SetObj { slide: s.clone(), id: 3, path: "/text/paras/0/runs/0/em".into(), value: json!("nope") }], "x")
        .unwrap_err();
    assert!(e.to_string().contains("nope"), "{e}");
    // `learn` never changes the look: same as the plain tag, unlike `key`
    assert_eq!(st.look("body", Some(LEARN_EM)), st.look("body", None));
    assert_ne!(st.look("body", Some("key")).unwrap().color, st.look("body", Some(LEARN_EM)).unwrap().color);
    // a run of a defined tag with `learn` passes the run check on its own
    let mut r = Run::text("SBL", "body", st.look("body", None).unwrap());
    r.em = Some(LEARN_EM.into());
    st.check_run(&r).unwrap();
    // a new run (no old value) tagged + learn passes check_change too
    st.check_change(None, &serde_json::to_value(&r).unwrap()).unwrap();
}

#[test]
fn export_strips_learn_and_stays_byte_identical() {
    let (od, cd) = load();
    let st = style();
    let plain = pptx_out::export_bytes(&cd, &od, Some(&st), &no_assets).unwrap();
    let mut ed = Editor::new(cd, Some(st.clone()));
    let ops = mark_all(&ed.deck);
    assert!(ops.len() >= 3, "synthetic deck has text runs: {}", ops.len());
    ed.apply(ops, "標示不熟").unwrap();
    assert!(has_learn(&ed.deck));
    let marked = pptx_out::export_bytes(&ed.deck, &od, Some(&st), &no_assets).unwrap();
    let a = ODeck::from_bytes(&plain).unwrap();
    let b = ODeck::from_bytes(&marked).unwrap();
    assert_eq!(a.pkg.parts.len(), b.pkg.parts.len());
    for p in &a.pkg.parts {
        assert!(b.pkg.get(&p.name) == Some(p.data.as_slice()), "{} differs once learn is stripped", p.name);
    }
    // the editor's deck itself is untouched by the export
    assert!(has_learn(&ed.deck));
}

#[test]
fn edited_learn_run_exports_plain() {
    let (od, cd) = load();
    let st = style();
    let mut ed = Editor::new(cd, Some(st.clone()));
    let s = ed.deck.slides[1].uid.clone();
    // rewrite the first paragraph of a text box: one plain run, one learn run, one key run
    let run = |text: &str, em: Option<&str>| {
        let mut r = Run::text(text, "body", st.look("body", em).unwrap());
        r.em = em.map(str::to_string);
        r
    };
    let runs = vec![run("We use ", None), run("spectral boundary loss", Some(LEARN_EM)), run(" +3.1 Dice", Some("key"))];
    ed.apply(vec![Op::SetObj { slide: s.clone(), id: 3, path: "/text/paras/0/runs".into(), value: serde_json::to_value(&runs).unwrap() }], "edit").unwrap();
    let bytes = pptx_out::export_bytes(&ed.deck, &od, Some(&st), &no_assets).unwrap();
    let back = ODeck::from_bytes(&bytes).unwrap();
    let xml = String::from_utf8_lossy(back.pkg.get("ppt/slides/slide2.xml").unwrap()).to_string();
    assert!(!xml.contains("learn"), "no trace of the mark");
    // the learn run: black, not bold, no underline
    let at = xml.find("spectral boundary loss").unwrap();
    let rpr = &xml[xml[..at].rfind("<a:rPr").unwrap()..at];
    assert!(!rpr.contains("FF0000"), "{rpr}");
    assert!(!rpr.contains(" u=") && !rpr.contains("b=\"1\""), "{rpr}");
    assert!(rpr.contains("000000"), "{rpr}");
    // the key run keeps its red bold
    let at = xml.find(" +3.1 Dice").unwrap();
    let rpr = &xml[xml[..at].rfind("<a:rPr").unwrap()..at];
    assert!(rpr.contains("FF0000") && rpr.contains("b=\"1\""), "{rpr}");
    // reopened: the mark is gone, the key stays
    let cd2 = pptx_in::import(&back, Some(&st), None);
    let mut d2 = cd2.clone();
    assert_eq!(strip_learn(&mut d2), 0);
}
