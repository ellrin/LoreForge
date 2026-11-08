//! Auto-extract: a draft registry from an existing paper.
//!
//! Heuristics only (definitions found by [`crate::symbols`], labelled
//! equations, floats, abbreviations). Meanings the text does not give can be
//! filled by an assistant ([`NotationAssistant`]); the real one lives in the
//! app and calls `ym-llm`, tests use [`MockAssistant`]. This crate never
//! talks to a model itself.

use crate::abbrev;
use crate::math::{self, Atom};
use crate::model::{Entry, Kind, Notation};
use crate::scan::{Area, Scan};
use crate::symbols::{self, DefKind, Symbols};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Draft {
    pub notation: Notation,
    /// Entry id → up to three sentences around its first uses (for an assistant / review UI).
    pub contexts: BTreeMap<String, Vec<String>>,
}

/// A readable id: `\tilde{\boldsymbol{p}}^{\mathrm{f}}` → `p-bold-tilde-f`.
pub fn symbol_id(s: &math::Sym) -> String {
    let mut parts: Vec<String> = Vec::new();
    let base = s.base.trim_start_matches('\\');
    parts.push(if base.chars().count() == 1 && base.chars().all(|c| c.is_ascii_uppercase()) { format!("{}-up", base.to_lowercase()) } else { base.to_string() });
    match s.font {
        math::Font::Plain => {}
        f => parts.push(format!("{f:?}").to_lowercase()),
    }
    for a in &s.accents {
        parts.push(a.to_string());
    }
    for n in [s.sub_name(), s.sup_name()].into_iter().flatten() {
        let t: String = n.replace("\\mathrm", "").replace("\\boldsymbol", "").chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if !t.is_empty() {
            parts.push(t);
        }
    }
    parts.join("-")
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// `\mathbb{R}^{a\times b}` → ("a×b"), the shape part of a domain.
fn shape_of(domain: &str) -> Option<String> {
    let rest = domain.strip_prefix("\\mathbb{R}")?;
    let inner = rest.strip_prefix("^{").and_then(|r| r.strip_suffix('}')).or(rest.strip_prefix('^'))?;
    Some(inner.replace("\\times", "×"))
}

/// Build a draft registry from a scanned paper.
pub fn draft(scan: &Scan, syms: &Symbols) -> Draft {
    let mut n = Notation::default();
    let mut contexts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let ctx_of = |offs: &[usize]| -> Vec<String> {
        let mut seen = Vec::new();
        for o in offs {
            if let Some(s) = scan.sentence_at(*o)
                && !seen.contains(&s) {
                    seen.push(s);
                }
            if seen.len() >= 3 {
                break;
            }
        }
        seen.into_iter().map(|s| scan.sentence_text(s)).collect()
    };
    // symbols, in order of first appearance
    let mut infos: Vec<&symbols::SymbolInfo> = syms.table.values().collect();
    infos.sort_by_key(|i| i.first_use().map(|o| o.off).unwrap_or(usize::MAX));
    let mut key_to_id: BTreeMap<String, String> = BTreeMap::new();
    for info in infos {
        let real_def = info.defs.iter().any(|d| !matches!(d.kind, DefKind::Inherited | DefKind::Derived));
        if !real_def && info.occs.len() < 2 {
            continue;
        }
        let id = n.free_id(&symbol_id(&info.sample));
        let mut e = Entry::new(&id, Kind::Symbol);
        e.latex = Some(info.key.clone());
        e.meaning_en = info.meaning();
        if let Some(d) = info.first_def() {
            let l = scan.loc(d.off, "definition");
            e.defined_at = Some(format!("{}:{}", l.file, l.line));
        }
        if let Some(d) = info.defs.iter().find(|d| d.kind == DefKind::Member) {
            e.domain = d.rhs.clone();
            e.shape = d.rhs.as_deref().and_then(shape_of);
        }
        e.tags.push("auto".into());
        if info.defs.is_empty() {
            e.tags.push("undefined".into());
        }
        let offs: Vec<usize> = info.defs.iter().map(|d| d.off).chain(info.occs.iter().map(|o| o.off)).collect();
        contexts.insert(id.clone(), ctx_of(&offs));
        key_to_id.insert(info.key.clone(), id);
        n.entries.push(e);
    }
    // custom operators
    for (name, offs) in &syms.operators {
        let id = n.free_id(&format!("op-{}", slug(name)));
        let mut e = Entry::new(&id, Kind::Operator);
        e.latex = Some(format!("\\operatorname{{{name}}}"));
        e.tags.push("auto".into());
        contexts.insert(id.clone(), ctx_of(offs));
        n.entries.push(e);
    }
    // labelled display equations
    for seg in scan.math.iter().filter(|m| m.display && !m.labels.is_empty()) {
        let label = seg.labels[0].clone();
        let id = n.free_id(&slug(&label));
        let mut e = Entry::new(&id, Kind::Equation);
        let body = strip_labels(&seg.src);
        e.latex = Some(body);
        e.label = Some(label);
        let mut v = Vec::new();
        math::collect_syms(&seg.atoms, &mut v);
        let mut uses: Vec<String> = Vec::new();
        for s in v.iter().flat_map(|s| std::iter::once(s.clone()).chain(s.qualifiers.clone()).chain(s.script_uses.clone())) {
            if let Some(i) = key_to_id.get(&s.key())
                && !uses.contains(i) {
                    uses.push(i.clone());
                }
        }
        e.uses = uses;
        let lhs: Vec<String> = symbols::row_lhs(&first_row(&seg.atoms)).into_iter().map(|l| l.sym.key()).collect();
        if let Some(k) = lhs.first() {
            e.meaning_en = syms.table.get(k).and_then(|i| i.meaning());
        }
        let l = scan.loc(seg.start, "equation");
        e.defined_at = Some(format!("{}:{}", l.file, l.line));
        e.tags.push("auto".into());
        contexts.insert(id.clone(), ctx_of(&[seg.start]));
        n.entries.push(e);
    }
    // figures and tables
    for f in &scan.floats {
        let Some(label) = f.labels.first() else { continue };
        let kind = if f.kind == "table" { Kind::Table } else { Kind::Figure };
        let id = n.free_id(&slug(label));
        let mut e = Entry::new(&id, kind);
        e.label = Some(label.clone());
        e.file = f.graphics.first().cloned();
        if let Some(c) = f.caption {
            let c = &scan.captions[c];
            let raw = scan.doc.text[c.start..c.end].split_whitespace().collect::<Vec<_>>().join(" ");
            e.caption = Some(raw);
        }
        e.tags.push("auto".into());
        n.entries.push(e);
    }
    // abbreviations
    for (short, a) in abbrev::analyze(scan) {
        let id = n.free_id(&format!("abbr-{}", slug(&short)));
        let mut e = Entry::new(&id, Kind::Abbrev);
        e.short = Some(short.clone());
        let body_exp = a.expansions.iter().find(|x| x.area != Area::Abstract).or(a.expansions.first());
        e.long = body_exp.map(|x| x.long.clone());
        e.tags.push("auto".into());
        if e.long.is_none() {
            e.tags.push("needs-expansion".into());
        }
        let offs: Vec<usize> = a.uses.iter().map(|u| u.off).collect();
        contexts.insert(id.clone(), ctx_of(&offs));
        n.entries.push(e);
    }
    Draft { notation: n, contexts }
}

fn first_row(atoms: &[Atom]) -> Vec<Atom> {
    let mut out = Vec::new();
    for a in atoms {
        match a {
            Atom::Row => break,
            Atom::Group(g) if g.iter().any(|x| matches!(x, Atom::Row)) => return first_row(g),
            _ => out.push(a.clone()),
        }
    }
    out
}

fn strip_labels(src: &str) -> String {
    let mut s = src.to_string();
    while let Some(i) = s.find("\\label{") {
        let end = s[i..].find('}').map(|e| i + e + 1).unwrap_or(s.len());
        s.replace_range(i..end, "");
    }
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

// ── assistant hook ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistItem {
    pub id: String,
    pub kind: Kind,
    /// LaTeX or the abbreviation.
    pub text: String,
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistRequest {
    /// Language for `meaning` ("zh-TW").
    pub lang: String,
    pub items: Vec<AssistItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    pub id: String,
    #[serde(default)]
    pub meaning: Option<String>,
    #[serde(default)]
    pub meaning_en: Option<String>,
    #[serde(default)]
    pub gloss: Option<String>,
    /// Long form for abbreviations.
    #[serde(default)]
    pub long: Option<String>,
}

/// Fills meanings the heuristics could not find. Implemented by apps on top
/// of `ym-llm` (one structured-output task); synchronous so this crate stays
/// runtime-free — an async app wraps its call in `block_on` or runs it on a
/// worker thread.
pub trait NotationAssistant {
    fn suggest(&self, req: &AssistRequest) -> Result<Vec<Suggestion>, String>;
}

/// Canned answers by id (tests, demos).
#[derive(Debug, Clone, Default)]
pub struct MockAssistant {
    pub answers: BTreeMap<String, Suggestion>,
    /// Requests received (for assertions).
    pub seen: std::cell::RefCell<Vec<AssistRequest>>,
}

impl NotationAssistant for MockAssistant {
    fn suggest(&self, req: &AssistRequest) -> Result<Vec<Suggestion>, String> {
        self.seen.borrow_mut().push(req.clone());
        Ok(req.items.iter().filter_map(|i| self.answers.get(&i.id).cloned()).collect())
    }
}

/// The request for entries that still lack a meaning (or a long form).
pub fn assist_request(d: &Draft, lang: &str) -> AssistRequest {
    let items = d
        .notation
        .entries
        .iter()
        .filter(|e| match e.kind() {
            Kind::Abbrev => e.long.is_none(),
            Kind::Figure | Kind::Table => false,
            _ => e.meaning.is_none(),
        })
        .map(|e| AssistItem {
            id: e.id.clone(),
            kind: e.kind(),
            text: e.latex.clone().or(e.short.clone()).unwrap_or_default(),
            contexts: d.contexts.get(&e.id).cloned().unwrap_or_default(),
        })
        .collect();
    AssistRequest { lang: lang.into(), items }
}

/// Ask the assistant and merge its answers into empty fields only. Returns
/// how many entries changed. Suggestions are tagged `suggested` for review.
pub fn refine(d: &mut Draft, a: &dyn NotationAssistant, lang: &str) -> Result<usize, String> {
    let req = assist_request(d, lang);
    if req.items.is_empty() {
        return Ok(0);
    }
    let mut changed = 0;
    for s in a.suggest(&req)? {
        let Some(e) = d.notation.get_mut(&s.id) else { continue };
        let mut touched = false;
        for (field, val) in [(&mut e.meaning, s.meaning), (&mut e.meaning_en, s.meaning_en), (&mut e.gloss, s.gloss), (&mut e.long, s.long)] {
            if field.is_none() && val.as_deref().is_some_and(|v| !v.trim().is_empty()) {
                *field = val;
                touched = true;
            }
        }
        if touched {
            if !e.has_tag("suggested") {
                e.tags.push("suggested".into());
            }
            e.tags.retain(|t| !(t == "needs-expansion" && e.long.is_some()));
            changed += 1;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAPER: &str = "\\begin{document}\n\\begin{abstract}\nWidgets.\n\\end{abstract}\n\\section{Method}\nA widget network (WN) maps $\\boldsymbol{u}\\in\\mathbb{R}^{3\\times 4}$ to a score. Let $s$ denote the score. The WN uses a CNN.\n\\begin{equation}\n\\label{eq:score}\ns = \\operatorname{mean}(\\boldsymbol{u}) + \\gamma.\n\\end{equation}\nHere, $\\gamma$ is a bias.\n\\begin{figure}\n\\includegraphics{w.png}\n\\caption{The widget.}\n\\label{fig:w}\n\\end{figure}\nSee Fig.~\\ref{fig:w} and \\eqref{eq:score}.\n\\end{document}\n";

    #[test]
    fn drafts_entries_of_every_kind() {
        let scan = Scan::from_str("main.tex", PAPER);
        let syms = symbols::analyze(&scan, &Default::default());
        let d = draft(&scan, &syms);
        let n = &d.notation;
        let u = n.find_latex("\\boldsymbol{u}").expect("u");
        assert_eq!(u.id, "u-bold");
        assert_eq!(u.shape.as_deref(), Some("3×4"));
        assert_eq!(n.find_latex("s").unwrap().meaning_en.as_deref(), Some("score"));
        assert_eq!(n.find_latex("\\gamma").unwrap().meaning_en.as_deref(), Some("bias"));
        assert!(n.entries.iter().any(|e| e.kind() == Kind::Operator && e.latex.as_deref() == Some("\\operatorname{mean}")));
        let eq = n.entries.iter().find(|e| e.kind() == Kind::Equation).unwrap();
        assert_eq!(eq.label.as_deref(), Some("eq:score"));
        assert!(eq.uses.contains(&"gamma".to_string()));
        let fig = n.entries.iter().find(|e| e.kind() == Kind::Figure).unwrap();
        assert_eq!((fig.file.as_deref(), fig.caption.as_deref()), (Some("w.png"), Some("The widget.")));
        let wn = n.entries.iter().find(|e| e.short.as_deref() == Some("WN")).unwrap();
        assert_eq!(wn.long.as_deref(), Some("widget network"));
        let cnn = n.entries.iter().find(|e| e.short.as_deref() == Some("CNN")).unwrap();
        assert!(cnn.has_tag("needs-expansion"));
        assert!(!d.contexts[&cnn.id].is_empty());
    }

    #[test]
    fn assistant_fills_only_empty_fields() {
        let scan = Scan::from_str("main.tex", PAPER);
        let syms = symbols::analyze(&scan, &Default::default());
        let mut d = draft(&scan, &syms);
        let cnn = d.notation.entries.iter().find(|e| e.short.as_deref() == Some("CNN")).unwrap().id.clone();
        let mut mock = MockAssistant::default();
        mock.answers.insert(cnn.clone(), Suggestion { id: cnn.clone(), long: Some("convolutional neural network".into()), meaning: Some("卷積神經網路".into()), ..Default::default() });
        mock.answers.insert("s".into(), Suggestion { id: "s".into(), meaning_en: Some("overwritten?".into()), meaning: Some("分數".into()), ..Default::default() });
        let changed = refine(&mut d, &mock, "zh-TW").unwrap();
        assert_eq!(changed, 2);
        let e = d.notation.get(&cnn).unwrap();
        assert_eq!(e.long.as_deref(), Some("convolutional neural network"));
        assert!(e.has_tag("suggested") && !e.has_tag("needs-expansion"));
        let s = d.notation.get("s").unwrap();
        assert_eq!(s.meaning_en.as_deref(), Some("score"));
        assert_eq!(s.meaning.as_deref(), Some("分數"));
        assert!(mock.seen.borrow()[0].items.iter().any(|i| i.id == cnn));
    }
}
