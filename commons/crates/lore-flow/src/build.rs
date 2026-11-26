//! L3 — canvas objects. Panels, modules, connectors, line labels and the
//! legend become `lore_canvas::Object`s named `flow:<fig>/<node>[/<part>]`,
//! ready to insert as one undoable batch ([`insert_ops`]).
//!
//! Lines with up to three corners are PowerPoint connectors (straight or
//! bent) attached to the modules' connection sites, so they follow when a
//! module is moved; lines with more corners are open polylines.

use std::collections::BTreeMap;

use lore_canvas::{Attach, ConnObj, Frame, Kind, Object, Op, Paint, Stroke};

use crate::layout::{Layout, Rect};
use crate::modules::{self, Ctx, EMU_PT, Library, emu};
use crate::route::{Route, Side};
use crate::spec::*;
use crate::text::{MathSource, Vars};
use crate::theme::{Corners, Theme};

#[derive(Debug, Clone)]
pub struct Built {
    /// Back to front.
    pub objects: Vec<Object>,
    /// Node id → id of its object (the shape, or the group).
    pub node_ids: BTreeMap<String, u32>,
    /// Next free object id.
    pub next_id: u32,
    /// Picture files the objects refer to (`file:<path>` assets, as written in
    /// the graph): the app copies them into the deck before inserting.
    pub pictures: Vec<String>,
    pub warnings: Vec<String>,
}

/// Object name of a node.
pub fn node_name(fig: &str, node: &str) -> String {
    format!("flow:{fig}/{node}")
}

/// Is this object part of figure `fig` (any figure when `fig` is empty)?
pub fn is_flow_object(o: &Object, fig: &str) -> bool {
    if fig.is_empty() { o.name.starts_with("flow:") } else { o.name.starts_with(&format!("flow:{fig}/")) }
}

/// The connection site a line end is attached to: the point it really uses (0..16, as
/// `figure::SITES`) on a rectangle, the side's own site on an ellipse (which has eight).
fn end_site(side: Side, used: u8, ellipse: bool) -> u32 {
    if ellipse { site(side, true) } else { used as u32 }
}

fn site(side: Side, ellipse: bool) -> u32 {
    // PowerPoint's site order: rectangles top, left, bottom, right;
    // ellipses eight sites counter-clockwise from the top
    match (side, ellipse) {
        (Side::Top, _) => 0,
        (Side::Left, false) => 1,
        (Side::Bottom, false) => 2,
        (Side::Right, false) => 3,
        (Side::Left, true) => 2,
        (Side::Bottom, true) => 4,
        (Side::Right, true) => 6,
    }
}

fn rot_cw(v: (f64, f64), deg: f64) -> (f64, f64) {
    let (s, c) = deg.to_radians().sin_cos();
    (v.0 * c - v.1 * s, v.0 * s + v.1 * c)
}

/// Corner points (EMU) a connector geometry draws inside `f`.
/// The inverse of [`fit_connector`]; also used by the preview.
pub fn connector_path(geom: &str, adj: &BTreeMap<String, f64>, f: &Frame) -> Vec<(f64, f64)> {
    let (w, h) = (f.w as f64, f.h as f64);
    let a = |k: &str, d: f64| adj.get(k).copied().unwrap_or(d) / 100_000.0;
    let local: Vec<(f64, f64)> = match geom {
        "bentConnector2" => vec![(0.0, 0.0), (w, 0.0), (w, h)],
        "bentConnector3" => {
            let x = w * a("adj1", 50_000.0);
            vec![(0.0, 0.0), (x, 0.0), (x, h), (w, h)]
        }
        "bentConnector4" => {
            let (x, y) = (w * a("adj1", 50_000.0), h * a("adj2", 50_000.0));
            vec![(0.0, 0.0), (x, 0.0), (x, y), (w, y), (w, h)]
        }
        _ => vec![(0.0, 0.0), (w, h)],
    };
    let c = (f.x as f64 + w / 2.0, f.y as f64 + h / 2.0);
    local
        .into_iter()
        .map(|(x, y)| {
            let mut u = (x - w / 2.0, y - h / 2.0);
            if f.flip_h {
                u.0 = -u.0;
            }
            if f.flip_v {
                u.1 = -u.1;
            }
            let v = rot_cw(u, f.rot);
            (c.0 + v.0, c.1 + v.1)
        })
        .collect()
}

