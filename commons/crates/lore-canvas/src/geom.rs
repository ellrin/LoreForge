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

/// Connection sites of an object in slide space (top-level objects only),
/// following PowerPoint's site order: rectangles 0 top, 1 left, 2 bottom,
/// 3 right; ellipses 8 sites counter-clockwise from the top.
pub fn sites(o: &Object) -> Vec<(f64, f64)> {
    let f = &o.frame;
    let (x, y, w, h) = (f.x as f64, f.y as f64, f.w as f64, f.h as f64);
    let prst = o.kind.sp().map(|s| s.geom.prst.as_str()).unwrap_or("rect");
    let local: Vec<(f64, f64)> = if prst == "ellipse" {
        let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        (0..8)
            .map(|i| {
                // 0 = top, then counter-clockwise in 45° steps
                let a = (-90.0 - 45.0 * i as f64).to_radians();
                (cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect()
    } else {
        vec![
            (x + w / 2.0, y),
            (x, y + h / 2.0),
            (x + w / 2.0, y + h),
            (x + w, y + h / 2.0),
        ]
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
