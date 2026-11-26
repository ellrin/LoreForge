//! The baseplate: a figure is built on a grid of cells whose size is fixed before anything is
//! drawn (`docs/flow-blocks.md` §10, the author's layered build).
//!
//! A [`Format`] is chosen up front: a width on the page and any ratio. The ratio fixes the
//! plate's shape; the plate is that ratio times the smallest whole multiplier that holds the
//! figure, so blocks are as large as the shape allows. How small the text then is on the page
//! (the width divided by the cells across) is reported, not a reason to fail.
//! Blocks take whole cells (their picture, their words and a margin); rows and columns of the
//! plate are sized in whole cells and decide what is aligned with what; the room between them
//! is sized by the lines that have to pass. Text and lines are not bound to the grid.
//!
//! Layers, in build order: plate → base areas (panels) → modules' frames and lines → blocks →
//! words. This module measures and places; routing, drawing and the look checks stay where they
//! are (`route`, `build`, `look`).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::layout::{self, Layout, PanelBox, Placed, Rect, RegionBox};
use crate::modules::{self, Body, Ctx};
use crate::spec::{EdgeKind, Graph, Node, Panel};
use crate::text::{self, NoMath};

/// A page format: two independent settings the author gives before anything is drawn. Any width
/// and any ratio are valid (two columns at 1:1, one column at 16:9 …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    /// Width of the figure on the page, in points (one column, two columns, or any width).
    pub width_pt: f64,
    /// Width : height. It fixes the plate's SHAPE only, not how much fits: the plate is this
    /// ratio times the smallest whole multiplier that holds the figure.
    pub ratio: (u32, u32),
    /// Text smaller than this on the page is reported (the figure is still drawn).
    pub min_text_pt: f64,
}

impl Format {
    pub fn new(width_pt: f64, ratio: (u32, u32)) -> Format {
        Format { width_pt, ratio: reduce(ratio), min_text_pt: 6.0 }
    }
    /// The plate for a multiplier: cells across and down.
    pub fn plate(&self, k: u32) -> (u32, u32) {
        let r = reduce(self.ratio);
        (r.0 * k.max(1), r.1 * k.max(1))
    }
    /// The smallest multiplier whose plate is at least `need` cells across and down.
    pub fn multiplier(&self, need: (u32, u32)) -> u32 {
        let r = reduce(self.ratio);
        need.0.div_ceil(r.0).max(need.1.div_ceil(r.1)).max(1)
    }
    /// Size of one cell on the page for a plate `cols` cells across, in points.
    pub fn cell_pt(&self, cols: u32) -> f64 {
        self.width_pt / cols.max(1) as f64
    }
}

/// A ratio in lowest terms (16:9 stays, 32:18 becomes 16:9, 0 counts as 1).
pub fn reduce(r: (u32, u32)) -> (u32, u32) {
    let (a, b) = (r.0.max(1), r.1.max(1));
    let mut g = (a, b);
    while g.1 != 0 {
        g = (g.1, g.0 % g.1);
    }
    (a / g.0, b / g.0)
}

/// A rectangle of cells: column and row of its top-left cell, cells across and down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cells {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What one block needs on the plate, in cells: its picture, its words and its margin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    /// Index of the node in the graph.
    pub node: usize,
    pub w: u32,
    pub h: u32,
    /// How much it may be enlarged to fill spare room (1.0 = a block whose size is its words).
    pub grow: f64,
}

/// The area a figure needs, in cells, before it is placed: the sum the author asked to see.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Budget {
    /// Cells of the blocks' footprints.
    pub blocks: u32,
    /// Cells of the margins inside base areas and between them.
    pub margins: u32,
    /// Cells of the channels lines run in (from how many lines pass between neighbours).
    pub channels: u32,
    /// The least plate that holds an arrangement: cells across and down.
    pub least: (u32, u32),
}

/// One way of arranging the figure on the plate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Which arrangement this is, for the author: `left-to-right`, `top-to-bottom`, `two-rows` …
    pub arrangement: String,
    /// The plate: cells across and down.
    pub plate: (u32, u32),
    /// Widths of the plate's columns and of the gaps between them, in cells (alternating: column,
    /// gap, column …), and the same for rows. Columns and rows decide alignment.
    pub columns: Vec<u32>,
    pub rows: Vec<u32>,
    /// Where each block sits (same order as the graph's nodes).
    pub blocks: Vec<Cells>,
    /// Where each base area (panel) sits (same order as the layout's panels).
    pub areas: Vec<Cells>,
    pub budget: Budget,
    /// Cells of the plate left empty, as a share (0..1).
    pub empty: f64,
    /// The smallest text on the page for this plate and the format's width, in points.
    pub text_pt: f64,
    /// When `text_pt` is under the format's limit: what would help (a wider width, labels that
    /// could break, less detail). Empty when the text is large enough.
    pub ways: Vec<String>,
}

/// Everything `place` needs to measure blocks (the same things `layout::layout_with` takes).
pub struct Measure<'a> {
    pub theme: &'a crate::theme::Theme,
    pub vars: &'a crate::text::Vars,
    pub library: &'a crate::modules::Library,
    pub image_size: &'a dyn Fn(&str) -> Option<(u32, u32)>,
}

/// One cell of the plate in natural points (the theme's font size is 11 pt natural). The cell is
/// the grid's grain only: how large it prints follows from the plate (`Format::cell_pt`).
pub const CELL: f64 = 4.0;
/// Room one line needs in a channel: the router keeps a line 7 pt off a frame, so `n` lines in
/// one channel need `7 (n + 1)`.
const LINE_ROOM: f64 = 7.0;
/// The most a picture-like block is enlarged to fill spare room (its words keep their size).
pub const GROW_MAX: f64 = 2.0;
/// Enlargements tried: 1.00, 1.05 … GROW_MAX (a coarse ladder, so `to_layout` finds the same).
const GROW_STEPS: usize = 21;
/// Margin inside every framed panel, between its blocks' footprints and its frame (cells; each
/// footprint has its own one-cell margin besides). A title adds its row on top.
pub const FRAME_PAD: u32 = 1;
/// Arrangements worked out in full (the rest are ranked on their area alone).
const KEEP: usize = 8;
/// At most this many arrangements are offered to the user.
pub const OPTIONS: usize = 4;

fn grow_at(i: usize) -> f64 {
    ((100 + 5 * i) as f64 / 100.0).min(GROW_MAX)
}

/// Whole cells that hold `v` natural points.
fn cells(v: f64) -> u32 {
    if v <= 1e-6 { 0 } else { (v / CELL - 1e-6).ceil() as u32 }
}

/// A panel as the layout makes it (the loose nodes form a bare panel `_`, first or last), so the
/// areas come in the same order as the layout's panels.
struct Def {
    id: String,
    title: String,
    bare: bool,
    down: bool,
    at: String,
    nodes: Vec<usize>,
}

fn defs(g: &Graph) -> Vec<Def> {
    let mut panels: Vec<Panel> = g.panels.clone();
    let loose: Vec<usize> = (0..g.nodes.len()).filter(|i| g.nodes[*i].panel.is_empty()).collect();
    if !loose.is_empty() {
        let p = Panel { id: "_".into(), bare: true, ..Default::default() };
        let feeds = g.edges.iter().filter(|e| g.node(&e.from).is_some_and(|n| n.panel.is_empty()) && g.node(&e.to).is_some_and(|n| !n.panel.is_empty())).count();
        let fed = g.edges.iter().filter(|e| g.node(&e.to).is_some_and(|n| n.panel.is_empty()) && g.node(&e.from).is_some_and(|n| !n.panel.is_empty())).count();
        if g.panels.is_empty() || feeds >= fed {
            panels.insert(0, p);
        } else {
            panels.push(p);
        }
    }
    panels
        .into_iter()
        .map(|p| {
            let nodes: Vec<usize> = (0..g.nodes.len()).filter(|i| if p.id == "_" { g.nodes[*i].panel.is_empty() } else { g.nodes[*i].panel == p.id }).collect();
            Def { bare: p.bare || p.id == "_", down: p.dir == "down", title: p.title.clone(), at: p.title_at.clone(), id: p.id, nodes }
        })
        .collect()
}

