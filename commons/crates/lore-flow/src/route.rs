//! L2, part two — the lines. Orthogonal routes on a sparse grid (A*):
//! no segment passes through a module, a panel title or the legend; feedback
//! leaves and enters at the bottom and runs under the row; lines between
//! panels prefer the gaps (crossing a panel border costs); lines that share
//! neither end do not run on top of each other.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

use crate::figure::{EdgeEnds, SITES, edge_key};
use crate::layout::{Layout, Placed, Rect};
use crate::spec::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    Top,
    Left,
    Bottom,
    Right,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Top, Side::Left, Side::Bottom, Side::Right];
    /// Outward unit vector.
    pub fn normal(self) -> (f64, f64) {
        match self {
            Side::Top => (0.0, -1.0),
            Side::Left => (-1.0, 0.0),
            Side::Bottom => (0.0, 1.0),
            Side::Right => (1.0, 0.0),
        }
    }
    /// Direction index (0 right, 1 down, 2 left, 3 up) of leaving through this side.
    fn dir(self) -> usize {
        match self {
            Side::Right => 0,
            Side::Bottom => 1,
            Side::Left => 2,
            Side::Top => 3,
        }
    }
    pub fn opposite(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
    /// Mid-point of this side of a rect.
    pub fn port(self, r: &Rect) -> (f64, f64) {
        match self {
            Side::Top => (r.cx(), r.y),
            Side::Left => (r.x, r.cy()),
            Side::Bottom => (r.cx(), r.bottom()),
            Side::Right => (r.right(), r.cy()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// Index into the graph's edges.
    pub edge: usize,
    /// Corner points from the source port to the target port.
    pub points: Vec<(f64, f64)>,
    pub from_side: Side,
    pub to_side: Side,
    /// The connection point (0..16, numbered as `figure::SITES`) each end uses. For a module
    /// that is an ellipse these are the middle of the side (the ellipse's own sites are chosen
    /// in `build`).
    pub from_site: u8,
    pub to_site: u8,
    /// No clear path was found; the line is drawn straight.
    pub forced: bool,
    /// What went wrong with the author's choices for this line (a point or a turn that could
    /// not be honoured); `build` passes them on as warnings.
    pub warnings: Vec<String>,
}

impl Route {
    pub fn bends(&self) -> usize {
        self.points.len().saturating_sub(2)
    }
    /// Middle of the longest segment and whether that segment is horizontal.
    pub fn label_anchor(&self) -> ((f64, f64), bool) {
        let mut best = (0.0, ((0.0, 0.0), true));
        for w in self.points.windows(2) {
            let len = (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs();
            if len > best.0 + 1e-9 {
                best = (len, (((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0), (w[0].1 - w[1].1).abs() < 1e-6));
            }
        }
        best.1
    }
}

/// Does the open segment a–b pass through the inside of `r`?
pub fn segment_hits(a: (f64, f64), b: (f64, f64), r: &Rect) -> bool {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    let e = 1e-6;
    if (a.1 - b.1).abs() < e {
        a.1 > r.y + e && a.1 < r.bottom() - e && x0 < r.right() - e && x1 > r.x + e
    } else if (a.0 - b.0).abs() < e {
        a.0 > r.x + e && a.0 < r.right() - e && y0 < r.bottom() - e && y1 > r.y + e
    } else {
        // not axis-aligned (a forced straight line): bounding boxes
        x0 < r.right() - e && x1 > r.x + e && y0 < r.bottom() - e && y1 > r.y + e
    }
}

#[derive(Clone, Copy, PartialEq)]
struct State {
    f: f64,
    g: f64,
    i: usize,
    j: usize,
    d: usize,
}
impl Eq for State {}
impl Ord for State {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal).then(o.g.partial_cmp(&self.g).unwrap_or(Ordering::Equal)).then((o.i, o.j, o.d).cmp(&(self.i, self.j, self.d)))
    }
}
impl PartialOrd for State {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// Lines that hug an obstacle cost a little more than free tracks.
    xw: Vec<f64>,
    yw: Vec<f64>,
    /// Horizontal edge (i, j) → (i+1, j): index j * (nx-1) + i.
    h_block: Vec<bool>,
    v_block: Vec<bool>,
    h_cost: Vec<f64>,
    v_cost: Vec<f64>,
    /// Who runs along each grid edge: (from node, to node).
    h_use: Vec<Vec<(usize, usize)>>,
    v_use: Vec<Vec<(usize, usize)>>,
    /// Routes passing straight through a grid point, horizontally / vertically.
    h_through: Vec<u16>,
    v_through: Vec<u16>,
}

/// Grid lines of one axis from (position, weight, fixed) candidates: fixed
/// lines (ports, the tracks beside modules) are always kept, the others only
/// when no line is within `eps`; wide gaps get a free track in the middle.
fn axis(mut lines: Vec<(f64, f64, bool)>, eps: f64) -> (Vec<f64>, Vec<f64>) {
    lines.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.partial_cmp(&b.0).unwrap()).then(a.1.partial_cmp(&b.1).unwrap()));
    let mut all: Vec<(f64, f64)> = Vec::new();
    for (v, w, fixed) in lines {
        let near = all.iter_mut().find(|(m, _)| (*m - v).abs() < if fixed { 1e-6 } else { eps });
        match near {
            Some(m) if fixed => m.1 = m.1.min(w),
            Some(_) => {}
            None => all.push((v, w)),
        }
    }
    all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut mids = Vec::new();
    for w in all.windows(2) {
        if w[1].0 - w[0].0 >= 4.0 * eps {
            mids.push(((w[0].0 + w[1].0) / 2.0, 1.0));
        }
    }
    all.extend(mids);
    all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    (all.iter().map(|a| a.0).collect(), all.iter().map(|a| a.1).collect())
}

impl Grid {
    fn nx(&self) -> usize {
        self.xs.len()
    }
    fn ny(&self) -> usize {
        self.ys.len()
    }
    fn hi(&self, i: usize, j: usize) -> usize {
        j * (self.nx() - 1) + i
    }
    fn vi(&self, i: usize, j: usize) -> usize {
        i * (self.ny() - 1) + j
    }
    fn pi(&self, i: usize, j: usize) -> usize {
        j * self.nx() + i
    }
    fn find(v: &[f64], x: f64) -> Option<usize> {
        let k = v.partition_point(|a| *a < x - 1e-6);
        (k < v.len() && (v[k] - x).abs() < 1e-5).then_some(k)
    }
}

/// Which sides a line may use and what each costs.
fn side_cost(kind: EdgeKind, is_source: bool, side: Side, me: &Rect, other: &Rect, down: bool, same_panel: bool, is_op: bool, k: f64) -> f64 {
    let (dx, dy) = (other.cx() - me.cx(), other.cy() - me.cy());
    let n = side.normal();
    let facing = n.0 * dx + n.1 * dy;
    let major = if dx.abs() >= dy.abs() { n.0 != 0.0 } else { n.1 != 0.0 };
    let by_position = if facing > 1e-6 && major {
        0.0
    } else if facing > 1e-6 {
        8.0 * k
    } else if facing.abs() <= 1e-6 {
        24.0 * k
    } else {
        45.0 * k
    };
    let (out_side, in_side) = if down { (Side::Bottom, Side::Top) } else { (Side::Right, Side::Left) };
    match kind {
        EdgeKind::Feedback => {
            // under the row (beside it in a top-down panel)
            let want = if down { Side::Right } else { Side::Bottom };
            if side == want { 0.0 } else { 80.0 * k }
        }
        EdgeKind::Skip => {
            if side == Side::Bottom || side == Side::Top && !down || down && (side == Side::Left || side == Side::Right) {
                0.0
            } else {
                12.0 * k
            }
        }
        _ if kind.is_flow() && same_panel => {
            let ahead = if down { dy } else { dx } * if is_source { 1.0 } else { -1.0 } > 1e-6;
            if !ahead {
                return by_position;
            }
            let want = if is_source { out_side } else { in_side };
            if side == want {
                0.0
            } else if side == want.opposite() {
                55.0 * k
            } else if facing > 1e-6 {
                if is_op { 3.0 * k } else { 22.0 * k }
            } else {
                40.0 * k
            }
        }
        _ => by_position,
    }
}

// ── connection points ────────────────────────────────────────────────────

/// How a line's end is tied to its block while routing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum End {
    /// The router picks the side; the point is the middle.
    Free,
    /// The author named a connection point; the side follows from it (a corner lies on two
    /// sides and the router takes the one it leaves more cheaply).
    Site(u8),
    /// Side and slot already decided (slot 0 corner, 1 quarter, 2 middle, 3 three quarters,
    /// 4 corner, counted left to right on top and bottom and top to bottom on left and right).
    At(Side, usize),
}

