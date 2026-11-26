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

use crate::knowledge::{Example, Knowledge, ModuleNote, Picked, pick_examples, pick_modules};
use crate::modules::Library;
use crate::notation::{NotationReport, check_graph};
use crate::spec::*;

/// Task id in lore-llm's router (a description alone).
pub const TASK: &str = "sc-flow-graph";
/// Task id when a picture is attached (needs an engine that sees images).
pub const TASK_VISION: &str = "sc-flow-graph-vision";

/// The task to route: with or without a picture.
pub fn task_for(has_image: bool) -> &'static str {
    if has_image { TASK_VISION } else { TASK }
}

/// The task definitions, to merge into an app's task table.
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
    /// Ids of nodes whose `style` was not one of their kind's styles: the style was cleared.
    pub cleared_styles: Vec<String>,
}

/// What knowledge was put into a prompt, in order (owned copies, for the author to see).
#[derive(Debug, Clone, Default)]
pub struct Shown {
    pub modules: Vec<Picked<ModuleNote>>,
    pub examples: Vec<Picked<Example>>,
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
    schema_with(lib)
}

/// The schema, with an optional `style` on every node (a string, not an enum: styles differ per kind).
pub fn schema_with(lib: &Library) -> Value {
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
                    "style": { "type": "string" },
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

fn system_with(p: &FlowPrompt, k: &Knowledge, shown: &Shown) -> String {
    let with_styles = !k.modules.is_empty() || !k.examples.is_empty();
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
        let st = kind_styles(p.library, &k);
        if with_styles && st.len() > 1 {
            let mut names: Vec<&str> = st.iter().take(8).map(|x| x.as_str()).collect();
            if st.len() > 8 {
                names.push("…");
            }
            s.push_str(&format!("  - {k}: {d} (styles: {})\n", names.join(", ")));
        } else {
            s.push_str(&format!("  - {k}: {d}\n"));
        }
    }
    s.push_str(
        "- role: input | model | op | data | output | gt | loss | merge | note (gt = ground truth / annotation).\n\
- label: the text on or beside the module. Math goes between $ signs as LaTeX. Keep it as short as the description has it.\n\
- var: the notation id of the variable this node IS (an image x, a prediction). Use it instead of typing the symbol again.\n\
- caption: a size or remark under the module (e.g. $512\\times512$).\n\
- panel: the id of the panel the node is in, when the figure has stages or sub-figures.\n\
- Whole networks are ONE node (unet, layer-bars, cuboid-seq or box), not one node per layer; list the layers in params.layers.\n",
    );
    s.push_str(&notes_section(shown));
    s.push_str(
        "\n## Edges\n\
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
Never invent an id without listing it. `latex` is the bare symbol (`\\hat{y}`): no `$`, no definition such as `\\in \\{0,1\\}^{K}` (that goes into `meaning`).\n\
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
    let mut n = 0;
    for ex in &shown.examples {
        n += 1;
        s.push_str(&format!(
            "\n## Example {n} (the author's own figure: {})\nDescription:\n{}\n\nGraph:\nThe same figure in the short text form (for you to read; you still reply with JSON):\n```\n{}\n```\n",
            ex.item.title.trim(),
            ex.item.description.trim(),
            cut(&plain_dsl(&ex.item.dsl), MAX_DSL).trim_end()
        ));
    }
    for (desc, g) in p.examples.iter().take(MAX_EXAMPLES.saturating_sub(shown.examples.len())) {
        n += 1;
        s.push_str(&format!("\n## Example {n}\nDescription:\n{}\n\nGraph:\n{}\n", desc.trim(), serde_json::to_string(g).unwrap_or_default()));
    }
    s
}

const MAX_NOTE: usize = 600;
const MAX_DSL: usize = 2500;
const MAX_EXAMPLES: usize = 4;
const MAX_NOTES: usize = 12;
const MAX_MADE: usize = 3;

/// Styles a node of `kind` may name: the built-in ones and the library's.
fn kind_styles(lib: &Library, kind: &str) -> Vec<String> {
    let mut v: Vec<String> = crate::modules::generator_styles(kind).iter().map(|s| s.to_string()).collect();
    for s in lib.styles(kind) {
        if !v.contains(&s) {
            v.push(s);
        }
    }
    v
}

/// At most `max` characters, cut at a line end when there is one, with `…` after a cut.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    let head = match head.rfind('\n') {
        Some(i) if i > 0 => &head[..i],
        _ => &head[..],
    };
    format!("{}…", head.trim_end())
}