/// How far a block reaches from the middle of its picture (left, right) and from its port line
/// (up, down), in cells, a one-cell margin included. Columns align on the pictures' middles and
/// rows on the port lines, as in `layout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Ext {
    lf: u32,
    rt: u32,
    up: u32,
    dn: u32,
}

impl Ext {
    fn w(&self) -> u32 {
        self.lf + self.rt
    }
    fn h(&self) -> u32 {
        self.up + self.dn
    }
    fn within(&self, o: &Ext) -> bool {
        self.lf <= o.lf && self.rt <= o.rt && self.up <= o.up && self.dn <= o.dn
    }
}

fn ext_of(b: &Body) -> Ext {
    let ax = b.art.0 + b.art.2 / 2.0;
    let half = b.reserve_w / 2.0;
    let l = ax.max(half - (b.w / 2.0 - ax));
    let r = (b.w - ax).max(half + (b.w / 2.0 - ax));
    Ext { lf: cells(l) + 1, rt: cells(r) + 1, up: cells(b.cy) + 1, dn: cells(b.h - b.cy) + 1 }
}

/// Every block measured once (through `modules::make`, as `layout` and `build` do), and again at
/// each enlargement asked for.
struct Meas<'a> {
    g: &'a Graph,
    m: &'a Measure<'a>,
    defs: Vec<Def>,
    panel_of: Vec<usize>,
    bodies: Vec<Body>,
    ext: Vec<Ext>,
    growable: Vec<bool>,
    grown: RefCell<BTreeMap<(usize, usize), Ext>>,
    /// Smallest text in the drawing, natural points.
    text_nat: f64,
}

fn body_at(n: &Node, step: usize, down: bool, m: &Measure) -> Body {
    let mut n = n.clone();
    n.params.remove("grow");
    if step > 0 {
        n.params.insert("grow".into(), serde_json::json!(grow_at(step)));
    }
    modules::make(&n, &Ctx { theme: m.theme, vars: m.vars, math: &NoMath, lib: m.library, k: 1.0, symmetric: !down, image_size: m.image_size })
}

impl<'a> Meas<'a> {
    fn new(g: &'a Graph, m: &'a Measure<'a>) -> Meas<'a> {
        let defs = defs(g);
        let mut panel_of = vec![0; g.nodes.len()];
        for (pi, d) in defs.iter().enumerate() {
            for &n in &d.nodes {
                panel_of[n] = pi;
            }
        }
        let bodies: Vec<Body> = g.nodes.iter().enumerate().map(|(i, n)| body_at(n, 0, defs[panel_of[i]].down, m)).collect();
        let ext = bodies.iter().map(ext_of).collect();
        let growable = g.nodes.iter().map(modules::growable).collect();
        let mut text_nat = f64::MAX;
        for b in &bodies {
            let t = crate::look::smallest_text(&b.objs, 1.0, 1.0);
            if t > 0.0 {
                text_nat = text_nat.min(t);
            }
        }
        if g.edges.iter().any(|e| !e.label.is_empty()) {
            text_nat = text_nat.min(m.theme.small_size());
        }
        if defs.iter().any(|d| !d.title.is_empty() && !d.bare && d.at != "none") {
            text_nat = text_nat.min(m.theme.title_size());
        }
        if text_nat == f64::MAX {
            text_nat = m.theme.font.size;
        }
        Meas { g, m, defs, panel_of, bodies, ext, growable, grown: RefCell::new(BTreeMap::new()), text_nat }
    }

    fn ext_at(&self, n: usize, step: usize) -> Ext {
        if step == 0 || !self.growable[n] {
            return self.ext[n];
        }
        if let Some(e) = self.grown.borrow().get(&(n, step)) {
            return *e;
        }
        let e = ext_of(&body_at(&self.g.nodes[n], step, self.defs[self.panel_of[n]].down, self.m));
        self.grown.borrow_mut().insert((n, step), e);
        e
    }

    /// The largest enlargement of a block that `fits` (step 0 when none does).
    fn grow_step(&self, n: usize, fits: impl Fn(&Ext) -> bool) -> usize {
        if !self.growable[n] {
            return 0;
        }
        let (mut lo, mut hi) = (0usize, GROW_STEPS - 1);
        if fits(&self.ext_at(n, hi)) {
            return hi;
        }
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if fits(&self.ext_at(n, mid)) { lo = mid } else { hi = mid }
        }
        lo
    }
}

/// One base area's grid: the columns and rows its blocks sit in, the gaps between them and its
/// padding, all in cells.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Grid {
    col: BTreeMap<usize, usize>,
    row: BTreeMap<usize, usize>,
    lf: Vec<u32>,
    rt: Vec<u32>,
    up: Vec<u32>,
    dn: Vec<u32>,
    cgap: Vec<u32>,
    rgap: Vec<u32>,
    /// left, top, right, bottom (title band included)
    pad: [u32; 4],
    /// Cells above and below the frame taken by a title written outside it.
    out_top: u32,
    out_bottom: u32,
    folded: bool,
}

impl Grid {
    fn w(&self) -> u32 {
        self.pad[0] + self.pad[2] + self.lf.iter().sum::<u32>() + self.rt.iter().sum::<u32>() + self.cgap.iter().sum::<u32>()
    }
    fn h(&self) -> u32 {
        self.pad[1] + self.pad[3] + self.up.iter().sum::<u32>() + self.dn.iter().sum::<u32>() + self.rgap.iter().sum::<u32>()
    }
    /// Cells of the channels between columns and rows (their crossings counted once).
    fn channels(&self) -> u32 {
        let iw = self.lf.iter().sum::<u32>() + self.rt.iter().sum::<u32>() + self.cgap.iter().sum::<u32>();
        let ih = self.up.iter().sum::<u32>() + self.dn.iter().sum::<u32>() + self.rgap.iter().sum::<u32>();
        let (cg, rg) = (self.cgap.iter().sum::<u32>(), self.rgap.iter().sum::<u32>());
        cg * ih + rg * iw - cg * rg
    }
}

fn pinned(n: &Node) -> bool {
    n.params.contains_key("col") || n.params.contains_key("lane")
}

