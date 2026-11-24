//! 改頁 through L-Echo: the author selects one or two slides while rehearsing and says what to
//! change. A strong model (task `sc-page-edit`) sees the slides as data and as pictures and returns
//! a constrained plan of edits; the plan is validated here and compiled to editor ops (the model
//! never writes ops, sizes, coordinates or alpha). The ops are applied to a copy of the deck, each
//! affected slide is rendered (`crate::PageShot`), checked by code (overflow, `deckpolish::overlaps`,
//! the deck's lore-notation check) and by a model looking at the picture (`sc-page-check`), and one
//! corrective `sc-page-edit` round runs when anything failed; what still fails is returned, never
//! hidden. Only 套用 brings the ops to the live deck, as one undo step 「L-Echo 改頁」.
//!
//! Plan (first slice), `{"edits": [...]}`, each `{"edit": <kind>, …}` one of:
//! - `set_text {slide, obj, paras: [string]}`: an object that holds text, each paragraph within the
//!   limit of the object's role (`todeck::plan::limit`), `$…$` inline math (registry macros);
//! - `set_notes {slide, text}`: speaker notes in `plan::NOTES_SHAPE`;
//! - `split_slide {slide, groups: [[member]], titles: [string], notes: [string]}`: every groupable
//!   object of the slide in exactly one group, one title and one notes text per group. A member is
//!   an object id or `{obj, paras: [n]}` (1-based paragraphs of a text box): the box's paragraphs
//!   are divided between the pages, each page getting the box with its own paragraphs (the
//!   paragraphs themselves untouched; a display equation goes with the line above it). Each page's
//!   notes keep the old notes' sentences that define a symbol the page uses (`carry_symbols`);
//! - `insert_slide {after, kind, title, blocks: [{role, text}], notes?}`: `kind` a 論文報告 page role,
//!   blocks `bullet` / `takeaway` / `suggest`, built by `todeck::build::build` like that page kind.
//! - `split_object {slide, obj, parts: [string], arrange: "2x2" | "row" | "column"}`: one text
//!   object becomes 2–6 boxes in its place (its x, y and width; down into the free body area),
//!   the parts together exactly its text (whitespace and list markers aside), each within its
//!   role's limit; `2x2` takes four parts; a part that does not fit its cell at the role's size
//!   refuses the edit.
//! - `highlight_crop {slide, picture, mode: "crop" | "highlight", box: [x0, y0, x1, y1], part?}`:
//!   `box` in fractions of the picture as shown (≥ 5 % each side), its edges moved to the blank
//!   gutters of a PNG (`lore_docsrc::crop::snap_cut`). `crop`: only that region (`srcRect`), as
//!   large as its aspect allows in the picture's frame, or beside the lines by `build::part_frame`
//!   when the slide is that picture and one 內文; `highlight`: the picture at the style's ghost
//!   alpha with the region clear on top (`build::clear_piece`, the drawing of `build::ghosted`).
//!   `part` is refused: a finished deck does not keep the main figure's confirmed parts.
//! - `add_equation {slide, latex, explains?: obj, line?: n, display: true}`: a display equation
//!   right under line `n` (1-based, of the object's lines of text, equations not counted) in the same box, or, with one
//!   paired equation already there, both as the paired columns (`build::paired_columns`); at
//!   most `limit::EQUATIONS` per slide; `display: false` is refused (an inline `$…$` is a
//!   `set_text` of the whole paragraph).
//!
//! A `split_slide` lays each resulting page out again in the base's frames (`reflow`), like a
//! freshly built page of those objects; shapes that are not role content keep their frames (warned).
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/decks/{name}/page-edit | `{slides: [uid] (1–2), instruction, material?: {reader_paper?, pages?}, engine?}` → proposal |
//! | POST | /api/decks/{name}/page-edit/{id}/revise | `{instruction}` → proposal (same id; the earlier turns go along) |
//! | GET | /api/decks/{name}/page-edit/{id}/deck | the preview deck (for `print.html?preview=`) |
//! | GET | /api/decks/{name}/page-edit/{id}/image/{which} | `before-0`, `after-0`, … PNG |
//! | POST | /api/decks/{name}/page-edit/{id}/apply | `{request?}` → changed slides; 409 when a slide changed since |
//! | POST | /api/decks/{name}/page-edit/{id}/undo | undo, only while the editor's next undo is that edit: through the page that applied it (`PageQueue::undo`), the server itself when that page is gone; `{undone: "page" | "server" | "already"}` (already: the author undid it in the app) |
//!
//! A proposal is `{id, before: [url], after: [url], plan, findings: [{slide, page?, kind, text}], warnings}`.
//! Each proposal and apply is a line of `decks/<name>/page-edit-log.jsonl`; previews live in memory
//! (the last [`KEEP`] per deck) and vanish on restart.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_canvas::{Deck, Editor, Frame, Kind, Object, Op, Para, Run, Slide, TextBody};
use lore_llm::{Image, LlmRequest};
use lore_notation::Notation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::deckstyle::{body_of, chrome_ph, title_object};
use crate::todeck::plan::{self, NOTES_SHAPE, limit, notes_shape_ok, reads_symbols};
use crate::{ApiError, Shared};

/// The undo label of an applied page edit (the page's own apply must use it too).
pub const LABEL: &str = "L-Echo 改頁";
pub const LOG: &str = "page-edit-log.jsonl";
/// Previews kept per deck.
pub const KEEP: usize = 4;
/// Width (CSS px) of the page pictures.
const SHOT_W: u32 = 1280;
/// Material: at most this many pages of a paper, this many characters each.
pub const PAGES_MAX: usize = 3;
pub const PAGE_CHARS: usize = 3000;
pub const INSTRUCTION_CHARS: usize = 2000;
/// Every edit kind of the plan.
pub const KINDS: [&str; 7] = ["set_text", "set_notes", "split_slide", "insert_slide", "split_object", "highlight_crop", "add_equation"];
/// Arrangements of a `split_object`.
pub const ARRANGE: [&str; 3] = ["2x2", "row", "column"];
/// A `highlight_crop` box: at least this share of each side of the picture.
pub const BOX_MIN: f64 = 0.05;
/// Block roles of an inserted slide.
pub const BLOCK_ROLES: [&str; 3] = ["bullet", "takeaway", "suggest"];
/// Page kinds an inserted slide may have: the 論文報告 roles, one cover per deck.
pub fn insert_kinds() -> Vec<&'static str> {
    plan::ROLES.iter().copied().filter(|r| *r != "cover").collect()
}

// ── The plan ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub role: String,
    pub text: String,
}

/// What a group of a `split_slide` holds: a whole object, or some paragraphs (1-based) of a text box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Member {
    Obj(u32),
    Paras { obj: u32, paras: Vec<usize> },
}

impl Member {
    fn obj(&self) -> u32 {
        match self {
            Member::Obj(id) | Member::Paras { obj: id, .. } => *id,
        }
    }
}

impl From<u32> for Member {
    fn from(id: u32) -> Member {
        Member::Obj(id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum Edit {
    SetText {
        #[serde(default)]
        slide: String,
        obj: u32,
        paras: Vec<String>,
    },
    SetNotes {
        #[serde(default)]
        slide: String,
        text: String,
    },
    SplitSlide {
        #[serde(default)]
        slide: String,
        groups: Vec<Vec<Member>>,
        titles: Vec<String>,
        #[serde(default)]
        notes: Vec<String>,
    },
    InsertSlide {
        after: String,
        kind: String,
        title: String,
        #[serde(default)]
        blocks: Vec<Block>,
        #[serde(default)]
        notes: String,
    },
    SplitObject {
        #[serde(default)]
        slide: String,
        obj: u32,
        parts: Vec<String>,
        arrange: String,
    },
    HighlightCrop {
        #[serde(default)]
        slide: String,
        picture: u32,
        mode: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<u32>,
        #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
        region: Option<Vec<f64>>,
    },
    AddEquation {
        #[serde(default)]
        slide: String,
        latex: String,
        /// The text object holding the line the equation explains.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explains: Option<u32>,
        /// That line: 1-based, of the object's lines of text (equations not counted).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
        #[serde(default = "yes")]
        display: bool,
    },
}

fn yes() -> bool {
    true
}

impl Edit {
    fn slide(&self) -> &str {
        match self {
            Edit::SetText { slide, .. } | Edit::SetNotes { slide, .. } | Edit::SplitSlide { slide, .. } | Edit::SplitObject { slide, .. } | Edit::HighlightCrop { slide, .. } | Edit::AddEquation { slide, .. } => slide,
            Edit::InsertSlide { after, .. } => after,
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Edit::SetText { .. } => "set_text",
            Edit::SetNotes { .. } => "set_notes",
            Edit::SplitSlide { .. } => "split_slide",
            Edit::InsertSlide { .. } => "insert_slide",
            Edit::SplitObject { .. } => "split_object",
            Edit::HighlightCrop { .. } => "highlight_crop",
            Edit::AddEquation { .. } => "add_equation",
        }
    }
}

/// One finding of the checks: the slide (uid, empty when not one slide), its kind and a line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub slide: String,
    /// The slide's page number once the edit is applied (a slide the edit makes has no other name
    /// for the author); the text never repeats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
}

/// What an unknown kind's reason adds for the model after the author's warning.
const UNKNOWN_TAIL: &str = "（只能用 ";

impl Finding {
    /// The finding as one line with its page (for the model, and the log's reader).
    pub fn line(&self) -> String {
        match self.page {
            Some(n) if !self.text.starts_with(&format!("第 {n} 頁")) => format!("第 {n} 頁{}", self.text),
            _ => self.text.clone(),
        }
    }
}

/// The model's plan as edits: (edits, reasons for the model and the author, warnings).
/// One edit that cannot be read never fails the others.
pub fn parse_plan(v: &Value, targets: &[String]) -> (Vec<Edit>, Vec<String>, Vec<String>) {
    let (mut edits, mut reasons, mut warnings) = (vec![], vec![], vec![]);
    let Some(items) = v.get("edits").and_then(Value::as_array) else {
        reasons.push("回答裡沒有 edits".into());
        return (edits, reasons, warnings);
    };
    for (i, it) in items.iter().enumerate() {
        let kind = it.get("edit").and_then(Value::as_str).unwrap_or("");
        match serde_json::from_value::<Edit>(it.clone()) {
            Ok(mut e) => {
                // with one slide the slide may be left out
                if let (Edit::SetText { slide, .. } | Edit::SetNotes { slide, .. } | Edit::SplitSlide { slide, .. } | Edit::SplitObject { slide, .. } | Edit::HighlightCrop { slide, .. } | Edit::AddEquation { slide, .. }, [only]) = (&mut e, targets) {
                    if slide.is_empty() {
                        *slide = only.clone();
                    }
                }
                edits.push(e)
            }
            Err(_) if kind.is_empty() || !KINDS.contains(&kind) => {
                // one line for both readers: the author is told once (`run` does not repeat a
                // reason that is already a warning), the model reads the kinds it may use
                let w = format!("不認得的修改「{}」，沒有做", plan::clip(kind, 30));
                if !warnings.contains(&w) {
                    warnings.push(w.clone());
                }
                let r = format!("{w}{UNKNOWN_TAIL}{}）", KINDS.join(", "));
                if !reasons.contains(&r) {
                    reasons.push(r);
                }
            }
            Err(e) => reasons.push(format!("第 {} 個修改（{kind}）：格式不對（{e}）", i + 1)),
        }
    }
    (edits, reasons, warnings)
}

// ── Roles and limits ─────────────────────────────────────────────────────────

/// Characters as the limits count them: inline math as half its source (`plan::check_plan`'s rule).
fn chars(s: &str) -> usize {
    let mut n = 0usize;
    for (i, part) in s.split('$').enumerate() {
        let c = part.chars().count();
        n += if i % 2 == 1 { c.div_ceil(2) } else { c };
    }
    n
}

/// What an object is on its slide: `title`, `body`, `takeaway`, `caption`, `text` (editable) or
/// `kicker`, `ref`, `chrome`, `picture`, `table`, `group`, `shape` (not editable as text here).
pub fn obj_role(s: &Slide, o: &Object, slide_h: i64) -> &'static str {
    let slot = o.slot.as_deref().unwrap_or("");
    if chrome_ph(o) {
        return "chrome";
    }
    if title_object(s, slide_h).is_some_and(|t| t.id == o.id) || slot == "title" || matches!(o.ph.as_deref(), Some("title" | "ctrTitle")) {
        return "title";
    }
    if o.name == "kicker" {
        return "kicker";
    }
    if o.name == "Ref" {
        return "ref";
    }
    if slot.starts_with("deco") {
        return "chrome";
    }
    match &o.kind {
        Kind::Picture(_) => return "picture",
        Kind::Table(_) => return "table",
        Kind::Group(_) => return "group",
        _ => {}
    }
    let Some(b) = body_of(o) else { return "shape" };
    let tags: BTreeSet<&str> = b.paras.iter().flat_map(|p| p.runs.iter()).filter_map(|r| r.tag.as_deref()).collect();
    if slot.starts_with("takeaway") || o.name == "takeaway" || tags.contains("takeaway") {
        "takeaway"
    } else if slot.starts_with("caption") || o.name == "圖說" || tags.contains("caption") {
        "caption"
    } else if slot.starts_with("text") || tags.contains("body") {
        "body"
    } else {
        "text"
    }
}

/// Does the slide show something besides text (picture, table, figure, display equation, an
/// empty picture slot)? Then its body takes the limits beside a figure.
fn slide_visual(s: &Slide) -> bool {
    s.objects.iter().any(|o| {
        matches!(o.kind, Kind::Picture(_) | Kind::Table(_) | Kind::Group(_))
            || (o.is_empty_slot() && o.slot.as_deref().is_some_and(|x| x.starts_with("image")))
            || body_of(o).is_some_and(|b| b.paras.iter().any(|p| !p.runs.is_empty() && p.runs.iter().all(|r| r.kind == "math")))
    })
}

fn slide_has_display_math(s: &Slide) -> bool {
    s.objects.iter().any(|o| body_of(o).is_some_and(|b| b.paras.iter().any(|p| !p.runs.is_empty() && p.runs.iter().all(|r| r.kind == "math"))))
}

/// (paragraphs, characters per paragraph) an object of `role` may hold on a slide of `page_role`;
/// `None`: not edited as text here.
pub fn limits(role: &str, page_role: &str, s: &Slide) -> Option<(usize, usize)> {
    Some(match role {
        "title" if page_role == "cover" => (2, 200),
        "title" => (1, limit::TITLE),
        "takeaway" => (1, limit::TAKEAWAY),
        "caption" => (2, 140),
        "body" | "text" => {
            if page_role == "takeaway" {
                (limit::TAKE_HOME, limit::BULLET)
            } else if slide_has_display_math(s) {
                (limit::BULLETS_EQ, limit::BULLET_VISUAL)
            } else if slide_visual(s) {
                (limit::BULLETS_VISUAL, limit::BULLET_VISUAL)
            } else {
                (limit::BULLETS, limit::BULLET)
            }
        }
        _ => return None,
    })
}

/// The objects a split puts into groups: everything but the title, the kicker, `Ref:`, the
/// layout's chrome and hidden objects (those stay on every part, or on the first).
pub fn groupable(s: &Slide, slide_h: i64) -> Vec<u32> {
    s.objects.iter().filter(|o| !o.hidden && !matches!(obj_role(s, o, slide_h), "title" | "kicker" | "ref" | "chrome")).map(|o| o.id).collect()
}

fn page_roles(deck: &Deck) -> Vec<String> {
    crate::deckpolish::roles_for(deck, &[])
}

// ── Validate and compile ─────────────────────────────────────────────────────

/// LaTeX → OMML (`latex`, display, size in pt): pandoc through `sc_core::latex_to_omml`, a fake in tests.
pub type MathFn = dyn Fn(&str, bool, f64) -> Result<String, String> + Send + Sync;

/// What the checks and the build read besides the deck.
pub struct Env<'a> {
    /// The workspace and the deck's source package (an inserted slide is built with them; `None`:
    /// no insert).
    pub build: Option<(&'a sc_core::Workspace, &'a lore_ooxml::Deck)>,
    pub dir: &'a Path,
    pub pandoc: &'a lore_math::Pandoc,
    /// The deck's registry macros (shared registry + the deck's), expanded before OMML.
    pub macros: BTreeMap<String, String>,
    pub math: &'a MathFn,
    /// OMML fingerprint → LaTeX (the session's cache), for the notation check.
    pub latex: &'a BTreeMap<String, String>,
    pub glob: &'a Notation,
    pub doc: &'a Notation,
    pub lang: String,
}

/// A plan made concrete: the edits kept, their ops (in order, one transaction), the deck with them,
/// the slides they touched (preview uids, deck order) and why the other edits were left out.
pub struct Prepared {
    pub kept: Vec<Edit>,
    pub ops: Vec<Op>,
    pub preview: Editor,
    pub touched: Vec<String>,
    pub reasons: Vec<String>,
    /// What the kept edits did differently from the plain case (a box not snapped, a shape that
    /// kept its place in a split), for the author.
    pub warnings: Vec<String>,
}

/// Each edit in turn: checked, compiled against the deck as the earlier edits left it, applied to a
/// copy, and the deck's notation check compared before and after it (a new error or warning, such
/// as an undefined symbol, refuses that edit).
pub fn prepare(live: &Editor, targets: &[String], edits: &[Edit], env: &Env) -> Prepared {
    let mut work = live.clone();
    let mut out = Prepared { kept: vec![], ops: vec![], preview: live.clone(), touched: vec![], reasons: vec![], warnings: vec![] };
    let mut touched: BTreeSet<String> = targets.iter().cloned().collect();
    let split: Vec<&str> = edits.iter().filter(|e| matches!(e, Edit::SplitSlide { .. })).map(Edit::slide).collect();
    // the notes first: a symbol an edit brings is defined there before the notation check
    let mut order: Vec<(usize, &Edit)> = edits.iter().enumerate().collect();
    order.sort_by_key(|(_, e)| !matches!(e, Edit::SetNotes { .. }));
    for (i, e) in order {
        let at = format!("第 {} 個修改（{}）", i + 1, e.kind());
        if !targets.iter().any(|t| t == e.slide()) {
            out.reasons.push(format!("{at}：只能改交來的頁（{}）", targets.join("、")));
            continue;
        }
        if !matches!(e, Edit::SplitSlide { .. } | Edit::InsertSlide { .. }) && split.contains(&e.slide()) {
            out.reasons.push(format!("{at}：這一頁要拆頁，拆頁的標題和備註寫在 split_slide 裡，不要再另外改"));
            continue;
        }
        if let Err(r) = check(&work, e, env) {
            out.reasons.extend(r.into_iter().map(|r| format!("{at}：{r}")));
            continue;
        }
        let (ops, warn) = match compile(&work, e, env) {
            Ok(x) => x,
            Err(r) => {
                out.reasons.push(format!("{at}：{r}"));
                continue;
            }
        };
        let (before, was) = (notation_keys(&work.deck, env), work.deck.clone());
        let applied = match work.apply(ops.clone(), LABEL) {
            Ok(a) => a,
            Err(r) => {
                out.reasons.push(format!("{at}：編輯器不接受（{r}）"));
                continue;
            }
        };
        let (after, mut warn) = (notation_issues(&work.deck, env), warn);
        let mut new: Vec<String> = vec![];
        for (k, i) in after.iter().filter(|(k, _)| !before.contains_key(*k)) {
            match order_was_so(i, &was, &work.deck, &applied.slides, env) {
                Some(w) => warn.push(w),
                None => new.push(i["message"].as_str().unwrap_or(&k.1).to_string()),
            }
        }
        if !new.is_empty() {
            work.undo();
            out.reasons.push(format!("{at}：沒有通過對照表檢查：{}", new.join("；")));
            continue;
        }
        touched.extend(applied.slides);
        out.ops.extend(ops);
        out.kept.push(e.clone());
        out.warnings.extend(warn);
    }
    out.touched = work.deck.slides.iter().map(|s| s.uid.clone()).filter(|u| touched.contains(u)).collect();
    out.preview = work;
    out
}

fn has_text(o: &Object) -> bool {
    matches!(o.kind, Kind::Text(_) | Kind::Shape(_)) && body_of(o).is_some()
}

/// Inline math of a line: `$` balanced, each piece converts to OMML after the registry's macros.
fn math_ok(text: &str, size: f64, env: &Env) -> Vec<String> {
    if text.matches('$').count() % 2 == 1 {
        return vec![format!("「{}」的 $ 沒有成對", plan::clip(text, 30))];
    }
    text.split('$')
        .enumerate()
        .filter(|(i, p)| i % 2 == 1 && !p.trim().is_empty())
        .filter_map(|(_, p)| (env.math)(&crate::todeck::build::expand_macros(p, &env.macros), false, size).err().map(|e| format!("${}$ 轉不成方程式（{}）", plan::clip(p, 40), plan::clip(&e, 80))))
        .collect()
}

fn line_ok(text: &str, max: usize, what: &str) -> Option<String> {
    if reads_symbols(text) {
        return Some(format!("{what}「{}」只是在念符號，請寫進講者備註", plan::clip(text, 30)));
    }
    (chars(text) > max).then(|| format!("{what}「{}」有 {} 字，最多 {max} 字（不會縮字）", plan::clip(text, 30), chars(text)))
}

