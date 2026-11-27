//! Filling a page: a frame of a fixed shape, areas in a stated arrangement, and in every area
//! its blocks in their order, drawn as large as they go. No lines yet: this is the base draft
//! the lines are later drawn on.
//!
//! The problem, one level at a time:
//! - a block has a size that follows from one number, its area's enlargement `g` (its picture is
//!   `g` times its natural size, its words keep theirs);
//! - a frame (an area, or a region inside one) holds its blocks in order, in rows it may break
//!   anywhere; every way of breaking gives a shape (width, height), and the shapes no other
//!   beats are kept;
//! - the page holds the areas in rows: areas of one row share its height and split the width.
//!
//! Solved in three steps, each a search on one number that only makes things larger:
//! 1. the largest text: the smallest page (in natural points) that holds everything unenlarged;
//! 2. every area's pictures enlarged together as far as the page allows;
//! 3. the areas that still have room enlarged further, together, until none can grow.
//!
//! The air between blocks is a stated number of text sizes ([`Air`]), not what is left over.
use crate::layout::{self, Layout, PanelBox, Placed, RTree, Rect};
use crate::modules::{self, Body, Ctx};
use crate::plate::{Format, Measure};
use crate::spec::{Graph, Node};
use crate::text::{self, NoMath};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The air round the blocks, in text sizes (the theme's font size).
#[derive(Debug, Clone, PartialEq)]
pub struct Air {
    /// Between neighbouring blocks of a row, and between rows.
    pub gap: f64,
    /// Between a frame and the blocks inside it.
    pub pad: f64,
    /// Between two areas.
    pub between: f64,
    /// The most a picture is enlarged.
    pub grow_max: f64,
    /// The most the pictures of one area are enlarged over those of the area enlarged least
    /// (the pictures of one figure are of one scale, nearly): an area with room to spare keeps
    /// it as air.
    pub grow_apart: f64,
    /// The largest the text is drawn on the page, in points (0 = as large as it goes). With a
    /// limit the page that is left goes to the pictures.
    pub text_max_pt: f64,
    /// How much of the text size paths are worth. An area whose text states a grid (`col=`,
    /// `lane=`: its rows are the paths of the flow) stands on it when that makes its lines
    /// simpler than rows broken freely do, as long as the page's text keeps this share of what
    /// rows broken freely give. 0 = never on the grid, 1 = only when it costs nothing.
    pub paths: f64,
    /// The least room between two blocks a line runs between, in text sizes: a shaft that is
    /// seen and its arrowhead. Words on the line widen it further (their width across, their
    /// height down). Only where a line runs; on a stated grid.
    pub line: f64,
    /// The least size the text should have on the page, in points. Under it the room for
    /// lines is given back first; a page that still cannot reach it is marked `Packed.small`
    /// (it holds more blocks or words than its width carries at that size).
    pub text_min_pt: f64,
    /// The user's own pick, over the choice made here: every area whose objects state their
    /// column and row stands on that grid, whatever it costs the text.
    pub stated: bool,
}

impl Default for Air {
    fn default() -> Air {
        Air { gap: 1.5, pad: 0.8, between: 0.6, grow_max: 4.0, grow_apart: 1.5, text_max_pt: 0.0, paths: 0.6, line: 2.5, text_min_pt: 5.0, stated: false }
    }
}

/// One area as packed.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaPack {
    pub id: String,
    /// The enlargement of its pictures.
    pub grow: f64,
    /// Its blocks row by row (a region is written `[id]`).
    pub rows: Vec<Vec<String>>,
    /// The area stands on the grid its text states (its rows are the paths of the flow), not in
    /// rows broken freely.
    pub grid: bool,
    /// The share of the area's frame its blocks cover.
    pub cover: f64,
}

/// A page packed: the layout (natural points, the page at the origin), the graph to draw it with
/// (every enlarged block carries its `grow=`), and the measures.
#[derive(Debug, Clone)]
pub struct Packed {
    pub layout: Layout,
    pub graph: Graph,
    /// The theme's font size on the page, in points.
    pub text_pt: f64,
    /// The share of the page the blocks cover.
    pub cover: f64,
    pub areas: Vec<AreaPack>,
    /// What is left of the page in the direction that does not bind, as a share of it:
    /// (across, down). Both near 0 = the page is full both ways.
    pub slack: (f64, f64),
    /// The blocks that are notes, not objects (see [`is_note`]): they took no room in the
    /// filling and were written in the air beside the block they belong to.
    pub notes: Vec<String>,
    /// The notes among them that found no free room and lie over something.
    pub crowded: Vec<String>,
    /// The text is under `Air.text_min_pt`.
    pub small: bool,
    /// Objects whose surroundings are not what they need (a brace whose open side has nothing
    /// long enough, or whose tip points at nothing), in words.
    pub unmet: Vec<String>,
}

/// How thin a brace that gathers is drawn, of a free-standing one.
const THIN: f64 = 0.6;

/// A note, not an object: a block that is words only (`label`) and sits on lines none of which
/// carries the flow (no `data` line). It says something about a line or a block (a weight fed in
/// from the side, the name of a comparison), so it takes no room in the filling and does not
/// move the objects; it is written in the air next to the block it is tied to. Words without any
/// line (a legend's rows, a heading) stay objects: nothing says where else they belong.
pub fn is_note(g: &Graph, n: usize) -> bool {
    let node = &g.nodes[n];
    // a brace that gathers named things is a relation between them: it lies along them
    if gathers(g, n).is_some() {
        return true;
    }
    // stated by the one who draws: `by=<block>` ties any mark to that block
    if node.str("by").is_some_and(|b| b != node.id && g.node_index(b).is_some()) {
        return true;
    }
    if node.module != "label" {
        return false;
    }
    let mut any = false;
    for e in g.edges.iter().filter(|e| e.from == node.id || e.to == node.id) {
        any = true;
        if e.kind == crate::spec::EdgeKind::Data {
            return false;
        }
    }
    any
}

/// The things a mark gathers (`gathers=[first,last]`, or more names): a brace or a dimension
/// mark that lies along them, as long as they are, on the side of `toward=<block>` (else its
/// own `dir`). It takes no cell. `None` when it names nothing of its own area.
pub fn gathers(g: &Graph, n: usize) -> Option<Vec<usize>> {
    let node = &g.nodes[n];
    let names: Vec<String> = match node.params.get("gathers")? {
        serde_json::Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        serde_json::Value::String(t) => t.split(',').map(|x| x.trim().to_string()).collect(),
        _ => return None,
    };
    let v: Vec<usize> = names.iter().filter_map(|id| g.node_index(id)).filter(|m| *m != n && g.nodes[*m].panel == node.panel && !g.nodes[*m].params.contains_key("gathers")).collect();
    (!v.is_empty()).then_some(v)
}

#[derive(Debug, Clone)]
enum Item {
    Leaf(usize),
    Group(usize, Vec<Item>),
}

struct Area {
    id: String,
    title: String,
    bare: bool,
    down: bool,
    items: Vec<Item>,
    notes: Vec<usize>,
    /// The area's blocks on the grid the figure's text states, when it states one.
    grid: Option<Grid>,
}

/// The grid the text states for an area: every object names its column and its row (`col=`,
/// `lane=`: where it stands along the flow and which of the parallel paths it is on), no two the
/// same cell. A row is then a path, kept whole, and the blocks of different paths stand under
/// each other, so the lines between them are straight. A region frames the cells of its blocks.
#[derive(Debug, Clone)]
struct Grid {
    cols: usize,
    rows: usize,
    /// (block, column, row)
    cell: Vec<(usize, usize, usize)>,
    /// Per cell of `cell`: the last column the block reaches (`span=` columns from its own).
    last: Vec<usize>,
    /// Per cell: the last row the block reaches (`tall=` rows from its own).
    low: Vec<usize>,
    regs: Vec<GReg>,
}

#[derive(Debug, Clone)]
struct GReg {
    r: usize,
    c0: usize,
    c1: usize,
    r0: usize,
    r1: usize,
    /// The regions of the grid inside this one, itself included (indexes into `Grid.regs`).
    inside: Vec<usize>,
    /// Its blocks (indexes into `Grid.cell`).
    cells: Vec<usize>,
}

/// A grid measured at one enlargement: column widths, each row's parts, the room region frames
/// take before and after each column and row, each region's own insets (left, right, top,
/// bottom), and the size of it all without the area's title and padding.
struct GridGeom {
    cw: Vec<f64>,
    parts: Vec<(f64, f64, f64, f64)>,
    left: Vec<f64>,
    right: Vec<f64>,
    top: Vec<f64>,
    bottom: Vec<f64>,
    inset: Vec<(f64, f64, f64, f64)>,
    /// The air after each column but the last and under each row but the last: the gap, wider
    /// where a line runs and wider again for its words.
    gx: Vec<f64>,
    gy: Vec<f64>,
    w: f64,
    h: f64,
}

/// `stretch=true|wide|both`: the block is as wide as the columns it lies over.
fn wide(n: &crate::spec::Node) -> bool {
    n.flag("stretch") || matches!(n.str("stretch"), Some("wide" | "both"))
}

/// `label_at=above`: the words a block writes under its picture are written above it.
fn above(n: &crate::spec::Node) -> bool {
    n.str("label_at") == Some("above")
}

/// `stretch=tall|both`: the block is as tall as the rows it stands beside.
fn high(n: &crate::spec::Node) -> bool {
    matches!(n.str("stretch"), Some("tall" | "both"))
}

