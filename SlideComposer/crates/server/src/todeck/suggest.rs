//! 微調's 建議: right after the outline, a model reads it (with the author's style summary and
//! the paper's figures) and proposes at most [`MAX`] concrete edits, each with a one-line reason
//! (lore-llm task `sc-outline-review`, class `create`). Best-effort: no engine, a reply that is
//! not JSON of the schema, or nothing worth saying all give an empty list, never a failed job.
//!
//! An edit names review rows by their ids (`s1…sN`, the same ids as the review window's
//! outline); the page applies 套用 on its rows, so the review body stays the documented one.
//!
//! | `op` | fields | meaning |
//! |---|---|---|
//! | `add` | `to`, `kind`, `title`, `point`, `fig?` | new slide after row `to` (`""` = first) |
//! | `remove` | `id` | drop the slide |
//! | `move` | `id`, `to` | put the slide after row `to` (`""` = first) |
//! | `retitle` | `id`, `title` | new title |
//! | `point` | `id`, `point` | new take-away / first point |
//! | `figure` | `id`, `fig` | show this figure or table (`""` = none) |
//! | `merge` | `id`, `with`, `title?` | fold row `with` into `id` (`with` is removed) |
//! | `split` | `id`, `title`, `point` | a second slide of the same kind right after `id` |

use std::collections::BTreeSet;

use lore_llm::LlmRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::plan::{Plan, PlanSlide, ROLES, clip, target_slides};
use super::source::Material;
use crate::llm::Llm;

