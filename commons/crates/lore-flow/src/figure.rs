//! The figure the flow editor makes (`docs/flow-figures.md`): the L1 text plus the
//! author's layout, colours, line meanings and legend. This is what rides with the
//! document (a SlideComposer figure group, PaperComposer's `<stem>.flow.json`) so the
//! figure re-opens in the editor.
//!
//! ```json
//! { "lore_flow": 1,
//!   "dsl": "fig seg\npanel a \"Stage 1\"\n  x: image \"$x$\"\n …",
//!   "layout": "two-left-big-right",          // a layout template id ("" = automatic)
//!   "dirs": { "b": "down" },                  // a block turned top-to-bottom
//!   "theme": "classic-bw",
//!   "types": [{ "id": "conv", "label": "conv block", "fill": "F9D5E5", "line": "B03A6E" }],
//!   "blocks": { "a": "conv" }, "nodes": { "net": "conv" },
//!   "lines": { "supervision": { "color": "2E7D32", "dash": "dash", "meaning": "ground truth" } },
//!   "legend": { "show": "on", "pos": "below", "rows": [{ "key": "line:supervision", "label": "ground truth" }, { "key": "type:conv", "label": "conv block" }] },
//!   "frame": [672, 357],
//!   "discussion": { … the answers of the steps … } }
//! ```
//!
//! Layout templates and themes are data: the public neutral ones here
//! (`layouts.json`, `themes/classic-bw.toml`) and the author's own in
//! `private/shared/flow/{layouts,themes}/` ([`Catalogue::load`]).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::{LayoutOptions, LegendChoice, LegendMark, LegendRow, PanelTemplate, Rect};
use crate::modules::Library;
use crate::spec::{EdgeKind, Graph};
use crate::theme::Theme;
use crate::{Drawing, Figure, MathSource};

pub const VERSION: u32 = 1;
/// The public layout templates.
pub const LAYOUTS: &str = include_str!("../layouts.json");

fn one() -> u32 {
    VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FlowSpec {
    #[serde(default = "one")]
    pub lore_flow: u32,
    /// The L1 text (`dsl`): what is in the figure.
    #[serde(default)]
    pub dsl: String,
    /// Layout template id; empty = the arrangement that fits best.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub layout: String,
    /// Block (panel) id → `right` | `down`, over the text and the template.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dirs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    /// Kinds of block with their colours (the legend's boxes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<BlockType>,
    /// Block (panel) id → type id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blocks: BTreeMap<String, String>,
    /// Node id → type id (a module coloured as its kind of block).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, String>,
    /// Line kind → how it looks and what it means.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lines: BTreeMap<String, LineLook>,
    #[serde(default)]
    pub legend: LegendSpec,
    /// The box the figure fits in (pt); `None` = a 16:9 slide's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<[f64; 2]>,
    /// The discussion that led here (kept so the editor re-opens on it).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub discussion: Value,
    /// Keys a newer editor wrote: kept.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BlockType {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub fill: String,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LineLook {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// `solid` `dash` `sysDash` `sysDot` `lgDash` `dashDot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// What the line means (the legend's words).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meaning: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LegendSpec {
    /// `auto` (lore-flow decides), `on` (the rows), `off`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub show: String,
    /// `below` (default) | `above`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pos: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<LegendEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LegendEntry {
    /// `line:<kind>`, `type:<id>` or `block:<panel id>`.
    pub key: String,
    #[serde(default)]
    pub label: String,
}

/// A layout template (public `layouts.json` or the author's `layouts/*.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FlowLayout {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// `row` (blocks side by side), `column` (stacked), `grid` (`cols` per row);
    /// empty when `panels` gives the rects.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arrange: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cols: usize,
    /// Direction inside every block: `right` | `down` | empty (the one that fits).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    /// Block rects as fractions of the figure (exactly this many blocks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<Rect>,
    /// Direction per block, in order (over `dir`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<String>,
    /// From the author's private folder.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
}
fn is_zero(v: &usize) -> bool {
    *v == 0
}

