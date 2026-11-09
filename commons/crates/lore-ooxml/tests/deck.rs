mod common;

use lore_ooxml::anim::{Entrance, EntranceAnim};
use lore_ooxml::inventory::inventory;
use lore_ooxml::model::*;
use lore_ooxml::shapes::{self, Rect};
use lore_ooxml::style::{StylePack, extract_draft};
use lore_ooxml::xml::canonical_eq_bytes;
use lore_ooxml::{Deck, Package};

fn deck() -> (Vec<u8>, Deck) {
    let b = common::synthetic_deck();
    let d = Deck::from_bytes(&b).unwrap();
    (b, d)
}

#[test]
fn unedited_roundtrip_is_canonically_equal_and_other_parts_byte_equal() {
    let (src, d) = deck();
    let out = d.to_bytes().unwrap();
    let a = Package::from_bytes(&src).unwrap();
    let b = Package::from_bytes(&out).unwrap();
    assert_eq!(a.parts.len(), b.parts.len());
    for p in &a.parts {
        let q = b.get(&p.name).unwrap();
        if p.name.starts_with("ppt/slides/slide")
            || p.name.starts_with("ppt/notesSlides/notesSlide")
        {
            canonical_eq_bytes(&p.data, q)
                .unwrap()
                .unwrap_or_else(|e| panic!("{}: {e}", p.name));
        } else {
            assert_eq!(p.data, q, "{}", p.name);
        }
    }
    // also via JSON (what an editor would store)
    for s in &d.slides {
        let j = serde_json::to_string(&s.doc).unwrap();
        let back: SlideDoc = serde_json::from_str(&j).unwrap();
        canonical_eq_bytes(a.get(&s.part).unwrap(), &back.to_bytes())
            .unwrap()
            .unwrap();
    }
}

