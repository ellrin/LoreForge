//! 微調's 預想選項 (the author's interview rule: the model pre-thinks 2–3 options grounded in
//! his own records, he picks one or says his own; never a blank form). Inside the `outline`
//! stage, after the plan and its 建議, one best-effort JSON call (task [`TASK`], Opus → Sonnet,
//! no class) proposes
//!
//! - `points`: for each content slide (not cover, take-home, thanks) 2–3 alternative points that
//!   differ in angle, each tied to the paper's figures, tables, equations or sections;
//! - `brief`: 2–3 lines only the presenter can add (e.g. one of his own earlier papers as the
//!   predecessor), tied to his own records.
//!
//! Records the options may cite, bounded, each with an id: the paper's material (`paper:<id>`,
//! `paper:sec<n>`, `paper:abstract`), his L-Paper papers as `GET /api/todeck/papers` lists them
//! (`pc:<folder name>`: title and the head of the abstract, through lore-docsrc's own LaTeX
//! reader; the paper being presented is left out), his own decks in the L-Slide workspace
//! (`deck:<folder>`: the cover title of a deck imported from his files, `deckstyle::qualifies`)
//! and the L-Brain cards the `terms` stage matched (`card:<id>`: the term). His voice lines go
//! along for wording only and are never a record. Titles and abstract lines pass the same
//! exclusions as the voice examples (`deckvoice::Exclude`: his name and id, deny words, e-mail,
//! links, confidential marks); nothing comes from L-Shelf.
//!
//! The reply is checked in code ([`validate`], the idea of L-Paper's `draft::candidates`): every
//! `from` id must be a record that was sent (none, or any unknown one, drops the option), every
//! number in the text must be in the records' text digit for digit (whole tokens), duplicates
//! go, at most [`MAX`] each. The dropped count is one job warning; a failed or unusable call is
//! a warning and no `interview` (the window then takes free answers).

use std::collections::BTreeSet;
use std::path::Path;

use lore_llm::LlmRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::plan::{Plan, clip, strip_typed_refs};
use super::source::Material;
use super::suggest::point_of;
use super::terms::TermsResult;
use crate::deckvoice::Exclude;
use crate::llm::Llm;

pub const TASK: &str = "sc-deck-interview";
/// Options per slide and brief options at most.
pub const MAX: usize = 3;
const MAX_PC: usize = 8;
const MAX_DECKS: usize = 12;
const MAX_CARDS: usize = 20;
const ABSTRACT_HEAD: usize = 300;
/// A long deck: about 20 slides × 3 options, in Chinese by default.
const TOKENS: u32 = 16_000;

/// One option: its text and the record ids it stands on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cand {
    pub text: String,
    pub from: Vec<String>,
}

/// The options for one review row (`s<n>`, the ids of `review.outline`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointCands {
    pub id: String,
    pub cands: Vec<Cand>,
}

/// A record the options cite, as the window shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordRef {
    pub id: String,
    pub label: String,
}

/// `outline.json` → `interview`, shown as `review.interview`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interview {
    pub points: Vec<PointCands>,
    pub brief: Vec<Cand>,
    /// The records the kept options cite (in the order they were sent).
    pub records: Vec<RecordRef>,
}

/// A record sent with the request.
#[derive(Debug, Clone, PartialEq)]
pub struct Rec {
    pub id: String,
    pub label: String,
    pub text: String,
}

/// Slides that get options: every one but the cover, the take-home page and the thanks.
fn content(role: &str) -> bool {
    !matches!(role, "cover" | "takeaway" | "thanks")
}

