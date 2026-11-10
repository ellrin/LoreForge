//! A plain SVG drawing of canvas objects, to look at a figure without the
//! app or PowerPoint. It draws from the same objects that are exported
//! (preset and custom shapes, text runs, connectors with their flips,
//! rotation and adjust values, groups), so a wrong connector shows here too.
//! Math is drawn with the Unicode approximation of [`crate::text`].

use lore_canvas::{Frame, Kind, Object, Paint, SpObj, Stroke, TextBody};

use crate::build::connector_path;
use crate::modules::EMU_PT;
use crate::text::latex_pieces;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn n(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn fill_attr(p: &Paint) -> String {
    match p {
        Paint::Solid { color, alpha } if *alpha < 0.999 => format!("fill=\"#{color}\" fill-opacity=\"{}\"", n(*alpha)),
        Paint::Solid { color, .. } => format!("fill=\"#{color}\""),
        Paint::Gradient { stops, .. } => format!("fill=\"#{}\"", stops.first().map(|s| s.color.as_str()).unwrap_or("CCCCCC")),
        _ => "fill=\"none\"".into(),
    }
}

fn dash_attr(dash: &str, w: f64) -> String {
    let pat: &[f64] = match dash {
        "dash" => &[4.0, 3.0],
        "sysDash" => &[3.0, 1.0],
        "sysDot" | "dot" => &[1.0, 1.0],
        "lgDash" => &[8.0, 3.0],
        "dashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        _ => return String::new(),
    };
    format!(" stroke-dasharray=\"{}\"", pat.iter().map(|v| n(v * w)).collect::<Vec<_>>().join(" "))
}

fn stroke_attr(s: &Option<Stroke>) -> String {
    match s {
        Some(s) => format!("stroke=\"#{}\" stroke-width=\"{}\"{}{}", s.color, n(s.width_pt), dash_attr(&s.dash, s.width_pt), if s.alpha < 0.999 { format!(" stroke-opacity=\"{}\"", n(s.alpha)) } else { String::new() }),
        None => "stroke=\"none\"".into(),
    }
}

struct Xf {
    /// group → page: scale then offset (pt)
    sx: f64,
    sy: f64,
    ox: f64,
    oy: f64,
}

impl Xf {
    fn rect(&self, f: &Frame) -> (f64, f64, f64, f64) {
        (self.ox + f.x as f64 / EMU_PT * self.sx, self.oy + f.y as f64 / EMU_PT * self.sy, f.w as f64 / EMU_PT * self.sx, f.h as f64 / EMU_PT * self.sy)
    }
    fn pt(&self, p: (f64, f64)) -> (f64, f64) {
        (self.ox + p.0 / EMU_PT * self.sx, self.oy + p.1 / EMU_PT * self.sy)
    }
}

fn arrow(out: &mut String, tip: (f64, f64), from: (f64, f64), kind: &str, s: &Stroke) {
    if kind == "none" {
        return;
    }
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let size = 3.0 * s.width_pt.max(1.6);
    let (bx, by) = (tip.0 - ux * size, tip.1 - uy * size);
    let (px, py) = (-uy * size / 2.0, ux * size / 2.0);
    match kind {
        "oval" => out.push_str(&format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#{}\"/>", n(tip.0), n(tip.1), n(size / 2.0), s.color)),
        "arrow" => out.push_str(&format!("<polyline points=\"{},{} {},{} {},{}\" fill=\"none\" stroke=\"#{}\" stroke-width=\"{}\"/>", n(bx + px), n(by + py), n(tip.0), n(tip.1), n(bx - px), n(by - py), s.color, n(s.width_pt))),
        _ => out.push_str(&format!("<polygon points=\"{},{} {},{} {},{}\" fill=\"#{}\"/>", n(tip.0), n(tip.1), n(bx + px), n(by + py), n(bx - px), n(by - py), s.color)),
    }
}

fn polyline(out: &mut String, pts: &[(f64, f64)], s: &Stroke) {
    if pts.len() < 2 {
        return;
    }
    // stop the line inside the arrowhead so thick lines do not poke through the tip
    let mut p: Vec<(f64, f64)> = pts.to_vec();
    let size = 3.0 * s.width_pt.max(1.6);
    let shorten = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (a.0 - b.0, a.1 - b.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len > size { (a.0 - dx / len * size * 0.6, a.1 - dy / len * size * 0.6) } else { a }
    };
    let last = p.len() - 1;
    if s.tail != "none" && s.tail != "arrow" {
        p[last] = shorten(p[last], p[last - 1]);
    }
    if s.head != "none" && s.head != "arrow" {
        p[0] = shorten(p[0], p[1]);
    }
    out.push_str(&format!("<polyline points=\"{}\" fill=\"none\" {} stroke-linejoin=\"round\"/>", p.iter().map(|q| format!("{},{}", n(q.0), n(q.1))).collect::<Vec<_>>().join(" "), stroke_attr(&Some(s.clone()))));
    arrow(out, pts[last], pts[last - 1], &s.tail, s);
    arrow(out, pts[0], pts[1], &s.head, s);
}

fn text_svg(out: &mut String, tb: &TextBody, x: f64, y: f64, w: f64, h: f64, scale: f64) {
    let vert = tb.vert.as_deref();
    // a rotated body is laid out in a box of swapped sides and turned about the centre
    let (bw, bh) = if matches!(vert, Some("vert270") | Some("vert")) { (h, w) } else { (w, h) };
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (bx, by) = (cx - bw / 2.0, cy - bh / 2.0);
    let sizes: Vec<f64> = tb.paras.iter().map(|p| p.runs.iter().map(|r| r.look.size).fold(p.end.as_ref().map(|e| e.size).unwrap_or(0.0), f64::max) * scale).collect();
    let total: f64 = sizes.iter().map(|s| s * 1.2).sum();
    let ins: Vec<f64> = tb.insets.iter().map(|v| *v as f64 / EMU_PT * scale).collect();
    let mut ty = match tb.anchor.as_str() {
        "ctr" => by + (bh - total) / 2.0,
        "b" => by + bh - total - ins[3],
        _ => by + ins[1],
    };
    let rot = match vert {
        Some("vert270") => format!(" transform=\"rotate(-90 {} {})\"", n(cx), n(cy)),
        Some("vert") => format!(" transform=\"rotate(90 {} {})\"", n(cx), n(cy)),
        _ => String::new(),
    };
    for (p, size) in tb.paras.iter().zip(&sizes) {
        let (anchor, tx) = match p.align.as_str() {
            "ctr" => ("middle", bx + bw / 2.0),
            "r" => ("end", bx + bw - ins[2]),
            _ => ("start", bx + ins[0]),
        };
        let base = ty + size * 0.92;
        ty += size * 1.2;
        if p.runs.is_empty() {
            continue;
        }
        out.push_str(&format!("<text x=\"{}\" y=\"{}\" text-anchor=\"{anchor}\"{rot}>", n(tx), n(base)));
        for r in &p.runs {
            let fs = r.look.size * scale;
            let mut span = |text: &str, italic: bool, bold: bool, script: i8, font: &str| {
                let (shift, k) = match script {
                    1 => (-0.38, 0.68),
                    -1 => (0.2, 0.68),
                    _ => (0.0, 1.0),
                };
                out.push_str(&format!(
                    "<tspan font-family=\"{}\" font-size=\"{}\"{}{} fill=\"#{}\"{}>{}</tspan>",
                    esc(font),
                    n(fs * k),
                    if italic { " font-style=\"italic\"" } else { "" },
                    if bold { " font-weight=\"bold\"" } else { "" },
                    r.look.color,
                    if shift != 0.0 { format!(" dy=\"{}\"", n(fs * shift)) } else { String::new() },
                    esc(text)
                ));
                if shift != 0.0 {
                    // come back to the baseline
                    out.push_str(&format!("<tspan dy=\"{}\" font-size=\"0.1\">\u{200B}</tspan>", n(-fs * shift)));
                }
            };
            let sans = format!("{}, Carlito, 'Helvetica Neue', Arial, 'PingFang TC', sans-serif", r.look.font);
            match r.kind.as_str() {
                "math" => {
                    let math = "'Cambria Math', 'STIX Two Math', 'STIX Two Text', 'Times New Roman', serif";
                    for pc in latex_pieces(r.latex.as_deref().unwrap_or(&r.text)) {
                        span(&pc.text, pc.italic, pc.bold || r.look.bold, pc.script, math);
                    }
                }
                "br" => {}
                _ => {
                    let script = if r.look.baseline > 0.0 {
                        1
                    } else if r.look.baseline < 0.0 {
                        -1
                    } else {
                        0
                    };
                    let font = if r.look.italic || script != 0 { "'STIX Two Text', 'Times New Roman', serif".to_string() } else { sans.clone() };
                    span(&r.text, r.look.italic, r.look.bold, script, &font);
                }
            }
        }
        out.push_str("</text>");
    }
}

fn preset_path(prst: &str, sp: &SpObj, x: f64, y: f64, w: f64, h: f64) -> String {
    let adj = |k: &str, d: f64| sp.geom.adj.get(k).copied().unwrap_or(d) / 100_000.0;
    let pts = |v: &[(f64, f64)]| format!("M{} Z", v.iter().map(|p| format!("{} {}", n(x + p.0 * w), n(y + p.1 * h))).collect::<Vec<_>>().join(" L"));
    match prst {
        "trapezoid" => {
            let a = adj("adj", 25_000.0) * w.min(h) / w;
            pts(&[(a, 0.0), (1.0 - a, 0.0), (1.0, 1.0), (0.0, 1.0)])
        }
        "parallelogram" => {
            let a = adj("adj", 25_000.0) * w.min(h) / w;
            pts(&[(a, 0.0), (1.0, 0.0), (1.0 - a, 1.0), (0.0, 1.0)])
        }
        "diamond" => pts(&[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
        "hexagon" => {
            let a = adj("adj", 25_000.0) * w.min(h) / w;
            pts(&[(a, 0.0), (1.0 - a, 0.0), (1.0, 0.5), (1.0 - a, 1.0), (a, 1.0), (0.0, 0.5)])
        }
        "triangle" => pts(&[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        _ => pts(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
    }
}

fn draw(out: &mut String, o: &Object, xf: &Xf) {
    if o.hidden {
        return;
    }
    let (x, y, w, h) = xf.rect(&o.frame);
    let f = &o.frame;
    let mut tf = String::new();
    if f.rot != 0.0 && !matches!(o.kind, Kind::Connector(_)) {
        tf.push_str(&format!(" transform=\"rotate({} {} {})\"", n(f.rot), n(x + w / 2.0), n(y + h / 2.0)));
    }
    match &o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            out.push_str(&format!("<g{tf}>"));
            let paint = format!("{} {}", fill_attr(&sp.fill), stroke_attr(&sp.line));
            if !(sp.fill.is_none() && sp.line.is_none()) {
                match sp.geom.prst.as_str() {
                    "custom" => {
                        let path = sp.geom.path.clone().unwrap_or_default();
                        let closed = path.trim_end().ends_with(['Z', 'z']);
                        let pts = lore_canvas::geom::path_vertices(&path);
                        let abs: Vec<(f64, f64)> = pts.iter().map(|(u, v)| (x + if f.flip_h { 1.0 - u } else { *u } * w, y + if f.flip_v { 1.0 - v } else { *v } * h)).collect();
                        if closed {
                            out.push_str(&format!("<polygon points=\"{}\" {paint} stroke-linejoin=\"round\"/>", abs.iter().map(|p| format!("{},{}", n(p.0), n(p.1))).collect::<Vec<_>>().join(" ")));
                        } else if let Some(s) = &sp.line {
                            polyline(out, &abs, s);
                        }
                    }
                    "ellipse" => out.push_str(&format!("<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" {paint}/>", n(x + w / 2.0), n(y + h / 2.0), n(w / 2.0), n(h / 2.0))),
                    "rect" => out.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {paint}/>", n(x), n(y), n(w), n(h))),
                    "roundRect" => {
                        let r = sp.geom.adj.get("adj").copied().unwrap_or(16_667.0) / 100_000.0 * w.min(h);
                        out.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" {paint}/>", n(x), n(y), n(w), n(h), n(r)));
                    }
                    other => out.push_str(&format!("<path d=\"{}\" {paint} stroke-linejoin=\"round\"/>", preset_path(other, sp, x, y, w, h))),
                }
            }
            if let Some(tb) = &sp.text {
                text_svg(out, tb, x, y, w, h, xf.sx.min(xf.sy));
            }
            out.push_str("</g>");
        }
        Kind::Connector(c) => {
            if let Some(s) = &c.line {
                let pts: Vec<(f64, f64)> = connector_path(&c.geom, &c.adj, f).into_iter().map(|p| xf.pt(p)).collect();
                polyline(out, &pts, s);
            }
        }
        Kind::Group(g) => {
            let sx = if g.ch.w == 0 { 1.0 } else { f.w as f64 / g.ch.w as f64 };
            let sy = if g.ch.h == 0 { 1.0 } else { f.h as f64 / g.ch.h as f64 };
            let inner = Xf { sx: xf.sx * sx, sy: xf.sy * sy, ox: x - g.ch.x as f64 / EMU_PT * xf.sx * sx, oy: y - g.ch.y as f64 / EMU_PT * xf.sy * sy };
            out.push_str(&format!("<g{tf}>"));
            for ch in &g.children {
                draw(out, ch, &inner);
            }
            out.push_str("</g>");
        }
        Kind::Picture(_) => out.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#DDDDDD\" stroke=\"#999999\"/>", n(x), n(y), n(w), n(h))),
        _ => {}
    }
}

/// Draw objects on a page of `w` × `h` pt (white paper). `zoom` = CSS px per pt.
pub fn svg(objects: &[Object], w: f64, h: f64, zoom: f64) -> String {
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\"><rect width=\"100%\" height=\"100%\" fill=\"#FFFFFF\"/>",
        n(w),
        n(h),
        n(w * zoom),
        n(h * zoom)
    );
    let xf = Xf { sx: 1.0, sy: 1.0, ox: 0.0, oy: 0.0 };
    for o in objects {
        draw(&mut out, o, &xf);
    }
    out.push_str("</svg>\n");
    out
}