fn notes_ok(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return Some("講者備註是空的".into());
    }
    (!notes_shape_ok(text)).then(|| "講者備註要是一句開場（最多一個逗號）＋編號條列「1. …」".into())
}

/// Why `e` cannot be done on `ed`'s deck as it is (empty: it can).
pub fn check(ed: &Editor, e: &Edit, env: &Env) -> Result<(), Vec<String>> {
    let deck = &ed.deck;
    let h = deck.size.h;
    let mut r = vec![];
    let Some(idx) = deck.slide_index(e.slide()) else { return Err(vec![format!("找不到投影片 {}", e.slide())]) };
    let s = &deck.slides[idx];
    let roles = page_roles(deck);
    let page_role = roles.get(idx).map(String::as_str).unwrap_or("");
    match e {
        Edit::SetText { obj, paras, .. } => {
            let Some(o) = s.find(*obj).filter(|o| has_text(o)) else { return Err(vec![format!("第 {} 頁沒有可以放字的物件 {obj}", idx + 1)]) };
            let role = obj_role(s, o, h);
            let Some((np, nc)) = limits(role, page_role, s) else { return Err(vec![format!("物件 {obj}（{role}）由程式產生，不在這裡改")]) };
            if paras.is_empty() || paras.iter().all(|p| p.trim().is_empty()) {
                r.push(format!("物件 {obj} 不能改成空的"));
            }
            if paras.len() > np {
                r.push(format!("物件 {obj}（{role}）有 {} 段，最多 {np} 段；多的請拆頁", paras.len()));
            }
            let size = body_of(o).and_then(|b| b.paras.iter().flat_map(|p| p.runs.iter()).map(|x| x.look.size).next()).unwrap_or(18.0);
            for p in paras {
                r.extend(line_ok(p, nc, "這一段"));
                r.extend(math_ok(p, size, env));
            }
        }
        Edit::SetNotes { text, .. } => r.extend(notes_ok(text)),
        Edit::SplitSlide { groups, titles, notes, .. } => {
            if groups.len() < 2 {
                r.push("拆頁至少要兩組".into());
            }
            if titles.len() != groups.len() {
                r.push(format!("{} 組卻有 {} 個標題：每組一個", groups.len(), titles.len()));
            }
            if notes.len() != groups.len() {
                r.push(format!("{} 組卻有 {} 份講者備註：每組一份", groups.len(), notes.len()));
            }
            if title_object(s, h).is_none() {
                r.push("這一頁沒有標題物件，沒辦法拆".into());
            }
            if let Err(e) = split_parts(s, h, groups) {
                r.extend(e);
            }
            for t in titles {
                if t.trim().is_empty() {
                    r.push("標題不能是空的".into());
                }
                r.extend(line_ok(t, limit::TITLE, "標題"));
                r.extend(math_ok(t, 24.0, env));
            }
            notes.iter().filter_map(|n| notes_ok(n)).for_each(|x| r.push(x));
        }
        Edit::InsertSlide { kind, title, blocks, notes, .. } => {
            if env.build.is_none() || ed.style.is_none() {
                r.push("這份簡報沒有底版，不能插頁".into());
            }
            if !insert_kinds().contains(&kind.as_str()) {
                r.push(format!("不認得的頁類「{}」：只能是 {}", plan::clip(kind, 20), insert_kinds().join(", ")));
            }
            if title.trim().is_empty() {
                r.push("新頁要有標題".into());
            }
            r.extend(line_ok(title, limit::TITLE, "標題"));
            r.extend(math_ok(title, 24.0, env));
            let by = |role: &str| blocks.iter().filter(|b| b.role == role).collect::<Vec<_>>();
            for b in blocks.iter().filter(|b| !BLOCK_ROLES.contains(&b.role.as_str())) {
                r.push(format!("不認得的區塊「{}」：只能是 {}", plan::clip(&b.role, 20), BLOCK_ROLES.join(", ")));
            }
            let text_only = plan::text_only_ok(kind);
            let (nb, nc) = if kind == "takeaway" { (limit::TAKE_HOME, limit::BULLET) } else { (limit::BULLETS_VISUAL, limit::BULLET_VISUAL) };
            let bullets = by("bullet");
            if bullets.len() > nb {
                r.push(format!("{} 行，最多 {nb} 行", bullets.len()));
            }
            for b in &bullets {
                r.extend(line_ok(&b.text, nc, "這一行"));
                r.extend(math_ok(&b.text, 18.0, env));
            }
            let take = by("takeaway");
            if take.len() > 1 {
                r.push("結論句只能一句".into());
            }
            for b in &take {
                r.extend(line_ok(&b.text, limit::TAKEAWAY, "結論句"));
                r.extend(math_ok(&b.text, 18.0, env));
            }
            let suggest = by("suggest");
            if !text_only && suggest.iter().all(|b| b.text.trim().is_empty()) {
                r.push(format!("「{kind}」頁不能只有文字：給一個 suggest 區塊，說要放哪張圖（留空的圖框）"));
            }
            if text_only && !suggest.is_empty() && kind == "thanks" {
                r.push("結尾頁不放圖".into());
            }
            for b in &suggest {
                r.extend(line_ok(&b.text, limit::SUGGEST, "建議放的圖"));
            }
            if kind == "thanks" && !bullets.is_empty() {
                r.push("結尾頁只有標題".into());
            }
            if !notes.trim().is_empty() || !text_only {
                r.extend(notes_ok(notes));
            }
        }
        Edit::SplitObject { obj, parts, arrange, .. } => {
            let Some(o) = s.find(*obj).filter(|o| has_text(o)) else { return Err(vec![format!("第 {} 頁沒有可以放字的物件 {obj}", idx + 1)]) };
            let role = obj_role(s, o, h);
            let Some((_, nc)) = limits(role, page_role, s).filter(|_| role != "title") else { return Err(vec![format!("物件 {obj}（{role}）不拆")]) };
            if geo(ed, env).is_none() {
                r.push("這份簡報沒有底版，不能重新排版".into());
            }
            if !(2..=6).contains(&parts.len()) {
                r.push(format!("拆成 {} 塊：要 2 到 6 塊", parts.len()));
            }
            if !ARRANGE.contains(&arrange.as_str()) {
                r.push(format!("不認得的排法「{}」：只能是 {}", plan::clip(arrange, 20), ARRANGE.join(", ")));
            } else if arrange == "2x2" && parts.len() != 4 {
                r.push(format!("2x2 要剛好 4 塊，這裡有 {} 塊：改用 row 或 column", parts.len()));
            }
            let size = body_of(o).and_then(|b| b.paras.iter().flat_map(|p| p.runs.iter()).map(|x| x.look.size).next()).unwrap_or(18.0);
            for p in &clean_parts(parts) {
                if p.trim().is_empty() {
                    r.push("有一塊是空的".into());
                }
                r.extend(line_ok(p, nc, "這一塊"));
                r.extend(math_ok(p, size, env));
            }
            let was: Vec<String> = body_of(o).map(|b| b.paras.iter().map(|p| para_text(p, env.latex)).collect()).unwrap_or_default();
            let parts: Vec<String> = parts.iter().map(|p| expand_inline(p, &env.macros)).collect();
            if let Some(d) = text_differs(&was, &parts) {
                r.push(d);
            }
        }
        Edit::HighlightCrop { picture, mode, part, region, .. } => {
            let Some(o) = s.find(*picture) else { return Err(vec![format!("第 {} 頁沒有物件 {picture}", idx + 1)]) };
            let Kind::Picture(p) = &o.kind else { return Err(vec![format!("物件 {picture} 不是圖片")]) };
            if !matches!(mode.as_str(), "crop" | "highlight") {
                r.push(format!("不認得的 mode「{}」：只能是 crop 或 highlight", plan::clip(mode, 20)));
            }
            if part.is_some() {
                r.push("這份簡報沒有記下架構圖確認過的分段（只在論文報告的工作快取裡），不能用 part：請用 box 框出那一段".into());
            }
            match region {
                None if part.is_none() => r.push("要給 box：[x0, y0, x1, y1]，圖的比例 0–1".into()),
                None => {}
                Some(b) => r.extend(box_problem(b)),
            }
            if mode == "highlight" && ed.style.is_none() {
                r.push("這份簡報沒有底版，不知道變淡的程度".into());
            }
            if mode == "crop" && p.alpha < 0.999 {
                r.push(format!("物件 {picture} 是變淡的整張圖（上面疊著清楚的段），不能再裁切"));
            }
        }
        Edit::AddEquation { latex, explains, line, display, .. } => {
            if !display {
                return Err(vec!["行內的式子請用 set_text：給那一段完整的新文字，式子寫成 $…$".into()]);
            }
            if geo(ed, env).is_none() {
                r.push("這份簡報沒有底版，不能重新排版".into());
            }
            let latex = latex.trim();
            if latex.is_empty() || latex.contains('$') {
                r.push("latex 要是一條式子的 LaTeX（不含 $）".into());
            } else if let Err(e) = (env.math)(&crate::todeck::build::expand_macros(latex, &env.macros), true, 20.0) {
                r.push(format!("「{}」轉不成方程式（{}）", plan::clip(latex, 40), plan::clip(&e, 80)));
            }
            let n_eq: usize = s.objects.iter().filter_map(body_of).map(|b| b.paras.iter().filter(|p| is_display(p)).count()).sum();
            if n_eq >= limit::EQUATIONS {
                r.push(format!("這一頁已經有 {n_eq} 條式子，一頁最多 {} 條：放到別頁（insert_slide 或 split_slide）", limit::EQUATIONS));
            }
            match eq_target(s, h, *explains, *line) {
                Err(e) => r.push(e),
                Ok((o, at)) => {
                    let b = body_of(o).unwrap();
                    if b.paras.get(at + 1).is_some_and(is_display) {
                        r.push(format!("物件 {} 的那一行下面已經有一條式子", o.id));
                    }
                    // with an equation the body takes the limits beside a visual
                    let lines: Vec<String> = s.objects.iter().filter(|x| obj_role(s, x, h) == "body").filter_map(body_of).flat_map(|b| b.paras.iter().filter(|p| !is_display(p)).map(|p| para_text(p, env.latex))).filter(|t| !t.trim().is_empty()).collect();
                    if page_role != "takeaway" && (lines.len() > limit::BULLETS_EQ || lines.iter().any(|l| chars(l) > limit::BULLET_VISUAL)) {
                        r.push(format!("加了式子，這一頁最多 {} 行、每行 {} 字（不縮字）：先刪短或拆頁", limit::BULLETS_EQ, limit::BULLET_VISUAL));
                    }
                }
            }
        }
    }
    if r.is_empty() { Ok(()) } else { Err(r) }
}

/// `$…$` pieces of a line with the registry's macros expanded (as the slide holds them).
fn expand_inline(text: &str, macros: &BTreeMap<String, String>) -> String {
    let parts: Vec<&str> = text.split('$').collect();
    parts.iter().enumerate().map(|(i, p)| if i % 2 == 1 && i + 1 < parts.len() { format!("${}$", crate::todeck::build::expand_macros(p, macros)) } else { p.to_string() }).collect()
}

/// A paragraph that is an equation alone (display math).
fn is_display(p: &Para) -> bool {
    !p.runs.is_empty() && p.runs.iter().all(|r| r.kind == "math")
}

/// The body object and the paragraph (0-based) an equation goes under: the named line, else the
/// last line of the slide's only body object.
fn eq_target(s: &Slide, h: i64, explains: Option<u32>, line: Option<u32>) -> Result<(&Object, usize), String> {
    let o = match explains {
        Some(id) => s.find(id).filter(|o| has_text(o) && obj_role(s, o, h) == "body").ok_or_else(|| format!("物件 {id} 不是這一頁的內文"))?,
        None => {
            let bodies: Vec<&Object> = s.objects.iter().filter(|o| !o.hidden && has_text(o) && obj_role(s, o, h) == "body").collect();
            match bodies.as_slice() {
                [one] => *one,
                [] => return Err("這一頁沒有內文，式子沒有可以配的行".into()),
                _ => return Err("這一頁有好幾個內文：用 explains 和 line 指出式子說明的是哪一行".into()),
            }
        }
    };
    let b = body_of(o).ok_or("物件沒有文字")?;
    // line n counts the lines of text only: an equation set earlier in the plan never moves it
    let lines: Vec<usize> = (0..b.paras.len()).filter(|i| !is_display(&b.paras[*i])).collect();
    let at = match line {
        Some(n) => *(n as usize).checked_sub(1).and_then(|i| lines.get(i)).ok_or_else(|| format!("物件 {} 沒有第 {n} 行（有 {} 行字）", o.id, lines.len()))?,
        None if explains.is_some() => return Err("explains 要和 line 一起給".into()),
        None => *lines.last().ok_or("內文沒有文字行")?,
    };
    Ok((o, at))
}

/// Why a `highlight_crop` box cannot be used.
fn box_problem(b: &[f64]) -> Option<String> {
    let [x0, y0, x1, y1] = b else { return Some(format!("box 要有 4 個數 [x0, y0, x1, y1]，這裡有 {} 個", b.len())) };
    if [x0, y0, x1, y1].iter().any(|v| !v.is_finite() || **v < 0.0 || **v > 1.0) {
        return Some("box 的數要在 0 到 1 之間（圖的比例）".into());
    }
    if x1 - x0 < BOX_MIN || y1 - y0 < BOX_MIN {
        return Some(format!("box 太小或是反的：每一邊至少要圖的 {:.0} %", BOX_MIN * 100.0));
    }
    None
}

/// A list marker at the start of a line (`•`, `-`, `1.`, `(a)`, `b)` …), with its space.
fn strip_marker(s: &str) -> &str {
    let t = s.trim_start();
    for m in ["•", "·", "▪", "◦", "‣", "-", "–", "—", "*", "+"] {
        if let Some(r) = t.strip_prefix(m) {
            return r;
        }
    }
    let cs: Vec<(usize, char)> = t.char_indices().collect();
    let (paren, start) = if cs.first().is_some_and(|c| c.1 == '(') { (true, 1) } else { (false, 0) };
    let mut k = start;
    while k < cs.len() && cs[k].1.is_ascii_digit() {
        k += 1;
    }
    let digits = k > start;
    if !digits && k < cs.len() && cs[k].1.is_ascii_alphabetic() && cs.get(k + 1).is_some_and(|c| c.1 == ')') {
        k += 1;
    }
    if k > start && k < cs.len() && (cs[k].1 == ')' || (!paren && digits && cs[k].1 == '.')) && cs.get(k + 1).is_none_or(|c| c.1.is_whitespace()) {
        return &t[cs[k].0 + cs[k].1.len_utf8()..];
    }
    s
}

/// What joins the items of a crammed line: the marks between parts, and the words.
const SEPARATORS: [char; 6] = [';', '；', ',', '，', '、', '/'];
const JOINERS: [&str; 5] = ["and", "or", "和", "與", "及"];

/// The parts as the boxes will hold them: no list marker in front, no separator or joining word
/// at the end ("…;", "…, and"); a full stop stays only when every part ends with one.
pub fn clean_parts(parts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = parts
        .iter()
        .map(|p| {
            let mut t = strip_marker(p).trim().to_string();
            loop {
                let before = t.len();
                t = t.trim_end_matches(|c: char| SEPARATORS.contains(&c) || c.is_whitespace()).to_string();
                for j in JOINERS {
                    let lower = t.to_lowercase();
                    if lower.ends_with(j) && (j.chars().all(|c| !c.is_ascii()) || lower[..lower.len() - j.len()].ends_with(|c: char| c.is_whitespace() || SEPARATORS.contains(&c))) {
                        t.truncate(t.len() - j.len());
                    }
                }
                if t.len() == before {
                    break t;
                }
            }
        })
        .collect();
    let stop = |t: &String| t.ends_with('.') || t.ends_with('。');
    if !out.iter().all(stop) {
        for t in out.iter_mut() {
            while stop(t) {
                t.pop();
            }
        }
    }
    out
}

/// Whether the parts are the original text split (whitespace, list markers, and the separators
/// and joining words between parts ignored); `Some` says where they differ.
fn text_differs(was: &[String], parts: &[String]) -> Option<String> {
    let squash = |t: &str| -> (Vec<char>, Vec<usize>) {
        let cs: Vec<char> = t.chars().collect();
        let at = cs.iter().enumerate().filter(|(_, c)| !c.is_whitespace()).map(|(i, _)| i).collect();
        (cs, at)
    };
    let orig_t: String = was.iter().map(|l| strip_marker(l).trim()).collect::<Vec<_>>().join(" ");
    let (ot, oi) = squash(&orig_t);
    let a: Vec<char> = oi.iter().map(|i| ot[*i]).collect();
    let near_o = |k: usize| oi.get(k).map(|i| ot[*i..].iter().take(20).collect::<String>()).unwrap_or_default();
    // where a part may start after `k`: right there, after the separators, after a joining word too
    let starts = |k: usize| -> Vec<usize> {
        let mut out = vec![k];
        let mut j = k;
        while j < a.len() && SEPARATORS.contains(&a[j]) {
            j += 1;
        }
        out.push(j);
        for w in JOINERS {
            let wc: Vec<char> = w.chars().collect();
            if a.len() >= j + wc.len() && a[j..j + wc.len()].iter().zip(&wc).all(|(x, y)| x.to_lowercase().eq(y.to_lowercase())) {
                out.push(j + wc.len());
            }
        }
        out
    };
    let fail = |what: String| Some(format!("拆出來的字和原本的不一樣（{what}）：只拆不改字，改字另外用 set_text"));
    let mut k = 0usize;
    for (n, p) in clean_parts(parts).iter().enumerate() {
        let pc: Vec<char> = p.chars().filter(|c| !c.is_whitespace()).collect();
        let fits = |at: usize| a.len() >= at + pc.len() && a[at..at + pc.len()] == pc[..];
        let cands = if n == 0 { vec![k] } else { starts(k) };
        match cands.iter().copied().find(|at| fits(*at)) {
            Some(at) => k = at + pc.len(),
            None => {
                let at = *cands.get(1).unwrap_or(&k);
                let same = a.get(at..).unwrap_or(&[]).iter().zip(&pc).take_while(|(x, y)| x == y).count();
                let theirs: String = p.chars().filter(|c| !c.is_whitespace()).skip(same).take(16).collect();
                return if at + same >= a.len() { fail(format!("拆出來的多了「{theirs}」")) } else { fail(format!("從原本的「{}」起，拆出來的是「{theirs}」", near_o(at + same))) };
            }
        }
    }
    // a full stop or a separator at the very end of the original may be left out
    while k < a.len() && (SEPARATORS.contains(&a[k]) || a[k] == '.' || a[k] == '。') {
        k += 1;
    }
    if k < a.len() {
        return fail(format!("原本的「{}」沒有放進任何一塊", near_o(k)));
    }
    None
}

/// The text runs of `text` in `tpl`'s look and tag; `$…$` as OMML math runs.
fn rich(text: &str, tpl: &Run, env: &Env) -> Result<Vec<Run>, String> {
    let parts: Vec<&str> = text.split('$').collect();
    let mut out = vec![];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 1 && i + 1 < parts.len() {
            let latex = crate::todeck::build::expand_macros(part, &env.macros);
            let omml = (env.math)(&latex, false, tpl.look.size)?;
            let mut look = tpl.look.clone();
            look.font = "Cambria Math".into();
            look.ea_font = "Cambria Math".into();
            out.push(Run { kind: "math".into(), text: latex.clone(), tag: None, em: None, look, field: None, omml: Some(omml), latex: Some(latex), src: None, k: 0 });
        } else {
            out.push(Run { kind: "text".into(), text: part.to_string(), field: None, omml: None, latex: None, src: None, k: 0, ..tpl.clone() });
        }
    }
    Ok(out)
}

/// New paragraphs in the object's own formatting: paragraph i takes the look of the old
/// paragraph i (else the last), its runs the look and tag of that paragraph's first text run.
fn rebuild(old: &TextBody, texts: &[String], env: &Env) -> Result<Vec<Para>, String> {
    let any_run = old.paras.iter().flat_map(|p| p.runs.iter()).find(|r| r.kind == "text").cloned();
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut p = old.paras.get(i).or(old.paras.last()).cloned().unwrap_or_else(|| Para::new(vec![]));
            let tpl = p.runs.iter().find(|r| r.kind == "text").cloned().or_else(|| any_run.clone()).unwrap_or_else(|| Run::text("", "body", p.end.clone().unwrap_or_default()));
            p.src = None;
            p.k = 0;
            p.runs = rich(t, &tpl, env)?;
            Ok(p)
        })
        .collect()
}

/// What one page of a split holds: object id → the whole object (`None`) or these of its
/// paragraphs (0-based, in the box's order).
type Part = BTreeMap<u32, Option<Vec<usize>>>;

