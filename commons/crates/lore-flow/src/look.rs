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

fn walk(o: &Object, xf: &Xf, path: &str, out: &mut Vec<(String, Item)>) {
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
            if c.line.is_some() {
                out.push((name, Item::Line(connector_path(&c.geom, &c.adj, &o.frame).into_iter().map(|p| xf.pt(p)).collect())));
            }
        }
        Kind::Group(g) => {
            let f = &o.frame;
            let sx = if g.ch.w == 0 { 1.0 } else { f.w as f64 / g.ch.w as f64 };
            let sy = if g.ch.h == 0 { 1.0 } else { f.h as f64 / g.ch.h as f64 };
            let inner = Xf { sx: xf.sx * sx, sy: xf.sy * sy, ox: r.x - g.ch.x as f64 / EMU_PT * xf.sx * sx, oy: r.y - g.ch.y as f64 / EMU_PT * xf.sy * sy };
            for ch in &g.children {
                walk(ch, &inner, &name, out);
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
    for o in objects {
        walk(o, &root, "", &mut items);
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
