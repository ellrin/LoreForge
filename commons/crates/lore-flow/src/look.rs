//! Checks of how a drawn figure looks, on its canvas objects in drawing order: a line that
//! runs over words, and a line that shows through a shape drawn over it with no fill. Both
//! break the author's rule that a line never covers a block; 0 of each is the aim.

use lore_canvas::{Frame, Kind, Object, Paint};

use crate::build::connector_path;
use crate::layout::Rect;
use crate::modules::EMU_PT;

/// What one check found: the line's name and the other object's, both as paths of names.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub line: String,
    pub other: String,
    /// Where (pt, on the page).
    pub at: (f64, f64),
}

#[derive(Debug, Default)]
pub struct Report {
    /// A line drawn over words, or under words with nothing opaque between them.
    pub over_text: Vec<Hit>,
    /// A line passing under a shape that has an outline but no fill: it shows through.
    pub through_open: Vec<Hit>,
    /// Words that run past the shape they are written in (the shape's edge cuts them), or a
    /// frame's border that runs through words.
    pub words_cut: Vec<Hit>,
    /// A figure line whose start or end touches nothing: no block's drawing, no words of a
    /// block, no other line.
    pub loose_ends: Vec<Hit>,
    /// Two figure lines that share no block and run on top of each other for a stretch.
    pub on_line: Vec<Hit>,
    /// A figure line that passes behind a block that is not one of its ends.
    pub behind_block: Vec<Hit>,
    /// A figure line drawn OVER a module's frame (a region) that holds neither of its ends: it
    /// should pass beneath the module, hidden.
    pub over_module: Vec<Hit>,
    /// An arrowhead covered by something drawn over it: the head must always show.
    pub hidden_heads: Vec<Hit>,
}

/// `flow:<fig>/edge:<from>-<to>` → (from, to); `None` for a label, a junction dot or anything else.
fn edge_of(name: &str) -> Option<(&str, &str)> {
    let rest = name.split_once('/')?.1;
    let e = rest.strip_prefix("edge:")?;
    if e.contains('/') {
        return None;
    }
    e.split_once('-')
}

/// The block an object belongs to (`flow:<fig>/<id>/…`), not a frame, a line or a legend.
fn block_of(name: &str) -> Option<&str> {
    let rest = name.split_once('/')?.1;
    let id = rest.split('/').next()?;
    (!id.contains(':')).then_some(id)
}

fn near(r: &Rect, p: (f64, f64), tol: f64) -> bool {
    let g = r.inflate(tol);
    p.0 >= g.x && p.0 <= g.right() && p.1 >= g.y && p.1 <= g.bottom()
}

/// Distance from a point to a segment.
fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-9 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// The length two axis-aligned segments run on top of each other (within 0.8 pt).
fn overlap(a: ((f64, f64), (f64, f64)), b: ((f64, f64), (f64, f64))) -> f64 {
    let horiz = |s: ((f64, f64), (f64, f64))| (s.0.1 - s.1.1).abs() < 0.3;
    let vert = |s: ((f64, f64), (f64, f64))| (s.0.0 - s.1.0).abs() < 0.3;
    if horiz(a) && horiz(b) && (a.0.1 - b.0.1).abs() < 0.8 {
        (a.0.0.max(a.1.0).min(b.0.0.max(b.1.0)) - a.0.0.min(a.1.0).max(b.0.0.min(b.1.0))).max(0.0)
    } else if vert(a) && vert(b) && (a.0.0 - b.0.0).abs() < 0.8 {
        (a.0.1.max(a.1.1).min(b.0.1.max(b.1.1)) - a.0.1.min(a.1.1).max(b.0.1.min(b.1.1))).max(0.0)
    } else {
        0.0
    }
}

enum Item {
    Line(Vec<(f64, f64)>),
    /// Words that run past their own drawn shape.
    Spill(Rect),
    Text(Rect),
    Shape { rect: Rect, opaque: bool, outlined: bool },
}

