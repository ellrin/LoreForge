//! L2, part one — where everything sits.
//!
//! Panels are arranged by the template (rows of panels, or fraction rects);
//! inside a panel the nodes are layered along the flow: edges that are not
//! part of the flow (feedback, supervision, skip, shared …) are set aside,
//! the layer of a node is its longest path from a source, the longest chain
//! becomes the straight trunk and every other chain takes the nearest free
//! lane beside it. Cells are centred, columns and rows equally spaced.
//! Everything is computed at natural size and then fitted into the frame.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::modules::{self, Ctx, Library};
use crate::spec::*;
use crate::text::{self, NoMath, Vars};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    pub fn inflate(&self, m: f64) -> Rect {
        Rect::new(self.x - m, self.y - m, self.w + 2.0 * m, self.h + 2.0 * m)
    }
    /// Overlap with positive area.
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() - 1e-6 && o.x < self.right() - 1e-6 && self.y < o.bottom() - 1e-6 && o.y < self.bottom() - 1e-6
    }
    pub fn contains(&self, o: &Rect) -> bool {
        o.x >= self.x - 1e-6 && o.y >= self.y - 1e-6 && o.right() <= self.right() + 1e-6 && o.bottom() <= self.bottom() + 1e-6
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    fn map(&self, ox: f64, oy: f64, k: f64) -> Rect {
        Rect::new(ox + self.x * k, oy + self.y * k, self.w * k, self.h * k)
    }
}

/// Panel rects as fractions of the figure frame (same idea as slide arrangements).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelTemplate {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// One rect per panel, in the order the panels are declared.
    pub panels: Vec<Rect>,
}

impl PanelTemplate {
    pub fn from_json(s: &str) -> Result<PanelTemplate, String> {
        let t: PanelTemplate = serde_json::from_str(s).map_err(|e| format!("template: {e}"))?;
        if t.panels.is_empty() || t.panels.iter().any(|r| r.w <= 0.0 || r.h <= 0.0 || r.x < 0.0 || r.y < 0.0 || r.right() > 1.0001 || r.bottom() > 1.0001) {
            return Err("template: panel rects are fractions inside 0..1".into());
        }
        for (i, a) in t.panels.iter().enumerate() {
            if t.panels[i + 1..].iter().any(|b| a.overlaps(b)) {
                return Err("template: panel rects overlap".into());
            }
        }
        Ok(t)
    }
}

