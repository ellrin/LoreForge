//! Lines on a packed page, by construction of four shapes only: straight, one bend (an L), two
//! bends going on in the same direction (a step), or two bends round the outside of what lies
//! between two blocks of one row or column (up, across, down: taken when the straight line would
//! be hidden beneath blocks or along other lines; its outer run lies clear of the blocks between
//! and of their words, inside the frame that holds both blocks when there is room, never off the
//! page). No line has more than two bends; what may be chosen is the side a line leaves and
//! arrives at and, for a step, where its middle run lies. A line whose blocks stand in one row
//! or one column is straight even with a block between them: it passes beneath that block (the
//! blocks are drawn over the lines) and its arrowhead shows. Among the shapes that exist for a
//! line the cheapest is taken: fewest bends, then fewest blocks passed beneath, then not along
//! a line already drawn, then not through a block's words, then the shortest.

use crate::layout::{Layout, Placed, Rect};
use crate::route::{Route, Side, segment_hits, site_of, site_point};
use crate::spec::{EdgeKind, Graph};

type Pt = (f64, f64);

/// The share of the gap between two blocks at which a step's middle run may lie.
const MIDDLES: [f64; 3] = [0.5, 0.3, 0.7];

struct Cand {
    pts: Vec<Pt>,
    from: Side,
    to: Side,
}

fn mid(n: &Placed, s: Side) -> Pt {
    site_point(n, s, 2)
}

/// Every line of the three shapes from `a` to `b`.
fn shapes(a: &Placed, b: &Placed) -> Vec<Cand> {
    let (ra, rb) = (&a.art, &b.art);
    let mut v = Vec::new();
    // which way b lies from a, when their pictures are apart in that direction
    let across = if rb.x >= ra.right() { Some((Side::Right, Side::Left)) } else if rb.right() <= ra.x { Some((Side::Left, Side::Right)) } else { None };
    let down = if rb.y >= ra.bottom() { Some((Side::Bottom, Side::Top)) } else if rb.bottom() <= ra.y { Some((Side::Top, Side::Bottom)) } else { None };
    // straight, whenever one line can join the two: their pictures face each other (they share
    // a stretch of height, or of width). The line runs through the arriving block's middle when
    // that lies on the leaving block's side, else through the leaving block's middle, else
    // through the middle of what they share
    let level = |lo_a: f64, hi_a: f64, pa: f64, lo_b: f64, hi_b: f64, pb: f64| -> Option<f64> {
        let (lo, hi) = (lo_a.max(lo_b), hi_a.min(hi_b));
        (hi - lo > 1.0).then(|| if pb > lo_a + 0.5 && pb < hi_a - 0.5 { pb } else if pa > lo_b + 0.5 && pa < hi_b - 0.5 { pa } else { (lo + hi) / 2.0 })
    };
    if let (Some((s, t)), Some(y)) = (across, level(ra.y, ra.bottom(), a.port_y, rb.y, rb.bottom(), b.port_y)) {
        v.push(Cand { pts: vec![(mid(a, s).0, y), (mid(b, t).0, y)], from: s, to: t });
    }
    if let (Some((s, t)), Some(x)) = (down, level(ra.x, ra.right(), ra.cx(), rb.x, rb.right(), rb.cx())) {
        v.push(Cand { pts: vec![(x, mid(a, s).1), (x, mid(b, t).1)], from: s, to: t });
    }
    // one bend: across first, then down into b; or down first, then across into b
    let side_x = if rb.cx() > ra.right() { Some(Side::Right) } else if rb.cx() < ra.x { Some(Side::Left) } else { None };
    let into_y = if a.port_y < rb.y { Some(Side::Top) } else if a.port_y > rb.bottom() { Some(Side::Bottom) } else { None };
    if let (Some(s), Some(t)) = (side_x, into_y) {
        v.push(Cand { pts: vec![mid(a, s), (rb.cx(), a.port_y), mid(b, t)], from: s, to: t });
    }
    let side_y = if b.port_y > ra.bottom() { Some(Side::Bottom) } else if b.port_y < ra.y { Some(Side::Top) } else { None };
    let into_x = if ra.cx() < rb.x { Some(Side::Left) } else if ra.cx() > rb.right() { Some(Side::Right) } else { None };
    if let (Some(s), Some(t)) = (side_y, into_x) {
        v.push(Cand { pts: vec![mid(a, s), (ra.cx(), b.port_y), mid(b, t)], from: s, to: t });
    }
    // two bends: a step whose middle run lies in the gap between the two
    if let Some((s, t)) = across.filter(|_| level(ra.y, ra.bottom(), a.port_y, rb.y, rb.bottom(), b.port_y).is_none()) {
        let (p, q) = (mid(a, s), mid(b, t));
        for f in MIDDLES {
            let x = p.0 + (q.0 - p.0) * f;
            v.push(Cand { pts: vec![p, (x, p.1), (x, q.1), q], from: s, to: t });
        }
    }
    if let Some((s, t)) = down.filter(|_| level(ra.x, ra.right(), ra.cx(), rb.x, rb.right(), rb.cx()).is_none()) {
        let (p, q) = (mid(a, s), mid(b, t));
        for f in MIDDLES {
            let y = p.1 + (q.1 - p.1) * f;
            v.push(Cand { pts: vec![p, (p.0, y), (q.0, y), q], from: s, to: t });
        }
    }
    v
}

