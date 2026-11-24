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
//! - `split_slide {slide, groups: [[obj]], titles: [string], notes: [string]}`: every groupable
//!   object of the slide in exactly one group, one title and one notes text per group;
//! - `insert_slide {after, kind, title, blocks: [{role, text}], notes?}`: `kind` a 論文報告 page role,
//!   blocks `bullet` / `takeaway` / `suggest`, built by `todeck::build::build` like that page kind.
//!
//! `split_object`, `highlight_crop`, `add_equation` are not supported yet (a warning).
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/decks/{name}/page-edit | `{slides: [uid] (1–2), instruction, material?: {reader_paper?, pages?}, engine?}` → proposal |
//! | POST | /api/decks/{name}/page-edit/{id}/revise | `{instruction}` → proposal (same id; the earlier turns go along) |
//! | GET | /api/decks/{name}/page-edit/{id}/deck | the preview deck (for `print.html?preview=`) |
//! | GET | /api/decks/{name}/page-edit/{id}/image/{which} | `before-0`, `after-0`, … PNG |
//! | POST | /api/decks/{name}/page-edit/{id}/apply | `{request?}` → changed slides; 409 when a slide changed since |
//! | POST | /api/decks/{name}/page-edit/{id}/undo | undo, only while the editor's next undo is that edit |
//!
//! A proposal is `{id, before: [url], after: [url], plan, findings: [{slide, kind, text}], warnings}`.
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
use lore_canvas::{Deck, Editor, Kind, Object, Op, Para, Run, Slide, TextBody};
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
/// Edit kinds of a later slice: refused with a warning.
pub const LATER: [&str; 3] = ["split_object", "highlight_crop", "add_equation"];
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
        groups: Vec<Vec<u32>>,
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
}

impl Edit {
    fn slide(&self) -> &str {
        match self {
            Edit::SetText { slide, .. } | Edit::SetNotes { slide, .. } | Edit::SplitSlide { slide, .. } => slide,
            Edit::InsertSlide { after, .. } => after,
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Edit::SetText { .. } => "set_text",
            Edit::SetNotes { .. } => "set_notes",
            Edit::SplitSlide { .. } => "split_slide",
            Edit::InsertSlide { .. } => "insert_slide",
        }
    }
}

/// One finding of the checks: the slide (uid, empty when not one slide), its kind and a line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub slide: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
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
        if LATER.contains(&kind) {
            let w = format!("「{kind}」還不支援（這一版只能改文字、講者備註、拆頁、插頁），沒有做");
            if !warnings.contains(&w) {
                warnings.push(w.clone());
            }
            reasons.push(format!("edit {}: {kind} is not supported yet; use only set_text, set_notes, split_slide, insert_slide", i + 1));
            continue;
        }
        match serde_json::from_value::<Edit>(it.clone()) {
            Ok(mut e) => {
                // with one slide the slide may be left out
                if let (Edit::SetText { slide, .. } | Edit::SetNotes { slide, .. } | Edit::SplitSlide { slide, .. }, [only]) = (&mut e, targets) {
                    if slide.is_empty() {
                        *slide = only.clone();
                    }
                }
                edits.push(e)
            }
            Err(_) if kind.is_empty() || !["set_text", "set_notes", "split_slide", "insert_slide"].contains(&kind) => {
                warnings.push(format!("不認得的修改「{}」，沒有做", plan::clip(kind, 30)));
                reasons.push(format!("edit {}: unknown kind \"{}\"", i + 1, plan::clip(kind, 30)));
            }
            Err(e) => reasons.push(format!("edit {} ({kind}): 格式不對（{e}）", i + 1)),
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
}

