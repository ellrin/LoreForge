//! Finish: how a module is painted, never what is drawn.
//!
//! A *style* is a construction (SegNet slabs, a transformer block, an unrolled
//! RNN); a *finish* is the paint any style accepts: fill or outline or
//! gradient, translucency, colour by layer type or one colour, corner radius,
//! line weight and colours. The finish comes from the theme (`[node] finish`,
//! then `[modules.<kind>] finish / line_w / radius`) and last from the node
//! (`finish=outline,translucent`, `fill=`, `line=`, `line_w=`, `radius=`).
//!
//! Words: `solid` | `outline` | `gradient` | `flat` (fill without outline),
//! `translucent` | `opaque`, `typed` (colour by layer type / role, with the
//! colour legend) | `mono` (one colour), `round` | `square`, `thin` | `bold`.

use lore_canvas::{Paint, Stop, Stroke};

use crate::spec::Node;
use crate::theme::{Theme, shade};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Solid,
    Outline,
    Gradient,
    Flat,
}

/// Every finish word, for parsers and the LLM's list.
pub const WORDS: [&str; 12] = ["solid", "outline", "gradient", "flat", "translucent", "opaque", "typed", "mono", "round", "square", "thin", "bold"];

#[derive(Debug, Clone, PartialEq)]
pub struct Finish {
    pub look: Look,
    pub alpha: f64,
    /// Colour parts by layer type / role (and name the colours in the legend).
    pub typed: bool,
    /// Corner radius of rectangular parts, a fraction of the short side.
    pub radius: f64,
    /// Line weight in pt at natural size.
    pub lw: f64,
    /// The module's own colours (node, then kind, then role).
    pub fill: String,
    pub line: String,
    pub text: String,
    pub paper: String,
    pub tensor: String,
    pub ink: String,
    pub muted: String,
}

fn hex(n: &Node, k: &str) -> Option<String> {
    let v = n.str(k)?.trim_start_matches('#');
    (v == "none" || (v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()))).then(|| v.to_uppercase())
}

impl Finish {
    /// Apply finish words (space or comma separated). Unknown words are an error.
    pub fn apply(&mut self, words: &str) -> Result<(), String> {
        for w in words.split([' ', ',', '+']).map(str::trim).filter(|w| !w.is_empty()) {
            match w {
                "solid" => self.look = Look::Solid,
                "outline" => self.look = Look::Outline,
                "gradient" => self.look = Look::Gradient,
                "flat" => self.look = Look::Flat,
                "translucent" => self.alpha = 0.6,
                "opaque" => self.alpha = 1.0,
                "typed" | "colour" | "color" => self.typed = true,
                "mono" => self.typed = false,
                "round" => self.radius = 0.2,
                "square" => self.radius = 0.0,
                "thin" => self.lw *= 0.6,
                "bold" => self.lw *= 1.6,
                other => return Err(format!("finish: unknown word {other:?} (allowed: {})", WORDS.join(", "))),
            }
        }
        Ok(())
    }

    /// The finish of a node: theme, then its kind, then the node itself.
    /// Words that do not parse are ignored here (`notation::lint` reports them).
    pub fn of(n: &Node, theme: &Theme) -> Finish {
        let r = theme.role(n.role_or_default());
        let m = theme.modules.get(&n.module);
        let mut f = Finish {
            look: Look::Solid,
            alpha: 1.0,
            typed: false,
            radius: theme.node.radius,
            lw: theme.node.line_w,
            fill: m.and_then(|m| m.fill.clone()).unwrap_or_else(|| r.fill.clone()),
            line: m.and_then(|m| m.line.clone()).unwrap_or_else(|| r.line.clone()),
            text: m.and_then(|m| m.text.clone()).unwrap_or_else(|| r.text.clone()),
            paper: theme.colors.paper.clone(),
            tensor: theme.colors.tensor.clone(),
            ink: theme.colors.ink.clone(),
            muted: theme.colors.muted.clone(),
        };
        let _ = f.apply(&theme.node.finish);
        if let Some(m) = m {
            if let Some(w) = &m.finish {
                let _ = f.apply(w);
            }
            if let Some(v) = m.line_w {
                f.lw = v;
            }
            if let Some(v) = m.radius {
                f.radius = v;
            }
        }
        if let Some(w) = n.str("finish") {
            let _ = f.apply(w);
        } else if let Some(serde_json::Value::Array(a)) = n.params.get("finish") {
            for w in a.iter().filter_map(|v| v.as_str()) {
                let _ = f.apply(w);
            }
        }
        if let Some(c) = hex(n, "fill") {
            f.fill = c;
        }
        if let Some(c) = hex(n, "line") {
            f.line = c;
        }
        if let Some(v) = n.num("line_w") {
            f.lw = v;
        }
        if let Some(v) = n.num("radius") {
            f.radius = v;
        }
        f
    }

    /// The module's own colour for parts that must show on paper.
    pub fn base(&self) -> String {
        if self.fill == "none" || self.fill.eq_ignore_ascii_case(&self.paper) { self.tensor.clone() } else { self.fill.clone() }
    }

    fn outline_colour(&self, col: &str) -> String {
        // an outline drawing keeps the type colour in the line, darkened when it is pale
        if self.typed && !col.eq_ignore_ascii_case(&self.base()) {
            let lum = |h: &str| (0..3).map(|i| u8::from_str_radix(h.get(i * 2..i * 2 + 2).unwrap_or("00"), 16).unwrap_or(0) as f64).sum::<f64>() / 765.0;
            if lum(col) > 0.75 { shade(col, -0.45) } else { col.to_string() }
        } else {
            self.line.clone()
        }
    }

    /// Fill and outline of a part painted in `col`, at scale `k`.
    pub fn part(&self, col: &str, k: f64) -> (Paint, Option<Stroke>) {
        let stroke = |c: &str, w: f64| if c == "none" || w <= 0.0 { None } else { Some(Stroke::new(c, w * k)) };
        match self.look {
            Look::Solid => (solid(col, self.alpha), stroke(&self.line, self.lw)),
            Look::Flat => (solid(col, self.alpha), None),
            Look::Outline => (solid(&self.paper, self.alpha), stroke(&self.outline_colour(col), self.lw * 1.25)),
            Look::Gradient => {
                let stops = vec![Stop { pos: 0.0, color: shade(col, 0.6), alpha: self.alpha }, Stop { pos: 1.0, color: col.to_uppercase(), alpha: self.alpha }];
                (Paint::Gradient { stops, angle: 90.0, path: false }, stroke(&self.line, self.lw))
            }
        }
    }

    /// A face of a 3-D block: `t` = how much lighter (> 0) or darker (< 0) than the front.
    pub fn face(&self, col: &str, t: f64, k: f64) -> (Paint, Option<Stroke>) {
        match self.look {
            Look::Outline => self.part(col, k),
            Look::Gradient if t != 0.0 => (solid(&shade(col, t), self.alpha), Some(Stroke::new(&self.line, self.lw * 0.6 * k))),
            Look::Flat => (solid(&shade(col, t), self.alpha), None),
            _ => (solid(&shade(col, t), self.alpha), if self.line == "none" { None } else { Some(Stroke::new(&self.line, self.lw * 0.6 * k)) }),
        }
    }
}

fn solid(col: &str, alpha: f64) -> Paint {
    if col == "none" || col.is_empty() { Paint::None } else { Paint::Solid { color: col.to_uppercase(), alpha } }
}