/// The columns and rows of one panel: the order `layout::plan_panel` gives (the main chain in one
/// row, a loss beside its source, the author's `col=` / `lane=` winning), a block joined only by
/// a side line moved into that block's column; `fold` puts the second half of a long chain in a
/// row of its own. `None` when folding does not apply.
fn grid(me: &Meas, sizes: &[layout::Size], rt: &layout::RTree, pi: usize, fold: bool) -> Option<Grid> {
    let (g, m, d) = (me.g, me.m, &me.defs[pi]);
    if d.nodes.is_empty() {
        return None;
    }
    let plan = layout::plan_panel(g, &d.nodes, sizes, d.down, m.theme, rt, if d.bare { 2.0 } else { m.theme.space.pad });
    let mut layer: BTreeMap<usize, i64> = plan.layer.iter().map(|(k, v)| (*k, *v as i64)).collect();
    let mut lane: BTreeMap<usize, i64> = plan.lane.iter().map(|(k, v)| (*k, *v as i64)).collect();
    let inside: BTreeSet<usize> = d.nodes.iter().copied().collect();
    let idx = |id: &str| g.node_index(id).filter(|i| inside.contains(i));
    // a block tied only by side lines to one block: in that block's column, a row below or above
    for &n in &d.nodes {
        if pinned(&g.nodes[n]) {
            continue;
        }
        let id = &g.nodes[n].id;
        let ties: Vec<(usize, EdgeKind)> = g
            .edges
            .iter()
            .filter_map(|e| if e.from == *id { idx(&e.to).map(|t| (t, e.kind)) } else if e.to == *id { idx(&e.from).map(|t| (t, e.kind)) } else { None })
            .collect();
        let partners: BTreeSet<usize> = ties.iter().map(|t| t.0).collect();
        if ties.is_empty() || ties.iter().any(|t| t.1 != EdgeKind::Side) || partners.len() != 1 {
            continue;
        }
        let p = *partners.iter().next().unwrap();
        let (pl, pr) = (layer[&p], lane[&p]);
        let taken = |l: i64, r: i64, layer: &BTreeMap<usize, i64>, lane: &BTreeMap<usize, i64>| d.nodes.iter().any(|o| *o != n && layer[o] == l && lane[o] == r);
        if let Some(r) = [pr + 1, pr - 1, pr + 2, pr - 2].into_iter().find(|r| !taken(pl, *r, &layer, &lane)) {
            layer.insert(n, pl);
            lane.insert(n, r);
        }
    }
    let compact = |m: &mut BTreeMap<usize, i64>| {
        let vals: Vec<i64> = m.values().copied().collect::<BTreeSet<_>>().into_iter().collect();
        for v in m.values_mut() {
            *v = vals.iter().position(|x| x == v).unwrap() as i64;
        }
    };
    compact(&mut layer);
    compact(&mut lane);
    if fold {
        let n_layers = layer.values().max().map_or(0, |m| m + 1);
        if d.down || d.nodes.iter().any(|n| pinned(&g.nodes[*n])) || n_layers < 4 {
            return None;
        }
        let cut = (n_layers + 1) / 2;
        let n_lanes = lane.values().max().map_or(0, |m| m + 1);
        for &n in &d.nodes {
            if layer[&n] >= cut {
                *layer.get_mut(&n).unwrap() -= cut;
                *lane.get_mut(&n).unwrap() += n_lanes;
            }
        }
        compact(&mut layer);
        compact(&mut lane);
    }
    let (col, row): (BTreeMap<usize, usize>, BTreeMap<usize, usize>) = if d.down {
        (lane.iter().map(|(k, v)| (*k, *v as usize)).collect(), layer.iter().map(|(k, v)| (*k, *v as usize)).collect())
    } else {
        (layer.iter().map(|(k, v)| (*k, *v as usize)).collect(), lane.iter().map(|(k, v)| (*k, *v as usize)).collect())
    };
    let nc = col.values().max().map_or(0, |m| m + 1);
    let nr = row.values().max().map_or(0, |m| m + 1);
    let (mut lf, mut rtv, mut up, mut dn) = (vec![0u32; nc], vec![0u32; nc], vec![0u32; nr], vec![0u32; nr]);
    for &n in &d.nodes {
        let e = me.ext[n];
        let (c, r) = (col[&n], row[&n]);
        lf[c] = lf[c].max(e.lf);
        rtv[c] = rtv[c].max(e.rt);
        up[r] = up[r].max(e.up);
        dn[r] = dn[r].max(e.dn);
    }
    // the lines inside the panel, by the cells of their ends
    let lines: Vec<((usize, usize), (usize, usize), &crate::spec::Edge)> = g
        .edges
        .iter()
        .filter_map(|e| {
            let (a, b) = (idx(&e.from)?, idx(&e.to)?);
            Some(((col[&a], row[&a]), (col[&b], row[&b]), e))
        })
        .collect();
    let margins = 2.0 * CELL;
    let small = m.theme.small_size();
    let cgap: Vec<u32> = (0..nc.saturating_sub(1))
        .map(|i| {
            let cross: Vec<_> = lines.iter().filter(|(a, b, _)| a.0.min(b.0) <= i && i < a.0.max(b.0)).collect();
            let runs = cross.iter().filter(|(a, b, _)| a.1 != b.1).count();
            let along = if cross.iter().any(|(a, b, _)| a.1 == b.1) { m.theme.space.gap_x } else { 0.0 };
            let label = cross.iter().filter(|(a, b, e)| !e.label.is_empty() && a.0.abs_diff(b.0) == 1).map(|(_, _, e)| text::width(&e.label, small) + 14.0).fold(0.0, f64::max);
            cells(along.max(label).max(LINE_ROOM * (runs as f64 + 1.0)) - margins).max(1)
        })
        .collect();
    // lines that go round a row (feedback, skips, or past a block between their ends) run in
    // the gap under it, or under the panel's last row
    let around = |r: usize| lines.iter().filter(|(a, b, e)| a.1 == r && b.1 == r && (a.0.abs_diff(b.0) > 1 || matches!(e.kind, EdgeKind::Feedback | EdgeKind::Skip))).count();
    let rgap: Vec<u32> = (0..nr.saturating_sub(1))
        .map(|i| {
            let cross: Vec<_> = lines.iter().filter(|(a, b, _)| a.1.min(b.1) <= i && i < a.1.max(b.1)).collect();
            let runs = cross.iter().filter(|(a, b, _)| a.0 != b.0).count() + around(i);
            let along = if cross.iter().any(|(a, b, _)| a.0 == b.0) { m.theme.space.gap_y } else { 0.0 };
            let label = if cross.iter().any(|(a, b, e)| !e.label.is_empty() && a.1.abs_diff(b.1) == 1) { small * 1.4 + 10.0 } else { 0.0 };
            cells(along.max(label).max(LINE_ROOM * (runs as f64 + 1.0)) - margins).max(1)
        })
        .collect();
    // padding and the title
    let ts = m.theme.title_size();
    let has_title = !d.title.is_empty() && !d.bare && d.at != "none";
    let title_h = if has_title { cells(ts * 1.3 + 2.0) } else { 0 };
    let outside = m.theme.panel.title.starts_with("outside");
    let (out_top, out_bottom) = if !has_title {
        (0, 0)
    } else if d.at == "above" || (d.at.is_empty() && outside) {
        (title_h, 0)
    } else if d.at == "below" {
        (0, title_h)
    } else {
        (0, 0)
    };
    let in_top = if has_title && out_top == 0 && out_bottom == 0 { title_h } else { 0 };
    let p = if d.bare { 0 } else { FRAME_PAD };
    let last_around = if nr > 0 { around(nr - 1) } else { 0 };
    let bottom = p + if last_around > 0 { cells(LINE_ROOM * (last_around as f64 + 1.0)) } else { 0 };
    let mut gr = Grid { col, row, lf, rt: rtv, up, dn, cgap, rgap, pad: [p, if in_top > 0 { in_top } else { p } + out_top, p, bottom + out_bottom], out_top, out_bottom, folded: fold };
    // a title wider than the blocks widens the panel
    if has_title {
        let tw = cells(text::width(&d.title, ts) * 1.06) + 2 * (FRAME_PAD + 1);
        let w = gr.w();
        if tw > w {
            gr.pad[0] += (tw - w) / 2;
            gr.pad[2] += tw - w - (tw - w) / 2;
        }
    }
    Some(gr)
}

/// The footprint of every block of the graph on a format's grid.
pub fn footprints(g: &Graph, _f: &Format, m: &Measure) -> Vec<Footprint> {
    let me = Meas::new(g, m);
    (0..g.nodes.len()).map(|i| Footprint { node: i, w: me.ext[i].w(), h: me.ext[i].h(), grow: if me.growable[i] { GROW_MAX } else { 1.0 } }).collect()
}

/// A candidate before the spare room is handed out: which areas share a row, and which panels fold.
struct Cand {
    rows: Vec<Vec<usize>>,
    folds: Vec<bool>,
    /// Gaps between neighbouring areas of each row, and between rows.
    hgap: Vec<Vec<u32>>,
    vgap: Vec<u32>,
    need: (u32, u32),
    /// Cells the areas need (each at its own size, before rows of areas are evened out) and the
    /// channels between them: the rest of the plate is empty.
    used: u64,
    budget: Budget,
}