impl FlowLayout {
    /// Usable for a figure of `n` blocks.
    pub fn fits(&self, n: usize) -> bool {
        if self.panels.is_empty() {
            match self.arrange.as_str() {
                "grid" => n > self.cols.max(1),
                _ => n >= 1,
            }
        } else {
            self.panels.len() == n
        }
    }

    /// Put the template on the graph: arrangement, panel template, directions.
    pub fn apply(&self, g: &mut Graph) -> Option<PanelTemplate> {
        g.arrange.clear();
        g.template.clear();
        let ids: Vec<String> = g.panels.iter().map(|p| p.id.clone()).collect();
        let mut tpl = None;
        if !self.panels.is_empty() {
            g.template = self.id.clone();
            tpl = Some(PanelTemplate { id: self.id.clone(), name: self.name.clone(), panels: self.panels.clone() });
        } else {
            match self.arrange.as_str() {
                "row" => g.template = "cols".into(),
                "column" => g.template = "rows".into(),
                "grid" => g.arrange = ids.chunks(self.cols.max(1)).map(|c| c.to_vec()).collect(),
                _ => {}
            }
        }
        for (i, p) in g.panels.iter_mut().enumerate() {
            let d = self.dirs.get(i).filter(|d| !d.is_empty()).unwrap_or(&self.dir);
            if !d.is_empty() {
                p.dir = d.clone();
            }
        }
        tpl
    }
}

/// Themes, layout templates and drawings on offer.
#[derive(Debug, Clone)]
pub struct Catalogue {
    /// (id, name, theme).
    pub themes: Vec<(String, String, Theme)>,
    pub layouts: Vec<FlowLayout>,
    pub library: Library,
}

impl Default for Catalogue {
    fn default() -> Self {
        Catalogue::builtin()
    }
}

impl Catalogue {
    /// The public ones only.
    pub fn builtin() -> Catalogue {
        let t = Theme::classic_bw();
        let layouts: Vec<FlowLayout> = serde_json::from_str(LAYOUTS).expect("bundled layouts.json parses");
        Catalogue { themes: vec![(t.id.clone(), t.name.clone(), t)], layouts, library: Library::default() }
    }

    /// The public ones plus the author's (`dir` = `private/shared/flow`):
    /// `layouts/*.json` (one template or a list), `themes/*.toml`, `modules/`.
    /// A private id replaces the public one. Files that do not parse are skipped.
    pub fn load(dir: Option<&Path>) -> Catalogue {
        Catalogue::load_all(dir.into_iter().collect::<Vec<_>>().as_slice())
    }

    /// As [`Catalogue::load`] over several folders in order (the shared one, then an
    /// app's own `flow/`); a later one wins. `templates/` is read like `layouts/`.
    pub fn load_all(dirs: &[&Path]) -> Catalogue {
        let mut c = Catalogue::builtin();
        for dir in dirs {
            c.add_dir(dir);
        }
        c
    }

    fn add_dir(&mut self, dir: &Path) {
        let c = self;
        for f in files(&dir.join("layouts"), "json").into_iter().chain(files(&dir.join("templates"), "json")) {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let list: Vec<FlowLayout> = serde_json::from_str::<Vec<FlowLayout>>(&text).or_else(|_| serde_json::from_str::<FlowLayout>(&text).map(|l| vec![l])).unwrap_or_default();
            for mut l in list {
                if l.id.is_empty() || (!l.panels.is_empty() && PanelTemplate::from_json(&serde_json::json!({ "id": l.id, "panels": l.panels }).to_string()).is_err()) {
                    continue;
                }
                l.private = true;
                c.layouts.retain(|x| x.id != l.id);
                c.layouts.push(l);
            }
        }
        for f in files(&dir.join("themes"), "toml") {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            if let Ok(t) = Theme::from_toml(&text) {
                c.themes.retain(|x| x.0 != t.id);
                c.themes.push((t.id.clone(), if t.name.is_empty() { t.id.clone() } else { t.name.clone() }, t));
            }
        }
        if dir.join("modules").is_dir()
            && let Ok(lib) = Library::load_dir(&dir.join("modules"))
        {
            for t in lib.kinds().iter().flat_map(|k| lib.styles(k).into_iter().filter_map(|st| lib.get(k, &st).cloned()).collect::<Vec<_>>()) {
                let _ = c.library.add(t);
            }
        }
    }