/// Where a block's words are written: the part of its frame above its picture, and the part
/// under it, each only when words lie there.
fn word_rects(n: &Placed) -> Vec<Rect> {
    let (x0, x1) = (n.reserve.x.min(n.rect.x), n.reserve.right().max(n.rect.right()));
    let mut v = Vec::new();
    if n.words.0 && n.art.y - n.rect.y > 0.5 {
        v.push(Rect::new(x0, n.rect.y, x1 - x0, n.art.y - n.rect.y));
    }
    if n.words.1 && n.rect.bottom() - n.art.bottom() > 0.5 {
        v.push(Rect::new(x0, n.art.bottom(), x1 - x0, n.rect.bottom() - n.art.bottom()));
    }
    v
}

/// How much a line round the outside of a row counts over its two bends: it is taken when the
/// straight line would be hidden (beneath two blocks, or along several lines), not for one block.
const ROUND: f64 = 130.0;

/// The lines round the outside of what lies between two blocks of one row (out of the top of
/// both, or the bottom of both) or of one column (the left of both, or the right): up, across,
/// down. The outer run lies clear of every block between the two and of the words above or
/// under them (their whole frames), a little further out.
fn rounds(a: &Placed, b: &Placed, lay: &Layout) -> Vec<Cand> {
    let all = &lay.nodes;
    let (ra, rb) = (&a.art, &b.art);
    let mut v = Vec::new();
    // a block with its words: the picture, and the frame's part above or under it only where
    // words are written there (the frame is laid evenly about the picture's middle line)
    let whole = |n: &Placed| {
        let w = word_rects(n);
        (w.iter().map(|r| r.x).fold(n.art.x, f64::min), w.iter().map(|r| r.y).fold(n.art.y, f64::min), w.iter().map(|r| r.right()).fold(n.art.right(), f64::max), w.iter().map(|r| r.bottom()).fold(n.art.bottom(), f64::max))
    };
    let out = (ra.h.min(rb.h).min(ra.w.min(rb.w)) * 0.25).clamp(4.0, 10.0);
    // the frames that hold both blocks: the outer run stays inside the innermost of them when
    // it has the room there (a little inside its edge, still clear of the blocks), and no
    // line of this shape leaves the page
    let holds = |r: &Rect| r.contains(ra) && r.contains(rb);
    // (a frame's title is not room: where it is written along an edge, the frame ends at it)
    let less = |r: Rect, t: Option<Rect>| -> Rect {
        match t {
            Some(t) if t.w >= t.h && t.cy() < r.cy() && t.bottom() > r.y => Rect::new(r.x, t.bottom(), r.w, r.bottom() - t.bottom()),
            Some(t) if t.w >= t.h && t.cy() >= r.cy() && t.y < r.bottom() => Rect::new(r.x, r.y, r.w, t.y - r.y),
            _ => r,
        }
    };
    let frame: Option<Rect> = lay.regions.iter().map(|r| (r.rect, r.title_rect)).filter(|r| holds(&r.0)).chain(lay.panels.iter().filter(|p| !p.bare).map(|p| (p.rect, p.title_rect)).filter(|r| holds(&r.0))).min_by(|x, y| (x.0.w * x.0.h).partial_cmp(&(y.0.w * y.0.h)).unwrap()).map(|(r, t)| less(r, t));
    let keep = |want: f64, edge: Option<f64>, clear: f64, sign: f64| -> f64 {
        // `sign` -1: a run above or left (smaller is further out); +1: below or right
        match edge {
            Some(e) if (want - e) * sign > -2.0 && (clear - (e - sign * 2.0)) * sign < -2.0 => e - sign * 2.0,
            _ => want,
        }
    };
    let on_page = |c: &Cand| c.pts.iter().all(|p| p.0 >= lay.bounds.x && p.0 <= lay.bounds.right() && p.1 >= lay.bounds.y && p.1 <= lay.bounds.bottom());
    if ra.bottom().min(rb.bottom()) - ra.y.max(rb.y) > 1.0 && (rb.x >= ra.right() || rb.right() <= ra.x) {
        // one row: every block whose frame lies between the two middles and in the row's band
        let (x0, x1) = (ra.cx().min(rb.cx()), ra.cx().max(rb.cx()));
        let (y0, y1) = (ra.y.min(rb.y), ra.bottom().max(rb.bottom()));
        let between: Vec<(f64, f64, f64, f64)> = all.iter().map(whole).filter(|w| w.2 > x0 && w.0 < x1 && w.3 > y0 && w.1 < y1).collect();
        let (ct, cb) = (between.iter().map(|w| w.1).fold(y0, f64::min), between.iter().map(|w| w.3).fold(y1, f64::max));
        let top = keep(ct - out, frame.map(|f| f.y), ct, -1.0);
        let bottom = keep(cb + out, frame.map(|f| f.bottom()), cb, 1.0);
        for (side, y) in [(Side::Top, top), (Side::Bottom, bottom)] {
            let (p, q) = (mid(a, side), mid(b, side));
            v.push(Cand { pts: vec![p, (p.0, y), (q.0, y), q], from: side, to: side });
        }
    }
    if ra.right().min(rb.right()) - ra.x.max(rb.x) > 1.0 && (rb.y >= ra.bottom() || rb.bottom() <= ra.y) {
        let (y0, y1) = (ra.cy().min(rb.cy()), ra.cy().max(rb.cy()));
        let (x0, x1) = (ra.x.min(rb.x), ra.right().max(rb.right()));
        let between: Vec<(f64, f64, f64, f64)> = all.iter().map(whole).filter(|w| w.3 > y0 && w.1 < y1 && w.2 > x0 && w.0 < x1).collect();
        let (cl, cr) = (between.iter().map(|w| w.0).fold(x0, f64::min), between.iter().map(|w| w.2).fold(x1, f64::max));
        let left = keep(cl - out, frame.map(|f| f.x), cl, -1.0);
        let right = keep(cr + out, frame.map(|f| f.right()), cr, 1.0);
        for (side, x) in [(Side::Left, left), (Side::Right, right)] {
            let (p, q) = (mid(a, side), mid(b, side));
            v.push(Cand { pts: vec![p, (x, p.1), (x, q.1), q], from: side, to: side });
        }
    }
    v.retain(|c| on_page(c));
    v
}

