//! An icon's SVG body -> shapes: one per source path, line or filled, holes wound for the
//! non-zero rule.

use super::Shape;
use crate::path::{self, Seg};
use std::collections::HashMap;

type Attrs = Vec<(String, String)>;

fn attr<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

enum Tok {
    Open(String, Attrs, bool),
    Close(String),
}

/// Tags of an SVG fragment. `None` on a tag that is not closed.
fn tokens(s: &str) -> Option<Vec<Tok>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut quote = 0u8;
        while j < b.len() {
            let c = b[j];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        if j >= b.len() {
            return None;
        }
        let inner = &s[i + 1..j];
        i = j + 1;
        if inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        if let Some(n) = inner.strip_prefix('/') {
            out.push(Tok::Close(n.trim().to_string()));
            continue;
        }
        let (inner, selfclose) = match inner.strip_suffix('/') {
            Some(x) => (x, true),
            None => (inner, false),
        };
        let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
        let name = inner[..name_end].to_string();
        let mut attrs = Vec::new();
        let r = &inner[name_end..];
        let rb = r.as_bytes();
        let mut k = 0;
        while k < rb.len() {
            while k < rb.len() && rb[k].is_ascii_whitespace() {
                k += 1;
            }
            let ks = k;
            while k < rb.len() && rb[k] != b'=' && !rb[k].is_ascii_whitespace() {
                k += 1;
            }
            if k == ks {
                break;
            }
            let key = r[ks..k].to_string();
            while k < rb.len() && rb[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < rb.len() && rb[k] == b'=' {
                k += 1;
                while k < rb.len() && rb[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < rb.len() && (rb[k] == b'"' || rb[k] == b'\'') {
                    let q = rb[k];
                    let vs = k + 1;
                    let mut ve = vs;
                    while ve < rb.len() && rb[ve] != q {
                        ve += 1;
                    }
                    attrs.push((key, r[vs..ve.min(rb.len())].to_string()));
                    k = ve + 1;
                } else {
                    return None;
                }
            } else {
                attrs.push((key, String::new()));
            }
        }
        out.push(Tok::Open(name, attrs, selfclose));
    }
    Some(out)
}

#[derive(Clone)]
struct Style {
    fill: String,
    stroke: String,
    stroke_width: f64,
    evenodd: bool,
}

impl Style {
    fn with(&self, a: &Attrs) -> Style {
        let mut s = self.clone();
        if let Some(v) = attr(a, "fill") {
            s.fill = v.trim().to_string();
        }
        if let Some(v) = attr(a, "stroke") {
            s.stroke = v.trim().to_string();
        }
        if let Some(v) = attr(a, "stroke-width") {
            s.stroke_width = v.trim().parse().unwrap_or(s.stroke_width);
        }
        if let Some(v) = attr(a, "fill-rule") {
            s.evenodd = v.trim() == "evenodd";
        }
        s
    }
}

fn n(a: &Attrs, k: &str) -> f64 {
    attr(a, k).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0)
}

/// The path data of a drawing element.
fn element_d(tag: &str, a: &Attrs) -> Option<String> {
    Some(match tag {
        "path" => attr(a, "d")?.to_string(),
        "circle" => {
            let (cx, cy, r) = (n(a, "cx"), n(a, "cy"), n(a, "r"));
            ellipse_d(cx, cy, r, r)
        }
        "ellipse" => ellipse_d(n(a, "cx"), n(a, "cy"), n(a, "rx"), n(a, "ry")),
        "rect" => {
            let (x, y, w, h) = (n(a, "x"), n(a, "y"), n(a, "width"), n(a, "height"));
            let (mut rx, mut ry) = (n(a, "rx"), n(a, "ry"));
            if attr(a, "rx").is_some() && attr(a, "ry").is_none() {
                ry = rx;
            } else if attr(a, "ry").is_some() && attr(a, "rx").is_none() {
                rx = ry;
            }
            rx = rx.min(w / 2.0).max(0.0);
            ry = ry.min(h / 2.0).max(0.0);
            if rx > 0.0 && ry > 0.0 {
                format!(
                    "M{} {}H{}A{rx} {ry} 0 0 1 {} {}V{}A{rx} {ry} 0 0 1 {} {}H{}A{rx} {ry} 0 0 1 {} {}V{}A{rx} {ry} 0 0 1 {} {}Z",
                    x + rx, y, x + w - rx, x + w, y + ry, y + h - ry, x + w - rx, y + h, x + rx, x, y + h - ry, y + ry, x + rx, y
                )
            } else {
                format!("M{x} {y}H{}V{}H{x}Z", x + w, y + h)
            }
        }
        "line" => format!("M{} {}L{} {}", n(a, "x1"), n(a, "y1"), n(a, "x2"), n(a, "y2")),
        "polyline" | "polygon" => {
            let v: Vec<f64> = attr(a, "points")?
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|t| !t.is_empty())
                .map(|t| t.parse::<f64>())
                .collect::<Result<_, _>>()
                .ok()?;
            if v.len() < 4 || v.len() % 2 != 0 {
                return None;
            }
            let mut d = String::new();
            for (i, p) in v.chunks(2).enumerate() {
                d.push_str(&format!("{}{} {}", if i == 0 { "M" } else { "L" }, p[0], p[1]));
            }
            if tag == "polygon" {
                d.push('Z');
            }
            d
        }
        _ => return None,
    })
}

