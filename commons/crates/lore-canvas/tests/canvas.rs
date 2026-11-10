//! Offline tests on the synthetic deck of lore-ooxml's test suite (no real deck content).

#[path = "../../lore-ooxml/tests/common/mod.rs"]
mod common;

use serde_json::json;
use lore_canvas::reveal::{RevealSpec, reveal};
use lore_canvas::*;
use lore_ooxml::Deck as ODeck;
use lore_ooxml::model::Shape;

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
white = "FFFFFF"
red = "FF0000"
grey = "A6A6A6"
[palette.tints.supervised]
hex = "E2F0D9"
[palette.sequences]
numbered-circle = ["F7D9F0", "ECD8FE"]
[palette.alpha]
overlay = 0.35
[emphasis.key]
color = "FF0000"
bold = true
[emphasis.strong]
bold = true
[components.numbered-circle]
geometry = "ellipse"
size_in = 0.24
line = "000000"
text_tag = "caption"
fills = "numbered-circle"
[components.highlight-box]
geometry = "rect"
line = "FF0000"
line_width_pt = 2.25
[components.tab-card]
frame_geometry = "roundRect"
frame_dash = "dash"
label_text_tag = "body"
[arrows.default]
color = "000000"
width_pt = 1.5
tail = "triangle"
[ghost]
picture_alpha = 0.2
line_alpha = 0.2
fill_alpha = 0.3
text_color = "A6A6A6"
agenda_bold_current = true
"#;

fn style() -> StyleSystem {
    StyleSystem::new(lore_ooxml::style::StylePack::from_toml(STYLE).unwrap())
}

fn load() -> (ODeck, Deck) {
    let od = ODeck::from_bytes(&common::synthetic_deck()).unwrap();
    let st = style();
    let cd = pptx_in::import(&od, Some(&st), Some("test".into()));
    (od, cd)
}

fn no_assets(_: &str) -> Option<Vec<u8>> {
    None
}

#[test]
fn import_covers_every_kind() {
    let (_, cd) = load();
    assert_eq!(cd.slides.len(), 2);
    assert!(cd.slides[1].hidden);
    assert_eq!(cd.slides[1].notes, "Speaker note");
    let mut kinds = std::collections::BTreeSet::new();
    cd.slides[1].walk(&mut |o| {
        kinds.insert(o.kind.name());
    });
    for k in ["text", "shape", "picture", "connector", "group", "table", "locked"] {
        assert!(kinds.contains(k), "{k} missing: {kinds:?}");
    }
    let s = &cd.slides[1];
    // text box: numbering, justify, line spacing, emphasis tag
    let Kind::Text(tb) = &s.find(3).unwrap().kind else { panic!() };
    let t = tb.text.as_ref().unwrap();
    assert_eq!(t.paras[0].align, "just");
    assert_eq!(t.paras[0].line, Some(0.9));
    assert!(matches!(&t.paras[0].bullet, Bullet::Num { scheme, .. } if scheme == "arabicPeriod"));
    assert_eq!(t.paras[0].runs[0].tag.as_deref(), Some("body"));
    assert_eq!(t.paras[0].runs[1].em.as_deref(), Some("key"));
    assert_eq!(t.paras[1].level, 1);
    // picture: crop, alpha, effects, asset
    let Kind::Picture(p) = &s.find(5).unwrap().kind else { panic!() };
    assert_eq!(p.asset, "pkg:ppt/media/image2.png");
    assert!((p.crop.l - 0.1).abs() < 1e-9 && (p.crop.r - 0.25).abs() < 1e-9);
    assert!((p.alpha - 0.25).abs() < 1e-9);
    assert!(p.effects.iter().any(|e| e == "backgroundRemoval"));
    // connector attachments
    let Kind::Connector(c) = &s.find(6).unwrap().kind else { panic!() };
    assert_eq!(c.start.as_ref().map(|a| (a.id, a.site)), Some((3, 3)));
    assert_eq!(c.line.as_ref().unwrap().tail, "triangle");
    // nested group + custom geometry
    let Kind::Shape(f) = &s.find(10).unwrap().kind else { panic!() };
    assert_eq!(f.geom.prst, "custom");
    assert!(f.geom.path.as_deref().unwrap().starts_with('M'));
    // equation keeps OMML
    let eq = s.find(12).unwrap();
    let Kind::Text(e) = &eq.kind else { panic!() };
    assert!(e.equation);
    let r = &e.text.as_ref().unwrap().paras[0].runs[0];
    assert_eq!(r.kind, "math");
    assert!(r.omml.as_deref().unwrap().contains("m:oMath"));
    // table with merged cells and fill
    let Kind::Table(tb) = &s.find(11).unwrap().kind else { panic!() };
    assert_eq!(tb.rows[1].cells[0].span, 2);
    assert!(tb.rows[1].cells[1].hmerge);
    assert_eq!(tb.rows[0].cells[1].fill.as_deref(), Some("F2DFF7"));
}

