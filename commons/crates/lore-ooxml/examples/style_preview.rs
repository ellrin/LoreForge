//! Build a sample deck from a style pack (template.pptx + style.toml): a cover,
//! a "tags & components" slide and a ghosting (progressive reveal) pair.
//! Exercises: new deck from template, slides on named layouts, tags/emphasis,
//! components, tables, OMML, pictures with crop/alpha, connectors, animations.
//!
//! ```sh
//! LORE_STYLE_DIR=…/styles/my-style LORE_OUT=/tmp/preview.pptx cargo run -p lore-ooxml --example style_preview
//! ```

use std::path::PathBuf;

#[path = "roundtrip/visual.rs"]
#[allow(dead_code)]
mod visual;

use lore_ooxml::anim::{Entrance, EntranceAnim, Trigger};
use lore_ooxml::model::*;
use lore_ooxml::shapes::{self, Rect};
use lore_ooxml::style::StylePack;
use lore_ooxml::xml::parse_element;
use lore_ooxml::{Deck, Package};

fn text(pack: &StylePack, id: u32, r: Rect, lines: &[&[(&str, Option<&str>, &str)]]) -> Shape {
    let paras = lines
        .iter()
        .map(|runs| {
            shapes::paragraph(
                runs.iter()
                    .filter_map(|(t, e, s)| pack.run(t, *e, s))
                    .collect(),
            )
        })
        .collect();
    shapes::text_box(id, &format!("TextBox {id}"), r, paras)
}

fn tint(pack: &StylePack, name: &str) -> String {
    pack.palette
        .tints
        .get(name)
        .map(|t| t.hex.clone())
        .unwrap_or_else(|| "EDEDED".into())
}

fn solid(hex: &str, alpha: Option<f64>) -> Fill {
    let mut c = Color::srgb(hex);
    c.set_alpha(alpha);
    Fill::Solid(Some(c))
}

fn label_text(sh: &mut Shape, pack: &StylePack, tag: &str, s: &str) {
    if let Shape::Sp(sp) = sh
        && let Some(tb) = sp.text.as_mut()
    {
        tb.paragraphs[0].runs = pack.run(tag, None, s).into_iter().collect();
    }
}

fn arrow(
    pack: &StylePack,
    name: &str,
    id: u32,
    from: (f64, f64),
    to: (f64, f64),
    alpha: Option<f64>,
) -> Shape {
    let a = &pack.arrows[name];
    let mut l = Line::new(a.width_pt, &a.color);
    if let Some(Fill::Solid(Some(c))) = l.fill.as_mut() {
        c.set_alpha(alpha);
    }
    if a.dash != "solid" {
        l.dash = Some(Dash::Preset(a.dash.clone()));
    }
    l.tail = Some(LineEnd::arrow(&a.tail));
    let e = |v: f64| (v * EMU_PER_PT as f64) as i64;
    let mut s = shapes::connector(
        id,
        &format!("Arrow {id}"),
        (e(from.0), e(from.1)),
        (e(to.0), e(to.1)),
        l,
    );
    if let Shape::Cxn(c) = &mut s {
        c.props.geom = Some(Geometry::preset(&a.connector));
    }
    s
}

