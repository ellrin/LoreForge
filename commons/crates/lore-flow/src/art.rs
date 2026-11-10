//! Art styles: the constructions a module kind can be drawn as.
//!
//! A style here is a different drawing (SegNet slabs, a transformer block, an
//! unrolled RNN …), recognisable as a thumbnail. How it is painted (fill,
//! outline, gradient, colour by layer type, radius, line weight) is the
//! [`Finish`], which every style accepts.
//!
//! Every part is a native shape, text box or line. Coordinates in this file
//! are natural sizes in pt; the [`Pen`] scales them.

use std::collections::BTreeMap;

use lore_canvas::{Geom, Kind, Object, Paint, SpObj, Stroke};
use serde::{Deserialize, Serialize};

use crate::finish::Finish;
use crate::modules::{Core, Ctx, LabelPos, frame, obj, poly, round_adj, seg, shape};
use crate::spec::Node;
use crate::text::{self, Type};
use crate::theme::shade;

/// One entry of the colour legend: a layer type and its colour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub key: String,
    pub label: String,
    pub color: String,
}

fn lum(h: &str) -> f64 {
    (0..3).map(|i| u8::from_str_radix(h.get(i * 2..i * 2 + 2).unwrap_or("00"), 16).unwrap_or(0) as f64).sum::<f64>() / 765.0
}

/// Draws parts in natural coordinates, painted by the finish.
pub(crate) struct Pen<'c, 'a> {
    pub cx: &'c Ctx<'a>,
    pub f: Finish,
    pub objs: Vec<Object>,
    pub legend: Vec<Swatch>,
    pub anchors: Vec<(String, (f64, f64))>,
    /// The role colour this drawing takes in a matched pair (`enc`, `dec`).
    pub role: Option<String>,
}

impl<'c, 'a> Pen<'c, 'a> {
    pub fn new(n: &Node, cx: &'c Ctx<'a>) -> Self {
        Pen { cx, f: Finish::of(n, cx.theme), objs: Vec::new(), legend: Vec::new(), anchors: Vec::new(), role: n.str("_role").map(str::to_string) }
    }
    fn k(&self) -> f64 {
        self.cx.k
    }
    /// Colour of a layer type (and a legend entry) when the finish colours by type.
    pub fn col(&mut self, key: &str) -> String {
        if !self.f.typed {
            return self.f.base();
        }
        let c = self.cx.theme.layer(key).to_string();
        if !self.legend.iter().any(|s| s.key == key) {
            self.legend.push(Swatch { key: key.into(), label: self.cx.theme.layer_label(key), color: c.clone() });
        }
        c
    }
    /// The drawing's own colour: its role colour in a pair, else the module's.
    pub fn main(&mut self) -> String {
        match self.role.clone() {
            Some(r) => self.col(&r),
            None => self.f.base(),
        }
    }
    /// A colour that shows as a line on paper.
    pub fn ink_of(&self, col: &str) -> String {
        if lum(col) > 0.72 { shade(col, -0.5) } else { col.to_string() }
    }
    pub fn anchor(&mut self, name: &str, p: (f64, f64)) {
        let k = self.k();
        self.anchors.push((name.into(), (p.0 * k, p.1 * k)));
    }
    fn push(&mut self, o: Object) -> usize {
        self.objs.push(o);
        self.objs.len() - 1
    }
    /// A rectangular part (rounded by the finish's radius).
    pub fn rect(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64, col: &str) -> usize {
        let k = self.k();
        let (p, s) = self.f.part(col, k);
        let r = self.f.radius;
        let mut o = shape(name, if r > 0.0 { "roundRect" } else { "rect" }, x * k, y * k, w * k, h * k, p, s);
        if r > 0.0 {
            round_adj(&mut o, r);
        }
        self.push(o)
    }
    /// A cell of a grid or vector: never rounded, thin outline.
    pub fn cell(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64, col: &str) -> usize {
        let k = self.k();
        let (p, s) = self.f.part(col, k);
        let s = s.map(|mut s| {
            s.width_pt = s.width_pt.min(0.5 * k);
            s
        });
        self.push(shape(name, "rect", x * k, y * k, w * k, h * k, p, s))
    }
    pub fn ellipse(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64, col: &str) -> usize {
        let k = self.k();
        let (p, s) = self.f.part(col, k);
        self.push(shape(name, "ellipse", x * k, y * k, w * k, h * k, p, s))
    }
    pub fn polygon(&mut self, name: &str, pts: &[(f64, f64)], col: &str) -> usize {
        let k = self.k();
        let (p, s) = self.f.part(col, k);
        self.push(poly(name, &pts.iter().map(|q| (q.0 * k, q.1 * k)).collect::<Vec<_>>(), true, p, s))
    }
    fn face(&mut self, name: &str, pts: &[(f64, f64)], col: &str, t: f64) -> usize {
        let k = self.k();
        let (p, s) = self.f.face(col, t, k);
        self.push(poly(name, &pts.iter().map(|q| (q.0 * k, q.1 * k)).collect::<Vec<_>>(), true, p, s))
    }
    /// A 3-D block: front face `t × h` at (x, y), `d` deep towards the upper right.
    pub fn cuboid(&mut self, name: &str, x: f64, y: f64, t: f64, h: f64, d: f64, col: &str) {
        let (dx, dy) = (d * 0.5, d * 0.42);
        let sh = self.cx.theme.shade.clone();
        self.face(&format!("{name}-top"), &[(x, y), (x + dx, y - dy), (x + t + dx, y - dy), (x + t, y)], col, sh.top);
        self.face(&format!("{name}-side"), &[(x + t, y), (x + t + dx, y - dy), (x + t + dx, y + h - dy), (x + t, y + h)], col, sh.side);
        let k = self.k();
        let (p, s) = self.f.face(col, 0.0, k);
        self.push(shape(&format!("{name}-front"), "rect", x * k, y * k, t * k, h * k, p, s));
    }
    /// A sheet seen at an angle: left edge `skew` lower than the right edge.
    pub fn plane(&mut self, name: &str, x: f64, y: f64, w: f64, h: f64, skew: f64, col: &str) -> usize {
        self.polygon(name, &[(x, y + skew), (x + w, y), (x + w, y + h - skew), (x, y + h)], col)
    }
    pub fn line(&mut self, name: &str, a: (f64, f64), b: (f64, f64), col: &str, w: f64, dash: &str, tail: &str) -> usize {
        let k = self.k();
        let mut s = Stroke::new(col, w * k);
        s.dash = dash.into();
        s.tail = tail.into();
        self.push(seg(name, (a.0 * k, a.1 * k), (b.0 * k, b.1 * k), s))
    }
    /// A thin ink arrow.
    pub fn arrow(&mut self, name: &str, a: (f64, f64), b: (f64, f64)) -> usize {
        let c = self.f.ink.clone();
        self.line(name, a, b, &c, 0.75, "solid", "triangle")
    }
    pub fn wire(&mut self, name: &str, a: (f64, f64), b: (f64, f64)) -> usize {
        let c = self.f.muted.clone();
        self.line(name, a, b, &c, 0.4, "solid", "none")
    }
    /// An open polyline (elbows), arrowhead at the end.
    pub fn path(&mut self, name: &str, pts: &[(f64, f64)], col: &str, w: f64, dash: &str, tail: &str) -> usize {
        let k = self.k();
        let mut s = Stroke::new(col, w * k);
        s.dash = dash.into();
        s.tail = tail.into();
        self.push(poly(name, &pts.iter().map(|q| (q.0 * k, q.1 * k)).collect::<Vec<_>>(), false, Paint::None, Some(s)))
    }
    /// A quadratic arc from `a` to `b` bent through control point `c`.
    pub fn arc(&mut self, name: &str, a: (f64, f64), c: (f64, f64), b: (f64, f64), col: &str, w: f64, dash: &str, tail: &str) -> usize {
        let k = self.k();
        let at = |t: f64| ((1.0 - t) * (1.0 - t) * a.0 + 2.0 * (1.0 - t) * t * c.0 + t * t * b.0, (1.0 - t) * (1.0 - t) * a.1 + 2.0 * (1.0 - t) * t * c.1 + t * t * b.1);
        let pts: Vec<(f64, f64)> = (0..=20).map(|i| at(i as f64 / 20.0)).collect();
        let x0 = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        let x1 = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        let y0 = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let y1 = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let (bw, bh) = ((x1 - x0).max(0.01), (y1 - y0).max(0.01));
        let u = |p: (f64, f64)| format!("{:.4} {:.4}", (p.0 - x0) / bw, (p.1 - y0) / bh);
        let path = format!("M{} Q{} {}", u(a), u(c), u(b));
        let mut s = Stroke::new(col, w * k);
        s.dash = dash.into();
        s.tail = tail.into();
        let geom = Geom { prst: "custom".into(), adj: BTreeMap::new(), path: Some(path) };
        self.push(obj(name, frame(x0 * k, y0 * k, bw * k, bh * k), Kind::Shape(SpObj { geom, fill: Paint::None, line: Some(s), text: None, equation: false })))
    }
    /// Type sizes: the node label, small (captions) and tiny (text inside parts).
    pub fn ty_label(&self) -> Type {
        self.cx.ty(1.0, false, &self.f.text)
    }
    pub fn ty_small(&self) -> Type {
        self.cx.small(&self.f.ink)
    }
    pub fn ty_tiny(&self) -> Type {
        let mut t = self.cx.small(&self.f.ink);
        t.size *= 0.66;
        t
    }
    /// Width of a text at a type, in natural pt.
    pub fn tw(&self, s: &str, ty: &Type) -> f64 {
        text::block(&self.cx.vars.resolve(s), ty.size).0 / self.k()
    }
    /// A centred text box whose top is at `y`; returns its height.
    pub fn text(&mut self, name: &str, s: &str, cx: f64, y: f64, ty: &Type) -> f64 {
        let k = self.k();
        let (o, _, h) = self.cx.text(name, s, ty, cx * k, y * k);
        self.push(o);
        h / k
    }
    /// A text box centred on (cx, cy).
    pub fn text_at(&mut self, name: &str, s: &str, cx: f64, cy: f64, ty: &Type) -> f64 {
        let k = self.k();
        let lines = text::block(&self.cx.vars.resolve(s), ty.size).1 as f64;
        let h = lines * ty.size * 1.25 / k;
        self.text(name, s, cx, cy - h / 2.0, ty);
        h
    }
    /// Text inside part `i`.
    pub fn put(&mut self, i: usize, s: &str, ty: &Type, vert: bool) {
        let mut o = self.objs[i].clone();
        self.cx.put_text_pub(&mut o, s, ty, vert);
        self.objs[i] = o;
    }
    pub fn core(&mut self, w: f64, h: f64, label: LabelPos) -> Core {
        let k = self.k();
        Core { w: w * k, h: h * k, objs: std::mem::take(&mut self.objs), single: false, ellipse: false, label, legend: std::mem::take(&mut self.legend), anchors: std::mem::take(&mut self.anchors) }
    }
}