/// The lines that leave `a` on side `s` and reach `b` on side `t`, built from those sides: an L
/// when the sides are square to each other, a step (its middle run at the shares of the gap in
/// [`MIDDLES`]) when they face the same way along one axis, and a bracket round the outside of
/// both blocks when they are the same side. Only those that keep the rule and leave and arrive
/// through their sides are lines of the three shapes (see [`goes_by_its_sides`]); the others
/// are what a stated side is drawn with when no line of the three shapes has it.
fn sided(a: &Placed, b: &Placed, s: Side, t: Side) -> Vec<Cand> {
    let (p, q) = (mid(a, s), mid(b, t));
    let across = |x: Side| matches!(x, Side::Left | Side::Right);
    let mut v = Vec::new();
    if across(s) != across(t) {
        let corner = if across(s) { (q.0, p.1) } else { (p.0, q.1) };
        v.push(Cand { pts: vec![p, corner, q], from: s, to: t });
    } else if t == s.opposite() {
        if (across(s) && (p.1 - q.1).abs() < 0.5) || (!across(s) && (p.0 - q.0).abs() < 0.5) {
            v.push(Cand { pts: vec![p, q], from: s, to: t });
        }
        for f in MIDDLES {
            let pts = if across(s) {
                let x = p.0 + (q.0 - p.0) * f;
                vec![p, (x, p.1), (x, q.1), q]
            } else {
                let y = p.1 + (q.1 - p.1) * f;
                vec![p, (p.0, y), (q.0, y), q]
            };
            v.push(Cand { pts, from: s, to: t });
        }
    } else {
        // the same side of both: round the outside of the two, half the shorter picture out
        let (ra, rb) = (&a.art, &b.art);
        let pts = match s {
            Side::Top => {
                let y = ra.y.min(rb.y) - ra.h.min(rb.h) / 2.0;
                vec![p, (p.0, y), (q.0, y), q]
            }
            Side::Bottom => {
                let y = ra.bottom().max(rb.bottom()) + ra.h.min(rb.h) / 2.0;
                vec![p, (p.0, y), (q.0, y), q]
            }
            Side::Left => {
                let x = ra.x.min(rb.x) - ra.w.min(rb.w) / 2.0;
                vec![p, (x, p.1), (x, q.1), q]
            }
            Side::Right => {
                let x = ra.right().max(rb.right()) + ra.w.min(rb.w) / 2.0;
                vec![p, (x, p.1), (x, q.1), q]
            }
        };
        v.push(Cand { pts, from: s, to: t });
    }
    v
}