fn main() {
    let dir = PathBuf::from(std::env::var("LORE_STYLE_DIR").expect("LORE_STYLE_DIR"));
    let out = PathBuf::from(std::env::var("LORE_OUT").expect("LORE_OUT"));
    let pack = StylePack::load_dir(&dir).expect("style.toml");
    let tpl = Package::open(dir.join(&pack.pack.template)).expect("template.pptx");
    let mut deck = Deck::from_package(tpl).unwrap();
    assert!(deck.slides.is_empty());

    // ---- cover
    let c = deck.add_slide(&pack.layouts["cover"]).unwrap();
    {
        let doc = &mut deck.slides[c].doc;
        for sh in doc.shapes_mut() {
            if let Shape::Sp(sp) = sh {
                let kind = sp
                    .nv
                    .ph
                    .as_ref()
                    .map(|p| p.kind().to_string())
                    .unwrap_or_default();
                // positions as on the original covers
                let (tag, s, xf) = match kind.as_str() {
                    "ctrTitle" => (
                        "cover-title",
                        "Style Pack Preview: A Synthetic Paper Title",
                        Xfrm::new(393272, 1681899, 8448895, 724635),
                    ),
                    "subTitle" => (
                        "subtitle",
                        "Speaker: ellrin  Supervisor: daidai",
                        Xfrm::new(5230702, 4468564, 2821922, 446568),
                    ),
                    _ => continue,
                };
                sp.props.xfrm = Some(xf);
                if let Some(tb) = sp.text.as_mut() {
                    tb.paragraphs = vec![shapes::paragraph(
                        pack.run(tag, None, s).into_iter().collect(),
                    )];
                }
            }
        }
        let id = doc.next_shape_id();
        doc.shapes_mut().push(text(
            &pack,
            id,
            Rect::pt(31.0, 190.0, 400.0, 24.0),
            &[&[("venue", None, "Journal of Examples, 2026, IF 0.0")]],
        ));
        doc.shapes_mut().push(text(
            &pack,
            id + 1,
            Rect::pt(31.0, 215.0, 400.0, 60.0),
            &[
                &[("body", Some("strong"), "Keywords")],
                &[("body", None, "style pack, round trip, progressive reveal")],
            ],
        ));
    }

    // ---- tags & components
    let s = deck.add_slide(&pack.layouts["content"]).unwrap();
    deck.set_placeholder_text(s, "title", &["Tags and components"])
        .unwrap();
    {
        let doc = &mut deck.slides[s].doc;
        if let Some(Shape::Sp(sp)) = doc
            .shapes_mut()
            .iter_mut()
            .find(|x| x.nv().and_then(|n| n.ph.as_ref()).is_some())
            && let Some(tb) = sp.text.as_mut()
        {
            tb.paragraphs = vec![shapes::paragraph(
                pack.run("title", None, "Tags and components")
                    .into_iter()
                    .collect(),
            )];
        }
        let mut id = doc.next_shape_id();
        let mut next = || {
            id += 1;
            id - 1
        };
        doc.shapes_mut().push(text(
            &pack,
            next(),
            Rect::pt(20.0, 75.0, 330.0, 200.0),
            &[
                &[
                    ("label", None, "Shape generation: "),
                    ("body", None, "body 14 pt with a "),
                    ("body", Some("key"), "key result"),
                ],
                &[
                    ("body-dense", None, "body-dense 12 pt for crowded slides, "),
                    ("body-dense", Some("note"), "x_i = notation note"),
                ],
                &[("takeaway", None, "1. take-home message 16 pt")],
                &[
                    ("caption", None, "caption / legend 12 pt  "),
                    ("caption", Some("concept-1"), "concept-1 "),
                    ("caption", Some("concept-2"), "concept-2 "),
                    ("caption", Some("concept-3"), "concept-3"),
                ],
            ],
        ));
        // numbered circles
        let seq = pack.palette.sequences["numbered-circle"].clone();
        let d = pack.components["numbered-circle"]
            .get("size_in")
            .and_then(|v| v.as_float())
            .unwrap_or(0.24)
            * 72.0;
        for (i, hex) in seq.iter().enumerate() {
            let mut sh = shapes::preset_shape(
                next(),
                "Oval",
                "ellipse",
                Rect::pt(370.0 + i as f64 * 26.0, 80.0, d, d),
                Some(solid(hex, None)),
                Some(Line::new(0.75, "000000")),
            );
            label_text(&mut sh, &pack, "caption", &(i + 1).to_string());
            if let Shape::Sp(sp) = &mut sh
                && let Some(tb) = sp.text.as_mut()
            {
                tb.body.attrs.set("lIns", "0");
                tb.body.attrs.set("rIns", "0");
                tb.body.attrs.set("tIns", "0");
                tb.body.attrs.set("bIns", "0");
            }
            doc.shapes_mut().push(sh);
        }
        // model trapezoid (rotated 90°)
        let mut trap = shapes::preset_shape(
            next(),
            "Encoder",
            "trapezoid",
            Rect::pt(380.0, 120.0, 80.0, 50.0),
            Some(solid("F7D9F0", None)),
            None,
        );
        if let Shape::Sp(sp) = &mut trap {
            sp.props.xfrm.as_mut().unwrap().attrs.set("rot", "5400000");
            sp.props.line = Some(Line {
                fill: Some(Fill::NoFill),
                ..Line::new(0.75, "000000")
            });
        }
        label_text(&mut trap, &pack, "caption", "Encoder");
        doc.shapes_mut().push(trap);
        // tab card: dashed card + small solid label
        let card_tint = tint(&pack, "supervised");
        let mut card = shapes::preset_shape(
            next(),
            "Card",
            "roundRect",
            Rect::pt(490.0, 115.0, 200.0, 80.0),
            Some(solid(&card_tint, None)),
            None,
        );
        if let Shape::Sp(sp) = &mut card {
            sp.props.line = Some(Line {
                dash: Some(Dash::Preset("dash".into())),
                ..Line::new(1.0, "548235")
            });
        }
        doc.shapes_mut().push(card);
        let mut tab = shapes::preset_shape(
            next(),
            "Card label",
            "roundRect",
            Rect::pt(495.0, 106.0, 90.0, 18.7),
            Some(solid(&card_tint, None)),
            Some(Line::new(1.0, "548235")),
        );
        label_text(&mut tab, &pack, "caption", "Supervised");
        doc.shapes_mut().push(tab);
        // arrows: encoder -> card, default/bold/dashed samples
        doc.shapes_mut().push(arrow(
            &pack,
            "default",
            next(),
            (450.0, 145.0),
            (488.0, 150.0),
            None,
        ));
        doc.shapes_mut().push(arrow(
            &pack,
            "bold",
            next(),
            (380.0, 215.0),
            (450.0, 215.0),
            None,
        ));
        doc.shapes_mut().push(arrow(
            &pack,
            "dashed",
            next(),
            (470.0, 215.0),
            (540.0, 215.0),
            None,
        ));
        doc.shapes_mut().push(arrow(
            &pack,
            "elbow",
            next(),
            (560.0, 205.0),
            (640.0, 230.0),
            None,
        ));
        // semantic tints as 35 % overlays
        let alpha = pack.palette.alpha.get("overlay").copied();
        for (i, (name, t)) in pack.palette.tints.iter().enumerate() {
            let mut sw = shapes::preset_shape(
                next(),
                "Tint",
                "rect",
                Rect::pt(370.0 + i as f64 * 46.0, 245.0, 42.0, 30.0),
                Some(solid(&t.hex, alpha)),
                None,
            );
            label_text(&mut sw, &pack, "reference", name);
            doc.shapes_mut().push(sw);
        }
        // native table with the deck's table style + highlight box
        let tbl_id = next();
        let rows = [
            ("Data", "Dataset A, 500 samples"),
            ("Optimizer", "Adam, lr 1e-4"),
            ("Epochs", "200"),
        ];
        let sz = (pack.tags["table"].size * 100.0) as i64;
        let font = &pack.tags["table"].font;
        let mut tr = String::new();
        for (k, v) in rows {
            tr.push_str(&format!(
                r#"<a:tr h="228600"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="{sz}" b="1"><a:latin typeface="{font}"/></a:rPr><a:t>{k}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="{sz}"><a:latin typeface="{font}"/></a:rPr><a:t>{v}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr>"#
            ));
        }
        let table = format!(
            r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{tbl_id}" name="Table {tbl_id}"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="254000" y="3302000"/><a:ext cx="3200000" cy="685800"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{{5940675A-B579-460E-94D1-54222C63F5DA}}</a:tableStyleId></a:tblPr><a:tblGrid><a:gridCol w="1100000"/><a:gridCol w="2100000"/></a:tblGrid>{tr}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#
        );
        doc.shapes_mut()
            .push(Shape::parse(&parse_element(table.as_bytes()).unwrap()));
        let hb = &pack.components["highlight-box"];
        let w = hb
            .get("line_width_pt")
            .and_then(|v| v.as_float())
            .unwrap_or(2.25);
        doc.shapes_mut().push(shapes::preset_shape(
            next(),
            "Highlight",
            "rect",
            Rect::pt(19.0, 278.0, 255.0, 22.0),
            Some(Fill::NoFill),
            Some(Line::new(w, "FF0000")),
        ));
        // OMML equation (Cambria Math), as PowerPoint stores it
        let eq_id = next();
        let r = |t: &str| {
            format!(
                r#"<m:r><a:rPr lang="en-US" sz="1400" i="1"><a:latin typeface="Cambria Math"/></a:rPr><m:t>{t}</m:t></m:r>"#
            )
        };
        let omml = format!(
            r#"<a14:m><m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">{}<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>{}<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub></m:oMath></a14:m>"#,
            r("L="),
            r("L"),
            r("adv"),
            r("+λ"),
            r("L"),
            r("rec")
        );
        let mut eq = shapes::text_box(
            eq_id,
            "Equation",
            Rect::pt(370.0, 290.0, 200.0, 24.0),
            vec![],
        );
        if let Shape::Sp(sp) = &mut eq {
            sp.text.as_mut().unwrap().paragraphs = vec![Paragraph {
                props: None,
                runs: vec![Run::Math(parse_element(omml.as_bytes()).unwrap())],
                end_props: None,
            }];
        }
        doc.shapes_mut().push(Shape::Alt(AltContent {
            attrs: lore_ooxml::xml::Attrs(vec![
                (
                    "xmlns:mc".into(),
                    "http://schemas.openxmlformats.org/markup-compatibility/2006".into(),
                ),
                (
                    "xmlns:a14".into(),
                    "http://schemas.microsoft.com/office/drawing/2010/main".into(),
                ),
            ]),
            choices: vec![AltBranch {
                attrs: lore_ooxml::xml::Attrs(vec![("Requires".into(), "a14".into())]),
                shapes: vec![eq],
            }],
            fallback: None,
        }));
        doc.shapes_mut().push(text(
            &pack,
            next(),
            Rect::pt(60.0, 352.0, 500.0, 14.0),
            &[&[(
                "reference",
                None,
                "Ref: synthetic preview generated by lore-ooxml style_preview",
            )]],
        ));
    }

    // ---- ghosting pair: step 1 normal, step 2 with step-1 objects ghosted
    let logo = deck.pkg.get("ppt/media/image2.png").map(|b| b.to_vec());
    for step in 0..2 {
        let g = deck.add_slide(&pack.layouts["content"]).unwrap();
        deck.set_placeholder_text(
            g,
            "title",
            &[if step == 0 {
                "Progressive reveal (step 1)"
            } else {
                "Progressive reveal (step 2)"
            }],
        )
        .unwrap();
        let rid = logo.clone().map(|b| deck.add_image(g, b, "png").unwrap());
        let doc = &mut deck.slides[g].doc;
        let mut id = doc.next_shape_id();
        let ghost = step == 1;
        let pa = ghost.then_some(pack.ghost.picture_alpha);
        if let Some(rid) = &rid {
            doc.shapes_mut().push(shapes::picture(
                id,
                "Figure",
                rid,
                Rect::pt(30.0, 90.0, 200.0, 120.0),
                Some(SrcRect {
                    l: 0,
                    t: 0,
                    r: 20000,
                    b: 0,
                }),
                pa,
            ));
            id += 1;
        }
        let la = ghost.then_some(pack.ghost.line_alpha);
        doc.shapes_mut().push(arrow(
            &pack,
            "default",
            id,
            (240.0, 150.0),
            (300.0, 150.0),
            la,
        ));
        id += 1;
        let mut t1 = text(
            &pack,
            id,
            Rect::pt(30.0, 220.0, 260.0, 24.0),
            &[&[("body", None, "Step 1: crop of the architecture figure")]],
        );
        if ghost && let Shape::Sp(sp) = &mut t1 {
            for p in &mut sp.text.as_mut().unwrap().paragraphs {
                for r in &mut p.runs {
                    if let Some(Some(rp)) = r.props_mut() {
                        rp.fill = Some(Fill::solid(&pack.ghost.text_color));
                    }
                }
            }
        }
        doc.shapes_mut().push(t1);
        id += 1;
        if ghost {
            let tint_hex = tint(&pack, "adversarial");
            let mut box2 = shapes::preset_shape(
                id,
                "Step 2",
                "roundRect",
                Rect::pt(310.0, 110.0, 200.0, 80.0),
                Some(solid(&tint_hex, None)),
                Some(Line {
                    dash: Some(Dash::Preset("dash".into())),
                    ..Line::new(1.0, "FF0000")
                }),
            );
            label_text(&mut box2, &pack, "label", "Step 2: current part");
            doc.shapes_mut().push(box2);
            // optional animation demo on the current object
            doc.set_entrance_animations(&[EntranceAnim {
                trigger: Trigger::OnClick,
                ..EntranceAnim::new(id, Entrance::Fade)
            }]);
        }
    }

    let bytes = deck.to_bytes().unwrap();
    // sanity: reopen
    let back = Deck::from_bytes(&bytes).unwrap();
    println!("slides: {}", back.slides.len());
    for (i, s) in back.slides.iter().enumerate() {
        println!(
            "  {} [{}] {} objects",
            i + 1,
            back.slide_layout(i).map(|l| l.name()).unwrap_or("?"),
            s.doc.shapes().len()
        );
    }
    // never overwrite an existing file
    assert!(
        !out.exists(),
        "{} exists — choose a new LORE_OUT",
        out.display()
    );
    std::fs::write(&out, bytes).unwrap();
    println!("wrote {}", out.display());

    // optional PNG previews: LORE_PREVIEW_DIR (files that already exist are kept)
    if let Ok(pdir) = std::env::var("LORE_PREVIEW_DIR") {
        let pdir = PathBuf::from(pdir);
        std::fs::create_dir_all(&pdir).unwrap();
        let Some(ren) = visual::Renderer::detect() else {
            println!("no renderer available — skipping previews");
            return;
        };
        let work = out.with_extension("render");
        let pdf = ren.to_pdf(&out, &work).expect("render");
        let tmp = work.join("png");
        std::fs::create_dir_all(&tmp).unwrap();
        let st = std::process::Command::new("pdftoppm")
            .args(["-png", "-r", "144"])
            .arg(&pdf)
            .arg(tmp.join("slide"))
            .status()
            .expect("pdftoppm");
        assert!(st.success());
        let mut pngs: Vec<_> = std::fs::read_dir(&tmp)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        pngs.sort();
        for (i, p) in pngs.iter().enumerate() {
            let dst = pdir.join(format!("slide-{}.png", i + 1));
            if dst.exists() {
                println!("keep existing {}", dst.display());
            } else {
                std::fs::copy(p, &dst).unwrap();
                println!("preview {} ({})", dst.display(), ren.name());
            }
        }
    }
}
