//! The LLM task `sc-flow-graph`: a description of a figure (optionally with a
//! picture of it) → the L1 graph, and nothing else. The model picks module
//! kinds, roles and line kinds from closed lists, refers to variables by
//! notation id and lists the ones the registry lacks in `vars_new`. The reply
//! is checked against a JSON schema, then structurally ([`Graph::validate`]),
//! then against the notation registry ([`crate::notation::check_graph`]).
//! Coordinates and colours never come from the model.

use lore_llm::{Image, LlmError, LlmProvider, LlmRequest, Message, TaskTable, complete_json};
use lore_notation::Notation;
use serde_json::{Value, json};

use crate::modules::Library;
use crate::notation::{NotationReport, check_graph};
use crate::spec::*;

/// Task id in lore-llm's router.
pub const TASK: &str = "sc-flow-graph";

/// The task definition, to merge into an app's task table.
pub fn task_table() -> TaskTable {
    TaskTable::parse(include_str!("../tasks.toml")).expect("bundled tasks.toml parses")
}

/// What the model is asked.
pub struct FlowPrompt<'a> {
    /// The author's description of the figure.
    pub description: &'a str,
    /// A picture of the figure (a sketch, the paper's figure).
    pub image: Option<Image>,
    /// The registry in force (global + document).
    pub notation: Option<&'a Notation>,
    pub library: &'a Library,
    /// Worked examples: (description, graph). Few-shot for local models.
    pub examples: &'a [(String, Graph)],
}

#[derive(Debug, Clone)]
pub struct Proposal {
    pub graph: Graph,
    /// The graph in the text form the author corrects.
    pub dsl: String,
    pub notation: NotationReport,
    /// The engine that answered.
    pub provider: String,
    /// The first reply was structurally wrong and had to be corrected.
    pub retried: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FlowError {
    #[error("{0}")]
    Llm(#[from] LlmError),
    #[error("the model's graph is not usable: {0}")]
    Invalid(String),
}

/// Module kinds on offer: the built-in ones and the library's templates.
pub fn module_list(lib: &Library) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = MODULES.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    for kind in lib.kinds() {
        let d = lib.description(&kind);
        v.push((kind, if d.is_empty() { "a saved drawing".to_string() } else { d }));
    }
    v
}

/// JSON schema of the reply. Every list is closed.
pub fn schema(lib: &Library) -> Value {
    let modules: Vec<String> = module_list(lib).into_iter().map(|m| m.0).collect();
    let kinds: Vec<&str> = EdgeKind::ALL.iter().map(|k| k.as_str()).collect();
    let mut roles: Vec<&str> = ROLES.to_vec();
    roles.push("");
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["fig", "panels", "nodes", "edges", "vars_new"],
        "properties": {
            "fig": { "type": "string" },
            "arrange": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } },
            "panels": { "type": "array", "items": {
                "type": "object", "additionalProperties": false, "required": ["id", "title"],
                "properties": { "id": { "type": "string" }, "title": { "type": "string" }, "dir": { "type": "string", "enum": ["right", "down"] } }
            } },
            "nodes": { "type": "array", "items": {
                "type": "object", "additionalProperties": false, "required": ["id", "module", "label"],
                "properties": {
                    "id": { "type": "string" },
                    "module": { "type": "string", "enum": modules },
                    "role": { "type": "string", "enum": roles },
                    "label": { "type": "string" },
                    "var": { "type": "string" },
                    "caption": { "type": "string" },
                    "panel": { "type": "string" },
                    "params": {
                        "type": "object", "additionalProperties": false,
                        "properties": {
                            "n": { "type": "integer" }, "depth": { "type": "integer" },
                            "rows": { "type": "integer" }, "cols": { "type": "integer" },
                            "look": { "type": "string" }, "shape": { "type": "string" }, "dir": { "type": "string" },
                            "layers": { "type": "array", "items": { "type": "string" } },
                            "labels": { "type": "array", "items": { "type": "string" } },
                            "vertical": { "type": "boolean" }, "taper": { "type": "boolean" }
                        }
                    }
                }
            } },
            "edges": { "type": "array", "items": {
                "type": "object", "additionalProperties": false, "required": ["from", "to", "kind"],
                "properties": { "from": { "type": "string" }, "to": { "type": "string" }, "kind": { "type": "string", "enum": kinds }, "label": { "type": "string" } }
            } },
            "vars_new": { "type": "array", "items": {
                "type": "object", "additionalProperties": false, "required": ["id", "latex", "meaning"],
                "properties": { "id": { "type": "string" }, "latex": { "type": "string" }, "meaning": { "type": "string" } }
            } }
        }
    })
}