/// Of a line's two ends, how many do not go by their sides: the first run goes out of the side
/// it leaves by, the last run into the side it arrives by.
fn ends_astray(c: &Cand) -> usize {
    let n = c.pts.len();
    if n < 2 {
        return 2;
    }
    let runs_along = |p: Pt, q: Pt, d: (f64, f64)| {
        let (dx, dy) = (q.0 - p.0, q.1 - p.1);
        dx * d.0 + dy * d.1 > 0.05 && (dx * d.1 - dy * d.0).abs() < 0.05
    };
    let (o, i) = (c.from.normal(), c.to.normal());
    !runs_along(c.pts[0], c.pts[1], o) as usize + !runs_along(c.pts[n - 2], c.pts[n - 1], (-i.0, -i.1)) as usize
}

/// A line leaves through the side it names and arrives through its other side, and keeps the rule.
fn goes_by_its_sides(c: &Cand) -> bool {
    let r = Route { edge: 0, points: c.pts.clone(), from_side: c.from, to_side: c.to, from_site: 0, to_site: 0, forced: false, warnings: Vec::new(), curve: false, branch: false };
    ends_astray(c) == 0 && keeps_the_rule(&r)
}

/// Two runs lie along each other (same line, more than a point in common).
fn along(a: (Pt, Pt), b: (Pt, Pt)) -> bool {
    let h = |s: (Pt, Pt)| (s.0.1 - s.1.1).abs() < 0.5;
    let w = |s: (Pt, Pt)| (s.0.0 - s.1.0).abs() < 0.5;
    let over = |a0: f64, a1: f64, b0: f64, b1: f64| a0.min(a1).max(b0.min(b1)) + 1.0 < a0.max(a1).min(b0.max(b1));
    (h(a) && h(b) && (a.0.1 - b.0.1).abs() < 1.5 && over(a.0.0, a.1.0, b.0.0, b.1.0)) || (w(a) && w(b) && (a.0.0 - b.0.0).abs() < 1.5 && over(a.0.1, a.1.1, b.0.1, b.1.1))
}