/// Site numbers (`figure::SITES`) along each side, in the order of [`Side::ALL`]
/// (top, left, bottom, right) and by slot.
const SIDE_SITES: [[u8; 5]; 4] = [[4, 5, 0, 6, 7], [4, 15, 1, 14, 13], [13, 12, 2, 11, 10], [7, 8, 3, 9, 10]];

fn side_ix(s: Side) -> usize {
    match s {
        Side::Top => 0,
        Side::Left => 1,
        Side::Bottom => 2,
        Side::Right => 3,
    }
}

/// The connection point (0..16) at `slot` (0..5) of a side.
pub fn site_of(side: Side, slot: usize) -> u8 {
    SIDE_SITES[side_ix(side)][slot.min(4)]
}

/// The sides and slots a connection point lies on: one for a side point, two for a corner.
pub fn sides_of(site: u8) -> Vec<(Side, usize)> {
    let mut v = Vec::new();
    for s in Side::ALL {
        for (slot, x) in SIDE_SITES[side_ix(s)].iter().enumerate() {
            if *x == site {
                v.push((s, slot));
            }
        }
    }
    v
}

/// Where a connection point of a block is (the body's centre line on the left and right).
pub fn site_point(n: &Placed, side: Side, slot: usize) -> (f64, f64) {
    let f = slot.min(4) as f64 * 0.25;
    let r = &n.rect;
    match side {
        Side::Top => (r.x + r.w * f, r.y),
        Side::Bottom => (r.x + r.w * f, r.bottom()),
        Side::Left => (r.x, if slot == 2 { n.port_y } else { r.y + r.h * f }),
        Side::Right => (r.right(), if slot == 2 { n.port_y } else { r.y + r.h * f }),
    }
}

/// The key of every edge (`figure::edge_key`: the nth edge between the same two nodes).
pub fn edge_keys(g: &Graph) -> Vec<String> {
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    g.edges
        .iter()
        .map(|e| {
            let n = seen.entry((e.from.as_str(), e.to.as_str())).or_insert(0);
            *n += 1;
            edge_key(&e.from, &e.to, *n)
        })
        .collect()
}

/// Slots (1 quarter, 2 middle, 3 three quarters) for `n` lines that share one side, in the
/// order of their other ends; `taken` are the slots the author fixed. One line takes the
/// middle, two take the quarters, three or more use all three, the extra ones sharing a
/// point with their neighbours (earlier points get the extra line).
fn spread(n: usize, taken: &[usize]) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let mut free: Vec<usize> = (1..=3).filter(|s| !taken.contains(s)).collect();
    if free.is_empty() {
        free = vec![1, 2, 3];
    }
    let pattern: &[usize] = match n {
        1 => &[2],
        2 => &[1, 3],
        _ => &[1, 2, 3],
    };
    let base: Vec<usize> = if pattern.iter().all(|s| free.contains(s)) { pattern.to_vec() } else { free };
    let l = n.min(base.len());
    let mut out = Vec::new();
    for (i, slot) in base.iter().take(l).enumerate() {
        let size = n / l + usize::from(i < n % l);
        out.extend(std::iter::repeat_n(*slot, size));
    }
    out
}

/// Route every edge of the graph over a finished layout. The author's choices (see
/// [`route_with`]) are the ones the layout carries on its nodes.
pub fn route(g: &Graph, lay: &Layout) -> Vec<Route> {
    let mut ends = BTreeMap::new();
    for n in &lay.nodes {
        for (k, v) in &n.ends {
            ends.insert(k.clone(), v.clone());
        }
    }
    route_with(g, lay, &ends)
}