struct Xf {
    sx: f64,
    sy: f64,
    ox: f64,
    oy: f64,
}

impl Xf {
    fn rect(&self, f: &Frame) -> Rect {
        Rect::new(self.ox + f.x as f64 / EMU_PT * self.sx, self.oy + f.y as f64 / EMU_PT * self.sy, f.w as f64 / EMU_PT * self.sx, f.h as f64 / EMU_PT * self.sy)
    }
    fn pt(&self, p: (f64, f64)) -> (f64, f64) {
        (self.ox + p.0 / EMU_PT * self.sx, self.oy + p.1 / EMU_PT * self.sy)
    }
}

fn has_words(o: &Object) -> bool {
    o.kind.sp().and_then(|sp| sp.text.as_ref()).is_some_and(|tb| tb.paras.iter().any(|p| p.runs.iter().any(|r| !r.text.trim().is_empty())))
}

/// Where the words of a text body sit in their frame: as many lines as it has, each as wide as
/// its characters, placed by the body's anchor and each paragraph's alignment (an estimate).
fn words_box(t: &lore_canvas::TextBody, r: Rect, scale: f64) -> Rect {
    let mut h = 0.0;
    let mut w: f64 = 0.0;
    let mut align = "l".to_string();
    for p in &t.paras {
        let size = p.runs.iter().map(|x| x.look.size).fold(0.0, f64::max).max(1.0) * scale;
        // the crate's own measure of the words, a little wider when bold; words may run past
        // their frame, so the box is not clamped to it
        let pw: f64 = p.runs.iter().map(|x| crate::text::width(&x.text, x.look.size * scale) * if x.look.bold { 1.08 } else { 1.0 }).sum();
        h += size * 1.2;
        w = w.max(pw);
        align = p.align.clone();
    }
    let vertical = t.vert.as_deref().is_some_and(|v| v.starts_with("vert") || v.starts_with("eaVert"));
    let (w, h) = if vertical { (h, w) } else { (w, h) };
    let x = if vertical { r.cx() - w / 2.0 } else {
        match align.as_str() {
            "ctr" => r.cx() - w / 2.0,
            "r" => r.right() - w,
            _ => r.x,
        }
    };
    let y = if vertical { r.cy() - h / 2.0 } else {
        match t.anchor.as_str() {
            "ctr" => r.cy() - h / 2.0,
            "b" => r.bottom() - h,
            _ => r.y,
        }
    };
    Rect::new(x, y, w, h).inflate(-0.5)
}

/// (name of a line, it has a head at its start, at its end)
type Heads = Vec<(String, bool, bool)>;

