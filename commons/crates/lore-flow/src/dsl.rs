//! A short text form of the L1 graph, meant to be typed and corrected by hand.
//!
//! ```text
//! fig demo
//! arrange a / b c                     # rows of panels ("template rows|cols" also works)
//! panel a "a. Training" dir=right
//!   x:    image "$x$"
//!   aug:  box "Aug($x$)"
//!   seg:  unet "$\theta_{seg}$" depth=3
//!   loss: loss "$\mathcal L_{seg}$"
//!   y:    image "$y$" role=gt look=mask
//!   x -> aug -> seg -> loss
//!   y -> loss : supervision
//!   loss -> seg : feedback "update"
//! end
//! var w_b = "\omega_b" "weight of batch b"
//! ```
//!
//! One statement per line; `#` starts a comment. Node keys `role`, `var`,
//! `caption`, `style` are fields, every other `key=value` is a module parameter.

use serde_json::Value;

use crate::spec::*;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct DslError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
}

impl Tok {
    fn s(&self) -> &str {
        match self {
            Tok::Word(s) | Tok::Quoted(s) => s,
        }
    }
}

fn tokens(line: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let cs: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '#' {
            break;
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            let mut closed = false;
            while i < cs.len() {
                if cs[i] == '\\' && cs.get(i + 1) == Some(&'"') {
                    s.push('"');
                    i += 2;
                } else if cs[i] == '"' {
                    closed = true;
                    i += 1;
                    break;
                } else {
                    s.push(cs[i]);
                    i += 1;
                }
            }
            if !closed {
                return Err("a quoted text is not closed".into());
            }
            // key="value" glues onto the word before it
            if let Some(Tok::Word(w)) = out.last_mut()
                && w.ends_with('=')
            {
                w.push('"');
                w.push_str(&s);
                continue;
            }
            out.push(Tok::Quoted(s));
        } else {
            let mut s = String::new();
            let mut depth = 0;
            while i < cs.len() {
                let c = cs[i];
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                } else if depth == 0 && (c.is_whitespace() || c == '"') {
                    break;
                }
                s.push(c);
                i += 1;
            }
            out.push(Tok::Word(s));
        }
    }
    Ok(out)
}

fn value(s: &str) -> Value {
    if let Some(q) = s.strip_prefix('"') {
        return Value::String(q.to_string());
    }
    if s == "true" || s == "false" {
        return Value::Bool(s == "true");
    }
    // six hex digits are a colour even when they read as a number (123456, 000000, 12E345)
    if s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Value::String(s.to_string());
    }
    if let Ok(n) = s.parse::<i64>() {
        return Value::from(n);
    }
    if let Ok(n) = s.parse::<f64>() {
        return Value::from(n);
    }
    if s.starts_with('[') && s.ends_with(']') {
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            return v;
        }
        let inner = &s[1..s.len() - 1];
        return Value::Array(inner.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).map(value).collect());
    }
    Value::String(s.to_string())
}

/// Parse the text form into a graph (validated).
pub fn parse(text: &str) -> Result<Graph, DslError> {
    parse_with(text, &[])
}

