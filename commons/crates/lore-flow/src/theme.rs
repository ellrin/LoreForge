//! Themes: every colour, line style, type size and spacing of a figure.
//! A theme file is TOML and only lists what it changes from `classic-bw`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::spec::EdgeKind;

pub const CLASSIC_BW: &str = include_str!("../themes/classic-bw.toml");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub font: Font,
    pub colors: Colors,
    pub panel: PanelStyle,
    pub node: NodeStyle,
    pub roles: BTreeMap<String, RoleStyle>,
    pub edges: BTreeMap<String, EdgeStyle>,
    /// Per module kind (`[modules.unet]`): colours and art style, over the role's.
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleStyle>,
    /// Colour and legend name of each layer type (`[layers.conv]`), used by
    /// drawings coloured by layer type or role.
    #[serde(default)]
    pub layers: BTreeMap<String, LayerStyle>,
    pub shade: Shade,
    pub space: Space,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Font {
    /// Module names and labels.
    pub family: String,
    /// Variables and equations (Office Math).
    pub math: String,
    /// East Asian text; empty = the deck's own.
    #[serde(default)]
    pub ea: String,
    /// Panel titles; empty = `family`.
    #[serde(default)]
    pub title: String,
    /// Captions, sizes, line labels, legend; empty = `family`.
    #[serde(default)]
    pub caption: String,
    pub size: f64,
    pub title_scale: f64,
    pub small_scale: f64,
    /// Sizes in pt that override the scales when > 0.
    #[serde(default)]
    pub title_size: f64,
    #[serde(default)]
    pub small_size: f64,
    #[serde(default = "yes")]
    pub title_bold: bool,
}
fn yes() -> bool {
    true
}

/// Colours and style of one kind of module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModuleStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Finish words (see [`crate::finish`]), after `[node] finish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