fn grid_of(g: &Graph, rt: &RTree, objects: &[usize], down: bool) -> Option<Grid> {
    if objects.is_empty() {
        return None;
    }
    let mut at: Vec<(usize, f64, f64)> = Vec::new();
    for &n in objects {
        let (c, l) = (g.nodes[n].num("col")?, g.nodes[n].num("lane")?);
        at.push(if down { (n, l, c) } else { (n, c, l) });
    }
    let index = |vals: Vec<f64>| -> Vec<f64> {
        let mut v = vals;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        v
    };
    let (xs, ys) = (index(at.iter().map(|a| a.1).collect()), index(at.iter().map(|a| a.2).collect()));
    let pos = |v: &[f64], x: f64| v.iter().position(|y| (*y - x).abs() < 1e-6).unwrap();
    let cell: Vec<(usize, usize, usize)> = at.iter().map(|(n, c, l)| (*n, pos(&xs, *c), pos(&ys, *l))).collect();
    // a block with `span=k` reaches over the stated columns up to k - 1 past its own
    let last: Vec<usize> = at.iter().zip(&cell).map(|((n, c, _), k)| {
        let to = *c + g.nodes[*n].num("span").unwrap_or(1.0).max(1.0) - 1.0;
        xs.iter().rposition(|x| *x <= to + 1e-6).unwrap_or(k.1).max(k.1)
    }).collect();
    let low: Vec<usize> = at.iter().zip(&cell).map(|((n, _, l), k)| {
        let to = *l + g.nodes[*n].num("tall").unwrap_or(1.0).max(1.0) - 1.0;
        ys.iter().rposition(|y| *y <= to + 1e-6).unwrap_or(k.2).max(k.2)
    }).collect();
    for (i, a) in cell.iter().enumerate() {
        if (0..i).any(|j| cell[j].2 <= low[i] && a.2 <= low[j] && cell[j].1 <= last[i] && a.1 <= last[j]) {
            return None;
        }
    }
    let mut regs: Vec<GReg> = Vec::new();
    for r in 0..g.regions.len() {
        let mine: Vec<&(usize, usize, usize)> = cell.iter().filter(|c| rt.all[r].contains(&c.0)).collect();
        if mine.is_empty() {
            continue;
        }
        regs.push(GReg { r, c0: mine.iter().map(|c| c.1).min().unwrap(), c1: cell.iter().zip(&last).filter(|(c, _)| rt.all[r].contains(&c.0)).map(|(_, l)| *l).max().unwrap(), r0: mine.iter().map(|c| c.2).min().unwrap(), r1: cell.iter().zip(&low).filter(|(c, _)| rt.all[r].contains(&c.0)).map(|(_, l)| *l).max().unwrap(), inside: Vec::new(), cells: (0..cell.len()).filter(|k| rt.all[r].contains(&cell[*k].0)).collect() });
    }
    for i in 0..regs.len() {
        regs[i].inside = (0..regs.len()).filter(|j| *j == i || (rt.all[regs[*j].r].is_subset(&rt.all[regs[i].r]) && rt.all[regs[*j].r] != rt.all[regs[i].r])).collect();
    }
    Some(Grid { cols: xs.len(), rows: ys.len(), cell, last, low, regs })
}

/// A block's tight box at one enlargement: its width, how far its middle line lies under its
/// top (`up`), how far its picture reaches under that line (`dn`), and the height of the words
/// under the picture (`words`). In a row the middle lines coincide and the words start on one
/// line under the row's tallest picture, so a row is max up + max dn + max words tall.
#[derive(Debug, Clone, Copy)]
struct Box3 {
    w: f64,
    up: f64,
    dn: f64,
    words: f64,
    /// The height of the words written above the picture instead (`label_at=above`), with
    /// their gap; 0 for every other block (its words above, if any, are part of `up`).
    over: f64,
    /// A frame (a region), not a block: it is as tall as its row and sets no line for words.
    frame: bool,
}

/// A row's height above its middle line, its height below it, how far under the line the
/// words of its blocks start (under the tallest picture among the blocks; a frame in the row
/// does not push them down), and how far over the line the words written above a picture
/// (`label_at=above`) end (over the tallest picture, likewise).
fn row_parts(b: &[Box3]) -> (f64, f64, f64, f64) {
    let leaf_up = b.iter().filter(|x| !x.frame).map(|x| x.up).fold(0.0, f64::max);
    let over = b.iter().filter(|x| !x.frame).map(|x| x.over).fold(0.0, f64::max);
    let up = b.iter().map(|x| x.up).fold(leaf_up + over, f64::max);
    let leaf_dn = b.iter().filter(|x| !x.frame).map(|x| x.dn).fold(0.0, f64::max);
    let words = b.iter().filter(|x| !x.frame).map(|x| x.words).fold(0.0, f64::max);
    let below = b.iter().filter(|x| x.frame).map(|x| x.dn).fold(leaf_dn + words, f64::max);
    (up, below, leaf_dn, leaf_up)
}

/// The columns the rows of a frame share, so the items of different rows lie under each other.
/// The longest rows set them. A shorter row starts as many columns in as centres it when that
/// lands on columns (the difference is even), else at the first column, and joins only when each
/// of its items is at most [`JOIN`] times as wide as its column (the column then holds it); a
/// row with an item too wide for that is laid out on its own, centred (`None`).
/// Returns each column's width, each row's first column, and the width they all need.
fn columns(b: &[Box3], ends: &[usize], gap: f64) -> (Vec<f64>, Vec<Option<usize>>, f64) {
    let mut rows = Vec::new();
    let mut a = 0;
    for &e in ends {
        rows.push((a, e));
        a = e;
    }
    let n = rows.iter().map(|(a, z)| z - a).max().unwrap_or(0);
    let mut w = vec![0.0f64; n];
    for (a, z) in rows.iter().filter(|(a, z)| z - a == n) {
        for i in *a..*z {
            w[i - a] = w[i - a].max(b[i].w);
        }
    }
    let mut first = Vec::new();
    let mut need: f64 = 0.0;
    for (a, z) in &rows {
        let l = z - a;
        let f = if (n - l) % 2 == 0 { (n - l) / 2 } else { 0 };
        if l == n || (*a..*z).all(|i| b[i].w <= w[f + i - a] * JOIN + 0.01) {
            for i in *a..*z {
                w[f + i - a] = w[f + i - a].max(b[i].w);
            }
            first.push(Some(f));
        } else {
            need = need.max(b[*a..*z].iter().map(|x| x.w).sum::<f64>() + gap * (l - 1) as f64);
            first.push(None);
        }
    }
    let need = need.max(w.iter().sum::<f64>() + gap * n.saturating_sub(1) as f64);
    (w, first, need)
}

/// The most row widths tried when a frame's rows are broken freely.
const WIDTHS: usize = 40;

/// How much wider than a column an item of a shorter row may be and still stand in it.
const JOIN: f64 = 1.3;

/// One way a frame holds its items: its size, where its rows end, and which shape each item
/// that is itself a frame takes.
#[derive(Debug, Clone)]
struct Shape {
    w: f64,
    h: f64,
    ends: Vec<usize>,
    kids: Vec<usize>,
}

struct Ctx2<'a> {
    g: &'a Graph,
    m: &'a Measure<'a>,
    air: &'a Air,
    fs: f64,
    cache: RefCell<BTreeMap<(usize, u32), Box3>>,
    /// Per area and per region: a band kept free above and below its rows, for notes that found
    /// no room in the air (the same above and below, so the objects stay centred).
    band_area: Vec<f64>,
    band_reg: Vec<f64>,
}

fn steps(g: f64) -> u32 {
    (g * 100.0).round() as u32
}

fn body(n: &Node, grow: u32, symmetric: bool, m: &Measure) -> Body {
    let mut n = n.clone();
    n.params.remove("grow");
    if grow > 100 {
        n.params.insert("grow".into(), serde_json::json!(grow as f64 / 100.0));
    }
    modules::make(&n, &Ctx { theme: m.theme, vars: m.vars, math: &NoMath, lib: m.library, k: 1.0, symmetric, image_size: m.image_size })
}