/// A made figure's text without what only the drawing decides (colours, positions, sizes).
fn plain_dsl(dsl: &str) -> String {
    const DROP: [&str; 20] = [
        "color", "colour", "fill", "stroke", "frame", "outline", "gradient", "radius", "x", "y", "w", "h", "width", "height", "size", "pos", "place", "ends", "opacity", "font",
    ];
    let mut out = Vec::new();
    for line in dsl.lines() {
        let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let mut chunks: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        let mut prev = ' ';
        for c in line.trim().chars() {
            if c == '"' && prev != '\\' {
                quoted = !quoted;
            }
            if c.is_whitespace() && !quoted {
                if !cur.is_empty() {
                    chunks.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
            prev = c;
        }
        if !cur.is_empty() {
            chunks.push(cur);
        }
        let kept: Vec<String> = chunks.into_iter().filter(|c| !c.split_once('=').is_some_and(|(k, _)| !k.starts_with('"') && DROP.contains(&k.to_lowercase().as_str()))).collect();
        out.push(format!("{indent}{}", kept.join(" ")));
    }
    out.join("\n")
}

fn notes_section(shown: &Shown) -> String {
    if shown.modules.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n## The author's notes on modules\n");
    for m in &shown.modules {
        let m = &m.item;
        let style = if m.style.is_empty() { String::new() } else { format!(" / {}", m.style) };
        s.push_str(&format!("- {}{style}: {}", m.kind, cut(m.prompt.trim(), MAX_NOTE)));
        let vars = |v: &[crate::knowledge::Var]| v.iter().map(|x| format!("{} ({})", x.id, x.kind)).collect::<Vec<_>>().join(", ");
        match (m.inputs.is_empty(), m.outputs.is_empty()) {
            (false, false) => s.push_str(&format!(" in: {}; out: {}", vars(&m.inputs), vars(&m.outputs))),
            (false, true) => s.push_str(&format!(" in: {}", vars(&m.inputs))),
            (true, false) => s.push_str(&format!(" out: {}", vars(&m.outputs))),
            _ => {}
        }
        s.push('\n');
    }
    s.push_str("A module's `style` may be set to one of the styles listed for its kind above when its note fits; otherwise leave `style` out.\n");
    s
}

/// The picks for a description, as owned copies.
fn shown_for(description: &str, k: &Knowledge) -> Shown {
    let own = |p: Picked<&ModuleNote>| Picked { item: p.item.clone(), score: p.score, why: p.why };
    let own_e = |p: Picked<&Example>| Picked { item: p.item.clone(), score: p.score, why: p.why };
    Shown {
        modules: pick_modules(description, k, MAX_NOTES).into_iter().map(own).collect(),
        examples: pick_examples(description, k, MAX_MADE).into_iter().map(own_e).collect(),
    }
}

/// The request for a description (image attached when there is one).
pub fn request(p: &FlowPrompt) -> LlmRequest {
    request_with(p, &Knowledge::default()).0
}

/// As [`request`], with the author's knowledge: what was put into the prompt comes back too.
pub fn request_with(p: &FlowPrompt, k: &Knowledge) -> (LlmRequest, Shown) {
    let shown = shown_for(p.description, k);
    let user = format!("Description of the figure:\n\n{}\n\nReturn the graph as JSON.", p.description.trim());
    let msg = match &p.image {
        Some(img) => Message::user_with_images(format!("{user}\n\nThe attached picture shows the figure; use it for what the text leaves open."), vec![img.clone()]),
        None => Message::user(user),
    };
    (LlmRequest::chat(system_with(p, k, &shown), vec![msg]).json(schema_with(p.library)).max_tokens(8000), shown)
}

/// A node whose style is not one of its kind's loses it; their ids.
fn clear_styles(g: &mut Graph, lib: &Library) -> Vec<String> {
    let mut cleared = Vec::new();
    for n in &mut g.nodes {
        if !n.style.is_empty() && !kind_styles(lib, &n.module).contains(&n.style) {
            n.style.clear();
            cleared.push(n.id.clone());
        }
    }
    cleared
}

/// Make a model's ids usable: lower-case, `-`/spaces to `_`, same change in
/// every reference. (Models like `x-b` or `Lseg`; ids are case-sensitive names.)
fn tidy(g: &mut Graph) {
    let fix = |s: &str| -> String {
        let t: String = s.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        // runs of replaced characters collapse; none left at the ends
        let mut out = String::new();
        for c in t.chars() {
            if c == '_' && out.ends_with('_') {
                continue;
            }
            out.push(c);
        }
        let out = out.trim_matches('_').to_string();
        if out.is_empty() { "n".into() } else { out }
    };
    // a backslash doubled once too often in the JSON (`\\mathbf`) is a command, not a line break
    let unescape = |s: &mut String| {
        let cs: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < cs.len() {
            if cs[i] == '\\' && cs.get(i + 1) == Some(&'\\') && cs.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
                i += 1;
                continue;
            }
            out.push(cs[i]);
            i += 1;
        }
        *s = out;
    };
    for n in &mut g.nodes {
        unescape(&mut n.label);
        unescape(&mut n.caption);
    }
    for v in &mut g.vars_new {
        unescape(&mut v.latex);
        v.latex = crate::spec::bare_latex(&v.latex);
    }
    for e in &mut g.edges {
        unescape(&mut e.label);
    }
    for p in &mut g.panels {
        unescape(&mut p.title);
    }
    g.fig = fix(&g.fig);
    if g.fig == "n" {
        g.fig = "fig".into();
    }
    for p in &mut g.panels {
        p.id = fix(&p.id);
        // "right" is only the model's default: the layout may still turn the panel to fit
        if p.dir == "right" {
            p.dir.clear();
        }
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

/// How an app runs a JSON request for a task (its own routing, engines, keys);
/// the same shape as lore-plot's, so one adapter serves both editors.
#[async_trait::async_trait]
pub trait Llm: Send + Sync {
    /// The reply as JSON (schema-checked) and the engine that answered.
    async fn complete_json(&self, task: &str, req: LlmRequest) -> Result<(Value, String), String>;
}

/// As [`propose`], through an app's [`Llm`] (task [`task_for`]).
pub async fn propose_via(llm: &dyn Llm, p: &FlowPrompt<'_>) -> Result<Proposal, String> {
    propose_with(llm, p, &Knowledge::default()).await.map(|x| x.0)
}

/// As [`propose_via`], with the author's knowledge; also returns what the model was shown.
pub async fn propose_with(llm: &dyn Llm, p: &FlowPrompt<'_>, k: &Knowledge) -> Result<(Proposal, Shown), String> {
    let task = task_for(p.image.is_some());
    let (req, shown) = request_with(p, k);
    let (v, engine) = llm.complete_json(task, req.clone()).await?;
    let (mut graph, retried) = match parse_graph(v.clone(), p.library) {
        Ok(g) => (g, None),
        Err(why) => {
            let mut again = req;
            again.messages.push(Message::assistant(v.to_string()));
            again.messages.push(Message::user(format!("That graph cannot be drawn: {why}. Return the whole corrected JSON object.")));
            let (v2, _) = llm.complete_json(task, again).await?;
            (parse_graph(v2, p.library).map_err(|e| format!("模型給的圖畫不出來：{e}"))?, Some(why))
        }
    };
    let cleared_styles = clear_styles(&mut graph, p.library);
    let empty = Notation { version: 1, entries: vec![], conventional: vec![], ignored: vec![] };
    let notation = check_graph(&mut graph, p.notation.unwrap_or(&empty));
    let dsl = crate::dsl::to_dsl(&graph);
    Ok((Proposal { graph, dsl, notation, provider: engine, retried, cleared_styles }, shown))
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
    let cleared_styles = clear_styles(&mut graph, p.library);
    let empty = Notation { version: 1, entries: vec![], conventional: vec![], ignored: vec![] };
    let notation = check_graph(&mut graph, p.notation.unwrap_or(&empty));
    let dsl = crate::dsl::to_dsl(&graph);
    Ok(Proposal { graph, dsl, notation, provider: provider.id(), retried, cleared_styles })
}

// ── the text draft ───────────────────────────────────────────────────────────
// The model writes the figure language itself; the program checks it, draws it once and sends a
// failure back once with the parser's own words.

/// A figure written as text by the model, checked and drawn once.
#[derive(Debug, Clone)]
pub struct TextProposal {
    /// The figure text (what the model wrote, plus `var` lines for the symbols it listed).
    pub dsl: String,
    pub graph: Graph,
    /// The engine that answered.
    pub provider: String,
    /// The first reply could not be drawn and was sent back once.
    pub retried: bool,
    /// What the successful draw noticed (a line with too many corners, an unknown colour, an
    /// alignment not met). Told to the author, not sent back to the model.
    pub warnings: Vec<String>,
}

/// JSON schema of the text draft's reply.
pub fn text_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["dsl", "vars_new"],
        "properties": {
            "dsl": { "type": "string" },
            "vars_new": { "type": "array", "items": {
                "type": "object", "additionalProperties": false, "required": ["id", "latex", "meaning"],
                "properties": { "id": { "type": "string" }, "latex": { "type": "string" }, "meaning": { "type": "string" } }
            } }
        }
    })
}

