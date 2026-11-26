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
    /// The largest the text is drawn on the page, in points (0 = as large as it goes). With a
    /// limit the page that is left goes to the pictures.
    pub text_max_pt: f64,
}

impl Default for Air {
    fn default() -> Air {
        Air { gap: 1.5, pad: 0.8, between: 0.6, grow_max: 4.0, text_max_pt: 0.0 }
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
}

/// A note, not an object: a block that is words only (`label`) and sits on lines none of which
/// carries the flow (no `data` line). It says something about a line or a block (a weight fed in
/// from the side, the name of a comparison), so it takes no room in the filling and does not
/// move the objects; it is written in the air next to the block it is tied to. Words without any
/// line (a legend's rows, a heading) stay objects: nothing says where else they belong.
pub fn is_note(g: &Graph, n: usize) -> bool {
    let node = &g.nodes[n];
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
    /// A frame (a region), not a block: it is as tall as its row and sets no line for words.
    frame: bool,
}

/// A row's height above its middle line, its height below it, and how far under the line the
/// words of its blocks start (under the tallest picture among the blocks; a frame in the row
/// does not push them down).
fn row_parts(b: &[Box3]) -> (f64, f64, f64) {
    let up = b.iter().map(|x| x.up).fold(0.0, f64::max);
    let leaf_dn = b.iter().filter(|x| !x.frame).map(|x| x.dn).fold(0.0, f64::max);
    let words = b.iter().filter(|x| !x.frame).map(|x| x.words).fold(0.0, f64::max);
    let below = b.iter().filter(|x| x.frame).map(|x| x.dn).fold(leaf_dn + words, f64::max);
    (up, below, leaf_dn)
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
        let out = Box3 { w, up: b.axis, dn: b.h - b.below - b.axis, words: b.below, frame: false };
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
                Item::Group(r, kids) => self.group_shapes(*r, kids, grow).iter().enumerate().map(|(k, s)| (Box3 { w: s.w, up: s.h / 2.0, dn: s.h / 2.0, words: 0.0, frame: true }, k)).collect(),
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
                let (up, below, _) = row_parts(&b[a..z]);
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
            Area { bare: bare || id == "_", id, title, down, items, notes }
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
    if g.nodes.is_empty() {
        return None;
    }
    // notes go in the air the objects leave; a frame whose notes found none gets a band for
    // them above and below its rows, and the page is packed again
    let (mut ba, mut br) = (vec![0.0; areas(g).len()], vec![0.0; g.regions.len()]);
    let mut out = None;
    for _ in 0..3 {
        let (p, want) = pack_with(g, f, m, arrangement, air, &ba, &br)?;
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
fn pack_with(g: &Graph, f: &Format, m: &Measure, arrangement: &str, air: &Air, band_area: &[f64], band_reg: &[f64]) -> Option<(Packed, Vec<(Frame2, f64)>)> {
    let ar = areas(g);
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
                let mut v = cx.shapes(&a.items, grow[i], band, tw + if a.bare { 0.0 } else { cx.pad() }, !a.bare);
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
    while !active.is_empty() {
        let base = grow.clone();
        // the largest common addition for the active areas
        let (mut a, mut b) = (0u32, top - active.iter().map(|i| base[*i]).min().unwrap());
        let with = |d: u32| -> Vec<u32> {
            let mut v = base.clone();
            for &i in &active {
                v[i] = (base[i] + d).min(top);
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
        // who can still take one more step alone?
        let before = active.len();
        active.retain(|&i| {
            if grow[i] >= top {
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
    let mut reg_rect: Vec<Option<Rect>> = vec![None; g.regions.len()];
    let mut out_areas = Vec::new();
    let (mut notes, mut crowded) = (Vec::new(), Vec::new());
    for (i, a) in ar.iter().enumerate() {
        let (band, tw) = cx.area_title(a);
        let s = &sh[i][ft.pick[i]];
        let mut rows_txt = Vec::new();
        place(&cx, &a.items, grow[i], s, rects[i], band, cx.band_area[i], !a.bare, &mut tight, &mut drop, &mut reg_rect, Some(&mut rows_txt));
        let _ = tw;
        // the notes, in the air the objects left: each inside the innermost frame it belongs to
        let pad = if a.bare { 0.0 } else { cx.pad() };
        let inner = Rect::new(rects[i].x + pad, rects[i].y + band + if band > 0.0 { 0.0 } else { pad }, rects[i].w - 2.0 * pad, rects[i].h - band - if band > 0.0 { pad } else { 2.0 * pad });
        let objects = leaves(&a.items);
        let mut taken: Vec<Rect> = objects.iter().map(|n| tight[*n]).collect();
        let num = |n: usize, k: &str| g.nodes[n].num(k).unwrap_or(0.0);
        let side = |a: f64, b: f64| if a > b { 1.0 } else if a < b { -1.0 } else { 0.0 };
        for &n in &a.notes {
            let b = cx.leaf(n, 100);
            let id = &g.nodes[n].id;
            // its frame: the smallest region that names it, else the area
            let home = (0..g.regions.len()).filter(|r| g.regions[*r].members.iter().any(|x| x == id) && reg_rect[*r].is_some()).min_by(|x, y| (reg_rect[*x].unwrap().w * reg_rect[*x].unwrap().h).partial_cmp(&(reg_rect[*y].unwrap().w * reg_rect[*y].unwrap().h)).unwrap());
            let room = match home {
                Some(r) => {
                    let (rr, t) = (reg_rect[r].unwrap(), cx.region_title(r).0);
                    Rect::new(rr.x + cx.pad(), rr.y + t, rr.w - 2.0 * cx.pad(), rr.h - t - cx.pad())
                }
                None => inner,
            };
            // in the area's own room the frames inside it are taken whole
            let mut shut: Vec<Rect> = taken.clone();
            if home.is_none() {
                shut.extend(reg_rect.iter().flatten().filter(|rr| rects[i].contains(rr)).copied());
            }
            let partner = g.edges.iter().filter_map(|e| if &e.from == id { g.node_index(&e.to) } else if &e.to == id { g.node_index(&e.from) } else { None }).find(|p| objects.contains(p));
            let by = partner.map(|p| {
                let (dc, dl) = (side(num(n, "col"), num(p, "col")), side(num(n, "lane"), num(p, "lane")));
                (tight[p], if a.down { (dl, dc) } else { (dc, dl) })
            });
            let size = (b.w, b.up + b.dn + b.words);
            let (r, ok) = note_place(size, by, room, &shut, cx.fs * 0.5);
            tight[n] = r;
            taken.push(r);
            notes.push(id.clone());
            if !ok {
                crowded.push(id.clone());
                want.push((home.map(Frame2::Region).unwrap_or(Frame2::Area(i)), size.1 + cx.fs * 0.6));
            }
        }
        let covered: f64 = leaves(&a.items).iter().map(|n| tight[*n].w * tight[*n].h).sum();
        out_areas.push(AreaPack { id: a.id.clone(), grow: grow[i] as f64 / 100.0, rows: rows_txt, cover: covered / (rects[i].w * rects[i].h).max(1.0) });
    }
    // the layout: each block drawn as `build` draws it (its frame mirrored about the picture's
    // middle line in a left-to-right area), laid so that its picture lies where the tight box has it
    let mut graph = g.clone();
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
    for a in &ar {
        let ns: Vec<usize> = leaves(&a.items).into_iter().chain(a.notes.iter().copied()).collect();
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
    Some((Packed { layout: lay, graph, text_pt: cx.fs * f.width_pt / pw, cover, areas: out_areas, slack: (1.0 - ft.used.0 / pw, 1.0 - ft.used.1 / ph), notes, crowded }, want))
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
fn place(cx: &Ctx2, items: &[Item], grow: u32, s: &Shape, rect: Rect, top: f64, band: f64, framed: bool, tight: &mut [Rect], drop: &mut [f64], reg: &mut [Option<Rect>], mut rows_txt: Option<&mut Vec<Vec<String>>>) {
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
                (Box3 { w: sh.w, up: sh.h / 2.0, dn: sh.h / 2.0, words: 0.0, frame: true }, Some(sh))
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
    let parts: Vec<(f64, f64, f64)> = bounds.iter().map(|(a, z)| row_parts(&boxes[*a..*z])).collect();
    let inner_h = rect.h - top - 2.0 * band - if top > 0.0 { pad } else { 2.0 * pad };
    let need_h: f64 = parts.iter().map(|(u, d, _)| u + d).sum::<f64>() + gap * (bounds.len() - 1) as f64;
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
        let (up, below, dn) = parts[ri];
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
                    tight[*n] = Rect::new(x, line - b.up, b.w, b.up + b.dn + d + b.words);
                    drop[*n] = d;
                    names.push(cx.g.nodes[*n].id.clone());
                }
                Item::Group(rg, kids) => {
                    // the frames of one row are equally tall
                    let r = Rect::new(x, y, b.w, up + below);
                    reg[*rg] = Some(r);
                    names.push(format!("[{}]", cx.g.regions[*rg].id));
                    place(cx, kids, grow, sh.as_ref().unwrap(), r, cx.region_title(*rg).0, cx.band_reg[*rg], true, tight, drop, reg, None);
                }
            }
        }
        if let Some(t) = rows_txt.as_deref_mut() {
            t.push(names);
        }
        y += up + below + gap + spare_h;
    }
}

/// Where a note is written: in free room of its area, as near as it gets to the side of the
/// block it is tied to on which the figure's text has it (its column and row against that
/// block's; below it when they say nothing). `None` when the area has no free room for it.
fn note_place(size: (f64, f64), by: Option<(Rect, (f64, f64))>, inner: Rect, taken: &[Rect], step: f64) -> (Rect, bool) {
    let (w, h) = size;
    let air = step * 0.6;
    let want = match by {
        Some((p, (dx, dy))) => {
            let (dx, dy) = if dx == 0.0 && dy == 0.0 { (0.0, 1.0) } else { (dx, dy) };
            (p.cx() + dx * (p.w / 2.0 + w / 2.0 + air), p.cy() + dy * (p.h / 2.0 + h / 2.0 + air))
        }
        None => (inner.cx(), inner.cy()),
    };
    let clamp = |v: f64, lo: f64, hi: f64| if hi < lo { (lo + hi) / 2.0 } else { v.max(lo).min(hi) };
    let at = |cx: f64, cy: f64| Rect::new(cx - w / 2.0, cy - h / 2.0, w, h);
    let free = |r: &Rect| !taken.iter().any(|t| t.inflate(air * 0.5).overlaps(r));
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