impl Ctx2<'_> {
    fn leaf(&self, n: usize, grow: u32) -> Box3 {
        let grow = if modules::growable(&self.g.nodes[n]) { grow } else { 100 };
        if let Some(b) = self.cache.borrow().get(&(n, grow)) {
            return *b;
        }
        let b = body(&self.g.nodes[n], grow, false, self.m);
        let w = b.w.max(b.reserve_w);
        // words written above the picture (`label_at=above`) stand apart from it, like the
        // words under a picture
        let over = if above(&self.g.nodes[n]) { b.above } else { 0.0 };
        let out = Box3 { w, up: b.axis - over, dn: b.h - b.below - b.axis, words: b.below, over, frame: false };
        self.cache.borrow_mut().insert((n, grow), out);
        out
    }

    fn gap(&self) -> f64 {
        self.air.gap * self.fs
    }

    fn pad(&self) -> f64 {
        self.air.pad * self.fs
    }

    /// The band a region's title takes at its top, and the least width the title needs (as
    /// `plate` measures it: the drawing's size, a long title on two lines).
    fn region_title(&self, r: usize) -> (f64, f64) {
        let (w, _, band) = crate::plate::reg_title(&self.g.regions[r], self.m.theme);
        (band, w)
    }

    /// The shapes a region is offered in: all of them, or of many the narrowest, the middle one
    /// and the widest.
    fn group_shapes(&self, r: usize, kids: &[Item], grow: u32) -> Vec<Shape> {
        let (band, tw) = self.region_title(r);
        let mut all = self.shapes(kids, grow, band, tw, true);
        for s in &mut all {
            s.h += 2.0 * self.band_reg[r];
        }
        if all.len() > 3 { vec![all[0].clone(), all[all.len() / 2].clone(), all[all.len() - 1].clone()] } else { all }
    }

    /// A grid measured (see [`GridGeom`]).
    fn grid_geom(&self, gr: &Grid, grow: u32) -> GridGeom {
        let (pad, gap) = (self.pad(), self.gap());
        let mut cw = vec![0.0f64; gr.cols];
        let mut by_row: Vec<Vec<Box3>> = vec![Vec::new(); gr.rows];
        for (k, (&(n, c, r), &l)) in gr.cell.iter().zip(&gr.last).enumerate() {
            let b = self.leaf(n, grow);
            // a block over several rows sets the height of none alone
            let b = if gr.low[k] > r { Box3 { up: 0.0, dn: 0.0, words: 0.0, over: 0.0, ..b } } else { b };
            // a block that takes the width of its columns (`stretch`) asks for none itself
            if l == c && !wide(&self.g.nodes[n]) {
                cw[c] = cw[c].max(b.w);
            }
            by_row[r].push(b);
        }
        let parts: Vec<(f64, f64, f64, f64)> = by_row.iter().map(|b| row_parts(b)).collect();
        // the air between columns and between rows: the gap; where a line runs, room for its
        // shaft and arrowhead; where the line carries words, room for them
        let (mut gx, mut gy) = (vec![gap; gr.cols.saturating_sub(1)], vec![gap; gr.rows.saturating_sub(1)]);
        let line = self.air.line * self.fs;
        let cell = |id: &str| self.g.node_index(id).and_then(|n| gr.cell.iter().find(|c| c.0 == n)).map(|c| (c.1, c.2));
        for e in &self.g.edges {
            let (Some((c1, r1)), Some((c2, r2))) = (cell(&e.from), cell(&e.to)) else { continue };
            let small = self.fs * 0.85;
            let (lw, lh) = if e.label.is_empty() { (0.0, 0.0) } else { (text::block(&e.label, small).0 + self.fs, text::block(&e.label, small).1 as f64 * small * 1.25) };
            let (ca, cb, ra, rb) = (c1.min(c2), c1.max(c2), r1.min(r2), r1.max(r2));
            for g in &mut gx[ca..cb] {
                *g = g.max(line);
            }
            for g in &mut gy[ra..rb] {
                *g = g.max(line);
            }
            // the words stand on the first run: across a row, or down a column
            if cb > ca && (rb == ra || cb - ca == 1) {
                gx[ca] = gx[ca].max(lw);
            } else if rb > ra {
                gy[ra] = gy[ra].max(line + lh);
            }
        }
        // each region's own insets: its padding and title band, with those of the regions inside
        // it that start (or end) at the same column or row
        let inset: Vec<(f64, f64, f64, f64)> = gr
            .regs
            .iter()
            .map(|a| {
                let ins: Vec<&GReg> = a.inside.iter().map(|j| &gr.regs[*j]).collect();
                let band = self.band_reg[a.r];
                (
                    pad * ins.iter().filter(|q| q.c0 == a.c0).count() as f64,
                    pad * ins.iter().filter(|q| q.c1 == a.c1).count() as f64,
                    ins.iter().filter(|q| q.r0 == a.r0).map(|q| self.region_title(q.r).0.max(pad)).sum::<f64>() + band,
                    pad * ins.iter().filter(|q| q.r1 == a.r1).count() as f64 + band,
                )
            })
            .collect();
        let (mut left, mut right, mut top, mut bottom) = (vec![0.0f64; gr.cols], vec![0.0f64; gr.cols], vec![0.0f64; gr.rows], vec![0.0f64; gr.rows]);
        for (a, i) in gr.regs.iter().zip(&inset) {
            left[a.c0] = left[a.c0].max(i.0);
            right[a.c1] = right[a.c1].max(i.1);
            top[a.r0] = top[a.r0].max(i.2);
            bottom[a.r1] = bottom[a.r1].max(i.3);
        }
        // a frame is at least as wide as its title: the innermost first
        let mut order: Vec<usize> = (0..gr.regs.len()).collect();
        order.sort_by_key(|i| gr.regs[*i].inside.len());
        for i in order {
            let a = &gr.regs[i];
            let span: f64 = (a.c0..=a.c1).map(|c| cw[c]).sum::<f64>() + (a.c0..a.c1).map(|c| gx[c] + right[c] + left[c + 1]).sum::<f64>() + inset[i].0 + inset[i].1;
            let need = self.region_title(a.r).1 + 2.0 * pad;
            if need > span {
                let add = (need - span) / (a.c1 - a.c0 + 1) as f64;
                for w in &mut cw[a.c0..=a.c1] {
                    *w += add;
                }
            }
        }
        // a block over several columns: they are together at least as wide as it
        for (&(n, c, _), &l) in gr.cell.iter().zip(&gr.last).filter(|(c, l)| **l > c.1) {
            let have: f64 = (c..=l).map(|k| cw[k]).sum::<f64>() + (c..l).map(|k| gx[k] + right[k] + left[k + 1]).sum::<f64>();
            let need = self.leaf(n, grow).w;
            if need > have && !wide(&self.g.nodes[n]) {
                let add = (need - have) / (l - c + 1) as f64;
                for w in &mut cw[c..=l] {
                    *w += add;
                }
            }
        }
        // a block over several rows: they are together at least as tall as it
        let mut parts = parts;
        for (k, &(n, _, r)) in gr.cell.iter().enumerate().filter(|(k, c)| gr.low[*k] > c.2) {
            let l = gr.low[k];
            let have: f64 = (r..=l).map(|i| parts[i].0 + parts[i].1).sum::<f64>() + (r..l).map(|i| gy[i] + bottom[i] + top[i + 1]).sum::<f64>();
            let b = self.leaf(n, grow);
            let need = b.over + b.up + b.dn + b.words;
            if need > have && !high(&self.g.nodes[n]) {
                let add = (need - have) / (l - r + 1) as f64 / 2.0;
                for p in &mut parts[r..=l] {
                    p.0 += add;
                    p.1 += add;
                }
            }
        }
        let w = cw.iter().sum::<f64>() + gx.iter().sum::<f64>() + left.iter().sum::<f64>() + right.iter().sum::<f64>();
        let h = parts.iter().map(|p| p.0 + p.1).sum::<f64>() + gy.iter().sum::<f64>() + top.iter().sum::<f64>() + bottom.iter().sum::<f64>();
        GridGeom { cw, parts, left, right, top, bottom, inset, gx, gy, w, h }
    }

    /// The one shape of an area on a grid: the grid, the area's title and its padding.
    fn grid_shape(&self, a: &Area, gr: &Grid, grow: u32) -> Shape {
        let geo = self.grid_geom(gr, grow);
        let (band, tw) = self.area_title(a);
        let pad = if a.bare { 0.0 } else { self.pad() };
        Shape { w: geo.w.max(tw + pad) + 2.0 * pad, h: geo.h + band + if band > 0.0 { pad } else { 2.0 * pad }, ends: vec![], kids: vec![] }
    }

    fn area_title(&self, a: &Area) -> (f64, f64) {
        if a.bare || a.title.is_empty() {
            return (0.0, 0.0);
        }
        let ts = self.m.theme.title_size();
        (ts * 1.3 + 2.0, text::width(&a.title, ts) * 1.06)
    }

    /// The shapes a frame can take at one enlargement: every way of breaking its items into
    /// rows, with every shape of the frames among them, the unbeaten ones kept (narrowest first).
    /// `top` = the title's band, `min_w` = the title's width.
    fn shapes(&self, items: &[Item], grow: u32, top: f64, min_w: f64, framed: bool) -> Vec<Shape> {
        let n = items.len();
        let pad = if framed { self.pad() } else { 0.0 };
        if n == 0 {
            return vec![Shape { w: min_w + 2.0 * pad, h: top + 2.0 * pad, ends: vec![], kids: vec![] }];
        }
        // each item's own shapes (a block has one)
        let own: Vec<Vec<(Box3, usize)>> = items
            .iter()
            .map(|it| match it {
                Item::Leaf(i) => vec![(self.leaf(*i, grow), 0)],
                Item::Group(r, kids) => self.group_shapes(*r, kids, grow).iter().enumerate().map(|(k, s)| (Box3 { w: s.w, up: s.h / 2.0, dn: s.h / 2.0, words: 0.0, over: 0.0, frame: true }, k)).collect(),
            })
            .collect();
        let mut combos: Vec<Vec<usize>> = vec![vec![]];
        for o in &own {
            let mut next = Vec::new();
            for c in &combos {
                for k in 0..o.len() {
                    let mut c2 = c.clone();
                    c2.push(k);
                    next.push(c2);
                }
            }
            combos = next;
            if combos.len() > 4000 {
                // too many: every further frame takes its first shape
                combos.truncate(4000);
            }
        }
        let gap = self.gap();
        let mut out: Vec<Shape> = Vec::new();
        for combo in &combos {
            let b: Vec<Box3> = combo.iter().enumerate().map(|(i, k)| own[i][*k].0).collect();
            let kids: Vec<usize> = combo.iter().enumerate().map(|(i, k)| own[i][*k].1).collect();
            // for every number of rows, the breaks with the least height, then the least width:
            // best[k][i] = (height, width) of items ..i in k rows
            let row = |a: usize, z: usize| -> (f64, f64) {
                let w = b[a..z].iter().map(|x| x.w).sum::<f64>() + gap * (z - a - 1) as f64;
                let (up, below, _, _) = row_parts(&b[a..z]);
                (w, up + below)
            };
            // widths worth trying: every row's width; for each, the fewest-height breaking
            let mut widths: Vec<f64> = Vec::new();
            for a in 0..n {
                for z in a + 1..=n {
                    widths.push(row(a, z).0);
                }
            }
            widths.sort_by(|x, y| x.partial_cmp(y).unwrap());
            widths.dedup_by(|x, y| (*x - *y).abs() < 0.01);
            // many blocks give thousands of widths that differ by little: an even sample of
            // them finds the same shapes to within one block's width
            if widths.len() > WIDTHS {
                let n = widths.len();
                widths = (0..WIDTHS).map(|i| widths[i * (n - 1) / (WIDTHS - 1)]).collect();
            }
            let widest_one = b.iter().map(|x| x.w).fold(0.0, f64::max);
            for &wl in widths.iter().filter(|w| **w >= widest_one - 0.01) {
                // least total height with every row at most `wl` wide
                let mut best: Vec<Option<(f64, usize)>> = vec![None; n + 1];
                best[0] = Some((0.0, 0));
                for z in 1..=n {
                    for a in 0..z {
                        let Some((h0, _)) = best[a] else { continue };
                        let (w, h) = row(a, z);
                        if w > wl + 0.01 {
                            continue;
                        }
                        let tot = h0 + h + if a > 0 { gap } else { 0.0 };
                        if best[z].is_none_or(|(t, _)| tot < t - 0.01) {
                            best[z] = Some((tot, a));
                        }
                    }
                }
                let Some((h, _)) = best[n] else { continue };
                let mut ends = Vec::new();
                let mut z = n;
                while z > 0 {
                    ends.push(z);
                    z = best[z].unwrap().1;
                }
                ends.reverse();
                // the width is that of the columns the rows share
                let (_, _, w) = columns(&b, &ends, gap);
                out.push(Shape { w: w.max(min_w) + 2.0 * pad, h: h + top + if top > 0.0 { pad } else { 2.0 * pad }, ends, kids: kids.clone() });
            }
        }
        // keep the unbeaten: sorted by width, each must be lower than every narrower one
        out.sort_by(|x, y| x.w.partial_cmp(&y.w).unwrap().then(x.h.partial_cmp(&y.h).unwrap()));
        let mut kept: Vec<Shape> = Vec::new();
        for s in out {
            if kept.last().is_none_or(|k| s.h < k.h - 0.01) {
                kept.push(s);
            }
        }
        kept
    }
}