const MAX_TEXT_EXAMPLE: usize = 4500;
const MAX_LIBRARY_LINES: usize = 120;

/// The language, taught once. Compact and exact: every sentence is one the parser reads.
const LANGUAGE_GUIDE: &str = r##"You draw a figure by WRITING ITS TEXT in a small figure language. A program checks the text, lays it out and draws it. Reply with one JSON object {"dsl": "<the figure text>", "vars_new": [{"id","latex","meaning"}]}. Inside the JSON string a newline is \n, a quote is \" and every LaTeX backslash is doubled (\\mathcal).

## What the author wants of a figure
- The same areas and blocks the description names, no more, no fewer, named as the description names them.
- Every block pinned to a cell (col= and lane=) so the picture's proportions are deliberate, not left to chance.
- A straight main flow, left to right (or top to bottom), with as few corners as possible: give blocks that a line joins the same lane so the line is straight.
- One colour per path or per kind of thing; regions round blocks of one kind; do not colour everything.
- Few crossing lines; no line should have to cross a block.

## The language (one statement per line; # starts a comment)
fig <name>
panel a "a. Title" [dir=right|down]
  ...blocks, regions and lines of the panel...
end
- A panel is a top-level area. Several panels: `arrange a b / c` (rows of panel ids: a and b side by side, c under them).
- Panel options are a region's: dashed | dotted | solid | none (the frame's line), tint=<colour>, round, title=top-left|top|above|below|left|none.
    panel b "Stage 2" tint=blue round none title=above     # a pale rounded area without a black frame
- A line between panels: `a -> b : cross`.

### Blocks
  id: kind "label" key=value ...
- id: lower-case letters, digits, _ (unique among blocks, regions and panels). kind: one of the kinds listed below.
- role=input|model|op|data|output|gt|loss|merge|note only shades the block. caption="16x16" adds a small line under it. style=<name> picks the kind's art style.
- A block's own colour: fill=blue (pale fill, blue outline), or fill=F5E6C8; line=<colour>; text=<colour>.
    enc: encoder "Encoder" style=trapezoid fill=blue
- Pins: col=<n> (0 = first along the flow) and lane=<n> (0 = top). Columns and lanes are counted PER PANEL; gaps are closed up. Pin EVERY block of a panel and you decide the layout: blocks with no line between them can share a row, one block can sit under another, an input can come from the right. Lines never move a pinned block.
    x: image "$x$" look=photo col=0 lane=1
    f: box "Model" col=1 lane=1
    y: label "Output" col=2 lane=1
- Words: `$...$` is LaTeX; `@id` is a registered symbol. A label may be empty ("").

### Regions (frames round blocks; they nest)
  region g "Group title" dashed tint=orange round title=above
    a1: box "A" col=1 lane=0
    region inner "Inner" dotted
      a2: op "+" col=2 lane=0
    end
  end
- Options: dashed | dotted | solid | none, tint=<colour>, round, title=top-left|top|above|below|left. Blocks declared inside are members; a region may also list only ids: `region r "T" dashed` / `  n1 n2` / `end`. A region frames its members' cells: pin the members to adjacent cells.

### Lines
  a -> b                          a data line
  a -> b -> c                     a chain
  a -> b : kind "label" color=orange width=2.5 curve
  a>b -> c : optional             leaves the MIDDLE of the line a->b (a branch off a line)