/// As [`route`] with the author's choices given: by [`edge_keys`], the connection point of
/// each end and where the line turns (`turns` in the figure's natural size, mapped through
/// the layout's origin and scale). The sides are chosen as without choices; the point on a
/// side is then assigned so lines that share a side of a block use different points, and the
/// lines are routed again from those points.
pub fn route_with(g: &Graph, lay: &Layout, ends: &BTreeMap<String, EdgeEnds>) -> Vec<Route> {
    let keys = edge_keys(g);
    let ne = g.edges.len();
    let mut notes: Vec<Vec<String>> = vec![Vec::new(); ne];
    let mut fixed1 = vec![[End::Free; 2]; ne];
    let name = |ei: usize| format!("{} → {}", g.edges[ei].from, g.edges[ei].to);
    for (ei, key) in keys.iter().enumerate() {
        let Some(c) = ends.get(key) else { continue };
        for (i, v) in [c.from, c.to].into_iter().enumerate() {
            match v {
                Some(s) if (s as usize) < SITES => fixed1[ei][i] = End::Site(s),
                Some(s) => notes[ei].push(format!("{}: 連接點 {s} 不存在，已忽略", name(ei))),
                None => {}
            }
        }
    }
    let first = run(g, lay, &fixed1, None);

    // the point on each side: lines sharing a side of a block, in the order of their other ends
    struct Use {
        edge: usize,
        end: usize,
        key: f64,
        slot: Option<usize>,
    }
    let mut groups: BTreeMap<(usize, Side), Vec<Use>> = BTreeMap::new();
    let mut fixed2 = vec![[End::Free; 2]; ne];
    for (ei, r) in first.iter().enumerate() {
        if r.forced {
            continue;
        }
        let e = &g.edges[ei];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        for (end, node, other, side, site) in [(0, a, b, r.from_side, r.from_site), (1, b, a, r.to_side, r.to_site)] {
            let n = &lay.nodes[node];
            if n.ellipse {
                fixed2[ei][end] = End::At(side, 2);
                continue;
            }
            let o = &lay.nodes[other].rect;
            let key = if matches!(side, Side::Top | Side::Bottom) { o.cx() } else { o.cy() };
            let slot = match fixed1[ei][end] {
                End::Site(_) => sides_of(site).into_iter().find(|(s, _)| *s == side).map(|(_, sl)| sl),
                _ => None,
            };
            groups.entry((node, side)).or_default().push(Use { edge: ei, end, key, slot });
        }
    }
    let mut moved = false;
    for ((_, side), mut uses) in groups {
        let taken: Vec<usize> = uses.iter().filter_map(|u| u.slot).collect();
        for u in uses.iter().filter(|u| u.slot.is_some()) {
            fixed2[u.edge][u.end] = End::At(side, u.slot.unwrap());
        }
        uses.retain(|u| u.slot.is_none());
        uses.sort_by(|x, y| x.key.partial_cmp(&y.key).unwrap_or(Ordering::Equal).then((x.edge, x.end).cmp(&(y.edge, y.end))));
        for (u, slot) in uses.iter().zip(spread(uses.len(), &taken)) {
            fixed2[u.edge][u.end] = End::At(side, slot);
            moved |= slot != 2;
        }
    }
    let mut out = if moved { run(g, lay, &fixed2, Some(&first)) } else { first };

    // where the author wants the line to turn
    for (ei, key) in keys.iter().enumerate() {
        let Some(c) = ends.get(key).filter(|c| !c.turns.is_empty()) else { continue };
        let e = &g.edges[ei];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        match (!out[ei].forced).then(|| turned(lay, a, b, &out[ei], &c.turns)).flatten() {
            Some(pts) => out[ei].points = pts,
            None => notes[ei].push(format!("{}: 轉彎的位置無法照辦，已忽略", name(ei))),
        }
    }
    for (r, n) in out.iter_mut().zip(notes) {
        r.warnings.extend(n);
    }
    out
}

/// One run of a routed line the author may slide: an interior run, between the run that leaves
/// the first block and the run that arrives at the second.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// Its place in [`EdgeEnds::turns`].
    pub index: usize,
    /// A vertical run is slid sideways (its value is an x), a horizontal one up and down (a y).
    pub vertical: bool,
    /// Where it is now, as `turns` writes it: pt at the figure's natural size.
    pub value: f64,
    /// Its two ends, in the layout's coordinates (as [`Route::points`]).
    pub a: (f64, f64),
    pub b: (f64, f64),
}

/// The runs of `r` the author may slide, in order (`a` = the layout index of the line's first
/// block). Writing every `value`, in order, as the line's `turns` gives back the same points;
/// the editor then changes one of them. Empty when the line has no interior run (straight, or
/// one corner), when it is `forced`, or when its runs do not fit what [`turned`] accepts.
pub fn runs(lay: &Layout, a: usize, r: &Route) -> Vec<Run> {
    let _ = (lay, a, r);
    Vec::new()
}

/// The route with its interior runs moved to the author's coordinates (`turns`: the x of a
/// vertical run or the y of a horizontal run, in order, natural size). The first run leaves
/// the first block's side and the last arrives at the second block's side; so the number of
/// runs must fit the two sides (a line that leaves and arrives horizontally has an odd number
/// of runs). `None` when it does not, when a run would go back into a block, or when the path
/// would cut through another block.
fn turned(lay: &Layout, a: usize, b: usize, r: &Route, turns: &[f64]) -> Option<Vec<(f64, f64)>> {
    let k = lay.scale;
    let (ox, oy) = lay.nodes[a].origin;
    let (p, q) = (r.points[0], *r.points.last().unwrap());
    let n = turns.len();
    let horizontal = |s: Side| matches!(s, Side::Left | Side::Right);
    let orient: Vec<bool> = (0..n + 2).map(|j| horizontal(r.from_side) ^ (j % 2 == 1)).collect();
    if orient[n + 1] != horizontal(r.to_side) {
        return None;
    }
    let mut coord = vec![if orient[0] { p.1 } else { p.0 }];
    for (i, t) in turns.iter().enumerate() {
        coord.push(if orient[i + 1] { oy + t * k } else { ox + t * k });
    }
    coord.push(if orient[n + 1] { q.1 } else { q.0 });
    let mut pts = vec![p];
    for j in 1..=n + 1 {
        pts.push(if orient[j - 1] { (coord[j], coord[j - 1]) } else { (coord[j - 1], coord[j]) });
    }
    pts.push(q);
    let (fnrm, tnrm) = (r.from_side.normal(), r.to_side.normal());
    let (d0, d1) = ((pts[1].0 - p.0, pts[1].1 - p.1), (q.0 - pts[pts.len() - 2].0, q.1 - pts[pts.len() - 2].1));
    if d0.0 * fnrm.0 + d0.1 * fnrm.1 <= 1e-6 || d1.0 * tnrm.0 + d1.1 * tnrm.1 >= -1e-6 {
        return None;
    }
    let last = pts.len() - 2;
    for (si, w) in pts.windows(2).enumerate() {
        for (ni, nd) in lay.nodes.iter().enumerate() {
            if (ni == a && si == 0) || (ni == b && si == last) {
                continue;
            }
            if segment_hits(w[0], w[1], &nd.reserve) {
                return None;
            }
        }
    }
    Some(simplify(pts))
}

