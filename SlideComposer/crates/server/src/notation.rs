//! The notation registry (對照表, lore-notation) of a deck: symbols, equations,
//! abbreviations. The deck folder's `notation.json` is the document file; the
//! author's `private/shared/notation.json`, when present, is the global one.
//!
//! Slides become a small LaTeX document for the checks and the auto-extract:
//! one file per slide (`slide-03`), one line per paragraph, math as `$…$`
//! (a paragraph that is only math as `\[…\]`). `lines` maps every line back to
//! its slide and object so a finding can jump there.

use std::collections::BTreeMap;

use lore_canvas::{Deck, Kind, Object, Para};
use lore_notation::{CheckOptions, Document, Notation, Origin, check_document, extract_document};
use serde_json::{Value, json};

/// (slide uid, object id) of every line of a slide file.
pub type Lines = BTreeMap<String, Vec<(String, u32)>>;

pub fn slide_file(n: usize) -> String {
    format!("slide-{n:02}")
}

fn para_tex(p: &Para, latex: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut text_seen = false;
    let mut math: Vec<String> = Vec::new();
    for r in &p.runs {
        match r.kind.as_str() {
            "math" => {
                let l = r.latex.clone().or_else(|| r.omml.as_ref().and_then(|o| latex.get(&format!("{:016x}", lore_canvas::fnv(o.as_bytes()))).cloned()));
                if let Some(l) = l {
                    out += &format!("${l}$");
                    math.push(l);
                }
            }
            "br" => out.push(' '),
            _ => {
                if !r.text.trim().is_empty() {
                    text_seen = true;
                }
                out += &r.text.replace(['\n', '\r'], " ");
            }
        }
    }
    if !text_seen && math.len() == 1 {
        return format!("\\[{}\\]", math[0]);
    }
    out
}

fn object_lines(o: &Object, latex: &BTreeMap<String, String>, uid: &str, text: &mut String, map: &mut Vec<(String, u32)>) {
    let mut bodies = Vec::new();
    match &o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => bodies.extend(sp.text.iter()),
        Kind::Table(t) => {
            for r in &t.rows {
                for c in &r.cells {
                    bodies.push(&c.text);
                }
            }
        }
        Kind::Group(g) => {
            for c in &g.children {
                object_lines(c, latex, uid, text, map);
            }
        }
        _ => {}
    }
    for tb in bodies {
        for p in &tb.paras {
            let line = para_tex(p, latex);
            if line.trim().is_empty() {
                continue;
            }
            text.push_str(&line);
            text.push('\n');
            map.push((uid.to_string(), o.id));
        }
    }
}

/// The deck as LaTeX files (one per slide, hidden slides included) and the line map.
pub fn deck_document(deck: &Deck, latex: &BTreeMap<String, String>) -> (Vec<(String, String)>, Lines) {
    let mut files = Vec::new();
    let mut lines = Lines::new();
    for (i, s) in deck.slides.iter().enumerate() {
        let name = slide_file(i + 1);
        let mut text = String::new();
        let mut map = Vec::new();
        for o in &s.objects {
            object_lines(o, latex, &s.uid, &mut text, &mut map);
        }
        files.push((name.clone(), text));
        lines.insert(name, map);
    }
    (files, lines)
}

/// `\input` every slide file from a root so the checker reads them in order.
fn document(files: &[(String, String)]) -> Document {
    let mut all = vec![("main.tex".to_string(), files.iter().map(|(n, _)| format!("\\input{{{n}}}\n")).collect::<String>())];
    all.extend(files.iter().map(|(n, t)| (format!("{n}.tex"), t.clone())));
    Document::from_files(&all)
}

pub fn load(dir: &std::path::Path, global: Option<&std::path::Path>) -> (Notation, Notation) {
    let doc = Notation::load(&dir.join("notation.json")).unwrap_or_default();
    let glob = global.and_then(|g| Notation::load(g).ok()).unwrap_or_default();
    (glob, doc)
}

/// What the panel shows: the merged registry (entries carry `origin`), the
/// check report on the slides, and the line map for jumps.
pub fn view(deck: &Deck, latex: &BTreeMap<String, String>, global: &Notation, doc: &Notation) -> Value {
    let (files, lines) = deck_document(deck, latex);
    let eff = Notation::effective(global, doc);
    let report = check_document(document(&files), &eff, &CheckOptions { caption_defines: true, ..Default::default() });
    let mut n = serde_json::to_value(&eff).unwrap_or(json!({}));
    let merged = Notation::merged(global, doc);
    n["entries"] = Value::Array(
        merged
            .into_iter()
            .map(|m| {
                let mut e = serde_json::to_value(&m.entry).unwrap_or(json!({}));
                e["origin"] = json!(if m.origin == Origin::Global { "global" } else { "document" });
                e
            })
            .collect(),
    );
    let lines: BTreeMap<String, Vec<Value>> = lines
        .into_iter()
        .map(|(k, v)| (format!("{k}.tex"), v.into_iter().map(|(u, id)| json!([u, id])).collect()))
        .collect();
    json!({ "notation": n, "report": report, "lines": lines })
}