/// Bounding box of drawn objects in natural pt.
fn extent(objs: &[Object], k: f64) -> (f64, f64, f64, f64) {
    let b = lore_canvas::geom::bounds(objs.iter().map(|o| &o.frame));
    let e = crate::modules::EMU_PT * k;
    (b.x as f64 / e, b.y as f64 / e, b.w as f64 / e, b.h as f64 / e)
}

/// Move everything so the drawing starts at (0, 0); returns its size.
fn normalise(pen: &mut Pen) -> (f64, f64) {
    let k = pen.k();
    let (x, y, w, h) = extent(&pen.objs, k);
    let (dx, dy) = (crate::modules::emu(-x * k), crate::modules::emu(-y * k));
    for o in &mut pen.objs {
        o.frame.x += dx;
        o.frame.y += dy;
    }
    for a in &mut pen.anchors {
        a.1.0 -= x * k;
        a.1.1 -= y * k;
    }
    (w, h)
}

fn count(n: &Node, key: &str, default: usize, lo: usize, hi: usize) -> usize {
    n.num(key).map(|v| v as usize).unwrap_or(default).clamp(lo, hi)
}

/// Built-in styles of the kinds with art styles (the first is the default).
pub fn styles(kind: &str) -> &'static [&'static str] {
    match kind {
        "encoder" | "decoder" | "autoencoder" => &CODER,
        "nn" => &NN,
        _ => &[],
    }
}

pub const CODER: [&str; 17] = ["trapezoid", "bars", "cubes", "slabs", "planes", "lenet", "fc", "resnet", "dense", "unet", "transformer", "rnn", "vit", "fpn", "vae", "graph", "layers"];

pub const NN: [&str; 15] = ["circles", "labelled", "dropout", "neuron", "pills", "fans", "attention", "graph", "recurrent", "lstm", "conv", "residual", "heads", "moe", "rbm"];

// ───────────────────────────── encoder / decoder ─────────────────────────────

/// Size fraction along the flow: an encoder shrinks, a decoder grows.
fn frac(t: f64, dec: bool, narrow: f64) -> f64 {
    if dec { narrow + (1.0 - narrow) * t } else { 1.0 - (1.0 - narrow) * t }
}

/// Map a layer name to its type.
pub fn layer_key(name: &str) -> &'static str {
    let l = text::plain(name).to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| l.contains(x));
    if has(&["softmax", "sigmoid"]) {
        "softmax"
    } else if has(&["upsampl", "unpool", "deconv", "transpose", "up-conv", "upconv", "upsample"]) {
        "up"
    } else if has(&["pool"]) {
        "pool"
    } else if has(&["attention", "attn", "mha"]) {
        "attn"
    } else if has(&["norm"]) {
        "norm"
    } else if has(&["feed", "ffn", "mlp"]) {
        "ffn"
    } else if has(&["fc", "dense", "linear", "fully"]) {
        "fc"
    } else if has(&["embed"]) {
        "embed"
    } else if has(&["conv", "relu", "bn"]) {
        "conv"
    } else {
        "other"
    }
}

pub(crate) fn coder(n: &Node, cx: &Ctx, dec: bool) -> Core {
    let which = crate::modules::style_name(n, cx.theme);
    let mut p = Pen::new(n, cx);
    match which.as_str() {
        "bars" => bars(n, &mut p, dec),
        "cubes" | "3d" => cubes(n, &mut p, dec),
        "slabs" => slabs(n, &mut p, dec),
        "planes" | "stacked" => planes(n, &mut p, dec),
        "lenet" => lenet(n, &mut p, dec),
        "fc" => fc(n, &mut p, dec),
        "resnet" => resnet(n, &mut p, dec),
        "dense" => dense(n, &mut p, dec),
        "unet" => unet_half(n, &mut p, dec),
        "transformer" => transformer(n, &mut p, dec),
        "rnn" => rnn(n, &mut p, dec),
        "vit" => vit(n, &mut p, dec),
        "fpn" => fpn(n, &mut p, dec),
        "vae" => vae(n, &mut p, dec),
        "graph" => graph_coder(n, &mut p, dec),
        "layers" => layer_list(n, &mut p, dec),
        _ => trapezoid(n, &mut p, dec),
    }
}

fn trapezoid(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let (w, h) = (n.num("w").unwrap_or(58.0), n.num("h").unwrap_or(46.0));
    let (a, b) = (frac(0.0, dec, 0.5), frac(1.0, dec, 0.5));
    let c = p.main();
    let i = p.polygon("", &[(0.0, h * (1.0 - a) / 2.0), (w, h * (1.0 - b) / 2.0), (w, h * (1.0 + b) / 2.0), (0.0, h * (1.0 + a) / 2.0)], &c);
    let ty = p.ty_label();
    p.put(i, &n.label, &ty, false);
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Drawn)
}

