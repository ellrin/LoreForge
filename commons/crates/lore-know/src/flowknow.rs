//! Figures as knowledge: the variables a piece takes and gives, how its parts connect, what can be
//! worked out from a made figure by code, and the bridge to `lore_flow::knowledge` (what a draft
//! is handed).
//!
//! The text forms stored in a figure item's fields:
//!
//! * `inputs`, `outputs`: one variable per line, `id | kind | meaning`; `kind` is one of
//!   [`VAR_KINDS`] or empty (= `other`); empty lines and lines starting with `#` are skipped.
//! * `connects`: one line per connection, `from -> to` or `from -> to : kind`.
//!
//! The parsers here are this crate's own (`lore_flow::knowledge::vars_from_text` is written
//! elsewhere and not called).

use lore_flow::Library;
use lore_flow::figure::FlowSpec;
use lore_flow::knowledge::{Example, Knowledge, ModuleNote, VAR_KINDS, Var};
use serde_json::Value;

use crate::formats::figure::what;
use crate::types::{Item, facet};

// ── the text forms ───────────────────────────────────────────────────────────

/// One line of a variable list as written (the kind is not checked).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VarLine {
    pub raw: String,
    pub id: String,
    pub kind: String,
    pub meaning: String,
}

/// The non-empty, non-comment lines of a variable list.
pub(crate) fn var_lines(text: &str) -> Vec<VarLine> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut p = l.splitn(3, '|').map(str::trim);
            let id = p.next().unwrap_or("").to_string();
            let kind = p.next().unwrap_or("").to_string();
            let meaning = p.next().unwrap_or("").to_string();
            VarLine { raw: l.to_string(), id, kind, meaning }
        })
        .collect()
}

/// The variables of a list; an empty or unknown kind is `other`.
pub fn vars_of(text: &str) -> Vec<Var> {
    var_lines(text)
        .into_iter()
        .filter(|l| !l.id.is_empty())
        .map(|l| {
            let kind = if VAR_KINDS.contains(&l.kind.as_str()) { l.kind } else { "other".to_string() };
            Var { id: l.id, kind, meaning: l.meaning }
        })
        .collect()
}

/// The text form of a variable list.
pub fn vars_text(vars: &[Var]) -> String {
    let one = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "/");
    vars.iter().map(|v| format!("{} | {} | {}", one(&v.id), one(&v.kind), one(&v.meaning))).collect::<Vec<_>>().join("\n")
}

/// `(from, to, kind)` of a connection line, `None` when it is not `a -> b` or `a -> b : kind`.
pub(crate) fn parse_connect(line: &str) -> Option<(String, String, String)> {
    let (edge, kind) = match line.rsplit_once(" : ") {
        Some((e, k)) => (e, k.trim()),
        None => (line, ""),
    };
    let (a, b) = edge.split_once("->")?;
    let (a, b) = (a.trim(), b.trim());
    (!a.is_empty() && !b.is_empty() && !b.contains("->")).then(|| (a.to_string(), b.to_string(), kind.to_string()))
}

// ── derived from a made figure ───────────────────────────────────────────────

/// What code can read off a made figure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Derived {
    pub description: String,
    pub inputs: Vec<Var>,
    pub outputs: Vec<Var>,
    pub connects: Vec<String>,
}

fn guess_kind(module: &str, role: &str) -> &'static str {
    match (role, module) {
        ("gt", "image") => "mask",
        ("loss", _) | (_, "loss") => "loss",
        (_, "image" | "image-stack") => "image",
        (_, "feature-stack") => "feature",
        (_, "vector-bar") => "vector",
        _ => "other",
    }
}

fn plain_label(label: &str) -> bool {
    let l = label.trim();
    !l.is_empty() && l.chars().count() <= 24 && !l.contains(['$', '@', '\n', ':', '|']) && !l.contains("->")
}

