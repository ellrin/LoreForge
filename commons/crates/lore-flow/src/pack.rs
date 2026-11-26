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
}

/// A block's tight box at one enlargement: width, height, and how far its picture's middle line
/// lies under its top.
#[derive(Debug, Clone, Copy)]
struct Box3 {
    w: f64,
    h: f64,
    up: f64,
}

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
        let out = Box3 { w, h: b.h, up: b.art.1 + b.art.3 / 2.0 };
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
        let all = self.shapes(kids, grow, band, tw, true);
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
                Item::Group(r, kids) => self.group_shapes(*r, kids, grow).iter().enumerate().map(|(k, s)| (Box3 { w: s.w, h: s.h, up: s.h / 2.0 }, k)).collect(),
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
                let up = b[a..z].iter().map(|x| x.up).fold(0.0, f64::max);
                let dn = b[a..z].iter().map(|x| x.h - x.up).fold(0.0, f64::max);
                (w, up + dn)
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
                let mut a = 0;
                let mut w: f64 = 0.0;
                for &e in &ends {
                    w = w.max(row(a, e).0);
                    a = e;
                }
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
            let nodes: Vec<usize> = (0..g.nodes.len()).filter(|i| if id == "_" { g.nodes[*i].panel.is_empty() } else { g.nodes[*i].panel == id }).collect();
            let items = build(&nodes, 0, &chain);
            Area { bare: bare || id == "_", id, title, down, items }
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
    let ar = areas(g);
    let rows = rows_of(g, &ar, arrangement);
    let cx = Ctx2 { g, m, air, fs: m.theme.font.size, cache: RefCell::new(BTreeMap::new()) };
    let between = air.between * cx.fs;
    let ratio = f.ratio.1.max(1) as f64 / f.ratio.0.max(1) as f64;
    let shapes_at = |grow: &[u32]| -> Vec<Vec<Shape>> {
        ar.iter()
            .enumerate()
            .map(|(i, a)| {
                let (band, tw) = cx.area_title(a);
                cx.shapes(&a.items, grow[i], band, tw + if a.bare { 0.0 } else { cx.pad() }, !a.bare)
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
    let mut reg_rect: Vec<Option<Rect>> = vec![None; g.regions.len()];
    let mut out_areas = Vec::new();
    for (i, a) in ar.iter().enumerate() {
        let (band, tw) = cx.area_title(a);
        let s = &sh[i][ft.pick[i]];
        let mut rows_txt = Vec::new();
        place(&cx, &a.items, grow[i], s, rects[i], band, !a.bare, &mut tight, &mut reg_rect, Some(&mut rows_txt));
        let _ = tw;
        let covered: f64 = leaves(&a.items).iter().map(|n| tight[*n].w * tight[*n].h).sum();
        out_areas.push(AreaPack { id: a.id.clone(), grow: grow[i] as f64 / 100.0, rows: rows_txt, cover: covered / (rects[i].w * rects[i].h).max(1.0) });
    }
    // the layout: each block drawn as `build` draws it (its frame mirrored about the picture's
    // middle line in a left-to-right area), laid so that its picture lies where the tight box has it
    let mut graph = g.clone();
    let area_of: BTreeMap<usize, usize> = ar.iter().enumerate().flat_map(|(i, a)| leaves(&a.items).into_iter().map(move |n| (n, i))).collect();
    let mut nodes = Vec::new();
    for (n, node) in g.nodes.iter().enumerate() {
        let pi = area_of[&n];
        let gr = if modules::growable(node) { grow[pi] } else { 100 };
        graph.nodes[n].params.remove("grow");
        if gr > 100 {
            graph.nodes[n].params.insert("grow".into(), serde_json::json!(gr as f64 / 100.0));
        }
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
        let ns = leaves(&a.items);
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
    Some(Packed { layout: lay, graph, text_pt: cx.fs * f.width_pt / pw, cover, areas: out_areas, slack: (1.0 - ft.used.0 / pw, 1.0 - ft.used.1 / ph) })
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

/// Lay the items of a frame out in `rect` as shape `s` says: rows spread evenly down the frame,
/// the items of a row evenly across it, each on its row's middle line. A frame among the items
/// keeps its own size.
#[allow(clippy::too_many_arguments)]
fn place(cx: &Ctx2, items: &[Item], grow: u32, s: &Shape, rect: Rect, top: f64, framed: bool, tight: &mut [Rect], reg: &mut [Option<Rect>], mut rows_txt: Option<&mut Vec<Vec<String>>>) {
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
                (Box3 { w: sh.w, h: sh.h, up: sh.h / 2.0 }, Some(sh))
            }
        })
        .collect();
    let mut bounds = Vec::new();
    let mut a = 0;
    for &e in &s.ends {
        bounds.push((a, e));
        a = e;
    }
    let row_h: Vec<(f64, f64)> = bounds.iter().map(|(a, z)| (sub[*a..*z].iter().map(|x| x.0.up).fold(0.0, f64::max), sub[*a..*z].iter().map(|x| x.0.h - x.0.up).fold(0.0, f64::max))).collect();
    let inner_h = rect.h - top - if top > 0.0 { pad } else { 2.0 * pad };
    let need_h: f64 = row_h.iter().map(|(u, d)| u + d).sum::<f64>() + gap * (bounds.len() - 1) as f64;
    let spare_h = (inner_h - need_h).max(0.0) / (bounds.len() + 1) as f64;
    let mut y = rect.y + top + if top > 0.0 { 0.0 } else { pad } + spare_h;
    for (ri, (a, z)) in bounds.iter().enumerate() {
        let need_w: f64 = sub[*a..*z].iter().map(|x| x.0.w).sum::<f64>() + gap * (z - a - 1) as f64;
        let spare_w = (rect.w - 2.0 * pad - need_w).max(0.0) / (z - a + 1) as f64;
        let mut x = rect.x + pad + spare_w;
        let line = y + row_h[ri].0;
        let mut names = Vec::new();
        for i in *a..*z {
            let (b, sh) = &sub[i];
            let r = Rect::new(x, line - b.up, b.w, b.h);
            match &items[i] {
                Item::Leaf(n) => {
                    tight[*n] = r;
                    names.push(cx.g.nodes[*n].id.clone());
                }
                Item::Group(rg, kids) => {
                    // the frames of one row are equally tall
                    let r = Rect::new(x, y, b.w, row_h[ri].0 + row_h[ri].1);
                    reg[*rg] = Some(r);
                    names.push(format!("[{}]", cx.g.regions[*rg].id));
                    place(cx, kids, grow, sh.as_ref().unwrap(), r, cx.region_title(*rg).0, true, tight, reg, None);
                }
            }
            x += b.w + gap + spare_w;
        }
        if let Some(t) = rows_txt.as_deref_mut() {
            t.push(names);
        }
        y += row_h[ri].0 + row_h[ri].1 + gap + spare_h;
    }
}
