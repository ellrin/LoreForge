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
    /// The table row to emphasise: the text of its first cell (the proposed method's row).
    pub key_row: String,
    /// A chain the program draws as a flow diagram (lore-flow): the boxes' labels in order —
    /// steps, a before → now timeline, issue → solution.
    pub flow: Vec<String>,
    /// No picture of the paper fits this slide: which picture the presenter should put there
    /// (one short line). The build leaves an empty picture slot with it.
    pub suggest: String,
    /// The part of the main figure this slide is about (1-based, one of the parts a model
    /// confirmed, `figparts.rs`; 0 = none): the build shows that part cropped from the figure
    /// beside the lines. Not on a walk-through slide, which shows the whole figure.
    pub part: u32,
    /// On the training slide: the parts of the main figure that are trained (1-based, confirmed
    /// parts): the whole figure with those clear and the rest faint.
    pub trained: Vec<u32>,
    /// For each of `equations`, in order: the bullet (1-based) that explains it, 0 = none.
    /// The build sets each equation with its line.
    pub explains: Vec<u32>,
    pub cites: Vec<String>,
    pub notes: String,
}

impl PlanSlide {
    /// The bullet (0-based) that explains equation `i`, when the pairing is usable: a bullet
    /// of this slide, explaining this equation only.
    pub fn explained_by(&self, i: usize) -> Option<usize> {
        let b = *self.explains.get(i)? as usize;
        let once = self.explains.iter().take(self.equations.len()).filter(|x| **x as usize == b).count() == 1;
        (b >= 1 && b <= self.bullets.len() && i < self.equations.len() && once).then(|| b - 1)
    }

    /// Keep the equations `keep` accepts, each with its pairing.
    pub fn retain_equations(&mut self, keep: impl Fn(&str) -> bool) {
        let kept: Vec<bool> = self.equations.iter().map(|e| keep(e)).collect();
        let mut i = 0;
        self.explains.retain(|_| {
            i += 1;
            kept.get(i - 1).copied().unwrap_or(false)
        });
        let mut i = 0;
        self.equations.retain(|_| {
            i += 1;
            kept[i - 1]
        });
    }

    /// Keep the bullets `keep` accepts; the pairings follow their bullets (one whose bullet
    /// goes is unpaired).
    pub fn retain_bullets(&mut self, keep: impl Fn(&str) -> bool) {
        let kept: Vec<bool> = self.bullets.iter().map(|b| keep(b)).collect();
        if kept.iter().all(|k| *k) {
            return;
        }
        for x in &mut self.explains {
            let b = *x as usize;
            *x = if b >= 1 && b <= kept.len() && kept[b - 1] { kept[..b].iter().filter(|k| **k).count() as u32 } else { 0 };
        }
        let mut i = 0;
        self.bullets.retain(|_| {
            i += 1;
            kept[i - 1]
        });
    }

    /// After bullets or equations were cut from the end: no pairing past what is left.
    pub fn fix_explains(&mut self) {
        self.explains.truncate(self.equations.len());
        let n = self.bullets.len() as u32;
        self.explains.iter_mut().filter(|x| **x > n).for_each(|x| *x = 0);
    }

    /// The equations (0-based) that have bullets on the slide but no usable pairing.
    pub fn unpaired(&self) -> Vec<usize> {
        if self.bullets.is_empty() {
            return vec![];
        }
        (0..self.equations.len()).filter(|i| self.explained_by(*i).is_none()).collect()
    }
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
    pub const SUGGEST: usize = 90;
    /// A drawn chain: its boxes and the characters of one label.
    pub const FLOW: usize = 6;
    pub const FLOW_LABEL: usize = 34;
    pub const STEPS: usize = 6;
    pub const STEP: usize = 60;
    pub const KV: usize = 8;
    pub const TAKE_HOME: usize = 5;
    /// Display equations on one slide, and the bullets beside them.
    pub const EQUATIONS: usize = 2;
    pub const BULLETS_EQ: usize = 3;
    /// A walk-through slide of the main figure: its lines appear one by one over the faint side.
    pub const WALK_LINES: usize = 4;
    pub const WALK_LINE: usize = 60;
}

/// A slide that walks through one part of the main figure (`build.rs`).
/// Does the slide show something besides text: a figure, a table, key-value rows or an equation?
pub fn has_visual(s: &PlanSlide) -> bool {
    !s.figure.is_empty() || !s.table.is_empty() || !s.kv.is_empty() || !s.equations.is_empty() || s.flow.len() >= 2 || s.part >= 1 || !s.trained.is_empty()
}

/// Only these may be text alone: every other slide argues from something it shows (the
/// author, 2026-11-20: a page of bullets reads as undigested).
pub fn text_only_ok(role: &str) -> bool {
    matches!(role, "cover" | "thanks" | "takeaway")
}