- kind: data (default), supervision, feedback, loss-out, compare, optional, shared, skip, concat, down, up, cross. Dashed by default: supervision, feedback, optional, shared, cross. After the colon the order is: kind, "label", color=, width=, curve.
- color= gives a line its own colour: one colour per path. width= is the thickness in pt (1 to 4). curve draws a curved line (for a skip over blocks).
- Two paths that join meet in an `op` block (label + for add, x for multiply, . element-wise, C concatenation).

### Alignment
  align row x enc bb              same height (centres on one row)
  align col a1 a3                 one above the other (same centre column)
Use pins inside a panel; use align across panels.

### Colours
Names: red orange yellow green teal blue purple pink brown grey black, or RRGGBB (six hex digits; digits only are read as a number, so use a name or a hex with a letter). They go on blocks (fill= line= text=), lines (color=), regions and panels (tint=). A name gives a pale fill and a strong outline. There is no beige: write F5E6C8.

### Looks and picture blocks
- Pictures from a paper are never drawn: `image "" look=photo|mask|heatmap|plot`; `image-stack n=3`.
- A small chart: `c: chart "Name" chart=bell|bells|membership|bars|scatter|surface n=3 labels=[A,B,C] values=[1,2,3]`.
- An icon: `d: icon "Doctor" icon=healthicons:doctor-outline` (icon=<set:name>; only ids you are sure of).
- A network as bars: `lb: layer-bars "" layers=[Conv,MBConv1,FC] colors=[pink,purple] through=orange` (colors= one outline colour per bar, the last repeats; filled=true also fills them pale; through=<colour>|true draws the dashed line "data flows through these layers"). Other forms: `unet depth=3`, `cuboid-seq shape=down n=4`, `vector-bar n=4 dir=v`, `matrix rows=3 cols=3`.

### Saved blocks of the library, with their own words
A saved block (listed below) is a kind like any other. It carries fixed words; texts=[...] replaces them in order, `_` keeps one, "" blanks one:
    m: encoder-decoder-ladder "Backbone" style=five texts=[Enc,_,"",Dec] col=1 lane=0
The program warns when you give more words than the block has. Maths $...$ works inside them.

### Symbols
`@id` in a label is a symbol of the notation table. A symbol that is not in the table is listed in vars_new as {id, latex, meaning} (or as a line `var id = "latex" "meaning"`) and then used as @id. `latex` is the bare symbol (\\hat{y}): no $, no definition. Never type a reference such as "Table 1" or "Fig. 2".
"##;

fn text_system(p: &FlowPrompt, shown: &Shown) -> String {
    let mut s = String::from(LANGUAGE_GUIDE);
    s.push_str("\n## Kinds of blocks (kind: what it draws)\n");
    for (kind, d) in MODULES.iter() {
        s.push_str(&format!("  - {kind}: {d}\n"));
    }
    s.push_str(&library_section(p.library));
    s.push_str(&notes_section(shown));
    match p.notation.filter(|n| !n.entries.is_empty()) {
        Some(n) => {
            s.push_str("\n## Notation table (id, LaTeX, meaning): write @id in a label\n");
            for e in n.entries.iter().filter(|e| e.latex.is_some()).take(200) {
                s.push_str(&format!("- {} — ${}$ — {}\n", e.id, e.latex.as_deref().unwrap_or(""), e.meaning.as_deref().or(e.meaning_en.as_deref()).unwrap_or("")));
            }
        }
        None => s.push_str("\n## Notation table\n(empty: every variable goes into vars_new)\n"),
    }
    let mut n = 0;
    for ex in &shown.examples {
        n += 1;
        s.push_str(&format!(
            "\n## Example {n} (the author's own figure: {})\nDescription:\n{}\n\nFigure text (colours, pins and options are part of what it says):\n```\n{}\n```\n",
            ex.item.title.trim(),
            ex.item.description.trim(),
            cut(&ex.item.dsl, MAX_TEXT_EXAMPLE).trim_end()
        ));
    }
    for (desc, g) in p.examples.iter().take(MAX_EXAMPLES.saturating_sub(shown.examples.len())) {
        n += 1;
        s.push_str(&format!("\n## Example {n}\nDescription:\n{}\n\nFigure text:\n```\n{}\n```\n", desc.trim(), cut(&crate::dsl::to_dsl(g), MAX_TEXT_EXAMPLE).trim_end()));
    }
    s
}