fn cross_count(g: &Graph, me: &Meas, left: &BTreeSet<usize>, right: &BTreeSet<usize>) -> usize {
    g.edges
        .iter()
        .filter(|e| {
            let (Some(a), Some(b)) = (g.node_index(&e.from), g.node_index(&e.to)) else { return false };
            let (pa, pb) = (me.panel_of[a], me.panel_of[b]);
            (left.contains(&pa) && right.contains(&pb)) || (left.contains(&pb) && right.contains(&pa))
        })
        .count()
}

fn candidate(me: &Meas, grids: &[Grid], rows: Vec<Vec<usize>>, folds: Vec<bool>) -> Cand {
    let (g, m) = (me.g, me.m);
    let gap = |n: usize| cells(m.theme.space.panel_gap.max(LINE_ROOM * (n as f64 + 1.0)));
    let hgap: Vec<Vec<u32>> = rows
        .iter()
        .map(|r| {
            (1..r.len())
                .map(|j| {
                    let (l, rr): (BTreeSet<usize>, BTreeSet<usize>) = (r[..j].iter().copied().collect(), r[j..].iter().copied().collect());
                    gap(cross_count(g, me, &l, &rr))
                })
                .collect()
        })
        .collect();
    let vgap: Vec<u32> = (1..rows.len())
        .map(|j| {
            let (a, b): (BTreeSet<usize>, BTreeSet<usize>) = (rows[..j].iter().flatten().copied().collect(), rows[j..].iter().flatten().copied().collect());
            gap(cross_count(g, me, &a, &b))
        })
        .collect();
    let row_w: Vec<u32> = rows.iter().zip(&hgap).map(|(r, hg)| r.iter().map(|p| grids[*p].w()).sum::<u32>() + hg.iter().sum::<u32>()).collect();
    let row_h: Vec<u32> = rows.iter().map(|r| r.iter().map(|p| grids[*p].h()).max().unwrap_or(0)).collect();
    let w = row_w.iter().copied().max().unwrap_or(0);
    let h = row_h.iter().sum::<u32>() + vgap.iter().sum::<u32>();
    let blocks: u32 = me.ext.iter().map(|e| e.w() * e.h()).sum();
    let channels: u32 = rows.iter().flatten().map(|p| grids[*p].channels()).sum::<u32>()
        + rows.iter().zip(&hgap).zip(&row_h).map(|((_, hg), rh)| hg.iter().sum::<u32>() * rh).sum::<u32>()
        + vgap.iter().sum::<u32>() * w;
    let margins = (w * h).saturating_sub(blocks + channels);
    let used = rows.iter().flatten().map(|p| grids[*p].w() as u64 * grids[*p].h() as u64).sum::<u64>() + rows.iter().zip(&hgap).zip(&row_h).map(|((_, hg), rh)| hg.iter().sum::<u32>() as u64 * *rh as u64).sum::<u64>() + vgap.iter().sum::<u32>() as u64 * w as u64;
    Cand { rows, folds, hgap, vgap, need: (w, h), used, budget: Budget { blocks, margins, channels, least: (w, h) } }
}

/// Hand `extra` cells out: first to the slots that want room (up to what they want, in proportion),
/// the rest evenly over `even` slots. Returns (to the wanting, to the even slots).
fn share(extra: u32, want: &[u32], even: usize) -> (Vec<u32>, Vec<u32>) {
    let total: u32 = want.iter().sum();
    let give = extra.min(total);
    let mut a: Vec<u32> = want.iter().map(|w| if total == 0 { 0 } else { (give as u64 * *w as u64 / total as u64) as u32 }).collect();
    let mut left = give - a.iter().sum::<u32>();
    for (i, w) in want.iter().enumerate() {
        if left == 0 {
            break;
        }
        if a[i] < *w {
            a[i] += 1;
            left -= 1;
        }
    }
    let rest = extra - a.iter().sum::<u32>();
    let n = even.max(1) as u32;
    let b: Vec<u32> = (0..even.max(1)).map(|i| rest / n + if (i as u32) < rest % n { 1 } else { 0 }).collect();
    (a, b)
}

/// What the growable blocks of a grid could use: per column, as wide as they get when each row may
/// grow by `h_more` cells; per row, as tall as they get in the columns as they are now.
fn wants(me: &Meas, gr: &Grid, nodes: &[usize], h_more: u32) -> (Vec<u32>, Vec<u32>) {
    let mut cw = vec![0u32; gr.lf.len()];
    let mut rw = vec![0u32; gr.up.len()];
    for &n in nodes.iter().filter(|n| me.growable[**n]) {
        let (c, r) = (gr.col[&n], gr.row[&n]);
        let s = me.grow_step(n, |e| e.up <= gr.up[r] + h_more && e.dn <= gr.dn[r] + h_more);
        cw[c] = cw[c].max(me.ext_at(n, s).w().saturating_sub(gr.lf[c] + gr.rt[c]));
        let s = me.grow_step(n, |e| e.lf <= gr.lf[c] && e.rt <= gr.rt[c]);
        rw[r] = rw[r].max(me.ext_at(n, s).h().saturating_sub(gr.up[r] + gr.dn[r]));
    }
    (cw, rw)
}

/// `extra` in proportion to `weights` (whole cells, the remainder to the first); `None` when
/// every weight is 0.
fn by_weight(extra: u32, weights: &[u32]) -> Option<Vec<u32>> {
    let total: u64 = weights.iter().map(|w| *w as u64).sum();
    if total == 0 {
        return None;
    }
    let mut v: Vec<u32> = weights.iter().map(|w| (extra as u64 * *w as u64 / total) as u32).collect();
    let mut left = extra - v.iter().sum::<u32>();
    let mut i = 0;
    while left > 0 {
        if weights[i % weights.len()] > 0 {
            v[i % weights.len()] += 1;
            left -= 1;
        }
        i += 1;
    }
    Some(v)
}

/// A grid `ew` cells wider: the columns of growable blocks first (each column's share of `cwant`
/// taken by enlarging its blocks as far as the share and their rows, `h_more` cells taller, allow),
/// then the gaps between the columns evenly (each by at most an average column), the rest as
/// margin, equal on both sides.
fn spread_w(me: &Meas, gr: &mut Grid, nodes: &[usize], cwant: &[u32], ew: u32, h_more: u32) {
    let nc = gr.lf.len();
    let (budget, _) = share(ew, cwant, 1);
    let mut used = 0;
    for c in 0..nc {
        let (l0, r0) = (gr.lf[c], gr.rt[c]);
        for &n in nodes.iter().filter(|n| me.growable[**n] && gr.col[n] == c) {
            let r = gr.row[&n];
            let (l1, r1) = (gr.lf[c], gr.rt[c]);
            let room = budget[c] - ((l1 - l0) + (r1 - r0));
            let s = me.grow_step(n, |e| e.up <= gr.up[r] + h_more && e.dn <= gr.dn[r] + h_more && e.lf.saturating_sub(l1) + e.rt.saturating_sub(r1) <= room);
            let e = me.ext_at(n, s);
            gr.lf[c] = l1.max(e.lf);
            gr.rt[c] = r1.max(e.rt);
        }
        used += (gr.lf[c] - l0) + (gr.rt[c] - r0);
    }
    let mut rest = ew - used;
    if nc >= 2 {
        // a gap grows by at most an average column: beyond that the blocks drift apart
        let cap = (gr.lf.iter().sum::<u32>() + gr.rt.iter().sum::<u32>()) / nc as u32;
        let (_, e) = share(rest.min(cap * (nc as u32 - 1)), &[], nc - 1);
        for (gp, x) in gr.cgap.iter_mut().zip(e) {
            *gp += x;
            rest -= x;
        }
    }
    gr.pad[0] += rest / 2;
    gr.pad[2] += rest - rest / 2;
}