/// The document file from what the panel sends: global rows the author did not
/// touch stay in the global file; `origin` is dropped.
pub fn from_panel(v: &Value) -> Result<Notation, String> {
    let mut v = v.clone();
    if let Some(es) = v.get_mut("entries").and_then(Value::as_array_mut) {
        es.retain(|e| e.get("origin").and_then(Value::as_str) != Some("global"));
        for e in es.iter_mut() {
            if let Some(o) = e.as_object_mut() {
                o.remove("origin");
            }
        }
    }
    Notation::from_json(&v.to_string()).map_err(|e| e.to_string())
}

/// Auto-extract from the slides; new entries (by id) join the document file, tagged `auto`.
pub fn extract_into(deck: &Deck, latex: &BTreeMap<String, String>, doc: &mut Notation) -> usize {
    let (files, _) = deck_document(deck, latex);
    let draft = extract_document(document(&files));
    let mut added = 0;
    for e in draft.notation.entries {
        if doc.get(&e.id).is_none() && doc.find_latex(e.latex.as_deref().unwrap_or("\u{0}")).is_none() {
            doc.entries.push(e);
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> Deck {
        serde_json::from_value(json!({
            "format": 1, "size": { "w": 9144000, "h": 5143500 }, "style": null, "next_uid": 3,
            "slides": [
                { "uid": "s1", "layout": null, "objects": [
                    { "id": 2, "name": "body", "frame": { "x": 0, "y": 0, "w": 100, "h": 100 }, "type": "text", "geom": { "prst": "rect" }, "fill": { "kind": "none" },
                      "text": { "paras": [
                        { "runs": [ { "kind": "text", "text": "The loss ", "look": { "font": "Arial", "size": 14.0, "color": "000000" } },
                                    { "kind": "math", "text": "", "latex": "\\mathcal{L}", "look": { "font": "Cambria Math", "size": 14.0, "color": "000000" } },
                                    { "kind": "text", "text": " is the mean squared error (MSE).", "look": { "font": "Arial", "size": 14.0, "color": "000000" } } ] },
                        { "runs": [ { "kind": "math", "text": "", "latex": "\\mathcal{L} = \\frac{1}{n}\\sum_i (y_i - \\hat{y}_i)^2", "look": { "font": "Cambria Math", "size": 14.0, "color": "000000" } } ] } ] } } ] },
                { "uid": "s2", "layout": null, "objects": [] }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn slides_become_latex_lines_that_map_back() {
        let (files, lines) = deck_document(&deck(), &BTreeMap::new());
        assert_eq!(files[0].0, "slide-01");
        assert_eq!(files[0].1, "The loss $\\mathcal{L}$ is the mean squared error (MSE).\n\\[\\mathcal{L} = \\frac{1}{n}\\sum_i (y_i - \\hat{y}_i)^2\\]\n");
        assert_eq!(lines["slide-01"], [("s1".to_string(), 2), ("s1".to_string(), 2)]);
        assert!(files[1].1.is_empty());
    }

    #[test]
    fn extract_adds_once_and_global_rows_stay_global() {
        let mut doc = Notation::default();
        let added = extract_into(&deck(), &BTreeMap::new(), &mut doc);
        assert!(added > 0, "nothing extracted");
        assert_eq!(extract_into(&deck(), &BTreeMap::new(), &mut doc), 0, "second run adds nothing");
        let v = view(&deck(), &BTreeMap::new(), &Notation::default(), &doc);
        assert!(v["report"].is_object());
        assert_eq!(v["lines"]["slide-01.tex"][0], json!(["s1", 2]));

        let panel = json!({ "version": 1, "entries": [
            { "id": "w", "kind": "symbol", "latex": "w", "origin": "global" },
            { "id": "x", "kind": "symbol", "latex": "x", "meaning": "input", "origin": "document" } ] });
        let n = from_panel(&panel).unwrap();
        assert_eq!(n.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["x"]);
        assert!(!n.to_json().contains("origin"));
    }
}
