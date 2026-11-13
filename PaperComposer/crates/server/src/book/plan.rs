//! `plan`: the terms the author does not know, grouped into chapters and ordered by what each
//! needs first (KnowledgeBase `prerequisite` links win over the model's order); per chapter the
//! terms to explain, the depth, what to look up and figure ideas. Then 建議 (`pc-book-suggest`,
//! best effort) and the review window's edits (`apply_review`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::source::Material;

pub const DEPTHS: &[&str] = &["brief", "standard", "deep"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FigureIdea {
    /// `plot` (a lore-plot figure of a function or of numbers the sources give) · `paper` (a
    /// figure of the starting paper, only when its licence allows reuse). Flow diagrams wait for
    /// PaperComposer's flow tool (the same path as the editor's).
    pub kind: String,
    pub idea: String,
    /// `paper`: the paper's figure id (`fig2`).
    pub paper_fig: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Chapter {
    /// `c1`, `c2` … (`n1` … for chapters the author added in the review window).
    pub id: String,
    pub title: String,
    /// Terms this chapter explains, the first ones first.
    pub terms: Vec<String>,
    /// brief · standard · deep
    pub depth: String,
    /// What the reader can do after it (one sentence).
    pub goal: String,
    /// Web search queries for related open-access papers.
    pub queries: Vec<String>,
    pub figures: Vec<FigureIdea>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Plan {
    /// For a new book.
    pub title: String,
    pub abstract_text: String,
    pub chapters: Vec<Chapter>,
}

/// A term the book must explain, with how well the author knows it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Teach {
    pub term: String,
    /// `unknown` (no card, or 要學) · `learning` (shorter, never skipped)
    pub state: String,
    pub n: usize,
    pub card: Option<String>,
}

/// How many chapters each length asks for.
pub fn chapter_range(length: &str) -> (usize, usize) {
    match length {
        "short" => (2, 4),
        "long" => (7, 10),
        _ => (4, 7),
    }
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "abstract", "chapters"],
        "properties": {
            "title": { "type": "string" },
            "abstract": { "type": "string" },
            "chapters": { "type": "array", "minItems": 1, "items": {
                "type": "object",
                "required": ["title", "terms", "depth"],
                "properties": {
                    "title": { "type": "string" },
                    "terms": { "type": "array", "items": { "type": "string" } },
                    "depth": { "type": "string", "enum": DEPTHS },
                    "goal": { "type": "string" },
                    "queries": { "type": "array", "items": { "type": "string" } },
                    "figures": { "type": "array", "items": {
                        "type": "object", "required": ["kind", "idea"],
                        "properties": { "kind": { "type": "string", "enum": ["plot", "paper"] }, "idea": { "type": "string" }, "paper_fig": { "type": "string" } }
                    } }
                }
            } }
        }
    })
}

fn lang_line(lang: &str) -> &'static str {
    match lang {
        "en" => "Chapter titles in English.",
        "zh" => "Chapter titles in Traditional Chinese (Taiwan usage) with the English term where it helps.",
        _ => "Chapter titles in English.",
    }
}

