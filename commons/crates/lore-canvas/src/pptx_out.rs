//! Canvas scene → PPTX (lore-ooxml), lossless for untouched content:
//!
//! * a slide whose content fingerprint is unchanged keeps its original part
//!   (bytes identical);
//! * an edited slide is rebuilt object by object: untouched objects are their
//!   original XML; edited objects are their original XML with the changed
//!   fields patched; new objects are built from scratch;
//! * new / duplicated slides get new parts (rels copied from their source);
//!   removed slides are dropped; the slide list follows the canvas order.

use std::collections::{BTreeMap, BTreeSet};

use lore_ooxml::Deck as ODeck;
use lore_ooxml::anim::{Entrance, EntranceAnim, Trigger};
use lore_ooxml::deck::{NotesEntry, SlideEntry};
use lore_ooxml::model::{self as om, Shape};
use lore_ooxml::package::*;
use lore_ooxml::xml::{Attrs, Element, Node, parse_element};

use crate::model::*;
use crate::style::{GHOST_EM, StyleSystem};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("ooxml: {0}")]
    Ooxml(#[from] lore_ooxml::Error),
    #[error("{0}")]
    Other(String),
}

/// Source of `file:` assets (bytes of a file inside the deck folder).
pub trait Assets {
    fn get(&self, path: &str) -> Option<Vec<u8>>;
}
impl<F: Fn(&str) -> Option<Vec<u8>>> Assets for F {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self(path)
    }
}

const NOTES_CT: &str = "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";

fn parse_shape(xml: &str) -> Option<Shape> {
    parse_element(xml.as_bytes()).ok().map(|e| Shape::parse(&e))
}


fn set_xfrm(x: &mut Option<om::Xfrm>, f: &Frame, tag: &str) {
    let xf = x.get_or_insert_with(|| om::Xfrm {
        tag: tag.into(),
        ..Default::default()
    });
    xf.off = Some((f.x, f.y));
    xf.ext = Some((f.w.max(0), f.h.max(0)));
    let rot = (f.rot.rem_euclid(360.0) * 60_000.0).round() as i64;
    if rot != 0 {
        xf.attrs.set("rot", rot.to_string());
    } else {
        xf.attrs.remove("rot");
    }
    for (k, v) in [("flipH", f.flip_h), ("flipV", f.flip_v)] {
        if v {
            xf.attrs.set(k, "1");
        } else {
            xf.attrs.remove(k);
        }
    }
}

fn color_el(hex: &str, alpha: f64) -> om::Color {
    let mut c = om::Color::srgb(hex);
    if alpha < 0.999 {
        c.set_alpha(Some(alpha));
    }
    c
}

/// `a:gradFill` from stops (pos 0..1), a linear angle in degrees, or a path (radial) gradient.
pub(crate) fn grad_fill(stops: &[Stop], angle: f64, path: bool) -> Element {
    let mut gs = Element::new("a:gsLst");
    for st in stops {
        let mut g = Element::new("a:gs").with_attr("pos", ((st.pos.clamp(0.0, 1.0) * 100_000.0).round() as i64).to_string());
        g.push(color_el(&st.color, st.alpha).to_element());
        gs.push(g);
    }
    let mut e = Element::new("a:gradFill").with_attr("rotWithShape", "1");
    e.push(gs);
    if path {
        let mut p = Element::new("a:path").with_attr("path", "circle");
        p.push(Element::new("a:fillToRect").with_attr("l", "50000").with_attr("t", "50000").with_attr("r", "50000").with_attr("b", "50000"));
        e.push(p);
    } else {
        e.push(Element::new("a:lin").with_attr("ang", ((angle * 60_000.0).round() as i64).to_string()).with_attr("scaled", "0"));
    }
    e
}

fn fill_of(p: &Paint, orig: Option<&om::Fill>) -> Option<om::Fill> {
    match p {
        Paint::None => Some(om::Fill::NoFill),
        Paint::Solid { color, alpha } => Some(om::Fill::Solid(Some(color_el(color, *alpha)))),
        Paint::Gradient { stops, angle, path } => match orig {
            // alpha edits on a kept gradient: patch stop colours' alpha
            Some(om::Fill::Gradient(e)) => {
                let mut e = e.clone();
                if let Some(gs) = e.child_mut("a:gsLst") {
                    for (g, st) in gs.elements_mut().zip(stops.iter()) {
                        if let Some(Node::Element(c)) = g.children.iter_mut().find(|n| matches!(n, Node::Element(_))) {
                            let mut col = om::Color::parse(c);
                            col.set_alpha(if st.alpha < 0.999 { Some(st.alpha) } else { None });
                            *c = col.to_element();
                        }
                    }
                }
                Some(om::Fill::Gradient(e))
            }
            // a new gradient (a base's band, a takeaway bar): built from the stops
            _ => Some(om::Fill::Gradient(grad_fill(stops, *angle, *path))),
        },
        Paint::Other { .. } => orig.cloned(),
    }
}

fn line_of(s: &Option<Stroke>, orig: Option<&om::Line>) -> Option<om::Line> {
    let Some(s) = s else {
        let mut l = orig.cloned().unwrap_or(om::Line {
            tag: "a:ln".into(),
            ..Default::default()
        });
        l.fill = Some(om::Fill::NoFill);
        return Some(l);
    };
    let mut l = orig.cloned().unwrap_or(om::Line {
        tag: "a:ln".into(),
        ..Default::default()
    });
    l.set_width_pt(s.width_pt);
    l.fill = Some(om::Fill::Solid(Some(color_el(&s.color, s.alpha))));
    l.dash = if s.dash == "solid" && l.dash.is_none() {
        None
    } else {
        Some(om::Dash::Preset(s.dash.clone()))
    };
    let end = |k: &str, prev: Option<&om::LineEnd>| -> Option<om::LineEnd> {
        if k == "none" && prev.is_none() {
            return None;
        }
        let mut e = prev.cloned().unwrap_or_default();
        e.attrs.set("type", k);
        Some(e)
    };
    l.head = end(&s.head, l.head.as_ref());
    l.tail = end(&s.tail, l.tail.as_ref());
    Some(l)
}