/// The paper's material as records: abstract, sections (bounded), figure and table captions
/// (table cells too, so a result's numbers can be checked), usable equations.
pub fn paper_records(m: &Material) -> Vec<Rec> {
    let mut out = vec![];
    if !m.abstract_text.trim().is_empty() {
        out.push(Rec { id: "paper:abstract".into(), label: "摘要".into(), text: clip(&m.abstract_text, 1500) });
    }
    let per = (9000 / m.sections.len().max(1)).clamp(300, 1500);
    for (i, (h, t)) in m.sections.iter().enumerate() {
        out.push(Rec { id: format!("paper:sec{}", i + 1), label: h.clone(), text: clip(t, per) });
    }
    for (i, f) in m.figures.iter().enumerate() {
        out.push(Rec { id: format!("paper:{}", f.id), label: format!("圖 {}", i + 1), text: clip(&f.caption, 300) });
    }
    for (i, t) in m.tables.iter().enumerate() {
        let mut text = clip(&t.caption, 300);
        for row in t.grid.iter().flatten().take(14) {
            text += &format!("\n| {} |", row.join(" | "));
        }
        out.push(Rec { id: format!("paper:{}", t.id), label: format!("表 {}", i + 1), text });
    }
    for e in m.equations.iter().filter(|e| e.usable()) {
        out.push(Rec { id: format!("paper:{}", e.id), label: format!("式 {}", e.label.as_deref().unwrap_or(&e.id)), text: clip(&e.latex, 200) });
    }
    out
}

/// The head of an abstract: whole sentences that pass the exclusions, about [`ABSTRACT_HEAD`]
/// characters.
fn abstract_head(text: &str, ex: &Exclude) -> String {
    // exactly the sentences the exclusions saw go (a sentence too long for them stops the head)
    let mut out = String::new();
    for s in text.split_inclusive('。').flat_map(|x| x.split_inclusive(". ")).map(str::trim) {
        if s.is_empty() || s.chars().count() > 190 || !ex.allows(s) || out.chars().count() + s.chars().count() > ABSTRACT_HEAD {
            break;
        }
        out = format!("{out} {s}").trim().to_string();
    }
    out
}

/// L-Paper's papers in the LoreForge folder only (`private/PaperComposer/papers/*`, through
/// lore-home), as `{path, title, name}`: what the isolated tests use instead of
/// `source::pc_papers`, which also follows L-Paper's own settings on this machine.
pub fn papers_in_home(home: &lore_home::Home) -> Vec<Value> {
    let Some(dir) = home.app_dir("PaperComposer").map(|d| d.join("papers")) else { return vec![] };
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| super::source::is_paper(p)).collect();
    v.sort();
    v.into_iter()
        .map(|d| {
            let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let title = lore_docsrc::latex::extract_dir(&d).ok().map(|c| lore_docsrc::latex::plain::tidy(&c.meta.title)).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| name.clone());
            json!({ "path": d, "title": title, "name": name })
        })
        .collect()
}

/// The author's own records (blocking). `pc`: L-Paper's papers as `GET /api/todeck/papers`
/// lists them (`source::pc_papers`, or [`papers_in_home`] in tests); the decks and cards are
/// what the voice examples and the `terms` checkpoint already read. `presented`: the folder of
/// the paper being presented, left out.
pub fn own_records(home: &lore_home::Home, pc: &[Value], presented: Option<&Path>, terms: &TermsResult) -> Vec<Rec> {
    let ex = Exclude::for_home(home);
    let same = |a: &Path, b: &Path| a == b || a.canonicalize().ok().is_some_and(|x| b.canonicalize().ok().as_ref() == Some(&x));
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    for p in pc {
        if out.len() >= MAX_PC {
            break;
        }
        let (Some(path), Some(name), Some(title)) = (p["path"].as_str(), p["name"].as_str(), p["title"].as_str()) else { continue };
        // the folder name is sent as the id: it passes the exclusions like the title
        if presented.is_some_and(|x| same(Path::new(path), x)) || !ex.allows(title) || !ex.allows(name) || !seen.insert(name.to_string()) {
            continue;
        }
        let abs = lore_docsrc::latex::extract_dir(Path::new(path)).ok().and_then(|c| c.meta.abstract_text).map(|a| abstract_head(&lore_docsrc::latex::plain::tidy(&a), &ex)).unwrap_or_default();
        let text = if abs.is_empty() { title.to_string() } else { format!("{title}. {abs}") };
        out.push(Rec { id: format!("pc:{name}"), label: title.to_string(), text });
    }
    if let Some(dir) = crate::deckstyle::author_decks(home) {
        let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        dirs.sort();
        let mut n = 0;
        for d in dirs {
            if n >= MAX_DECKS {
                break;
            }
            let Some(deck) = std::fs::read(d.join("deck.json")).ok().and_then(|b| serde_json::from_slice::<lore_canvas::Deck>(&b).ok()) else { continue };
            let Some(name) = d.file_name().map(|x| x.to_string_lossy().to_string()) else { continue };
            if !crate::deckstyle::qualifies(&deck) {
                continue;
            }
            let title = deck.slides.first().map(|s| crate::deckstyle::title_text(s, deck.size.h)).unwrap_or_default();
            if title.trim().is_empty() || !ex.allows(&title) || !ex.allows(&name) {
                continue;
            }
            out.push(Rec { id: format!("deck:{name}"), label: title.clone(), text: title });
            n += 1;
        }
    }
    let mut cards = BTreeSet::new();
    for t in terms.terms.iter().filter(|t| t.card.is_some()) {
        if cards.len() >= MAX_CARDS {
            break;
        }
        let id = t.card.clone().unwrap_or_default();
        if cards.insert(id.clone()) {
            out.push(Rec { id: format!("card:{id}"), label: t.text.clone(), text: t.text.clone() });
        }
    }
    out
}