/// The plan prompt. `teach`: terms to explain; `prereq`: `(term, needs term)` from the KnowledgeBase;
/// `existing`: chapter titles the book already has (加進這本書: only new topics); `reusable`: the
/// paper's figures may be reproduced (licence).
#[allow(clippy::too_many_arguments)]
pub fn prompt(m: &Material, teach: &[Teach], prereq: &[(String, String)], existing: &[String], length: &str, lang: &str, reusable: bool) -> (String, String) {
    let (lo, hi) = chapter_range(length);
    let system = format!(
        "You plan a set of course lecture notes written for one reader, in the manner of university lecture notes \
(for example Stanford CS224n's notes): chapters that build from the basics to depth, intuition before equations.\n\
The reader has just read a research paper and does not yet know some of its terms. Plan {lo}–{hi} chapters that teach those terms \
so the reader can follow the paper.\n\
Rules:\n\
- Every listed term must be explained in exactly one chapter; put terms with state `learning` in the same chapters but expect them to be short.\n\
- Order chapters by prerequisites: a chapter never relies on a term that a later chapter explains. Prerequisite pairs from the reader's \
knowledge base are given and must be respected.\n\
- Group related terms; a chapter has 2–8 terms. The list was found automatically in the paper's text: leave out entries that are \
plain English words, names, datasets, or fragments of a sentence (\"augmentation involves\", \"representations learned\"), and merge \
spellings of one term (an acronym and its expansion, a plural) into the term's usual name.\n\
- depth: `brief` (mostly known ideas), `standard`, `deep` (the paper's core method).\n\
- queries: 1–2 short web search queries per chapter for background papers (method names, survey topics), in English.\n\
- figures: 0–2 per chapter, only where a picture helps understanding: `plot` for the shape of a function or numbers the paper \
states{paper}. A pipeline or architecture is explained in words (diagrams come later).\n\
- Do not repeat topics the book already has.\n\
- {lang}\n\
Reply with JSON only.",
        paper = if reusable { ", `paper` with `paper_fig` to reproduce one of the paper's figures (its licence allows it)" } else { "" },
        lang = lang_line(lang),
    );
    let mut user = format!("Paper: {}\n", m.title);
    if !m.abstract_text.is_empty() {
        user += &format!("Abstract: {}\n", m.abstract_text);
    }
    user += &format!("Sections: {}\n", m.sections.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>().join(" · "));
    if reusable && !m.figures.is_empty() {
        user += "Figures of the paper:\n";
        for f in &m.figures {
            user += &format!("- {}: {}\n", f.id, f.caption.chars().take(200).collect::<String>());
        }
    }
    user += "\nTerms to teach (state, times used in the paper):\n";
    for t in teach {
        user += &format!("- {} ({}, {}×)\n", t.term, t.state, t.n);
    }
    if !prereq.is_empty() {
        user += "\nPrerequisites from the knowledge base (A needs B first):\n";
        for (a, b) in prereq {
            user += &format!("- {a} needs {b}\n");
        }
    }
    let qs: Vec<&str> = m.questions.iter().map(|q| q.text.as_str()).chain(m.extras.questions.iter().map(String::as_str)).collect();
    if !qs.is_empty() {
        user += "\nQuestions the reader asked while reading (answer them somewhere in the notes):\n";
        for q in qs.iter().take(20) {
            user += &format!("- {q}\n");
        }
    }
    if !existing.is_empty() {
        user += &format!("\nThe book already has these chapters (do not repeat them): {}\n", existing.join(" · "));
    }
    (system, user)
}

/// Keep what the plan can use: known depths, terms trimmed and each once, no empty chapters,
/// ids `c1…`, `paper` figures only when reusable and of a figure the paper has.
pub fn clean(v: &Value, m: &Material, reusable: bool) -> Plan {
    let s = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut chapters = vec![];
    for c in v["chapters"].as_array().into_iter().flatten() {
        let terms: Vec<String> = c["terms"].as_array().into_iter().flatten().map(s).filter(|t| !t.is_empty() && seen.insert(t.to_lowercase())).collect();
        let title = s(&c["title"]);
        if title.is_empty() || terms.is_empty() {
            continue;
        }
        let depth = Some(s(&c["depth"])).filter(|d| DEPTHS.contains(&d.as_str())).unwrap_or_else(|| "standard".into());
        let figures = c["figures"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| FigureIdea { kind: s(&f["kind"]), idea: s(&f["idea"]), paper_fig: s(&f["paper_fig"]) })
            .filter(|f| match f.kind.as_str() {
                "plot" => !f.idea.is_empty(),
                "paper" => reusable && m.figures.iter().any(|x| x.id == f.paper_fig && x.file.is_some()),
                _ => false,
            })
            .take(2)
            .collect();
        chapters.push(Chapter {
            id: String::new(),
            title,
            terms,
            depth,
            goal: s(&c["goal"]),
            queries: c["queries"].as_array().into_iter().flatten().map(s).filter(|q| !q.is_empty()).take(2).collect(),
            figures,
        });
    }
    for (i, c) in chapters.iter_mut().enumerate() {
        c.id = format!("c{}", i + 1);
    }
    Plan { title: s(&v["title"]), abstract_text: s(&v["abstract"]), chapters }
}