/// As [`spread_w`], down: rows of growable blocks (in the columns as they are), the gaps between
/// rows, then equal margins.
fn spread_h(me: &Meas, gr: &mut Grid, nodes: &[usize], rwant: &[u32], eh: u32) {
    let nr = gr.up.len();
    let (budget, _) = share(eh, rwant, 1);
    let mut used = 0;
    for r in 0..nr {
        let (u0, d0) = (gr.up[r], gr.dn[r]);
        for &n in nodes.iter().filter(|n| me.growable[**n] && gr.row[n] == r) {
            let c = gr.col[&n];
            let (u1, d1) = (gr.up[r], gr.dn[r]);
            let room = budget[r] - ((u1 - u0) + (d1 - d0));
            let s = me.grow_step(n, |e| e.lf <= gr.lf[c] && e.rt <= gr.rt[c] && e.up.saturating_sub(u1) + e.dn.saturating_sub(d1) <= room);
            let e = me.ext_at(n, s);
            gr.up[r] = u1.max(e.up);
            gr.dn[r] = d1.max(e.dn);
        }
        used += (gr.up[r] - u0) + (gr.dn[r] - d0);
    }
    let mut rest = eh - used;
    if nr >= 2 {
        let cap = (gr.up.iter().sum::<u32>() + gr.dn.iter().sum::<u32>()) / nr as u32;
        let (_, e) = share(rest.min(cap * (nr as u32 - 1)), &[], nr - 1);
        for (gp, x) in gr.rgap.iter_mut().zip(e) {
            *gp += x;
            rest -= x;
        }
    }
    gr.pad[1] += rest / 2;
    gr.pad[3] += rest - rest / 2;
}

/// Like with like: the growable blocks of one row of one area, and blocks of the same kind
/// anywhere, take the same enlargement (the smallest any of them can take). `row(a, b)` says
/// whether two blocks share a row.
fn alike(me: &Meas, steps: &mut [usize], row: &dyn Fn(usize, usize) -> bool) {
    let n = steps.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(r: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while r[i] != i {
            r[i] = r[r[i]];
            i = r[i];
        }
        i
    }
    for a in 0..n {
        for b in a + 1..n {
            if !me.growable[a] || !me.growable[b] {
                continue;
            }
            let same_kind = me.g.nodes[a].module == me.g.nodes[b].module;
            let same_row = me.panel_of[a] == me.panel_of[b] && row(a, b);
            if same_kind || same_row {
                let (x, y) = (find(&mut root, a), find(&mut root, b));
                root[x] = y;
            }
        }
    }
    let mut least: BTreeMap<usize, usize> = BTreeMap::new();
    for i in (0..n).filter(|i| me.growable[*i]) {
        let r = find(&mut root, i);
        let e = least.entry(r).or_insert(usize::MAX);
        *e = (*e).min(steps[i]);
    }
    for i in (0..n).filter(|i| me.growable[*i]) {
        let r = find(&mut root, i);
        steps[i] = least[&r];
    }
}

fn arrangement_name(me: &Meas, c: &Cand) -> String {
    let kind = if c.rows.len() == 1 && c.rows[0].len() > 1 {
        "left-to-right"
    } else if c.rows.len() > 1 && c.rows.iter().all(|r| r.len() == 1) {
        "top-to-bottom"
    } else if c.rows.len() == 2 {
        "two-rows"
    } else if c.rows.len() == 1 {
        "one-area"
    } else {
        "rows"
    };
    let rows: Vec<String> = c.rows.iter().map(|r| r.iter().map(|p| format!("{}{}", me.defs[*p].id, if c.folds[*p] { "(folded)" } else { "" })).collect::<Vec<_>>().join(" ")).collect();
    format!("{kind}: {}", rows.join(" / "))
}