/// The areas of the graph in order (loose blocks form a bare area, as in `plate`), each with its
/// blocks in the order they are written. A region is one item at the place of its first block.
fn areas(g: &Graph) -> Vec<Area> {
    let mut panels: Vec<(String, String, bool, bool)> = g.panels.iter().map(|p| (p.id.clone(), p.title.clone(), p.bare, p.dir == "down")).collect();
    if g.nodes.iter().any(|n| n.panel.is_empty()) {
        let feeds = g.edges.iter().filter(|e| g.node(&e.from).is_some_and(|n| n.panel.is_empty()) && g.node(&e.to).is_some_and(|n| !n.panel.is_empty())).count();
        let fed = g.edges.iter().filter(|e| g.node(&e.to).is_some_and(|n| n.panel.is_empty()) && g.node(&e.from).is_some_and(|n| !n.panel.is_empty())).count();
        let p = ("_".to_string(), String::new(), true, false);
        if g.panels.is_empty() || feeds >= fed { panels.insert(0, p) } else { panels.push(p) }
    }
    let rt = RTree::new(g);
    // the chain of regions a node lies in, outermost first
    let chain = |n: usize| -> Vec<usize> {
        let id = &g.nodes[n].id;
        let mut at = g.regions.iter().position(|r| r.members.iter().any(|m| m == id));
        let mut v = Vec::new();
        while let Some(r) = at {
            v.push(r);
            at = (0..g.regions.len()).find(|p| rt.kids[*p].contains(&r));
            if v.len() > g.regions.len() {
                break;
            }
        }
        v.reverse();
        v
    };
    fn build(nodes: &[usize], depth: usize, chain: &dyn Fn(usize) -> Vec<usize>) -> Vec<Item> {
        let mut out: Vec<Item> = Vec::new();
        let mut done: Vec<usize> = Vec::new();
        for &n in nodes {
            let c = chain(n);
            match c.get(depth) {
                None => out.push(Item::Leaf(n)),
                Some(r) if !done.contains(r) => {
                    done.push(*r);
                    let inside: Vec<usize> = nodes.iter().copied().filter(|x| chain(*x).get(depth) == Some(r)).collect();
                    out.push(Item::Group(*r, build(&inside, depth + 1, chain)));
                }
                _ => {}
            }
        }
        out
    }
    panels
        .into_iter()
        .map(|(id, title, bare, down)| {
            let all: Vec<usize> = (0..g.nodes.len()).filter(|i| if id == "_" { g.nodes[*i].panel.is_empty() } else { g.nodes[*i].panel == id }).collect();
            // notes take no room; an area of notes only keeps them as its objects
            let mut notes: Vec<usize> = all.iter().copied().filter(|n| is_note(g, *n)).collect();
            if notes.len() == all.len() {
                notes.clear();
            }
            let nodes: Vec<usize> = all.iter().copied().filter(|n| !notes.contains(n)).collect();
            let items = build(&nodes, 0, &chain);
            let grid = grid_of(g, &rt, &nodes, down);
            Area { bare: bare || id == "_", id, title, down, items, notes, grid }
        })
        .collect()
}

/// The rows of areas: the graph's `arrange` line, or `rows` given as ids (`a / b c`); every area
/// a row of its own when neither names them all.
fn rows_of(g: &Graph, ar: &[Area], named: &str) -> Vec<Vec<usize>> {
    let by = |id: &str| ar.iter().position(|a| a.id == id);
    let stated: Vec<Vec<String>> = if named.trim().is_empty() {
        g.arrange.clone()
    } else {
        let s = named.rsplit(": ").next().unwrap_or(named);
        s.split('/').map(|r| r.split_whitespace().map(|x| x.trim_end_matches("(folded)").to_string()).collect()).collect()
    };
    let mut rows: Vec<Vec<usize>> = stated.iter().map(|r| r.iter().filter_map(|id| by(id)).collect::<Vec<_>>()).filter(|r| !r.is_empty()).collect();
    if let Some(b) = by("_").filter(|b| !rows.iter().flatten().any(|x| x == b)) {
        if !rows.is_empty() {
            if b == 0 { rows[0].insert(0, b) } else { rows.last_mut().unwrap().push(b) }
        }
    }
    let mut all: Vec<usize> = rows.iter().flatten().copied().collect();
    all.sort();
    if all == (0..ar.len()).collect::<Vec<_>>() { rows } else { (0..ar.len()).map(|i| vec![i]).collect() }
}

/// The areas on a page `w` by `h` (natural points): for each row of areas its height and each
/// area's shape, or `None` when they do not fit.
struct Fit {
    /// Per row of areas: its height.
    row_h: Vec<f64>,
    /// Per area: the index of its shape.
    pick: Vec<usize>,
    /// Width and height used.
    used: (f64, f64),
}

fn fit(sh: &[Vec<Shape>], rows: &[Vec<usize>], w: f64, h: f64, between: f64) -> Option<Fit> {
    let mut pick = vec![0usize; sh.len()];
    let mut row_h = Vec::new();
    let mut used_w: f64 = 0.0;
    for r in rows {
        let room = w - between * (r.len() - 1) as f64;
        // the lowest height at which the areas of the row, each at its narrowest, fit across
        let mut ys: Vec<f64> = r.iter().flat_map(|p| sh[*p].iter().map(|s| s.h)).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut got = None;
        for y in ys {
            let ch: Option<Vec<usize>> = r.iter().map(|p| sh[*p].iter().position(|s| s.h <= y + 0.01)).collect();
            let Some(ch) = ch else { continue };
            let tot: f64 = r.iter().zip(&ch).map(|(p, k)| sh[*p][*k].w).sum();
            if tot <= room + 0.01 {
                got = Some((y, ch, tot));
                break;
            }
        }
        let (y, ch, tot) = got?;
        for (p, k) in r.iter().zip(ch) {
            pick[*p] = k;
        }
        used_w = used_w.max(tot + between * (r.len() - 1) as f64);
        row_h.push(y);
    }
    let tot_h = row_h.iter().sum::<f64>() + between * (rows.len().saturating_sub(1)) as f64;
    (tot_h <= h + 0.01).then_some(Fit { row_h, pick, used: (used_w, tot_h) })
}

/// Pack the figure on a page: see the module's words. `arrangement` names the rows of areas
/// (`a / b c`); empty = the graph's own `arrange`, else one area per row. `None` for a figure
/// without blocks.
pub fn pack(g: &Graph, f: &Format, m: &Measure, arrangement: &str, air: &Air) -> Option<Packed> {
    let mut p = pack_air(g, f, m, arrangement, air)?;
    // text under the least size: the room kept for lines goes back to the plain gap, which
    // gives the text what it can; a page still under it says so (`Packed.small`)
    if p.text_pt < air.text_min_pt && air.line > air.gap {
        if let Some(q) = pack_air(g, f, m, arrangement, &Air { line: air.gap, ..air.clone() }) {
            if q.text_pt > p.text_pt + 1e-6 {
                p = q;
            }
        }
    }
    p.small = p.text_pt < air.text_min_pt - 1e-6;
    Some(p)
}