/// One routing pass. `fixed` says, per edge, how each end is tied to its block; `fallback` is an
/// earlier pass whose route is kept for an edge this pass cannot route.
fn run(g: &Graph, lay: &Layout, fixed: &[[End; 2]], fallback: Option<&[Route]>) -> Vec<Route> {
    let k = lay.scale;
    let m = 7.0 * k; // distance of a line from a module
    let c = 3.0 * k; // clearance that counts as "through"
    let bend = 18.0 * k;
    let obstacles: Vec<Rect> = lay.nodes.iter().map(|n| n.reserve.inflate(c)).collect();
    let mut walls: Vec<Rect> = obstacles.clone();
    walls.extend(lay.panels.iter().filter_map(|p| p.title_rect));
    if !lay.legend.is_empty() {
        let top = lay.legend.iter().map(|l| l.text.y).fold(f64::MAX, f64::min);
        walls.push(Rect::new(lay.bounds.x, top, lay.bounds.w, lay.bounds.bottom() - top));
    }

    // grid lines: (position, cost per length, fixed)
    const HUG: f64 = 1.06; // right beside a module
    const GAP: f64 = 0.9; // the middle of a gap between panels
    let (mut lx, mut ly): (Vec<(f64, f64, bool)>, Vec<(f64, f64, bool)>) = (Vec::new(), Vec::new());
    for n in &lay.nodes {
        lx.push((n.rect.cx(), 1.0, true));
        ly.push((n.port_y, 1.0, true));
        lx.extend([(n.reserve.x - m, HUG, true), (n.reserve.right() + m, HUG, true)]);
        ly.extend([(n.reserve.y - m, HUG, true), (n.reserve.bottom() + m, HUG, true)]);
    }
    for w in &walls[obstacles.len()..] {
        lx.extend([(w.x - m + c, HUG, false), (w.right() + m - c, HUG, false)]);
        ly.extend([(w.y - m + c, HUG, false), (w.bottom() + m - c, HUG, false)]);
    }
    for (pi, p) in lay.panels.iter().enumerate() {
        // gaps between this panel and its neighbours, and a channel inside each border
        for q in &lay.panels[pi + 1..] {
            if q.rect.x >= p.rect.right() - 1e-6 {
                lx.push(((p.rect.right() + q.rect.x) / 2.0, GAP, true));
            } else if p.rect.x >= q.rect.right() - 1e-6 {
                lx.push(((q.rect.right() + p.rect.x) / 2.0, GAP, true));
            }
            if q.rect.y >= p.rect.bottom() - 1e-6 {
                ly.push(((p.rect.bottom() + q.rect.y) / 2.0, GAP, true));
            } else if p.rect.y >= q.rect.bottom() - 1e-6 {
                ly.push(((q.rect.bottom() + p.rect.y) / 2.0, GAP, true));
            }
        }
        if !p.bare {
            lx.extend([(p.rect.x + 5.0 * k, 1.0, false), (p.rect.right() - 5.0 * k, 1.0, false)]);
            ly.extend([(p.rect.y + 5.0 * k, 1.0, false), (p.rect.bottom() - 5.0 * k, 1.0, false)]);
        }
    }
    // room to go around the whole figure
    lx.extend([(lay.bounds.x - 2.0 * m, 1.0, true), (lay.bounds.right() + 2.0 * m, 1.0, true)]);
    ly.push((lay.bounds.y - 2.0 * m, 1.0, true));
    if lay.legend.is_empty() {
        ly.push((lay.bounds.bottom() + 2.0 * m, 1.0, true));
    }
    // the points the lines really use on the sides
    for (ei, e) in g.edges.iter().enumerate() {
        for (end, id) in [(0, &e.from), (1, &e.to)] {
            let n = &lay.nodes[g.node_index(id).unwrap()];
            if n.ellipse {
                continue;
            }
            let at: Vec<(Side, usize)> = match fixed[ei][end] {
                End::Free => vec![],
                End::Site(s) => sides_of(s),
                End::At(sd, sl) => vec![(sd, sl)],
            };
            for (sd, sl) in at {
                let p = site_point(n, sd, sl);
                match sd {
                    Side::Top | Side::Bottom => lx.push((p.0, 1.0, true)),
                    _ => ly.push((p.1, 1.0, true)),
                }
            }
        }
    }
    let (xs, xw) = axis(lx, 2.5 * k);
    let (ys, yw) = axis(ly, 2.5 * k);
    let (nx, ny) = (xs.len(), ys.len());
    let mut grid = Grid {
        h_block: vec![false; (nx - 1) * ny],
        v_block: vec![false; nx * (ny - 1)],
        h_cost: vec![0.0; (nx - 1) * ny],
        v_cost: vec![0.0; nx * (ny - 1)],
        h_use: vec![Vec::new(); (nx - 1) * ny],
        v_use: vec![Vec::new(); nx * (ny - 1)],
        h_through: vec![0; nx * ny],
        v_through: vec![0; nx * ny],
        xs,
        ys,
        xw,
        yw,
    };
    for w in &walls {
        for j in 0..ny {
            let y = grid.ys[j];
            if y > w.y + 1e-6 && y < w.bottom() - 1e-6 {
                for i in 0..nx - 1 {
                    if grid.xs[i] < w.right() - 1e-6 && grid.xs[i + 1] > w.x + 1e-6 {
                        let idx = grid.hi(i, j);
                        grid.h_block[idx] = true;
                    }
                }
            }
        }
        for i in 0..nx {
            let x = grid.xs[i];
            if x > w.x + 1e-6 && x < w.right() - 1e-6 {
                for j in 0..ny - 1 {
                    if grid.ys[j] < w.bottom() - 1e-6 && grid.ys[j + 1] > w.y + 1e-6 {
                        let idx = grid.vi(i, j);
                        grid.v_block[idx] = true;
                    }
                }
            }
        }
    }
    // crossing a panel border costs, so long lines keep to the gaps
    let border = 14.0 * k;
    for p in lay.panels.iter().filter(|p| !p.bare) {
        let r = &p.rect;
        for j in 0..ny {
            if grid.ys[j] > r.y && grid.ys[j] < r.bottom() {
                for i in 0..nx - 1 {
                    for bx in [r.x, r.right()] {
                        if grid.xs[i] < bx && grid.xs[i + 1] >= bx {
                            let idx = grid.hi(i, j);
                            grid.h_cost[idx] += border;
                        }
                    }
                }
            }
        }
        for i in 0..nx {
            if grid.xs[i] > r.x && grid.xs[i] < r.right() {
                for j in 0..ny - 1 {
                    for by in [r.y, r.bottom()] {
                        if grid.ys[j] < by && grid.ys[j + 1] >= by {
                            let idx = grid.vi(i, j);
                            grid.v_cost[idx] += border;
                        }
                    }
                }
            }
        }
    }

    // short flow lines first, then the lines that go around
    let centre_dist = |e: &Edge| {
        let (a, b) = (&lay.nodes[g.node_index(&e.from).unwrap()].rect, &lay.nodes[g.node_index(&e.to).unwrap()].rect);
        (a.cx() - b.cx()).abs() + (a.cy() - b.cy()).abs()
    };
    let mut order: Vec<usize> = (0..g.edges.len()).collect();
    let rank = |e: &Edge| match e.kind {
        EdgeKind::Feedback => 3,
        EdgeKind::Cross | EdgeKind::Skip => 2,
        k if k.is_flow() => 0,
        _ => 1,
    };
    order.sort_by(|a, b| {
        let (ea, eb) = (&g.edges[*a], &g.edges[*b]);
        rank(ea).cmp(&rank(eb)).then(centre_dist(ea).partial_cmp(&centre_dist(eb)).unwrap_or(Ordering::Equal)).then(a.cmp(b))
    });

    let mut out: Vec<Option<Route>> = vec![None; g.edges.len()];
    for ei in order {
        let e = &g.edges[ei];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        let (na, nb) = (&lay.nodes[a], &lay.nodes[b]);
        let same_panel = na.panel == nb.panel;
        let down = lay.panels[na.panel].down;
        // candidate ports: (side, port point, escape point, grid index, cost) and the slot used
        let ports = |n: &crate::layout::Placed, other: &Rect, is_source: bool, idx: usize, end: End| -> (Vec<Port>, Vec<usize>) {
            let mut v = Vec::new();
            let mut slots = Vec::new();
            let opts: Vec<(Side, usize)> = match end {
                End::Free => Side::ALL.iter().map(|s| (*s, 2)).collect(),
                End::Site(s) => sides_of(s).into_iter().map(|(sd, sl)| (sd, if n.ellipse { 2 } else { sl })).collect(),
                End::At(sd, sl) => vec![(sd, if n.ellipse { 2 } else { sl })],
            };
            for (s, slot) in opts {
                let p = site_point(n, s, slot);
                let esc = match s {
                    Side::Left => (n.reserve.x - m, p.1),
                    Side::Right => (n.reserve.right() + m, p.1),
                    Side::Top => (p.0, n.reserve.y - m),
                    Side::Bottom => (p.0, n.reserve.bottom() + m),
                };
                let (Some(i), Some(j)) = (Grid::find(&grid.xs, esc.0), Grid::find(&grid.ys, esc.1)) else { continue };
                // the stub from the port must be clear of everything else
                if walls.iter().enumerate().any(|(wi, w)| wi != idx && segment_hits(p, esc, w)) {
                    continue;
                }
                let is_op = g.nodes[idx].module == "op";
                let cost = side_cost(e.kind, is_source, s, &n.rect, other, down, same_panel, is_op, k) + (esc.0 - p.0).abs() + (esc.1 - p.1).abs();
                v.push((s, p, (i, j), cost));
                slots.push(slot);
            }
            (v, slots)
        };
        let (starts, s_slots) = ports(na, &nb.rect, true, a, fixed[ei][0]);
        let (goals, g_slots) = ports(nb, &na.rect, false, b, fixed[ei][1]);
        let found = astar(&grid, &starts, &goals, (a, b), bend, k);
        let r = match found {
            Some((path, si, gi)) => {
                let (s_side, s_port, _, _) = starts[si];
                let (g_side, g_port, _, _) = goals[gi];
                // register usage
                for w in path.windows(2) {
                    let ((i0, j0), (i1, j1)) = (w[0], w[1]);
                    if j0 == j1 {
                        let idx = grid.hi(i0.min(i1), j0);
                        grid.h_use[idx].push((a, b));
                    } else {
                        let idx = grid.vi(i0, j0.min(j1));
                        grid.v_use[idx].push((a, b));
                    }
                }
                for w in path.windows(3) {
                    let idx = grid.pi(w[1].0, w[1].1);
                    if w[0].1 == w[1].1 && w[1].1 == w[2].1 {
                        grid.h_through[idx] += 1;
                    } else if w[0].0 == w[1].0 && w[1].0 == w[2].0 {
                        grid.v_through[idx] += 1;
                    }
                }
                let mut pts = vec![s_port];
                pts.extend(path.iter().map(|(i, j)| (grid.xs[*i], grid.ys[*j])));
                pts.push(g_port);
                Route { edge: ei, points: simplify(pts), from_side: s_side, to_side: g_side, from_site: site_of(s_side, s_slots[si]), to_site: site_of(g_side, g_slots[gi]), forced: false, warnings: Vec::new() }
            }
            None if fallback.is_some_and(|f| !f[ei].forced) => fallback.unwrap()[ei].clone(),
            None => {
                // nearest pair of ports, straight
                let mut best = (f64::MAX, Side::Right, Side::Left);
                for s in Side::ALL {
                    for t in Side::ALL {
                        let (p, q) = (na.port(s), nb.port(t));
                        let d = (p.0 - q.0).abs() + (p.1 - q.1).abs();
                        if d < best.0 {
                            best = (d, s, t);
                        }
                    }
                }
                Route { edge: ei, points: vec![na.port(best.1), nb.port(best.2)], from_side: best.1, to_side: best.2, from_site: site_of(best.1, 2), to_site: site_of(best.2, 2), forced: true, warnings: Vec::new() }
            }
        };
        out[ei] = Some(r);
    }
    out.into_iter().map(|r| r.unwrap()).collect()
}