fn system(p: &FlowPrompt) -> String {
    let mut s = String::from(
        "You turn the description of a figure from a machine-learning paper into a data-flow graph. \
A program draws the figure from your graph: you decide WHAT is in it and how it connects, never where it sits or what colour it has.\n\n\
Reply with one JSON object: {fig, arrange?, panels, nodes, edges, vars_new}.\n\n\
## Nodes\n\
Every thing the description names is one node: {id, module, role?, label, var?, caption?, panel?, params?}.\n\
- id: short, lower-case letters, digits and _ (unique).\n\
- module: one of the kinds below, nothing else.\n",
    );
    for (k, d) in module_list(p.library) {
        s.push_str(&format!("  - {k}: {d}\n"));
    }
    s.push_str(
        "- role: input | model | op | data | output | gt | loss | merge | note (gt = ground truth / annotation).\n\
- label: the text on or beside the module. Math goes between $ signs as LaTeX. Keep it as short as the description has it.\n\
- var: the notation id of the variable this node IS (an image x, a prediction). Use it instead of typing the symbol again.\n\
- caption: a size or remark under the module (e.g. $512\\times512$).\n\
- panel: the id of the panel the node is in, when the figure has stages or sub-figures.\n\
- Whole networks are ONE node (unet, layer-bars, cuboid-seq or box), not one node per layer; list the layers in params.layers.\n\n\
## Edges\n\
{from, to, kind, label?}. kind is one of:\n\
  data (main flow) | supervision (ground truth into a loss) | feedback (a loss or weight going back to update a model) | \
loss-out (a model's output that is a loss) | compare (prediction against target, double-headed) | optional (weak or optional path) | \
shared (shared weights) | skip (skip connection) | concat (an input of a concatenation node) | down | up | \
cross (from one panel to another) | stage | repeat | zoom (an enlarged detail of a node).\n\
- Two paths that join meet in an op node (label + for add, x for multiply, . for element-wise product, C for concatenation).\n\
- A prediction and its target both go INTO the loss node: prediction by data, target by supervision.\n\n\
## Panels\n\
One panel per stage / sub-figure (a., b., c. …) with its title; `dir` is \"right\" unless the description says top to bottom. \
`arrange` lists rows of panel ids when the description says how the panels are placed (e.g. one above two).\n\n\
## Variables\n\
- Loss functions are $\\mathcal L_{name}$; predictions carry a hat or prime; tensors and images are bold.\n\
- Refer to a variable of the notation table below by its id: write @id inside a label, or set `var`.\n\
- A variable that is not in the table goes into vars_new as {id, latex, meaning} and is then referred to the same way (@id / var). \
Never invent an id without listing it.\n\
- Never write a typed reference such as \"Table 1\" or \"Fig. 2\".\n",
    );
    match p.notation.filter(|n| !n.entries.is_empty()) {
        Some(n) => {
            s.push_str("\n## Notation table (id — LaTeX — meaning)\n");
            for e in n.entries.iter().filter(|e| e.latex.is_some()).take(200) {
                s.push_str(&format!("- {} — ${}$ — {}\n", e.id, e.latex.as_deref().unwrap_or(""), e.meaning.as_deref().or(e.meaning_en.as_deref()).unwrap_or("")));
            }
        }
        None => s.push_str("\n## Notation table\n(empty: every variable goes into vars_new)\n"),
    }
    for (i, (desc, g)) in p.examples.iter().enumerate() {
        s.push_str(&format!("\n## Example {}\nDescription:\n{}\n\nGraph:\n{}\n", i + 1, desc.trim(), serde_json::to_string(g).unwrap_or_default()));
    }
    s
}