/// Work out description, variables and connections of a made figure. A spec whose text does not
/// parse derives nothing.
pub fn derive(spec: &FlowSpec, lib: &Library) -> Derived {
    let Ok(g) = spec.graph(lib) else { return Derived::default() };
    let description = spec.discussion.get("source").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let (mut inputs, mut outputs): (Vec<Var>, Vec<Var>) = (vec![], vec![]);
    for n in &g.nodes {
        let role = n.role_or_default();
        let list = match role {
            "input" | "gt" => &mut inputs,
            "output" | "loss" => &mut outputs,
            _ => continue,
        };
        let v = Var {
            id: if n.var.trim().is_empty() { n.id.clone() } else { n.var.trim().to_string() },
            kind: guess_kind(&n.module, role).to_string(),
            meaning: n.label.trim().to_string(),
        };
        if !list.iter().any(|x| x.id == v.id) {
            list.push(v);
        }
    }
    // a label names a node only when it is plain and no other node has the same
    let name = |id: &str| -> String {
        match g.nodes.iter().find(|n| n.id == id) {
            Some(n) if plain_label(&n.label) && g.nodes.iter().filter(|m| m.label.trim() == n.label.trim()).count() == 1 => n.label.trim().to_string(),
            _ => id.to_string(),
        }
    };
    let connects = g
        .edges
        .iter()
        .map(|e| {
            let base = format!("{} -> {}", name(&e.from), name(&e.to));
            if e.kind == lore_flow::spec::EdgeKind::Data { base } else { format!("{base} : {}", e.kind.as_str()) }
        })
        .collect();
    Derived { description, inputs, outputs, connects }
}

/// Put `d` into the item's empty fields (`description`, `inputs`, `outputs`, `connects`); what the
/// author wrote stays. `prompt` is never set here.
pub fn fill(item: &mut Item, d: &Derived) {
    let mut put = |name: &str, text: String| {
        if !text.trim().is_empty() && item.field(name).trim().is_empty() {
            item.set(name, text);
        }
    };
    put("description", d.description.clone());
    put("inputs", vars_text(&d.inputs));
    put("outputs", vars_text(&d.outputs));
    put("connects", d.connects.join("\n"));
}

/// [`derive`] and [`fill`] for a made figure item whose `params` is its flow spec (an object or
/// its JSON text). An item whose params do not read as a spec is left as it is.
pub fn fill_item(item: &mut Item, lib: &Library) {
    if let Some(spec) = spec_of(item) {
        let d = derive(&spec, lib);
        fill(item, &d);
    }
}

fn spec_of(item: &Item) -> Option<FlowSpec> {
    let v = match item.fields.get("params")? {
        Value::String(s) => serde_json::from_str(s).ok()?,
        v => v.clone(),
    };
    FlowSpec::from_value(v).ok()
}

// ── the bridge to lore-flow ──────────────────────────────────────────────────

fn tags_of(item: &Item) -> Vec<String> {
    let mut t: Vec<String> = [facet::DOMAIN, facet::KIND].iter().filter_map(|f| item.tags.get(*f)).flatten().cloned().collect();
    t.dedup();
    t
}

/// A generator or a saved drawing as a module the model may use.
pub fn module_note(item: &Item) -> Option<ModuleNote> {
    if !matches!(item.field("what"), what::GENERATOR | what::DRAWING) {
        return None;
    }
    Some(ModuleNote {
        kind: item.field("kind").to_string(),
        style: item.field("style").to_string(),
        title: item.title.clone(),
        prompt: item.field("prompt").trim().to_string(),
        inputs: vars_of(item.field("inputs")),
        outputs: vars_of(item.field("outputs")),
        tags: tags_of(item),
    })
}