fn geom_to(g: &Geom, orig: Option<&om::Geometry>) -> om::Geometry {
    if g.prst == "custom" {
        if let (Some(om::Geometry::Custom(e)), None) = (orig, &g.path) {
            return om::Geometry::Custom(e.clone());
        }
        return om::Geometry::Custom(cust_geom(g.path.as_deref().unwrap_or("")));
    }
    let extras = match orig {
        Some(om::Geometry::Preset { extras, prst, .. }) if *prst == g.prst => extras.clone(),
        _ => Default::default(),
    };
    om::Geometry::Preset {
        prst: g.prst.clone(),
        adj: Some(
            g.adj
                .iter()
                .map(|(k, v)| (k.clone(), format!("val {}", v.round() as i64)))
                .collect(),
        ),
        extras,
    }
}

/// SVG path (0..1 box, M/L/C/Q/Z) → `a:custGeom` with a 100000 × 100000 path space.
pub fn cust_geom(path: &str) -> Element {
    const S: f64 = 100_000.0;
    let mut p = Element::new("a:path").with_attr("w", "100000").with_attr("h", "100000");
    let toks: Vec<String> = path
        .replace(',', " ")
        .split_whitespace()
        .flat_map(|t| {
            // split leading command letters from numbers ("M0.1" → "M", "0.1")
            let mut v = Vec::new();
            let mut cur = String::new();
            for ch in t.chars() {
                if ch.is_ascii_alphabetic() {
                    if !cur.is_empty() {
                        v.push(std::mem::take(&mut cur));
                    }
                    v.push(ch.to_string());
                } else {
                    cur.push(ch);
                }
            }
            if !cur.is_empty() {
                v.push(cur);
            }
            v
        })
        .collect();
    let mut i = 0;
    let num = |i: &mut usize| -> f64 {
        let v = toks.get(*i).and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0);
        *i += 1;
        v
    };
    let pt = |x: f64, y: f64| {
        Element::new("a:pt")
            .with_attr("x", ((x * S).round() as i64).to_string())
            .with_attr("y", ((y * S).round() as i64).to_string())
    };
    while i < toks.len() {
        let cmd = toks[i].clone();
        i += 1;
        let (name, n) = match cmd.as_str() {
            "M" => ("a:moveTo", 1),
            "L" => ("a:lnTo", 1),
            "C" => ("a:cubicBezTo", 3),
            "Q" => ("a:quadBezTo", 2),
            "Z" | "z" => {
                p.push(Element::new("a:close"));
                continue;
            }
            _ => continue,
        };
        let mut e = Element::new(name);
        for _ in 0..n {
            let x = num(&mut i);
            let y = num(&mut i);
            e.push(pt(x, y));
        }
        p.push(e);
    }
    // one connection site per vertex (same order as geom::sites), positions as guides of w / h
    let mut gd = Element::new("a:gdLst");
    let mut cxn = Element::new("a:cxnLst");
    for (k, (x, y)) in crate::geom::path_vertices(path).into_iter().enumerate() {
        gd.push(Element::new("a:gd").with_attr("name", format!("cx{k}")).with_attr("fmla", format!("*/ w {} 100000", (x * S).round() as i64)));
        gd.push(Element::new("a:gd").with_attr("name", format!("cy{k}")).with_attr("fmla", format!("*/ h {} 100000", (y * S).round() as i64)));
        let mut c = Element::new("a:cxn").with_attr("ang", "0");
        c.push(Element::new("a:pos").with_attr("x", format!("cx{k}")).with_attr("y", format!("cy{k}")));
        cxn.push(c);
    }
    let mut g = Element::new("a:custGeom");
    g.push(Element::new("a:avLst"));
    g.push(gd);
    g.push(Element::new("a:ahLst"));
    g.push(cxn);
    g.push(
        Element::new("a:rect")
            .with_attr("l", "0")
            .with_attr("t", "0")
            .with_attr("r", "r")
            .with_attr("b", "b"),
    );
    g.push(Element::new("a:pathLst").with_child(p));
    g
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn rpr_from_look(base: Option<om::RunProps>, look: &Look, tag: &str) -> om::RunProps {
    let mut r = base.unwrap_or_else(|| {
        let mut r = om::RunProps::new();
        r.attrs.set("lang", "en-US");
        r.attrs.set("altLang", "zh-TW");
        r
    });
    r.tag = tag.into();
    r.set_size_pt(look.size);
    r.attrs.set("b", if look.bold { "1" } else { "0" });
    if look.italic {
        r.attrs.set("i", "1");
    } else {
        r.attrs.remove("i");
    }
    if look.underline {
        r.attrs.set("u", "sng");
    } else {
        r.attrs.remove("u");
    }
    if look.strike {
        r.attrs.set("strike", "sngStrike");
    } else {
        r.attrs.remove("strike");
    }
    if look.baseline != 0.0 {
        r.attrs.set("baseline", ((look.baseline * 1000.0).round() as i64).to_string());
    } else {
        r.attrs.remove("baseline");
    }
    r.attrs.remove("dirty");
    r.fill = Some(om::Fill::Solid(Some(color_el(&look.color, look.alpha))));
    let ea = if look.ea_font.is_empty() { None } else { Some(look.ea_font.as_str()) };
    r.set_font(&look.font, ea);
    r
}

fn run_look(style: Option<&StyleSystem>, r: &Run) -> Look {
    match (style, &r.tag) {
        (Some(st), Some(t)) => {
            let mut l = st.look(t, r.em.as_deref()).unwrap_or_else(|| r.look.clone());
            // size may sit anywhere inside the tag's range
            if let Some(ts) = st.tag(t)
                && let Some([a, b]) = ts.size_range
                && r.look.size >= a
                && r.look.size <= b
            {
                l.size = r.look.size;
            }
            l
        }
        (Some(st), None) if r.em.as_deref() == Some(GHOST_EM) => {
            let mut l = r.look.clone();
            l.color = st.pack.ghost.text_color.clone();
            l
        }
        _ => r.look.clone(),
    }
}

/// Set the colour of every `a:rPr` inside an OMML element (PowerPoint flavour).
pub fn recolor_math(e: &mut Element, hex: &str) {
    for c in e.children.iter_mut() {
        if let Node::Element(c) = c {
            if c.name == "a:rPr" {
                c.children.retain(|n| !matches!(n, Node::Element(x) if om::Fill::is_fill(&x.name)));
                let pos = c
                    .children
                    .iter()
                    .position(|n| matches!(n, Node::Element(x) if x.local() == "ln"))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                c.children.insert(pos, Node::Element(om::Fill::solid(hex).to_element()));
            } else {
                recolor_math(c, hex);
            }
        }
    }
}