pub fn is_walk(s: &PlanSlide) -> bool {
    s.role == "method" && s.module >= 1 && !s.figure.is_empty()
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
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (i, s) in p.slides.iter().enumerate() {
        let at = format!("Slide {} (\"{}\")", i + 1, clip(&s.title, 40));
        if !text_only_ok(&s.role) && !seen.insert(s.title.trim().to_lowercase()) {
            out.push(format!("{at}: another slide already has this title. Each title says what its own slide tells."));
        }
        if !matches!(s.role.as_str(), "cover") && chars(&s.title) > limit::TITLE {
            out.push(format!("{at}: the title has {} characters, at most {}.", chars(&s.title), limit::TITLE));
        }
        let visual = !s.figure.is_empty() || !s.table.is_empty() || !s.kv.is_empty() || !s.suggest.is_empty() || !s.flow.is_empty() || s.part >= 1 || !s.trained.is_empty();
        if !text_only_ok(&s.role) {
            if !has_visual(s) && s.suggest.trim().is_empty() {
                out.push(format!("{at}: it shows nothing but text. Give it a figure, table or equation id it argues from, or `suggest` = which picture the presenter should put there."));
            }
            if s.takeaway.trim().is_empty() && needs_takeaway(&s.role) {
                out.push(format!("{at}: it has no takeaway. A results or ablation slide ends in the one conclusion its lines lead to."));
            }
        }
        for b in s.bullets.iter().chain(&s.steps).chain(std::iter::once(&s.takeaway)).filter(|b| reads_symbols(b)) {
            out.push(format!("{at}: the line \"{}\" only reads the symbols again. Leave it off the slide and say what the symbols mean in the notes.", clip(b, 40)));
        }
        if !text_only_ok(&s.role) && !s.notes.trim().is_empty() && !notes_shape_ok(&s.notes) {
            out.push(format!("{at}: the notes must be one opening sentence (at most one comma) saying what the slide covers, then a numbered list, one item per line (\"1. …\")."));
        }
        if needs_reading_notes(s) && s.notes.trim().is_empty() {
            out.push(format!("{at}: it shows a table or a chart but has no notes. The notes say what each row is, or how to read the chart, then one conclusion per panel."));
        }
        if s.flow.len() == 1 || s.flow.len() > limit::FLOW || s.flow.iter().any(|x| chars(x) > limit::FLOW_LABEL) {
            out.push(format!("{at}: `flow` is 2 to {} boxes of at most {} characters each.", limit::FLOW, limit::FLOW_LABEL));
        }
        if chars(&s.suggest) > limit::SUGGEST {
            out.push(format!("{at}: `suggest` has {} characters, at most {}.", chars(&s.suggest), limit::SUGGEST));
        }
        let (nb, nc) = if s.role == "takeaway" { (limit::TAKE_HOME, limit::BULLET) } else if is_walk(s) { (limit::WALK_LINES, limit::WALK_LINE) } else if visual { (limit::BULLETS_VISUAL, limit::BULLET_VISUAL) } else { (limit::BULLETS, limit::BULLET) };
        if s.bullets.len() > nb {
            out.push(format!("{at}: {} bullets, at most {nb}{}; cut or move some to another slide.", s.bullets.len(), if is_walk(s) { " on a walk-through slide" } else if visual { " next to a figure or table" } else { "" }));
        }
        if s.equations.len() > limit::EQUATIONS || (!s.equations.is_empty() && s.bullets.len() > limit::BULLETS_EQ) {
            out.push(format!("{at}: {} equations with {} bullets; at most {} equations and {} bullets on one slide — make a second slide.", s.equations.len(), s.bullets.len(), limit::EQUATIONS, limit::BULLETS_EQ));
        }
        if !s.unpaired().is_empty() {
            out.push(format!("{at}: `explains` must name, for each equation in order, the number (1-based) of the bullet that explains it, each bullet for one equation only; it has {:?} for {} equations and {} bullets.", s.explains, s.equations.len(), s.bullets.len()));
        }
        if !s.trained.is_empty() && !s.kv.is_empty() {
            out.push(format!("{at}: a training slide with `trained` shows the figure, so it carries no kv rows; put the settings on a second training slide."));
        }
        if is_walk(s) && !s.equations.is_empty() {
            out.push(format!("{at}: a walk-through slide carries no equations; move them to a method slide with module 0 or to the loss slide."));
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
        let visual = !s.figure.is_empty() || !s.table.is_empty() || !s.kv.is_empty() || !s.suggest.is_empty() || !text_only_ok(&s.role);
        // the trained parts take the figure's room: the settings table goes to a slide of its own
        let mut s = s;
        let mut settings = None;
        if !s.trained.is_empty() && !s.kv.is_empty() {
            notes.push(format!("「{}」同時有訓練的段落與設定表，一頁放不下：設定表拆到下一頁", clip(&s.title, 30)));
            settings = Some(PlanSlide { role: s.role.clone(), title: format!("{} (2)", s.title), kv: std::mem::take(&mut s.kv), ..Default::default() });
        }
        if !text_only_ok(&s.role) && !has_visual(&s) && s.suggest.trim().is_empty() {
            notes.push(format!("「{}」沒有圖表可以對應，留了一個空的圖框，請放一張圖", clip(&s.title, 30)));
        }
        if s.takeaway.trim().is_empty() && needs_takeaway(&s.role) {
            notes.push(format!("「{}」沒有結論句", clip(&s.title, 30)));
        }
        if s.bullets.iter().chain(&s.steps).chain(std::iter::once(&s.takeaway)).any(|b| reads_symbols(b)) {
            notes.push(format!("「{}」有一行只是在念符號（where … denotes），請移到講者備註", clip(&s.title, 30)));
        }
        if !text_only_ok(&s.role) && !s.notes.trim().is_empty() && !notes_shape_ok(&s.notes) {
            notes.push(format!("「{}」的講者備註不是「一句開場＋編號條列」", clip(&s.title, 30)));
        }
        if needs_reading_notes(&s) && s.notes.trim().is_empty() {
            notes.push(format!("「{}」有表或圖表，但講者備註沒有說明每一列或怎麼看", clip(&s.title, 30)));
        }
        let nb = if s.role == "takeaway" { limit::TAKE_HOME } else if is_walk(&s) { limit::WALK_LINES } else if visual || !s.equations.is_empty() { limit::BULLETS_VISUAL.min(limit::BULLETS_EQ) } else { limit::BULLETS };
        let nc = if s.role == "takeaway" { limit::BULLET } else if is_walk(&s) { limit::WALK_LINE } else if visual { limit::BULLET_VISUAL } else { limit::BULLET };
        let long = s.bullets.iter().filter(|b| chars(b) > nc).count();
        if long > 0 {
            notes.push(format!("「{}」有 {long} 行超過 {nc} 字，沒有縮字，請自行刪短", clip(&s.title, 30)));
        }
        if chars(&s.title) > limit::TITLE && s.role != "cover" {
            notes.push(format!("標題「{}」超過 {} 字，可能換行", clip(&s.title, 30), limit::TITLE));
        }
        let many_eq = s.equations.len() > limit::EQUATIONS;
        if (s.bullets.len() > nb || many_eq) && !matches!(s.role.as_str(), "cover" | "thanks") {
            let mut first = s.clone();
            let rest: Vec<String> = first.bullets.split_off(nb.min(first.bullets.len()));
            let rest_eq: Vec<String> = first.equations.split_off(limit::EQUATIONS.min(first.equations.len()));
            // the pairings of what stays (a continuation slide's equations are unpaired: said at the build)
            first.fix_explains();
            notes.push(format!("「{}」有 {} 行{}，一頁放不下：拆成兩頁", clip(&s.title, 30), s.bullets.len(), if s.equations.is_empty() { String::new() } else { format!("與 {} 條方程式", s.equations.len()) }));
            // a walk-through slide's second half stays on the same part of the figure
            first.takeaway = String::new();
            out.push(first);
            // as many more slides as the lines need; the last one carries the conclusion
            let mut rest = rest;
            let mut rest_eq = rest_eq;
            loop {
                let later: Vec<String> = rest.split_off(nb.min(rest.len()));
                let later_eq: Vec<String> = rest_eq.split_off(limit::EQUATIONS.min(rest_eq.len()));
                let last = later.is_empty() && later_eq.is_empty();
                out.push(PlanSlide {
                    role: s.role.clone(),
                    title: format!("{} ({})", s.title, out.iter().filter(|o: &&PlanSlide| o.title == s.title || o.title.starts_with(&format!("{} (", s.title))).count() + 1),
                    bullets: std::mem::replace(&mut rest, later),
                    equations: std::mem::replace(&mut rest_eq, later_eq),
                    figure: if is_walk(&s) { s.figure.clone() } else { String::new() },
                    module: if is_walk(&s) { s.module } else { 0 },
                    part: s.part,
                    suggest: s.suggest.clone(),
                    takeaway: if last { s.takeaway.clone() } else { String::new() },
                    ..Default::default()
                });
                if last {
                    break;
                }
            }
        } else {
            out.push(s);
        }
        out.extend(settings);
    }
    p.slides = out;
    notes
}

/// The parts a slide may crop or mark: the main figure's, when a model looked at the figure
/// and cut it (an equal split is a guessed box, never cropped).
fn confirmed(parts: Option<&super::figparts::FigureParts>) -> usize {
    parts.filter(|p| p.walk() && p.by == "model").map_or(0, |p| p.parts.len())
}

/// What `part` and `trained` name that the main figure's confirmed parts do not have, one
/// English line each (for the corrective request, like [`check_plan`]).
pub fn part_problems(p: &Plan, parts: Option<&super::figparts::FigureParts>) -> Vec<String> {
    let n = confirmed(parts);
    let mut out = vec![];
    for (i, s) in p.slides.iter().enumerate() {
        let at = format!("Slide {} (\"{}\")", i + 1, clip(&s.title, 40));
        if s.part as usize > n {
            out.push(format!("{at}: part = {} but the main figure has {n} parts; use one of them or part = 0.", s.part));
        }
        if s.part >= 1 && !s.figure.is_empty() {
            out.push(format!("{at}: a slide with `part` shows that part of the main figure; leave figure = \"\"."));
        }
        if s.trained.iter().any(|t| *t as usize > n) {
            out.push(format!("{at}: trained = {:?} but the main figure has {n} parts.", s.trained));
        }
    }
    out
}

/// `part` and `trained` against the parts the build will get: what names no confirmed part
/// is taken off (the slide keeps its usual layout) with one report line each (繁中), never
/// silently. `recut`: the figure was cut again after the plan was written (方法拆幾個模組 or
/// another main figure), so the plan's numbers no longer name the parts it meant.
pub fn settle_parts(p: &mut Plan, parts: Option<&super::figparts::FigureParts>, recut: bool) -> Vec<String> {
    let n = confirmed(parts);
    let guessed = parts.is_some_and(|x| x.walk() && x.by != "model");
    let mut notes = vec![];
    let any_trained = p.slides.iter().any(|x| x.role == "training" && !x.trained.is_empty());
    let mut asked = false;
    for s in &mut p.slides {
        let title = clip(&s.title, 30);
        let why = |k: u32| {
            if recut {
                "架構圖重新分段了，原本指定的段落不再對得上".to_string()
            } else if guessed {
                "架構圖是平均切的，不是模型確認的範圍".to_string()
            } else if n == 0 {
                "架構圖沒有分段".to_string()
            } else {
                format!("架構圖沒有第 {k} 段（共 {n} 段）")
            }
        };
        if s.part >= 1 {
            if !s.figure.is_empty() {
                notes.push(format!("「{title}」已經有自己的圖，沒有改放架構圖的第 {} 段", s.part));
                s.part = 0;
            } else if recut || s.part as usize > n {
                notes.push(format!("「{title}」：{}，這頁沒有放裁切的圖，照原本的版面", why(s.part)));
                s.part = 0;
            }
        }
        if s.role != "training" {
            continue;
        }
        if !s.trained.is_empty() {
            let bad = s.trained.iter().copied().find(|t| *t as usize > n);
            if recut || bad.is_some() {
                notes.push(format!("「{title}」：{}，沒有標出訓練的段落，照原本的版面", why(bad.unwrap_or(0))));
                s.trained.clear();
            } else if s.trained.len() >= n {
                notes.push(format!("「{title}」：架構圖的每一段都訓練，沒有可以標示的差別，照原本的版面"));
                s.trained.clear();
            } else if !s.figure.is_empty() || !s.table.is_empty() {
                notes.push(format!("「{title}」已經有自己的圖表，沒有標出訓練的段落"));
                s.trained.clear();
            }
        } else if n >= 2 && !any_trained && !asked {
            asked = true;
            notes.push(format!("「{title}」沒有指出訓練的是架構圖的哪一段，照原本的版面"));
        }
    }
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
                    "kv": { "type": "array", "items": list }, "key_row": s, "flow": list, "suggest": s,
                    "part": { "type": "integer" }, "trained": { "type": "array", "items": { "type": "integer" } }, "explains": { "type": "array", "items": { "type": "integer" } },
                    "cites": list, "notes": s
                }
            } }
        }
    })
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n).collect::<String>() + "…" }
}