/// The groups of a `split_slide` as pages, or why they cannot be: every groupable object in
/// exactly one group, whole or by paragraphs; a box divided by paragraphs holds text, each of
/// its lines in exactly one group, a display equation with the line above it (left out, it
/// follows that line by itself), blank paragraphs with the paragraph before them.
fn split_parts(s: &Slide, h: i64, groups: &[Vec<Member>]) -> Result<Vec<Part>, Vec<String>> {
    let all: BTreeSet<u32> = groupable(s, h).into_iter().collect();
    let mut r = vec![];
    let mut whole: BTreeMap<u32, usize> = BTreeMap::new();
    let mut by_para: BTreeMap<u32, BTreeMap<usize, usize>> = BTreeMap::new();
    let mixed = |id: u32| format!("物件 {id} 有的組給整個、有的組給段落：一個物件只用一種寫法");
    for (gi, g) in groups.iter().enumerate() {
        if g.is_empty() {
            r.push(format!("第 {} 組是空的（只有一個文字框的頁，用 {{\"obj\": id, \"paras\": [n, …]}} 把它的段落分到各組）", gi + 1));
        }
        for m in g {
            let id = m.obj();
            if !all.contains(&id) {
                r.push(format!("物件 {id} 不能分組（不存在，或是標題、頁首、Ref 這類每頁都有的）"));
                continue;
            }
            match m {
                Member::Obj(_) => {
                    if by_para.contains_key(&id) {
                        r.push(mixed(id));
                    } else if let Some(prev) = whole.insert(id, gi) {
                        r.push(format!("物件 {id} 同時在第 {} 組和第 {} 組", prev + 1, gi + 1));
                    }
                }
                Member::Paras { paras, .. } => {
                    if whole.contains_key(&id) {
                        r.push(mixed(id));
                        continue;
                    }
                    let Some(b) = s.find(id).filter(|o| has_text(o)).and_then(body_of) else {
                        r.push(format!("物件 {id} 沒有文字，不能照段落拆：整個放進一組"));
                        continue;
                    };
                    if paras.is_empty() {
                        r.push(format!("物件 {id} 在第 {} 組沒有給段落", gi + 1));
                    }
                    let at = by_para.entry(id).or_default();
                    for n in paras {
                        if *n == 0 || *n > b.paras.len() {
                            r.push(format!("物件 {id} 沒有第 {n} 段（它有 {} 段，從 1 數起）", b.paras.len()));
                        } else if let Some(prev) = at.insert(n - 1, gi).filter(|prev| *prev != gi) {
                            r.push(format!("物件 {id} 的第 {n} 段同時在第 {} 組和第 {} 組", prev + 1, gi + 1));
                        }
                    }
                }
            }
        }
    }
    let missing: Vec<String> = all.iter().filter(|id| !whole.contains_key(id) && !by_para.contains_key(id)).map(|id| id.to_string()).collect();
    if !missing.is_empty() {
        r.push(format!("物件 {} 沒有放進任何一組", missing.join("、")));
    }
    let mut out: Vec<Part> = vec![Part::new(); groups.len()];
    for (id, gi) in &whole {
        out[*gi].insert(*id, None);
    }
    for (id, at) in &by_para {
        let Some(b) = s.find(*id).and_then(body_of) else { continue };
        // (the group of the line above: an equation and a blank paragraph go with it)
        let mut above: Option<usize> = None;
        for (i, p) in b.paras.iter().enumerate() {
            let blank = p.runs.iter().all(|x| x.kind != "math" && x.text.trim().is_empty());
            let g = match (at.get(&i).copied(), above) {
                (_, Some(a)) if blank => Some(a),
                (_, None) if blank => None,
                (Some(g), Some(a)) if is_display(p) && g != a => {
                    r.push(format!("物件 {id} 的第 {} 段是式子，要和它上面那一行放在同一組", i + 1));
                    Some(a)
                }
                (None, Some(a)) if is_display(p) => Some(a),
                (Some(g), _) => Some(g),
                (None, _) => {
                    r.push(format!("物件 {id} 的第 {} 段沒有放進任何一組", i + 1));
                    None
                }
            };
            let Some(g) = g else { continue };
            if (!is_display(p) && !blank) || above.is_none() {
                above = Some(g);
            }
            if let Some(v) = out[g].entry(*id).or_insert_with(|| Some(vec![])) {
                v.push(i);
            }
        }
    }
    if r.is_empty() { Ok(out) } else { Err(r) }
}

/// A slide cut down to one page of a split: the groupable objects of `part` (a box divided by
/// paragraphs with its own paragraphs, cloned as they are), the others as they were (`hidden`: the
/// hidden ones too, on the page that stays).
fn keep_part(s: &mut Slide, grp: &BTreeSet<u32>, part: &Part, hidden: bool) {
    s.objects.retain(|o| if grp.contains(&o.id) { part.contains_key(&o.id) } else { hidden || !o.hidden });
    for o in s.objects.iter_mut() {
        let Some(Some(idx)) = part.get(&o.id) else { continue };
        if let Some(tb) = o.kind.sp_mut().and_then(|sp| sp.text.as_mut()) {
            tb.paras = idx.iter().filter_map(|i| tb.paras.get(*i).cloned()).collect();
        }
    }
}

/// One slide's text as the notation check reads it, and the symbols it uses / its notes define.
fn one_slide(s: &Slide) -> Deck {
    let mut d = Deck::new(1, 1, None);
    d.slides.push(s.clone());
    d
}

fn used_keys(s: &Slide, env: &Env, eff: &Notation) -> BTreeSet<String> {
    let (files, _) = crate::notation::deck_document(&one_slide(s), env.latex);
    let Some((name, text)) = files.into_iter().next() else { return BTreeSet::new() };
    let opt = lore_notation::CheckOptions { caption_defines: true, ..Default::default() };
    let doc = lore_notation::Document::from_files(&[("main.tex".to_string(), format!("\\input{{{name}}}\n")), (format!("{name}.tex"), text)]);
    let scan = lore_notation::scan_document(doc, eff, &opt);
    let syms = lore_notation::symbols::analyze(&scan, &lore_notation::symbols::SymbolOptions { caption_defines: true });
    syms.table.values().filter(|i| !i.occs.is_empty()).map(|i| i.key.clone()).collect()
}

fn notes_keys(notes: &str, eff: &Notation) -> BTreeSet<String> {
    let mut s = Slide::new("s1".into(), None);
    s.notes = notes.to_string();
    crate::notation::notes_definitions(&one_slide(&s), eff).into_iter().map(|(_, k)| k).collect()
}

/// The sentences of speaker notes, the numbering of an item taken off (`$…$` is never cut).
fn sentences(notes: &str) -> Vec<String> {
    let num = regex::Regex::new(r"^\(?\d{1,2}[.)、．]\s*").unwrap();
    let mut out = vec![];
    for line in notes.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let cs: Vec<char> = num.replace(line, "").chars().collect();
        let (mut cur, mut math) = (String::new(), false);
        for (i, c) in cs.iter().enumerate() {
            cur.push(*c);
            if *c == '$' {
                math = !math;
            }
            let end = !math && (matches!(c, '。' | '！' | '？') || (matches!(c, '.' | '!' | '?') && cs.get(i + 1).is_none_or(|n| n.is_whitespace())));
            if end {
                out.push(cur.trim().to_string());
                cur.clear();
            }
        }
        if !cur.trim().is_empty() {
            out.push(cur.trim().to_string());
        }
    }
    out
}

/// The notes of each page of a split with the symbol definitions of the slide's old notes: a
/// sentence there that defines a symbol ("$x$ denotes …", the notes' symbol item of 論文報告) goes,
/// as it is, to every page that uses the symbol and whose own notes do not define it
/// (`build::notes_with_symbols`: one more numbered item). Without this the split would take the
/// definition away from the equation it was written for.
fn carry_symbols(old: &str, pages: &[&Slide], notes: &[String], env: &Env) -> Vec<String> {
    let eff = Notation::effective(env.glob, env.doc);
    let defs: Vec<(BTreeSet<String>, String)> = sentences(old).into_iter().filter(|t| t.contains('$')).map(|t| (notes_keys(&t, &eff), t)).filter(|(k, _)| !k.is_empty()).collect();
    notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let n = n.trim();
            let Some(page) = pages.get(i).filter(|_| !defs.is_empty()) else { return n.to_string() };
            let (used, have) = (used_keys(page, env, &eff), notes_keys(n, &eff));
            let add: Vec<String> = defs.iter().filter(|(k, _)| k.iter().any(|x| used.contains(x) && !have.contains(x))).map(|(_, t)| t.clone()).collect();
            crate::todeck::build::notes_with_symbols(n, &add, &env.lang)
        })
        .collect()
}

fn fresh_uid(deck: &Deck, n: &mut u64) -> String {
    loop {
        let u = format!("s{}", (*n).max(1));
        *n = (*n).max(1) + 1;
        if deck.slide(&u).is_none() {
            return u;
        }
    }
}

/// The ops of one checked edit on `ed`'s deck as it is now, and what it did differently from the
/// plain case (for the author).
pub fn compile(ed: &Editor, e: &Edit, env: &Env) -> Result<(Vec<Op>, Vec<String>), String> {
    let deck = &ed.deck;
    let h = deck.size.h;
    let idx = deck.slide_index(e.slide()).ok_or("找不到投影片")?;
    let s = &deck.slides[idx];
    let mut warn: Vec<String> = vec![];
    let ops = match e {
        Edit::SetText { slide, obj, paras } => {
            let o = s.find(*obj).ok_or("找不到物件")?;
            let old = body_of(o).ok_or("物件沒有文字")?;
            let paras = rebuild(old, paras, env)?;
            let mut ops = vec![Op::SetObj { slide: slide.clone(), id: *obj, path: "/text/paras".into(), value: serde_json::to_value(&paras).map_err(|e| e.to_string())? }];
            // 論文報告 made a 內文 box as high as its text: with more text it grows the same way
            // (`fit_h`), into free room only, and the conclusion line follows; else the code check
            // reports the text over its box
            if let Some(g) = geo(ed, env).filter(|_| obj_role(s, o, h) == "body") {
                let need = fit_h(&TextBody { paras, ..old.clone() }, o.frame.w);
                if need > o.frame.h && o.frame.y + need <= free_bottom(s, &g, &o.frame, &[o.id]) {
                    let f = Frame { h: need, ..o.frame.clone() };
                    ops.push(Op::SetObj { slide: slide.clone(), id: *obj, path: "/frame".into(), value: serde_json::to_value(&f).map_err(|e| e.to_string())? });
                    let mut t = s.clone();
                    if let Some(x) = t.find_mut(*obj) {
                        x.frame = f;
                    }
                    takeaway_follow(&t, &g, slide, &mut ops)?;
                }
            }
            ops
        }
        Edit::SetNotes { slide, text } => vec![Op::SetSlide { slide: slide.clone(), path: "/notes".into(), value: json!(text.trim()) }],
        Edit::SplitSlide { slide, groups, titles, notes } => {
            let grp: BTreeSet<u32> = groupable(s, h).into_iter().collect();
            let parts = split_parts(s, h, groups).map_err(|e| e.join("；"))?;
            let title_id = title_object(s, h).map(|o| o.id).ok_or("沒有標題物件")?;
            let title_paras = |t: &str| -> Result<Vec<Para>, String> { rebuild(s.find(title_id).and_then(body_of).ok_or("標題沒有文字")?, &[t.to_string()], env) };
            let anims_for = |part: &Part, rest: bool| s.anims.iter().filter(|a| if grp.contains(&a.obj) { part.contains_key(&a.obj) } else { rest }).cloned().collect::<Vec<_>>();
            let val = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
            // the pages as slides: the first is this slide, the others are new
            let mut n = deck.next_uid;
            let mut pages: Vec<Slide> = vec![];
            for (gi, part) in parts.iter().enumerate() {
                let mut ns = s.clone();
                if gi > 0 {
                    ns.uid = fresh_uid(deck, &mut n);
                    ns.anims = anims_for(part, false);
                }
                keep_part(&mut ns, &grp, part, gi == 0);
                if let Some(tb) = ns.find_mut(title_id).and_then(|o| o.kind.sp_mut()).and_then(|sp| sp.text.as_mut()) {
                    tb.paras = title_paras(&titles[gi])?;
                }
                pages.push(ns);
            }
            let notes = carry_symbols(&s.notes, &pages.iter().collect::<Vec<_>>(), notes, env);
            // each part laid out again like a freshly built page of its objects
            let pg = geo(ed, env);
            if pg.is_none() {
                warn.push(format!("第 {} 頁拆開後照原本的位置（這份簡報沒有底版，不能重新排版）", idx + 1));
            }
            let mut ops = vec![];
            for o in s.objects.iter().filter(|o| grp.contains(&o.id)) {
                match parts[0].get(&o.id) {
                    None => ops.push(Op::DeleteObj { slide: slide.clone(), id: o.id }),
                    Some(None) => {}
                    Some(Some(_)) => ops.push(Op::SetObj { slide: slide.clone(), id: o.id, path: "/text/paras".into(), value: val(serde_json::to_value(pages[0].find(o.id).and_then(body_of).map(|b| &b.paras)))? }),
                }
            }
            ops.push(Op::SetObj { slide: slide.clone(), id: title_id, path: "/text/paras".into(), value: val(serde_json::to_value(title_paras(&titles[0])?))? });
            ops.push(Op::SetSlide { slide: slide.clone(), path: "/notes".into(), value: json!(notes[0]) });
            if let Some(g) = &pg {
                for (id, f) in reflow(&pages[0], g, idx + 1, &mut warn) {
                    ops.push(Op::SetObj { slide: slide.clone(), id, path: "/frame".into(), value: val(serde_json::to_value(f))? });
                }
            }
            if !s.anims.is_empty() {
                ops.push(Op::SetSlide { slide: slide.clone(), path: "/anims".into(), value: val(serde_json::to_value(anims_for(&parts[0], true)))? });
            }
            for (gi, mut ns) in pages.into_iter().enumerate().skip(1) {
                ns.notes = notes[gi].clone();
                if let Some(geo) = &pg {
                    for (id, f) in reflow(&ns, geo, idx + gi + 1, &mut warn) {
                        if let Some(o) = ns.find_mut(id) {
                            o.frame = f;
                        }
                    }
                }
                ops.push(Op::InsertSlide { index: idx + gi, slide: ns });
            }
            ops
        }
        Edit::InsertSlide { kind, title, blocks, notes, .. } => {
            let (ws, source) = env.build.ok_or("這份簡報沒有底版")?;
            let st = ed.style.as_ref().ok_or("這份簡報沒有底版")?;
            let text = |role: &'static str| blocks.iter().filter(move |b| b.role == role).map(|b| b.text.trim().to_string()).filter(|t| !t.is_empty());
            let ps = plan::PlanSlide { role: kind.clone(), title: title.trim().into(), bullets: text("bullet").collect(), takeaway: text("takeaway").next().unwrap_or_default(), suggest: text("suggest").next().unwrap_or_default(), notes: notes.trim().into(), ..Default::default() };
            let mut n = deck.next_uid;
            let slide = built_slide(ws, st, source, env, ps, fresh_uid(deck, &mut n))?;
            vec![Op::InsertSlide { index: idx + 1, slide }]
        }
        Edit::SplitObject { slide, obj, parts, arrange } => split_object(ed, env, s, slide, *obj, parts, arrange)?,
        Edit::HighlightCrop { slide, picture, mode, region, .. } => highlight_crop(ed, env, s, idx, slide, *picture, mode, region.as_deref().unwrap_or(&[]), &mut warn)?,
        Edit::AddEquation { slide, latex, explains, line, .. } => add_equation(ed, env, s, slide, latex.trim(), *explains, *line)?,
    };
    Ok((ops, warn))
}

// ── Layout of the new kinds (the base's frames, 論文報告's own rules) ──────────

/// The frames a content page of this deck's base gets from 論文報告's build (`None`: no base).
pub struct Geo {
    pub content: Frame,
    pub band: Option<Frame>,
    pub gap: i64,
    pub w: i64,
    pub h: i64,
}

pub fn geo(ed: &Editor, env: &Env) -> Option<Geo> {
    let (ws, source) = env.build?;
    let st = ed.style.as_ref()?;
    let pf = crate::todeck::build::content_frames(ws, st, source);
    let (w, h) = (ed.deck.size.w, ed.deck.size.h);
    Some(Geo { content: pf.content, band: pf.band, gap: h / 60, w, h })
}

/// The height a text body needs at width `w` (`build::text_h`), with the box's own insets (a
/// built box has none; an imported or hand-made one does, and 排版用字 measures inside them).
fn fit_h(b: &TextBody, w: i64) -> i64 {
    crate::todeck::build::text_h(b, w) + b.insets[1] + b.insets[3]
}

fn max_id(objs: &[Object]) -> u32 {
    objs.iter().map(|o| match &o.kind {
        Kind::Group(g) => o.id.max(max_id(&g.children)),
        _ => o.id,
    }).max().unwrap_or(0)
}

fn bottom_of(f: &Frame) -> i64 {
    f.y + f.h
}

fn across(a: &Frame, b: &Frame) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w
}

/// The conclusion line under the content (not the base's dark band, which stays put).
fn takeaway_line<'a>(s: &'a Slide, g: &Geo) -> Option<&'a Object> {
    if g.band.is_some() {
        return None;
    }
    s.objects.iter().find(|o| !o.hidden && obj_role(s, o, g.h) == "takeaway")
}

/// The lowest edge the body's content may reach: the body area, less a conclusion line under it.
fn body_bottom(s: &Slide, g: &Geo) -> i64 {
    bottom_of(&g.content) - takeaway_line(s, g).map(|t| t.frame.h + g.gap).unwrap_or(0)
}

/// How far down `region` may grow: to the body's bottom or the next object under it.
fn free_bottom(s: &Slide, g: &Geo, region: &Frame, skip: &[u32]) -> i64 {
    let mut b = body_bottom(s, g);
    for o in s.objects.iter().filter(|o| !o.hidden && !skip.contains(&o.id)) {
        if matches!(obj_role(s, o, g.h), "title" | "kicker" | "ref" | "chrome" | "takeaway") {
            continue;
        }
        if o.frame.y >= region.y + g.h / 200 && across(&o.frame, region) {
            b = b.min(o.frame.y - g.gap);
        }
    }
    b
}

/// The conclusion line moved under the content when the content now reaches it (as the build
/// puts it: one gap under the lowest edge, inside the body area).
fn takeaway_follow(s: &Slide, g: &Geo, slide: &str, ops: &mut Vec<Op>) -> Result<(), String> {
    let Some(t) = takeaway_line(s, g) else { return Ok(()) };
    let low = s.objects.iter().filter(|o| !o.hidden && o.id != t.id && !matches!(obj_role(s, o, g.h), "title" | "kicker" | "ref" | "chrome")).map(|o| bottom_of(&o.frame)).max().unwrap_or(g.content.y);
    if t.frame.y >= low + g.gap {
        return Ok(());
    }
    let c = &g.content;
    let f = Frame { y: (low + g.gap).min(bottom_of(c) - t.frame.h).max(c.y), ..t.frame.clone() };
    ops.push(Op::SetObj { slide: slide.into(), id: t.id, path: "/frame".into(), value: serde_json::to_value(f).map_err(|e| e.to_string())? });
    Ok(())
}

/// A cropped part (aspect `aspect`) with an optional caption in the body area `full`, by the build's
/// part-beside-text frames (`build::part_frame`), the caption's height kept free under the picture:
/// (picture, caption, text frame).
fn part_layout(full: &Frame, aspect: f64, cap: Option<&TextBody>, has_text: bool, eq_w: i64, gap: i64, h: i64) -> (Frame, Option<Frame>, Frame) {
    let cap_room = cap.map(|b| fit_h(b, full.w * 48 / 100) + h / 120).unwrap_or(0);
    let area = Frame { h: full.h - cap_room, ..full.clone() };
    let (pf, side) = crate::todeck::build::part_frame(&area, aspect, has_text, eq_w, gap);
    let cf = cap.map(|b| Frame::new(pf.x, bottom_of(&pf) + h / 120, pf.w, fit_h(b, pf.w)));
    let low = cf.as_ref().map(bottom_of).unwrap_or(bottom_of(&pf));
    let bottom = bottom_of(full);
    let tf = if side {
        let tx = pf.x + pf.w + 2 * gap;
        Frame::new(tx, full.y, full.x + full.w - tx, (bottom - full.y).max(h / 10))
    } else {
        Frame::new(full.x, low + gap, full.w, (bottom - low - gap).max(0))
    };
    (pf, cf, tf)
}

/// The widest display equation of some text objects (`build::math_w`), to decide a column.
fn eq_widest(objs: &[&Object]) -> i64 {
    objs.iter().filter_map(|o| body_of(o)).flat_map(|b| b.paras.iter()).filter(|p| is_display(p)).map(|p| crate::todeck::build::math_w(p.runs[0].latex.as_deref().unwrap_or(""), p.runs[0].look.size)).max().unwrap_or(0)
}