fn set_bullet(pp: &mut om::ParaProps, b: &Bullet) {
    pp.bullet = Some(match b {
        Bullet::None => om::Bullet::None,
        Bullet::Char { ch } => {
            if pp.bullet_font.is_none() {
                pp.bullet_font = Some(
                    Element::new("a:buFont")
                        .with_attr("typeface", "Arial")
                        .with_attr("panose", "020B0604020202020204")
                        .with_attr("pitchFamily", "34")
                        .with_attr("charset", "0"),
                );
            }
            om::Bullet::Char(Attrs(vec![("char".into(), ch.clone())]))
        }
        Bullet::Num { scheme, start } => {
            let mut a = Attrs(vec![("type".into(), scheme.clone())]);
            if *start != 1 {
                a.set("startAt", start.to_string());
            }
            if pp.bullet_font.is_none() {
                pp.bullet_font = Some(Element::new("a:buFont").with_attr("typeface", "+mj-lt"));
            }
            om::Bullet::AutoNum(a)
        }
    });
}

fn para_props(p: &Para, base: Option<om::ParaProps>) -> om::ParaProps {
    let mut pp = base.unwrap_or_else(om::ParaProps::new);
    pp.tag = "a:pPr".into();
    let set = |pp: &mut om::ParaProps, k: &str, v: String, default: &str| {
        if v == default && pp.attrs.get(k).is_none() {
            return;
        }
        pp.attrs.set(k, v);
    };
    set(&mut pp, "marL", p.mar_l.to_string(), "0");
    set(&mut pp, "indent", p.indent.to_string(), "0");
    if p.level > 0 {
        pp.attrs.set("lvl", p.level.to_string());
    } else {
        pp.attrs.remove("lvl");
    }
    pp.attrs.set("algn", p.align.clone());
    pp.line_spacing = match (p.line, p.line_pt) {
        (_, Some(pt)) => Some(om::Spacing::Pts((pt * 100.0).round() as i64)),
        (Some(m), None) => Some(om::Spacing::Pct((m * 100_000.0).round() as i64)),
        _ => None,
    };
    pp.space_before = p.before_pt.map(|v| om::Spacing::Pts((v * 100.0).round() as i64)).or(
        if pp.space_before.is_some() { Some(om::Spacing::Pts(0)) } else { None },
    );
    pp.space_after = p.after_pt.map(|v| om::Spacing::Pts((v * 100.0).round() as i64)).or(
        if pp.space_after.is_some() { Some(om::Spacing::Pts(0)) } else { None },
    );
    set_bullet(&mut pp, &p.bullet);
    pp
}

/// Build the ooxml text body of `cb`, reusing the original paragraphs/runs.
pub(crate) fn text_out(cb: &TextBody, orig: Option<&om::TextBody>, style: Option<&StyleSystem>, tag: &str) -> om::TextBody {
    let mut tb = orig.cloned().unwrap_or_else(|| {
        let mut t = om::TextBody::new(tag);
        t.body.attrs = Attrs(vec![("rtlCol".into(), "0".into())]);
        t
    });
    let b = &mut tb.body.attrs;
    let def = [91_440, 45_720, 91_440, 45_720];
    for (i, k) in ["lIns", "tIns", "rIns", "bIns"].iter().enumerate() {
        if cb.insets[i] != def[i] || b.get(k).is_some() {
            b.set(k, cb.insets[i].to_string());
        }
    }
    if cb.anchor != "t" || b.get("anchor").is_some() {
        b.set("anchor", cb.anchor.clone());
    }
    if !cb.wrap {
        b.set("wrap", "none");
    } else if b.get("wrap").is_some() {
        b.set("wrap", "square");
    }
    if let Some(v) = &cb.vert {
        b.set("vert", v.clone());
    }
    tb.body.autofit = match cb.autofit.as_str() {
        "norm" => {
            let mut a = Attrs::new();
            if cb.font_scale < 0.999 {
                a.set("fontScale", ((cb.font_scale * 100_000.0).round() as i64).to_string());
            }
            if cb.ln_reduction > 0.0 {
                a.set("lnSpcReduction", ((cb.ln_reduction * 100_000.0).round() as i64).to_string());
            }
            Some(om::Autofit::Normal(a))
        }
        "shape" => Some(om::Autofit::Shape),
        _ => {
            if tb.body.autofit.is_some() {
                Some(om::Autofit::None)
            } else {
                None
            }
        }
    };
    let oparas: Vec<om::Paragraph> = orig.map(|o| o.paragraphs.clone()).unwrap_or_default();
    let mut paras = Vec::new();
    for p in &cb.paras {
        let base = p.src.and_then(|i| oparas.get(i as usize));
        let props = match base {
            Some(b) if para_k(p) == p.k => b.props.clone(),
            Some(b) => Some(para_props(p, b.props.clone())),
            None => {
                let empty = para_k(&Para::new(vec![]));
                if para_k(p) == empty {
                    None
                } else {
                    Some(para_props(p, None))
                }
            }
        };
        let mut runs = Vec::new();
        for r in &p.runs {
            let orun = r
                .src
                .and_then(|[pi, ri]| oparas.get(pi as usize).and_then(|op| op.runs.get(ri as usize)));
            let same = orun.is_some() && run_k(r) == r.k;
            let look = run_look(style, r);
            let o = match r.kind.as_str() {
                "text" => match orun {
                    Some(om::Run::Text { props, .. }) if same => om::Run::Text {
                        props: props.clone(),
                        text: r.text.clone(),
                    },
                    Some(om::Run::Text { props, .. }) => om::Run::Text {
                        props: Some(rpr_from_look(props.clone(), &look, "a:rPr")),
                        text: r.text.clone(),
                    },
                    _ => om::Run::Text {
                        props: Some(rpr_from_look(None, &look, "a:rPr")),
                        text: r.text.clone(),
                    },
                },
                "br" => match orun {
                    Some(x @ om::Run::Break { .. }) if same => x.clone(),
                    _ => om::Run::Break {
                        props: Some(rpr_from_look(None, &look, "a:rPr")),
                    },
                },
                "field" => match orun {
                    Some(om::Run::Field { attrs, props, para_props, .. }) => om::Run::Field {
                        attrs: attrs.clone(),
                        props: if same { props.clone() } else { Some(rpr_from_look(props.clone(), &look, "a:rPr")) },
                        para_props: para_props.clone(),
                        text: Some(r.text.clone()),
                    },
                    _ => om::Run::Field {
                        attrs: Attrs(vec![
                            ("id".into(), "{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}".into()),
                            ("type".into(), r.field.clone().unwrap_or_else(|| "slidenum".into())),
                        ]),
                        props: Some(rpr_from_look(None, &look, "a:rPr")),
                        para_props: None,
                        text: Some(r.text.clone()),
                    },
                },
                "math" => match orun {
                    Some(x) if same => x.clone(),
                    _ => match r.omml.as_deref().and_then(|m| parse_element(m.as_bytes()).ok()) {
                        Some(mut e) => {
                            // emphasis on an equation (ghost, key …) recolours every math run
                            if r.em.is_some() {
                                recolor_math(&mut e, &look.color);
                            }
                            om::Run::Math(e)
                        }
                        None => continue,
                    },
                },
                _ => match orun {
                    Some(x) => x.clone(),
                    None => continue,
                },
            };
            runs.push(o);
        }
        let end_props = match (base, &p.end) {
            (Some(b), _) if b.end_props.is_some() && para_k(p) == p.k => b.end_props.clone(),
            (Some(b), Some(l)) => Some(rpr_from_look(b.end_props.clone(), l, "a:endParaRPr")),
            (None, Some(l)) => Some(rpr_from_look(None, l, "a:endParaRPr")),
            (b, None) => b.and_then(|b| b.end_props.clone()),
        };
        paras.push(om::Paragraph {
            props,
            runs,
            end_props,
        });
    }
    if paras.is_empty() {
        paras.push(om::Paragraph::default());
    }
    tb.paragraphs = paras;
    tb
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

pub(crate) struct Out<'a> {
    pub pkg: &'a mut Package,
    pub part: String,
    pub rels: Relationships,
    pub style: Option<&'a StyleSystem>,
    pub assets: &'a dyn Assets,
    pub next_id: u32,
    /// Uid of the slide being written (names equation fallback pictures).
    pub slide: String,
}