/// A legend the author chose (instead of the automatic one): rows of
/// marks and words, one centred block under (or over) the figure.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LegendChoice {
    pub rows: Vec<LegendRow>,
    /// Over the figure instead of under it.
    pub above: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegendRow {
    pub label: String,
    pub mark: LegendMark,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegendMark {
    /// A short sample of a line kind as the theme draws it.
    Line(EdgeKind),
    /// A small box in these colours (a block type).
    Box { fill: String, line: String },
}

#[derive(Debug, Clone)]
pub struct LayoutOptions {
    /// Where the figure goes on the slide (pt).
    pub frame: Rect,
    /// Largest enlargement of the natural size.
    pub max_scale: f64,
    /// Used when the graph's `template` names it.
    pub template: Option<PanelTemplate>,
    /// The author's legend; `None` = automatic (line kinds under, layer
    /// colours over the figure). `Some` with no rows = no legend.
    pub legend: Option<LegendChoice>,
    /// Blocks moved by hand: node id → offset (pt, at the figure's natural size, so it is
    /// multiplied by the layout's scale). Applied after the layout; ids the graph does not
    /// have are ignored.
    pub place: BTreeMap<String, [f64; 2]>,
    /// The author's choices for single lines, by `figure::edge_key`. The router reads them
    /// from the layout (see [`Placed::ends`]).
    pub ends: BTreeMap<String, crate::figure::EdgeEnds>,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        // a 16:9 slide of 10 in with a margin
        LayoutOptions { frame: Rect::new(24.0, 24.0, 672.0, 357.0), max_scale: 1.5, template: None, legend: None, place: BTreeMap::new(), ends: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The frame connectors attach to.
    pub rect: Rect,
    /// The room the node needs (text may overflow the frame sideways).
    pub reserve: Rect,
    /// Centre line of the drawn body: where lines enter the left and right sides.
    pub port_y: f64,
    /// Index into [`Layout::panels`].
    pub panel: usize,
    pub layer: usize,
    pub lane: usize,
    pub single: bool,
    pub ellipse: bool,
    /// The author's choices for the lines that leave this node (`figure::edge_key` → ends),
    /// as given: the router reads them from here, because the layout is the only thing
    /// `draw` hands it. Turns are still in natural size, see [`Placed::origin`].
    pub ends: BTreeMap<String, crate::figure::EdgeEnds>,
    /// Where the natural-size origin lies in the figure: a natural coordinate `t` is at
    /// `origin + t * Layout::scale`.
    pub origin: (f64, f64),
}

impl Placed {
    /// Where a line meets this side: the middle of the top and bottom, the
    /// body's centre line on the left and right.
    pub fn port(&self, side: crate::route::Side) -> (f64, f64) {
        use crate::route::Side;
        match side {
            Side::Top => (self.rect.cx(), self.rect.y),
            Side::Bottom => (self.rect.cx(), self.rect.bottom()),
            Side::Left => (self.rect.x, self.port_y),
            Side::Right => (self.rect.right(), self.port_y),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelBox {
    pub id: String,
    pub title: String,
    pub rect: Rect,
    pub title_rect: Option<Rect>,
    pub bare: bool,
    pub down: bool,
}

/// A region placed: its frame round its members (and its sub-regions), in the layout's
/// coordinates. Outer regions come before the regions inside them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegionBox {
    pub id: String,
    pub title: String,
    pub rect: Rect,
    /// Where the title is written; `None` = no title.
    pub title_rect: Option<Rect>,
    /// 0 = directly in a panel, 1 = inside a region, …
    pub depth: usize,
    /// As [`crate::spec::Region`]: `solid` | `dashed` | `dotted` | `none`, the tint, round
    /// corners, where the title is.
    pub line: String,
    pub tint: String,
    pub round: bool,
    pub title_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegendItem {
    pub kind: EdgeKind,
    /// The words; empty = the line kind's name.
    pub label: String,
    /// Drawn as a box in (fill, line) instead of a line sample.
    pub boxed: Option<(String, String)>,
    /// Sample line from → to.
    pub line: ((f64, f64), (f64, f64)),
    /// Top-left of the text and its size.
    pub text: Rect,
}

/// One entry of the colour legend (layer types, roles): a swatch and its name.
#[derive(Debug, Clone, PartialEq)]
pub struct SwatchItem {
    pub swatch: crate::art::Swatch,
    pub at: Rect,
    pub text: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// One per graph node, same order.
    pub nodes: Vec<Placed>,
    pub panels: Vec<PanelBox>,
    /// The graph's regions, placed (empty until the layout frames them).
    pub regions: Vec<RegionBox>,
    pub legend: Vec<LegendItem>,
    /// The colour legend over the figure: every layer type a module coloured,
    /// when there are two or more.
    pub swatches: Vec<SwatchItem>,
    /// Bounding box of the whole figure.
    pub bounds: Rect,
    /// Scale applied to the natural size.
    pub scale: f64,
}

struct Size {
    w: f64,
    h: f64,
    reserve_w: f64,
    cy: f64,
    single: bool,
    ellipse: bool,
    legend: Vec<crate::art::Swatch>,
}

struct PanelPlan {
    nodes: Vec<usize>,
    layer: BTreeMap<usize, usize>,
    lane: BTreeMap<usize, usize>,
    /// Positions of the node frames relative to the content origin.
    pos: BTreeMap<usize, Rect>,
    content: (f64, f64),
    down: bool,
}

// ── regions ────────────────────────────────────────────────────────────

/// Padding between a region's members and its frame (pt, natural size).
const REG_PAD: f64 = 8.0;
/// Least clear space between a frame and what lies beside it (pt, natural size).
const REG_GAP: f64 = 6.0;

/// The graph's regions as a forest.
struct RTree {
    kids: Vec<Vec<usize>>,
    roots: Vec<usize>,
    /// Every node of a region, those of its sub-regions included.
    all: Vec<BTreeSet<usize>>,
    /// Outer regions before the regions inside them.
    order: Vec<usize>,
    depth: Vec<usize>,
}

impl RTree {
    fn new(g: &Graph) -> RTree {
        let n = g.regions.len();
        let find = |id: &str| g.regions.iter().position(|r| r.id == id);
        let mut parent: Vec<Option<usize>> = (0..n).map(|i| find(&g.regions[i].parent).filter(|p| *p != i)).collect();
        // a parent that leads back to the region itself is ignored
        for i in 0..n {
            let mut at = parent[i];
            let mut steps = 0;
            while let Some(p) = at {
                if p == i || steps > n {
                    parent[i] = None;
                    break;
                }
                at = parent[p];
                steps += 1;
            }
        }
        let mut kids = vec![Vec::new(); n];
        let mut roots = Vec::new();
        for i in 0..n {
            match parent[i] {
                Some(p) => kids[p].push(i),
                None => roots.push(i),
            }
        }
        let mut order = Vec::new();
        let mut depth = vec![0; n];
        fn walk(i: usize, d: usize, kids: &[Vec<usize>], order: &mut Vec<usize>, depth: &mut [usize]) {
            order.push(i);
            depth[i] = d;
            for &c in &kids[i] {
                walk(c, d + 1, kids, order, depth);
            }
        }
        for &r in &roots {
            walk(r, 0, &kids, &mut order, &mut depth);
        }
        let mut all: Vec<BTreeSet<usize>> = g.regions.iter().map(|r| r.members.iter().filter_map(|id| g.node_index(id)).collect()).collect();
        for &i in order.iter().rev() {
            if let Some(p) = parent[i] {
                let own = all[i].clone();
                all[p].extend(own);
            }
        }
        RTree { kids, roots, all, order, depth }
    }
}

fn title_at(r: &Region) -> &str {
    match r.title_at.as_str() {
        "top" | "above" | "below" | "left" => r.title_at.as_str(),
        _ => "top-left",
    }
}

fn title_w(r: &Region, theme: &Theme) -> f64 {
    if r.title.is_empty() { 0.0 } else { text::width(&r.title, theme.small_size()) * 1.06 }
}

fn title_line(theme: &Theme) -> f64 {
    theme.small_size() * 1.3
}

/// The band a title takes (its line and a little air).
fn title_band(r: &Region, theme: &Theme) -> f64 {
    if r.title.is_empty() { 0.0 } else { title_line(theme) + 2.0 }
}

type Cells = ((usize, usize), (usize, usize));

/// What the plan of one panel knows about its regions: where each lies in the grid and
/// how much room its frame, title and nesting take on each side (0 left, 1 top, 2 right, 3 bottom).
struct RCtx<'a> {
    g: &'a Graph,
    rt: &'a RTree,
    theme: &'a Theme,
    /// Live regions: (x range, y range) in grid cells.
    range: BTreeMap<usize, Cells>,
    /// Widest and tallest room among the members.
    lb: BTreeMap<usize, (f64, f64)>,
}

impl RCtx<'_> {
    /// Growth of the frame itself on one side.
    fn inner(&self, i: usize, side: usize) -> f64 {
        let r = &self.g.regions[i];
        let at = title_at(r);
        let (lbw, lbh) = self.lb[&i];
        let tw = title_w(r, self.theme);
        let band = title_band(r, self.theme);
        let over = |lb: f64| ((tw - lb) / 2.0).max(0.0);
        let mut e = REG_PAD;
        match side {
            0 => e += if at == "left" { band } else { over(lbw) },
            2 => e += if at == "left" { 0.0 } else { over(lbw) },
            1 => e += if at == "left" { over(lbh) } else if at == "top" || at == "top-left" { band } else { 0.0 },
            _ => e += if at == "left" { over(lbh) } else { 0.0 },
        }
        e
    }
    /// Room of the title that lies outside the frame.
    fn outside(&self, i: usize, side: usize) -> f64 {
        let r = &self.g.regions[i];
        match (title_at(r), side) {
            ("above", 1) | ("below", 3) => title_band(r, self.theme),
            _ => 0.0,
        }
    }
    fn edge(&self, i: usize, side: usize) -> usize {
        let (x, y) = self.range[&i];
        match side {
            0 => x.0,
            1 => y.0,
            2 => x.1,
            _ => y.1,
        }
    }
    /// How far the region reaches beyond its outermost members on one side, the frames of the
    /// regions inside it that end at the same line included.
    fn total(&self, i: usize, side: usize) -> f64 {
        let at = self.edge(i, side);
        let kid = self.rt.kids[i].iter().filter(|c| self.range.contains_key(c) && self.edge(**c, side) == at).map(|c| self.total(*c, side)).fold(0.0, f64::max);
        self.inner(i, side) + kid + self.outside(i, side)
    }
    /// Least gaps between the columns (x) and rows (y) so the frames clear what is beside them,
    /// and how far the frames reach beyond the grid.
    fn spacing(&self, nx: usize, ny: usize, avail: f64) -> (Vec<f64>, Vec<f64>, [f64; 4]) {
        let mut start = [vec![0.0f64; nx.max(1)], vec![0.0f64; ny.max(1)]];
        let mut end = [vec![0.0f64; nx.max(1)], vec![0.0f64; ny.max(1)]];
        for &i in self.range.keys() {
            let (x, y) = self.range[&i];
            start[0][x.0] = start[0][x.0].max(self.total(i, 0));
            end[0][x.1] = end[0][x.1].max(self.total(i, 2));
            start[1][y.0] = start[1][y.0].max(self.total(i, 1));
            end[1][y.1] = end[1][y.1].max(self.total(i, 3));
        }
        let need = |axis: usize, n: usize| -> Vec<f64> {
            (0..n.saturating_sub(1)).map(|b| if end[axis][b] > 0.0 || start[axis][b + 1] > 0.0 { end[axis][b] + start[axis][b + 1] + REG_GAP } else { 0.0 }).collect()
        };
        let m = |v: f64| if v > 0.0 { (v + REG_GAP - avail).max(0.0) } else { 0.0 };
        let margin = [m(start[0][0]), m(start[1][0]), m(end[0][nx.max(1) - 1]), m(end[1][ny.max(1) - 1])];
        (need(0, nx), need(1, ny), margin)
    }
}

fn cell_box(set: &BTreeSet<usize>, layer: &BTreeMap<usize, usize>, lane: &BTreeMap<usize, usize>) -> (usize, usize, usize, usize) {
    let mut b = (usize::MAX, 0, usize::MAX, 0);
    for n in set {
        b = (b.0.min(layer[n]), b.1.max(layer[n]), b.2.min(lane[n]), b.3.max(lane[n]));
    }
    b
}

fn boxes_meet(a: (usize, usize, usize, usize), b: (usize, usize, usize, usize)) -> bool {
    a.0 <= b.1 && b.0 <= a.1 && a.2 <= b.3 && b.2 <= a.3
}

/// Does a region's block of cells hold a block that is not its own, or meet a sibling's block?
fn region_conflict(rt: &RTree, scope: &BTreeSet<usize>, kids: &[usize], layer: &BTreeMap<usize, usize>, lane: &BTreeMap<usize, usize>) -> bool {
    let live: Vec<(usize, BTreeSet<usize>)> = kids
        .iter()
        .filter_map(|&k| {
            let s: BTreeSet<usize> = rt.all[k].iter().copied().filter(|n| scope.contains(n)).collect();
            (!s.is_empty()).then_some((k, s))
        })
        .collect();
    let boxes: Vec<_> = live.iter().map(|(_, s)| cell_box(s, layer, lane)).collect();
    for a in 0..live.len() {
        for b in a + 1..live.len() {
            if boxes_meet(boxes[a], boxes[b]) {
                return true;
            }
        }
    }
    let claimed: BTreeSet<usize> = live.iter().flat_map(|(_, s)| s.iter().copied()).collect();
    for n in scope.difference(&claimed) {
        let c = (layer[n], layer[n], lane[n], lane[n]);
        if boxes.iter().any(|b| boxes_meet(*b, c)) {
            return true;
        }
    }
    live.iter().any(|(k, s)| region_conflict(rt, s, &rt.kids[*k], layer, lane))
}

/// Give every region a block of cells of its own (regions inside it get blocks inside that).
/// Units (a region, or a single block that is in no region) are taken in the order of their
/// mean lane and each is put at the first lane that is free over the layers it spans, so two
/// regions that follow one another along the flow share a row; only units that share layers
/// are put one under the other. (A lane block per region, whatever its layers, turned a row
/// of three framed networks into a staircase.) Returns how many lanes the scope took.
fn stack_lanes(rt: &RTree, scope: &BTreeSet<usize>, kids: &[usize], layer: &BTreeMap<usize, usize>, old: &BTreeMap<usize, usize>, out: &mut BTreeMap<usize, usize>, base: usize) -> usize {
    struct Unit {
        key: f64,
        first: usize,
        nodes: BTreeSet<usize>,
        region: Option<usize>,
    }
    let mut units: Vec<Unit> = Vec::new();
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    for &k in kids {
        let s: BTreeSet<usize> = rt.all[k].iter().copied().filter(|n| scope.contains(n) && !claimed.contains(n)).collect();
        if s.is_empty() {
            continue;
        }
        claimed.extend(s.iter().copied());
        let key = s.iter().map(|n| old[n] as f64).sum::<f64>() / s.len() as f64;
        units.push(Unit { key, first: *s.iter().next().unwrap(), nodes: s, region: Some(k) });
    }
    for n in scope.difference(&claimed) {
        units.push(Unit { key: old[n] as f64, first: *n, nodes: BTreeSet::from([*n]), region: None });
    }
    units.sort_by(|a, b| a.key.partial_cmp(&b.key).unwrap().then(a.first.cmp(&b.first)));
    // the first free lane over each layer, counted from `base`
    let mut sky: BTreeMap<usize, usize> = BTreeMap::new();
    for u in units {
        let layers: BTreeSet<usize> = u.nodes.iter().map(|n| layer[n]).collect();
        let (l0, l1) = (*layers.iter().next().unwrap(), *layers.iter().next_back().unwrap());
        let at = (l0..=l1).map(|l| sky.get(&l).copied().unwrap_or(0)).max().unwrap_or(0);
        let h = match u.region {
            Some(k) => stack_lanes(rt, &u.nodes, &rt.kids[k], layer, old, out, base + at).max(1),
            None => {
                for n in &u.nodes {
                    out.insert(*n, base + at);
                }
                1
            }
        };
        for l in l0..=l1 {
            sky.insert(l, at + h);
        }
    }
    sky.values().copied().max().unwrap_or(0)
}

/// The frames of the regions, outer before inner, with the room each frame (title included)
/// takes: `(index in the graph, box, frame plus the title that lies outside it)`.
/// `rooms` = the room of every node; `k` scales the padding and the title.
fn frame_regions(g: &Graph, rt: &RTree, rooms: &[Rect], theme: &Theme, k: f64) -> Vec<(usize, RegionBox, Rect)> {
    let n = g.regions.len();
    let mut got: Vec<Option<(RegionBox, Rect)>> = vec![None; n];
    for &i in rt.order.iter().rev() {
        let r = &g.regions[i];
        let mut base: Option<Rect> = None;
        let mut add = |x: Rect| base = Some(base.map_or(x, |b| b.union(&x)));
        for id in &r.members {
            if let Some(ni) = g.node_index(id) {
                add(rooms[ni]);
            }
        }
        for &c in &rt.kids[i] {
            if let Some((_, ext)) = &got[c] {
                add(*ext);
            }
        }
        let Some(b) = base else { continue };
        let at = title_at(r);
        let (p, band, th, tw) = (REG_PAD * k, title_band(r, theme) * k, title_line(theme) * k, title_w(r, theme) * k);
        let left = p + if at == "left" { band } else { 0.0 };
        let top = p + if at == "top" || at == "top-left" { band } else { 0.0 };
        let mut f = Rect::new(b.x - left, b.y - top, b.w + left + p, b.h + top + p);
        if !r.title.is_empty() {
            if at == "left" {
                if f.h < tw + 2.0 * p {
                    let d = tw + 2.0 * p - f.h;
                    f.y -= d / 2.0;
                    f.h += d;
                }
            } else if f.w < tw + 2.0 * p {
                let d = tw + 2.0 * p - f.w;
                f.x -= d / 2.0;
                f.w += d;
            }
        }
        let (title_rect, ext) = if r.title.is_empty() {
            (None, f)
        } else {
            match at {
                "top" => (Some(Rect::new(f.cx() - tw / 2.0, f.y + k, tw, th)), f),
                "above" => (Some(Rect::new(f.cx() - tw / 2.0, f.y - band + k, tw, th)), f.union(&Rect::new(f.cx() - tw / 2.0, f.y - band, tw, band))),
                "below" => (Some(Rect::new(f.cx() - tw / 2.0, f.bottom() + k, tw, th)), f.union(&Rect::new(f.cx() - tw / 2.0, f.bottom(), tw, band))),
                "left" => (Some(Rect::new(f.x + k, f.cy() - tw / 2.0, th, tw)), f),
                _ => (Some(Rect::new(f.x + p, f.y + k, tw, th)), f),
            }
        };
        let line = if r.line.is_empty() { "solid".to_string() } else { r.line.clone() };
        let at_out = if r.title_at.is_empty() { "top-left".to_string() } else { r.title_at.clone() };
        got[i] = Some((RegionBox { id: r.id.clone(), title: r.title.clone(), rect: f, title_rect, depth: rt.depth[i], line, tint: r.tint.clone(), round: r.round, title_at: at_out }, ext));
    }
    rt.order.iter().filter_map(|&i| got[i].take().map(|(b, e)| (i, b, e))).collect()
}

// ── alignment ──────────────────────────────────────────────────────────

fn align_values(g: &Graph, a: &Align, node: &dyn Fn(usize) -> (f64, f64)) -> Option<Vec<f64>> {
    if a.axis != "row" && a.axis != "col" {
        return None;
    }
    let v: Vec<f64> = a.ids.iter().filter_map(|id| g.node_index(id)).map(|i| {
        let (port, cx) = node(i);
        if a.axis == "row" { port } else { cx }
    }).collect();
    (v.len() >= 2).then_some(v)
}

fn spread(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::MIN, f64::max) - v.iter().copied().fold(f64::MAX, f64::min)
}

/// Shift everything of panel `pi` at or below `cut` (a y, or an x) by `d`, let the panel grow
/// by `d` and push the panels that lie beyond it along. Nothing moves towards anything else,
/// so blocks never come to overlap.
fn shift_from(nodes: &mut [Option<Placed>], panels: &mut [PanelBox], rects: &mut [Rect], pi: usize, vertical: bool, cut: f64, d: f64) {
    for n in nodes.iter_mut().flatten().filter(|n| n.panel == pi) {
        let room = n.rect.union(&n.reserve);
        if (if vertical { room.y } else { room.x }) >= cut - 1e-6 {
            if vertical {
                n.rect.y += d;
                n.reserve.y += d;
                n.port_y += d;
            } else {
                n.rect.x += d;
                n.reserve.x += d;
            }
        }
    }
    let beyond = if vertical { rects[pi].bottom() } else { rects[pi].right() };
    if vertical {
        panels[pi].rect.h += d;
        rects[pi].h += d;
    } else {
        panels[pi].rect.w += d;
        rects[pi].w += d;
    }
    for q in 0..panels.len() {
        if q == pi || (if vertical { rects[q].y } else { rects[q].x }) < beyond - 1e-6 {
            continue;
        }
        let (dx, dy) = if vertical { (0.0, d) } else { (d, 0.0) };
        for r in [&mut panels[q].rect, &mut rects[q]] {
            r.x += dx;
            r.y += dy;
        }
        if let Some(t) = panels[q].title_rect.as_mut() {
            t.x += dx;
            t.y += dy;
        }
        for n in nodes.iter_mut().flatten().filter(|n| n.panel == q) {
            n.rect.x += dx;
            n.reserve.x += dx;
            n.rect.y += dy;
            n.reserve.y += dy;
            n.port_y += dy;
        }
    }
}

/// Make the author's alignments true, as far as moving whole rows / columns of a panel (and the
/// panels beyond it) can do it. A constraint that cannot be met leaves everything as it was.
fn align_pass(g: &Graph, nodes: &mut [Option<Placed>], panels: &mut [PanelBox], rects: &mut [Rect]) {
    if g.align.is_empty() {
        return;
    }
    let values = |a: &Align, nodes: &[Option<Placed>]| align_values(g, a, &|i| nodes[i].as_ref().map(|n| (n.port_y, n.rect.cx())).unwrap_or((0.0, 0.0)));
    for _pass in 0..4 {
        let mut changed = false;
        for a in &g.align {
            let Some(v) = values(a, nodes) else { continue };
            if spread(&v) <= 0.25 {
                continue;
            }
            let snap = (nodes.to_vec(), panels.to_vec(), rects.to_vec());
            let ids: Vec<usize> = a.ids.iter().filter_map(|id| g.node_index(id)).collect();
            let vertical = a.axis == "row";
            for _ in 0..ids.len() * 2 + 2 {
                let v = values(a, nodes).unwrap();
                let target = v.iter().copied().fold(f64::MIN, f64::max);
                let Some((k, _)) = v.iter().enumerate().filter(|(_, x)| **x < target - 0.25).min_by(|x, y| x.1.partial_cmp(y.1).unwrap()) else { break };
                let me = nodes[ids[k]].clone().unwrap();
                let down = panels[me.panel].down;
                // the row (or column) of the block moves as one: its start is the cut
                let same = |n: &Placed| if vertical { if down { n.layer } else { n.lane } } else if down { n.lane } else { n.layer };
                let cut = nodes
                    .iter()
                    .flatten()
                    .filter(|n| n.panel == me.panel && same(n) == same(&me))
                    .map(|n| {
                        let r = n.rect.union(&n.reserve);
                        if vertical { r.y } else { r.x }
                    })
                    .fold(f64::MAX, f64::min);
                shift_from(nodes, panels, rects, me.panel, vertical, cut, target - v[k]);
            }
            if values(a, nodes).is_some_and(|v| spread(&v) <= 0.25) {
                changed = true;
            } else {
                nodes.clone_from_slice(&snap.0);
                panels.clone_from_slice(&snap.1);
                rects.clone_from_slice(&snap.2);
            }
        }
        if !changed {
            break;
        }
    }
}

/// Level what can be level between two panels that stand side by side: a line from the right of
/// a block of one panel to the left of a block of the next, each the only line on that side, is
/// made straight by moving the content of one of the two panels up or down inside its frame
/// (the panel with room for it; never out of the frame, a panel is moved once).
fn level_panels(g: &Graph, needs: &[PanelNeed], nodes: &mut [Option<Placed>], panels: &[PanelBox]) {
    let side_count = |ni: usize, right: bool, nodes: &[Option<Placed>]| -> usize {
        let me = nodes[ni].as_ref().unwrap();
        let id = &g.nodes[ni].id;
        g.edges
            .iter()
            .filter(|e| e.off.is_empty() && (e.from == *id || e.to == *id))
            .filter_map(|e| g.node_index(if e.from == *id { &e.to } else { &e.from }))
            .filter(|o| {
                let o = nodes[*o].as_ref().unwrap();
                if right { o.rect.cx() > me.rect.cx() } else { o.rect.cx() < me.rect.cx() }
            })
            .count()
    };
    let mut moved: BTreeSet<usize> = BTreeSet::new();
    for e in g.edges.iter().filter(|e| e.kind.is_flow() && e.off.is_empty()) {
        let (Some(a), Some(b)) = (g.node_index(&e.from), g.node_index(&e.to)) else { continue };
        let (pa, pb) = (nodes[a].clone().unwrap(), nodes[b].clone().unwrap());
        if pa.panel == pb.panel || panels[pa.panel].down || panels[pb.panel].down || pa.rect.right() > pb.rect.x - 1e-6 {
            continue;
        }
        let d = pa.port_y - pb.port_y;
        if d.abs() < 0.5 || side_count(a, true, nodes) != 1 || side_count(b, false, nodes) != 1 {
            continue;
        }
        // (panel, shift): b's panel moves by d, or a's by -d
        for (p, dy) in [(pb.panel, d), (pa.panel, -d)] {
            if moved.contains(&p) {
                continue;
            }
            let room = nodes.iter().flatten().filter(|n| n.panel == p).fold(None::<Rect>, |acc, n| {
                let r = n.rect.union(&n.reserve);
                Some(acc.map_or(r, |a| a.union(&r)))
            });
            let Some(room) = room else { continue };
            let f = panels[p].rect;
            let (lo, hi) = (f.y + needs[p].top, f.bottom() - needs[p].bottom);
            if room.y + dy < lo - 1e-6 || room.bottom() + dy > hi + 1e-6 {
                continue;
            }
            for n in nodes.iter_mut().flatten().filter(|n| n.panel == p) {
                n.rect.y += dy;
                n.reserve.y += dy;
                n.port_y += dy;
            }
            moved.insert(pa.panel);
            moved.insert(pb.panel);
            break;
        }
    }
}

/// The alignments of the graph that the layout does not meet (tolerance 0.5 pt), each as a
/// line of words naming the axis and the blocks.
pub fn unmet_alignments(g: &Graph, lay: &Layout) -> Vec<String> {
    let mut out = Vec::new();
    for a in &g.align {
        let v = align_values(g, a, &|i| lay.nodes.get(i).map(|n| (n.port_y, n.rect.cx())).unwrap_or((0.0, 0.0)));
        if v.is_some_and(|v| spread(&v) > 0.5) {
            out.push(format!("align {}: {}", a.axis, a.ids.join(" ")));
        }
    }
    out
}

/// Would adding u → v close a cycle? (is u reachable from v)
fn reaches(succ: &BTreeMap<usize, Vec<usize>>, from: usize, to: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(x) = stack.pop() {
        if x == to {
            return true;
        }
        if seen.insert(x) {
            stack.extend(succ.get(&x).into_iter().flatten().copied());
        }
    }
    false
}

fn plan_panel(g: &Graph, nodes: &[usize], sizes: &[Size], down: bool, theme: &Theme, rt: &RTree, avail: f64) -> PanelPlan {
    let inside: BTreeSet<usize> = nodes.iter().copied().collect();
    let idx = |id: &str| g.node_index(id).filter(|i| inside.contains(i));
    // flow edges inside the panel, acyclic
    let mut succ: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut pred: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for e in g.edges.iter().filter(|e| e.kind.is_flow()) {
        let (Some(a), Some(b)) = (idx(&e.from), idx(&e.to)) else { continue };
        if reaches(&succ, b, a) || succ.get(&a).is_some_and(|v| v.contains(&b)) {
            continue;
        }
        succ.entry(a).or_default().push(b);
        pred.entry(b).or_default().push(a);
    }
    let has_flow = |n: usize| succ.contains_key(&n) || pred.contains_key(&n);
    // layers: longest path from a source
    let mut layer: BTreeMap<usize, usize> = BTreeMap::new();
    fn depth(n: usize, pred: &BTreeMap<usize, Vec<usize>>, memo: &mut BTreeMap<usize, usize>) -> usize {
        if let Some(d) = memo.get(&n) {
            return *d;
        }
        let d = pred.get(&n).into_iter().flatten().map(|p| depth(*p, pred, memo) + 1).max().unwrap_or(0);
        memo.insert(n, d);
        d
    }
    for &n in nodes {
        if has_flow(n) {
            depth(n, &pred, &mut layer);
        }
    }
    // an input sits right before what it feeds
    for &n in nodes {
        if !pred.contains_key(&n)
            && let Some(s) = succ.get(&n)
        {
            let m = s.iter().map(|x| layer[x]).min().unwrap_or(1);
            layer.insert(n, m.saturating_sub(1));
        }
    }
    // nodes outside the flow follow the node they are tied to
    let mut anchor: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pending: Vec<usize> = nodes.iter().copied().filter(|n| !has_flow(*n)).collect();
    for _ in 0..nodes.len() + 1 {
        let mut rest = Vec::new();
        for &n in &pending {
            let id = &g.nodes[n].id;
            let ties: Vec<(usize, bool, EdgeKind)> = g
                .edges
                .iter()
                .filter_map(|e| {
                    if e.from == *id {
                        idx(&e.to).filter(|t| layer.contains_key(t)).map(|t| (t, true, e.kind))
                    } else if e.to == *id {
                        idx(&e.from).filter(|t| layer.contains_key(t)).map(|t| (t, false, e.kind))
                    } else {
                        None
                    }
                })
                .collect();
            // what it is compared with decides before where it comes from
            let tie = ties.iter().find(|t| t.2 == EdgeKind::Compare || (t.1 && t.2 == EdgeKind::Supervision)).or(ties.first()).copied();
            match tie {
                Some((t, outgoing, kind)) => {
                    // a target sits to the right of the loss it is compared in (arrow pointing back)
                    let right_free = !succ.contains_key(&t);
                    let beside = kind == EdgeKind::Compare || (outgoing && kind == EdgeKind::Supervision);
                    let l = if beside && right_free { layer[&t] + 1 } else { layer[&t] };
                    layer.insert(n, l);
                    anchor.insert(n, t);
                }
                None => rest.push(n),
            }
        }
        if rest.len() == pending.len() {
            break;
        }
        pending = rest;
    }
    for &n in nodes {
        layer.entry(n).or_insert(0);
        if let Some(c) = g.nodes[n].num("col") {
            layer.insert(n, c.max(0.0) as usize);
        }
    }

    // lanes: the longest chain is the trunk, the others take the nearest free lane
    let mut lane: BTreeMap<usize, i32> = BTreeMap::new();
    let mut used: BTreeSet<(usize, i32)> = BTreeSet::new();
    let mut remaining: BTreeSet<usize> = nodes.iter().copied().filter(|n| has_flow(*n)).collect();
    // topological order (pinned columns may run against the flow)
    let mut order: Vec<usize> = Vec::new();
    {
        let mut left: BTreeMap<usize, usize> = nodes.iter().map(|n| (*n, pred.get(n).map(|p| p.len()).unwrap_or(0))).collect();
        let mut ready: BTreeSet<usize> = left.iter().filter(|(_, c)| **c == 0).map(|(n, _)| *n).collect();
        while let Some(n) = ready.pop_first() {
            order.push(n);
            for s in succ.get(&n).into_iter().flatten() {
                let c = left.get_mut(s).unwrap();
                *c -= 1;
                if *c == 0 {
                    ready.insert(*s);
                }
            }
        }
    }
    while !remaining.is_empty() {
        let mut len: BTreeMap<usize, (usize, Option<usize>)> = BTreeMap::new();
        for &n in order.iter().filter(|n| remaining.contains(n)) {
            let best = pred.get(&n).into_iter().flatten().filter(|p| remaining.contains(p)).map(|p| (len[p].0, *p)).max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            len.insert(n, best.map(|(l, p)| (l + 1, Some(p))).unwrap_or((1, None)));
        }
        let end = *len.iter().max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.0.cmp(a.0))).unwrap().0;
        let mut path = vec![end];
        while let Some(p) = len[path.last().unwrap()].1 {
            path.push(p);
        }
        path.reverse();
        let (head, tail) = (path[0], *path.last().unwrap());
        let placed_pred: Vec<usize> = pred.get(&head).into_iter().flatten().copied().filter(|p| lane.contains_key(p)).collect();
        let placed_succ: Vec<usize> = succ.get(&tail).into_iter().flatten().copied().filter(|s| lane.contains_key(s)).collect();
        let mut from = layer[&head];
        let mut to = layer[&tail];
        if let Some(p) = placed_pred.iter().map(|p| layer[p]).max() {
            from = from.min(p + 1);
        }
        if let Some(s) = placed_succ.iter().map(|s| layer[s]).min() {
            to = to.max(s.saturating_sub(1));
        }
        let (from, to) = (from.min(to), from.max(to));
        let free = |l: i32, used: &BTreeSet<(usize, i32)>| (from..=to).all(|c| !used.contains(&(c, l)));
        let pick = if lane.is_empty() {
            0
        } else {
            let (reference, prefer) = match (placed_succ.first(), placed_pred.first()) {
                (Some(s), None) => (lane[s], -1),
                (Some(s), Some(_)) => (lane[s], 1),
                (None, Some(p)) => (lane[p], 1),
                (None, None) => (used.iter().map(|u| u.1).max().unwrap_or(0) + 1, 1),
            };
            let mut cands = vec![reference];
            for d in 1..=(nodes.len() as i32 + 2) {
                cands.push(reference + d * prefer);
                cands.push(reference - d * prefer);
            }
            cands.into_iter().find(|l| free(*l, &used)).unwrap_or(reference)
        };
        for c in from..=to {
            used.insert((c, pick));
        }
        for n in path {
            lane.insert(n, pick);
            remaining.remove(&n);
        }
    }
    // nodes outside the flow: beside their anchor
    for &n in nodes {
        if lane.contains_key(&n) {
            continue;
        }
        let l = layer[&n];
        let pick = match anchor.get(&n).and_then(|a| lane.get(a).copied().map(|al| (al, layer[a]))) {
            Some((al, alayer)) => {
                let mut cands = if alayer != l { vec![al] } else { vec![] };
                for d in 1..=(nodes.len() as i32 + 2) {
                    cands.push(al - d);
                    cands.push(al + d);
                }
                cands.into_iter().find(|c| !used.contains(&(l, *c))).unwrap_or(al)
            }
            None => {
                let mut c = 0;
                while used.contains(&(l, c)) {
                    c += 1;
                }
                c
            }
        };
        used.insert((l, pick));
        lane.insert(n, pick);
    }
    for &n in nodes {
        if let Some(v) = g.nodes[n].num("lane") {
            lane.insert(n, v as i32);
        }
    }
    // compact layers and lanes to 0..n
    let layers: Vec<usize> = layer.values().copied().collect::<BTreeSet<_>>().into_iter().collect();
    let lanes: Vec<i32> = lane.values().copied().collect::<BTreeSet<_>>().into_iter().collect();
    let layer: BTreeMap<usize, usize> = layer.into_iter().map(|(n, l)| (n, layers.iter().position(|x| *x == l).unwrap())).collect();
    let mut lane: BTreeMap<usize, usize> = lane.into_iter().map(|(n, l)| (n, lanes.iter().position(|x| *x == l).unwrap())).collect();

    // regions: the blocks of one region lie together. When a region's block of cells holds a
    // block that is not its own (or meets a sibling's), every region gets lanes of its own.
    let mut rctx: Option<RCtx> = None;
    if !rt.order.is_empty() {
        let scope: BTreeSet<usize> = inside.clone();
        if region_conflict(rt, &scope, &rt.roots, &layer, &lane) {
            let mut out = BTreeMap::new();
            stack_lanes(rt, &scope, &rt.roots, &layer, &lane, &mut out, 0);
            for &n in nodes {
                out.entry(n).or_insert(0);
            }
            lane = out;
        }
        let mut range = BTreeMap::new();
        let mut lb = BTreeMap::new();
        for &i in &rt.order {
            let set: Vec<usize> = rt.all[i].iter().copied().filter(|n| scope.contains(n)).collect();
            if set.is_empty() {
                continue;
            }
            let (l, c) = (set.iter().map(|n| layer[n]), set.iter().map(|n| lane[n]));
            let (lr, cr) = ((l.clone().min().unwrap(), l.max().unwrap()), (c.clone().min().unwrap(), c.max().unwrap()));
            range.insert(i, if down { (cr, lr) } else { (lr, cr) });
            lb.insert(i, (set.iter().map(|n| sizes[*n].reserve_w).fold(0.0, f64::max), set.iter().map(|n| sizes[*n].h).fold(0.0, f64::max)));
        }
        if !range.is_empty() {
            rctx = Some(RCtx { g, rt, theme, range, lb });
        }
    }

    // cells: along = the flow axis, across = the lane axis
    let (n_along, n_across) = (layers.len(), lane.values().max().map_or(0, |m| m + 1));
    let mut along = vec![0.0f64; n_along];
    // a row is as tall as the tallest part above plus the tallest part below the centre line
    let mut up = vec![0.0f64; n_across];
    let mut dn = vec![0.0f64; n_across];
    let mut across = vec![0.0f64; n_across];
    for &n in nodes {
        let s = &sizes[n];
        if down {
            along[layer[&n]] = along[layer[&n]].max(s.h);
            across[lane[&n]] = across[lane[&n]].max(s.reserve_w);
        } else {
            along[layer[&n]] = along[layer[&n]].max(s.reserve_w);
            up[lane[&n]] = up[lane[&n]].max(s.cy);
            dn[lane[&n]] = dn[lane[&n]].max(s.h - s.cy);
        }
    }
    if !down {
        for i in 0..n_across {
            across[i] = up[i] + dn[i];
        }
    }
    // room for labels on the arrows between neighbouring layers
    let (gap_along, gap_across) = if down { (theme.space.gap_y, theme.space.gap_x) } else { (theme.space.gap_x, theme.space.gap_y) };
    let mut gaps = vec![gap_along; n_along.saturating_sub(1)];
    for e in g.edges.iter().filter(|e| !e.label.is_empty()) {
        let (Some(a), Some(b)) = (idx(&e.from), idx(&e.to)) else { continue };
        let (la, lb) = (layer[&a].min(layer[&b]), layer[&a].max(layer[&b]));
        if lb == la + 1 {
            let need = if down { theme.small_size() * 1.4 + 10.0 } else { text::width(&e.label, theme.small_size()) + 14.0 };
            gaps[la] = gaps[la].max(need);
        }
    }
    let mut across_gaps = vec![gap_across; n_across.saturating_sub(1)];
    let mut margin = [0.0f64; 4];
    if let Some(cx) = &rctx {
        let (nx, ny) = if down { (n_across, n_along) } else { (n_along, n_across) };
        let (xn, yn, m) = cx.spacing(nx, ny, avail);
        let (xg, yg) = if down { (&mut across_gaps, &mut gaps) } else { (&mut gaps, &mut across_gaps) };
        for (g_, n) in xg.iter_mut().zip(xn) {
            *g_ = g_.max(n);
        }
        for (g_, n) in yg.iter_mut().zip(yn) {
            *g_ = g_.max(n);
        }
        margin = m;
    }
    let mut along_pos = vec![0.0; n_along];
    for i in 1..n_along {
        along_pos[i] = along_pos[i - 1] + along[i - 1] + gaps[i - 1];
    }
    let mut across_pos = vec![0.0; n_across];
    for i in 1..n_across {
        across_pos[i] = across_pos[i - 1] + across[i - 1] + across_gaps[i - 1];
    }
    let mut pos = BTreeMap::new();
    for &n in nodes {
        let s = &sizes[n];
        let r = if down {
            let (cx, cy) = (across_pos[lane[&n]] + across[lane[&n]] / 2.0, along_pos[layer[&n]] + along[layer[&n]] / 2.0);
            Rect::new(cx - s.w / 2.0, cy - s.h / 2.0, s.w, s.h)
        } else {
            let cx = along_pos[layer[&n]] + along[layer[&n]] / 2.0;
            Rect::new(cx - s.w / 2.0, across_pos[lane[&n]] + up[lane[&n]] - s.cy, s.w, s.h)
        };
        pos.insert(n, Rect::new(r.x + margin[0], r.y + margin[1], r.w, r.h));
    }
    let total_along = along_pos.last().copied().unwrap_or(0.0) + along.last().copied().unwrap_or(0.0);
    let total_across = across_pos.last().copied().unwrap_or(0.0) + across.last().copied().unwrap_or(0.0);
    let content = if down { (total_across, total_along) } else { (total_along, total_across) };
    let content = (content.0 + margin[0] + margin[2], content.1 + margin[1] + margin[3]);
    PanelPlan { nodes: nodes.to_vec(), layer, lane, pos, content, down }
}