/// Chapters reordered so that a chapter comes after the chapters holding what its terms need
/// (`prereq` term pairs). Stable: otherwise the model's order; a cycle keeps the model's order.
pub fn order(chapters: Vec<Chapter>, prereq: &[(String, String)]) -> Vec<Chapter> {
    let at = |term: &str| chapters.iter().position(|c| c.terms.iter().any(|t| t.eq_ignore_ascii_case(term)));
    let mut needs: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (a, b) in prereq {
        if let (Some(i), Some(j)) = (at(a), at(b))
            && i != j
        {
            needs.entry(i).or_default().insert(j);
        }
    }
    let n = chapters.len();
    let mut placed = vec![false; n];
    let mut out_idx = vec![];
    while out_idx.len() < n {
        let next = (0..n).find(|&i| !placed[i] && needs.get(&i).is_none_or(|d| d.iter().all(|&j| placed[j])));
        let i = next.unwrap_or_else(|| (0..n).find(|&i| !placed[i]).unwrap()); // a cycle: model's order
        placed[i] = true;
        out_idx.push(i);
    }
    let mut slots: Vec<Option<Chapter>> = chapters.into_iter().map(Some).collect();
    out_idx.into_iter().filter_map(|i| slots[i].take()).collect()
}

// ── 建議 ─────────────────────────────────────────────────────────────────────

pub const OPS: &[&str] = &["retitle", "depth", "move", "remove", "merge", "add"];

pub fn suggest_schema() -> Value {
    json!({
        "type": "object", "required": ["suggestions"],
        "properties": { "suggestions": { "type": "array", "maxItems": 5, "items": {
            "type": "object", "required": ["op", "reason"],
            "properties": {
                "op": { "type": "string", "enum": OPS },
                "id": { "type": "string" }, "to": { "type": "string" }, "with": { "type": "string" },
                "title": { "type": "string" }, "depth": { "type": "string", "enum": DEPTHS },
                "terms": { "type": "array", "items": { "type": "string" } },
                "reason": { "type": "string" }
            }
        } } }
    })
}

pub fn suggest_prompt(plan: &Plan, teach: &[Teach], lang: &str) -> (String, String) {
    let system = format!("You review the outline of lecture notes written for one reader before they are written. Suggest at most 5 concrete \
edits that make the notes easier to learn from: retitle {{id,title}}, depth {{id,depth}}, move {{id,to}} (put after `to`, \"\" = first), \
remove {{id}}, merge {{id,with}} (`with` folds into `id`), add {{to,title,terms}}. Each with a one-line reason in Traditional Chinese \
(Taiwan usage); in a reason name a chapter by its title, never by its id. New titles: {}. Only suggest what clearly helps; \
an empty list is fine. Reply with JSON only.",
        lang_line(lang).trim_end_matches('.').to_lowercase()
    );
    let mut user = String::from("Outline:\n");
    for c in &plan.chapters {
        user += &format!("- {} [{}] {} — terms: {}\n", c.id, c.depth, c.title, c.terms.join(", "));
    }
    let unknown: Vec<&str> = teach.iter().filter(|t| t.state == "unknown").map(|t| t.term.as_str()).collect();
    user += &format!("\nTerms the reader does not know at all: {}\n", unknown.join(", "));
    (system, user)
}

/// Suggestions that name rows of this plan, at most 5.
pub fn clean_suggestions(v: &Value, plan: &Plan) -> Vec<Value> {
    let has = |id: &str| plan.chapters.iter().any(|c| c.id == id);
    v["suggestions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| {
            let id = x["id"].as_str().unwrap_or("");
            match x["op"].as_str().unwrap_or("") {
                "retitle" => has(id) && x["title"].as_str().is_some_and(|t| !t.trim().is_empty()),
                "depth" => has(id) && x["depth"].as_str().is_some_and(|d| DEPTHS.contains(&d)),
                "move" => has(id) && x["to"].as_str().is_some_and(|t| t.is_empty() || has(t)),
                "remove" => has(id),
                "merge" => has(id) && x["with"].as_str().is_some_and(|w| has(w) && w != id),
                "add" => x["title"].as_str().is_some_and(|t| !t.trim().is_empty()) && x["to"].as_str().is_none_or(|t| t.is_empty() || has(t)),
                _ => false,
            }
        })
        .take(5)
        .cloned()
        .collect()
}

// ── the review window ────────────────────────────────────────────────────────

