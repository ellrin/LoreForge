//! L1 — the data-flow graph. This is everything an author (or an LLM) says
//! about a figure: nodes, edges, panels and variables. No coordinates, no
//! colours; those come from the layout and the theme.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Module kinds the program can draw (the closed list an LLM picks from).
/// Module templates loaded from a library extend it at run time.
pub const MODULES: [(&str, &str); 17] = [
    ("box", "a named step or model: rectangle with the label inside (params: shape=rect|round|trapezoid, vertical=true for an upright bar)"),
    ("op", "operator circle; label is one of + x . C (add, multiply, element-wise, concatenate) or a short word such as Or"),
    ("loss", "a loss or metric; label is its math, e.g. $\\mathcal L_{seg}$"),
    ("label", "free text without a frame (an output class, a note)"),
    ("image", "one picture / mask / heat map (params: look=photo|mask|heatmap|plot)"),
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
            "image" | "image-stack" | "feature-stack" | "vector-bar" | "matrix" => "data",
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VarNew {
    pub id: String,
    pub latex: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meaning: String,
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
