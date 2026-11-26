//! Lines on a packed page, by construction of three shapes only: straight, one bend (an L), or
//! two bends going on in the same direction (a step). No line has more than two bends and none
//! goes back the way an earlier run of it came; what may be chosen is the side a line leaves and
//! arrives at and, for a step, where its middle run lies. A line whose blocks stand in one row
//! or one column is straight even with a block between them: it passes beneath that block (the
//! blocks are drawn over the lines) and its arrowhead shows. Among the shapes that exist for a
//! line the cheapest is taken: fewest bends, then fewest blocks passed beneath, then not along
//! a line already drawn, then not through a block's words, then the shortest.

use crate::layout::{Layout, Placed};
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
    // straight
    if let Some((s, t)) = across.filter(|_| (a.port_y - b.port_y).abs() < 0.5) {
        v.push(Cand { pts: vec![mid(a, s), mid(b, t)], from: s, to: t });
    }
    if let Some((s, t)) = down.filter(|_| (ra.cx() - rb.cx()).abs() < 0.5) {
        v.push(Cand { pts: vec![mid(a, s), mid(b, t)], from: s, to: t });
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
    if let Some((s, t)) = across.filter(|_| (a.port_y - b.port_y).abs() >= 0.5) {
        let (p, q) = (mid(a, s), mid(b, t));
        for f in MIDDLES {
            let x = p.0 + (q.0 - p.0) * f;
            v.push(Cand { pts: vec![p, (x, p.1), (x, q.1), q], from: s, to: t });
        }
    }
    if let Some((s, t)) = down.filter(|_| (ra.cx() - rb.cx()).abs() >= 0.5) {
        let (p, q) = (mid(a, s), mid(b, t));
        for f in MIDDLES {
            let y = p.1 + (q.1 - p.1) * f;
            v.push(Cand { pts: vec![p, (p.0, y), (q.0, y), q], from: s, to: t });
        }
    }
    v
}

/// Two runs lie along each other (same line, more than a point in common).
fn along(a: (Pt, Pt), b: (Pt, Pt)) -> bool {
    let h = |s: (Pt, Pt)| (s.0.1 - s.1.1).abs() < 0.5;
    let w = |s: (Pt, Pt)| (s.0.0 - s.1.0).abs() < 0.5;
    let over = |a0: f64, a1: f64, b0: f64, b1: f64| a0.min(a1).max(b0.min(b1)) + 1.0 < a0.max(a1).min(b0.max(b1));
    (h(a) && h(b) && (a.0.1 - b.0.1).abs() < 1.5 && over(a.0.0, a.1.0, b.0.0, b.1.0)) || (w(a) && w(b) && (a.0.0 - b.0.0).abs() < 1.5 && over(a.0.1, a.1.1, b.0.1, b.1.1))
}

/// How many blocks other than its own a line passes (beneath).
pub fn beneath(g: &Graph, lay: &Layout, r: &Route) -> usize {
    let e = &g.edges[r.edge];
    let own = [g.node_index(&e.from), g.node_index(&e.to)];
    lay.nodes.iter().enumerate().filter(|(i, n)| !own.contains(&Some(*i)) && r.points.windows(2).any(|w| segment_hits(w[0], w[1], &n.art.inflate(-0.5)))).count()
}

/// A line keeps the rule: at most two bends and no run back the way an earlier run came.
pub fn keeps_the_rule(r: &Route) -> bool {
    let sign = |d: f64| if d.abs() < 0.05 { 0.0 } else { d.signum() };
    let runs: Vec<(f64, f64)> = r.points.windows(2).map(|w| (sign(w[1].0 - w[0].0), sign(w[1].1 - w[0].1))).filter(|d| *d != (0.0, 0.0)).collect();
    let back = runs.iter().enumerate().any(|(i, a)| runs[..i].iter().any(|b| (a.0 != 0.0 && a.0 == -b.0) || (a.1 != 0.0 && a.1 == -b.1)));
    r.bends() <= 2 && !back
}

/// The lines of a packed figure (see the module's words), the flow's own lines first.
pub fn lines(g: &Graph, lay: &Layout) -> Vec<Route> {
    let mut order: Vec<usize> = (0..g.edges.len()).collect();
    order.sort_by_key(|i| (g.edges[*i].kind != EdgeKind::Data, *i));
    let mut done: Vec<(usize, Route)> = Vec::new();
    for ei in order {
        let e = &g.edges[ei];
        let (Some(ia), Some(ib)) = (g.node_index(&e.from), g.node_index(&e.to)) else { continue };
        if ia == ib {
            continue;
        }
        let (a, b) = (&lay.nodes[ia], &lay.nodes[ib]);
        let words = |n: &Placed, s: Side| (s == Side::Top && n.words.0) || (s == Side::Bottom && n.words.1);
        let cost = |c: &Cand| -> f64 {
            let bends = c.pts.len() as f64 - 2.0;
            // a block of words only cannot hide a line: passing it counts as much as many pictures
            let under: f64 = lay.nodes.iter().enumerate().filter(|(i, n)| *i != ia && *i != ib && c.pts.windows(2).any(|w| segment_hits(w[0], w[1], &n.art.inflate(-0.5)))).map(|(i, _)| if g.nodes[i].module == "label" { 8.0 } else { 1.0 }).sum();
            // along a line of other blocks (a line that shares an end with this one may share its run)
            let shared = done
                .iter()
                .filter(|(o, _)| {
                    let x = &g.edges[*o];
                    x.from != e.from && x.to != e.to && x.from != e.to && x.to != e.from
                })
                .filter(|(_, r)| c.pts.windows(2).any(|w| r.points.windows(2).any(|q| along((w[0], w[1]), (q[0], q[1])))))
                .count() as f64;
            let wordy = words(a, c.from) as u8 as f64 + words(b, c.to) as u8 as f64;
            let len: f64 = c.pts.windows(2).map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()).sum();
            bends * 100.0 + under * 250.0 + shared * 60.0 + wordy * 150.0 + len / 1000.0
        };
        let best = shapes(a, b).into_iter().min_by(|x, y| cost(x).partial_cmp(&cost(y)).unwrap());
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
        let r = match best {
            Some(c) => Route { edge: ei, points: c.pts, from_side: c.from, to_side: c.to, from_site: site_of(c.from, 2), to_site: site_of(c.to, 2), forced: false, warnings: Vec::new(), curve: e.curve, branch: false },
            // the two pictures lie over each other: nothing to draw but the join of their middles
            None => Route { edge: ei, points: vec![(a.art.cx(), a.art.cy()), (b.art.cx(), b.art.cy())], from_side: Side::Right, to_side: Side::Left, from_site: site_of(Side::Right, 2), to_site: site_of(Side::Left, 2), forced: true, warnings: Vec::new(), curve: false, branch: false },
        };
        done.push((ei, r));
    }
    done.sort_by_key(|d| d.0);
    done.into_iter().map(|d| d.1).collect()
}