/// The asset path under which an app offers the fallback picture of an
/// equation box: a PNG of how the box looks, for viewers without Office Math
/// (Keynote, Google Slides, LibreOffice). Asked only for new or edited boxes;
/// PowerPoint's own fallback of an untouched box is kept.
pub fn equation_fallback_key(slide_uid: &str, object_id: u32) -> String {
    format!("fallback:{slide_uid}:{object_id}")
}

/// Equation boxes that export will ask a fallback picture for (new, or text
/// changed), with their slide uid: render these before exporting.
pub fn equations_needing_fallback(deck: &Deck) -> Vec<(String, u32, Frame)> {
    fn walk(slide: &str, objs: &[Object], out: &mut Vec<(String, u32, Frame)>) {
        for o in objs {
            match &o.kind {
                Kind::Text(sp) | Kind::Shape(sp) if sp.equation => {
                    let changed = crate::model::changed_fields(o);
                    if o.src.is_none() || changed.iter().any(|c| c == "text") {
                        out.push((slide.to_string(), o.id, o.frame.clone()));
                    }
                }
                Kind::Group(g) => walk(slide, &g.children, out),
                _ => {}
            }
        }
    }
    let mut out = vec![];
    for s in &deck.slides {
        walk(&s.uid, &s.objects, &mut out);
    }
    out
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

impl Out<'_> {
    /// Relationship id of an image asset for the current slide (reused when present).
    fn image_rel(&mut self, asset: &str) -> Option<String> {
        let target = if let Some(p) = asset.strip_prefix("pkg:") {
            if !self.pkg.has(p) {
                return None;
            }
            p.to_string()
        } else if asset.starts_with("file:") || asset.starts_with("fallback:") {
            let (bytes, ext) = match asset.strip_prefix("file:") {
                Some(f) => (self.assets.get(f)?, f.rsplit('.').next().unwrap_or("png").to_lowercase()),
                None => (self.assets.get(asset)?, "png".to_string()),
            };
            // identical media already in the package → share it
            if let Some(p) = self
                .pkg
                .parts
                .iter()
                .find(|p| p.name.starts_with("ppt/media/") && p.data == bytes)
            {
                p.name.clone()
            } else {
                let name = self.pkg.free_name("ppt/media/image", &format!(".{ext}"));
                self.pkg.set(&name, bytes);
                let mut ct = ContentTypes::load(self.pkg).ok()?;
                ct.ensure_default(&ext, mime(&ext));
                ct.store(self.pkg);
                name
            }
        } else {
            return None;
        };
        if let Some(r) = self
            .rels
            .items
            .iter()
            .find(|r| r.rel_type == REL_IMAGE && !r.external && resolve_target(&self.part, &r.target) == target)
        {
            return Some(r.id.clone());
        }
        Some(self.rels.add(REL_IMAGE, &relative_target(&self.part, &target)))
    }

    fn fresh_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }
}

fn set_nv(nv: &mut om::NvProps, o: &Object, changed: &[String]) {
    let all = changed.iter().any(|c| c == "*new*");
    if all || changed.iter().any(|c| c == "name") {
        nv.cnv_attrs.set("name", o.name.clone());
    }
    if all || changed.iter().any(|c| c == "hidden") {
        if o.hidden {
            nv.cnv_attrs.set("hidden", "1");
        } else {
            nv.cnv_attrs.remove("hidden");
        }
    }
    nv.cnv_attrs.set("id", o.id.to_string());
}

fn has(changed: &[String], k: &str) -> bool {
    changed.iter().any(|c| c == k || c == "*new*" || c == "type")
}

