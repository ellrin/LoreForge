//! Geometry helpers: bounds, group coordinate spaces, connection sites,
//! connector routing, alignment and distribution.

use crate::model::*;
use std::collections::BTreeMap;

/// Axis-aligned union of frames (rotation ignored).
pub fn bounds<'a>(frames: impl Iterator<Item = &'a Frame>) -> Frame {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for f in frames {
        x0 = x0.min(f.x);
        y0 = y0.min(f.y);
        x1 = x1.max(f.x + f.w);
        y1 = y1.max(f.y + f.h);
    }
    if x0 == i64::MAX {
        return Frame::default();
    }
    Frame::new(x0, y0, x1 - x0, y1 - y0)
}

/// Map an object's frame from a group's child space to the group's parent space
/// (recursively for nested groups only the top level is mapped; children keep
/// their own child spaces).
pub fn child_to_parent(group: &Frame, ch: &Frame, o: &mut Object) {
    let sx = if ch.w == 0 { 1.0 } else { group.w as f64 / ch.w as f64 };
    let sy = if ch.h == 0 { 1.0 } else { group.h as f64 / ch.h as f64 };
    let f = &mut o.frame;
    // map centre so rotation stays about the centre
    let cx = group.x as f64 + (f.x as f64 + f.w as f64 / 2.0 - ch.x as f64) * sx;
    let cy = group.y as f64 + (f.y as f64 + f.h as f64 / 2.0 - ch.y as f64) * sy;
    let w = f.w as f64 * sx;
    let h = f.h as f64 * sy;
    f.x = (cx - w / 2.0).round() as i64;
    f.y = (cy - h / 2.0).round() as i64;
    f.w = w.round() as i64;
    f.h = h.round() as i64;
    if group.rot != 0.0 {
        // rotate the centre around the group centre
        let (gx, gy) = group.center();
        let (px, py) = rotate((cx, cy), (gx, gy), group.rot);
        f.x = (px - w / 2.0).round() as i64;
        f.y = (py - h / 2.0).round() as i64;
        f.rot = (f.rot + group.rot).rem_euclid(360.0);
    }
    if group.flip_h {
        f.flip_h = !f.flip_h;
    }
    if group.flip_v {
        f.flip_v = !f.flip_v;
    }
}

pub fn rotate(p: (f64, f64), c: (f64, f64), deg: f64) -> (f64, f64) {
    let (s, co) = deg.to_radians().sin_cos();
    let (dx, dy) = (p.0 - c.0, p.1 - c.1);
    (c.0 + dx * co - dy * s, c.1 + dx * s + dy * co)
}

/// Connection sites of an object in slide space (top-level objects only).
/// Rectangles (`rect`, `roundRect`, text boxes) and every object without a
/// shape of its own (pictures, groups, tables) have sixteen, in the frozen order
/// of `lore_flow::figure::SITES`: 0 top, 1 left, 2 bottom, 3 right (PowerPoint's
/// order for a rectangle), then clockwise from the top-left corner 4 top-left,
/// 5 top ¼, 6 top ¾, 7 top-right, 8 right ¼, 9 right ¾, 10 bottom-right,
/// 11 bottom ¾, 12 bottom ¼, 13 bottom-left, 14 left ¾, 15 left ¼ (¼ and ¾
/// measured left to right on top and bottom, top to bottom on the sides).
/// Ellipses have sixteen: 0–7 counter-clockwise from the top in 45° steps (0 top,
/// 1 upper left, 2 left …), 8–15 the points half way between, in the same order
/// (8 = 22.5° left of the top, 9 = 67.5° …). Every other known preset (see
/// [`preset_sites_unit`]) has sixteen too: the point where the ray from the centre to
/// the rectangle's site of the same number leaves the outline. Custom paths have one
/// per vertex; a preset not known here keeps four (0–3 as above).
pub fn sites(o: &Object) -> Vec<(f64, f64)> {
    let f = &o.frame;
    let (mut x, mut y, mut w, mut h) = (f.x as f64, f.y as f64, f.w as f64, f.h as f64);
    // a group that names one child `port` (a flow block: the drawn picture without its words)
    // connects at that child's rectangle
    if let crate::model::Kind::Group(g) = &o.kind
        && let Some(p) = g.children.iter().find(|c| c.name == "port" || c.name.ends_with("/port"))
        && g.ch.w > 0
        && g.ch.h > 0
    {
        let (sx, sy) = (w / g.ch.w as f64, h / g.ch.h as f64);
        let pf = &p.frame;
        (x, y, w, h) = (x + (pf.x - g.ch.x) as f64 * sx, y + (pf.y - g.ch.y) as f64 * sy, pf.w as f64 * sx, pf.h as f64 * sy);
    }
    let prst = o.kind.sp().map(|s| s.geom.prst.as_str()).unwrap_or("rect");
    let sixteen = o.kind.sp().is_none() || matches!(prst, "rect" | "roundRect");
    let custom = o.kind.sp().filter(|s| s.geom.prst == "custom").and_then(|s| s.geom.path.as_deref()).map(path_vertices).unwrap_or_default();
    let preset = if sixteen || !custom.is_empty() {
        None
    } else {
        o.kind.sp().and_then(|s| preset_sites_unit(prst, &s.geom.adj, w, h, f.flip_h, f.flip_v))
    };
    let local: Vec<(f64, f64)> = if !custom.is_empty() {
        // custom shapes (polygons): one site per vertex, in path order
        custom
            .iter()
            .map(|&(u, v)| (x + if f.flip_h { 1.0 - u } else { u } * w, y + if f.flip_v { 1.0 - v } else { v } * h))
            .collect()
    } else if let Some(u) = preset {
        u.into_iter().map(|(a, b)| (x + a * w, y + b * h)).collect()
    } else if sixteen {
        RECT_UNIT.iter().map(|&(a, b)| (x + a * w, y + b * h)).collect()
    } else {
        RECT_UNIT[..4].iter().map(|&(a, b)| (x + a * w, y + b * h)).collect()
    };
    if f.rot == 0.0 {
        return local;
    }
    let c = f.center();
    local.into_iter().map(|p| rotate(p, c, f.rot)).collect()
}

