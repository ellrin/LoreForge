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
    /// The page the figure is made for (its width and ratio, chosen before anything is drawn).
    /// With one, the text draft is told the page's shape, is measured on the baseplate and is
    /// sent back once with the numbers when its own shape leaves the page empty.
    pub page: Option<crate::plate::Format>,
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
  data (main flow) | side (a data line beside the main flow: a branch, an input joining in) | supervision (ground truth into a loss) | feedback (a loss or weight going back to update a model) | \
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
    /// What the program still sees wrong in how the kept draft looks ([`looks`]).
    pub looks: Vec<String>,
    /// The first drawable draft had faults in how it looks and was sent back once; the second
    /// was better and is the one kept.
    pub polished: bool,
    /// With a page: the smallest text of the kept draft on that page, in points.
    pub page_text_pt: Option<f64>,
    /// With a page: what still holds the kept draft small there ([`shape_words`]); empty when
    /// its shape is the page's.
    pub shape: Vec<String>,
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
/// Room for the list of saved blocks in the drafting prompt (about 400 lines of 110 characters).
const MAX_LIBRARY_CHARS: usize = 48_000;

/// The language, taught once. Compact and exact: every sentence is one the parser reads.
const LANGUAGE_GUIDE: &str = r##"You draw a figure by WRITING ITS TEXT in a small figure language. A program checks the text, lays it out and draws it. Reply with one JSON object {"dsl": "<the figure text>", "vars_new": [{"id","latex","meaning"}]}. Inside the JSON string a newline is \n, a quote is \" and every LaTeX backslash is doubled (\\mathcal).

## What the author wants of a figure
- The same areas and blocks the description names, no more, no fewer, named as the description names them.
- Every block pinned to a cell (col= and lane=) so the picture's proportions are deliberate, not left to chance.
- A straight main flow, left to right (or top to bottom), with as few corners as possible: give blocks that a line joins the same lane so the line is straight.
- One colour per path or per kind of thing; regions round blocks of one kind; do not colour everything.
- Few crossing lines; no line should have to cross a block.

## How to place a panel (the author's recipe)
1. Each path the description draws from an input to an output (one per panel, or one per row when the description draws parallel rows: training over inference, view 1 … view V) is one chain with `: data`, its blocks pinned in one lane, col = 0, 1, 2 … in the order of the chain.
2. Every other line has the kind of its meaning: `side` (a branch, an input that joins in, a prediction into its own loss), `supervision` (ground truth or a label into a loss), `feedback` (a loss or weights going back), `skip` (over blocks, drawn above the row), `shared` (two blocks that share weights: ONE line, not two), `compare` (two things compared), `cross` (only between two panels). The main path ends at the prediction; losses are never on it. Only paths that must be told apart get `color=`.
3. A block that joins a path at block X sits in X's column, in the lane directly above or below, so its line is straight; an input that comes earlier sits one column before X in its own lane.
4. A loss is a `loss` block, never a label or a box. It sits beside the block it is computed from: right of it when that block ends the path, otherwise in the same column directly above (the first loss) or below (the second). Its ground truth sits on the far side of the loss, in the loss's lane.
5. A line may leave the middle of another line (`a>b -> c`) when the description says a path splits on its way or copies a value along (a branch to a second model, a stop-gradient copy, a second ground truth for another loss). A block that feeds three or more rows keeps one line to each.
6. Repeated rows (view 1, view 2, … view V; branch 1 … R) share columns: the same col for the same role, one lane per row, and an `ellipsis` in its own lane between the last two rows.
7. Pin a panel top to bottom (`dir=down`, col = 0, 1, 2 down the panel) only when the description says the panel runs from top to bottom.
8. A name written over or under several blocks is a region with no frame: `region r "Bad quality" none title=above` around them. Never a `label` block in a lane of its own above them.
9. Words written inside a box in the description are the box's label, not a caption.
10. A legend for the whole figure goes in its own frameless panel (`panel leg "" bare`) in its own row of `arrange` where the description puts it, items pinned in lane 0 in the listed order. A key that explains one panel's symbols may stay inside that panel, in its own column.
11. `align` only between panels, only to make a line between panels straight; inside a panel the pins do it.

## The language (one statement per line; # starts a comment)
fig <name>
panel a "a. Title" [dir=right|down]
  ...blocks, regions and lines of the panel...