/// Export one object (plus mask shapes for pictures).
pub(crate) fn object_out(out: &mut Out, o: &Object) -> Vec<Shape> {
    let orig = o.src.as_ref().and_then(|s| parse_shape(&s.xml));
    if untouched(o)
        && let Some(s) = &orig
    {
        return vec![s.clone()];
    }
    let changed = changed_fields(o);
    let mut extra = Vec::new();
    let shape = match &o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            let (mut s, alt) = match orig {
                Some(Shape::Sp(s)) => (s, None),
                Some(Shape::Alt(a))
                    if matches!(a.choices.first().and_then(|c| c.shapes.first()), Some(Shape::Sp(_))) =>
                {
                    let Some(Shape::Sp(s)) = a.choices[0].shapes.first().cloned() else { unreachable!() };
                    (s, Some(a))
                }
                _ => {
                    let sh = if matches!(o.kind, Kind::Text(_)) {
                        lore_ooxml::shapes::text_box(o.id, &o.name, lore_ooxml::shapes::Rect { x: 0, y: 0, cx: 0, cy: 0 }, vec![])
                    } else {
                        lore_ooxml::shapes::preset_shape(o.id, &o.name, &sp.geom.prst, lore_ooxml::shapes::Rect { x: 0, y: 0, cx: 0, cy: 0 }, None, None)
                    };
                    let Shape::Sp(mut s) = sh else { unreachable!() };
                    if let Some(ph) = &o.ph {
                        // new placeholder (title of a new slide): inherits position/style from the layout
                        s.nv.kind.attrs.remove("txBox");
                        s.nv.kind.locks = Some(Element::new("a:spLocks").with_attr("noGrp", "1"));
                        let mut a = Attrs::new();
                        if ph != "obj" && ph != "body" {
                            a.set("type", ph.clone());
                        } else {
                            a.set("idx", "1");
                        }
                        s.nv.ph = Some(om::Placeholder { attrs: a, children: vec![] });
                    }
                    (s, None)
                }
            };
            set_nv(&mut s.nv, o, &changed);
            if has(&changed, "frame") {
                set_xfrm(&mut s.props.xfrm, &o.frame, "a:xfrm");
            }
            if has(&changed, "geom") {
                s.props.geom = Some(geom_to(&sp.geom, s.props.geom.as_ref()));
            }
            if has(&changed, "fill") {
                if let Some(f) = fill_of(&sp.fill, s.props.fill.as_ref()) {
                    s.props.fill = Some(f);
                }
            }
            if has(&changed, "line") {
                s.props.line = line_of(&sp.line, s.props.line.as_ref());
            }
            if has(&changed, "text") {
                s.text = sp
                    .text
                    .as_ref()
                    .map(|t| text_out(t, s.text.as_ref(), out.style, "p:txBody"));
            }
            // A picture of the box for viewers without Office Math, when the
            // equation is new or its text changed and the app offers one.
            let wants_fallback = sp.equation && (o.src.is_none() || has(&changed, "text"));
            let fallback = if wants_fallback {
                out.image_rel(&equation_fallback_key(&out.slide, o.id)).map(|rid| {
                    let f = &o.frame;
                    let pic = lore_ooxml::shapes::picture(o.id, &o.name, &rid, lore_ooxml::shapes::Rect { x: f.x, y: f.y, cx: f.w, cy: f.h }, None, None);
                    om::AltBranch { attrs: Attrs(vec![]), shapes: vec![pic] }
                })
            } else {
                None
            };
            match alt {
                Some(mut a) => {
                    a.choices[0].shapes[0] = Shape::Sp(s);
                    if fallback.is_some() {
                        a.fallback = fallback;
                    }
                    Shape::Alt(a)
                }
                None if sp.equation && o.src.is_none() => Shape::Alt(om::AltContent {
                    attrs: Attrs(vec![
                        ("xmlns:mc".into(), "http://schemas.openxmlformats.org/markup-compatibility/2006".into()),
                        ("xmlns:a14".into(), "http://schemas.microsoft.com/office/drawing/2010/main".into()),
                    ]),
                    choices: vec![om::AltBranch {
                        attrs: Attrs(vec![("Requires".into(), "a14".into())]),
                        shapes: vec![Shape::Sp(s)],
                    }],
                    fallback,
                }),
                None => Shape::Sp(s),
            }
        }
        Kind::Connector(c) => {
            let mut x = match orig {
                Some(Shape::Cxn(x)) => x,
                _ => {
                    let Shape::Cxn(x) = lore_ooxml::shapes::connector(o.id, &o.name, (0, 0), (0, 0), om::Line::new(1.0, "000000")) else {
                        unreachable!()
                    };
                    x
                }
            };
            set_nv(&mut x.nv, o, &changed);
            if has(&changed, "frame") {
                set_xfrm(&mut x.props.xfrm, &o.frame, "a:xfrm");
            }
            if has(&changed, "geom") || has(&changed, "adj") {
                x.props.geom = Some(geom_to(
                    &Geom {
                        prst: c.geom.clone(),
                        adj: c.adj.clone(),
                        path: None,
                    },
                    x.props.geom.as_ref(),
                ));
            }
            if has(&changed, "line") {
                x.props.line = line_of(&c.line, x.props.line.as_ref());
            }
            if has(&changed, "start") {
                x.nv.kind.st_cxn = c.start.as_ref().map(|a| (a.id, a.site));
            }
            if has(&changed, "end") {
                x.nv.kind.end_cxn = c.end.as_ref().map(|a| (a.id, a.site));
            }
            Shape::Cxn(x)
        }
        Kind::Picture(p) => {
            let mut pic = match orig {
                Some(Shape::Pic(p)) => p,
                _ => {
                    let rid = out.image_rel(&p.asset).unwrap_or_default();
                    let Shape::Pic(x) = lore_ooxml::shapes::picture(o.id, &o.name, &rid, lore_ooxml::shapes::Rect { x: 0, y: 0, cx: 0, cy: 0 }, None, None) else {
                        unreachable!()
                    };
                    x
                }
            };
            set_nv(&mut pic.nv, o, &changed);
            if has(&changed, "frame") {
                set_xfrm(&mut pic.props.xfrm, &o.frame, "a:xfrm");
            }
            if has(&changed, "asset") && o.src.is_some() {
                if let Some(rid) = out.image_rel(&p.asset)
                    && let Some(b) = pic.fill.blip.as_mut()
                {
                    b.attrs.set("r:embed", rid);
                }
            }
            if has(&changed, "crop") {
                let c = |v: f64| (v * 100_000.0).round() as i64;
                let r = om::SrcRect {
                    l: c(p.crop.l),
                    t: c(p.crop.t),
                    r: c(p.crop.r),
                    b: c(p.crop.b),
                };
                pic.fill.set_crop(Some(r));
            }
            if has(&changed, "alpha")
                && let Some(b) = pic.fill.blip.as_mut()
            {
                b.set_alpha(if p.alpha < 0.999 { Some(p.alpha) } else { None });
            }
            if has(&changed, "line") {
                pic.props.line = match &p.line {
                    None if pic.props.line.is_none() => None,
                    l => line_of(l, pic.props.line.as_ref()),
                };
            }
            if has(&changed, "clip") {
                pic.props.geom = Some(geom_to(&Geom::preset(&p.clip), pic.props.geom.as_ref()));
            }
            for (i, m) in p.masks.iter().enumerate() {
                let f = &o.frame;
                let r = lore_ooxml::shapes::Rect {
                    x: f.x + (m.x * f.w as f64).round() as i64,
                    y: f.y + (m.y * f.h as f64).round() as i64,
                    cx: (m.w * f.w as f64).round() as i64,
                    cy: (m.h * f.h as f64).round() as i64,
                };
                let id = out.fresh_id();
                let mut sh = lore_ooxml::shapes::preset_shape(
                    id,
                    &format!("SC-mask:{}:{i}", o.id),
                    "rect",
                    r,
                    Some(om::Fill::solid("FFFFFF")),
                    Some({
                        let mut l = om::Line::new(0.0, "FFFFFF");
                        l.fill = Some(om::Fill::NoFill);
                        l
                    }),
                );
                if let Shape::Sp(s) = &mut sh {
                    s.text = None;
                }
                extra.push(sh);
            }
            Shape::Pic(pic)
        }
        Kind::Group(g) => {
            let mut grp = match orig {
                Some(Shape::Group(g)) => g,
                _ => {
                    let Shape::Group(g) = lore_ooxml::shapes::group(o.id, &o.name, vec![]) else { unreachable!() };
                    g
                }
            };
            set_nv(&mut grp.nv, o, &changed);
            set_xfrm(&mut grp.props.xfrm, &o.frame, "a:xfrm");
            if let Some(x) = grp.props.xfrm.as_mut() {
                x.ch_off = Some((g.ch.x, g.ch.y));
                x.ch_ext = Some((g.ch.w, g.ch.h));
            }
            grp.children = g.children.iter().flat_map(|c| object_out(out, c)).collect();
            Shape::Group(grp)
        }
        Kind::Table(t) => {
            let mut f = match orig {
                Some(Shape::Frame(f)) => f,
                _ => {
                    let Shape::Frame(f) = Shape::parse(&parse_element(new_table_xml(o.id, &o.name).as_bytes()).expect("static xml")) else {
                        unreachable!()
                    };
                    f
                }
            };
            set_nv(&mut f.nv, o, &changed);
            if has(&changed, "frame") {
                set_xfrm(&mut f.xfrm, &o.frame, "p:xfrm");
            }
            if let om::FrameContent::Table(ot) = &mut f.content
                && (has(&changed, "rows") || has(&changed, "cols") || has(&changed, "style_id") || has(&changed, "first_row") || has(&changed, "band_row"))
            {
                table_out(t, ot, out.style);
            }
            Shape::Frame(f)
        }
        Kind::Locked(_) => match orig {
            Some(s) => s,
            None => return vec![],
        },
    };
    let mut v = vec![shape];
    v.extend(extra);
    v
}