#[test]
fn unedited_export_is_byte_identical() {
    let (od, cd) = load();
    // through deck.json
    let cd: Deck = serde_json::from_str(&serde_json::to_string(&cd).unwrap()).unwrap();
    let st = style();
    let bytes = pptx_out::export_bytes(&cd, &od, Some(&st), &no_assets).unwrap();
    let back = ODeck::from_bytes(&bytes).unwrap();
    for p in &od.pkg.parts {
        assert!(back.pkg.get(&p.name) == Some(p.data.as_slice()), "{}", p.name);
    }
    assert_eq!(back.pkg.parts.len(), od.pkg.parts.len());
}

#[test]
fn edits_patch_only_what_changed() {
    let (od, cd) = load();
    let st = style();
    let mut ed = Editor::new(cd, Some(st.clone()));
    let s = ed.deck.slides[1].uid.clone();
    // move picture, change text, add arrow, change notes
    let mut f = ed.deck.slides[1].find(5).unwrap().frame.clone();
    f.x += 100_000;
    let arrow = components::arrow(&st, "default", (100.0, 100.0), (900_000.0, 100.0), 40).unwrap();
    ed.apply(
        vec![
            Op::SetObj { slide: s.clone(), id: 5, path: "/frame".into(), value: serde_json::to_value(&f).unwrap() },
            Op::SetObj { slide: s.clone(), id: 3, path: "/text/paras/0/runs/0/text".into(), value: json!("Changed ") },
            Op::InsertObj { slide: s.clone(), parent: None, index: 99, obj: arrow },
            Op::SetSlide { slide: s.clone(), path: "/notes".into(), value: json!("New note\nline 2") },
        ],
        "edit",
    )
    .unwrap();
    let bytes = pptx_out::export_bytes(&ed.deck, &od, Some(&st), &no_assets).unwrap();
    // untouched slide 1 stays identical, slide 2 changes
    let back = ODeck::from_bytes(&bytes).unwrap();
    assert!(back.pkg.get("ppt/slides/slide1.xml") == od.pkg.get("ppt/slides/slide1.xml"));
    assert!(back.pkg.get("ppt/slides/slide2.xml") != od.pkg.get("ppt/slides/slide2.xml"));
    // untouched objects of the edited slide keep their XML exactly
    let orig = &od.slides[1].doc;
    let new = &back.slides[1].doc;
    for id in [4u32, 6, 7, 11, 12, 13] {
        let a = orig.shapes().iter().find(|x| x.id() == Some(id)).map(|x| x.to_element().to_xml_string());
        let b = new.shapes().iter().find(|x| x.id() == Some(id)).map(|x| x.to_element().to_xml_string());
        assert_eq!(a, b, "object {id}");
    }
    // the edited picture keeps its effects, crop and alpha
    let pic = new.shapes().iter().find(|x| x.id() == Some(5)).unwrap();
    let Shape::Pic(p) = pic else { panic!() };
    assert_eq!(p.props.xfrm.as_ref().unwrap().off, Some((400_000, 2_500_000)));
    assert!(p.fill.blip.as_ref().unwrap().has_background_removal());
    // reopen: edits are there
    let cd2 = pptx_in::import(&back, Some(&st), None);
    let s2 = &cd2.slides[1];
    let Kind::Text(t) = &s2.find(3).unwrap().kind else { panic!() };
    assert!(t.text.as_ref().unwrap().plain_text().starts_with("Changed key & result"));
    assert_eq!(t.text.as_ref().unwrap().paras[0].runs[1].em.as_deref(), Some("key"));
    assert!(matches!(s2.find(40).unwrap().kind, Kind::Connector(_)));
    assert_eq!(s2.notes, "New note\nline 2");
    assert_eq!(s2.find(5).unwrap().frame.x, 400_000);
}