/// A page laid out again like a freshly built page of its objects (`build::build`'s rules): a
/// table across the top, a faint figure with its clear parts as one unit at the top, a part of a
/// figure (a cropped picture) beside the lines (`part_frame`), a whole picture across or beside
/// with its caption (`picture_frame`), a flow figure across the top, an empty picture slot left
/// with the text right; the text from the top of what is left; the conclusion line one gap under
/// it. The title, kicker, `Ref:` and the layout's objects stay; shapes that are no role content
/// keep their frames (warned). Returns the frames that changed.
pub fn reflow(s: &Slide, g: &Geo, page: usize, warn: &mut Vec<String>) -> Vec<(u32, Frame)> {
    use crate::todeck::build::{picture_frame, text_h};
    let (c, gap, h, w) = (&g.content, g.gap, g.h, g.w);
    let mut out: BTreeMap<u32, Frame> = BTreeMap::new();
    let vis: Vec<&Object> = s.objects.iter().filter(|o| !o.hidden).collect();
    let role = |o: &Object| obj_role(s, o, h);
    let take = takeaway_line(s, g);
    let bottom = body_bottom(s, g);
    let full = Frame::new(c.x, c.y, c.w, bottom - c.y);
    let pics: Vec<&Object> = vis.iter().copied().filter(|o| matches!(o.kind, Kind::Picture(_)) && !matches!(role(o), "chrome" | "title" | "kicker" | "ref")).collect();
    let table = vis.iter().copied().find(|o| matches!(o.kind, Kind::Table(_)));
    let group = vis.iter().copied().find(|o| role(o) == "group");
    let slot = vis.iter().copied().find(|o| o.is_empty_slot() && o.slot.as_deref().is_some_and(|x| x.starts_with("image")));
    let captions: Vec<&Object> = vis.iter().copied().filter(|o| role(o) == "caption").collect();
    // the body, and a box that is equations only (a paired equation under its line)
    let eq_box = |o: &Object| role(o) == "text" && body_of(o).is_some_and(|b| !b.paras.is_empty() && b.paras.iter().all(is_display));
    let mut texts: Vec<&Object> = vis.iter().copied().filter(|o| role(o) == "body" || eq_box(o)).collect();
    for o in vis.iter().filter(|o| matches!(role(o), "shape" | "text") && !o.is_empty_slot() && !eq_box(o)) {
        warn.push(format!("第 {page} 頁的「{}」不是版面裡的角色（自己畫的形狀或文字），留在原本的位置", obj_label(s, o.id)));
    }
    let alpha = |o: &Object| if let Kind::Picture(p) = &o.kind { p.alpha } else { 1.0 };
    let ghost = pics.iter().copied().find(|o| alpha(o) < 0.999);
    // the lines of a walk-through sit on its faint figure: they move with it
    let mut unit: Vec<&Object> = vec![];
    if let Some(gh) = ghost {
        let inside = |o: &Object| o.frame.x >= gh.frame.x && o.frame.y >= gh.frame.y && o.frame.x + o.frame.w <= gh.frame.x + gh.frame.w && bottom_of(&o.frame) <= bottom_of(&gh.frame);
        unit = texts.iter().copied().filter(|o| inside(o)).collect();
        texts.retain(|o| !inside(o));
    }
    let has_text = !texts.is_empty();
    let mut low = c.y;
    let below = |low: i64, by: i64| {
        let y = low + by;
        Frame::new(c.x, y, c.w, (bottom - y).max(0))
    };
    let mut tf = Frame::new(c.x, c.y, c.w, (bottom - c.y).max(h / 10));
    if let Some(t) = table {
        let f = Frame { x: c.x, y: c.y, ..t.frame.clone() };
        low = bottom_of(&f) + h / 80;
        out.insert(t.id, f);
        tf = below(low, gap);
    } else if let Some(gh) = ghost {
        let dy = c.y - gh.frame.y;
        for o in pics.iter().chain(unit.iter()) {
            out.insert(o.id, Frame { y: o.frame.y + dy, ..o.frame.clone() });
        }
        low = bottom_of(&gh.frame) + dy;
        tf = below(low, gap);
    } else if let [p] = pics.as_slice() {
        let crop = if let Kind::Picture(x) = &p.kind { x.crop } else { lore_canvas::Crop::default() };
        let aspect = p.frame.w.max(1) as f64 / p.frame.h.max(1) as f64;
        let cap = captions.first().copied();
        if crop != lore_canvas::Crop::default() {
            let (pf, cf, t) = part_layout(&full, aspect, cap.and_then(|o| body_of(o)), has_text, eq_widest(&texts), gap, h);
            low = cf.as_ref().map(bottom_of).unwrap_or(bottom_of(&pf));
            if let (Some(o), Some(cf)) = (cap, cf) {
                out.insert(o.id, cf);
            }
            tf = t;
            out.insert(p.id, pf);
        } else {
            let cap_body = cap.and_then(|o| body_of(o));
            let cap_full = cap_body.map(|b| text_h(b, full.w) + h / 120).unwrap_or(0);
            let (pw, cap_w, side) = picture_frame(&full, aspect, has_text, cap_full);
            let cap_h = cap_body.map(|b| text_h(b, cap_w) + h / 120).unwrap_or(0);
            let ph = (pw as f64 / aspect) as i64;
            let pf = Frame::new(if side { full.x } else { full.x + (full.w - pw) / 2 }, full.y, pw, ph);
            low = bottom_of(&pf);
            if let Some(o) = cap {
                let cf = Frame::new(full.x, bottom_of(&pf) + h / 120, cap_w, cap_h - h / 120);
                low = bottom_of(&cf);
                out.insert(o.id, cf);
            }
            tf = if side {
                let tx = full.x + pw.max(cap_w) + 2 * gap;
                Frame::new(tx, c.y, (full.x + full.w - tx).max(w / 6), (bottom - c.y).max(h / 10))
            } else {
                below(low, gap)
            };
            out.insert(p.id, pf);
        }
    } else if !pics.is_empty() {
        warn.push(format!("第 {page} 頁有 {} 張圖，圖的位置沒有動", pics.len()));
        low = pics.iter().map(|o| bottom_of(&o.frame)).max().unwrap_or(c.y);
        tf = below(low, gap);
    } else if let Some(gp) = group {
        let f = Frame { x: c.x + (c.w - gp.frame.w) / 2, y: c.y, ..gp.frame.clone() };
        low = bottom_of(&f);
        out.insert(gp.id, f);
        tf = below(low, 2 * gap);
    } else if let Some(sl) = slot {
        let sw = c.w * 52 / 100;
        let f = Frame::new(c.x, c.y, sw, (bottom - c.y).min(sw * 10 / 16));
        out.insert(sl.id, f);
        let tx = c.x + sw + 2 * gap;
        tf = Frame::new(tx, c.y, (c.x + c.w - tx).max(w / 6), (bottom - c.y).max(h / 10));
    }
    match texts.as_slice() {
        [] => {}
        [t] => {
            let need = body_of(t).map(|b| fit_h(b, tf.w)).unwrap_or(h / 20);
            let f = Frame::new(tf.x, tf.y, tf.w, need.min(tf.h.max(h / 20)).max(h / 20));
            low = low.max(bottom_of(&f));
            out.insert(t.id, f);
        }
        many => {
            // boxes laid out together (columns, a split line): moved as one block
            let (bx, by) = (many.iter().map(|o| o.frame.x).min().unwrap_or(0), many.iter().map(|o| o.frame.y).min().unwrap_or(0));
            let (br, bb) = (many.iter().map(|o| o.frame.x + o.frame.w).max().unwrap_or(0), many.iter().map(|o| bottom_of(&o.frame)).max().unwrap_or(0));
            if br - bx <= tf.w + h / 200 {
                let (dx, dy) = (tf.x - bx, tf.y - by);
                for o in many {
                    out.insert(o.id, Frame { x: o.frame.x + dx, y: o.frame.y + dy, ..o.frame.clone() });
                }
                low = low.max(bb + dy);
            } else {
                warn.push(format!("第 {page} 頁的文字框比文字區寬，位置沒有動"));
                low = low.max(bb);
            }
        }
    }
    if let Some(t) = take {
        let f = Frame::new(c.x, (low + gap).min(bottom_of(c) - t.frame.h).max(c.y), c.w, t.frame.h);
        out.insert(t.id, f);
    }
    out.into_iter().filter(|(id, f)| s.find(*id).is_some_and(|o| &o.frame != f)).collect()
}

/// `split_object`: the parts as boxes in the object's place (its x, y and width, down into the
/// free body area), in the object's own formatting, by the arrangement's cells with the build's
/// gutters (`2 × gap` across, as the paired columns; `gap` down). Nothing shrinks: a part that does
/// not fit its cell is a refusal.
fn split_object(ed: &Editor, env: &Env, s: &Slide, slide: &str, obj: u32, parts: &[String], arrange: &str) -> Result<Vec<Op>, String> {
    let g = geo(ed, env).ok_or("這份簡報沒有底版")?;
    let o = s.find(obj).ok_or("找不到物件")?;
    let tb = body_of(o).ok_or("物件沒有文字")?;
    let parts = clean_parts(parts);
    let bodies = parts.iter().map(|p| Ok(TextBody { paras: rebuild(tb, std::slice::from_ref(p), env)?, ..tb.clone() })).collect::<Result<Vec<_>, String>>()?;
    let cols: usize = match arrange {
        "row" => parts.len(),
        "column" => 1,
        _ => 2,
    };
    let (gx, gy) = (2 * g.gap, g.gap);
    let cw = (o.frame.w - (cols as i64 - 1) * gx) / cols as i64;
    let measure = if cols > 1 { cw * 88 / 100 } else { cw };
    let hs: Vec<i64> = bodies.iter().map(|b| fit_h(b, measure)).collect();
    let mut frames = vec![];
    let mut y = o.frame.y;
    for (r, row) in hs.chunks(cols).enumerate() {
        let rh = row.iter().copied().max().unwrap_or(0);
        for k in 0..row.len() {
            frames.push(Frame::new(o.frame.x + k as i64 * (cw + gx), y, cw, rh));
        }
        y += rh + if (r + 1) * cols < hs.len() { gy } else { 0 };
    }
    let bottom = free_bottom(s, &g, &o.frame, &[obj]);
    if y > bottom + g.h / 100 {
        let pct = |v: i64| v as f64 * 100.0 / g.h as f64;
        let other: Vec<&str> = ARRANGE.iter().copied().filter(|a| *a != arrange && (*a != "2x2" || parts.len() == 4)).collect();
        let narrow = if cols > 1 { format!("，每格寬 {:.0} % 頁寬", cw as f64 * 100.0 / g.w as f64) } else { String::new() };
        return Err(format!("排成 {arrange} 要 {:.0} % 頁高、這裡只有 {:.0} %{narrow}（不縮字）：每塊少幾個字，或改用 {}", pct(y - o.frame.y), pct(bottom - o.frame.y), if other.is_empty() { "split_slide".to_string() } else { other.join(" 或 ") }));
    }
    let at = s.objects.iter().position(|x| x.id == obj).ok_or("找不到物件")?;
    let mut next = max_id(&s.objects) + 1;
    let mut ops = vec![];
    let mut after = s.clone();
    after.objects.retain(|x| x.id != obj);
    for (k, (b, f)) in bodies.into_iter().zip(frames).enumerate() {
        let mut n = o.clone();
        n.name = format!("{} {}", o.name, k + 1);
        n.frame = f;
        if let Some(sp) = n.kind.sp_mut() {
            sp.text = Some(b);
        }
        if k == 0 {
            ops.push(Op::SetObj { slide: slide.into(), id: obj, path: String::new(), value: serde_json::to_value(&n).map_err(|e| e.to_string())? });
        } else {
            n.id = next;
            next += 1;
            n.slot = None;
            n.src = None;
            ops.push(Op::InsertObj { slide: slide.into(), parent: None, index: at + k, obj: n.clone() });
        }
        after.objects.push(n);
    }
    takeaway_follow(&after, &g, slide, &mut ops)?;
    Ok(ops)
}

/// The box's edges moved to the blank gutters of the picture (a PNG in the deck's assets), each
/// on its own axis (`snap_cut`, a window of 4 % of the image): how many moved, `None` when the
/// picture cannot be read.
fn snap_box(env: &Env, asset: &str, base: lore_canvas::Crop, b: &mut [f64; 4]) -> Option<usize> {
    const SNAP: f64 = 0.04;
    use lore_docsrc::crop::{Axis, ink_profile, snap_cut};
    let rel = asset.strip_prefix("file:").filter(|r| !r.contains("..") && !r.starts_with('/'))?;
    let bytes = std::fs::read(env.dir.join(rel)).ok().filter(|b| b.starts_with(b"\x89PNG"))?;
    let (px, py) = (ink_profile(&bytes, Axis::Vertical).ok()?, ink_profile(&bytes, Axis::Horizontal).ok()?);
    let (sw, sh) = (1.0 - base.l - base.r, 1.0 - base.t - base.b);
    let mut moved = 0;
    let mut nb = *b;
    for (k, (prof, off, span)) in [(&px, base.l, sw), (&py, base.t, sh), (&px, base.l, sw), (&py, base.t, sh)].into_iter().enumerate() {
        // an edge on the picture's own border stays there
        if b[k] < 0.02 || b[k] > 0.98 {
            continue;
        }
        let (c, ok) = snap_cut(prof, off + b[k] * span, SNAP);
        let v = ((c - off) / span.max(1e-6)).clamp(0.0, 1.0);
        // the middle of a wide blank band can be far away: only a move within the window counts
        if ok && (c - (off + b[k] * span)).abs() <= SNAP && (v - b[k]).abs() > 1e-9 {
            nb[k] = v;
            moved += 1;
        }
    }
    // a snap that made the box too small or turned it over is not taken
    if box_problem(&nb).is_none() {
        *b = nb;
        Some(moved)
    } else {
        Some(0)
    }
}

/// `highlight_crop`: the box snapped to the picture's gutters; `highlight` = the picture at the
/// style's ghost alpha with the box clear over it (`build::clear_piece`); `crop` = the picture
/// shows only the box (`srcRect`), beside the lines (`build::part_frame`) when the slide is this
/// picture and one 內文, else as large as its aspect allows in the picture's own frame.
#[allow(clippy::too_many_arguments)]
fn highlight_crop(ed: &Editor, env: &Env, s: &Slide, idx: usize, slide: &str, picture: u32, mode: &str, region: &[f64], warn: &mut Vec<String>) -> Result<Vec<Op>, String> {
    use crate::todeck::build::{clear_piece, contain};
    let o = s.find(picture).ok_or("找不到物件")?;
    let Kind::Picture(p) = &o.kind else { return Err("不是圖片".into()) };
    let mut b: [f64; 4] = region.try_into().map_err(|_| "box 要有 4 個數")?;
    match snap_box(env, &p.asset, p.crop, &mut b) {
        Some(n) if n > 0 => {}
        Some(_) => warn.push(format!("第 {} 頁的框：邊旁邊沒有空白可以對齊，照模型給的位置", idx + 1)),
        None => warn.push(format!("第 {} 頁的框：讀不到這張圖的像素（不是簡報資料夾裡的 PNG），沒有對齊空白，照模型給的位置", idx + 1)),
    }
    let set = |id: u32, path: &str, v: Value| Op::SetObj { slide: slide.into(), id, path: path.into(), value: v };
    let mut ops = vec![];
    if mode == "highlight" {
        let st = ed.style.as_ref().ok_or("沒有底版")?;
        let ghost = st.pack.ghost.picture_alpha.clamp(0.1, 1.0);
        ops.push(set(picture, "/alpha", json!(p.alpha.min(ghost))));
        let mut piece = clear_piece(&p.asset, &o.frame, b, p.crop, "清楚的一段");
        piece.id = max_id(&s.objects) + 1;
        let at = s.objects.iter().position(|x| x.id == picture).ok_or("找不到物件")?;
        ops.push(Op::InsertObj { slide: slide.into(), parent: None, index: at + 1, obj: piece });
        return Ok(ops);
    }
    let (sw, sh) = (1.0 - p.crop.l - p.crop.r, 1.0 - p.crop.t - p.crop.b);
    let crop = lore_canvas::Crop { l: p.crop.l + b[0] * sw, t: p.crop.t + b[1] * sh, r: 1.0 - (p.crop.l + b[2] * sw), b: 1.0 - (p.crop.t + b[3] * sh) };
    ops.push(set(picture, "/crop", serde_json::to_value(crop).map_err(|e| e.to_string())?));
    let (rw, rh) = (o.frame.w as f64 * (b[2] - b[0]), o.frame.h as f64 * (b[3] - b[1]));
    let aspect = rw.max(1.0) / rh.max(1.0);
    let h = s_h(ed);
    let g = geo(ed, env);
    // the slide's own content besides the picture: beside the lines only when it is one 內文
    let others: Vec<&Object> = s.objects.iter().filter(|x| !x.hidden && x.id != picture && !matches!(obj_role(s, x, h), "title" | "kicker" | "ref" | "chrome" | "takeaway" | "caption")).collect();
    let caps: Vec<&Object> = s.objects.iter().filter(|x| !x.hidden && obj_role(s, x, h) == "caption").collect();
    match (&g, others.as_slice(), caps.len()) {
        (Some(g), [t], 0 | 1) if obj_role(s, t, h) == "body" => {
            let (c, gap) = (&g.content, g.gap);
            let bottom = body_bottom(s, g);
            let full = Frame::new(c.x, c.y, c.w, bottom - c.y);
            let cap = caps.first().copied();
            let (pf, cf, tf) = part_layout(&full, aspect, cap.and_then(|o| body_of(o)), true, eq_widest(&[t]), gap, h);
            let need = body_of(t).map(|b| fit_h(b, tf.w)).unwrap_or(h / 20);
            let f = Frame::new(tf.x, tf.y, tf.w, need.min(tf.h.max(h / 20)).max(h / 20));
            ops.push(set(picture, "/frame", serde_json::to_value(&pf).map_err(|e| e.to_string())?));
            ops.push(set(t.id, "/frame", serde_json::to_value(&f).map_err(|e| e.to_string())?));
            if let (Some(o), Some(cf)) = (cap, &cf) {
                ops.push(set(o.id, "/frame", serde_json::to_value(cf).map_err(|e| e.to_string())?));
            }
            // the conclusion line one gap under the content, as the build puts it
            if let Some(tk) = takeaway_line(s, g) {
                let low = bottom_of(&pf).max(bottom_of(&f)).max(cf.as_ref().map(bottom_of).unwrap_or(0));
                let nf = Frame::new(c.x, (low + gap).min(bottom_of(c) - tk.frame.h).max(c.y), c.w, tk.frame.h);
                if nf != tk.frame {
                    ops.push(set(tk.id, "/frame", serde_json::to_value(&nf).map_err(|e| e.to_string())?));
                }
            }
        }
        _ => {
            let pf = contain(&o.frame, rw.max(1.0) as u32, rh.max(1.0) as u32);
            ops.push(set(picture, "/frame", serde_json::to_value(&pf).map_err(|e| e.to_string())?));
            // a caption right under the picture follows it up
            for cap in s.objects.iter().filter(|x| !x.hidden && obj_role(s, x, h) == "caption" && across(&x.frame, &o.frame)) {
                let gap_was = cap.frame.y - bottom_of(&o.frame);
                if (-h / 50..=h / 20).contains(&gap_was) {
                    let nf = Frame { y: bottom_of(&pf) + h / 120, ..cap.frame.clone() };
                    ops.push(set(cap.id, "/frame", serde_json::to_value(&nf).map_err(|e| e.to_string())?));
                }
            }
        }
    }
    Ok(ops)
}

fn s_h(ed: &Editor) -> i64 {
    ed.deck.size.h
}