/// Connector geometry, frame and adjust values that draw the corner points
/// `pts` (EMU): a straight or bent connector for up to three corners.
pub fn fit_connector(pts: &[(f64, f64)]) -> Option<(String, Frame, BTreeMap<String, f64>)> {
    let n = pts.len().checked_sub(2)?;
    let (p0, pn) = (pts[0], pts[pts.len() - 1]);
    if n == 0 {
        return Some(("straightConnector1".into(), lore_canvas::geom::connector_frame(p0, pn), BTreeMap::new()));
    }
    if n > 3 {
        return None;
    }
    const MIN: f64 = 12_700.0;
    let tol = MIN / 2.0 + 60.0;
    let c = ((p0.0 + pn.0) / 2.0, (p0.1 + pn.1) / 2.0);
    for rot in [0.0, 90.0, 270.0, 180.0] {
        let d = rot_cw((pn.0 - p0.0, pn.1 - p0.1), -rot);
        let (mut w, mut h) = (d.0.abs(), d.1.abs());
        // an adjust value is a fraction of the extent, so the extent may not vanish
        if n >= 2 && w < MIN {
            w = MIN;
        }
        if n >= 3 && h < MIN {
            h = MIN;
        }
        for flip_h in [false, true] {
            for flip_v in [false, true] {
                let q: Vec<(f64, f64)> = pts
                    .iter()
                    .map(|p| {
                        let mut u = rot_cw((p.0 - c.0, p.1 - c.1), -rot);
                        if flip_h {
                            u.0 = -u.0;
                        }
                        if flip_v {
                            u.1 = -u.1;
                        }
                        (u.0 + w / 2.0, u.1 + h / 2.0)
                    })
                    .collect();
                let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol;
                if !near(q[0], (0.0, 0.0)) || !near(q[n + 1], (w, h)) {
                    continue;
                }
                // the first segment runs along local x
                if (q[1].1 - q[0].1).abs() > tol || (q[1].0 - q[0].0).abs() <= tol {
                    continue;
                }
                let mut adj = BTreeMap::new();
                let ok = match n {
                    1 => near(q[1], (w, 0.0)),
                    2 => {
                        adj.insert("adj1".to_string(), (q[1].0 / w * 100_000.0).round());
                        (q[2].0 - q[1].0).abs() <= tol && (q[2].1 - h).abs() <= tol
                    }
                    _ => {
                        adj.insert("adj1".to_string(), (q[1].0 / w * 100_000.0).round());
                        adj.insert("adj2".to_string(), (q[2].1 / h * 100_000.0).round());
                        (q[2].0 - q[1].0).abs() <= tol && (q[3].1 - q[2].1).abs() <= tol && (q[3].0 - w).abs() <= tol
                    }
                };
                if !ok {
                    continue;
                }
                let f = Frame { x: (c.0 - w / 2.0).round() as i64, y: (c.1 - h / 2.0).round() as i64, w: w.round() as i64, h: h.round() as i64, rot, flip_h, flip_v };
                return Some((format!("bentConnector{}", n + 1), f, adj));
            }
        }
    }
    None
}

/// The colour a line names (`Edge.color`): a name's strong line colour, or the hex itself.
fn own_line_colour(name: &str, theme: &Theme) -> Option<String> {
    theme.tint(name).map(|(_, line)| line)
}