/// Two runs go opposite ways.
fn against(a: (Pt, Pt), b: (Pt, Pt)) -> bool {
    let d = |s: (Pt, Pt)| (s.1.0 - s.0.0, s.1.1 - s.0.1);
    let (u, v) = (d(a), d(b));
    u.0 * v.0 + u.1 * v.1 < 0.0
}

/// How many blocks other than its own a line passes (beneath).
pub fn beneath(g: &Graph, lay: &Layout, r: &Route) -> usize {
    let e = &g.edges[r.edge];
    let own = [g.node_index(&e.from), g.node_index(&e.to)];
    lay.nodes.iter().enumerate().filter(|(i, n)| !own.contains(&Some(*i)) && r.points.windows(2).any(|w| segment_hits(w[0], w[1], &n.art.inflate(-0.5)))).count()
}

/// How much of a line is seen: its length less the stretches beneath a block other than its
/// own two (the blocks are drawn over the lines) and the stretches along a line drawn before it
/// (the flow's own lines first, then in the order written, as [`lines`] draws them): drawn over
/// each other the two read as one, and what shows there is the earlier line. A line whose seen
/// length is no longer than its arrowhead is not drawn as far as the reader can tell; `build`
/// says so.
pub fn seen(g: &Graph, lay: &Layout, routes: &[Route], i: usize) -> f64 {
    let rank = |k: usize| (g.edges[routes[k].edge].kind != EdgeKind::Data, routes[k].edge);
    let r = &routes[i];
    let e = &g.edges[r.edge];
    let own = [g.node_index(&e.from), g.node_index(&e.to)];
    let mut total = 0.0;
    for w in r.points.windows(2) {
        let (p, q) = (w[0], w[1]);
        let flat = (p.1 - q.1).abs() < 0.5;
        let upright = (p.0 - q.0).abs() < 0.5;
        let len = (q.0 - p.0).abs() + (q.1 - p.1).abs();
        if !(flat ^ upright) {
            // a point, or a slanted line (forced): taken as seen
            total += len;
            continue;
        }
        // the run as an interval along its own axis, the covered stretches in the same terms
        let (lo, hi) = if flat { (p.0.min(q.0), p.0.max(q.0)) } else { (p.1.min(q.1), p.1.max(q.1)) };
        let mut cover: Vec<(f64, f64)> = Vec::new();
        for (k, n) in lay.nodes.iter().enumerate() {
            if own.contains(&Some(k)) || !segment_hits(p, q, &n.art.inflate(-0.5)) {
                continue;
            }
            let b = n.art;
            cover.push(if flat { (b.x, b.right()) } else { (b.y, b.bottom()) });
        }
        for (j, o) in routes.iter().enumerate() {
            if rank(j) >= rank(i) {
                continue;
            }
            for u in o.points.windows(2) {
                if along((p, q), (u[0], u[1])) {
                    cover.push(if flat { (u[0].0.min(u[1].0), u[0].0.max(u[1].0)) } else { (u[0].1.min(u[1].1), u[0].1.max(u[1].1)) });
                }
            }
        }
        let mut cover: Vec<(f64, f64)> = cover.into_iter().map(|(a, b)| (a.max(lo), b.min(hi))).filter(|(a, b)| b > a).collect();
        cover.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (mut hidden, mut reach) = (0.0, lo);
        for (a, b) in cover {
            let a = a.max(reach);
            if b > a {
                hidden += b - a;
                reach = b;
            }
        }
        total += (hi - lo) - hidden;
    }
    total
}