/// One layer type: its colour and the name the colour legend gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayerStyle {
    pub color: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Colors {
    pub ink: String,
    pub paper: String,
    pub muted: String,
    pub image: String,
    pub tensor: String,
    pub cycle: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelStyle {
    pub fill: String,
    pub line: String,
    pub line_w: f64,
    pub dash: String,
    pub radius: f64,
    pub title: String,
    /// Fills used in turn by the panels of one figure (overrides `fill`).
    #[serde(default)]
    pub fills: Vec<String>,
    /// Outline colours used in turn (overrides `line`).
    #[serde(default)]
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeStyle {
    pub line_w: f64,
    pub radius: f64,
    pub loss: String,
    /// Finish of every module (see [`crate::finish`]): `solid`, `outline`,
    /// `gradient`, `flat`, `translucent`, `typed` / `mono` …
    #[serde(default)]
    pub finish: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleStyle {
    pub fill: String,
    pub line: String,
    #[serde(default = "black")]
    pub text: String,
}
fn black() -> String {
    "000000".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeStyle {
    pub color: String,
    pub width: f64,
    #[serde(default = "solid")]
    pub dash: String,
    #[serde(default = "none")]
    pub head: String,
    #[serde(default = "none")]
    pub tail: String,
}
fn solid() -> String {
    "solid".into()
}
fn none() -> String {
    "none".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shade {
    pub top: f64,
    pub side: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub gap_x: f64,
    pub gap_y: f64,
    pub pad: f64,
    pub panel_gap: f64,
}

fn merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

fn hex_ok(s: &str) -> bool {
    s == "none" || (s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

impl Theme {
    pub fn classic_bw() -> Theme {
        toml::from_str(CLASSIC_BW).expect("built-in theme parses")
    }

    /// A theme file laid over `classic-bw`.
    pub fn from_toml(text: &str) -> Result<Theme, String> {
        let mut base: toml::Value = toml::from_str(CLASSIC_BW).expect("built-in theme parses");
        let over: toml::Value = toml::from_str(text).map_err(|e| format!("theme: {e}"))?;
        merge(&mut base, over);
        let t: Theme = base.try_into().map_err(|e| format!("theme: {e}"))?;
        t.check()?;
        Ok(t)
    }

    fn check(&self) -> Result<(), String> {
        let mut cols: Vec<(&str, &str)> = vec![
            ("colors.ink", &self.colors.ink),
            ("colors.paper", &self.colors.paper),
            ("colors.muted", &self.colors.muted),
            ("colors.image", &self.colors.image),
            ("colors.tensor", &self.colors.tensor),
            ("panel.fill", &self.panel.fill),
            ("panel.line", &self.panel.line),
        ];
        for c in self.colors.cycle.iter().chain(&self.panel.fills).chain(&self.panel.lines) {
            cols.push(("colour list", c));
        }
        for r in self.roles.values() {
            cols.extend([("roles.*.fill", r.fill.as_str()), ("roles.*.line", r.line.as_str()), ("roles.*.text", r.text.as_str())]);
        }
        for e in self.edges.values() {
            cols.push(("edges.*.color", &e.color));
        }
        for m in self.modules.values() {
            for c in [&m.fill, &m.line, &m.text].into_iter().flatten() {
                cols.push(("modules.*", c));
            }
        }
        for l in self.layers.values() {
            cols.push(("layers.*.color", &l.color));
        }
        if let Some((k, v)) = cols.into_iter().find(|(_, v)| !hex_ok(v)) {
            return Err(format!("theme: {k} = {v:?} is not a 6-digit hex colour"));
        }
        if let Some(k) = self.edges.keys().find(|k| EdgeKind::parse(k).is_none()) {
            return Err(format!("theme: [edges.{k}] is not a line kind"));
        }
        if !self.roles.contains_key("default") || !self.edges.contains_key("data") {
            return Err("theme: [roles.default] and [edges.data] are required".into());
        }
        Ok(())
    }

    pub fn role(&self, role: &str) -> &RoleStyle {
        self.roles.get(role).or_else(|| self.roles.get("default")).expect("checked")
    }
    pub fn edge(&self, kind: EdgeKind) -> &EdgeStyle {
        self.edges.get(kind.as_str()).or_else(|| self.edges.get("data")).expect("checked")
    }
    pub fn title_size(&self) -> f64 {
        if self.font.title_size > 0.0 { self.font.title_size } else { self.font.size * self.font.title_scale }
    }
    pub fn small_size(&self) -> f64 {
        if self.font.small_size > 0.0 { self.font.small_size } else { self.font.size * self.font.small_scale }
    }
    pub fn panel_fill(&self, i: usize) -> &str {
        if self.panel.fills.is_empty() { &self.panel.fill } else { &self.panel.fills[i % self.panel.fills.len()] }
    }
    pub fn panel_line(&self, i: usize) -> &str {
        if self.panel.lines.is_empty() { &self.panel.line } else { &self.panel.lines[i % self.panel.lines.len()] }
    }
    /// Colour of a layer type (`conv`, `pool`, `enc` …); unknown types take
    /// the `other` entry, else the tensor colour.
    pub fn layer(&self, key: &str) -> &str {
        self.layers.get(key).or_else(|| self.layers.get("other")).map(|l| l.color.as_str()).unwrap_or(&self.colors.tensor)
    }
    /// Legend name of a layer type.
    pub fn layer_label(&self, key: &str) -> String {
        match self.layers.get(key) {
            Some(l) if !l.label.is_empty() => l.label.clone(),
            _ => key.to_string(),
        }
    }
    /// Colour of the `i`-th class (layer kinds, cells).
    pub fn cycle(&self, i: usize) -> &str {
        if self.colors.cycle.is_empty() { &self.colors.ink } else { &self.colors.cycle[i % self.colors.cycle.len()] }
    }
}

/// Lighten (`t > 0`, towards white) or darken (`t < 0`, towards black) a hex colour.
pub fn shade(hex: &str, t: f64) -> String {
    let p = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64;
    let f = |v: f64| {
        let o = if t >= 0.0 { v + (255.0 - v) * t } else { v * (1.0 + t) };
        o.round().clamp(0.0, 255.0) as u8
    };
    format!("{:02X}{:02X}{:02X}", f(p(0)), f(p(2)), f(p(4)))
}

/// A colour by name for a block, a line or a region's background, from the document's own
/// palette (never the app's screen theme): `(fill, line)` as `RRGGBB`, the fill pale and the
/// line strong. Names: red, orange, yellow, green, teal, blue, purple, pink, brown, grey,
/// black. An `RRGGBB` is given back as both. `None` = not a colour.
pub fn named(name: &str) -> Option<(String, String)> {
    let n = name.trim();
    let hex = n.trim_start_matches('#');
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let h = hex.to_uppercase();
        return Some((h.clone(), h));
    }
    let (fill, line) = match n.to_ascii_lowercase().as_str() {
        "red" => ("FDE3E3", "C62828"),
        "orange" => ("FFE9D0", "D84315"),
        "yellow" => ("FFF5C2", "9A7400"),
        "green" => ("DDF3DF", "2E7D32"),
        "teal" => ("D3F1ED", "00796B"),
        "blue" => ("DCEBFB", "1565C0"),
        "purple" => ("EADCF6", "7B1FA2"),
        "pink" => ("FCE0EC", "C2185B"),
        "brown" => ("EBE0D7", "6D4C41"),
        "grey" | "gray" => ("E6E6E6", "616161"),
        "black" => ("D9D9D9", "000000"),
        _ => return None,
    };
    Some((fill.to_string(), line.to_string()))
}

#[cfg(test)]
mod tests {
    use super::named;

    fn lum(hex: &str) -> f64 {
        let c = |i: usize| {
            let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(0) + 0.7152 * c(2) + 0.0722 * c(4)
    }
    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn named_colours_are_pale_and_strong() {
        for n in ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "brown", "grey", "gray", "black"] {
            let (fill, line) = named(n).unwrap_or_else(|| panic!("{n}"));
            assert!(contrast(&fill, "000000") >= 7.0, "{n} fill {fill} under black text: {}", contrast(&fill, "000000"));
            assert!(contrast(&line, "FFFFFF") >= 3.0, "{n} line {line} on white: {}", contrast(&line, "FFFFFF"));
        }
        assert_eq!(named("Grey"), named("gray"));
    }

    #[test]
    fn a_hex_is_taken_as_it_is() {
        assert_eq!(named("#ff8800"), Some(("FF8800".into(), "FF8800".into())));
        assert_eq!(named("12aB34"), Some(("12AB34".into(), "12AB34".into())));
    }

    #[test]
    fn an_unknown_name_is_no_colour() {
        for n in ["", "bluish", "12345", "GGGGGG", "none"] {
            assert_eq!(named(n), None, "{n}");
        }
    }
}
