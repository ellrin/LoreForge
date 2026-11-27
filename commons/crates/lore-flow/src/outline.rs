//! The opening statement of a figure, before anything is drawn: its areas, the modules in each,
//! the frames round groups of them, the marks that belong to a module, how the modules are
//! connected (the flow's own paths, then the other lines by kind), and the shapes the whole
//! figure can take (width : height) with the text size each gives. It is what the one who draws
//! agrees to first; every later step works inside it.

use crate::pack::{self, Air};
use crate::plate::{Format, Measure};
use crate::spec::{EdgeKind, Graph};

/// One area of the figure (a panel; `_` holds the modules that are in none).
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineArea {
    pub id: String,
    pub title: String,
    /// (id, kind) of each module, in the order written.
    pub modules: Vec<(String, String)>,
    /// (title or id, members) of each frame whose members stand in this area.
    pub frames: Vec<(String, Vec<String>)>,
    /// (mark, the module it belongs to): small things that take no place of their own.
    pub marks: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub areas: Vec<OutlineArea>,
    /// The flow's own paths: chains of modules joined by data lines, each from a module no
    /// data line enters.
    pub paths: Vec<Vec<String>>,
    /// The other lines, by kind: (kind, from, to).
    pub others: Vec<(String, String, String)>,
    pub modules: usize,
    pub marks: usize,
    pub frames: usize,
    pub lines: usize,
}

pub fn outline(g: &Graph) -> Outline {
    let is_mark: Vec<bool> = (0..g.nodes.len()).map(|n| pack::is_note(g, n)).collect();
    let partner = |n: usize| -> String {
        let node = &g.nodes[n];
        node.str("by").map(str::to_string).or_else(|| g.edges.iter().find_map(|e| if e.from == node.id { Some(e.to.clone()) } else if e.to == node.id { Some(e.from.clone()) } else { None })).unwrap_or_default()
    };
    let mut ids: Vec<(String, String)> = g.panels.iter().map(|p| (p.id.clone(), p.title.clone())).collect();
    if g.nodes.iter().any(|n| n.panel.is_empty()) {
        ids.push(("_".into(), String::new()));
    }
    let areas: Vec<OutlineArea> = ids
        .into_iter()
        .map(|(id, title)| {
            let inside = |n: &crate::spec::Node| if id == "_" { n.panel.is_empty() } else { n.panel == id };
            let here: Vec<usize> = (0..g.nodes.len()).filter(|n| inside(&g.nodes[*n])).collect();
            OutlineArea {
                modules: here.iter().filter(|n| !is_mark[**n]).map(|n| (g.nodes[*n].id.clone(), g.nodes[*n].module.clone())).collect(),
                marks: here.iter().filter(|n| is_mark[**n]).map(|n| (g.nodes[*n].id.clone(), partner(*n))).collect(),
                frames: g.regions.iter().filter(|r| r.members.iter().any(|m| here.iter().any(|n| &g.nodes[*n].id == m))).map(|r| (if r.title.is_empty() { r.id.clone() } else { r.title.clone() }, r.members.clone())).collect(),
                id,
                title,
            }
        })
        .collect();
    // the flow's own paths: follow data lines from each module none enters, a line used once
    let data: Vec<(usize, &str, &str)> = g.edges.iter().enumerate().filter(|(_, e)| e.kind == EdgeKind::Data).map(|(i, e)| (i, e.from.as_str(), e.to.as_str())).collect();
    let mut used = vec![false; g.edges.len()];
    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut starts: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).filter(|id| data.iter().any(|d| d.1 == *id) && !data.iter().any(|d| d.2 == *id)).collect();
    // a ring of data lines has no such module: start at the first that still has a line unused
    loop {
        while let Some(s) = starts.pop() {
            while let Some(first) = data.iter().find(|d| d.1 == s && !used[d.0]) {
                let mut chain = vec![s.to_string()];
                let mut at = *first;
                loop {
                    used[at.0] = true;
                    chain.push(at.2.to_string());
                    match data.iter().find(|d| d.1 == at.2 && !used[d.0]) {
                        Some(next) => at = *next,
                        None => break,
                    }
                }
                paths.push(chain);
            }
        }
        match data.iter().find(|d| !used[d.0]) {
            Some(d) => starts.push(d.1),
            None => break,
        }
    }
    paths.sort_by(|a, b| b.len().cmp(&a.len()));
    let others: Vec<(String, String, String)> = g.edges.iter().filter(|e| e.kind != EdgeKind::Data).map(|e| (format!("{:?}", e.kind).to_lowercase(), e.from.clone(), e.to.clone())).collect();
    Outline { modules: areas.iter().map(|a| a.modules.len()).sum(), marks: areas.iter().map(|a| a.marks.len()).sum(), frames: g.regions.len(), lines: g.edges.len(), areas, paths, others }
}