/// A line keeps the rule: at most two bends (straight, an L, a step, or round the outside).
pub fn keeps_the_rule(r: &Route) -> bool {
    let sign = |d: f64| if d.abs() < 0.05 { 0.0 } else { d.signum() };
    let runs: Vec<(f64, f64)> = r.points.windows(2).map(|w| (sign(w[1].0 - w[0].0), sign(w[1].1 - w[0].1))).filter(|d| *d != (0.0, 0.0)).collect();
    // with two bends the first and last runs lie along one axis: the same way (a step) or
    // opposite ways (round the outside of what lies between); a zigzag needs a third bend
    r.bends() <= 2 && runs.len() <= 3
}

/// The lines of a packed figure (see the module's words), the flow's own lines first.
pub fn lines(g: &Graph, lay: &Layout) -> Vec<Route> {
    let mut order: Vec<usize> = (0..g.edges.len()).collect();
    order.sort_by_key(|i| (g.edges[*i].kind != EdgeKind::Data, *i));
    let mut done: Vec<(usize, Route)> = Vec::new();
    for ei in order {
        let e = &g.edges[ei];
        let (Some(ia), Some(ib)) = (g.node_index(&e.from), g.node_index(&e.to)) else { continue };
        let (a, b) = (&lay.nodes[ia], &lay.nodes[ib]);
        if ia == ib {
            // a line from a block back to itself has no shape of the three (it turns back): it
            // is kept, of no length, and says so
            let p = mid(a, Side::Right);
            let name = format!("{} → {}", e.from, e.to);
            done.push((ei, Route { edge: ei, points: vec![p, p], from_side: Side::Right, to_side: Side::Right, from_site: site_of(Side::Right, 2), to_site: site_of(Side::Right, 2), forced: true, warnings: vec![format!("{name}: 線的兩端是同一個方塊，三種線形都畫不出")], curve: false, branch: false }));
            continue;
        }
        let words = |n: &Placed, s: Side| (s == Side::Top && n.words.0) || (s == Side::Bottom && n.words.1);
        let cost = |c: &Cand| -> f64 {
            let bends = c.pts.len() as f64 - 2.0;
            // a block of words only cannot hide a line: passing it counts as much as many pictures
            let under: f64 = lay.nodes.iter().enumerate().filter(|(i, n)| *i != ia && *i != ib && c.pts.windows(2).any(|w| segment_hits(w[0], w[1], &n.art.inflate(-0.5)))).map(|(i, _)| if g.nodes[i].module == "label" { 8.0 } else { 1.0 }).sum();
            // along a line of other blocks; a line that shares an end with this one may share
            // its run going the same way (lines fanning out of one block, or into one), never
            // back along it (it would be drawn over that line and not be seen)
            let shared = done
                .iter()
                .filter(|(o, r)| {
                    let x = &g.edges[*o];
                    let kin = !(x.from != e.from && x.to != e.to && x.from != e.to && x.to != e.from);
                    c.pts.windows(2).any(|w| r.points.windows(2).any(|q| along((w[0], w[1]), (q[0], q[1])) && (!kin || against((w[0], w[1]), (q[0], q[1])))))
                })
                .count() as f64;
            let wordy = words(a, c.from) as u8 as f64 + words(b, c.to) as u8 as f64;
            // round the outside: more than its bends
            let round = c.pts.len() == 4 && c.from == c.to;
            // nor through the words of another block, of any shape, nor through a title
            let hits = |r: &Rect| c.pts.windows(2).any(|w| segment_hits(w[0], w[1], &r.inflate(-0.5)));
            let over_words = lay.nodes.iter().enumerate().filter(|(i, n)| *i != ia && *i != ib && word_rects(n).iter().any(|r| hits(r))).count() as f64
                + lay.panels.iter().filter_map(|p| p.title_rect).chain(lay.regions.iter().filter_map(|r| r.title_rect)).filter(|r| hits(r)).count() as f64;
            let len: f64 = c.pts.windows(2).map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()).sum();
            bends * 100.0 + under * 250.0 + shared * 60.0 + wordy * 150.0 + len / 1000.0 + if round { ROUND } else { 0.0 } + over_words * 400.0
        };
        // a side the one who draws states (`from=` / `to=`): only lines that leave and arrive
        // through it; when no line of the three shapes does, the stated side is kept, drawn
        // with no more bends than two, and the line says so
        let name = format!("{} → {}", e.from, e.to);
        let mut notes: Vec<String> = Vec::new();
        let stated = |w: &str, notes: &mut Vec<String>| -> Option<Side> {
            if w.is_empty() {
                return None;
            }
            let s = crate::route::side_named(w);
            if s.is_none() {
                notes.push(format!("{name}: 不認得的邊 {w}，已忽略"));
            }
            s
        };
        let (mut fs, mut ts) = (stated(&e.from_side, &mut notes), stated(&e.to_side, &mut notes));
        // the lines from a heading to what it is about (`gather`) all leave by the side that
        // faces those things together and arrive by the side facing back: one trunk, a fork
        if e.kind == EdgeKind::Gather && fs.is_none() && ts.is_none() {
            let them: Vec<Rect> = g.edges.iter().filter(|x| x.kind == EdgeKind::Gather && x.from == e.from).filter_map(|x| g.node_index(&x.to)).map(|i| lay.nodes[i].art).collect();
            let all = them.iter().skip(1).fold(them[0], |u, r| u.union(r));
            let s = if all.y >= a.art.bottom() { Some(Side::Bottom) } else if all.bottom() <= a.art.y { Some(Side::Top) } else if all.x >= a.art.right() { Some(Side::Right) } else if all.right() <= a.art.x { Some(Side::Left) } else { None };
            if let Some(s) = s {
                fs = Some(s);
                ts = Some(s.opposite());
            }
        }
        let cands: Vec<Cand> = if fs.is_none() && ts.is_none() {
            let mut v = shapes(a, b);
            v.extend(rounds(a, b, lay));
            v
        } else {
            let froms: Vec<Side> = fs.map(|s| vec![s]).unwrap_or(Side::ALL.to_vec());
            let tos: Vec<Side> = ts.map(|s| vec![s]).unwrap_or(Side::ALL.to_vec());
            let fits = |c: &Cand| froms.contains(&c.from) && tos.contains(&c.to);
            let mut built: Vec<Cand> = Vec::new();
            for &s in &froms {
                for &t in &tos {
                    built.extend(sided(a, b, s, t));
                }
            }
            let good: Vec<Cand> = shapes(a, b).into_iter().filter(|c| fits(c)).chain(built.iter().filter(|c| goes_by_its_sides(c)).map(|c| Cand { pts: c.pts.clone(), from: c.from, to: c.to })).collect();
            if good.is_empty() {
                let said = [("from", &e.from_side), ("to", &e.to_side)].iter().filter(|(_, w)| !w.is_empty()).map(|(k, w)| format!("{k}={w}")).collect::<Vec<_>>().join(" ");
                notes.push(format!("{name}: 照所說的邊（{said}）畫不出三種線形之一，保留所說的邊"));
                // of the lines built from the stated sides, those that still go out of and into
                // their sides (only the rule is given up); when none does (sides that face away
                // from each other need more than two bends), those that go astray at fewer ends
                let least = built.iter().map(ends_astray).min().unwrap_or(0);
                built.into_iter().filter(|c| ends_astray(c) == least).collect()
            } else {
                good
            }
        };
        let best = cands.into_iter().min_by(|x, y| cost(x).partial_cmp(&cost(y)).unwrap());
        // a line that bends and ends in the middle of a side where a line already ends the other
        // way (one leaves there, this one arrives) ends a quarter of the side further along
        // instead, on the side it comes from, so the two are told apart
        let best = best.map(|mut c| {
            if c.pts.len() > 2 {
                let n = c.pts.len();
                for (at, node, side, arriving) in [(0usize, ia, c.from, false), (n - 1, ib, c.to, true)] {
                    let taken = done.iter().any(|(o, r)| {
                        let x = &g.edges[*o];
                        (arriving && g.node_index(&x.from) == Some(node) && r.from_side == side) || (!arriving && g.node_index(&x.to) == Some(node) && r.to_side == side)
                    });
                    if !taken {
                        continue;
                    }
                    let next = if at == 0 { c.pts[1] } else { c.pts[n - 2] };
                    let here = c.pts[at];
                    // the end's own run is square to its side; the run after it says which way to move
                    let far = if at == 0 { c.pts.get(2).copied().unwrap_or(next) } else { c.pts[n.saturating_sub(3)] };
                    let nd = &lay.nodes[node];
                    let moved = match side {
                        Side::Left | Side::Right => {
                            let slot = if far.1 > here.1 { 3 } else { 1 };
                            let p = site_point(nd, side, slot);
                            // only when the middle run is on the same side of the new end as before
                            ((far.1 - p.1) * (far.1 - here.1) > 0.0).then_some((p, (next.0, p.1)))
                        }
                        Side::Top | Side::Bottom => {
                            let slot = if far.0 > here.0 { 3 } else { 1 };
                            let p = site_point(nd, side, slot);
                            ((far.0 - p.0) * (far.0 - here.0) > 0.0).then_some((p, (p.0, next.1)))
                        }
                    };
                    if let Some((p, corner)) = moved.filter(|_| n == 4) {
                        c.pts[at] = p;
                        if at == 0 { c.pts[1] = corner } else { c.pts[n - 2] = corner }
                    }
                }
            }
            c
        });
        // an end on a side where its own block's words are written lies at the far edge of
        // those words (the line starts under a caption, not across it), while its run reaches
        let best = best.map(|mut c| {
            let n = c.pts.len();
            for (at, nd, side) in [(0usize, a, c.from), (n - 1, b, c.to)] {
                if !words(nd, side) {
                    continue;
                }
                let edge = if side == Side::Top { nd.rect.y } else { nd.rect.bottom() };
                let next = if at == 0 { c.pts[1] } else { c.pts[n - 2] };
                let here = c.pts[at];
                if (next.0 - here.0).abs() < 0.05 && (next.1 - edge) * (edge - here.1) > 0.0 {
                    c.pts[at].1 = edge;
                }
            }
            c
        });
        let r = match best {
            Some(c) => Route { edge: ei, points: c.pts, from_side: c.from, to_side: c.to, from_site: site_of(c.from, 2), to_site: site_of(c.to, 2), forced: false, warnings: notes, curve: e.curve, branch: false },
            // the two pictures lie over each other: nothing to draw but the join of their middles
            None => Route { edge: ei, points: vec![(a.art.cx(), a.art.cy()), (b.art.cx(), b.art.cy())], from_side: Side::Right, to_side: Side::Left, from_site: site_of(Side::Right, 2), to_site: site_of(Side::Left, 2), forced: true, warnings: notes, curve: false, branch: false },
        };
        done.push((ei, r));
    }
    done.sort_by_key(|d| d.0);
    done.into_iter().map(|d| d.1).collect()
}