fn bars(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let steps = count(n, "n", 4, 2, 8);
    let (bw, gap, h) = (9.0, 5.0, 46.0);
    let c = p.main();
    for i in 0..steps {
        let bh = h * frac(i as f64 / (steps - 1) as f64, dec, 0.5);
        p.rect(&format!("bar{i}"), i as f64 * (bw + gap), (h - bh) / 2.0, bw, bh, &c);
    }
    let w = steps as f64 * (bw + gap) - gap;
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// PlotNeuralNet blocks: a conv block with its pooling (or upsampling) slice
/// and the channel count under it; space shrinks as channels grow.
fn cubes(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let steps = count(n, "n", 4, 2, 6);
    let mut sizes = n.list("sizes");
    if sizes.is_empty() {
        sizes = (0..steps).map(|i| format!("{}", 64 << i)).collect();
    }
    let ty = p.ty_tiny();
    let max_h = 46.0;
    let top = max_h * 0.55 * 0.42;
    let mid = top + max_h / 2.0;
    let mut x = 0.0;
    for j in 0..steps {
        let i = if dec { steps - 1 - j } else { j };
        let t = i as f64 / (steps - 1) as f64;
        let h = max_h * (1.0 - 0.55 * t);
        let th = 5.0 + 7.0 * t;
        let d = h * 0.55;
        let y = mid - h / 2.0;
        let (conv, side) = (p.col("conv"), p.col(if dec { "up" } else { "pool" }));
        if dec {
            p.cuboid(&format!("s{j}-up"), x, y, 2.4, h, d, &side);
            x += 2.4;
        }
        p.cuboid(&format!("s{j}-conv"), x, y, th, h, d, &conv);
        let lx = x + th / 2.0;
        x += th;
        if !dec {
            p.cuboid(&format!("s{j}-pool"), x, y, 2.4, h, d, &side);
            x += 2.4;
        }
        if let Some(s) = sizes.get(i) {
            p.text(&format!("s{j}-size"), s, lx, mid + max_h / 2.0 + 1.5, &ty);
        }
        x += d * 0.5 + 5.0;
    }
    let (w, h) = normalise(p);
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// SegNet: dense rows of thin, tall slabs, one colour per layer type, stages
/// shrinking (encoder) or growing (decoder).
fn slabs(_n: &Node, p: &mut Pen, dec: bool) -> Core {
    let stages: Vec<Vec<&str>> = if dec {
        vec![vec!["up", "conv", "conv", "conv"], vec!["up", "conv", "conv", "conv"], vec!["up", "conv", "conv"], vec!["up", "conv", "conv", "softmax"]]
    } else {
        vec![vec!["conv", "conv", "pool"], vec!["conv", "conv", "pool"], vec!["conv", "conv", "conv", "pool"], vec!["conv", "conv", "conv", "pool"]]
    };
    let levels = stages.len();
    let max_h = 52.0;
    let (t, step, between) = (2.6, 3.5, 5.0);
    let top = 10.0 * 0.42;
    let mid = top + max_h / 2.0;
    let mut x = 0.0;
    for (si, st) in stages.iter().enumerate() {
        // the level counts from the largest stage
        let level = if dec { levels - 1 - si } else { si };
        let f = 1.0 - 0.5 * level as f64 / (levels - 1) as f64;
        let h = max_h * f;
        let d = 10.0 * f;
        for (li, key) in st.iter().enumerate() {
            let c = p.col(key);
            p.cuboid(&format!("s{si}-{li}"), x, mid - h / 2.0, t, h, d, &c);
            if *key == "pool" || *key == "up" {
                p.anchor(&format!("{key}{level}"), (x + t / 2.0 + d * 0.25, mid - h / 2.0 - d * 0.21));
            }
            x += step;
        }
        x += between;
    }
    let w = x - between - step + t + 10.0 * 0.5;
    p.anchor("in", (0.0, mid));
    p.anchor("out", (w, mid));
    p.core(w, mid + max_h / 2.0 + top * 0.0 + (mid - max_h / 2.0), LabelPos::Above)
}

/// Pastel feature-map planes at an angle, two per stage.
fn planes(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let steps = count(n, "n", 4, 2, 6);
    let max_h = 50.0;
    let mut x = 0.0;
    let c = p.main();
    for j in 0..steps {
        let f = frac(j as f64 / (steps - 1) as f64, dec, 0.4);
        let h = max_h * f;
        let y = (max_h - h) / 2.0;
        for q in 0..2 {
            p.plane(&format!("s{j}-{q}"), x, y, 7.0, h, 6.0 * f, &c);
            x += 4.0;
        }
        x += 9.0;
    }
    let w = x - 9.0 - 4.0 + 7.0;
    p.anchor("in", (0.0, max_h / 2.0));
    p.anchor("out", (w, max_h / 2.0));
    p.core(w, max_h, LabelPos::Above)
}

/// NN-SVG LeNet: stacks of feature maps seen from the front, more maps as
/// they get smaller, the size under each stack.
fn lenet(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let steps = count(n, "n", 4, 2, 5);
    let mut sizes = n.list("sizes");
    if sizes.is_empty() {
        sizes = (0..steps).map(|i| format!("{}²", 128 >> i)).collect();
    }
    let ty = p.ty_tiny();
    let max_s = 38.0;
    let off = 2.6;
    let total_h = max_s + off * (steps as f64);
    let c = p.main();
    let mut x = 0.0;
    for j in 0..steps {
        let i = if dec { steps - 1 - j } else { j };
        let s = max_s * (1.0 - 0.6 * i as f64 / (steps - 1) as f64);
        let maps = 2 + i;
        let stack = s + off * (maps - 1) as f64;
        let y0 = (total_h - stack) / 2.0;
        for m in 0..maps {
            let back = (maps - 1 - m) as f64;
            p.cell(&format!("s{j}-m{m}"), x + back * off, y0 + (maps - 1) as f64 * off - back * off, s, s, &c);
        }
        if let Some(l) = sizes.get(i) {
            p.text(&format!("s{j}-size"), l, x + stack / 2.0, total_h + 1.5, &ty);
        }
        x += stack + 9.0;
    }
    let (w, h) = normalise(p);
    p.anchor("in", (0.0, total_h / 2.0));
    p.anchor("out", (w, total_h / 2.0));
    p.core(w, h, LabelPos::Above)
}

fn neuron_columns(p: &mut Pen, cols: &[usize], cols_x: &[f64], cy: f64, d: f64, vgap: f64, colours: &[String], prefix: &str) -> Vec<Vec<(f64, f64)>> {
    let centres: Vec<Vec<(f64, f64)>> = cols
        .iter()
        .zip(cols_x)
        .map(|(c, x)| {
            let ch = *c as f64 * d + (*c as f64 - 1.0) * vgap;
            (0..*c).map(|i| (*x, cy - ch / 2.0 + d / 2.0 + i as f64 * (d + vgap))).collect()
        })
        .collect();
    for l in 0..cols.len().saturating_sub(1) {
        for (a, pa) in centres[l].iter().enumerate() {
            for (b, pb) in centres[l + 1].iter().enumerate() {
                p.wire(&format!("{prefix}w{l}-{a}-{b}"), (pa.0 + d / 2.0, pa.1), (pb.0 - d / 2.0, pb.1));
            }
        }
    }
    for (l, col) in centres.iter().enumerate() {
        for (i, c) in col.iter().enumerate() {
            p.ellipse(&format!("{prefix}n{l}-{i}"), c.0 - d / 2.0, c.1 - d / 2.0, d, d, &colours[l]);
        }
    }
    centres
}

/// A fully connected funnel of neurons.
fn fc(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let mut cols: Vec<usize> = n.list("layers").iter().filter_map(|l| l.trim().parse::<usize>().ok()).map(|v| v.clamp(1, 8)).collect();
    if cols.is_empty() {
        cols = vec![5, 4, 3];
    }
    if dec {
        cols.reverse();
    }
    let (d, vgap, lgap) = (7.0, 3.0, 16.0);
    let tallest = *cols.iter().max().unwrap() as f64;
    let h = tallest * d + (tallest - 1.0) * vgap;
    let xs: Vec<f64> = (0..cols.len()).map(|i| d / 2.0 + i as f64 * (d + lgap)).collect();
    let c = p.main();
    let colours = vec![c; cols.len()];
    neuron_columns(p, &cols, &xs, h / 2.0, d, vgap, &colours, "");
    let w = cols.len() as f64 * d + (cols.len() - 1) as f64 * lgap;
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// ResNet stages: pairs of layers bridged by a residual arc, repeat counts below.
fn resnet(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let steps = count(n, "n", 4, 2, 6);
    let reps = ["×3", "×4", "×6", "×3", "×2", "×2"];
    let max_h = 44.0;
    let ty = p.ty_tiny();
    let arc_room = 9.0;
    let mid = arc_room + max_h / 2.0;
    let mut x = 0.0;
    for j in 0..steps {
        let i = if dec { steps - 1 - j } else { j };
        let h = max_h * (1.0 - 0.5 * i as f64 / (steps - 1) as f64);
        let y = mid - h / 2.0;
        let c = p.col(if dec && j > 0 { "up" } else { "conv" });
        let c2 = p.col("conv");
        p.rect(&format!("s{j}-a"), x, y, 6.0, h, &c);
        p.rect(&format!("s{j}-b"), x + 9.0, y, 6.0, h, &c2);
        let ink = p.f.ink.clone();
        p.arc(&format!("s{j}-skip"), (x + 1.0, y - 1.0), (x + 7.5, y - 13.0), (x + 14.0, y - 1.0), &ink, 0.75, "solid", "triangle");
        p.text(&format!("s{j}-n"), reps[i], x + 7.5, mid + max_h / 2.0 + 1.5, &ty);
        x += 15.0 + 8.0;
    }
    let (w, h) = normalise(p);
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// DenseNet: every layer of the block feeds every later one; a transition
/// layer pools (encoder) or upsamples (decoder).
fn dense(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let layers = count(n, "n", 4, 3, 6);
    let (lw, gap, h) = (6.0, 9.0, 34.0);
    let room = 16.0;
    let y = room;
    let mut x = 0.0;
    let trans = |p: &mut Pen, x: f64, key: &str| {
        let c = p.col(key);
        p.rect(&format!("trans-{key}"), x, y + h * 0.2, 5.0, h * 0.6, &c);
    };
    if dec {
        trans(p, x, "up");
        x += 5.0 + gap;
    }
    let start = x;
    let c = p.col("conv");
    for i in 0..layers {
        p.rect(&format!("l{i}"), x, y, lw, h, &c);
        x += lw + gap;
    }
    let ink = p.f.ink.clone();
    for a in 0..layers {
        for b in a + 1..layers {
            let (xa, xb) = (start + a as f64 * (lw + gap) + lw / 2.0, start + b as f64 * (lw + gap) + lw / 2.0);
            let bulge = 3.5 + 4.0 * (b - a) as f64;
            p.arc(&format!("skip{a}-{b}"), (xa, y - 0.5), ((xa + xb) / 2.0, y - 2.0 * bulge), (xb, y - 0.5), &ink, 0.5, "solid", "triangle");
        }
    }
    if !dec {
        trans(p, x, "pool");
        x += 5.0;
    } else {
        x -= gap;
    }
    let (w, hh) = normalise(p);
    let _ = x;
    p.anchor("in", (0.0, hh - h / 2.0));
    p.anchor("out", (w, hh - h / 2.0));
    p.core(w, hh, LabelPos::Below)
}

/// U-Net line art, one half: a pair of conv layers per level, each level
/// below the last (encoder) or above it (decoder).
fn unet_half(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let levels = count(n, "depth", 4, 2, 5);
    let (bw, pair_gap, vgap) = (4.0, 6.0, 6.0);
    let h0 = 26.0;
    let ink = p.f.ink.clone();
    let c = p.main();
    let step = { let c = p.col(if dec { "up" } else { "pool" }); p.ink_of(&c) };
    let hl = |l: usize| h0 * 0.72f64.powi(l as i32);
    // top of each level (level 0 at the top)
    let tops: Vec<f64> = (0..levels).scan(0.0, |y, l| { let t = *y; *y += hl(l) + vgap; Some(t) }).collect();
    // encoder: level 0 at the left; decoder: the deepest level at the left
    let mut x = 0.0;
    let mut prev: Option<(f64, usize)> = None;
    for j in 0..levels {
        let level = if dec { levels - 1 - j } else { j };
        let (y, h) = (tops[level], hl(level));
        if let Some((px, pl)) = prev {
            // the next level starts under (or over) the last bar of the previous one
            x = px;
            if dec {
                p.line(&format!("l{j}-up"), (x + bw / 2.0, tops[pl]), (x + bw / 2.0, y + h), &step, 1.0, "solid", "triangle");
            } else {
                p.line(&format!("l{j}-down"), (x + bw / 2.0, tops[pl] + hl(pl)), (x + bw / 2.0, y), &step, 1.0, "solid", "triangle");
            }
            x += bw + pair_gap;
        }
        p.rect(&format!("l{j}-a"), x, y, bw, h, &c);
        p.line(&format!("l{j}-conv"), (x + bw, y + h / 2.0), (x + bw + pair_gap, y + h / 2.0), &ink, 0.6, "solid", "triangle");
        let x2 = x + bw + pair_gap;
        p.rect(&format!("l{j}-b"), x2, y, bw, h, &c);
        p.anchor(&format!("lvl{level}"), if dec { (x, y + h / 2.0) } else { (x2 + bw, y + h / 2.0) });
        prev = Some((x2, level));
        x = x2;
    }
    let w = x + bw;
    let h = tops[levels - 1] + hl(levels - 1);
    p.anchor("in", (0.0, hl(0) / 2.0));
    p.anchor("out", (w, if dec { hl(0) / 2.0 } else { h }));
    p.core(w, h, LabelPos::Above)
}

/// A transformer block: sub-layers bottom to top, residual arrows, × N.
fn transformer(_n: &Node, p: &mut Pen, dec: bool) -> Core {
    let subs: Vec<(&str, &str, f64)> = if dec {
        vec![("attn", "Masked Multi-Head\\\\Attention", 15.0), ("norm", "Add & Norm", 8.0), ("attn", "Multi-Head\\\\Attention", 15.0), ("norm", "Add & Norm", 8.0), ("ffn", "Feed Forward", 10.0), ("norm", "Add & Norm", 8.0)]
    } else {
        vec![("attn", "Multi-Head\\\\Attention", 15.0), ("norm", "Add & Norm", 8.0), ("ffn", "Feed Forward", 10.0), ("norm", "Add & Norm", 8.0)]
    };
    let ty = p.ty_tiny();
    let widest = subs.iter().flat_map(|s| s.1.split("\\\\")).map(|l| p.tw(l, &ty)).fold(0.0, f64::max);
    let (sw, pad, gap) = ((widest + 8.0).max(50.0), 7.0, 7.0);
    let inner_h: f64 = subs.iter().map(|s| s.2).sum::<f64>() + gap * (subs.len() - 1) as f64;
    let (fx, fy) = (12.0, 9.0);
    let fw = sw + 2.0 * pad;
    let fh = inner_h + 2.0 * pad;
    let frame_col = p.main();
    let frame_col = if p.role.is_some() { shade(&frame_col, 0.6) } else { frame_col };
    p.rect("block", fx, fy, fw, fh, &frame_col);
    let sx = fx + pad;
    let cxm = sx + sw / 2.0;
    let ink = p.f.ink.clone();
    // bottom to top
    let mut y = fy + fh - pad;
    let mut tops = Vec::new();
    for (i, (key, label, h)) in subs.iter().enumerate() {
        y -= h;
        let c = p.col(key);
        let id = p.rect(&format!("sub{i}"), sx, y, sw, *h, &c);
        p.put(id, label, &ty, false);
        tops.push((y, *h));
        y -= gap;
    }
    // arrows between sub-layers and the residual paths (from below a sub-layer to the norm above it)
    p.arrow("in", (cxm, fy + fh + 8.0), (cxm, tops[0].0 + tops[0].1));
    for i in 1..tops.len() {
        p.arrow(&format!("a{i}"), (cxm, tops[i - 1].0), (cxm, tops[i].0 + tops[i].1));
    }
    let mut i = 0;
    while i + 1 < tops.len() {
        let from_y = if i == 0 { fy + fh - pad * 0.45 } else { tops[i - 1].0 - gap / 2.0 };
        let to_y = tops[i + 1].0 + tops[i + 1].1 / 2.0;
        p.path(&format!("res{i}"), &[(cxm, from_y), (sx - 3.5, from_y), (sx - 3.5, to_y), (sx, to_y)], &ink, 0.6, "solid", "triangle");
        i += 2;
    }
    p.arrow("out", (cxm, tops.last().unwrap().0), (cxm, fy - 8.0));
    let nty = p.ty_small();
    // N× outside the block, on the side no line comes in from
    p.text_at("times", "N×", if dec { fx + fw + 6.5 } else { fx - 6.5 }, fy + fh / 2.0, &nty);
    if dec {
        // the encoder's output enters the cross-attention from the left
        let cross = tops[2].0 + tops[2].1 / 2.0;
        p.anchor("cross", (fx, cross));
    }
    p.anchor("top", (cxm, fy - 8.0));
    let (w, h) = (fx + fw, fy + fh + 8.0);
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// An RNN unrolled over time: inputs below (encoder) or outputs above (decoder).
fn rnn(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let cells = count(n, "n", 3, 2, 5);
    let (cw, ch, gap) = (16.0, 14.0, 11.0);
    let ty = p.ty_tiny();
    let math = p.ty_small();
    let c = p.col("cell");
    let row = 14.0;
    let y = if dec { row } else { 0.0 };
    let x0 = if dec { 10.0 } else { 0.0 };
    for i in 0..cells {
        let x = x0 + i as f64 * (cw + gap);
        let id = p.rect(&format!("c{i}"), x, y, cw, ch, &c);
        let name = if dec { format!("$s_{}$", i + 1) } else { format!("$h_{}$", i + 1) };
        p.put(id, &name, &ty, false);
        if i + 1 < cells {
            p.arrow(&format!("h{i}"), (x + cw, y + ch / 2.0), (x + cw + gap, y + ch / 2.0));
        }
        if dec {
            p.arrow(&format!("o{i}"), (x + cw / 2.0, y), (x + cw / 2.0, y - 5.0));
            p.text(&format!("y{i}"), &format!("$\\hat y_{}$", i + 1), x + cw / 2.0, y - 5.0 - math.size * 1.25 / p.cx.k, &math);
        } else {
            p.arrow(&format!("i{i}"), (x + cw / 2.0, y + ch + 5.0), (x + cw / 2.0, y + ch));
            p.text(&format!("x{i}"), &format!("$x_{}$", i + 1), x + cw / 2.0, y + ch + 5.0, &math);
        }
    }
    let last = x0 + (cells - 1) as f64 * (cw + gap) + cw;
    if dec {
        p.arrow("ctx", (0.0, y + ch / 2.0), (x0, y + ch / 2.0));
    } else {
        p.arrow("ctx", (last, y + ch / 2.0), (last + 10.0, y + ch / 2.0));
    }
    let (w, h) = normalise(p);
    let cy = if dec { h - ch / 2.0 - (h - row - ch) } else { ch / 2.0 };
    p.anchor("in", (0.0, cy));
    p.anchor("out", (w, cy));
    p.core(w, h, if dec { LabelPos::Below } else { LabelPos::Above })
}

/// ViT: the picture cut into patches, a row of tokens (class token first)
/// into the transformer; the decoder runs the other way (MAE).
fn vit(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let g = count(n, "n", 3, 2, 4);
    let (ps, pg) = (7.0, 1.5);
    let grid = g as f64 * ps + (g - 1) as f64 * pg;
    let (tok, tg) = (6.5, 2.0);
    let ntok = 5;
    let row_w = ntok as f64 * tok + (ntok - 1) as f64 * tg;
    let ty = p.ty_tiny();
    let (bw, bh) = (row_w + 6.0, 16.0);
    let img = p.cx.theme.colors.image.clone();
    let tk = p.col("token");
    let cls = p.col("cls");
    let box_c = p.main();
    // the token row sits under the block, patches beside the row
    let row_y = bh + 9.0;
    let (gx, rx) = if dec { (row_w + 12.0 + 3.0, 0.0) } else { (0.0, grid + 12.0) };
    let gy = row_y + tok / 2.0 - grid / 2.0;
    for r in 0..g {
        for c in 0..g {
            p.cell(&format!("patch{r}-{c}"), gx + c as f64 * (ps + pg), gy + r as f64 * (ps + pg), ps, ps, &img);
        }
    }
    for i in 0..ntok {
        let x = rx + i as f64 * (tok + tg);
        p.cell(&format!("tok{i}"), x, row_y, tok, tok, if i == 0 && !dec { &cls } else { &tk });
        if dec {
            p.arrow(&format!("t{i}"), (x + tok / 2.0, bh), (x + tok / 2.0, row_y));
        } else {
            p.arrow(&format!("t{i}"), (x + tok / 2.0, row_y), (x + tok / 2.0, bh));
        }
    }
    let bx = rx + row_w / 2.0 - bw / 2.0;
    let id = p.rect("block", bx, 0.0, bw, bh, &box_c);
    p.put(id, "Transformer", &ty, false);
    if dec {
        p.arrow("to-patches", (rx + row_w + 2.0, row_y + tok / 2.0), (gx - 1.5, row_y + tok / 2.0));
    } else {
        p.arrow("to-tokens", (grid + 1.5, row_y + tok / 2.0), (rx - 2.0, row_y + tok / 2.0));
    }
    let (w, h) = normalise(p);
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// FPN: a pyramid of feature planes, bottom-up (encoder) or top-down with
/// lateral inputs (decoder).
fn fpn(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let levels = count(n, "n", 4, 3, 5);
    let ty = p.ty_tiny();
    let c = p.main();
    let ink = p.f.ink.clone();
    let paired = n.flag("_pair");
    let w0 = 40.0;
    let gap = 7.0;
    let stub = if dec && !paired { 12.0 } else { 0.0 };
    let mut y = 0.0;
    let mut rows = Vec::new();
    // top level first (smallest)
    for i in (0..levels).rev() {
        let w = w0 * 0.72f64.powi(i as i32);
        let hh = w * 0.32;
        rows.push((i, w, hh, y));
        y += hh + gap;
    }
    let total = y - gap;
    let axis = stub + w0 / 2.0 + 3.0;
    for (i, w, hh, y) in &rows {
        let x = axis - w / 2.0;
        let skew = hh * 0.8;
        p.polygon(&format!("p{i}"), &[(x + skew, *y), (x + w + skew * 0.0 + skew, *y), (x + w, y + hh), (x, y + hh)], &c);
        let name = if dec { format!("P{}", i + 2) } else { format!("C{}", i + 2) };
        let right = x + w + skew;
        if dec {
            p.anchor(&format!("p{i}"), (x + skew * 0.5, y + hh / 2.0));
            if !paired {
                p.arrow(&format!("lat{i}"), (0.0, y + hh / 2.0), (x + skew * 0.5 - 1.0, y + hh / 2.0));
            }
            p.text_at(&format!("n{i}"), &name, right + 6.5, y + hh / 2.0, &ty);
        } else {
            p.anchor(&format!("c{i}"), (right - skew * 0.5, y + hh / 2.0));
            p.text_at(&format!("n{i}"), &name, x - 6.0, y + hh / 2.0, &ty);
        }
    }
    for j in 0..rows.len() - 1 {
        let (a, b) = (rows[j], rows[j + 1]);
        let (top, bottom) = (a.3 + a.2, b.3);
        if dec {
            p.line(&format!("v{j}"), (axis + 2.0, top + 0.5), (axis + 2.0, bottom - 0.5), &ink, 0.75, "solid", "triangle");
        } else {
            p.line(&format!("v{j}"), (axis + 2.0, bottom - 0.5), (axis + 2.0, top + 0.5), &ink, 0.75, "solid", "triangle");
        }
    }
    let _ = total;
    let (w, h) = normalise(p);
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// VAE: the encoder ends in μ and σ and samples z; the decoder starts from z.
fn vae(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let (tw, th) = (40.0, 42.0);
    let c = p.main();
    let z = p.col("latent");
    let ty = p.ty_tiny();
    let lab = p.ty_label();
    let math = p.ty_small();
    let mid = th / 2.0;
    let paired = n.flag("_pair");
    if dec {
        let x = if paired { 0.0 } else { 18.0 };
        if !paired {
            let id = p.cell("z", 0.0, mid - 5.0, 10.0, 10.0, &z);
            p.put(id, "$z$", &ty, false);
            p.arrow("z-in", (10.0, mid), (x, mid));
        }
        let i = p.polygon("", &[(x, th * 0.25), (x + tw, 0.0), (x + tw, th), (x, th * 0.75)], &c);
        p.put(i, &n.label, &lab, false);
        p.anchor("in", (0.0, mid));
        p.anchor("out", (x + tw, mid));
        return p.core(x + tw, th, LabelPos::Drawn);
    }
    let i = p.polygon("", &[(0.0, 0.0), (tw, th * 0.25), (tw, th * 0.75), (0.0, th)], &c);
    p.put(i, &n.label, &lab, false);
    let bx = tw + 8.0;
    let mu = p.cell("mu", bx, mid - 13.0, 11.0, 9.0, &c);
    p.put(mu, "$\\mu$", &math, false);
    let sg = p.cell("sigma", bx, mid + 4.0, 11.0, 9.0, &c);
    p.put(sg, "$\\sigma$", &math, false);
    let ink = p.f.ink.clone();
    p.line("to-mu", (tw, mid - 8.5), (bx, mid - 8.5), &ink, 0.6, "solid", "none");
    p.line("to-sigma", (tw, mid + 8.5), (bx, mid + 8.5), &ink, 0.6, "solid", "none");
    let ox = bx + 19.0;
    p.path("mu-z", &[(bx + 11.0, mid - 8.5), (ox - 2.0, mid - 8.5), (ox + 3.0, mid - 4.0)], &ink, 0.6, "solid", "none");
    p.path("sigma-z", &[(bx + 11.0, mid + 8.5), (ox - 2.0, mid + 8.5), (ox + 3.0, mid + 4.0)], &ink, 0.6, "solid", "none");
    let (paper, line) = (p.f.paper.clone(), p.f.line.clone());
    let k = p.k();
    let mut o = shape("sample", "ellipse", ox * k, (mid - 4.0) * k, 8.0 * k, 8.0 * k, Paint::solid(&paper), Some(Stroke::new(&line, 0.75 * k)));
    p.cx.put_text_pub(&mut o, "~", &ty, false);
    p.objs.push(o);
    p.text("eps", "$\\epsilon$", ox + 4.0, mid + 6.0, &math);
    let zx = ox + 14.0;
    p.arrow("to-z", (ox + 8.0, mid), (zx, mid));
    let id = p.cell("z", zx, mid - 5.0, 10.0, 10.0, &z);
    p.put(id, "$z$", &ty, false);
    p.anchor("in", (0.0, mid));
    p.anchor("out", (zx + 10.0, mid));
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Drawn)
}

/// Graph network: a graph read out to a vector (encoder); a vector decoded to
/// an adjacency matrix (decoder).
fn graph_coder(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let _ = n;
    let c = p.main();
    let z = p.col("latent");
    let cellc = p.f.tensor.clone();
    let paper = p.f.paper.clone();
    let cells = 4;
    let cs = 6.0;
    let vh = cells as f64 * cs;
    let h = 32.0;
    let vy = (h - vh) / 2.0;
    if dec {
        // in a pair the encoder's read-out vector is the latent
        let mx = if n.flag("_pair") {
            0.0
        } else {
            for i in 0..cells {
                p.cell(&format!("z{i}"), 0.0, vy + i as f64 * cs, cs, cs, &z);
            }
            p.arrow("to-adj", (cs + 1.5, h / 2.0), (cs + 11.0, h / 2.0));
            cs + 13.0
        };
        let ms = 5.4;
        let my = (h - 5.0 * ms) / 2.0;
        for r in 0..5 {
            for q in 0..5 {
                let on = r == q || (r * 3 + q * 2) % 5 == 1;
                p.cell(&format!("a{r}{q}"), mx + q as f64 * ms, my + r as f64 * ms, ms, ms, if on { &c } else { &paper });
            }
        }
        let _ = cellc;
        let w = mx + 5.0 * ms;
        p.anchor("in", (0.0, h / 2.0));
        p.anchor("out", (w, h / 2.0));
        return p.core(w, h, LabelPos::Above);
    }
    let pts = [(4.0, 6.0), (17.0, 3.0), (26.0, 14.0), (9.0, 20.0), (20.0, 28.0), (3.0, 29.0)];
    let edges = [(0, 1), (1, 2), (0, 3), (1, 3), (3, 4), (2, 4), (3, 5)];
    let ink = p.f.muted.clone();
    for (a, b) in edges {
        p.line(&format!("e{a}{b}"), pts[a], pts[b], &ink, 0.6, "solid", "none");
    }
    for (i, q) in pts.iter().enumerate() {
        p.ellipse(&format!("v{i}"), q.0 - 3.0, q.1 - 3.0, 6.0, 6.0, &c);
    }
    p.arrow("readout", (31.0, h / 2.0), (40.0, h / 2.0));
    for i in 0..cells {
        p.cell(&format!("z{i}"), 42.0, vy + i as f64 * cs, cs, cs, &z);
    }
    let w = 42.0 + cs;
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (w, h / 2.0));
    p.core(w, h, LabelPos::Above)
}

/// A list of layers, one labelled bar each, narrowing (encoder) or widening.
fn layer_list(n: &Node, p: &mut Pen, dec: bool) -> Core {
    let mut layers = n.list("layers");
    if layers.is_empty() {
        layers = if dec { vec!["Upsample", "Conv 3×3, 128", "Upsample", "Conv 3×3, 64", "Softmax"] } else { vec!["Conv 3×3, 64", "Max pool", "Conv 3×3, 128", "Max pool", "Conv 3×3, 256"] }.into_iter().map(String::from).collect();
    }
    let ty = p.ty_tiny();
    let longest = layers.iter().map(|l| p.tw(l, &ty)).fold(0.0, f64::max);
    let full = (longest + 8.0).max(48.0);
    let (bh, gap) = (9.0, 2.5);
    let count = layers.len();
    for (i, l) in layers.iter().enumerate() {
        let t = if count > 1 { i as f64 / (count - 1) as f64 } else { 0.0 };
        let need = p.tw(l, &ty) + 5.0;
        let w = (full * frac(t, dec, 0.68)).max(need);
        let c = p.col(layer_key(l));
        let id = p.rect(&format!("l{i}"), (full - w) / 2.0, i as f64 * (bh + gap), w, bh, &c);
        p.put(id, l, &ty, false);
    }
    let h = count as f64 * (bh + gap) - gap;
    p.anchor("in", (0.0, h / 2.0));
    p.anchor("out", (full, h / 2.0));
    p.core(full, h, LabelPos::Above)
}

// ───────────────────────────── matched pair ─────────────────────────────

/// An encoder, the bottleneck and the mirrored decoder in one drawing, with
/// Encoder / Latent space / Decoder under them and one colour legend.
pub(crate) fn pair(n: &Node, cx: &Ctx) -> Core {
    let which = crate::modules::style_name(n, cx.theme);
    let which = if which.is_empty() { "trapezoid".to_string() } else { which };
    let mut half = n.clone();
    half.label = String::new();
    half.style = which.clone();
    half.params.insert("_pair".into(), true.into());
    let role_coloured = matches!(which.as_str(), "trapezoid" | "bars" | "planes" | "lenet" | "fc" | "graph" | "vae" | "transformer");
    let mut e = half.clone();
    let mut d = half.clone();
    if role_coloured {
        e.params.insert("_role".into(), "enc".into());
        d.params.insert("_role".into(), "dec".into());
    }
    e.label = if matches!(which.as_str(), "trapezoid" | "vae") { "Enc".into() } else { String::new() };
    d.label = if matches!(which.as_str(), "trapezoid" | "vae") { "Dec".into() } else { String::new() };
    let ec = coder(&e, cx, false);
    let dc = coder(&d, cx, true);
    let k = cx.k;
    let mut p = Pen::new(n, cx);
    p.legend = ec.legend.clone();
    for s in &dc.legend {
        if !p.legend.iter().any(|x| x.key == s.key) {
            p.legend.push(s.clone());
        }
    }
    let latent = n.str("latent").map(str::to_string).unwrap_or_else(|| match which.as_str() {
        "slabs" | "unet" | "fpn" | "transformer" | "vae" | "vit" | "graph" => "none".into(),
        "rnn" => "context".into(),
        "fc" => "neurons".into(),
        _ => "vector".into(),
    });
    let (ew, eh, dw, dh) = (ec.w / k, ec.h / k, dc.w / k, dc.h / k);
    let anchor = |c: &Core, name: &str| c.anchors.iter().find(|a| a.0 == name).map(|a| (a.1.0 / k, a.1.1 / k));
    let e_out = anchor(&ec, "out").unwrap_or((ew, eh / 2.0));
    let d_in = anchor(&dc, "in").unwrap_or((0.0, dh / 2.0));
    // vertical placement: halves share their centre line (U-Net and FPN: their tops)
    let top_aligned = matches!(which.as_str(), "unet" | "fpn");
    let h = eh.max(dh);
    let (ey, dy) = if top_aligned { (0.0, 0.0) } else { ((h - eh) / 2.0, (h - dh) / 2.0) };
    let mid = if top_aligned { h } else { h / 2.0 };
    let ink = cx.theme.colors.ink.clone();
    // the bottleneck
    let mut x = ew;
    let mut latent_span = None;
    let lc = p.col("latent");
    match latent.as_str() {
        "vector" | "context" => {
            let cells = if latent == "context" { 3 } else { 4 };
            let cs = 6.0;
            let vy = mid - cells as f64 * cs / 2.0;
            x += 7.0;
            p.arrow("to-latent", (ew, ey + e_out.1), (x - 0.5, ey + e_out.1));
            for i in 0..cells {
                p.cell(&format!("z{i}"), x, vy + i as f64 * cs, cs, cs, &lc);
            }
            if latent == "context" {
                let ty = p.ty_small();
                p.text("c", "$c$", x + cs / 2.0, vy + cells as f64 * cs + 1.0, &ty);
            }
            latent_span = Some((x, x + cs));
            x += cs + 7.0;
            p.arrow("from-latent", (x - 7.0 + 0.5, mid), (x, mid));
        }
        "neurons" => {
            let (dd, vgap) = (7.0, 3.0);
            x += 16.0;
            let cx_ = x + dd / 2.0;
            let ys = [mid - (dd + vgap) / 2.0, mid + (dd + vgap) / 2.0];
            // wires to the last encoder column and the first decoder column
            let e_last: Vec<(f64, f64)> = ec.objs.iter().filter(|o| o.name.starts_with("n") && o.name.contains('-')).map(|o| o.frame.clone()).filter(|f| (f.x + f.w) as f64 / (crate::modules::EMU_PT * k) > ew - 0.5).map(|f| ((f.x + f.w) as f64 / (crate::modules::EMU_PT * k), ey + (f.y + f.h / 2) as f64 / (crate::modules::EMU_PT * k))).collect();
            let d_first: Vec<(f64, f64)> = dc.objs.iter().filter(|o| o.name.starts_with("n") && o.name.contains('-')).map(|o| o.frame.clone()).filter(|f| (f.x as f64) / (crate::modules::EMU_PT * k) < 0.5).map(|f| (0.0, dy + (f.y + f.h / 2) as f64 / (crate::modules::EMU_PT * k))).collect();
            let dx0 = x + dd + 16.0;
            for (i, y) in ys.iter().enumerate() {
                for (j, a) in e_last.iter().enumerate() {
                    p.wire(&format!("lw{i}-{j}"), *a, (x, *y));
                }
                for (j, b) in d_first.iter().enumerate() {
                    p.wire(&format!("rw{i}-{j}"), (x + dd, *y), (dx0, b.1));
                }
            }
            for (i, y) in ys.iter().enumerate() {
                p.ellipse(&format!("z{i}"), cx_ - dd / 2.0, y - dd / 2.0, dd, dd, &lc);
            }
            latent_span = Some((x, x + dd));
            x = dx0;
        }
        _ => {
            let gap = match which.as_str() {
                "unet" => 26.0,
                "fpn" => 18.0,
                "transformer" => 16.0,
                _ => 14.0,
            };
            if !matches!(which.as_str(), "unet" | "fpn" | "transformer") {
                p.arrow("to-dec", (ew, ey + e_out.1), (ew + gap, dy + d_in.1));
            }
            x += gap;
        }
    }
    let dx = x;
    // the halves
    let shift = |c: Core, ox: f64, oy: f64, pen: &mut Pen, pre: &str| {
        for mut o in c.objs {
            o.frame.x += crate::modules::emu(ox * k);
            o.frame.y += crate::modules::emu(oy * k);
            o.name = format!("{pre}-{}", if o.name.is_empty() { "body" } else { &o.name });
            pen.objs.push(o);
        }
    };
    let ea: Vec<(String, (f64, f64))> = ec.anchors.iter().map(|(n, q)| (n.clone(), (q.0 / k, q.1 / k + ey))).collect();
    let da: Vec<(String, (f64, f64))> = dc.anchors.iter().map(|(n, q)| (n.clone(), (q.0 / k + dx, q.1 / k + dy))).collect();
    shift(ec, 0.0, ey, &mut p, "enc");
    shift(dc, dx, dy, &mut p, "dec");
    let get = |v: &Vec<(String, (f64, f64))>, name: &str| v.iter().find(|a| a.0 == name).map(|a| a.1);
    let muted = cx.theme.colors.muted.clone();
    // joins between the halves
    match which.as_str() {
        "slabs" => {
            for l in 0..4 {
                if let (Some(a), Some(b)) = (get(&ea, &format!("pool{l}")), get(&da, &format!("up{l}"))) {
                    let lift = 8.0 + 5.0 * (3 - l) as f64;
                    let top = a.1.min(b.1) - lift;
                    p.arc(&format!("index{l}"), a, ((a.0 + b.0) / 2.0, top - lift * 0.6), b, &muted, 0.6, "dash", "triangle");
                }
            }
            let ty = p.ty_tiny();
            let top = ea.iter().filter(|a| a.0.starts_with("pool")).map(|a| a.1.1).fold(f64::MAX, f64::min);
            p.text_at("indices", "Pooling indices", (ew + dx) / 2.0, top - 8.0 - 5.0 * 3.0 - 2.0, &ty);
        }
        "unet" => {
            let levels = count(n, "depth", 4, 2, 5);
            for l in 0..levels - 1 {
                if let (Some(a), Some(b)) = (get(&ea, &format!("lvl{l}")), get(&da, &format!("lvl{l}"))) {
                    p.line(&format!("skip{l}"), (a.0 + 1.0, a.1), (b.0 - 1.0, b.1), &muted, 0.75, "solid", "triangle");
                }
            }
            // the deepest level is shared: join it with one arrow
            if let (Some(a), Some(b)) = (get(&ea, &format!("lvl{}", levels - 1)), get(&da, &format!("lvl{}", levels - 1))) {
                p.line("bottom", (a.0 + 1.0, a.1), (b.0 - 1.0, b.1), &ink, 0.75, "solid", "triangle");
            }
        }
        "fpn" => {
            for l in 0..5 {
                if let (Some(a), Some(b)) = (get(&ea, &format!("c{l}")), get(&da, &format!("p{l}"))) {
                    p.line(&format!("lat{l}"), (a.0 + 1.0, a.1), (b.0 - 1.0, b.1), &ink, 0.75, "solid", "triangle");
                }
            }
        }
        "transformer" => {
            if let (Some(a), Some(b)) = (get(&ea, "top"), get(&da, "cross")) {
                let up = a.1 - 3.0;
                p.path("memory", &[a, (a.0, up), ((a.0 + b.0) / 2.0 + 4.0, up), ((a.0 + b.0) / 2.0 + 4.0, b.1), (b.0, b.1)], &ink, 0.75, "solid", "triangle");
            }
        }
        _ => {}
    }
    // the legend reads in the order of the drawing: encoder, latent, decoder
    if let Some(i) = p.legend.iter().position(|s| s.key == "latent")
        && let Some(e) = p.legend.iter().position(|s| s.key == "enc")
    {
        let l = p.legend.remove(i);
        let at = if i < e { e } else { e + 1 };
        p.legend.insert(at.min(p.legend.len()), l);
    }
    // Encoder / Latent space / Decoder
    let ty = p.ty_small();
    let base = h + 3.0;
    let base = p.objs.iter().map(|o| (o.frame.y + o.frame.h) as f64 / (crate::modules::EMU_PT * k)).fold(base, f64::max) + 2.0;
    p.text("enc-name", &cx.theme.layer_label("enc"), ew / 2.0, base, &ty);
    if let Some((a, b)) = latent_span {
        p.text("latent-name", &cx.theme.layer_label("latent"), (a + b) / 2.0, base, &ty);
    }
    p.text("dec-name", &cx.theme.layer_label("dec"), dx + dw / 2.0, base, &ty);
    let (w, hh) = normalise(&mut p);
    p.core(w, hh, LabelPos::Above)
}

// ───────────────────────────── nn ─────────────────────────────

pub(crate) fn nn(n: &Node, cx: &Ctx) -> Core {
    let which = crate::modules::style_name(n, cx.theme);
    let mut p = Pen::new(n, cx);
    let mut layers: Vec<usize> = n.list("layers").iter().filter_map(|l| l.trim().parse::<f64>().ok()).map(|v| v.clamp(1.0, 8.0) as usize).collect();
    if layers.is_empty() {
        layers = vec![3, 4, 2];
    }
    match which.as_str() {
        "labelled" => nn_labelled(n, &mut p),
        "dropout" => nn_dropout(&layers, &mut p),
        "neuron" => nn_neuron(&mut p),
        "pills" => nn_pills(&layers, &mut p),
        "fans" => nn_fans(n, &mut p),
        "attention" => nn_attention(&mut p),
        "graph" => nn_graph(&mut p),
        "recurrent" => nn_recurrent(&mut p),
        "lstm" => nn_lstm(&mut p),
        "conv" => nn_conv(&mut p),
        "residual" => nn_residual(&mut p),
        "heads" => nn_heads(&mut p),
        "moe" => nn_moe(&mut p),
        "rbm" => nn_rbm(&mut p),
        _ => nn_circles(&layers, &mut p),
    }
}

fn columns_x(count: usize, d: f64, lgap: f64) -> Vec<f64> {
    (0..count).map(|i| d / 2.0 + i as f64 * (d + lgap)).collect()
}

fn nn_circles(layers: &[usize], p: &mut Pen) -> Core {
    let (d, vgap, lgap) = (9.0, 5.0, 24.0);
    let tallest = *layers.iter().max().unwrap() as f64;
    let h = tallest * d + (tallest - 1.0) * vgap;
    let c = p.main();
    neuron_columns(p, layers, &columns_x(layers.len(), d, lgap), h / 2.0, d, vgap, &vec![c; layers.len()], "");
    let w = layers.len() as f64 * d + (layers.len() - 1) as f64 * lgap;
    p.core(w, h, LabelPos::Above)
}

/// The UFLDL MLP: x₁…xₙ in, +1 bias units, x̂ out, layer names below.
fn nn_labelled(n: &Node, p: &mut Pen) -> Core {
    let inputs = count(n, "inputs", 3, 2, 6);
    let hidden = count(n, "hidden", 2, 1, 5);
    let (d, vgap, lgap) = (12.0, 4.0, 26.0);
    let math = p.ty_tiny();
    let small = p.ty_small();
    let c = p.main();
    let paper = p.f.paper.clone();
    let rows = inputs + 1;
    let h = rows as f64 * d + (rows - 1) as f64 * vgap;
    let xs = columns_x(3, d, lgap);
    let col_y = |count: usize, i: usize| {
        let ch = count as f64 * d + (count - 1) as f64 * vgap;
        (h - ch) / 2.0 + d / 2.0 + i as f64 * (d + vgap)
    };
    // units: input + bias, hidden + bias, output
    let sizes = [(inputs, true), (hidden, true), (inputs, false)];
    let mut cols: Vec<Vec<(f64, f64, bool)>> = Vec::new();
    for (l, (units, bias)) in sizes.iter().enumerate() {
        let total = units + *bias as usize;
        let start = if l == 2 { 0.0 } else { 0.0 };
        let _ = start;
        cols.push((0..total).map(|i| (xs[l], col_y(total, i), *bias && i == total - 1)).collect());
    }
    for l in 0..2 {
        for a in &cols[l] {
            for b in cols[l + 1].iter().filter(|b| !b.2) {
                p.wire(&format!("w{l}"), (a.0 + d / 2.0, a.1), (b.0 - d / 2.0, b.1));
            }
        }
    }
    for (l, col) in cols.iter().enumerate() {
        for (i, u) in col.iter().enumerate() {
            let id = p.ellipse(&format!("n{l}-{i}"), u.0 - d / 2.0, u.1 - d / 2.0, d, d, if u.2 { &paper } else { &c });
            let t = if u.2 {
                "+1".to_string()
            } else {
                match l {
                    0 => format!("$x_{}$", i + 1),
                    1 => String::new(),
                    _ => format!("$\\hat x_{}$", i + 1),
                }
            };
            if !t.is_empty() {
                p.put(id, &t, &math, false);
            }
        }
    }
    for (l, x) in xs.iter().enumerate() {
        p.text(&format!("layer{l}"), &format!("Layer $L_{}$", l + 1), *x, h + 2.0, &small);
    }
    let (w, hh) = normalise(p);
    p.core(w, hh, LabelPos::Above)
}

fn nn_dropout(layers: &[usize], p: &mut Pen) -> Core {
    let (d, vgap, lgap) = (9.0, 5.0, 24.0);
    let tallest = *layers.iter().max().unwrap() as f64;
    let h = tallest * d + (tallest - 1.0) * vgap;
    let xs = columns_x(layers.len(), d, lgap);
    let dropped = |l: usize, i: usize| l > 0 && l + 1 < layers.len() && (i + l) % 2 == 1;
    let c = p.main();
    let paper = p.f.paper.clone();
    let muted = p.f.muted.clone();
    let centre = |l: usize, i: usize| {
        let ch = layers[l] as f64 * d + (layers[l] - 1) as f64 * vgap;
        (xs[l], (h - ch) / 2.0 + d / 2.0 + i as f64 * (d + vgap))
    };
    for l in 0..layers.len() - 1 {
        for a in 0..layers[l] {
            for b in 0..layers[l + 1] {
                if !dropped(l, a) && !dropped(l + 1, b) {
                    let (pa, pb) = (centre(l, a), centre(l + 1, b));
                    p.wire(&format!("w{l}-{a}-{b}"), (pa.0 + d / 2.0, pa.1), (pb.0 - d / 2.0, pb.1));
                }
            }
        }
    }
    for l in 0..layers.len() {
        for i in 0..layers[l] {
            let q = centre(l, i);
            if dropped(l, i) {
                p.ellipse(&format!("n{l}-{i}"), q.0 - d / 2.0, q.1 - d / 2.0, d, d, &paper);
                let r = d * 0.36;
                p.line(&format!("x{l}-{i}a"), (q.0 - r, q.1 - r), (q.0 + r, q.1 + r), &muted, 0.75, "solid", "none");
                p.line(&format!("x{l}-{i}b"), (q.0 - r, q.1 + r), (q.0 + r, q.1 - r), &muted, 0.75, "solid", "none");
            } else {
                p.ellipse(&format!("n{l}-{i}"), q.0 - d / 2.0, q.1 - d / 2.0, d, d, &c);
            }
        }
    }
    let w = layers.len() as f64 * d + (layers.len() - 1) as f64 * lgap;
    p.core(w, h, LabelPos::Above)
}

/// One neuron: weighted inputs, Σ, the activation, the output.
fn nn_neuron(p: &mut Pen) -> Core {
    let math = p.ty_small();
    let tiny = p.ty_tiny();
    let c = p.main();
    let paper = p.f.paper.clone();
    let ink = p.f.ink.clone();
    let h = 40.0;
    let sx = 34.0;
    let sy = h / 2.0;
    let r = 8.0;
    for i in 0..3 {
        let y = 6.0 + i as f64 * 14.0;
        let id = p.ellipse(&format!("x{i}"), 0.0, y - 5.0, 10.0, 10.0, &paper);
        p.put(id, &format!("$x_{}$", i + 1), &tiny, false);
        p.line(&format!("w{i}"), (10.0, y), (sx - r * 0.75, sy + (y - sy) * 0.25), &ink, 0.6, "solid", "triangle");
        p.text_at(&format!("wl{i}"), &format!("$w_{}$", i + 1), 19.0, y + (sy - y) * 0.32 - 4.0, &tiny);
    }
    let id = p.ellipse("sum", sx - r, sy - r, 2.0 * r, 2.0 * r, &c);
    p.put(id, "Σ", &math, false);
    p.arrow("bias", (sx, sy - r - 9.0), (sx, sy - r));
    p.text_at("b", "$b$", sx + 4.0, sy - r - 9.0, &tiny);
    p.arrow("to-f", (sx + r, sy), (sx + r + 7.0, sy));
    let id = p.rect("act", sx + r + 7.0, sy - 7.0, 14.0, 14.0, &c);
    p.put(id, "$f$", &math, false);
    p.arrow("out", (sx + r + 21.0, sy), (sx + r + 31.0, sy));
    p.text_at("y", "$y$", sx + r + 35.0, sy, &math);
    let (w, hh) = normalise(p);
    p.core(w, hh, LabelPos::Above)
}

fn nn_pills(layers: &[usize], p: &mut Pen) -> Core {
    let (d, vgap, lgap, pad) = (8.0, 3.0, 26.0, 3.0);
    let tallest = *layers.iter().max().unwrap() as f64;
    let h = tallest * d + (tallest - 1.0) * vgap + 2.0 * pad;
    let xs: Vec<f64> = (0..layers.len()).map(|i| pad + d / 2.0 + i as f64 * (d + 2.0 * pad + lgap)).collect();
    let c = p.main();
    let paper = p.f.paper.clone();
    let centre = |l: usize, i: usize| {
        let ch = layers[l] as f64 * d + (layers[l] - 1) as f64 * vgap;
        (xs[l], (h - ch) / 2.0 + d / 2.0 + i as f64 * (d + vgap))
    };
    for l in 0..layers.len() - 1 {
        for a in 0..layers[l] {
            for b in 0..layers[l + 1] {
                let (pa, pb) = (centre(l, a), centre(l + 1, b));
                p.wire(&format!("w{l}-{a}-{b}"), (pa.0 + d / 2.0 + pad, pa.1), (pb.0 - d / 2.0 - pad, pb.1));
            }
        }
    }
    for (l, cnt) in layers.iter().enumerate() {
        let ch = *cnt as f64 * d + (*cnt as f64 - 1.0) * vgap + 2.0 * pad;
        let k = p.k();
        let (pp, s) = p.f.part(&paper, k);
        let mut o = shape(&format!("layer{l}"), "roundRect", (xs[l] - d / 2.0 - pad) * k, ((h - ch) / 2.0) * k, (d + 2.0 * pad) * k, ch * k, pp, s);
        round_adj(&mut o, 0.5);
        p.objs.push(o);
        for i in 0..*cnt {
            let q = centre(l, i);
            p.ellipse(&format!("n{l}-{i}"), q.0 - d / 2.0, q.1 - d / 2.0, d, d, &c);
        }
    }
    let w = layers.len() as f64 * (d + 2.0 * pad) + (layers.len() - 1) as f64 * lgap;
    p.core(w, h, LabelPos::Above)
}

/// Layers as upright bars joined by shaded fans (dense connections), sizes below.
fn nn_fans(n: &Node, p: &mut Pen) -> Core {
    let mut sizes = n.list("sizes");
    if sizes.is_empty() {
        sizes = vec!["784".into(), "256".into(), "64".into(), "10".into()];
    }
    let heights: Vec<f64> = (0..sizes.len()).map(|i| 44.0 * (1.0 - 0.6 * i as f64 / (sizes.len().max(2) - 1) as f64)).collect();
    let (bw, gap) = (6.0, 18.0);
    let h = 44.0;
    let ty = p.ty_tiny();
    let c = p.main();
    let fan = shade(&c, 0.55);
    for i in 0..sizes.len() - 1 {
        let (x0, x1) = (i as f64 * (bw + gap) + bw, (i + 1) as f64 * (bw + gap));
        let (a, b) = (heights[i], heights[i + 1]);
        let k = p.k();
        let pts = [(x0, (h - a) / 2.0), (x1, (h - b) / 2.0), (x1, (h + b) / 2.0), (x0, (h + a) / 2.0)];
        let line = p.f.muted.clone();
        p.objs.push(poly(&format!("fan{i}"), &pts.iter().map(|q| (q.0 * k, q.1 * k)).collect::<Vec<_>>(), true, Paint::Solid { color: fan.clone(), alpha: 0.6 }, Some(Stroke::new(&line, 0.4 * k))));
    }
    for (i, s) in sizes.iter().enumerate() {
        let x = i as f64 * (bw + gap);
        p.rect(&format!("layer{i}"), x, (h - heights[i]) / 2.0, bw, heights[i], &c);
        p.text(&format!("size{i}"), s, x + bw / 2.0, h + 1.5, &ty);
    }
    let (w, hh) = normalise(p);
    p.core(w, hh, LabelPos::Above)
}

/// An attention map: queries down the side, keys along the top, weights as shades.
fn nn_attention(p: &mut Pen) -> Core {
    let toks = 5;
    let cs = 7.0;
    let tiny = p.ty_tiny();
    let c = p.main();
    let c = if c.eq_ignore_ascii_case(&p.f.tensor) || lum(&c) > 0.85 { p.f.ink.clone() } else { c };
    let (ox, oy) = (9.0, 9.0);
    for r in 0..toks {
        p.text_at(&format!("q{r}"), &format!("$q_{}$", r + 1), ox - 4.5, oy + r as f64 * cs + cs / 2.0, &tiny);
        p.text_at(&format!("k{r}"), &format!("$k_{}$", r + 1), ox + r as f64 * cs + cs / 2.0, oy - 4.5, &tiny);
        for q in 0..toks {
            let wgt = if r == q { 0.0 } else { (0.35 + 0.13 * ((r * 7 + q * 3) % 5) as f64).min(0.95) };
            p.cell(&format!("a{r}{q}"), ox + q as f64 * cs, oy + r as f64 * cs, cs, cs, &shade(&c, wgt));
        }
    }
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Above)
}

/// Message passing on a graph: neighbours send to the highlighted node.
fn nn_graph(p: &mut Pen) -> Core {
    let pts = [(22.0, 18.0), (6.0, 6.0), (8.0, 31.0), (38.0, 6.0), (40.0, 30.0), (54.0, 17.0)];
    let edges = [(0, 1), (0, 2), (0, 3), (0, 4), (3, 5), (4, 5), (1, 2)];
    let c = p.main();
    let hi = p.ink_of(&c);
    let muted = p.f.muted.clone();
    for (a, b) in edges {
        let (pa, pb) = (pts[a], pts[b]);
        let to_centre = b == 0 || a == 0;
        if to_centre {
            let (s, t) = if a == 0 { (pb, pa) } else { (pa, pb) };
            let (dx, dy): (f64, f64) = (t.0 - s.0, t.1 - s.1);
            let l = (dx * dx + dy * dy).sqrt();
            p.line(&format!("m{a}{b}"), (s.0 + dx / l * 4.0, s.1 + dy / l * 4.0), (t.0 - dx / l * 6.0, t.1 - dy / l * 6.0), &hi, 0.75, "solid", "triangle");
        } else {
            p.line(&format!("e{a}{b}"), pa, pb, &muted, 0.6, "solid", "none");
        }
    }
    let paper = p.f.paper.clone();
    for (i, q) in pts.iter().enumerate() {
        let r = if i == 0 { 5.5 } else { 4.0 };
        p.ellipse(&format!("v{i}"), q.0 - r, q.1 - r, 2.0 * r, 2.0 * r, if i == 0 { &c } else { &paper });
    }
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Above)
}

/// The folded recurrent cell with its loop, = the same cell unrolled.
fn nn_recurrent(p: &mut Pen) -> Core {
    let tiny = p.ty_tiny();
    let small = p.ty_small();
    let c = p.col("cell");
    let ink = p.f.ink.clone();
    let (cw, ch) = (14.0, 13.0);
    let y = 12.0;
    let cell = |p: &mut Pen, name: &str, x: f64, t: &str| {
        let id = p.rect(name, x, y, cw, ch, &c);
        p.put(id, "A", &tiny, false);
        p.arrow(&format!("{name}-in"), (x + cw / 2.0, y + ch + 6.0), (x + cw / 2.0, y + ch));
        p.arrow(&format!("{name}-out"), (x + cw / 2.0, y), (x + cw / 2.0, y - 6.0));
        p.text(&format!("{name}-x"), &format!("$x_{t}$"), x + cw / 2.0, y + ch + 6.0, &tiny);
        p.text(&format!("{name}-h"), &format!("$h_{t}$"), x + cw / 2.0, y - 6.0 - tiny.size * 1.25 / p.cx.k, &tiny);
    };
    cell(p, "fold", 0.0, "t");
    p.arc("loop", (cw, y + ch * 0.3), (cw + 11.0, y + ch / 2.0), (cw, y + ch * 0.75), &ink, 0.75, "solid", "triangle");
    p.text_at("eq", "=", cw + 15.0, y + ch / 2.0, &small);
    let x0 = cw + 22.0;
    for (i, t) in ["0", "1", "2"].iter().enumerate() {
        let x = x0 + i as f64 * (cw + 9.0);
        cell(p, &format!("u{i}"), x, t);
        if i < 2 {
            p.arrow(&format!("h{i}"), (x + cw, y + ch / 2.0), (x + cw + 9.0, y + ch / 2.0));
        }
    }
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Above)
}

/// The inside of an LSTM cell: the cell-state line on top, the four gates
/// along the bottom, the point-wise × and +, tanh on the way out.
fn nn_lstm(p: &mut Pen) -> Core {
    let tiny = p.ty_tiny();
    let c = p.main();
    let gate = p.col("fc");
    let ink = p.f.ink.clone();
    let paper = p.f.paper.clone();
    let line = p.f.line.clone();
    let lh = tiny.size / p.cx.k * 1.25;
    let gw = p.tw("tanh", &tiny) + 4.0;
    let gh = lh + 3.0;
    let sp = gw + 6.0;
    let xs: Vec<f64> = (0..4).map(|i| 12.0 + i as f64 * sp).collect();
    let xo = xs[3] + sp + 2.0;
    let w = xo + 12.0;
    let (cs_y, op_y, gy, hy) = (9.0, 22.0, 34.0, 46.0);
    let h = 52.0;
    p.rect("cell", 0.0, 0.0, w, h, &shade(&c, 0.5));
    let k = p.k();
    let op = |p: &mut Pen, name: &str, x: f64, y: f64, s: &str| {
        let mut o = shape(name, "ellipse", (x - 3.8) * k, (y - 3.8) * k, 7.6 * k, 7.6 * k, Paint::solid(&paper), Some(Stroke::new(&line, 0.6 * k)));
        p.cx.put_text_pub(&mut o, s, &tiny, false);
        p.objs.push(o);
    };
    p.text_at("c-in", "$c_{t-1}$", -7.0, cs_y - 5.0, &tiny);
    p.text_at("c-out", "$c_t$", w + 6.0, cs_y - 5.0, &tiny);
    p.text_at("h-in", "$h_{t-1}$", -7.0, hy + 5.0, &tiny);
    p.text_at("h-out", "$h_t$", w + 6.0, op_y + 7.0, &tiny);
    // cell state: through × (forget) and + (input), out to the right
    p.line("c0", (-12.0, cs_y), (xs[0] - 3.8, cs_y), &ink, 1.0, "solid", "none");
    op(p, "f-mul", xs[0], cs_y, "×");
    p.line("c1", (xs[0] + 3.8, cs_y), (xs[2] - 3.8, cs_y), &ink, 1.0, "solid", "none");
    op(p, "add", xs[2], cs_y, "+");
    p.line("c2", (xs[2] + 3.8, cs_y), (w + 12.0, cs_y), &ink, 1.0, "solid", "triangle");
    // h and x enter along the bottom and feed every gate
    p.line("h-line", (-12.0, hy), (xs[3], hy), &ink, 0.75, "solid", "none");
    for (i, (s, x)) in ["σ", "σ", "tanh", "σ"].iter().zip(&xs).enumerate() {
        p.line(&format!("g{i}-in"), (*x, hy), (*x, gy + gh / 2.0), &ink, 0.6, "solid", "triangle");
        let id = p.rect(&format!("g{i}"), x - gw / 2.0, gy - gh / 2.0, gw, gh, &gate);
        p.put(id, s, &tiny, false);
    }
    // forget gate up to its ×; input gate × candidate, up to +
    p.line("f-up", (xs[0], gy - gh / 2.0), (xs[0], cs_y + 3.8), &ink, 0.6, "solid", "triangle");
    p.path("i-up", &[(xs[1], gy - gh / 2.0), (xs[1], op_y), (xs[2] - 3.8, op_y)], &ink, 0.6, "solid", "triangle");
    op(p, "i-mul", xs[2], op_y, "×");
    p.line("g-up", (xs[2], gy - gh / 2.0), (xs[2], op_y + 3.8), &ink, 0.6, "solid", "triangle");
    p.line("i-to-c", (xs[2], op_y - 3.8), (xs[2], cs_y + 3.8), &ink, 0.6, "solid", "triangle");
    // output: tanh of the cell state × the output gate
    let tw_ = gw;
    p.line("c-down", (xo, cs_y), (xo, op_y - gh / 2.0 - 4.5), &ink, 0.6, "solid", "triangle");
    let id = p.rect("tanh-c", xo - tw_ / 2.0, op_y - gh / 2.0 - 4.5, tw_, gh, &paper);
    p.put(id, "tanh", &tiny, false);
    p.line("tanh-down", (xo, op_y + gh / 2.0 - 4.5), (xo, gy - 3.8), &ink, 0.6, "solid", "triangle");
    p.line("o-right", (xs[3] + gw / 2.0, gy), (xo - 3.8, gy), &ink, 0.6, "solid", "triangle");
    op(p, "o-mul", xo, gy, "×");
    p.path("h-out", &[(xo + 3.8, gy), (w - 4.0, gy), (w - 4.0, op_y + 2.0), (w + 12.0, op_y + 2.0)], &ink, 0.75, "solid", "triangle");
    let (ww, hh) = normalise(p);
    p.core(ww, hh, LabelPos::Above)
}

/// A convolution: a kernel window on the input, projected to one output cell.
fn nn_conv(p: &mut Pen) -> Core {
    let c = p.main();
    let hi = p.ink_of(&c);
    let paper = p.f.paper.clone();
    let cs = 6.0;
    let n_in = 5;
    let (ix, iy) = (0.0, 0.0);
    for r in 0..n_in {
        for q in 0..n_in {
            let inside = (1..4).contains(&r) && (0..3).contains(&q);
            p.cell(&format!("in{r}{q}"), ix + q as f64 * cs, iy + r as f64 * cs, cs, cs, if inside { &c } else { &paper });
        }
    }
    let k = p.k();
    let win = shape("window", "rect", ix * k, (iy + cs) * k, 3.0 * cs * k, 3.0 * cs * k, Paint::None, Some(Stroke::new(&hi, 1.25 * k)));
    p.objs.push(win);
    let (ox, oy) = (48.0, 6.0);
    for r in 0..3 {
        for q in 0..3 {
            p.cell(&format!("out{r}{q}"), ox + q as f64 * cs, oy + r as f64 * cs, cs, cs, if r == 1 && q == 0 { &hi } else { &paper });
        }
    }
    let muted = p.f.muted.clone();
    for (a, b) in [((ix + 3.0 * cs, iy + cs), (ox + cs, oy + cs)), ((ix + 3.0 * cs, iy + 4.0 * cs), (ox + cs, oy + 2.0 * cs)), ((ix, iy + cs), (ox, oy + cs)), ((ix, iy + 4.0 * cs), (ox, oy + 2.0 * cs))] {
        p.line("proj", a, b, &muted, 0.4, "dash", "none");
    }
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Above)
}

/// A residual block: two weight layers, the identity around them, ⊕, ReLU.
fn nn_residual(p: &mut Pen) -> Core {
    let tiny = p.ty_tiny();
    let small = p.ty_small();
    let c = p.col("conv");
    let ink = p.f.ink.clone();
    let paper = p.f.paper.clone();
    let line = p.f.line.clone();
    let bw = p.tw("weight", &tiny) + 6.0;
    let bh = tiny.size / p.cx.k * 2.5 + 3.0;
    let y = 12.0;
    let my = y + bh / 2.0;
    p.text_at("x", "$x$", 3.0, my, &small);
    let x1 = 16.0;
    p.arrow("in", (7.0, my), (x1, my));
    let a = p.rect("w1", x1, y, bw, bh, &c);
    p.put(a, "weight\\\\layer", &tiny, false);
    let x2 = x1 + bw + 12.0;
    p.arrow("relu", (x1 + bw, my), (x2, my));
    p.text_at("relu-l", "relu", (x1 + bw + x2) / 2.0, y + bh + 3.5, &tiny);
    let b = p.rect("w2", x2, y, bw, bh, &c);
    p.put(b, "weight\\\\layer", &tiny, false);
    let sx = x2 + bw + 8.0;
    p.arrow("to-add", (x2 + bw, my), (sx - 0.3, my));
    let k = p.k();
    let mut o = shape("add", "ellipse", sx * k, (my - 4.0) * k, 8.0 * k, 8.0 * k, Paint::solid(&paper), Some(Stroke::new(&line, 0.75 * k)));
    p.cx.put_text_pub(&mut o, "+", &tiny, false);
    p.objs.push(o);
    p.path("identity", &[(11.0, my), (11.0, 3.0), (sx + 4.0, 3.0), (sx + 4.0, my - 4.0)], &ink, 0.75, "solid", "triangle");
    p.text_at("id", "identity", (x1 + sx) / 2.0, 0.0, &tiny);
    p.arrow("out", (sx + 8.0, my), (sx + 16.0, my));
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Above)
}