end
- A panel is a top-level area. Several panels: `arrange a b / c` (rows of panel ids: a and b side by side, c under them).
- Panel options: none (no frame), bare (nothing drawn), tint=<colour> (only when the description gives the area a colour), title=top-left|top|above|below|left|none. The style draws every panel's frame and corners.
    panel b "Stage 2" tint=blue none title=above     # a pale blue area without a black frame
- Follow the description's words for the arrangement: 上／下 is rows (`arrange a / b`), 左／右 or 由左到右 is one row (`arrange a b`). When the description states no arrangement, write no `arrange` line: the program tries the arrangements on the page and the author picks one.
- A line between panels: `a -> b : cross`.

### Blocks
  id: kind "label" key=value ...
- id: lower-case letters, digits, _ (unique among blocks, regions and panels). kind: one of the kinds listed below.
- role=input|model|op|data|output|gt|loss|merge|note only shades the block. caption="16x16" adds a small line under it. style=<name> picks the kind's art style.
- A block's own colour: fill=blue (pale fill, blue outline), or fill=F5E6C8; line=<colour>; text=<colour>.
    enc: encoder "Encoder" style=trapezoid fill=blue
- Never write round, radius= or shape=round: corners come from the style.
- Pins: col=<n> (0 = first along the flow) and lane=<n> (0 = top). Columns and lanes are counted PER PANEL; gaps are closed up. Pin EVERY block of a panel and you decide the layout: blocks with no line between them can share a row, one block can sit under another, an input can come from the right. Lines never move a pinned block.
    x: image "$x$" look=photo col=0 lane=1
    f: box "Model" col=1 lane=1
    y: label "Output" col=2 lane=1
- Words: `$...$` is LaTeX; `@id` is a registered symbol. A label may be empty (""). A line break inside a label is two backslashes (written `\\\\` inside the JSON string).

### Regions (frames round blocks; they nest)
  region g "Group title" tint=orange title=above
    a1: box "A" col=1 lane=0
    region inner "Inner" dotted
      a2: op "+" col=2 lane=0
    end
  end