fn new_table_xml(id: u32, name: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{{5940675A-B579-460E-94D1-54222C63F5DA}}</a:tableStyleId></a:tblPr><a:tblGrid/></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
        name.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
    )
}

fn border_el(tag: &str, s: &Stroke) -> Element {
    let mut l = om::Line {
        tag: tag.into(),
        ..Default::default()
    };
    if s.width_pt <= 0.0 {
        l.attrs.set("w", "12700");
        l.fill = Some(om::Fill::NoFill);
    } else {
        l.set_width_pt(s.width_pt);
        l.fill = Some(om::Fill::Solid(Some(color_el(&s.color, s.alpha))));
        l.dash = Some(om::Dash::Preset(s.dash.clone()));
    }
    l.to_element()
}

fn table_out(t: &TableObj, ot: &mut om::Table, style: Option<&StyleSystem>) {
    let orows = ot.rows.clone();
    let ogrid = ot.grid.clone();
    ot.grid = t
        .cols
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let mut g = ogrid.get(i).cloned().unwrap_or_else(|| Element::new("a:gridCol"));
            g.attrs.set("w", w.to_string());
            g
        })
        .collect();
    if let Some(p) = ot.props.as_mut() {
        for (k, v) in [("firstRow", t.first_row), ("bandRow", t.band_row)] {
            if v {
                p.attrs.set(k, "1");
            } else {
                p.attrs.remove(k);
            }
        }
    }
    ot.rows = t
        .rows
        .iter()
        .map(|r| {
            let base = r.src.and_then(|i| orows.get(i as usize));
            let mut row = om::TableRow {
                attrs: base.map(|b| b.attrs.clone()).unwrap_or_default(),
                cells: vec![],
                ext_lst: base.and_then(|b| b.ext_lst.clone()),
            };
            row.attrs.set("h", r.h.to_string());
            for c in &r.cells {
                let oc = c
                    .src
                    .and_then(|[ri, ci]| orows.get(ri as usize).and_then(|x| x.cells.get(ci as usize)));
                let mut cell = oc.cloned().unwrap_or_default();
                for (k, v, d) in [("gridSpan", c.span, 1), ("rowSpan", c.rspan, 1)] {
                    if v != d {
                        cell.attrs.set(k, v.to_string());
                    } else {
                        cell.attrs.remove(k);
                    }
                }
                for (k, v) in [("hMerge", c.hmerge), ("vMerge", c.vmerge)] {
                    if v {
                        cell.attrs.set(k, "1");
                    } else {
                        cell.attrs.remove(k);
                    }
                }
                cell.text = Some(text_out(&c.text, cell.text.as_ref(), style, "a:txBody"));
                // tcPr: fill & borders when they differ from the source cell
                let mut tcpr = cell.props.clone().unwrap_or_else(|| Element::new("a:tcPr"));
                let ofill = tcpr
                    .child("a:solidFill")
                    .and_then(|f| f.elements().next())
                    .map(om::Color::parse)
                    .and_then(|c| c.hex());
                if ofill != c.fill {
                    tcpr.children.retain(|n| !matches!(n, Node::Element(e) if om::Fill::is_fill(&e.name)));
                    if let Some(h) = &c.fill {
                        tcpr.push(om::Fill::solid(h).to_element());
                    }
                }
                if oc.is_none() {
                    for (i, n) in ["a:lnL", "a:lnR", "a:lnT", "a:lnB"].iter().enumerate() {
                        if let Some(s) = &c.borders[i] {
                            tcpr.children.insert(i.min(tcpr.children.len()), Node::Element(border_el(n, s)));
                        }
                    }
                }
                cell.props = Some(tcpr);
                row.cells.push(cell);
            }
            row
        })
        .collect();
}

// ---------------------------------------------------------------------------
// Slides
// ---------------------------------------------------------------------------

fn anims_out(s: &Slide) -> Vec<EntranceAnim> {
    s.anims
        .iter()
        .map(|a| {
            let effect = match a.effect.as_str() {
                "fade" => Entrance::Fade,
                "fly_in" => Entrance::FlyIn,
                "wipe" => Entrance::Wipe,
                "zoom" => Entrance::Zoom,
                _ => Entrance::Appear,
            };
            let is_text = s
                .find(a.obj)
                .and_then(|o| o.kind.sp())
                .map(|sp| sp.text.is_some())
                .unwrap_or(false);
            EntranceAnim {
                shape_id: a.obj,
                effect,
                trigger: match a.trigger.as_str() {
                    "with" => Trigger::WithPrevious,
                    "after" => Trigger::AfterPrevious,
                    _ => Trigger::OnClick,
                },
                duration_ms: a.duration_ms,
                delay_ms: a.delay_ms,
                is_text_shape: is_text,
            }
        })
        .collect()
}

