//! Keeping a figure's variables in line with the notation registry
//! (`lore-notation`): new variables are reconciled with the registry, the
//! text of the figure goes through the registry's checks, and a few
//! conventions of figures are linted.
//!
//! New variables are only ever written to the *document's* notation
//! ([`register_new`], tag `flow-draft`), never to the global registry.

use lore_notation::{CheckOptions, Document, Entry, Kind, Notation};
use serde::Serialize;

use crate::spec::*;
use crate::text::{self, Vars};

/// Tag of entries made from a figure's `vars_new`.
pub const DRAFT_TAG: &str = "flow-draft";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum VarStatus {
    /// Not in the registry: will be added to the document's notation.
    New,
    /// The registry already has this LaTeX under `id`; references were moved to it.
    Existing { id: String, same_meaning: bool },
    /// The id was taken by another symbol; the variable was renamed.
    Renamed { from: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VarDecision {
    pub id: String,
    pub latex: String,
    pub meaning: String,
    #[serde(flatten)]
    pub status: VarStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// unknown_ref | use_registered | loss_style | hardcoded_reference | a lore-notation code
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct NotationReport {
    pub vars: Vec<VarDecision>,
    pub findings: Vec<Finding>,
}

impl NotationReport {
    pub fn by_code(&self, code: &str) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.code == code).collect()
    }
}

fn rename_refs(g: &mut Graph, map: &std::collections::BTreeMap<String, String>) {
    if map.is_empty() {
        return;
    }
    let swap = |s: &mut String| {
        if !s.contains('@') {
            return;
        }
        let cs: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < cs.len() {
            if cs[i] == '@' {
                let mut j = i + 1;
                while j < cs.len() && (cs[j].is_ascii_alphanumeric() || "_-.:".contains(cs[j])) {
                    j += 1;
                }
                while j > i + 1 && ".:-".contains(cs[j - 1]) {
                    j -= 1;
                }
                let id: String = cs[i + 1..j].iter().collect();
                out.push('@');
                out.push_str(map.get(&id).unwrap_or(&id));
                i = j;
            } else {
                out.push(cs[i]);
                i += 1;
            }
        }
        *s = out;
    };
    for n in &mut g.nodes {
        swap(&mut n.label);
        swap(&mut n.caption);
        if let Some(to) = map.get(&n.var) {
            n.var = to.clone();
        }
    }
    for e in &mut g.edges {
        swap(&mut e.label);
    }
    for p in &mut g.panels {
        swap(&mut p.title);
    }
}

/// Every piece of text a figure shows, with the node it belongs to.
fn texts(g: &Graph) -> Vec<(Option<&str>, &str)> {
    let mut v = Vec::new();
    for p in &g.panels {
        v.push((None, p.title.as_str()));
    }
    for n in &g.nodes {
        v.push((Some(n.id.as_str()), n.label.as_str()));
        v.push((Some(n.id.as_str()), n.caption.as_str()));
        for l in n.params.values().filter_map(|p| p.as_array()).flatten().filter_map(|x| x.as_str()) {
            v.push((Some(n.id.as_str()), l));
        }
    }
    for e in &g.edges {
        v.push((None, e.label.as_str()));
    }
    v.retain(|(_, s)| !s.is_empty());
    v
}

fn typed_reference(s: &str) -> Option<String> {
    // "Table 1", "Fig. 2", "Eq. (3)" typed by hand
    let words: Vec<&str> = s.split_whitespace().collect();
    for w in words.windows(2) {
        let head = w[0].trim_end_matches('.');
        if matches!(head, "Table" | "Tab" | "Figure" | "Fig" | "Eq" | "Equation" | "Section" | "Sec") && w[1].trim_matches(|c: char| "().,:".contains(c)).chars().all(|c| c.is_ascii_digit()) && w[1].chars().any(|c| c.is_ascii_digit()) {
            return Some(format!("{} {}", w[0], w[1]));
        }
    }
    None
}

/// Reconcile `vars_new` with the registry (rewriting the graph where an id
/// moves) and check the figure's text. `notation` = the registry in force
/// (global merged with the document's).
pub fn check_graph(g: &mut Graph, notation: &Notation) -> NotationReport {
    let mut rep = NotationReport::default();

    // 1. new variables against the registry
    let mut kept: Vec<VarNew> = Vec::new();
    let mut scratch = notation.clone();
    // all ids move at once: a reference renamed to an existing id is not caught by a later rename of that id
    let mut moves: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for v in std::mem::take(&mut g.vars_new) {
        if let Some(e) = notation.find_latex(&v.latex) {
            let known = [e.meaning.as_deref(), e.meaning_en.as_deref()].into_iter().flatten().map(|m| m.trim().to_lowercase()).collect::<Vec<_>>();
            let same = v.meaning.trim().is_empty() || known.is_empty() || known.contains(&v.meaning.trim().to_lowercase());
            if e.id != v.id {
                moves.insert(v.id.clone(), e.id.clone());
            }
            rep.vars.push(VarDecision { id: v.id.clone(), latex: v.latex.clone(), meaning: v.meaning.clone(), status: VarStatus::Existing { id: e.id.clone(), same_meaning: same } });
            continue;
        }
        if kept.iter().any(|k| k.id == v.id) {
            continue;
        }
        let mut v = v;
        let free = scratch.free_id(&v.id);
        let status = if free != v.id {
            let from = std::mem::replace(&mut v.id, free.clone());
            moves.insert(from.clone(), free.clone());
            VarStatus::Renamed { from }
        } else {
            VarStatus::New
        };
        let mut e = Entry::new(&v.id, Kind::Symbol);
        e.latex = Some(v.latex.clone());
        scratch.upsert(e);
        rep.vars.push(VarDecision { id: v.id.clone(), latex: v.latex.clone(), meaning: v.meaning.clone(), status });
        kept.push(v);
    }
    rename_refs(g, &moves);
    g.vars_new = kept;

    // 2. references and conventions
    let mut vars = Vars::from_notation(notation);
    vars.add_new(&g.vars_new);
    let mut add = |code: &str, message: String, node: Option<&str>| {
        let f = Finding { code: code.into(), message, node: node.map(str::to_string) };
        if !rep.findings.contains(&f) {
            rep.findings.push(f);
        }
    };
    for n in &g.nodes {
        if !n.var.is_empty() && !vars.ids.contains_key(&n.var) {
            add("unknown_ref", format!("variable {:?} is neither in the notation table nor in vars_new", n.var), Some(&n.id));
        }
        if n.module == "loss" {
            let math = text::math_of(&vars.resolve(&n.label));
            if !math.is_empty() && !math.iter().any(|m| m.contains("\\mathcal")) {
                add("loss_style", format!("loss {:?} is not written as \\mathcal L", n.label), Some(&n.id));
            }
        }
    }
    for (node, s) in texts(g) {
        for id in vars.unknown_refs(s) {
            add("unknown_ref", format!("@{id} is neither in the notation table nor in vars_new"), node);
        }
        if let Some(t) = typed_reference(&text::plain(s)) {
            add("hardcoded_reference", format!("{t:?} is typed by hand; refer through the notation table (\\ref) instead"), node);
        }
        for m in text::math_of(s) {
            if let Some(e) = notation.find_latex(&m) {
                add("use_registered", format!("${m}$ is registered as {:?}; write @{}", e.id, e.id), node);
            }
        }
    }

    // 3. the registry's own checks on the text of the figure
    let mut effective = notation.clone();
    register_new(&mut effective, &g.vars_new);
    let mut doc = String::new();
    for (_, s) in texts(g) {
        doc.push_str(&vars.resolve(s).replace('\n', " "));
        doc.push_str(".\n\n");
    }
    let report = lore_notation::check_document(Document::from_str("figure.tex", &doc), &effective, &CheckOptions::default());
    // (typed references are reported above, with the node they are on)
    for i in &report.issues {
        if matches!(i.code.as_str(), "symbol_multiple_meanings" | "meaning_multiple_symbols" | "registry_invalid" | "spelling_variants") {
            add(&i.code, i.message.clone(), None);
        }
    }
    rep
}

/// Add a figure's new variables to the *document's* notation, tagged
/// [`DRAFT_TAG`]. Existing ids are left alone. Returns the ids added.
pub fn register_new(doc_notation: &mut Notation, vars: &[VarNew]) -> Vec<String> {
    let mut added = Vec::new();
    for v in vars {
        if doc_notation.get(&v.id).is_some() || doc_notation.find_latex(&v.latex).is_some() {
            continue;
        }
        let mut e = Entry::new(&v.id, Kind::Symbol);
        e.latex = Some(v.latex.clone());
        if !v.meaning.is_empty() {
            e.meaning = Some(v.meaning.clone());
        }
        e.tags = vec![DRAFT_TAG.into()];
        doc_notation.upsert(e);
        added.push(v.id.clone());
    }
    added
}