/// Work a candidate out on its plate: spare cells to growable blocks then to margins, every block
/// in its cells, the expected bends.
fn realise(me: &Meas, f: &Format, base: &[Vec<Option<Grid>>], c: &Cand, spare: bool) -> (Placement, usize) {
    let (g, m) = (me.g, me.m);
    let k = f.multiplier(c.need);
    let (pw, ph) = if spare { f.plate(k) } else { c.need };
    let mut grids: Vec<Grid> = (0..me.defs.len()).map(|p| base[p][c.folds[p] as usize].clone().unwrap_or_else(|| base[p][0].clone().unwrap())).collect();
    let (hgap, vgap) = (&c.hgap, &c.vgap);
    let eh_all = ph - c.need.1;
    // How far an area can widen (or grow taller) without margin of its own: its growable blocks
    // and its gaps (each by at most an average column or row).
    let spread_cap = |sizes: Vec<u32>| -> u32 {
        let n = sizes.len() as u32;
        if n < 2 { 0 } else { sizes.iter().sum::<u32>() / n * (n - 1) }
    };
    // across: the figure is as wide as every row of areas can take without margin inside a frame
    // (but at least its least width); the rest of the plate is margin round the figure
    let cw: Vec<Vec<u32>> = (0..me.defs.len()).map(|p| wants(me, &grids[p], &me.defs[p].nodes, eh_all).0).collect();
    let absorb_w: Vec<u32> = (0..me.defs.len()).map(|p| cw[p].iter().sum::<u32>() + spread_cap((0..grids[p].lf.len()).map(|i| grids[p].lf[i] + grids[p].rt[i]).collect())).collect();
    let row_w: Vec<u32> = c.rows.iter().enumerate().map(|(ri, r)| r.iter().map(|p| grids[*p].w()).sum::<u32>() + hgap[ri].iter().sum::<u32>()).collect();
    let fw = c.rows.iter().enumerate().map(|(ri, r)| row_w[ri] + r.iter().map(|p| absorb_w[*p]).sum::<u32>()).min().unwrap_or(0).min(pw).max(c.need.0);
    for (ri, r) in c.rows.iter().enumerate() {
        let want: Vec<u32> = r.iter().map(|p| cw[*p].iter().sum()).collect();
        let (to_blocks, rest) = share(fw - row_w[ri], &want, 1);
        let caps: Vec<u32> = r.iter().zip(&to_blocks).map(|(p, b)| absorb_w[*p].saturating_sub(*b)).collect();
        let to_areas = by_weight(rest[0], &caps).unwrap_or_else(|| share(rest[0], &[], r.len()).1);
        for (j, &p) in r.iter().enumerate() {
            let nodes = me.defs[p].nodes.clone();
            spread_w(me, &mut grids[p], &nodes, &cw[p], to_blocks[j] + to_areas[j], eh_all);
        }
    }
    // down: the same, rows of areas instead of areas; a row of areas shares one height
    let row_h: Vec<u32> = c.rows.iter().map(|r| r.iter().map(|p| grids[*p].h()).max().unwrap_or(0)).collect();
    let rws: Vec<Vec<u32>> = (0..me.defs.len()).map(|p| wants(me, &grids[p], &me.defs[p].nodes, 0).1).collect();
    let reach: Vec<u32> = (0..me.defs.len()).map(|p| grids[p].h() + rws[p].iter().sum::<u32>() + spread_cap((0..grids[p].up.len()).map(|i| grids[p].up[i] + grids[p].dn[i]).collect())).collect();
    let grow_r: Vec<u32> = c.rows.iter().zip(&row_h).map(|(r, h)| r.iter().map(|p| reach[*p]).min().unwrap_or(0).saturating_sub(*h)).collect();
    let fh = (c.need.1 + grow_r.iter().sum::<u32>()).min(ph);
    let want: Vec<u32> = c.rows.iter().zip(&row_h).map(|(r, h)| r.iter().map(|p| (grids[*p].h() + rws[*p].iter().sum::<u32>()).saturating_sub(*h)).max().unwrap_or(0)).collect();
    let (to_blocks, rest) = share(fh - c.need.1, &want, 1);
    let caps: Vec<u32> = grow_r.iter().zip(&to_blocks).map(|(g_, b)| g_.saturating_sub(*b)).collect();
    let to_rows = by_weight(rest[0], &caps).unwrap_or_else(|| share(rest[0], &[], c.rows.len()).1);
    let row_h: Vec<u32> = row_h.iter().enumerate().map(|(i, h)| h + to_blocks[i] + to_rows[i]).collect();
    // the figure in the middle of the plate
    let (ox, oy) = ((pw - fw) / 2, (ph - fh) / 2);
    let mut areas = vec![Cells::default(); me.defs.len()];
    let mut y = oy;
    for (ri, r) in c.rows.iter().enumerate() {
        let mut x = ox;
        for (j, &p) in r.iter().enumerate() {
            let eh = row_h[ri] - grids[p].h();
            let nodes = me.defs[p].nodes.clone();
            spread_h(me, &mut grids[p], &nodes, &rws[p], eh);
            areas[p] = Cells { x, y, w: grids[p].w(), h: grids[p].h() };
            x += grids[p].w() + hgap[ri].get(j).copied().unwrap_or(0);
        }
        y += row_h[ri] + vgap.get(ri).copied().unwrap_or(0);
    }
    // blocks: on their column's middle line and their row's port line
    let mut blocks = vec![Cells::default(); g.nodes.len()];
    let mut axis = vec![(0u32, 0u32); g.nodes.len()];
    let mut steps = vec![0usize; g.nodes.len()];
    for (p, gr) in grids.iter().enumerate() {
        let a = areas[p];
        let mut cx = vec![0u32; gr.lf.len()];
        let mut x = a.x + gr.pad[0];
        for c_ in 0..gr.lf.len() {
            cx[c_] = x + gr.lf[c_];
            x += gr.lf[c_] + gr.rt[c_] + gr.cgap.get(c_).copied().unwrap_or(0);
        }
        let mut ry = vec![0u32; gr.up.len()];
        let mut y = a.y + gr.pad[1];
        for r in 0..gr.up.len() {
            ry[r] = y + gr.up[r];
            y += gr.up[r] + gr.dn[r] + gr.rgap.get(r).copied().unwrap_or(0);
        }
        for &n in &me.defs[p].nodes {
            let (c_, r) = (gr.col[&n], gr.row[&n]);
            let room = Ext { lf: gr.lf[c_], rt: gr.rt[c_], up: gr.up[r], dn: gr.dn[r] };
            steps[n] = me.grow_step(n, |e| e.within(&room));
            axis[n] = (cx[c_], ry[r]);
        }
    }
    // like with like, then every block in its cells
    alike(me, &mut steps, &|a, b| grids[me.panel_of[a]].row[&a] == grids[me.panel_of[b]].row[&b]);
    for n in 0..g.nodes.len() {
        let e = me.ext_at(n, steps[n]);
        blocks[n] = Cells { x: axis[n].0 - e.lf, y: axis[n].1 - e.up, w: e.w(), h: e.h() };
    }
    // lines that cannot run straight: two bends each
    let bends = 2 * g
        .edges
        .iter()
        .filter(|e| {
            let (Some(a), Some(b)) = (g.node_index(&e.from), g.node_index(&e.to)) else { return false };
            axis[a].0 != axis[b].0 && axis[a].1 != axis[b].1
        })
        .count();
    let first = &grids[0];
    let alternate = |a: &[u32], b: &[u32], gaps: &[u32]| -> Vec<u32> {
        let mut v = Vec::new();
        for i in 0..a.len() {
            v.push(a[i] + b[i]);
            if let Some(gp) = gaps.get(i) {
                v.push(*gp);
            }
        }
        v
    };
    let text_pt = me.text_nat * f.cell_pt(pw) / CELL;
    let mut ways = Vec::new();
    if text_pt < f.min_text_pt {
        ways.push(format!("加寬：{:.0} pt 寬時最小的字是 {:.1} pt（現在 {:.0} pt 寬、{:.1} pt）", f.width_pt * f.min_text_pt / text_pt, f.min_text_pt, f.width_pt, text_pt));
        let long: Vec<String> = g.nodes.iter().zip(&me.bodies).filter(|(n, b)| n.label.contains(' ') && b.reserve_w > b.w + 1.0).map(|(n, _)| n.id.clone()).collect();
        if !long.is_empty() {
            ways.push(format!("長標籤換行：{}", long.join("、")));
        }
        if let Some((pi, gr)) = grids.iter().enumerate().max_by_key(|(_, gr)| gr.lf.len()) {
            ways.push(format!("減少細節：區 {} 一排有 {} 欄", me.defs[pi].id, gr.lf.len()));
        }
    }
    let plate_cells = (pw as f64) * (ph as f64);
    let empty = (plate_cells - c.used as f64) / plate_cells;
    let _ = m;
    (
        Placement {
            arrangement: arrangement_name(me, c),
            plate: (pw, ph),
            columns: alternate(&first.lf, &first.rt, &first.cgap),
            rows: alternate(&first.up, &first.dn, &first.rgap),
            blocks,
            areas,
            budget: c.budget.clone(),
            empty,
            text_pt,
            ways,
        },
        bends,
    )
}

/// Every way of cutting `n` areas, in order, into rows.
fn row_cuts(n: usize) -> Vec<Vec<Vec<usize>>> {
    let mut out = Vec::new();
    for mask in 0..(1u32 << n.saturating_sub(1)) {
        let mut rows = vec![vec![0]];
        for i in 1..n {
            if mask & (1 << (i - 1)) != 0 {
                rows.push(vec![]);
            }
            rows.last_mut().unwrap().push(i);
        }
        out.push(rows);
    }
    out
}

fn sizes_of(me: &Meas) -> Vec<layout::Size> {
    me.bodies
        .iter()
        .map(|b| layout::Size { w: b.w, h: b.h, reserve_w: b.reserve_w, cy: b.cy, art: b.art, words: b.words, single: b.single, ellipse: b.ellipse, legend: b.legend.clone() })
        .collect()
}

type Base = Vec<Vec<Option<Grid>>>;

/// Every panel's grid (as is, and folded where that applies) and every arrangement of them.
fn candidates(me: &Meas) -> (Base, Vec<Cand>) {
    let g = me.g;
    let sizes = sizes_of(me);
    let rt = layout::RTree::new(g);
    let base: Vec<Vec<Option<Grid>>> = (0..me.defs.len()).map(|p| vec![grid(me, &sizes, &rt, p, false), grid(me, &sizes, &rt, p, true)]).collect();
    // panels without blocks get an empty grid
    let base: Vec<Vec<Option<Grid>>> = base
        .into_iter()
        .map(|v| if v[0].is_none() { vec![Some(Grid { col: BTreeMap::new(), row: BTreeMap::new(), lf: vec![], rt: vec![], up: vec![], dn: vec![], cgap: vec![], rgap: vec![], pad: [2, 2, 2, 2], out_top: 0, out_bottom: 0, folded: false }), None] } else { v })
        .collect();
    let n = me.defs.len();
    let foldable: Vec<usize> = (0..n).filter(|p| base[*p][1].is_some()).collect();
    let mut cands = Vec::new();
    for rows in row_cuts(n.min(6)) {
        // more than six areas: the rest go in a last row
        let mut rows = rows;
        if n > 6 {
            rows.last_mut().unwrap().extend(6..n);
        }
        for fm in 0..(1u32 << foldable.len().min(5)) {
            let mut folds = vec![false; n];
            for (b, p) in foldable.iter().take(5).enumerate() {
                folds[*p] = fm & (1 << b) != 0;
            }
            let grids: Vec<Grid> = (0..n).map(|p| base[p][folds[p] as usize].clone().unwrap()).collect();
            cands.push(candidate(me, &grids, rows.clone(), folds));
        }
    }
    (base, cands)
}

