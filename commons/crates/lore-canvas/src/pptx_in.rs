//! PPTX (lore-ooxml model) → canvas scene.
//!
//! Every object keeps its original XML and per-field fingerprints so the
//! exporter can write untouched objects verbatim and patch edited ones.

use std::collections::BTreeMap;

use serde::Serialize;
use lore_ooxml::Deck as ODeck;
use lore_ooxml::model::{self as om, Shape};
use lore_ooxml::package::{REL_IMAGE, Relationships, resolve_target};
use lore_ooxml::xml::Element;

use crate::model::*;
use crate::style::StyleSystem;

/// Layout background (master + layout non-placeholder shapes) for display.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Backdrop {
    pub layout: String,
    pub bg: Paint,
    pub objects: Vec<Object>,
}

pub(crate) struct Ctx<'a> {
    pub od: &'a ODeck,
    /// Slide index when importing a slide (placeholder inheritance).
    pub slide: Option<usize>,
    pub part: String,
    pub rels: Relationships,
    pub style: Option<&'a StyleSystem>,
}

impl Ctx<'_> {
    fn color(&self, c: &om::Color) -> Option<String> {
        self.od.resolve_color(c)
    }
    fn asset(&self, rid: &str) -> String {
        match self.rels.get(rid) {
            Some(r) if r.external => format!("url:{}", r.target),
            Some(r) => format!("pkg:{}", resolve_target(&self.part, &r.target)),
            None => format!("missing:{rid}"),
        }
    }
}

pub(crate) fn frame_of(x: Option<&om::Xfrm>) -> Frame {
    let Some(x) = x else {
        return Frame::default();
    };
    let (ox, oy) = x.off.unwrap_or((0, 0));
    let (cx, cy) = x.ext.unwrap_or((0, 0));
    Frame {
        x: ox,
        y: oy,
        w: cx,
        h: cy,
        rot: x.rot_deg(),
        flip_h: x.flip_h(),
        flip_v: x.flip_v(),
    }
}

fn adj_of(adj: &Option<Vec<(String, String)>>) -> BTreeMap<String, f64> {
    let mut m = BTreeMap::new();
    for (n, f) in adj.iter().flatten() {
        if let Some(v) = f.strip_prefix("val ").and_then(|v| v.trim().parse::<f64>().ok()) {
            m.insert(n.clone(), v);
        }
    }
    m
}

pub(crate) fn geom_of(g: Option<&om::Geometry>) -> Geom {
    match g {
        None => Geom::preset("rect"),
        Some(om::Geometry::Preset { prst, adj, .. }) => Geom {
            prst: prst.clone(),
            adj: adj_of(adj),
            path: None,
        },
        Some(om::Geometry::Custom(e)) => Geom {
            prst: "custom".into(),
            adj: BTreeMap::new(),
            path: Some(cust_path(e)),
        },
    }
}

/// custGeom → SVG path in a 0..1 box (moveTo, lnTo, cubicBezTo, quadBezTo, close;
/// arcTo approximated by a line to its end point).
pub fn cust_path(e: &Element) -> String {
    let mut out = String::new();
    let Some(lst) = e.child("a:pathLst") else {
        return out;
    };
    for p in lst.elements() {
        let w = p.attrs.get_i64("w").unwrap_or(0).max(1) as f64;
        let h = p.attrs.get_i64("h").unwrap_or(0).max(1) as f64;
        let pt = |e: &Element| {
            let x = e.attrs.get_i64("x").unwrap_or(0) as f64 / w;
            let y = e.attrs.get_i64("y").unwrap_or(0) as f64 / h;
            format!("{x:.5} {y:.5}")
        };
        for c in p.elements() {
            let pts: Vec<String> = c.elements().filter(|e| e.local() == "pt").map(pt).collect();
            match c.local() {
                "moveTo" => out.push_str(&format!("M{} ", pts.join(" "))),
                "lnTo" => out.push_str(&format!("L{} ", pts.join(" "))),
                "cubicBezTo" => out.push_str(&format!("C{} ", pts.join(" "))),
                "quadBezTo" => out.push_str(&format!("Q{} ", pts.join(" "))),
                "close" => out.push_str("Z "),
                _ => {}
            }
        }
    }
    out.trim().to_string()
}