fn walk(o: &Object, xf: &Xf, path: &str, out: &mut Vec<(String, Item)>, heads: &mut Heads) {
    if o.hidden {
        return;
    }
    let name = if o.name.is_empty() { path.to_string() } else if path.is_empty() { o.name.clone() } else { format!("{path}/{}", o.name) };
    let r = xf.rect(&o.frame);
    match &o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            let custom = sp.geom.prst == "custom";
            let path_d = sp.geom.path.clone().unwrap_or_default();
            let open_path = custom && !path_d.trim_end().ends_with(['Z', 'z']);
            if open_path && sp.line.is_some() {
                let f = &o.frame;
                let mut pts: Vec<(f64, f64)> = lore_canvas::geom::path_vertices(&path_d)
                    .into_iter()
                    .map(|(u, v)| (r.x + if f.flip_h { 1.0 - u } else { u } * r.w, r.y + if f.flip_v { 1.0 - v } else { v } * r.h))
                    .collect();
                // one cubic curve (`M x y C x1 y1 x2 y2 x3 y3`): points along it, not the chord
                let nums: Vec<f64> = path_d.split(|ch: char| ch.is_whitespace() || ch == ',' || ch.is_ascii_alphabetic()).filter_map(|t| t.parse().ok()).collect();
                if path_d.contains('C') && nums.len() == 8 {
                    let c: Vec<(f64, f64)> = nums
                        .chunks(2)
                        .map(|q| (r.x + if f.flip_h { 1.0 - q[0] } else { q[0] } * r.w, r.y + if f.flip_v { 1.0 - q[1] } else { q[1] } * r.h))
                        .collect();
                    pts = (0..=24)
                        .map(|i| {
                            let t = i as f64 / 24.0;
                            let u = 1.0 - t;
                            let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                            (b0 * c[0].0 + b1 * c[1].0 + b2 * c[2].0 + b3 * c[3].0, b0 * c[0].1 + b1 * c[1].1 + b2 * c[2].1 + b3 * c[3].1)
                        })
                        .collect();
                }
                if let Some(l) = &sp.line {
                    heads.push((name.clone(), l.head != "none" && !l.head.is_empty(), l.tail != "none" && !l.tail.is_empty()));
                }
                out.push((name.clone(), Item::Line(pts)));
            } else if sp.line.is_some() || !matches!(sp.fill, Paint::None) {
                let opaque = matches!(&sp.fill, Paint::Solid { .. }) && !matches!(&sp.fill, Paint::Solid { alpha, .. } if *alpha < 0.95);
                out.push((name.clone(), Item::Shape { rect: r, opaque, outlined: sp.line.is_some() }));
            }
            if let Some(t) = sp.text.as_ref().filter(|_| has_words(o)) {
                let wb = words_box(t, r, xf.sx.min(xf.sy));
                // words in a drawn shape (outline or fill) that run past its edge
                if (sp.line.is_some() || !matches!(sp.fill, Paint::None)) && (wb.x < r.x - 1.0 || wb.right() > r.right() + 1.0 || wb.y < r.y - 1.0 || wb.bottom() > r.bottom() + 1.0) {
                    out.push((name.clone(), Item::Spill(wb)));
                }
                out.push((name, Item::Text(wb)));
            }
        }
        Kind::Connector(c) => {
            if let Some(l) = &c.line {
                heads.push((name.clone(), l.head != "none" && !l.head.is_empty(), l.tail != "none" && !l.tail.is_empty()));
                out.push((name, Item::Line(connector_path(&c.geom, &c.adj, &o.frame).into_iter().map(|p| xf.pt(p)).collect())));
            }
        }
        Kind::Group(g) => {
            let f = &o.frame;
            let sx = if g.ch.w == 0 { 1.0 } else { f.w as f64 / g.ch.w as f64 };
            let sy = if g.ch.h == 0 { 1.0 } else { f.h as f64 / g.ch.h as f64 };
            let inner = Xf { sx: xf.sx * sx, sy: xf.sy * sy, ox: r.x - g.ch.x as f64 / EMU_PT * xf.sx * sx, oy: r.y - g.ch.y as f64 / EMU_PT * xf.sy * sy };
            for ch in &g.children {
                walk(ch, &inner, &name, out, heads);
            }
        }
        Kind::Picture(_) => out.push((name, Item::Shape { rect: r, opaque: true, outlined: false })),
        _ => {}
    }
}