/// The arrangements of the graph, best first, each on the smallest plate of the format's ratio
/// that holds it. `None` only for a graph without blocks.
///
/// These are OPTIONS for the user to pick from (at most [`OPTIONS`], one per way of cutting the
/// areas, in their order, into rows: all stacked, all in one row, two rows …; each with the best
/// folding of its long chains). The arrangement the graph's `arrange` line states comes first, as
/// given; the others follow best first: fewest empty cells (the plate less what the areas need
/// at their own size and the channels between them), then fewest expected bends. A pick by name
/// is [`arrangement`].
pub fn place(g: &Graph, f: &Format, m: &Measure) -> Option<Vec<Placement>> {
    if g.nodes.is_empty() {
        return None;
    }
    let me = Meas::new(g, m);
    let (base, mut cands) = candidates(&me);
    let empty_cells = |c: &Cand| {
        let (pw, ph) = f.plate(f.multiplier(c.need));
        (pw as u64 * ph as u64).saturating_sub(c.used)
    };
    cands.sort_by_key(|c| (empty_cells(c), c.folds.iter().filter(|x| **x).count()));
    // one option per way of cutting the areas into rows (its best folding): options differ
    let mut seen: Vec<Vec<Vec<usize>>> = Vec::new();
    cands.retain(|c| {
        if seen.contains(&c.rows) {
            false
        } else {
            seen.push(c.rows.clone());
            true
        }
    });
    // the arrangement the author wrote comes first, as given
    let stated = stated_rows(&me);
    let first = stated.as_ref().and_then(|rows| cands.iter().position(|c| &c.rows == rows));
    let mut out: Vec<(bool, u64, usize, usize, Placement)> = cands
        .iter()
        .enumerate()
        .take(KEEP)
        .chain(first.filter(|i| *i >= KEEP).map(|i| (i, &cands[i])))
        .map(|(i, c)| {
            let (p, bends) = realise(&me, f, &base, c, true);
            (Some(i) != first, empty_cells(c), bends, i, p)
        })
        .collect();
    out.sort_by_key(|o| (o.0, o.1, o.2, o.3));
    out.truncate(OPTIONS);
    Some(out.into_iter().map(|o| o.4).collect())
}

/// The rows of areas the graph's `arrange` line states, as area indices (`None` when it states
/// none, or names areas the figure does not have).
fn stated_rows(me: &Meas) -> Option<Vec<Vec<usize>>> {
    if me.g.arrange.is_empty() {
        return None;
    }
    let by_id = |id: &str| me.defs.iter().position(|d| d.id == id);
    let mut rows: Vec<Vec<usize>> = me.g.arrange.iter().map(|r| r.iter().filter_map(|id| by_id(id)).collect::<Vec<_>>()).filter(|r| !r.is_empty()).collect();
    // a bare panel the line does not mention keeps its side
    if let Some(b) = by_id("_").filter(|b| !rows.iter().flatten().any(|x| x == b)) {
        if b == 0 { rows.first_mut()?.insert(0, b) } else { rows.last_mut()?.push(b) }
    }
    let all: Vec<usize> = rows.iter().flatten().copied().collect();
    (all == (0..me.defs.len()).collect::<Vec<_>>()).then_some(rows)
}

/// The user's pick: the arrangement named as `place` names its options (`two-rows: a / b c`), or
/// by its rows alone (`a / b c`, `a / b(folded) / c`), worked out on its plate. Any way of
/// cutting the areas into rows can be named, not only the options offered.
pub fn arrangement(g: &Graph, f: &Format, m: &Measure, name: &str) -> Option<Placement> {
    let me = Meas::new(g, m);
    let (base, cands) = candidates(&me);
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let want = norm(name);
    let rows_of = |n: &str| norm(n.split_once(": ").map(|x| x.1).unwrap_or(n));
    let empty_cells = |c: &Cand| {
        let (pw, ph) = f.plate(f.multiplier(c.need));
        (pw as u64 * ph as u64).saturating_sub(c.used)
    };
    let hit = cands.iter().find(|c| norm(&arrangement_name(&me, c)) == want).or_else(|| {
        let w = rows_of(&want);
        let exact = cands.iter().find(|c| rows_of(&arrangement_name(&me, c)) == w);
        // without `(folded)` marks: the best folding of those rows
        exact.or_else(|| cands.iter().filter(|c| rows_of(&arrangement_name(&me, c)).replace("(folded)", "") == w).min_by_key(|c| empty_cells(c)))
    })?;
    Some(realise(&me, f, &base, hit, true).0)
}

/// How full one frame is: its content (the box of its blocks with their words, in cells) against
/// the frame, before the spare room of the plate was handed out and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameFill {
    /// `panel` or `region`.
    pub kind: String,
    pub id: String,
    /// Frame and content box, cells across and down: at the least arrangement, and on the plate.
    pub before: ((u32, u32), (u32, u32)),
    pub after: ((u32, u32), (u32, u32)),
}

impl FrameFill {
    fn share(fc: ((u32, u32), (u32, u32))) -> f64 {
        let ((fw, fh), (cw, ch)) = fc;
        if fw == 0 || fh == 0 { 0.0 } else { (cw as f64 * ch as f64) / (fw as f64 * fh as f64) }
    }
    /// Content / frame at the least arrangement.
    pub fn fill_before(&self) -> f64 {
        Self::share(self.before)
    }
    /// Content / frame on the plate.
    pub fn fill_after(&self) -> f64 {
        Self::share(self.after)
    }
    /// Cells of margin (frame less content) before and after.
    pub fn margin(&self) -> (u32, u32) {
        let m = |fc: ((u32, u32), (u32, u32))| (fc.0.0 * fc.0.1).saturating_sub(fc.1.0 * fc.1.1);
        (m(self.before), m(self.after))
    }
}

/// The fill of every framed panel and every region of a placement, before and after the plate's
/// spare room (the same arrangement worked out on its least area).
pub fn fills(g: &Graph, f: &Format, p: &Placement, m: &Measure) -> Vec<FrameFill> {
    let me = Meas::new(g, m);
    let (base, cands) = candidates(&me);
    let Some(c) = cands.iter().find(|c| arrangement_name(&me, c) == p.arrangement) else { return Vec::new() };
    let least = realise(&me, f, &base, c, false).0;
    let lay_b = to_layout(g, f, &least, m);
    let lay_a = to_layout(g, f, p, m);
    let to_cells = |r: Rect| (((r.w / CELL) - 1e-6).ceil().max(0.0) as u32, ((r.h / CELL) - 1e-6).ceil().max(0.0) as u32);
    let content = |lay: &Layout, pick: &dyn Fn(usize) -> bool, extra: &[Rect]| -> Option<Rect> {
        let mut rs: Vec<Rect> = lay.nodes.iter().enumerate().filter(|(i, _)| pick(*i)).map(|(_, n)| n.rect.union(&n.reserve)).collect();
        rs.extend_from_slice(extra);
        let first = *rs.first()?;
        Some(rs.iter().skip(1).fold(first, |a, b| a.union(b)))
    };
    let mut out = Vec::new();
    for (pi, d) in me.defs.iter().enumerate() {
        if d.bare || d.nodes.is_empty() {
            continue;
        }
        let one = |lay: &Layout| {
            let c = content(lay, &|i| lay.nodes[i].panel == pi, &[]).unwrap_or_default();
            (to_cells(lay.panels[pi].rect), to_cells(c))
        };
        out.push(FrameFill { kind: "panel".into(), id: d.id.clone(), before: one(&lay_b), after: one(&lay_a) });
    }
    for r in &g.regions {
        let one = |lay: &Layout| {
            let Some(rb) = lay.regions.iter().find(|b| b.id == r.id) else { return ((0, 0), (0, 0)) };
            let kids: Vec<Rect> = g.regions.iter().filter(|k| k.parent == r.id).filter_map(|k| lay.regions.iter().find(|b| b.id == k.id).map(|b| b.rect)).collect();
            let c = content(lay, &|i| r.members.iter().any(|mm| *mm == g.nodes[i].id), &kids).unwrap_or_default();
            (to_cells(rb.rect), to_cells(c))
        };
        out.push(FrameFill { kind: "region".into(), id: r.id.clone(), before: one(&lay_b), after: one(&lay_a) });
    }
    out
}