/// Multi-head attention: Linear for V, K, Q, h heads stacked, Concat, Linear.
fn nn_heads(p: &mut Pen) -> Core {
    let tiny = p.ty_tiny();
    let attn = p.col("attn");
    let fc = p.col("fc");
    let lh = tiny.size / p.cx.k * 1.25;
    let lin_w = p.tw("Linear", &tiny) + 5.0;
    let head_w = p.tw("Scaled Dot-Product", &tiny) + 7.0;
    let w = head_w.max(3.0 * lin_w + 8.0) + 6.0;
    let cx_ = w / 2.0;
    let bh = lh + 3.0;
    let top = p.rect("linear-out", cx_ - lin_w * 0.75, 0.0, lin_w * 1.5, bh, &fc);
    p.put(top, "Linear", &tiny, false);
    let cy = bh + 6.0;
    let cat = p.rect("concat", cx_ - lin_w * 0.75, cy, lin_w * 1.5, bh, &fc);
    p.put(cat, "Concat", &tiny, false);
    let hy = cy + bh + 9.0;
    let hh = 2.0 * lh + 3.0;
    for i in (0..3).rev() {
        let off = i as f64 * 2.5;
        let id = p.rect(&format!("head{i}"), cx_ - head_w / 2.0 + off, hy - off, head_w, hh, &attn);
        if i == 0 {
            p.put(id, "Scaled Dot-Product\\\\Attention", &tiny, false);
        }
    }
    p.text_at("h", "$h$", cx_ + head_w / 2.0 + 9.0, hy - 2.0, &tiny);
    p.arrow("up1", (cx_, hy - 5.0), (cx_, cy + bh));
    p.arrow("up2", (cx_, cy), (cx_, bh));
    let ly = hy + hh + 8.0;
    for (i, s) in ["V", "K", "Q"].iter().enumerate() {
        let x = cx_ + (i as f64 - 1.0) * (lin_w + 4.0) - lin_w / 2.0;
        let id = p.rect(&format!("lin{i}"), x, ly, lin_w, bh, &fc);
        p.put(id, "Linear", &tiny, false);
        p.arrow(&format!("v{i}"), (x + lin_w / 2.0, ly), (x + lin_w / 2.0, hy + hh));
        p.arrow(&format!("in{i}"), (x + lin_w / 2.0, ly + bh + 6.0), (x + lin_w / 2.0, ly + bh));
        p.text(&format!("n{i}"), s, x + lin_w / 2.0, ly + bh + 6.0, &tiny);
    }
    let (ww, hh2) = normalise(p);
    p.core(ww, hh2, LabelPos::Above)
}

