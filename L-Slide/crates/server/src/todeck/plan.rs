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

/// How many slides a deck of this length has: 短 5–10, 標準 10–15, 長 more than 15 (the
/// prompt asks for 16–24). Counted on the plan — cover and thanks included, the copies a
/// line-by-line reveal adds are not.
pub fn target_slides(length: &str) -> (usize, usize) {
    match length {
        "short" => (5, 10),
        "long" => (16, 24),
        _ => (10, 15),
    }
}

/// The limits a slide's text must keep so that every slide uses one size per role and
/// nothing is shrunk to fit (the build never shrinks; a plan over a limit is asked again,
/// then split or reported).
pub mod limit {
    pub const TITLE: usize = 48;
    pub const BULLETS: usize = 5;
    pub const BULLET: usize = 90;
    /// Next to a figure or a table.
    pub const BULLETS_VISUAL: usize = 3;
    pub const BULLET_VISUAL: usize = 80;
    pub const TAKEAWAY: usize = 100;
    pub const STEPS: usize = 6;
    pub const STEP: usize = 60;
    pub const KV: usize = 8;
    pub const TAKE_HOME: usize = 5;
}

fn chars(s: &str) -> usize {
    // inline math counts as its rendered length, roughly half the source
    let mut n = 0usize;
    for (i, part) in s.split('$').enumerate() {
        let c = part.chars().count();
        n += if i % 2 == 1 { c.div_ceil(2) } else { c };
    }
    n
}

/// What of `p` breaks the limits or the length, one English line each (for the corrective
/// request; empty = the plan is fine). Slides are named by their 1-based position.
pub fn check_plan(p: &Plan, length: &str) -> Vec<String> {
    let (lo, hi) = target_slides(length);
    let mut out = vec![];
    let n = p.slides.len();
    if n < lo || n > hi {
        out.push(format!("The plan has {n} slides; it must have between {lo} and {hi} (cover and thanks included)."));
    }
    for (i, s) in p.slides.iter().enumerate() {
        let at = format!("Slide {} (\"{}\")", i + 1, clip(&s.title, 40));
        if !matches!(s.role.as_str(), "cover") && chars(&s.title) > limit::TITLE {
            out.push(format!("{at}: the title has {} characters, at most {}.", chars(&s.title), limit::TITLE));
        }
        let visual = !s.figure.is_empty() || !s.table.is_empty() || !s.kv.is_empty();
        let (nb, nc) = if s.role == "takeaway" { (limit::TAKE_HOME, limit::BULLET) } else if visual { (limit::BULLETS_VISUAL, limit::BULLET_VISUAL) } else { (limit::BULLETS, limit::BULLET) };
        if s.bullets.len() > nb {
            out.push(format!("{at}: {} bullets, at most {nb}{}; cut or move some to another slide.", s.bullets.len(), if visual { " next to a figure or table" } else { "" }));
        }
        for b in s.bullets.iter().filter(|b| chars(b) > nc) {
            out.push(format!("{at}: the bullet \"{}\" has {} characters, at most {nc}; shorten it.", clip(b, 30), chars(b)));
        }
        if chars(&s.takeaway) > limit::TAKEAWAY {
            out.push(format!("{at}: the takeaway has {} characters, at most {}.", chars(&s.takeaway), limit::TAKEAWAY));
        }
        if s.steps.len() > limit::STEPS || s.steps.iter().any(|x| chars(x) > limit::STEP) {
            out.push(format!("{at}: at most {} steps of at most {} characters.", limit::STEPS, limit::STEP));
        }
        if s.kv.len() > limit::KV {
            out.push(format!("{at}: {} key-value rows, at most {}.", s.kv.len(), limit::KV));
        }
    }
    out
}