/// `add_equation` (display): the equation paragraph in 論文報告's look (tag `equation` when the
/// style has it, centred, 6 pt before and after) right under its line in the same box, the box
/// grown to its text; with one paired equation already in that box and both narrow enough for a
/// column, the two pairs as the paired columns (`build::paired_columns`) and the other lines under
/// them, as the build sets two paired equations. Nothing shrinks: no room is a refusal.
fn add_equation(ed: &Editor, env: &Env, s: &Slide, slide: &str, latex: &str, explains: Option<u32>, line: Option<u32>) -> Result<Vec<Op>, String> {
    use crate::todeck::build::{EMU_IN, math_w, paired_columns};
    let g = geo(ed, env).ok_or("這份簡報沒有底版")?;
    let st = ed.style.as_ref().ok_or("這份簡報沒有底版")?;
    let h = g.h;
    let (o, at) = eq_target(s, h, explains, line)?;
    let tb = body_of(o).ok_or("物件沒有文字")?;
    let eq_tag = ["equation", "body"].into_iter().find(|t| st.tag(t).is_some()).unwrap_or("body");
    let look = st.look(eq_tag, None).unwrap_or_default();
    let x = crate::todeck::build::expand_macros(latex, &env.macros);
    let omml = (env.math)(&x, true, look.size)?;
    let run = Run { kind: "math".into(), text: x.clone(), tag: st.tag("equation").map(|_| "equation".to_string()), em: None, look: look.clone(), field: None, omml: Some(omml), latex: Some(x.clone()), src: None, k: 0 };
    let mut eq = Para::new(vec![run]);
    eq.end = Some(look.clone());
    eq.align = "ctr".into();
    eq.before_pt = Some(6.0);
    eq.after_pt = Some(6.0);
    let bottom = free_bottom(s, &g, &o.frame, &[o.id]);
    let mut ops = vec![];
    let mut after = s.clone();
    // one paired equation already in the box: two columns when both fit one
    let old: Vec<usize> = (0..tb.paras.len()).filter(|i| is_display(&tb.paras[*i]) && *i >= 1 && !is_display(&tb.paras[*i - 1])).collect();
    if let [ei] = old.as_slice() {
        let tf = Frame::new(o.frame.x, o.frame.y, o.frame.w, bottom - o.frame.y);
        let col_w = (tf.w - 2 * g.gap) / 2;
        let wide = |p: &Para| math_w(p.runs[0].latex.as_deref().unwrap_or(""), p.runs[0].look.size);
        if wide(&tb.paras[*ei]) <= col_w - EMU_IN / 4 && math_w(&x, look.size) <= col_w - EMU_IN / 4 {
            let mut pairs = vec![(ei - 1, tb.paras[*ei].clone()), (at, eq.clone())];
            pairs.sort_by_key(|(l, _)| *l);
            // the box's own properties (insets, anchor) on every new box: they line up with the rest
            let like = |paras: Vec<Para>| TextBody { paras, ..tb.clone() };
            let cols: Vec<(TextBody, TextBody)> = pairs.iter().map(|(l, e)| (like(vec![tb.paras[*l].clone()]), like(vec![e.clone()]))).collect();
            let rest: Vec<Para> = tb.paras.iter().enumerate().filter(|(i, _)| *i != *ei && *i != ei - 1 && *i != at).map(|(_, p)| p.clone()).collect();
            let (mut objs, ch) = paired_columns(&tf, g.gap, cols);
            let mut low = tf.y + ch;
            if !rest.is_empty() {
                let rb = like(rest);
                let ry = tf.y + ch + g.gap;
                let need = fit_h(&rb, tf.w);
                objs.push(crate::todeck::build::text_obj(crate::todeck::build::PAIR_TEXT, Frame::new(tf.x, ry, tf.w, need.max(h / 20)), rb, Some("text-3")));
                low = ry + need;
            }
            if low > bottom + h / 100 {
                return Err(format!("兩條式子分兩欄放不下（要 {:.0} % 頁高、只有 {:.0} %，不縮字）：先刪短或拆頁", (low - tf.y) as f64 * 100.0 / h as f64, (bottom - tf.y) as f64 * 100.0 / h as f64));
            }
            let pos = s.objects.iter().position(|x| x.id == o.id).ok_or("找不到物件")?;
            let mut next = max_id(&s.objects) + 1;
            ops.push(Op::DeleteObj { slide: slide.into(), id: o.id });
            after.objects.retain(|x| x.id != o.id);
            for (k, mut n) in objs.into_iter().enumerate() {
                n.id = next;
                next += 1;
                ops.push(Op::InsertObj { slide: slide.into(), parent: None, index: pos + k, obj: n.clone() });
                after.objects.push(n);
            }
            takeaway_follow(&after, &g, slide, &mut ops)?;
            return Ok(ops);
        }
    }
    let mut paras = tb.paras.clone();
    paras.insert(at + 1, eq);
    let nb = TextBody { paras: paras.clone(), ..tb.clone() };
    let need = fit_h(&nb, o.frame.w);
    if o.frame.y + need > bottom + h / 100 {
        return Err(format!("加了式子，這個框要 {:.0} % 頁高、只有 {:.0} %（不縮字）：先刪短或拆頁", need as f64 * 100.0 / h as f64, (bottom - o.frame.y) as f64 * 100.0 / h as f64));
    }
    let f = Frame { h: need.max(o.frame.h), ..o.frame.clone() };
    ops.push(Op::SetObj { slide: slide.into(), id: o.id, path: "/text/paras".into(), value: serde_json::to_value(&paras).map_err(|e| e.to_string())? });
    ops.push(Op::SetObj { slide: slide.into(), id: o.id, path: "/frame".into(), value: serde_json::to_value(&f).map_err(|e| e.to_string())? });
    if let Some(x) = after.find_mut(o.id) {
        x.frame = f;
    }
    takeaway_follow(&after, &g, slide, &mut ops)?;
    Ok(ops)
}

/// The dependency on `todeck/build.rs` for whole pages: a page of `ps.role` built by 論文報告's own build
/// (starter objects, `page_frames`, the academic arrangements, the role tags, the empty picture slot
/// with its suggestion), through its public `build` with a one-slide plan and no material.
fn built_slide(ws: &sc_core::Workspace, st: &lore_canvas::StyleSystem, source: &lore_ooxml::Deck, env: &Env, ps: plan::PlanSlide, uid: String) -> Result<Slide, String> {
    use crate::todeck::build::{BuildInput, build};
    let p = plan::Plan { slides: vec![ps], ..Default::default() };
    let m = crate::todeck::source::Material::default();
    let mut u = Some(uid);
    let mut next = move || u.take().unwrap_or_default();
    let mut progress = |_: usize, _: usize| {};
    let built = build(&p, &m, BuildInput { ws, st, source, deck_dir: env.dir, pandoc: env.pandoc, macros: env.macros.clone(), lang: &env.lang, parts: None, presenter: None, supervisor: None, next_uid: &mut next, progress: &mut progress })?;
    built.slides.into_iter().next().ok_or_else(|| "沒有做出新頁".to_string())
}

// ── Code checks ──────────────────────────────────────────────────────────────

/// (code, subject) → message of the deck's notation errors and warnings (`notation::view`, notes
/// definitions counted), the infos left out.
/// A refusal in the author's words: 「第 1 個修改（set_text）：…」 is how the model is told (it needs
/// the index and the edit's name to correct its plan); he reads 「改文字：…」.
fn for_author(reason: &str) -> String {
    const NAMES: [(&str, &str); 7] = [
        ("set_text", "改文字"),
        ("set_notes", "改備忘稿"),
        ("split_slide", "拆頁"),
        ("insert_slide", "插入新頁"),
        ("split_object", "拆成多格"),
        ("highlight_crop", "裁圖或標亮"),
        ("add_equation", "加公式"),
    ];
    let mut out = reason.to_string();
    if let Some(rest) = out.strip_prefix("第 ") {
        if let Some((_, tail)) = rest.split_once(" 個修改（") {
            if let Some((kind, msg)) = tail.split_once("）：") {
                let name = NAMES.iter().find(|(k, _)| *k == kind).map(|(_, n)| *n).unwrap_or("這個修改");
                out = format!("{name}：{msg}");
            }
        }
    }
    for (k, n) in NAMES {
        out = out.replace(k, n);
    }
    // an object's number means nothing to him: the model pointed at something that is not there
    if let Some(at) = out.find("沒有可以放字的物件") {
        let tail = &out[at + "沒有可以放字的物件".len()..];
        let rest = tail.trim_start().trim_start_matches(|c: char| c.is_ascii_digit());
        out = format!("{}找不到要改的那段文字{}", &out[..at], rest);
    }
    out
}

fn notation_keys(deck: &Deck, env: &Env) -> BTreeMap<(String, String), String> {
    notation_issues(deck, env).into_iter().map(|(k, i)| (k, i["message"].as_str().unwrap_or("").to_string())).collect()
}

/// The same, each with the whole issue (its places).
fn notation_issues(deck: &Deck, env: &Env) -> BTreeMap<(String, String), Value> {
    let v = crate::notation::view(deck, env.latex, env.glob, env.doc);
    v["report"]["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| matches!(i["severity"].as_str(), Some("error" | "warning")))
        .map(|i| ((i["code"].as_str().unwrap_or("").to_string(), i["subject"].as_str().unwrap_or("").to_string()), i.clone()))
        .collect()
}

/// A new 「先用後定義」 that the edit did not make: before it, every line that uses the symbol
/// already stood on an earlier slide than every line that defines it (a line of a slide the edit
/// changed is looked up by its text on the changed slides as they were). The check counts the
/// distance in sentences across slides (a display equation may use a symbol defined up to three
/// sentences later, lore-notation's `grace_sentences`), so a split that puts one more page between
/// the two makes it speak about an order that was the same before. Such an issue does not refuse
/// the edit; the author is told (the returned line) and the deck's 對照表 shows it. A use and a
/// definition that shared a slide, or any other new issue, still refuse.
fn order_was_so(issue: &Value, was: &Deck, now: &Deck, changed: &[String], env: &Env) -> Option<String> {
    // (the same order read from the equation's side: 「方程式的變數到後面才定義」)
    if !matches!(issue["code"].as_str(), Some("used_before_definition" | "equation_undefined_variables")) || issue["subjects"].as_array().is_some_and(|a| !a.is_empty()) {
        return None;
    }
    let page = |l: &Value| l["file"].as_str().and_then(|f| f.trim_end_matches(".tex").strip_prefix("slide-")).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=now.slides.len()).contains(n));
    let uses = issue["locations"].as_array()?;
    // where the definitions are: the issue's related places, or (the equation's issue) each
    // variable's 「（slide-03.tex:4 才定義）」
    let defs: Vec<Value> = if issue["code"] == "equation_undefined_variables" {
        let at = regex::Regex::new(r"（(slide-\d+\.tex):(\d+) 才定義）").unwrap();
        let d: Vec<Option<Value>> = issue["details"].as_array()?.iter().map(|x| at.captures(x.as_str()?).map(|c| json!({ "file": &c[1], "line": c[2].parse::<u64>().unwrap_or(0) }))).collect();
        d.into_iter().collect::<Option<Vec<Value>>>()?
    } else {
        issue["related"].as_array()?.clone()
    };
    if uses.is_empty() || defs.is_empty() {
        return None;
    }
    let (new_files, _) = crate::notation::deck_document(now, env.latex);
    let (old_files, _) = crate::notation::deck_document(was, env.latex);
    // the slides (0-based, as they were) that held the line at `l`: an untouched slide is itself
    let held = |l: &Value| -> Option<Vec<usize>> {
        let n = page(l)?;
        let uid = &now.slides[n - 1].uid;
        if !changed.contains(uid) {
            return was.slide_index(uid).map(|i| vec![i]);
        }
        let text = new_files.get(n - 1)?.1.lines().nth((l["line"].as_u64()? as usize).checked_sub(1)?)?;
        let v: Vec<usize> = was.slides.iter().enumerate().filter(|(i, s)| changed.contains(&s.uid) && old_files.get(*i).is_some_and(|(_, t)| t.lines().any(|x| x == text))).map(|(i, _)| i).collect();
        (!v.is_empty()).then_some(v)
    };
    let (mut last_use, mut first_def, mut def_page) = (0, usize::MAX, 0);
    for u in uses {
        last_use = last_use.max(held(u)?.into_iter().max()?);
    }
    for d in &defs {
        first_def = first_def.min(held(d)?.into_iter().min()?);
        def_page = def_page.max(page(d)?);
    }
    if last_use >= first_def {
        return None;
    }
    let what = issue["subject"].as_str().map(|k| format!("符號 ${k}$ 在第 {} 頁先用到", page(&uses[0]).unwrap_or(0))).unwrap_or_else(|| format!("第 {} 頁的式子用到的符號", page(&uses[0]).unwrap_or(0)));
    Some(format!("{what}，第 {def_page} 頁才定義（修改前也是後面的頁才定義，這次只是又隔了一頁）"))
}

/// Colliding objects of one slide (`deckpolish::overlaps`): (object, other object or `None` for
/// the page edge, kind).
fn overlaps_of(s: &Slide, page: (i64, i64)) -> Vec<(u32, Option<u32>, &'static str)> {
    crate::deckpolish::overlaps(&s.objects, page)
        .into_iter()
        .filter(|o| o.area > 0.0005)
        .map(|o| (o.a, match o.b { crate::deckpolish::Peer::Obj(b) => Some(b), crate::deckpolish::Peer::Page => None }, o.kind))
        .collect()
}

fn obj_label(s: &Slide, id: u32) -> String {
    s.find(id).map(|o| body_of(o).map(|b| plan::clip(b.plain_text().trim(), 16)).filter(|t| !t.is_empty()).unwrap_or_else(|| o.name.clone())).unwrap_or_else(|| id.to_string())
}

/// Whether a text object's text is over its box as 排版用字's overflow check counts it
/// (`deckpolish::check_overflow`): the estimate over the inner height, and more than one line. A
/// single line is never over its box: the base's title frame is lower than one title line with its
/// insets (32 pt × 1.2 in a 42 pt frame with 3.6 pt insets reads 110 %), which is how the base
/// draws every title, not an overflow.
fn over_box(o: &Object) -> Option<f64> {
    let b = body_of(o)?;
    let (_, lines) = crate::deckpolish::text_size(b, o.frame.w);
    crate::deckpolish::fill_ratio(o).filter(|r| *r > 1.05 && lines > 1.0)
}

/// What the edit broke on the slides it touched: text over its box (the overflow estimate) and
/// objects that now collide or leave the page; what was already so before is not counted.
pub fn code_findings(before: &Deck, after: &Deck, touched: &[String]) -> Vec<Finding> {
    let page = (after.size.w, after.size.h);
    let mut out = vec![];
    for (n, s) in after.slides.iter().enumerate().filter(|(_, s)| touched.contains(&s.uid)) {
        let old = before.slide(&s.uid);
        // a slide the edit made (a split's part, an inserted page) has no earlier self: a box it
        // took from a slide handed over (same id) is measured against that box, so what the
        // estimate already said there is not this edit's finding
        let peers: Vec<&Slide> = if old.is_some() { vec![] } else { touched.iter().filter_map(|u| before.slide(u)).collect() };
        for o in s.objects.iter().filter(|o| !o.hidden && !o.is_empty_slot() && !chrome_ph(o)) {
            let Some(r) = over_box(o) else { continue };
            if body_of(o).is_none_or(|b| b.plain_text().trim().is_empty()) {
                continue;
            }
            let was = old.and_then(|x| x.find(o.id)).and_then(over_box).is_some() || peers.iter().filter_map(|x| x.find(o.id)).filter_map(over_box).any(|b| r <= b + 0.02);
            if !was {
                out.push(Finding { slide: s.uid.clone(), page: Some(n + 1), kind: "overflow".into(), text: format!("「{}」的字超出框（約 {:.0}%）", obj_label(s, o.id), r * 100.0) });
            }
        }
        let had: BTreeSet<(u32, Option<u32>, &str)> = old.map(|x| overlaps_of(x, page)).unwrap_or_default().into_iter().collect();
        for (a, b, kind) in overlaps_of(s, page) {
            if had.contains(&(a, b, kind)) {
                continue;
            }
            let text = match b {
                Some(b) => format!("「{}」和「{}」疊在一起", obj_label(s, a), obj_label(s, b)),
                None => format!("「{}」超出頁面", obj_label(s, a)),
            };
            out.push(Finding { slide: s.uid.clone(), page: Some(n + 1), kind: if b.is_some() { "overlap".into() } else { "off-page".into() }, text });
        }
    }
    out
}

// ── What the model sees ──────────────────────────────────────────────────────

fn para_text(p: &Para, latex: &BTreeMap<String, String>) -> String {
    p.runs
        .iter()
        .map(|r| match r.kind.as_str() {
            "math" => format!("${}$", r.latex.clone().or_else(|| r.omml.as_ref().and_then(|o| latex.get(&format!("{:016x}", lore_canvas::fnv(o.as_bytes()))).cloned())).unwrap_or_else(|| "?".into())),
            "br" => " ".into(),
            _ => r.text.clone(),
        })
        .collect()
}

fn frac(v: i64, of: i64) -> f64 {
    (v as f64 / of.max(1) as f64 * 1000.0).round() / 1000.0
}

/// The slides as the model reads them: per object its id, role, slot, frame (fractions of the
/// page), text with `$latex$`, picture asset and crop, whether a split may group it and the limits
/// of its role; the notes; the neighbouring titles.
pub fn context(deck: &Deck, latex: &BTreeMap<String, String>, targets: &[String]) -> Value {
    let (w, h) = (deck.size.w, deck.size.h);
    let roles = page_roles(deck);
    let titles: Vec<String> = deck.slides.iter().map(|s| crate::deckstyle::title_text(s, h)).collect();
    let mut slides = vec![];
    let mut near: BTreeSet<usize> = BTreeSet::new();
    for t in targets {
        let Some(i) = deck.slide_index(t) else { continue };
        near.extend(i.saturating_sub(2)..(i + 3).min(deck.slides.len()));
        let s = &deck.slides[i];
        let page_role = roles.get(i).map(String::as_str).unwrap_or("");
        let grp: BTreeSet<u32> = groupable(s, h).into_iter().collect();
        let objects: Vec<Value> = s
            .objects
            .iter()
            .map(|o| {
                let role = obj_role(s, o, h);
                let mut v = json!({ "id": o.id, "role": role, "frame": [frac(o.frame.x, w), frac(o.frame.y, h), frac(o.frame.w, w), frac(o.frame.h, h)], "groupable": grp.contains(&o.id) });
                if let Some(sl) = &o.slot {
                    v["slot"] = json!(sl);
                }
                if o.hidden {
                    v["hidden"] = json!(true);
                }
                if o.is_empty_slot() {
                    v["empty_slot"] = json!(o.name);
                }
                if let Some(b) = body_of(o) {
                    v["text"] = json!(b.paras.iter().map(|p| para_text(p, latex)).collect::<Vec<_>>());
                }
                if let Kind::Picture(p) = &o.kind {
                    v["picture"] = json!({ "asset": p.asset, "crop": [p.crop.l, p.crop.t, p.crop.r, p.crop.b] });
                }
                if let Some((np, nc)) = limits(role, page_role, s).filter(|_| has_text(o)) {
                    v["limit"] = json!({ "paras": np, "chars": nc });
                }
                v
            })
            .collect();
        slides.push(json!({ "slide": s.uid, "page": i + 1, "role": page_role, "notes": s.notes, "objects": objects }));
    }
    let neighbours: Vec<Value> = near.into_iter().filter(|i| !targets.contains(&deck.slides[*i].uid)).map(|i| json!({ "page": i + 1, "title": titles[i] })).collect();
    json!({ "pages": deck.slides.len(), "slides": slides, "neighbours": neighbours })
}

pub const EDIT_SYSTEM: &str = "You change one or two slides of a presenter's talk while he rehearses (L-Slide). \
You never write coordinates, sizes, fonts, colours or transparency: you return a small plan of edits and the program lays it out with the deck's own frames and text styles. \
Return JSON {\"edits\": [...]}; each edit is one of\n\
- {\"edit\":\"set_text\",\"slide\":uid,\"obj\":id,\"paras\":[\"…\"]}: the whole text of an object that holds text (its paragraphs, in its own formatting);\n\
- {\"edit\":\"set_notes\",\"slide\":uid,\"text\":\"…\"}: the slide's speaker notes;\n\
- {\"edit\":\"split_slide\",\"slide\":uid,\"groups\":[[member,…],…],\"titles\":[\"…\"],\"notes\":[\"…\"]}: the slide becomes as many slides as groups, in order; every object marked groupable in exactly one group, one title and one notes text per group (the other objects, such as the title box, stay on every part). A member is an object id, or {\"obj\":id,\"paras\":[n,…]} to divide the paragraphs of one text box between the slides: n counts the entries of that object's text from 1, every paragraph of the box goes to exactly one group, an equation paragraph stays with the line above it. A slide that is a title and one text box (several definitions or points in one box) is split this way, never with an empty group. Symbol definitions in the old notes are kept by the program on the slides that use the symbol;\n\
- {\"edit\":\"insert_slide\",\"after\":uid,\"kind\":page kind,\"title\":\"…\",\"blocks\":[{\"role\":\"bullet\"|\"takeaway\"|\"suggest\",\"text\":\"…\"}],\"notes\":\"…\"}: a new slide after that one, built like the deck's pages of that kind;\n\
- {\"edit\":\"split_object\",\"slide\":uid,\"obj\":id,\"parts\":[\"…\"],\"arrange\":\"2x2\"|\"row\"|\"column\"}: one box that crams several items (definitions, inputs, steps) becomes 2–6 boxes in its place; the parts together are exactly its text (you only split, never reword: rewording is a set_text); 2x2 takes exactly four parts;\n\
- {\"edit\":\"highlight_crop\",\"slide\":uid,\"picture\":id,\"mode\":\"crop\"|\"highlight\",\"box\":[x0,y0,x1,y1]}: only one block of a picture matters: crop shows only that block (larger, beside the lines), highlight keeps the whole picture faint with that block clear; box = fractions 0–1 of the picture as shown (image), around the whole block with a little of its blank margin, at least 0.05 each side;\n\
- {\"edit\":\"add_equation\",\"slide\":uid,\"latex\":\"…\",\"explains\":id,\"line\":n,\"display\":true}: a display equation set right under line n (1-based, counting that object's lines of text, not its equations) which explains it; at most two equations on a slide; an inline symbol inside a sentence is instead a set_text with the whole new paragraph and $…$.\n\
Rules: edit only the slides given. Each object comes with its limit (paragraphs, characters per paragraph; inline math counts half its source): nothing is shrunk to fit, so stay within it, and move what does not fit to another slide (split_slide) or to the notes. \
Inline math is $LaTeX$ with the deck's registered macros; a symbol used for the first time is defined in the notes (\"$x$ denotes …\"). Never type a reference such as \"Table 1\" or \"Fig. 3\". \
A line that only reads symbols again (\"where … denotes\") belongs in the notes. A split_slide is the only edit of its slide. \
A new slide that is not a take-home or a thanks page never has text alone: give it a `suggest` block saying which picture the presenter should put in its empty picture slot (one short line). \
Do what the instruction asks and change nothing else.\n";