/// The rectangle's sixteen sites in the unit box, in the frozen order.
pub const RECT_UNIT: [(f64, f64); 16] = [
    (0.5, 0.0), (0.0, 0.5), (0.5, 1.0), (1.0, 0.5),
    (0.0, 0.0), (0.25, 0.0), (0.75, 0.0), (1.0, 0.0),
    (1.0, 0.25), (1.0, 0.75), (1.0, 1.0),
    (0.75, 1.0), (0.25, 1.0), (0.0, 1.0),
    (0.0, 0.75), (0.0, 0.25),
];

type Poly = Vec<(f64, f64)>;

/// Presets whose outline [`exact_outline`] knows exactly (so they can be written as a
/// custom geometry with the same outline).
pub const EXACT_PRESETS: &[&str] = &[
    "triangle", "rtTriangle", "diamond", "parallelogram", "trapezoid", "pentagon", "hexagon", "octagon",
    "chevron", "homePlate", "rightArrow", "leftArrow", "upArrow", "downArrow", "leftRightArrow", "star5", "plus",
];

/// Outline of a preset in the unit box (`w`, `h`: the frame's size, which the adjust values
/// relative to the short side need). `exact` is false where the outline is a polygon
/// approximation (can, cloud) or a silhouette (cube).
fn outline(prst: &str, adj: &BTreeMap<String, f64>, w: f64, h: f64) -> Option<(Poly, bool)> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let a = |k: &str, d: f64| adj.get(k).copied().unwrap_or(d);
    let ss = w.min(h);
    let (uw, uh) = (ss / w, ss / h); // one `ss` as a part of the width / the height
    let k = 1.0 / 100000.0;
    let star = |inner: f64| -> Poly {
        let (cx, cy, rx, ry) = (0.5, 0.5 * 1.10557, 0.5 * 1.05146, 0.5 * 1.10557);
        let n = if inner < 0.0 { 5 } else { 10 };
        (0..n)
            .map(|i| {
                let (ang, r) = if inner < 0.0 { (-90.0 + 72.0 * i as f64, 1.0) } else { (-90.0 + 36.0 * i as f64, if i % 2 == 0 { 1.0 } else { inner }) };
                let t = f64::to_radians(ang);
                (cx + rx * r * t.cos(), cy + ry * r * t.sin())
            })
            .collect()
    };
    let (poly, exact): (Poly, bool) = match prst {
        "triangle" => {
            let x = a("adj", 50000.0).clamp(0.0, 100000.0) * k;
            (vec![(0.0, 1.0), (x, 0.0), (1.0, 1.0)], true)
        }
        "rtTriangle" => (vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)], true),
        "diamond" => (vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)], true),
        "parallelogram" => {
            let x2 = (a("adj", 25000.0).clamp(0.0, 100000.0 * w / ss) * k * uw).min(1.0);
            (vec![(0.0, 1.0), (x2, 0.0), (1.0, 0.0), (1.0 - x2, 1.0)], true)
        }
        "trapezoid" => {
            let x2 = (a("adj", 25000.0).clamp(0.0, 50000.0 * w / ss) * k * uw).min(0.5);
            (vec![(0.0, 1.0), (x2, 0.0), (1.0 - x2, 0.0), (1.0, 1.0)], true)
        }
        "pentagon" => (star(-1.0), true),
        "star5" => (star(a("adj", 19098.0).clamp(0.0, 50000.0) / 50000.0), true),
        "hexagon" => {
            let x1 = (a("adj", 25000.0).clamp(0.0, 50000.0 * w / ss) * k * uw).min(0.5);
            (vec![(0.0, 0.5), (x1, 0.0), (1.0 - x1, 0.0), (1.0, 0.5), (1.0 - x1, 1.0), (x1, 1.0)], true)
        }
        "octagon" => {
            let v = a("adj", 29289.0).clamp(0.0, 50000.0) * k;
            let (x, y) = (v * uw, v * uh);
            (vec![(0.0, y), (x, 0.0), (1.0 - x, 0.0), (1.0, y), (1.0, 1.0 - y), (1.0 - x, 1.0), (x, 1.0), (0.0, 1.0 - y)], true)
        }
        "chevron" => {
            let x1 = (a("adj", 50000.0).clamp(0.0, 100000.0 * w / ss) * k * uw).min(1.0);
            (vec![(0.0, 0.0), (1.0 - x1, 0.0), (1.0, 0.5), (1.0 - x1, 1.0), (0.0, 1.0), (x1, 0.5)], true)
        }
        "homePlate" => {
            let d = (a("adj", 50000.0).clamp(0.0, 100000.0 * w / ss) * k * uw).min(1.0);
            (vec![(0.0, 0.0), (1.0 - d, 0.0), (1.0, 0.5), (1.0 - d, 1.0), (0.0, 1.0)], true)
        }
        "rightArrow" | "leftArrow" => {
            let half = a("adj1", 50000.0).clamp(0.0, 100000.0) * k / 2.0;
            let d = (a("adj2", 50000.0).clamp(0.0, 100000.0 * w / ss) * k * uw).min(1.0);
            let (y1, y2) = (0.5 - half, 0.5 + half);
            let p = vec![(0.0, y1), (1.0 - d, y1), (1.0 - d, 0.0), (1.0, 0.5), (1.0 - d, 1.0), (1.0 - d, y2), (0.0, y2)];
            (if prst == "leftArrow" { p.into_iter().map(|(x, y)| (1.0 - x, y)).collect() } else { p }, true)
        }
        "upArrow" | "downArrow" => {
            let half = a("adj1", 50000.0).clamp(0.0, 100000.0) * k / 2.0;
            let d = (a("adj2", 50000.0).clamp(0.0, 100000.0 * h / ss) * k * uh).min(1.0);
            let (x1, x2) = (0.5 - half, 0.5 + half);
            let p = vec![(0.0, d), (x1, d), (x1, 1.0), (x2, 1.0), (x2, d), (1.0, d), (0.5, 0.0)];
            (if prst == "downArrow" { p.into_iter().map(|(x, y)| (x, 1.0 - y)).collect() } else { p }, true)
        }
        "leftRightArrow" => {
            let half = a("adj1", 50000.0).clamp(0.0, 100000.0) * k / 2.0;
            let d = (a("adj2", 50000.0).clamp(0.0, 50000.0 * w / ss) * k * uw).min(0.5);
            let (y1, y2) = (0.5 - half, 0.5 + half);
            (vec![(0.0, 0.5), (d, 0.0), (d, y1), (1.0 - d, y1), (1.0 - d, 0.0), (1.0, 0.5), (1.0 - d, 1.0), (1.0 - d, y2), (d, y2), (d, 1.0)], true)
        }
        "plus" => {
            let v = a("adj", 25000.0).clamp(0.0, 50000.0) * k;
            let (x, y) = (v * uw, v * uh);
            (
                vec![(0.0, y), (x, y), (x, 0.0), (1.0 - x, 0.0), (1.0 - x, y), (1.0, y), (1.0, 1.0 - y), (1.0 - x, 1.0 - y), (1.0 - x, 1.0), (x, 1.0), (x, 1.0 - y), (0.0, 1.0 - y)],
                true,
            )
        }
        "can" => {
            let ry = (a("adj", 25000.0).clamp(0.0, 50000.0 * h / ss) * k * uh / 2.0).min(0.5);
            let n = 24;
            let mut p = Vec::new();
            for i in 0..=n {
                let t = std::f64::consts::PI * (1.0 + i as f64 / n as f64);
                p.push((0.5 + 0.5 * t.cos(), ry + ry * t.sin()));
            }
            for i in 0..=n {
                let t = std::f64::consts::PI * i as f64 / n as f64;
                p.push((0.5 + 0.5 * t.cos(), 1.0 - ry + ry * t.sin()));
            }
            (p, false)
        }
        "cube" => {
            let v = a("adj", 25000.0).clamp(0.0, 100000.0) * k;
            let (x, y) = (v * uw, v * uh);
            (vec![(0.0, y), (x, 0.0), (1.0, 0.0), (1.0, 1.0 - y), (1.0 - x, 1.0), (0.0, 1.0)], false)
        }
        // a cloud is taken as its bounding ellipse
        "cloud" => (
            (0..360)
                .map(|i| {
                    let t = f64::to_radians(i as f64);
                    (0.5 + 0.5 * t.cos(), 0.5 + 0.5 * t.sin())
                })
                .collect(),
            false,
        ),
        _ => return None,
    };
    Some((poly, exact))
}