/// Where a segment enters a rect's inside: its middle point in the rect, if any.
fn crossing(a: (f64, f64), b: (f64, f64), r: &Rect) -> Option<(f64, f64)> {
    if r.w <= 0.5 || r.h <= 0.5 {
        return None;
    }
    // clip the segment to the rect (Liang-Barsky)
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0 - r.x), (dx, r.right() - a.0), (-dy, a.1 - r.y), (dy, r.bottom() - a.1)] {
        if p.abs() < 1e-9 {
            if q <= 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t1 - t0 < 1e-6 {
        return None;
    }
    let t = (t0 + t1) / 2.0;
    Some((a.0 + dx * t, a.1 + dy * t))
}

fn inside(r: &Rect, p: (f64, f64)) -> bool {
    p.0 > r.x && p.0 < r.right() && p.1 > r.y && p.1 < r.bottom()
}

/// Check a figure's objects (in drawing order).
pub fn check(objects: &[Object]) -> Report {
    let root = Xf { sx: 1.0, sy: 1.0, ox: 0.0, oy: 0.0 };
    let mut items = Vec::new();
    let mut heads: Heads = Vec::new();
    for o in objects {
        walk(o, &root, "", &mut items, &mut heads);
    }
    let mut rep = Report::default();
    for (name, it) in &items {
        if let Item::Spill(r) = it {
            rep.words_cut.push(Hit { line: name.clone(), other: name.clone(), at: (r.cx(), r.cy()) });
        }
    }
    // frame borders (panels, regions) through words that are not the frame's own title
    for (fname, it) in &items {
        let Item::Shape { rect, .. } = it else { continue };
        if !(fname.contains("/panel:") || fname.contains("/region:")) || fname.ends_with("/title") {
            continue;
        }
        let edges = [((rect.x, rect.y), (rect.right(), rect.y)), ((rect.right(), rect.y), (rect.right(), rect.bottom())), ((rect.x, rect.bottom()), (rect.right(), rect.bottom())), ((rect.x, rect.y), (rect.x, rect.bottom()))];
        for (tname, t) in &items {
            let Item::Text(tb) = t else { continue };
            if tname.starts_with(fname.as_str()) {
                continue;
            }
            if let Some(p) = edges.iter().find_map(|(a, b)| crossing(*a, *b, tb)) {
                rep.words_cut.push(Hit { line: fname.clone(), other: tname.clone(), at: p });
            }
        }
    }
    // the figure's own lines: loose ends, lines on lines, lines behind blocks
    let edges: Vec<(usize, &str, &str, &Vec<(f64, f64)>)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, (n, it))| match (edge_of(n), it) {
            (Some((a, b)), Item::Line(pts)) if pts.len() >= 2 => Some((i, a, b, pts)),
            _ => None,
        })
        .collect();
    for &(li, from, to, pts) in &edges {
        let lname = &items[li].0;
        for (which, p) in [("start", pts[0]), ("end", pts[pts.len() - 1])] {
            let touches = items.iter().enumerate().any(|(oi, (oname, o))| {
                if oi == li || oname.starts_with(lname.as_str()) {
                    return false;
                }
                match o {
                    Item::Shape { rect, .. } => !(oname.contains("/panel:") || oname.contains("/region:")) && near(rect, p, 3.0),
                    // words of a block (a label block is only words): within a little of them,
                    // the more the larger they are
                    Item::Text(t) => block_of(oname).is_some() && near(t, p, (0.45 * t.h.min(t.w)).max(4.5)),
                    Item::Line(q) => q.windows(2).any(|s| seg_dist(p, s[0], s[1]) < 2.0),
                    Item::Spill(_) => false,
                }
            });
            if !touches {
                rep.loose_ends.push(Hit { line: lname.clone(), other: which.to_string(), at: p });
            }
        }
        // drawn over a module's frame that holds neither end (it should pass beneath it)
        for (oi, (oname, o)) in items.iter().enumerate() {
            let Item::Shape { rect, opaque, .. } = o else { continue };
            if !oname.contains("/region:") || oname.ends_with("/title") {
                continue;
            }
            let inner = rect.inflate(-1.0);
            let holds = |p: (f64, f64)| p.0 > rect.x - 1.0 && p.0 < rect.right() + 1.0 && p.1 > rect.y - 1.0 && p.1 < rect.bottom() + 1.0;
            if holds(pts[0]) || holds(pts[pts.len() - 1]) || (oi > li && *opaque) {
                continue;
            }
            if let Some(at) = pts.windows(2).find_map(|sg| crossing(sg[0], sg[1], &inner)) {
                rep.over_module.push(Hit { line: lname.clone(), other: oname.clone(), at });
            }
        }
        // an arrowhead under something opaque drawn after the line
        if let Some((_, at_start, at_end)) = heads.iter().find(|h| &h.0 == lname) {
            let n = pts.len();
            for (has, tip, from) in [(*at_start, pts[0], pts[1]), (*at_end, pts[n - 1], pts[n - 2])] {
                if !has {
                    continue;
                }
                let d = ((tip.0 - from.0).powi(2) + (tip.1 - from.1).powi(2)).sqrt().max(1e-6);
                let back = (tip.0 + (from.0 - tip.0) / d * 3.0, tip.1 + (from.1 - tip.1) / d * 3.0);
                let cover = items.iter().enumerate().find(|(oi, (_, o))| *oi > li && matches!(o, Item::Shape { rect, opaque: true, .. } if { let g = rect.inflate(-0.5); back.0 > g.x && back.0 < g.right() && back.1 > g.y && back.1 < g.bottom() }));
                if let Some((_, (oname, _))) = cover {
                    rep.hidden_heads.push(Hit { line: lname.clone(), other: oname.clone(), at: tip });
                }
            }
        }
        // behind a block that is not one of its ends
        let mut seen: Vec<&str> = Vec::new();
        for (oname, o) in &items {
            let (Item::Shape { rect, opaque: true, .. }, Some(b)) = (o, block_of(oname)) else { continue };
            if b == from || b == to || seen.contains(&b) {
                continue;
            }
            if let Some(at) = pts.windows(2).find_map(|sg| crossing(sg[0], sg[1], &rect.inflate(-1.5))) {
                seen.push(b);
                rep.behind_block.push(Hit { line: lname.clone(), other: oname.clone(), at });
            }
        }
    }
    for (x, &(li, fa, ta, pa)) in edges.iter().enumerate() {
        for &(lj, fb, tb, pb) in &edges[x + 1..] {
            // lines that share a block may share a trunk on purpose
            if fa == fb || ta == tb || fa == tb || ta == fb {
                continue;
            }
            let hit = pa.windows(2).find_map(|u| pb.windows(2).find(|v| overlap((u[0], u[1]), (v[0], v[1])) > 6.0).map(|_| u[0]));
            if let Some(at) = hit {
                rep.on_line.push(Hit { line: items[li].0.clone(), other: items[lj].0.clone(), at });
            }
        }
    }
    for (li, (lname, it)) in items.iter().enumerate() {
        let Item::Line(pts) = it else { continue };
        for (oi, (oname, other)) in items.iter().enumerate() {
            if oi == li || oname.starts_with(lname.as_str()) || lname.starts_with(&format!("{oname}/")) {
                continue;
            }
            match other {
                Item::Text(t) => {
                    let Some(p) = pts.windows(2).find_map(|s| crossing(s[0], s[1], t)) else { continue };
                    // over the words, or under them with nothing opaque in between
                    let covered = oi > li && items[li + 1..oi].iter().any(|(_, x)| matches!(x, Item::Shape { rect, opaque: true, .. } if inside(rect, p)));
                    if !covered {
                        rep.over_text.push(Hit { line: lname.clone(), other: oname.clone(), at: p });
                    }
                }
                Item::Shape { rect, opaque: false, outlined: true } if oi > li => {
                    let inner = rect.inflate(-1.0);
                    let ends = [pts.first(), pts.last()].into_iter().flatten().any(|p| inside(&inner, *p));
                    if ends || oname.contains("/panel:") || oname.contains("/region:") {
                        continue;
                    }
                    let Some(p) = pts.windows(2).find_map(|s| crossing(s[0], s[1], &inner)) else { continue };
                    // something opaque drawn after the line and before the shape hides it
                    if !items[li + 1..oi].iter().any(|(_, x)| matches!(x, Item::Shape { rect, opaque: true, .. } if inside(rect, p))) {
                        rep.through_open.push(Hit { line: lname.clone(), other: oname.clone(), at: p });
                    }
                }
                _ => {}
            }
        }
    }
    rep
}