pub const MAX: usize = 5;
pub const OPS: [&str; 8] = ["add", "remove", "move", "retitle", "point", "figure", "merge", "split"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Suggestion {
    pub op: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// `add` / `move`: the row it goes after (`""` = first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// `merge`: the row folded in.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub with: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<String>,
    /// `figure` / `add`: a figure or table id (`""` = none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fig: Option<String>,
    /// One line, Traditional Chinese.
    pub reason: String,
}

/// The line a review row shows under its title: the take-away, else the first bullet.
pub fn point_of(s: &PlanSlide) -> String {
    if !s.takeaway.trim().is_empty() { s.takeaway.clone() } else { s.bullets.first().cloned().unwrap_or_default() }
}

pub fn figs_of(s: &PlanSlide) -> Vec<String> {
    [&s.figure, &s.table].into_iter().filter(|x| !x.is_empty()).cloned().collect()
}

/// The review window's rows of a plan (`s1…sN`).
pub fn rows(plan: &Plan) -> Vec<Value> {
    plan.slides.iter().enumerate().map(|(i, s)| json!({ "id": format!("s{}", i + 1), "kind": s.role, "title": s.title, "figs": figs_of(s), "point": point_of(s) })).collect()
}

pub fn schema() -> Value {
    let s = json!({ "type": "string" });
    json!({
        "type": "object", "additionalProperties": false, "required": ["suggestions"],
        "properties": { "suggestions": { "type": "array", "items": {
            "type": "object", "additionalProperties": false, "required": ["op", "reason"],
            "properties": {
                "op": { "type": "string", "enum": OPS }, "id": s, "to": s, "with": s,
                "kind": { "type": "string", "enum": ROLES }, "title": s, "point": s, "fig": s, "reason": s
            }
        } } }
    })
}

/// The request: system + user text. `style`: the author's statistical summary; `voice`: the
/// examples block (may be empty).
pub fn prompt(plan: &Plan, m: &Material, length: &str, lang: &str, style: &str, voice: &str) -> (String, String) {
    let (lo, hi) = target_slides(length);
    let language = if lang == "en" { "English" } else { "Traditional Chinese as used in Taiwan (technical terms in English)" };
    let mut system = format!(
        "You review the outline of a paper-report seminar deck before the slides are made, for the presenter who will give the talk. \
Propose at most {MAX} concrete edits that clearly improve it, most useful first; reply {{\"suggestions\": []}} when nothing is worth changing. Typical reasons: \
no results slide or no take-home slide; a slide that carries too much (split it) or two thin slides on one point (merge); \
an order unlike the presenter's usual decks (move); a title that does not match the presenter's habits (retitle); \
a figure or table of the paper that would fit a slide better, or a slide that lacks one (figure); a slide that adds nothing (remove).\n\n\
Each edit is one JSON object: op ∈ add | remove | move | retitle | point | figure | merge | split, and reason = one short line in Traditional Chinese (at most 30 characters) saying why.\n\
- add: to = row id it goes after (\"\" = first), kind, title, point, fig (optional).\n\
- remove: id. move: id, to = row id it goes after (\"\" = first).\n\
- retitle: id, title. point: id, point (the slide's take-away).\n\
- figure: id, fig = a figure or table id from the list (\"\" = none).\n\
- merge: id (kept), with (folded in and removed), title (optional).\n\
- split: id, title and point of the second slide (same kind, right after it).\n\
Use only row ids and figure/table ids that are listed. Never edit the cover or the thanks slide. Write titles and points in {language}. \
The deck should have {lo}–{hi} slides."
    );
    if !style.trim().is_empty() {
        system += &format!("\n\n{}", style.trim());
    }
    if !voice.trim().is_empty() {
        system += &format!("\n\n{}", voice.trim());
    }
    let mut u = format!("Paper: {}\n\n## Outline (id · kind · title · figures · point)\n", m.title);
    for r in rows(plan) {
        let figs: Vec<&str> = r["figs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        u += &format!("- {} · {} · {} · [{}] · {}\n", r["id"].as_str().unwrap_or(""), r["kind"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), figs.join(", "), clip(r["point"].as_str().unwrap_or(""), 160));
    }
    let used: BTreeSet<&str> = plan.slides.iter().flat_map(|s| [s.figure.as_str(), s.table.as_str()]).filter(|x| !x.is_empty()).collect();
    u += "\n## The paper's figures and tables (id: caption)\n";
    for f in m.figures.iter().chain(&m.tables) {
        u += &format!("- {}{}: {}\n", f.id, if used.contains(f.id.as_str()) { " (used)" } else { "" }, clip(&f.caption, 220));
    }
    if !m.abstract_text.is_empty() {
        u += &format!("\n## Abstract\n{}\n", clip(&m.abstract_text, 1500));
    }
    (system, u)
}

/// Only edits that can be applied: known ops, row ids of this plan, figure/table ids of the
/// material, roles; cover / thanks untouched; at most [`MAX`], no duplicates.
pub fn clean(list: Vec<Suggestion>, plan: &Plan, m: &Material) -> Vec<Suggestion> {
    let n = plan.slides.len();
    let row = |id: &str| id.strip_prefix('s').and_then(|x| x.parse::<usize>().ok()).filter(|&i| i >= 1 && i <= n);
    let fixed = |id: &str| row(id).is_some_and(|i| matches!(plan.slides[i - 1].role.as_str(), "cover" | "thanks"));
    let figs: BTreeSet<&str> = m.figures.iter().chain(&m.tables).map(|f| f.id.as_str()).collect();
    let fig_ok = |f: &Option<String>| f.as_deref().is_none_or(|f| f.is_empty() || figs.contains(f));
    let text = |t: &Option<String>| t.as_deref().is_some_and(|t| !t.trim().is_empty());
    let mut out: Vec<Suggestion> = vec![];
    for mut s in list {
        s.reason = s.reason.lines().next().unwrap_or("").trim().to_string();
        s.title = s.title.map(|t| super::plan::strip_typed_refs(t.trim()));
        s.point = s.point.map(|t| super::plan::strip_typed_refs(t.trim()));
        let to_ok = s.to.as_deref().is_none_or(|t| t.is_empty() || row(t).is_some());
        let ok = !s.reason.is_empty()
            && to_ok
            && fig_ok(&s.fig)
            && match s.op.as_str() {
                "add" => ROLES.contains(&s.kind.as_str()) && !matches!(s.kind.as_str(), "cover" | "thanks") && text(&s.title),
                "remove" => row(&s.id).is_some() && !fixed(&s.id),
                "move" => row(&s.id).is_some() && !fixed(&s.id) && s.to.is_some() && s.to.as_deref() != Some(s.id.as_str()),
                "retitle" => row(&s.id).is_some() && !fixed(&s.id) && text(&s.title),
                "point" => row(&s.id).is_some() && !fixed(&s.id) && text(&s.point),
                "figure" => row(&s.id).is_some() && !fixed(&s.id) && s.fig.is_some(),
                "merge" => row(&s.id).is_some() && row(&s.with).is_some() && s.id != s.with && !fixed(&s.id) && !fixed(&s.with),
                "split" => row(&s.id).is_some() && !fixed(&s.id) && text(&s.title),
                _ => false,
            };
        if !ok || out.contains(&s) {
            continue;
        }
        if s.op == "add" && s.to.is_none() {
            s.to = Some(String::new());
        }
        out.push(s);
        if out.len() == MAX {
            break;
        }
    }
    out
}

/// Ask for the suggestions (best-effort: `[]` on any failure). `on_chars`: streamed characters.
pub async fn suggest(llm: &Llm, plan: &Plan, m: &Material, length: &str, lang: &str, style: &str, voice: &str, engine: Option<&str>, on_chars: &(dyn Fn(u64) + Send + Sync)) -> Vec<Suggestion> {
    let (system, user) = prompt(plan, m, length, lang, style, voice);
    let req = LlmRequest::prompt(system, user).json(schema()).max_tokens(2000);
    match super::outline::ask_json(llm, crate::llm::TASK_OUTLINE_REVIEW, req, engine, on_chars).await {
        Ok((v, _)) => clean(serde_json::from_value(v["suggestions"].clone()).unwrap_or_default(), plan, m),
        Err(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::source::Fig;

    fn plan() -> (Plan, Material) {
        let ps = |role: &str, title: &str| PlanSlide { role: role.into(), title: title.into(), ..Default::default() };
        let plan = Plan { slides: vec![ps("cover", "T"), ps("background", "Why"), ps("method", "Encoder"), ps("thanks", "Thanks")], ..Default::default() };
        let m = Material { figures: vec![Fig { id: "fig1".into(), caption: "Overview".into(), ..Default::default() }], tables: vec![Fig { id: "tab1".into(), ..Default::default() }], ..Default::default() };
        (plan, m)
    }

    #[test]
    fn only_applicable_edits_are_kept() {
        let (p, m) = plan();
        let v: Vec<Suggestion> = serde_json::from_value(json!([
            { "op": "add", "to": "s3", "kind": "results", "title": "Main results", "point": "Ours wins", "fig": "tab1", "reason": "沒有結果頁" },
            { "op": "remove", "id": "s1", "reason": "封面不能刪" },
            { "op": "figure", "id": "s3", "fig": "fig9", "reason": "沒有這張圖" },
            { "op": "retitle", "id": "s2", "title": "Why grading is slow (Fig. 2)", "reason": "標題太短" },
            { "op": "merge", "id": "s2", "with": "s2", "reason": "自己" },
            { "op": "move", "id": "s9", "to": "", "reason": "沒有這頁" },
            { "op": "teleport", "id": "s2", "reason": "?" },
            { "op": "split", "id": "s3", "title": "Encoder, part 2", "point": "", "reason": "內容太多\n第二行" },
            { "op": "add", "to": "s3", "kind": "results", "title": "Main results", "point": "Ours wins", "fig": "tab1", "reason": "沒有結果頁" },
            { "op": "point", "id": "s2", "point": "Hand grading is slow", "reason": "重點不清楚" },
            { "op": "figure", "id": "s3", "fig": "fig1", "reason": "放架構圖" }
        ]))
        .unwrap();
        let out = clean(v, &p, &m);
        let ops: Vec<&str> = out.iter().map(|s| s.op.as_str()).collect();
        assert_eq!(ops, ["add", "retitle", "split", "point", "figure"]);
        assert_eq!(out[1].title.as_deref(), Some("Why grading is slow"), "typed references dropped");
        assert_eq!(out[2].reason, "內容太多");
    }

    #[test]
    fn prompt_lists_rows_and_unused_figures() {
        let (mut p, m) = plan();
        p.slides[2].figure = "fig1".into();
        let (sys, user) = prompt(&p, &m, "short", "en", "STYLE-SUMMARY", "");
        assert!(sys.contains("at most 5") && sys.contains("STYLE-SUMMARY") && sys.contains("9–11"));
        assert!(user.contains("- s3 · method · Encoder · [fig1]") && user.contains("- fig1 (used): Overview") && user.contains("- tab1: "), "{user}");
    }
}
