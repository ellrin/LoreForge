//! A small chart inside a flow figure (kind `chart`): a distribution, a few curves, a bar
//! chart, clusters, a surface. Drawn as plain canvas shapes (lines and filled paths) so it
//! stays vector on a slide; what the curves are comes from plain functions here, never from a
//! picture.

use std::collections::BTreeMap;

use lore_canvas::model::{Frame, Geom, Kind, Look, Object, Para, Paint, Run, SpObj, Stroke, TextBody};
use serde_json::Value;

/// The chart kinds a block may ask for (`chart=`).
pub const KINDS: [&str; 6] = ["bell", "bells", "membership", "bars", "scatter", "surface"];

/// Series colours of a document (Okabe–Ito, colour-blind safe: the plotting tool's first
/// palette). Kept here as plain data: `lore-plot` is a server crate (axum, tokio) and a figure
/// needs only these numbers.
pub const SERIES: [&str; 6] = ["0072B2", "E69F00", "009E73", "D55E00", "CC79A7", "56B4E9"];

const EMU: f64 = 12700.0;
const SAMPLES: usize = 48;
const LABEL_H: f64 = 8.0;
/// Highest a curve or bar goes, as a share of the plot height.
const TOP: f64 = 0.94;

fn emu(pt: f64) -> i64 {
    (pt * EMU).round() as i64
}

fn get_str(p: &BTreeMap<String, Value>, k: &str) -> Option<String> {
    match p.get(k)? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn get_num(p: &BTreeMap<String, Value>, k: &str, d: f64) -> f64 {
    match p.get(k) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(d),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(d),
        _ => d,
    }
}

fn get_flag(p: &BTreeMap<String, Value>, k: &str, d: bool) -> bool {
    match p.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.trim().to_lowercase().as_str(), "false" | "no" | "0" | "off"),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        _ => d,
    }
}

fn get_list(p: &BTreeMap<String, Value>, k: &str) -> Vec<Value> {
    match p.get(k) {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => {
            let t = s.trim().trim_start_matches('[').trim_end_matches(']');
            t.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).map(|x| Value::String(x.to_string())).collect()
        }
        _ => vec![],
    }
}

fn get_texts(p: &BTreeMap<String, Value>, k: &str) -> Vec<String> {
    get_list(p, k)
        .iter()
        .map(|v| match v {
            Value::String(s) => s.trim().trim_matches('"').to_string(),
            o => o.to_string(),
        })
        .collect()
}

/// The plot area inside the box, in pt.
#[derive(Clone, Copy)]
struct Area {
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Area {
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    /// Unit coordinates (y up) to box points.
    fn at(&self, u: f64, v: f64) -> (f64, f64) {
        (self.x0 + u * self.w(), self.y1 - v * self.h())
    }
}

type Pts = Vec<(f64, f64)>;

struct Ctx<'a> {
    w: f64,
    h: f64,
    ink: &'a str,
    out: Vec<Object>,
}

fn blank(frame: Frame, kind: Kind) -> Object {
    Object { id: 0, name: String::new(), frame, hidden: false, ph: None, slot: None, kind, src: None }
}