#[test]
fn rules_reject_unknown_tags_and_off_palette_colours() {
    let (_, cd) = load();
    let mut ed = Editor::new(cd, Some(style()));
    let s = ed.deck.slides[1].uid.clone();
    let e = ed
        .apply(vec![Op::SetObj { slide: s.clone(), id: 3, path: "/text/paras/0/runs/0/tag".into(), value: json!("huge") }], "x")
        .unwrap_err();
    assert!(e.to_string().contains("huge"), "{e}");
    let e = ed
        .apply(vec![Op::SetObj { slide: s.clone(), id: 4, path: "/fill".into(), value: json!({"kind": "solid", "color": "123456"}) }], "x")
        .unwrap_err();
    assert!(e.to_string().contains("123456"));
    // palette colour is fine; tag change to a defined tag is fine
    ed.apply(vec![Op::SetObj { slide: s.clone(), id: 4, path: "/fill".into(), value: json!({"kind": "solid", "color": "E2F0D9"}) }], "ok")
        .unwrap();
    // nothing half-applied after a failure
    assert_eq!(ed.undo.len(), 1);
}

#[test]
fn undo_redo_and_grouping() {
    let (_, cd) = load();
    let before = cd.clone();
    let mut ed = Editor::new(cd, Some(style()));
    let s = ed.deck.slides[1].uid.clone();
    ed.apply(vec![Op::Group { slide: s.clone(), ids: vec![3, 4], id: Some(50), name: None }], "group").unwrap();
    assert!(matches!(ed.deck.slides[1].find(50).unwrap().kind, Kind::Group(_)));
    assert!(ed.deck.slides[1].objects.iter().all(|o| o.id != 3));
    ed.apply(vec![Op::Ungroup { slide: s.clone(), id: 50 }], "ungroup").unwrap();
    assert_eq!(ed.deck.slides[1].find(3).unwrap().frame, before.slides[1].find(3).unwrap().frame);
    ed.undo().unwrap();
    ed.undo().unwrap();
    assert_eq!(ed.deck, before);
    ed.redo().unwrap();
    assert!(ed.deck.slides[1].find(50).is_some());
    // slide ops
    let dup = {
        let mut x = ed.deck.slides[0].clone();
        x.uid = "s99".into();
        x
    };
    ed.apply(vec![Op::InsertSlide { index: 0, slide: dup }, Op::MoveSlide { slide: s.clone(), index: 0 }], "slides").unwrap();
    assert_eq!(ed.deck.slides[0].uid, s);
    ed.undo().unwrap();
    assert_eq!(ed.deck.slides.len(), 2);
}

