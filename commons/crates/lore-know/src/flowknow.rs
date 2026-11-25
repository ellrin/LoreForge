//! Figures as knowledge: the variables a piece takes and gives, how its parts connect, what can be
//! worked out from a made figure by code, and the bridge to `lore_flow::knowledge` (what a draft
//! is handed).
//!
//! The text forms stored in a figure item's fields:
//!
//! * `inputs`, `outputs`: one variable per line, `id | kind | meaning`; `kind` is one of
//!   [`VAR_KINDS`] or empty (= `other`); empty lines and lines starting with `#` are skipped.
//! * `connects`: one line per connection, `from -> to` or `from -> to : kind`.
//! * `commands` (指揮文字), `spoken` (口語說明): how the figure was commanded, in the five steps,
//!   and how one would say it; both derived by code and kept once the author has edited them.
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
    /// 指揮文字: the text that makes the figure, under the author's five steps.
    pub commands: String,
    /// 口語說明: how one would say it.
    pub spoken: String,
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
    if let Some(t) = &spec.timeline {
        return Derived { commands: timeline_commands(t), spoken: timeline_spoken(t), ..Default::default() };
    }
    let Ok(g) = spec.graph(lib) else { return Derived::default() };
    let description = spec.discussion.get("source").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let (mut inputs, mut outputs): (Vec<Var>, Vec<Var>) = (vec![], vec![]);
    for n in &g.nodes {
        let role = n.role_or_default();
        // what a figure takes and gives is read from its flow, since few figures name their
        // roles: a piece of data nothing flows into is an input, one nothing flows on from is
        // an output. A named role (input, gt, output, loss) decides by itself; models and
        // operations are never variables.
        let fed = g.edges.iter().any(|e| e.to == n.id);
        let feeds = g.edges.iter().any(|e| e.from == n.id);
        let list = match role {
            "input" | "gt" => &mut inputs,
            "output" | "loss" => &mut outputs,
            "model" | "op" | "merge" => continue,
            _ if !fed && feeds => &mut inputs,
            _ if fed && !feeds => &mut outputs,
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
    Derived { description, inputs, outputs, connects, commands: commands_of_dsl(&spec.dsl), spoken: spoken_of_graph(&g) }
}

// ── how the figure is commanded ──────────────────────────────────────────────

/// The DSL text regrouped under the five steps (積木, 標籤, 串起來, 區域, 對齊); a line that fits
/// nowhere goes first under `# 0 開頭`. Lines are classified by their first token or by `->`.
pub fn commands_of_dsl(dsl: &str) -> String {
    let (mut head, mut nodes, mut edges, mut tail, mut align) = (vec![], vec![], vec![], vec![], vec![]);
    // blocks: (opening line, member node ids), nested ones open on a stack
    let mut done: Vec<(String, Vec<String>)> = vec![];
    let mut open: Vec<(String, Vec<String>)> = vec![];
    for raw in dsl.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        let first = t.split_whitespace().next().unwrap_or("");
        // an arrow counts only outside quotes (a label may say "A -> B")
        let arrow = t.split('"').step_by(2).any(|part| part.contains("->"));
        let node = first.len() > 1 && first.ends_with(':');
        if arrow && !node && !matches!(first, "panel" | "region") {
            edges.push(t.to_string());
        } else if matches!(first, "panel" | "region") {
            open.push((t.to_string(), vec![]));
        } else if first == "end" {
            if let Some((l, m)) = open.pop() {
                done.push((l, m));
            }
        } else if first == "arrange" {
            tail.push(t.to_string());
        } else if first == "align" {
            align.push(t.to_string());
        } else if first.len() > 1 && first.ends_with(':') {
            nodes.push(t.to_string());
            if let Some((_, m)) = open.last_mut() {
                m.push(first.trim_end_matches(':').to_string());
            }
        } else if !open.is_empty() && !t.contains(':') && !t.starts_with('#') {
            // inside a region: a line of ids names its members
            if let Some((_, m)) = open.last_mut() {
                m.extend(t.split_whitespace().map(str::to_string));
            }
        } else {
            head.push(t.to_string());
        }
    }
    // a block never closed is still a block
    while let Some(b) = open.pop() {
        done.push(b);
    }
    // blocks come in the order they were opened
    let mut regions: Vec<String> = vec![];
    for (l, m) in &done {
        regions.push(l.clone());
        if !m.is_empty() {
            regions.push(format!("  {}", m.join(" ")));
        }
        regions.push("end".to_string());
    }
    regions.extend(tail);
    let mut out = String::new();
    let mut section = |title: &str, lines: &[String]| {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(title);
        for l in lines {
            out.push('\n');
            out.push_str(l);
        }
    };
    if !head.is_empty() {
        section("# 0 開頭", &head);
    }
    section("# 1 積木", &nodes);
    section("# 2 標籤", &["（每個積木的名字與標籤寫在上面那一行）".to_string()]);
    section("# 3 串起來", &edges);
    section("# 4 區域", &regions);
    section("# 5 對齊", &align);
    out
}