pub const CHECK_SYSTEM: &str = "You look at slides of a talk right after an edit, as rendered pictures, and report only problems you can see: \
\"wrapped\" (a title or a line broken onto an extra line where it should fit on one), \"overlap\" (text over text or over a picture), \
\"cut-off\" (text cut by its box or by the page edge), \"empty\" (a large empty area the edit left), \"other\". \
An empty picture slot is drawn as a dashed frame with its suggestion line: it is where the presenter will put a picture, not a problem. \
Return JSON {\"findings\":[{\"slide\":uid,\"kind\":…,\"text\":one short line in Traditional Chinese}]}, an empty list when the slides are fine.";

fn edit_schema() -> Value {
    json!({ "type": "object", "required": ["edits"], "properties": { "edits": { "type": "array", "items": { "type": "object" } } } })
}

fn check_schema() -> Value {
    json!({ "type": "object", "required": ["findings"], "properties": { "findings": { "type": "array", "items": { "type": "object", "properties": { "slide": { "type": "string" }, "kind": { "type": "string" }, "text": { "type": "string" } } } } } })
}

/// One earlier exchange of a proposal (revise sends them along).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub instruction: String,
    pub plan: Value,
    pub findings: Vec<Finding>,
}

/// What failed in a round, for the corrective request.
pub struct Correction<'a> {
    pub plan: &'a Value,
    pub problems: Vec<String>,
    pub after: &'a [(String, Vec<u8>)],
}

#[allow(clippy::too_many_arguments)]
pub fn edit_request(ctx: &Value, instruction: &str, material: Option<&str>, history: &[Turn], before: &[(String, Vec<u8>)], fix: Option<&Correction>) -> LlmRequest {
    let mut user = String::new();
    user += &format!("Page kinds for insert_slide: {}. Block roles: {}.\n", insert_kinds().join(", "), BLOCK_ROLES.join(", "));
    user += &format!("Speaker notes:\n{NOTES_SHAPE}\n\n");
    user += &format!("The slides (JSON):\n{ctx}\n\n");
    for (k, (uid, _)) in before.iter().enumerate() {
        user += &format!("- image {}: slide {uid} as it is now\n", k + 1);
    }
    if let Some(m) = material {
        user += &format!("\nMaterial the presenter attached (a paper, its pages as text):\n{m}\n");
    }
    for (k, t) in history.iter().enumerate() {
        user += &format!("\nEarlier request {}: {}\nYour plan then: {}\n", k + 1, t.instruction, t.plan);
        if !t.findings.is_empty() {
            user += &format!("Problems left then: {}\n", t.findings.iter().map(Finding::line).collect::<Vec<_>>().join("; "));
        }
    }
    user += &format!("\nThe presenter's instruction:\n{instruction}\n");
    let mut images: Vec<&[u8]> = before.iter().map(|(_, b)| b.as_slice()).collect();
    if let Some(f) = fix {
        let base = before.len();
        for (k, (uid, _)) in f.after.iter().enumerate() {
            user += &format!("- image {}: slide {uid} after your plan\n", base + k + 1);
        }
        images.extend(f.after.iter().map(|(_, b)| b.as_slice()));
        user += &format!("\nYour plan: {}\nWhat failed (fix every point, keep what was right, return the whole plan again):\n{}\n", f.plan, f.problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n"));
    }
    let mut req = LlmRequest::prompt(EDIT_SYSTEM, user).json(edit_schema()).max_tokens(4000);
    for b in images {
        req = req.image(Image::from_bytes("image/png", b));
    }
    req
}

pub fn check_request(instruction: &str, after: &[(String, Vec<u8>)], looks: &[String]) -> LlmRequest {
    let mut user = format!("The edit asked for: {instruction}\n");
    for l in looks {
        user += &format!("Look closely: {l}\n");
    }
    for (k, (uid, _)) in after.iter().enumerate() {
        user += &format!("- image {}: slide {uid}\n", k + 1);
    }
    let mut req = LlmRequest::prompt(CHECK_SYSTEM, user).json(check_schema()).max_tokens(1500);
    for (_, b) in after {
        req = req.image(Image::from_bytes("image/png", b));
    }
    req
}

/// What the picture check must look at for the kept edits: a region a model chose by its box.
pub fn looks_for(edits: &[Edit]) -> Vec<String> {
    edits
        .iter()
        .filter_map(|e| match e {
            Edit::HighlightCrop { slide, picture, mode, .. } => Some(if mode == "crop" {
                format!("on slide {slide} picture {picture} now shows only a region a model chose by a box: does it hold the whole block meant, nothing cut through, no stray piece of a neighbour? Report \"other\" if not.")
            } else {
                format!("on slide {slide} picture {picture} is faint with one region clear, chosen by a box: is the clear region the whole block meant, its edges on blank space, not through a box or a label? Report \"other\" if not.")
            }),
            _ => None,
        })
        .collect()
}

fn findings_of(v: &Value, slides: &[String]) -> Vec<Finding> {
    v["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| serde_json::from_value::<Finding>(f.clone()).ok())
        .filter(|f| !f.text.trim().is_empty())
        .map(|mut f| {
            if !slides.contains(&f.slide) {
                f.slide = String::new();
            }
            f.text = plan::clip(f.text.trim(), 200);
            f
        })
        .collect()
}

// ── Integration points ───────────────────────────────────────────────────────

/// Hands compiled ops to the page of an open L-Echo request (the page applies them as one undo
/// step labelled [`LABEL`] and answers): `Ok(message)` or `(409 refused | 410 closed | 504 no answer, text)`.
/// A request that is not open is never queued: the server applies the edit itself.
#[async_trait::async_trait]
pub trait PageQueue: Send + Sync {
    fn is_open(&self, request: &str) -> bool;
    async fn apply(&self, request: &str, edit: &str, ops: &[Op]) -> Result<String, (u16, String)>;
    /// Asks the page of `request` to take the applied edit back with its own undo, so that what it
    /// shows follows (same answers as `apply`); `None`: this queue cannot, the server undoes.
    async fn undo(&self, _request: &str, _edit: &str) -> Option<Result<String, (u16, String)>> {
        None
    }
}

/// No page queue: every apply is done by the server.
pub struct NoPage;

#[async_trait::async_trait]
impl PageQueue for NoPage {
    fn is_open(&self, _: &str) -> bool {
        false
    }
    async fn apply(&self, _: &str, _: &str, _: &[Op]) -> Result<String, (u16, String)> {
        Err((410, "沒有開著的 L-Echo 要求".into()))
    }
}

/// The real queue: lore-shell's page requests (`lore_shell::pageedit`).
pub struct ShellQueue(pub lore_shell::pageedit::PageEdits);

#[async_trait::async_trait]
impl PageQueue for ShellQueue {
    fn is_open(&self, request: &str) -> bool {
        self.0.is_open(request)
    }
    async fn apply(&self, request: &str, edit: &str, ops: &[Op]) -> Result<String, (u16, String)> {
        let ops = serde_json::to_value(ops).map_err(|e| (500, e.to_string()))?;
        self.0.paged(lore_shell::pageedit::Paged { request: request.into(), edit: Some(edit.into()), ops: Some(ops), end: false }).await
    }
    async fn undo(&self, request: &str, edit: &str) -> Option<Result<String, (u16, String)>> {
        // not ops: the page reads `{undo: true}` as "your own undo, if this edit is the next one"
        Some(self.0.paged(lore_shell::pageedit::Paged { request: request.into(), edit: Some(edit.into()), ops: Some(json!({ "undo": true })), end: false }).await)
    }
}

/// Text of some pages of an L-Reader paper: `(sha, pages)` → text, bounded.
pub type PaperFn = dyn Fn(&str, &[u32]) -> Result<String, String> + Send + Sync;

pub struct Deps {
    pub queue: Arc<dyn PageQueue>,
    pub math: Arc<MathFn>,
    pub paper: Arc<PaperFn>,
}

/// The real paper reader: L-Reader's PDF (read-only, `reader::Reader`), `pdftotext` per page.
fn reader_text(home: &lore_home::Home, sha: &str, pages: &[u32]) -> Result<String, String> {
    if !crate::reader::valid_sha(sha) {
        return Err("reader_paper 不是 L-Reader 論文的 sha256".into());
    }
    let r = crate::reader::Reader::discover(home).ok_or("找不到 L-Reader 的資料夾")?;
    let pdf = r.pdf(sha).ok_or("L-Reader 裡找不到這篇論文的 PDF")?;
    let title = r.paper_json(sha).map(|m| crate::reader::title_of(&m)).unwrap_or_default();
    let pop = lore_docsrc::pdf::poppler::Poppler::discover();
    let pages: Vec<u32> = if pages.is_empty() { vec![1] } else { pages.iter().copied().filter(|p| *p >= 1).take(PAGES_MAX).collect() };
    let mut out = format!("Title: {title}\n");
    for p in pages {
        let o = std::process::Command::new(&pop.pdftotext).args(["-f", &p.to_string(), "-l", &p.to_string(), "-enc", "UTF-8"]).arg(&pdf).arg("-").output().map_err(|e| format!("pdftotext 不能執行：{e}"))?;
        if !o.status.success() {
            return Err(format!("讀不出第 {p} 頁的文字"));
        }
        out += &format!("\n[page {p}]\n{}\n", plan::clip(&crate::reader::clean(&String::from_utf8_lossy(&o.stdout)), PAGE_CHARS));
    }
    Ok(out)
}

// ── Previews and routes ──────────────────────────────────────────────────────

struct Preview {
    id: String,
    slides: Vec<String>,
    hash: u64,
    turns: Vec<Turn>,
    material: Option<String>,
    engine: Option<String>,
    edits: Vec<Edit>,
    deck: Value,
    before: Vec<Vec<u8>>,
    after: Vec<Vec<u8>>,
    /// `editor.rev` right after this edit was applied.
    applied: Option<u64>,
    /// The L-Echo request whose page applied it (its 復原 goes through that page too).
    via: Option<String>,
}

pub struct Pe {
    s: Shared,
    deps: Deps,
    store: Mutex<HashMap<String, VecDeque<Preview>>>,
    n: AtomicU64,
}

type St = State<Arc<Pe>>;

/// The routes with the real services: pandoc, L-Reader, and the page queue of `edits` (lore-shell's
/// page requests; `None`: the server applies every edit itself).
pub fn router(s: &Shared, edits: Option<lore_shell::pageedit::PageEdits>) -> Router {
    let pandoc = s.pandoc.clone();
    let home = s.home.clone();
    let queue: Arc<dyn PageQueue> = match edits {
        Some(pe) => Arc::new(ShellQueue(pe)),
        None => Arc::new(NoPage),
    };
    router_with(s, Deps { queue, math: Arc::new(move |l: &str, d: bool, z: f64| sc_core::latex_to_omml(&pandoc, l, d, z).map_err(|e| e.to_string())), paper: Arc::new(move |sha: &str, p: &[u32]| reader_text(&home, sha, p)) })
}

pub fn router_with(s: &Shared, deps: Deps) -> Router {
    let pe = Arc::new(Pe { s: s.clone(), deps, store: Mutex::new(HashMap::new()), n: AtomicU64::new(0) });
    Router::new()
        .route("/api/decks/{name}/page-edit", post(propose))
        .route("/api/decks/{name}/page-edit/{id}/revise", post(revise))
        .route("/api/decks/{name}/page-edit/{id}/deck", get(preview_deck))
        .route("/api/decks/{name}/page-edit/{id}/image/{which}", get(image))
        .route("/api/decks/{name}/page-edit/{id}/apply", post(apply))
        .route("/api/decks/{name}/page-edit/{id}/undo", post(undo))
        .with_state(pe)
}

fn err(code: StatusCode, m: impl Into<String>) -> ApiError {
    ApiError(code, m.into())
}

fn body_json(b: &Bytes) -> Result<Value, ApiError> {
    if b.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(b).map_err(|e| err(StatusCode::BAD_REQUEST, format!("內容不是 JSON：{e}")))
}

fn slides_hash(deck: &Deck, uids: &[String]) -> u64 {
    let v: Vec<Option<&Slide>> = uids.iter().map(|u| deck.slide(u)).collect();
    lore_canvas::fnv(&serde_json::to_vec(&v).unwrap_or_default())
}

fn log(dir: &Path, v: Value) {
    use std::io::Write;
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut v = v;
    v["at"] = json!(at);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(LOG)) {
        let _ = writeln!(f, "{v}");
    }
}

/// The deck as the print page reads it (`print.html?preview=`): the shape of `GET /api/decks/{name}`.
fn print_json(s: &Shared, sess: &sc_core::Session, deck: &Deck) -> Value {
    let style = sess.editor.style.as_ref().map(|st| serde_json::to_value(&st.pack).unwrap_or_default());
    let dv = serde_json::to_value(deck).unwrap_or_default();
    let mut names = crate::deck_font_names(&dv);
    if let Some(st) = &style {
        names.extend(crate::deck_font_names(st));
    }
    let fonts = lore_fonts::report(&crate::font_table(s), crate::installed_fonts(s), names);
    let stacks: serde_json::Map<String, Value> = fonts.iter().map(|r| (r.name.clone(), json!(r.chain))).collect();
    json!({ "name": sess.name, "deck": dv, "backdrops": sess.backdrops, "style": style, "font_stacks": stacks })
}

struct Ask {
    slides: Vec<String>,
    instruction: String,
    material: Option<String>,
    engine: Option<String>,
}

/// The request body's material: only an L-Reader paper in this slice (anything else: 400).
fn material(pe: &Pe, v: &Value) -> Result<Option<String>, ApiError> {
    let Some(m) = v.get("material").filter(|m| !m.is_null()) else { return Ok(None) };
    let Some(obj) = m.as_object() else { return Err(err(StatusCode::BAD_REQUEST, "material 要是物件")) };
    if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "reader_paper" | "pages")) {
        return Err(err(StatusCode::BAD_REQUEST, format!("這一版只能附 L-Reader 的論文（reader_paper），還不能附「{}」", plan::clip(k, 30))));
    }
    let Some(sha) = obj.get("reader_paper").and_then(Value::as_str) else {
        return if obj.is_empty() { Ok(None) } else { Err(err(StatusCode::BAD_REQUEST, "pages 要和 reader_paper 一起給")) };
    };
    let pages: Vec<u32> = obj.get("pages").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|n| n as u32).collect()).unwrap_or_default();
    (pe.deps.paper)(sha, &pages).map(Some).map_err(|e| err(StatusCode::BAD_REQUEST, e))
}

async fn propose(State(pe): St, UrlPath(name): UrlPath<String>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let v = body_json(&body)?;
    let slides: Vec<String> = v["slides"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    if slides.is_empty() || slides.len() > 2 {
        return Err(err(StatusCode::BAD_REQUEST, "一次交一到兩頁"));
    }
    let instruction = v["instruction"].as_str().unwrap_or("").trim().to_string();
    if instruction.is_empty() || instruction.chars().count() > INSTRUCTION_CHARS {
        return Err(err(StatusCode::BAD_REQUEST, format!("要說改什麼（最多 {INSTRUCTION_CHARS} 字）")));
    }
    let engine = crate::llm::page_edit_engine(v["engine"].as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e))?.map(String::from);
    let material = material(&pe, &v)?;
    run(&pe, &name, Ask { slides, instruction, material, engine }, vec![], None).await.map(Json)
}

async fn revise(State(pe): St, UrlPath((name, id)): UrlPath<(String, String)>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let v = body_json(&body)?;
    let instruction = v["instruction"].as_str().unwrap_or("").trim().to_string();
    if instruction.is_empty() || instruction.chars().count() > INSTRUCTION_CHARS {
        return Err(err(StatusCode::BAD_REQUEST, "要說改什麼"));
    }
    let (slides, turns, material, engine) = {
        let store = pe.store.lock().unwrap();
        let p = store.get(&name).and_then(|q| q.iter().find(|p| p.id == id)).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個修改（伺服器重新啟動過，或已經太舊）"))?;
        if p.applied.is_some() {
            return Err(err(StatusCode::CONFLICT, "這個修改已經套用，請重新提出"));
        }
        (p.slides.clone(), p.turns.clone(), p.material.clone(), p.engine.clone())
    };
    run(&pe, &name, Ask { slides, instruction, material, engine }, turns, Some(id)).await.map(Json)
}

/// Everything the sync step needs, taken from the session.
fn with_env<T>(pe: &Pe, sess: &sc_core::Session, f: impl FnOnce(&Env) -> T) -> T {
    let (glob, doc) = crate::notation::load(&sess.dir, crate::global_notation(&pe.s).as_deref());
    let macros = lore_notation::macros::katex_macros(&Notation::effective(&glob, &doc));
    let titles: String = sess.editor.deck.slides.iter().map(|s| crate::deckstyle::title_text(s, sess.editor.deck.size.h)).collect::<Vec<_>>().join(" ");
    let lang = if crate::deckstyle::mostly_cjk(&titles) { "zh" } else { "en" }.to_string();
    let env = Env { build: Some((&pe.s.ws, &sess.source)), dir: &sess.dir, pandoc: &pe.s.pandoc, macros, math: pe.deps.math.as_ref(), latex: &sess.latex, glob: &glob, doc: &doc, lang };
    f(&env)
}

struct Round {
    kept: Vec<Edit>,
    deck: Value,
    reasons: Vec<String>,
    code: Vec<Finding>,
    touched: Vec<String>,
    warnings: Vec<String>,
}

fn uenc(s: &str) -> String {
    crate::urlenc(s)
}