/// Every saved block kind and style of the library, one line each, components first.
fn library_section(lib: &Library) -> String {
    let mut rows: Vec<(bool, String, String, String)> = Vec::new();
    for (kind, styles) in &lib.templates {
        if MODULES.iter().any(|(m, _)| m == kind) {
            continue;
        }
        for (style, t) in styles {
            rows.push((t.level == "component", kind.clone(), style.clone(), t.description.clone()));
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let total = rows.len();
    let mut s = String::from("\n## Saved blocks of the library (kind, style, level: what it is). Used like any kind, with style= and texts=[...]\n");
    for (comp, kind, style, d) in rows.into_iter().take(MAX_LIBRARY_LINES) {
        let st = if style == crate::modules::DEFAULT_STYLE { String::new() } else { format!(" style={style}") };
        let level = if comp { " (component)" } else { "" };
        let d = if d.trim().is_empty() { "a saved drawing".to_string() } else { cut(d.trim(), 160) };
        s.push_str(&format!("  - {kind}{st}{level}: {d}\n"));
    }
    if total > MAX_LIBRARY_LINES {
        s.push_str(&format!("  - … and {} more\n", total - MAX_LIBRARY_LINES));
    }
    s
}

/// The request of the text draft (a picture attached when there is one).
pub fn text_request_with(p: &FlowPrompt, k: &Knowledge) -> (LlmRequest, Shown) {
    let shown = shown_for(p.description, k);
    let user = format!("Description of the figure:\n\n{}\n\nReturn the JSON object with the figure text.", p.description.trim());
    let msg = match &p.image {
        Some(img) => Message::user_with_images(format!("{user}\n\nThe attached picture shows the figure; use it for what the text leaves open."), vec![img.clone()]),
        None => Message::user(user),
    };
    (LlmRequest::chat(text_system(p, &shown), vec![msg]).json(text_schema()).max_tokens(8000), shown)
}

/// The text of a reply: its `dsl` with a `var` line for each symbol of `vars_new` the text does not
/// declare itself. A doubled backslash before a letter (a JSON escaped once too often) is one.
fn reply_text(v: &Value) -> Result<String, String> {
    let dsl = v.get("dsl").and_then(Value::as_str).ok_or("the reply has no \"dsl\" text")?;
    let cs: Vec<char> = dsl.chars().collect();
    let mut text = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && cs.get(i + 1) == Some(&'\\') && cs.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        text.push(cs[i]);
        i += 1;
    }
    let mut text = text.trim_end().to_string();
    text.push('\n');
    if let Some(list) = v.get("vars_new").and_then(Value::as_array) {
        for x in list {
            let id = x.get("id").and_then(Value::as_str).unwrap_or("").trim().trim_start_matches('@').to_string();
            let latex = crate::spec::bare_latex(x.get("latex").and_then(Value::as_str).unwrap_or(""));
            if id.is_empty() || latex.is_empty() {
                continue;
            }
            let declared = text.lines().any(|l| l.trim_start().strip_prefix("var ").is_some_and(|r| r.trim_start().split(|c: char| c.is_whitespace() || c == '=').next() == Some(id.as_str())));
            if !declared {
                let meaning = x.get("meaning").and_then(Value::as_str).unwrap_or("").replace('"', "'");
                text.push_str(&format!("var {id} = \"{}\" \"{meaning}\"\n", latex.replace('"', "'")));
            }
        }
    }
    Ok(text)
}

/// Parse, validate and draw once. The error is what stopped it, in the parser's own words
/// ("line 7: …"); the warnings are what the draw noticed.
fn check_text(text: &str, p: &FlowPrompt) -> Result<(Graph, Vec<String>), String> {
    let kinds = p.library.kinds();
    let g = crate::dsl::parse_with(text, &kinds).map_err(|e| e.to_string())?;
    g.validate(&kinds).map_err(|e| e.to_string())?;
    let d = crate::Drawing { library: p.library.clone(), notation: p.notation, ..Default::default() };
    let fig = crate::draw(&g, &d).map_err(|e| e.to_string())?;
    Ok((g, fig.built.warnings.clone()))
}

fn reply_checked(v: &Value, p: &FlowPrompt) -> Result<(String, Graph, Vec<String>), String> {
    let t = reply_text(v)?;
    let (g, w) = check_text(&t, p)?;
    Ok((t, g, w))
}

/// A figure from a description, written by the model as text (tasks [`task_for`], the same engines
/// as [`propose_with`]); also returns what the model was shown from the author's knowledge. A
/// text that does not parse, validate or draw is sent back once with the exact message.
pub async fn propose_text_with(llm: &dyn Llm, p: &FlowPrompt<'_>, k: &Knowledge) -> Result<(TextProposal, Shown), String> {
    let task = task_for(p.image.is_some());
    let (req, shown) = text_request_with(p, k);
    let (v, engine) = llm.complete_json(task, req.clone()).await?;
    let (dsl, graph, warnings, retried) = match reply_checked(&v, p) {
        Ok((t, g, w)) => (t, g, w, false),
        Err(why) => {
            let mut again = req;
            again.messages.push(Message::assistant(v.to_string()));
            again.messages.push(Message::user(format!("That figure cannot be drawn: {why}. Return the whole corrected JSON object.")));
            let (v2, _) = llm.complete_json(task, again).await?;
            let (t, g, w) = reply_checked(&v2, p).map_err(|e| format!("模型寫的圖畫不出來：{e}"))?;
            (t, g, w, true)
        }
    };
    Ok((TextProposal { dsl, graph, provider: engine, retried, warnings }, shown))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::Var;
    use std::sync::Mutex;

    struct Script {
        replies: Mutex<Vec<Value>>,
        seen: Mutex<Vec<LlmRequest>>,
    }
    #[async_trait::async_trait]
    impl Llm for Script {
        async fn complete_json(&self, _task: &str, req: LlmRequest) -> Result<(Value, String), String> {
            self.seen.lock().unwrap().push(req);
            let mut r = self.replies.lock().unwrap();
            if r.is_empty() { Err("no more replies".into()) } else { Ok((r.remove(0), "script".into())) }
        }
    }
    fn script(replies: Vec<Value>) -> Script {
        Script { replies: Mutex::new(replies), seen: Mutex::new(vec![]) }
    }
    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().build().unwrap()
    }
    fn lib() -> Library {
        let mut lib = Library::default();
        for st in ["plain", "fancy"] {
            lib.add_json(&format!(r#"{{ "kind": "gadget", "style": "{st}", "description": "a block", "w": 60, "h": 40, "objects": [] }}"#)).unwrap();
        }
        lib
    }
    fn prompt<'a>(lib: &'a Library, desc: &'a str, ex: &'a [(String, Graph)]) -> FlowPrompt<'a> {
        FlowPrompt { description: desc, image: None, notation: None, library: lib, examples: ex }
    }
    fn note(kind: &str, prompt: &str) -> ModuleNote {
        ModuleNote {
            kind: kind.into(),
            prompt: prompt.into(),
            inputs: vec![Var { id: "x".into(), kind: "volume".into(), meaning: String::new() }],
            outputs: vec![Var { id: "y".into(), kind: "mask".into(), meaning: String::new() }],
            ..Default::default()
        }
    }
    fn made(id: &str, title: &str, dsl: &str) -> Example {
        Example {
            id: id.into(),
            title: title.into(),
            description: "a CT volume to a mask".into(),
            dsl: dsl.into(),
            inputs: vec![Var { id: "x".into(), kind: "volume".into(), meaning: String::new() }],
            ..Default::default()
        }
    }
    const DESC: &str = "A CT volume goes into a unet and gives a segmentation mask";

    #[test]
    fn with_no_knowledge_the_request_is_what_it_was() {
        let lib = lib();
        let p = prompt(&lib, DESC, &[]);
        let old = request(&p);
        let (new, shown) = request_with(&p, &Knowledge::default());
        // the whole text is equal: no style list is added until there is knowledge
        assert_eq!(old.system, new.system);
        assert_eq!(old.messages[0].text(), new.messages[0].text());
        assert!(shown.modules.is_empty() && shown.examples.is_empty());
        for gone in ["(styles:", "notes on modules", "the author's own figure"] {
            assert!(!new.system.contains(gone), "{gone}");
        }
    }

    #[test]
    fn the_authors_note_on_a_module_reaches_the_prompt() {
        let lib = lib();
        let k = Knowledge { modules: vec![note("unet", "Use it for 3D segmentation.")], examples: vec![] };
        let (req, shown) = request_with(&prompt(&lib, DESC, &[]), &k);
        assert_eq!(shown.modules.len(), 1);
        assert!(req.system.contains("## The author's notes on modules\n- unet: Use it for 3D segmentation. in: x (volume); out: y (mask)\n"), "{}", req.system);
        assert!(req.system.contains("may be set to one of the styles listed"));
        // the section sits after the module list and before the edges
        assert!(req.system.find("gadget: a block").unwrap() < req.system.find("notes on modules").unwrap());
        assert!(req.system.find("notes on modules").unwrap() < req.system.find("## Edges").unwrap());
        // styles of a kind are listed once there is knowledge
        assert!(req.system.contains("gadget: a block (styles: plain, fancy)") || req.system.contains("gadget: a block (styles: fancy, plain)"), "{}", req.system);
        assert!(req.system.contains("(styles: bars, lineart)"));
    }

    #[test]
    fn a_made_figure_is_shown_as_a_worked_example_before_the_folders_examples() {
        let lib = lib();
        let folder = vec![("A folder example.".to_string(), crate::dsl::parse("fig e\nx: image \"$x$\"\n").unwrap())];
        let dsl = "fig m\nx: image \"$x$\" fill=ff0000 w=40\nf: unet \"U\" depth=3 color=\"red\"\nx -> f\n";
        let k = Knowledge { modules: vec![], examples: vec![made("a", "My seg", dsl)] };
        let (req, shown) = request_with(&prompt(&lib, DESC, &folder), &k);
        assert_eq!(shown.examples.len(), 1);
        let s = &req.system;
        let a = s.find("## Example 1 (the author's own figure: My seg)").unwrap();
        let b = s.find("## Example 2\n").unwrap();
        assert!(a < b);
        assert!(s.contains("The same figure in the short text form (for you to read; you still reply with JSON):\n```\nfig m"));
        // nothing about colours or sizes comes with it
        assert!(!s.contains("fill=ff0000") && !s.contains("w=40") && !s.contains("color=\"red\""), "{s}");
        assert!(s.contains("depth=3") && s.contains("A folder example."));
    }

    #[test]
    fn at_most_four_examples_in_all() {
        let lib = lib();
        let folder: Vec<(String, Graph)> = (0..5).map(|i| (format!("folder {i}"), crate::dsl::parse("fig e\nx: image \"$x$\"\n").unwrap())).collect();
        let k = Knowledge { modules: vec![], examples: (0..5).map(|i| made(&format!("m{i}"), &format!("T{i}"), "fig m\n")).collect() };
        let (req, shown) = request_with(&prompt(&lib, DESC, &folder), &k);
        assert_eq!(shown.examples.len(), 3);
        assert_eq!(req.system.matches("\n## Example ").count(), 4);
        assert!(req.system.contains("folder 0") && !req.system.contains("folder 1"));
        // without made figures the folder's four still show
        let (req, _) = request_with(&prompt(&lib, DESC, &folder), &Knowledge::default());
        assert_eq!(req.system.matches("\n## Example ").count(), 4);
    }

    #[test]
    fn long_notes_are_cut() {
        let lib = lib();
        let long = format!("{}\n{}", "a".repeat(400), "b".repeat(400));
        let dsl = (0..400).map(|i| format!("n{i}: box \"label number {i}\"")).collect::<Vec<_>>().join("\n");
        let k = Knowledge { modules: vec![note("unet", &long)], examples: vec![made("a", "T", &dsl)] };
        let (req, _) = request_with(&prompt(&lib, DESC, &[]), &k);
        assert!(!req.system.contains(&"b".repeat(400)));
        assert!(req.system.contains(&format!("- unet: {}…", "a".repeat(400))));
        assert!(req.system.contains("n0:"));
        assert!(!req.system.contains("n399:"));
        assert!(req.system.contains("…\n```"));
    }

    #[test]
    fn what_was_shown_is_returned_in_order() {
        let lib = lib();
        let mut weak = note("encoder", "An encoder.");
        weak.inputs.clear();
        weak.outputs.clear();
        weak.tags = vec!["segmentation".into()];
        let k = Knowledge { modules: vec![weak, note("unet", "U."), note("decoder", "")], examples: vec![made("m1", "One", "fig m\n")] };
        let (_, shown) = request_with(&prompt(&lib, DESC, &[]), &k);
        assert_eq!(shown.modules.iter().map(|m| m.item.kind.as_str()).collect::<Vec<_>>(), ["unet", "encoder"]);
        assert_eq!(shown.modules[0].why, "takes volume, gives mask");
        assert_eq!(shown.examples[0].item.id, "m1");
    }

    fn reply(style: &str) -> Value {
        json!({ "fig": "f", "panels": [], "nodes": [
            { "id": "x", "module": "image", "label": "" },
            { "id": "g", "module": "gadget", "label": "G", "style": style },
            { "id": "u", "module": "unet", "label": "U", "style": "bars" } ],
          "edges": [ { "from": "x", "to": "g", "kind": "data" }, { "from": "g", "to": "u", "kind": "data" } ], "vars_new": [] })
    }

    #[test]
    fn a_node_may_name_a_style() {
        let lib = lib();
        let s = schema_with(&lib);
        assert_eq!(s["properties"]["nodes"]["items"]["properties"]["style"], json!({ "type": "string" }));
        assert_eq!(schema(&lib), s);
        let llm = script(vec![reply("fancy")]);
        let (prop, _) = rt().block_on(propose_with(&llm, &prompt(&lib, "…", &[]), &Knowledge::default())).unwrap();
        assert_eq!(prop.graph.node("g").unwrap().style, "fancy");
        assert_eq!(prop.graph.node("u").unwrap().style, "bars");
        assert!(prop.cleared_styles.is_empty());
        assert!(prop.dsl.contains("style=fancy"));
    }

    #[test]
    fn a_style_the_kind_does_not_have_is_cleared_not_refused() {
        let lib = lib();
        let llm = script(vec![reply("sparkly")]);
        let prop = rt().block_on(propose_via(&llm, &prompt(&lib, "…", &[]))).unwrap();
        assert_eq!(prop.graph.node("g").unwrap().style, "");
        assert_eq!(prop.cleared_styles, ["g"]);
        assert!(prop.retried.is_none());
        assert_eq!(llm.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn propose_with_returns_what_the_model_was_shown() {
        let lib = lib();
        let k = Knowledge { modules: vec![note("unet", "U.")], examples: vec![made("m1", "One", "fig m\n")] };
        let llm = script(vec![reply("plain")]);
        let (_, shown) = rt().block_on(propose_with(&llm, &prompt(&lib, DESC, &[]), &k)).unwrap();
        assert_eq!(shown.modules.len(), 1);
        assert_eq!(shown.examples[0].item.title, "One");
        let sent = llm.seen.lock().unwrap();
        assert!(sent[0].system.contains("- unet: U."));
        assert!(sent[0].system.contains("the author's own figure: One"));
    }

    // ── the text draft ──

    fn text_reply(dsl: &str) -> Value {
        json!({ "dsl": dsl, "vars_new": [] })
    }
    const PLAIN: &str = "fig f\nx: image \"$x$\"\nm: box \"Model\"\ny: label \"Out\"\nx -> m -> y\n";
    const RICH: &str = "fig f\npanel a \"a. Train\" tint=blue round none title=above\n  x: image \"$x$\" look=photo col=0 lane=0\n  region g \"Group\" dashed tint=orange round title=above\n    f: box \"Model\" fill=blue col=1 lane=0\n    h: gadget \"G\" texts=[A,B] col=2 lane=0\n  end\n  y: label \"Out\" col=3 lane=0\n  x -> f : data color=orange width=2.5\n  f -> h : skip curve color=green\n  h -> y\nend\n";

    fn last_user(req: &LlmRequest) -> String {
        req.messages.last().unwrap().text()
    }

    #[test]
    fn a_text_draft_is_parsed_checked_and_drawn() {
        let lib = lib();
        let llm = script(vec![json!({ "dsl": PLAIN, "vars_new": [ { "id": "w_b", "latex": "$\\omega_b$", "meaning": "weight" } ] })]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert_eq!(t.graph.nodes.len(), 3);
        assert!(!t.retried && t.provider == "script");
        // a symbol of vars_new becomes a var line of the text, and of the graph
        assert!(t.dsl.contains("var w_b = \"\\omega_b\" \"weight\""), "{}", t.dsl);
        assert_eq!(t.graph.vars_new[0].id, "w_b");
        assert_eq!(llm.seen.lock().unwrap().len(), 1);
        let sent = &llm.seen.lock().unwrap()[0];
        assert!(sent.json_schema.is_some() && !serde_json::to_string(&sent.json_schema).unwrap().contains("nodes"));
    }

    #[test]
    fn a_draft_with_regions_pins_and_colours_is_accepted() {
        let lib = lib();
        let llm = script(vec![text_reply(RICH)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert!(!t.retried, "{:?}", t.warnings);
        assert_eq!(t.graph.regions.len(), 1);
        assert_eq!(t.graph.regions[0].members.len(), 2);
        assert_eq!(t.graph.edges.len(), 3);
        assert!(t.dsl.contains("texts=[A,B]") && t.dsl.contains("width=2.5") && t.dsl.contains("lane=0"));
    }

    #[test]
    fn a_draft_that_does_not_parse_is_sent_back_once_with_the_line() {
        let lib = lib();
        let bad = "fig f\nx: box \"A\"\ny: box \"B\"\nx -> y -> nowhere\n";
        let msg = match crate::dsl::parse_with(bad, &lib.kinds()) {
            Err(e) => e.to_string(),
            Ok(g) => g.validate(&lib.kinds()).unwrap_err().to_string(),
        };
        assert!(!msg.is_empty());
        let llm = script(vec![text_reply(bad), text_reply(PLAIN)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert!(t.retried);
        let seen = llm.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let back = last_user(&seen[1]);
        assert!(back.contains(&msg) && back.contains("whole corrected JSON object"), "{back}");
        assert_eq!(seen[1].messages.len(), 3);
    }

    #[test]
    fn a_syntax_error_names_its_line() {
        let lib = lib();
        let bad = "fig f\nx: box \"A\n";
        let llm = script(vec![text_reply(bad), text_reply(PLAIN)]);
        rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        let back = last_user(&llm.seen.lock().unwrap()[1]);
        assert!(back.contains("line 2"), "{back}");
    }

    #[test]
    fn a_second_failure_is_one_line() {
        let lib = lib();
        let bad = "fig f\nx: box \"A\n";
        let llm = script(vec![text_reply(bad), text_reply(bad)]);
        let e = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap_err();
        assert!(e.starts_with("模型寫的圖畫不出來：") && !e.contains('\n'), "{e}");
        assert_eq!(llm.seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_reply_without_text_is_sent_back() {
        let lib = lib();
        let llm = script(vec![json!({ "vars_new": [] }), text_reply(PLAIN)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert!(t.retried);
    }

    #[test]
    fn draw_warnings_come_back_and_are_not_sent_back() {
        let lib = lib();
        let dsl = "fig f\nx: image \"$x$\"\nm: box \"Model\"\nx -> m : data color=notacolour\n";
        let llm = script(vec![text_reply(dsl)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert!(!t.retried);
        assert!(t.warnings.iter().any(|w| w.contains("notacolour")), "{:?}", t.warnings);
        assert_eq!(llm.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_text_prompt_teaches_every_sentence() {
        let lib = lib();
        let (req, _) = text_request_with(&prompt(&lib, DESC, &[]), &Knowledge::default());
        for kw in ["region", "align row", "col=", "lane=", "tint=", "color=", "width=", "curve", "texts=", "colors=", "through=", "chart=", "icon=", "align col", "dashed", "title=above", "a>b -> c", "fill=", "caption=", "style="] {
            assert!(req.system.contains(kw), "{kw}");
        }
        assert!(req.system.lines().count() < 220, "{}", req.system.lines().count());
        assert!(req.system.contains("(empty: every variable goes into vars_new)"));
        assert!(req.messages[0].text().contains(DESC));
    }

    #[test]
    fn examples_are_shown_with_their_colours_and_pins() {
        let lib = lib();
        let dsl = "fig m\nx: image \"$x$\" fill=ff0000 w=40 col=2 lane=1\nf: unet \"U\" depth=3 color=\"red\"\nx -> f : data color=orange\n";
        let k = Knowledge { modules: vec![], examples: vec![made("a", "My seg", dsl)] };
        let folder = vec![("A folder example.".to_string(), crate::dsl::parse("fig e\nx: image \"$x$\"\n").unwrap())];
        let (req, shown) = text_request_with(&prompt(&lib, DESC, &folder), &k);
        assert_eq!(shown.examples.len(), 1);
        let s = &req.system;
        for kept in ["fill=ff0000", "w=40", "col=2 lane=1", "color=\"red\"", "color=orange"] {
            assert!(s.contains(kept), "{kept}");
        }
        assert!(s.contains("## Example 1 (the author's own figure: My seg)") && s.contains("A folder example."));
        let long = (0..600).map(|i| format!("n{i}: box \"label number {i}\" col={i}")).collect::<Vec<_>>().join("\n");
        let k = Knowledge { modules: vec![], examples: vec![made("a", "T", &long)] };
        let (req, _) = text_request_with(&prompt(&lib, DESC, &[]), &k);
        assert!(req.system.contains("n0:") && !req.system.contains("n599:"));
    }

    #[test]
    fn the_library_blocks_are_listed_with_their_level() {
        let mut lib = lib();
        lib.add_json(r#"{ "kind": "tiny-conv", "description": "one convolution", "level": "component", "w": 30, "h": 20, "objects": [] }"#).unwrap();
        let (req, _) = text_request_with(&prompt(&lib, DESC, &[]), &Knowledge::default());
        let s = &req.system;
        assert!(s.contains("tiny-conv (component): one convolution"), "{s}");
        assert!(s.contains("gadget style=plain: a block") && s.contains("gadget style=fancy: a block"), "{s}");
        // components come first
        assert!(s.find("tiny-conv").unwrap() < s.find("gadget style=").unwrap());
        // the list is capped
        let mut big = Library::default();
        for i in 0..150 {
            big.add_json(&format!(r#"{{ "kind": "blk{i:03}", "description": "d", "w": 10, "h": 10, "objects": [] }}"#)).unwrap();
        }
        let (req, _) = text_request_with(&prompt(&big, DESC, &[]), &Knowledge::default());
        assert!(req.system.contains("blk119") && !req.system.contains("blk120") && req.system.contains("and 30 more"));
    }

    #[test]
    fn registered_symbols_are_offered_when_there_is_a_registry() {
        let lib = lib();
        let n: Notation = serde_json::from_value(json!({ "version": 1, "entries": [
            { "id": "w_b", "kind": "symbol", "latex": "\\omega_b", "meaning": "weight of batch b" } ] })).unwrap();
        let mut p = prompt(&lib, DESC, &[]);
        p.notation = Some(&n);
        let (req, _) = text_request_with(&p, &Knowledge::default());
        assert!(req.system.contains("- w_b — $\\omega_b$ — weight of batch b"), "{}", req.system);
        assert!(!req.system.contains("(empty: every variable goes into vars_new)"));
        // a registered symbol is drawn from the registry
        let dsl = "fig f\nx: box \"@w_b\"\ny: label \"Out\"\nx -> y\n";
        let llm = script(vec![text_reply(dsl)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &p, &Knowledge::default())).unwrap();
        assert!(!t.retried);
    }
}