/// Each edit in turn: checked, compiled against the deck as the earlier edits left it, applied to a
/// copy, and the deck's notation check compared before and after it (a new error or warning, such
/// as an undefined symbol, refuses that edit).
pub fn prepare(live: &Editor, targets: &[String], edits: &[Edit], env: &Env) -> Prepared {
    let mut work = live.clone();
    let mut out = Prepared { kept: vec![], ops: vec![], preview: live.clone(), touched: vec![], reasons: vec![] };
    let mut touched: BTreeSet<String> = targets.iter().cloned().collect();
    let split: Vec<&str> = edits.iter().filter(|e| matches!(e, Edit::SplitSlide { .. })).map(Edit::slide).collect();
    for (i, e) in edits.iter().enumerate() {
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
        let ops = match compile(&work, e, env) {
            Ok(ops) => ops,
            Err(r) => {
                out.reasons.push(format!("{at}：{r}"));
                continue;
            }
        };
        let before = notation_keys(&work.deck, env);
        let applied = match work.apply(ops.clone(), LABEL) {
            Ok(a) => a,
            Err(r) => {
                out.reasons.push(format!("{at}：編輯器不接受（{r}）"));
                continue;
            }
        };
        let after = notation_keys(&work.deck, env);
        let new: Vec<String> = after.iter().filter(|(k, _)| !before.contains_key(*k)).map(|(_, m)| m.clone()).collect();
        if !new.is_empty() {
            work.undo();
            out.reasons.push(format!("{at}：沒有通過對照表檢查：{}", new.join("；")));
            continue;
        }
        touched.extend(applied.slides);
        out.ops.extend(ops);
        out.kept.push(e.clone());
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
            let all: BTreeSet<u32> = groupable(s, h).into_iter().collect();
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
            let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
            for (gi, g) in groups.iter().enumerate() {
                if g.is_empty() {
                    r.push(format!("第 {} 組是空的", gi + 1));
                }
                for id in g {
                    if !all.contains(id) {
                        r.push(format!("物件 {id} 不能分組（不存在，或是標題、頁首、Ref 這類每頁都有的）"));
                    } else if let Some(prev) = seen.insert(*id, gi) {
                        r.push(format!("物件 {id} 同時在第 {} 組和第 {} 組", prev + 1, gi + 1));
                    }
                }
            }
            let missing: Vec<String> = all.iter().filter(|id| !seen.contains_key(id)).map(|id| id.to_string()).collect();
            if !missing.is_empty() {
                r.push(format!("物件 {} 沒有放進任何一組", missing.join("、")));
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
    }
    if r.is_empty() { Ok(()) } else { Err(r) }
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

fn fresh_uid(deck: &Deck, n: &mut u64) -> String {
    loop {
        let u = format!("s{}", (*n).max(1));
        *n = (*n).max(1) + 1;
        if deck.slide(&u).is_none() {
            return u;
        }
    }
}

/// The ops of one checked edit on `ed`'s deck as it is now.
pub fn compile(ed: &Editor, e: &Edit, env: &Env) -> Result<Vec<Op>, String> {
    let deck = &ed.deck;
    let h = deck.size.h;
    let idx = deck.slide_index(e.slide()).ok_or("找不到投影片")?;
    let s = &deck.slides[idx];
    match e {
        Edit::SetText { slide, obj, paras } => {
            let o = s.find(*obj).ok_or("找不到物件")?;
            let paras = rebuild(body_of(o).ok_or("物件沒有文字")?, paras, env)?;
            Ok(vec![Op::SetObj { slide: slide.clone(), id: *obj, path: "/text/paras".into(), value: serde_json::to_value(paras).map_err(|e| e.to_string())? }])
        }
        Edit::SetNotes { slide, text } => Ok(vec![Op::SetSlide { slide: slide.clone(), path: "/notes".into(), value: json!(text.trim()) }]),
        Edit::SplitSlide { slide, groups, titles, notes } => {
            let grp: BTreeSet<u32> = groupable(s, h).into_iter().collect();
            let title_id = title_object(s, h).map(|o| o.id).ok_or("沒有標題物件")?;
            let title_paras = |t: &str| -> Result<Vec<Para>, String> { rebuild(s.find(title_id).and_then(body_of).ok_or("標題沒有文字")?, &[t.to_string()], env) };
            let anims_for = |keep: &dyn Fn(u32) -> bool| s.anims.iter().filter(|a| keep(a.obj)).cloned().collect::<Vec<_>>();
            let mut ops = vec![];
            for o in s.objects.iter().filter(|o| grp.contains(&o.id) && !groups[0].contains(&o.id)) {
                ops.push(Op::DeleteObj { slide: slide.clone(), id: o.id });
            }
            ops.push(Op::SetObj { slide: slide.clone(), id: title_id, path: "/text/paras".into(), value: serde_json::to_value(title_paras(&titles[0])?).map_err(|e| e.to_string())? });
            ops.push(Op::SetSlide { slide: slide.clone(), path: "/notes".into(), value: json!(notes[0].trim()) });
            if !s.anims.is_empty() {
                let first = anims_for(&|id| !grp.contains(&id) || groups[0].contains(&id));
                ops.push(Op::SetSlide { slide: slide.clone(), path: "/anims".into(), value: serde_json::to_value(first).map_err(|e| e.to_string())? });
            }
            let mut n = deck.next_uid;
            for gi in 1..groups.len() {
                let g = &groups[gi];
                let mut ns = s.clone();
                ns.uid = fresh_uid(deck, &mut n);
                ns.objects.retain(|o| (!grp.contains(&o.id) && !o.hidden) || g.contains(&o.id));
                if let Some(tb) = ns.find_mut(title_id).and_then(|o| o.kind.sp_mut()).and_then(|sp| sp.text.as_mut()) {
                    tb.paras = title_paras(&titles[gi])?;
                }
                ns.notes = notes[gi].trim().to_string();
                ns.anims = anims_for(&|id| g.contains(&id));
                ops.push(Op::InsertSlide { index: idx + gi, slide: ns });
            }
            Ok(ops)
        }
        Edit::InsertSlide { kind, title, blocks, notes, .. } => {
            let (ws, source) = env.build.ok_or("這份簡報沒有底版")?;
            let st = ed.style.as_ref().ok_or("這份簡報沒有底版")?;
            let text = |role: &'static str| blocks.iter().filter(move |b| b.role == role).map(|b| b.text.trim().to_string()).filter(|t| !t.is_empty());
            let ps = plan::PlanSlide { role: kind.clone(), title: title.trim().into(), bullets: text("bullet").collect(), takeaway: text("takeaway").next().unwrap_or_default(), suggest: text("suggest").next().unwrap_or_default(), notes: notes.trim().into(), ..Default::default() };
            let mut n = deck.next_uid;
            let slide = built_slide(ws, st, source, env, ps, fresh_uid(deck, &mut n))?;
            Ok(vec![Op::InsertSlide { index: idx + 1, slide }])
        }
    }
}

/// The one dependency on `todeck/build.rs`: a page of `ps.role` built by 論文報告's own build
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
fn notation_keys(deck: &Deck, env: &Env) -> BTreeMap<(String, String), String> {
    let v = crate::notation::view(deck, env.latex, env.glob, env.doc);
    v["report"]["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| matches!(i["severity"].as_str(), Some("error" | "warning")))
        .map(|i| ((i["code"].as_str().unwrap_or("").to_string(), i["subject"].as_str().unwrap_or("").to_string()), i["message"].as_str().unwrap_or("").to_string()))
        .collect()
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

/// What the edit broke on the slides it touched: text over its box (the overflow estimate) and
/// objects that now collide or leave the page; what was already so before is not counted.
pub fn code_findings(before: &Deck, after: &Deck, touched: &[String]) -> Vec<Finding> {
    let page = (after.size.w, after.size.h);
    let mut out = vec![];
    for (n, s) in after.slides.iter().enumerate().filter(|(_, s)| touched.contains(&s.uid)) {
        let old = before.slide(&s.uid);
        for o in s.objects.iter().filter(|o| !o.hidden && !o.is_empty_slot() && !chrome_ph(o)) {
            let Some(r) = crate::deckpolish::fill_ratio(o).filter(|r| *r > 1.02) else { continue };
            if body_of(o).is_none_or(|b| b.plain_text().trim().is_empty()) {
                continue;
            }
            let was = old.and_then(|x| x.find(o.id)).and_then(crate::deckpolish::fill_ratio).is_some_and(|r| r > 1.02);
            if !was {
                out.push(Finding { slide: s.uid.clone(), kind: "overflow".into(), text: format!("第 {} 頁「{}」的字超出框（約 {:.0}%）", n + 1, obj_label(s, o.id), r * 100.0) });
            }
        }
        let had: BTreeSet<(u32, Option<u32>, &str)> = old.map(|x| overlaps_of(x, page)).unwrap_or_default().into_iter().collect();
        for (a, b, kind) in overlaps_of(s, page) {
            if had.contains(&(a, b, kind)) {
                continue;
            }
            let text = match b {
                Some(b) => format!("第 {} 頁「{}」和「{}」疊在一起", n + 1, obj_label(s, a), obj_label(s, b)),
                None => format!("第 {} 頁「{}」超出頁面", n + 1, obj_label(s, a)),
            };
            out.push(Finding { slide: s.uid.clone(), kind: if b.is_some() { "overlap".into() } else { "off-page".into() }, text });
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
- {\"edit\":\"split_slide\",\"slide\":uid,\"groups\":[[id,…],…],\"titles\":[\"…\"],\"notes\":[\"…\"]}: the slide becomes as many slides as groups, in order; every object marked groupable in exactly one group, one title and one notes text per group (the other objects, such as the title box, stay on every part);\n\
- {\"edit\":\"insert_slide\",\"after\":uid,\"kind\":page kind,\"title\":\"…\",\"blocks\":[{\"role\":\"bullet\"|\"takeaway\"|\"suggest\",\"text\":\"…\"}],\"notes\":\"…\"}: a new slide after that one, built like the deck's pages of that kind.\n\
Rules: edit only the slides given. Each object comes with its limit (paragraphs, characters per paragraph; inline math counts half its source): nothing is shrunk to fit, so stay within it, and move what does not fit to another slide (split_slide) or to the notes. \
Inline math is $LaTeX$ with the deck's registered macros; a symbol used for the first time is defined in the notes (\"$x$ denotes …\"). Never type a reference such as \"Table 1\" or \"Fig. 3\". \
A line that only reads symbols again (\"where … denotes\") belongs in the notes. A split_slide is the only edit of its slide. \
A new slide that is not a take-home or a thanks page never has text alone: give it a `suggest` block saying which picture the presenter should put in its empty picture slot (one short line). \
Not supported yet (do not use): split_object, highlight_crop, add_equation. Do what the instruction asks and change nothing else.\n";

pub const CHECK_SYSTEM: &str = "You look at slides of a talk right after an edit, as rendered pictures, and report only problems you can see: \
\"wrapped\" (a title or a line broken onto an extra line where it should fit on one), \"overlap\" (text over text or over a picture), \
\"cut-off\" (text cut by its box or by the page edge), \"empty\" (a large empty area the edit left), \"other\". \
Empty picture slots are not drawn: a reserved gap beside the text is expected. \
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
            user += &format!("Problems left then: {}\n", t.findings.iter().map(|f| f.text.as_str()).collect::<Vec<_>>().join("; "));
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

pub fn check_request(instruction: &str, after: &[(String, Vec<u8>)]) -> LlmRequest {
    let mut user = format!("The edit asked for: {instruction}\n");
    for (k, (uid, _)) in after.iter().enumerate() {
        user += &format!("- image {}: slide {uid}\n", k + 1);
    }
    let mut req = LlmRequest::prompt(CHECK_SYSTEM, user).json(check_schema()).max_tokens(1500);
    for (_, b) in after {
        req = req.image(Image::from_bytes("image/png", b));
    }
    req
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
        let png = shot.png(name, u, None, SHOT_W).await.map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
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
                    Round { kept: p.kept, deck: print_json(&pe2.s, &sess, &p.preview.deck), reasons: p.reasons, code, touched: p.touched }
                })
            })
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        };
        reasons.extend(r.reasons);
        // the preview is registered before the pictures: the print page reads it back
        store_preview(pe, name, Preview { id: id.clone(), slides: ask.slides.clone(), hash, turns: turns.clone(), material: ask.material.clone(), engine: ask.engine.clone(), edits: r.kept.clone(), deck: r.deck, before: before.iter().map(|(_, b)| b.clone()).collect(), after: vec![], applied: None });
        let mut after = vec![];
        for u in &r.touched {
            let png = shot.png(name, u, Some(&preview_path), SHOT_W).await.map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
            after.push((u.clone(), png));
        }
        let seen = if r.kept.is_empty() || after.is_empty() { vec![] } else { findings_of(&call(check_request(&ask.instruction, &after), crate::llm::TASK_PAGE_CHECK).await?, &r.touched) };
        let mut found = r.code;
        found.extend(seen);
        if round == 0 && (!reasons.is_empty() || !found.is_empty()) {
            round += 1;
            let mut problems = reasons.clone();
            problems.extend(found.iter().map(|f| f.text.clone()));
            plan_v = call(edit_request(&ctx, &ask.instruction, ask.material.as_deref(), &turns, &before, Some(&Correction { plan: &plan_v, problems, after: &after })), crate::llm::TASK_PAGE_EDIT).await?;
            continue;
        }
        // what was still refused after the corrective round is said, never applied
        for r in reasons {
            warnings.push(format!("沒有做：{r}"));
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
    }
    log(&dir, json!({ "id": id, "applied": true, "by": by }));
    out["applied"] = json!(by);
    out["message"] = json!(message);
    Ok(Json(out))
}

async fn undo(State(pe): St, UrlPath((name, id)): UrlPath<(String, String)>) -> Result<Json<Value>, ApiError> {
    let rev = {
        let store = pe.store.lock().unwrap();
        let p = store.get(&name).and_then(|q| q.iter().find(|p| p.id == id)).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個修改"))?;
        p.applied.ok_or_else(|| err(StatusCode::CONFLICT, "這個修改還沒有套用"))?
    };
    let sess = crate::session(&pe.s, &name)?;
    let out = {
        let mut sess = sess.lock().unwrap();
        if sess.editor.can_undo() != Some(LABEL) || sess.editor.rev != rev {
            return Err(err(StatusCode::CONFLICT, "套用之後又有別的修改，請用一般的復原"));
        }
        let a = sess.editor.undo().ok_or_else(|| err(StatusCode::CONFLICT, "沒有可復原的動作"))?;
        let dir = sess.dir.clone();
        let v = crate::changed(&mut sess, a)?.0;
        log(&dir, json!({ "id": id, "undone": true }));
        v
    };
    if let Some(p) = pe.store.lock().unwrap().get_mut(&name).and_then(|q| q.iter_mut().find(|p| p.id == id)) {
        p.applied = None;
    }
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
        Edit::SplitSlide { slide: "s2".into(), groups, titles, notes }
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
            (Edit::SplitSlide { slide: "s2".into(), groups: vec![vec![2], vec![4]], titles: vec!["Only one".into()], notes: vec![] }, "標題"),
        ] {
            let p = with(|env| prepare(&ed, &["s2".to_string()], &[e], env));
            assert!(p.ops.is_empty() && p.reasons.iter().any(|r| r.contains(want)), "{want}: {:?}", p.reasons);
        }
        let p = with(|env| prepare(&ed, &["s2".to_string()], &[split(vec![vec![2], vec![]], 2)], env));
        assert!(p.reasons.iter().any(|r| r.contains("沒有放進任何一組")), "{:?}", p.reasons);
    }

    #[test]
    fn unsupported_kinds_are_warnings_and_unknown_ones_too() {
        let v = json!({ "edits": [
            { "edit": "split_object", "slide": "s2", "obj": 2 },
            { "edit": "highlight_crop" },
            { "edit": "set_notes", "text": "One opening line.\n1. A point." },
            { "edit": "make_it_pop" },
        ] });
        let (edits, reasons, warnings) = parse_plan(&v, &["s2".to_string()]);
        assert_eq!(edits, vec![Edit::SetNotes { slide: "s2".into(), text: "One opening line.\n1. A point.".into() }], "one slide: the slide may be left out");
        assert!(warnings.iter().any(|w| w.contains("split_object") && w.contains("還不支援")));
        assert!(warnings.iter().any(|w| w.contains("highlight_crop")));
        assert!(warnings.iter().any(|w| w.contains("make_it_pop")));
        assert_eq!(reasons.len(), 3);
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