/// Mixture of experts: a router sends the input to a few of the experts.
fn nn_moe(p: &mut Pen) -> Core {
    let tiny = p.ty_tiny();
    let c = p.main();
    let paper = p.f.paper.clone();
    let line = p.f.line.clone();
    let ink = p.f.ink.clone();
    let muted = p.f.muted.clone();
    let lh = tiny.size / p.cx.k * 1.25;
    let (ew, eh) = (p.tw("Expert 4", &tiny) + 6.0, lh + 3.5);
    let rw = p.tw("Router", &tiny) + 6.0;
    let gap = 3.0;
    let h = 4.0 * eh + 3.0 * gap;
    let r = p.rect("router", 0.0, h / 2.0 - eh * 0.75, rw, eh * 1.5, &c);
    p.put(r, "Router", &tiny, false);
    let ex = rw + 14.0;
    let active = [true, false, true, false];
    let sx = ex + ew + 12.0;
    for (i, on) in active.iter().enumerate() {
        let y = i as f64 * (eh + gap);
        let id = p.rect(&format!("e{i}"), ex, y, ew, eh, if *on { &c } else { &paper });
        p.put(id, &format!("Expert {}", i + 1), &tiny, false);
        let (col, w, dash) = if *on { (&ink, 0.75, "solid") } else { (&muted, 0.4, "dash") };
        p.line(&format!("r{i}"), (rw, h / 2.0), (ex, y + eh / 2.0), col, w, dash, "triangle");
        if *on {
            p.line(&format!("o{i}"), (ex + ew, y + eh / 2.0), (sx, h / 2.0 + if y + eh / 2.0 < h / 2.0 { -2.5 } else { 2.5 }), &ink, 0.6, "solid", "triangle");
        }
    }
    let k = p.k();
    let mut o = shape("sum", "ellipse", sx * k, (h / 2.0 - 4.0) * k, 8.0 * k, 8.0 * k, Paint::solid(&paper), Some(Stroke::new(&line, 0.75 * k)));
    p.cx.put_text_pub(&mut o, "+", &tiny, false);
    p.objs.push(o);
    let (w, hh) = normalise(p);
    p.core(w, hh, LabelPos::Above)
}