impl Outline {
    /// The statement as lines of text for the one who draws.
    pub fn text(&self) -> String {
        let mut s = format!("{} 個區塊、{} 個模塊、{} 個外框、{} 個標記、{} 條線\n", self.areas.len(), self.modules, self.frames, self.marks, self.lines);
        for a in &self.areas {
            let name = if a.title.is_empty() { a.id.clone() } else { format!("{}「{}」", a.id, a.title) };
            s += &format!("區塊 {name}：{} 個模塊\n", a.modules.len());
            s += &format!("  模塊：{}\n", a.modules.iter().map(|(id, k)| format!("{id}({k})")).collect::<Vec<_>>().join("、"));
            for (t, m) in &a.frames {
                s += &format!("  外框 {t}：{}\n", m.join("、"));
            }
            if !a.marks.is_empty() {
                s += &format!("  標記：{}\n", a.marks.iter().map(|(m, p)| if p.is_empty() { m.clone() } else { format!("{m}（屬於 {p}）") }).collect::<Vec<_>>().join("、"));
            }
        }
        s += &format!("主線 {} 條：\n", self.paths.len());
        for p in &self.paths {
            s += &format!("  {}\n", p.join(" → "));
        }
        let mut kinds: Vec<&str> = self.others.iter().map(|o| o.0.as_str()).collect();
        kinds.sort();
        kinds.dedup();
        for k in kinds {
            s += &format!("{k}：{}\n", self.others.iter().filter(|o| o.0 == k).map(|o| format!("{} → {}", o.1, o.2)).collect::<Vec<_>>().join("、"));
        }
        s
    }
}

/// One shape the whole figure can take.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Width : height.
    pub ratio: (u32, u32),
    /// The text size on a page `width_pt` wide.
    pub text_pt: f64,
    /// The share of the page the pictures cover.
    pub cover: f64,
    /// Text under the least size (`Air.text_min_pt`).
    pub small: bool,
}

/// The shapes to choose from at the start.
pub const SHAPES: [(u32, u32); 8] = [(1, 1), (4, 3), (3, 2), (16, 9), (2, 1), (5, 2), (3, 1), (4, 1)];

/// What each shape in `among` gives for this figure at `width_pt`: packed as the base draft is,
/// largest text first (of those with the largest, the page best filled). The ones a figure
/// cannot be packed in are left out.
pub fn shapes(g: &Graph, width_pt: f64, m: &Measure, arrangement: &str, air: &Air, among: &[(u32, u32)]) -> Vec<Shape> {
    let mut v: Vec<Shape> = among
        .iter()
        .filter_map(|r| {
            let p = pack::pack(g, &Format::new(width_pt, *r), m, arrangement, air)?;
            Some(Shape { ratio: *r, text_pt: p.text_pt, cover: p.cover, small: p.small })
        })
        .collect();
    // largest text first; shapes whose text is as large as the largest (within [`SAME`]) are
    // as good for reading, so of those the page best filled comes first
    let best = v.iter().map(|s| s.text_pt).fold(0.0, f64::max);
    let top = |s: &Shape| s.text_pt >= best * (1.0 - SAME);
    v.sort_by(|a, b| match (top(a), top(b)) {
        (true, true) => b.cover.partial_cmp(&a.cover).unwrap(),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => b.text_pt.partial_cmp(&a.text_pt).unwrap(),
    });
    v
}