#[test]
fn typed_model_sees_every_object_kind() {
    let (_, d) = deck();
    assert_eq!(d.slides.len(), 2);
    assert_eq!(
        d.layouts.iter().map(|l| l.name()).collect::<Vec<_>>(),
        ["Title Slide", "1_Title+Content"]
    );
    let s = &d.slides[1];
    assert!(s.doc.hidden() && !d.slides[0].doc.hidden());
    assert_eq!(s.notes.as_ref().unwrap().doc.plain_text(), "Speaker note");
    let sh = s.doc.shapes();
    // text box: bullets, numbering, levels, alignment, spacing, runs
    let Shape::Sp(tb) = &sh[1] else { panic!() };
    assert_eq!(sh[1].kind_name(), "textbox");
    let t = tb.text.as_ref().unwrap();
    let p0 = &t.paragraphs[0];
    let pp = p0.props.as_ref().unwrap();
    assert_eq!(pp.align(), Some("just"));
    assert_eq!(pp.line_spacing, Some(Spacing::Pct(90000)));
    assert_eq!(pp.space_before, Some(Spacing::Pts(600)));
    assert!(
        matches!(&pp.bullet, Some(Bullet::AutoNum(a)) if a.get("type") == Some("arabicPeriod"))
    );
    assert_eq!(p0.plain_text(), "First key & result");
    let r1 = p0.runs[1].props().unwrap();
    assert_eq!(
        (r1.size_pt(), r1.bold(), r1.color().and_then(|c| c.hex())),
        (Some(14.0), Some(true), Some("FF0000".into()))
    );
    assert_eq!(t.paragraphs[1].level(), 1);
    assert!(matches!(t.paragraphs[1].runs[1], Run::Break { .. }));
    assert!(matches!(t.paragraphs[1].runs[2], Run::Field { .. }));
    // card: rotated dashed roundRect with 35% alpha tint
    let Shape::Sp(card) = &sh[2] else { panic!() };
    assert_eq!(card.props.xfrm.as_ref().unwrap().rot_deg(), 90.0);
    assert_eq!(card.props.geom.as_ref().unwrap().name(), "roundRect");
    let c = card.props.fill.as_ref().unwrap().color().unwrap();
    assert_eq!((c.hex().unwrap().as_str(), c.alpha()), ("E2F0D9", 0.35));
    assert_eq!(
        card.props.line.as_ref().unwrap().dash,
        Some(Dash::Preset("dash".into()))
    );
    // picture: crop, alpha, outline, background removal
    let Shape::Pic(pic) = &sh[3] else { panic!() };
    let crop = pic.fill.crop().unwrap();
    assert_eq!((crop.l, crop.t, crop.r, crop.b), (10000, 0, 25000, 5000));
    let blip = pic.fill.blip.as_ref().unwrap();
    assert_eq!(
        (blip.embed(), blip.alpha(), blip.has_background_removal()),
        (Some("rId2"), 0.25, true)
    );
    assert_eq!(pic.props.line.as_ref().unwrap().width_pt(), Some(2.25));
    // connector
    let Shape::Cxn(cx) = &sh[4] else { panic!() };
    assert_eq!(
        (cx.nv.kind.st_cxn, cx.nv.kind.end_cxn),
        (Some((3, 3)), Some((4, 1)))
    );
    let ln = cx.props.line.as_ref().unwrap();
    assert_eq!(ln.tail.as_ref().unwrap().kind(), "triangle");
    assert_eq!(ln.fill.as_ref().unwrap().color().unwrap().alpha(), 0.2);
    // nested groups: absolute rect of the oval resolves both transforms
    let abs = shapes::absolute_rects(&s.doc.tree);
    let oval = abs.iter().find(|(id, _)| *id == 9).unwrap().1;
    assert_eq!(
        (oval.x, oval.y, oval.cx, oval.cy),
        (6_500_000, 3_000_000, 150_000, 150_000)
    );
    // table
    let Shape::Frame(f) = &sh[6] else { panic!() };
    let FrameContent::Table(tbl) = &f.content else {
        panic!()
    };
    assert_eq!(
        tbl.style_id().as_deref(),
        Some("{5940675A-B579-460E-94D1-54222C63F5DA}")
    );
    assert_eq!(tbl.col_widths(), [1_500_000, 1_500_000]);
    assert_eq!(tbl.rows[1].cells[0].attrs.get("gridSpan"), Some("2"));
    // OMML alternate content: typed choice with Math run + picture fallback
    let Shape::Alt(alt) = &sh[7] else { panic!() };
    assert!(alt.has_math());
    let Shape::Sp(ch) = &alt.choices[0].shapes[0] else {
        panic!()
    };
    assert!(matches!(
        ch.text.as_ref().unwrap().paragraphs[0].runs[0],
        Run::Math(_)
    ));
    assert!(
        matches!(&alt.fallback.as_ref().unwrap().shapes[0], Shape::Sp(f) if matches!(f.props.fill, Some(Fill::Blip(_))))
    );
    // locked: chart frame + contentPart
    let inv = inventory(&d);
    assert_eq!(inv[1].locked, ["graphicFrame:chart", "p:contentPart"]);
    assert!(!inv[1].fully_editable() && inv[0].fully_editable());
    assert_eq!(inv[1].counts.get("math"), Some(&1));
    assert_eq!(inv[1].raw_features.get("a:custGeom"), Some(&1));
}