/// The enlargement step of each block in a placement: the largest whose cells are no larger
/// than the cells it was given (the same ladder `place` climbs, so the same step).
fn steps(me: &Meas, p: &Placement) -> Vec<usize> {
    let cell = |n: usize| p.blocks.get(n).copied().unwrap_or_default();
    let mut st: Vec<usize> = (0..me.g.nodes.len())
        .map(|n| {
            let c = cell(n);
            me.grow_step(n, |e| e.w() <= c.w && e.h() <= c.h)
        })
        .collect();
    // the same grouping as `place`: blocks of one row of an area hold its port line, so their
    // cells overlap down; blocks of different rows never do
    alike(me, &mut st, &|a, b| {
        let (x, y) = (cell(a), cell(b));
        x.y < y.y + y.h && y.y < x.y + x.h
    });
    st
}

/// The graph as the placement draws it: each enlarged block carries its `grow=` (the drawing
/// reads it), every other block none. `build` must be given this graph with the layout.
pub fn grown(g: &Graph, _f: &Format, p: &Placement, m: &Measure) -> Graph {
    let me = Meas::new(g, m);
    let st = steps(&me, p);
    let mut out = g.clone();
    for (n, s) in out.nodes.iter_mut().zip(st) {
        n.params.remove("grow");
        if s > 0 {
            n.params.insert("grow".into(), serde_json::json!(grow_at(s)));
        }
    }
    out
}

/// A placement as the layout the router and the drawing take: every rectangle on cell borders.
///
/// Natural size (`scale` 1, one cell = [`CELL`] pt), the plate at the origin and as the bounds.
/// Areas are the panels' rectangles (a title written outside takes its band from the area);
/// each block lies in its cells with its picture's middle on its column's line and its port
/// line on its row's line, both cell borders. Draw with [`grown`]'s graph.
pub fn to_layout(g: &Graph, f: &Format, p: &Placement, m: &Measure) -> Layout {
    let _ = f;
    let me = Meas::new(g, m);
    let st = steps(&me, p);
    let ts = m.theme.title_size();
    let pad = (FRAME_PAD + 1) as f64 * CELL;
    let outside = m.theme.panel.title.starts_with("outside");
    let mut panels = Vec::new();
    for (pi, d) in me.defs.iter().enumerate() {
        let a = rect_of(p.areas.get(pi).copied().unwrap_or_default(), CELL);
        let has_title = !d.title.is_empty() && !d.bare && d.at != "none";
        let th_cells = if has_title { cells(ts * 1.3 + 2.0) as f64 * CELL } else { 0.0 };
        let out_top = has_title && (d.at == "above" || (d.at.is_empty() && outside));
        let out_bottom = has_title && d.at == "below";
        let frame = Rect::new(a.x, a.y + if out_top { th_cells } else { 0.0 }, a.w, a.h - if out_top || out_bottom { th_cells } else { 0.0 });
        let title_rect = has_title.then(|| {
            let tw = text::width(&d.title, ts) * 1.06;
            let th = ts * 1.3;
            match d.at.as_str() {
                "above" => Rect::new(frame.cx() - tw / 2.0, a.y, tw, th),
                "below" => Rect::new(frame.cx() - tw / 2.0, frame.bottom() + 1.0, tw, th),
                "top" => Rect::new(frame.cx() - tw / 2.0, frame.y + 1.0, tw, th),
                "top-left" | "left" => Rect::new(frame.x + pad * 0.6, frame.y + 1.0, tw, th),
                _ => {
                    let centre = m.theme.panel.title.ends_with("center");
                    let x = if centre { frame.cx() - tw / 2.0 } else if outside { frame.x } else { frame.x + pad * 0.6 };
                    let y = if outside { a.y } else { frame.y + 1.0 };
                    Rect::new(x, y, tw, th)
                }
            }
        });
        panels.push(PanelBox { id: d.id.clone(), title: d.title.clone(), rect: frame, title_rect, bare: d.bare, down: d.down });
    }
    let mut nodes = Vec::new();
    for (i, n) in g.nodes.iter().enumerate() {
        let pi = me.panel_of[i];
        let b = body_at(n, st[i], me.defs[pi].down, m);
        let e = ext_of(&b);
        let c = p.blocks.get(i).copied().unwrap_or_default();
        let (axis, port) = ((c.x + e.lf) as f64 * CELL, (c.y + e.up) as f64 * CELL);
        let ax = b.art.0 + b.art.2 / 2.0;
        let rect = Rect::new(axis - ax, port - b.cy, b.w, b.h);
        let reserve = Rect::new(rect.cx() - b.reserve_w / 2.0, rect.y, b.reserve_w, rect.h);
        let art = Rect::new(rect.x + b.art.0, rect.y + b.art.1, b.art.2, b.art.3);
        nodes.push(Placed { rect, reserve, port_y: port, art, words: b.words, panel: pi, layer: 0, lane: 0, single: b.single, ellipse: b.ellipse, ends: BTreeMap::new(), origin: (0.0, 0.0) });
    }
    // layer and lane: the order of the columns and rows within each panel
    for (pi, d) in me.defs.iter().enumerate() {
        let _ = pi;
        let xs: Vec<i64> = d.nodes.iter().map(|n| (nodes[*n].art.cx() * 100.0).round() as i64).collect::<BTreeSet<_>>().into_iter().collect();
        let ys: Vec<i64> = d.nodes.iter().map(|n| (nodes[*n].port_y * 100.0).round() as i64).collect::<BTreeSet<_>>().into_iter().collect();
        for &n in &d.nodes {
            let x = xs.iter().position(|v| *v == (nodes[n].art.cx() * 100.0).round() as i64).unwrap_or(0);
            let y = ys.iter().position(|v| *v == (nodes[n].port_y * 100.0).round() as i64).unwrap_or(0);
            let (layer, lane) = if d.down { (y, x) } else { (x, y) };
            nodes[n].layer = layer;
            nodes[n].lane = lane;
        }
    }
    let regions: Vec<RegionBox> = if g.regions.is_empty() {
        Vec::new()
    } else {
        let rt = layout::RTree::new(g);
        let rooms: Vec<Rect> = nodes.iter().map(|n| n.rect.union(&n.reserve)).collect();
        layout::frame_regions(g, &rt, &rooms, m.theme, 1.0).into_iter().map(|(_, b, _)| b).collect()
    };
    let bounds = Rect::new(0.0, 0.0, p.plate.0 as f64 * CELL, p.plate.1 as f64 * CELL);
    Layout { nodes, panels, regions, legend: Vec::new(), swatches: Vec::new(), bounds, scale: 1.0 }
}

/// A cell rectangle in the layout's coordinates (points at the layout's scale).
pub fn rect_of(c: Cells, cell: f64) -> Rect {
    Rect::new(c.x as f64 * cell, c.y as f64 * cell, c.w as f64 * cell, c.h as f64 * cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ratio_fixes_the_shape_and_the_need_fixes_the_size() {
        let f = Format::new(500.0, (16, 9));
        assert_eq!(f.plate(10), (160, 90));
        assert_eq!(f.plate(20), (320, 180));
        // a figure that needs 190 x 100 cells gets the smallest 16:9 plate that holds it
        let k = f.multiplier((190, 100));
        assert_eq!((k, f.plate(k)), (12, (192, 108)));
        // any ratio: two columns at 1:1, one column at 5:6, a ratio not in lowest terms
        assert_eq!(Format::new(500.0, (1, 1)).plate(Format::new(500.0, (1, 1)).multiplier((190, 100))), (190, 190));
        assert_eq!(Format::new(250.0, (5, 6)).plate(16), (80, 96));
        assert_eq!(Format::new(500.0, (32, 18)).ratio, (16, 9));
        // the cell on the page shrinks as the plate grows: legibility is a separate report
        assert!(f.cell_pt(320) < f.cell_pt(160));
        assert_eq!(rect_of(Cells { x: 2, y: 3, w: 10, h: 10 }, 3.0).w, 30.0);
    }
}
