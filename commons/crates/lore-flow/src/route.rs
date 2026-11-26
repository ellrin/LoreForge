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
    /// The edge asked to be drawn as one smooth curve ([`curve_path`]); the points stay the
    /// orthogonal route.
    pub curve: bool,
    /// The line leaves the middle of another line (`Edge.off`), not a block: `points[0]` is on
    /// that line, `from_site` is 0 and it has no slidable runs.
    pub branch: bool,
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
    /// The author named the side (`from=` / `to=`); the point on it is the router's.
    OnSide(Side),
}

/// The side a line's `from=` / `to=` names.
pub fn side_named(w: &str) -> Option<Side> {
    match w {
        "top" => Some(Side::Top),
        "bottom" => Some(Side::Bottom),
        "left" => Some(Side::Left),
        "right" => Some(Side::Right),
        _ => None,
    }
}

/// Does this side of the block hold words (its label, variable or caption above or below the
/// picture)? Lines leave and enter there only when no other side has a way.
pub fn words_side(n: &Placed, side: Side) -> bool {
    match side {
        Side::Top => n.words.0,
        Side::Bottom => n.words.1,
        _ => false,
    }
}

/// Where a line's end meets a block: the connection point ([`site_point`]), except on a side
/// that holds words, where it stops outside the words, 2 pt (scaled) from them.
pub fn end_point(n: &Placed, side: Side, slot: usize, k: f64) -> (f64, f64) {
    let p = site_point(n, side, slot);
    if !words_side(n, side) {
        return p;
    }
    match side {
        Side::Top => (p.0, n.rect.y.min(n.art.y) - 2.0 * k),
        Side::Bottom => (p.0, n.rect.bottom().max(n.art.bottom()) + 2.0 * k),
        _ => p,
    }
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
    // the block's port: its drawn picture, without its words
    let r = &n.art;
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
    for (ei, e) in g.edges.iter().enumerate() {
        for (i, w) in [&e.from_side, &e.to_side].into_iter().enumerate() {
            if w.is_empty() || fixed1[ei][i] != End::Free {
                continue;
            }
            match side_named(w) {
                Some(s) => fixed1[ei][i] = End::OnSide(s),
                None => notes[ei].push(format!("{}: 不認得的邊 {w}，已忽略", name(ei))),
            }
        }
    }
    let parent = branch_parents(g, &keys);
    let skip: Vec<bool> = parent.iter().map(Option::is_some).collect();
    for (ei, e) in g.edges.iter().enumerate() {
        if !e.off.is_empty() && parent[ei].is_none() {
            notes[ei].push(format!("{}: 找不到要岔出的線", name(ei)));
        }
    }
    let first = run(g, lay, &fixed1, None, &skip);

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
        if r.forced || skip[ei] {
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
    let level: Vec<bool> = first.iter().enumerate().map(|(ei, r)| !skip[ei] && is_level(g, lay, r)).collect();
    for ((node, side), mut uses) in groups {
        let mut taken: Vec<usize> = uses.iter().filter_map(|u| u.slot).collect();
        for u in uses.iter().filter(|u| u.slot.is_some()) {
            fixed2[u.edge][u.end] = End::At(side, u.slot.unwrap());
        }
        uses.retain(|u| u.slot.is_none());
        // the main line: one whose other end is level with this block keeps the middle and
        // stays straight; the others are spread round it
        if !taken.contains(&2)
            && let Some(p) = uses.iter().position(|u| level[u.edge])
        {
            let u = uses.remove(p);
            fixed2[u.edge][u.end] = End::At(side, 2);
            taken.push(2);
            // two or more lines that leave beside a straight main line leave from its point and
            // turn off it, a T off the main line (a lone one keeps its quarter point, below)
            if u.end == 0 && uses.len() >= 2 &&uses.iter().all(|v| v.end == 0 && off_middle(lay, node, side, v.key).is_some()) {
                for v in &uses {
                    fixed2[v.edge][v.end] = End::At(side, 2);
                }
                moved = true;
                continue;
            }
            // a lone neighbour goes to the quarter on the side it lies on
            if uses.len() == 1 {
                let o = &lay.nodes[node];
                let mid = if matches!(side, Side::Top | Side::Bottom) { o.rect.cx() } else { o.port_y };
                let slot = if uses[0].key < mid { 1 } else { 3 };
                fixed2[uses[0].edge][uses[0].end] = End::At(side, slot);
                moved = true;
                continue;
            }
        }
        // a fork: two lines leaving (or entering) one side towards opposite sides of the block's
        // middle share the middle point and split at one column, like a T, instead of leaving
        // at two points that make two separate bends
        if uses.len() == 2 && taken.is_empty() && uses[0].end == uses[1].end {
            let o = &lay.nodes[node];
            let mid = if matches!(side, Side::Top | Side::Bottom) { o.rect.cx() } else { o.port_y };
            let gap = 2.0 * lay.scale;
            if (uses[0].key < mid - gap && uses[1].key > mid + gap) || (uses[1].key < mid - gap && uses[0].key > mid + gap) {
                for u in &uses {
                    fixed2[u.edge][u.end] = End::At(side, 2);
                }
                moved = true;
                continue;
            }
        }
        // a fork the same way: lines leaving one side that all turn the same way (all up or all
        // down) leave as one trunk from the middle and split where the nearer target needs
        if uses.len() >= 2 && taken.is_empty() && uses.iter().all(|u| u.end == 0) {
            let ways: Vec<Option<bool>> = uses.iter().map(|u| off_middle(lay, node, side, u.key)).collect();
            if ways.iter().all(|w| w.is_some() && *w == ways[0]) {
                for u in &uses {
                    fixed2[u.edge][u.end] = End::At(side, 2);
                }
                moved = true;
                continue;
            }
        }
        uses.sort_by(|x, y| x.key.partial_cmp(&y.key).unwrap_or(Ordering::Equal).then((x.edge, x.end).cmp(&(y.edge, y.end))));
        for (u, slot) in uses.iter().zip(spread(uses.len(), &taken)) {
            fixed2[u.edge][u.end] = End::At(side, slot);
            moved |= slot != 2;
        }
    }
    let mut out = if moved { run(g, lay, &fixed2, Some(&first), &skip) } else { first };
    one_trunk(g, lay, &mut out);

    // where the author wants the line to turn
    for (ei, key) in keys.iter().enumerate() {
        let Some(c) = ends.get(key).filter(|c| !c.turns.is_empty() && !skip[ei]) else { continue };
        let e = &g.edges[ei];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        match (!out[ei].forced).then(|| turned(lay, a, b, &out[ei], &c.turns)).flatten() {
            Some(pts) => out[ei].points = pts,
            None => notes[ei].push(format!("{}: 轉彎的位置無法照辦，已忽略", name(ei))),
        }
    }
    // the lines that leave the middle of another line, after the final paths of their parents
    route_branches(g, lay, ends, &keys, &parent, &mut out, &mut notes);
    // a line that had to leave or enter through a block's words and bend says so (straight
    // through the words' side is fine: it stops short of them)
    for (ei, r) in out.iter().enumerate() {
        if r.forced || r.bends() == 0 {
            continue;
        }
        let e = &g.edges[ei];
        let (Some(a), Some(b)) = (g.node_index(&e.from), g.node_index(&e.to)) else { continue };
        if (!r.branch && words_side(&lay.nodes[a], r.from_side)) || words_side(&lay.nodes[b], r.to_side) {
            notes[ei].push(format!("{}: 線從文字那一邊進出", name(ei)));
        }
    }
    for ((r, n), e) in out.iter_mut().zip(notes).zip(&g.edges) {
        r.warnings.extend(n);
        r.curve = e.curve;
    }
    out
}

/// Lines that leave one point of a block and turn the same way share one trunk: each turns
/// where the nearest of them turns (when its path stays clear of the blocks there), so they
/// split off one line instead of running side by side.
fn one_trunk(g: &Graph, lay: &Layout, out: &mut [Route]) {
    let n = out.len();
    for i in 0..n {
        let r = &out[i];
        if r.forced || r.branch || r.points.len() < 3 {
            continue;
        }
        let horizontal = matches!(r.from_side, Side::Left | Side::Right);
        let first = |r: &Route| if horizontal { (r.points[1].0 - r.points[0].0).abs() } else { (r.points[1].1 - r.points[0].1).abs() };
        let way = |r: &Route| if horizontal { (r.points[2].1 - r.points[1].1).signum() } else { (r.points[2].0 - r.points[1].0).signum() };
        let from = &g.edges[r.edge].from;
        // the line of this fork that turns first
        let mates: Vec<usize> = (0..n)
            .filter(|j| {
                let s = &out[*j];
                !s.forced && !s.branch && s.points.len() >= 3 && g.edges[s.edge].from == *from && s.from_side == r.from_side && (s.points[0].0 - r.points[0].0).abs() < 1e-6 && (s.points[0].1 - r.points[0].1).abs() < 1e-6 && way(s) == way(r)
            })
            .collect();
        let Some(&near) = mates.iter().min_by(|a, b| first(&out[**a]).partial_cmp(&first(&out[**b])).unwrap_or(Ordering::Equal)) else { continue };
        if near == i || r.points.len() < 4 || (first(&out[near]) - first(r)).abs() < 1e-6 {
            continue;
        }
        let t = out[near].points[1];
        let mut pts = r.points.clone();
        if horizontal {
            pts[1].0 = t.0;
            pts[2].0 = t.0;
        } else {
            pts[1].1 = t.1;
            pts[2].1 = t.1;
        }
        // the third run must still go the way it went
        let (d_old, d_new) = if horizontal { (r.points[3].0 - r.points[2].0, pts[3].0 - pts[2].0) } else { (r.points[3].1 - r.points[2].1, pts[3].1 - pts[2].1) };
        if d_old * d_new <= 1e-9 {
            continue;
        }
        let e = &g.edges[r.edge];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        let last = pts.len() - 2;
        // clear of the blocks and of the titles (words), as the router keeps its lines
        let titles: Vec<Rect> = lay.panels.iter().filter_map(|p| p.title_rect).chain(lay.regions.iter().filter(|r| !r.title.is_empty()).filter_map(|r| r.title_rect)).collect();
        let clear = pts.windows(2).enumerate().all(|(si, w)| {
            lay.nodes.iter().enumerate().all(|(ni, nd)| (ni == a && si == 0) || (ni == b && si == last) || !segment_hits(w[0], w[1], &nd.reserve.inflate(3.0 * lay.scale))) && titles.iter().all(|t| !segment_hits(w[0], w[1], t))
        }) && lay.legend.iter().all(|l| pts.windows(2).all(|w| !segment_hits(w[0], w[1], &l.text)));
        if clear {
            out[i].points = simplify(pts);
        }
    }
}

/// Which way a line on `side` of block `node` turns to reach the other end's middle `key`:
/// `Some(true)` past the block's middle (down / right), `Some(false)` before it, `None` when it
/// is within 2 pt (scaled) of the middle.
fn off_middle(lay: &Layout, node: usize, side: Side, key: f64) -> Option<bool> {
    let o = &lay.nodes[node];
    let mid = if matches!(side, Side::Top | Side::Bottom) { o.rect.cx() } else { o.port_y };
    let gap = 2.0 * lay.scale;
    if key > mid + gap {
        Some(true)
    } else if key < mid - gap {
        Some(false)
    } else {
        None
    }
}

/// Is this line level: its two ends face each other and their middle points lie on one line
/// (within 0.5 pt)? Such a line is drawn straight from the middle of both sides.
fn is_level(g: &Graph, lay: &Layout, r: &Route) -> bool {
    if r.forced || r.branch || r.from_side.opposite() != r.to_side {
        return false;
    }
    let e = &g.edges[r.edge];
    let (Some(a), Some(b)) = (g.node_index(&e.from), g.node_index(&e.to)) else { return false };
    let (pa, pb) = (lay.nodes[a].port(r.from_side), lay.nodes[b].port(r.to_side));
    let along = if matches!(r.from_side, Side::Top | Side::Bottom) { (pa.0 - pb.0).abs() } else { (pa.1 - pb.1).abs() };
    along <= 0.5
}

/// For every edge that leaves the middle of another line (`off`): the index of that line.
/// `None` for an ordinary edge, and for an `off` edge whose key is unknown or that is part of a
/// cycle of `off` edges (it is then routed from its block).
fn branch_parents(g: &Graph, keys: &[String]) -> Vec<Option<usize>> {
    let up = |ei: usize| -> Option<usize> {
        let o = &g.edges[ei].off;
        if o.is_empty() { None } else { keys.iter().position(|k| k == o) }
    };
    (0..g.edges.len())
        .map(|ei| {
            let p = up(ei)?;
            let mut at = p;
            for _ in 0..=g.edges.len() {
                if at == ei {
                    return None;
                }
                match up(at) {
                    Some(n) => at = n,
                    None => return Some(p),
                }
            }
            // a cycle that does not contain this edge: its parent is still a line
            Some(p)
        })
        .collect()
}

/// The first point of every line that leaves the middle of another line: where the drawer may
/// put a dot.
pub fn junctions(g: &Graph, routes: &[Route]) -> Vec<(f64, f64)> {
    let _ = g;
    routes.iter().filter(|r| r.branch).filter_map(|r| r.points.first().copied()).collect()
}

/// The control points `[p0, c1, c2, p3]` of the one cubic Bézier a curved line is drawn as:
/// `c1` leaves the first point along its side's normal and `c2` reaches the last one against its
/// side's normal, each 40 % of the straight distance between the ends away (at least 12 pt;
/// the route does not know the figure's scale, so the minimum is in natural units). `None` for
/// a line that is not curved, is `forced`, or has fewer than two points.
pub fn curve_path(r: &Route) -> Option<Vec<(f64, f64)>> {
    if !r.curve || r.forced || r.points.len() < 2 {
        return None;
    }
    let (p0, p3) = (r.points[0], *r.points.last().unwrap());
    let d = ((p3.0 - p0.0).powi(2) + (p3.1 - p0.1).powi(2)).sqrt();
    let len = (0.4 * d).max(12.0);
    let (n0, n3) = (r.from_side.normal(), r.to_side.normal());
    Some(vec![p0, (p0.0 + n0.0 * len, p0.1 + n0.1 * len), (p3.0 + n3.0 * len, p3.1 + n3.1 * len), p3])
}

/// Does the curve with control points `c` (a cubic Bezier, sampled at 24 points) stay out of the
/// rooms of the blocks? Returns the index of the first room (not one of `skip`) it passes through.
pub fn curve_clear(c: &[(f64, f64)], rooms: &[Rect], skip: &[usize]) -> Option<usize> {
    if c.len() != 4 {
        return None;
    }
    // fine enough steps that a line of words (about 10 pt high) is not stepped over
    for i in 1..96 {
        let t = i as f64 / 96.0;
        let u = 1.0 - t;
        let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        let p = (b0 * c[0].0 + b1 * c[1].0 + b2 * c[2].0 + b3 * c[3].0, b0 * c[0].1 + b1 * c[1].1 + b2 * c[2].1 + b3 * c[3].1);
        for (ri, r) in rooms.iter().enumerate() {
            if !skip.contains(&ri) && p.0 > r.x + 1e-6 && p.0 < r.right() - 1e-6 && p.1 > r.y + 1e-6 && p.1 < r.bottom() - 1e-6 {
                return Some(ri);
            }
        }
    }
    None
}

/// Mark the grid edges that the polyline `pts` runs along, by `f` (h: grid edge index of a
/// horizontal one, v: of a vertical one).
fn along_grid(grid: &Grid, pts: &[(f64, f64)], mut h: impl FnMut(usize), mut v: impl FnMut(usize)) {
    for w in pts.windows(2) {
        if (w[0].1 - w[1].1).abs() < 1e-6 {
            if let Some(j) = Grid::find(&grid.ys, w[0].1) {
                for i in 0..grid.nx() - 1 {
                    if grid.xs[i] >= w[0].0.min(w[1].0) - 1e-6 && grid.xs[i + 1] <= w[0].0.max(w[1].0) + 1e-6 {
                        h(grid.hi(i, j));
                    }
                }
            }
        } else if (w[0].0 - w[1].0).abs() < 1e-6
            && let Some(i) = Grid::find(&grid.xs, w[0].0)
        {
            for j in 0..grid.ny() - 1 {
                if grid.ys[j] >= w[0].1.min(w[1].1) - 1e-6 && grid.ys[j + 1] <= w[0].1.max(w[1].1) + 1e-6 {
                    v(grid.vi(i, j));
                }
            }
        }
    }
}

/// Route the branches, parents first. A branch starts on its parent's final path: at the middle
/// of the parent's longest run (at least 8 pt, scaled, long on each side) from which there is a
/// way to its block, else at the point from which the rest of the way is cheapest; it leaves at
/// a right angle to the run it starts on (never along it), at least 8 pt (scaled) from that
/// run's ends when the run is long enough, and never inside a block.
fn route_branches(g: &Graph, lay: &Layout, ends: &BTreeMap<String, EdgeEnds>, keys: &[String], parent: &[Option<usize>], out: &mut [Route], notes: &mut [Vec<String>]) {
    let k = lay.scale;
    let m = 7.0 * k;
    let bend = 18.0 * k;
    // a branch leaves at least this far from the ends of the run it starts on
    let margin = 8.0 * k;
    let mut done: Vec<bool> = parent.iter().map(Option::is_none).collect();
    while let Some(ei) = (0..parent.len()).find(|i| !done[*i] && done[parent[*i].unwrap()]) {
        done[ei] = true;
        let e = &g.edges[ei];
        let pp: Vec<(f64, f64)> = out[parent[ei].unwrap()].points.clone();
        let b = g.node_index(&e.to).unwrap();
        let a = g.node_index(&e.from).unwrap();
        let nb = &lay.nodes[b];
        // the author's point on the target, if any
        let mut fixed = vec![[End::Free; 2]; g.edges.len()];
        if let Some(Some(s)) = ends.get(&keys[ei]).map(|c| c.to)
            && (s as usize) < SITES
        {
            fixed[ei][1] = End::Site(s);
        } else if let Some(s) = side_named(&e.to_side) {
            fixed[ei][1] = End::OnSide(s);
        }
        // the axis-aligned segments of the parent, and tracks through them
        let mut segs: Vec<((f64, f64), (f64, f64))> = Vec::new();
        let (mut ex, mut ey) = (Vec::new(), Vec::new());
        for w in pp.windows(2) {
            let horiz = (w[0].1 - w[1].1).abs() < 1e-6;
            let vert = (w[0].0 - w[1].0).abs() < 1e-6;
            if horiz == vert {
                continue;
            }
            let (lo, hi) = if horiz { (w[0].0.min(w[1].0), w[0].0.max(w[1].0)) } else { (w[0].1.min(w[1].1), w[0].1.max(w[1].1)) };
            let (lo, hi) = if hi - lo >= 2.0 * margin { (lo + margin, hi - margin) } else { ((lo + hi) / 2.0, (lo + hi) / 2.0) };
            for t in [lo, (lo + hi) / 2.0, hi] {
                if horiz {
                    ex.push(t);
                } else {
                    ey.push(t);
                }
            }
            if horiz {
                ey.push(w[0].1);
            } else {
                ex.push(w[0].0);
            }
            segs.push((w[0], w[1]));
        }
        // a branch inside one panel keeps to that panel's own tracks, as its parent does
        let pe = &g.edges[parent[ei].unwrap()];
        let inside = [&pe.from, &pe.to, &e.to].iter().all(|id| g.node_index(id).is_some_and(|i| lay.nodes[i].panel == nb.panel));
        let only = (lay.panels.len() > 1 && inside).then_some(nb.panel);
        let (mut grid, walls) = make_grid(g, lay, &fixed, &ex, &ey, only);
        // the lines already drawn count as used (a branch may cross them but not run on them)
        for (ej, r) in out.iter().enumerate() {
            if ej == ei || !done[ej] || r.forced {
                continue;
            }
            let who = (g.node_index(&g.edges[ej].from).unwrap(), g.node_index(&g.edges[ej].to).unwrap());
            let (mut hs, mut vs) = (Vec::new(), Vec::new());
            along_grid(&grid, &r.points, |i| hs.push(i), |i| vs.push(i));
            for i in hs {
                grid.h_use[i].push(who);
            }
            for i in vs {
                grid.v_use[i].push(who);
            }
        }
        // the parent's own tracks are not a way for the branch
        for (p, q) in &segs {
            let (mut hs, mut vs) = (Vec::new(), Vec::new());
            along_grid(&grid, &[*p, *q], |i| hs.push(i), |i| vs.push(i));
            for i in hs {
                grid.h_block[i] = true;
            }
            for i in vs {
                grid.v_block[i] = true;
            }
        }
        // starts: a grid point on a parent segment, leaving at a right angle
        let mut starts: Vec<Port> = Vec::new();
        // the starts at the middle of each run long enough to leave mid-way, by run
        let mut mids: Vec<(f64, Vec<usize>)> = Vec::new();
        for (p, q) in &segs {
            let horiz = (p.1 - q.1).abs() < 1e-6;
            let (lo, hi) = if horiz { (p.0.min(q.0), p.0.max(q.0)) } else { (p.1.min(q.1), p.1.max(q.1)) };
            let (len, middle) = (hi - lo, (lo + hi) / 2.0);
            mids.push((len, Vec::new()));
            let (lo, hi) = if hi - lo >= 2.0 * margin { (lo + margin - 1e-6, hi - margin + 1e-6) } else { ((lo + hi) / 2.0 - 1e-6, (lo + hi) / 2.0 + 1e-6) };
            let along: &[f64] = if horiz { &grid.xs } else { &grid.ys };
            for &t in along {
                if t < lo || t > hi {
                    continue;
                }
                let pt = if horiz { (t, p.1) } else { (p.0, t) };
                if lay.nodes.iter().any(|n| n.reserve.x < pt.0 && pt.0 < n.reserve.right() && n.reserve.y < pt.1 && pt.1 < n.reserve.bottom()) {
                    continue;
                }
                let (Some(i), Some(j)) = (Grid::find(&grid.xs, pt.0), Grid::find(&grid.ys, pt.1)) else { continue };
                let sides = if horiz { [Side::Top, Side::Bottom] } else { [Side::Left, Side::Right] };
                for s in sides {
                    if len >= 2.0 * margin && (t - middle).abs() < 1e-6 {
                        mids.last_mut().unwrap().1.push(starts.len());
                    }
                    starts.push((s, pt, (i, j), 0.0));
                }
            }
        }
        // the longest run first
        mids.retain(|m| !m.1.is_empty());
        mids.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(Ordering::Equal));
        // goals: the sides of the target block
        let mut goals: Vec<Port> = Vec::new();
        let mut g_slots: Vec<usize> = Vec::new();
        let opts: Vec<(Side, usize)> = match fixed[ei][1] {
            End::Free => Side::ALL.iter().map(|s| (*s, 2)).collect(),
            End::Site(s) => sides_of(s).into_iter().map(|(sd, sl)| (sd, if nb.ellipse { 2 } else { sl })).collect(),
            End::At(sd, sl) => vec![(sd, sl)],
            End::OnSide(sd) => vec![(sd, 2)],
        };
        let from_rect = &lay.nodes[a].rect;
        let down = lay.panels[nb.panel].down;
        for (s, slot) in opts {
            let p = end_point(nb, s, slot, k);
            let esc = match s {
                Side::Left => (nb.reserve.x - m, p.1),
                Side::Right => (nb.reserve.right() + m, p.1),
                Side::Top => (p.0, nb.reserve.y - m),
                Side::Bottom => (p.0, nb.reserve.bottom() + m),
            };
            let (Some(i), Some(j)) = (Grid::find(&grid.xs, esc.0), Grid::find(&grid.ys, esc.1)) else { continue };
            if walls.iter().enumerate().any(|(wi, w)| wi != b && segment_hits(p, esc, w)) {
                continue;
            }
            let is_op = g.nodes[b].module == "op";
            let cost = side_cost(e.kind, false, s, &nb.rect, from_rect, down, false, is_op, k) + (esc.0 - p.0).abs() + (esc.1 - p.1).abs();
            goals.push((s, p, (i, j), cost));
            g_slots.push(slot);
        }
        let free_goals: Vec<usize> = (0..goals.len()).filter(|i| fixed[ei][1] != End::Free || !words_side(nb, goals[*i].0)).collect();
        // the middle of the longest run that has a way, then any point; a free side before the
        // words' side
        let all_goals: Vec<usize> = (0..goals.len()).collect();
        let all_starts: Vec<usize> = (0..starts.len()).collect();
        let mut stages: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
        for gs in [&free_goals, &all_goals] {
            stages.extend(mids.iter().map(|m| (m.1.clone(), gs.clone())));
            stages.push((all_starts.clone(), gs.clone()));
        }
        match search_preferring(&grid, &starts, &goals, (a, b), bend, k, &stages) {
            Some((path, si, gi)) => {
                let (s_side, s_pt, _, _) = starts[si];
                let (g_side, g_port, _, _) = goals[gi];
                let mut pts = vec![s_pt];
                pts.extend(path.iter().map(|(i, j)| (grid.xs[*i], grid.ys[*j])));
                pts.push(g_port);
                out[ei] = Route { edge: ei, points: simplify(pts), from_side: s_side, to_side: g_side, from_site: 0, to_site: site_of(g_side, g_slots[gi]), forced: false, warnings: Vec::new(), curve: false, branch: true };
            }
            None => {
                // nothing clear: straight from the middle of the parent's line to the nearest side
                let from = match pp.len() {
                    0 => (0.0, 0.0),
                    1 => pp[0],
                    n => ((pp[n / 2 - 1].0 + pp[n / 2].0) / 2.0, (pp[n / 2 - 1].1 + pp[n / 2].1) / 2.0),
                };
                let dist = |x: &Side| {
                    let q = nb.port(*x);
                    (from.0 - q.0).abs() + (from.1 - q.1).abs()
                };
                let t = Side::ALL.iter().copied().min_by(|s, t| dist(s).partial_cmp(&dist(t)).unwrap_or(Ordering::Equal)).unwrap_or(Side::Left);
                notes[ei].push(format!("{} → {}: 找不到乾淨的路線，岔出的線畫成直線", e.from, e.to));
                out[ei] = Route { edge: ei, points: vec![from, nb.port(t)], from_side: t.opposite(), to_side: t, from_site: 0, to_site: site_of(t, 2), forced: true, warnings: Vec::new(), curve: false, branch: true };
            }
        }
    }
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
    let (Some(node), pts) = (lay.nodes.get(a), &r.points) else { return Vec::new() };
    // first and last point plus at least one interior run: 3 runs, 4 points
    if r.forced || r.branch || r.curve || pts.len() < 4 || lay.scale <= 0.0 {
        return Vec::new();
    }
    let horizontal = |s: Side| matches!(s, Side::Left | Side::Right);
    let first = horizontal(r.from_side);
    // every run axis-aligned, longer than nothing, and alternating from the first block's side
    // (a zero-length or collinear run would be dropped by `turned`'s simplify: refuse the line)
    let ok = pts.windows(2).enumerate().all(|(j, w)| {
        let (dx, dy) = ((w[1].0 - w[0].0).abs(), (w[1].1 - w[0].1).abs());
        let hor = first ^ (j % 2 == 1);
        dx.max(dy) > 1e-6 && if hor { dy <= 1e-6 } else { dx <= 1e-6 }
    });
    let n = pts.len() - 1;
    if !ok || (first ^ ((n - 1) % 2 == 1)) != horizontal(r.to_side) {
        return Vec::new();
    }
    // `turned` wants the line to leave and arrive straight out of / into its sides
    let (fnrm, tnrm) = (r.from_side.normal(), r.to_side.normal());
    let (d0, d1) = ((pts[1].0 - pts[0].0, pts[1].1 - pts[0].1), (pts[n].0 - pts[n - 1].0, pts[n].1 - pts[n - 1].1));
    if d0.0 * fnrm.0 + d0.1 * fnrm.1 <= 1e-6 || d1.0 * tnrm.0 + d1.1 * tnrm.1 >= -1e-6 {
        return Vec::new();
    }
    let (ox, oy) = node.origin;
    (1..n - 1)
        .map(|j| {
            let vertical = !(first ^ (j % 2 == 1));
            let value = if vertical { (pts[j].0 - ox) / lay.scale } else { (pts[j].1 - oy) / lay.scale };
            Run { index: j - 1, vertical, value, a: pts[j], b: pts[j + 1] }
        })
        .collect()
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

/// The frames a line should not run along: panels (not bare) and regions.
fn frame_rects(lay: &Layout) -> Vec<Rect> {
    lay.panels.iter().filter(|p| !p.bare).map(|p| p.rect).chain(lay.regions.iter().map(|r| r.rect)).collect()
}

/// How many stretches of the lines run along a frame's border (within 4.5 pt at natural size,
/// for more than 6 pt), or turn on it: lines drawn on a frame. 0 is the aim.
pub fn hugging(lay: &Layout, routes: &[Route]) -> usize {
    let k = lay.scale;
    let near = 4.5 * k;
    let frames = frame_rects(lay);
    let mut n = 0;
    for r in routes {
        for w in r.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = (a.0 - b.0).abs() + (a.1 - b.1).abs();
            if len < 6.0 * k {
                continue;
            }
            let horizontal = (a.1 - b.1).abs() < 1e-6;
            let vertical = (a.0 - b.0).abs() < 1e-6;
            let along = frames.iter().any(|f| {
                if horizontal {
                    let over = a.0.max(b.0).min(f.right()) - a.0.min(b.0).max(f.x);
                    over > 6.0 * k && ((a.1 - f.y).abs() < near || (a.1 - f.bottom()).abs() < near)
                } else if vertical {
                    let over = a.1.max(b.1).min(f.bottom()) - a.1.min(b.1).max(f.y);
                    over > 6.0 * k && ((a.0 - f.x).abs() < near || (a.0 - f.right()).abs() < near)
                } else {
                    false
                }
            });
            if along {
                n += 1;
            }
        }
    }
    n
}