#[test]
fn edits_are_written_and_unknown_parts_survive() {
    let (_, mut d) = deck();
    {
        let s = &mut d.slides[1].doc;
        let Shape::Sp(tb) = &mut s.shapes_mut()[1] else {
            panic!()
        };
        if let Run::Text { text, .. } = &mut tb.text.as_mut().unwrap().paragraphs[0].runs[0] {
            *text = "Edited ".into();
        }
        tb.props.xfrm.as_mut().unwrap().off = Some((111, 222));
        let Shape::Pic(p) = &mut s.shapes_mut()[3] else {
            panic!()
        };
        p.fill.set_crop(Some(SrcRect {
            l: 0,
            t: 1000,
            r: 0,
            b: 0,
        }));
        p.fill.blip.as_mut().unwrap().set_alpha(None);
        let Shape::Sp(card) = &mut s.shapes_mut()[2] else {
            panic!()
        };
        card.props
            .fill
            .as_mut()
            .unwrap()
            .color_mut()
            .unwrap()
            .set_alpha(Some(0.4));
    }
    let out = Deck::from_bytes(&d.to_bytes().unwrap()).unwrap();
    let x = out.slides[1].doc.to_document().root.to_xml_string();
    assert!(x.contains("<a:t>Edited </a:t>") && x.contains(r#"<a:off x="111" y="222"/>"#));
    assert!(x.contains(r#"<a:srcRect t="1000"/>"#) && !x.contains("alphaModFix"));
    assert!(x.contains(r#"<a:alpha val="40000"/>"#));
    // untouched raw pieces are still there
    for k in [
        "a14:backgroundRemoval",
        "a:outerShdw",
        "a:custGeom",
        "c:chart",
        "p:contentPart",
        "p14:creationId",
        "a16:creationId",
    ] {
        assert!(x.contains(k), "{k} lost");
    }
}

#[test]
fn placeholder_inheritance_slide_layout_master() {
    let (_, d) = deck();
    let s = &d.slides[1];
    let title = &s.doc.shapes()[0];
    // slide title has no xfrm; layout title has none either -> master's
    let x = d.effective_xfrm(1, title).unwrap();
    assert_eq!(x.off, Some((254000, 203200)));
    let Shape::Sp(sp) = title else { panic!() };
    let p = &sp.text.as_ref().unwrap().paragraphs[0];
    let r = d.resolve_run(1, sp, p, p.runs[0].props());
    assert_eq!(r.size_pt, Some(24.0));
    assert_eq!(r.bold, Some(true));
    assert_eq!(r.latin.as_deref(), Some("Heading Sans"));
    assert_eq!(r.ea.as_deref(), Some("Heading TC"));
    assert_eq!(r.color.as_deref(), Some("000000"));
    // cover ctrTitle uses the layout xfrm
    let cover = &d.slides[0].doc.shapes()[0];
    assert_eq!(
        d.effective_xfrm(0, cover).unwrap().off,
        Some((400000, 1600000))
    );
    // text box falls back to presentation defaultTextStyle + theme minor font
    let Shape::Sp(tb) = &s.doc.shapes()[1] else {
        panic!()
    };
    let p1 = &tb.text.as_ref().unwrap().paragraphs[1];
    let r = d.resolve_run(1, tb, p1, p1.runs[0].props());
    assert_eq!(
        (r.size_pt, r.latin.as_deref(), r.ea.as_deref()),
        (Some(12.0), Some("Calibri"), Some("Body TC"))
    );
    // shape with p:style: font colour from fontRef (lt1 -> FFFFFF)
    let Shape::Sp(card) = &s.doc.shapes()[2] else {
        panic!()
    };
    let pc = &card.text.as_ref().unwrap().paragraphs[0];
    assert_eq!(
        d.resolve_run(1, card, pc, pc.runs[0].props())
            .color
            .as_deref(),
        Some("FFFFFF")
    );
}

#[test]
fn template_then_new_deck_on_named_layout_with_animations() {
    let (_, d) = deck();
    let tpl = d.template_package().unwrap();
    let names: Vec<_> = tpl.names().collect();
    assert!(
        names.iter().all(|n| !n.contains("slides/")
            && !n.contains("notesSlides")
            && !n.contains("thumbnail"))
    );
    assert!(names.contains(&"ppt/media/image1.png"), "master logo kept");
    assert!(
        !names.contains(&"ppt/media/image2.png") && !names.contains(&"ppt/media/image3.png"),
        "slide media dropped"
    );
    let pres = String::from_utf8(tpl.get("ppt/presentation.xml").unwrap().to_vec()).unwrap();
    assert!(!pres.contains("sldIdLst") && !pres.contains("sectionLst"));
    let core = String::from_utf8(tpl.get("docProps/core.xml").unwrap().to_vec()).unwrap();
    assert!(!core.contains("Secret") && !core.contains("ellrin"));
    let ct = String::from_utf8(tpl.get("[Content_Types].xml").unwrap().to_vec()).unwrap();
    assert!(!ct.contains("slide1.xml") && !ct.contains("notesSlide"));

    let mut nd = Deck::from_package(tpl).unwrap();
    assert_eq!(nd.slides.len(), 0);
    let c = nd.add_slide("Title Slide").unwrap();
    nd.set_placeholder_text(c, "ctrTitle", &["Hello"]).unwrap();
    let i = nd.add_slide("1_Title+Content").unwrap();
    nd.set_placeholder_text(i, "title", &["Method"]).unwrap();
    let rid = nd.add_image(i, b"\x89PNG-new".to_vec(), "png").unwrap();
    let id = nd.slides[i].doc.next_shape_id();
    let pack = StylePack::from_toml(
        "[pack]\nid='t'\nname='t'\nslide_cx=9144000\nslide_cy=5143500\n[tags.body]\nfont='Calibri'\nsize=14\n[emphasis.key]\ncolor='FF0000'\nbold=true\n",
    )
    .unwrap();
    let run = pack.run("body", Some("key"), "key result").unwrap();
    {
        let doc = &mut nd.slides[i].doc;
        doc.shapes_mut().push(shapes::text_box(
            id,
            "TextBox",
            Rect::pt(20.0, 80.0, 300.0, 40.0),
            vec![shapes::paragraph(vec![run])],
        ));
        doc.shapes_mut().push(shapes::picture(
            id + 1,
            "Picture",
            &rid,
            Rect::pt(340.0, 80.0, 200.0, 150.0),
            Some(SrcRect {
                l: 5000,
                t: 0,
                r: 0,
                b: 0,
            }),
            Some(0.25),
        ));
        doc.shapes_mut().push(shapes::connector(
            id + 2,
            "Arrow",
            (100, 100),
            (50, 300),
            Line {
                tail: Some(LineEnd::arrow("triangle")),
                ..Line::new(1.5, "000000")
            },
        ));
        doc.set_entrance_animations(&[
            EntranceAnim::new(id, Entrance::Fade),
            EntranceAnim {
                is_text_shape: false,
                ..EntranceAnim::new(id + 1, Entrance::Wipe)
            },
        ]);
    }
    let bytes = nd.to_bytes().unwrap();
    let back = Deck::from_bytes(&bytes).unwrap();
    assert_eq!(back.slides.len(), 2);
    assert_eq!(back.slide_layout(0).unwrap().name(), "Title Slide");
    assert_eq!(back.slides[0].doc.plain_text(), "Hello");
    let s = &back.slides[1].doc;
    assert!(s.plain_text().contains("Method") && s.plain_text().contains("key result"));
    let anims = s.entrance_animations();
    assert_eq!(
        anims.iter().map(|a| a.effect).collect::<Vec<_>>(),
        [Entrance::Fade, Entrance::Wipe]
    );
    let x = s.to_document().root.to_xml_string();
    assert!(
        x.contains(r#"<a:alphaModFix amt="25000"/>"#) && x.contains(r#"<a:srcRect l="5000"/>"#)
    );
    assert!(x.contains(r#"flipH="1""#) && x.contains(r#"<a:tailEnd type="triangle"/>"#));
    assert!(x.contains(r#"sz="1400" b="1""#) && x.contains(r#"<a:srgbClr val="FF0000"/>"#));
    let pkg = Package::from_bytes(&bytes).unwrap();
    assert!(
        pkg.has("ppt/media/image2.png"),
        "new media gets a free name"
    );
    // ids are unique per slide
    let mut ids = vec![];
    s.tree.walk(&mut |sh, _| ids.extend(sh.id()));
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(n, ids.len());
}

#[test]
fn style_draft_from_synthetic_deck() {
    let (_, d) = deck();
    let draft = extract_draft(&[d]);
    assert_eq!(draft.slides, 2);
    assert!(draft.layouts.contains_key("Title Slide"));
    assert_eq!(
        draft.circle_fills.first().map(|c| c.0.as_str()),
        Some("F7D9F0")
    );
    assert_eq!(draft.dashed_round_rects, 1);
    assert_eq!(draft.picture_alpha.get("25"), Some(&1));
    let pack = draft.to_style_pack("syn", "Synthetic");
    assert_eq!(
        pack.layouts.get("cover").map(String::as_str),
        Some("Title Slide")
    );
    assert_eq!(
        pack.tags.get("title").map(|t| (t.size, t.bold)),
        Some((24.0, true))
    );
    assert!(pack.tags.contains_key("equation"));
    let toml = pack.to_toml();
    assert_eq!(StylePack::from_toml(&toml).unwrap(), pack);
}

#[test]
fn save_new_never_overwrites() {
    let (_, d) = deck();
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("out.pptx");
    d.save_new(&p).unwrap();
    assert!(d.save_new(&p).is_err());
}

#[test]
fn color_transforms_match_powerpoint_palette() {
    use lore_ooxml::deck::apply_color_mods;
    use lore_ooxml::xml::Element;
    let mut c = Color::scheme("accent6");
    c.mods
        .push(Element::new("a:lumMod").with_attr("val", "75000"));
    assert_eq!(apply_color_mods("70AD47", &c), "548235");
    let mut c = Color::scheme("bg1");
    c.mods
        .push(Element::new("a:lumMod").with_attr("val", "65000"));
    assert_eq!(apply_color_mods("FFFFFF", &c), "A6A6A6");
}
