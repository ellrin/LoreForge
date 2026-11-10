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