fn style_ref(sp_style: Option<&Element>, name: &str) -> Option<(u32, Option<om::Color>)> {
    let r = sp_style?.child(&format!("a:{name}"))?;
    let idx = r.attr("idx").and_then(|v| v.parse().ok()).unwrap_or(0);
    Some((idx, r.elements().next().map(om::Color::parse)))
}

fn paint_of(ctx: &Ctx, f: Option<&om::Fill>, style: Option<&Element>) -> Paint {
    match f {
        Some(om::Fill::NoFill) => Paint::None,
        Some(om::Fill::Solid(c)) => match c {
            Some(c) => Paint::Solid {
                color: ctx.color(c).unwrap_or_else(|| "000000".into()),
                alpha: c.alpha(),
            },
            None => Paint::None,
        },
        Some(om::Fill::Gradient(e)) => {
            let mut stops: Vec<Stop> = e
                .child("a:gsLst")
                .map(|l| {
                    l.elements()
                        .filter_map(|gs| {
                            let c = gs.elements().next().map(om::Color::parse)?;
                            Some(Stop {
                                pos: gs.attrs.get_i64("pos").unwrap_or(0) as f64 / 100_000.0,
                                color: ctx.color(&c).unwrap_or_else(|| "FFFFFF".into()),
                                alpha: c.alpha(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            stops.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap_or(std::cmp::Ordering::Equal));
            Paint::Gradient {
                stops,
                angle: e
                    .child("a:lin")
                    .and_then(|l| l.attrs.get_i64("ang"))
                    .unwrap_or(0) as f64
                    / 60_000.0,
                path: e.child("a:path").is_some(),
            }
        }
        Some(om::Fill::Blip(_)) => Paint::Other {
            label: "picture".into(),
        },
        Some(om::Fill::Pattern(_)) => Paint::Other {
            label: "pattern".into(),
        },
        Some(om::Fill::Group) => Paint::Other {
            label: "group".into(),
        },
        None => match style_ref(style, "fillRef") {
            Some((idx, Some(c))) if idx > 0 => Paint::Solid {
                color: ctx.color(&c).unwrap_or_else(|| "4472C4".into()),
                alpha: c.alpha(),
            },
            _ => Paint::None,
        },
    }
}

fn stroke_of(ctx: &Ctx, l: Option<&om::Line>, style: Option<&Element>) -> Option<Stroke> {
    let sref = style_ref(style, "lnRef").filter(|(i, _)| *i > 0);
    let style_col = sref
        .as_ref()
        .and_then(|(_, c)| c.as_ref())
        .and_then(|c| ctx.color(c).map(|h| (h, c.alpha())));
    let style_w = sref.as_ref().map(|(i, _)| match i {
        1 => 0.5,
        2 => 1.0,
        _ => 1.5,
    });
    let (color, alpha) = match l.and_then(|l| l.fill.as_ref()) {
        Some(om::Fill::NoFill) => return None,
        Some(om::Fill::Solid(Some(c))) => (ctx.color(c).unwrap_or_else(|| "000000".into()), c.alpha()),
        Some(om::Fill::Gradient(_)) => ("808080".into(), 1.0),
        _ => style_col?,
    };
    let dash = match l.and_then(|l| l.dash.as_ref()) {
        Some(om::Dash::Preset(p)) => p.clone(),
        Some(om::Dash::Custom(_)) => "dash".into(),
        None => "solid".into(),
    };
    Some(Stroke {
        color,
        alpha,
        width_pt: l.and_then(|l| l.width_pt()).or(style_w).unwrap_or(0.75),
        dash,
        head: l
            .and_then(|l| l.head.as_ref())
            .map(|h| h.kind().to_string())
            .unwrap_or_else(|| "none".into()),
        tail: l
            .and_then(|l| l.tail.as_ref())
            .map(|h| h.kind().to_string())
            .unwrap_or_else(|| "none".into()),
    })
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Inheritance chain for one text body: list styles from most to least
/// specific, plus body properties (bodyPr attrs) likewise.
#[derive(Default)]
pub(crate) struct Chain<'a> {
    pub lists: Vec<&'a om::ListStyle>,
    pub bodies: Vec<&'a om::BodyProps>,
    pub font_ref: Option<om::Color>,
    pub role: String,
}

impl<'a> Chain<'a> {
    fn level_props(&self, lvl: u32) -> Vec<&'a om::ParaProps> {
        let mut v = Vec::new();
        for l in &self.lists {
            if let Some(p) = l.level(lvl) {
                v.push(p);
            }
        }
        for l in &self.lists {
            if let Some(p) = l.default_level() {
                v.push(p);
            }
        }
        v
    }
}

fn chain_for_sp<'a>(ctx: &Ctx<'a>, sp: &'a om::Sp) -> Chain<'a> {
    let od = ctx.od;
    let mut c = Chain::default();
    if let Some(ls) = sp.text.as_ref().and_then(|t| t.list_style.as_ref()) {
        c.lists.push(ls);
    }
    if let Some(t) = &sp.text {
        c.bodies.push(&t.body);
    }
    let layout = ctx.slide.and_then(|i| od.slide_layout(i));
    let master = layout
        .and_then(|l| l.master.as_deref())
        .and_then(|m| od.master(m))
        .or(od.masters.first());
    match (&sp.nv.ph, ctx.slide) {
        (Some(ph), Some(si)) => {
            c.role = ph.kind().to_string();
            let (l, m) = od.inherited(si, ph);
            for s in [l, m].into_iter().flatten() {
                if let Shape::Sp(s) = s
                    && let Some(t) = &s.text
                {
                    if let Some(ls) = &t.list_style {
                        c.lists.push(ls);
                    }
                    c.bodies.push(&t.body);
                }
            }
            if let Some(m) = master {
                let st = match lore_ooxml::deck::master_type(ph.kind()) {
                    "title" => m.title_style.as_ref(),
                    "body" => m.body_style.as_ref(),
                    _ => m.other_style.as_ref(),
                };
                if let Some(st) = st {
                    c.lists.push(st);
                }
            }
        }
        _ => {
            if let Some(d) = &od.default_text_style {
                c.lists.push(d);
            }
        }
    }
    c.font_ref = sp
        .style
        .as_ref()
        .and_then(|s| s.child("a:fontRef"))
        .and_then(|f| f.elements().next())
        .map(om::Color::parse);
    c
}

fn theme_for<'a>(ctx: &Ctx<'a>) -> Option<&'a lore_ooxml::deck::Theme> {
    ctx.slide
        .and_then(|i| ctx.od.slide_layout(i))
        .and_then(|l| ctx.od.theme_for_layout(l))
        .or(ctx.od.theme())
}

pub(crate) fn look_of(ctx: &Ctx, rpr: Option<&om::RunProps>, lvl: u32, ch: &Chain) -> Look {
    let mut props: Vec<&om::RunProps> = Vec::new();
    if let Some(r) = rpr {
        props.push(r);
    }
    for p in ch.level_props(lvl) {
        if let Some(d) = &p.def_rpr {
            props.push(d);
        }
    }
    let first = |f: &dyn Fn(&om::RunProps) -> Option<String>| props.iter().find_map(|r| f(r));
    let latin = first(&|r| r.latin_font().map(str::to_string)).unwrap_or_else(|| "+mn-lt".into());
    let ea = first(&|r| r.ea_font().map(str::to_string)).unwrap_or_else(|| "+mn-ea".into());
    let theme = theme_for(ctx);
    let (latin, ea) = match theme {
        Some(t) => (t.resolve_font(&latin), t.resolve_font(&ea)),
        None => (latin, ea),
    };
    let size = props.iter().find_map(|r| r.size_pt()).unwrap_or(18.0);
    let b = props.iter().find_map(|r| r.bold()).unwrap_or(false);
    let i = props.iter().find_map(|r| r.italic()).unwrap_or(false);
    let u = first(&|r| r.attrs.get("u").map(str::to_string))
        .map(|u| u != "none")
        .unwrap_or(false);
    let strike = first(&|r| r.attrs.get("strike").map(str::to_string))
        .map(|s| s != "noStrike")
        .unwrap_or(false);
    let baseline = props
        .iter()
        .find_map(|r| r.attrs.get_i64("baseline"))
        .unwrap_or(0) as f64
        / 1000.0;
    let col = props
        .iter()
        .find_map(|r| r.color().cloned())
        .or_else(|| ch.font_ref.clone());
    let (color, alpha) = match col {
        Some(c) => (ctx.color(&c).unwrap_or_else(|| "000000".into()), c.alpha()),
        None => (
            ctx.color(&om::Color::scheme("tx1"))
                .unwrap_or_else(|| "000000".into()),
            1.0,
        ),
    };
    Look {
        font: latin,
        ea_font: ea,
        size,
        bold: b,
        italic: i,
        underline: u,
        strike,
        color,
        alpha,
        baseline,
    }
}

fn para_props_of(p: &om::Paragraph, ch: &Chain, size: f64) -> Para {
    let lvl = p.level();
    let mut list: Vec<&om::ParaProps> = Vec::new();
    if let Some(pp) = &p.props {
        list.push(pp);
    }
    list.extend(ch.level_props(lvl));
    let attr = |k: &str| list.iter().find_map(|pp| pp.attrs.get(k).map(str::to_string));
    let mut out = Para::new(vec![]);
    out.level = lvl;
    out.align = attr("algn").unwrap_or_else(|| "l".into());
    out.mar_l = attr("marL").and_then(|v| v.parse().ok()).unwrap_or(0);
    out.indent = attr("indent").and_then(|v| v.parse().ok()).unwrap_or(0);
    if let Some(s) = list.iter().find_map(|pp| pp.line_spacing.as_ref()) {
        match s {
            om::Spacing::Pct(v) => out.line = Some(*v as f64 / 100_000.0),
            om::Spacing::Pts(v) => out.line_pt = Some(*v as f64 / 100.0),
            _ => {}
        }
    }
    let pts = |s: &om::Spacing| match s {
        om::Spacing::Pts(v) => Some(*v as f64 / 100.0),
        om::Spacing::Pct(v) => Some(*v as f64 / 100_000.0 * size * 1.2),
        _ => None,
    };
    out.before_pt = list
        .iter()
        .find_map(|pp| pp.space_before.as_ref())
        .and_then(pts)
        .filter(|v| *v != 0.0);
    out.after_pt = list
        .iter()
        .find_map(|pp| pp.space_after.as_ref())
        .and_then(pts)
        .filter(|v| *v != 0.0);
    out.bullet = match list.iter().find_map(|pp| pp.bullet.as_ref()) {
        Some(om::Bullet::Char(a)) => Bullet::Char {
            ch: a.get("char").unwrap_or("•").to_string(),
        },
        Some(om::Bullet::AutoNum(a)) => Bullet::Num {
            scheme: a.get("type").unwrap_or("arabicPeriod").to_string(),
            start: a.get_i64("startAt").unwrap_or(1) as u32,
        },
        Some(om::Bullet::Blip(_)) => Bullet::Char { ch: "•".into() },
        _ => Bullet::None,
    };
    out
}

pub(crate) fn text_of(ctx: &Ctx, tb: &om::TextBody, ch: &Chain) -> TextBody {
    let mut out = TextBody::default();
    let battr = |k: &str| ch.bodies.iter().find_map(|b| b.attrs.get(k).map(str::to_string));
    let ins = |k: &str, d: i64| battr(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    out.insets = [
        ins("lIns", 91_440),
        ins("tIns", 45_720),
        ins("rIns", 91_440),
        ins("bIns", 45_720),
    ];
    out.anchor = battr("anchor").unwrap_or_else(|| "t".into());
    out.wrap = battr("wrap").map(|w| w != "none").unwrap_or(true);
    out.vert = battr("vert").filter(|v| v != "horz");
    match ch.bodies.iter().find_map(|b| b.autofit.as_ref()) {
        Some(om::Autofit::Normal(a)) => {
            out.autofit = "norm".into();
            out.font_scale = a.get_i64("fontScale").unwrap_or(100_000) as f64 / 100_000.0;
            out.ln_reduction = a.get_i64("lnSpcReduction").unwrap_or(0) as f64 / 100_000.0;
        }
        Some(om::Autofit::Shape) => out.autofit = "shape".into(),
        _ => out.autofit = "none".into(),
    }
    let role = ch.role.as_str();
    for (pi, p) in tb.paragraphs.iter().enumerate() {
        let lvl = p.level();
        let end = look_of(ctx, p.end_props.as_ref(), lvl, ch);
        let mut para = para_props_of(p, ch, end.size);
        for (ri, r) in p.runs.iter().enumerate() {
            let mut run = Run {
                kind: "text".into(),
                text: String::new(),
                tag: None,
                em: None,
                look: Look::default(),
                field: None,
                omml: None,
                latex: None,
                src: Some([pi as u32, ri as u32]),
                k: 0,
            };
            match r {
                om::Run::Text { props, text } => {
                    run.look = look_of(ctx, props.as_ref(), lvl, ch);
                    run.text = text.clone();
                }
                om::Run::Break { props } => {
                    run.kind = "br".into();
                    run.look = look_of(ctx, props.as_ref(), lvl, ch);
                }
                om::Run::Field { attrs, props, text, .. } => {
                    run.kind = "field".into();
                    run.look = look_of(ctx, props.as_ref(), lvl, ch);
                    run.field = attrs.get("type").map(str::to_string);
                    run.text = text.clone().unwrap_or_default();
                }
                om::Run::Math(e) => {
                    run.kind = "math".into();
                    let first_rpr = e.find_all("a:rPr").first().map(|x| om::RunProps::parse(x));
                    run.look = look_of(ctx, first_rpr.as_ref().or(p.end_props.as_ref()), lvl, ch);
                    run.omml = Some(e.to_xml_string());
                    run.text = r.plain_text();
                }
                om::Run::Raw(e) => {
                    run.look = end.clone();
                    if let Some(m) = e.find_all("a14:m").first() {
                        run.kind = "math".into();
                        run.omml = Some((*m).to_xml_string());
                    } else {
                        run.kind = "raw".into();
                    }
                    run.text = r.plain_text();
                }
            }
            if let Some(st) = ctx.style
                && matches!(run.kind.as_str(), "text" | "field" | "br" | "math")
            {
                let r = if run.kind == "math" { "math" } else { role };
                if let Some((t, e)) = st.match_look(&run.look, r) {
                    run.tag = Some(t);
                    run.em = e;
                }
            }
            run.k = run_k(&run);
            para.runs.push(run);
        }
        para.end = Some(end);
        para.src = Some(pi as u32);
        para.k = para_k(&para);
        out.paras.push(para);
    }
    out
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

/// Fill and borders of a `a:tcPr`.
pub(crate) fn cell_style(ctx: &Ctx, tcpr: Option<&Element>) -> (Option<String>, [Option<Stroke>; 4]) {
    let Some(t) = tcpr else {
        return (None, [None, None, None, None]);
    };
    let fill = t
        .child("a:solidFill")
        .and_then(|f| f.elements().next())
        .map(om::Color::parse)
        .and_then(|c| ctx.color(&c));
    let b = |n: &str| -> Option<Stroke> {
        let l = om::Line::parse(t.child(n)?);
        match &l.fill {
            Some(om::Fill::NoFill) => Some(Stroke {
                width_pt: 0.0,
                ..Stroke::new("FFFFFF", 0.0)
            }),
            _ => stroke_of(ctx, Some(&l), None),
        }
    };
    (fill, [b("a:lnL"), b("a:lnR"), b("a:lnT"), b("a:lnB")])
}

fn table_of(ctx: &Ctx, t: &om::Table) -> TableObj {
    let mut ch = Chain {
        role: "table".into(),
        ..Default::default()
    };
    if let Some(d) = &ctx.od.default_text_style {
        ch.lists.push(d);
    }
    let rows = t
        .rows
        .iter()
        .enumerate()
        .map(|(ri, r)| Row {
            h: r.attrs.get_i64("h").unwrap_or(0),
            src: Some(ri as u32),
            cells: r
                .cells
                .iter()
                .enumerate()
                .map(|(ci, c)| {
                    let mut body = om::BodyProps::default();
                    if let Some(p) = &c.props {
                        for (k, d) in [("marL", "lIns"), ("marR", "rIns"), ("marT", "tIns"), ("marB", "bIns"), ("anchor", "anchor")] {
                            if let Some(v) = p.attr(k) {
                                body.attrs.set(d, v);
                            }
                        }
                    }
                    let mut chain = Chain {
                        role: "table".into(),
                        lists: ch.lists.clone(),
                        ..Default::default()
                    };
                    chain.bodies.push(&body);
                    let text = c
                        .text
                        .as_ref()
                        .map(|tb| text_of(ctx, tb, &chain))
                        .unwrap_or_default();
                    let (fill, borders) = cell_style(ctx, c.props.as_ref());
                    Cell {
                        text,
                        span: c.attrs.get_i64("gridSpan").unwrap_or(1) as u32,
                        rspan: c.attrs.get_i64("rowSpan").unwrap_or(1) as u32,
                        hmerge: c.attrs.get_bool("hMerge").unwrap_or(false),
                        vmerge: c.attrs.get_bool("vMerge").unwrap_or(false),
                        fill,
                        borders,
                        src: Some([ri as u32, ci as u32]),
                    }
                })
                .collect(),
        })
        .collect();
    let tblpr = t.props.as_ref();
    TableObj {
        cols: t.col_widths(),
        rows,
        style_id: t.style_id(),
        first_row: tblpr.and_then(|p| p.attrs.get_bool("firstRow")).unwrap_or(false),
        band_row: tblpr.and_then(|p| p.attrs.get_bool("bandRow")).unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

fn finish(mut o: Object, s: &Shape) -> Object {
    o.src = Some(ObjSrc {
        xml: s.to_element().to_xml_string(),
        k: BTreeMap::new(),
    });
    let k = object_ks(&o);
    if let Some(src) = &mut o.src {
        src.k = k;
    }
    o
}

fn sp_obj(ctx: &Ctx, sp: &om::Sp, equation: bool, frame: Frame) -> Object {
    let ch = chain_for_sp(ctx, sp);
    let text = sp.text.as_ref().map(|t| text_of(ctx, t, &ch));
    let spo = SpObj {
        geom: geom_of(sp.props.geom.as_ref()),
        fill: paint_of(ctx, sp.props.fill.as_ref(), sp.style.as_ref()),
        line: stroke_of(ctx, sp.props.line.as_ref(), sp.style.as_ref()),
        text,
        equation,
    };
    let is_text = equation
        || sp.nv.ph.is_some()
        || sp.nv.kind.attrs.get_bool("txBox") == Some(true);
    Object {
        id: sp.nv.id(),
        name: sp.nv.name().to_string(),
        frame,
        hidden: sp.nv.hidden(),
        ph: sp.nv.ph.as_ref().map(|p| p.kind().to_string()),
        kind: if is_text { Kind::Text(spo) } else { Kind::Shape(spo) },
        src: None,
    }
}

/// Convert one lore-ooxml shape. `top` = top-level on a slide (placeholder frames
/// are inherited from the layout).
pub(crate) fn object_of(ctx: &Ctx, s: &Shape) -> Object {
    let frame = match (ctx.slide, s.nv().and_then(|n| n.ph.as_ref())) {
        (Some(i), Some(_)) => frame_of(ctx.od.effective_xfrm(i, s).as_ref()),
        _ => frame_of(s.xfrm()),
    };
    let o = match s {
        Shape::Sp(sp) => sp_obj(ctx, sp, false, frame),
        Shape::Pic(p) => {
            let blip = p.fill.blip.clone().unwrap_or_default();
            let crop = p.fill.crop().unwrap_or_default();
            let mut effects = Vec::new();
            for e in &blip.effects {
                let n = e.local();
                if n == "extLst" {
                    if e.contains("a14:backgroundRemoval") {
                        effects.push("backgroundRemoval".into());
                    }
                    if e.contains("a14:imgEffect") {
                        effects.push("imgEffect".into());
                    }
                } else if n != "alphaModFix" {
                    effects.push(n.to_string());
                }
            }
            Object {
                id: p.nv.id(),
                name: p.nv.name().to_string(),
                frame,
                hidden: p.nv.hidden(),
                ph: p.nv.ph.as_ref().map(|x| x.kind().to_string()),
                kind: Kind::Picture(PicObj {
                    asset: blip
                        .embed()
                        .map(|r| ctx.asset(r))
                        .or_else(|| blip.attrs.get("r:link").map(|r| ctx.asset(r)))
                        .unwrap_or_default(),
                    crop: Crop {
                        l: crop.l as f64 / 100_000.0,
                        t: crop.t as f64 / 100_000.0,
                        r: crop.r as f64 / 100_000.0,
                        b: crop.b as f64 / 100_000.0,
                    },
                    alpha: blip.alpha(),
                    line: stroke_of(ctx, p.props.line.as_ref(), p.style.as_ref()),
                    clip: geom_of(p.props.geom.as_ref()).prst,
                    masks: vec![],
                    effects,
                }),
                src: None,
            }
        }
        Shape::Cxn(c) => {
            let g = geom_of(c.props.geom.as_ref());
            let at = |v: &Option<(u32, u32)>| v.map(|(id, site)| Attach { id, site });
            Object {
                id: c.nv.id(),
                name: c.nv.name().to_string(),
                frame,
                hidden: c.nv.hidden(),
                ph: None,
                kind: Kind::Connector(ConnObj {
                    geom: g.prst,
                    adj: g.adj,
                    line: stroke_of(ctx, c.props.line.as_ref(), c.style.as_ref()),
                    start: at(&c.nv.kind.st_cxn),
                    end: at(&c.nv.kind.end_cxn),
                    text: None,
                }),
                src: None,
            }
        }
        Shape::Group(g) => {
            let x = g.props.xfrm.as_ref();
            let ch = Frame {
                x: x.and_then(|x| x.ch_off).map(|v| v.0).unwrap_or(frame.x),
                y: x.and_then(|x| x.ch_off).map(|v| v.1).unwrap_or(frame.y),
                w: x.and_then(|x| x.ch_ext).map(|v| v.0).unwrap_or(frame.w),
                h: x.and_then(|x| x.ch_ext).map(|v| v.1).unwrap_or(frame.h),
                ..Default::default()
            };
            let inner = Ctx {
                od: ctx.od,
                slide: None,
                part: ctx.part.clone(),
                rels: ctx.rels.clone(),
                style: ctx.style,
            };
            Object {
                id: g.nv.id(),
                name: g.nv.name().to_string(),
                frame,
                hidden: g.nv.hidden(),
                ph: None,
                kind: Kind::Group(GroupObj {
                    ch,
                    children: g.children.iter().map(|c| object_of(&inner, c)).collect(),
                }),
                src: None,
            }
        }
        Shape::Frame(f) => match &f.content {
            om::FrameContent::Table(t) => Object {
                id: f.nv.id(),
                name: f.nv.name().to_string(),
                frame,
                hidden: f.nv.hidden(),
                ph: None,
                kind: Kind::Table(table_of(ctx, t)),
                src: None,
            },
            om::FrameContent::Other(_) => Object {
                id: f.nv.id(),
                name: f.nv.name().to_string(),
                frame,
                hidden: f.nv.hidden(),
                ph: None,
                kind: Kind::Locked(LockedObj {
                    kind: format!(
                        "graphicFrame:{}",
                        f.uri().rsplit('/').next().unwrap_or("other")
                    ),
                }),
                src: None,
            },
        },
        Shape::Alt(a) => {
            let inner = a.choices.first().map(|c| c.shapes.as_slice()).unwrap_or(&[]);
            match inner {
                [Shape::Sp(sp)] => sp_obj(ctx, sp, a.has_math(), frame_of(sp.props.xfrm.as_ref())),
                [one] => {
                    let mut o = object_of(ctx, one);
                    o.src = None;
                    o
                }
                _ => Object {
                    id: s.id().unwrap_or(0),
                    name: s.nv().map(|n| n.name().to_string()).unwrap_or_default(),
                    frame,
                    hidden: false,
                    ph: None,
                    kind: Kind::Locked(LockedObj {
                        kind: "mc:AlternateContent".into(),
                    }),
                    src: None,
                },
            }
        }
        Shape::Locked(l) => {
            let e = &l.element;
            let x = e
                .find_all("a:xfrm")
                .first()
                .map(|x| om::Xfrm::parse(x))
                .or_else(|| e.find_all("p:xfrm").first().map(|x| om::Xfrm::parse(x)));
            let nv = e.find_all("p:cNvPr");
            let nv = nv.first();
            Object {
                id: nv.and_then(|n| n.attr("id")).and_then(|v| v.parse().ok()).unwrap_or(0),
                name: nv.and_then(|n| n.attr("name")).unwrap_or("").to_string(),
                frame: frame_of(x.as_ref()),
                hidden: false,
                ph: None,
                kind: Kind::Locked(LockedObj { kind: l.kind.clone() }),
                src: None,
            }
        }
    };
    finish(o, s)
}

/// Paint of a `p:bg`.
fn bg_of(ctx: &Ctx, bg: Option<&Element>) -> Option<Paint> {
    let bg = bg?;
    if let Some(pr) = bg.child("p:bgPr") {
        let f = pr.elements().find(|e| om::Fill::is_fill(&e.name)).map(om::Fill::parse);
        return Some(paint_of(ctx, f.as_ref(), None));
    }
    if let Some(r) = bg.child("p:bgRef")
        && let Some(c) = r.elements().next().map(om::Color::parse)
    {
        return Some(Paint::Solid {
            color: ctx.color(&c).unwrap_or_else(|| "FFFFFF".into()),
            alpha: 1.0,
        });
    }
    None
}

/// Layout backdrops (master + layout shapes that are not placeholders).
pub fn backdrops(od: &ODeck) -> Vec<Backdrop> {
    let mut out = Vec::new();
    for l in &od.layouts {
        let master = l.master.as_deref().and_then(|m| od.master(m));
        let mut objs = Vec::new();
        let mut bg = None;
        let show_master = l.doc.attrs.get("showMasterSp").map(|v| v != "0").unwrap_or(true);
        if let Some(m) = master {
            let mctx = Ctx {
                od,
                slide: None,
                part: m.part.clone(),
                rels: od.pkg.rels(&m.part).unwrap_or_default(),
                style: None,
            };
            bg = bg_of(&mctx, m.doc.background.as_ref());
            if show_master {
                for s in m.doc.shapes() {
                    if s.nv().and_then(|n| n.ph.as_ref()).is_none() {
                        objs.push(object_of(&mctx, s));
                    }
                }
            }
        }
        let lctx = Ctx {
            od,
            slide: None,
            part: l.part.clone(),
            rels: l.rels.clone(),
            style: None,
        };
        if let Some(b) = bg_of(&lctx, l.doc.background.as_ref()) {
            bg = Some(b);
        }
        for s in l.doc.shapes() {
            if s.nv().and_then(|n| n.ph.as_ref()).is_none() {
                objs.push(object_of(&lctx, s));
            }
        }
        for o in &mut objs {
            o.src = None;
            strip_src(o);
        }
        out.push(Backdrop {
            layout: l.name().to_string(),
            bg: bg.unwrap_or_else(|| Paint::solid("FFFFFF")),
            objects: objs,
        });
    }
    out
}

fn strip_src(o: &mut Object) {
    o.src = None;
    if let Kind::Group(g) = &mut o.kind {
        for c in &mut g.children {
            strip_src(c);
        }
    }
}

/// Plain text of a notes slide's body placeholder.
pub fn notes_text(doc: &om::SlideDoc) -> String {
    for s in doc.shapes() {
        if let Shape::Sp(sp) = s
            && sp.nv.ph.as_ref().map(|p| p.kind()) == Some("body")
            && let Some(t) = &sp.text
        {
            return t.plain_text();
        }
    }
    String::new()
}

pub(crate) fn anim_name(e: lore_ooxml::anim::Entrance) -> &'static str {
    use lore_ooxml::anim::Entrance::*;
    match e {
        Appear => "appear",
        Fade => "fade",
        FlyIn => "fly_in",
        Wipe => "wipe",
        Zoom => "zoom",
    }
}

/// Import a whole deck.
pub fn import(od: &ODeck, style: Option<&StyleSystem>, style_id: Option<String>) -> Deck {
    let mut d = Deck::new(od.slide_size.0, od.slide_size.1, style_id);
    for (i, e) in od.slides.iter().enumerate() {
        let ctx = Ctx {
            od,
            slide: Some(i),
            part: e.part.clone(),
            rels: e.rels.clone(),
            style,
        };
        let objects: Vec<Object> = e.doc.shapes().iter().map(|s| object_of(&ctx, s)).collect();
        let anims: Vec<Anim> = e
            .doc
            .entrance_animations()
            .into_iter()
            .map(|a| Anim {
                obj: a.shape_id,
                effect: anim_name(a.effect).into(),
                trigger: match a.trigger {
                    lore_ooxml::anim::Trigger::OnClick => "click",
                    lore_ooxml::anim::Trigger::WithPrevious => "with",
                    lore_ooxml::anim::Trigger::AfterPrevious => "after",
                }
                .into(),
                duration_ms: a.duration_ms,
                delay_ms: a.delay_ms,
            })
            .collect();
        let uid = d.new_uid();
        let mut s = Slide {
            uid,
            layout: od.slide_layout(i).map(|l| l.name().to_string()),
            hidden: e.doc.hidden(),
            notes: e.notes.as_ref().map(|n| notes_text(&n.doc)).unwrap_or_default(),
            objects,
            anims,
            src: None,
        };
        s.src = Some(SlideSrc {
            part: e.part.clone(),
            k: slide_k(&s),
            notes_k: fp(&s.notes),
            anims_k: fp(&s.anims),
        });
        d.slides.push(s);
    }
    let _ = REL_IMAGE;
    d
}