async fn run(pe: &Arc<Pe>, name: &str, ask: Ask, turns: Vec<Turn>, id: Option<String>) -> Result<Value, ApiError> {
    let s = pe.s.clone();
    let sess = crate::session(&s, name)?;
    let (ctx, hash) = {
        let sess = sess.lock().unwrap();
        let deck = &sess.editor.deck;
        if let Some(u) = ask.slides.iter().find(|u| deck.slide(u).is_none()) {
            return Err(err(StatusCode::NOT_FOUND, format!("找不到投影片 {u}")));
        }
        (context(deck, &sess.latex, &ask.slides), slides_hash(deck, &ask.slides))
    };
    let shot = crate::pageshot_of(&s);
    let mut before = vec![];
    for u in &ask.slides {
        // (the deck's own route as the preview: 前 is drawn like 後, an empty picture slot with its 建議放 line)
        let png = shot.png(name, u, Some(&format!("/api/decks/{}", uenc(name))), SHOT_W).await.map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
        before.push((u.clone(), png));
    }
    let llm = crate::todeck::llm(&s);
    let engine = ask.engine.as_deref();
    let call = |req: LlmRequest, task: &'static str| async move { llm.complete_json(task, req, engine).await.map(|a| a.value).map_err(|(m, _)| err(StatusCode::BAD_GATEWAY, m)) };
    let id = id.unwrap_or_else(|| format!("pe{}-{}", std::process::id() % 10_000, pe.n.fetch_add(1, Ordering::Relaxed) + 1));
    let preview_path = format!("/api/decks/{}/page-edit/{}/deck", uenc(name), uenc(&id));

    let mut plan_v = call(edit_request(&ctx, &ask.instruction, ask.material.as_deref(), &turns, &before, None), crate::llm::TASK_PAGE_EDIT).await?;
    let mut warnings: Vec<String> = vec![];
    let mut round = 0;
    let (kept, findings, after) = loop {
        let (edits, mut reasons, warn) = parse_plan(&plan_v, &ask.slides);
        for w in warn {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        let r: Round = {
            let (pe2, sess2, slides) = (pe.clone(), sess.clone(), ask.slides.clone());
            tokio::task::spawn_blocking(move || {
                let sess = sess2.lock().unwrap();
                with_env(&pe2, &sess, |env| {
                    let p = prepare(&sess.editor, &slides, &edits, env);
                    let code = code_findings(&sess.editor.deck, &p.preview.deck, &p.touched);
                    Round { kept: p.kept, deck: print_json(&pe2.s, &sess, &p.preview.deck), reasons: p.reasons, code, touched: p.touched, warnings: p.warnings }
                })
            })
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        };
        reasons.extend(r.reasons);
        let order: Vec<String> = r.deck["deck"]["slides"].as_array().into_iter().flatten().filter_map(|x| x["uid"].as_str().map(String::from)).collect();
        // the preview is registered before the pictures: the print page reads it back
        store_preview(pe, name, Preview { id: id.clone(), slides: ask.slides.clone(), hash, turns: turns.clone(), material: ask.material.clone(), engine: ask.engine.clone(), edits: r.kept.clone(), deck: r.deck, before: before.iter().map(|(_, b)| b.clone()).collect(), after: vec![], applied: None, via: None });
        let mut after = vec![];
        for u in &r.touched {
            let png = shot.png(name, u, Some(&preview_path), SHOT_W).await.map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
            after.push((u.clone(), png));
        }
        let seen = if r.kept.is_empty() || after.is_empty() { vec![] } else { findings_of(&call(check_request(&ask.instruction, &after, &looks_for(&r.kept)), crate::llm::TASK_PAGE_CHECK).await?, &r.touched) };
        let mut found = r.code;
        found.extend(seen);
        // each finding with its page as the edit leaves it (a new slide has no other name for him)
        for f in found.iter_mut().filter(|f| f.page.is_none()) {
            f.page = order.iter().position(|u| *u == f.slide).map(|i| i + 1);
        }
        if round == 0 && (!reasons.is_empty() || !found.is_empty()) {
            round += 1;
            let mut problems = reasons.clone();
            problems.extend(found.iter().map(Finding::line));
            plan_v = call(edit_request(&ctx, &ask.instruction, ask.material.as_deref(), &turns, &before, Some(&Correction { plan: &plan_v, problems, after: &after })), crate::llm::TASK_PAGE_EDIT).await?;
            continue;
        }
        // what was still refused after the corrective round is said, never applied
        for r in reasons {
            // (an unknown kind is already a warning in his words: said once)
            if r.split_once(UNKNOWN_TAIL).is_some_and(|(w, _)| warnings.iter().any(|x| x == w)) {
                continue;
            }
            let w = format!("沒有做：{}", for_author(&r));
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        for w in r.warnings {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        break (r.kept, found, after);
    };
    let plan_out = json!({ "edits": kept });
    let mut t = turns;
    t.push(Turn { instruction: ask.instruction.clone(), plan: plan_out.clone(), findings: findings.clone() });
    {
        let mut store = pe.store.lock().unwrap();
        if let Some(p) = store.get_mut(name).and_then(|q| q.iter_mut().find(|p| p.id == id)) {
            p.turns = t;
            p.after = after.iter().map(|(_, b)| b.clone()).collect();
        }
    }
    let dir = sess.lock().unwrap().dir.clone();
    log(&dir, json!({ "id": id, "slides": ask.slides, "instruction": ask.instruction, "plan": plan_out, "findings": findings, "warnings": warnings, "applied": false }));
    let url = |w: &str, i: usize| format!("/api/decks/{}/page-edit/{}/image/{w}-{i}", uenc(name), uenc(&id));
    Ok(json!({
        "id": id,
        "before": (0..before.len()).map(|i| url("before", i)).collect::<Vec<_>>(),
        "after": (0..after.len()).map(|i| url("after", i)).collect::<Vec<_>>(),
        "plan": plan_out,
        "findings": findings,
        "warnings": warnings,
    }))
}

fn store_preview(pe: &Pe, name: &str, p: Preview) {
    let mut store = pe.store.lock().unwrap();
    let q = store.entry(name.to_string()).or_default();
    q.retain(|x| x.id != p.id);
    q.push_back(p);
    while q.len() > KEEP {
        q.pop_front();
    }
}

async fn preview_deck(State(pe): St, UrlPath((name, id)): UrlPath<(String, String)>) -> Result<Json<Value>, ApiError> {
    let store = pe.store.lock().unwrap();
    let p = store.get(&name).and_then(|q| q.iter().find(|p| p.id == id)).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個修改"))?;
    Ok(Json(p.deck.clone()))
}

async fn image(State(pe): St, UrlPath((name, id, which)): UrlPath<(String, String, String)>) -> Result<Response, ApiError> {
    let store = pe.store.lock().unwrap();
    let p = store.get(&name).and_then(|q| q.iter().find(|p| p.id == id)).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個修改"))?;
    let (list, n) = which.split_once('-').ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這張圖"))?;
    let n: usize = n.parse().map_err(|_| err(StatusCode::NOT_FOUND, "沒有這張圖"))?;
    let png = match list {
        "before" => p.before.get(n),
        "after" => p.after.get(n),
        _ => None,
    }
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這張圖"))?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "no-store")], png.clone()).into_response())
}

async fn apply(State(pe): St, UrlPath((name, id)): UrlPath<(String, String)>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let v = body_json(&body)?;
    let request = v["request"].as_str().unwrap_or("").to_string();
    let (slides, hash, edits) = {
        let store = pe.store.lock().unwrap();
        let p = store.get(&name).and_then(|q| q.iter().find(|p| p.id == id)).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個修改（伺服器重新啟動過，或已經太舊）"))?;
        if p.applied.is_some() {
            return Err(err(StatusCode::CONFLICT, "這個修改已經套用過"));
        }
        (p.slides.clone(), p.hash, p.edits.clone())
    };
    if edits.is_empty() {
        return Err(err(StatusCode::CONFLICT, "這個提案沒有可以套用的修改"));
    }
    let sess = crate::session(&pe.s, &name)?;
    // the ops again, on the deck as it is now (fresh slide ids), once nothing they touch changed
    let ops = {
        let (pe2, sess2, slides2) = (pe.clone(), sess.clone(), slides.clone());
        tokio::task::spawn_blocking(move || {
            let sess = sess2.lock().unwrap();
            if slides_hash(&sess.editor.deck, &slides2) != hash {
                let n: Vec<String> = slides2.iter().map(|u| sess.editor.deck.slide_index(u).map(|i| (i + 1).to_string()).unwrap_or_else(|| "（已刪除）".into())).collect();
                return Err(err(StatusCode::CONFLICT, format!("第 {} 頁在提出修改之後又改過了，請重新提出", n.join("、"))));
            }
            with_env(&pe2, &sess, |env| {
                let p = prepare(&sess.editor, &slides2, &edits, env);
                if !p.reasons.is_empty() || p.kept.len() != edits.len() {
                    return Err(err(StatusCode::CONFLICT, format!("修改已經不能套用：{}", p.reasons.join("；"))));
                }
                Ok(p.ops)
            })
        })
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??
    };
    let mut by = "server";
    let mut message = String::new();
    // an open L-Echo request: its page applies the ops (its answer passed through); else the server
    if !request.is_empty() && pe.deps.queue.is_open(&request) {
        match pe.deps.queue.apply(&request, &id, &ops).await {
            Ok(m) => {
                by = "page";
                message = m;
            }
            Err((code, m)) => return Err(err(StatusCode::from_u16(code).unwrap_or(StatusCode::CONFLICT), m)),
        }
    }
    let mut out = {
        let mut sess = sess.lock().unwrap();
        if by == "server" {
            let a = sess.editor.apply(ops, LABEL)?;
            crate::changed(&mut sess, a)?.0
        } else {
            if sess.editor.can_undo() != Some(LABEL) {
                return Err(err(StatusCode::CONFLICT, "頁面說已套用，但編輯器裡找不到這一步"));
            }
            json!({ "status": crate::status(&sess) })
        }
    };
    let (rev, dir) = {
        let sess = sess.lock().unwrap();
        (sess.editor.rev, sess.dir.clone())
    };
    if let Some(p) = pe.store.lock().unwrap().get_mut(&name).and_then(|q| q.iter_mut().find(|p| p.id == id)) {
        p.applied = Some(rev);
        p.via = (by == "page").then(|| request.clone());
    }
    log(&dir, json!({ "id": id, "applied": true, "by": by }));
    out["applied"] = json!(by);
    out["message"] = json!(message);
    Ok(Json(out))
}

/// Where an applied edit stands in the editor's history.
#[derive(PartialEq)]
enum Step {
    /// The next undo.
    Next,
    /// Already undone there (the author's own ⌘Z): the next redo.
    Undone,
    /// Something else was done since.
    Buried,
}

fn step_of(ed: &Editor, rev: u64) -> Step {
    if ed.can_undo() == Some(LABEL) && ed.rev == rev {
        Step::Next
    } else if ed.can_redo() == Some(LABEL) && ed.rev == rev + 1 {
        Step::Undone
    } else {
        Step::Buried
    }
}

const BURIED: &str = "之後又改過，復原請在 L-Slide";

async fn undo(State(pe): St, UrlPath((name, id)): UrlPath<(String, String)>) -> Result<Json<Value>, ApiError> {
    let (rev, via) = {
        let store = pe.store.lock().unwrap();
        let p = store.get(&name).and_then(|q| q.iter().find(|p| p.id == id)).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個修改"))?;
        (p.applied.ok_or_else(|| err(StatusCode::CONFLICT, "這個修改還沒有套用"))?, p.via.clone())
    };
    let sess = crate::session(&pe.s, &name)?;
    let forget = || {
        if let Some(p) = pe.store.lock().unwrap().get_mut(&name).and_then(|q| q.iter_mut().find(|p| p.id == id)) {
            p.applied = None;
        }
    };
    // already taken back in the app: nothing to do, and said as it is (not an error)
    let already = |sess: &sc_core::Session| {
        forget();
        Ok(Json(json!({ "status": crate::status(sess), "undone": "already" })))
    };
    let step = step_of(&sess.lock().unwrap().editor, rev);
    match step {
        Step::Next => {}
        Step::Undone => return already(&sess.lock().unwrap()),
        Step::Buried => return Err(err(StatusCode::CONFLICT, BURIED)),
    }
    // the page that applied it is still there: it undoes (the server alone would leave it showing
    // the edit). A page that is gone (closed, reloaded: 410, or no answer: 504) leaves it to the server.
    if let Some(req) = via.filter(|r| pe.deps.queue.is_open(r))
        && let Some(r) = pe.deps.queue.undo(&req, &id).await
    {
        match r {
            Ok(_) => {
                let sess = sess.lock().unwrap();
                if step_of(&sess.editor, rev) == Step::Next {
                    return Err(err(StatusCode::CONFLICT, "頁面說已復原，但編輯器裡還是這一步"));
                }
                log(&sess.dir, json!({ "id": id, "undone": true, "by": "page" }));
                forget();
                return Ok(Json(json!({ "status": crate::status(&sess), "undone": "page" })));
            }
            Err((410 | 504, _)) => {}
            Err((code, m)) => return Err(err(StatusCode::from_u16(code).unwrap_or(StatusCode::CONFLICT), m)),
        }
    }
    let mut out = {
        let mut sess = sess.lock().unwrap();
        // (again: the page may have undone it late, after the wait ran out)
        match step_of(&sess.editor, rev) {
            Step::Next => {}
            Step::Undone => return already(&sess),
            Step::Buried => return Err(err(StatusCode::CONFLICT, BURIED)),
        }
        let a = sess.editor.undo().ok_or_else(|| err(StatusCode::CONFLICT, "沒有可復原的動作"))?;
        let dir = sess.dir.clone();
        let v = crate::changed(&mut sess, a)?.0;
        log(&dir, json!({ "id": id, "undone": true, "by": "server" }));
        v
    };
    forget();
    out["undone"] = json!("server");
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_canvas::{Frame, Geom, Look, Paint, SpObj};

    fn text_obj(id: u32, name: &str, slot: Option<&str>, y: i64, tag: &str, paras: &[&str]) -> Object {
        let look = Look { size: if tag == "title" { 28.0 } else { 18.0 }, ..Default::default() };
        let paras = paras.iter().map(|t| Para::new(vec![Run::text(t, tag, look.clone())])).collect();
        Object {
            id,
            name: name.into(),
            frame: Frame::new(400_000, y, 8_000_000, 900_000),
            hidden: false,
            ph: None,
            slot: slot.map(String::from),
            kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(TextBody { paras, ..Default::default() }), equation: false }),
            src: None,
        }
    }

    /// Three synthetic content slides: a title, a body of two lines, a caption-free picture slot.
    fn editor() -> Editor {
        let mut d = Deck::new(9_144_000, 5_143_500, None);
        for (i, t) in ["Synthetic Opening", "Method of the Synthetic Grader", "Results on Synthetic Data"].iter().enumerate() {
            let uid = d.new_uid();
            let mut s = Slide::new(uid, None);
            s.objects.push(text_obj(1, "Title", Some("title"), 200_000, "title", &[t]));
            s.objects.push(text_obj(2, "內文", Some("text-1"), 1_300_000, "body", &["An encoder finds the lesions", "A decoder grades them"]));
            s.objects.push(text_obj(3, "kicker", None, 50_000, "body", &["Method"]));
            s.notes = format!("Slide {} covers one step.\n1. $\\mathbf{{x}}$ denotes the input image.", i + 1);
            d.slides.push(s);
        }
        Editor::new(d, None)
    }

    fn fake_math(l: &str, _: bool, _: f64) -> Result<String, String> {
        if l.contains("\\bad") { Err("unknown command".into()) } else { Ok(format!("<m:oMath><m:r><m:t>{}</m:t></m:r></m:oMath>", l.len())) }
    }

    fn with<T>(f: impl FnOnce(&Env) -> T) -> T {
        let dir = tempfile::tempdir().unwrap();
        let (latex, glob, doc) = (BTreeMap::new(), Notation::default(), Notation::default());
        let pandoc = lore_math::Pandoc::default();
        let mut macros = BTreeMap::new();
        macros.insert("\\vx".to_string(), "\\mathbf{x}".to_string());
        let env = Env { build: None, dir: dir.path(), pandoc: &pandoc, macros, math: &fake_math, latex: &latex, glob: &glob, doc: &doc, lang: "en".into() };
        f(&env)
    }

    fn one(e: Edit) -> (Prepared, Editor) {
        let ed = editor();
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[e], env));
        (p, ed)
    }

    #[test]
    fn a_refusal_is_worded_for_the_author() {
        assert_eq!(for_author("第 1 個修改（set_text）：這一段有 83 字，最多 80 字（不會縮字）"), "改文字：這一段有 83 字，最多 80 字（不會縮字）");
        assert_eq!(for_author("第 2 個修改（split_slide）：拆頁的標題和備註寫在 split_slide 裡"), "拆頁：拆頁的標題和備註寫在 拆頁 裡");
        assert_eq!(for_author("模型沒有回應"), "模型沒有回應");
        assert_eq!(for_author("第 1 個修改（set_text）：第 8 頁沒有可以放字的物件 99"), "改文字：第 8 頁找不到要改的那段文字");
    }

    #[test]
    fn set_text_compiles_to_one_set_obj_in_the_objects_formatting() {
        let (p, _) = one(Edit::SetText { slide: "s2".into(), obj: 2, paras: vec!["The encoder finds lesions".into(), "Grading uses $\\vx$ only".into()] });
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
        assert_eq!(p.ops.len(), 1);
        let Op::SetObj { slide, id, path, value } = &p.ops[0] else { panic!() };
        assert_eq!((slide.as_str(), *id, path.as_str()), ("s2", 2, "/text/paras"));
        let paras: Vec<Para> = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].runs[0].tag.as_deref(), Some("body"));
        assert_eq!(paras[0].runs[0].look.size, 18.0, "the role's size, never shrunk");
        let math = paras[1].runs.iter().find(|r| r.kind == "math").unwrap();
        assert_eq!(math.latex.as_deref(), Some("\\mathbf{x}"), "registry macro expanded before OMML");
        assert!(math.omml.is_some());
        assert_eq!(p.touched, vec!["s2".to_string()]);
    }

    #[test]
    fn set_text_reject_reasons() {
        let long = "x".repeat(limit::BULLET + 1);
        for (e, want) in [
            (Edit::SetText { slide: "s2".into(), obj: 9, paras: vec!["a".into()] }, "沒有可以放字的物件"),
            (Edit::SetText { slide: "s2".into(), obj: 3, paras: vec!["a".into()] }, "由程式產生"),
            (Edit::SetText { slide: "s2".into(), obj: 2, paras: vec![long] }, "最多"),
            (Edit::SetText { slide: "s2".into(), obj: 2, paras: (0..6).map(|i| format!("line {i}")).collect() }, "段"),
            (Edit::SetText { slide: "s2".into(), obj: 1, paras: vec!["t".repeat(limit::TITLE + 2)] }, "最多"),
            (Edit::SetText { slide: "s2".into(), obj: 2, paras: vec!["where $x$ denotes the input".into()] }, "念符號"),
            (Edit::SetText { slide: "s2".into(), obj: 2, paras: vec!["odd $x dollar".into()] }, "沒有成對"),
            (Edit::SetText { slide: "s2".into(), obj: 2, paras: vec!["a $\\bad{x}$ b".into()] }, "轉不成方程式"),
            (Edit::SetText { slide: "s1".into(), obj: 2, paras: vec!["a".into()] }, "只能改交來的頁"),
            (Edit::SetText { slide: "s2".into(), obj: 2, paras: vec![" ".into()] }, "空的"),
        ] {
            let (p, _) = one(e);
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains(want)), "{want}: {:?}", p.reasons);
        }
    }

    #[test]
    fn latex_refused_when_the_notation_check_fails() {
        // a new symbol nobody defines: a new undefined_symbol warning → the edit is refused
        let (p, _) = one(Edit::SetText { slide: "s2".into(), obj: 2, paras: vec!["The weight $\\omega_k$ grows each step".into()] });
        assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains("對照表")), "{:?}", p.reasons);
        // defined in the slide's notes in the same plan: accepted
        let ed = editor();
        let edits = [Edit::SetNotes { slide: "s2".into(), text: "This slide covers the weight.\n1. $\\omega_k$ denotes the weight of step k.".into() }, Edit::SetText { slide: "s2".into(), obj: 2, paras: vec!["The weight $\\omega_k$ grows each step".into()] }];
        let p = with(|env| prepare(&ed, &["s2".to_string()], &edits, env));
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
        assert_eq!(p.kept.len(), 2);
    }

    #[test]
    fn notes_follow_the_shape() {
        let (p, _) = one(Edit::SetNotes { slide: "s2".into(), text: "This slide shows the grader.\n1. The encoder.\n2. The decoder.".into() });
        assert!(p.reasons.is_empty());
        assert!(matches!(&p.ops[0], Op::SetSlide { path, .. } if path == "/notes"));
        for bad in ["1. only a list\n2. no opening", "One line, no list, at all", ""] {
            let (p, _) = one(Edit::SetNotes { slide: "s2".into(), text: bad.into() });
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains("講者備註")), "{bad}: {:?}", p.reasons);
        }
    }

    fn split(groups: Vec<Vec<u32>>, n: usize) -> Edit {
        let titles = (0..n).map(|i| format!("Part {}", i + 1)).collect();
        let notes = (0..n).map(|i| format!("This part covers step {}.\n1. One point.", i + 1)).collect();
        Edit::SplitSlide { slide: "s2".into(), groups: groups.into_iter().map(|g| g.into_iter().map(Member::Obj).collect()).collect(), titles, notes }
    }

    fn by_paras(groups: &[&[usize]]) -> Edit {
        let Edit::SplitSlide { titles, notes, .. } = split(vec![], groups.len()) else { unreachable!() };
        Edit::SplitSlide { slide: "s2".into(), groups: groups.iter().map(|g| vec![Member::Paras { obj: 2, paras: g.to_vec() }]).collect(), titles, notes }
    }

    fn math(latex: &str) -> Para {
        Para::new(vec![Run { kind: "math".into(), latex: Some(latex.into()), ..Run::text("", "body", Look { size: 18.0, ..Default::default() }) }])
    }

    fn body_mut(ed: &mut Editor, slide: usize, id: u32) -> &mut TextBody {
        ed.deck.slides[slide].find_mut(id).and_then(|o| o.kind.sp_mut()).and_then(|sp| sp.text.as_mut()).unwrap()
    }

    /// His case: a title and ONE box of several definitions becomes several pages.
    #[test]
    fn a_one_box_page_splits_by_paragraphs() {
        let mut ed = editor();
        let lines = ["Recall is found over all lesions", "Precision is found over reported", "It follows this rule", "F1 joins both in one number"];
        let b = body_mut(&mut ed, 1, 2);
        b.paras = lines.iter().map(|t| Para::new(vec![Run::text(t, "body", Look { size: 18.0, ..Default::default() })])).collect();
        b.paras.insert(3, math("F = \\frac{2 P R}{P + R}"));
        let was = b.paras.clone();
        // the plan as a model writes it (the equation, paragraph 4, is left out: it follows its line)
        let v = json!({ "edits": [{ "edit": "split_slide", "groups": [[{ "obj": 2, "paras": [1] }], [{ "obj": 2, "paras": [2, 3] }], [{ "obj": 2, "paras": [5] }]],
            "titles": ["Recall", "Precision", "F1"], "notes": ["Recall first.\n1. One point.", "Precision next.\n1. One point.", "F1 last.\n1. One point."] }] });
        let (edits, reasons, _) = parse_plan(&v, &["s2".to_string()]);
        assert!(reasons.is_empty(), "{reasons:?}");
        assert_eq!(serde_json::to_value(&edits[0]).unwrap()["groups"], v["edits"][0]["groups"], "the plan goes back as it came");
        let p = with(|env| prepare(&ed, &["s2".to_string()], &edits, env));
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
        let d = &p.preview.deck;
        assert_eq!(d.slides.len(), 5);
        let want: [&[usize]; 3] = [&[0], &[1, 2, 3], &[4]];
        for (i, idx) in want.iter().enumerate() {
            let s = &d.slides[1 + i];
            let got = &body_of(s.find(2).expect("the box is on every page")).unwrap().paras;
            let exp: Vec<Para> = idx.iter().map(|k| was[*k].clone()).collect();
            assert_eq!(serde_json::to_value(got).unwrap(), serde_json::to_value(&exp).unwrap(), "page {}: its own paragraphs, untouched", i + 1);
            assert_eq!(crate::deckstyle::title_text(s, d.size.h), ["Recall", "Precision", "F1"][i]);
            assert!(s.find(3).is_some() && notes_shape_ok(&s.notes));
        }
        assert_eq!(p.touched.len(), 3);
        // each line in exactly one group; a number that is there; an equation with its line; one way per box
        for (e, want) in [
            (by_paras(&[&[1], &[2, 3]]), "第 5 段沒有放進任何一組"),
            (by_paras(&[&[1, 2], &[2, 3, 5]]), "第 2 段同時在"),
            (by_paras(&[&[1, 2, 3], &[5, 6]]), "沒有第 6 段"),
            (by_paras(&[&[1, 2, 3], &[4, 5]]), "是式子，要和它上面那一行放在同一組"),
            (by_paras(&[&[1, 2, 3, 5], &[]]), "沒有給段落"),
            (Edit::SplitSlide { slide: "s2".into(), groups: vec![vec![Member::Obj(2)], vec![Member::Paras { obj: 2, paras: vec![5] }]], titles: vec!["A".into(), "B".into()], notes: vec!["One.\n1. P.".into(), "Two.\n1. P.".into()] }, "只用一種寫法"),
        ] {
            let p = with(|env| prepare(&ed, &["s2".to_string()], &[e], env));
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains(want)), "{want}: {:?}", p.reasons);
        }
        // the old way with an empty group is told how
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[split(vec![vec![2], vec![]], 2)], env));
        assert!(p.reasons.iter().any(|r| r.contains("第 2 組是空的") && r.contains("paras")), "{:?}", p.reasons);
    }

    /// The walk's fixture: two paired equations whose symbol the slide's notes define (論文報告's
    /// symbol item); a later slide's notes define it too. The split's own notes do not: without the
    /// carried sentence the page that keeps the equation would use the symbol before its definition.
    #[test]
    fn a_split_carries_the_notes_symbol_definitions_to_the_pages_that_use_them() {
        let mut ed = editor();
        ed.deck.slides[0].notes = "Slide 1 opens.\n1. One point.".into();
        ed.deck.slides[1].notes = "Slide 2 covers the loss.\n1. One point.\n2. $\\mathbf{x}$ denotes the input image. $\\hat{\\mathbf{x}}$ denotes its copy.".into();
        body_mut(&mut ed, 1, 2).paras.push(math("L_{r} = \\lVert \\mathbf{x} - \\hat{\\mathbf{x}} \\rVert^2"));
        ed.deck.slides[1].objects.push(text_obj(4, "內文 2", Some("text-2"), 3_000_000, "body", &["Grading compares scores"]));
        let e = split(vec![vec![2], vec![4]], 2);
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[e.clone()], env));
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
        let (a, b) = (&p.preview.deck.slides[1], &p.preview.deck.slides[2]);
        assert_eq!(a.notes, "This part covers step 1.\n1. One point.\n2. $\\mathbf{x}$ denotes the input image. $\\hat{\\mathbf{x}}$ denotes its copy.", "the page with the equation keeps both sentences, as they were");
        assert_eq!(b.notes, "This part covers step 2.\n1. One point.", "the page without the symbols gets none");
        assert!(notes_shape_ok(&a.notes));
        // notes that already define the symbol are left alone
        let Edit::SplitSlide { groups, titles, .. } = e else { unreachable!() };
        let own = "This part covers step 1.\n1. $\\mathbf{x}$ denotes the picture. $\\hat{\\mathbf{x}}$ denotes what comes back.".to_string();
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[Edit::SplitSlide { slide: "s2".into(), groups, titles, notes: vec![own.clone(), "Two.\n1. P.".into()] }], env));
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
        assert_eq!(p.preview.deck.slides[1].notes, own);
        assert_eq!(sentences("Open, it.\n1. $a.b$ is one. Two here\n2. 三。四"), ["Open, it.", "$a.b$ is one.", "Two here", "三。", "四"]);
    }

    /// The other half of the walk's refusal: an earlier slide's display equation uses a symbol
    /// that the handed slide's second equation defines; the check lets that pass while the
    /// definition is within its three sentences, and names it once the split puts a page between.
    /// The order was the same before, so the split is kept and the author told; a symbol the edit
    /// itself uses too early is still refused.
    #[test]
    fn an_order_that_was_already_so_does_not_refuse_a_split() {
        let mut ed = editor();
        for s in ed.deck.slides.iter_mut() {
            s.notes = "This slide covers one step.\n1. One point.".into();
        }
        let b = body_mut(&mut ed, 0, 2);
        b.paras = vec![b.paras[0].clone(), math("L = L_{r} + L_{c}")];
        body_mut(&mut ed, 1, 2).paras = vec![Para::new(vec![Run::text("Reconstruction compares the image with its copy", "body", Look::default())]), math("L_{r} = \\lVert u \\rVert^2")];
        let mut o = text_obj(4, "內文 2", Some("text-2"), 3_000_000, "body", &["Grading compares the scores with the label"]);
        if let Some(sp) = o.kind.sp_mut() {
            sp.text.as_mut().unwrap().paras.push(math("L_{c} = \\sum_{k} v_k"));
        }
        ed.deck.slides[1].objects.push(o);
        let key = ("used_before_definition".to_string(), "L_{c}".to_string());
        with(|env| {
            assert!(!notation_keys(&ed.deck, env).contains_key(&key), "within the check's three sentences before the split: {:?}", notation_keys(&ed.deck, env).keys());
            let p = prepare(&ed, &["s2".to_string()], &[split(vec![vec![2], vec![4]], 2)], env);
            assert!(p.reasons.is_empty(), "{:?}", p.reasons);
            assert!(notation_keys(&p.preview.deck, env).contains_key(&key), "the check itself is as strict as before");
            assert!(p.warnings.iter().any(|w| w.contains("$L_{c}$") && w.contains("第 1 頁") && w.contains("第 3 頁")), "{:?}", p.warnings);
            // the edit's own slide using a symbol too early: refused
            let mut ed = ed.clone();
            body_mut(&mut ed, 1, 2).paras.push(math("Q = W + L_{r}"));
            body_mut(&mut ed, 1, 4).paras.push(math("W = \\sum_{k} v_k"));
            ed.deck.slides[1].objects.push(text_obj(5, "內文 3", Some("text-3"), 4_000_000, "body", &["A line about something else"]));
            let w = ("used_before_definition".to_string(), "W".to_string());
            assert!(!notation_keys(&ed.deck, env).contains_key(&w), "{:?}", notation_keys(&ed.deck, env));
            let p = prepare(&ed, &["s2".to_string()], &[split(vec![vec![2], vec![5], vec![4]], 3)], env);
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains("沒有通過對照表檢查")), "{:?} {:?} {:?}", p.reasons, p.warnings, notation_keys(&p.preview.deck, env));
        });
    }

    /// A title as 論文報告 builds it (32 pt in the base's 42 pt frame with insets): one line reads
    /// 110 % in the estimate and is no finding on a page the edit makes; two lines are.
    #[test]
    fn a_generated_one_line_title_is_not_over_its_box() {
        let ed = editor();
        let mut after = ed.deck.clone();
        let mut s = after.slides[1].clone();
        s.uid = "s9".into();
        let t = s.find_mut(1).unwrap();
        t.frame = Frame::new(487_680, 630_936, 11_216_640, 534_924);
        let tb = t.kind.sp_mut().unwrap().text.as_mut().unwrap();
        tb.insets = [91_440, 45_720, 91_440, 45_720];
        tb.paras = vec![Para::new(vec![Run::text("Why three lesions", "title", Look { size: 32.0, ..Default::default() })])];
        after.slides.insert(2, s);
        let r = crate::deckpolish::fill_ratio(after.slides[2].find(1).unwrap()).unwrap();
        assert!((1.08..1.12).contains(&r), "the estimate itself: {r}");
        let f = code_findings(&ed.deck, &after, &["s2".to_string(), "s9".to_string()]);
        assert!(f.iter().all(|f| f.kind != "overflow"), "{f:?}");
        let tb = after.slides[2].find_mut(1).unwrap().kind.sp_mut().unwrap().text.as_mut().unwrap();
        tb.paras = vec![Para::new(vec![Run::text(&"A title that is far too long for one line of the page ".repeat(2), "title", Look { size: 32.0, ..Default::default() })])];
        let f = code_findings(&ed.deck, &after, &["s2".to_string(), "s9".to_string()]);
        assert!(f.iter().any(|f| f.kind == "overflow" && f.slide == "s9" && f.page == Some(3) && !f.text.contains("第 3 頁") && f.line().starts_with("第 3 頁「")), "{f:?}");
    }

    #[test]
    fn an_unknown_kind_is_said_once_in_his_words() {
        let v = json!({ "edits": [{ "edit": "recolour", "slide": "s2" }, { "edit": "recolour", "slide": "s2" }, { "edit": "set_text", "slide": "s2" }] });
        let (edits, reasons, warnings) = parse_plan(&v, &["s2".to_string()]);
        assert!(edits.is_empty());
        assert_eq!(warnings, ["不認得的修改「recolour」，沒有做"]);
        assert_eq!(reasons.len(), 2, "{reasons:?}");
        assert!(reasons[0].starts_with(&warnings[0]) && reasons[0].contains(UNKNOWN_TAIL) && reasons[0].contains("split_slide"), "the model also reads what it may use: {}", reasons[0]);
        assert!(for_author(&reasons[1]).starts_with("改文字：格式不對"), "{}", for_author(&reasons[1]));
    }

    #[test]
    fn split_covers_every_object_exactly_once() {
        let mut ed = editor();
        // a second content object to split from the first
        ed.deck.slides[1].objects.push(text_obj(4, "內文 2", Some("text-2"), 3_000_000, "body", &["A third line"]));
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[split(vec![vec![2], vec![4]], 2)], env));
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
        let d = &p.preview.deck;
        assert_eq!(d.slides.len(), 4);
        let (a, b) = (&d.slides[1], &d.slides[2]);
        assert_eq!(a.uid, "s2", "the first part keeps the slide (and its source part)");
        assert!(a.find(2).is_some() && a.find(4).is_none() && b.find(4).is_some() && b.find(2).is_none());
        for (s, t) in [(a, "Part 1"), (b, "Part 2")] {
            assert_eq!(crate::deckstyle::title_text(s, d.size.h), t);
            assert!(s.find(3).is_some(), "the kicker stays on every part");
            assert!(notes_shape_ok(&s.notes));
        }
        assert_eq!(p.touched, vec!["s2".to_string(), b.uid.clone()]);
        // one missing, one twice, a title in a group, one group, titles/notes counts
        for (e, want) in [
            (split(vec![vec![2], vec![2]], 2), "同時在"),
            (split(vec![vec![2], vec![1, 4]], 2), "不能分組"),
            (split(vec![vec![2]], 1), "至少要兩組"),
            (Edit::SplitSlide { slide: "s2".into(), groups: vec![vec![Member::Obj(2)], vec![Member::Obj(4)]], titles: vec!["Only one".into()], notes: vec![] }, "標題"),
        ] {
            let p = with(|env| prepare(&ed, &["s2".to_string()], &[e], env));
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains(want)), "{want}: {:?}", p.reasons);
        }
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[split(vec![vec![2], vec![]], 2)], env));
        assert!(p.reasons.iter().any(|r| r.contains("沒有放進任何一組")), "{:?}", p.reasons);
    }

    #[test]
    fn the_new_kinds_parse_and_unknown_ones_are_warnings() {
        let v = json!({ "edits": [
            { "edit": "split_object", "obj": 2, "parts": ["a", "b"], "arrange": "row" },
            { "edit": "highlight_crop", "picture": 5, "mode": "crop", "box": [0.1, 0.2, 0.5, 0.9] },
            { "edit": "add_equation", "latex": "y = x", "explains": 2, "line": 1 },
            { "edit": "highlight_crop" },
            { "edit": "set_notes", "text": "One opening line.\n1. A point." },
            { "edit": "make_it_pop" },
        ] });
        let (edits, reasons, warnings) = parse_plan(&v, &["s2".to_string()]);
        assert_eq!(edits.len(), 4, "{reasons:?}");
        assert_eq!(edits[0], Edit::SplitObject { slide: "s2".into(), obj: 2, parts: vec!["a".into(), "b".into()], arrange: "row".into() }, "one slide: the slide may be left out");
        assert_eq!(edits[1], Edit::HighlightCrop { slide: "s2".into(), picture: 5, mode: "crop".into(), part: None, region: Some(vec![0.1, 0.2, 0.5, 0.9]) });
        assert_eq!(edits[2], Edit::AddEquation { slide: "s2".into(), latex: "y = x".into(), explains: Some(2), line: Some(1), display: true }, "display by default");
        assert!(!warnings.iter().any(|w| w.contains("還不支援")));
        assert!(warnings.iter().any(|w| w.contains("make_it_pop")));
        assert_eq!(reasons.len(), 2, "{reasons:?}");
        assert!(reasons[0].contains("highlight_crop") && reasons[0].contains("格式不對"));
        // the plan written back keeps `box`
        assert!(serde_json::to_string(&edits[1]).unwrap().contains("\"box\":[0.1,0.2,0.5,0.9]"));
    }

    #[test]
    fn split_object_text_must_be_the_original_split() {
        let was = vec!["• Recall: found over all lesions; Precision: found that are real".to_string()];
        assert!(text_differs(&was, &["Recall: found over all lesions;".into(), "1. Precision: found that are real".into()]).is_none(), "markers and spaces aside");
        let d = text_differs(&was, &["Recall: found over all lesions;".into(), "Precision: the found ones that are real".into()]).unwrap();
        assert!(d.contains("只拆不改字") && d.contains("「found that are real」"), "{d}");
        assert!(text_differs(&was, &["Recall: found over all lesions;".into()]).unwrap().contains("沒有放進任何一塊"));
        // what joined the items is not part of any box: separators and joining words between parts
        let line = vec!["Recall: found; Precision: real, and F1: both.".to_string()];
        let parts: Vec<String> = ["Recall: found;", "Precision: real, and", "F1: both."].iter().map(|x| x.to_string()).collect();
        assert!(text_differs(&line, &parts).is_none());
        assert_eq!(clean_parts(&parts), ["Recall: found", "Precision: real", "F1: both"], "a full stop stays only when every part has one");
        assert!(text_differs(&line, &["Recall: found".into(), "Precision: real".into(), "F1: both".into()]).is_none());
        assert_eq!(clean_parts(&["One.".into(), "Two.".into()]), ["One.", "Two."]);
        assert_eq!(clean_parts(&["召回率：找到的比例；".into(), "精確率：對的比例和".into(), "F1：兩者合一".into()]), ["召回率：找到的比例", "精確率：對的比例", "F1：兩者合一"]);
        assert!(text_differs(&["召回率：找到的比例；精確率：對的比例和F1：兩者合一".into()], &["召回率：找到的比例".into(), "精確率：對的比例".into(), "F1：兩者合一".into()]).is_none());
        // a part that starts like a joining word is not cut
        assert!(text_differs(&["Apps: iOS, Android".into()], &["Apps: iOS".into(), "Android".into()]).is_none());
        assert_eq!(strip_marker("(a) one"), " one");
        assert_eq!(strip_marker("2) two"), " two");
        assert_eq!(strip_marker("1.5 times"), "1.5 times");
    }

    #[test]
    fn a_box_edge_moves_to_a_near_gutter_never_across_a_wide_blank_band() {
        let dir = tempfile::tempdir().unwrap();
        // two blocks with a wide blank band between them (1/6 … 5/6)
        let mut img = image::RgbaImage::from_pixel(900, 300, image::Rgba([255, 255, 255, 255]));
        for x in (0..150u32).chain(750..900) {
            for y in 40..260u32 {
                img.put_pixel(x, y, image::Rgba([52, 101, 164, 255]));
            }
        }
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        img.save(dir.path().join("assets/band.png")).unwrap();
        let (latex, glob, doc) = (BTreeMap::new(), Notation::default(), Notation::default());
        let pandoc = lore_math::Pandoc::default();
        let env = Env { build: None, dir: dir.path(), pandoc: &pandoc, macros: BTreeMap::new(), math: &fake_math, latex: &latex, glob: &glob, doc: &doc, lang: "en".into() };
        let mut b = [0.3, 0.1, 0.9, 0.9];
        let moved = snap_box(&env, "file:assets/band.png", Default::default(), &mut b).unwrap();
        assert!((b[0] - 0.3).abs() < 1e-9, "the band's middle is far: the model's edge stays: {b:?}");
        assert!((b[1] - 0.067).abs() < 0.01 && (b[3] - 0.933).abs() < 0.01, "the top and bottom margins are near: {b:?}");
        assert!((b[2] - 0.9).abs() < 1e-9, "inside a block, no gutter near: unchanged");
        assert_eq!(moved, 2);
        assert!(snap_box(&env, "pkg:ppt/media/image1.png", Default::default(), &mut b).is_none(), "a picture of the package is not read");
    }

    #[test]
    fn new_kinds_reject_reasons_without_a_base() {
        let mut ed = editor();
        ed.deck.slides[1].objects.push(Object {
            id: 5,
            name: "圖".into(),
            frame: lore_canvas::Frame::new(400_000, 2_300_000, 4_000_000, 2_000_000),
            hidden: false,
            ph: None,
            slot: Some("image-1".into()),
            kind: Kind::Picture(lore_canvas::PicObj { asset: "file:assets/none.png".into(), crop: Default::default(), alpha: 0.3, line: None, clip: "rect".into(), masks: vec![], effects: vec![] }),
            src: None,
        });
        let four = |a: &str| Edit::SplitObject { slide: "s2".into(), obj: 2, parts: vec!["An encoder finds".into(), "the lesions".into(), "A decoder grades".into()], arrange: a.into() };
        let hc = |mode: &str, part: Option<u32>, b: Option<Vec<f64>>| Edit::HighlightCrop { slide: "s2".into(), picture: 5, mode: mode.into(), part, region: b };
        let eq = |l: &str, explains: Option<u32>, line: Option<u32>, display: bool| Edit::AddEquation { slide: "s2".into(), latex: l.into(), explains, line, display };
        for (e, want) in [
            (four("2x2"), "2x2 要剛好 4 塊"),
            (four("grid"), "不認得的排法"),
            (four("row"), "沒有底版"),
            (Edit::SplitObject { slide: "s2".into(), obj: 2, parts: vec!["An encoder finds the lesions".into()], arrange: "row".into() }, "2 到 6 塊"),
            (Edit::SplitObject { slide: "s2".into(), obj: 2, parts: vec!["An encoder finds the lesions".into(), "A decoder sorts them".into()], arrange: "row".into() }, "只拆不改字"),
            (Edit::SplitObject { slide: "s2".into(), obj: 1, parts: vec!["Method of".into(), "the Synthetic Grader".into()], arrange: "row".into() }, "不拆"),
            (hc("crop", Some(2), None), "part"),
            (hc("crop", None, None), "要給 box"),
            (hc("crop", None, Some(vec![0.1, 0.1, 0.12, 0.9])), "至少要圖的 5 %"),
            (hc("crop", None, Some(vec![0.1, 0.1, 1.2, 0.9])), "0 到 1"),
            (hc("crop", None, Some(vec![0.1, 0.1, 0.9])), "4 個數"),
            (hc("crop", None, Some(vec![0.1, 0.1, 0.5, 0.9])), "變淡的整張圖"),
            (hc("highlight", None, Some(vec![0.1, 0.1, 0.5, 0.9])), "不知道變淡的程度"),
            (hc("zoom", None, Some(vec![0.1, 0.1, 0.5, 0.9])), "mode"),
            (Edit::HighlightCrop { slide: "s2".into(), picture: 2, mode: "crop".into(), part: None, region: Some(vec![0.1, 0.1, 0.5, 0.9]) }, "不是圖片"),
            (eq("y = x", Some(2), Some(1), false), "set_text"),
            (eq("y = \\bad{x}", Some(2), Some(1), true), "轉不成方程式"),
            (eq("$y$", Some(2), Some(1), true), "不含 $"),
            (eq("y = x", Some(2), Some(9), true), "沒有第 9 行"),
            (eq("y = x", Some(1), Some(1), true), "不是這一頁的內文"),
            (eq("y = x", Some(2), None, true), "line 一起給"),
        ] {
            let p = with(|env| prepare(&ed, &["s2".to_string()], &[e], env));
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains(want)), "{want}: {:?}", p.reasons);
        }
    }

    #[test]
    fn insert_without_a_base_is_refused() {
        let (p, _) = one(Edit::InsertSlide { after: "s2".into(), kind: "method".into(), title: "A new step".into(), blocks: vec![Block { role: "suggest".into(), text: "the encoder figure".into() }], notes: "This slide covers a step.\n1. A point.".into() });
        assert!(p.reasons.iter().any(|r| r.contains("沒有底版")), "{:?}", p.reasons);
    }

    #[test]
    fn code_checks_find_new_overflow_only() {
        let ed = editor();
        let mut after = ed.deck.clone();
        if let Some(sp) = after.slides[1].find_mut(2).and_then(|o| o.kind.sp_mut()) {
            sp.text.as_mut().unwrap().paras = (0..12).map(|i| Para::new(vec![Run::text(&format!("A long synthetic line number {i} that wraps"), "body", Look::default())])).collect();
        }
        let f = code_findings(&ed.deck, &after, &["s2".to_string()]);
        assert!(f.iter().any(|f| f.kind == "overflow" && f.slide == "s2"), "{f:?}");
        assert!(code_findings(&after, &after, &["s2".to_string()]).iter().all(|f| f.kind != "overflow"), "what was already so is not counted");
    }
}