/// What a layout's frames get wrong: `(uneven, sparse)`. Uneven = pairs of framed panels side by
/// side (they overlap for more than half their height) whose tops or bottoms differ by more than
/// 1 pt, or stacked (more than half their width) whose left or right edges differ. Sparse =
/// framed panels whose blocks' box fills less than 45% of the panel.
pub fn frames(lay: &crate::layout::Layout) -> (usize, usize) {
    let ps: Vec<&crate::layout::PanelBox> = lay.panels.iter().filter(|p| !p.bare).collect();
    let mut uneven = 0;
    for (i, a) in ps.iter().enumerate() {
        for b in &ps[i + 1..] {
            let (ra, rb) = (&a.rect, &b.rect);
            let vshare = ra.bottom().min(rb.bottom()) - ra.y.max(rb.y);
            let hshare = ra.right().min(rb.right()) - ra.x.max(rb.x);
            if vshare > 0.5 * ra.h.min(rb.h) && hshare <= 0.0 && ((ra.y - rb.y).abs() > 1.0 || (ra.bottom() - rb.bottom()).abs() > 1.0) {
                uneven += 1;
            } else if hshare > 0.5 * ra.w.min(rb.w) && vshare <= 0.0 && ((ra.x - rb.x).abs() > 1.0 || (ra.right() - rb.right()).abs() > 1.0) {
                uneven += 1;
            }
        }
    }
    let mut sparse = 0;
    for (pi, p) in lay.panels.iter().enumerate().filter(|(_, p)| !p.bare) {
        let rs: Vec<Rect> = lay.nodes.iter().filter(|n| n.panel == pi).map(|n| n.rect.union(&n.reserve)).collect();
        if rs.is_empty() {
            continue;
        }
        let (x0, y0) = rs.iter().fold((f64::MAX, f64::MAX), |m, r| (m.0.min(r.x), m.1.min(r.y)));
        let (x1, y1) = rs.iter().fold((f64::MIN, f64::MIN), |m, r| (m.0.max(r.right()), m.1.max(r.bottom())));
        if (x1 - x0) * (y1 - y0) < 0.45 * p.rect.w * p.rect.h {
            sparse += 1;
        }
    }
    (uneven, sparse)
}

