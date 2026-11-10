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
use crate::theme::Theme;

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

fn stroke_of(theme: &Theme, kind: EdgeKind, k: f64) -> Stroke {
    let e = theme.edge(kind);
    let mut s = Stroke::new(&e.color, (e.width * k).max(0.5));
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

    // panels
    let mut framed = 0;
    for p in &lay.panels {
        if p.bare {
            continue;
        }
        let r = &p.rect;
        let fill = theme.panel_fill(framed);
        let line = theme.panel_line(framed);
        framed += 1;
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
        o.id = id;
        o.name = format!("flow:{fig}/panel:{}", p.id);
        id += 1;
        objects.push(o);
        if let Some(t) = &p.title_rect {
            let ty = cx.title();
            let (mut o, _, _) = cx.text("", &p.title, &ty, t.cx(), t.y);
            if !theme.panel.title.ends_with("center") {
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
    }

    // modules
    let mut node_ids = BTreeMap::new();
    for (n, pl) in g.nodes.iter().zip(&lay.nodes) {
        let body = modules::make(n, &Ctx { theme, vars, math, lib, k, symmetric: !lay.panels[pl.panel].down, image_size });
        let (o, next) = modules::place(body, &node_name(fig, &n.id), pl.rect.x, pl.rect.y, id);
        node_ids.insert(n.id.clone(), o.id);
        id = next;
        objects.push(o);
    }

    // lines
    let small = cx.small(&theme.colors.ink);
    let mut labels = Vec::new();
    let mut label_rects: Vec<Rect> = Vec::new();
    for r in routes {
        let e = &g.edges[r.edge];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        let name = format!("flow:{fig}/edge:{}-{}", e.from, e.to);
        let st = stroke_of(theme, e.kind, k);
        let pts: Vec<(f64, f64)> = r.points.iter().map(|p| (p.0 * EMU_PT, p.1 * EMU_PT)).collect();
        if r.forced {
            warnings.push(format!("{} -> {}: no clear path, drawn straight", e.from, e.to));
        }
        let o = match fit_connector(&pts) {
            Some((geom, fr, adj)) => top_level(
                &name,
                fr,
                Kind::Connector(ConnObj {
                    geom,
                    adj,
                    line: Some(st),
                    start: Some(Attach { id: node_ids[&e.from], site: site(r.from_side, lay.nodes[a].ellipse) }),
                    end: Some(Attach { id: node_ids[&e.to], site: site(r.to_side, lay.nodes[b].ellipse) }),
                    text: None,
                }),
                id,
            ),
            None => {
                let mut o = modules::poly(&name, &r.points, false, Paint::None, Some(st));
                o.id = id;
                o
            }
        };
        id += 1;
        objects.push(o);
        if !e.label.is_empty() {
            let (mut o, w, h) = cx.text("", &e.label, &small, 0.0, 0.0);
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
            let clear = |c: &Rect| !lay.nodes.iter().any(|n| n.reserve.overlaps(c)) && !label_rects.iter().any(|l: &Rect| l.overlaps(c)) && !lay.panels.iter().any(|p| p.title_rect.is_some_and(|t| t.overlaps(c)) || (!p.bare && c.overlaps(&p.rect) && !p.rect.contains(c)));
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
    objects.extend(labels);

    // legend
    for l in &lay.legend {
        let mut line = modules::seg("", l.line.0, l.line.1, stroke_of(theme, l.kind, k));
        line.id = id;
        line.name = format!("flow:{fig}/legend:{}", l.kind.as_str());
        id += 1;
        objects.push(line);
        let ty = cx.small(&theme.colors.ink);
        let (mut o, w, h) = cx.text("", l.kind.label(), &ty, 0.0, 0.0);
        o.frame.x = emu(l.text.x);
        o.frame.y = emu(l.text.cy() - h / 2.0);
        let _ = w;
        o.id = id;
        o.name = format!("flow:{fig}/legend:{}/label", l.kind.as_str());
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