/// What the planner is told when the paper's pictures go along with the request.
/// The language of the speaker notes (備忘稿), which need not be the slides' language.
pub fn notes_rule(notes_lang: &str) -> String {
    let l = if notes_lang == "en" { "English" } else { "Traditional Chinese as used in Taiwan (technical and medical terms, model names and metrics stay in English)" };
    format!("\n- Write every `notes` in {l}, whatever the language of the slide text.")
}

/// How the speaker notes are written (any presenter's good practice; the presenter's own
/// sentence patterns come from the private notes guide, `crate::deckvoice::notes_guide`).
pub const NOTES_SHAPE: &str = "- notes: what the presenter says on this slide (speaker notes): the explanation the slide's few words leave out, never a reading of the slide. \
Shape: one opening sentence with at most one comma that says what this slide covers, then a numbered list, one item per line (\"1. …\", \"2. …\"), each item at most about four sentences. \
A definition goes in this order: which observed values it comes from, how they are split or combined in everyday words, only then the name of the quantity. \
Never use a term before it has been explained, and never explain jargon with more jargon. \
An overview slide's notes carry no numbers; a detail slide's may. A table slide's notes say what each row is (not every number); a chart slide's notes say how to read it \
(axes, lines, colours, bands), then one conclusion per panel. The meaning of the symbols on the slide is said here. \
When the slide answers why something is defined this way, the notes give the reason, not the formula again and not a summary of the paper.";

/// A slide line that only reads symbols again ("where $x$ denotes …"): its place is the notes.
pub fn reads_symbols(line: &str) -> bool {
    let re = regex::Regex::new(r"(?i)^\s*where\b.*\b(?:denotes?|represents?|stands? for|is the|are the)\b|^\s*其中.*(?:表示|代表)").unwrap();
    re.is_match(line)
}

/// Speaker notes in the asked shape: one opening line (no list mark, at most one comma outside
/// math), then numbered lines. Empty notes are not judged here.
pub fn notes_shape_ok(notes: &str) -> bool {
    let lines: Vec<&str> = notes.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let Some(first) = lines.first() else { return true };
    let item = regex::Regex::new(r"^\(?\d{1,2}[.)、．]").unwrap();
    let plain: String = first.split('$').step_by(2).collect();
    let commas = plain.chars().filter(|c| matches!(c, ',' | '，')).count();
    !item.is_match(first) && commas <= 1 && lines.len() >= 2 && lines[1..].iter().all(|l| item.is_match(l))
}

/// A slide whose notes must explain what it shows: a table, or a figure on a results or
/// ablation slide (a chart).
fn needs_reading_notes(s: &PlanSlide) -> bool {
    !s.table.is_empty() || (!s.figure.is_empty() && matches!(s.role.as_str(), "results" | "ablation"))
}

/// Only these must carry a takeaway (the author, after a rehearsal: the key line is not
/// needed on every page).
fn needs_takeaway(role: &str) -> bool {
    matches!(role, "results" | "ablation")
}

pub const PICTURES_RULE: &str = "\n- The paper's figures (and the tables that are shown as pictures) are attached as images, in the order of the list at the end: \
look at each one before you decide where it goes, and put an analysis or descriptor figure on the slide that talks about what it shows.";