fn ellipse_d(cx: f64, cy: f64, rx: f64, ry: f64) -> String {
    format!("M{} {}A{rx} {ry} 0 1 0 {} {}A{rx} {ry} 0 1 0 {} {}Z", cx - rx, cy, cx + rx, cy, cx - rx, cy)
}

const DRAWN: [&str; 7] = ["path", "circle", "ellipse", "rect", "line", "polyline", "polygon"];

/// The shapes of an icon body drawn on a `w` x `h` grid. `None` when the body uses something
/// that cannot be turned into paths.
pub fn shapes(body: &str, w: f64, h: f64) -> Option<Vec<Shape>> {
    let toks = tokens(body)?;
    let mut stack = vec![Style { fill: "black".into(), stroke: "none".into(), stroke_width: 1.0, evenodd: false }];
    let mut defs = 0usize;
    let mut skip = 0usize;
    let mut ids: HashMap<String, (String, Attrs)> = HashMap::new();
    let mut out: Vec<Shape> = Vec::new();
    for t in &toks {
        match t {
            Tok::Open(name, a, selfclose) => {
                if skip > 0 {
                    if !selfclose {
                        skip += 1;
                    }
                    continue;
                }
                if attr(a, "transform").is_some() {
                    return None;
                }
                let name = name.as_str();
                match name {
                    "svg" | "g" => {
                        if !selfclose {
                            let s = stack.last()?.with(a);
                            stack.push(s);
                        }
                    }
                    "defs" => {
                        if !selfclose {
                            defs += 1;
                        }
                    }
                    // a clip that is the whole box (all the sets use) changes nothing
                    "clipPath" | "title" | "desc" | "style" => {
                        if !selfclose {
                            skip = 1;
                        }
                    }
                    _ if DRAWN.contains(&name) => {
                        if defs > 0 {
                            if let Some(id) = attr(a, "id") {
                                ids.insert(id.to_string(), (name.to_string(), a.clone()));
                            }
                        } else {
                            let st = stack.last()?.with(a);
                            emit(name, a, &st, w, h, &mut out)?;
                        }
                    }
                    "use" => {
                        if defs > 0 {
                            continue;
                        }
                        let href = attr(a, "href").or_else(|| attr(a, "xlink:href"))?;
                        let (tag, ra) = ids.get(href.strip_prefix('#')?)?;
                        if n(a, "x") != 0.0 || n(a, "y") != 0.0 || attr(ra, "transform").is_some() {
                            return None;
                        }
                        let st = stack.last()?.with(a).with(ra);
                        emit(tag, ra, &st, w, h, &mut out)?;
                    }
                    _ => return None,
                }
            }
            Tok::Close(name) => {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match name.as_str() {
                    "g" | "svg" => {
                        if stack.len() > 1 {
                            stack.pop();
                        }
                    }
                    "defs" => defs = defs.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn emit(tag: &str, a: &Attrs, st: &Style, w: f64, h: f64, out: &mut Vec<Shape>) -> Option<()> {
    let d = element_d(tag, a)?;
    let segs = path::parse(&d)?;
    let filled = st.fill != "none";
    let stroked = st.stroke != "none" && st.stroke_width > 0.0;
    if filled {
        let f = prepare_fill(&segs, st.evenodd);
        if !f.is_empty() {
            out.push(Shape { path: path::write(&f, w, h), stroke: None });
        }
    }
    if stroked && segs.iter().any(|g| !matches!(g, Seg::M(..) | Seg::Z)) {
        out.push(Shape { path: path::write(&segs, w, h), stroke: Some(st.stroke_width / w) });
    }
    Some(())
}

// ---- holes ----

type P = (f64, f64);

struct Sub {
    start: P,
    segs: Vec<Seg>,
}

fn end_of(g: &Seg, fallback: P) -> P {
    match *g {
        Seg::M(x, y) | Seg::L(x, y) | Seg::C(_, _, _, _, x, y) => (x, y),
        Seg::Z => fallback,
    }
}

fn subpaths(segs: &[Seg]) -> Vec<Sub> {
    let mut out: Vec<Sub> = Vec::new();
    for g in segs {
        match *g {
            Seg::M(x, y) => out.push(Sub { start: (x, y), segs: Vec::new() }),
            Seg::Z => {}
            _ => {
                if let Some(s) = out.last_mut() {
                    s.segs.push(*g);
                }
            }
        }
    }
    out.retain(|s| !s.segs.is_empty());
    out
}

/// The outline as a polyline (curves sampled).
pub fn flatten(start: P, segs: &[Seg]) -> Vec<P> {
    let mut pts = vec![start];
    let mut cur = start;
    for g in segs {
        match *g {
            Seg::C(a, b, c, d, x, y) => {
                for i in 1..=8 {
                    let t = i as f64 / 8.0;
                    let u = 1.0 - t;
                    pts.push((
                        u * u * u * cur.0 + 3.0 * u * u * t * a + 3.0 * u * t * t * c + t * t * t * x,
                        u * u * u * cur.1 + 3.0 * u * u * t * b + 3.0 * u * t * t * d + t * t * t * y,
                    ));
                }
                cur = (x, y);
            }
            Seg::L(x, y) | Seg::M(x, y) => {
                pts.push((x, y));
                cur = (x, y);
            }
            Seg::Z => {}
        }
    }
    pts
}

fn area(p: &[P]) -> f64 {
    let mut s = 0.0;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        s += a.0 * b.1 - b.0 * a.1;
    }
    s / 2.0
}

/// Even-odd point in polygon.
fn inside(pt: P, poly: &[P]) -> bool {
    let mut c = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > pt.1) != (b.1 > pt.1) && pt.0 < (b.0 - a.0) * (pt.1 - a.1) / (b.1 - a.1) + a.0 {
            c = !c;
        }
        j = i;
    }
    c
}

fn reverse(s: &Sub) -> Sub {
    // close explicitly, then walk the segments backwards with swapped controls
    let mut segs = s.segs.clone();
    let mut pts = vec![s.start];
    let mut cur = s.start;
    for g in &segs {
        cur = end_of(g, cur);
        pts.push(cur);
    }
    if (cur.0 - s.start.0).abs() > 1e-9 || (cur.1 - s.start.1).abs() > 1e-9 {
        segs.push(Seg::L(s.start.0, s.start.1));
        pts.push(s.start);
    }
    let mut out = Vec::new();
    for k in (0..segs.len()).rev() {
        let to = pts[k];
        out.push(match segs[k] {
            Seg::C(a, b, c, d, _, _) => Seg::C(c, d, a, b, to.0, to.1),
            _ => Seg::L(to.0, to.1),
        });
    }
    Sub { start: *pts.last().unwrap(), segs: out }
}

/// A filled path as closed subpaths. `evenodd`: wound so that the non-zero rule gives the
/// same picture (even nesting depth one way, odd the other).
fn prepare_fill(segs: &[Seg], evenodd: bool) -> Vec<Seg> {
    let mut subs = subpaths(segs);
    if evenodd && subs.len() > 1 {
        let polys: Vec<Vec<P>> = subs.iter().map(|s| flatten(s.start, &s.segs)).collect();
        for i in 0..subs.len() {
            let p = &polys[i];
            let samples: Vec<P> = (0..5).map(|k| p[(k * p.len() / 5).min(p.len() - 1)]).collect();
            let depth = (0..subs.len())
                .filter(|&j| j != i && polys[j].len() > 2)
                .filter(|&j| samples.iter().filter(|&&s| inside(s, &polys[j])).count() * 2 > samples.len())
                .count();
            let a = area(p);
            if a == 0.0 {
                continue;
            }
            let want_positive = depth % 2 == 0;
            if (a > 0.0) != want_positive {
                subs[i] = reverse(&subs[i]);
            }
        }
    }
    let mut out = Vec::new();
    for s in subs {
        out.push(Seg::M(s.start.0, s.start.1));
        out.extend(s.segs);
        out.push(Seg::Z);
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{icon, sets};

    /// Points of a shape's path, flattened per subpath (0..1 box).
    pub fn polys(path: &str) -> Vec<Vec<P>> {
        let segs = path::parse(path).unwrap();
        subpaths(&segs).iter().map(|s| flatten(s.start, &s.segs)).collect()
    }

    /// Non-zero winding number of `pt` over all subpaths.
    pub fn winding(path: &str, pt: P) -> i32 {
        let mut w = 0;
        for p in polys(path) {
            for i in 0..p.len() {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                let cross = (b.0 - a.0) * (pt.1 - a.1) - (pt.0 - a.0) * (b.1 - a.1);
                if a.1 <= pt.1 {
                    if b.1 > pt.1 && cross > 0.0 {
                        w += 1;
                    }
                } else if b.1 <= pt.1 && cross < 0.0 {
                    w -= 1;
                }
            }
        }
        w
    }

    fn signed_areas(path: &str) -> Vec<f64> {
        polys(path).iter().map(|p| area(p)).collect()
    }

    #[test]
    fn a_group_passes_its_stroke_to_its_paths() {
        let b = r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M1 1h4"/><path d="M1 3h4"/></g>"#;
        let s = shapes(b, 24.0, 24.0).unwrap();
        assert_eq!(s.len(), 2);
        assert!(s.iter().all(|x| x.stroke == Some(2.0 / 24.0)));
        assert!(s[0].path.starts_with("M "));
    }

    #[test]
    fn a_filled_path_among_stroked_ones_is_filled() {
        let b = r#"<g fill="none" stroke="currentColor" stroke-width="2"><path d="M1 1h4"/><path fill="currentColor" stroke="none" d="M1 5h4v4z"/><path fill="currentColor" d="M10 5h4v4z"/></g>"#;
        let s = shapes(b, 24.0, 24.0).unwrap();
        // stroked, filled, then filled + stroked (filled first)
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].stroke, Some(2.0 / 24.0));
        assert_eq!(s[1].stroke, None);
        assert!(s[1].path.ends_with('Z'));
        assert_eq!(s[2].stroke, None);
        assert_eq!(s[3].stroke, Some(2.0 / 24.0));
    }

    #[test]
    fn a_dot_is_kept_as_a_short_line() {
        let ic = icon("tabler:server").unwrap();
        assert_eq!(ic.shapes.len(), 1);
        assert_eq!(ic.shapes[0].stroke, Some(2.0 / 24.0));
        let p = polys(&ic.shapes[0].path);
        let dot = p.iter().any(|s| s.len() == 2 && {
            let d = ((s[1].0 - s[0].0).powi(2) + (s[1].1 - s[0].1).powi(2)).sqrt();
            d > 0.0 && d < 0.001
        });
        assert!(dot, "{}", ic.shapes[0].path);
    }

    #[test]
    fn the_whole_box_clip_is_ignored() {
        let set = &sets::all()[1];
        let mut seen = 0;
        // the clips are only in the -negative drawings
        for name in set.names() {
            let f = set.get(name).unwrap();
            if !f.body.contains("clip-path=") {
                continue;
            }
            seen += 1;
            // nothing comes from <defs> (the clip path is there): never more shapes than drawn elements
            let drawn = f.body.split("<defs>").map(|p| p.split_once("</defs>").map_or(p, |x| x.1)).collect::<String>();
            let n = drawn.matches("<path").count() + drawn.matches("<use").count();
            let s = shapes(f.body, f.w, f.h).map_or(0, |s| s.len());
            assert!(s <= n * 2, "{name}: {s} shapes from {n} elements");
        }
        assert!(seen > 100, "{seen}");
        // burn-negative: the box is a path of its own (filled), the rest inherit fill none
        assert_eq!(icon("healthicons:burn-negative").unwrap().shapes.len(), 1);
        assert!(!icon("healthicons:crutches-negative").unwrap().shapes.is_empty());
    }

    #[test]
    fn every_coordinate_is_inside_the_unit_box_give_or_take() {
        let mut bad = Vec::new();
        for set in sets::all() {
            for name in set.names() {
                let id = format!("{}:{}", set.id, name);
                let Some(ic) = icon(&id) else { continue };
                for sh in &ic.shapes {
                    for t in sh.path.split_whitespace() {
                        if let Ok(v) = t.parse::<f64>() {
                            if !(-0.05..=1.05).contains(&v) {
                                bad.push(id.clone());
                                break;
                            }
                        }
                    }
                }
            }
        }
        bad.dedup();
        assert!(bad.len() <= 5, "{} outside: {:?}", bad.len(), &bad[..bad.len().min(20)]);
    }

    fn square(x0: f64, x1: f64) -> String {
        format!("M{x0} {x0}H{x1}V{x1}H{x0}z")
    }

    #[test]
    fn a_ring_written_even_odd_is_wound_in_and_out() {
        let b = format!(r#"<path fill-rule="evenodd" d="{}{}"/>"#, square(1.0, 9.0), square(3.0, 7.0));
        let s = shapes(&b, 10.0, 10.0).unwrap();
        assert_eq!(s.len(), 1);
        let a = signed_areas(&s[0].path);
        assert_eq!(a.len(), 2);
        assert!(a[0] * a[1] < 0.0, "{a:?}");
        // as the non-zero rule reads it: the ring is in, the middle is out
        assert_ne!(winding(&s[0].path, (0.2, 0.2)), 0);
        assert_eq!(winding(&s[0].path, (0.5, 0.5)), 0);
        // the same two squares without evenodd keep their winding (both the same way)
        let b = format!(r#"<path d="{}{}"/>"#, square(1.0, 9.0), square(3.0, 7.0));
        let s = shapes(&b, 10.0, 10.0).unwrap();
        let a = signed_areas(&s[0].path);
        assert!(a[0] * a[1] > 0.0);
    }

    #[test]
    fn a_hole_in_a_hole_is_filled_again() {
        let b = format!(
            r#"<path fill-rule="evenodd" d="{}{}{}"/>"#,
            square(0.5, 9.5),
            square(2.0, 8.0),
            square(4.0, 6.0)
        );
        let s = shapes(&b, 10.0, 10.0).unwrap();
        let a = signed_areas(&s[0].path);
        assert!(a[0] * a[1] < 0.0 && a[1] * a[2] < 0.0 && a[0] * a[2] > 0.0, "{a:?}");
        assert_ne!(winding(&s[0].path, (0.1, 0.1)), 0);
        assert_eq!(winding(&s[0].path, (0.3, 0.3)), 0);
        assert_ne!(winding(&s[0].path, (0.5, 0.5)), 0);
    }

    #[test]
    fn health_icons_doctor_outline_keeps_its_face_open() {
        let ic = icon("healthicons:doctor-outline").unwrap();
        assert_eq!(ic.shapes.len(), 1);
        let p = &ic.shapes[0].path;
        assert_eq!(winding(p, (24.0 / 48.0, 16.0 / 48.0)), 0, "face must be open");
        // on the head ring (radius between 8 and 10 around (24,16))
        assert_ne!(winding(p, (24.0 / 48.0, 7.0 / 48.0)), 0, "ring must be filled");
        // inside the body
        assert_ne!(winding(p, (24.0 / 48.0, 41.0 / 48.0)), 0);
    }

    #[test]
    fn nearly_every_icon_of_both_sets_converts() {
        let mut none = Vec::new();
        let mut total = 0;
        for set in sets::all() {
            for name in set.names() {
                let id = format!("{}:{}", set.id, name);
                total += 1;
                match icon(&id) {
                    None => none.push(id),
                    Some(ic) => {
                        for sh in &ic.shapes {
                            assert!(sh.path.starts_with('M'), "{id}");
                            assert!(
                                sh.path.split_whitespace().all(|t| matches!(t, "M" | "L" | "C" | "Z") || t.parse::<f64>().is_ok()),
                                "{id}: {}",
                                sh.path
                            );
                        }
                    }
                }
            }
        }
        println!("{} of {} do not convert: {:?}", none.len(), total, none);
        assert!(none.len() <= 10, "{none:?}");
    }
}