/// The request for a description (image attached when there is one).
pub fn request(p: &FlowPrompt) -> LlmRequest {
    let user = format!("Description of the figure:\n\n{}\n\nReturn the graph as JSON.", p.description.trim());
    let msg = match &p.image {
        Some(img) => Message::user_with_images(format!("{user}\n\nThe attached picture shows the figure; use it for what the text leaves open."), vec![img.clone()]),
        None => Message::user(user),
    };
    LlmRequest::chat(system(p), vec![msg]).json(schema(p.library)).max_tokens(8000)
}

/// Make a model's ids usable: lower-case, `-`/spaces to `_`, same change in
/// every reference. (Models like `x-b` or `Lseg`; ids are case-sensitive names.)
fn tidy(g: &mut Graph) {
    let fix = |s: &str| -> String {
        let t: String = s.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        if t.is_empty() { "n".into() } else { t }
    };
    g.fig = fix(&g.fig);
    for p in &mut g.panels {
        p.id = fix(&p.id);
    }
    for n in &mut g.nodes {
        n.id = fix(&n.id);
        n.panel = if n.panel.trim().is_empty() { String::new() } else { fix(&n.panel) };
        n.var = n.var.trim().trim_start_matches('@').to_string();
    }
    for e in &mut g.edges {
        e.from = fix(&e.from);
        e.to = fix(&e.to);
    }
    for r in &mut g.arrange {
        for id in r.iter_mut() {
            *id = fix(id);
        }
    }
    // an arrangement that forgets or invents panels is dropped (the layout picks one)
    let ids: Vec<&str> = g.panels.iter().map(|p| p.id.as_str()).collect();
    let named: Vec<&str> = g.arrange.iter().flatten().map(|s| s.as_str()).collect();
    if !g.arrange.is_empty() && (named.iter().any(|n| !ids.contains(n)) || ids.iter().any(|i| !named.contains(i))) {
        g.arrange.clear();
    }
    // duplicate edges
    let mut seen = std::collections::BTreeSet::new();
    g.edges.retain(|e| seen.insert((e.from.clone(), e.to.clone(), e.kind)));
}

fn parse_graph(v: Value, lib: &Library) -> Result<Graph, String> {
    let mut g: Graph = serde_json::from_value(v).map_err(|e| e.to_string())?;
    tidy(&mut g);
    g.validate(&lib.kinds()).map_err(|e| e.to_string())?;
    Ok(g)
}

/// Ask `provider` for the graph of a description. A reply that does not hold
/// together (unknown node in an edge, duplicate id …) is sent back once with
/// the reason.
pub async fn propose(provider: &dyn LlmProvider, p: &FlowPrompt<'_>) -> Result<Proposal, FlowError> {
    let req = request(p);
    let v = complete_json(provider, &req).await?;
    let (mut graph, retried) = match parse_graph(v.clone(), p.library) {
        Ok(g) => (g, None),
        Err(why) => {
            let mut again = req.clone();
            again.messages.push(Message::assistant(v.to_string()));
            again.messages.push(Message::user(format!("That graph cannot be drawn: {why}. Return the whole corrected JSON object.")));
            let v2 = complete_json(provider, &again).await?;
            (parse_graph(v2, p.library).map_err(FlowError::Invalid)?, Some(why))
        }
    };
    let empty = Notation { version: 1, entries: vec![], conventional: vec![], ignored: vec![] };
    let notation = check_graph(&mut graph, p.notation.unwrap_or(&empty));
    let dsl = crate::dsl::to_dsl(&graph);
    Ok(Proposal { graph, dsl, notation, provider: provider.id(), retried })
}