/// The pictures the planner looks at: every figure with a picture and every table that is
/// shown as a picture, as (id, kind, picture), at most 10.
pub fn pictures(m: &Material) -> Vec<(&str, &'static str, &super::source::Img)> {
    let figs = m.figures.iter().filter_map(|f| Some((f.id.as_str(), "figure", f.image.as_ref()?)));
    let tabs = m.tables.iter().filter(|t| t.grid.as_ref().is_none_or(|g| g.is_empty())).filter_map(|t| Some((t.id.as_str(), "table", t.image.as_ref()?)));
    figs.chain(tabs).take(10).collect()
}

pub fn prompt(m: &Material, length: &str, lang: &str, parts: Option<&super::figparts::FigureParts>) -> (String, String) {
    let (lo, hi) = target_slides(length);
    let language = if lang == "en" { "English" } else { "Traditional Chinese as used in Taiwan (keep technical terms, model names and metrics in English)" };
    let (lt, lb, lbc, lv, lvc, ltk, ls, lf, lfc) = (limit::TITLE, limit::BULLETS, limit::BULLET, limit::BULLETS_VISUAL, limit::BULLET_VISUAL, limit::TAKEAWAY, limit::SUGGEST, limit::FLOW, limit::FLOW_LABEL);
    // how the method is shown depends on what a model saw in the main figure (`figparts.rs`)
    let (wl, wc) = (limit::WALK_LINES, limit::WALK_LINE);
    let method = match parts.filter(|p| !p.figure.is_empty()) {
        Some(p) if p.walk() => {
            let list: Vec<String> = p.parts.iter().enumerate().map(|(i, x)| format!("{}. {}{}", i + 1, x.label, if x.about.is_empty() { String::new() } else { format!(" ({})", x.about) })).collect();
            format!(
                "- overview: the big picture — figure = \"{f}\", shown whole across the slide with its caption; bullets = [] (notes carry what the presenter says).\n\
- method, the walk-through: exactly {n} slides, one for each part of that figure, in this order: {parts}. For each: module = the part's number, figure = \"{f}\", \
2–{wl} bullets of at most {wc} characters that explain this part only (the part is shown clearly, the rest of the figure faint, and the bullets appear one by one beside it); equations = [].\n\
- method, details (optional, after the walk-through): module = 0, figure = \"\"; equations = ids of the method's equations, bullets explain them. \
When such a slide is about one part of that figure, part = the part's number: the program crops that part from the figure and sets it beside the bullets (never the whole figure small). \
Only these {n} parts exist; part = 0 when the slide is about no single part.\n\
- training, with this figure: trained = the numbers of the parts whose weights are trained (the program shows the figure with those parts clear and the rest faint, the bullets under it); \
[] only when the paper does not say. A training slide with trained has kv = []: put the settings on a second training slide.\n",
                f = p.figure,
                n = p.parts.len(),
                parts = list.join("; ")
            )
        }
        Some(p) => format!(
            "- overview: the big picture — figure = \"{f}\", shown whole across the slide with its caption; at most 2 short bullets.\n\
- method (1–3): module = 0 and figure = \"\" (the figure is shown once, on the overview); equations = ids of the method's equations; bullets explain the steps.\n",
            f = p.figure
        ),
        None => "- overview: the big picture — figure = the main architecture figure when the paper has one; at most 2 short bullets.\n\
- method (1–3): module = 0 and figure = \"\"; equations = ids of the method's equations; bullets explain the steps.\n"
            .to_string(),
    };
    // with confirmed parts of the main figure, a slide about one of them takes `part`
    let part_hint = if parts.is_some_and(|p| p.walk() && !p.figure.is_empty()) { " For a part of the main figure set part instead (see the method lines above): the program crops it." } else { "" };
    let system = format!(
        "You plan a paper-report seminar deck about one research paper, the way the presenter always builds them. \
A program lays out the slides from your plan: you decide WHAT each slide says and which of the paper's figures, tables and equations it shows, never where things go.\n\n\
Reply with one JSON object {{venue, keywords, slides}}. Between {lo} and {hi} slides in total (cover and thanks included), in this order (skip a role the paper has no material for, repeat method / results when needed):\n\
- cover: the paper title as title; venue = journal or conference and year if the text says so (else empty); keywords = 3–6.\n\
- background (1–2): the field or clinical context.\n\
- challenge: the problem and the intuition.\n\
{method}\
- loss: the training objective; equations = the loss equation ids.\n\
- training: bullets say which part of the pipeline is trained, what stays fixed, and which loss trains it; kv = [[key, value], …] rows (data, model, optimiser, learning rate, epochs, batch size, hardware…), only values the paper states.\n\
- results: table = the main results table id (or figure); key_row = the first-column text of the row to emphasise (the proposed method), exactly as in the table; one bullet says what the rows are (each compared setting: which model or variant, set up how); takeaway = one sentence reading the result.\n\
- ablation: table or figure id; one bullet says what each row or variant changes; takeaway = one sentence.\n\
- visual: qualitative figure id; bullets about what it shows, including failure cases if any.\n\
- opinion: title \"Some opinions\"; the presenter's critical view (strengths, limits, questions).\n\
- takeaway: title \"Take home messages\"; 3–5 messages in bullets.\n\
- thanks: title \"Thanks\".\n\n\
Rules:\n\
- A title says what THIS slide tells — its subject and the angle taken (\"Why frame-level models fail on full recordings\", \"Depth separates obstructed from patent\"), at most {lt} characters. \
Every title is used once in the deck: two slides never share one, and a walk-through slide's title names its part and what that part does. No agenda slide.\n\
- Every slide uses the same text size and nothing is shrunk to fit, so the limits are hard: at most {lb} bullets of at most {lbc} characters each; \
on a slide with a figure, a table or kv rows at most {lv} bullets of at most {lvc} characters; on a slide with equations at most 2 equations and 3 bullets; a takeaway of at most {ltk} characters. \
When a point needs more room, cut words or make a second slide — never go over.\n\
- Every slide except cover, takeaway and thanks argues from something it SHOWS: a figure, table or equation id. When the paper has nothing that fits the slide (background, challenge, opinion…), draw it: \
flow = 2 to {lf} short box labels in order (at most {lfc} characters each), which the program draws as a chain of boxes joined by arrows — the steps of a procedure, a timeline \
(what was done before → what changed → what this paper does; start a label with the year when the text gives one), or issue → cause → solution. Prefer flow whenever the slide's point is a sequence or a contrast in time. \
Only when a diagram cannot stand for the picture (a photo, a plot) set suggest = one line of at most {ls} characters naming the picture the presenter should put there. \
For each slide first weigh which of the three serves its point best — something the paper shows, a drawn flow, or a slot for the presenter's own picture; they are three formats, not a ranking of fallbacks, and a deck normally mixes them. \
With flow or suggest keep to at most {lv} bullets. Never a slide of bullets alone.\n\
- A picture earns its space: put a figure on a slide only when the bullets explain what it shows, beside it. When only one part of a large figure matters to the slide, \
do not show the whole figure small: draw that part as a flow, or suggest = the part to crop.{part_hint}\n\
- One thing per slide: a slide explains one idea (one input, one function, one rule, one group of results). When the paper defines several inputs, a function and several rules, \
give them their own slides instead of packing them together. Spread the content evenly: no slide crammed while the next is nearly empty.\n\
- Each input quantity or variable is its own bullet: its plain name and a definition in everyday words (what it is measured or computed from), the symbol only after that. \
Never a bullet of symbols or jargon alone.\n\
- Abbreviated states, classes or categories: one bullet (or kv row) per item, with its full name, its formula or rule, and the reason that item maps to that rule.\n\
- No \"where $x$ denotes …\" line on a slide: the meaning of the symbols goes in the notes.\n\
- Each equation stays with its own explanation: one equation per slide, unless the same bullets explain two; then the bullets follow the equations' order. \
On a slide with equations and bullets, explains = for each equation in order the number (1-based) of the bullet that explains it, each bullet for one equation: the program sets every equation with its line.\n\
- A table: one bullet names what the rows are (each compared setting). A chart (a plot of results): one bullet says how to read it (what each panel shows, \
what the axes, lines, colours and bands mean), then one conclusion per panel.\n\
- takeaway: the ONE conclusion of a slide, a full sentence a listener can repeat. Results and ablation slides always have one; any other slide has one only when it reaches \
a conclusion worth repeating, and a detail slide (one input, one definition, one step) may leave it empty, its title already says its point. Each bullet is a step toward the slide's point \
and says what to look at in what the slide shows (a number, a trend, a part of the picture). Never a summary of what a section of the paper says, never a list of what the authors did. \
A bullet that does not lead to the slide's point is cut.\n\
- Bullets: short, spoken. Inline math between $ signs as LaTeX. Never start a bullet with a number, a dash or a bullet sign: the program numbers and marks them.\n\
- Never write figure, table or equation numbers (no \"Fig. 3\", \"Table 2\", \"Eq. (1)\"): the slide shows the item itself.\n\
- figure / table / equations use ONLY ids from the material lists that are not marked as unavailable; \"\" or [] when none.\n\
- A figure or table appears on ONE slide, the slide that talks about it (only the main figure repeats, on the overview and its walk-through slides, and as a part or with the trained parts).\n\
- cites: bibliography keys from the list for background claims from other work, at most 2 per slide.\n\
{NOTES_SHAPE}\n\
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

/// The presenter's brief (微調's 補充: what only the presenter can say, one line each) as a block
/// at the end of the planner's user text. An empty brief gives `""`: the request is unchanged.
pub fn brief_block(brief: &[String]) -> String {
    let lines: Vec<String> = brief.iter().map(|b| b.trim()).filter(|b| !b.is_empty()).map(|b| format!("- {b}")).collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "\n\n## What only the presenter can say\nThe presenter adds these points, which the paper does not contain. Put each one on the slide where it belongs \
(earlier work of the presenter on a background or opinion slide), in the bullets, the takeaway or the notes, keeping what it says; never drop one:\n{}",
        lines.join("\n")
    )
}

/// Words of a line for the brief check: latin words of 3+ letters and CJK character pairs.
fn brief_tokens(s: &str) -> BTreeSet<String> {
    crate::deckvoice::tokens(&s.replace('$', " "))
}

/// The brief lines no slide carries. A line is carried when one slide's bullets, takeaway and
/// notes together hold at least half of the line's words (latin words of 3+ letters, CJK
/// character pairs, `deckvoice::tokens`); a line without such words counts as carried.
pub fn brief_missing(p: &Plan, brief: &[String]) -> Vec<String> {
    let slides: Vec<BTreeSet<String>> = p.slides.iter().map(|s| brief_tokens(&format!("{} {} {}", s.bullets.join(" "), s.takeaway, s.notes))).collect();
    brief
        .iter()
        .map(|b| b.trim())
        .filter(|b| !b.is_empty())
        .filter(|b| {
            let want = brief_tokens(b);
            !want.is_empty() && !slides.iter().any(|have| want.intersection(have).count() * 2 >= want.len())
        })
        .map(String::from)
        .collect()
}

/// [`brief_missing`] as lines of the corrective request (English, like [`check_plan`]'s).
pub fn brief_problems(p: &Plan, brief: &[String]) -> Vec<String> {
    brief_missing(p, brief).iter().map(|b| format!("The presenter's point \"{}\" is on no slide: put it in the bullets, takeaway or notes of the slide where it belongs.", clip(b, 80))).collect()
}

/// The job's warning for a brief line the final plan does not carry.
pub fn brief_warning(line: &str) -> String {
    format!("補充的「{}」沒有放進大綱", clip(line, 40))
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
        s.retain_equations(|id| m.equations.iter().find(|e| e.id == id).is_some_and(|e| e.usable() || e.image.is_some()));
        if s.equations.len() < before {
            notes.push(format!("「{title}」有 {} 條方程式讀不出來，沒有放", before - s.equations.len()));
        }
    }
    notes
}