/// Text sizes within this share of each other read the same.
pub const SAME: f64 = 0.03;

/// How a module is drawn: `solid` (with depth) or `flat`; `None` for what has no look of its
/// own to keep (photos and plain pictures, words, operators, losses, plain boxes, marks).
pub fn look(n: &crate::spec::Node, lib: &crate::modules::Library, theme: &crate::theme::Theme) -> Option<&'static str> {
    let style = crate::modules::style_name(n, theme);
    let saved = if style.is_empty() { None } else { lib.get(&n.module, &style) }.or_else(|| if crate::spec::MODULES.iter().any(|(m, _)| *m == n.module) { None } else { lib.get(&n.module, "") });
    if let Some(t) = saved {
        return Some(t.look());
    }
    match n.module.as_str() {
        "cuboid-seq" => Some("solid"),
        "encoder" | "decoder" | "autoencoder" | "nn" | "unet" | "layer-bars" | "feature-stack" | "vector-bar" | "matrix" => Some("flat"),
        _ => None,
    }
}

/// What a module shows, for the comparison of looks: `network` (a model: its layers or stages),
/// `feature` (what flows between models: maps, tokens, embeddings), or nothing to compare.
/// A written `role=` decides (`model`, `data`); else the built-in kind; else what the saved
/// drawing says about itself (its kind and its description).
pub fn family(n: &crate::spec::Node, lib: &crate::modules::Library) -> Option<&'static str> {
    match n.role.as_str() {
        "model" => return Some("network"),
        "data" => return Some("feature"),
        "" => {}
        _ => return None,
    }
    match n.module.as_str() {
        "encoder" | "decoder" | "autoencoder" | "nn" | "unet" | "layer-bars" | "cuboid-seq" => return Some("network"),
        "feature-stack" | "vector-bar" | "matrix" => return Some("feature"),
        _ => {}
    }
    if crate::spec::MODULES.iter().any(|(m, _)| *m == n.module) {
        return None;
    }
    let said = format!("{} {}", n.module, lib.description(&n.module)).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| said.split(|c: char| !c.is_alphanumeric()).any(|x| x == *w || x.strip_suffix('s') == Some(*w)));
    if has(&["network", "encoder", "decoder", "backbone", "generator", "discriminator", "classifier", "transformer", "model", "layer", "unet", "ladder"]) {
        Some("network")
    } else if has(&["feature", "tensor", "token", "embedding", "map", "volume", "cube", "cuboid", "slab", "prism", "voxel"]) {
        Some("feature")
    } else {
        None
    }
}

/// A figure keeps one look for the things of one family: its networks are all drawn with depth
/// or all flat, and so are its features. Returns, per family that mixes the two, (family, the
/// solid ones, the flat ones); empty when the figure is of one piece.
pub fn mixed_looks(g: &Graph, lib: &crate::modules::Library, theme: &crate::theme::Theme) -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut by: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> = Default::default();
    for (i, n) in g.nodes.iter().enumerate() {
        if pack::is_note(g, i) {
            continue;
        }
        let (Some(fam), Some(l)) = (family(n, lib), look(n, lib, theme)) else { continue };
        let e = by.entry(fam.to_string()).or_default();
        if l == "solid" { e.0.push(n.id.clone()) } else { e.1.push(n.id.clone()) }
    }
    by.into_iter().filter(|(_, (s, f))| !s.is_empty() && !f.is_empty()).map(|(r, (s, f))| (r, s, f)).collect()
}