/// What is still over a limit after the corrective request, handled without shrinking:
/// bullets over the count move to a second slide of the same role right after (same title);
/// anything else over a limit is left as written and reported. Returns the report lines
/// (繁中) for the job's warnings.
pub fn enforce_limits(p: &mut Plan) -> Vec<String> {
    let mut notes = vec![];
    let mut out: Vec<PlanSlide> = vec![];
    for s in std::mem::take(&mut p.slides) {
        let visual = !s.figure.is_empty() || !s.table.is_empty() || !s.kv.is_empty();
        let nb = if s.role == "takeaway" { limit::TAKE_HOME } else if visual { limit::BULLETS_VISUAL } else { limit::BULLETS };
        let nc = if visual && s.role != "takeaway" { limit::BULLET_VISUAL } else { limit::BULLET };
        let long = s.bullets.iter().filter(|b| chars(b) > nc).count();
        if long > 0 {
            notes.push(format!("「{}」有 {long} 行超過 {nc} 字，沒有縮字，請自行刪短", clip(&s.title, 30)));
        }
        if chars(&s.title) > limit::TITLE && s.role != "cover" {
            notes.push(format!("標題「{}」超過 {} 字，可能換行", clip(&s.title, 30), limit::TITLE));
        }
        if s.bullets.len() > nb && !matches!(s.role.as_str(), "cover" | "thanks") {
            let mut first = s.clone();
            let rest: Vec<String> = first.bullets.split_off(nb);
            notes.push(format!("「{}」有 {} 行，一頁放不下：拆成兩頁", clip(&s.title, 30), s.bullets.len()));
            let more = PlanSlide { role: s.role.clone(), title: s.title.clone(), bullets: rest, cites: vec![], notes: String::new(), ..Default::default() };
            first.takeaway = String::new();
            let mut more = more;
            more.takeaway = s.takeaway.clone();
            out.push(first);
            out.push(more);
        } else {
            out.push(s);
        }
    }
    p.slides = out;
    notes
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
    let (lt, lb, lbc, lv, lvc, ltk) = (limit::TITLE, limit::BULLETS, limit::BULLET, limit::BULLETS_VISUAL, limit::BULLET_VISUAL, limit::TAKEAWAY);
    let system = format!(
        "You plan a paper-report seminar deck about one research paper, the way the presenter always builds them. \
A program lays out the slides from your plan: you decide WHAT each slide says and which of the paper's figures, tables and equations it shows, never where things go.\n\n\
Reply with one JSON object {{venue, keywords, slides}}. Between {lo} and {hi} slides in total (cover and thanks included), in this order (skip a role the paper has no material for, repeat method / results when needed):\n\
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
- takeaway: title \"Take home messages\"; 3–5 messages in bullets.\n\
- thanks: title \"Thanks\".\n\n\
Rules:\n\
- Titles are short noun phrases (\"Results of ablation study\", \"Sampling method: …\"), at most {lt} characters; no agenda slide.\n\
- Every slide uses the same text size and nothing is shrunk to fit, so the limits are hard: at most {lb} bullets of at most {lbc} characters each; \
on a slide with a figure, a table or kv rows at most {lv} bullets of at most {lvc} characters; a takeaway of at most {ltk} characters. \
When a point needs more room, cut words or make a second slide — never go over.\n\
- Bullets: short, spoken, start with a verb. Inline math between $ signs as LaTeX. Never start a bullet with a number, a dash or a bullet sign: the program numbers and marks them.\n\
- Never write figure, table or equation numbers (no \"Fig. 3\", \"Table 2\", \"Eq. (1)\"): the slide shows the item itself.\n\
- figure / table / equations use ONLY ids from the material lists that are not marked as unavailable; \"\" or [] when none.\n\
- A figure or table appears on ONE slide, the slide that talks about it (only the main architecture figure repeats, on the overview and its method slides).\n\
- cites: bibliography keys from the list for background claims from other work, at most 2 per slide.\n\
- notes: one or two sentences the presenter says (speaker notes).\n\
- Write all slide text in {language}."
    );
    let mut u = String::new();
    // a draft paper (L-Paper 待補): results still to come are marked, never filled in
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
        let na = if f.image.is_none() { " [UNAVAILABLE: no picture could be taken from the paper, do not use this id]" } else { "" };
        u += &format!("- {}{na}: {}\n", f.id, clip(&f.caption, 300));
    }
    u += "\n## Tables (id: caption)\n";
    for t in &m.tables {
        let how = match (&t.grid, &t.image) {
            (Some(g), _) if !g.is_empty() => " (shown as a table)",
            (_, Some(_)) => " (shown as a picture of the paper's table)",
            _ => " [UNAVAILABLE: do not use this id]",
        };
        u += &format!("- {}{how}: {}\n", t.id, clip(&t.caption, 300));
        if let Some(g) = &t.grid {
            for row in g.iter().take(14) {
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
    u += &format!("\nPlan between {lo} and {hi} slides in total.");
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

/// Material a slide asks for but the deck cannot show (a figure without a picture, a table
/// without cells or picture, an equation without LaTeX or crop) is taken off the slide, and
/// a figure or table already shown on an earlier slide is taken off the later one (only the
/// main figure repeats, on the overview and its method slides). One report line each (繁中).
pub fn drop_unusable(p: &mut Plan, m: &Material) -> Vec<String> {
    let mut notes = vec![];
    let main = p.slides.iter().find(|s| s.role == "overview").map(|s| s.figure.clone()).unwrap_or_default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for s in &mut p.slides {
        let title = clip(&s.title, 30);
        if !s.figure.is_empty() {
            let has = m.figures.iter().chain(&m.tables).find(|f| f.id == s.figure).is_some_and(|f| f.image.is_some());
            let walk = s.figure == main && matches!(s.role.as_str(), "overview" | "method");
            if !has {
                notes.push(format!("「{title}」要用的 {} 沒有圖檔，這頁沒有放圖", s.figure));
                s.figure.clear();
            } else if !walk && !seen.insert(s.figure.clone()) {
                notes.push(format!("「{title}」的 {} 前面的頁已經用過，這頁不重複放", s.figure));
                s.figure.clear();
            } else {
                seen.insert(s.figure.clone());
            }
        }
        if !s.table.is_empty() {
            let has = m.tables.iter().find(|t| t.id == s.table).is_some_and(|t| t.image.is_some() || t.grid.as_ref().is_some_and(|g| !g.is_empty()));
            if !has {
                notes.push(format!("「{title}」要用的 {} 讀不出內容，這頁沒有放表", s.table));
                s.table.clear();
            } else if !seen.insert(s.table.clone()) {
                notes.push(format!("「{title}」的 {} 前面的頁已經用過，這頁不重複放", s.table));
                s.table.clear();
            }
        }
        let before = s.equations.len();
        s.equations.retain(|id| m.equations.iter().find(|e| &e.id == id).is_some_and(|e| e.usable() || e.image.is_some()));
        if s.equations.len() < before {
            notes.push(format!("「{title}」有 {} 條方程式讀不出來，沒有放", before - s.equations.len()));
        }
    }
    notes
}

/// A line without the list mark a model wrote in front of it (`1.`, `2)`, `(3)`, `•`, `-`):
/// the builder numbers and marks the lines itself. A number that belongs to the sentence
/// ("3 rules give a state", "2.5 mm", "10-fold") stays.
pub fn strip_lead(s: &str) -> String {
    let re = regex::Regex::new(r"^\s*(?:[•·▪◦●○*]\s*|[-–—]\s+|\(?\d{1,2}[.)、]\s+|\(\d{1,2}\)\s*)").unwrap();
    let mut t = s.trim().to_string();
    for _ in 0..2 {
        let n = re.replace(&t, "").trim_start().to_string();
        if n == t {
            break;
        }
        t = n;
    }
    t
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
            *b = strip_lead(&strip_typed_refs(b));
        }
        s.takeaway = strip_lead(&s.takeaway);
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
mod limit_tests {
    use super::*;

    #[test]
    fn list_marks_a_model_wrote_are_dropped_but_numbers_of_the_sentence_stay() {
        for (a, b) in [("1. Combine both", "Combine both"), ("2) Use the queue", "Use the queue"), ("(3) Keep rules", "Keep rules"), ("• Prefer it", "Prefer it"), ("- Train longer", "Train longer"), ("3 rules give a state", "3 rules give a state"), ("2.5 mm slices", "2.5 mm slices"), ("10-fold validation", "10-fold validation"), ("1. 2. Twice", "Twice")] {
            assert_eq!(strip_lead(a), b);
        }
    }

    #[test]
    fn lengths_and_limits_are_checked_then_split_never_shrunk() {
        assert_eq!((target_slides("short"), target_slides("standard"), target_slides("long")), ((5, 10), (10, 15), (16, 24)));
        let ps = |role: &str, n: usize| PlanSlide { role: role.into(), title: format!("{role} slide"), bullets: (0..n).map(|i| format!("Point number {i}")).collect(), ..Default::default() };
        let mut p = Plan { slides: vec![ps("cover", 0), ps("background", 7), ps("method", 2), ps("thanks", 0)], ..Default::default() };
        p.slides[2].figure = "fig1".into();
        p.slides[2].bullets[0] = "x".repeat(limit::BULLET_VISUAL + 1);
        let problems = check_plan(&p, "standard");
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("4 slides") && problems[0].contains("between 10 and 15"));
        assert!(problems[1].contains("7 bullets") && problems[2].contains("shorten it"));
        let notes = enforce_limits(&mut p);
        assert_eq!(p.slides.iter().map(|s| s.bullets.len()).collect::<Vec<_>>(), [0, 5, 2, 2, 0], "the bullets over the count go to a second slide");
        assert_eq!((p.slides[1].title.as_str(), p.slides[2].title.as_str(), p.slides[2].role.as_str()), ("background slide", "background slide", "background"));
        assert_eq!(notes.len(), 2, "the split and the long line are reported: {notes:?}");
        assert!(check_plan(&Plan { slides: (0..12).map(|_| ps("background", 3)).collect(), ..Default::default() }, "standard").is_empty());
    }

    #[test]
    fn material_without_a_picture_and_repeats_leave_the_slide_with_a_note() {
        use crate::todeck::source::{Fig, Img};
        let img = || Some(Img { bytes: vec![], file: String::new(), ext: "png".into(), w: 10, h: 10 });
        let m = Material { figures: vec![Fig { id: "fig1".into(), image: img(), ..Default::default() }, Fig { id: "fig2".into(), ..Default::default() }, Fig { id: "fig3".into(), image: img(), ..Default::default() }], ..Default::default() };
        let ps = |role: &str, fig: &str| PlanSlide { role: role.into(), title: role.into(), figure: fig.into(), ..Default::default() };
        let mut p = Plan { slides: vec![ps("overview", "fig1"), ps("method", "fig1"), ps("results", "fig2"), ps("visual", "fig3"), ps("ablation", "fig3"), ps("opinion", "fig1")], ..Default::default() };
        let notes = drop_unusable(&mut p, &m);
        assert_eq!(p.slides.iter().map(|s| s.figure.as_str()).collect::<Vec<_>>(), ["fig1", "fig1", "", "fig3", "", ""], "only the main figure repeats, on its walk-through");
        assert_eq!(notes.len(), 3, "{notes:?}");
        let (_, user) = prompt(&m, "standard", "en");
        assert!(user.contains("fig2 [UNAVAILABLE"), "the planner is told what cannot be shown");
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    /// An L-Paper draft with 待補 marks: the slides' material shows them as marks, the
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