fn is_hex(s: &str) -> bool {
    let h = s.trim().trim_start_matches('#');
    h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn stroke_of(theme: &Theme, kind: EdgeKind, k: f64) -> Stroke {
    stroke_coloured(theme, kind, k, None, 0.0)
}

/// As [`stroke_of`], in the line's own colour when it has one; dash, width and heads stay the kind's.
/// A positive `width` (pt at natural size) replaces the kind's width.
fn stroke_coloured(theme: &Theme, kind: EdgeKind, k: f64, own: Option<&str>, width: f64) -> Stroke {
    let e = theme.edge(kind);
    let w = if width > 0.0 { width } else { e.width };
    let mut s = Stroke::new(own.unwrap_or(&e.color), (w * k).max(0.5));
    s.dash = e.dash.clone();
    s.head = e.head.clone();
    s.tail = e.tail.clone();
    s
}

fn top_level(name: &str, fr: Frame, kind: Kind, id: u32) -> Object {
    Object { id, name: name.into(), frame: fr, hidden: false, ph: None, slot: None, kind, src: None }
}

/// Turn a laid-out, routed graph into canvas objects with ids from `first_id`.
#[allow(clippy::too_many_arguments)]
pub fn build(g: &Graph, theme: &Theme, vars: &Vars, lib: &Library, math: &dyn MathSource, lay: &Layout, routes: &[Route], first_id: u32) -> Built {
    build_with(g, theme, vars, lib, math, lay, routes, first_id, &modules::no_images)
}

/// As [`build`], knowing the pixel sizes of the picture files the graph names.
#[allow(clippy::too_many_arguments)]
pub fn build_with(g: &Graph, theme: &Theme, vars: &Vars, lib: &Library, math: &dyn MathSource, lay: &Layout, routes: &[Route], first_id: u32, image_size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Built {
    let k = lay.scale;
    let cx = Ctx { theme, vars, math, lib, k, symmetric: true, image_size };
    let mut id = first_id.max(2);
    let mut objects = Vec::new();
    let mut warnings = Vec::new();
    let fig = &g.fig;

    // frames (panels, regions) as units: the frame and its title, with the frame's rect; their
    // order against the lines is decided below
    let mut units: Vec<(Rect, std::ops::Range<usize>)> = Vec::new();
    // panels
    let mut framed = 0;
    for p in &lay.panels {
        let start = objects.len();
        if p.bare {
            continue;
        }
        let r = &p.rect;
        let fill = theme.panel_fill(framed);
        let line = theme.panel_line(framed);
        framed += 1;
        let own = g.panels.iter().find(|q| q.id == p.id);
        let styled = own.is_some_and(|q| !q.line.is_empty() || !q.tint.is_empty() || q.round);
        let mut o = if let Some(q) = own.filter(|_| styled) {
            // the panel's own options: drawn like a region's frame
            let named = theme.tint(&q.tint);
            let fill = named.as_ref().map(|(f, _)| Paint::solid(f)).unwrap_or(Paint::None);
            let round = theme.corners.map(|c| c.panel != 0.0).unwrap_or(q.round);
            let mut o = modules::shape("", if round { "roundRect" } else { "rect" }, r.x, r.y, r.w, r.h, fill, None);
            // soft frames: a tinted panel is outlined a step deeper than its fill, not in the
            // tint's strong colour (an outer frame stays pale)
            let colour = match (&named, theme.tint_mid(&q.tint).filter(|_| theme.region.soft)) {
                (Some(_), Some(m)) if !is_hex(&q.tint) => m,
                (Some((_, l)), _) if !is_hex(&q.tint) => l.clone(),
                _ if line != "none" => line.to_string(),
                _ => theme.colors.muted.clone(),
            };
            let dash = match q.line.as_str() {
                "dashed" => "dash".to_string(),
                "dotted" => "sysDot".to_string(),
                "solid" => "solid".to_string(),
                _ => theme.panel.dash.clone(),
            };
            if let Some(sp) = o.kind.sp_mut() {
                if q.line != "none" {
                    let mut s = Stroke::new(&colour, (theme.panel.line_w.max(1.0) * k).max(0.5));
                    s.dash = dash;
                    sp.line = Some(s);
                }
                if round {
                    sp.geom.adj.insert("adj".into(), Corners::adj(theme.corners.map(|c| c.panel).unwrap_or(8.0), k, r.w, r.h));
                }
            }
            o
        } else if let Some(c) = theme.corners {
            // a corner scheme: the panel's radius in points, whatever its size
            let mut o = modules::shape("", if c.panel != 0.0 { "roundRect" } else { "rect" }, r.x, r.y, r.w, r.h, if fill == "none" { Paint::None } else { Paint::solid(fill) }, None);
            if let Some(sp) = o.kind.sp_mut() {
                if line != "none" && theme.panel.line_w > 0.0 {
                    let mut s = Stroke::new(line, (theme.panel.line_w * k).max(0.5));
                    s.dash = theme.panel.dash.clone();
                    sp.line = Some(s);
                }
                if c.panel != 0.0 {
                    sp.geom.adj.insert("adj".into(), Corners::adj(c.panel, k, r.w, r.h));
                }
            }
            o
        } else {
            let mut o = modules::shape("", if theme.panel.radius > 0.0 { "roundRect" } else { "rect" }, r.x, r.y, r.w, r.h, if fill == "none" { Paint::None } else { Paint::solid(fill) }, None);
            if line != "none" && theme.panel.line_w > 0.0 {
                let mut s = Stroke::new(line, (theme.panel.line_w * k).max(0.5));
                s.dash = theme.panel.dash.clone();
                if let Some(sp) = o.kind.sp_mut() {
                    sp.line = Some(s);
                }
            }
            if theme.panel.radius > 0.0
                && let Some(sp) = o.kind.sp_mut()
            {
                // radius is a fraction of the short side; panels are large, so keep it modest
                sp.geom.adj.insert("adj".into(), (theme.panel.radius * 100_000.0 * (60.0 * k / r.w.min(r.h)).min(1.0)).round());
            }
            o
        };
        o.id = id;
        o.name = format!("flow:{fig}/panel:{}", p.id);
        id += 1;
        objects.push(o);
        if let Some(t) = &p.title_rect {
            let ty = cx.title();
            let (mut o, _, _) = cx.text("", &p.title, &ty, t.cx(), t.y);
            let at = own.map(|q| q.title_at.as_str()).unwrap_or("");
            if at == "left" {
                o.frame = modules::frame(t.x, t.y, t.w, t.h);
                if let Some(tb) = o.kind.sp_mut().and_then(|sp| sp.text.as_mut()) {
                    tb.paras.iter_mut().for_each(|p| p.align = "ctr".into());
                    tb.vert = Some("vert270".into());
                }
            } else if at == "top-left" || (at.is_empty() && !theme.panel.title.ends_with("center")) {
                // left-aligned from the title's corner, however wide the font turns out
                o.frame.x = emu(t.x);
                if let Some(tb) = o.kind.sp_mut().and_then(|sp| sp.text.as_mut()) {
                    tb.paras.iter_mut().for_each(|p| p.align = "l".into());
                }
            }
            o.id = id;
            o.name = format!("flow:{fig}/panel:{}/title", p.id);
            id += 1;
            objects.push(o);
        }
        units.push((p.rect, start..objects.len()));
    }

    // regions: outer first, so an inner tint lies over an outer one; plain shapes under the nodes
    let mut region_fills: Vec<(Rect, Option<String>)> = Vec::new();
    for rb in &lay.regions {
        let start = objects.len();
        let r = &rb.rect;
        let own = theme.tint(&rb.tint);
        let mid = theme.tint_mid(&rb.tint).filter(|_| theme.region.soft);
        let inner = rb.depth > 0;
        let fill = match (&own, &mid) {
            (Some(_), Some(m)) if inner => Paint::solid(m),
            (Some((f, _)), _) => Paint::solid(f),
            // soft frames: a module's frame is opaque, in the colour of what it sits on, so a
            // line that passes beneath it does not show through
            _ if theme.region.soft => {
                let under = region_fills.iter().rev().find(|(o, _)| o.contains(r)).and_then(|(_, f)| f.clone());
                Paint::solid(&under.unwrap_or_else(|| theme.colors.paper.clone()))
            }
            _ => Paint::None,
        };
        region_fills.push((*r, match &fill {
            Paint::Solid { color, .. } => Some(color.clone()),
            _ => None,
        }));
        // a corner scheme decides over the region's own `round`
        let radius = match theme.corners {
            Some(c) => if rb.depth == 0 { c.region } else { c.inner },
            None => if rb.round { 8.0 } else { 0.0 },
        };
        let prst = if radius != 0.0 { "roundRect" } else { "rect" };
        let mut o = modules::shape("", prst, r.x, r.y, r.w, r.h, fill, None);
        let colour = match (&own, &mid) {
            (Some(_), Some(m)) if !inner && !is_hex(&rb.tint) => m.clone(),
            (Some((_, l)), _) if !is_hex(&rb.tint) => l.clone(),
            // soft frames: a region without a tint is a module's frame, drawn like a block
            _ if theme.region.soft && own.is_none() => theme.colors.ink.clone(),
            _ => theme.colors.muted.clone(),
        };
        let (dash, on) = match rb.line.as_str() {
            "none" => ("solid", false),
            "dashed" => ("dash", true),
            "dotted" => ("sysDot", true),
            _ if inner && !theme.region.inner_dash.is_empty() => (theme.region.inner_dash.as_str(), true),
            _ => ("solid", true),
        };
        let weight = if theme.region.soft && own.is_none() {
            theme.node.line_w
        } else if inner {
            theme.region.inner_w
        } else {
            theme.region.line_w
        };
        if let Some(sp) = o.kind.sp_mut() {
            if on {
                let mut st = Stroke::new(&colour, (weight * k).max(0.5));
                st.dash = dash.into();
                sp.line = Some(st);
            }
            if radius != 0.0 {
                sp.geom.adj.insert("adj".into(), Corners::adj(radius, k, r.w, r.h));
            }
        }
        o.id = id;
        o.name = format!("flow:{fig}/region:{}", rb.id);
        id += 1;
        objects.push(o);
        if let Some(t) = &rb.title_rect
            && !rb.title.is_empty()
        {
            let title_colour = match &own {
                Some((_, l)) if theme.region.title_tint && !is_hex(&rb.tint) => l.clone(),
                _ => theme.colors.ink.clone(),
            };
            let ty = cx.ty(1.0, true, &title_colour);
            let (mut o, _, _) = cx.text("", &rb.title, &ty, t.cx(), t.y);
            o.frame = modules::frame(t.x, t.y, t.w, t.h);
            let align = if rb.title_at == "top-left" || rb.title_at.is_empty() { "l" } else { "ctr" };
            if let Some(tb) = o.kind.sp_mut().and_then(|sp| sp.text.as_mut()) {
                tb.paras.iter_mut().for_each(|p| p.align = align.into());
                if rb.title_at == "left" {
                    tb.vert = Some("vert270".into());
                }
            }
            o.id = id;
            o.name = format!("flow:{fig}/region:{}/title", rb.id);
            id += 1;
            objects.push(o);
        }
        units.push((*r, start..objects.len()));
    }

    // modules
    let mut node_ids = BTreeMap::new();
    let mut node_objs: Vec<Object> = Vec::new();
    for (n, pl) in g.nodes.iter().zip(&lay.nodes) {
        let body = modules::make(n, &Ctx { theme, vars, math, lib, k, symmetric: !lay.panels[pl.panel].down, image_size });
        // what a block could not draw (an icon the sets do not have)
        warnings.extend(body.warn.iter().cloned());
        let (o, next) = modules::place(body, &node_name(fig, &n.id), pl.rect.x, pl.rect.y, id);
        node_ids.insert(n.id.clone(), o.id);
        id = next;
        node_objs.push(o);
    }

    // lines: drawn under the blocks, so a line that has to cross a block runs beneath it; the
    // dots where lines branch and the labels lie over the blocks
    let mut line_objs: Vec<Object> = Vec::new();
    let mut dots: Vec<Object> = Vec::new();
    let small = cx.small(&theme.colors.ink);
    let mut labels = Vec::new();
    let mut label_rects: Vec<Rect> = Vec::new();
    for r in routes {
        let e = &g.edges[r.edge];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        let name = format!("flow:{fig}/edge:{}-{}", e.from, e.to);
        let mut own: Option<String> = None;
        if !e.color.trim().is_empty() {
            own = own_line_colour(&e.color, theme);
            if own.is_none() {
                warnings.push(format!("{} → {}: 不認得的顏色 {}", e.from, e.to, e.color));
            }
        }
        let mut st = stroke_coloured(theme, e.kind, k, own.as_deref(), e.width);
        // a line into a junction (a dot where lines meet) ends on the dot, without a head
        if g.nodes[b].module == "junction" {
            st.tail = "none".into();
        }
        let pts: Vec<(f64, f64)> = r.points.iter().map(|p| (p.0 * EMU_PT, p.1 * EMU_PT)).collect();
        let short = |ni: usize, side: Side| !r.forced && crate::route::words_side(&lay.nodes[ni], side);
        if r.forced {
            warnings.push(format!("{} -> {}: no clear path, drawn straight", e.from, e.to));
        }
        warnings.extend(r.warnings.iter().cloned());
        // one smooth curve: a free shape from end to end (it keeps its arrowheads; a curve
        // cannot be a connector, so it is not glued to its blocks)
        let mut curve_pts = crate::route::curve_path(r).filter(|c| c.len() == 4);
        if let Some(c) = &curve_pts {
            // blocks, then titles (words a curve may not cross either)
            let mut rooms: Vec<Rect> = lay.nodes.iter().map(|n| n.rect.union(&n.reserve)).collect();
            rooms.extend(lay.regions.iter().filter(|g| !g.title.is_empty()).filter_map(|g| g.title_rect));
            rooms.extend(lay.panels.iter().filter_map(|p| p.title_rect));
            if let Some(hit) = crate::route::curve_clear(c, &rooms, &[a, b]) {
                let what = g.nodes.get(hit).map(|n| n.id.clone()).unwrap_or_else(|| "標題".into());
                warnings.push(format!("{} → {}: 曲線會穿過 {what}，改走直角", e.from, e.to));
                curve_pts = None;
            }
        }
        let curved = curve_pts.map(|c| {
            let (x0, x1) = c.iter().fold((f64::MAX, f64::MIN), |m, p| (m.0.min(p.0), m.1.max(p.0)));
            let (y0, y1) = c.iter().fold((f64::MAX, f64::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
            let (w, h) = ((x1 - x0).max(0.01), (y1 - y0).max(0.01));
            let u = |p: (f64, f64)| format!("{:.5} {:.5}", (p.0 - x0) / w, (p.1 - y0) / h);
            let mut o = modules::shape(&name, "rect", x0, y0, w, h, Paint::None, Some(st.clone()));
            if let Some(sp) = o.kind.sp_mut() {
                sp.geom = lore_canvas::Geom { prst: "custom".into(), adj: BTreeMap::new(), path: Some(format!("M {} C {} {} {}", u(c[0]), u(c[1]), u(c[2]), u(c[3]))) };
            }
            o.id = id;
            o
        });
        let o = match curved {
            Some(o) => o,
            None => match fit_connector(&pts) {
            Some((geom, fr, adj)) => top_level(
                &name,
                fr,
                Kind::Connector(ConnObj {
                    geom,
                    adj,
                    line: Some(st.clone()),
                    // a line that leaves another line starts at no block
                    // a line that leaves another line starts at no block; an end that stops
                    // short of a block's words is not glued either (glued, the canvas would pull
                    // it onto the picture, through the words)
                    start: (!r.branch && !short(a, r.from_side)).then(|| Attach { id: node_ids[&e.from], site: end_site(r.from_side, r.from_site, lay.nodes[a].ellipse) }),
                    end: (!short(b, r.to_side)).then(|| Attach { id: node_ids[&e.to], site: end_site(r.to_side, r.to_site, lay.nodes[b].ellipse) }),
                    text: None,
                }),
                id,
            ),
            None => {
                warnings.push(if r.bends() > 3 { format!("{} → {}: 轉太多彎，沒有接在方塊上", e.from, e.to) } else { format!("{} → {}: 線的形狀接不到方塊上，沒有接在方塊上", e.from, e.to) });
                let mut o = modules::poly(&name, &r.points, false, Paint::None, Some(st.clone()));
                o.id = id;
                o
            }
            },
        };
        id += 1;
        line_objs.push(o);
        // where a line leaves another line: a dot in its colour
        if r.branch && let Some(p) = r.points.first() {
            let d = 3.2 * k;
            let mut o = modules::shape(&format!("{name}/junction"), "ellipse", p.0 - d / 2.0, p.1 - d / 2.0, d, d, Paint::solid(&st.color), None);
            o.id = id;
            id += 1;
            dots.push(o);
        }
        if !e.label.is_empty() {
            let own_small = own.as_ref().map(|c| cx.small(c));
            let (mut o, w, h) = cx.text("", &e.label, own_small.as_ref().unwrap_or(&small), 0.0, 0.0);
            let gap = 1.5 * k;
            // along the longest segments first, on the side that is free of modules and other labels
            let mut segs: Vec<((f64, f64), (f64, f64))> = r.points.windows(2).map(|s| (s[0], s[1])).collect();
            segs.sort_by(|a, b| {
                let len = |s: &((f64, f64), (f64, f64))| (s.1.0 - s.0.0).abs() + (s.1.1 - s.0.1).abs();
                len(b).partial_cmp(&len(a)).unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut cands: Vec<Rect> = Vec::new();
            for (p, q) in &segs {
                let horizontal = (p.1 - q.1).abs() < 1e-6;
                for t in [0.5, 0.3, 0.7, 0.15, 0.85] {
                    let (mx, my) = (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
                    if horizontal {
                        cands.push(Rect::new(mx - w / 2.0, my - h - gap, w, h));
                        cands.push(Rect::new(mx - w / 2.0, my + gap, w, h));
                    } else {
                        cands.push(Rect::new(mx + gap + k, my - h / 2.0, w, h));
                        cands.push(Rect::new(mx - gap - k - w, my - h / 2.0, w, h));
                    }
                }
            }
            // clear of blocks, other labels, titles, frame borders and every other line
            let on_line = |c: &Rect| routes.iter().any(|q| q.edge != r.edge && q.points.windows(2).any(|w| crate::route::segment_hits(w[0], w[1], c)));
            let clear = |c: &Rect| {
                !lay.nodes.iter().any(|n| n.reserve.overlaps(c))
                    && !label_rects.iter().any(|l: &Rect| l.overlaps(c))
                    && !lay.panels.iter().any(|p| p.title_rect.is_some_and(|t| t.overlaps(c)) || (!p.bare && c.overlaps(&p.rect) && !p.rect.contains(c)))
                    && !lay.regions.iter().any(|g| g.title_rect.is_some_and(|t| t.overlaps(c)))
                    && !on_line(c)
            };
            let pick = cands.iter().find(|c| clear(c)).or(cands.first()).copied().unwrap_or(Rect::new(0.0, 0.0, w, h));
            label_rects.push(pick);
            o.frame.x = emu(pick.x);
            o.frame.y = emu(pick.y);
            o.id = id;
            o.name = format!("{name}/label");
            id += 1;
            labels.push(o);
        }
    }
    // frames and lines in one order: a line lies over every frame that holds one of its ends
    // and under every frame it only passes through, so a line that has to cross a module runs
    // beneath it (an opaque frame hides it) instead of over it
    let frames: Vec<Vec<Object>> = {
        let mut all: Vec<Option<Object>> = objects.drain(..).map(Some).collect();
        units.iter().map(|(_, r)| r.clone().filter_map(|i| all[i].take()).collect()).collect()
    };
    let rects: Vec<Rect> = units.iter().map(|u| u.0).collect();
    let ends: Vec<[(f64, f64); 2]> = routes
        .iter()
        .map(|r| {
            let e = &g.edges[r.edge];
            let at = |id: &str| g.node_index(id).map(|i| (lay.nodes[i].rect.cx(), lay.nodes[i].rect.cy())).unwrap_or((0.0, 0.0));
            [if r.branch { r.points.first().copied().unwrap_or(at(&e.from)) } else { at(&e.from) }, at(&e.to)]
        })
        .collect();
    let (order, under) = frame_order(&rects, &ends, &routes.iter().map(|r| r.points.clone()).collect::<Vec<_>>());
    for i in under {
        let e = &g.edges[routes[i].edge];
        warnings.push(format!("{} → {}: 線穿過的框也有線穿進來，沒辦法全部壓在框下", e.from, e.to));
    }
    let mut frames: Vec<Option<Vec<Object>>> = frames.into_iter().map(Some).collect();
    let mut lines: Vec<Option<Object>> = line_objs.into_iter().map(Some).collect();
    for item in order {
        match item {
            Layer::Frame(i) => objects.extend(frames[i].take().unwrap_or_default()),
            Layer::Line(i) => objects.extend(lines[i].take()),
        }
    }
    objects.extend(node_objs);
    objects.extend(dots);
    objects.extend(labels);

    // legend
    for (li, l) in lay.legend.iter().enumerate() {
        let key = if l.boxed.is_some() { format!("box{li}") } else { l.kind.as_str().to_string() };
        let mut line = match &l.boxed {
            Some((fill, stroke)) => {
                let (a, b) = l.line;
                let paint = if fill == "none" { Paint::None } else { Paint::solid(fill) };
                let st = if stroke == "none" { None } else { Some(Stroke::new(stroke, 0.75 * k)) };
                modules::shape("", "rect", a.0, a.1, b.0 - a.0, b.1 - a.1, paint, st)
            }
            None => modules::seg("", l.line.0, l.line.1, stroke_of(theme, l.kind, k)),
        };
        line.id = id;
        line.name = format!("flow:{fig}/legend:{key}");
        id += 1;
        objects.push(line);
        let ty = cx.small(&theme.colors.ink);
        let words = if l.label.is_empty() { l.kind.label() } else { l.label.as_str() };
        let (mut o, w, h) = cx.text("", words, &ty, 0.0, 0.0);
        o.frame.x = emu(l.text.x);
        o.frame.y = emu(l.text.cy() - h / 2.0);
        let _ = w;
        o.id = id;
        if l.boxed.is_some()
            && let Some(tb) = o.kind.sp_mut().and_then(|sp| sp.text.as_mut())
        {
            tb.paras.iter_mut().for_each(|p| p.align = "l".into());
        }
        o.name = format!("flow:{fig}/legend:{key}/label");
        id += 1;
        objects.push(o);
    }

    // colour legend
    for s in &lay.swatches {
        let a = &s.at;
        let mut sw = modules::shape("", "rect", a.x, a.y, a.w, a.h, Paint::solid(&s.swatch.color), Some(Stroke::new(&theme.colors.ink, 0.5 * k)));
        sw.id = id;
        sw.name = format!("flow:{fig}/colours:{}", s.swatch.key);
        id += 1;
        objects.push(sw);
        let ty = cx.small(&theme.colors.ink);
        let (mut o, _, h) = cx.text("", &s.swatch.label, &ty, 0.0, 0.0);
        o.frame.x = emu(s.text.x);
        o.frame.y = emu(s.text.cy() - h / 2.0);
        if let Some(tb) = o.kind.sp_mut().and_then(|sp| sp.text.as_mut()) {
            tb.paras.iter_mut().for_each(|p| p.align = "l".into());
        }
        o.id = id;
        o.name = format!("flow:{fig}/colours:{}/label", s.swatch.key);
        id += 1;
        objects.push(o);
    }

    // plain numbers in the file: widths to 1/100 pt, type sizes to 1/10 pt
    fn tidy(o: &mut Object) {
        let r2 = |v: f64| (v * 100.0).round() / 100.0;
        let r1 = |v: f64| (v * 10.0).round() / 10.0;
        match &mut o.kind {
            Kind::Text(sp) | Kind::Shape(sp) => {
                if let Some(l) = &mut sp.line {
                    l.width_pt = r2(l.width_pt);
                }
                if let Some(tb) = &mut sp.text {
                    for p in &mut tb.paras {
                        p.runs.iter_mut().for_each(|r| r.look.size = r1(r.look.size));
                        if let Some(e) = &mut p.end {
                            e.size = r1(e.size);
                        }
                    }
                }
            }
            Kind::Connector(c) => {
                if let Some(l) = &mut c.line {
                    l.width_pt = r2(l.width_pt);
                }
            }
            Kind::Picture(p) => {
                if let Some(l) = &mut p.line {
                    l.width_pt = r2(l.width_pt);
                }
            }
            Kind::Group(g) => g.children.iter_mut().for_each(tidy),
            _ => {}
        }
    }
    objects.iter_mut().for_each(tidy);
    let mut pictures: Vec<String> = Vec::new();
    fn collect(o: &Object, out: &mut Vec<String>) {
        match &o.kind {
            Kind::Picture(p) => {
                if let Some(f) = p.asset.strip_prefix("file:")
                    && !out.iter().any(|x| x == f)
                {
                    out.push(f.to_string());
                }
            }
            Kind::Group(g) => g.children.iter().for_each(|c| collect(c, out)),
            _ => {}
        }
    }
    objects.iter().for_each(|o| collect(o, &mut pictures));
    Built { objects, node_ids, next_id: id, pictures, warnings }
}

/// The batch that puts a figure on a slide: one `InsertObj` per object, on top
/// of what is there (`at` = the slide's current object count). Apply it with a
/// single `Editor::apply` so one undo removes the whole figure.
pub fn insert_ops(slide: &str, at: usize, objects: &[Object]) -> Vec<Op> {
    objects.iter().enumerate().map(|(i, o)| Op::InsertObj { slide: slide.to_string(), parent: None, index: at + i, obj: o.clone() }).collect()
}

/// PowerPoint glues connectors to shapes, never to groups. Before exporting,
/// point every connector end that is attached to a module group at the group's
/// `port` child (an invisible rectangle with the same four sites).
pub fn retarget_for_pptx(objects: &mut [Object]) {
    let mut port: BTreeMap<u32, u32> = BTreeMap::new();
    for o in objects.iter() {
        if let Kind::Group(g) = &o.kind
            && o.name.starts_with("flow:")
            && let Some(p) = g.children.iter().find(|c| c.name.ends_with("/port"))
        {
            port.insert(o.id, p.id);
        }
    }
    for o in objects.iter_mut() {
        if let Kind::Connector(c) = &mut o.kind {
            for end in [&mut c.start, &mut c.end] {
                if let Some(a) = end
                    && let Some(p) = port.get(&a.id)
                {
                    a.id = *p;
                }
            }
        }
    }
}

/// One item of the drawing order of frames and lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layer {
    Frame(usize),
    Line(usize),
}

/// The drawing order of frames (rects; an outer frame before the frames inside it) and lines
/// (their two ends and their points): a line over every frame that holds one of its ends and
/// under every frame it only passes through. Where two lines ask for opposite orders of the
/// same frames, the later is drawn over them; those lines are returned second.
pub fn frame_order(frames: &[Rect], ends: &[[(f64, f64); 2]], lines: &[Vec<(f64, f64)>]) -> (Vec<Layer>, Vec<usize>) {
    let (nf, nl) = (frames.len(), lines.len());
    let n = nf + nl;
    let inside = |r: &Rect, p: (f64, f64)| p.0 > r.x && p.0 < r.right() && p.1 > r.y && p.1 < r.bottom();
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut before = vec![0usize; n];
    let edge = |a: usize, b: usize, after: &mut Vec<Vec<usize>>, before: &mut Vec<usize>| {
        after[a].push(b);
        before[b] += 1;
    };
    for i in 0..nf {
        for j in 0..nf {
            if i != j && frames[i].contains(&frames[j]) && !(frames[j].contains(&frames[i]) && j < i) {
                edge(i, j, &mut after, &mut before);
            }
        }
    }
    for (l, pts) in lines.iter().enumerate() {
        for (f, r) in frames.iter().enumerate() {
            if ends[l].iter().any(|p| inside(r, *p)) {
                edge(f, nf + l, &mut after, &mut before);
            } else if pts.windows(2).any(|s| crate::route::segment_hits(s[0], s[1], r)) {
                edge(nf + l, f, &mut after, &mut before);
            }
        }
    }
    let mut done = vec![false; n];
    let mut out = Vec::with_capacity(n);
    let mut forced = Vec::new();
    for _ in 0..n {
        // the first item (frames first, then lines, each in their own order) that waits for nothing
        let pick = (0..n).find(|&i| !done[i] && before[i] == 0).unwrap_or_else(|| {
            // a cycle: draw a frame that waits only for lines passing through it; those lines
            // then lie over it (its parents are drawn first, so a line's own frames stay under it)
            let waits = |i: usize| (0..n).filter(|&a| !done[a] && after[a].contains(&i)).collect::<Vec<_>>();
            let f = (0..nf).find(|&f| !done[f] && waits(f).iter().all(|&a| a >= nf)).or_else(|| (0..n).find(|&i| !done[i])).expect("one left");
            for a in waits(f) {
                if a >= nf && !forced.contains(&(a - nf)) {
                    forced.push(a - nf);
                }
            }
            f
        });
        done[pick] = true;
        for &b in &after[pick] {
            before[b] = before[b].saturating_sub(1);
        }
        out.push(if pick < nf { Layer::Frame(pick) } else { Layer::Line(pick - nf) });
    }
    (out, forced)
}
