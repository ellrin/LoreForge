//! Geometry helpers: bounds, group coordinate spaces, connection sites,
//! connector routing, alignment and distribution.

use crate::model::*;

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
/// Ellipses have 8 sites counter-clockwise from the top; custom paths one per
/// vertex; other presets (triangle, diamond …) keep four (0–3 as above).
pub fn sites(o: &Object) -> Vec<(f64, f64)> {
    let f = &o.frame;
    let (x, y, w, h) = (f.x as f64, f.y as f64, f.w as f64, f.h as f64);
    let prst = o.kind.sp().map(|s| s.geom.prst.as_str()).unwrap_or("rect");
    let sixteen = o.kind.sp().is_none() || matches!(prst, "rect" | "roundRect");
    let custom = o.kind.sp().filter(|s| s.geom.prst == "custom").and_then(|s| s.geom.path.as_deref()).map(path_vertices).unwrap_or_default();
    let local: Vec<(f64, f64)> = if !custom.is_empty() {
        // custom shapes (polygons): one site per vertex, in path order
        custom
            .iter()
            .map(|&(u, v)| (x + if f.flip_h { 1.0 - u } else { u } * w, y + if f.flip_v { 1.0 - v } else { v } * h))
            .collect()
    } else if prst == "ellipse" {
        let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        (0..8)
            .map(|i| {
                // 0 = top, then counter-clockwise in 45° steps
                let a = (-90.0 - 45.0 * i as f64).to_radians();
                (cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect()
    } else {
        let mut v = vec![
            (x + w / 2.0, y),
            (x, y + h / 2.0),
            (x + w / 2.0, y + h),
            (x + w, y + h / 2.0),
        ];
        if sixteen {
            let (r, b) = (x + w, y + h);
            let (x1, x3, y1, y3) = (x + w / 4.0, x + w * 0.75, y + h / 4.0, y + h * 0.75);
            v.extend([
                (x, y),   // 4 top-left
                (x1, y),  // 5 top ¼
                (x3, y),  // 6 top ¾
                (r, y),   // 7 top-right
                (r, y1),  // 8 right ¼
                (r, y3),  // 9 right ¾
                (r, b),   // 10 bottom-right
                (x3, b),  // 11 bottom ¾
                (x1, b),  // 12 bottom ¼
                (x, b),   // 13 bottom-left
                (x, y3),  // 14 left ¾
                (x, y1),  // 15 left ¼
            ]);
        }
        v
    };
    if f.rot == 0.0 {
        return local;
    }
    let c = f.center();
    local.into_iter().map(|p| rotate(p, c, f.rot)).collect()
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
        assert_eq!(sites(&shape("ellipse", Frame::new(0, 0, 400, 400))).len(), 8);
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
    fn other_presets_keep_four() {
        for p in ["triangle", "diamond", "trapezoid"] {
            assert_eq!(sites(&shape(p, Frame::new(0, 0, 400, 400))).len(), 4, "{p}");
        }
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
}