#[test]
fn moving_a_shape_reroutes_attached_connectors() {
    let (_, cd) = load();
    let mut ed = Editor::new(cd, Some(style()));
    let s = ed.deck.slides[1].uid.clone();
    let mut f = ed.deck.slides[1].find(3).unwrap().frame.clone();
    f.y += 500_000;
    ed.apply(vec![Op::SetObj { slide: s.clone(), id: 3, path: "/frame".into(), value: serde_json::to_value(&f).unwrap() }], "move").unwrap();
    let c = ed.deck.slides[1].find(6).unwrap();
    let (a, _) = geom::connector_points(&c.frame);
    // starts at the text box's right site (index 3)
    assert!((a.0 - (f.x + f.w) as f64).abs() < 2.0 && (a.1 - (f.y as f64 + f.h as f64 / 2.0)).abs() < 2.0, "{a:?}");
}

#[test]
fn reveal_ghosts_other_steps_and_exports_new_slides() {
    let (od, cd) = load();
    let st = style();
    let slide = cd.slides[1].clone();
    let mut n = 100;
    let steps = RevealSpec::Ghost { steps: vec![vec![3], vec![5, 6], vec![4]], future: "ghost".into() };
    let ghost = |l: &Look| Look { color: "A6A6A6".into(), ..l.clone() };
    let out = reveal(&slide, &steps, &st.pack.ghost, &ghost, &mut || {
        n += 1;
        format!("s{n}")
    });
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].uid, slide.uid);
    // step 1: picture ghosted, text current
    let Kind::Picture(p) = &out[0].find(5).unwrap().kind else { panic!() };
    assert!((p.alpha - 0.2).abs() < 1e-9);
    let Kind::Text(t) = &out[0].find(3).unwrap().kind else { panic!() };
    assert!(t.text.as_ref().unwrap().paras[0].runs[0].em.is_none());
    // step 2: text ghosted, connector current
    let Kind::Text(t) = &out[1].find(3).unwrap().kind else { panic!() };
    assert_eq!(t.text.as_ref().unwrap().paras[0].runs[0].em.as_deref(), Some("ghost"));
    let Kind::Connector(c) = &out[1].find(6).unwrap().kind else { panic!() };
    assert!((c.line.as_ref().unwrap().alpha - 0.2).abs() < 1e-9, "connector alpha already 0.2 in fixture");
    // through ops (validated) and export
    let mut ed = Editor::new(cd, Some(st.clone()));
    let mut ops = vec![Op::DeleteSlide { slide: slide.uid.clone() }];
    for (i, s) in out.into_iter().enumerate() {
        ops.push(Op::InsertSlide { index: 1 + i, slide: s });
    }
    ed.apply(ops, "reveal").unwrap();
    let back = ODeck::from_bytes(&pptx_out::export_bytes(&ed.deck, &od, Some(&st), &no_assets).unwrap()).unwrap();
    assert_eq!(back.slides.len(), 4);
    let cd2 = pptx_in::import(&back, Some(&st), None);
    let Kind::Text(t) = &cd2.slides[2].find(3).unwrap().kind else { panic!() };
    let r = &t.text.as_ref().unwrap().paras[0].runs[0];
    assert_eq!(r.look.color, "A6A6A6");
    assert_eq!(r.em.as_deref(), Some("ghost"));
    // the duplicated slides share the picture media
    assert_eq!(back.pkg.parts.iter().filter(|p| p.name.starts_with("ppt/media/")).count(), od.pkg.parts.iter().filter(|p| p.name.starts_with("ppt/media/")).count());
}

#[test]
fn agenda_reveal_bolds_one_item_per_slide() {
    let (_, cd) = load();
    let st = style();
    let mut n = 0;
    let out = reveal(
        &cd.slides[1],
        // explicit paragraphs (the default list skips the level-2 item, see below)
        &RevealSpec::Agenda { obj: 3, paras: Some(vec![0, 1]), em: "strong".into() },
        &st.pack.ghost,
        &|l: &Look| l.clone(),
        &mut || {
            n += 1;
            format!("x{n}")
        },
    );
    assert_eq!(out.len(), 2);
    let Kind::Text(t) = &out[1].find(3).unwrap().kind else { panic!() };
    assert_eq!(t.text.as_ref().unwrap().paras[1].runs[0].em.as_deref(), Some("strong"));
    assert!(t.text.as_ref().unwrap().paras[0].runs[0].em.is_none());
}