fn simplify(pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for p in pts {
        if let Some(l) = out.last()
            && (l.0 - p.0).abs() < 1e-6
            && (l.1 - p.1).abs() < 1e-6
        {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let collinear = ((a.0 - b.0).abs() < 1e-6 && (b.0 - p.0).abs() < 1e-6) || ((a.1 - b.1).abs() < 1e-6 && (b.1 - p.1).abs() < 1e-6);
            if collinear {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

type Port = (Side, (f64, f64), (usize, usize), f64);

/// Cheapest path from any start port to any goal port. Returns the grid points
/// and the indices of the chosen start and goal.
fn astar(grid: &Grid, starts: &[Port], goals: &[Port], who: (usize, usize), bend: f64, k: f64) -> Option<(Vec<(usize, usize)>, usize, usize)> {
    if starts.is_empty() || goals.is_empty() {
        return None;
    }
    let (nx, ny) = (grid.nx(), grid.ny());
    let sid = |i: usize, j: usize, d: usize| (j * nx + i) * 4 + d;
    let mut best = vec![f64::INFINITY; nx * ny * 4];
    let mut prev: Vec<u32> = vec![u32::MAX; nx * ny * 4];
    let mut origin: Vec<u16> = vec![u16::MAX; nx * ny * 4];
    let h = |i: usize, j: usize| goals.iter().map(|gl| (grid.xs[i] - grid.xs[gl.2.0]).abs() + (grid.ys[j] - grid.ys[gl.2.1]).abs() + gl.3).fold(f64::INFINITY, f64::min);
    let mut heap = BinaryHeap::new();
    for (si, s) in starts.iter().enumerate() {
        let (i, j) = s.2;
        let d = s.0.dir();
        let id = sid(i, j, d);
        if s.3 < best[id] {
            best[id] = s.3;
            origin[id] = si as u16;
            heap.push(State { f: s.3 + h(i, j), g: s.3, i, j, d });
        }
    }
    let overlap = 70.0 * k;
    let cross = 9.0 * k;
    let mut done: Option<(usize, usize, f64)> = None; // (state id, goal index, total)
    while let Some(st) = heap.pop() {
        let id = sid(st.i, st.j, st.d);
        if st.g > best[id] + 1e-9 {
            continue;
        }
        if let Some((_, _, total)) = done
            && st.f >= total - 1e-9
        {
            break;
        }
        // arriving at a goal's escape point heading into its port
        for (gi, gl) in goals.iter().enumerate() {
            if gl.2 == (st.i, st.j) && gl.0.dir() != st.d {
                // turning into the port is a corner like any other
                let total = st.g + gl.3 + if gl.0.opposite().dir() == st.d { 0.0 } else { bend };
                if done.is_none_or(|d| total < d.2 - 1e-9) {
                    done = Some((id, gi, total));
                }
            }
        }
        for nd in 0..4 {
            if nd == (st.d + 2) % 4 {
                continue;
            }
            let (ni, nj) = match nd {
                0 if st.i + 1 < nx => (st.i + 1, st.j),
                1 if st.j + 1 < ny => (st.i, st.j + 1),
                2 if st.i > 0 => (st.i - 1, st.j),
                3 if st.j > 0 => (st.i, st.j - 1),
                _ => continue,
            };
            let horizontal = nd % 2 == 0;
            let (eidx, len, weight) = if horizontal {
                let e = grid.hi(st.i.min(ni), st.j);
                (e, (grid.xs[ni] - grid.xs[st.i]).abs(), grid.yw[st.j])
            } else {
                let e = grid.vi(st.i, st.j.min(nj));
                (e, (grid.ys[nj] - grid.ys[st.j]).abs(), grid.xw[st.i])
            };
            if if horizontal { grid.h_block[eidx] } else { grid.v_block[eidx] } {
                continue;
            }
            let mut cost = len * weight + if horizontal { grid.h_cost[eidx] } else { grid.v_cost[eidx] };
            if nd != st.d {
                cost += bend;
            }
            let users = if horizontal { &grid.h_use[eidx] } else { &grid.v_use[eidx] };
            if users.iter().any(|u| u.0 != who.0 && u.1 != who.1) {
                cost += overlap + 2.0 * len;
            }
            let pidx = grid.pi(ni, nj);
            cost += cross * if horizontal { grid.v_through[pidx] } else { grid.h_through[pidx] } as f64;
            let g2 = st.g + cost;
            let nid = sid(ni, nj, nd);
            if g2 < best[nid] - 1e-9 {
                best[nid] = g2;
                prev[nid] = id as u32;
                origin[nid] = origin[id];
                heap.push(State { f: g2 + h(ni, nj), g: g2, i: ni, j: nj, d: nd });
            }
        }
    }
    let (mut id, gi, _) = done?;
    let si = origin[id] as usize;
    let mut path = Vec::new();
    loop {
        let p = id / 4;
        path.push((p % nx, p / nx));
        if prev[id] == u32::MAX {
            break;
        }
        id = prev[id] as usize;
    }
    path.reverse();
    Some((path, si, gi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Drawing, Figure, LayoutOptions, dsl};
    use lore_canvas::Kind;

    fn draw_with(src: &str, place: &[(&str, [f64; 2])], ends: &[(&str, EdgeEnds)]) -> (Graph, Figure) {
        let g = dsl::parse(src).unwrap();
        let options = LayoutOptions { place: place.iter().map(|(k, v)| (k.to_string(), *v)).collect(), ends: ends.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), ..Default::default() };
        let f = crate::draw(&g, &Drawing { options, ..Default::default() }).unwrap();
        (g, f)
    }

    fn draw_plain(src: &str) -> (Graph, Figure) {
        draw_with(src, &[], &[])
    }

    fn ends(from: Option<u8>, to: Option<u8>, turns: &[f64]) -> EdgeEnds {
        EdgeEnds { from, to, turns: turns.to_vec() }
    }

    const CHAIN: &str = "fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\na -> b -> c\n";
    const TWO_IN: &str = "fig t\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\na -> c\nb -> c\n";
    const THREE_IN: &str = "fig t\na: box \"A\"\nb: box \"B\"\nd: box \"D\"\nc: box \"C\"\na -> c\nb -> c\nd -> c\n";

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    fn on_boundary(p: (f64, f64), r: &Rect) -> bool {
        let inside = p.0 >= r.x - 1e-6 && p.0 <= r.right() + 1e-6 && p.1 >= r.y - 1e-6 && p.1 <= r.bottom() + 1e-6;
        inside && ((p.0 - r.x).abs() < 1e-6 || (p.0 - r.right()).abs() < 1e-6 || (p.1 - r.y).abs() < 1e-6 || (p.1 - r.bottom()).abs() < 1e-6)
    }

    fn edge(g: &Graph, f: &Figure, from: &str, to: &str) -> Route {
        f.routes.iter().find(|r| g.edges[r.edge].from == from && g.edges[r.edge].to == to).unwrap().clone()
    }

    #[test]
    fn a_moved_block_is_shifted_and_its_lines_end_on_it() {
        let (g, base) = draw_plain(CHAIN);
        let (_, moved) = draw_with(CHAIN, &[("b", [10.0, 20.0])], &[]);
        let k = base.layout.scale;
        let (b0, b1) = (&base.layout.nodes[1].rect, &moved.layout.nodes[1].rect);
        assert!((b1.x - b0.x - 10.0 * k).abs() < 1e-6 && (b1.y - b0.y - 20.0 * k).abs() < 1e-6);
        assert_eq!(moved.layout.nodes[0].rect, base.layout.nodes[0].rect);
        let mut ended = 0;
        for r in &moved.routes {
            let e = &g.edges[r.edge];
            if e.to == "b" {
                assert!(on_boundary(*r.points.last().unwrap(), b1), "{:?}", r.points);
                ended += 1;
            }
            if e.from == "b" {
                assert!(on_boundary(r.points[0], b1), "{:?}", r.points);
                ended += 1;
            }
            assert!(!r.forced);
            assert!(r.points.windows(2).all(|w| (w[0].0 - w[1].0).abs() < 1e-6 || (w[0].1 - w[1].1).abs() < 1e-6));
        }
        assert_eq!(ended, 2);
        // the panel and the bounds hold the moved block
        let p = &moved.layout.panels[0].rect;
        assert!(p.contains(b1) && moved.layout.bounds.contains(p));
    }

    #[test]
    fn a_block_cannot_be_moved_out_of_the_figure() {
        let (_, f) = draw_with(CHAIN, &[("b", [10_000.0, 10_000.0]), ("a", [-10_000.0, -10_000.0])], &[]);
        let frame = LayoutOptions::default().frame.inflate(0.01);
        for n in &f.layout.nodes[..2] {
            assert!(frame.contains(&n.rect), "{:?}", n.rect);
        }
        assert!(f.layout.panels[0].rect.contains(&f.layout.nodes[0].rect) && f.layout.panels[0].rect.contains(&f.layout.nodes[1].rect));
    }

    #[test]
    fn an_unknown_id_in_place_is_ignored() {
        let (_, base) = draw_plain(CHAIN);
        let (_, f) = draw_with(CHAIN, &[("nope", [50.0, 50.0])], &[]);
        assert_eq!(f.layout, base.layout);
        assert_eq!(f.routes, base.routes);
    }

    #[test]
    fn moving_nothing_changes_nothing() {
        let (_, a) = draw_plain(TWO_IN);
        let (_, b) = draw_with(TWO_IN, &[], &[]);
        assert_eq!(a.layout, b.layout);
        assert_eq!(a.routes, b.routes);
        assert!(a.layout.nodes.iter().all(|n| n.ends.is_empty()));
    }

    #[test]
    fn one_arrow_into_a_side_uses_the_middle() {
        let (g, f) = draw_plain(CHAIN);
        let r = edge(&g, &f, "a", "b");
        let b = &f.layout.nodes[1];
        assert_eq!((r.from_side, r.to_side, r.from_site, r.to_site), (Side::Right, Side::Left, 3, 1));
        assert!(near(*r.points.last().unwrap(), (b.rect.x, b.rect.cy())));
    }

    #[test]
    fn two_arrows_into_a_side_use_the_quarter_points() {
        let (g, f) = draw_plain(TWO_IN);
        let c = &f.layout.nodes[2];
        let (ra, rb) = (edge(&g, &f, "a", "c"), edge(&g, &f, "b", "c"));
        assert_eq!((ra.to_side, rb.to_side), (Side::Left, Side::Left));
        let (upper, lower) = if f.layout.nodes[0].rect.cy() < f.layout.nodes[1].rect.cy() { (&ra, &rb) } else { (&rb, &ra) };
        assert!(near(*upper.points.last().unwrap(), (c.rect.x, c.rect.y + c.rect.h / 4.0)));
        assert!(near(*lower.points.last().unwrap(), (c.rect.x, c.rect.y + c.rect.h * 0.75)));
        assert_eq!((upper.to_site, lower.to_site), (15, 14));
        assert!(!ra.forced && !rb.forced);
    }

    #[test]
    fn three_arrows_into_a_side_use_three_points_without_crossing() {
        let (g, f) = draw_plain(THREE_IN);
        let c = &f.layout.nodes[3];
        let mut ends: Vec<(f64, f64, u8)> = Vec::new();
        for from in ["a", "b", "d"] {
            let r = edge(&g, &f, from, "c");
            assert_eq!(r.to_side, Side::Left);
            let i = g.node_index(from).unwrap();
            ends.push((f.layout.nodes[i].rect.cy(), r.points.last().unwrap().1, r.to_site));
        }
        ends.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        // the order of the sources along the side is the order of the points: no crossing
        assert!(ends[0].1 < ends[1].1 && ends[1].1 < ends[2].1, "{ends:?}");
        assert!((ends[0].1 - (c.rect.y + c.rect.h / 4.0)).abs() < 1e-6 && (ends[1].1 - c.rect.cy()).abs() < 1e-6 && (ends[2].1 - (c.rect.y + c.rect.h * 0.75)).abs() < 1e-6);
        assert_eq!(ends.iter().map(|e| e.2).collect::<Vec<_>>(), [15, 1, 14]);
    }

    #[test]
    fn an_ellipse_keeps_one_point_a_side() {
        let src = "fig t\na: box \"A\"\nb: box \"B\"\nc: op \"+\"\na -> c\nb -> c\n";
        let (g, f) = draw_plain(src);
        let c = &f.layout.nodes[2];
        assert!(c.ellipse);
        for from in ["a", "b"] {
            let r = edge(&g, &f, from, "c");
            assert!(near(*r.points.last().unwrap(), c.port(r.to_side)), "{:?}", r.points);
            assert_eq!(r.to_site, site_of(r.to_side, 2));
        }
        // and it is attached to the ellipse's own site
        for o in f.objects() {
            if let Kind::Connector(cn) = &o.kind {
                assert!(cn.end.as_ref().unwrap().site < 8);
            }
        }
    }

    #[test]
    fn an_end_uses_the_point_the_author_chose() {
        let (g, f) = draw_with(CHAIN, &[], &[("a>b", ends(Some(8), Some(15), &[]))]);
        let (a, b) = (&f.layout.nodes[0], &f.layout.nodes[1]);
        let r = edge(&g, &f, "a", "b");
        assert!(near(r.points[0], (a.rect.right(), a.rect.y + a.rect.h / 4.0)), "{:?}", r.points);
        assert!(near(*r.points.last().unwrap(), (b.rect.x, b.rect.y + b.rect.h / 4.0)), "{:?}", r.points);
        assert_eq!((r.from_site, r.to_site), (8, 15));
        assert!(!r.forced && r.warnings.is_empty());
    }

    #[test]
    fn a_corner_point_can_be_chosen() {
        let (g, f) = draw_with(CHAIN, &[], &[("a>b", ends(None, Some(4), &[]))]);
        let b = &f.layout.nodes[1];
        let r = edge(&g, &f, "a", "b");
        assert!(near(*r.points.last().unwrap(), (b.rect.x, b.rect.y)), "{:?}", r.points);
        assert_eq!(r.to_site, 4);
        assert!(matches!(r.to_side, Side::Top | Side::Left));
        assert!(!r.forced);
        // the attached site is the corner
        let o = f.objects().iter().find(|o| o.name == "flow:m/edge:a-b").unwrap();
        let Kind::Connector(c) = &o.kind else { panic!("a connector") };
        assert_eq!(c.end.as_ref().unwrap().site, 4);
    }

    #[test]
    fn the_second_edge_between_two_blocks_has_its_own_key() {
        let src = "fig t\na: box \"A\"\nb: box \"B\"\na -> b\na -> b : optional\n";
        let g = dsl::parse(src).unwrap();
        assert_eq!(edge_keys(&g), ["a>b", "a>b#2"]);
        let (_, f) = draw_with(src, &[], &[("a>b#2", ends(None, Some(15), &[]))]);
        assert_eq!(f.routes[1].to_site, 15);
        assert_ne!(f.routes[0].to_site, 15);
        assert!(f.routes.iter().all(|r| r.warnings.is_empty()));
    }

    /// The natural-size coordinate that lands on the figure's `v`.
    fn natural(f: &Figure, v: f64, horizontal_run: bool) -> f64 {
        let o = f.layout.nodes[0].origin;
        (v - if horizontal_run { o.1 } else { o.0 }) / f.layout.scale
    }

    #[test]
    fn a_turn_is_placed_where_the_author_put_it() {
        let (g, base) = draw_plain(TWO_IN);
        let (a, c) = (&base.layout.nodes[0].rect, &base.layout.nodes[2].rect);
        let x = a.right() + 0.7 * (c.x - a.right());
        let (_, f) = draw_with(TWO_IN, &[], &[("a>c", ends(None, None, &[natural(&base, x, false)]))]);
        let r = edge(&g, &f, "a", "c");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.points.windows(2).any(|w| (w[0].0 - x).abs() < 1e-6 && (w[1].0 - x).abs() < 1e-6 && (w[0].1 - w[1].1).abs() > 1e-6), "{:?} should turn at x = {x}", r.points);
        assert!(f.built.warnings.is_empty(), "{:?}", f.built.warnings);
    }

    #[test]
    fn a_turn_that_cannot_be_honoured_is_ignored_with_a_warning() {
        let (g, base) = draw_plain(TWO_IN);
        // two runs cannot join a right side to a left side
        let (_, f) = draw_with(TWO_IN, &[], &[("a>c", ends(None, None, &[10.0, 20.0])), ("nope>x", ends(None, None, &[5.0]))]);
        assert_eq!(edge(&g, &f, "a", "c").points, edge(&g, &base, "a", "c").points);
        assert!(f.built.warnings.iter().any(|w| w.contains("a → c") && w.contains("轉彎")), "{:?}", f.built.warnings);
        assert!(!f.built.warnings.iter().any(|w| w.contains("nope")));
    }

    #[test]
    fn connectors_are_attached_to_the_point_they_use() {
        let (g, f) = draw_plain(TWO_IN);
        let mut seen = Vec::new();
        for r in &f.routes {
            let e = &g.edges[r.edge];
            let o = f.objects().iter().find(|o| o.name == format!("flow:t/edge:{}-{}", e.from, e.to)).unwrap();
            let Kind::Connector(c) = &o.kind else { panic!("a connector") };
            assert_eq!(c.end.as_ref().unwrap().site as u8, r.to_site);
            assert_eq!(c.start.as_ref().unwrap().site as u8, r.from_site);
            seen.push(c.end.as_ref().unwrap().site);
        }
        seen.sort();
        assert_eq!(seen, [14, 15]);
    }

    #[test]
    fn a_line_with_too_many_turns_says_so() {
        let (g, base) = draw_plain(TWO_IN);
        let (a, c) = (&base.layout.nodes[0].rect, &base.layout.nodes[2].rect);
        let p = edge(&g, &base, "a", "c").points[0];
        let (x1, x3) = (a.right() + 0.25 * (c.x - a.right()), a.right() + 0.75 * (c.x - a.right()));
        let y2 = p.1 - 8.0;
        let turns = [natural(&base, x1, false), natural(&base, y2, true), natural(&base, x3, false)];
        let (_, f) = draw_with(TWO_IN, &[], &[("a>c", ends(None, None, &turns))]);
        let r = edge(&g, &f, "a", "c");
        assert_eq!(r.bends(), 4, "{:?} {:?}", r.points, r.warnings);
        assert!(f.built.warnings.iter().any(|w| w == "a → c: 轉太多彎，沒有接在方塊上"), "{:?}", f.built.warnings);
        let o = f.objects().iter().find(|o| o.name == "flow:t/edge:a-c").unwrap();
        assert!(matches!(o.kind, Kind::Shape(_)));
    }

    #[test]
    fn no_route_cuts_through_a_block_after_a_move() {
        let src = "fig w\npanel p \"P\"\n a: box \"A\"\n b: box \"B\"\n c: box \"C\"\npanel q \"Q\"\n d: box \"D\"\n e: box \"E\"\n f: box \"F\"\nend\na -> b -> c\nd -> e -> f\nc -> d : cross\nb -> e : cross\n";
        for id in ["a", "b", "c", "d", "e", "f"] {
            let (g, f) = draw_with(src, &[(id, [30.0, 20.0])], &[]);
            for r in &f.routes {
                let e = &g.edges[r.edge];
                for w in r.points.windows(2) {
                    for (ni, n) in f.layout.nodes.iter().enumerate() {
                        let nid = &g.nodes[ni].id;
                        if *nid == e.from || *nid == e.to {
                            continue;
                        }
                        assert!(!segment_hits(w[0], w[1], &n.rect), "moving {id}: {} → {} cuts through {nid}: {:?}", e.from, e.to, r.points);
                    }
                }
            }
        }
    }

    #[test]
    fn lines_stay_clean_when_they_spread_over_the_points() {
        let mut srcs: Vec<String> = vec![
            "fig s\na: box \"A\"\nb: box \"Wide middle step\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c -> d\na -> d : skip\nd -> a : feedback\na -> c : optional\n".into(),
            "fig f\na: image \"in\"\nb: box \"B\"\nc: box \"C\"\nd: op \"+\"\ne: box \"E\"\nf: loss \"L\"\ng: image \"gt\" role=gt\na -> b -> d -> e -> f\na -> c -> d\ng -> f : supervision\nf -> b : feedback\nf -> c : feedback\nb -> e : skip\n".into(),
            "fig g\npanel p \"P\"\n a: box \"A\"\n b: box \"B\"\npanel q \"Q\"\n c: box \"C\"\n d: box \"D\"\npanel r \"R\"\n e: box \"E\"\nend\na -> b\nc -> d\nb -> c : cross\nd -> a : cross\nb -> e : cross\ne -> c : cross\n".into(),
        ];
        let mut big = String::from("fig big\n");
        for r in 0..4 {
            for c in 0..6 {
                big.push_str(&format!("n{r}_{c}: box \"N{r}{c}\"\n"));
            }
            big.push_str(&(0..6).map(|c| format!("n{r}_{c}")).collect::<Vec<_>>().join(" -> "));
            big.push('\n');
        }
        big.push_str("n0_5 -> n1_0 : feedback\nn1_2 -> n2_4 : skip\nn3_5 -> n0_0 : feedback\nn2_1 -> n3_3\nn0_2 -> n3_4 : optional\n");
        srcs.push(big);
        for src in srcs {
            let (g, f) = draw_plain(&src);
            for r in &f.routes {
                let e = &g.edges[r.edge];
                let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
                assert!(!r.forced, "{} -> {} found no path", e.from, e.to);
                let slot = |site: u8, side: Side| sides_of(site).into_iter().find(|(s, _)| *s == side).unwrap().1;
                assert!(near(r.points[0], site_point(&f.layout.nodes[a], r.from_side, slot(r.from_site, r.from_side))));
                assert!(near(*r.points.last().unwrap(), site_point(&f.layout.nodes[b], r.to_side, slot(r.to_site, r.to_side))));
                for (si, w) in r.points.windows(2).enumerate() {
                    assert!((w[0].0 - w[1].0).abs() < 1e-6 || (w[0].1 - w[1].1).abs() < 1e-6);
                    for (ni, n) in f.layout.nodes.iter().enumerate() {
                        let own = (ni == a && si == 0) || (ni == b && si == r.points.len() - 2);
                        if own {
                            continue;
                        }
                        assert!(!segment_hits(w[0], w[1], &n.reserve), "{} -> {} runs through {}", e.from, e.to, g.nodes[ni].id);
                    }
                }
            }
        }
    }
}