/// As [`parse`], also accepting the template kinds of a module library.
pub fn parse_with(text: &str, extra_modules: &[String]) -> Result<Graph, DslError> {
    let mut g = Graph { fig: "fig".into(), ..Default::default() };
    let mut panel = String::new();
    // open `region … end` blocks (indexes into g.regions), innermost last
    let mut open: Vec<usize> = Vec::new();
    // regions that named a panel (`in=` or the panel they were written in)
    let mut region_in: Vec<(usize, String)> = Vec::new();
    let mut last = 0;
    for (ln, raw) in text.lines().enumerate() {
        let line = ln + 1;
        last = line;
        let err = |m: String| DslError { line, message: m };
        let toks = tokens(raw).map_err(&err)?;
        if toks.is_empty() {
            continue;
        }
        let head = toks[0].s().to_string();
        let is_word = matches!(toks[0], Tok::Word(_));
        if toks.iter().any(|t| *t == Tok::Word("->".into())) {
            // a -> b -> c [: kind] ["label"]
            let mut chain: Vec<String> = Vec::new();
            let mut kind = EdgeKind::Data;
            let mut label = String::new();
            let mut color = String::new();
            let mut curve = false;
            let mut width = 0.0f64;
            let mut i = 0;
            let mut expect_node = true;
            while i < toks.len() {
                match &toks[i] {
                    Tok::Word(w) if w == "->" => {
                        if expect_node {
                            return Err(err("an arrow needs a node on both sides".into()));
                        }
                        expect_node = true;
                    }
                    Tok::Word(w) if w == ":" || w.starts_with(':') || (w.ends_with(':') && expect_node) => {
                        // "b : kind", "b :kind" or "b: kind"
                        let (node, k) = match w.split_once(':') {
                            Some((a, b)) => (a.to_string(), b.to_string()),
                            None => (String::new(), String::new()),
                        };
                        if !node.is_empty() {
                            chain.push(node);
                        }
                        let k = if k.is_empty() {
                            i += 1;
                            toks.get(i).map(|t| t.s().to_string()).unwrap_or_default()
                        } else {
                            k
                        };
                        kind = EdgeKind::parse(&k).ok_or_else(|| err(format!("unknown line kind {k:?} ({})", EdgeKind::ALL.map(|k| k.as_str()).join(", "))))?;
                        expect_node = false;
                    }
                    Tok::Word(w) if expect_node => {
                        chain.push(w.clone());
                        expect_node = false;
                    }
                    Tok::Word(w) if !expect_node && w == "curve" => curve = true,
                    Tok::Word(w) if !expect_node && w.starts_with("color=") => color = w["color=".len()..].trim_start_matches('"').to_string(),
                    Tok::Word(w) if !expect_node && w.starts_with("width=") => {
                        width = w["width=".len()..].trim_matches('"').parse().map_err(|_| err(format!("a line's width is a number of points (got {w:?})")))?;
                    }
                    Tok::Quoted(q) if !expect_node => label = q.clone(),
                    t => return Err(err(format!("did not expect {:?} here", t.s()))),
                }
                i += 1;
            }
            if chain.len() < 2 || expect_node {
                return Err(err("an arrow needs a node on both sides".into()));
            }
            for w in chain.windows(2) {
                // `x>e1 -> c`: the line leaves the line `x>e1`
                let (from, off) = match w[0].split_once('>') {
                    Some((f, _)) => (f.to_string(), w[0].clone()),
                    None => (w[0].clone(), String::new()),
                };
                g.edges.push(Edge { from, to: w[1].clone(), kind, label: label.clone(), color: color.clone(), curve, off, width });
            }
            continue;
        }
        if is_word && head.ends_with(':') && head.len() > 1 {
            // id: module "label" key=value …
            let id = head.trim_end_matches(':').to_string();
            if let Some(&r) = open.last() {
                g.regions[r].members.push(id.clone());
            }
            let mut n = Node { id, module: "box".into(), panel: panel.clone(), ..Default::default() };
            let mut seen_module = false;
            for t in &toks[1..] {
                match t {
                    Tok::Quoted(q) => {
                        if n.label.is_empty() {
                            n.label = q.clone();
                        } else if n.caption.is_empty() {
                            n.caption = q.clone();
                        } else {
                            return Err(err("a node takes a label and at most one caption".into()));
                        }
                    }
                    Tok::Word(w) => match w.split_once('=') {
                        Some((k, v)) if !k.is_empty() => match k {
                            "role" => n.role = v.trim_start_matches('"').to_string(),
                            "var" => n.var = v.trim_start_matches('"').to_string(),
                            "style" => n.style = v.trim_start_matches('"').to_string(),
                            "caption" => n.caption = v.trim_start_matches('"').to_string(),
                            "label" => n.label = v.trim_start_matches('"').to_string(),
                            _ => {
                                n.params.insert(k.to_string(), value(v));
                            }
                        },
                        _ if !seen_module && n.label.is_empty() => {
                            n.module = w.clone();
                            seen_module = true;
                        }
                        _ => {
                            // a bare word after the module is a flag
                            n.params.insert(w.clone(), Value::Bool(true));
                        }
                    },
                }
            }
            g.nodes.push(n);
            continue;
        }
        const KEYWORDS: [&str; 10] = ["fig", "template", "legend", "arrange", "panel", "end", "var", "region", "align", "arrange"];
        if !open.is_empty() && !KEYWORDS.contains(&head.as_str()) {
            // members of the innermost region: node ids separated by spaces
            let r = *open.last().unwrap();
            g.regions[r].members.extend(toks.iter().map(|t| t.s().to_string()));
            continue;
        }
        match head.as_str() {
            "region" => {
                let id = toks.get(1).map(|t| t.s().to_string()).ok_or_else(|| err("region needs an id".into()))?;
                let mut r = Region { id, ..Default::default() };
                let mut in_panel = String::new();
                for t in &toks[2..] {
                    match t {
                        Tok::Quoted(q) => r.title = q.clone(),
                        Tok::Word(w) if matches!(w.as_str(), "solid" | "dashed" | "dotted" | "none") => r.line = w.clone(),
                        Tok::Word(w) if w == "round" => r.round = true,
                        Tok::Word(w) => match w.split_once('=') {
                            Some(("tint", v)) => r.tint = v.trim_start_matches('"').to_string(),
                            Some(("title", v)) => r.title_at = v.trim_start_matches('"').to_string(),
                            Some(("line", v)) => r.line = v.trim_start_matches('"').to_string(),
                            Some(("in", v)) => in_panel = v.trim_start_matches('"').to_string(),
                            Some(("parent", v)) => r.parent = v.trim_start_matches('"').to_string(),
                            _ => return Err(err(format!("region takes a title, dashed|dotted|solid|none, tint=, round, title=, in= and parent= (got {w:?})"))),
                        },
                    }
                }
                if r.parent.is_empty()
                    && let Some(&p) = open.last()
                {
                    r.parent = g.regions[p].id.clone();
                }
                if in_panel.is_empty() {
                    in_panel = panel.clone();
                }
                g.regions.push(r);
                let idx = g.regions.len() - 1;
                if !in_panel.is_empty() {
                    region_in.push((idx, in_panel));
                }
                open.push(idx);
            }
            "align" => {
                let axis = toks.get(1).map(|t| t.s().to_string()).ok_or_else(|| err("align needs row or col".into()))?;
                g.align.push(Align { axis, ids: toks[2..].iter().map(|t| t.s().to_string()).collect() });
            }
            "fig" => g.fig = toks.get(1).map(|t| t.s().to_string()).ok_or_else(|| err("fig needs an id".into()))?,
            "template" => g.template = toks.get(1).map(|t| t.s().to_string()).ok_or_else(|| err("template needs a name".into()))?,
            "legend" => {
                g.legend = match toks.get(1).map(|t| t.s()) {
                    Some("on") | Some("true") | None => Some(true),
                    Some("off") | Some("false") => Some(false),
                    Some("auto") => None,
                    Some(o) => return Err(err(format!("legend on, off or auto (got {o:?})"))),
                }
            }
            "arrange" => {
                let mut rows = vec![vec![]];
                for t in &toks[1..] {
                    if t.s() == "/" {
                        rows.push(vec![]);
                    } else {
                        rows.last_mut().unwrap().push(t.s().to_string());
                    }
                }
                rows.retain(|r: &Vec<String>| !r.is_empty());
                g.arrange = rows;
            }
            "panel" => {
                let id = toks.get(1).map(|t| t.s().to_string()).ok_or_else(|| err("panel needs an id".into()))?;
                let mut p = Panel { id: id.clone(), ..Default::default() };
                for t in &toks[2..] {
                    match t {
                        Tok::Quoted(q) => p.title = q.clone(),
                        Tok::Word(w) if w == "bare" => p.bare = true,
                        Tok::Word(w) if matches!(w.as_str(), "solid" | "dashed" | "dotted" | "none") => p.line = w.clone(),
                        Tok::Word(w) if w == "round" => p.round = true,
                        Tok::Word(w) => match w.split_once('=') {
                            Some(("dir", v)) if v == "right" || v == "down" => p.dir = v.to_string(),
                            Some(("title", v)) if matches!(v, "top-left" | "top" | "above" | "below" | "left" | "none") => p.title_at = v.to_string(),
                            Some(("title", v)) => p.title = v.trim_start_matches('"').to_string(),
                            Some(("tint", v)) => p.tint = v.trim_start_matches('"').to_string(),
                            Some(("line", v)) => p.line = v.trim_start_matches('"').to_string(),
                            _ => return Err(err(format!("panel takes a title, dir=right|down, bare, dashed|dotted|solid|none, tint=, round and title= (got {w:?})"))),
                        },
                    }
                }
                g.panels.push(p);
                panel = id;
            }
            "end" => {
                if open.pop().is_none() {
                    panel.clear();
                }
            }
            "var" => {
                // var id = "latex" ["meaning"]
                // (a quote right after `=` is glued onto the word before it)
                let mut id = String::new();
                let mut strings: Vec<String> = Vec::new();
                for t in &toks[1..] {
                    match t {
                        Tok::Quoted(q) => strings.push(q.clone()),
                        Tok::Word(w) => {
                            let (a, b) = match w.split_once("=\"") {
                                Some((a, b)) => (a, Some(b)),
                                None => (w.trim_end_matches('='), None),
                            };
                            if id.is_empty() && !a.is_empty() {
                                id = a.to_string();
                            }
                            if let Some(b) = b {
                                strings.push(b.to_string());
                            }
                        }
                    }
                }
                if id.is_empty() || strings.is_empty() {
                    return Err(err("var needs an id and its LaTeX: var w = \"\\omega\"".into()));
                }
                let latex = crate::spec::bare_latex(&strings.remove(0));
                let meaning = strings.into_iter().next().unwrap_or_default();
                g.vars_new.push(VarNew { id, latex, meaning });
            }
            _ => return Err(err(format!("did not understand {head:?} (expected fig, template, arrange, legend, panel, end, var, `id: module \"label\"` or `a -> b`)"))),
        }
    }
    // `in=` (or the panel a region was written in): members without a panel get it
    for (idx, p) in &region_in {
        let mut ids: Vec<String> = Vec::new();
        let mut todo = vec![g.regions[*idx].id.clone()];
        let mut guard = 0;
        while let Some(r) = todo.pop() {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            for x in g.regions.iter().filter(|x| x.id == r) {
                ids.extend(x.members.iter().cloned());
            }
            todo.extend(g.regions.iter().filter(|x| x.parent == r && x.id != r).map(|x| x.id.clone()));
        }
        for n in g.nodes.iter_mut().filter(|n| n.panel.is_empty() && ids.contains(&n.id)) {
            n.panel = p.clone();
        }
    }
    g.validate(extra_modules).map_err(|e| DslError { line: last, message: e.to_string() })?;
    Ok(g)
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

fn param(v: &Value) -> String {
    match v {
        Value::String(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && s.parse::<f64>().is_err() && s != "true" && s != "false" => s.clone(),
        Value::String(s) => quote(s),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
        other => other.to_string(),
    }
}

fn node_line(n: &Node, indent: &str) -> String {
    let mut s = format!("{indent}{}: {}", n.id, n.module);
    if !n.label.is_empty() {
        s.push(' ');
        s.push_str(&quote(&n.label));
    }
    if !n.role.is_empty() {
        s.push_str(&format!(" role={}", n.role));
    }
    if !n.var.is_empty() {
        s.push_str(&format!(" var={}", n.var));
    }
    if !n.style.is_empty() {
        s.push_str(&format!(" style={}", n.style));
    }
    if !n.caption.is_empty() {
        s.push_str(&format!(" caption={}", quote(&n.caption)));
    }
    for (k, v) in &n.params {
        s.push_str(&format!(" {k}={}", param(v)));
    }
    s
}

/// The text form of a graph (what the author edits before drawing).
/// `parse(to_dsl(g)) == g` for every valid graph.
pub fn to_dsl(g: &Graph) -> String {
    let mut out = format!("fig {}\n", g.fig);
    if !g.template.is_empty() {
        out.push_str(&format!("template {}\n", g.template));
    }
    if !g.arrange.is_empty() {
        out.push_str(&format!("arrange {}\n", g.arrange.iter().map(|r| r.join(" ")).collect::<Vec<_>>().join(" / ")));
    }
    match g.legend {
        Some(true) => out.push_str("legend on\n"),
        Some(false) => out.push_str("legend off\n"),
        None => {}
    }
    // panels first (declaration order is kept by writing every panel header, then its nodes)
    let edge_line = |e: &Edge, indent: &str| {
        let from = if e.off.is_empty() { &e.from } else { &e.off };
        let mut s = format!("{indent}{from} -> {}", e.to);
        if e.kind != EdgeKind::Data {
            s.push_str(&format!(" : {}", e.kind.as_str()));
        }
        if !e.label.is_empty() {
            s.push(' ');
            s.push_str(&quote(&e.label));
        }
        if !e.color.is_empty() {
            s.push_str(&format!(" color={}", e.color));
        }
        if e.curve {
            s.push_str(" curve");
        }
        if e.width != 0.0 {
            s.push_str(&format!(" width={}", e.width));
        }
        s
    };
    let free: Vec<&Node> = g.nodes.iter().filter(|n| n.panel.is_empty()).collect();
    for n in &free {
        out.push_str(&node_line(n, ""));
        out.push('\n');
    }
    for p in &g.panels {
        out.push_str(&format!("panel {}", p.id));
        if !p.title.is_empty() {
            out.push(' ');
            out.push_str(&quote(&p.title));
        }
        if !p.dir.is_empty() {
            out.push_str(&format!(" dir={}", p.dir));
        }
        if p.bare {
            out.push_str(" bare");
        }
        if !p.line.is_empty() {
            if matches!(p.line.as_str(), "solid" | "dashed" | "dotted" | "none") {
                out.push_str(&format!(" {}", p.line));
            } else {
                out.push_str(&format!(" line={}", param(&Value::String(p.line.clone()))));
            }
        }
        if !p.tint.is_empty() {
            out.push_str(&format!(" tint={}", p.tint));
        }
        if p.round {
            out.push_str(" round");
        }
        if !p.title_at.is_empty() {
            out.push_str(&format!(" title={}", p.title_at));
        }
        out.push('\n');
        for n in g.nodes.iter().filter(|n| n.panel == p.id) {
            out.push_str(&node_line(n, "  "));
            out.push('\n');
        }
    }
    if !g.panels.is_empty() {
        out.push_str("end\n");
    }
    for r in &g.regions {
        out.push_str(&format!("region {}", r.id));
        if !r.title.is_empty() {
            out.push(' ');
            out.push_str(&quote(&r.title));
        }
        if !r.line.is_empty() {
            if matches!(r.line.as_str(), "solid" | "dashed" | "dotted" | "none") {
                out.push_str(&format!(" {}", r.line));
            } else {
                out.push_str(&format!(" line={}", param(&Value::String(r.line.clone()))));
            }
        }
        if !r.tint.is_empty() {
            out.push_str(&format!(" tint={}", r.tint));
        }
        if r.round {
            out.push_str(" round");
        }
        if !r.title_at.is_empty() {
            out.push_str(&format!(" title={}", r.title_at));
        }
        if !r.parent.is_empty() {
            out.push_str(&format!(" parent={}", r.parent));
        }
        out.push('\n');
        for chunk in r.members.chunks(10) {
            out.push_str(&format!("  {}\n", chunk.join(" ")));
        }
        out.push_str("end\n");
    }
    for a in &g.align {
        out.push_str(&format!("align {} {}\n", a.axis, a.ids.join(" ")));
    }
    for e in &g.edges {
        out.push_str(&edge_line(e, ""));
        out.push('\n');
    }
    for v in &g.vars_new {
        out.push_str(&format!("var {} = {}", v.id, quote(&v.latex)));
        if !v.meaning.is_empty() {
            out.push(' ');
            out.push_str(&quote(&v.meaning));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "fig t\nx: image \"$x$\"\ne1: box \"E1\"\ne2: box \"E2\"\nd1: box \"D1\"\n";

    fn with(extra: &str) -> Graph {
        parse(&format!("{BASE}{extra}")).unwrap()
    }
    fn err(extra: &str) -> String {
        parse(&format!("{BASE}{extra}")).unwrap_err().message
    }

    #[test]
    fn a_region_frames_its_members() {
        let g = with("region bb \"Backbone\" dashed tint=grey round title=above\n  e1 e2\n  d1\nend\n");
        let r = &g.regions[0];
        assert_eq!(r.id, "bb");
        assert_eq!(r.title, "Backbone");
        assert_eq!(r.line, "dashed");
        assert_eq!(r.tint, "grey");
        assert!(r.round);
        assert_eq!(r.title_at, "above");
        assert_eq!(r.members, ["e1", "e2", "d1"]);
        // the title is optional
        let g2 = parse("fig t\na: box \"A\"\nb: box \"B\"\nregion r dotted\n a b\nend\n").unwrap();
        assert_eq!(g2.regions[0].title, "");
        assert_eq!(g2.regions[0].line, "dotted");
    }

    #[test]
    fn a_region_inside_a_panel_takes_that_panel() {
        let g = parse("fig t\npanel p \"P\"\n  region r\n    a b\n  end\n  a: box \"A\"\n  b: box \"B\"\nend\n").unwrap();
        assert_eq!(g.node("a").unwrap().panel, "p");
        assert_eq!(g.regions[0].members, ["a", "b"]);
        // `in=` gives the panel to loose members
        let g = parse("fig t\npanel p\n  c: box \"C\"\nend\na: box \"A\"\nb: box \"B\"\nregion r in=p\n a b\nend\n").unwrap();
        assert_eq!(g.node("a").unwrap().panel, "p");
        assert_eq!(g.node("b").unwrap().panel, "p");
        // a panel the node already has is kept
        let g = parse("fig t\npanel p\n  a: box \"A\"\nend\npanel q\n  b: box \"B\"\nend\nregion r in=q\n b\nend\n").unwrap();
        assert_eq!(g.node("a").unwrap().panel, "p");
    }

    #[test]
    fn regions_nest_by_position_and_by_parent() {
        let g = with("region outer\n  region inner\n    e1\n  end\n  e2\nend\n");
        assert_eq!(g.regions[0].id, "outer");
        assert_eq!(g.regions[1].parent, "outer");
        assert_eq!(g.regions[1].members, ["e1"]);
        assert_eq!(g.regions[0].members, ["e2"]);
        let h = with("region outer\n e2\nend\nregion inner parent=outer\n e1\nend\n");
        assert_eq!(h.regions[1].parent, "outer");
    }

    #[test]
    fn a_node_declared_in_a_region_is_a_member() {
        let g = parse("fig t\nregion r\n  a: box \"A\"\n  b: op \"+\"\nend\n").unwrap();
        assert_eq!(g.regions[0].members, ["a", "b"]);
        assert_eq!(g.nodes.len(), 2);
    }

    #[test]
    fn a_region_survives_text_and_back() {
        let g = with("n: box \"Conv\" fill=blue line=1F4E79\npanel p \"P\"\n  q: box \"Q\"\nend\nregion outer \"Out\" dashed tint=1F4E79 round title=top-left\n  e1 e2\nend\nregion inner \"In\" parent=outer\n  e1\nend\nregion empty none\nend\nalign row e1 d1\nalign col e1 e2 d1\nx -> e1 : data color=orange\ne1 -> e2 : skip \"s\" color=1F4E79 curve\nx>e1 -> d1 : optional\n");
        let text = to_dsl(&g);
        assert_eq!(parse(&text).unwrap(), g);
        assert_eq!(to_dsl(&parse(&text).unwrap()), text);
    }

    #[test]
    fn align_row_and_col_are_read_and_written_back() {
        let g = with("align row e1 d1\nalign col e1 e2 d1\n");
        assert_eq!(g.align[0], Align { axis: "row".into(), ids: vec!["e1".into(), "d1".into()] });
        assert_eq!(g.align[1].axis, "col");
        assert_eq!(g.align[1].ids.len(), 3);
        let t = to_dsl(&g);
        assert!(t.contains("align row e1 d1\nalign col e1 e2 d1\n"));
        assert_eq!(parse(&t).unwrap(), g);
    }

    #[test]
    fn a_line_takes_its_own_colour_and_curve() {
        let g = with("x -> e1 : data color=orange\ne1 -> e2 \"lab\" curve\ne2 -> d1 : skip color=1F4E79\n");
        assert_eq!(g.edges[0].color, "orange");
        assert!(!g.edges[0].curve);
        assert!(g.edges[1].curve);
        assert_eq!(g.edges[1].label, "lab");
        assert_eq!(g.edges[2].kind, EdgeKind::Skip);
        assert_eq!(g.edges[2].color, "1F4E79");
    }

    #[test]
    fn the_extras_of_a_chain_apply_to_each_line() {
        let g = with("x -> e1 -> e2 : skip curve color=red\n");
        assert_eq!(g.edges.len(), 2);
        for e in &g.edges {
            assert!(e.curve);
            assert_eq!(e.color, "red");
            assert_eq!(e.kind, EdgeKind::Skip);
        }
    }

    #[test]
    fn a_line_may_leave_another_line() {
        let g = with("x -> e1\nx>e1 -> d1 : optional\nx -> e1\nx>e1#2 -> e2\n");
        assert_eq!(g.edges[1].off, "x>e1");
        assert_eq!(g.edges[1].from, "x");
        assert_eq!(g.edges[1].to, "d1");
        assert_eq!(g.edges[3].off, "x>e1#2");
        assert_eq!(g.edges[3].from, "x");
    }

    #[test]
    fn edge_extras_survive_text_and_back() {
        let g = with("x -> e1 : data color=orange curve\nx>e1 -> d1 : optional \"or\" color=blue\n");
        let t = to_dsl(&g);
        assert!(t.contains("x -> e1 color=orange curve\n"));
        assert!(t.contains("x>e1 -> d1 : optional \"or\" color=blue\n"));
        assert_eq!(parse(&t).unwrap(), g);
    }

    #[test]
    fn a_graph_without_the_new_sentences_prints_as_before() {
        let a = "fig demo\narrange a / b\npanel a \"a. Training\" dir=right\n  x:    image \"$x$\"\n  seg:  unet \"$\\theta$\" depth=3\npanel b\n  loss: loss \"L\" role=loss\nend\nx -> seg\nseg -> loss : feedback \"update\"\nvar w = \"\\omega\" \"weight\"\n";
        let ta = to_dsl(&parse(a).unwrap());
        assert_eq!(
            ta,
            "fig demo\narrange a / b\npanel a \"a. Training\" dir=right\n  x: image \"$x$\"\n  seg: unet \"$\\theta$\" depth=3\npanel b\n  loss: loss \"L\" role=loss\nend\nx -> seg\nseg -> loss : feedback \"update\"\nvar w = \"\\omega\" \"weight\"\n"
        );
        let b = "fig e\nx: image \"$x$\"\nf: box \"Model\"\nx -> f\n";
        assert_eq!(to_dsl(&parse(b).unwrap()), b);
    }

    #[test]
    fn colours_survive_the_round_trip() {
        let g = with("n: box \"C\" fill=blue line=blue text=1F4E79\nm: box \"M\" fill=1F4E79\n");
        assert_eq!(g.node("n").unwrap().str("fill"), Some("blue"));
        assert_eq!(g.node("m").unwrap().str("fill"), Some("1F4E79"));
        assert_eq!(parse(&to_dsl(&g)).unwrap(), g);
    }

    #[test]
    fn validation_refuses_bad_regions_through_the_text() {
        assert!(err("region e1\nend\n").contains("also the id"));
        assert!(err("region r\n zz\nend\n").contains("not a node"));
    }
}
