//! The notation registry (對照表, lore-notation) of a deck: symbols, equations,
//! abbreviations. The deck folder's `notation.json` is the document file; the
//! author's `private/shared/notation.json`, when present, is the global one.
//!
//! Slides become a small LaTeX document for the checks and the auto-extract:
//! one file per slide (`slide-03`), one line per paragraph, math as `$…$`
//! (a paragraph that is only math as `\[…\]`). `lines` maps every line back to
//! its slide and object so a finding can jump there.

use std::collections::{BTreeMap, BTreeSet};

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

/// (slide file, symbol key) for every symbol a slide's speaker notes define ("$x$ denotes …"),
/// read as the same slide files with the notes as text, one paragraph a line.
pub fn notes_definitions(deck: &Deck, notation: &Notation) -> BTreeSet<(String, String)> {
    let files: Vec<(String, String)> = deck
        .slides
        .iter()
        .enumerate()
        .map(|(i, s)| {
            // a % would hide the rest of the line from the LaTeX reader
            let text: Vec<String> = s.notes.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.replace('%', "\\%")).collect();
            (slide_file(i + 1), text.join("\n\n"))
        })
        .collect();
    let mut out = BTreeSet::new();
    if files.iter().all(|(_, t)| t.is_empty()) {
        return out;
    }
    let opt = CheckOptions { caption_defines: true, ..Default::default() };
    let scan = lore_notation::scan_document(document(&files), notation, &opt);
    let syms = lore_notation::symbols::analyze(&scan, &lore_notation::symbols::SymbolOptions { caption_defines: true });
    for info in syms.table.values() {
        for d in &info.defs {
            let file = scan.loc(d.off, "body").file;
            out.insert((file.trim_end_matches(".tex").to_string(), info.key.clone()));
        }
    }
    out
}

/// Slide number of a slide file (`slide-07.tex` → 7).
fn slide_no(file: &str) -> Option<usize> {
    file.trim_end_matches(".tex").strip_prefix("slide-")?.parse().ok()
}