fn timeline_commands(t: &lore_flow::timeline::TimelineSpec) -> String {
    let one = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "/");
    let mut out = String::from("# 1 點");
    for p in &t.points {
        out.push_str(&format!("\n{} | {} | {} | {}", one(&p.time), one(&p.title), one(&p.body), one(&p.icon)));
    }
    out.push_str(&format!("\n# 2 樣式\n{}", t.style.trim()));
    out
}

const MAX_SPOKEN: usize = 60;

fn capped(mut sentences: Vec<String>) -> String {
    if sentences.len() > MAX_SPOKEN {
        let k = sentences.len() - MAX_SPOKEN;
        sentences.truncate(MAX_SPOKEN);
        sentences.push(format!("…（其餘 {k} 句省略）"));
    }
    sentences.join("\n")
}

fn timeline_spoken(t: &lore_flow::timeline::TimelineSpec) -> String {
    let one = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let pts: Vec<String> = t.points.iter().map(|p| format!("{} {}", one(&p.time), one(&p.title)).trim().to_string()).collect();
    let mut s = vec![format!("一條時間軸，共 {} 個點：{}。", pts.len(), pts.join("、"))];
    if pts.len() > MAX_SPOKEN {
        s = vec![format!("一條時間軸，共 {} 個點。", pts.len())];
    }
    capped(s)
}

fn spoken_of_graph(g: &lore_flow::spec::Graph) -> String {
    let name = |id: &str| -> String {
        let l = g.nodes.iter().find(|n| n.id == id).map(|n| n.label.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        if l.is_empty() { id.to_string() } else { l }
    };
    let list = |ids: Vec<&str>| ids.into_iter().map(name).collect::<Vec<_>>().join("、");
    let mut out: Vec<String> = vec![];
    let titled = |title: &str, id: &str| if title.trim().is_empty() { id.to_string() } else { title.split_whitespace().collect::<Vec<_>>().join(" ") };
    for p in g.panels.iter().filter(|p| !p.bare) {
        let inside = list(g.nodes.iter().filter(|n| n.panel == p.id).map(|n| n.id.as_str()).collect());
        let t = titled(&p.title, &p.id);
        out.push(if inside.is_empty() { format!("先放〈{t}〉這一區。") } else { format!("先放〈{t}〉這一區，裡面是 {inside}。") });
    }
    for r in &g.regions {
        let inside = list(r.members.iter().map(String::as_str).collect());
        let t = titled(&r.title, &r.id);
        out.push(if inside.is_empty() { format!("先放〈{t}〉這一區。") } else { format!("先放〈{t}〉這一區，裡面是 {inside}。") });
    }
    for e in &g.edges {
        let kind = if e.kind == lore_flow::spec::EdgeKind::Data { String::new() } else { format!("（{}）", e.kind.as_str()) };
        let label = e.label.split_whitespace().collect::<Vec<_>>().join(" ");
        let label = if label.is_empty() { String::new() } else { format!("，標〈{label}〉") };
        out.push(format!("{} 接到 {}{kind}{label}。", name(&e.from), name(&e.to)));
    }
    for a in &g.align {
        let how = if a.axis == "col" { "排成一欄" } else { "排成一列" };
        out.push(format!("{} {how}。", list(a.ids.iter().map(String::as_str).collect())));
    }
    capped(out)
}

/// Put `d` into the item's empty fields (`description`, `inputs`, `outputs`, `connects`, `commands`, `spoken`); what the
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
    put("commands", d.commands.clone());
    put("spoken", d.spoken.clone());
}

