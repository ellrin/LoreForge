//! `outline`'s prompt and schema, and the cleaning of the model's plan (ids the material
//! has, typed references dropped, cover first and thanks last).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::source::Material;

/// Page roles of a paper report, in the author's order (design §7.1).
pub const ROLES: [&str; 13] = ["cover", "background", "challenge", "overview", "method", "loss", "training", "results", "ablation", "visual", "opinion", "takeaway", "thanks"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanSlide {
    pub role: String,
    pub title: String,
    pub bullets: Vec<String>,
    pub figure: String,
    pub table: String,
    pub equations: Vec<String>,
    pub takeaway: String,
    pub steps: Vec<String>,
    pub module: u32,
    pub kv: Vec<Vec<String>>,
    pub cites: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub venue: String,
    pub keywords: Vec<String>,
    pub slides: Vec<PlanSlide>,
}

pub fn target_slides(length: &str) -> (usize, usize) {
    match length {
        "short" => (9, 11),
        "long" => (24, 30),
        _ => (15, 20),
    }
}

pub fn schema() -> Value {
    let s = json!({ "type": "string" });
    let list = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["venue", "keywords", "slides"],
        "properties": {
            "venue": s, "keywords": list,
            "slides": { "type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["role", "title", "bullets"],
                "properties": {
                    "role": { "type": "string", "enum": ROLES },
                    "title": s, "bullets": list, "figure": s, "table": s, "equations": list,
                    "takeaway": s, "steps": list, "module": { "type": "integer" },
                    "kv": { "type": "array", "items": list }, "cites": list, "notes": s
                }
            } }
        }
    })
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n).collect::<String>() + "…" }
}