/// A definition in a slide's speaker notes counts as one on that slide, the way a definition
/// on a slide counts today: a use on that slide or a later one is defined, a use on an earlier
/// slide is used before its definition. So in `undefined_symbol` / `used_before_definition`
/// issues the uses from the first such slide on are no locations; an issue left with none is
/// dropped (only when its list was not cut at `max_locations`); an `undefined_symbol` left with
/// earlier uses becomes `used_before_definition`; the symbol reads defined.
fn apply_notes_definitions(report: &mut lore_notation::Report, defined: &BTreeSet<(String, String)>, max_locations: usize) {
    // key → (first slide whose notes define it, that slide's file)
    let mut first: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (file, key) in defined {
        if let Some(n) = slide_no(file) {
            let e = first.entry(key.clone()).or_insert((n, file.clone()));
            if n < e.0 {
                *e = (n, file.clone());
            }
        }
    }
    if first.is_empty() {
        return;
    }
    let mut now_defined: BTreeSet<String> = BTreeSet::new();
    report.issues.retain_mut(|i| {
        if !matches!(i.code.as_str(), "undefined_symbol" | "used_before_definition") || !i.subjects.is_empty() {
            return true;
        }
        let Some(key) = i.subject.clone() else { return true };
        let Some((at, at_file)) = first.get(&key).cloned() else { return true };
        let full = i.locations.len() < max_locations;
        let before = i.locations.len();
        i.locations.retain(|l| slide_no(&l.file).is_none_or(|n| n < at));
        if i.locations.len() == before {
            return true;
        }
        now_defined.insert(key.clone());
        if i.locations.is_empty() && full {
            return false;
        }
        if i.code == "undefined_symbol" && !i.locations.is_empty() {
            let l = &i.locations[0];
            i.code = "used_before_definition".into();
            i.message = format!("符號 ${key}$ 先用後定義：第一次用在 {}:{}，到 {at_file}.tex 的講者備註才定義 — 把定義移到第一次使用的地方", l.file, l.line);
        }
        true
    });
    for s in report.symbols.iter_mut().filter(|s| now_defined.contains(&s.key)) {
        s.defined = true;
    }
    report.summary.clear();
    for i in &report.issues {
        *report.summary.entry(i.code.clone()).or_insert(0) += 1;
    }
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
    let opt = CheckOptions { caption_defines: true, ..Default::default() };
    let mut report = check_document(document(&files), &eff, &opt);
    apply_notes_definitions(&mut report, &notes_definitions(deck, &eff), opt.max_locations);
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

    fn line_deck(slides: &[(&str, &str)]) -> Deck {
        let slides: Vec<Value> = slides
            .iter()
            .enumerate()
            .map(|(i, (math, notes))| json!({ "uid": format!("s{}", i + 1), "layout": null, "notes": notes, "objects": [
                { "id": 2, "name": "body", "frame": { "x": 0, "y": 0, "w": 100, "h": 100 }, "type": "text", "geom": { "prst": "rect" }, "fill": { "kind": "none" },
                  "text": { "paras": [ { "runs": [ { "kind": "math", "text": "", "latex": math, "look": { "font": "Cambria Math", "size": 14.0, "color": "000000" } } ] } ] } } ] }))
            .collect();
        serde_json::from_value(json!({ "format": 1, "size": { "w": 9144000, "h": 5143500 }, "style": null, "next_uid": 9, "slides": slides })).unwrap()
    }

    fn undefined(deck: &Deck, key: &str) -> Vec<String> {
        let v = view(deck, &BTreeMap::new(), &Notation::default(), &Notation::default());
        let issues = v["report"]["issues"].as_array().cloned().unwrap_or_default();
        issues.iter().filter(|i| i["code"] == "undefined_symbol" && i["subject"] == key).flat_map(|i| i["locations"].as_array().cloned().unwrap_or_default()).map(|l| l["file"].as_str().unwrap_or("").to_string()).collect()
    }

    #[test]
    fn a_symbol_the_notes_define_counts_like_one_on_the_slide() {
        // defined nowhere: undefined
        assert_eq!(undefined(&line_deck(&[("\\boldsymbol{u} + \\boldsymbol{q}", "")]), "\\boldsymbol{u}"), ["slide-01.tex"]);
        // defined in the slide's notes: defined, the issue is gone and the symbol reads defined
        let notes = "The input of the model.\n1. $\\boldsymbol{u}$ denotes the input image.";
        let one = line_deck(&[("\\boldsymbol{u} + \\boldsymbol{q}", notes)]);
        assert!(undefined(&one, "\\boldsymbol{u}").is_empty());
        let v = view(&one, &BTreeMap::new(), &Notation::default(), &Notation::default());
        assert!(v["report"]["symbols"].as_array().unwrap().iter().any(|s| s["key"] == "\\boldsymbol{u}" && s["defined"] == true), "{}", v["report"]["symbols"]);
        assert_eq!(v["report"]["summary"]["undefined_symbol"].as_u64(), v["report"]["issues"].as_array().map(|a| a.iter().filter(|i| i["code"] == "undefined_symbol").count() as u64).filter(|n| *n > 0));
        // a symbol the notes do not define stays undefined
        assert_eq!(undefined(&one, "\\boldsymbol{q}"), ["slide-01.tex"]);
        // like a definition on a slide: a later slide's use is defined, an earlier one is used
        // before the definition (not undefined)
        let later = line_deck(&[("\\boldsymbol{u} + \\boldsymbol{q}", notes), ("\\boldsymbol{u} - 1", "")]);
        assert!(undefined(&later, "\\boldsymbol{u}").is_empty());
        let earlier = line_deck(&[("\\boldsymbol{u} - 1", ""), ("\\boldsymbol{u} + \\boldsymbol{q}", notes)]);
        assert!(undefined(&earlier, "\\boldsymbol{u}").is_empty());
        let v = view(&earlier, &BTreeMap::new(), &Notation::default(), &Notation::default());
        let early: Vec<&Value> = v["report"]["issues"].as_array().unwrap().iter().filter(|i| i["code"] == "used_before_definition" && i["subject"] == "\\boldsymbol{u}").collect();
        assert_eq!(early.len(), 1, "{}", v["report"]["issues"]);
        assert_eq!(early[0]["locations"].as_array().unwrap().iter().map(|l| l["file"].as_str().unwrap()).collect::<Vec<_>>(), ["slide-01.tex"]);
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