- Options: none (no frame), dashed or dotted (only when the description draws that group's frame so), tint=<colour>, title=top-left|top|above|below|left. A region with tint is a group of like things; a region without tint is the frame of one named sub-network. Blocks declared inside are members; a region may also list only ids: `region r "T" dashed` / `  n1 n2` / `end`. A region frames its members' cells: pin the members to adjacent cells.

### Lines
  a -> b                          a data line
  a -> b -> c                     a chain
  a -> b : kind "label" color=orange curve
  a>b -> c : optional             leaves the MIDDLE of the line a->b (a branch off a line)
- kind: data (default; the one main line from input to output), side (any other data line: a branch, an input joining in), supervision, feedback, loss-out, compare, optional, shared, skip, concat, down, up, cross. Dashed by default: supervision, feedback, optional, shared, cross. After the colon the order is: kind, "label", color=, curve, from=, to=.
- color= only when the description says paths have different colours: one colour name per path, the same along the whole path. Never write width=: the line's kind sets its weight. curve only for a line that arcs over other blocks (a skip) or between parallel rows; an ordinary join is never curved.
- from=top|bottom|left|right and to=… choose the side a line leaves or enters a block; leave them out unless the description says where the line goes in.
- An `op` block (label + for add, x for multiply, . element-wise, C concatenation) only where the description names that operation. Two lines that simply arrive at one block each go to that block; a line that splits off another is the branch form `a>b -> c`.
- A line the description calls dashed (虛線) is a dashed kind by its meaning, along its whole length: supervision, feedback, optional, shared or cross.
- Two lines the description says meet midway and then go on as one (兩條線匯合後進 X) are ONE line leaving the middle of the first: `a>b -> X : <kind> "label"`, where a -> b is a line the first block already has; the second source gets no line of its own.
- Outputs gathered by a brace with no arrow: prefer a saved block that draws the outputs with their brace; otherwise a `brace` block in the column right after the outputs, the next line starting at the brace, and no line from the outputs to the gathered value.

### Alignment
  align row x enc bb              same height (centres on one row)
  align col a1 a3                 one above the other (same centre column)
Use pins inside a panel; use align across panels.

### Colours
Names: red orange yellow green teal blue purple pink brown grey black, or RRGGBB (six hex digits; digits only are read as a number, so use a name or a hex with a letter). They go on blocks (fill= line= text=), lines (color=), regions and panels (tint=). A name gives a pale fill and a strong outline. There is no beige: write F5E6C8.

### Looks and picture blocks
- A photo or heat map from a paper is never drawn: `image "" look=photo|heatmap|plot`; `image-stack n=3`. A mask, a stack of video frames, a video and anything else the description names that a saved block draws uses that block. Two named pictures under one title are a `region r "Title" none title=above` holding two picture blocks.
- A small chart: `c: chart "Name" chart=bell|bells|membership|bars|scatter|surface n=3 labels=[A,B,C] values=[1,2,3]`.
- An icon: `d: icon "Doctor" icon=healthicons:doctor-outline` (icon=<set:name>; only ids you are sure of).
- A network as bars: `lb: layer-bars "" layers=[Conv,MBConv1,FC] colors=[pink,purple] through=orange` (colors= one outline colour per bar, the last repeats; filled=true also fills them pale; through=<colour>|true draws the dashed line "data flows through these layers"). Built-in forms (`unet depth=3`, `nn`, `encoder` / `decoder`, `chart=membership|surface`, `cuboid-seq shape=down n=4`, `vector-bar n=4 dir=v`, `matrix rows=3 cols=3`) only when no saved block below says it is used instead of them.

### Saved blocks of the library, with their own words
A saved block (listed below) is a kind like any other. It carries fixed words; texts=[...] replaces them in order, `_` keeps one, "" blanks one:
    m: encoder-decoder-ladder "Backbone" style=five texts=[Enc,_,"",Dec] col=1 lane=0
The program warns when you give more words than the block has. Maths $...$ works inside them. The list below shows each block's own words in [its words: …]: replace every one that is a placeholder (Num, Den, Model, Item …) with the description's words through texts=[…]. The label is the block's NAME as the description writes it; never repeat in the label a word the block already prints (× N, a title).

### Symbols
`@id` in a label is a symbol of the notation table. A symbol that is not in the table is listed in vars_new as {id, latex, meaning} (or as a line `var id = "latex" "meaning"`) and then used as @id. `latex` is the bare symbol (\\hat{y}): no $, no definition. Never type a reference such as "Table 1" or "Fig. 2".
"##;

fn text_system(p: &FlowPrompt, shown: &Shown) -> String {
    let mut s = String::from(LANGUAGE_GUIDE);
    s.push_str("\n## Kinds of blocks (kind: what it draws)\n");
    for (kind, d) in MODULES.iter() {
        s.push_str(&format!("  - {kind}: {d}\n"));
    }
    s.push_str(&library_section(p.library, p.description));
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

/// Every saved block of the library, one line each: the modules first (the figures are built
/// from them), then the components; within a level, the blocks whose name or description shares
/// words with the description come first. Nothing is cut off by count: a block the model is not
/// shown is a block it draws as a plain box.
fn library_section(lib: &Library, description: &str) -> String {
    let words: Vec<String> = description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let lower = description.to_lowercase();
    let hits = |kind: &str, style: &str, d: &str, names: &[String]| -> usize {
        let hay = format!("{kind} {style} {d}").to_ascii_lowercase();
        let mut seen: Vec<&str> = Vec::new();
        for w in &words {
            if !seen.contains(&w.as_str()) && hay.contains(w.as_str()) {
                seen.push(w);
            }
        }
        // a name of the block found in the description (any language) counts for much more
        let named = names.iter().filter(|n| !n.trim().is_empty() && lower.contains(&n.trim().to_lowercase())).count();
        seen.len() + 10 * named
    };
    // (is a module, shared words, kind, style, description and what follows it)
    let mut rows: Vec<(bool, usize, String, String, String)> = Vec::new();
    for (kind, styles) in &lib.templates {
        if MODULES.iter().any(|(m, _)| m == kind) {
            continue;
        }
        for (style, t) in styles {
            let mut d = if t.description.trim().is_empty() { "a saved drawing".to_string() } else { cut(t.description.trim(), 110) };
            if !t.replaces.is_empty() {
                d.push_str(&format!(" (use instead of the built-in `{}`)", t.replaces.join("`, `")));
            }
            let fixed = t.fixed_texts();
            if !fixed.is_empty() {
                let shown: Vec<String> = fixed.iter().take(6).map(|w| cut(w, 18)).collect();
                d.push_str(&format!(" [its words: {}{}]", shown.join(" | "), if fixed.len() > 6 { " …" } else { "" }));
            }
            rows.push((t.level != "component", hits(kind, style, &t.description, &t.names), kind.clone(), style.clone(), d));
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
    let mut s = String::from(
        "\n## Saved blocks of the library (kind, style: what it is). Used like any kind, with style= and texts=[...]. When the description names a thing one of these draws (a U-Net, an augmentation, a classifier head, a rule card, a video …) use the block, never a plain box.\n",
    );
    let mut budget = MAX_LIBRARY_CHARS;
    let mut left = 0usize;
    for (module, _, kind, style, d) in rows {
        let st = if style == crate::modules::DEFAULT_STYLE { String::new() } else { format!(" style={style}") };
        let level = if module { "" } else { " (component)" };
        let line = format!("  - {kind}{st}{level}: {d}\n");
        if line.len() > budget {
            left += 1;
            continue;
        }
        budget -= line.len();
        s.push_str(&line);
    }
    if left > 0 {
        s.push_str(&format!("  - … and {left} more\n"));
    }
    s
}

/// The request of the text draft (a picture attached when there is one).
pub fn text_request_with(p: &FlowPrompt, k: &Knowledge) -> (LlmRequest, Shown) {
    let shown = shown_for(p.description, k);
    let page = p.page.as_ref().map(|f| format!("\n\n{}", page_words(f))).unwrap_or_default();
    let user = format!("Description of the figure:\n\n{}{page}\n\nReturn the JSON object with the figure text.", p.description.trim());
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
    // inside `$…$` a doubled backslash before a letter is LaTeX escaped once too often: one.
    // Outside maths a doubled backslash is a line break and stays; a written-out `\n` there (a
    // newline the model spelt instead of breaking the line) becomes that line break too.
    let mut math = false;
    while i < cs.len() {
        let c = cs[i];
        if c == '$' {
            math = !math;
        } else if c == '\n' {
            math = false;
        }
        if c == '\\' && cs.get(i + 1) == Some(&'\\') && cs.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) && math {
            i += 1;
            continue;
        }
        if c == '\\' && !math && cs.get(i + 1) == Some(&'n') && (i == 0 || cs[i - 1] != '\\') {
            text.push_str("\\\\");
            i += 2;
            continue;
        }
        text.push(c);
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
fn check_text(text: &str, p: &FlowPrompt) -> Result<Checked, String> {
    let kinds = p.library.kinds();
    let g = crate::dsl::parse_with(text, &kinds).map_err(|e| e.to_string())?;
    g.validate(&kinds).map_err(|e| e.to_string())?;
    let d = crate::Drawing { library: p.library.clone(), notation: p.notation, ..Default::default() };
    let fig = crate::draw(&g, &d).map_err(|e| e.to_string())?;
    let mut found = looks(&fig);
    found.extend(block_advice(&g, p.library));
    let on_page = p.page.as_ref().and_then(|f| on_page(&g, f, p.library));
    Ok(Checked { text: text.to_string(), graph: g, warnings: fig.built.warnings.clone(), looks: found, page_text_pt: on_page.as_ref().map(|b| b.text_pt), shape: on_page.map(|b| p.page.as_ref().map(|f| shape_words(&b, f)).unwrap_or_default()).unwrap_or_default() })
}

/// One draft, checked: its text and graph, what the draw noticed, what looks wrong, and with a
/// page how it sits there.
struct Checked {
    text: String,
    graph: Graph,
    warnings: Vec<String>,
    looks: Vec<String>,
    page_text_pt: Option<f64>,
    shape: Vec<String>,
}

/// The page in words, for the first request: its shape and what that asks of the areas.
pub fn page_words(f: &crate::plate::Format) -> String {
    let (w, h) = f.ratio;
    let shape = if w * 10 >= h * 13 {
        "wider than tall: areas run long from left to right, each in as few lanes as its lines allow; areas stacked as rows, or a wide one over two side by side, use it best"
    } else if h * 10 >= w * 13 {
        "taller than wide: areas are stacked, and a long path is folded into two lanes rather than run in one long row"
    } else {
        "about square: two rows of areas, or areas of two lanes, use it best"
    };
    format!("The figure is made for a page area of ratio {w}:{h} (width:height), {:.0} pt wide; it is {shape}. A program measures every block in cells and places the figure on that area: a figure whose own shape is far from {w}:{h} leaves part of the page empty and its text small.", f.width_pt)
}

/// How a graph sits on a page: what binds the arrangement its text states (the best one when
/// it states none). `None` for a graph without blocks.
fn on_page(g: &Graph, f: &crate::plate::Format, lib: &Library) -> Option<crate::plate::Binding> {
    let theme = crate::Theme::classic_bw();
    let mut vars = crate::text::Vars::default();
    vars.add_new(&g.vars_new);
    let m = crate::plate::Measure { theme: &theme, vars: &vars, library: lib, image_size: &crate::modules::no_images };
    let options = crate::plate::place(g, f, &m)?;
    let p = options.iter().find(|p| p.stated).or(options.first())?;
    Some(crate::plate::binding(g, f, p, &m))
}

/// The text on the page must gain at least this much from a better shape before a draft is
/// sent back for its shape alone.
const SHAPE_GAIN: f64 = 1.15;

/// What holds a figure small on its page, in words the drafting model can act on, with the
/// numbers: the cells it needs against the page's shape, the areas that set the binding side and
/// what to do about them. Empty when a better shape would win less than [`SHAPE_GAIN`].
pub fn shape_words(b: &crate::plate::Binding, f: &crate::plate::Format) -> Vec<String> {
    if b.text_pt <= 0.0 || b.text_if_shaped_pt < b.text_pt * SHAPE_GAIN {
        return Vec::new();
    }
    let (w, h) = f.ratio;
    let setters: Vec<String> = b.areas.iter().filter(|a| a.binds).map(|a| format!("{} ({} lane(s) × {} column(s), {} × {} cells)", a.id, a.rows, a.cols, a.w, a.h)).collect();
    let which = if setters.is_empty() { String::new() } else { format!(" It is set by: {}.", setters.join("; ")) };
    let mut out = vec![format!(
        "On the page ({w}:{h}) the figure as pinned needs {} × {} cells, so its {} sets the size: {} cells of the page's {} stay empty and the smallest text is {:.1} pt (it would be {:.1} pt were the figure the page's shape).{which}",
        b.need.0,
        b.need.1,
        if b.tall { "height" } else { "width" },
        b.spare,
        if b.tall { "width" } else { "height" },
        b.text_pt,
        b.text_if_shaped_pt
    )];
    out.push(if b.tall {
        "Make it less tall and wider: bring the blocks of a second lane into the first where the lines allow (an input may join from the right of its block, a result may sit after it), or put two areas side by side with `arrange`. Do not change what the figure shows.".to_string()
    } else {
        "Make it less wide and taller: fold a long row into two lanes (the second half under the first), move an input that joins a block into the lane above or below that block, or stack areas with `arrange`. Do not change what the figure shows.".to_string()
    });
    out
}

/// What a draft gets wrong in its choice of blocks, in words the drafting model can act on: a
/// built-in kind where a saved block of the library is to be used instead, and a label that
/// repeats words the block already prints.
pub fn block_advice(g: &Graph, lib: &Library) -> Vec<String> {
    let mut out = Vec::new();
    for n in &g.nodes {
        let saved = lib.templates.get(&n.module).is_some_and(|_| !MODULES.iter().any(|(m, _)| *m == n.module));
        if !saved {
            // a built-in kind a saved block replaces
            let hit = lib.templates.iter().flat_map(|(k, styles)| styles.iter().map(move |(st, t)| (k, st, t))).find(|(_, _, t)| t.stands_in_for(n));
            if let Some((k, st, t)) = hit {
                let style = if st.as_str() == crate::modules::DEFAULT_STYLE { String::new() } else { format!(" style={st}") };
                out.push(format!("block {} uses the built-in {}; the library's `{k}{style}` draws it ({}): use that block", n.id, n.module, cut(t.description.trim(), 90)));
            }
            continue;
        }
        // a label that repeats the block's own words
        let style = crate::modules::style_name(n, &crate::Theme::classic_bw());
        if let Some(t) = lib.get(&n.module, &style) {
            let label = n.label.trim();
            if !label.is_empty()
                && let Some(w) = t.fixed_texts().into_iter().find(|w| w.chars().count() >= 2 && label.contains(w.as_str()))
            {
                out.push(format!("block {} already prints '{w}': do not repeat it in the label", n.id));
            }
        }
    }
    out
}

/// What the program sees wrong in how a drawn figure looks, in words the drafting model can act
/// on: lines over words, lines showing through a shape, words cut by a shape or frame, lines
/// drawn on a frame. Empty = clean.
pub fn looks(fig: &crate::Figure) -> Vec<String> {
    // `flow:<fig>/edge:a-b/…` → "the line a → b"; `flow:<fig>/<node>/…` → "block <node>"
    let who = |name: &str| -> String {
        let rest = name.split_once('/').map(|x| x.1).unwrap_or(name);
        if let Some(e) = rest.strip_prefix("edge:") {
            let e = e.split('/').next().unwrap_or(e);
            return format!("the line {}", e.replacen('-', " → ", 1));
        }
        for (key, what) in [("panel:", "panel"), ("region:", "region")] {
            if let Some(r) = rest.strip_prefix(key) {
                return format!("{what} {}", r.split('/').next().unwrap_or(r));
            }
        }
        format!("block {}", rest.split('/').next().unwrap_or(rest))
    };
    let rep = crate::look::check(fig.objects());
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for h in &rep.over_text {
        push(format!("{} runs over the words of {}", who(&h.line), who(&h.other)));
    }
    for h in &rep.through_open {
        push(format!("{} shows through {}", who(&h.line), who(&h.other)));
    }
    for h in &rep.over_module {
        push(format!("{} is drawn over {} without ending in it", who(&h.line), who(&h.other)));
    }
    for h in &rep.hidden_heads {
        push(format!("the arrowhead of {} is covered by {}", who(&h.line), who(&h.other)));
    }
    for h in &rep.behind_block {
        push(format!("{} passes behind {}", who(&h.line), who(&h.other)));
    }
    for h in &rep.loose_ends {
        push(format!("the {} of {} touches nothing", h.other, who(&h.line)));
    }
    for h in &rep.on_line {
        push(format!("{} runs on top of {}", who(&h.line), who(&h.other)));
    }
    for h in &rep.words_cut {
        if h.line == h.other {
            push(format!("the words of {} run past its own shape", who(&h.line)));
        } else {
            push(format!("the border of {} cuts the words of {}", who(&h.line), who(&h.other)));
        }
    }
    let n = crate::route::hugging(&fig.layout, &fig.routes);
    if n > 0 {
        push(format!("{n} stretch(es) of line run along a frame's border"));
    }
    out
}

fn reply_checked(v: &Value, p: &FlowPrompt) -> Result<Checked, String> {
    check_text(&reply_text(v)?, p)
}

/// Is a second draft better than the first? Fewer faults in how it looks without smaller text on
/// the page, or larger text on the page (by a twentieth at least) without more faults.
fn better(new: &Checked, old: &Checked) -> bool {
    let (n, o) = (new.looks.len(), old.looks.len());
    match (new.page_text_pt, old.page_text_pt) {
        (Some(a), Some(b)) => (n < o && a >= b * 0.95) || (n <= o && a >= b * 1.05),
        _ => n < o,
    }
}

/// A figure from a description, written by the model as text (tasks [`task_for`], the same engines
/// as [`propose_with`]); also returns what the model was shown from the author's knowledge. A
/// text that does not parse, validate or draw is sent back once with the exact message.
pub async fn propose_text_with(llm: &dyn Llm, p: &FlowPrompt<'_>, k: &Knowledge) -> Result<(TextProposal, Shown), String> {
    let task = task_for(p.image.is_some());
    let (req, shown) = text_request_with(p, k);
    let (v, engine) = llm.complete_json(task, req.clone()).await?;
    let mut req = req;
    let (mut v, mut kept, mut retried) = match reply_checked(&v, p) {
        Ok(c) => (v, c, false),
        Err(why) => {
            req.messages.push(Message::assistant(v.to_string()));
            req.messages.push(Message::user(format!("That figure cannot be drawn: {why}. Return the whole corrected JSON object.")));
            let (v2, _) = llm.complete_json(task, req.clone()).await?;
            let c = reply_checked(&v2, p).map_err(|e| format!("模型寫的圖畫不出來：{e}"))?;
            (v2, c, true)
        }
    };
    // it draws, but the program sees what the author would not accept (faults in how it looks,
    // or a shape that leaves the page empty): sent back once with the exact findings and the
    // numbers; the better of the two drafts is kept
    let mut polished = false;
    if !kept.looks.is_empty() || !kept.shape.is_empty() {
        let mut say = String::new();
        if !kept.looks.is_empty() {
            say.push_str(&format!(
                "The figure draws, but the program found these faults in how it looks:\n- {}\nFix them with pins (col=, lane=), a line's side (from=, to=), a line's kind or where a block sits; do not change what the figure shows.",
                kept.looks.join("\n- ")
            ));
        }
        if !kept.shape.is_empty() {
            if !say.is_empty() {
                say.push_str("\n\n");
            }
            say.push_str(&kept.shape.join("\n"));
        }
        say.push_str("\nReturn the whole corrected JSON object.");
        req.messages.push(Message::assistant(v.to_string()));
        req.messages.push(Message::user(say));
        if let Ok((v2, _)) = llm.complete_json(task, req).await
            && let Ok(c) = reply_checked(&v2, p)
            && better(&c, &kept)
        {
            (v, kept) = (v2, c);
            polished = true;
            retried = true;
        }
    }
    let _ = v;
    Ok((TextProposal { dsl: kept.text, graph: kept.graph, provider: engine, retried, warnings: kept.warnings, looks: kept.looks, polished, page_text_pt: kept.page_text_pt, shape: kept.shape }, shown))
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
        FlowPrompt { description: desc, image: None, notation: None, library: lib, examples: ex, page: None }
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
    #[test]
    fn the_page_is_told_in_the_first_request() {
        let lib = lib();
        let mut p = prompt(&lib, DESC, &[]);
        let (req, _) = text_request_with(&p, &Knowledge::default());
        assert!(!format!("{:?}", req.messages).contains("page area"), "no page, no page words");
        p.page = Some(crate::plate::Format::new(500.0, (16, 9)));
        let (req, _) = text_request_with(&p, &Knowledge::default());
        let said = format!("{:?}", req.messages);
        assert!(said.contains("ratio 16:9") && said.contains("500 pt wide") && said.contains("wider than tall"), "{said}");
        // each shape gets its own advice
        assert!(page_words(&crate::plate::Format::new(250.0, (5, 6))).contains("about square"));
        assert!(page_words(&crate::plate::Format::new(250.0, (2, 3))).contains("taller than wide"));
    }

    fn binding(tall: bool, now: f64, shaped: f64) -> crate::plate::Binding {
        crate::plate::Binding {
            tall,
            need: (167, 170),
            plate: (255, 170),
            spare: 88,
            areas: vec![
                crate::plate::AreaNeed { id: "a".into(), cols: 6, rows: 1, w: 160, h: 40, binds: false },
                crate::plate::AreaNeed { id: "b".into(), cols: 8, rows: 2, w: 167, h: 60, binds: true },
            ],
            text_pt: now,
            text_if_shaped_pt: shaped,
        }
    }

    #[test]
    fn shape_words_give_the_numbers_and_the_areas_that_bind() {
        let f = crate::plate::Format::new(500.0, (3, 2));
        let w = shape_words(&binding(true, 3.4, 4.3), &f).join(" ");
        for part in ["3:2", "167 × 170 cells", "height sets the size", "88 cells of the page's width", "3.4 pt", "4.3 pt", "b (2 lane(s) × 8 column(s), 167 × 60 cells)", "less tall and wider"] {
            assert!(w.contains(part), "missing {part:?} in {w}");
        }
        assert!(!w.contains("a (1 lane"), "an area that does not bind is not named: {w}");
        let w = shape_words(&binding(false, 3.4, 4.3), &f).join(" ");
        assert!(w.contains("width sets the size") && w.contains("less wide and taller") && w.contains("fold a long row"), "{w}");
    }

    #[test]
    fn no_shape_words_when_little_is_to_win() {
        let f = crate::plate::Format::new(500.0, (3, 2));
        assert!(shape_words(&binding(true, 4.0, 4.3), &f).is_empty(), "under 15 percent to win: not sent back");
        assert!(shape_words(&binding(true, 0.0, 4.3), &f).is_empty());
    }

    #[test]
    fn a_second_draft_is_kept_for_fewer_faults_or_larger_text_on_the_page() {
        let c = |looks: usize, text: Option<f64>| Checked { text: String::new(), graph: Graph::default(), warnings: vec![], looks: vec!["x".to_string(); looks], page_text_pt: text, shape: vec![] };
        // without a page: fewer faults only
        assert!(better(&c(1, None), &c(2, None)) && !better(&c(2, None), &c(2, None)));
        // with a page: larger text at the same faults; fewer faults at about the same text
        assert!(better(&c(2, Some(4.3)), &c(2, Some(3.4))));
        assert!(better(&c(1, Some(3.3)), &c(2, Some(3.4))));
        // never: more faults for larger text, or much smaller text for fewer faults
        assert!(!better(&c(3, Some(5.0)), &c(2, Some(3.4))));
        assert!(!better(&c(1, Some(2.5)), &c(2, Some(3.4))));
        assert!(!better(&c(2, Some(3.5)), &c(2, Some(3.4))), "a twentieth at least");
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
    fn a_draft_that_looks_wrong_is_sent_back_once_and_the_better_kept() {
        // a saved block whose one part holds its label: a long word runs past the part
        let mut lib = Library::default();
        lib.add_json(r#"{ "kind": "capped", "style": "x", "description": "a block", "w": 40, "h": 20, "objects": [
            { "id": 0, "name": "cap", "type": "shape", "frame": { "x": 0, "y": 0, "w": 508000, "h": 254000 }, "geom": { "prst": "rect" },
              "fill": { "kind": "solid", "color": "FFFFFF" }, "line": { "color": "000000", "width_pt": 1.0 },
              "text": { "insets": [0, 0, 0, 0], "anchor": "ctr", "wrap": true, "paras": [ { "align": "ctr", "runs": [ { "kind": "text", "text": "{{label}}", "look": { "font": "Arial", "size": 10, "color": "role:text" } } ] } ] } } ] }"#)
            .unwrap();
        let long = "fig f\nn: capped \"Supercalifragilisticexpialidocious\" style=x\n";
        let short = "fig f\nn: capped \"Ok\" style=x\n";
        let llm = script(vec![text_reply(long), text_reply(short)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert!(t.polished && t.looks.is_empty(), "{:?}", t.looks);
        assert!(t.dsl.contains("\"Ok\""));
        let seen = llm.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(last_user(&seen[1]).contains("run past its own shape"), "{}", last_user(&seen[1]));
        drop(seen);
        // a second draft that is no better is not kept
        let llm = script(vec![text_reply(long), text_reply(long)]);
        let (t, _) = rt().block_on(propose_text_with(&llm, &prompt(&lib, DESC, &[]), &Knowledge::default())).unwrap();
        assert!(!t.polished && !t.looks.is_empty());
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
        // modules come first: the figures are built from them
        assert!(s.find("gadget style=").unwrap() < s.find("tiny-conv").unwrap());
        // every block is listed: 150 are not cut to the first 120 (a block the model is not shown
        // is one it draws as a plain box), and a block the description names leads its level
        let mut big = Library::default();
        for i in 0..150 {
            big.add_json(&format!(r#"{{ "kind": "blk{i:03}", "description": "d", "w": 10, "h": 10, "objects": [] }}"#)).unwrap();
        }
        big.add_json(r#"{ "kind": "zz-ladder", "description": "a U-Net drawn as a ladder", "w": 10, "h": 10, "objects": [] }"#).unwrap();
        let (req, _) = text_request_with(&prompt(&big, "The model is a U-Net with skip lines.", &[]), &Knowledge::default());
        let s = &req.system;
        assert!(s.contains("blk000") && s.contains("blk149") && !s.contains("more\n"), "all listed");
        assert!(s.find("zz-ladder").unwrap() < s.find("blk000").unwrap(), "the named block first");
        // a reply's label line break survives, a spelt-out newline becomes one, LaTeX in maths is repaired
        let v = json!({ "dsl": "fig f\na: box \"One\\\\Two\"\nb: box \"Up\\nDown\"\nc: label \"$\\\\mathcal L$\"\n", "vars_new": [] });
        let t = reply_text(&v).unwrap();
        assert!(t.contains("\"One\\\\Two\"") && t.contains("\"Up\\\\Down\"") && t.contains("$\\mathcal L$"), "{t}");
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