fn set_notes_text(doc: &mut om::SlideDoc, text: &str) -> bool {
    for s in doc.shapes_mut() {
        if let Shape::Sp(sp) = s
            && sp.nv.ph.as_ref().map(|p| p.kind()) == Some("body")
        {
            let tb = sp.text.get_or_insert_with(|| om::TextBody::new("p:txBody"));
            let first = tb.paragraphs.first().cloned().unwrap_or_default();
            let rpr = first.runs.iter().find_map(|r| r.props().cloned());
            tb.paragraphs = text
                .split('\n')
                .map(|l| om::Paragraph {
                    props: first.props.clone(),
                    runs: if l.is_empty() {
                        vec![]
                    } else {
                        vec![om::Run::Text {
                            props: rpr.clone().or_else(|| Some(om::RunProps::new())),
                            text: l.into(),
                        }]
                    },
                    end_props: first.end_props.clone(),
                })
                .collect();
            return true;
        }
    }
    false
}

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const GRP: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;

fn empty_slide() -> om::SlideDoc {
    om::SlideDoc::parse(
        format!(r#"{DECL}<p:sld {NS}><p:cSld><p:spTree>{GRP}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#).as_bytes(),
    )
    .expect("static slide")
}

fn new_notes(pkg: &mut Package, slide_part: &str, slide_rels: &mut Relationships, text: &str) -> Result<Option<NotesEntry>, ExportError> {
    let pres_rels = pkg.rels("ppt/presentation.xml")?;
    let Some(nm) = pres_rels
        .by_type(REL_NOTES_MASTER)
        .next()
        .map(|r| resolve_target("ppt/presentation.xml", &r.target))
    else {
        return Ok(None);
    };
    let part = pkg.free_name("ppt/notesSlides/notesSlide", ".xml");
    let xml = format!(
        r#"{DECL}<p:notes {NS}><p:cSld><p:spTree>{GRP}<p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="zh-TW" altLang="en-US"/></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#
    );
    let mut doc = om::SlideDoc::parse(xml.as_bytes())?;
    set_notes_text(&mut doc, text);
    pkg.set(&part, doc.to_bytes());
    let mut rels = Relationships::default();
    rels.add(REL_NOTES_MASTER, &relative_target(&part, &nm));
    rels.add(REL_SLIDE, &relative_target(&part, slide_part));
    pkg.set_rels(&part, &rels);
    let mut ct = ContentTypes::load(pkg)?;
    ct.set_override(&part, NOTES_CT);
    ct.store(pkg);
    slide_rels.add(REL_NOTES_SLIDE, &relative_target(slide_part, &part));
    Ok(Some(NotesEntry { part, doc }))
}

fn remove_part(pkg: &mut Package, ct: &mut ContentTypes, part: &str) {
    pkg.remove(part);
    pkg.remove(&rels_path(part));
    ct.remove_override(part);
}

/// Export the canvas deck onto `src` (the package it was imported from, or a
/// style template). Returns a new lore-ooxml deck ready to save.
pub fn export(cd: &Deck, src: &ODeck, style: Option<&StyleSystem>, assets: &dyn Assets) -> Result<ODeck, ExportError> {
    Ok(export_parts(cd, src, style, assets)?.0)
}

/// Export; also returns the slide/notes parts that were not regenerated (their
/// source bytes can be kept verbatim).
pub fn export_parts(cd: &Deck, src: &ODeck, style: Option<&StyleSystem>, assets: &dyn Assets) -> Result<(ODeck, Vec<String>), ExportError> {
    let mut od = src.clone();
    let mut keep: Vec<String> = Vec::new();
    let pres = od.pres_part.clone();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let by_part: BTreeMap<String, usize> = src
        .slides
        .iter()
        .enumerate()
        .map(|(i, s)| (s.part.clone(), i))
        .collect();
    let mut entries: Vec<SlideEntry> = Vec::new();
    let mut max_sld_id = src.slides.iter().map(|s| s.sld_id).max().unwrap_or(255).max(255);
    let mut pres_rels = od.pkg.rels(&pres)?;
    for cs in &cd.slides {
        let src_idx = cs.src.as_ref().and_then(|s| by_part.get(&s.part).copied());
        let reuse = src_idx.filter(|i| !claimed.contains(&src.slides[*i].part));
        let (mut entry, fresh) = match (reuse, src_idx) {
            (Some(i), _) => {
                claimed.insert(src.slides[i].part.clone());
                (src.slides[i].clone(), false)
            }
            (None, Some(i)) => {
                // duplicate of a source slide: new part, same relationships (minus notes)
                let mut e = src.slides[i].clone();
                e.part = od.pkg.free_name("ppt/slides/slide", ".xml");
                e.rels.items.retain(|r| r.rel_type != REL_NOTES_SLIDE);
                for r in &mut e.rels.items {
                    if !r.external {
                        let abs = resolve_target(&src.slides[i].part, &r.target);
                        r.target = relative_target(&e.part, &abs);
                    }
                }
                e.notes = None;
                (e, true)
            }
            (None, None) => {
                let part = od.pkg.free_name("ppt/slides/slide", ".xml");
                (
                    SlideEntry {
                        part,
                        sld_id: 0,
                        doc: empty_slide(),
                        rels: Relationships::default(),
                        layout: None,
                        notes: None,
                    },
                    true,
                )
            }
        };
        if fresh {
            // reserve the part name now
            od.pkg.set(&entry.part, Vec::new());
            max_sld_id += 1;
            entry.sld_id = max_sld_id;
            let mut ct = ContentTypes::load(&od.pkg)?;
            ct.set_override(&entry.part, CT_SLIDE);
            ct.store(&mut od.pkg);
        }
        // layout
        let want_layout = cs
            .layout
            .as_deref()
            .and_then(|n| od.layout_by_name(n))
            .or_else(|| entry.layout.as_deref().and_then(|p| od.layout(p)))
            .or_else(|| od.layouts.iter().find(|l| l.kind() != "title"))
            .or(od.layouts.first())
            .map(|l| l.part.clone());
        let layout_changed = want_layout.is_some() && want_layout != entry.layout;
        if layout_changed && let Some(lp) = &want_layout {
            entry.rels.items.retain(|r| r.rel_type != REL_SLIDE_LAYOUT);
            entry.rels.add(REL_SLIDE_LAYOUT, &relative_target(&entry.part, lp));
            entry.layout = Some(lp.clone());
        }
        let content_same = !fresh && cs.src.as_ref().map(|s| s.k == slide_k(cs)).unwrap_or(false) && !layout_changed;
        if !content_same {
            let mut o = Out {
                pkg: &mut od.pkg,
                part: entry.part.clone(),
                rels: entry.rels.clone(),
                style,
                assets,
                next_id: cs.max_id() + 1000,
                slide: cs.uid.clone(),
            };
            let shapes: Vec<Shape> = cs.objects.iter().filter(|x| !x.is_empty_slot()).flat_map(|x| object_out(&mut o, x)).collect();
            entry.rels = o.rels;
            entry.doc.tree.children = shapes;
            entry.doc.set_hidden(cs.hidden);
        }
        let anims_same = !fresh && cs.src.as_ref().map(|s| s.anims_k == fp(&cs.anims)).unwrap_or(false);
        if !anims_same && (fresh || entry.doc.timing.is_some() || !cs.anims.is_empty()) {
            entry.doc.set_entrance_animations(&anims_out(cs));
        }
        let notes_same = !fresh && cs.src.as_ref().map(|s| s.notes_k == fp(&cs.notes)).unwrap_or(false);
        if !notes_same {
            match &mut entry.notes {
                Some(n) => {
                    set_notes_text(&mut n.doc, &cs.notes);
                }
                None if !cs.notes.trim().is_empty() => {
                    let mut rels = entry.rels.clone();
                    entry.notes = new_notes(&mut od.pkg, &entry.part, &mut rels, &cs.notes)?;
                    entry.rels = rels;
                }
                None => {}
            }
        }
        if fresh || !content_same || layout_changed || !notes_same {
            od.pkg.set_rels(&entry.part, &entry.rels);
        }
        if content_same && anims_same {
            keep.push(entry.part.clone());
        }
        if notes_same && let Some(n) = &entry.notes {
            keep.push(n.part.clone());
        }
        if fresh {
            let rid = pres_rels.add(REL_SLIDE, &relative_target(&pres, &entry.part));
            let _ = rid;
        }
        entries.push(entry);
    }
    // drop unclaimed source slides
    let mut ct = ContentTypes::load(&od.pkg)?;
    for s in &src.slides {
        if !claimed.contains(&s.part) {
            remove_part(&mut od.pkg, &mut ct, &s.part);
            if let Some(n) = &s.notes {
                remove_part(&mut od.pkg, &mut ct, &n.part);
            }
            pres_rels.items.retain(|r| r.rel_type != REL_SLIDE || resolve_target(&pres, &r.target) != s.part);
        }
    }
    ct.store(&mut od.pkg);
    // slide id list in canvas order
    let rid_of = |part: &str, rels: &Relationships| -> Option<String> {
        rels.items
            .iter()
            .find(|r| r.rel_type == REL_SLIDE && resolve_target(&pres, &r.target) == part)
            .map(|r| r.id.clone())
    };
    let new_list: Vec<(u32, String)> = entries
        .iter()
        .map(|e| (e.sld_id, rid_of(&e.part, &pres_rels).unwrap_or_default()))
        .collect();
    let old_list: Vec<(u32, String)> = src
        .slides
        .iter()
        .map(|e| (e.sld_id, rid_of(&e.part, &src.pkg.rels(&pres).unwrap_or_default()).unwrap_or_default()))
        .collect();
    if new_list != old_list {
        od.pkg.set_rels(&pres, &pres_rels);
        let mut pd = od.pkg.xml(&pres)?;
        if pd.root.child("p:sldIdLst").is_none() {
            let pos = pd
                .root
                .children
                .iter()
                .position(|c| matches!(c, Node::Element(e) if e.local() == "sldSz"))
                .unwrap_or(pd.root.children.len());
            pd.root.children.insert(pos, Node::Element(Element::new("p:sldIdLst")));
        }
        let lst = pd.root.child_mut("p:sldIdLst").unwrap();
        lst.children = new_list
            .iter()
            .map(|(id, rid)| {
                Node::Element(
                    Element::new("p:sldId")
                        .with_attr("id", id.to_string())
                        .with_attr("r:id", rid.clone()),
                )
            })
            .collect();
        if new_list.is_empty() {
            pd.root.children.retain(|c| !matches!(c, Node::Element(e) if e.local() == "sldIdLst"));
        }
        od.pkg.set(&pres, pd.to_bytes());
    }
    od.slides = entries;
    prune_media(&mut od.pkg);
    Ok((od, keep))
}

/// Remove media parts no relationship points at any more.
fn prune_media(pkg: &mut Package) {
    let mut used: BTreeSet<String> = BTreeSet::new();
    let rels_parts: Vec<String> = pkg.names().filter(|n| n.ends_with(".rels")).map(str::to_string).collect();
    for rp in rels_parts {
        let owner = if rp == "_rels/.rels" {
            String::new()
        } else {
            rp.replace("/_rels/", "/").trim_end_matches(".rels").to_string()
        };
        if let Some(b) = pkg.get(&rp)
            && let Ok(r) = Relationships::parse(b)
        {
            for it in r.items {
                if !it.external {
                    used.insert(resolve_target(&owner, &it.target));
                }
            }
        }
    }
    let dead: Vec<String> = pkg
        .names()
        .filter(|n| n.starts_with("ppt/media/") && !used.contains(*n))
        .map(str::to_string)
        .collect();
    if dead.is_empty() {
        return;
    }
    let mut ct = ContentTypes::load(pkg).ok();
    for d in dead {
        pkg.remove(&d);
        if let Some(c) = ct.as_mut() {
            c.remove_override(&d);
        }
    }
    if let Some(c) = ct {
        c.store(pkg);
    }
}

/// Export straight to bytes.
/// Unchanged slide and notes parts keep their source bytes exactly.
pub fn export_bytes(cd: &Deck, src: &ODeck, style: Option<&StyleSystem>, assets: &dyn Assets) -> Result<Vec<u8>, ExportError> {
    let (od, keep) = export_parts(cd, src, style, assets)?;
    let mut pkg = od.to_package();
    for p in keep {
        if let Some(b) = src.pkg.get(&p) {
            pkg.set(&p, b.to_vec());
        }
    }
    Ok(pkg.to_bytes()?)
}
