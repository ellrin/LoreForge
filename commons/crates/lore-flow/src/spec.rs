//! L1 — the data-flow graph. This is everything an author (or an LLM) says
//! about a figure: nodes, edges, panels and variables. No coordinates, no
//! colours; those come from the layout and the theme.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Module kinds the program can draw (the closed list an LLM picks from).
/// Module templates loaded from a library extend it at run time.
pub const MODULES: [(&str, &str); 19] = [
    ("box", "a named step or model: rectangle with the label inside (params: shape=rect|round|trapezoid, vertical=true for an upright bar)"),
    ("op", "operator circle; label is one of + x . C (add, multiply, element-wise, concatenate) or a short word such as Or"),
    ("loss", "a loss or metric; label is its math, e.g. $\\mathcal L_{seg}$"),
    ("label", "free text without a frame (an output class, a note)"),
    ("image", "one picture / mask / heat map (params: look=photo|mask|heatmap|plot)"),
    ("chart", "a small chart drawn by the plotting tool (params: chart=bell|bells|membership|bars|scatter|surface, n, labels=[..]): a distribution, a set of curves, a bar chart, clusters"),
    ("icon", "a small picture from the icon sets (params: icon=<set:name>, e.g. tabler:server, healthicons:doctor-outline)"),
    ("image-stack", "several pictures stacked with an offset (params: n)"),
    ("feature-stack", "feature maps / tensor channels stacked (params: n, look=flat|3d)"),
    ("vector-bar", "a vector or embedding as a row/column of cells (params: n, dir=v|h)"),
    ("matrix", "a grid of cells (params: rows, cols)"),
    ("layer-bars", "a network drawn as a row of upright layer bars (params: layers=[names], taper=true)"),
    ("cuboid-seq", "a network drawn as 3-D blocks (params: shape=down|up|hourglass|flat, n, or dims=[[w,h,d],..])"),
    ("unet", "a flat U-shaped encoder-decoder with skip lines (params: depth)"),
    ("encoder", "an encoder that narrows along the flow (style: trapezoid, bars, cubes, slabs = SegNet, planes, lenet, fc, resnet, dense, unet, transformer, rnn, vit, fpn, vae, graph, layers)"),
    ("decoder", "a decoder that widens along the flow (same styles as encoder, mirrored)"),
    ("autoencoder", "an encoder, its bottleneck and the mirrored decoder as one drawing with one colour legend (style: as encoder; params: latent=vector|neurons|context|none)"),
    ("nn", "a small network of units (style: circles, labelled, dropout, neuron, pills, fans, attention, graph, recurrent, lstm, conv, residual, heads, moe, rbm; params: layers=[3,4,2])"),
    ("brace", "a brace or dimension mark that gathers several things (params: dir=right|left|up|down)"),
];

/// What a node is in the story (decides its colour through the theme).
pub const ROLES: [&str; 9] = ["input", "model", "op", "data", "output", "gt", "loss", "merge", "note"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// Main data flow.
    #[default]
    Data,
    /// Ground truth / label source into a loss.
    Supervision,
    /// Gradient or weight going back to a model; drawn below the row.
    Feedback,
    /// A model's output that is a loss.
    LossOut,
    /// Prediction compared with a target (double-headed).
    Compare,
    /// Weak or optional path.
    Optional,
    /// Shared weights (no arrowhead).
    Shared,
    /// Skip connection around the trunk.
    Skip,
    /// One of several inputs of a concatenation.
    Concat,
    Down,
    Up,
    /// Between panels; runs in the gaps between them.
    Cross,
    /// A stage transition (heavy arrow).
    Stage,
    /// Repetition (dotted, no head).
    Repeat,
    /// Projection lines of an enlarged detail.
    Zoom,
}