/// The smallest text size in the drawing, in points when the figure is `width_pt` wide (a
/// column of a paper is about 250 pt, a full page width about 500 pt).
pub fn smallest_text(objects: &[Object], figure_w: f64, width_pt: f64) -> f64 {
    fn walk(o: &Object, k: f64, min: &mut f64) {
        match &o.kind {
            Kind::Text(sp) | Kind::Shape(sp) => {
                if let Some(t) = &sp.text {
                    for r in t.paras.iter().flat_map(|p| p.runs.iter()).filter(|r| !r.text.trim().is_empty() && r.look.size > 0.0) {
                        *min = min.min(r.look.size * k);
                    }
                }
            }
            Kind::Group(g) => {
                let s = if g.ch.w == 0 { 1.0 } else { o.frame.w as f64 / g.ch.w as f64 };
                g.children.iter().for_each(|c| walk(c, k * s, min));
            }
            _ => {}
        }
    }
    let mut min = f64::MAX;
    objects.iter().for_each(|o| walk(o, 1.0, &mut min));
    if min == f64::MAX { 0.0 } else { min * width_pt / figure_w.max(1.0) }
}

/// What the frames of a layout get wrong against each other and against their blocks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameFaults {
    /// Two frames (panels or regions) that overlap although neither lies inside the other:
    /// `(id, id)`.
    pub overlaps: Vec<(String, String)>,
    /// A title that touches another title, or is crossed by the border of a frame that is not
    /// its own: `(the title's frame id, the other frame's id)`.
    pub titles: Vec<(String, String)>,
    /// A block outside the frame it belongs to (its panel, a region it is a member of), or
    /// inside a region it is not a member of: `(node id, frame id)`.
    pub strays: Vec<(String, String)>,
}