    pub fn theme(&self, id: &str) -> Theme {
        self.themes.iter().find(|t| t.0 == id).map(|t| t.2.clone()).unwrap_or_else(Theme::classic_bw)
    }

    pub fn layout(&self, id: &str) -> Option<&FlowLayout> {
        self.layouts.iter().find(|l| l.id == id)
    }
}

fn files(dir: &Path, ext: &str) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == ext)).collect();
    v.sort();
    v
}

fn hex(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches('#');
    (s == "none" || (s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit()))).then(|| if s == "none" { s.to_string() } else { s.to_uppercase() })
}

const DASHES: [&str; 7] = ["solid", "dash", "sysDash", "sysDot", "lgDash", "dashDot", "lgDashDot"];

impl FlowSpec {
    pub fn from_value(v: Value) -> Result<FlowSpec, String> {
        let s: FlowSpec = serde_json::from_value(v).map_err(|e| format!("flow spec: {e}"))?;
        if s.lore_flow > VERSION {
            return Err(format!("flow spec version {} is newer than this program ({VERSION})", s.lore_flow));
        }
        Ok(s)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// The L1 graph of the text.
    pub fn graph(&self, lib: &Library) -> Result<Graph, String> {
        crate::dsl::parse_with(&self.dsl, &lib.kinds()).map_err(|e| e.to_string())
    }

    /// Problems a figure would be drawn with anyway (a type that does not exist, a
    /// colour that is not one, a block id the text lacks …), in the author's words.
    pub fn problems(&self, g: &Graph) -> Vec<String> {
        let mut out = Vec::new();
        let types: Vec<&str> = self.types.iter().map(|t| t.id.as_str()).collect();
        for t in &self.types {
            if hex(&t.fill).is_none() || hex(&t.line).is_none() {
                out.push(format!("類型 {} 的顏色不是 6 位十六進位", t.id));
            }
        }
        for (b, t) in &self.blocks {
            if !g.panels.iter().any(|p| &p.id == b) {
                out.push(format!("沒有 block {b}"));
            }
            if !types.contains(&t.as_str()) {
                out.push(format!("沒有類型 {t}"));
            }
        }
        for (n, t) in &self.nodes {
            if g.node(n).is_none() {
                out.push(format!("沒有節點 {n}"));
            }
            if !types.contains(&t.as_str()) {
                out.push(format!("沒有類型 {t}"));
            }
        }
        for (b, d) in &self.dirs {
            if !g.panels.iter().any(|p| &p.id == b) {
                out.push(format!("沒有 block {b}"));
            }
            if d != "right" && d != "down" {
                out.push(format!("block {b} 的方向只能是 right 或 down"));
            }
        }
        for (k, l) in &self.lines {
            if EdgeKind::parse(k).is_none() {
                out.push(format!("沒有線的種類 {k}"));
            }
            if l.color.as_deref().is_some_and(|c| hex(c).is_none()) {
                out.push(format!("{k} 的顏色不是 6 位十六進位"));
            }
            if l.dash.as_deref().is_some_and(|d| !DASHES.contains(&d)) {
                out.push(format!("{k} 的線型只能是 {}", DASHES.join("、")));
            }
        }
        out
    }

    /// The graph, theme and layout options this spec draws with.
    pub fn prepare(&self, cat: &Catalogue, frame: Rect) -> Result<(Graph, Theme, LayoutOptions), String> {
        let mut g = self.graph(&cat.library)?;
        let mut theme = cat.theme(&self.theme);
        let mut opt = LayoutOptions { frame, ..Default::default() };
        // layout template, then the per-block directions
        if let Some(l) = cat.layout(&self.layout).filter(|l| l.fits(g.panels.len())) {
            opt.template = l.apply(&mut g);
        }
        for p in &mut g.panels {
            if let Some(d) = self.dirs.get(&p.id).filter(|d| *d == "right" || *d == "down") {
                p.dir = d.clone();
            }
        }
        // block colours: the panels' fills and outlines in order
        let ty = |id: &str| self.types.iter().find(|t| t.id == id);
        if !self.blocks.is_empty() {
            let (fills, lines): (Vec<String>, Vec<String>) = g
                .panels
                .iter()
                .enumerate()
                .map(|(i, p)| match self.blocks.get(&p.id).and_then(|t| ty(t)) {
                    Some(t) => (hex(&t.fill).unwrap_or_else(|| theme.panel_fill(i).to_string()), hex(&t.line).unwrap_or_else(|| theme.panel_line(i).to_string())),
                    None => (theme.panel_fill(i).to_string(), theme.panel_line(i).to_string()),
                })
                .unzip();
            theme.panel.fills = fills;
            theme.panel.lines = lines;
        }
        for n in &mut g.nodes {
            if let Some(t) = self.nodes.get(&n.id).and_then(|t| ty(t)) {
                if let Some(f) = hex(&t.fill) {
                    n.params.insert("fill".into(), Value::String(f));
                }
                if let Some(l) = hex(&t.line) {
                    n.params.insert("line".into(), Value::String(l));
                }
            }
        }
        // line looks
        for (k, l) in &self.lines {
            let Some(kind) = EdgeKind::parse(k) else { continue };
            let mut e = theme.edge(kind).clone();
            if let Some(c) = l.color.as_deref().and_then(hex) {
                e.color = c;
            }
            if let Some(d) = l.dash.as_deref().filter(|d| DASHES.contains(d)) {
                e.dash = d.to_string();
            }
            if let Some(w) = l.width.filter(|w| *w > 0.0 && *w < 20.0) {
                e.width = w;
            }
            theme.edges.insert(k.clone(), e);
        }
        // legend
        opt.legend = match self.legend.show.as_str() {
            "off" => Some(LegendChoice::default()),
            _ if !self.legend.rows.is_empty() => {
                let rows = self.legend.rows.iter().filter_map(|r| self.legend_row(r, &g, &theme)).collect();
                Some(LegendChoice { rows, above: self.legend.pos == "above" })
            }
            _ => None,
        };
        Ok((g, theme, opt))
    }

    fn legend_row(&self, r: &LegendEntry, g: &Graph, theme: &Theme) -> Option<LegendRow> {
        let (kind, id) = r.key.split_once(':')?;
        let mark = match kind {
            "line" => LegendMark::Line(EdgeKind::parse(id)?),
            "type" => {
                let t = self.types.iter().find(|t| t.id == id)?;
                LegendMark::Box { fill: hex(&t.fill)?, line: hex(&t.line)? }
            }
            "block" => {
                let i = g.panels.iter().position(|p| p.id == id)?;
                LegendMark::Box { fill: theme.panel_fill(i).to_string(), line: theme.panel_line(i).to_string() }
            }
            _ => return None,
        };
        let label = if r.label.trim().is_empty() { self.default_label(&r.key, g) } else { r.label.clone() };
        Some(LegendRow { label, mark })
    }

    fn default_label(&self, key: &str, g: &Graph) -> String {
        match key.split_once(':') {
            Some(("line", k)) => self.lines.get(k).map(|l| l.meaning.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| EdgeKind::parse(k).map(|k| k.label().to_string()).unwrap_or_default()),
            Some(("type", t)) => self.types.iter().find(|x| x.id == t).map(|x| if x.label.is_empty() { x.id.clone() } else { x.label.clone() }).unwrap_or_default(),
            Some(("block", b)) => g.panels.iter().find(|p| p.id == b).map(|p| if p.title.is_empty() { p.id.clone() } else { p.title.clone() }).unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Legend rows that would make sense for this figure: the kinds of block
    /// coloured, then every non-data line kind used (the author picks).
    pub fn legend_suggestions(&self, g: &Graph) -> Vec<LegendEntry> {
        let mut out: Vec<LegendEntry> = Vec::new();
        for t in &self.types {
            if self.blocks.values().chain(self.nodes.values()).any(|x| *x == t.id) {
                out.push(LegendEntry { key: format!("type:{}", t.id), label: self.default_label(&format!("type:{}", t.id), g) });
            }
        }
        for k in g.line_kinds() {
            if k != EdgeKind::Data {
                let key = format!("line:{}", k.as_str());
                out.push(LegendEntry { label: self.default_label(&key, g), key });
            }
        }
        out
    }

    /// The box the figure fits in, at (0, 0).
    pub fn frame_rect(&self) -> Rect {
        match self.frame {
            Some([w, h]) if w > 10.0 && h > 10.0 => Rect::new(0.0, 0.0, w, h),
            _ => Rect::new(0.0, 0.0, 672.0, 357.0),
        }
    }
}

/// One line per block: `block 1 (title): a -> b -> c`, the flow edges in order
/// (what the author reads to check the figure before drawing it).
pub fn linear(g: &Graph) -> Vec<String> {
    let mut out = Vec::new();
    let mut groups: Vec<(String, String)> = g.panels.iter().map(|p| (p.id.clone(), p.title.clone())).collect();
    if g.nodes.iter().any(|n| n.panel.is_empty()) {
        groups.push((String::new(), String::new()));
    }
    for (i, (pid, title)) in groups.iter().enumerate() {
        let ids: Vec<&str> = g.nodes.iter().filter(|n| &n.panel == pid).map(|n| n.id.as_str()).collect();
        // chains: follow flow edges from nodes with no flow edge into them (inside the block)
        let flow: Vec<(&str, &str)> = g.edges.iter().filter(|e| e.kind.is_flow() && ids.contains(&e.from.as_str()) && ids.contains(&e.to.as_str())).map(|e| (e.from.as_str(), e.to.as_str())).collect();
        let mut used = std::collections::BTreeSet::new();
        let mut chains: Vec<String> = Vec::new();
        for start in ids.iter().filter(|id| !flow.iter().any(|(_, t)| t == *id)) {
            let mut chain = vec![*start];
            used.insert(*start);
            let mut cur = *start;
            while let Some((_, t)) = flow.iter().find(|(f, t)| *f == cur && !used.contains(t)) {
                chain.push(t);
                used.insert(t);
                cur = t;
            }
            chains.push(chain.join(" -> "));
        }
        for id in &ids {
            if !used.contains(id) {
                chains.push(id.to_string());
            }
        }
        let head = if pid.is_empty() { "其他".to_string() } else if title.is_empty() { format!("block {}", i + 1) } else { format!("block {} ({title})", i + 1) };
        out.push(format!("{head}: {}", chains.join(" ; ")));
    }
    out
}

/// Draw a spec into `frame` (pt).
pub fn render(spec: &FlowSpec, cat: &Catalogue, notation: Option<&lore_notation::Notation>, math: &dyn MathSource, frame: Rect, first_id: u32) -> Result<(Figure, Graph), String> {
    let (g, theme, options) = spec.prepare(cat, frame)?;
    let d = Drawing { theme, library: cat.library.clone(), notation, math, options, first_id, pictures: None };
    let fig = crate::draw(&g, &d).map_err(|e| e.to_string())?;
    Ok((fig, g))
}

/// A plain SVG of a drawn figure, cropped to it (previews, thumbnails).
pub fn preview_svg(fig: &Figure, width_px: f64) -> String {
    let b = fig.layout.bounds.inflate(6.0);
    let mut objs = fig.objects().to_vec();
    let dx = crate::modules::emu(-b.x);
    let dy = crate::modules::emu(-b.y);
    for o in &mut objs {
        o.frame.x += dx;
        o.frame.y += dy;
    }
    let zoom = if b.w > 0.0 { width_px / b.w } else { 1.0 };
    crate::preview::svg(&objs, b.w, b.h, zoom)
}

/// The figure as an SVG file of its print size (`width`/`height` in mm, 1 pt = 25.4/72 mm): a
/// paper figure (lore-plot's PDF export reads the mm size). With the `frame` it was drawn in
/// (a paper's column width), the file is that wide, so `\includegraphics[width=\linewidth]`
/// prints it at the designed size; only the empty space above and below is cropped (that
/// changes no scale). Without one, it is cropped to the drawing.
pub fn svg_mm(fig: &Figure, frame: Option<Rect>) -> String {
    let c = fig.layout.bounds.inflate(6.0);
    let b = match frame {
        // the frame's width (never narrower than the drawing), the drawing's height
        Some(f) if f.w > 10.0 => {
            let x0 = f.x.min(c.x);
            let x1 = (f.x + f.w).max(c.x + c.w);
            Rect::new(x0, c.y, x1 - x0, c.h)
        }
        _ => c,
    };
    let svg = {
        let mut objs = fig.objects().to_vec();
        let dx = crate::modules::emu(-b.x);
        let dy = crate::modules::emu(-b.y);
        for o in &mut objs {
            o.frame.x += dx;
            o.frame.y += dy;
        }
        crate::preview::svg(&objs, b.w, b.h, 1.0)
    };
    let mm = |v: f64| format!("{:.2}mm", v * 25.4 / 72.0);
    let Some(start) = svg.find("<svg") else { return svg };
    let end = start + svg[start..].find('>').unwrap_or(0);
    let head = &svg[start..end];
    let set = |head: &str, k: &str, v: &str| -> String {
        match head.find(&format!(" {k}=\"")) {
            Some(i) => {
                let j = i + k.len() + 3;
                let close = j + head[j..].find('"').unwrap_or(0);
                format!("{}{v}{}", &head[..j], &head[close..])
            }
            None => head.to_string(),
        }
    };
    let head = set(&set(head, "width", &mm(b.w)), "height", &mm(b.h));
    format!("{}{}{}", &svg[..start], head, &svg[end..])
}

/// The drawn objects as one picture: a group named `flow:<fig>` that keeps its
/// aspect ratio (text, line widths and arrowheads scale with it) and carries the
/// spec, ready to insert (`id` = the group's id).
pub fn as_picture(fig: &Figure, fig_id: &str, spec: &FlowSpec, id: u32) -> lore_canvas::Object {
    let mut children = fig.objects().to_vec();
    crate::build::retarget_for_pptx(&mut children);
    let b = lore_canvas::geom::bounds(children.iter().map(|o| &o.frame));
    lore_canvas::Object {
        id,
        name: format!("flow:{fig_id}"),
        frame: b.clone(),
        hidden: false,
        ph: None,
        slot: None,
        kind: lore_canvas::Kind::Group(lore_canvas::GroupObj { ch: b, children, lock_aspect: true, data: Some(spec.to_value()) }),
        src: None,
    }
}

/// Neutral block colours for figures on white paper: (name, fill, outline).
/// Never the app theme's colours. The author's own list: `private/shared/flow/palette.json`.
pub const PALETTE: &[(&str, &str, &str)] = &[
    ("粉", "F9D5E5", "B03A6E"),
    ("藍", "D6E6F7", "2F6DB0"),
    ("綠", "DCEFD6", "3C8D2F"),
    ("橘", "FCE6CC", "C26A10"),
    ("紫", "E6DDF3", "6B4FA0"),
    ("黃", "FBF1C7", "A8860B"),
    ("青", "D3EFEF", "1F8080"),
    ("灰", "EDEDED", "595959"),
    ("白", "FFFFFF", "000000"),
];