#[test]
fn agenda_reveal_skips_empty_and_sub_items() {
    let (_, cd) = load();
    let st = style();
    let mut slide = cd.slides[1].clone();
    let look = st.look("body", None).unwrap();
    let p = |t: &str, level: u32| Para { level, ..Para::new(vec![Run::text(t, "body", look.clone())]) };
    let paras = vec![p("Intro", 0), p("", 0), p("   ", 0), p("detail", 1), p("Method", 0), Para::new(vec![]), p("Results", 0)];
    let o = slide.find_mut(3).unwrap();
    let Kind::Text(s) = &mut o.kind else { panic!() };
    s.text.as_mut().unwrap().paras = paras;
    let mut n = 0;
    let out = reveal(
        &slide,
        &RevealSpec::Agenda { obj: 3, paras: None, em: "strong".into() },
        &st.pack.ghost,
        &|l: &Look| l.clone(),
        &mut || {
            n += 1;
            format!("x{n}")
        },
    );
    assert_eq!(out.len(), 3, "one slide per non-empty level-0 item");
    let strong = |i: usize| {
        let Kind::Text(t) = &out[i].find(3).unwrap().kind else { panic!() };
        t.text.as_ref().unwrap().paras.iter().enumerate().filter(|(_, p)| p.runs.iter().any(|r| r.em.as_deref() == Some("strong"))).map(|(k, _)| k).collect::<Vec<_>>()
    };
    assert_eq!(strong(0), vec![0]);
    assert_eq!(strong(1), vec![4]);
    assert_eq!(strong(2), vec![6]);
}