/// A native table takes its rows' height: under a tall one only so many lines fit at the
/// body size. The lines over that go to the slide's speaker notes (said so) — nothing is
/// shrunk and nothing overprints the takeaway. Rows are counted at about 5 % of the slide
/// height each, a line of text at about 6 %, the body area at about 68 %.
pub fn fit_under_tables(p: &mut Plan, m: &Material) -> Vec<String> {
    let mut notes = vec![];
    for s in &mut p.slides {
        let rows = if !s.kv.is_empty() { s.kv.len() } else { m.tables.iter().find(|t| t.id == s.table).and_then(|t| t.grid.as_ref()).map_or(0, Vec::len) };
        if rows == 0 || s.bullets.is_empty() {
            continue;
        }
        let take = if s.takeaway.trim().is_empty() { 0.0 } else { 11.0 };
        let room = ((68.0 - 5.2 * rows as f64 - take - 3.0) / 6.0).floor().max(0.0) as usize;
        if s.bullets.len() > room {
            let moved = s.bullets.split_off(room);
            s.fix_explains();
            notes.push(format!("「{}」的表格有 {rows} 列，下面只放得下 {room} 行：其餘 {} 行移到講者備註", clip(&s.title, 30), moved.len()));
            s.notes = [s.notes.trim().to_string(), moved.join("\n")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n");
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
        s.explains.truncate(s.equations.len());
        s.retain_equations(|e| eqs.contains(e));
        s.cites.retain(|c| m.refs.contains_key(c));
        s.title = strip_typed_refs(&s.title);
        s.takeaway = strip_typed_refs(&s.takeaway);
        for b in s.bullets.iter_mut().chain(s.steps.iter_mut()) {
            *b = strip_lead(&strip_typed_refs(b));
        }
        s.takeaway = strip_lead(&s.takeaway);
        s.retain_bullets(|b| !b.is_empty());
        s.fix_explains();
        // the trained parts belong to the training slide; a part to a slide that is not itself
        // the overview or a walk-through of the figure
        if s.role != "training" {
            s.trained.clear();
        }
        s.trained.retain(|t| *t >= 1);
        s.trained.sort_unstable();
        s.trained.dedup();
        if matches!(s.role.as_str(), "cover" | "thanks" | "takeaway" | "overview") || (s.role == "method" && s.module >= 1) {
            s.part = 0;
        }
        s.kv.retain(|r| r.len() >= 2 && !r[0].trim().is_empty());
    }
    if p.slides.first().map(|s| s.role.as_str()) != Some("cover") {
        p.slides.insert(0, PlanSlide { role: "cover".into(), title: m.title.clone(), ..Default::default() });
    }
    if p.slides.last().map(|s| s.role.as_str()) != Some("thanks") {
        p.slides.push(PlanSlide { role: "thanks".into(), title: "Thanks".into(), ..Default::default() });
    }
    // a walk-through slide (module ≥ 1) shows the big picture; any other method slide has no
    // figure of its own unless the plan gave it a different one
    let main = p.slides.iter().find(|s| s.role == "overview" && !s.figure.is_empty()).map(|s| s.figure.clone());
    if let Some(f) = main {
        for s in p.slides.iter_mut().filter(|s| s.role == "method") {
            if s.module >= 1 {
                s.figure = f.clone();
            } else if s.figure == f {
                s.figure.clear();
            }
        }
        // a part of the main figure, or its trained parts: the field names the figure already
        for s in p.slides.iter_mut().filter(|s| (s.part >= 1 || !s.trained.is_empty()) && s.figure == f) {
            s.figure.clear();
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
        let ps = |role: &str, n: usize| PlanSlide {
            role: role.into(),
            title: format!("{role} slide"),
            bullets: (0..n).map(|i| format!("Point number {i}")).collect(),
            suggest: if text_only_ok(role) { String::new() } else { "a picture of it".into() },
            takeaway: if text_only_ok(role) { String::new() } else { "So this follows".into() },
            ..Default::default()
        };
        // a page of bullets alone, or one without a conclusion, is sent back
        let bare = Plan { slides: (0..12).map(|_| PlanSlide { role: "background".into(), title: "T".into(), bullets: vec!["One".into()], ..Default::default() }).collect(), ..Default::default() };
        let why = check_plan(&bare, "standard");
        assert!(why.iter().any(|x| x.contains("nothing but text")) && !why.iter().any(|x| x.contains("no takeaway")), "a detail slide may have no key line: {why:?}");
        let mut res = bare.clone();
        for s in &mut res.slides {
            s.role = "results".into();
        }
        assert!(check_plan(&res, "standard").iter().any(|x| x.contains("no takeaway")), "a results slide still needs its conclusion");
        assert!(check_plan(&Plan { slides: vec![PlanSlide { role: "takeaway".into(), title: "Take home messages".into(), bullets: vec!["One".into()], ..Default::default() }; 12], ..Default::default() }, "standard").is_empty(), "the take-home page may be text alone");
        let mut p = Plan { slides: vec![ps("cover", 0), ps("background", 7), ps("method", 2), ps("thanks", 0)], ..Default::default() };
        p.slides[2].figure = "fig1".into();
        p.slides[2].bullets[0] = "x".repeat(limit::BULLET_VISUAL + 1);
        let problems = check_plan(&p, "standard");
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("4 slides") && problems[0].contains("between 10 and 15"));
        assert!(problems[1].contains("7 bullets") && problems[2].contains("shorten it"));
        let notes = enforce_limits(&mut p);
        assert_eq!(p.slides.iter().map(|s| s.bullets.len()).collect::<Vec<_>>(), [0, 3, 3, 1, 2, 0], "the bullets over the count go to more slides of the same kind");
        assert_eq!((p.slides[1].title.as_str(), p.slides[2].title.as_str(), p.slides[2].role.as_str()), ("background slide", "background slide (2)", "background"), "a continuation is numbered: no two slides share a title");
        assert_eq!((p.slides[1].takeaway.as_str(), p.slides[3].takeaway.as_str(), p.slides[3].suggest.as_str()), ("", "So this follows", "a picture of it"), "the conclusion closes the last of them; each keeps its picture slot");
        assert_eq!(notes.len(), 2, "the split and the long line are reported: {notes:?}");
        let twelve = |role: &str| (0..12).map(|i| PlanSlide { title: format!("{role} {i}"), ..ps(role, 3) }).collect::<Vec<_>>();
        assert!(check_plan(&Plan { slides: twelve("background"), ..Default::default() }, "standard").is_empty());
        // the same title twice is sent back: a title says what its own slide tells
        assert!(check_plan(&Plan { slides: (0..12).map(|_| ps("background", 3)).collect(), ..Default::default() }, "standard").iter().any(|x| x.contains("already has this title")));
        // equations: two a slide with three bullets; the rest goes to a second slide
        let mut q = Plan { slides: vec![PlanSlide { role: "method".into(), title: "Details".into(), bullets: (0..5).map(|i| format!("Line {i}")).collect(), equations: vec!["eq1".into(), "eq2".into(), "eq3".into()], ..Default::default() }], ..Default::default() };
        assert!(check_plan(&q, "short").iter().any(|x| x.contains("3 equations")));
        enforce_limits(&mut q);
        assert_eq!(q.slides.iter().map(|s| (s.bullets.len(), s.equations.len())).collect::<Vec<_>>(), [(3, 2), (2, 1)]);
        // a tall table leaves room for fewer lines: the others go to the notes, said so
        let m = Material { tables: vec![crate::todeck::source::Fig { id: "tab1".into(), grid: Some(vec![vec!["a".to_string(), "b".into()]; 8]), ..Default::default() }], ..Default::default() };
        let mut r = Plan { slides: vec![PlanSlide { role: "results".into(), title: "Results".into(), table: "tab1".into(), bullets: vec!["One".into(), "Two".into(), "Three".into()], takeaway: "Ours wins".into(), notes: "Say this.".into(), ..Default::default() }], ..Default::default() };
        let notes = fit_under_tables(&mut r, &m);
        assert_eq!((r.slides[0].bullets.len(), r.slides[0].notes.as_str(), notes.len()), (2, "Say this.\nThree", 1));
    }

    #[test]
    fn symbol_readings_note_shape_and_unexplained_tables_are_sent_back_then_reported() {
        assert!(reads_symbols("where $x$ denotes the input") && reads_symbols("Where N represents the count") && reads_symbols("其中 $x$ 表示輸入"));
        assert!(!reads_symbols("Where the model fails: small lesions") && !reads_symbols("With two stages, the model is the best") && !reads_symbols("Where $x$ is large, the loss saturates"));
        assert!(notes_shape_ok("This page covers the inputs.\n1. The first input comes from the scan, split by side.\n2) The second, $f(a,b,c)$, combines both."));
        assert!(notes_shape_ok(""), "empty notes are not judged here");
        for bad in ["One sentence, two clauses, three commas.\n1. Item", "1. Starts with an item\n2. Next", "Just one sentence.", "Opening line.\nNot numbered", "Opening line.\n1. One\nloose line"] {
            assert!(!notes_shape_ok(bad), "{bad}");
        }
        let ok = |i: usize| PlanSlide { role: "method".into(), title: format!("Method {i}"), bullets: vec!["Plain line".into()], flow: vec!["a".into(), "b".into()], ..Default::default() };
        let mut p = Plan { slides: (0..12).map(ok).collect(), ..Default::default() };
        assert!(check_plan(&p, "standard").is_empty(), "{:?}", check_plan(&p, "standard"));
        p.slides[0] = PlanSlide { role: "cover".into(), title: "Paper".into(), notes: "Hello, everyone, today one paper.".into(), ..Default::default() };
        assert!(check_plan(&p, "standard").is_empty(), "the cover's greeting is not judged");
        p.slides[1].bullets.push("where $x$ denotes the input".into());
        p.slides[2].notes = "No list here, just prose, many commas.".into();
        p.slides[3] = PlanSlide { role: "results".into(), title: "Results".into(), table: "tab1".into(), takeaway: "Ours wins".into(), ..Default::default() };
        p.slides[4] = PlanSlide { role: "ablation".into(), title: "Ablation".into(), figure: "fig2".into(), takeaway: "Each part helps".into(), notes: "How to read the chart.\n1. Blue is ours.".into(), ..Default::default() };
        let why = check_plan(&p, "standard");
        assert_eq!(why.len(), 3, "{why:?}");
        assert!(why[0].contains("Slide 2") && why[0].contains("only reads the symbols"));
        assert!(why[1].contains("Slide 3") && why[1].contains("numbered list"));
        assert!(why[2].contains("Slide 4") && why[2].contains("what each row is"));
        let notes = enforce_limits(&mut p);
        assert!(notes.iter().any(|n| n.contains("念符號")) && notes.iter().any(|n| n.contains("一句開場")) && notes.iter().any(|n| n.contains("每一列")), "every one left is a warning: {notes:?}");
    }

    #[test]
    fn the_prompt_carries_the_slide_and_notes_rules() {
        let (system, _) = prompt(&Material { title: "x".into(), ..Default::default() }, "standard", "en", None);
        for rule in [
            "One thing per slide",
            "Each input quantity or variable is its own bullet",
            "one bullet says what the rows are",
            "how to read it (what each panel shows",
            "A picture earns its space",
            "which part of the pipeline is trained",
            "Each equation stays with its own explanation",
            "full name, its formula or rule, and the reason",
            "No \"where $x$ denotes",
            "Spread the content evenly",
            "a detail slide (one input, one definition, one step) may leave it empty",
            NOTES_SHAPE,
        ] {
            assert!(system.contains(rule), "{rule}");
        }
        assert!(NOTES_SHAPE.contains("opening sentence with at most one comma") && NOTES_SHAPE.contains("which observed values") && NOTES_SHAPE.contains("overview slide's notes carry no numbers") && NOTES_SHAPE.contains("give the reason"));
        assert!(notes_rule("zh").contains("technical and medical terms"));
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
        let (_, user) = prompt(&m, "standard", "en", None);
        assert!(user.contains("fig2 [UNAVAILABLE"), "the planner is told what cannot be shown");
    }
}

#[cfg(test)]
mod brief_tests {
    use super::*;

    #[test]
    fn the_brief_is_a_block_of_the_prompt_and_an_empty_one_changes_nothing() {
        let m = Material { title: "Synthetic grading".into(), abstract_text: "We grade synthetic lesions.".into(), ..Default::default() };
        let (_, user) = prompt(&m, "standard", "en", None);
        assert_eq!(brief_block(&[]), "");
        assert_eq!(brief_block(&["  ".into(), String::new()]), "", "blank lines are no brief");
        assert_eq!(format!("{user}{}", brief_block(&[])), user, "an empty brief leaves the request byte-identical");
        let b = brief_block(&["Our earlier two-stage grader is the predecessor".into(), "Mention the lab's open dataset".into()]);
        assert!(b.contains("## What only the presenter can say") && b.contains("- Our earlier two-stage grader is the predecessor\n- Mention the lab's open dataset"), "{b}");
    }

    #[test]
    fn a_brief_line_no_slide_carries_is_reported() {
        let s = |bullets: &[&str], notes: &str| PlanSlide { role: "background".into(), title: "T".into(), bullets: bullets.iter().map(|x| x.to_string()).collect(), notes: notes.into(), ..Default::default() };
        let p = Plan { slides: vec![s(&["Our earlier grader used one stage"], ""), s(&["Results"], "This page shows results.\n1. 我們先前的分級器只有一個階段")], ..Default::default() };
        let brief = vec!["Our earlier one-stage grader is the predecessor".to_string(), "先前的分級器".to_string(), "Mention the public synthetic dataset".to_string(), "  ".to_string()];
        assert_eq!(brief_missing(&p, &brief), ["Mention the public synthetic dataset"], "half of a line's words on one slide carry it; notes count");
        assert_eq!(brief_problems(&p, &brief).len(), 1);
        assert_eq!(brief_warning("Mention the public synthetic dataset"), "補充的「Mention the public synthetic dataset」沒有放進大綱");
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
        let (system, user) = prompt(&m, "standard", "en", None);
        assert!(system.contains(PENDING_RULE) && user.contains("| Ours | 0.72 | [pending] |"));
        let plain = super::super::source::Material { title: "x".into(), ..Default::default() };
        assert!(!prompt(&plain, "standard", "en", None).0.contains(PENDING_RULE), "no marks, no rule");
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::todeck::figparts::{FigureParts, Part};

    fn three(by: &str) -> FigureParts {
        FigureParts { figure: "fig1".into(), split: true, axis: "x".into(), cuts: vec![1.0 / 3.0, 2.0 / 3.0], parts: (0..3).map(|i| Part { label: format!("P{i}"), about: String::new() }).collect(), slides: vec![], by: by.into(), why: String::new() }
    }

    #[test]
    fn the_pairing_follows_its_equations_and_lines_through_every_cut() {
        let mut s = PlanSlide { role: "loss".into(), bullets: vec!["a".into(), "".into(), "c".into()], equations: vec!["eq1".into(), "eq99".into(), "eq2".into()], explains: vec![3, 1, 1], ..Default::default() };
        assert_eq!((s.explained_by(0), s.explained_by(1)), (Some(2), None), "a bullet named for two equations pairs neither");
        // an equation the paper does not have goes with its pairing; an empty bullet shifts the numbers
        s.retain_equations(|e| e != "eq99");
        assert_eq!((s.equations.clone(), s.explains.clone()), (vec!["eq1".to_string(), "eq2".into()], vec![3, 1]));
        s.retain_bullets(|b| !b.is_empty());
        assert_eq!((s.bullets.len(), s.explains.clone(), s.explained_by(0), s.explained_by(1)), (2, vec![2, 1], Some(1), Some(0)));
        assert!(s.unpaired().is_empty());
        // the bullet goes: its equation is unpaired, the other keeps its line
        s.retain_bullets(|b| b != "a");
        assert_eq!((s.explains.clone(), s.unpaired()), (vec![1, 0], vec![1]));
        // lines cut from the end
        let mut t = PlanSlide { bullets: vec!["a".into()], equations: vec!["eq1".into()], explains: vec![4, 2], ..Default::default() };
        t.fix_explains();
        assert_eq!(t.explains, [0]);
        // the plan as the model wrote it: cleaned with the material, split over the limits
        let m = Material { equations: vec![crate::todeck::source::Eq { id: "eq1".into(), latex: "a=b".into(), ..Default::default() }], ..Default::default() };
        let p = Plan { slides: vec![PlanSlide { role: "loss".into(), title: "Loss".into(), bullets: vec!["Fig. 2".into(), "Why".into()], equations: vec!["eq99".into(), "eq1".into()], explains: vec![1, 2], ..Default::default() }], ..Default::default() };
        let p = clean_plan(p, &m);
        assert_eq!((p.slides[1].bullets.clone(), p.slides[1].equations.clone(), p.slides[1].explains.clone()), (vec!["Why".to_string()], vec!["eq1".to_string()], vec![1]));
        // no pairing, or a wrong one, is sent back; a slide of equations without lines has nothing to pair
        let ok = |i: usize| PlanSlide { role: "loss".into(), title: format!("Loss {i}"), bullets: vec!["Line".into()], equations: vec!["eq1".into()], explains: vec![1], ..Default::default() };
        let mut q = Plan { slides: (0..12).map(ok).collect(), ..Default::default() };
        assert!(check_plan(&q, "standard").is_empty(), "{:?}", check_plan(&q, "standard"));
        q.slides[1].explains = vec![];
        q.slides[2].explains = vec![5];
        q.slides[3].bullets.clear();
        q.slides[3].explains.clear();
        let why = check_plan(&q, "standard");
        assert!(why.len() == 2 && why[0].contains("Slide 2") && why[0].contains("`explains`") && why[1].contains("Slide 3"), "{why:?}");
    }

    #[test]
    fn a_part_or_trained_parts_the_figure_does_not_have_leave_the_slide_with_a_note() {
        let s = |role: &str, title: &str| PlanSlide { role: role.into(), title: title.into(), bullets: vec!["Line".into()], ..Default::default() };
        let plan = || Plan {
            slides: vec![
                PlanSlide { part: 2, ..s("method", "Good part") },
                PlanSlide { part: 4, ..s("method", "No such part") },
                PlanSlide { part: 1, figure: "fig2".into(), ..s("visual", "Own figure") },
                PlanSlide { trained: vec![1, 2], ..s("training", "Trained") },
            ],
            ..Default::default()
        };
        let model = three("model");
        let mut p = plan();
        assert_eq!(part_problems(&p, Some(&model)).len(), 2, "{:?}", part_problems(&p, Some(&model)));
        let notes = settle_parts(&mut p, Some(&model), false);
        assert_eq!((p.slides[0].part, p.slides[1].part, p.slides[2].part, p.slides[3].trained.clone()), (2, 0, 0, vec![1, 2]));
        assert!(notes.len() == 2 && notes[0].contains("No such part") && notes[0].contains("架構圖沒有第 4 段（共 3 段）") && notes[1].contains("Own figure"), "{notes:?}");
        // an unknown trained part, every part trained, none named: the usual layout, each said
        for (trained, word) in [(vec![2, 5], "架構圖沒有第 5 段"), (vec![1, 2, 3], "每一段都訓練"), (vec![], "沒有指出訓練的是架構圖的哪一段")] {
            let mut p = plan();
            p.slides[3].trained = trained;
            let notes = settle_parts(&mut p, Some(&model), false);
            assert!(p.slides[3].trained.is_empty() && notes.iter().any(|n| n.contains("Trained") && n.contains(word)), "{word}: {notes:?}");
        }
        // an equal split is a guessed box, a figure cut again no longer has the parts the plan named, no parts at all
        for (parts, recut, word) in [(Some(three("equal")), false, "平均切的"), (Some(three("model")), true, "重新分段"), (None, false, "沒有分段")] {
            let mut p = plan();
            let notes = settle_parts(&mut p, parts.as_ref(), recut);
            assert!(p.slides.iter().all(|s| s.part == 0 && s.trained.is_empty()), "{word}");
            assert!(notes.iter().filter(|n| n.contains(word)).count() >= 2, "{word}: {notes:?}");
        }
        // the trained parts take the figure's room: the settings table gets its own slide, said so
        let mut p = plan();
        p.slides[3].kv = vec![vec!["Epochs".into(), "30".into()]];
        assert!(check_plan(&p, "standard").iter().any(|x| x.contains("carries no kv")));
        let notes = enforce_limits(&mut p);
        assert_eq!((p.slides[3].kv.len(), p.slides[4].kv.len(), p.slides[4].title.as_str(), p.slides[4].role.as_str()), (0, 1, "Trained (2)", "training"));
        assert!(notes.iter().any(|n| n.contains("設定表拆到下一頁")), "{notes:?}");
        // cleaning: trained only on the training slide, no part on a walk-through or the overview
        let m = Material::default();
        let c = clean_plan(Plan { slides: vec![PlanSlide { part: 1, trained: vec![2, 2, 0, 1], ..s("training", "T") }, PlanSlide { part: 1, trained: vec![1], module: 2, ..s("method", "W") }, PlanSlide { part: 2, ..s("overview", "O") }], ..Default::default() }, &m);
        assert_eq!((c.slides[1].part, c.slides[1].trained.clone(), c.slides[2].part, c.slides[2].trained.len(), c.slides[3].part), (1, vec![1, 2], 0, 0, 0));
        assert!(has_visual(&PlanSlide { part: 1, ..Default::default() }) && has_visual(&PlanSlide { trained: vec![1], ..Default::default() }));
        // a plan that names the main figure AND a part of it: the part stands, the figure is implied
        let m = Material { figures: vec![crate::todeck::source::Fig { id: "fig1".into(), ..Default::default() }], ..Default::default() };
        let mut c = clean_plan(Plan { slides: vec![PlanSlide { figure: "fig1".into(), ..s("overview", "O") }, PlanSlide { figure: "fig1".into(), part: 2, ..s("loss", "L") }, PlanSlide { figure: "fig1".into(), trained: vec![1], ..s("training", "T") }], ..Default::default() }, &m);
        assert_eq!((c.slides[2].figure.as_str(), c.slides[2].part, c.slides[3].figure.as_str()), ("", 2, ""));
        assert!(settle_parts(&mut c, Some(&model), false).is_empty() && c.slides[2].part == 2 && c.slides[3].trained == [1]);
        // a part slide over the line limit: its second half keeps the part
        let mut long = Plan { slides: vec![PlanSlide { part: 2, bullets: (0..5).map(|i| format!("Line {i}")).collect(), ..s("method", "Long") }], ..Default::default() };
        enforce_limits(&mut long);
        assert_eq!(long.slides.iter().map(|s| (s.part, s.bullets.len())).collect::<Vec<_>>(), [(2, 3), (2, 2)]);
    }

    #[test]
    fn the_planner_is_told_the_three_fields() {
        let m = Material { title: "x".into(), ..Default::default() };
        let (with, _) = prompt(&m, "standard", "en", Some(&three("model")));
        for rule in ["part = the part's number", "Only these 3 parts exist", "trained = the numbers of the parts whose weights are trained", "A training slide with trained has kv = []", "explains = for each equation in order the number (1-based) of the bullet that explains it", "For a part of the main figure set part instead"] {
            assert!(with.contains(rule), "{rule}");
        }
        let (without, _) = prompt(&m, "standard", "en", None);
        assert!(without.contains("explains = for each equation") && !without.contains("trained =") && !without.contains("part = the part"), "no parts, no part fields");
        let props = &schema()["properties"]["slides"]["items"]["properties"];
        assert!(props["part"]["type"] == "integer" && props["trained"]["type"] == "array" && props["explains"]["type"] == "array");
    }
}