/// The plan after the author's review: rows in the page's order, unchecked rows dropped, titles
/// and depths edited, new rows (`n1` …) with their terms. `{skipped: true}` keeps the plan.
pub fn apply_review(plan: &Plan, review: &Value) -> Plan {
    if review["skipped"] == true || review["skip"] == true {
        return plan.clone();
    }
    let Some(rows) = review["chapters"].as_array() else { return plan.clone() };
    let mut out = vec![];
    for r in rows {
        if r["include"] == false {
            continue;
        }
        let id = r["id"].as_str().unwrap_or("");
        let mut c = plan.chapters.iter().find(|c| c.id == id).cloned().unwrap_or_else(|| Chapter { id: id.to_string(), depth: "standard".into(), ..Default::default() });
        if let Some(t) = r["title"].as_str().map(str::trim).filter(|t| !t.is_empty()) {
            c.title = t.to_string();
        }
        if let Some(d) = r["depth"].as_str().filter(|d| DEPTHS.contains(d)) {
            c.depth = d.to_string();
        }
        if let Some(ts) = r["terms"].as_array() {
            c.terms = ts.iter().filter_map(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect();
        }
        if c.id.is_empty() || c.title.is_empty() {
            continue;
        }
        out.push(c);
    }
    Plan { chapters: out, ..plan.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: &str, terms: &[&str]) -> Chapter {
        Chapter { id: id.into(), title: id.into(), terms: terms.iter().map(|s| s.to_string()).collect(), depth: "standard".into(), ..Default::default() }
    }

    #[test]
    fn prerequisites_move_chapters_forward() {
        let cs = vec![ch("c1", &["InfoNCE"]), ch("c2", &["softmax"]), ch("c3", &["temperature"])];
        let p = vec![("InfoNCE".to_string(), "softmax".to_string()), ("softmax".into(), "temperature".into())];
        let ids: Vec<String> = order(cs.clone(), &p).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["c3", "c2", "c1"]);
        let cyc = vec![("InfoNCE".to_string(), "softmax".to_string()), ("softmax".into(), "InfoNCE".into())];
        assert_eq!(order(cs, &cyc).len(), 3, "a cycle loses nothing");
    }

    #[test]
    fn the_plan_is_cleaned() {
        let m = Material { figures: vec![super::super::source::Fig { id: "fig1".into(), file: Some("fig1.png".into()), ..Default::default() }], ..Default::default() };
        let v = json!({ "title": "T", "chapters": [
            { "title": "A", "terms": ["x", "y", "x"], "depth": "huge", "figures": [{ "kind": "paper", "idea": "i", "paper_fig": "fig1" }, { "kind": "plot", "idea": "" }] },
            { "title": "B", "terms": ["X"], "depth": "deep" },
            { "title": "", "terms": ["z"], "depth": "brief" }
        ] });
        let p = clean(&v, &m, false);
        assert_eq!(p.chapters.len(), 1, "B repeats x, the third has no title");
        assert_eq!((p.chapters[0].id.as_str(), p.chapters[0].depth.as_str(), p.chapters[0].terms.len()), ("c1", "standard", 2));
        assert!(p.chapters[0].figures.is_empty(), "no paper figure without the licence, no empty plot");
        assert_eq!(clean(&v, &m, true).chapters[0].figures.len(), 1);
    }

    #[test]
    fn review_reorders_drops_and_adds() {
        let plan = Plan { chapters: vec![ch("c1", &["a"]), ch("c2", &["b"]), ch("c3", &["c"])], ..Default::default() };
        let r = json!({ "chapters": [
            { "id": "c3", "title": "Third first", "depth": "deep" },
            { "id": "c1", "include": false },
            { "id": "n1", "title": "New", "terms": ["d"] },
            { "id": "c2", "depth": "nonsense" }
        ] });
        let p = apply_review(&plan, &r);
        let got: Vec<(&str, &str, &str)> = p.chapters.iter().map(|c| (c.id.as_str(), c.title.as_str(), c.depth.as_str())).collect();
        assert_eq!(got, [("c3", "Third first", "deep"), ("n1", "New", "standard"), ("c2", "c2", "standard")]);
        assert_eq!(apply_review(&plan, &json!({ "skipped": true })), plan);
        let s = clean_suggestions(&json!({ "suggestions": [{ "op": "remove", "id": "c9", "reason": "x" }, { "op": "depth", "id": "c1", "depth": "deep", "reason": "y" }] }), &plan);
        assert_eq!(s.len(), 1);
    }
}