fn num4(v: f64) -> String {
    let v = if v.abs() < 5e-5 { 0.0 } else { v };
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn custom(d: String, fill: Paint, line: Option<Stroke>) -> SpObj {
    SpObj { geom: Geom { prst: "custom".into(), adj: Default::default(), path: Some(d) }, fill, line, text: None, equation: false }
}

fn round(mut st: Stroke) -> Stroke {
    st.cap = "round".into();
    st
}

impl Ctx<'_> {
    /// Sub-paths (each a polyline, optionally closed) as one custom shape whose frame is their
    /// bounding box.
    fn path(&mut self, subs: &[(Pts, bool)], fill: Paint, line: Option<Stroke>) {
        let all: Vec<(f64, f64)> = subs.iter().flat_map(|(p, _)| p.iter().copied()).collect();
        if all.len() < 2 {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &all {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let (ew, eh) = ((x1 - x0).max(1e-3), (y1 - y0).max(1e-3));
        let mut d = String::new();
        for (pts, closed) in subs {
            for (i, &(x, y)) in pts.iter().enumerate() {
                let u = ((x - x0) / ew).clamp(0.0, 1.0);
                let v = ((y - y0) / eh).clamp(0.0, 1.0);
                if !d.is_empty() {
                    d.push(' ');
                }
                d.push_str(&format!("{} {} {}", if i == 0 { "M" } else { "L" }, num4(u), num4(v)));
            }
            if *closed {
                d.push_str(" Z");
            }
        }
        let frame = Frame::new(emu(x0), emu(y0), emu(ew), emu(eh));
        self.out.push(blank(frame, Kind::Shape(custom(d, fill, line))));
    }

    fn curve(&mut self, pts: Pts, color: &str) {
        self.path(&[(pts, false)], Paint::None, Some(round(Stroke::new(color, 1.25))));
    }

    /// The area under a curve down to the baseline, in the series colour at 25 % alpha.
    fn shade(&mut self, curve: &Pts, base: f64, color: &str) {
        let (Some(first), Some(last)) = (curve.first().copied(), curve.last().copied()) else { return };
        let mut pts = curve.clone();
        pts.push((last.0, base));
        pts.push((first.0, base));
        self.path(&[(pts, true)], Paint::Solid { color: color.to_uppercase(), alpha: 0.25 }, None);
    }

    fn axes(&mut self, a: &Area) {
        let st = round(Stroke::new(self.ink, 0.75));
        self.path(&[(vec![(a.x0, a.y0), (a.x0, a.y1), (a.x1, a.y1)], false)], Paint::None, Some(st));
    }

    /// Small text centred in the cell `[x, x + cw]`, top at `y`.
    fn label(&mut self, text: &str, x: f64, cw: f64, y: f64) {
        let cw = cw.min(self.w);
        let x = x.clamp(0.0, (self.w - cw).max(0.0));
        let look = Look { size: 6.0, color: self.ink.to_uppercase(), ..Look::default() };
        let mut para = Para::new(vec![Run::text(text, "body", look)]);
        para.align = "ctr".into();
        let tb = TextBody { insets: [0; 4], anchor: "t".into(), wrap: false, paras: vec![para], ..TextBody::default() };
        let frame = Frame::new(emu(x), emu(y), emu(cw), emu(LABEL_H.min(self.h - y)));
        let sp = SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(tb), equation: false };
        self.out.push(blank(frame, Kind::Text(sp)));
    }

    /// One label per cell, `n` cells across the plot, under the x axis.
    fn labels(&mut self, names: &[String], a: &Area, n: usize) {
        if names.is_empty() || n == 0 {
            return;
        }
        let cw = a.w() / n as f64;
        for (i, t) in names.iter().take(n).enumerate() {
            self.label(t, a.x0 + i as f64 * cw, cw, a.y1 + 2.0);
        }
    }
}

fn sample<F: Fn(f64) -> f64>(f: F, a: &Area) -> Pts {
    (0..SAMPLES)
        .map(|i| {
            let u = i as f64 / (SAMPLES - 1) as f64;
            a.at(u, f(u) * TOP)
        })
        .collect()
}

fn gauss(x: f64, mu: f64, sigma: f64) -> f64 {
    (-0.5 * ((x - mu) / sigma).powi(2)).exp()
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// The shapes of one chart in a `w` × `h` box (pt, origin top-left), drawn in `ink` (`RRGGBB`)
/// with the series in the document palette. `params` are the block's (`chart`, `n`, `labels`,
/// …). Object ids are 0 and names `chart`, `chart-2` …. Empty = no such chart (the block then
/// draws a dashed slot and says so).
pub fn objects(params: &BTreeMap<String, Value>, w: f64, h: f64, ink: &str) -> Vec<Object> {
    let kind = get_str(params, "chart").unwrap_or_default().to_lowercase();
    if !KINDS.contains(&kind.as_str()) || w < 8.0 || h < 8.0 {
        return Vec::new();
    }
    let ink = ink.trim_start_matches('#').to_uppercase();
    let names = get_texts(params, "labels");
    let label_h = if names.is_empty() { 0.0 } else { LABEL_H + 1.0 };
    let a = Area { x0: 2.0, x1: w - 2.0, y0: 2.0, y1: (h - 2.0 - label_h).max(6.0) };
    let mut c = Ctx { w, h, ink: &ink, out: Vec::new() };
    let axes = get_flag(params, "axes", true);
    let n_of = |lo: usize, hi: usize, d: usize| (get_num(params, "n", d as f64).round().max(lo as f64) as usize).min(hi);
    let col = |i: usize| SERIES[i % SERIES.len()];

    match kind.as_str() {
        "bell" => {
            let (mu, sigma) = (get_num(params, "mu", 0.0), get_num(params, "sigma", 1.0).abs().max(1e-9));
            let (lo, hi) = (mu - 3.5 * sigma, mu + 3.5 * sigma);
            let curve = sample(|u| gauss(lo + u * (hi - lo), mu, sigma), &a);
            if get_flag(params, "fill", false) {
                c.shade(&curve, a.y1, col(0));
            }
            if axes {
                c.axes(&a);
            }
            c.curve(curve, col(0));
            c.labels(&names, &a, 1);
        }
        "bells" => {
            let n = n_of(2, 5, 2);
            // equal sigma, neighbours cross at half height: sigma = spacing / (2 sqrt(2 ln 2))
            let d = 0.6 / n as f64;
            let sigma = d / 2.355;
            let mid = |i: usize| 0.5 + (i as f64 - (n as f64 - 1.0) / 2.0) * d;
            let curves: Vec<Pts> = (0..n).map(|i| sample(|u| gauss(u, mid(i), sigma), &a)).collect();
            if axes {
                c.axes(&a);
            }
            for (i, cv) in curves.into_iter().enumerate() {
                c.curve(cv, col(i));
            }
            c.labels(&names, &a, n);
        }
        "membership" => {
            let n = n_of(2, 5, 3);
            let s = 1.0 / (n as f64 - 1.0);
            let k = 8.0 / s;
            let sigma = (s / 2.0) / 1.1774;
            let mut curves: Vec<Pts> = vec![sample(|u| sigmoid(-k * (u - s / 2.0)), &a)];
            for i in 1..n - 1 {
                curves.push(sample(|u| gauss(u, i as f64 * s, sigma), &a));
            }
            curves.push(sample(|u| sigmoid(k * (u - (1.0 - s / 2.0))), &a));
            if axes {
                c.axes(&a);
            }
            for (i, cv) in curves.into_iter().enumerate() {
                c.curve(cv, col(i));
            }
            c.labels(&names, &a, n);
        }
        "bars" => {
            let mut vals: Vec<f64> = get_list(params, "values")
                .iter()
                .filter_map(|v| match v {
                    Value::Number(n) => n.as_f64(),
                    Value::String(s) => s.trim().parse().ok(),
                    _ => None,
                })
                .map(|v| v.max(0.0))
                .take(12)
                .collect();
            if vals.is_empty() {
                vals = vec![0.5, 0.8, 0.6, 0.9, 0.4];
            }
            let top = vals.iter().copied().fold(1.0, f64::max);
            let each = get_str(params, "colors").is_some_and(|s| s.eq_ignore_ascii_case("each"));
            if axes {
                c.axes(&a);
            }
            let cw = a.w() / vals.len() as f64;
            for (i, v) in vals.iter().enumerate() {
                let bh = v / top * TOP * a.h();
                if bh < 1e-6 {
                    continue;
                }
                let bw = cw * 0.78;
                let x = a.x0 + i as f64 * cw + (cw - bw) / 2.0;
                let sp = SpObj { geom: Geom::preset("rect"), fill: Paint::solid(col(if each { i } else { 0 })), line: None, text: None, equation: false };
                c.out.push(blank(Frame::new(emu(x), emu(a.y1 - bh), emu(bw), emu(bh)), Kind::Shape(sp)));
            }
            c.labels(&names, &a, vals.len());
        }
        "scatter" => {
            let n = n_of(1, 4, 3);
            let centres = [(0.3, 0.68), (0.72, 0.62), (0.45, 0.25), (0.8, 0.25)];
            // a fixed seed: the same points on every run
            let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
            let mut rnd = move || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((seed >> 33) as f64 + 1.0) / ((1u64 << 31) as f64 + 1.0)
            };
            let (sigma, r) = (0.075, 1.3);
            let mut clusters: Vec<Pts> = Vec::new();
            for i in 0..n {
                let (cx, cy) = if n == 1 { (0.5, 0.5) } else { centres[i] };
                let mut pts = Vec::new();
                for _ in 0..12 {
                    let (u1, u2) = (rnd(), rnd());
                    let m = (-2.0 * u1.ln()).sqrt() * sigma;
                    let ang = std::f64::consts::TAU * u2;
                    let (x, y) = a.at((cx + m * ang.cos()).clamp(0.02, 0.98), (cy + m * ang.sin()).clamp(0.02, 0.98));
                    pts.push((x.clamp(a.x0 + r, a.x1 - r), y.clamp(a.y0 + r, a.y1 - r)));
                }
                clusters.push(pts);
            }
            if axes {
                c.axes(&a);
            }
            for (i, pts) in clusters.iter().enumerate() {
                // all points of a cluster as one path of small circles (four Bézier arcs each)
                let k = 0.5523 * r;
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &(x, y) in pts {
                    x0 = x0.min(x - r);
                    y0 = y0.min(y - r);
                    x1 = x1.max(x + r);
                    y1 = y1.max(y + r);
                }
                let (ew, eh) = (x1 - x0, y1 - y0);
                let u = |x: f64| num4(((x - x0) / ew).clamp(0.0, 1.0));
                let v = |y: f64| num4(((y - y0) / eh).clamp(0.0, 1.0));
                let mut d = String::new();
                for &(x, y) in pts {
                    if !d.is_empty() {
                        d.push(' ');
                    }
                    d.push_str(&format!(
                        "M {} {} C {} {} {} {} {} {} C {} {} {} {} {} {} C {} {} {} {} {} {} C {} {} {} {} {} {} Z",
                        u(x + r), v(y),
                        u(x + r), v(y + k), u(x + k), v(y + r), u(x), v(y + r),
                        u(x - k), v(y + r), u(x - r), v(y + k), u(x - r), v(y),
                        u(x - r), v(y - k), u(x - k), v(y - r), u(x), v(y - r),
                        u(x + k), v(y - r), u(x + r), v(y - k), u(x + r), v(y)
                    ));
                }
                c.out.push(blank(Frame::new(emu(x0), emu(y0), emu(ew), emu(eh)), Kind::Shape(custom(d, Paint::solid(col(i)), None))));
            }
            if get_flag(params, "ellipse", false) {
                for (i, pts) in clusters.iter().enumerate() {
                    let m = pts.len() as f64;
                    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / m, pts.iter().map(|p| p.1).sum::<f64>() / m);
                    let sx = (pts.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>() / m).sqrt();
                    let sy = (pts.iter().map(|p| (p.1 - my).powi(2)).sum::<f64>() / m).sqrt();
                    let (rx, ry) = ((2.0 * sx).max(3.0), (2.0 * sy).max(3.0));
                    let (x0, y0) = ((mx - rx).max(a.x0), (my - ry).max(a.y0));
                    let (x1, y1) = ((mx + rx).min(a.x1), (my + ry).min(a.y1));
                    let mut st = Stroke::new(col(i), 1.0);
                    st.dash = "sysDash".into();
                    let sp = SpObj { geom: Geom::preset("ellipse"), fill: Paint::None, line: Some(st), text: None, equation: false };
                    c.out.push(blank(Frame::new(emu(x0), emu(y0), emu(x1 - x0), emu(y1 - y0)), Kind::Shape(sp)));
                }
            }
            c.labels(&names, &a, n);
        }
        _ => {
            // surface: an isometric wireframe of a smooth plateau, back to front
            const G: usize = 6;
            let z = |i: f64, j: f64| {
                let (u, v) = (i / (G as f64 - 1.0) * 2.0 - 1.0, j / (G as f64 - 1.0) * 2.0 - 1.0);
                sigmoid(6.0 * (0.62 - (u * u + v * v).sqrt()))
            };
            let proj = |i: f64, j: f64, z: f64| ((i - j) * 0.866, (i + j) * 0.5 - z * 3.4);
            let rows: Vec<Pts> = (0..G).map(|j| (0..G).map(|i| proj(i as f64, j as f64, z(i as f64, j as f64))).collect()).collect();
            let cols: Vec<Pts> = (0..G).map(|i| (0..G).map(|j| proj(i as f64, j as f64, z(i as f64, j as f64))).collect()).collect();
            let mut axl: Vec<Pts> = Vec::new();
            if axes {
                let o = (-0.6, G as f64 - 0.4);
                axl.push(vec![proj(o.0, o.1, 0.0), proj(o.0 + 2.0, o.1, 0.0)]);
                axl.push(vec![proj(o.0, o.1, 0.0), proj(o.0, o.1 - 2.0, 0.0)]);
                axl.push(vec![proj(o.0, o.1, 0.0), proj(o.0, o.1, 1.0)]);
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &(x, y) in rows.iter().chain(cols.iter()).chain(axl.iter()).flatten() {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            let sc = (a.w() / (x1 - x0)).min(a.h() / (y1 - y0));
            let (ox, oy) = (a.x0 + (a.w() - (x1 - x0) * sc) / 2.0, a.y0 + (a.h() - (y1 - y0) * sc) / 2.0);
            let map = |p: &Pts| -> Pts { p.iter().map(|&(x, y)| (ox + (x - x0) * sc, oy + (y - y0) * sc)).collect() };
            let line = round(Stroke::new(col(0), 0.75));
            // far rows (small j) and far columns (small i) first
            for (r, cl) in rows.iter().zip(cols.iter()) {
                c.path(&[(map(r), false)], Paint::None, Some(line.clone()));
                c.path(&[(map(cl), false)], Paint::None, Some(line.clone()));
            }
            for l in &axl {
                let st = round(Stroke::new(c.ink, 0.75));
                c.path(&[(map(l), false)], Paint::None, Some(st));
            }
            c.labels(&names, &a, 1);
        }
    }
    let mut out = c.out;
    for (i, o) in out.iter_mut().enumerate() {
        o.name = if i == 0 { "chart".into() } else { format!("chart-{}", i + 1) };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(kv: &[(&str, Value)]) -> BTreeMap<String, Value> {
        kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    fn make(kv: &[(&str, Value)]) -> Vec<Object> {
        objects(&p(kv), 84.0, 56.0, "1D3B63")
    }
    fn sp(o: &Object) -> &SpObj {
        o.kind.sp().unwrap()
    }
    /// All number pairs of a path, in order.
    fn pts(o: &Object) -> Vec<(f64, f64)> {
        let d = sp(o).geom.path.clone().unwrap();
        let nums: Vec<f64> = d.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        nums.chunks(2).map(|c| (c[0], c[1])).collect()
    }
    /// The curves: round-capped lines 1.25 pt wide.
    fn curves(os: &[Object]) -> Vec<&Object> {
        os.iter().filter(|o| matches!(&o.kind, Kind::Shape(s) if s.line.as_ref().is_some_and(|l| l.width_pt == 1.25))).collect()
    }
    fn color(o: &Object) -> String {
        sp(o).line.as_ref().unwrap().color.clone()
    }
    fn fills(os: &[Object]) -> Vec<&Object> {
        os.iter().filter(|o| matches!(&o.kind, Kind::Shape(s) if matches!(s.fill, Paint::Solid { alpha, .. } if alpha < 1.0))).collect()
    }

    #[test]
    fn a_bell_is_one_curve_highest_at_its_mean() {
        let os = make(&[("chart", json!("bell"))]);
        let cs = curves(&os);
        assert_eq!(cs.len(), 1);
        let q = pts(cs[0]);
        let top = q.iter().cloned().fold((0.0, 2.0), |m, p| if p.1 < m.1 { p } else { m });
        assert!((top.0 - 0.5).abs() < 0.03, "peak at {}", top.0);
        assert!(q[0].1 > 0.95 && q[q.len() - 1].1 > 0.95, "tails reach the baseline");
        assert!(sp(cs[0]).line.as_ref().unwrap().cap == "round");
        assert!(fills(&os).is_empty());
    }

    #[test]
    fn a_filled_bell_has_a_shape_under_the_curve() {
        let os = make(&[("chart", json!("Bell")), ("fill", json!(true))]);
        let f = fills(&os);
        assert_eq!(f.len(), 1);
        assert!(sp(f[0]).line.is_none());
        assert!(sp(f[0]).geom.path.as_ref().unwrap().ends_with('Z'));
        // the fill comes before the curve
        let (fi, ci) = (os.iter().position(|o| o == f[0]).unwrap(), os.iter().position(|o| o == curves(&os)[0]).unwrap());
        assert!(fi < ci);
    }

    #[test]
    fn bells_overlap_and_each_has_its_colour() {
        let os = make(&[("chart", json!("bells")), ("n", json!(3))]);
        let cs = curves(&os);
        assert_eq!(cs.len(), 3);
        let cols: std::collections::BTreeSet<String> = cs.iter().map(|o| color(o)).collect();
        assert_eq!(cols.len(), 3);
        for pair in cs.windows(2) {
            let (a, b) = (&pair[0].frame, &pair[1].frame);
            assert!(a.x + a.w > b.x, "neighbouring bells overlap");
        }
        assert_eq!(make(&[("chart", json!("bells"))]).iter().filter(|o| curves(std::slice::from_ref(o)).len() == 1).count(), 2);
    }

    #[test]
    fn membership_first_falls_last_rises() {
        let os = make(&[("chart", json!("membership")), ("n", json!(3))]);
        let cs = curves(&os);
        assert_eq!(cs.len(), 3);
        let (f, m, l) = (pts(cs[0]), pts(cs[1]), pts(cs[2]));
        assert!(f[0].1 < 0.05 && f[f.len() - 1].1 > 0.95, "first: high then low");
        assert!(l[0].1 > 0.95 && l[l.len() - 1].1 < 0.05, "last: low then high");
        let top = m.iter().cloned().fold((0.0, 2.0), |a, p| if p.1 < a.1 { p } else { a });
        assert!((top.0 - 0.5).abs() < 0.05, "the middle one is a bell");
    }

    #[test]
    fn bars_follow_their_values() {
        let os = make(&[("chart", json!("bars")), ("values", json!([0.25, 0.5, 1.0]))]);
        let bars: Vec<&Object> = os.iter().filter(|o| sp(o).geom.prst == "rect").collect();
        assert_eq!(bars.len(), 3);
        let r = bars[2].frame.h as f64 / bars[0].frame.h as f64;
        assert!((r - 4.0).abs() < 0.01, "ratio {r}");
        let bottoms: Vec<i64> = bars.iter().map(|b| b.frame.y + b.frame.h).collect();
        assert!(bottoms.windows(2).all(|w| w[0] == w[1]), "all stand on the axis");
        let one: std::collections::BTreeSet<String> = bars.iter().map(|b| format!("{:?}", sp(b).fill)).collect();
        assert_eq!(one.len(), 1);
        let each = make(&[("chart", json!("bars")), ("colors", json!("each"))]);
        let n: std::collections::BTreeSet<String> = each.iter().filter(|o| sp(o).geom.prst == "rect").map(|b| format!("{:?}", sp(b).fill)).collect();
        assert_eq!(n.len(), 5);
    }

    #[test]
    fn scatter_is_the_same_every_time() {
        let a = make(&[("chart", json!("scatter")), ("ellipse", json!(true))]);
        let b = make(&[("chart", json!("scatter")), ("ellipse", json!(true))]);
        assert_eq!(a, b);
        // axes + 3 clusters + 3 outlines
        assert_eq!(a.len(), 7);
        let dots = a.iter().filter(|o| sp(o).geom.path.as_ref().is_some_and(|d| d.matches('M').count() == 12)).count();
        assert_eq!(dots, 3);
    }

    #[test]
    fn a_surface_is_lines_only() {
        let os = make(&[("chart", json!("surface"))]);
        assert!(os.len() >= 12 && os.len() < 40);
        for o in &os {
            let s = sp(o);
            assert!(s.fill.is_none());
            assert!(s.line.is_some());
            assert_eq!(s.geom.prst, "custom");
        }
        let bare = make(&[("chart", json!("surface")), ("axes", json!(false))]);
        assert_eq!(bare.len(), 12);
    }

    #[test]
    fn every_chart_stays_in_its_box() {
        for (w, h) in [(84.0, 56.0), (40.0, 30.0), (200.0, 120.0)] {
            for k in KINDS {
                for extra in [json!(false), json!(true)] {
                    let params = p(&[
                        ("chart", json!(k)),
                        ("fill", extra.clone()),
                        ("ellipse", extra.clone()),
                        ("n", json!(5)),
                        ("labels", json!(["alpha", "beta", "gamma", "delta", "eps"])),
                    ]);
                    let os = objects(&params, w, h, "000000");
                    assert!(!os.is_empty(), "{k}");
                    let (we, he) = (emu(w), emu(h));
                    for o in &os {
                        let f = &o.frame;
                        assert!(f.x >= 0 && f.y >= 0 && f.x + f.w <= we && f.y + f.h <= he, "{k} {} {:?}", o.name, f);
                        if let Some(d) = sp(o).geom.path.as_ref() {
                            for t in d.split_whitespace().filter_map(|t| t.parse::<f64>().ok()) {
                                assert!((-1e-6..=1.0 + 1e-6).contains(&t), "{k} coordinate {t}");
                            }
                        }
                    }
                    assert!(os.len() <= 45, "{k} has {} objects", os.len());
                }
            }
        }
    }

    #[test]
    fn axes_can_be_left_out() {
        for k in KINDS {
            let with = make(&[("chart", json!(k))]);
            let without = make(&[("chart", json!(k)), ("axes", json!(false))]);
            assert!(without.len() < with.len(), "{k}");
        }
        let os = make(&[("chart", json!("bell")), ("axes", json!("false"))]);
        assert_eq!(os.len(), 1);
        assert!(os.iter().all(|o| sp(o).line.as_ref().unwrap().width_pt != 0.75));
    }

    #[test]
    fn labels_are_written() {
        let os = make(&[("chart", json!("bells")), ("labels", json!(["low", "high"]))]);
        let texts: Vec<(String, f64)> = os
            .iter()
            .filter_map(|o| match &o.kind {
                Kind::Text(s) => {
                    let t = s.text.as_ref().unwrap();
                    Some((t.plain_text(), t.paras[0].runs[0].look.size))
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec![("low".to_string(), 6.0), ("high".to_string(), 6.0)]);
        let ax = os.iter().find(|o| sp(o).line.as_ref().is_some_and(|l| l.width_pt == 0.75)).unwrap();
        let lab = os.iter().find(|o| matches!(o.kind, Kind::Text(_))).unwrap();
        assert!(lab.frame.y >= ax.frame.y + ax.frame.h, "labels sit under the x axis");
        // the labels may also be written as a bracket list in a string
        assert_eq!(make(&[("chart", json!("bell")), ("labels", json!("[x]"))]).iter().filter(|o| matches!(o.kind, Kind::Text(_))).count(), 1);
    }

    #[test]
    fn an_unknown_chart_draws_nothing() {
        assert!(make(&[("chart", json!("pie"))]).is_empty());
        assert!(make(&[]).is_empty());
        assert!(make(&[("chart", json!(3))]).is_empty());
    }

    #[test]
    fn names_count_up_from_chart() {
        let os = make(&[("chart", json!("bells")), ("n", json!(3)), ("labels", json!(["a", "b", "c"]))]);
        assert_eq!(os[0].name, "chart");
        for (i, o) in os.iter().enumerate().skip(1) {
            assert_eq!(o.name, format!("chart-{}", i + 1));
            assert_eq!(o.id, 0);
        }
    }
}