pub fn schema() -> Value {
    let s = json!({ "type": "string" });
    let cand = json!({ "type": "object", "additionalProperties": false, "required": ["text", "from"], "properties": { "text": s, "from": { "type": "array", "items": s } } });
    json!({
        "type": "object", "additionalProperties": false, "required": ["points", "brief"],
        "properties": {
            "points": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["id", "cands"], "properties": { "id": s, "cands": { "type": "array", "items": cand } } } },
            "brief": { "type": "array", "items": cand }
        }
    })
}

/// The request: system + user text. `voice`: the presenter's example lines (wording only).
pub fn prompt(plan: &Plan, m: &Material, recs: &[Rec], brief: &[String], lang: &str, voice: &str) -> (String, String) {
    let language = if lang == "en" { "English" } else { "Traditional Chinese as used in Taiwan (technical terms, model names and metrics in English)" };
    let mut system = format!(
        "You prepare answer options for the presenter of a paper-report talk, before the slides are made. The presenter picks one option or writes their own.\n\
1. points: for each slide listed under Slides, 2 or 3 alternative points. A point is the slide's one conclusion, a full sentence of at most 100 characters. \
The options of one slide differ in ANGLE (what to look at, which comparison, which consequence), never rewordings of each other or of the current point. \
Each is tied to a figure, table, equation or section of the paper: cite its record ids in `from`.\n\
2. brief: 2 or 3 lines only the presenter can add, which the paper itself does not say: for example that one of the presenter's own earlier papers or talks \
(pc:, deck: records) is the predecessor of this work, or a concept the presenter already knows (card: records) the audience should recall. Each cites the records it stands on.\n\
3. `from` uses only record ids listed under Records, exactly as written, at least one per option. Numbers only as the records write them, digit for digit; never a number the records do not have. \
Never invent facts about the presenter's work beyond what its record says.\n\
4. Write `text` in {language}; short and spoken. Never write figure, table or equation numbers.\n\
5. Leave a slide out when the records cannot support two different options. Reply with one JSON object {{points, brief}} and nothing else."
    );
    if !voice.trim().is_empty() {
        system += &format!("\n\nThe wording of the options follows the presenter's own lines below; they are no records and are never cited.\n{}", voice.trim());
    }
    let mut u = format!("Paper: {}\n\n## Slides (id · kind · title · current point)\n", m.title);
    for (i, s) in plan.slides.iter().enumerate().filter(|(_, s)| content(&s.role)) {
        u += &format!("- s{} · {} · {} · {}\n", i + 1, s.role, s.title, clip(&point_of(s), 160));
    }
    let given: Vec<&str> = brief.iter().map(|b| b.trim()).filter(|b| !b.is_empty()).collect();
    if !given.is_empty() {
        u += "\n## Lines the presenter already added (do not repeat them)\n";
        for b in given {
            u += &format!("- {b}\n");
        }
    }
    u += "\n## Records (id (label): text)\n";
    for r in recs {
        u += &format!("- {} ({}): {}\n", r.id, r.label, r.text.replace('\n', " "));
    }
    (system, u)
}