/// Empty the fields of `item` that still hold exactly what its present spec derives (nobody
/// edited them), so a save with a changed spec fills them afresh; `also` names fields to empty
/// whatever they hold. Call it BEFORE the new spec is put into `params`. What the author wrote
/// differs from the derived text and stays. `prompt` is never touched.
pub fn forget_derived(item: &mut Item, lib: &Library, also: &[&str]) {
    let Some(spec) = spec_of(item) else { return };
    let d = derive(&spec, lib);
    let was = [("inputs", vars_text(&d.inputs)), ("outputs", vars_text(&d.outputs)), ("connects", d.connects.join("\n")), ("commands", d.commands.clone()), ("spoken", d.spoken.clone())];
    for (name, text) in was {
        if also.contains(&name) || (!text.trim().is_empty() && item.field(name).trim() == text.trim()) {
            item.set(name, String::new());
        }
    }
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
    let commands = item.field("commands").trim();
    let dsl = match item.fields.get("params") {
        Some(Value::Object(o)) => o.get("dsl").and_then(Value::as_str).unwrap_or("").to_string(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok().and_then(|v| v.get("dsl").and_then(Value::as_str).map(str::to_string)).unwrap_or_default(),
        _ => String::new(),
    };
    // how the figure was commanded, when it is known, is what the draft reads in place of the raw text
    let dsl = if commands.is_empty() { dsl } else { commands.to_string() };
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
    fn without_named_roles_the_flow_says_what_goes_in_and_out() {
        let spec = FlowSpec { dsl: "fig t\nx: image \"Input\"\nf: box \"Model\"\ny: label \"Output\"\nx -> f -> y\n".into(), ..Default::default() };
        let d = derive(&spec, &Library::default());
        assert_eq!(d.inputs.iter().map(|v| (v.id.as_str(), v.kind.as_str())).collect::<Vec<_>>(), [("x", "image")]);
        assert_eq!(d.outputs.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["y"]);
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

    const PANEL_DSL: &str = "fig t\nlegend on\narrange a / b\npanel a \"Train\" dir=right\n  x: image \"Scan\" role=input\n  seg: unet \"Seg\"\n  x -> seg\nend\ny: image \"Mask\" role=gt\ny -> seg : supervision \"mask\"\nalign row x y\n";

    #[test]
    fn commands_regroup_the_text_under_the_five_steps() {
        let c = commands_of_dsl(DSL);
        let heads: Vec<&str> = c.lines().filter(|l| l.starts_with("# ")).collect();
        assert_eq!(heads, ["# 0 開頭", "# 1 積木", "# 2 標籤", "# 3 串起來", "# 4 區域", "# 5 對齊"]);
        let at = |h: &str| c.lines().position(|l| l == h).unwrap();
        assert_eq!(c.lines().nth(at("# 0 開頭") + 1), Some("fig t"));
        let nodes: Vec<&str> = c.lines().skip(at("# 1 積木") + 1).take_while(|l| !l.starts_with('#')).collect();
        assert_eq!(nodes.len(), 6);
        assert!(nodes[0].starts_with("x: image \"Scan\""));
        assert!(c.contains("# 2 標籤\n（每個積木的名字與標籤寫在上面那一行）\n# 3 串起來\nx -> aug -> seg -> p\ny -> loss : supervision\np -> loss\n# 4 區域"));
    }

    #[test]
    fn a_node_inside_a_panel_is_a_block_and_a_member() {
        let c = commands_of_dsl(PANEL_DSL);
        assert!(c.contains("# 1 積木\nx: image \"Scan\" role=input\nseg: unet \"Seg\"\ny: image \"Mask\" role=gt\n# 2"), "{c}");
        assert!(c.contains("# 4 區域\npanel a \"Train\" dir=right\n  x seg\nend\narrange a / b\n# 5 對齊\nalign row x y"), "{c}");
        assert!(c.contains("# 3 串起來\nx -> seg\ny -> seg : supervision \"mask\"\n"), "{c}");
    }

    #[test]
    fn a_timelines_commands_list_its_points() {
        use lore_flow::timeline::{Point, TimelineSpec};
        let t = TimelineSpec {
            style: "chevrons".into(),
            points: vec![
                Point { time: "2020".into(), title: "Start".into(), body: "a\nb".into(), icon: "flag".into(), ..Default::default() },
                Point { time: "2021".into(), title: "End".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let spec = FlowSpec { timeline: Some(t), ..Default::default() };
        let d = derive(&spec, &Library::default());
        assert_eq!(d.commands, "# 1 點\n2020 | Start | a b | flag\n2021 | End |  | \n# 2 樣式\nchevrons");
        assert_eq!(d.spoken, "一條時間軸，共 2 個點：2020 Start、2021 End。");
    }

    #[test]
    fn spoken_says_regions_then_lines_then_alignment() {
        let mut g = lore_flow::dsl::parse(&PANEL_DSL.replace("align row x y\n", "").replace("arrange a / b", "arrange a")).unwrap();
        g.align.push(lore_flow::spec::Align { axis: "row".into(), ids: vec!["x".into(), "y".into()] });
        let s = spoken_of_graph(&g);
        assert_eq!(s, "先放〈Train〉這一區，裡面是 Scan、Seg。\nScan 接到 Seg。\nMask 接到 Seg（supervision），標〈mask〉。\nScan、Mask 排成一列。");
    }

    #[test]
    fn spoken_is_capped() {
        let mut dsl = String::from("fig t\n");
        for i in 0..70 {
            dsl.push_str(&format!("a{i}: box \"A{i}\"\n"));
        }
        for i in 0..69 {
            dsl.push_str(&format!("a{i} -> a{}\n", i + 1));
        }
        let d = derive(&spec(&dsl, json!(null)), &Library::default());
        let lines: Vec<&str> = d.spoken.lines().collect();
        assert_eq!(lines.len(), 61);
        assert_eq!(lines[60], "…（其餘 9 句省略）");
    }

    #[test]
    fn commands_and_spoken_are_filled_only_when_empty() {
        let d = derive(&spec(DSL, json!(null)), &Library::default());
        assert!(d.commands.starts_with("# 0 開頭") && d.spoken.starts_with("Scan 接到 Aug。"), "{d:?}");
        let mut it = Item::new("c", format::FIGURE, "F", Source::author());
        it.set("commands", "my own words");
        fill(&mut it, &d);
        assert_eq!(it.field("commands"), "my own words");
        assert_eq!(it.field("spoken"), d.spoken);
    }

    #[test]
    fn the_authors_own_usage_is_never_written() {
        let mut it = Item::new("c", format::FIGURE, "F", Source::author());
        it.set("what", "spec").set("params", json!({"dsl": DSL}));
        fill_item(&mut it, &Library::default());
        assert!(!it.field("commands").is_empty() && !it.field("spoken").is_empty());
        assert_eq!(it.field("prompt"), "");
    }

    #[test]
    fn a_made_figures_commands_reach_the_next_draft() {
        let mut f = Item::new("c", format::FIGURE, "Whole", Source::author());
        f.set("what", "spec").set("params", json!({"dsl": DSL}));
        assert_eq!(example(&f).unwrap().dsl, DSL);
        fill_item(&mut f, &Library::default());
        let e = example(&f).unwrap();
        assert!(e.dsl.starts_with("# 0 開頭\nfig t") && e.dsl.contains("# 3 串起來"), "{}", e.dsl);
    }
    /// Found by a review of the joined pieces: a region written with a line of ids lost its
    /// members to the heading, and a label with an arrow in it was filed as a line.
    #[test]
    fn a_regions_id_line_is_its_members_and_an_arrow_in_a_label_is_no_line() {
        let c = commands_of_dsl("fig t\na: box \"A -> B\"\nb: box \"B\"\nregion r \"R\" dashed\n  a b\nend\na -> b\n");
        let part = |h: &str| c.split("\n# ").find(|p| p.starts_with(h) || p.starts_with(&format!("# {h}"))).unwrap_or("").to_string();
        assert!(part("1 積木").contains("a: box \"A -> B\""), "{c}");
        assert!(!part("3 串起來").contains("box"), "{c}");
        assert!(part("3 串起來").contains("a -> b"), "{c}");
        assert!(part("4 區域").contains("region r \"R\" dashed\n  a b\nend"), "{c}");
        assert!(!part("0 開頭").contains("a b"), "{c}");
    }
}