/// The outline of a preset in the unit box, only when it is exactly the preset's own (so a
/// custom geometry with it looks the same). `adj` are the values of `Geom.adj`.
pub fn exact_outline(prst: &str, adj: &BTreeMap<String, f64>, w: f64, h: f64) -> Option<Vec<(f64, f64)>> {
    outline(prst, adj, w, h).filter(|(_, e)| *e).map(|(p, _)| p)
}

/// Where the ray from the box centre through `target` leaves the polygon.
/// `c` is the box centre, or for a shape whose centre lies on its outline (a right triangle) its
/// vertex centroid. A ray that leaves nowhere (it starts in a notch, as a chevron's) ends at the
/// centre when that is on the outline, else at the outline point nearest to `target`.
fn leave_polygon(poly: &[(f64, f64)], c: (f64, f64), target: (f64, f64)) -> (f64, f64) {
    let d = (target.0 - c.0, target.1 - c.1);
    let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
    let mut best: Option<f64> = None;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = (q.0 - p.0, q.1 - p.1);
        let den = cross(d, e);
        if den.abs() < 1e-14 {
            continue;
        }
        let pc = (p.0 - c.0, p.1 - c.1);
        let t = cross(pc, e) / den;
        let s = cross(pc, d) / den;
        if (-1e-9..=1.0 + 1e-9).contains(&s) && t > 1e-9 && best.is_none_or(|b| t > b) {
            best = Some(t);
        }
    }
    if let Some(t) = best {
        return (c.0 + t * d.0, c.1 + t * d.1);
    }
    let mut nearest = (f64::MAX, c);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ex, ey) = (q.0 - p.0, q.1 - p.1);
        let l2 = ex * ex + ey * ey;
        for &r in &[c, target] {
            let t = if l2 == 0.0 { 0.0 } else { (((r.0 - p.0) * ex + (r.1 - p.1) * ey) / l2).clamp(0.0, 1.0) };
            let n = (p.0 + t * ex, p.1 + t * ey);
            let dist = (n.0 - r.0).hypot(n.1 - r.1);
            if r == c && dist < 1e-9 {
                return c;
            }
            if r == target && dist < nearest.0 {
                nearest = (dist, n);
            }
        }
    }
    nearest.1
}

