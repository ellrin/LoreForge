//! L2, part two — the lines. Orthogonal routes on a sparse grid (A*):
//! no segment passes through a module, a panel title or the legend; feedback
//! leaves and enters at the bottom and runs under the row; lines between
//! panels prefer the gaps (crossing a panel border costs); lines that share
//! neither end do not run on top of each other.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::layout::{Layout, Rect};
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
    /// No clear path was found; the line is drawn straight.
    pub forced: bool,
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

/// Route every edge of the graph over a finished layout.
pub fn route(g: &Graph, lay: &Layout) -> Vec<Route> {
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
        // candidate ports: (side, port point, escape point, grid index, cost)
        let ports = |n: &crate::layout::Placed, other: &Rect, is_source: bool, idx: usize| -> Vec<(Side, (f64, f64), (usize, usize), f64)> {
            let mut v = Vec::new();
            for s in Side::ALL {
                let p = n.port(s);
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
            }
            v
        };
        let starts = ports(na, &nb.rect, true, a);
        let goals = ports(nb, &na.rect, false, b);
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
                Route { edge: ei, points: simplify(pts), from_side: s_side, to_side: g_side, forced: false }
            }
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
                Route { edge: ei, points: vec![na.port(best.1), nb.port(best.2)], from_side: best.1, to_side: best.2, forced: true }
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