fn pack_air(g: &Graph, f: &Format, m: &Measure, arrangement: &str, air: &Air) -> Option<Packed> {
    if g.nodes.is_empty() {
        return None;
    }
    let n = areas(g).len();
    let has: Vec<bool> = areas(g).iter().map(|a| a.grid.is_some()).collect();
    // every area in rows broken freely: the largest the text gets
    let free = pack_choice(g, f, m, arrangement, air, &vec![false; n])?;
    if !has.iter().any(|h| *h) || (air.paths <= 0.0 && !air.stated) {
        return Some(free);
    }
    // an area stands on the grid its text states when that makes its own lines simpler than
    // rows broken freely do (fewer bends, fewer blocks passed beneath); where it does not, the
    // freely broken rows are larger and no worse
    let all = pack_choice(g, f, m, arrangement, air, &has)?;
    if air.stated {
        return Some(all);
    }
    let trouble = |p: &Packed, area: usize| -> usize {
        crate::lines::lines(&p.graph, &p.layout)
            .iter()
            .filter(|r| {
                let e = &p.graph.edges[r.edge];
                [&e.from, &e.to].iter().all(|id| p.graph.node_index(id).is_some_and(|n| p.layout.nodes[n].panel == area))
            })
            .map(|r| {
                // a block of words hides nothing: a line through it counts as eight blocks passed
                let e = &p.graph.edges[r.edge];
                let own = [p.graph.node_index(&e.from), p.graph.node_index(&e.to)];
                let words = p.layout.nodes.iter().enumerate().filter(|(i, nd)| !own.contains(&Some(*i)) && p.graph.nodes[*i].module == "label" && r.points.windows(2).any(|w| crate::route::segment_hits(w[0], w[1], &nd.art.inflate(-0.5)))).count();
                r.bends() + 2 * crate::lines::beneath(&p.graph, &p.layout, r) + 14 * words
            })
            .sum()
    };
    let mut on: Vec<bool> = (0..n).map(|i| has[i] && trouble(&all, i) < trouble(&free, i)).collect();
    if !on.iter().any(|x| *x) {
        return Some(free);
    }
    // an area whose freely broken rows would send a line through a block of words (which hides
    // nothing, so line and words lie over each other) keeps its grid whatever it costs
    let through_words = |p: &Packed, area: usize| -> bool {
        crate::lines::lines(&p.graph, &p.layout).iter().any(|r| {
            let e = &p.graph.edges[r.edge];
            let own = [p.graph.node_index(&e.from), p.graph.node_index(&e.to)];
            own.iter().all(|n| n.is_some_and(|n| p.layout.nodes[n].panel == area))
                && p.layout.nodes.iter().enumerate().any(|(i, nd)| !own.contains(&Some(i)) && p.graph.nodes[i].module == "label" && r.points.windows(2).any(|w| crate::route::segment_hits(w[0], w[1], &nd.art.inflate(-0.5))))
        })
    };
    let must: Vec<bool> = (0..n).map(|i| on[i] && through_words(&free, i) && !through_words(&all, i)).collect();
    // while the grids cost more of the text than `Air.paths` allows, the area whose rows broken
    // freely give the most back goes free
    let mut cur = pack_choice(g, f, m, arrangement, air, &on)?;
    while cur.text_pt < air.paths * free.text_pt - 1e-6 && (0..n).any(|i| on[i] && !must[i]) {
        let mut best: Option<(usize, Packed)> = None;
        for i in (0..n).filter(|i| on[*i] && !must[*i]) {
            let mut v = on.clone();
            v[i] = false;
            if let Some(p) = pack_choice(g, f, m, arrangement, air, &v) {
                if best.as_ref().is_none_or(|b| p.text_pt > b.1.text_pt + 1e-6) {
                    best = Some((i, p));
                }
            }
        }
        let Some((i, p)) = best else { break };
        on[i] = false;
        cur = p;
    }
    Some(cur)
}