impl EdgeKind {
    pub const ALL: [EdgeKind; 15] = [
        EdgeKind::Data,
        EdgeKind::Supervision,
        EdgeKind::Feedback,
        EdgeKind::LossOut,
        EdgeKind::Compare,
        EdgeKind::Optional,
        EdgeKind::Shared,
        EdgeKind::Skip,
        EdgeKind::Concat,
        EdgeKind::Down,
        EdgeKind::Up,
        EdgeKind::Cross,
        EdgeKind::Stage,
        EdgeKind::Repeat,
        EdgeKind::Zoom,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Data => "data",
            EdgeKind::Supervision => "supervision",
            EdgeKind::Feedback => "feedback",
            EdgeKind::LossOut => "loss-out",
            EdgeKind::Compare => "compare",
            EdgeKind::Optional => "optional",
            EdgeKind::Shared => "shared",
            EdgeKind::Skip => "skip",
            EdgeKind::Concat => "concat",
            EdgeKind::Down => "down",
            EdgeKind::Up => "up",
            EdgeKind::Cross => "cross",
            EdgeKind::Stage => "stage",
            EdgeKind::Repeat => "repeat",
            EdgeKind::Zoom => "zoom",
        }
    }
    pub fn parse(s: &str) -> Option<EdgeKind> {
        EdgeKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
    /// Edges that order the nodes along the flow. The others are drawn after
    /// the layout is fixed.
    pub fn is_flow(self) -> bool {
        matches!(self, EdgeKind::Data | EdgeKind::LossOut | EdgeKind::Concat | EdgeKind::Down | EdgeKind::Up | EdgeKind::Stage | EdgeKind::Optional | EdgeKind::Repeat)
    }
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Data => "Data flow",
            EdgeKind::Supervision => "Supervision",
            EdgeKind::Feedback => "Feedback",
            EdgeKind::LossOut => "Loss",
            EdgeKind::Compare => "Compare",
            EdgeKind::Optional => "Optional",
            EdgeKind::Shared => "Shared weights",
            EdgeKind::Skip => "Skip connection",
            EdgeKind::Concat => "Concatenation",
            EdgeKind::Down => "Downsample",
            EdgeKind::Up => "Upsample",
            EdgeKind::Cross => "Across stages",
            EdgeKind::Stage => "Stage",
            EdgeKind::Repeat => "Repeat",
            EdgeKind::Zoom => "Detail",
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(w: &f64) -> bool {
    *w == 0.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Graph {
    /// Short id of the figure; object names are `flow:<fig>/<node>`.
    #[serde(default = "default_fig")]
    pub fig: String,
    /// `rows`, `cols`, or the id of a panel template. Empty = automatic.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub template: String,
    /// Rows of panel ids (an explicit arrangement; wins over `template`).
    /// `_` stands for the nodes that are in no panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arrange: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<Panel>,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Frames round groups of blocks, nestable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
    /// Blocks the author wants on one line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub align: Vec<Align>,
    /// Variables the figure introduces that the notation registry does not have yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vars_new: Vec<VarNew>,
    /// `Some(true)` always, `Some(false)` never, `None` = when four or more
    /// kinds of non-data lines are used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<bool>,
}
fn default_fig() -> String {
    "fig".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Panel {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Flow direction inside the panel: `right` (default) or `down`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    /// No frame and no title (loose nodes that sit beside the panels).
    #[serde(default, skip_serializing_if = "is_false")]
    pub bare: bool,
    /// Frame line: empty = solid as the theme draws it, `dashed`, `dotted`, `solid`, `none`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub line: String,
    /// Fill: a name of [`crate::theme::named`] or `RRGGBB`; empty = as the theme.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tint: String,
    /// Round corners.
    #[serde(default, skip_serializing_if = "is_false")]
    pub round: bool,
    /// Where the title goes: empty = inside, top-left; `top-left`, `top` (inside, centred),
    /// `above`, `below` (outside the frame), `left` (upright strip), `none` (no title, no room).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Node {
    pub id: String,
    /// One of [`MODULES`] or a library template kind.
    #[serde(default = "default_module")]
    pub module: String,
    /// One of [`ROLES`]; empty = decided from the module.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// Text with `$math$` and `@notation-id` references.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Notation id of the variable this node stands for (shown as its label
    /// when there is none, else under it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub var: String,
    /// Small text under the node (a size, a remark).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub panel: String,
    /// Art style of the module (a built-in style or a saved drawing of the
    /// library); empty = the theme's choice for this kind.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}
fn default_module() -> String {
    "box".into()
}

impl Node {
    pub fn num(&self, k: &str) -> Option<f64> {
        self.params.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    }
    pub fn str(&self, k: &str) -> Option<&str> {
        self.params.get(k).and_then(|v| v.as_str())
    }
    pub fn flag(&self, k: &str) -> bool {
        match self.params.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.as_str(), "true" | "yes" | "on" | "1"),
            Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
            _ => false,
        }
    }
    pub fn list(&self, k: &str) -> Vec<String> {
        match self.params.get(k) {
            Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect(),
            Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
            _ => vec![],
        }
    }
    /// The role, or the one its module implies.
    pub fn role_or_default(&self) -> &str {
        if !self.role.is_empty() {
            return &self.role;
        }
        match self.module.as_str() {
            "op" => "op",
            "brace" => "note",
            "loss" => "loss",
            "label" => "note",
            "image" | "icon" | "image-stack" | "feature-stack" | "vector-bar" | "matrix" => "data",
            _ => "model",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Edge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The line's own colour: a name of [`crate::theme::named`] (`orange`, `blue` …) or
    /// `RRGGBB`. Empty = the colour of its kind. One path of a figure, one colour.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// Drawn as one smooth curve between its ends instead of right-angled runs.
    #[serde(default, skip_serializing_if = "is_false")]
    pub curve: bool,
    /// The line leaves the middle of another line, not a block: that line's
    /// [`crate::figure::edge_key`] (`a>b`). `from` is then that line's `from` (for the graph's
    /// order) and the router starts at a point on that line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub off: String,
    /// The line's own width in pt at natural size; 0 = the width of its kind.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: f64,
}

/// A region: the blocks of one kind of thing, framed together (the author's "依照標籤給區域").
/// Regions nest (`parent`); a region lies inside one panel, or loose when the figure has none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Region {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The region this one lies in; empty = directly in its panel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parent: String,
    /// Node ids framed by this region (the nodes of its sub-regions are framed too, without
    /// being listed here).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// The frame's line: `solid` (default), `dashed`, `dotted`, `none`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub line: String,
    /// Background: a name of [`crate::theme::named`] (its pale fill is used) or `RRGGBB`;
    /// empty = no background.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tint: String,
    /// Round corners.
    #[serde(default, skip_serializing_if = "is_false")]
    pub round: bool,
    /// Where the title goes: `top-left` (default, inside), `top` (centred inside), `above`
    /// (centred over the frame), `below`, `left` (upright beside it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title_at: String,
}

/// Blocks put on one line: `row` = the same height on the page (their middles), `col` = one
/// above the other (their centres share an x). Across panels and regions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Align {
    /// `row` | `col`.
    pub axis: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VarNew {
    pub id: String,
    pub latex: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meaning: String,
}

/// A variable's LaTeX without the maths delimiters a model sometimes wraps it in (`$\mathbf{x}$`,
/// `\(\hat{y}\)`): `latex` is the bare maths (the registry, `@id` and the figure put `$…$` around it).
pub fn bare_latex(s: &str) -> String {
    let t = s.trim();
    let inner = t
        .strip_prefix("$$").and_then(|r| r.strip_suffix("$$"))
        .or_else(|| t.strip_prefix('$').and_then(|r| r.strip_suffix('$')))
        .or_else(|| t.strip_prefix("\\(").and_then(|r| r.strip_suffix("\\)")))
        .or_else(|| t.strip_prefix("\\[").and_then(|r| r.strip_suffix("\\]")));
    match inner {
        // only one pair around the whole: `$a$ and $b$` is text, not one formula
        Some(i) if !i.contains('$') && !i.trim().is_empty() => i.trim().to_string(),
        _ => t.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SpecError {
    #[error("figure has no nodes")]
    Empty,
    #[error("bad id {0:?} (letters, digits, - and _ only)")]
    BadId(String),
    #[error("node id {0:?} is used twice")]
    DuplicateNode(String),
    #[error("panel id {0:?} is used twice")]
    DuplicatePanel(String),
    #[error("node {node:?} uses module {module:?}, which is not in the library")]
    UnknownModule { node: String, module: String },
    #[error("node {node:?} has role {role:?} (allowed: input, model, op, data, output, gt, loss, merge, note)")]
    UnknownRole { node: String, role: String },
    #[error("node {node:?} is in panel {panel:?}, which does not exist")]
    UnknownPanel { node: String, panel: String },
    #[error("edge {from} -> {to}: node {missing:?} does not exist")]
    UnknownNode { from: String, to: String, missing: String },
    #[error("edge {0} -> {0} connects a node to itself")]
    SelfLoop(String),
    #[error("arrange names panel {0:?}, which does not exist")]
    ArrangeUnknown(String),
    #[error("arrange leaves out panel {0:?}")]
    ArrangeMissing(String),
    #[error("region id {0:?} is used twice")]
    DuplicateRegion(String),
    #[error("region id {0:?} is also the id of a node or a panel")]
    RegionIdClash(String),
    #[error("region {region:?} lists {member:?}, which is not a node")]
    RegionMemberUnknown { region: String, member: String },
    #[error("region {region:?} lies in {parent:?}, which is not a region")]
    RegionParentUnknown { region: String, parent: String },
    #[error("region {0:?} is inside itself")]
    RegionCycle(String),
    #[error("node {node:?} is listed in regions {a:?} and {b:?}, and neither lies inside the other")]
    RegionOverlap { node: String, a: String, b: String },
    #[error("region {0:?} has blocks of two different panels")]
    RegionTwoPanels(String),
    #[error("region {region:?} has line {value:?} (allowed: solid, dashed, dotted, none)")]
    RegionLine { region: String, value: String },
    #[error("region {region:?} has title position {value:?} (allowed: top-left, top, above, below, left)")]
    RegionTitleAt { region: String, value: String },
    #[error("panel {panel:?} has line {value:?} (allowed: solid, dashed, dotted, none)")]
    PanelLine { panel: String, value: String },
    #[error("panel {panel:?} has title position {value:?} (allowed: top-left, top, above, below, left, none)")]
    PanelTitleAt { panel: String, value: String },
    #[error("line {from} -> {to} has width {width} (allowed: 0 to 12)")]
    EdgeWidth { from: String, to: String, width: f64 },
    #[error("align {0:?} is not row or col")]
    AlignAxis(String),
    #[error("align names {0:?}, which is not a node")]
    AlignUnknown(String),
    #[error("align needs at least two blocks")]
    AlignTooFew,
    #[error("line {edge} leaves {off:?}, which is not another line of the figure")]
    EdgeOffUnknown { edge: String, off: String },
    #[error("line {0} cannot leave itself")]
    EdgeOffSelf(String),
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Graph {
    pub fn from_json(s: &str) -> Result<Graph, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Structural check. `extra_modules` = template kinds of the module library.
    pub fn validate(&self, extra_modules: &[String]) -> Result<(), SpecError> {
        if self.nodes.is_empty() {
            return Err(SpecError::Empty);
        }
        if !valid_id(&self.fig) {
            return Err(SpecError::BadId(self.fig.clone()));
        }
        let mut panels = BTreeSet::new();
        for p in &self.panels {
            if !valid_id(&p.id) || p.id == "_" {
                return Err(SpecError::BadId(p.id.clone()));
            }
            if !panels.insert(p.id.as_str()) {
                return Err(SpecError::DuplicatePanel(p.id.clone()));
            }
        }
        let mut ids = BTreeSet::new();
        for n in &self.nodes {
            if !valid_id(&n.id) {
                return Err(SpecError::BadId(n.id.clone()));
            }
            if !ids.insert(n.id.as_str()) {
                return Err(SpecError::DuplicateNode(n.id.clone()));
            }
            if !MODULES.iter().any(|(m, _)| *m == n.module) && !extra_modules.contains(&n.module) {
                return Err(SpecError::UnknownModule { node: n.id.clone(), module: n.module.clone() });
            }
            if !n.role.is_empty() && !ROLES.contains(&n.role.as_str()) {
                return Err(SpecError::UnknownRole { node: n.id.clone(), role: n.role.clone() });
            }
            if !n.panel.is_empty() && !panels.contains(n.panel.as_str()) {
                return Err(SpecError::UnknownPanel { node: n.id.clone(), panel: n.panel.clone() });
            }
        }
        for e in &self.edges {
            for end in [&e.from, &e.to] {
                if !ids.contains(end.as_str()) {
                    return Err(SpecError::UnknownNode { from: e.from.clone(), to: e.to.clone(), missing: end.clone() });
                }
            }
            if e.from == e.to {
                return Err(SpecError::SelfLoop(e.from.clone()));
            }
        }
        if !self.arrange.is_empty() {
            let mut seen = BTreeSet::new();
            for id in self.arrange.iter().flatten() {
                if id != "_" && !panels.contains(id.as_str()) {
                    return Err(SpecError::ArrangeUnknown(id.clone()));
                }
                seen.insert(id.as_str());
            }
            if let Some(p) = self.panels.iter().find(|p| !seen.contains(p.id.as_str())) {
                return Err(SpecError::ArrangeMissing(p.id.clone()));
            }
        }
        self.validate_regions(&ids, &panels)?;
        self.validate_edge_off()?;
        Ok(())
    }

    fn validate_regions(&self, ids: &BTreeSet<&str>, panels: &BTreeSet<&str>) -> Result<(), SpecError> {
        let mut regions: BTreeSet<&str> = BTreeSet::new();
        for p in &self.panels {
            if !p.line.is_empty() && !matches!(p.line.as_str(), "solid" | "dashed" | "dotted" | "none") {
                return Err(SpecError::PanelLine { panel: p.id.clone(), value: p.line.clone() });
            }
            if !p.title_at.is_empty() && !matches!(p.title_at.as_str(), "top-left" | "top" | "above" | "below" | "left" | "none") {
                return Err(SpecError::PanelTitleAt { panel: p.id.clone(), value: p.title_at.clone() });
            }
        }
        for e in &self.edges {
            if !(0.0..=12.0).contains(&e.width) {
                return Err(SpecError::EdgeWidth { from: e.from.clone(), to: e.to.clone(), width: e.width });
            }
        }
        for r in &self.regions {
            if !valid_id(&r.id) {
                return Err(SpecError::BadId(r.id.clone()));
            }
            if !regions.insert(r.id.as_str()) {
                return Err(SpecError::DuplicateRegion(r.id.clone()));
            }
            if ids.contains(r.id.as_str()) || panels.contains(r.id.as_str()) {
                return Err(SpecError::RegionIdClash(r.id.clone()));
            }
        }
        for r in &self.regions {
            for m in &r.members {
                if !ids.contains(m.as_str()) {
                    return Err(SpecError::RegionMemberUnknown { region: r.id.clone(), member: m.clone() });
                }
            }
            if !r.parent.is_empty() && !regions.contains(r.parent.as_str()) {
                return Err(SpecError::RegionParentUnknown { region: r.id.clone(), parent: r.parent.clone() });
            }
            if !r.line.is_empty() && !matches!(r.line.as_str(), "solid" | "dashed" | "dotted" | "none") {
                return Err(SpecError::RegionLine { region: r.id.clone(), value: r.line.clone() });
            }
            if !r.title_at.is_empty() && !matches!(r.title_at.as_str(), "top-left" | "top" | "above" | "below" | "left") {
                return Err(SpecError::RegionTitleAt { region: r.id.clone(), value: r.title_at.clone() });
            }
        }
        let parent_of = |id: &str| self.regions.iter().find(|r| r.id == id).map(|r| r.parent.as_str()).unwrap_or("");
        for r in &self.regions {
            let mut cur = r.parent.as_str();
            let mut steps = 0;
            while !cur.is_empty() {
                if cur == r.id || steps > self.regions.len() {
                    return Err(SpecError::RegionCycle(r.id.clone()));
                }
                cur = parent_of(cur);
                steps += 1;
            }
        }
        // is `a` inside `b` (or the same)?
        let inside = |a: &str, b: &str| {
            let mut cur = a;
            while !cur.is_empty() {
                if cur == b {
                    return true;
                }
                cur = parent_of(cur);
            }
            false
        };
        let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
        for r in &self.regions {
            for m in &r.members {
                match owner.get(m.as_str()) {
                    Some(&other) if other != r.id && !inside(other, &r.id) && !inside(&r.id, other) => {
                        return Err(SpecError::RegionOverlap { node: m.clone(), a: other.to_string(), b: r.id.clone() });
                    }
                    Some(&other) if inside(other, &r.id) => {} // keep the inner one
                    _ => {
                        owner.insert(m.as_str(), r.id.as_str());
                    }
                }
            }
            let mut seen: Option<&str> = None;
            for m in &r.members {
                if let Some(n) = self.node(m)
                    && !n.panel.is_empty()
                {
                    match seen {
                        Some(p) if p != n.panel => return Err(SpecError::RegionTwoPanels(r.id.clone())),
                        _ => seen = Some(n.panel.as_str()),
                    }
                }
            }
        }
        for a in &self.align {
            if a.axis != "row" && a.axis != "col" {
                return Err(SpecError::AlignAxis(a.axis.clone()));
            }
            if let Some(x) = a.ids.iter().find(|i| !ids.contains(i.as_str())) {
                return Err(SpecError::AlignUnknown(x.clone()));
            }
            if a.ids.len() < 2 {
                return Err(SpecError::AlignTooFew);
            }
        }
        Ok(())
    }

    fn validate_edge_off(&self) -> Result<(), SpecError> {
        let mut count: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        let mut keys: Vec<String> = Vec::new();
        for e in &self.edges {
            let n = count.entry((e.from.as_str(), e.to.as_str())).or_insert(0);
            *n += 1;
            keys.push(crate::figure::edge_key(&e.from, &e.to, *n));
        }
        for (e, key) in self.edges.iter().zip(&keys) {
            if e.off.is_empty() {
                continue;
            }
            if &e.off == key {
                return Err(SpecError::EdgeOffSelf(key.clone()));
            }
            if !keys.contains(&e.off) {
                return Err(SpecError::EdgeOffUnknown { edge: key.clone(), off: e.off.clone() });
            }
        }
        Ok(())
    }

    /// Line kinds in use (decides the automatic legend).
    pub fn line_kinds(&self) -> Vec<EdgeKind> {
        let set: BTreeSet<EdgeKind> = self.edges.iter().map(|e| e.kind).collect();
        set.into_iter().collect()
    }
    pub fn wants_legend(&self) -> bool {
        match self.legend {
            Some(b) => b,
            None => self.line_kinds().iter().filter(|k| **k != EdgeKind::Data).count() >= 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Graph {
        let n = |id: &str, panel: &str| Node { id: id.into(), module: "box".into(), panel: panel.into(), ..Default::default() };
        Graph {
            fig: "t".into(),
            panels: vec![Panel { id: "p".into(), ..Default::default() }, Panel { id: "q".into(), ..Default::default() }],
            nodes: vec![n("a", "p"), n("b", "p"), n("c", "q"), n("d", "")],
            edges: vec![Edge { from: "a".into(), to: "b".into(), ..Default::default() }, Edge { from: "b".into(), to: "c".into(), ..Default::default() }],
            ..Default::default()
        }
    }
    fn region(id: &str, parent: &str, members: &[&str]) -> Region {
        Region { id: id.into(), parent: parent.into(), members: members.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }
    fn refuses(g: Graph) -> SpecError {
        g.validate(&[]).unwrap_err()
    }

    #[test]
    fn a_valid_figure_with_regions_passes() {
        let mut g = base();
        g.regions = vec![region("o", "", &["a"]), region("i", "o", &["a", "b"])];
        g.align = vec![Align { axis: "row".into(), ids: vec!["a".into(), "c".into()] }];
        g.edges.push(Edge { from: "a".into(), to: "d".into(), off: "a>b".into(), ..Default::default() });
        assert_eq!(g.validate(&[]), Ok(()));
    }
    #[test]
    fn validation_refuses_a_bad_region_id() {
        let mut g = base();
        g.regions = vec![region("bad id", "", &["a"])];
        assert!(matches!(refuses(g), SpecError::BadId(_)));
    }
    #[test]
    fn validation_refuses_a_duplicate_region() {
        let mut g = base();
        g.regions = vec![region("r", "", &["a"]), region("r", "", &["b"])];
        assert!(matches!(refuses(g), SpecError::DuplicateRegion(_)));
    }
    #[test]
    fn validation_refuses_a_region_named_like_a_node_or_panel() {
        for id in ["a", "p"] {
            let mut g = base();
            g.regions = vec![region(id, "", &["a"])];
            assert!(matches!(refuses(g), SpecError::RegionIdClash(_)));
        }
    }
    #[test]
    fn validation_refuses_a_member_that_is_not_a_node() {
        let mut g = base();
        g.regions = vec![region("r", "", &["zz"])];
        assert!(matches!(refuses(g), SpecError::RegionMemberUnknown { .. }));
    }
    #[test]
    fn validation_refuses_a_parent_that_is_not_a_region() {
        let mut g = base();
        g.regions = vec![region("r", "nope", &["a"])];
        assert!(matches!(refuses(g), SpecError::RegionParentUnknown { .. }));
    }
    #[test]
    fn validation_refuses_a_region_inside_itself() {
        let mut g = base();
        g.regions = vec![region("r", "s", &["a"]), region("s", "r", &["b"])];
        assert!(matches!(refuses(g), SpecError::RegionCycle(_)));
        let mut g = base();
        g.regions = vec![region("r", "r", &["a"])];
        assert!(matches!(refuses(g), SpecError::RegionCycle(_)));
    }
    #[test]
    fn validation_refuses_a_node_in_two_unrelated_regions() {
        let mut g = base();
        g.regions = vec![region("r", "", &["a"]), region("s", "", &["a"])];
        assert!(matches!(refuses(g), SpecError::RegionOverlap { .. }));
    }
    #[test]
    fn validation_refuses_a_region_across_two_panels() {
        let mut g = base();
        g.regions = vec![region("r", "", &["a", "c"])];
        assert!(matches!(refuses(g), SpecError::RegionTwoPanels(_)));
    }
    #[test]
    fn validation_refuses_an_unknown_region_line_and_title_position() {
        let mut g = base();
        g.regions = vec![Region { line: "wavy".into(), ..region("r", "", &["a"]) }];
        assert!(matches!(refuses(g), SpecError::RegionLine { .. }));
        let mut g = base();
        g.regions = vec![Region { title_at: "middle".into(), ..region("r", "", &["a"]) }];
        assert!(matches!(refuses(g), SpecError::RegionTitleAt { .. }));
    }
    #[test]
    fn validation_refuses_a_bad_align() {
        let mut g = base();
        g.align = vec![Align { axis: "diag".into(), ids: vec!["a".into(), "b".into()] }];
        assert!(matches!(refuses(g), SpecError::AlignAxis(_)));
        let mut g = base();
        g.align = vec![Align { axis: "row".into(), ids: vec!["a".into(), "zz".into()] }];
        assert!(matches!(refuses(g), SpecError::AlignUnknown(_)));
        let mut g = base();
        g.align = vec![Align { axis: "col".into(), ids: vec!["a".into()] }];
        assert_eq!(refuses(g), SpecError::AlignTooFew);
    }
    #[test]
    fn validation_refuses_an_off_that_is_no_line() {
        let mut g = base();
        g.edges.push(Edge { from: "a".into(), to: "d".into(), off: "a>c".into(), ..Default::default() });
        assert!(matches!(refuses(g), SpecError::EdgeOffUnknown { .. }));
        let mut g = base();
        g.edges.push(Edge { from: "a".into(), to: "d".into(), off: "a>b#2".into(), ..Default::default() });
        assert!(matches!(refuses(g), SpecError::EdgeOffUnknown { .. }));
    }
    #[test]
    fn validation_refuses_a_line_leaving_itself() {
        let mut g = base();
        g.edges.push(Edge { from: "a".into(), to: "d".into(), off: "a>d".into(), ..Default::default() });
        assert!(matches!(refuses(g), SpecError::EdgeOffSelf(_)));
    }
}