pub fn prompt(m: &Material, length: &str, lang: &str) -> (String, String) {
    let (lo, hi) = target_slides(length);
    let language = if lang == "en" { "English" } else { "Traditional Chinese as used in Taiwan (keep technical terms, model names and metrics in English)" };
    let system = format!(
        "You plan a paper-report seminar deck about one research paper, the way the presenter always builds them. \
A program lays out the slides from your plan: you decide WHAT each slide says and which of the paper's figures, tables and equations it shows, never where things go.\n\n\
Reply with one JSON object {{venue, keywords, slides}}. {lo}–{hi} slides in this order (skip a role the paper has no material for, repeat method / results when needed):\n\
- cover: the paper title as title; venue = journal or conference and year if the text says so (else empty); keywords = 3–6.\n\
- background (1–2): the field or clinical context.\n\
- challenge: the problem and the intuition.\n\
- overview: the big picture — figure = the main architecture figure; steps = 3–6 short numbered steps of the method, in order (each later gets its own method slide).\n\
- method: one per step, module = the step number (1-based); figure = the same architecture figure; equations = ids of the equations of that module; bullets explain it.\n\
- loss: the training objective; equations = the loss equation ids.\n\
- training: training details; kv = [[key, value], …] rows (data, model, optimiser, learning rate, epochs, batch size, hardware…), only values the paper states.\n\
- results: table = the main results table id (or figure); takeaway = one sentence reading the result.\n\
- ablation: table or figure id; takeaway = one sentence.\n\
- visual: qualitative figure id; bullets about what it shows, including failure cases if any.\n\
- opinion: title \"Some opinions\"; the presenter's critical view (strengths, limits, questions).\n\
- takeaway: title \"Take home messages\"; 3–5 numbered messages in bullets.\n\
- thanks: title \"Thanks\".\n\n\
Rules:\n\
- Titles are short noun phrases (\"Results of ablation study\", \"Sampling method: …\"); no agenda slide.\n\
- Bullets: short, spoken, start with a verb; at most 5 per slide, at most 90 characters each. Inline math between $ signs as LaTeX.\n\
- Never write figure, table or equation numbers (no \"Fig. 3\", \"Table 2\", \"Eq. (1)\"): the slide shows the item itself.\n\
- figure / table / equations use ONLY ids from the material lists; \"\" or [] when none.\n\
- cites: bibliography keys from the list for background claims from other work.\n\
- notes: one or two sentences the presenter says (speaker notes).\n\
- Write all slide text in {language}."
    );
    let mut u = String::new();
    // a draft paper (PaperComposer 待補): results still to come are marked, never filled in
    let pending = m.abstract_text.contains("[pending")
        || m.sections.iter().any(|(_, t)| t.contains("[pending"))
        || m.tables.iter().any(|t| t.grid.iter().flatten().flatten().any(|c| c.contains("[pending")));
    let system = if pending { system + PENDING_RULE } else { system };
    u += &format!("Title: {}\n", m.title);
    if !m.authors.is_empty() {
        u += &format!("Authors: {}\n", m.authors.join(", "));
    }
    if !m.venue.is_empty() {
        u += &format!("Venue: {} {}\n", m.venue, m.year);
    }
    if !m.keywords.is_empty() {
        u += &format!("Keywords: {}\n", m.keywords.join(", "));
    }
    if !m.abstract_text.is_empty() {
        u += &format!("\nAbstract:\n{}\n", clip(&m.abstract_text, 2500));
    }
    let budget = 22_000usize;
    let per = (budget / m.sections.len().max(1)).clamp(600, 4000);
    u += "\n## Sections\n";
    for (h, t) in &m.sections {
        u += &format!("\n### {h}\n{}\n", clip(t, per));
    }
    u += "\n## Figures (id: caption)\n";
    for f in &m.figures {
        u += &format!("- {}: {}\n", f.id, clip(&f.caption, 300));
    }
    u += "\n## Tables (id: caption)\n";
    for t in &m.tables {
        u += &format!("- {}: {}\n", t.id, clip(&t.caption, 300));
        if let Some(g) = &t.grid {
            for row in g.iter().take(8) {
                u += &format!("    | {} |\n", row.join(" | "));
            }
        }
    }
    u += "\n## Equations (id: LaTeX)\n";
    for e in &m.equations {
        let label = e.label.as_deref().map(|l| format!(" {l}")).unwrap_or_default();
        if e.usable() {
            u += &format!("- {}{label}: {}\n", e.id, clip(&e.latex, 300));
        } else if e.image.is_some() {
            u += &format!("- {}{label}: (shown as a picture of the paper's equation)\n", e.id);
        }
    }
    if !m.refs.is_empty() {
        u += "\n## Bibliography (key: entry)\n";
        for (k, v) in m.refs.iter().take(80) {
            u += &format!("- {k}: {}\n", clip(v, 160));
        }
    }
    u += &format!("\nPlan {lo}–{hi} slides.");
    (system, u)
}

/// The rule for a draft with results still to come (`[pending: …]` in the material).
pub const PENDING_RULE: &str = "\n- This is a draft: text marked [pending: …] or [pending] is a result the authors still wait for. \
Where a slide uses it, write the mark as it is (\"[pending: external Dice]\"); never a number, a trend or a conclusion for it. \
A sentence ending in [pending] states the result the authors EXPECT, not one they found: on a slide keep the mark at its end \
(\"Gain holds on a second site [pending]\"), never as a takeaway or a take-home message without it.";

/// Typed cross-references ("Fig. 3", "Table 2", "Eq. (1)") out of slide text: a slide shows
/// the item itself, and the author's rule is `\ref`, never a typed number (lore-notation's
/// `hardcoded_reference`).
pub fn strip_typed_refs(s: &str) -> String {
    let re = regex::Regex::new(r"(?i)\s*\(?\b(?:fig(?:ure)?s?\.?|tab(?:le)?s?\.?|eqs?\.?|equations?|sec(?:tion)?s?\.?)\s*~?\(?\d+[a-z]?\)?\)?").unwrap();
    let out = re.replace_all(s, "");
    let out = out.replace(" ,", ",").replace(" .", ".").replace("()", "");
    out.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches([',', ';', ' ']).to_string()
}