#[test]
fn new_objects_components_tables_equations_export() {
    let (od, cd) = load();
    let st = style();
    let mut ed = Editor::new(cd, Some(st.clone()));
    let s = ed.deck.slides[0].uid.clone();
    let circle = components::instantiate(&st, "numbered-circle", 100_000, 100_000, 2, 20).unwrap();
    let card = components::instantiate(&st, "tab-card", 1_000_000, 1_000_000, 1, 21).unwrap();
    let look = st.look("body", None).unwrap();
    let cell = |t: &str| Cell {
        text: TextBody { paras: vec![Para::new(vec![Run::text(t, "body", look.clone())])], ..TextBody::default() },
        span: 1,
        rspan: 1,
        hmerge: false,
        vmerge: false,
        fill: None,
        borders: [None, None, None, None],
        src: None,
    };
    let table = Object {
        id: 30,
        name: "表格".into(),
        frame: Frame::new(500_000, 3_000_000, 3_000_000, 600_000),
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Table(TableObj {
            cols: vec![1_500_000, 1_500_000],
            rows: vec![Row { h: 300_000, cells: vec![cell("Epochs"), cell("200")], src: None }],
            style_id: None,
            first_row: false,
            band_row: false,
        }),
        src: None,
    };
    let omml = r#"<a14:m><m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>x</m:t></m:r></m:oMath></a14:m>"#;
    let eq = Object {
        id: 31,
        name: "方程式".into(),
        frame: Frame::new(500_000, 4_000_000, 1_000_000, 300_000),
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Text(SpObj {
            geom: Geom::preset("rect"),
            fill: Paint::None,
            line: None,
            text: Some(TextBody {
                paras: vec![Para::new(vec![Run {
                    kind: "math".into(),
                    text: "x".into(),
                    tag: None,
                    em: None,
                    look: look.clone(),
                    field: None,
                    omml: Some(omml.into()),
                    latex: Some("x".into()),
                    src: None,
                    k: 0,
                }])],
                ..TextBody::default()
            }),
            equation: true,
        }),
        src: None,
    };
    let mut pic = Object {
        id: 32,
        name: "圖".into(),
        frame: Frame::new(5_000_000, 3_000_000, 1_000_000, 1_000_000),
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Picture(PicObj {
            asset: "file:assets/a.png".into(),
            crop: Crop { l: 0.1, t: 0.0, r: 0.0, b: 0.2 },
            alpha: 0.5,
            line: None,
            clip: "rect".into(),
            masks: vec![FracRect { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }],
            effects: vec![],
        }),
        src: None,
    };
    pic.name = "圖".into();
    ed.apply(
        [circle, card, table, eq, pic]
            .into_iter()
            .map(|o| Op::InsertObj { slide: s.clone(), parent: None, index: 99, obj: o })
            .collect(),
        "insert",
    )
    .unwrap();
    let need = pptx_out::equations_needing_fallback(&ed.deck);
    assert_eq!(need.iter().map(|(sl, id, _)| (sl.as_str(), *id)).collect::<Vec<_>>(), [(s.as_str(), 31)], "only the new equation");
    let fb_key = pptx_out::equation_fallback_key(&s, 31);
    let assets = |p: &str| match p {
        "assets/a.png" => Some(b"\x89PNG-new".to_vec()),
        k if k == fb_key => Some(b"\x89PNG-equation".to_vec()),
        _ => None,
    };
    let bytes = pptx_out::export_bytes(&ed.deck, &od, Some(&st), &assets).unwrap();
    let back = ODeck::from_bytes(&bytes).unwrap();
    let cd2 = pptx_in::import(&back, Some(&st), None);
    let s2 = &cd2.slides[0];
    let Kind::Shape(c) = &s2.find(20).unwrap().kind else { panic!() };
    assert_eq!(c.geom.prst, "ellipse");
    assert_eq!(c.fill, Paint::solid("ECD8FE"));
    assert_eq!(c.text.as_ref().unwrap().plain_text(), "2");
    assert!(matches!(s2.find(21).unwrap().kind, Kind::Group(_)));
    let Kind::Table(t) = &s2.find(30).unwrap().kind else { panic!() };
    assert_eq!(t.rows[0].cells[1].text.plain_text(), "200");
    let Kind::Text(e) = &s2.find(31).unwrap().kind else { panic!() };
    assert!(e.equation);
    let Kind::Picture(p) = &s2.find(32).unwrap().kind else { panic!() };
    assert!((p.alpha - 0.5).abs() < 1e-6 && (p.crop.b - 0.2).abs() < 1e-6);
    assert!(back.pkg.parts.iter().any(|x| x.name.starts_with("ppt/media/") && x.data == b"\x89PNG-new"));
    // the new equation carries the offered picture for viewers without Office Math
    let media = back.pkg.parts.iter().find(|x| x.data == b"\x89PNG-equation").expect("fallback picture in the package").name.clone();
    let slide_xml = String::from_utf8(back.pkg.parts.iter().find(|x| x.name == back.slides[0].part).unwrap().data.clone()).unwrap();
    let fb = slide_xml.split("<mc:Fallback").nth(1).expect("an mc:Fallback branch");
    assert!(fb.split("</mc:Fallback>").next().unwrap().contains("<p:pic"), "the fallback is a picture");
    assert!(media.starts_with("ppt/media/") && media.ends_with(".png"));
    // mask exported as a white rectangle after the picture
    assert!(s2.objects.iter().any(|o| o.name.starts_with("SC-mask:32")));
}

#[test]
fn match_look_and_components() {
    let st = style();
    let mut l = st.look("body", None).unwrap();
    assert_eq!(st.match_look(&l, ""), Some(("body".into(), None)));
    l.color = "FF0000".into();
    l.bold = true;
    assert_eq!(st.match_look(&l, ""), Some(("body".into(), Some("key".into()))));
    l.size = 33.0;
    assert_eq!(st.match_look(&l, ""), None);
    assert!(components::instantiate(&st, "highlight-box", 0, 0, 1, 5).is_some());
    assert_eq!(st.color("numbered-circle[1]").as_deref(), Some("ECD8FE"));
}