/// Numbers a reader would check (as L-Paper's `draft::candidates::numbers`): anything with a
/// decimal point or a percent sign, or two digits and more; digits right after a letter (`T2`,
/// `fig3`) are not numbers.
pub fn numbers(text: &str) -> Vec<String> {
    let mut out = vec![];
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let prev_alpha = i > 0 && (cs[i - 1].is_ascii_alphabetic() || cs[i - 1] == '_');
        if cs[i].is_ascii_digit() && !prev_alpha {
            let mut j = i;
            while j < cs.len() && (cs[j].is_ascii_digit() || (cs[j] == '.' && cs.get(j + 1).is_some_and(|c| c.is_ascii_digit()))) {
                j += 1;
            }
            let n: String = cs[i..j].iter().collect();
            if n.contains('.') || cs.get(j) == Some(&'%') || n.len() >= 2 {
                out.push(n);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Reply {
    points: Vec<PointCands>,
    brief: Vec<Cand>,
}

/// Keep only options that stand on the records sent (see the module doc). Returns what is
/// kept (`None` when nothing is) and how many options were dropped.
pub fn validate(v: &Value, plan: &Plan, recs: &[Rec]) -> (Option<Interview>, usize) {
    let reply: Reply = serde_json::from_value(v.clone()).unwrap_or_default();
    let ids: BTreeSet<&str> = recs.iter().map(|r| r.id.as_str()).collect();
    let corpus: BTreeSet<String> = recs.iter().flat_map(|r| numbers(&format!("{} {}", r.label, r.text))).collect();
    let rows: BTreeSet<String> = plan.slides.iter().enumerate().filter(|(_, s)| content(&s.role)).map(|(i, _)| format!("s{}", i + 1)).collect();
    let mut dropped = 0usize;
    let mut keep = |list: Vec<Cand>| -> Vec<Cand> {
        let mut out: Vec<Cand> = vec![];
        for c in list {
            let text = strip_typed_refs(c.text.trim());
            if text.is_empty() || out.iter().any(|k| k.text == text) {
                continue;
            }
            let mut from: Vec<String> = c.from.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
            from.dedup();
            if from.is_empty() || from.iter().any(|f| !ids.contains(f.as_str())) || numbers(&text).iter().any(|n| !corpus.contains(n)) {
                dropped += 1;
                continue;
            }
            if out.len() < MAX {
                out.push(Cand { text, from });
            }
        }
        out
    };
    let mut points: Vec<PointCands> = vec![];
    for p in reply.points {
        let id = p.id.trim().to_string();
        if !rows.contains(&id) || points.iter().any(|x| x.id == id) {
            continue;
        }
        let cands = keep(p.cands);
        if !cands.is_empty() {
            points.push(PointCands { id, cands });
        }
    }
    let brief = keep(reply.brief);
    if points.is_empty() && brief.is_empty() {
        return (None, dropped);
    }
    let cited: BTreeSet<&str> = points.iter().flat_map(|p| &p.cands).chain(&brief).flat_map(|c| c.from.iter().map(String::as_str)).collect();
    let records = recs.iter().filter(|r| cited.contains(r.id.as_str())).map(|r| RecordRef { id: r.id.clone(), label: r.label.clone() }).collect();
    (Some(Interview { points, brief, records }), dropped)
}

/// Ask for the options (best-effort). Returns the interview (or `None`) and the job warnings.
#[allow(clippy::too_many_arguments)]
pub async fn interview(llm: &Llm, plan: &Plan, m: &Material, recs: &[Rec], brief: &[String], lang: &str, voice: &str, engine: Option<&str>, on_chars: &(dyn Fn(u64) + Send + Sync)) -> (Option<Interview>, Vec<String>) {
    let (system, user) = prompt(plan, m, recs, brief, lang, voice);
    let req = LlmRequest::prompt(system, user).json(schema()).max_tokens(TOKENS);
    match super::outline::ask_json(llm, TASK, req, engine, on_chars).await {
        Ok((v, _)) => {
            let (got, dropped) = validate(&v, plan, recs);
            let mut w = vec![];
            if dropped > 0 {
                w.push(format!("預想選項有 {dropped} 個沒有根據（引用了沒有送出的資料，或數字在資料裡找不到），已拿掉"));
            }
            if got.is_none() && dropped == 0 {
                w.push("預想選項沒有可用的內容：微調視窗照樣可以自己寫".to_string());
            }
            (got, w)
        }
        Err(e) => (None, vec![format!("預想選項沒有做（{}）：微調視窗照樣可以自己寫", e.message)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todeck::plan::PlanSlide;
    use crate::todeck::source::Fig;

    fn fixture() -> (Plan, Material, Vec<Rec>) {
        let ps = |role: &str, title: &str, takeaway: &str| PlanSlide { role: role.into(), title: title.into(), takeaway: takeaway.into(), ..Default::default() };
        let plan = Plan { slides: vec![ps("cover", "T", ""), ps("background", "Why grading is slow", "Hand grading does not scale"), ps("results", "Main results", "Ours wins"), ps("takeaway", "Take home messages", ""), ps("thanks", "Thanks", "")], ..Default::default() };
        let m = Material {
            title: "Two-stage grading".into(),
            abstract_text: "We grade synthetic lesions in two stages.".into(),
            tables: vec![Fig { id: "tab1".into(), caption: "Accuracy per method.".into(), grid: Some(vec![vec!["Ours".into(), "0.91".into()], vec!["Baseline".into(), "0.85".into()]]), ..Default::default() }],
            ..Default::default()
        };
        let mut recs = paper_records(&m);
        recs.push(Rec { id: "pc:early-grader".into(), label: "A one-stage grader for synthetic lesions".into(), text: "A one-stage grader for synthetic lesions. It reached 0.80 on the synthetic set.".into() });
        recs.push(Rec { id: "card:c42".into(), label: "lesion grading".into(), text: "lesion grading".into() });
        (plan, m, recs)
    }

    #[test]
    fn options_stand_on_the_records_sent() {
        let (plan, _, recs) = fixture();
        let reply = json!({
            "points": [
                { "id": "s3", "cands": [
                    { "text": "Ours reaches 0.91 against 0.85 (Table 1)", "from": ["paper:tab1"] },
                    { "text": "The gain holds on both metrics", "from": ["paper:tab1", "paper:abstract"] },
                    { "text": "Ours reaches 0.93", "from": ["paper:tab1"] },
                    { "text": "Better than every prior work", "from": ["paper:fig9"] },
                    { "text": "No source at all", "from": [] },
                    { "text": "The gain holds on both metrics", "from": ["paper:tab1"] }
                ] },
                { "id": "s1", "cands": [{ "text": "Cover", "from": ["paper:abstract"] }] },
                { "id": "s9", "cands": [{ "text": "No such row", "from": ["paper:abstract"] }] }
            ],
            "brief": [
                { "text": "Our earlier one-stage grader reached 0.80 and is the predecessor", "from": ["pc:early-grader"] },
                { "text": "Recall lesion grading from the card", "from": ["card:c42", "deck:never-sent"] }
            ]
        });
        let (got, dropped) = validate(&reply, &plan, &recs);
        let got = got.unwrap();
        assert_eq!(dropped, 4, "an invented number, an unknown id (twice) and an empty `from`");
        assert_eq!(got.points.len(), 1, "only content rows of this plan: {:?}", got.points);
        assert_eq!(got.points[0].cands.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Ours reaches 0.91 against 0.85", "The gain holds on both metrics"]);
        assert_eq!(got.brief, [Cand { text: "Our earlier one-stage grader reached 0.80 and is the predecessor".into(), from: vec!["pc:early-grader".into()] }]);
        assert_eq!(got.records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["paper:abstract", "paper:tab1", "pc:early-grader"]);
        let (none, n) = validate(&json!({ "points": [], "brief": [{ "text": "x", "from": ["pc:nope"] }] }), &plan, &recs);
        assert!(none.is_none() && n == 1);
    }

    #[test]
    fn the_request_lists_content_rows_and_only_records() {
        let (plan, m, recs) = fixture();
        let (system, user) = prompt(&plan, &m, &recs, &["Mention the lab dataset".into()], "en", "Write like the presenter. lines \"Split hard tasks\"");
        assert!(system.contains("2 or 3 alternative points") && system.contains("never cited") && system.contains("Split hard tasks"));
        assert!(user.contains("- s2 · background") && user.contains("- s3 · results") && !user.contains("- s1 ·") && !user.contains("- s4 ·"), "{user}");
        assert!(user.contains("- Mention the lab dataset") && user.contains("| Ours | 0.91 |"));
        let recs_part = user.split("## Records").nth(1).unwrap();
        for line in recs_part.lines().filter(|l| l.starts_with("- ")) {
            assert!(["- paper:", "- pc:", "- deck:", "- card:"].iter().any(|k| line.starts_with(k)), "{line}");
        }
    }

    #[test]
    fn numbers_are_what_a_reader_would_check() {
        assert_eq!(numbers("Dice 0.61, up 12%, fig3 and T2 are not numbers, 3 runs"), ["0.61", "12"]);
    }

    /// The author's own records: his L-Paper papers (title + abstract head, the presented one
    /// left out), his imported decks (cover title), matched cards; names, deny words and
    /// e-mail lines never go.
    #[test]
    fn own_records_are_bounded_and_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let home = lore_home::Home::at(dir.path());
        let shared = dir.path().join("private/shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("deny-words.txt"), "# synthetic\nexample-company\n").unwrap();
        let papers = dir.path().join("private/PaperComposer/papers");
        let write = |name: &str, title: &str, abs: &str| {
            let p = papers.join(name);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("main.tex"), format!("\\documentclass{{article}}\n\\title{{{title}}}\n\\begin{{document}}\n\\begin{{abstract}}\n{abs}\n\\end{{abstract}}\n\\section{{Intro}}\nText.\n\\end{{document}}\n")).unwrap();
            p
        };
        write("early-grader", "A one-stage grader", "We grade lesions in one stage. Contact ellrin@example.org for data. More text.");
        write("secret", "Work at Example-Company", "Nothing.");
        write("example-company-notes", "A clean title", "Nothing.");
        let long = format!("We study grading {} at example-company in the end. More.", "with many careful words ".repeat(10));
        write("long-abstract", "Grading in long sentences", &long);
        let presented = write("now", "Two-stage grading", "This paper.");
        let recs = own_records(&home, &papers_in_home(&home), Some(&presented), &TermsResult::default());
        let ids: Vec<&str> = recs.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"pc:early-grader") && !ids.contains(&"pc:now") && !ids.contains(&"pc:secret") && !ids.contains(&"pc:example-company-notes"), "a deny word in the title or the folder name keeps it out: {ids:?}");
        assert_eq!(recs.iter().find(|r| r.id == "pc:long-abstract").unwrap().text, "Grading in long sentences", "a sentence too long for the exclusions is not sent, not even its head");
        let r = recs.iter().find(|r| r.id == "pc:early-grader").unwrap();
        assert_eq!(r.text, "A one-stage grader. We grade lesions in one stage.", "the abstract head stops at a line the exclusions refuse");
        assert!(recs.iter().all(|r| !r.text.contains('@') && !r.text.contains(&dir.path().to_string_lossy().to_string())));
    }

    #[test]
    fn the_task_is_opus_then_sonnet_without_a_class() {
        let t = crate::llm::app_tasks();
        let spec = &t.task[TASK];
        assert_eq!((spec.default.as_str(), spec.fallback.as_deref(), spec.class), ("claude:opus", Some("claude:sonnet"), None));
    }
}