impl FrameFaults {
    pub fn count(&self) -> usize {
        self.overlaps.len() + self.titles.len() + self.strays.len()
    }
}

/// The frame faults of a layout (see [`FrameFaults`]). `g` is the graph the layout was made
/// from (the same nodes in the same order).
///
/// Decisions: a frame is a framed panel (`!bare`) or a region, panels first, then regions, each
/// in the layout's order. "Contains" is meant with 0.5 pt to spare. A title is crossed by
/// another frame's border when its rectangle shares area with that frame's rectangle but does
/// not lie wholly inside it (0.5 pt to spare): that covers a border running through the title
/// and a small frame lying inside the title. A title that merely touches a border from outside
/// shares no area and is fine. A region drawn with `line none` still counts as a frame.
pub fn frame_faults(g: &crate::spec::Graph, lay: &crate::layout::Layout) -> FrameFaults {
    struct F<'a> {
        id: &'a str,
        rect: Rect,
        title: Option<Rect>,
    }
    let mut frames: Vec<F> = lay.panels.iter().filter(|p| !p.bare).map(|p| F { id: &p.id, rect: p.rect, title: p.title_rect }).collect();
    frames.extend(lay.regions.iter().map(|r| F { id: &r.id, rect: r.rect, title: r.title_rect }));
    let area = |a: &Rect, b: &Rect, min: f64| a.right().min(b.right()) - a.x.max(b.x) > min && a.bottom().min(b.bottom()) - a.y.max(b.y) > min;
    let holds = |outer: &Rect, inner: &Rect, tol: f64| outer.inflate(tol).contains(inner);
    let push = |v: &mut Vec<(String, String)>, a: &str, b: &str| {
        let p = (a.to_string(), b.to_string());
        if !v.contains(&p) {
            v.push(p);
        }
    };
    let mut out = FrameFaults::default();
    // frames over each other
    for (i, a) in frames.iter().enumerate() {
        for b in &frames[i + 1..] {
            if area(&a.rect, &b.rect, 0.5) && !holds(&a.rect, &b.rect, 0.5) && !holds(&b.rect, &a.rect, 0.5) {
                push(&mut out.overlaps, a.id, b.id);
            }
        }
    }
    // titles against titles, and against other frames' borders
    for (i, a) in frames.iter().enumerate() {
        let Some(ta) = &a.title else { continue };
        for b in &frames[i + 1..] {
            if b.title.is_some_and(|tb| area(ta, &tb, 0.5)) {
                push(&mut out.titles, a.id, b.id);
            }
        }
        for b in frames.iter().filter(|b| b.id != a.id) {
            if area(ta, &b.rect, 0.0) && !holds(&b.rect, ta, -0.5) {
                push(&mut out.titles, a.id, b.id);
            }
        }
    }
    // blocks outside their frames, strangers inside a frame
    let region_of = |id: &str| lay.regions.iter().find(|r| r.id == id);
    // is the region `id` the region `top`, or inside it (by the parent chain)?
    let within = |id: &str, top: &str| {
        let mut cur = id;
        for _ in 0..=g.regions.len() {
            if cur == top {
                return true;
            }
            match g.regions.iter().find(|r| r.id == cur) {
                Some(r) if !r.parent.is_empty() => cur = &r.parent,
                _ => return false,
            }
        }
        false
    };
    for (nd, pl) in g.nodes.iter().zip(&lay.nodes) {
        if let Some(p) = lay.panels.get(pl.panel).filter(|p| !p.bare)
            && !holds(&p.rect, &pl.rect, 1.0)
        {
            push(&mut out.strays, &nd.id, &p.id);
        }
        for r in &g.regions {
            let Some(rb) = region_of(&r.id) else { continue };
            let member = g.regions.iter().any(|x| within(&x.id, &r.id) && x.members.contains(&nd.id));
            if member {
                if !holds(&rb.rect, &pl.rect, 1.0) {
                    push(&mut out.strays, &nd.id, &r.id);
                }
            } else if area(&pl.rect, &rb.rect, 1.0) {
                push(&mut out.strays, &nd.id, &r.id);
            }
        }
    }
    out
}