/// The sixteen connection sites of a preset in the unit box of its frame, in the numbering of
/// [`sites`]; `None` for a preset not known (it keeps four). `w`, `h` are the frame's size.
/// Flips mirror the outline (the numbering stays that of the rectangle, as for rectangles);
/// an ellipse's points do not turn with a flip, as before.
pub fn preset_sites_unit(prst: &str, adj: &BTreeMap<String, f64>, w: f64, h: f64, flip_h: bool, flip_v: bool) -> Option<Vec<(f64, f64)>> {
    if prst == "ellipse" {
        let at = |deg: f64| {
            let t = deg.to_radians();
            (0.5 + 0.5 * t.cos(), 0.5 + 0.5 * t.sin())
        };
        let mut v: Vec<(f64, f64)> = (0..8).map(|i| at(-90.0 - 45.0 * i as f64)).collect();
        v.extend((0..8).map(|i| at(-90.0 - 22.5 - 45.0 * i as f64)));
        return Some(v);
    }
    let (mut poly, _) = outline(prst, adj, w, h)?;
    for p in &mut poly {
        if flip_h {
            p.0 = 1.0 - p.0;
        }
        if flip_v {
            p.1 = 1.0 - p.1;
        }
    }
    let c = if prst == "rtTriangle" {
        let n = poly.len() as f64;
        (poly.iter().map(|p| p.0).sum::<f64>() / n, poly.iter().map(|p| p.1).sum::<f64>() / n)
    } else {
        (0.5, 0.5)
    };
    Some(RECT_UNIT.iter().map(|&t| leave_polygon(&poly, c, t)).collect())
}

/// Start and end point of a connector (slide space, before rotation is applied
/// to bent shapes — straight lines are exact).
pub fn connector_points(f: &Frame) -> ((f64, f64), (f64, f64)) {
    let (x0, x1) = if f.flip_h {
        ((f.x + f.w) as f64, f.x as f64)
    } else {
        (f.x as f64, (f.x + f.w) as f64)
    };
    let (y0, y1) = if f.flip_v {
        ((f.y + f.h) as f64, f.y as f64)
    } else {
        (f.y as f64, (f.y + f.h) as f64)
    };
    if f.rot == 0.0 {
        return ((x0, y0), (x1, y1));
    }
    let c = f.center();
    (rotate((x0, y0), c, f.rot), rotate((x1, y1), c, f.rot))
}

/// Frame of a connector running from `a` to `b`.
pub fn connector_frame(a: (f64, f64), b: (f64, f64)) -> Frame {
    Frame {
        x: a.0.min(b.0).round() as i64,
        y: a.1.min(b.1).round() as i64,
        w: (b.0 - a.0).abs().round() as i64,
        h: (b.1 - a.1).abs().round() as i64,
        rot: 0.0,
        flip_h: b.0 < a.0,
        flip_v: b.1 < a.1,
    }
}

/// Connectors (top level) whose attached ends no longer sit on their target
/// sites: new frames.
/// Only connectors attached to an object in `moved` are considered (imported
/// connectors are never nudged just because something else changed).
pub fn reroute(s: &Slide, moved: &std::collections::BTreeSet<u32>) -> Vec<(u32, Frame)> {
    let mut out = Vec::new();
    for o in &s.objects {
        let Kind::Connector(c) = &o.kind else { continue };
        let hit = |a: &Option<Attach>| a.as_ref().is_some_and(|a| moved.contains(&a.id));
        if !hit(&c.start) && !hit(&c.end) {
            continue;
        }
        let (mut a, mut b) = connector_points(&o.frame);
        let site = |at: &Attach| -> Option<(f64, f64)> {
            let t = s.objects.iter().find(|x| x.id == at.id)?;
            let v = sites(t);
            v.get(at.site as usize).copied()
        };
        if let Some(p) = c.start.as_ref().and_then(site) {
            a = p;
        }
        if let Some(p) = c.end.as_ref().and_then(site) {
            b = p;
        }
        let mut nf = connector_frame(a, b);
        nf.rot = 0.0;
        let (oa, ob) = connector_points(&o.frame);
        let moved = (oa.0 - a.0).abs() > 1.0 || (oa.1 - a.1).abs() > 1.0 || (ob.0 - b.0).abs() > 1.0 || (ob.1 - b.1).abs() > 1.0;
        if moved {
            out.push((o.id, nf));
        }
    }
    out
}

/// Nearest connection site within `tol` EMU of point `p` among top-level objects
/// (connectors and the object `exclude` skipped).
pub fn snap_site(s: &Slide, p: (f64, f64), tol: f64, exclude: Option<u32>) -> Option<Attach> {
    let mut best: Option<(f64, Attach)> = None;
    for o in &s.objects {
        if Some(o.id) == exclude || matches!(o.kind, Kind::Connector(_)) {
            continue;
        }
        for (i, q) in sites(o).into_iter().enumerate() {
            let d = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
            if d <= tol && best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
                best = Some((
                    d,
                    Attach {
                        id: o.id,
                        site: i as u32,
                    },
                ));
            }
        }
    }
    best.map(|(_, a)| a)
}

/// Alignment of frames: `left`, `center`, `right`, `top`, `middle`, `bottom`
/// relative to their union (or to the slide when only one frame is given).
pub fn align(frames: &[Frame], how: &str, slide: &Size) -> Vec<Frame> {
    let b = if frames.len() == 1 {
        Frame::new(0, 0, slide.w, slide.h)
    } else {
        bounds(frames.iter())
    };
    frames
        .iter()
        .map(|f| {
            let mut g = f.clone();
            match how {
                "left" => g.x = b.x,
                "center" => g.x = b.x + (b.w - f.w) / 2,
                "right" => g.x = b.x + b.w - f.w,
                "top" => g.y = b.y,
                "middle" => g.y = b.y + (b.h - f.h) / 2,
                "bottom" => g.y = b.y + b.h - f.h,
                _ => {}
            }
            g
        })
        .collect()
}

