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
}

impl Default for LayoutOptions {
    fn default() -> Self {
        // a 16:9 slide of 10 in with a margin
        LayoutOptions { frame: Rect::new(24.0, 24.0, 672.0, 357.0), max_scale: 1.5, template: None, legend: None }
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

fn plan_panel(g: &Graph, nodes: &[usize], sizes: &[Size], down: bool, theme: &Theme) -> PanelPlan {
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
    let lane: BTreeMap<usize, usize> = lane.into_iter().map(|(n, l)| (n, lanes.iter().position(|x| *x == l).unwrap())).collect();

    // cells: along = the flow axis, across = the lane axis
    let (n_along, n_across) = (layers.len(), lanes.len());
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
    let mut along_pos = vec![0.0; n_along];
    for i in 1..n_along {
        along_pos[i] = along_pos[i - 1] + along[i - 1] + gaps[i - 1];
    }
    let mut across_pos = vec![0.0; n_across];
    for i in 1..n_across {
        across_pos[i] = across_pos[i - 1] + across[i - 1] + gap_across;
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
        pos.insert(n, r);
    }
    let total_along = along_pos.last().copied().unwrap_or(0.0) + along.last().copied().unwrap_or(0.0);
    let total_across = across_pos.last().copied().unwrap_or(0.0) + across.last().copied().unwrap_or(0.0);
    let content = if down { (total_across, total_along) } else { (total_along, total_across) };
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
    let pad = theme.space.pad;
    let title_size = theme.title_size();
    let outside = theme.panel.title.starts_with("outside");
    let needs: Vec<PanelNeed> = defs
        .iter()
        .map(|p| {
            let nodes: Vec<usize> = (0..g.nodes.len()).filter(|i| if p.id == "_" { g.nodes[*i].panel.is_empty() } else { g.nodes[*i].panel == p.id }).collect();
            let plan = plan_panel(g, &nodes, &sizes, p.dir == "down", theme);
            let bare = p.bare || (p.id == "_");
            let has_title = !p.title.is_empty() && !bare;
            let title_h = if has_title { title_size * 1.3 + 2.0 } else { 0.0 };
            let title_w = if has_title { text::width(&p.title, title_size) * 1.06 + 2.0 * pad } else { 0.0 };
            // lines that go around the row need a channel under it
            let around = g.edges.iter().any(|e| matches!(e.kind, EdgeKind::Feedback | EdgeKind::Skip) && nodes.iter().any(|n| g.nodes[*n].id == e.from || g.nodes[*n].id == e.to));
            let (p_in, top, bottom) = if bare { (2.0, 2.0, 2.0) } else { (pad, pad + if outside { 0.0 } else { title_h }, pad + if around { 10.0 } else { 0.0 }) };
            let w = (plan.content.0 + 2.0 * p_in).max(title_w);
            let h = plan.content.1 + top + bottom + if outside { title_h } else { 0.0 };
            PanelNeed { id: p.id.clone(), title: p.title.clone(), bare, plan, w, h, top, bottom, outside_title: if outside { title_h } else { 0.0 } }
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
        let frame = Rect::new(r.x, r.y + need.outside_title, r.w, r.h - need.outside_title);
        let inner_h = frame.h - need.top - need.bottom;
        let ox = frame.x + (frame.w - need.plan.content.0) / 2.0;
        let oy = frame.y + need.top + (inner_h - need.plan.content.1) / 2.0;
        for &ni in &need.plan.nodes {
            let p = need.plan.pos[&ni];
            let rect = Rect::new(ox + p.x, oy + p.y, p.w, p.h);
            let s = &sizes[ni];
            let reserve = Rect::new(rect.cx() - s.reserve_w / 2.0, rect.y, s.reserve_w, rect.h);
            nodes[ni] = Some(Placed { rect, reserve, port_y: rect.y + s.cy, panel: i, layer: need.plan.layer[&ni], lane: need.plan.lane[&ni], single: s.single, ellipse: s.ellipse });
        }
        let title_rect = (!need.title.is_empty() && !need.bare).then(|| {
            let tw = text::width(&need.title, title_size) * 1.06;
            let th = title_size * 1.3;
            let centre = theme.panel.title.ends_with("center");
            let x = if centre { frame.cx() - tw / 2.0 } else if outside { frame.x } else { frame.x + pad * 0.6 };
            let y = if outside { r.y } else { frame.y + pad * 0.4 };
            Rect::new(x, y, tw, th)
        });
        panels.push(PanelBox { id: need.id.clone(), title: need.title.clone(), rect: frame, title_rect, bare: need.bare, down: need.plan.down });
    }
    let mut bounds = rects.iter().skip(1).fold(rects[0], |a, b| a.union(b));

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
            Placed { rect: p.rect.map(ox, oy, k), reserve: p.reserve.map(ox, oy, k), port_y: oy + p.port_y * k, ..p }
        })
        .collect();
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
    Layout { nodes, panels, legend, swatches, bounds: bounds.map(ox, oy, k), scale: k }
}