/// One packing with the areas named in `on_grid` on the grid their text states: notes go in the
/// air the objects leave; a frame whose notes found none gets a band for them above and below
/// its rows, and the page is packed again.
fn pack_choice(g: &Graph, f: &Format, m: &Measure, arrangement: &str, air: &Air, on_grid: &[bool]) -> Option<Packed> {
    let (mut ba, mut br) = (vec![0.0; on_grid.len()], vec![0.0; g.regions.len()]);
    let mut out = None;
    for _ in 0..3 {
        let (p, want) = pack_with(g, f, m, arrangement, air, &ba, &br, on_grid)?;
        let done = p.crowded.is_empty();
        out = Some(p);
        if done {
            break;
        }
        for (frame, h) in want {
            match frame {
                Frame2::Area(i) => ba[i] = ba[i].max(h),
                Frame2::Region(r) => br[r] = br[r].max(h),
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
enum Frame2 {
    Area(usize),
    Region(usize),
}

/// One packing with the given bands; with it, for every note without free room, its frame and
/// the band that would hold it.
#[allow(clippy::too_many_arguments)]
fn pack_with(g: &Graph, f: &Format, m: &Measure, arrangement: &str, air: &Air, band_area: &[f64], band_reg: &[f64], on_grid: &[bool]) -> Option<(Packed, Vec<(Frame2, f64)>)> {
    let mut ar = areas(g);
    for (a, on) in ar.iter_mut().zip(on_grid) {
        if !on {
            a.grid = None;
        }
    }
    let rows = rows_of(g, &ar, arrangement);
    let cx = Ctx2 { g, m, air, fs: m.theme.font.size, cache: RefCell::new(BTreeMap::new()), band_area: band_area.to_vec(), band_reg: band_reg.to_vec() };
    let mut want: Vec<(Frame2, f64)> = Vec::new();
    let between = air.between * cx.fs;
    let ratio = f.ratio.1.max(1) as f64 / f.ratio.0.max(1) as f64;
    let shapes_at = |grow: &[u32]| -> Vec<Vec<Shape>> {
        ar.iter()
            .enumerate()
            .map(|(i, a)| {
                let (band, tw) = cx.area_title(a);
                let mut v = match &a.grid {
                    Some(gr) => vec![cx.grid_shape(a, gr, grow[i])],
                    None => cx.shapes(&a.items, grow[i], band, tw + if a.bare { 0.0 } else { cx.pad() }, !a.bare),
                };
                for s in &mut v {
                    s.h += 2.0 * cx.band_area[i];
                }
                v
            })
            .collect()
    };
    // 1. the smallest page that holds everything unenlarged
    let mut grow = vec![100u32; ar.len()];
    let sh0 = shapes_at(&grow);
    let (mut lo, mut hi) = (1.0f64, 64.0f64);
    while fit(&sh0, &rows, hi, hi * ratio, between).is_none() {
        hi *= 2.0;
        if hi > 1e7 {
            return None;
        }
    }
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if fit(&sh0, &rows, mid, mid * ratio, between).is_some() { hi = mid } else { lo = mid }
    }
    let pw = if air.text_max_pt > 0.0 { hi.max(cx.fs * f.width_pt / air.text_max_pt) } else { hi };
    let ph = pw * ratio;
    // 2 and 3. enlarge the areas together; those that cannot grow further drop out, the rest go on
    let top = steps(air.grow_max).max(100);
    let fits = |grow: &[u32]| fit(&shapes_at(grow), &rows, pw, ph, between).is_some();
    let mut active: Vec<usize> = (0..ar.len()).filter(|i| has_growable(g, &ar[*i].items)).collect();
    let growing = active.clone();
    let mut cap = top;
    let mut first = true;
    while !active.is_empty() {
        let base = grow.clone();
        // the largest common addition for the active areas
        let (mut a, mut b) = (0u32, cap.saturating_sub(active.iter().map(|i| base[*i]).min().unwrap()));
        let with = |d: u32| -> Vec<u32> {
            let mut v = base.clone();
            for &i in &active {
                v[i] = (base[i] + d).min(cap);
            }
            v
        };
        if b > 0 && fits(&with(b)) {
            a = b;
        } else {
            while b - a > 1 {
                let mid = (a + b) / 2;
                if fits(&with(mid)) { a = mid } else { b = mid }
            }
        }
        grow = with(a);
        if first {
            // what all took together is the figure's scale: no area goes far over it
            first = false;
            let least = growing.iter().map(|i| grow[*i]).min().unwrap_or(100);
            cap = top.min((least as f64 * air.grow_apart.max(1.0)).floor() as u32).max(least);
        }
        // who can still take one more step alone?
        let before = active.len();
        active.retain(|&i| {
            if grow[i] >= cap {
                return false;
            }
            let mut v = grow.clone();
            v[i] += 1;
            fits(&v)
        });
        if a == 0 && active.len() == before {
            // each alone could, together they cannot: the first takes its step
            grow[active[0]] += 1;
        }
    }
    let sh = shapes_at(&grow);
    let ft = fit(&sh, &rows, pw, ph, between)?;
    // the rows of areas take the whole height, the areas of a row the whole width, in proportion
    let avail_h = ph - between * (rows.len() - 1) as f64;
    let sum_h: f64 = ft.row_h.iter().sum();
    let mut rects = vec![Rect::new(0.0, 0.0, 0.0, 0.0); ar.len()];
    let mut y = 0.0;
    for (ri, r) in rows.iter().enumerate() {
        let rh = ft.row_h[ri] * avail_h / sum_h;
        let avail_w = pw - between * (r.len() - 1) as f64;
        let sum_w: f64 = r.iter().map(|p| sh[*p][ft.pick[*p]].w).sum();
        let mut x = 0.0;
        for &p in r {
            let w = sh[p][ft.pick[p]].w * avail_w / sum_w;
            rects[p] = Rect::new(x, y, w, rh);
            x += w + between;
        }
        y += rh + between;
    }
    // every block in its place
    let mut tight: Vec<Rect> = vec![Rect::new(0.0, 0.0, 0.0, 0.0); g.nodes.len()];
    let mut drop: Vec<f64> = vec![0.0; g.nodes.len()];
    let mut rise: Vec<f64> = vec![0.0; g.nodes.len()];
    let mut reg_rect: Vec<Option<Rect>> = vec![None; g.regions.len()];
    let mut out_areas = Vec::new();
    let (mut notes, mut crowded) = (Vec::new(), Vec::new());
    let mut gather_dir: BTreeMap<usize, &'static str> = BTreeMap::new();
    let mut unmet: Vec<String> = Vec::new();
    // for an area on a grid: the middle of each column and the line of each row
    let mut axes: Vec<Option<(Vec<f64>, Vec<f64>)>> = vec![None; ar.len()];
    for (i, a) in ar.iter().enumerate() {
        let (band, tw) = cx.area_title(a);
        let s = &sh[i][ft.pick[i]];
        let mut rows_txt = Vec::new();
        match &a.grid {
            Some(gr) => axes[i] = Some(grid_place(&cx, a, gr, grow[i], rects[i], cx.band_area[i], &mut tight, (&mut drop, &mut rise), &mut reg_rect, &mut rows_txt)),
            None => place(&cx, &a.items, grow[i], s, rects[i], band, cx.band_area[i], !a.bare, &mut tight, (&mut drop, &mut rise), &mut reg_rect, Some(&mut rows_txt)),
        }
        let _ = tw;
        // the notes, in the air the objects left: each inside the innermost frame it belongs to
        let pad = if a.bare { 0.0 } else { cx.pad() };
        let inner = Rect::new(rects[i].x + pad, rects[i].y + band + if band > 0.0 { 0.0 } else { pad }, rects[i].w - 2.0 * pad, rects[i].h - band - if band > 0.0 { pad } else { 2.0 * pad });
        let objects = leaves(&a.items);
        let mut taken: Vec<(usize, Rect)> = objects.iter().map(|n| (*n, tight[*n])).collect();
        // (marks that gather are written last: what they gather may be marks placed before)
        let num = |n: usize, k: &str| g.nodes[n].num(k).unwrap_or(0.0);
        let side = |a: f64, b: f64| if a > b { 1.0 } else if a < b { -1.0 } else { 0.0 };
        // a mark tied to another mark is written after it, beside where that one was written
        let mut order: Vec<usize> = Vec::new();
        let mut left: Vec<usize> = a.notes.clone();
        while !left.is_empty() {
            let ready: Vec<usize> = left.iter().copied().filter(|n| !g.nodes[*n].str("by").and_then(|b| g.node_index(b)).is_some_and(|p| p != *n && left.contains(&p))).collect();
            let ready = if ready.is_empty() { left.clone() } else { ready };
            left.retain(|n| !ready.contains(n));
            order.extend(ready);
        }
        order.sort_by_key(|n| gathers(g, *n).is_some());
        for &n in &order {
            let b = cx.leaf(n, 100);
            let id = &g.nodes[n].id;
            // a mark that gathers: along what it gathers, as long as they are together, on the
            // side of the block it points at
            let along: Vec<Rect> = gathers(g, n).unwrap_or_default().iter().filter(|m| taken.iter().any(|t| t.0 == **m)).map(|m| tight[*m]).collect();
            if !along.is_empty() {
                let bb = along.iter().skip(1).fold(along[0], |u, r| u.union(r));
                let to = g.nodes[n].str("toward").and_then(|t| g.node_index(t)).filter(|t| taken.iter().any(|x| x.0 == *t)).map(|t| tight[t]);
                let dir = match to {
                    Some(t) if t.x >= bb.right() - 0.5 => "right",
                    Some(t) if t.right() <= bb.x + 0.5 => "left",
                    Some(t) if t.y >= bb.bottom() - 0.5 => "down",
                    Some(t) if t.bottom() <= bb.y + 0.5 => "up",
                    _ => match g.nodes[n].str("dir") {
                        Some("left") => "left",
                        Some("down") => "down",
                        Some("up") => "up",
                        _ => "right",
                    },
                };
                // thin, and close to what it gathers: a brace reads with its things, not alone
                let (nat_thick, nat_len) = (b.w.min(b.over + b.up + b.dn + b.words), b.w.max(b.over + b.up + b.dn + b.words));
                let thick = nat_thick * THIN;
                let air = cx.fs * 0.3;
                // what must surround it: its open side as long as a brace is at least, its tip an object
                let len = if matches!(dir, "left" | "right") { bb.h } else { bb.w };
                if len < nat_len * 0.5 {
                    unmet.push(format!("{id}：開口端的東西太短，括號括不出範圍"));
                }
                if to.is_none() {
                    unmet.push(format!("{id}：尖端沒有指向任何物件"));
                }
                let r = match dir {
                    "right" => Rect::new(bb.right() + air, bb.y, thick, bb.h),
                    "left" => Rect::new(bb.x - air - thick, bb.y, thick, bb.h),
                    "down" => Rect::new(bb.x, bb.bottom() + air, bb.w, thick),
                    _ => Rect::new(bb.x, bb.y - air - thick, bb.w, thick),
                };
                tight[n] = r;
                taken.push((n, r));
                notes.push(id.clone());
                gather_dir.insert(n, dir);
                continue;
            }
            // its frame: the smallest region that names it, else the area
            let home = (0..g.regions.len()).filter(|r| g.regions[*r].members.iter().any(|x| x == id) && reg_rect[*r].is_some()).min_by(|x, y| (reg_rect[*x].unwrap().w * reg_rect[*x].unwrap().h).partial_cmp(&(reg_rect[*y].unwrap().w * reg_rect[*y].unwrap().h)).unwrap());
            let room = match home {
                Some(r) => {
                    let (rr, t) = (reg_rect[r].unwrap(), cx.region_title(r).0);
                    Rect::new(rr.x + cx.pad(), rr.y + t, rr.w - 2.0 * cx.pad(), rr.h - t - cx.pad())
                }
                None => inner,
            };
            // the air kept from what is taken: a mark the one who draws tied to a block (`by=`)
            // keeps room for the shaft and arrowhead of each of its own lines (`Air.line`) from
            // the block at the line's other end, so none of its lines is too short to be seen;
            // else the little air a note keeps
            let tied = g.nodes[n].str("by").is_some();
            let joined = |x: usize| g.edges.iter().any(|e| (e.from == *id && g.node_index(&e.to) == Some(x)) || (e.to == *id && g.node_index(&e.from) == Some(x)));
            let line_room = cx.air.line * cx.fs;
            let keep = |x: usize| if tied && joined(x) { line_room } else { 0.0 };
            // in the area's own room the frames inside it are taken whole
            let mut shut: Vec<(Rect, f64)> = taken.iter().map(|(x, r)| (*r, keep(*x))).collect();
            if home.is_none() {
                shut.extend(reg_rect.iter().flatten().filter(|rr| rects[i].contains(rr)).map(|r| (*r, 0.0)));
            }
            let stated = g.nodes[n].str("by").and_then(|b| g.node_index(b)).filter(|p| *p != n && taken.iter().any(|t| t.0 == *p));
            let partner = stated.or_else(|| g.edges.iter().filter_map(|e| if &e.from == id { g.node_index(&e.to) } else if &e.to == id { g.node_index(&e.from) } else { None }).find(|p| objects.contains(p)));
            let by = partner.map(|p| {
                let (dc, dl) = (side(num(n, "col"), num(p, "col")), side(num(n, "lane"), num(p, "lane")));
                // `side=` says where about the block; else the mark's own column and row do
                let d = match g.nodes[n].str("side") {
                    Some("below") => (0.0, 1.0),
                    Some("above") => (0.0, -1.0),
                    Some("left") => (-1.0, 0.0),
                    Some("right") => (1.0, 0.0),
                    _ if a.down => (dl, dc),
                    _ => (dc, dl),
                };
                (tight[p], d, keep(p))
            });
            let size = (b.w, b.over + b.up + b.dn + b.words);
            let (r, ok) = note_place(size, by, room, &shut, cx.fs * 0.5);
            tight[n] = r;
            taken.push((n, r));
            notes.push(id.clone());
            if !ok {
                crowded.push(id.clone());
                // the band holds the mark and the room of its line to its block
                let line = by.map(|b| b.2).unwrap_or(0.0);
                want.push((home.map(Frame2::Region).unwrap_or(Frame2::Area(i)), size.1 + (cx.fs * 0.6).max(line)));
            }
        }
        let covered: f64 = leaves(&a.items).iter().map(|n| tight[*n].w * tight[*n].h).sum();
        out_areas.push(AreaPack { grid: a.grid.is_some(), id: a.id.clone(), grow: grow[i] as f64 / 100.0, rows: rows_txt, cover: covered / (rects[i].w * rects[i].h).max(1.0) });
    }
    // the layout: each block drawn as `build` draws it (its frame mirrored about the picture's
    // middle line in a left-to-right area), laid so that its picture lies where the tight box has it
    let mut graph = g.clone();
    for (n, dir) in &gather_dir {
        graph.nodes[*n].params.insert("dir".into(), serde_json::json!(dir));
        if graph.nodes[*n].num("thick").is_none() {
            graph.nodes[*n].params.insert("thick".into(), serde_json::json!(10.0 * THIN));
        }
    }
    let area_of: BTreeMap<usize, usize> = ar.iter().enumerate().flat_map(|(i, a)| leaves(&a.items).into_iter().chain(a.notes.iter().copied()).map(move |n| (n, i))).collect();
    let mut nodes = Vec::new();
    for (n, node) in g.nodes.iter().enumerate() {
        let pi = area_of[&n];
        let gr = if modules::growable(node) { grow[pi] } else { 100 };
        graph.nodes[n].params.remove("grow");
        if gr > 100 {
            graph.nodes[n].params.insert("grow".into(), serde_json::json!(gr as f64 / 100.0));
        }
        graph.nodes[n].params.remove("label_drop");
        if drop[n] > 0.05 {
            graph.nodes[n].params.insert("label_drop".into(), serde_json::json!((drop[n] * 100.0).round() / 100.0));
        }
        graph.nodes[n].params.remove("label_rise");
        if rise[n] > 0.05 {
            graph.nodes[n].params.insert("label_rise".into(), serde_json::json!((rise[n] * 100.0).round() / 100.0));
        }
        // a block that takes the width of its columns: its drawing stretched across, its height kept
        // (a mark that gathers is as long as what it gathers, on any area)
        let (is_wide, is_high) = match gather_dir.get(&n) {
            Some(d) => (matches!(*d, "up" | "down"), matches!(*d, "left" | "right")),
            None => (wide(node) && axes[pi].is_some(), high(node) && axes[pi].is_some()),
        };
        if is_wide || is_high {
            let nat = body(&graph.nodes[n], gr, false, m);
            let (tw, th) = (if is_wide { tight[n].w } else { nat.w }, if is_high { tight[n].h } else { nat.art.3 });
            if (tw - nat.w).abs() > 0.5 || (th - nat.art.3).abs() > 0.5 {
                let mut probe = graph.nodes[n].clone();
                probe.params.insert("w".into(), serde_json::json!(100.0));
                probe.params.insert("h".into(), serde_json::json!(100.0));
                let u = body(&probe, gr, false, m);
                if u.w > 0.0 && u.art.3 > 0.0 {
                    graph.nodes[n].params.insert("w".into(), serde_json::json!(tw * 100.0 / u.w));
                    graph.nodes[n].params.insert("h".into(), serde_json::json!(th * 100.0 / u.art.3));
                }
                // a mark whose length is its `size` (a brace) takes that instead
                let got = body(&graph.nodes[n], gr, false, m);
                if (is_high && (got.art.3 - th).abs() > 0.5) || (is_wide && (got.w - tw).abs() > 0.5) {
                    graph.nodes[n].params.remove("w");
                    graph.nodes[n].params.remove("h");
                    probe.params.remove("w");
                    probe.params.remove("h");
                    probe.params.insert("size".into(), serde_json::json!(100.0));
                    let u = body(&probe, gr, false, m);
                    let (have, want) = if is_high { (u.art.3, th) } else { (u.w, tw) };
                    if have > 0.0 {
                        graph.nodes[n].params.insert("size".into(), serde_json::json!(want * 100.0 / have));
                    }
                }
            }
        }
        let node = &graph.nodes[n];
        let t = body(node, gr, false, m);
        let b = body(node, gr, !ar[pi].down, m);
        let tr = tight[n];
        // the tight body sits in the middle of its box's width (a label wider than the picture)
        let tx = tr.x + (tr.w - t.w) / 2.0;
        let rect = Rect::new(tx + t.art.0 - b.art.0, tr.y + t.art.1 - b.art.1, b.w, b.h);
        let reserve = Rect::new(rect.cx() - b.reserve_w / 2.0, rect.y, b.reserve_w, rect.h);
        let art = Rect::new(rect.x + b.art.0, rect.y + b.art.1, b.art.2, b.art.3);
        nodes.push(Placed { rect, reserve, port_y: rect.y + b.cy, art, words: b.words, panel: pi, layer: 0, lane: 0, single: b.single, ellipse: b.ellipse, ends: BTreeMap::new(), origin: (0.0, 0.0) });
    }
    for (ai, a) in ar.iter().enumerate() {
        let ns: Vec<usize> = leaves(&a.items).into_iter().chain(a.notes.iter().copied()).collect();
        if let Some((xs, ys)) = &axes[ai] {
            // on a grid: the column and the row the block stands in (a note: the nearest)
            let near = |v: &[f64], x: f64| (0..v.len()).min_by(|a, b| (v[*a] - x).abs().partial_cmp(&(v[*b] - x).abs()).unwrap()).unwrap_or(0);
            for &n in &ns {
                let (c, r) = (near(xs, nodes[n].art.cx()), near(ys, nodes[n].art.cy()));
                (nodes[n].layer, nodes[n].lane) = if a.down { (r, c) } else { (c, r) };
            }
            continue;
        }
        let key = |v: f64| (v * 10.0).round() as i64;
        let mut xs: Vec<i64> = ns.iter().map(|n| key(nodes[*n].art.cx())).collect();
        let mut ys: Vec<i64> = ns.iter().map(|n| key(nodes[*n].port_y)).collect();
        xs.sort();
        xs.dedup();
        ys.sort();
        ys.dedup();
        for &n in &ns {
            let (x, y) = (xs.iter().position(|v| *v == key(nodes[n].art.cx())).unwrap(), ys.iter().position(|v| *v == key(nodes[n].port_y)).unwrap());
            (nodes[n].layer, nodes[n].lane) = if a.down { (y, x) } else { (x, y) };
        }
    }
    let ts = m.theme.title_size();
    let panels: Vec<PanelBox> = ar
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let r = rects[i];
            let (band, tw) = cx.area_title(a);
            let title_rect = (band > 0.0).then(|| Rect::new(r.x + cx.pad(), r.y + 1.0, tw, ts * 1.3));
            PanelBox { id: a.id.clone(), title: a.title.clone(), rect: r, title_rect, bare: a.bare, down: a.down }
        })
        .collect();
    let regions = if g.regions.is_empty() {
        Vec::new()
    } else {
        let rt = RTree::new(g);
        let rooms: Vec<Rect> = nodes.iter().map(|n| n.rect.union(&n.reserve)).collect();
        layout::frame_regions(g, &rt, &rooms, m.theme, 1.0)
            .into_iter()
            .map(|(i, mut b, _)| {
                if let Some(r) = reg_rect[i] {
                    let (tw, th, _) = crate::plate::reg_title(&g.regions[i], m.theme);
                    b.title = crate::plate::title_lines(&g.regions[i].title, m.theme.font.size);
                    b.title_rect = (!g.regions[i].title.is_empty()).then(|| Rect::new(r.x + cx.pad(), r.y + 1.0, tw, th));
                    b.rect = r;
                }
                b
            })
            .collect()
    };
    let lay = Layout { nodes, panels, regions, legend: Vec::new(), swatches: Vec::new(), bounds: Rect::new(0.0, 0.0, pw, ph), scale: 1.0 };
    let cover = tight.iter().map(|r| r.w * r.h).sum::<f64>() / (pw * ph);
    Some((Packed { layout: lay, graph, text_pt: cx.fs * f.width_pt / pw, cover, areas: out_areas, slack: (1.0 - ft.used.0 / pw, 1.0 - ft.used.1 / ph), notes, crowded, small: false, unmet }, want))
}

fn leaves(items: &[Item]) -> Vec<usize> {
    let mut v = Vec::new();
    for it in items {
        match it {
            Item::Leaf(n) => v.push(*n),
            Item::Group(_, k) => v.extend(leaves(k)),
        }
    }
    v
}

fn has_growable(g: &Graph, items: &[Item]) -> bool {
    leaves(items).iter().any(|n| modules::growable(&g.nodes[*n]))
}

/// Lay the items of a frame out in `rect` as shape `s` says: rows spread evenly down the frame
/// (so they are centred in it), the columns the rows share spread evenly across it, each item in
/// the middle of its column and on its row's middle line, the words under the pictures of a row
/// on one line (`drop` = how much lower a block's words start for that). A frame among the items
/// keeps its own size.
#[allow(clippy::too_many_arguments)]
/// `drop`: how far under its picture each block's words are written (`label_drop`); `rise`:
/// how far over it, for words written above (`label_rise`).
fn place(cx: &Ctx2, items: &[Item], grow: u32, s: &Shape, rect: Rect, top: f64, band: f64, framed: bool, tight: &mut [Rect], (drop, rise): (&mut [f64], &mut [f64]), reg: &mut [Option<Rect>], mut rows_txt: Option<&mut Vec<Vec<String>>>) {
    if items.is_empty() {
        return;
    }
    let pad = if framed { cx.pad() } else { 0.0 };
    let gap = cx.gap();
    // each item's box in the shape chosen
    let sub: Vec<(Box3, Option<Shape>)> = items
        .iter()
        .zip(&s.kids)
        .map(|(it, k)| match it {
            Item::Leaf(n) => (cx.leaf(*n, grow), None),
            Item::Group(r, kids) => {
                let all = cx.group_shapes(*r, kids, grow);
                let sh = all[(*k).min(all.len() - 1)].clone();
                (Box3 { w: sh.w, up: sh.h / 2.0, dn: sh.h / 2.0, words: 0.0, over: 0.0, frame: true }, Some(sh))
            }
        })
        .collect();
    let boxes: Vec<Box3> = sub.iter().map(|x| x.0).collect();
    let mut bounds = Vec::new();
    let mut a = 0;
    for &e in &s.ends {
        bounds.push((a, e));
        a = e;
    }
    let parts: Vec<(f64, f64, f64, f64)> = bounds.iter().map(|(a, z)| row_parts(&boxes[*a..*z])).collect();
    let inner_h = rect.h - top - 2.0 * band - if top > 0.0 { pad } else { 2.0 * pad };
    let need_h: f64 = parts.iter().map(|(u, d, _, _)| u + d).sum::<f64>() + gap * (bounds.len() - 1) as f64;
    let spare_h = (inner_h - need_h).max(0.0) / (bounds.len() + 1) as f64;
    // the columns, spread evenly across the frame
    let (cols, first, _) = columns(&boxes, &s.ends, gap);
    let need_w: f64 = cols.iter().sum::<f64>() + gap * (cols.len() - 1) as f64;
    let spare_w = (rect.w - 2.0 * pad - need_w).max(0.0) / (cols.len() + 1) as f64;
    let mut col_x = Vec::new();
    let mut x = rect.x + pad + spare_w;
    for w in &cols {
        col_x.push(x);
        x += w + gap + spare_w;
    }
    let mut y = rect.y + top + band + if top > 0.0 { 0.0 } else { pad } + spare_h;
    for (ri, (a, z)) in bounds.iter().enumerate() {
        let (up, below, dn, lup) = parts[ri];
        let line = y + up;
        let mut names = Vec::new();
        // a row on its own: its items spread evenly across the frame
        let own_w: f64 = boxes[*a..*z].iter().map(|x| x.w).sum::<f64>() + gap * (z - a - 1) as f64;
        let own_spare = (rect.w - 2.0 * pad - own_w).max(0.0) / (z - a + 1) as f64;
        let mut own_x = rect.x + pad + own_spare;
        for i in *a..*z {
            let (b, sh) = &sub[i];
            let x = match first[ri] {
                Some(f) => col_x[f + i - a] + (cols[f + i - a] - b.w) / 2.0,
                None => own_x,
            };
            own_x += b.w + gap + own_spare;
            match &items[i] {
                Item::Leaf(n) => {
                    let d = if b.words > 0.0 { dn - b.dn } else { 0.0 };
                    let r = if b.over > 0.0 { lup - b.up } else { 0.0 };
                    tight[*n] = Rect::new(x, line - b.up - r - b.over, b.w, b.over + r + b.up + b.dn + d + b.words);
                    drop[*n] = d;
                    rise[*n] = r;
                    names.push(cx.g.nodes[*n].id.clone());
                }
                Item::Group(rg, kids) => {
                    // the frames of one row are equally tall
                    let r = Rect::new(x, y, b.w, up + below);
                    reg[*rg] = Some(r);
                    names.push(format!("[{}]", cx.g.regions[*rg].id));
                    place(cx, kids, grow, sh.as_ref().unwrap(), r, cx.region_title(*rg).0, cx.band_reg[*rg], true, tight, (&mut *drop, &mut *rise), reg, None);
                }
            }
        }
        if let Some(t) = rows_txt.as_deref_mut() {
            t.push(names);
        }
        y += up + below + gap + spare_h;
    }
}

/// Lay an area's grid out in `rect`: the columns and the rows spread evenly (so the grid is
/// centred), every block in the middle of its column and on its row's line, every region around
/// the cells of its blocks. Returns each column's middle and each row's line.
#[allow(clippy::too_many_arguments)]
fn grid_place(cx: &Ctx2, a: &Area, gr: &Grid, grow: u32, rect: Rect, band: f64, tight: &mut [Rect], (drop, rise): (&mut [f64], &mut [f64]), reg: &mut [Option<Rect>], rows_txt: &mut Vec<Vec<String>>) -> (Vec<f64>, Vec<f64>) {
    let geo = cx.grid_geom(gr, grow);
    let (title, _) = cx.area_title(a);
    let pad = if a.bare { 0.0 } else { cx.pad() };
    let inner_w = rect.w - 2.0 * pad;
    let inner_h = rect.h - title - 2.0 * band - if title > 0.0 { pad } else { 2.0 * pad };
    let spare_w = (inner_w - geo.w).max(0.0) / (gr.cols + 1) as f64;
    let spare_h = (inner_h - geo.h).max(0.0) / (gr.rows + 1) as f64;
    let mut xs = Vec::new();
    let mut x = rect.x + pad;
    for c in 0..gr.cols {
        x += spare_w + geo.left[c];
        xs.push(x);
        x += geo.cw[c] + geo.right[c] + geo.gx.get(c).copied().unwrap_or(0.0);
    }
    let mut ys = Vec::new();
    let mut y = rect.y + title + band + if title > 0.0 { 0.0 } else { pad };
    for r in 0..gr.rows {
        y += spare_h + geo.top[r];
        ys.push(y);
        y += geo.parts[r].0 + geo.parts[r].1 + geo.bottom[r] + geo.gy.get(r).copied().unwrap_or(0.0);
    }
    let mut names: Vec<Vec<(usize, String)>> = vec![Vec::new(); gr.rows];
    for (k, (&(n, c, r), &l)) in gr.cell.iter().zip(&gr.last).enumerate() {
        let b = cx.leaf(n, grow);
        let (up, _, dn, lup) = geo.parts[r];
        let d = if b.words > 0.0 { dn - b.dn } else { 0.0 };
        // words over the picture: over the tallest picture of the row
        let rs = if b.over > 0.0 { lup - b.up } else { 0.0 };
        // over several rows: the middle of its picture and words in the middle of those rows
        let lo = gr.low[k];
        let (up, d, rs) = if lo > r {
            let mid = (ys[r] + ys[lo] + geo.parts[lo].0 + geo.parts[lo].1) / 2.0;
            (mid - ys[r] - (b.dn + b.words - b.up - b.over) / 2.0, 0.0, 0.0)
        } else {
            (up, d, rs)
        };
        let over = xs[l] + geo.cw[l] - xs[c];
        if high(&cx.g.nodes[n]) && lo > r {
            let w = if wide(&cx.g.nodes[n]) { over } else { b.w };
            tight[n] = Rect::new(xs[c] + (over - w) / 2.0, ys[r], w, ys[lo] + geo.parts[lo].0 + geo.parts[lo].1 - ys[r]);
            names[r].push((c, cx.g.nodes[n].id.clone()));
            continue;
        }
        if wide(&cx.g.nodes[n]) {
            tight[n] = Rect::new(xs[c], ys[r] + up - b.up - rs - b.over, over, b.over + rs + b.up + b.dn + d + b.words);
            drop[n] = d;
            rise[n] = rs;
            names[r].push((c, cx.g.nodes[n].id.clone()));
            continue;
        }
        tight[n] = Rect::new(xs[c] + (over - b.w) / 2.0, ys[r] + up - b.up - rs - b.over, b.w, b.over + rs + b.up + b.dn + d + b.words);
        drop[n] = d;
        rise[n] = rs;
        names[r].push((c, cx.g.nodes[n].id.clone()));
    }
    for mut row in names {
        row.sort();
        rows_txt.push(row.into_iter().map(|x| x.1).collect());
    }
    // a region's frame lies round its own blocks (what they take in their cells, not the whole
    // cells: a row is as tall as its tallest block, which may be none of the region's), never
    // past its cells; regions over the same rows keep one top and bottom, regions over the same
    // columns one left and right
    let mut fr: Vec<(f64, f64, f64, f64)> = gr
        .regs
        .iter()
        .zip(&geo.inset)
        .map(|(q, i)| {
            let (cx0, cx1) = (xs[q.c0] - i.0, xs[q.c1] + geo.cw[q.c1] + i.1);
            let (cy0, cy1) = (ys[q.r0] - i.2, ys[q.r1] + geo.parts[q.r1].0 + geo.parts[q.r1].1 + i.3);
            let t: Vec<Rect> = q.cells.iter().map(|k| tight[gr.cell[*k].0]).collect();
            let x0 = t.iter().map(|r| r.x).fold(f64::INFINITY, f64::min) - i.0;
            let x1 = t.iter().map(|r| r.right()).fold(f64::NEG_INFINITY, f64::max) + i.1;
            let y0 = t.iter().map(|r| r.y).fold(f64::INFINITY, f64::min) - i.2;
            let y1 = t.iter().map(|r| r.bottom()).fold(f64::NEG_INFINITY, f64::max) + i.3;
            // at least as wide as its title asks (the cells were made that wide)
            let need = cx.region_title(q.r).1 + 2.0 * cx.pad();
            let (x0, x1) = if x1 - x0 < need { let m = (x0 + x1) / 2.0; (m - need / 2.0, m + need / 2.0) } else { (x0, x1) };
            (x0.max(cx0), x1.min(cx1), y0.max(cy0), y1.min(cy1))
        })
        .collect();
    let was = fr.clone();
    for (i, q) in gr.regs.iter().enumerate() {
        for (j, o) in gr.regs.iter().enumerate() {
            if (o.r0, o.r1) == (q.r0, q.r1) {
                fr[i].2 = fr[i].2.min(was[j].2);
                fr[i].3 = fr[i].3.max(was[j].3);
            }
            if (o.c0, o.c1) == (q.c0, q.c1) {
                fr[i].0 = fr[i].0.min(was[j].0);
                fr[i].1 = fr[i].1.max(was[j].1);
            }
        }
    }
    for (q, f) in gr.regs.iter().zip(&fr) {
        reg[q.r] = Some(Rect::new(f.0, f.2, f.1 - f.0, f.3 - f.2));
    }
    ((0..gr.cols).map(|c| xs[c] + geo.cw[c] / 2.0).collect(), (0..gr.rows).map(|r| ys[r] + geo.parts[r].0).collect())
}

/// Where a note is written: in free room of its area, as near as it gets to the side of the
/// block it is tied to on which the figure's text has it (its column and row against that
/// block's; below it when they say nothing). `None` when the area has no free room for it.
///
/// Each taken rect comes with the room a line needs between it and the note (0 when no line
/// joins them; then the note keeps a little air): a mark with lines of its own keeps room for
/// their shafts and arrowheads, so none of them is too short to be seen.
fn note_place(size: (f64, f64), by: Option<(Rect, (f64, f64), f64)>, inner: Rect, taken: &[(Rect, f64)], step: f64) -> (Rect, bool) {
    let (w, h) = size;
    let air = step * 0.6;
    let want = match by {
        Some((p, (dx, dy), line)) => {
            let (dx, dy) = if dx == 0.0 && dy == 0.0 { (0.0, 1.0) } else { (dx, dy) };
            let gap = air.max(line);
            (p.cx() + dx * (p.w / 2.0 + w / 2.0 + gap), p.cy() + dy * (p.h / 2.0 + h / 2.0 + gap))
        }
        None => (inner.cx(), inner.cy()),
    };
    let by = by.map(|(p, d, _)| (p, d));
    let clamp = |v: f64, lo: f64, hi: f64| if hi < lo { (lo + hi) / 2.0 } else { v.max(lo).min(hi) };
    let at = |cx: f64, cy: f64| Rect::new(cx - w / 2.0, cy - h / 2.0, w, h);
    let free = |r: &Rect| !taken.iter().any(|(t, line)| t.inflate((air * 0.5).max(*line)).overlaps(r));
    let mut best: Option<(f64, Rect)> = None;
    let (x0, x1, y0, y1) = (inner.x + w / 2.0, inner.right() - w / 2.0, inner.y + h / 2.0, inner.bottom() - h / 2.0);
    let mut tries: Vec<(f64, f64)> = vec![(clamp(want.0, x0, x1), clamp(want.1, y0, y1))];
    let mut y = y0;
    while y <= y1 + 0.01 {
        let mut x = x0;
        while x <= x1 + 0.01 {
            tries.push((x, y));
            x += step;
        }
        y += step;
    }
    for (cx, cy) in tries {
        let r = at(cx, cy);
        if free(&r) {
            // beside or under its block before over it: over a block is where its own label
            // or a line coming down to it would be looked for
            let over = by.is_some_and(|(p, (_, dy))| dy >= 0.0 && cy < p.y);
            let d = (cx - want.0).powi(2) + (cy - want.1).powi(2) + if over { (step * 40.0).powi(2) } else { 0.0 };
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, r));
            }
        }
    }
    match best {
        Some((_, r)) => (r, true),
        None => (at(clamp(want.0, x0, x1), clamp(want.1, y0, y1)), false),
    }
}