/// Distribute frames evenly (`h` or `v`) keeping the outermost two in place.
pub fn distribute(frames: &[Frame], axis: &str) -> Vec<Frame> {
    let n = frames.len();
    if n < 3 {
        return frames.to_vec();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let horiz = axis == "h";
    idx.sort_by_key(|&i| if horiz { frames[i].x } else { frames[i].y });
    let total: i64 = frames.iter().map(|f| if horiz { f.w } else { f.h }).sum();
    let first = &frames[idx[0]];
    let last = &frames[idx[n - 1]];
    let span = if horiz {
        last.x + last.w - first.x
    } else {
        last.y + last.h - first.y
    };
    let gap = (span - total) as f64 / (n - 1) as f64;
    let mut out = frames.to_vec();
    let mut pos = if horiz { first.x } else { first.y } as f64;
    for &i in &idx {
        if horiz {
            out[i].x = pos.round() as i64;
            pos += frames[i].w as f64 + gap;
        } else {
            out[i].y = pos.round() as i64;
            pos += frames[i].h as f64 + gap;
        }
    }
    out
}

/// Vertices of a custom path (0..1 box, `M`/`L`/`C`/`Q`/`Z`): the end point of
/// every segment, without the repeated start of a closed path. These are a
/// custom shape's connection sites (`a:cxnLst` on export, in the same order).
pub fn path_vertices(path: &str) -> Vec<(f64, f64)> {
    let mut toks: Vec<String> = Vec::new();
    for t in path.replace(',', " ").split_whitespace() {
        let mut cur = String::new();
        for ch in t.chars() {
            if ch.is_ascii_alphabetic() {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
                toks.push(ch.to_string());
            } else {
                cur.push(ch);
            }
        }
        if !cur.is_empty() {
            toks.push(cur);
        }
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut i = 0;
    let mut start: Option<(f64, f64)> = None;
    while i < toks.len() {
        let pairs = match toks[i].as_str() {
            "M" | "L" => 1,
            "Q" => 2,
            "C" => 3,
            "Z" | "z" => {
                if let (Some(s), Some(l)) = (start, out.last())
                    && (s.0 - l.0).abs() < 1e-9
                    && (s.1 - l.1).abs() < 1e-9
                    && out.len() > 1
                {
                    out.pop();
                }
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let is_move = toks[i] == "M";
        i += 1;
        let mut last = (0.0, 0.0);
        for _ in 0..pairs {
            let x = toks.get(i).and_then(|t| t.parse().ok()).unwrap_or(0.0);
            let y = toks.get(i + 1).and_then(|t| t.parse().ok()).unwrap_or(0.0);
            i += 2;
            last = (x, y);
        }
        if is_move {
            start = Some(last);
        }
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

/// Scale an object by `k` about the origin of its parent's space: frame, text
/// sizes and insets, line widths (arrowheads follow the width), table cell
/// sizes, and a group's child space and children. A group resized as one
/// picture ([`crate::Op::Scale`]) uses this on its children.
pub fn scale_object(o: &mut Object, k: f64) {
    let sc = |v: Emu| (v as f64 * k).round() as Emu;
    o.frame.x = sc(o.frame.x);
    o.frame.y = sc(o.frame.y);
    o.frame.w = sc(o.frame.w);
    o.frame.h = sc(o.frame.h);
    fn text(tb: &mut TextBody, k: f64) {
        for i in &mut tb.insets {
            *i = (*i as f64 * k).round() as Emu;
        }
        for p in &mut tb.paras {
            for r in &mut p.runs {
                r.look.size = (r.look.size * k * 10.0).round() / 10.0;
            }
            if let Some(e) = &mut p.end {
                e.size = (e.size * k * 10.0).round() / 10.0;
            }
        }
    }
    match &mut o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            if let Some(l) = &mut sp.line {
                l.width_pt *= k;
            }
            if let Some(tb) = &mut sp.text {
                text(tb, k);
            }
        }
        Kind::Connector(c) => {
            if let Some(l) = &mut c.line {
                l.width_pt *= k;
            }
        }
        Kind::Picture(p) => {
            if let Some(l) = &mut p.line {
                l.width_pt *= k;
            }
        }
        Kind::Group(g) => {
            g.ch.x = sc(g.ch.x);
            g.ch.y = sc(g.ch.y);
            g.ch.w = sc(g.ch.w);
            g.ch.h = sc(g.ch.h);
            for c in &mut g.children {
                scale_object(c, k);
            }
        }
        Kind::Table(t) => {
            for c in &mut t.cols {
                *c = sc(*c);
            }
            for r in &mut t.rows {
                r.h = sc(r.h);
                for c in &mut r.cells {
                    text(&mut c.text, k);
                }
            }
        }
        Kind::Locked(_) => {}
    }
}

/// `o` resized to `w` wide as one picture: aspect kept, contents scaled
/// ([`scale_object`]), the top-left corner at (`x`, `y`).
pub fn scaled_to(o: &Object, x: Emu, y: Emu, w: Emu) -> Object {
    let k = if o.frame.w > 0 { w as f64 / o.frame.w as f64 } else { 1.0 };
    let mut n = o.clone();
    scale_object(&mut n, k);
    n.frame = Frame { x, y, w: (o.frame.w as f64 * k).round() as Emu, h: (o.frame.h as f64 * k).round() as Emu, ..o.frame.clone() };
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn shape(prst: &str, f: Frame) -> Object {
        Object {
            id: 1,
            name: "s".into(),
            frame: f,
            hidden: false,
            ph: None,
            slot: None,
            kind: Kind::Shape(SpObj { geom: Geom::preset(prst), fill: Paint::None, line: None, text: None, equation: false }),
            src: None,
        }
    }

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn a_rectangle_has_sixteen_points_five_a_side() {
        let v = sites(&shape("rect", Frame::new(0, 0, 400, 800)));
        assert_eq!(v.len(), 16);
        let want = [
            (200., 0.), (0., 400.), (200., 800.), (400., 400.),
            (0., 0.), (100., 0.), (300., 0.), (400., 0.),
            (400., 200.), (400., 600.), (400., 800.),
            (300., 800.), (100., 800.), (0., 800.),
            (0., 600.), (0., 200.),
        ];
        for (i, w) in want.iter().enumerate() {
            assert!(near(v[i], *w), "{i}: {:?} vs {:?}", v[i], w);
        }
        // five per side: corners, quarters and middles lie on the frame
        let top = v.iter().filter(|p| p.1 == 0.0).count();
        let bottom = v.iter().filter(|p| p.1 == 800.0).count();
        let left = v.iter().filter(|p| p.0 == 0.0).count();
        let right = v.iter().filter(|p| p.0 == 400.0).count();
        assert_eq!((top, bottom, left, right), (5, 5, 5, 5));
        assert_eq!(sites(&shape("roundRect", Frame::new(0, 0, 400, 800))), v);
    }

    #[test]
    fn the_first_four_points_are_where_they_always_were() {
        let v = sites(&shape("rect", Frame::new(100, 200, 600, 400)));
        assert!(near(v[0], (400., 200.)) && near(v[1], (100., 400.)) && near(v[2], (400., 600.)) && near(v[3], (700., 400.)));
    }

    #[test]
    fn the_points_turn_with_the_shape() {
        let mut f = Frame::new(0, 0, 400, 200);
        f.rot = 90.0;
        let v = sites(&shape("rect", f));
        // centre (200,100); top-left corner (0,0) turned 90° clockwise lands at (300,-100)
        assert!(near(v[4], (300., -100.)), "{:?}", v[4]);
        assert!(near(v[0], (300., 100.)), "{:?}", v[0]);
    }

    #[test]
    fn ellipses_and_custom_paths_keep_their_points() {
        let mut o = shape("custom", Frame::new(0, 0, 400, 400));
        if let Kind::Shape(sp) = &mut o.kind {
            sp.geom.path = Some("M0 0 L1 0 L0.5 1 Z".into());
        }
        assert_eq!(sites(&o).len(), 3);
    }

    #[test]
    fn a_group_and_a_picture_have_sixteen_too() {
        let g = Object {
            id: 2,
            name: "g".into(),
            frame: Frame::new(0, 0, 100, 100),
            hidden: false,
            ph: None,
            slot: None,
            kind: Kind::Group(GroupObj { ch: Frame::new(0, 0, 100, 100), children: vec![], lock_aspect: false, data: Default::default() }),
            src: None,
        };
        assert_eq!(sites(&g).len(), 16);
    }

    #[test]
    fn an_unknown_preset_keeps_four() {
        assert_eq!(sites(&shape("wave", Frame::new(0, 0, 400, 400))).len(), 4);
    }

    fn slide_with(block: Object, conn: Object) -> Slide {
        let mut s = Slide::new("s1".into(), None);
        s.objects = vec![block, conn];
        s
    }

    fn connector(to: u32, site: u32) -> Object {
        Object {
            id: 9,
            name: "c".into(),
            frame: Frame::new(0, 0, 0, 0),
            hidden: false,
            ph: None,
            slot: None,
            kind: Kind::Connector(ConnObj { geom: "line".into(), adj: Default::default(), line: None, start: None, end: Some(Attach { id: to, site }), text: None }),
            src: None,
        }
    }

    #[test]
    fn a_connector_on_a_quarter_point_follows_its_block() {
        let mut b = shape("rect", Frame::new(0, 0, 400, 400));
        let s0 = slide_with(b.clone(), connector(1, 5));
        b.frame.y += 1000;
        let mut s = s0.clone();
        s.objects[0] = b;
        let moved: BTreeSet<u32> = [1].into();
        let r = reroute(&s, &moved);
        assert_eq!(r.len(), 1);
        let (_, end) = connector_points(&r[0].1);
        assert!(near(end, (100., 1000.)), "{end:?}");
    }

    #[test]
    fn snapping_finds_the_nearest_of_sixteen() {
        let s = slide_with(shape("rect", Frame::new(0, 0, 400, 400)), connector(1, 0));
        let a = snap_site(&s, (305., 3.), 20.0, Some(9)).unwrap();
        assert_eq!((a.id, a.site), (1, 6));
        let a = snap_site(&s, (3., 395.), 20.0, None).unwrap();
        assert_eq!(a.site, 13);
        assert!(snap_site(&s, (150., 150.), 20.0, None).is_none());
    }

    // ---- sixteen sites for every preset ----

    const KNOWN: &[&str] = &[
        "triangle", "rtTriangle", "diamond", "parallelogram", "trapezoid", "pentagon", "hexagon", "octagon", "chevron", "homePlate",
        "rightArrow", "leftArrow", "upArrow", "downArrow", "leftRightArrow", "can", "cube", "star5", "plus", "cloud",
    ];

    #[test]
    fn group_with_a_port_connects_at_the_port() {
        use crate::model::{GroupObj, Kind};
        let mut port = with("rect", Frame::new(0, 50, 200, 100), &[]);
        port.name = "flow:f/n/port".into();
        let g = Object { id: 1, name: "flow:f/n".into(), frame: Frame::new(1000, 2000, 400, 600), hidden: false, ph: None, slot: None, kind: Kind::Group(GroupObj { ch: Frame::new(0, 0, 200, 300), children: vec![port], lock_aspect: false, data: None }), src: None };
        let s = sites(&g);
        assert_eq!(s.len(), 16);
        assert_eq!((s[0].0.round(), s[0].1.round()), (1200.0, 2100.0), "top middle of the port");
        assert_eq!((s[2].0.round(), s[2].1.round()), (1200.0, 2300.0), "bottom middle of the port");
        // a group without a port connects at its own frame
        let mut plain = g.clone();
        if let Kind::Group(gr) = &mut plain.kind {
            gr.children[0].name = "body".into();
        }
        assert_eq!((sites(&plain)[0].1).round(), 2000.0);
    }

    fn with(prst: &str, f: Frame, adj: &[(&str, f64)]) -> Object {
        let mut o = shape(prst, f);
        if let Kind::Shape(sp) = &mut o.kind {
            for (k, v) in adj {
                sp.geom.adj.insert((*k).into(), *v);
            }
        }
        o
    }

    fn dist_to_polygon(poly: &[(f64, f64)], p: (f64, f64)) -> f64 {
        let mut best = f64::MAX;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ex, ey) = (b.0 - a.0, b.1 - a.1);
            let l2 = ex * ex + ey * ey;
            let t = if l2 == 0.0 { 0.0 } else { (((p.0 - a.0) * ex + (p.1 - a.1) * ey) / l2).clamp(0.0, 1.0) };
            best = best.min(((a.0 + t * ex - p.0).powi(2) + (a.1 + t * ey - p.1).powi(2)).sqrt());
        }
        best
    }

    #[test]
    fn an_ellipse_has_sixteen_and_its_first_eight_are_where_they_were() {
        let v = sites(&shape("ellipse", Frame::new(0, 0, 400, 200)));
        assert_eq!(v.len(), 16);
        // the old eight: 0 top, then counter-clockwise in 45° steps
        for i in 0..8 {
            let a = (-90.0 - 45.0 * i as f64).to_radians();
            assert!(near(v[i], (200.0 + 200.0 * a.cos(), 100.0 + 100.0 * a.sin())), "{i}");
        }
        assert!(near(v[0], (200., 0.)) && near(v[2], (0., 100.)) && near(v[4], (200., 200.)) && near(v[6], (400., 100.)));
        // 8..15: in between, in the same counter-clockwise order, all on the ellipse
        for i in 0..8 {
            let a = (-90.0 - 22.5 - 45.0 * i as f64).to_radians();
            assert!(near(v[8 + i], (200.0 + 200.0 * a.cos(), 100.0 + 100.0 * a.sin())), "{}", 8 + i);
            let (dx, dy) = ((v[8 + i].0 - 200.0) / 200.0, (v[8 + i].1 - 100.0) / 100.0);
            assert!((dx * dx + dy * dy - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn a_rectangle_is_unchanged() {
        let v = sites(&shape("rect", Frame::new(10, 20, 400, 200)));
        assert_eq!(v.len(), 16);
        for (i, &(u, w)) in RECT_UNIT.iter().enumerate() {
            assert!(near(v[i], (10.0 + u * 400.0, 20.0 + w * 200.0)), "{i}");
        }
    }

    #[test]
    fn every_known_preset_has_sixteen_on_its_outline() {
        for p in KNOWN {
            for f in [Frame::new(0, 0, 400, 400), Frame::new(100, 50, 900, 300), Frame::new(0, 0, 200, 700)] {
                let o = shape(p, f.clone());
                let v = sites(&o);
                assert_eq!(v.len(), 16, "{p}");
                let (poly, _) = outline(p, &BTreeMap::new(), f.w as f64, f.h as f64).unwrap();
                for (i, q) in v.iter().enumerate() {
                    let u = ((q.0 - f.x as f64) / f.w as f64, (q.1 - f.y as f64) / f.h as f64);
                    assert!(dist_to_polygon(&poly, u) < 1e-6, "{p} site {i} at {u:?}");
                }
            }
        }
    }

    #[test]
    fn sites_zero_to_three_are_the_extremes_for_every_preset() {
        for p in KNOWN {
            let f = Frame::new(0, 0, 400, 400);
            let v = sites(&shape(p, f.clone()));
            if *p == "rtTriangle" {
                continue; // its rays start at the centroid (the box centre is on its hypotenuse)
            }
            // on the axes through the centre, on the right side of it
            assert!((v[0].0 - 200.0).abs() < 1e-6 && v[0].1 < 200.0, "{p} top {:?}", v[0]);
            assert!((v[2].0 - 200.0).abs() < 1e-6 && v[2].1 > 200.0, "{p} bottom {:?}", v[2]);
            assert!((v[1].1 - 200.0).abs() < 1e-6 && v[1].0 <= 200.0, "{p} left {:?}", v[1]);
            assert!((v[3].1 - 200.0).abs() < 1e-6 && v[3].0 > 200.0, "{p} right {:?}", v[3]);
        }
        // where the outline touches the middle of the frame's side, the site is that point
        let v = sites(&shape("diamond", Frame::new(0, 0, 400, 400)));
        assert!(near(v[0], (200., 0.)) && near(v[2], (200., 400.)) && near(v[1], (0., 200.)) && near(v[3], (400., 200.)));
        let v = sites(&shape("hexagon", Frame::new(0, 0, 400, 400)));
        assert!(near(v[1], (0., 200.)) && near(v[3], (400., 200.)) && near(v[0], (200., 0.)) && near(v[2], (200., 400.)));
        // a triangle's left site is the middle of its left edge, as in PowerPoint
        let v = sites(&shape("triangle", Frame::new(0, 0, 400, 400)));
        assert!(near(v[1], (100., 200.)) && near(v[3], (300., 200.)) && near(v[0], (200., 0.)));
    }

    #[test]
    fn adjust_values_move_the_sites() {
        let f = Frame::new(0, 0, 400, 400);
        let a = sites(&with("trapezoid", f.clone(), &[("adj", 25000.0)]));
        let b = sites(&with("trapezoid", f.clone(), &[("adj", 40000.0)]));
        // site 4 is where the diagonal from the centre meets the slanted left edge: x = y with
        // x = (ss * adj / 100000) * (1 - y / h)
        assert!(near(a[4], (80., 80.)) && near(b[4], (400. * 0.4 / 1.4, 400. * 0.4 / 1.4)), "{:?} {:?}", a[4], b[4]);
        assert!(a[4] != b[4]);
        // the left middle follows the slanted edge
        assert!(near(a[1], (50., 200.)) && near(b[1], (80., 200.)));
        let h1 = sites(&with("hexagon", f.clone(), &[("adj", 25000.0)]));
        let h2 = sites(&with("hexagon", f.clone(), &[("adj", 10000.0)]));
        assert!(h1[4] != h2[4]);
    }

    #[test]
    fn flips_and_rotation_apply() {
        let f = Frame::new(0, 0, 400, 400);
        let a = sites(&shape("rtTriangle", f.clone()));
        let mut ff = f.clone();
        ff.flip_h = true;
        let b = sites(&shape("rtTriangle", ff));
        // the right angle is at the left; flipped it is at the right: the left middle moves
        assert!(near(a[1], (0., 200.)), "{:?}", a[1]);
        assert!(b[1].0 > 100.0, "{:?}", b[1]);
        let mut fr = Frame::new(0, 0, 400, 200);
        fr.rot = 90.0;
        let d = sites(&shape("diamond", fr.clone()));
        // centre (200,100); the top (200,0) turned 90° clockwise lands at (300,100)
        assert!(near(d[0], (300., 100.)), "{:?}", d[0]);
        assert_eq!(sites(&shape("hexagon", fr)).len(), 16);
    }

    #[test]
    fn a_line_attached_to_site_ten_of_a_hexagon_follows_it() {
        let b = shape("hexagon", Frame::new(0, 0, 400, 400));
        let start = sites(&b)[10];
        let s0 = slide_with(b.clone(), connector(1, 10));
        let mut s = s0.clone();
        s.objects[0].frame.x += 700;
        s.objects[0].frame.y += 300;
        let moved: BTreeSet<u32> = [1].into();
        let r = reroute(&s, &moved);
        assert_eq!(r.len(), 1);
        let (_, end) = connector_points(&r[0].1);
        let tol = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= 1.0 && (a.1 - b.1).abs() <= 1.0;
        assert!(tol(end, (start.0 + 700.0, start.1 + 300.0)), "{end:?} vs {start:?}");
        assert!(tol(end, sites(&s.objects[0])[10]));
    }

    fn vectors() -> serde_json::Value {
        let mut cases = Vec::new();
        let mut names: Vec<&str> = KNOWN.to_vec();
        names.extend(["ellipse", "rect", "roundRect"]);
        let adjs: [&[(&str, f64)]; 3] = [&[], &[("adj", 40000.0)], &[("adj1", 30000.0), ("adj2", 60000.0)]];
        for p in names {
            for (fi, f) in [Frame::new(1_000_000, 500_000, 3_000_000, 1_500_000), Frame::new(0, 0, 914_400, 2_000_000)].into_iter().enumerate() {
                for adj in adjs {
                    for (fh, fv) in [(false, false), (true, false), (false, true)] {
                        if fi == 1 && (fh || fv) {
                            continue;
                        }
                        let mut fr = f.clone();
                        fr.flip_h = fh;
                        fr.flip_v = fv;
                        if adj.len() == 2 && fi == 1 {
                            fr.rot = 30.0;
                        }
                        let o = with(p, fr.clone(), adj);
                        let pts: Vec<serde_json::Value> = sites(&o).iter().map(|q| serde_json::json!([q.0.round() as i64, q.1.round() as i64])).collect();
                        let adjm: serde_json::Map<String, serde_json::Value> = adj.iter().map(|(k, v)| ((*k).to_string(), serde_json::json!(v))).collect();
                        cases.push(serde_json::json!({
                            "prst": p, "frame": {"x": fr.x, "y": fr.y, "w": fr.w, "h": fr.h, "rot": fr.rot, "flip_h": fh, "flip_v": fv},
                            "adj": adjm, "sites": pts
                        }));
                    }
                }
            }
        }
        serde_json::Value::Array(cases)
    }

    fn vectors_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sites.json")
    }

    /// `cargo test -p lore-canvas --lib write_sites_vectors -- --ignored` rewrites the shared vector.
    #[test]
    #[ignore]
    fn write_sites_vectors() {
        let p = vectors_path();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, serde_json::to_string_pretty(&vectors()).unwrap() + "\n").unwrap();
    }

    #[test]
    fn the_shared_sites_vector_is_current() {
        let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors_path()).expect("run write_sites_vectors")).unwrap();
        assert_eq!(on_disk, vectors(), "tests/data/sites.json is out of date: run write_sites_vectors --ignored");
    }
}