/// The routing grid of a layout: its tracks, the walls no line may pass through and the cost of
/// crossing a panel border. `extra_x` / `extra_y` are tracks to add (always kept). With `only`
/// a panel, the tracks are that panel's own (its blocks, frame, regions and titles, and a way
/// round it), so the lines inside it do not move when something changes in another panel; the
/// walls are still every block's.
fn make_grid(g: &Graph, lay: &Layout, fixed: &[[End; 2]], extra_x: &[f64], extra_y: &[f64], only: Option<usize>) -> (Grid, Vec<Rect>) {
    let k = lay.scale;
    let m = 7.0 * k; // distance of a line from a module
    let c = 3.0 * k; // clearance that counts as "through"
    let obstacles: Vec<Rect> = lay.nodes.iter().map(|n| n.reserve.inflate(c)).collect();
    let mut walls: Vec<Rect> = obstacles.clone();
    walls.extend(lay.panels.iter().filter_map(|p| p.title_rect));
    // a region's title is words: no line runs over it
    walls.extend(lay.regions.iter().filter(|r| !r.title.is_empty()).filter_map(|r| r.title_rect));
    if !lay.legend.is_empty() {
        let top = lay.legend.iter().map(|l| l.text.y).fold(f64::MAX, f64::min);
        walls.push(Rect::new(lay.bounds.x, top, lay.bounds.w, lay.bounds.bottom() - top));
    }

    // grid lines: (position, cost per length, fixed)
    const HUG: f64 = 1.06; // right beside a module
    const GAP: f64 = 0.9; // the middle of a gap between panels
    let (mut lx, mut ly): (Vec<(f64, f64, bool)>, Vec<(f64, f64, bool)>) = (Vec::new(), Vec::new());
    let home = only.map(|p| lay.panels[p].rect);
    let mine = |r: &Rect| home.is_none_or(|h| h.inflate(m).overlaps(r));
    for n in lay.nodes.iter().filter(|n| only.is_none_or(|p| n.panel == p)) {
        lx.push((n.rect.cx(), 1.0, true));
        ly.push((n.port_y, 1.0, true));
        lx.extend([(n.reserve.x - m, HUG, true), (n.reserve.right() + m, HUG, true)]);
        ly.extend([(n.reserve.y - m, HUG, true), (n.reserve.bottom() + m, HUG, true)]);
    }
    for w in walls[obstacles.len()..].iter().filter(|w| mine(w)) {
        lx.extend([(w.x - m + c, HUG, false), (w.right() + m - c, HUG, false)]);
        ly.extend([(w.y - m + c, HUG, false), (w.bottom() + m - c, HUG, false)]);
    }
    for (pi, p) in lay.panels.iter().enumerate() {
        if only.is_some_and(|o| o != pi) {
            continue;
        }
        // gaps between this panel and its neighbours, and a channel inside each border
        for q in lay.panels[pi + 1..].iter().filter(|_| only.is_none()) {
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
            lx.extend([(p.rect.x + m, 1.0, false), (p.rect.right() - m, 1.0, false)]);
            ly.extend([(p.rect.y + m, 1.0, false), (p.rect.bottom() - m, 1.0, false)]);
        }
    }
    // a lane on each side of a region's frame, clear of it
    for rb in lay.regions.iter().filter(|r| mine(&r.rect)) {
        let r = &rb.rect;
        lx.extend([(r.x - m, HUG, false), (r.right() + m, HUG, false)]);
        ly.extend([(r.y - m, HUG, false), (r.bottom() + m, HUG, false)]);
    }
    // room to go around the whole figure (round the panel, for a panel's own grid)
    let round = home.unwrap_or(lay.bounds);
    lx.extend([(round.x - 2.0 * m, 1.0, true), (round.right() + 2.0 * m, 1.0, true)]);
    ly.push((round.y - 2.0 * m, 1.0, true));
    if lay.legend.is_empty() || home.is_some() {
        ly.push((round.bottom() + 2.0 * m, 1.0, true));
    }
    // the points the lines really use on the sides
    for (ei, e) in g.edges.iter().enumerate() {
        for (end, id) in [(0, &e.from), (1, &e.to)] {
            let n = &lay.nodes[g.node_index(id).unwrap()];
            if n.ellipse || only.is_some_and(|p| n.panel != p) {
                continue;
            }
            let at: Vec<(Side, usize)> = match fixed[ei][end] {
                End::Free => vec![],
                End::Site(s) => sides_of(s),
                End::At(sd, sl) => vec![(sd, sl)],
                End::OnSide(sd) => vec![(sd, 2)],
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
    for x in extra_x {
        lx.push((*x, 1.0, true));
    }
    for y in extra_y {
        ly.push((*y, 1.0, true));
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
    // crossing a frame's border costs (a panel's or a region's), so long lines keep to the gaps
    // and stay out of modules they do not belong to; running along a border costs too, so a
    // line is never drawn on a frame
    let border = 14.0 * k;
    let near = 4.5 * k;
    for r in frame_rects(lay) {
        for (j, &y) in grid.ys.iter().enumerate() {
            if (y - r.y).abs() < near || (y - r.bottom()).abs() < near {
                for i in 0..nx - 1 {
                    let (x0, x1) = (grid.xs[i].max(r.x), grid.xs[i + 1].min(r.right()));
                    if x1 > x0 {
                        let idx = grid.hi(i, j);
                        grid.h_cost[idx] += 2.0 * (x1 - x0);
                    }
                }
            }
        }
        for (i, &x) in grid.xs.iter().enumerate() {
            if (x - r.x).abs() < near || (x - r.right()).abs() < near {
                for j in 0..ny - 1 {
                    let (y0, y1) = (grid.ys[j].max(r.y), grid.ys[j + 1].min(r.bottom()));
                    if y1 > y0 {
                        let idx = grid.vi(i, j);
                        grid.v_cost[idx] += 2.0 * (y1 - y0);
                    }
                }
            }
        }
    }
    let regions: Vec<Rect> = lay.regions.iter().map(|r| r.rect).collect();
    for r in lay.panels.iter().filter(|p| !p.bare).map(|p| &p.rect).chain(regions.iter()) {
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
    (grid, walls)
}

/// One routing pass. `fixed` says, per edge, how each end is tied to its block; `fallback` is an
/// earlier pass whose route is kept for an edge this pass cannot route.
fn run(g: &Graph, lay: &Layout, fixed: &[[End; 2]], fallback: Option<&[Route]>, skip: &[bool]) -> Vec<Route> {
    let k = lay.scale;
    let m = 7.0 * k; // distance of a line from a module
    let bend = 18.0 * k;
    let (global, walls) = make_grid(g, lay, fixed, &[], &[], None);
    // grids[0] is the whole figure's; with two or more panels, grids[1 + p] is panel p's own,
    // for the lines that stay inside it
    let mut grids = vec![global];
    if lay.panels.len() > 1 {
        grids.extend((0..lay.panels.len()).map(|p| make_grid(g, lay, fixed, &[], &[], Some(p)).0));
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
        if skip[ei] {
            out[ei] = Some(Route { edge: ei, points: Vec::new(), from_side: Side::Right, to_side: Side::Left, from_site: 0, to_site: 0, forced: false, warnings: Vec::new(), curve: false, branch: true });
            continue;
        }
        let e = &g.edges[ei];
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        let (na, nb) = (&lay.nodes[a], &lay.nodes[b]);
        let same_panel = na.panel == nb.panel;
        let down = lay.panels[na.panel].down;
        let gix = if grids.len() > 1 && same_panel { 1 + na.panel } else { 0 };
        let grid = &grids[gix];
        // candidate ports: (side, port point, escape point, grid index, cost) and the slot used
        let ports =|n: &crate::layout::Placed, other: &Rect, is_source: bool, idx: usize, end: End| -> (Vec<Port>, Vec<usize>) {
            let mut v = Vec::new();
            let mut slots = Vec::new();
            let opts: Vec<(Side, usize)> = match end {
                End::Free => Side::ALL.iter().map(|s| (*s, 2)).collect(),
                End::Site(s) => sides_of(s).into_iter().map(|(sd, sl)| (sd, if n.ellipse { 2 } else { sl })).collect(),
                End::At(sd, sl) => vec![(sd, if n.ellipse { 2 } else { sl })],
                End::OnSide(sd) => vec![(sd, 2)],
            };
            for (s, slot) in opts {
                let p = end_point(n, s, slot, k);
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
        // the sides tried first: never a side that holds words while another side has a way
        // (a free end only; the author's choice stands), and a skip over the top of its row
        let skip_up = e.kind == EdgeKind::Skip && !down && same_panel;
        let pick = |ports: &[Port], n: &Placed, end: End, no_words: bool, top: bool| -> Vec<usize> {
            (0..ports.len()).filter(|i| end != End::Free || ((!no_words || !words_side(n, ports[*i].0)) && (!top || ports[*i].0 == Side::Top))).collect()
        };
        let mut stages = Vec::new();
        if skip_up {
            stages.push((pick(&starts, na, fixed[ei][0], true, true), pick(&goals, nb, fixed[ei][1], true, true)));
        }
        stages.push((pick(&starts, na, fixed[ei][0], true, false), pick(&goals, nb, fixed[ei][1], true, false)));
        stages.push(((0..starts.len()).collect(), (0..goals.len()).collect()));
        // a words side costs nothing when the line through it runs straight to a side facing
        // it: the end stops short outside the words and the line keeps straight
        if let Some((si, gi)) = straight_through(&walls,&starts, &goals, na, nb, (a, b)) {
            let pts = vec![starts[si].1, goals[gi].1];
            for gr in grids.iter_mut() {
                let (mut hs, mut vs) = (Vec::new(), Vec::new());
                along_grid(gr, &pts, |i| hs.push(i), |i| vs.push(i));
                for i in hs {
                    gr.h_use[i].push((a, b));
                }
                for i in vs {
                    gr.v_use[i].push((a, b));
                }
            }
            let (s_side, g_side) = (starts[si].0, goals[gi].0);
            out[ei] = Some(Route { edge: ei, points: pts, from_side: s_side, to_side: g_side, from_site: site_of(s_side, s_slots[si]), to_site: site_of(g_side, g_slots[gi]), forced: false, warnings: Vec::new(), curve: false, branch: false });
            continue;
        }
        let found = search_preferring(grid, &starts, &goals, (a, b), bend, k, &stages);
        let r = match found {
            Some((path, si, gi)) => {
                let (s_side, s_port, _, _) = starts[si];
                let (g_side, g_port, _, _) = goals[gi];
                let mut pts = vec![s_port];
                pts.extend(path.iter().map(|(i, j)| (grid.xs[*i], grid.ys[*j])));
                pts.push(g_port);
                let pts = simplify(pts);
                // register usage, on the other grids by the tracks they share
                for other in grids.iter_mut().enumerate().filter(|(oi, _)| *oi != gix).map(|(_, o)| o) {
                    let (mut hs, mut vs) = (Vec::new(), Vec::new());
                    along_grid(other, &pts, |i| hs.push(i), |i| vs.push(i));
                    for i in hs {
                        other.h_use[i].push((a, b));
                    }
                    for i in vs {
                        other.v_use[i].push((a, b));
                    }
                }
                let grid = &mut grids[gix];
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
                Route { edge: ei, points: pts, from_side: s_side, to_side: g_side, from_site: site_of(s_side, s_slots[si]), to_site: site_of(g_side, g_slots[gi]), forced: false, warnings: Vec::new(), curve: false, branch: false }
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
                Route { edge: ei, points: vec![na.port(best.1), nb.port(best.2)], from_side: best.1, to_side: best.2, from_site: site_of(best.1, 2), to_site: site_of(best.2, 2), forced: true, warnings: Vec::new(), curve: false, branch: false }
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

/// A straight line (no bend) between a start and a goal on facing sides, at least one of which
/// holds words, clear of every other block and of the titles: the indices of the pair, the
/// shortest when there are several. `None` when no words side is involved (the search finds a
/// straight line by itself there) or none is straight and clear.
fn straight_through(walls:&[Rect], starts: &[Port], goals: &[Port], na: &Placed, nb: &Placed, who: (usize, usize)) -> Option<(usize, usize)> {
    let mut best: Option<(f64, usize, usize)> = None;
    for (si, s) in starts.iter().enumerate() {
        for (gi, t) in goals.iter().enumerate() {
            if t.0 != s.0.opposite() || !(words_side(na, s.0) || words_side(nb, t.0)) {
                continue;
            }
            let (p, q) = (s.1, t.1);
            let n = s.0.normal();
            let along = if n.0 == 0.0 { (p.0 - q.0).abs() } else { (p.1 - q.1).abs() };
            let ahead = (q.0 - p.0) * n.0 + (q.1 - p.1) * n.1;
            if along > 1e-6 || ahead <= 1e-6 {
                continue;
            }
            // every wall but the two blocks' own (walls start with the blocks, by node index),
            // and not through either block's own picture
            if walls.iter().enumerate().any(|(wi, w)| wi != who.0 && wi != who.1 && segment_hits(p, q, w)) || segment_hits(p, q, &na.art) || segment_hits(p, q, &nb.art) {
                continue;
            }
            if best.is_none_or(|b| ahead < b.0) {
                best = Some((ahead, si, gi));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// [`astar`] over the stages in order (each a subset of the starts and of the goals, by index):
/// the first stage with a path wins. The indices returned are into the full lists.
#[allow(clippy::too_many_arguments)]
fn search_preferring(grid: &Grid, starts: &[Port], goals: &[Port], who: (usize, usize), bend: f64, k: f64, stages: &[(Vec<usize>, Vec<usize>)]) -> Option<(Vec<(usize, usize)>, usize, usize)> {
    let mut tried: Vec<&(Vec<usize>, Vec<usize>)> = Vec::new();
    for st in stages {
        if st.0.is_empty() || st.1.is_empty() || tried.contains(&st) {
            continue;
        }
        tried.push(st);
        let s: Vec<Port> = st.0.iter().map(|i| starts[*i]).collect();
        let g: Vec<Port> = st.1.iter().map(|i| goals[*i]).collect();
        if let Some((path, si, gi)) = astar(grid, &s, &g, who, bend, k) {
            return Some((path, st.0[si], st.1[gi]));
        }
    }
    None
}

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
                assert!(on_boundary(*r.points.last().unwrap(), &moved.layout.nodes[1].art), "{:?}", r.points);
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
        // neither source is level with c (c stands in a lane of its own)
        const UNLEVEL: &str = "fig t\na: box \"A\"\nb: box \"B\"\nc: box \"C\" lane=1\na -> c\nb -> c\n";
        let (g, f) = draw_plain(UNLEVEL);
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
        // b is not level with c (a is, and keeps the straight line), so b's line has a run to move
        let (g, base) = draw_plain(TWO_IN);
        let (a, c) = (&base.layout.nodes[1].rect, &base.layout.nodes[2].rect);
        let x = a.right() + 0.7 * (c.x - a.right());
        let (_, f) = draw_with(TWO_IN, &[], &[("b>c", ends(None, None, &[natural(&base, x, false)]))]);
        let r = edge(&g, &f, "b", "c");
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
        // a is level with c and keeps the middle (1); b takes the upper quarter (15)
        assert_eq!(seen, [1, 15]);
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

    fn synthetic(points: &[(f64, f64)], from: Side, to: Side, forced: bool) -> Route {
        Route { edge: 0, points: points.to_vec(), from_side: from, to_side: to, from_site: 3, to_site: 1, forced, warnings: vec![], curve: false, branch: false }
    }

    const Z: [(f64, f64); 4] = [(10.0, 10.0), (30.0, 10.0), (30.0, 40.0), (60.0, 40.0)];

    const RUN_SRCS: [&str; 3] = [
        "fig s\na: box \"A\"\nb: box \"Wide middle step\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c -> d\na -> d : skip\nd -> a : feedback\na -> c : optional\n",
        "fig f\na: image \"in\"\nb: box \"B\"\nc: box \"C\"\nd: op \"+\"\ne: box \"E\"\nf: loss \"L\"\ng: image \"gt\" role=gt\na -> b -> d -> e -> f\na -> c -> d\ng -> f : supervision\nf -> b : feedback\nf -> c : feedback\nb -> e : skip\n",
        "fig t\na: box \"A\"\nb: box \"B\"\nd: box \"D\"\nc: box \"C\"\na -> c\nb -> c\nd -> c\n",
    ];

    fn ends_of(g: &Graph, r: &Route) -> (usize, usize) {
        let e = &g.edges[r.edge];
        (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap())
    }

    #[test]
    fn runs_of_a_z_line_is_its_middle_run() {
        let (_, f) = draw_plain(CHAIN);
        let lay = &f.layout;
        let v = runs(lay, 0, &synthetic(&Z, Side::Right, Side::Left, false));
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].index, v[0].vertical, v[0].a, v[0].b), (0, true, (30.0, 10.0), (30.0, 40.0)));
        assert!((v[0].value - (30.0 - lay.nodes[0].origin.0) / lay.scale).abs() < 1e-9);
    }

    #[test]
    fn runs_of_a_straight_line_and_of_one_corner_are_empty() {
        let (_, f) = draw_plain(CHAIN);
        let lay = &f.layout;
        let none = |pts: &[(f64, f64)], a: usize, from: Side, to: Side| runs(lay, a, &synthetic(pts, from, to, false)).is_empty();
        assert!(none(&[(10.0, 10.0), (60.0, 10.0)], 0, Side::Right, Side::Left));
        assert!(none(&[(10.0, 10.0), (60.0, 10.0), (60.0, 40.0)], 0, Side::Right, Side::Top));
        // no panic on odd input
        assert!(none(&[], 0, Side::Right, Side::Left));
        assert!(none(&[(1.0, 1.0)], 0, Side::Right, Side::Left));
        assert!(none(&Z, 99, Side::Right, Side::Left));
        // a diagonal, a zero-length run, a wrong first orientation
        assert!(none(&[(10.0, 10.0), (30.0, 12.0), (30.0, 40.0), (60.0, 40.0)], 0, Side::Right, Side::Left));
        assert!(none(&[(10.0, 10.0), (30.0, 10.0), (30.0, 10.0), (30.0, 40.0), (60.0, 40.0)], 0, Side::Right, Side::Left));
        assert!(none(&Z, 0, Side::Bottom, Side::Left));
        // leaves backwards
        assert!(none(&Z, 0, Side::Left, Side::Left));
    }

    #[test]
    fn runs_of_a_forced_line_are_empty() {
        let (_, f) = draw_plain(CHAIN);
        assert!(runs(&f.layout, 0, &synthetic(&Z, Side::Right, Side::Left, true)).is_empty());
    }

    #[test]
    fn runs_written_back_as_turns_give_the_same_points() {
        let mut seen = 0;
        for src in RUN_SRCS {
            let (g, f) = draw_plain(src);
            for r in &f.routes {
                let (a, b) = ends_of(&g, r);
                let v = runs(&f.layout, a, r);
                if v.is_empty() {
                    continue;
                }
                seen += 1;
                assert!(v.iter().enumerate().all(|(i, u)| u.index == i));
                let turns: Vec<f64> = v.iter().map(|u| u.value).collect();
                let back = turned(&f.layout, a, b, r, &turns).unwrap_or_else(|| panic!("{:?}", r.points));
                assert_eq!(back.len(), r.points.len(), "{back:?} vs {:?}", r.points);
                assert!(back.iter().zip(&r.points).all(|(x, y)| near(*x, *y)), "{back:?} vs {:?}", r.points);
                assert!(v.iter().all(|u| r.points.windows(2).any(|w| near(w[0], u.a) && near(w[1], u.b))));
            }
        }
        assert!(seen >= 3, "only {seen} lines with runs");
    }

    #[test]
    fn sliding_one_run_moves_only_that_run() {
        let mut checked = 0;
        for src in RUN_SRCS {
            let (g, f) = draw_plain(src);
            for r in &f.routes {
                let (a, b) = ends_of(&g, r);
                let v = runs(&f.layout, a, r);
                for i in 0..v.len() {
                    let mut t: Vec<f64> = v.iter().map(|u| u.value).collect();
                    t[i] += 0.5;
                    let Some(p) = turned(&f.layout, a, b, r, &t) else { continue };
                    let w = runs(&f.layout, a, &Route { points: p, ..r.clone() });
                    if w.len() != v.len() {
                        continue;
                    }
                    checked += 1;
                    for (j, (x, y)) in w.iter().zip(&v).enumerate() {
                        let want = if j == i { y.value + 0.5 } else { y.value };
                        assert!((x.value - want).abs() < 1e-6, "run {j} of {:?}", r.points);
                    }
                }
            }
        }
        assert!(checked >= 1);
    }

    #[test]
    fn runs_respect_a_scale_other_than_one() {
        let g = dsl::parse(TWO_IN).unwrap();
        let small = LayoutOptions { frame: Rect::new(0.0, 0.0, 120.0, 80.0), ..Default::default() };
        let f = crate::draw(&g, &Drawing { options: small, ..Default::default() }).unwrap();
        assert!((f.layout.scale - 1.0).abs() > 1e-3, "scale {}", f.layout.scale);
        let r = edge(&g, &f, "b", "c");
        let (a, b) = ends_of(&g, &r);
        let v = runs(&f.layout, a, &r);
        assert!(!v.is_empty(), "{:?}", r.points);
        let turns: Vec<f64> = v.iter().map(|u| u.value).collect();
        let back = turned(&f.layout, a, b, &r, &turns).unwrap();
        assert!(back.iter().zip(&r.points).all(|(x, y)| near(*x, *y)));
        let (ox, oy) = f.layout.nodes[a].origin;
        let u = &v[0];
        let want = if u.vertical { (u.a.0 - ox) / f.layout.scale } else { (u.a.1 - oy) / f.layout.scale };
        assert!((u.value - want).abs() < 1e-9);
    }

    #[test]
    fn runs_follow_turns_the_author_set() {
        let (g, base) = draw_plain(TWO_IN);
        let (a, c) = (&base.layout.nodes[1].rect, &base.layout.nodes[2].rect);
        let x = a.right() + 0.7 * (c.x - a.right());
        let t = natural(&base, x, false);
        let (_, f) = draw_with(TWO_IN, &[], &[("b>c", ends(None, None, &[t]))]);
        let r = edge(&g, &f, "b", "c");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let (a, _) = ends_of(&g, &r);
        let v = runs(&f.layout, a, &r);
        assert_eq!(v.len(), 1);
        assert!(v[0].vertical && (v[0].value - t).abs() < 1e-6, "{v:?} vs {t}");
    }

    // ---- lines that leave the middle of another line, and curved lines ----

    fn draw_graph(g: &Graph, place: &[(&str, [f64; 2])]) -> Figure {
        let options = LayoutOptions { place: place.iter().map(|(k, v)| (k.to_string(), *v)).collect(), ..Default::default() };
        crate::draw(g, &Drawing { options, ..Default::default() }).unwrap()
    }

    /// a -> b -> c in a row and d; `d` is fed from the line a>b (when `off` says so).
    const BRANCH_SRC: &str = "fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c\na -> d\n";

    fn branch_graph(off: &str) -> Graph {
        let mut g = dsl::parse(BRANCH_SRC).unwrap();
        let i = g.edges.iter().position(|e| e.to == "d").unwrap();
        g.edges[i].off = off.to_string();
        g
    }

    fn branch_of(g: &Graph, f: &Figure) -> Route {
        f.routes.iter().find(|r| g.edges[r.edge].to == "d").unwrap().clone()
    }

    fn on_path(p: (f64, f64), path: &[(f64, f64)]) -> bool {
        path.windows(2).any(|w| p.0 >= w[0].0.min(w[1].0) - 1e-6 && p.0 <= w[0].0.max(w[1].0) + 1e-6 && p.1 >= w[0].1.min(w[1].1) - 1e-6 && p.1 <= w[0].1.max(w[1].1) + 1e-6)
    }

    const PLACE_D: [(&str, [f64; 2]); 1] = [("d", [40.0, 80.0])];

    #[test]
    fn a_branch_starts_on_its_parent_line() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &PLACE_D);
        let r = branch_of(&g, &f);
        assert!(r.branch && !r.forced, "{r:?}");
        assert_eq!(r.from_site, 0);
        let parent = edge(&g, &f, "a", "b");
        assert!(on_path(r.points[0], &parent.points), "{:?} not on {:?}", r.points[0], parent.points);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn a_branch_ends_at_its_block() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &PLACE_D);
        let r = branch_of(&g, &f);
        let d = g.node_index("d").unwrap();
        assert!(on_boundary(*r.points.last().unwrap(), &f.layout.nodes[d].art), "{:?} art {:?} rect {:?}", r.points, f.layout.nodes[d].art, f.layout.nodes[d].rect);
    }

    #[test]
    fn a_branch_leaves_at_a_right_angle() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &PLACE_D);
        let r = branch_of(&g, &f);
        let parent = edge(&g, &f, "a", "b");
        let seg = parent.points.windows(2).find(|w| on_path(r.points[0], &[w[0], w[1]])).unwrap();
        let along = if (seg[0].1 - seg[1].1).abs() < 1e-6 { (1.0, 0.0) } else { (0.0, 1.0) };
        let first = (r.points[1].0 - r.points[0].0, r.points[1].1 - r.points[0].1);
        assert!(first.0 * along.0 + first.1 * along.1 < 1e-6 && first.0.abs() + first.1.abs() > 1e-6, "{:?}", r.points);
        let n = r.from_side.normal();
        assert!(first.0 * n.0 + first.1 * n.1 > 0.0, "side {:?} vs {first:?}", r.from_side);
    }

    #[test]
    fn a_branch_does_not_start_inside_a_block() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &PLACE_D);
        let r = branch_of(&g, &f);
        let p = r.points[0];
        for n in &f.layout.nodes {
            assert!(!(p.0 > n.rect.x + 1e-6 && p.0 < n.rect.right() - 1e-6 && p.1 > n.rect.y + 1e-6 && p.1 < n.rect.bottom() - 1e-6), "{p:?} in {:?}", n.rect);
        }
        let d = g.node_index("d").unwrap();
        for (ni, n) in f.layout.nodes.iter().enumerate().filter(|(i, _)| *i != d) {
            assert!(!r.points.windows(2).any(|w| segment_hits(w[0], w[1], &n.rect)), "cuts block {ni}: {:?}", r.points);
        }
    }

    #[test]
    fn a_branch_follows_when_its_parent_is_moved() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &[("d", [40.0, 80.0]), ("b", [60.0, 30.0])]);
        let r = branch_of(&g, &f);
        let parent = edge(&g, &f, "a", "b");
        assert!(on_path(r.points[0], &parent.points), "{:?} not on the moved parent {:?}", r.points[0], parent.points);
        let plain = draw_graph(&g, &PLACE_D);
        assert_ne!(edge(&g, &plain, "a", "b").points, parent.points);
    }

    #[test]
    fn a_branch_off_a_branch() {
        let mut g = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\ne: box \"E\"\na -> b -> c\na -> d\nd -> e\n").unwrap();
        let ia = g.edges.iter().position(|e| e.to == "d").unwrap();
        let ib = g.edges.iter().position(|e| e.from == "d").unwrap();
        g.edges[ia].off = "a>b".into();
        g.edges[ib].off = "a>d".into();
        let f = draw_graph(&g, &[("d", [40.0, 80.0]), ("e", [90.0, 80.0])]);
        let (r1, r2) = (&f.routes[ia], &f.routes[ib]);
        assert!(r1.branch && r2.branch && !r1.forced && !r2.forced, "{r1:?} {r2:?}");
        assert!(on_path(r2.points[0], &r1.points), "{:?} not on {:?}", r2.points[0], r1.points);
        let e = g.node_index("e").unwrap();
        assert!(on_boundary(*r2.points.last().unwrap(), &f.layout.nodes[e].art));
        assert_eq!(junctions(&g, &f.routes).len(), 2);
    }

    #[test]
    fn an_unknown_parent_is_an_ordinary_line_and_a_warning() {
        // a figure whose line names a line it does not have is refused when it is checked, so
        // the router is asked directly here: it draws an ordinary line and says so
        let g = branch_graph("x>y");
        assert!(crate::draw(&g, &Drawing::default()).is_err(), "validation names the line");
        let mut plain = g.clone();
        plain.edges.iter_mut().for_each(|e| e.off.clear());
        let lay = draw_graph(&plain, &PLACE_D).layout;
        let routes = route_with(&g, &lay, &BTreeMap::new());
        let r = routes.iter().find(|r| g.edges[r.edge].to == "d").unwrap();
        let (a, d) = (g.node_index("a").unwrap(), g.node_index("d").unwrap());
        assert!(!r.branch);
        assert!(on_boundary(r.points[0], &lay.nodes[a].art));
        assert!(on_boundary(*r.points.last().unwrap(), &lay.nodes[d].art));
        assert!(r.warnings.iter().any(|w| w.contains("找不到要岔出的線")), "{:?}", r.warnings);
        // a cycle passes the check and is the same
        let mut g = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\na -> b\nb -> c\n").unwrap();
        g.edges[0].off = "b>c".into();
        g.edges[1].off = "a>b".into();
        let f = draw_graph(&g, &[]);
        assert!(f.routes.iter().all(|r| !r.branch && r.warnings.iter().any(|w| w.contains("找不到要岔出的線"))));
        assert!(junctions(&g, &f.routes).is_empty());
    }

    #[test]
    fn a_branch_has_no_slidable_runs() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &PLACE_D);
        let r = branch_of(&g, &f);
        let a = g.node_index("a").unwrap();
        assert!(runs(&f.layout, a, &r).is_empty());
        let mut z = synthetic(&Z, Side::Right, Side::Left, false);
        assert!(!runs(&f.layout, 0, &z).is_empty());
        z.branch = true;
        assert!(runs(&f.layout, 0, &z).is_empty());
    }

    #[test]
    fn junctions_are_the_branch_points() {
        let g = branch_graph("a>b");
        let f = draw_graph(&g, &PLACE_D);
        let r = branch_of(&g, &f);
        assert_eq!(junctions(&g, &f.routes), vec![r.points[0]]);
        let plain = dsl::parse(BRANCH_SRC).unwrap();
        let f = draw_graph(&plain, &[]);
        assert!(junctions(&plain, &f.routes).is_empty());
    }

    fn curved_graph() -> Graph {
        let mut g = dsl::parse(CHAIN).unwrap();
        g.edges[0].curve = true;
        g
    }

    #[test]
    fn a_curved_line_keeps_its_ends_and_says_curve() {
        let f = draw_graph(&curved_graph(), &[]);
        let plain = draw_plain(CHAIN).1;
        let (r, p) = (&f.routes[0], &plain.routes[0]);
        assert!(r.curve && !f.routes[1].curve);
        assert_eq!((&r.points, r.from_side, r.to_side, r.from_site, r.to_site), (&p.points, p.from_side, p.to_side, p.from_site, p.to_site));
    }

    #[test]
    fn the_curve_leaves_and_arrives_along_the_sides() {
        let f = draw_graph(&curved_graph(), &[]);
        let r = &f.routes[0];
        let c = curve_path(r).unwrap();
        assert_eq!(c.len(), 4);
        assert!(near(c[0], r.points[0]) && near(c[3], *r.points.last().unwrap()));
        let (n0, n3) = (r.from_side.normal(), r.to_side.normal());
        let (d0, d3) = ((c[1].0 - c[0].0, c[1].1 - c[0].1), (c[3].0 - c[2].0, c[3].1 - c[2].1));
        assert!(d0.0 * n0.1 - d0.1 * n0.0 < 1e-9 && d0.0 * n0.0 + d0.1 * n0.1 > 0.0);
        assert!(d3.0 * n3.1 - d3.1 * n3.0 < 1e-9 && d3.0 * n3.0 + d3.1 * n3.1 < 0.0);
        let dist = ((c[3].0 - c[0].0).powi(2) + (c[3].1 - c[0].1).powi(2)).sqrt();
        let len = (d0.0 * d0.0 + d0.1 * d0.1).sqrt();
        assert!((len - (0.4 * dist).max(12.0)).abs() < 1e-6);
        let mut s = synthetic(&[(0.0, 0.0), (10.0, 0.0)], Side::Right, Side::Left, false);
        s.curve = true;
        let c = curve_path(&s).unwrap();
        assert!((c[1].0 - 12.0).abs() < 1e-9 && (c[2].0 + 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_plain_line_has_no_curve_path() {
        let f = draw_plain(CHAIN).1;
        assert!(f.routes.iter().all(|r| curve_path(r).is_none()));
        let mut forced = synthetic(&Z, Side::Right, Side::Left, true);
        forced.curve = true;
        assert!(curve_path(&forced).is_none());
    }

    #[test]
    fn equal_routes_prefer_fewer_bends() {
        // an empty 3 x 3 grid: from the right of (0,0) to the top of (2,2) the L (one corner)
        // and every staircase are equally long; the search must take the L
        let (nx, ny) = (3, 3);
        let grid = Grid {
            xs: vec![0.0, 10.0, 20.0],
            ys: vec![0.0, 10.0, 20.0],
            xw: vec![1.0; nx],
            yw: vec![1.0; ny],
            h_block: vec![false; (nx - 1) * ny],
            v_block: vec![false; nx * (ny - 1)],
            h_cost: vec![0.0; (nx - 1) * ny],
            v_cost: vec![0.0; nx * (ny - 1)],
            h_use: vec![Vec::new(); (nx - 1) * ny],
            v_use: vec![Vec::new(); nx * (ny - 1)],
            h_through: vec![0; nx * ny],
            v_through: vec![0; nx * ny],
        };
        let start: Port = (Side::Right, (-5.0, 0.0), (0, 0), 0.0);
        let goal: Port = (Side::Top, (20.0, 25.0), (2, 2), 0.0);
        let (path, _, _) = astar(&grid, &[start], &[goal], (0, 1), 18.0, 1.0).unwrap();
        let pts = simplify(path.iter().map(|(i, j)| (grid.xs[*i], grid.ys[*j])).collect());
        assert_eq!(pts, vec![(0.0, 0.0), (20.0, 0.0), (20.0, 20.0)], "{pts:?}");
    }

    #[test]
    fn routes_without_the_new_fields_are_what_they_were() {
        // captured from the router before this change
        let pts = |s: &str, i: usize| -> Vec<(f64, f64)> { draw_plain(s).1.routes[i].points.clone() };
        assert_eq!(pts(CHAIN, 0), vec![(291.0, 202.5), (330.0, 202.5)]);
        assert_eq!(pts(CHAIN, 1), vec![(390.0, 202.5), (429.0, 202.5)]);
        // changed on purpose: a is level with c, so its line is straight from the middle
        assert_eq!(pts(TWO_IN, 0), vec![(340.5, 234.0), (379.5, 234.0)]);
        assert_eq!(pts(TWO_IN, 1), vec![(340.5, 171.0), (360.0, 171.0), (360.0, 224.25), (379.5, 224.25)]);
        let f = draw_plain(RUN_SRCS[0]).1;
        let want: [(usize, &[(f64, f64)]); 3] = [
            // level now: straight from the middle
            (0, &[(197.115, 202.5), (236.115, 202.5)]),
            (3, &[(167.115, 183.0), (167.115, 172.5), (552.885, 172.5), (552.885, 183.0)]),
            (4, &[(552.885, 222.0), (552.885, 246.0), (167.115, 246.0), (167.115, 222.0)]),
        ];
        for (i, p) in want {
            assert!(f.routes[i].points.len() == p.len() && f.routes[i].points.iter().zip(p).all(|(a, b)| near(*a, *b)), "{i}: {:?}", f.routes[i].points);
        }
        assert!(f.routes.iter().all(|r| !r.branch && !r.curve));
    }
}