/// A made figure as a worked example.
pub fn example(item: &Item) -> Option<Example> {
    if item.field("what") != what::SPEC {
        return None;
    }
    let author = item.field("prompt").trim();
    let dsl = match item.fields.get("params") {
        Some(Value::Object(o)) => o.get("dsl").and_then(Value::as_str).unwrap_or("").to_string(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok().and_then(|v| v.get("dsl").and_then(Value::as_str).map(str::to_string)).unwrap_or_default(),
        _ => String::new(),
    };
    Some(Example {
        id: item.id.clone(),
        title: item.title.clone(),
        description: if author.is_empty() { item.field("description").trim().to_string() } else { author.to_string() },
        dsl,
        inputs: vars_of(item.field("inputs")),
        outputs: vars_of(item.field("outputs")),
        tags: tags_of(item),
    })
}

/// Every module and made figure among `items`, in order.
pub fn knowledge(items: &[Item]) -> Knowledge {
    Knowledge { modules: items.iter().filter_map(module_note).collect(), examples: items.iter().filter_map(example).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Source, format};
    use serde_json::json;

    const DSL: &str = "fig t\nx: image \"Scan\" role=input var=x\naug: box \"Aug\"\nseg: unet \"Seg\"\np: image \"Pred\" role=output\ny: image \"Mask\" role=gt look=mask\nloss: loss \"$L$\"\nx -> aug -> seg -> p\ny -> loss : supervision\np -> loss\n";

    fn spec(dsl: &str, discussion: Value) -> FlowSpec {
        FlowSpec::from_value(json!({"lore_flow": 1, "dsl": dsl, "discussion": discussion})).unwrap()
    }

    #[test]
    fn inputs_and_outputs_come_from_the_roles() {
        let d = derive(&spec(DSL, json!({"source": " Segment a scan. "})), &Library::default());
        assert_eq!(d.description, "Segment a scan.");
        let ids = |v: &[Var]| v.iter().map(|v| (v.id.clone(), v.kind.clone(), v.meaning.clone())).collect::<Vec<_>>();
        assert_eq!(ids(&d.inputs), [("x".into(), "image".into(), "Scan".into()), ("y".into(), "mask".into(), "Mask".into())]);
        assert_eq!(ids(&d.outputs), [("p".into(), "image".into(), "Pred".into()), ("loss".into(), "loss".into(), "$L$".into())]);
    }

    #[test]
    fn connections_are_one_line_per_edge_with_its_kind() {
        let d = derive(&spec(DSL, json!(null)), &Library::default());
        assert_eq!(d.description, "");
        assert_eq!(d.connects, ["Scan -> Aug", "Aug -> Seg", "Seg -> Pred", "Mask -> loss : supervision", "Pred -> loss"]);
    }

    #[test]
    fn fill_never_overwrites_the_authors_words() {
        let d = derive(&spec(DSL, json!({"source": "Derived text."})), &Library::default());
        let mut it = Item::new("c", format::FIGURE, "F", Source::author());
        it.set("description", "Mine.").set("outputs", "q | scalar | mine");
        fill(&mut it, &d);
        assert_eq!((it.field("description"), it.field("outputs")), ("Mine.", "q | scalar | mine"));
        assert!(it.field("inputs").starts_with("x | image | Scan\ny | mask | Mask"));
        assert!(it.field("connects").starts_with("Scan -> Aug\n"));
        assert_eq!(it.field("prompt"), "");
        // what was derived round-trips through the parser
        assert_eq!(vars_of(it.field("inputs")).len(), 2);
    }

    #[test]
    fn a_spec_that_does_not_parse_derives_nothing() {
        assert_eq!(derive(&spec("nonsense (((", json!({"source": "x"})), &Library::default()), Derived::default());
    }

    #[test]
    fn modules_and_made_figures_become_knowledge() {
        let mut m = Item::new("c", format::FIGURE, "U-Net", Source::lore("sc"));
        m.id = "c:unet.bars".into();
        m.set("what", "generator").set("kind", "unet").set("style", "bars").set("prompt", "For masks.").set("inputs", "x | image | scan\nz | weird | q");
        m.tag(facet::DOMAIN, "medical").tag(facet::KIND, "unet");
        let mut f = Item::new("c", format::FIGURE, "Whole", Source::author());
        f.id = "c:k1".into();
        f.set("what", "spec").set("params", json!({"dsl": DSL})).set("description", "Derived.").set("outputs", "p | image | pred");
        let mut r = Item::new("c", format::FIGURE, "Ref", Source::author());
        r.set("what", "reference");
        let k = knowledge(&[m, f.clone(), r]);
        assert_eq!(k.modules.len(), 1);
        let n = &k.modules[0];
        assert_eq!((n.kind.as_str(), n.style.as_str(), n.prompt.as_str()), ("unet", "bars", "For masks."));
        assert_eq!(n.inputs[1].kind, "other");
        assert_eq!(n.tags, ["medical", "unet"]);
        assert_eq!(k.examples.len(), 1);
        let e = &k.examples[0];
        assert_eq!((e.id.as_str(), e.description.as_str(), e.dsl.as_str()), ("c:k1", "Derived.", DSL));
        f.set("prompt", "Author's words.");
        assert_eq!(example(&f).unwrap().description, "Author's words.");
    }
}