struct PanelNeed {
    id: String,
    title: String,
    bare: bool,
    plan: PanelPlan,
    /// Needed outer size.
    w: f64,
    h: f64,
    /// Offset of the content area inside the panel and room above the frame.
    top: f64,
    bottom: f64,
    outside_title: f64,
    /// Room under the frame for a `below` title, and the upright strip of a `left` one.
    below_title: f64,
    left_strip: f64,
    /// Where the title is drawn: `""` (the theme's way), `top-left`, `top`, `above`, `below`, `left`.
    at: String,
}

fn arrangement_size(rows: &[Vec<usize>], needs: &[PanelNeed], gap: f64) -> (f64, f64) {
    let mut w = 0.0f64;
    let mut h = 0.0;
    for (i, r) in rows.iter().enumerate() {
        let rw: f64 = r.iter().map(|p| needs[*p].w).sum::<f64>() + gap * (r.len().saturating_sub(1)) as f64;
        let rh = r.iter().map(|p| needs[*p].h).fold(0.0, f64::max);
        w = w.max(rw);
        h += rh + if i > 0 { gap } else { 0.0 };
    }
    (w, h)
}

/// Lay a graph out. The graph must be valid ([`Graph::validate`]).
pub fn layout(g: &Graph, theme: &Theme, vars: &Vars, lib: &Library, opt: &LayoutOptions) -> Layout {
    layout_with(g, theme, vars, lib, opt, &modules::no_images)
}