/// A restricted Boltzmann machine: visible and hidden rows, every pair joined.
fn nn_rbm(p: &mut Pen) -> Core {
    let tiny = p.ty_tiny();
    let c = p.main();
    let paper = p.f.paper.clone();
    let d = 9.0;
    let (nv, nh) = (5, 3);
    let gap = 5.0;
    let wv = nv as f64 * d + (nv - 1) as f64 * gap;
    let hx = |i: usize| (wv - (nh as f64 * d + (nh - 1) as f64 * gap)) / 2.0 + i as f64 * (d + gap) + d / 2.0;
    let vx = |i: usize| i as f64 * (d + gap) + d / 2.0;
    let (hy, vy) = (d / 2.0, d / 2.0 + 26.0);
    for a in 0..nh {
        for b in 0..nv {
            p.wire(&format!("w{a}{b}"), (hx(a), hy + d / 2.0), (vx(b), vy - d / 2.0));
        }
    }
    for a in 0..nh {
        p.ellipse(&format!("h{a}"), hx(a) - d / 2.0, hy - d / 2.0, d, d, &c);
    }
    for b in 0..nv {
        p.ellipse(&format!("v{b}"), vx(b) - d / 2.0, vy - d / 2.0, d, d, &paper);
    }
    p.text_at("hl", "$h$", wv + 6.0, hy, &tiny);
    p.text_at("vl", "$v$", wv + 6.0, vy, &tiny);
    let (w, h) = normalise(p);
    p.core(w, h, LabelPos::Above)
}