/// Keep only ids the material has; enforce cover first and thanks last.
pub fn clean_plan(mut p: Plan, m: &Material) -> Plan {
    let figs: BTreeSet<&str> = m.figures.iter().map(|f| f.id.as_str()).collect();
    let tabs: BTreeSet<&str> = m.tables.iter().map(|f| f.id.as_str()).collect();
    let eqs: BTreeSet<&str> = m.equations.iter().map(|e| e.id.as_str()).collect();
    p.slides.retain(|s| ROLES.contains(&s.role.as_str()));
    p.slides.truncate(40);
    for s in &mut p.slides {
        if !figs.contains(s.figure.as_str()) {
            s.figure = if tabs.contains(s.figure.as_str()) && s.table.is_empty() { std::mem::take(&mut s.figure) } else { String::new() };
            if !s.figure.is_empty() {
                s.table = std::mem::take(&mut s.figure);
            }
        }
        if !tabs.contains(s.table.as_str()) {
            s.table.clear();
        }
        s.equations.retain(|e| eqs.contains(e.as_str()));
        s.cites.retain(|c| m.refs.contains_key(c));
        s.title = strip_typed_refs(&s.title);
        s.takeaway = strip_typed_refs(&s.takeaway);
        for b in s.bullets.iter_mut().chain(s.steps.iter_mut()) {
            *b = strip_typed_refs(b);
        }
        s.bullets.retain(|b| !b.is_empty());
        s.kv.retain(|r| r.len() >= 2 && !r[0].trim().is_empty());
    }
    if p.slides.first().map(|s| s.role.as_str()) != Some("cover") {
        p.slides.insert(0, PlanSlide { role: "cover".into(), title: m.title.clone(), ..Default::default() });
    }
    if p.slides.last().map(|s| s.role.as_str()) != Some("thanks") {
        p.slides.push(PlanSlide { role: "thanks".into(), title: "Thanks".into(), ..Default::default() });
    }
    // method slides without a figure use the big picture's
    let main = p.slides.iter().find(|s| s.role == "overview" && !s.figure.is_empty()).map(|s| s.figure.clone());
    if let Some(f) = main {
        for s in p.slides.iter_mut().filter(|s| s.role == "method" && s.figure.is_empty()) {
            s.figure = f.clone();
        }
    }
    p
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    /// A PaperComposer draft with 待補 marks: the slides' material shows them as marks, the
    /// planner is told to keep them, and a table cell on a slide reads `[pending]`.
    #[test]
    fn a_draft_with_pending_results_keeps_its_marks() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("main.tex"),
            "\\documentclass{article}\n\\input{pending}\n\\title{Weighted frames}\n\\begin{document}\n\\maketitle\n\\begin{abstract}\nWeighting holds on a second site\\pendingdep{P1}.\n\\end{abstract}\n\
\\section{Results}\nExternal Dice is \\pending{P1}{external Dice}, which keeps the ranking\\pendingdep{P1}.\n\n\
\\begin{table}\n\\caption{Dice per site.}\\label{tab:site}\n\\begin{tabular}{lcc}\nMethod & Internal & External \\\\\nOurs & 0.72 & \\pendingcell{P1}{Ours / External} \\\\\n\\end{tabular}\n\\end{table}\n\\end{document}\n",
        )
        .unwrap();
        let m = super::super::source::from_latex(d.path(), &d.path().join("cache")).unwrap();
        let text: String = m.sections.iter().map(|(_, t)| t.clone()).collect();
        assert!(text.contains("[pending: external Dice]") && !text.contains("pendingdep") && !text.contains("P1external"), "{text}");
        assert!(text.contains("keeps the ranking [pending]."), "a dependent sentence stays marked: {text}");
        assert!(m.abstract_text.contains("second site [pending]."), "the abstract too: {}", m.abstract_text);
        let grid = m.tables[0].grid.as_ref().unwrap();
        assert_eq!(grid[1], ["Ours", "0.72", "[pending]"]);
        let (system, user) = prompt(&m, "standard", "en");
        assert!(system.contains(PENDING_RULE) && user.contains("| Ours | 0.72 | [pending] |"));
        let plain = super::super::source::Material { title: "x".into(), ..Default::default() };
        assert!(!prompt(&plain, "standard", "en").0.contains(PENDING_RULE), "no marks, no rule");
    }
}