/// As [`layout`], knowing the pixel sizes of the picture files the graph names.
///
/// A panel that does not say its direction (and has no pinned node) is tried
/// left-to-right and top-down; the combination that fits the frame at the
/// largest scale is kept, left-to-right winning unless top-down is clearly better.
pub fn layout_with(g: &Graph, theme: &Theme, vars: &Vars, lib: &Library, opt: &LayoutOptions, image_size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Layout {
    let mut lay = layout_auto(g, theme, vars, lib, opt, image_size);
    apply_place(&mut lay, g, opt, theme);
    lay
}

/// Move the blocks the author moved: each named node's rectangle is shifted by its offset
/// (times the scale), kept inside the figure's frame and clear of the legends; the panel of
/// a moved node grows to hold it (its title keeps its corner) and the bounds grow with it.
fn apply_place(lay: &mut Layout, g: &Graph, opt: &LayoutOptions, theme: &Theme) {
    if opt.place.is_empty() {
        return;
    }
    let k = lay.scale;
    let frame = opt.frame;
    // the room a block may take: the frame, less the legends above and below the panels
    let content = lay.panels.iter().skip(1).fold(lay.panels[0].rect, |a, p| a.union(&p.rect));
    let (mut top, mut bottom) = (frame.y, frame.bottom());
    for r in lay.legend.iter().map(|l| l.text).chain(lay.swatches.iter().map(|s| s.text)) {
        if r.cy() >= content.bottom() {
            bottom = bottom.min(r.y);
        } else if r.cy() <= content.y {
            top = top.max(r.bottom());
        }
    }
    let margin = 3.0 * k;
    for (id, off) in &opt.place {
        let Some(i) = g.node_index(id) else { continue };
        let n = &mut lay.nodes[i];
        let u = n.rect.union(&n.reserve);
        let (mut dx, mut dy) = (off[0] * k, off[1] * k);
        let (lo, hi) = (frame.x - u.x, frame.right() - u.right());
        if lo <= hi {
            dx = dx.clamp(lo, hi);
        }
        let (lo, hi) = (top - u.y, bottom - u.bottom());
        if lo <= hi {
            dy = dy.clamp(lo, hi);
        }
        n.rect.x += dx;
        n.rect.y += dy;
        n.reserve.x += dx;
        n.reserve.y += dy;
        n.port_y += dy;
        let u = n.rect.union(&n.reserve).inflate(margin);
        let p = &mut lay.panels[n.panel];
        let grown = p.rect.union(&u);
        let (gx, gy) = (grown.x - p.rect.x, grown.y - p.rect.y);
        if gx != 0.0 || gy != 0.0 {
            p.title_rect = p.title_rect.map(|t| Rect::new(t.x + gx, t.y + gy, t.w, t.h));
        }
        p.rect = grown;
        lay.bounds = lay.bounds.union(&grown);
    }
    // the frames of the regions follow their members
    if !lay.regions.is_empty() {
        let rt = RTree::new(g);
        let rooms: Vec<Rect> = lay.nodes.iter().map(|n| n.rect.union(&n.reserve)).collect();
        let framed = frame_regions(g, &rt, &rooms, theme, k);
        for (i, _, ext) in &framed {
            let Some(&ni) = rt.all[*i].iter().next() else { continue };
            let p = &mut lay.panels[lay.nodes[ni].panel];
            let grown = p.rect.union(&ext.inflate(margin));
            let (gx, gy) = (grown.x - p.rect.x, grown.y - p.rect.y);
            if gx != 0.0 || gy != 0.0 {
                p.title_rect = p.title_rect.map(|t| Rect::new(t.x + gx, t.y + gy, t.w, t.h));
            }
            p.rect = grown;
            lay.bounds = lay.bounds.union(&grown);
        }
        lay.regions = framed.into_iter().map(|(_, b, _)| b).collect();
    }
}

fn layout_auto(g: &Graph, theme: &Theme, vars: &Vars, lib: &Library, opt: &LayoutOptions, image_size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Layout {
    let pinned = |p: &Panel| g.nodes.iter().any(|n| n.panel == p.id && (n.params.contains_key("col") || n.params.contains_key("lane")));
    let auto: Vec<usize> = (0..g.panels.len()).filter(|i| g.panels[*i].dir.is_empty() && !g.panels[*i].bare && !pinned(&g.panels[*i])).collect();
    if auto.is_empty() || auto.len() > 5 {
        return layout_fixed(g, theme, vars, lib, opt, image_size);
    }
    let mut best: Option<(f64, Layout)> = None;
    for mask in 0..(1u32 << auto.len()) {
        let mut h = g.clone();
        for (bit, pi) in auto.iter().enumerate() {
            h.panels[*pi].dir = if mask & (1 << bit) != 0 { "down".into() } else { "right".into() };
        }
        let lay = layout_fixed(&h, theme, vars, lib, opt, image_size);
        // each top-down panel must earn its place
        let score = lay.scale * 0.92f64.powi(mask.count_ones() as i32);
        if best.as_ref().is_none_or(|b| score > b.0 + 1e-9) {
            best = Some((score, lay));
        }
    }
    best.expect("at least one combination").1
}

fn layout_fixed(g: &Graph, theme: &Theme, vars: &Vars, lib: &Library, opt: &LayoutOptions, image_size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Layout {
    let down_panels: BTreeSet<&str> = g.panels.iter().filter(|p| p.dir == "down").map(|p| p.id.as_str()).collect();
    let sizes: Vec<Size> = g
        .nodes
        .iter()
        .map(|n| {
            let cx = Ctx { theme, vars, math: &NoMath, lib, k: 1.0, symmetric: !down_panels.contains(n.panel.as_str()), image_size };
            let b = modules::make(n, &cx);
            Size { w: b.w, h: b.h, reserve_w: b.reserve_w, cy: b.cy, single: b.single, ellipse: b.ellipse, legend: b.legend }
        })
        .collect();

    // panels in declaration order; loose nodes form a bare panel "_"
    let mut defs: Vec<Panel> = g.panels.clone();
    let loose: Vec<usize> = (0..g.nodes.len()).filter(|i| g.nodes[*i].panel.is_empty()).collect();
    if !loose.is_empty() {
        let p = Panel { id: "_".into(), bare: true, ..Default::default() };
        // inputs go first, everything else last
        let feeds = g.edges.iter().filter(|e| g.node(&e.from).is_some_and(|n| n.panel.is_empty()) && g.node(&e.to).is_some_and(|n| !n.panel.is_empty())).count();
        let fed = g.edges.iter().filter(|e| g.node(&e.to).is_some_and(|n| n.panel.is_empty()) && g.node(&e.from).is_some_and(|n| !n.panel.is_empty())).count();
        if g.panels.is_empty() || feeds >= fed {
            defs.insert(0, p);
        } else {
            defs.push(p);
        }
    }
    let rt = RTree::new(g);
    let pad = theme.space.pad;
    let title_size = theme.title_size();
    let outside = theme.panel.title.starts_with("outside");
    let needs: Vec<PanelNeed> = defs
        .iter()
        .map(|p| {
            let nodes: Vec<usize> = (0..g.nodes.len()).filter(|i| if p.id == "_" { g.nodes[*i].panel.is_empty() } else { g.nodes[*i].panel == p.id }).collect();
            let bare = p.bare || (p.id == "_");
            let plan = plan_panel(g, &nodes, &sizes, p.dir == "down", theme, &rt, if bare { 2.0 } else { theme.space.pad });
            let at = p.title_at.as_str();
            let has_title = !p.title.is_empty() && !bare && at != "none";
            let title_h = if has_title { title_size * 1.3 + 2.0 } else { 0.0 };
            let title_w = if has_title { text::width(&p.title, title_size) * 1.06 + 2.0 * pad } else { 0.0 };
            // where the title's room is taken
            let out_top = has_title && (at == "above" || (at.is_empty() && outside));
            let out_bottom = has_title && at == "below";
            let strip = if has_title && at == "left" { title_h } else { 0.0 };
            let in_top = has_title && !out_top && !out_bottom && strip == 0.0;
            // lines that go around the row need a channel under it
            let around = g.edges.iter().any(|e| matches!(e.kind, EdgeKind::Feedback | EdgeKind::Skip) && nodes.iter().any(|n| g.nodes[*n].id == e.from || g.nodes[*n].id == e.to));
            let (p_in, top, bottom) = if bare { (2.0, 2.0, 2.0) } else { (pad, pad + if in_top { title_h } else { 0.0 }, pad + if around { 10.0 } else { 0.0 }) };
            let w = (plan.content.0 + 2.0 * p_in + strip).max(title_w);
            let upright = if strip > 0.0 { text::width(&p.title, title_size) * 1.06 + 2.0 * pad } else { 0.0 };
            let h = (plan.content.1 + top + bottom).max(upright) + if out_top { title_h } else { 0.0 } + if out_bottom { title_h } else { 0.0 };
            PanelNeed { id: p.id.clone(), title: p.title.clone(), bare, plan, w, h, top, bottom, outside_title: if out_top { title_h } else { 0.0 }, below_title: if out_bottom { title_h } else { 0.0 }, left_strip: strip, at: at.to_string() }
        })
        .collect();

    let gap = theme.space.panel_gap;
    let by_id = |id: &str| needs.iter().position(|n| n.id == id);
    let n = needs.len();
    // panel rects at natural size
    let mut rects: Vec<Rect> = vec![Rect::default(); n];
    let frac = opt.template.as_ref().filter(|t| t.id == g.template && t.panels.len() >= n && g.arrange.is_empty());
    if let Some(t) = frac {
        let k = (0..n).map(|i| (t.panels[i].w * opt.frame.w / needs[i].w).min(t.panels[i].h * opt.frame.h / needs[i].h)).fold(f64::MAX, f64::min);
        let (fw, fh) = (opt.frame.w / k, opt.frame.h / k);
        for i in 0..n {
            let r = t.panels[i];
            rects[i] = Rect::new(r.x * fw, r.y * fh, r.w * fw, r.h * fh);
        }
    } else {
        let rows: Vec<Vec<usize>> = if !g.arrange.is_empty() {
            let mut rows: Vec<Vec<usize>> = g.arrange.iter().map(|r| r.iter().filter_map(|id| by_id(id)).collect()).collect();
            // a bare panel the arrangement does not mention keeps its side
            if let Some(b) = by_id("_").filter(|b| !rows.iter().flatten().any(|x| x == b)) {
                if b == 0 {
                    rows[0].insert(0, b);
                } else {
                    rows.last_mut().unwrap().push(b);
                }
            }
            rows.retain(|r| !r.is_empty());
            rows
        } else {
            match g.template.as_str() {
                "rows" => (0..n).map(|i| vec![i]).collect(),
                "cols" => vec![(0..n).collect()],
                _ => {
                    // the arrangement that fits the frame at the largest scale
                    let mut cands: Vec<Vec<Vec<usize>>> = vec![vec![(0..n).collect()], (0..n).map(|i| vec![i]).collect()];
                    for cut in 1..n {
                        cands.push(vec![(0..cut).collect(), (cut..n).collect()]);
                    }
                    for a in 1..n {
                        for b in a + 1..n {
                            cands.push(vec![(0..a).collect(), (a..b).collect(), (b..n).collect()]);
                        }
                    }
                    let score = |rows: &Vec<Vec<usize>>| {
                        let (w, h) = arrangement_size(rows, &needs, gap);
                        (opt.frame.w / w).min(opt.frame.h / h)
                    };
                    let mut best = 0;
                    for i in 1..cands.len() {
                        if score(&cands[i]) > score(&cands[best]) * 1.03 {
                            best = i;
                        }
                    }
                    cands.swap_remove(best)
                }
            }
        };
        let (total_w, _) = arrangement_size(&rows, &needs, gap);
        let mut y = 0.0;
        for r in &rows {
            let rh = r.iter().map(|p| needs[*p].h).fold(0.0, f64::max);
            let rw: f64 = r.iter().map(|p| needs[*p].w).sum::<f64>() + gap * (r.len() - 1) as f64;
            let sum_w: f64 = r.iter().map(|p| needs[*p].w).sum();
            let extra = total_w - rw;
            let mut x = 0.0;
            for &p in r {
                let w = needs[p].w + extra * needs[p].w / sum_w;
                rects[p] = Rect::new(x, y, w, rh);
                x += w + gap;
            }
            y += rh + gap;
        }
    }

    // nodes: content centred in the panel's inner area
    let mut nodes: Vec<Option<Placed>> = (0..g.nodes.len()).map(|_| None).collect();
    let mut panels = Vec::new();
    for (i, need) in needs.iter().enumerate() {
        let r = rects[i];
        let frame = Rect::new(r.x, r.y + need.outside_title, r.w, r.h - need.outside_title - need.below_title);
        let inner_h = frame.h - need.top - need.bottom;
        let ox = frame.x + need.left_strip + (frame.w - need.left_strip - need.plan.content.0) / 2.0;
        let oy = frame.y + need.top + (inner_h - need.plan.content.1) / 2.0;
        for &ni in &need.plan.nodes {
            let p = need.plan.pos[&ni];
            let rect = Rect::new(ox + p.x, oy + p.y, p.w, p.h);
            let s = &sizes[ni];
            let reserve = Rect::new(rect.cx() - s.reserve_w / 2.0, rect.y, s.reserve_w, rect.h);
            nodes[ni] = Some(Placed { rect, reserve, port_y: rect.y + s.cy, panel: i, layer: need.plan.layer[&ni], lane: need.plan.lane[&ni], single: s.single, ellipse: s.ellipse, ends: BTreeMap::new(), origin: (0.0, 0.0) });
        }
        let title_rect = (!need.title.is_empty() && !need.bare && need.at != "none").then(|| {
            let tw = text::width(&need.title, title_size) * 1.06;
            let th = title_size * 1.3;
            match need.at.as_str() {
                "above" => Rect::new(frame.cx() - tw / 2.0, r.y, tw, th),
                "below" => Rect::new(frame.cx() - tw / 2.0, frame.bottom() + 1.0, tw, th),
                "left" => Rect::new(frame.x + 1.0, frame.cy() - tw / 2.0, th, tw),
                "top" => Rect::new(frame.cx() - tw / 2.0, frame.y + pad * 0.4, tw, th),
                "top-left" => Rect::new(frame.x + pad * 0.6, frame.y + pad * 0.4, tw, th),
                _ => {
                    let centre = theme.panel.title.ends_with("center");
                    let x = if centre { frame.cx() - tw / 2.0 } else if outside { frame.x } else { frame.x + pad * 0.6 };
                    let y = if outside { r.y } else { frame.y + pad * 0.4 };
                    Rect::new(x, y, tw, th)
                }
            }
        });
        panels.push(PanelBox { id: need.id.clone(), title: need.title.clone(), rect: frame, title_rect, bare: need.bare, down: need.plan.down });
    }
    level_panels(g, &needs, &mut nodes, &panels);
    align_pass(g, &mut nodes, &mut panels, &mut rects);
    let mut bounds = rects.iter().skip(1).fold(rects[0], |a, b| a.union(b));
    let regions: Vec<RegionBox> = if rt.order.is_empty() {
        Vec::new()
    } else {
        let rooms: Vec<Rect> = nodes.iter().map(|n| n.as_ref().map(|n| n.rect.union(&n.reserve)).unwrap_or_default()).collect();
        frame_regions(g, &rt, &rooms, theme, 1.0).into_iter().map(|(_, b, _)| b).collect()
    };

    // legend: one centred row (wrapping) under the figure
    let mut legend = Vec::new();
    // kinds that look alike in this theme share one entry (the first kind's name)
    let mut kinds: Vec<EdgeKind> = Vec::new();
    for k in g.line_kinds() {
        let s = theme.edge(k);
        let same = |o: &EdgeKind| {
            let t = theme.edge(*o);
            t.color == s.color && t.dash == s.dash && t.head == s.head && t.tail == s.tail && (t.width - s.width).abs() < 0.6
        };
        if !kinds.iter().any(same) {
            kinds.push(k);
        }
    }
    let wanted = g.legend.unwrap_or(kinds.iter().filter(|k| **k != EdgeKind::Data).count() >= 4);
    if let Some(choice) = opt.legend.as_ref().filter(|c| !c.rows.is_empty()) {
        // the author's rows, centred, wrapping at the figure's width
        let size = theme.small_size();
        let (sample, sq, sp, between) = (26.0, size * 0.9, 5.0, 16.0);
        let mark_w = |r: &LegendRow| if matches!(r.mark, LegendMark::Line(_)) { sample } else { sq };
        let widths: Vec<f64> = choice.rows.iter().map(|r| mark_w(r) + sp + text::width(&r.label, size) + 2.0).collect();
        let mut rows: Vec<Vec<usize>> = vec![vec![]];
        let mut cur = 0.0;
        for (i, w) in widths.iter().enumerate() {
            if cur + w > bounds.w.max(240.0) && !rows.last().unwrap().is_empty() {
                rows.push(vec![]);
                cur = 0.0;
            }
            rows.last_mut().unwrap().push(i);
            cur += w + between;
        }
        let lh = size * 1.6;
        let height = lh * rows.len() as f64;
        let mut y = if choice.above { bounds.y - gap * 0.6 - height } else { bounds.bottom() + gap * 0.6 };
        let top = y;
        for r in rows {
            let total: f64 = r.iter().map(|i| widths[*i]).sum::<f64>() + between * (r.len() - 1) as f64;
            let mut x = bounds.x + (bounds.w - total) / 2.0;
            for i in r {
                let row = &choice.rows[i];
                let cy = y + lh / 2.0;
                let mw = mark_w(row);
                let (kind, boxed, line) = match &row.mark {
                    LegendMark::Line(k) => (*k, None, ((x, cy), (x + mw, cy))),
                    LegendMark::Box { fill, line } => (EdgeKind::Data, Some((fill.clone(), line.clone())), ((x, cy - sq / 2.0), (x + sq, cy + sq / 2.0))),
                };
                legend.push(LegendItem { kind, label: row.label.clone(), boxed, line, text: Rect::new(x + mw + sp, y, widths[i] - mw - sp, lh) });
                x += widths[i] + between;
            }
            y += lh;
        }
        if choice.above {
            bounds.h += bounds.y - top;
            bounds.y = top;
        } else {
            bounds.h = y - bounds.y;
        }
    } else if opt.legend.is_none() && wanted && kinds.len() > 1 {
        let size = theme.small_size();
        let (sample, sp, between) = (26.0, 5.0, 16.0);
        let widths: Vec<f64> = kinds.iter().map(|k| sample + sp + text::width(k.label(), size) + 2.0).collect();
        let mut rows: Vec<Vec<usize>> = vec![vec![]];
        let mut cur = 0.0;
        for (i, w) in widths.iter().enumerate() {
            if cur + w > bounds.w && !rows.last().unwrap().is_empty() {
                rows.push(vec![]);
                cur = 0.0;
            }
            rows.last_mut().unwrap().push(i);
            cur += w + between;
        }
        let lh = size * 1.6;
        let mut y = bounds.bottom() + gap * 0.6;
        for r in rows {
            let total: f64 = r.iter().map(|i| widths[*i]).sum::<f64>() + between * (r.len() - 1) as f64;
            let mut x = bounds.x + (bounds.w - total) / 2.0;
            for i in r {
                let cy = y + lh / 2.0;
                legend.push(LegendItem { kind: kinds[i], label: String::new(), boxed: None, line: ((x, cy), (x + sample, cy)), text: Rect::new(x + sample + sp, y, widths[i] - sample - sp, lh) });
                x += widths[i] + between;
            }
            y += lh;
        }
        bounds.h = y - bounds.y;
    }

    // colour legend: one centred row (wrapping) over the figure
    let mut swatches = Vec::new();
    let mut seen: Vec<crate::art::Swatch> = Vec::new();
    for s in sizes.iter().flat_map(|s| s.legend.iter()) {
        if !seen.iter().any(|x| x.key == s.key) {
            seen.push(s.clone());
        }
    }
    if opt.legend.is_none() && g.legend != Some(false) && seen.len() >= 2 {
        let size = theme.small_size();
        let (sq, sp, between) = (size * 0.9, 4.0, 12.0);
        let widths: Vec<f64> = seen.iter().map(|s| sq + sp + text::width(&s.label, size) + 2.0).collect();
        let mut rows: Vec<Vec<usize>> = vec![vec![]];
        let mut cur = 0.0;
        for (i, w) in widths.iter().enumerate() {
            if cur + w > bounds.w.max(240.0) && !rows.last().unwrap().is_empty() {
                rows.push(vec![]);
                cur = 0.0;
            }
            rows.last_mut().unwrap().push(i);
            cur += w + between;
        }
        let lh = size * 1.6;
        let height = lh * rows.len() as f64;
        let mut y = bounds.y - gap * 0.6 - height;
        for r in rows {
            let total: f64 = r.iter().map(|i| widths[*i]).sum::<f64>() + between * (r.len() - 1) as f64;
            let mut x = bounds.x + (bounds.w - total) / 2.0;
            for i in r {
                let cy = y + lh / 2.0;
                swatches.push(SwatchItem { swatch: seen[i].clone(), at: Rect::new(x, cy - sq / 2.0, sq, sq), text: Rect::new(x + sq + sp, y, widths[i] - sq - sp, lh) });
                x += widths[i] + between;
            }
            y += lh;
        }
        let top = bounds.y - gap * 0.6 - height;
        bounds.h += bounds.y - top;
        bounds.y = top;
    }

    // fit into the frame
    let k = (opt.frame.w / bounds.w).min(opt.frame.h / bounds.h).min(opt.max_scale);
    let ox = opt.frame.x + (opt.frame.w - bounds.w * k) / 2.0 - bounds.x * k;
    let oy = opt.frame.y + (opt.frame.h - bounds.h * k) / 2.0 - bounds.y * k;
    let nodes = nodes
        .into_iter()
        .map(|p| {
            let p = p.expect("every node is in a panel");
            Placed { rect: p.rect.map(ox, oy, k), reserve: p.reserve.map(ox, oy, k), port_y: oy + p.port_y * k, origin: (ox, oy), ..p }
        })
        .collect();
    let mut nodes: Vec<Placed> = nodes;
    let regions: Vec<RegionBox> = regions.into_iter().map(|b| RegionBox { rect: b.rect.map(ox, oy, k), title_rect: b.title_rect.map(|t| t.map(ox, oy, k)), ..b }).collect();
    for (n, pl) in g.nodes.iter().zip(&mut nodes) {
        pl.ends = opt.ends.iter().filter(|(key, _)| key.split(['>', '#']).next() == Some(n.id.as_str())).map(|(key, v)| (key.clone(), v.clone())).collect();
    }
    for p in &mut panels {
        p.rect = p.rect.map(ox, oy, k);
        p.title_rect = p.title_rect.map(|t| t.map(ox, oy, k));
    }
    for l in &mut legend {
        l.line = ((ox + l.line.0.0 * k, oy + l.line.0.1 * k), (ox + l.line.1.0 * k, oy + l.line.1.1 * k));
        l.text = l.text.map(ox, oy, k);
    }
    for s in &mut swatches {
        s.at = s.at.map(ox, oy, k);
        s.text = s.text.map(ox, oy, k);
    }
    Layout { nodes, panels, regions, legend, swatches, bounds: bounds.map(ox, oy, k), scale: k }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAIN: &str = r#"
fig demo
panel a "a. Training"
  x:    image "$\boldsymbol x$"
  aug:  box "Aug($x$)"
  net:  unet "$\theta$" depth=3
  pred: image "$\hat y$" look=mask
  loss: loss "$\mathcal L_{seg}$"
  y:    image "$y$" role=gt look=mask
panel b "b. Weights" dir=right
  q:    layer-bars "$\theta_q$" layers=[Conv,Conv,FC] taper
  v:    vector-bar "$\hat q$" n=5
  sys:  box "Fuzzy \\ system"
  w:    label "$\omega$"
end
x -> aug -> net -> pred -> loss
y -> loss : supervision
loss -> net : feedback "update"
q -> v -> sys -> w
w -> loss : cross
"#;
    const CHAIN: &str = "fig chain\nn0: box \"A\"\nn1: box \"B\"\nn2: box \"C\"\nn3: box \"D\"\nn0 -> n1 -> n2 -> n3\nn1 -> n3 : skip\n";

    fn lay_of(g: &Graph, opt: &LayoutOptions) -> Layout {
        layout(g, &Theme::classic_bw(), &Vars::default(), &Library::default(), opt)
    }

    fn room(l: &Layout, g: &Graph, id: &str) -> Rect {
        let n = &l.nodes[g.node_index(id).unwrap()];
        n.rect.union(&n.reserve)
    }

    fn region(id: &str, title: &str, parent: &str, members: &[&str]) -> Region {
        Region { id: id.into(), title: title.into(), parent: parent.into(), members: members.iter().map(|m| m.to_string()).collect(), ..Default::default() }
    }

    /// Two chains in one panel.
    fn two_chains() -> Graph {
        crate::dsl::parse("fig t\npanel p \"P\"\n a: box \"A\"\n b: box \"B\"\n c: box \"C\"\n d: box \"D\"\n e: box \"E\"\n f: box \"F\"\nend\na -> b -> c\nd -> e -> f\n").unwrap()
    }

    fn area(a: &Rect, b: &Rect) -> f64 {
        (a.right().min(b.right()) - a.x.max(b.x)).max(0.0) * (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0)
    }

    fn rbox<'a>(l: &'a Layout, id: &str) -> &'a RegionBox {
        l.regions.iter().find(|r| r.id == id).unwrap()
    }

    fn no_outsiders(g: &Graph, l: &Layout, rid: &str, members: &[&str]) {
        let f = rbox(l, rid).rect;
        for n in &g.nodes {
            if !members.contains(&n.id.as_str()) {
                assert!(area(&f, &room(l, g, &n.id)) < 1e-6, "{} lies inside {rid}", n.id);
            }
        }
    }

    #[test]
    fn a_region_frames_its_members_with_padding() {
        let mut g = two_chains();
        g.regions.push(region("r", "", "", &["a", "b"]));
        let l = lay_of(&g, &LayoutOptions::default());
        let f = rbox(&l, "r");
        let u = room(&l, &g, "a").union(&room(&l, &g, "b"));
        let p = REG_PAD * l.scale;
        assert!(f.rect.contains(&u));
        assert!(u.x - f.rect.x >= p - 1e-6 && f.rect.right() - u.right() >= p - 1e-6);
        assert!(u.y - f.rect.y >= p - 1e-6 && f.rect.bottom() - u.bottom() >= p - 1e-6);
        assert_eq!((f.line.as_str(), f.title_at.as_str(), f.depth), ("solid", "top-left", 0));
        assert!(f.title_rect.is_none());
    }

    #[test]
    fn a_region_with_a_title_has_room_for_it() {
        for at in ["top-left", "top", "above", "below", "left"] {
            let mut g = two_chains();
            let mut r = region("r", "Encoder", "", &["a", "b"]);
            r.title_at = at.into();
            g.regions.push(r);
            let l = lay_of(&g, &LayoutOptions::default());
            let f = rbox(&l, "r");
            let t = f.title_rect.unwrap_or_else(|| panic!("{at}: no title"));
            for n in &g.nodes {
                assert!(area(&t, &room(&l, &g, &n.id)) < 1e-6, "{at}: title over {}", n.id);
            }
            match at {
                "above" => assert!(t.bottom() <= f.rect.y + 1e-6),
                "below" => assert!(t.y >= f.rect.bottom() - 1e-6),
                _ => assert!(f.rect.contains(&t), "{at}: title outside the frame"),
            }
            assert!(l.panels[0].rect.contains(&t), "{at}: title outside the panel");
            assert!(f.rect.contains(&room(&l, &g, "a").union(&room(&l, &g, "b"))));
            no_outsiders(&g, &l, "r", &["a", "b"]);
        }
        // a title inside the top makes the frame taller
        let mut g = two_chains();
        g.regions.push(region("r", "T", "", &["a", "b"]));
        let with = lay_of(&g, &LayoutOptions::default());
        g.regions[0].title.clear();
        let without = lay_of(&g, &LayoutOptions::default());
        assert!(with.regions[0].rect.h / with.scale > without.regions[0].rect.h / without.scale + 5.0);
    }

    #[test]
    fn no_outsider_lies_inside_a_region_frame() {
        // a and c share a lane with b between them: the members are not contiguous in the flow
        let mut g = two_chains();
        g.regions.push(region("r", "", "", &["a", "c"]));
        let l = lay_of(&g, &LayoutOptions::default());
        no_outsiders(&g, &l, "r", &["a", "c"]);
        // members of two chains
        let mut g = two_chains();
        g.regions.push(region("r", "", "", &["b", "d"]));
        let l = lay_of(&g, &LayoutOptions::default());
        no_outsiders(&g, &l, "r", &["b", "d"]);
    }

    #[test]
    fn sibling_regions_do_not_overlap() {
        for (x, y) in [(["a", "b"], ["d", "e"]), (["a", "e"], ["b", "d"]), (["a", "c"], ["d", "f"])] {
            let mut g = two_chains();
            g.regions.push(region("r1", "One", "", &x));
            g.regions.push(region("r2", "Two", "", &y));
            let l = lay_of(&g, &LayoutOptions::default());
            let (a, b) = (rbox(&l, "r1").rect, rbox(&l, "r2").rect);
            assert!(!a.overlaps(&b), "{x:?} / {y:?}");
            let gap = (a.x - b.right()).max(b.x - a.right()).max(a.y - b.bottom()).max(b.y - a.bottom());
            assert!(gap >= REG_GAP * l.scale - 1e-6, "gap {gap}");
            no_outsiders(&g, &l, "r1", &x);
            no_outsiders(&g, &l, "r2", &y);
        }
    }

    #[test]
    fn regions_nest_and_outer_comes_first() {
        let mut g = two_chains();
        // declared inside first on purpose
        g.regions.push(region("inner", "In", "outer", &["a", "b"]));
        g.regions.push(region("outer", "Out", "", &["c"]));
        g.regions.push(region("deep", "", "inner", &["a"]));
        let l = lay_of(&g, &LayoutOptions::default());
        let ids: Vec<&str> = l.regions.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["outer", "inner", "deep"]);
        assert_eq!(l.regions.iter().map(|r| r.depth).collect::<Vec<_>>(), [0, 1, 2]);
        for w in l.regions.windows(2) {
            assert!(w[0].rect.contains(&w[1].rect));
        }
        // the outer one holds c, which it lists itself, and a, b through the inner one
        for id in ["a", "b", "c"] {
            assert!(rbox(&l, "outer").rect.contains(&room(&l, &g, id)));
        }
        no_outsiders(&g, &l, "outer", &["a", "b", "c"]);
    }

    #[test]
    fn a_panel_contains_its_regions() {
        let mut g = crate::dsl::parse("fig t\npanel p \"First\"\n a: box \"A\"\n b: box \"B\"\n c: box \"C\"\n d: box \"D\"\nend\npanel q \"Other\"\n e: box \"E\"\n f: box \"F\"\nend\na -> b\nc -> d\ne -> f\n").unwrap();
        for (i, at) in ["above", "top", "left"].iter().enumerate() {
            let mut r = region(&format!("r{i}"), "Sub network", "", [["a", "b"], ["c", "d"], ["e", "f"]][i].as_ref());
            r.title_at = at.to_string();
            g.regions.push(r);
        }
        let l = lay_of(&g, &LayoutOptions::default());
        for (r, panel) in l.regions.iter().zip([0, 0, 1]) {
            let pr = l.panels[panel].rect;
            assert!(pr.contains(&r.rect), "{} outside its panel", r.id);
            if let Some(t) = r.title_rect {
                assert!(pr.contains(&t), "{} title outside its panel", r.id);
            }
        }
        assert!(!l.regions[0].rect.overlaps(&l.regions[1].rect));
        // the panel's own title is clear of the frames
        let t = l.panels[0].title_rect.unwrap();
        assert!(l.regions.iter().all(|r| !r.rect.overlaps(&t)));
    }

    #[test]
    fn a_region_without_panels_works() {
        let mut g = crate::dsl::parse("fig t\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c\n").unwrap();
        assert!(g.panels.is_empty());
        g.regions.push(region("r", "Group", "", &["a", "b"]));
        g.regions.push(region("s", "", "", &["d"]));
        let l = lay_of(&g, &LayoutOptions::default());
        assert_eq!(l.regions.len(), 2);
        assert!(l.panels[0].rect.contains(&l.regions[0].rect));
        assert!(l.panels[0].rect.contains(&l.regions[1].rect));
        assert!(!l.regions[0].rect.overlaps(&l.regions[1].rect));
        no_outsiders(&g, &l, "r", &["a", "b"]);
        no_outsiders(&g, &l, "s", &["d"]);
    }

    #[test]
    fn a_moved_member_moves_its_region_frame() {
        let mut g = two_chains();
        g.regions.push(region("r", "Group", "", &["a", "b"]));
        let l0 = lay_of(&g, &LayoutOptions::default());
        let mut opt = LayoutOptions::default();
        opt.place.insert("a".into(), [0.0, 20.0]);
        let l1 = lay_of(&g, &opt);
        let (f0, f1) = (rbox(&l0, "r").rect, rbox(&l1, "r").rect);
        assert!(f1.contains(&room(&l1, &g, "a")) && f1.contains(&room(&l1, &g, "b")));
        assert!(f1.bottom() > f0.bottom() + 1.0, "the frame reaches down to the moved block");
        assert!(l1.panels[0].rect.contains(&f1));
        assert!(l1.bounds.contains(&f1));
    }

    #[test]
    fn a_graph_without_regions_lays_out_as_before() {
        let train: [[f64; 4]; 10] = [
            [71.26279832560063, 66.41918120414952, 57.85078286973353, 99.26668424238368],
            [163.29813470926763, 98.96024656837464, 64.37871780037506, 34.18455351393345],
            [261.86140602357614, 64.96887259592947, 136.7382140557338, 102.16730145882379],
            [432.7841735932434, 66.41918120414952, 57.85078286973353, 99.26668424238368],
            [524.8195099769105, 104.38375746241216, 31.88235531382201, 23.337531725858412],
            [590.8864188046659, 66.41918120414952, 57.85078286973353, 99.26668424238368],
            [221.36322843372315, 255.43750536228288, 74.71297128574108, 109.78500840051704],
            [331.09893218974895, 256.75229588204957, 13.147905197666711, 107.1554273609837],
            [379.26956985770033, 284.36289679714963, 68.78983999419222, 51.93422553078351],
            [482.24396336582606, 299.97603421937885, 16.392808200450855, 20.70795068632507],
        ];
        let chain: [[f64; 4]; 4] = [[181.5, 183.0, 60.0, 39.0], [280.5, 183.0, 60.0, 39.0], [379.5, 183.0, 60.0, 39.0], [478.5, 183.0, 60.0, 39.0]];
        let check = |src: &str, want: &[[f64; 4]], bounds: Rect| {
            let l = lay_of(&crate::dsl::parse(src).unwrap(), &LayoutOptions::default());
            assert!(l.regions.is_empty());
            for (n, w) in l.nodes.iter().zip(want) {
                let got = [n.rect.x, n.rect.y, n.rect.w, n.rect.h];
                assert!(got.iter().zip(w).all(|(a, b)| (a - b).abs() < 1e-6), "{got:?} vs {w:?}");
            }
            assert!((l.bounds.x - bounds.x).abs() < 1e-6 && (l.bounds.w - bounds.w).abs() < 1e-6 && (l.bounds.h - bounds.h).abs() < 1e-6);
        };
        check(TRAIN, &train, Rect::new(55.485312088400576, 24.0, 609.0293758231988, 357.0));
        check(CHAIN, &chain, Rect::new(178.5, 180.0, 363.0, 45.0));
    }

    fn side_by_side() -> Graph {
        let mut g = crate::dsl::parse("fig t\npanel a \"A\" dir=right\n a1: box \"A\"\nend\npanel b \"B\" dir=right\n b1: box \"B\"\n b2: box \"Tall\"\nend\n").unwrap();
        g.arrange = vec![vec!["a".into(), "b".into()]];
        g
    }

    fn stacked() -> Graph {
        let mut g = crate::dsl::parse("fig t\npanel a \"A\" dir=right\n x1: box \"A\"\n x2: box \"B\"\n x3: box \"C\"\nend\npanel b \"B\" dir=right\n y1: box \"D\"\nend\nx1 -> x2 -> x3\n").unwrap();
        g.arrange = vec![vec!["a".into()], vec!["b".into()]];
        g
    }

    fn clear(g: &Graph, l: &Layout) {
        for (i, a) in g.nodes.iter().enumerate() {
            for b in &g.nodes[i + 1..] {
                assert!(area(&room(l, g, &a.id), &room(l, g, &b.id)) < 1e-6, "{} / {}", a.id, b.id);
            }
        }
        for (i, a) in l.panels.iter().enumerate() {
            for b in &l.panels[i + 1..] {
                assert!(!a.rect.overlaps(&b.rect), "panels {} / {}", a.id, b.id);
            }
        }
        for (n, p) in g.nodes.iter().zip(&l.nodes) {
            assert!(l.panels[p.panel].rect.contains(&p.reserve), "{} left its panel", n.id);
        }
    }

    #[test]
    fn align_row_puts_port_lines_level() {
        let mut g = side_by_side();
        let base = lay_of(&g, &LayoutOptions::default());
        assert!((base.nodes[0].port_y - base.nodes[2].port_y).abs() > 1.0, "the example needs rows that differ");
        g.align.push(Align { axis: "row".into(), ids: vec!["a1".into(), "b2".into()] });
        let l = lay_of(&g, &LayoutOptions::default());
        assert!((l.nodes[0].port_y - l.nodes[2].port_y).abs() < 0.5, "{} vs {}", l.nodes[0].port_y, l.nodes[2].port_y);
        assert!(unmet_alignments(&g, &l).is_empty());
        clear(&g, &l);
    }

    #[test]
    fn align_col_shares_a_centre() {
        let mut g = stacked();
        let base = lay_of(&g, &LayoutOptions::default());
        assert!((base.nodes[2].rect.cx() - base.nodes[3].rect.cx()).abs() > 1.0, "the example needs columns that differ");
        g.align.push(Align { axis: "col".into(), ids: vec!["x3".into(), "y1".into()] });
        let l = lay_of(&g, &LayoutOptions::default());
        assert!((l.nodes[2].rect.cx() - l.nodes[3].rect.cx()).abs() < 0.5);
        assert!(unmet_alignments(&g, &l).is_empty());
        clear(&g, &l);
    }

    #[test]
    fn alignment_never_overlaps_blocks() {
        let mut g = side_by_side();
        g.align.push(Align { axis: "row".into(), ids: vec!["a1".into(), "b1".into()] });
        let l = lay_of(&g, &LayoutOptions::default());
        assert!(unmet_alignments(&g, &l).is_empty());
        clear(&g, &l);
        let mut g = stacked();
        g.align.push(Align { axis: "col".into(), ids: vec!["x2".into(), "y1".into()] });
        g.align.push(Align { axis: "row".into(), ids: vec!["x1".into(), "y1".into()] });
        let l = lay_of(&g, &LayoutOptions::default());
        clear(&g, &l);
        // with regions in the way
        let mut g = side_by_side();
        g.regions.push(region("r", "Bs", "", &["b1", "b2"]));
        g.align.push(Align { axis: "row".into(), ids: vec!["a1".into(), "b2".into()] });
        let l = lay_of(&g, &LayoutOptions::default());
        clear(&g, &l);
        assert!(l.panels[1].rect.contains(&l.regions[0].rect));
    }

    #[test]
    fn an_impossible_alignment_is_reported_unmet() {
        // two blocks of one panel, one row above the other: no spacing can level them
        let mut g = crate::dsl::parse("fig t\npanel p \"P\" dir=right\n a: box \"A\"\n b: box \"B\"\nend\n").unwrap();
        let plain = lay_of(&g, &LayoutOptions::default());
        assert!((plain.nodes[0].port_y - plain.nodes[1].port_y).abs() > 1.0);
        g.align.push(Align { axis: "row".into(), ids: vec!["a".into(), "b".into()] });
        let l = lay_of(&g, &LayoutOptions::default());
        assert_eq!(unmet_alignments(&g, &l), vec!["align row: a b".to_string()]);
        // nothing was moved for it
        for (x, y) in plain.nodes.iter().zip(&l.nodes) {
            assert!((x.rect.x - y.rect.x).abs() < 1e-6 && (x.rect.y - y.rect.y).abs() < 1e-6);
        }
        clear(&g, &l);
    }

    #[test]
    fn everything_fits_the_frame_with_regions() {
        let mut g = two_chains();
        let mut r1 = region("r1", "Encoder stage", "all", &["a", "b"]);
        r1.title_at = "above".into();
        let mut r2 = region("r2", "Second", "all", &["d", "e"]);
        r2.title_at = "below".into();
        g.regions.push(region("all", "Everything", "", &["a", "b", "c", "d", "e", "f"]));
        g.regions.push(r1);
        g.regions.push(r2);
        let opt = LayoutOptions::default();
        let l = lay_of(&g, &opt);
        assert!(opt.frame.inflate(0.01).contains(&l.bounds));
        for r in &l.regions {
            assert!(l.bounds.contains(&r.rect), "{}", r.id);
            if let Some(t) = r.title_rect {
                assert!(l.bounds.contains(&t), "{} title", r.id);
            }
        }
        assert!(l.scale > 0.0 && l.scale <= opt.max_scale);
        clear(&g, &l);
    }
}
