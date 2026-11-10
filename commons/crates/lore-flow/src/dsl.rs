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
//! `caption` are fields, every other `key=value` is a module parameter.

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
                    Tok::Quoted(q) if !expect_node => label = q.clone(),
                    t => return Err(err(format!("did not expect {:?} here", t.s()))),
                }
                i += 1;
            }
            if chain.len() < 2 || expect_node {
                return Err(err("an arrow needs a node on both sides".into()));
            }
            for w in chain.windows(2) {
                g.edges.push(Edge { from: w[0].clone(), to: w[1].clone(), kind, label: label.clone() });
            }
            continue;
        }
        if is_word && head.ends_with(':') && head.len() > 1 {
            // id: module "label" key=value …
            let id = head.trim_end_matches(':').to_string();
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
        match head.as_str() {
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
                        Tok::Word(w) => match w.split_once('=') {
                            Some(("dir", v)) if v == "right" || v == "down" => p.dir = v.to_string(),
                            Some(("title", v)) => p.title = v.trim_start_matches('"').to_string(),
                            _ => return Err(err(format!("panel takes a title, dir=right|down and bare (got {w:?})"))),
                        },
                    }
                }
                g.panels.push(p);
                panel = id;
            }
            "end" => panel.clear(),
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
                let latex = strings.remove(0);
                let meaning = strings.into_iter().next().unwrap_or_default();
                g.vars_new.push(VarNew { id, latex, meaning });
            }
            _ => return Err(err(format!("did not understand {head:?} (expected fig, template, arrange, legend, panel, end, var, `id: module \"label\"` or `a -> b`)"))),
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
        let mut s = format!("{indent}{} -> {}", e.from, e.to);
        if e.kind != EdgeKind::Data {
            s.push_str(&format!(" : {}", e.kind.as_str()));
        }
        if !e.label.is_empty() {
            s.push(' ');
            s.push_str(&quote(&e.label));
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
        out.push('\n');
        for n in g.nodes.iter().filter(|n| n.panel == p.id) {
            out.push_str(&node_line(n, "  "));
            out.push('\n');
        }
    }
    if !g.panels.is_empty() {
        out.push_str("end\n");
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