/// Points on the page per point of the layout when the whole figure is fitted into a page box
/// `width_pt` wide of ratio `w:h`: scaled until it touches the box, never stretched. A figure
/// whose shape is not the box's leaves part of the box empty.
pub fn page_scale(bounds: &Rect, width_pt: f64, ratio: (u32, u32)) -> f64 {
    let height_pt = width_pt * ratio.1.max(1) as f64 / ratio.0.max(1) as f64;
    (width_pt / bounds.w.max(1.0)).min(height_pt / bounds.h.max(1.0))
}

/// Share of the page box (ratio `w:h`) the blocks cover when the figure is fitted into it, 0..1:
/// the sum of the blocks' rectangles (picture and words) over the box. One measure for every
/// layout, the old one and the plate's.
pub fn page_cover(lay: &crate::layout::Layout, ratio: (u32, u32)) -> f64 {
    let s = page_scale(&lay.bounds, 1000.0, ratio);
    let box_area = 1000.0 * 1000.0 * ratio.1.max(1) as f64 / ratio.0.max(1) as f64;
    lay.nodes.iter().map(|n| n.rect.w * n.rect.h).sum::<f64>() * s * s / box_area
}

/// The smallest text on the page, in points, when the figure is fitted into a page box
/// `width_pt` wide of ratio `w:h` (unlike [`smallest_text`], a figure taller than the box is
/// scaled by its height).
pub fn page_text(objects: &[Object], lay: &crate::layout::Layout, width_pt: f64, ratio: (u32, u32)) -> f64 {
    smallest_text(objects, 1.0, 1.0) * page_scale(&lay.bounds, width_pt, ratio)
}

#[cfg(test)]
mod page_tests {
    use super::*;
    use crate::layout::{Layout, Placed};

    fn lay(w: f64, h: f64, blocks: &[(f64, f64)]) -> Layout {
        let nodes = blocks
            .iter()
            .map(|(bw, bh)| {
                let rect = Rect::new(0.0, 0.0, *bw, *bh);
                Placed { rect, reserve: rect, port_y: 0.0, art: rect, words: Default::default(), panel: 0, layer: 0, lane: 0, single: false, ellipse: false, ends: Default::default(), origin: (0.0, 0.0) }
            })
            .collect();
        Layout { nodes, panels: Vec::new(), regions: Vec::new(), legend: Vec::new(), swatches: Vec::new(), bounds: Rect::new(0.0, 0.0, w, h), scale: 1.0 }
    }

    #[test]
    fn a_figure_is_fitted_into_the_page_box_by_its_binding_side() {
        // 300 x 100 in a 3:2 box 600 pt wide: the width binds, twice the size
        assert_eq!(page_scale(&Rect::new(0.0, 0.0, 300.0, 100.0), 600.0, (3, 2)), 2.0);
        // 300 x 400 in the same box (400 pt high): the height binds, natural size
        assert_eq!(page_scale(&Rect::new(0.0, 0.0, 300.0, 400.0), 600.0, (3, 2)), 1.0);
    }

    #[test]
    fn cover_is_measured_on_the_page_box_not_on_the_figure() {
        // one block filling a square figure covers the whole of a square box
        assert!((page_cover(&lay(100.0, 100.0, &[(100.0, 100.0)]), (1, 1)) - 1.0).abs() < 1e-9);
        // the same figure in a 2:1 box covers half of it: the rest of the box is empty
        assert!((page_cover(&lay(100.0, 100.0, &[(100.0, 100.0)]), (2, 1)) - 0.5).abs() < 1e-9);
    }
}
