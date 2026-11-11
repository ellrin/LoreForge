//! 排版用字微調 (論文報告's `polish` stage, usable on any deck): exactly the six checks of
//! `docs/paper-deck.md` §排版用字微調, never rewriting a sentence.
//!
//! 1. text overflowing its box (an estimate: no font metrics) / runs below their tag's size;
//! 2. bullets per slide above the author's profile;
//! 3. titles of one page role consistent (fixed closing titles, no final period, length, pattern);
//! 4. caption and `Ref:` present (prefix in the author's form);
//! 5. at most one key-result emphasis per slide;
//! 6. word precision: `Fig.`/`Figure` per the author's style, registered spellings from the
//!    deck's 對照表, and one term per concept (the only model step: lore-llm, class `create`,
//!    suggestions the author applies one by one).
//!
//! Deterministic fixes are applied right away, each as its own editor transaction; model
//! suggestions wait for 套用; what needs new content is listed only. Every item records the
//! exact values it changed (`changes`: path, before, after), so it can be undone on its own —
//! only while those values are still what the fix wrote (a later edit is never clobbered).
//! The report is `decks/<name>/polish.json`.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/decks/{name}/polish | the last report (`{items: []}` when none) |
//! | POST | /api/decks/{name}/polish | `{engine?, terms?}` → run the checks, the report |
//! | POST | /api/decks/{name}/polish/{item} | `{action: undo \| apply}` → changed slides + `item` |
//! | POST | /api/decks/{name}/polish/changes | `{action, changes, label?}` → changed slides (a fix from another report) |

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use lore_canvas::{Deck, Editor, Kind, Look, Object, Op, Slide, StyleSystem, TextBody};
use lore_llm::LlmRequest;
use lore_notation::Notation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::deckstyle::{self, Profile, body_of, chrome_ph, is_key_em, mostly_cjk, norm_text, title_object, word_count};
use crate::{R, Shared, bad};

const EMU_PT: f64 = 12_700.0;

/// One value an item changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub slide: String,
    pub id: u32,
    /// JSON pointer inside the object (`/text/paras/0/runs/1/text`).
    pub path: String,
    pub before: Value,
    pub after: Value,
}

/// One finding of the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: usize,
    /// 1–6, the spec's numbering.
    pub check: u8,
    pub slide: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<u32>,
    pub message: String,
    /// `applied` | `suggested` | `manual` | `undone`
    pub state: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
}

impl Item {
    fn new(check: u8, slide: &str, object: Option<u32>, message: String, state: &str, changes: Vec<Change>) -> Item {
        Item { id: 0, check, slide: slide.into(), object, message, state: state.into(), changes }
    }
    fn ops(&self, forward: bool) -> Vec<Op> {
        let mut v: Vec<Op> = self.changes.iter().map(|c| Op::SetObj { slide: c.slide.clone(), id: c.id, path: c.path.clone(), value: if forward { c.after.clone() } else { c.before.clone() } }).collect();
        if !forward {
            v.reverse();
        }
        v
    }
    fn label(&self, forward: bool) -> String {
        let m: String = self.message.chars().take(24).collect();
        if forward { format!("微調：{m}") } else { format!("復原微調：{m}") }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub items: Vec<Item>,
    /// Checks that did not run and why (no model …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Model provider id of the term suggestions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine: String,
}

impl Report {
    pub fn count(&self, state: &str) -> usize {
        self.items.iter().filter(|i| i.state == state).count()
    }
    fn push(&mut self, mut it: Item) {
        it.id = self.items.iter().map(|i| i.id).max().map_or(1, |m| m + 1);
        self.items.push(it);
    }
}

// ── Values in an object ──────────────────────────────────────────────────────

fn current(deck: &Deck, slide: &str, id: u32, path: &str) -> Option<Value> {
    let o = deck.slide(slide)?.find(id)?;
    // a field left out when serialised (`em: None`) reads as null
    Some(serde_json::to_value(o).ok()?.pointer(path).cloned().unwrap_or(Value::Null))
}

/// Apply (`forward`) or undo an item as one transaction, only when every value it touches is
/// still the one it expects.
pub fn commit(editor: &mut Editor, item: &mut Item, forward: bool) -> Result<lore_canvas::Applied, String> {
    if item.changes.is_empty() {
        return Err("這一項沒有可套用的修正".into());
    }
    for c in &item.changes {
        let want = if forward { &c.before } else { &c.after };
        if current(&editor.deck, &c.slide, c.id, &c.path).as_ref() != Some(want) {
            return Err("這裡已再修改過，沒有動它".into());
        }
    }
    let a = editor.apply(item.ops(forward), &item.label(forward)).map_err(|e| e.to_string())?;
    item.state = if forward { "applied".into() } else { "undone".into() };
    Ok(a)
}

/// Text bodies of an object with their JSON path prefix (`/text`, `/rows/0/cells/1/text`).
fn bodies(o: &Object) -> Vec<(String, &TextBody)> {
    match &o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => sp.text.iter().map(|b| ("/text".to_string(), b)).collect(),
        Kind::Connector(c) => c.text.iter().map(|b| ("/text".to_string(), b)).collect(),
        Kind::Table(t) => t.rows.iter().enumerate().flat_map(|(r, row)| row.cells.iter().enumerate().map(move |(c, cell)| (format!("/rows/{r}/cells/{c}/text"), &cell.text))).collect(),
        _ => vec![],
    }
}

/// Every visible object of a slide, groups included.
fn all_objects(s: &Slide) -> Vec<&Object> {
    let mut v = vec![];
    s.walk(&mut |o| {
        if !o.hidden && !o.is_empty_slot() {
            v.push(o);
        }
    });
    v
}

// ── Roles ────────────────────────────────────────────────────────────────────

/// Roles of a deck's slides: the classifier on titles, overridden by `hints` (title → role,
/// e.g. the plan's) where a slide's title matches one. Reveal copies share their title, so
/// they get the role too (indices do not line up with a plan after the reveal).
pub fn roles_for(deck: &Deck, hints: &[(String, String)]) -> Vec<String> {
    let titles: Vec<String> = deck.slides.iter().map(|s| deckstyle::title_text(s, deck.size.h)).collect();
    let mut roles = deckstyle::classify(&titles);
    let hint: BTreeMap<String, &String> = hints.iter().map(|(t, r)| (norm_text(t), r)).collect();
    for (i, t) in titles.iter().enumerate() {
        if let Some(r) = hint.get(&norm_text(t)).filter(|_| !t.trim().is_empty()) {
            roles[i] = (*r).clone();
        }
    }
    if let (Some(first), true) = (roles.first_mut(), hints.iter().any(|(_, r)| r == "cover")) {
        *first = "cover".into();
    }
    roles
}

// ── Check 1: overflow / minimum size ─────────────────────────────────────────

fn char_em(c: char) -> f64 {
    if deckstyle_cjk(c) {
        1.0
    } else if c == ' ' {
        0.28
    } else if c.is_ascii_uppercase() || c == 'm' || c == 'w' || c == 'M' || c == 'W' {
        0.66
    } else if c.is_ascii_digit() || c.is_ascii_lowercase() {
        0.52
    } else if "il.,;:'|!".contains(c) {
        0.28
    } else {
        0.55
    }
}

fn deckstyle_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Estimated text height (EMU) of a body `w` wide, with run sizes `size(p, r)`.
fn est_height(b: &TextBody, w: i64, size: &dyn Fn(usize, usize) -> f64) -> f64 {
    est_lines(b, w, size).0
}

/// Estimated text height (EMU) and number of lines.
fn est_lines(b: &TextBody, w: i64, size: &dyn Fn(usize, usize) -> f64) -> (f64, f64) {
    let scale = if b.autofit == "norm" { b.font_scale.clamp(0.1, 1.0) } else { 1.0 };
    let mut total = 0.0;
    let mut all_lines = 0.0;
    for (pi, p) in b.paras.iter().enumerate() {
        let avail = (w - b.insets[0] - b.insets[2] - p.mar_l.max(0)) as f64;
        let avail = avail.max(EMU_PT * 20.0);
        // the paragraph end's size counts only for an empty paragraph
        let empty = p.runs.iter().all(|r| r.text.is_empty() && r.kind != "math");
        let mut max_size: f64 = if empty { p.end.as_ref().map(|l| l.size).unwrap_or(0.0) } else { 0.0 };
        let mut lines = 1.0;
        let mut line_w = 0.0;
        for (ri, r) in p.runs.iter().enumerate() {
            let sz = size(pi, ri) * scale;
            max_size = max_size.max(sz);
            if r.kind == "br" {
                lines += 1.0;
                line_w = 0.0;
                continue;
            }
            let text = if r.kind == "math" { r.latex.clone().unwrap_or_default() } else { r.text.clone() };
            let factor = if r.kind == "math" { 0.45 } else { 1.0 };
            for c in text.chars() {
                let cw = char_em(c) * factor * sz * EMU_PT;
                if b.wrap && line_w + cw > avail && line_w > 0.0 {
                    lines += 1.0;
                    line_w = 0.0;
                }
                line_w += cw;
            }
        }
        if max_size <= 0.0 {
            max_size = 18.0 * scale;
        }
        let mult = p.line.unwrap_or(1.0) * (1.0 - b.ln_reduction);
        let lh = p.line_pt.map(|x| x * EMU_PT).unwrap_or(max_size * 1.2 * mult * EMU_PT);
        total += lines * lh + (p.before_pt.unwrap_or(0.0) + p.after_pt.unwrap_or(0.0)) * EMU_PT;
        all_lines += lines;
    }
    (total, all_lines)
}

/// The smallest size a tag allows.
fn tag_min(st: &StyleSystem, tag: &str) -> Option<f64> {
    st.tag(tag).map(|t| t.size_range.map(|r| r[0]).unwrap_or(t.size))
}

fn run_path(prefix: &str, p: usize, r: usize, field: &str) -> String {
    format!("{prefix}/paras/{p}/runs/{r}/{field}")
}

fn check_min_size(deck: &Deck, st: &StyleSystem, out: &mut Vec<Item>) {
    for s in &deck.slides {
        for o in all_objects(s) {
            if chrome_ph(o) {
                continue;
            }
            for (prefix, b) in bodies(o) {
                let mut ch = vec![];
                for (pi, p) in b.paras.iter().enumerate() {
                    for (ri, r) in p.runs.iter().enumerate() {
                        let (Some(tag), true) = (&r.tag, r.kind == "text") else { continue };
                        let Some(lo) = tag_min(st, tag) else { continue };
                        if r.look.size < lo - 0.05 && !r.text.trim().is_empty() {
                            ch.push(Change { slide: s.uid.clone(), id: o.id, path: run_path(&prefix, pi, ri, "look/size"), before: json!(r.look.size), after: json!(lo) });
                        }
                    }
                }
                if !ch.is_empty() {
                    let lo = ch[0].after.clone();
                    out.push(Item::new(1, &s.uid, Some(o.id), format!("字級低於標籤規定，改為 {lo} pt"), "applied", ch));
                }
            }
        }
    }
}

/// Estimated text height over the box's inner height (> 1: overflows), for a text object.
pub fn fill_ratio(o: &Object) -> Option<f64> {
    let b = body_of(o)?;
    let avail = (o.frame.h - b.insets[1] - b.insets[3]).max(1) as f64;
    Some(est_height(b, o.frame.w, &|p, r| b.paras[p].runs[r].look.size) / avail)
}

fn check_overflow(deck: &Deck, st: Option<&StyleSystem>, out: &mut Vec<Item>) {
    let (sw, sh) = (deck.size.w, deck.size.h);
    for s in &deck.slides {
        let objs = all_objects(s);
        for o in s.objects.iter().filter(|o| !o.hidden && !o.is_empty_slot() && !chrome_ph(o)) {
            let Some(b) = body_of(o) else { continue };
            if b.autofit == "shape" || b.vert.as_deref().is_some_and(|v| v != "horz") || b.plain_text().trim().is_empty() || o.frame.rot != 0.0 {
                continue;
            }
            let avail = (o.frame.h - b.insets[1] - b.insets[3]) as f64;
            let (est, lines) = est_lines(b, o.frame.w, &|p, r| b.paras[p].runs[r].look.size);
            // a single line sticking out of a small label is a design choice, not an overflow
            if est <= avail * 1.05 || lines <= 1.0 {
                continue;
            }
            // (a) room below: down to the next object under it, or the slide's bottom margin
            let bottom = o.frame.y + o.frame.h;
            let limit = objs
                .iter()
                .filter(|x| x.id != o.id && !chrome_ph(x))
                .filter(|x| (x.frame.w as f64 * x.frame.h as f64) < sw as f64 * sh as f64 * 0.8)
                .filter(|x| x.frame.x < o.frame.x + o.frame.w && x.frame.x + x.frame.w > o.frame.x && x.frame.y >= bottom - sh / 200)
                .map(|x| x.frame.y)
                .min()
                .unwrap_or(sh)
                .min(sh - sh * 3 / 100);
            let need = ((est - avail) * 1.08) as i64 + 1;
            if bottom + need <= limit {
                let h = o.frame.h + need;
                out.push(Item::new(1, &s.uid, Some(o.id), "文字超出框：框往下加高".into(), "applied", vec![Change { slide: s.uid.clone(), id: o.id, path: "/frame/h".into(), before: json!(o.frame.h), after: json!(h) }]));
                continue;
            }
            // (b) smaller type, never below the tag's minimum
            let mut fixed = None;
            if let Some(st) = st {
                let mut factor = 0.95;
                while factor > 0.4 {
                    let size_of = |p: usize, r: usize| -> f64 {
                        let run = &b.paras[p].runs[r];
                        match (&run.tag, run.kind.as_str()) {
                            (Some(t), "text") => match tag_min(st, t) {
                                Some(lo) => ((run.look.size * factor * 2.0).round() / 2.0).max(lo).min(run.look.size),
                                None => run.look.size,
                            },
                            _ => run.look.size,
                        }
                    };
                    let e = est_height(b, o.frame.w, &size_of);
                    if e <= avail * 1.02 {
                        let mut ch = vec![];
                        for (pi, p) in b.paras.iter().enumerate() {
                            for (ri, r) in p.runs.iter().enumerate() {
                                let n = size_of(pi, ri);
                                if (n - r.look.size).abs() > 0.01 {
                                    ch.push(Change { slide: s.uid.clone(), id: o.id, path: run_path("/text", pi, ri, "look/size"), before: json!(r.look.size), after: json!(n) });
                                }
                            }
                        }
                        if !ch.is_empty() {
                            fixed = Some(ch);
                        }
                        break;
                    }
                    factor -= 0.05;
                }
            }
            match fixed {
                Some(ch) => out.push(Item::new(1, &s.uid, Some(o.id), "文字超出框：字級縮小（不低於標籤規定）".into(), "applied", ch)),
                None => out.push(Item::new(1, &s.uid, Some(o.id), "文字超出框，需要手動調整".into(), "manual", vec![])),
            }
        }
    }
}

// ── Check 2: bullets per slide ───────────────────────────────────────────────

fn check_bullets(deck: &Deck, roles: &[String], profile: Option<&Profile>, out: &mut Vec<Item>) {
    let mut seen = BTreeSet::new();
    for (s, role) in deck.slides.iter().zip(roles) {
        if matches!(role.as_str(), "cover" | "thanks") || s.hidden {
            continue;
        }
        let title = title_object(s, deck.size.h).map(|o| o.id);
        let n: usize = s
            .objects
            .iter()
            .filter(|o| !o.hidden && !chrome_ph(o) && Some(o.id) != title)
            .filter(|o| body_of(o).is_some_and(|b| deckstyle::ref_prefix(&b.plain_text()).is_none()))
            .map(deckstyle::bullet_count)
            .sum();
        let limit = profile.map(|p| p.bullet_limit(role)).unwrap_or(5);
        // reveal copies of one slide: one item
        let key = (deckstyle::title_text(s, deck.size.h), n);
        if n > limit && seen.insert(key) {
            out.push(Item::new(2, &s.uid, None, format!("條列 {n} 項，超過這種頁的上限 {limit} 項"), "manual", vec![]));
        }
    }
}

// ── Check 3: titles per role ─────────────────────────────────────────────────

/// Text changes that make a body read `text`: the first text run gets it, the others empty.
fn set_body_text(slide: &str, o: &Object, b: &TextBody, prefix: &str, text: &str) -> Option<Vec<Change>> {
    let mut ch = vec![];
    let mut first = true;
    for (pi, p) in b.paras.iter().enumerate() {
        for (ri, r) in p.runs.iter().enumerate() {
            if r.kind != "text" {
                if r.kind == "math" {
                    return None;
                }
                continue;
            }
            let want = if first { text.to_string() } else { String::new() };
            if !(first || !r.text.is_empty()) {
                continue;
            }
            first = false;
            if r.text != want {
                ch.push(Change { slide: slide.into(), id: o.id, path: run_path(prefix, pi, ri, "text"), before: json!(r.text), after: json!(want) });
            }
        }
    }
    (!first).then_some(ch)
}

fn check_titles(deck: &Deck, roles: &[String], profile: Option<&Profile>, out: &mut Vec<Item>) {
    let h = deck.size.h;
    let mut by_role: BTreeMap<&str, Vec<(usize, String)>> = BTreeMap::new();
    let mut flagged_len = BTreeSet::new();
    for (i, (s, role)) in deck.slides.iter().zip(roles).enumerate() {
        if role == "cover" {
            continue;
        }
        let Some(o) = title_object(s, h) else { continue };
        let Some(b) = body_of(o) else { continue };
        let title = deckstyle::title_text(s, h);
        if title.is_empty() {
            continue;
        }
        let rs = profile.and_then(|p| p.roles.get(role));
        // fixed closing titles
        if let Some(f) = rs.and_then(|r| r.fixed_title.as_ref())
            && deckstyle::FIXED_ROLES.contains(&role.as_str())
            && norm_text(f) != norm_text(&title)
            && mostly_cjk(f) == mostly_cjk(&title)
        {
            if let Some(ch) = set_body_text(&s.uid, o, b, "/text", f) {
                // a short generic title ("Conclusion") is replaced; anything longer may say
                // something of its own, so it is only offered
                let generic = word_count(&title) <= 3 && deckstyle::role_of_title(&title) == Some(role.as_str());
                out.push(Item::new(3, &s.uid, Some(o.id), format!("這種頁的標題一向是「{f}」"), if generic { "applied" } else { "suggested" }, ch));
                continue;
            }
        }
        // final period
        let no_period = rs.map(|r| r.period == 0.0).unwrap_or(true);
        if no_period && (title.ends_with('.') || title.ends_with('。')) && !title.ends_with("..") && !title.ends_with("etc.") {
            let last = b.paras.iter().enumerate().flat_map(|(pi, p)| p.runs.iter().enumerate().map(move |(ri, r)| (pi, ri, r))).filter(|(_, _, r)| r.kind == "text" && !r.text.trim().is_empty()).last();
            if let Some((pi, ri, r)) = last {
                let t = r.text.trim_end();
                if t.ends_with('.') || t.ends_with('。') {
                    let new = t.trim_end_matches(['.', '。']).to_string();
                    out.push(Item::new(3, &s.uid, Some(o.id), "標題去掉句點".into(), "applied", vec![Change { slide: s.uid.clone(), id: o.id, path: run_path("/text", pi, ri, "text"), before: json!(r.text), after: json!(new) }]));
                }
            }
        }
        // length
        let words = word_count(&title);
        let max = match rs.map(|r| &r.title_words) {
            Some(st) if st.n >= 3 => st.max.max(st.median * 1.5).ceil() as usize,
            Some(st) if st.n > 0 => (st.median * 1.5).ceil().max(st.max) as usize,
            _ => 12,
        };
        if words > max && flagged_len.insert(norm_text(&title)) {
            out.push(Item::new(3, &s.uid, Some(o.id), format!("標題 {words} 字，比這種頁的標題長（最多約 {max}）"), "manual", vec![]));
        }
        if !deckstyle::FIXED_ROLES.contains(&role.as_str()) {
            let v = by_role.entry(role.as_str()).or_default();
            if !v.iter().any(|(_, t)| norm_text(t) == norm_text(&title)) {
                v.push((i, title));
            }
        }
    }
    // pattern minority within a role
    for (role, titles) in by_role {
        if titles.len() < 3 {
            continue;
        }
        let mut count: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, t) in &titles {
            *count.entry(deckstyle::title_pattern(t)).or_default() += 1;
        }
        let (major, n) = count.iter().max_by_key(|(_, n)| **n).map(|(k, n)| (*k, *n)).unwrap();
        if n * 3 < titles.len() * 2 {
            continue;
        }
        for (i, t) in &titles {
            let p = deckstyle::title_pattern(t);
            if p != major {
                let s = &deck.slides[*i];
                out.push(Item::new(3, &s.uid, title_object(s, deck.size.h).map(|o| o.id), format!("{} 頁的標題多半是「{}」句型，這頁是「{}」", role_zh(role), pattern_zh(major), pattern_zh(p)), "manual", vec![]));
            }
        }
    }
}

fn role_zh(r: &str) -> &'static str {
    match r {
        "background" => "背景",
        "challenge" => "問題",
        "overview" => "架構總覽",
        "method" => "方法",
        "loss" => "損失函數",
        "training" => "訓練細節",
        "results" => "結果",
        "ablation" => "消融",
        "visual" => "視覺化",
        "opinion" => "心得",
        "takeaway" => "Take home",
        "thanks" => "結尾",
        _ => "這種",
    }
}

fn pattern_zh(p: &str) -> &'static str {
    match p {
        "colon" => "前綴：內容",
        "question" => "問句",
        "sentence" => "完整句子",
        "cjk" => "中文",
        _ => "名詞片語",
    }
}

// ── Check 4: caption and Ref: ────────────────────────────────────────────────

static CITE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\[\d+(?:\s*[,–-]\s*\d+)*\]|\bet al\.").unwrap());

fn check_captions_refs(deck: &Deck, roles: &[String], profile: Option<&Profile>, out: &mut Vec<Item>) {
    let (w, h) = (deck.size.w, deck.size.h);
    let area = w as f64 * h as f64;
    let (bp_text, bp_pics) = deckstyle::boilerplate(deck);
    // pictures: captioned anywhere → fine; otherwise one item per picture
    let mut captioned: BTreeSet<String> = BTreeSet::new();
    let mut missing: Vec<(String, String, u32)> = vec![];
    let target = profile.map(|p| p.refs.prefix.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| "Ref:".into());
    for (s, role) in deck.slides.iter().zip(roles) {
        if matches!(role.as_str(), "cover" | "thanks") || s.hidden {
            continue;
        }
        for p in deckstyle::big_pictures(s, area, 0.08, &bp_pics) {
            let Kind::Picture(pic) = &p.kind else { continue };
            if deckstyle::has_caption(s, p, h) {
                captioned.insert(pic.asset.clone());
            } else {
                missing.push((pic.asset.clone(), s.uid.clone(), p.id));
            }
        }
        // Ref: lines and citations
        let mut has_ref = false;
        let mut cites = false;
        for o in all_objects(s) {
            if chrome_ph(o) {
                continue;
            }
            for (prefix, b) in bodies(o) {
                if bp_text.contains(&norm_text(&b.plain_text())) {
                    continue;
                }
                for (pi, para) in b.paras.iter().enumerate() {
                    let t: String = para.runs.iter().map(|r| r.text.as_str()).collect();
                    match deckstyle::ref_prefix(&t) {
                        Some(pre) => {
                            has_ref = true;
                            if pre != target {
                                // the prefix inside its run (only when one run holds it)
                                let hit = para.runs.iter().enumerate().find(|(_, r)| r.kind == "text" && !r.text.trim().is_empty());
                                match hit {
                                    Some((ri, r)) if r.text.trim_start().starts_with(&pre) => {
                                        let lead = r.text.len() - r.text.trim_start().len();
                                        let new = format!("{}{}{}", &r.text[..lead], target, &r.text[lead + pre.len()..]);
                                        out.push(Item::new(4, &s.uid, Some(o.id), format!("參考文獻行改用「{target}」"), "applied", vec![Change { slide: s.uid.clone(), id: o.id, path: run_path(&prefix, pi, ri, "text"), before: json!(r.text), after: json!(new) }]));
                                    }
                                    _ => out.push(Item::new(4, &s.uid, Some(o.id), format!("參考文獻行應該用「{target}」"), "manual", vec![])),
                                }
                            }
                        }
                        None => cites |= CITE.is_match(&t),
                    }
                }
            }
        }
        if cites && !has_ref {
            out.push(Item::new(4, &s.uid, None, format!("引用了文獻，但這頁沒有「{target}」行"), "manual", vec![]));
        }
    }
    // only when the author captions most figures (or there is no profile to say otherwise)
    if profile.is_some_and(|p| p.captions.figure_slides >= 3 && p.captions.with_caption < 0.5) {
        return;
    }
    let mut once = BTreeSet::new();
    for (asset, uid, id) in missing {
        if !captioned.contains(&asset) && once.insert(asset) {
            out.push(Item::new(4, &uid, Some(id), "圖沒有圖說".into(), "manual", vec![]));
        }
    }
}

// ── Check 5: one key-result emphasis per slide ───────────────────────────────

struct Span<'a> {
    obj: &'a Object,
    prefix: String,
    para: usize,
    runs: Vec<usize>,
}

fn is_takeaway(o: &Object) -> bool {
    let n = o.name.to_lowercase();
    n.contains("takeaway") || n.contains("take home") || o.slot.as_deref().is_some_and(|s| s.starts_with("takeaway")) || body_of(o).is_some_and(|b| b.paras.iter().any(|p| p.runs.iter().any(|r| r.tag.as_deref() == Some("takeaway"))))
}

fn check_emphasis(deck: &Deck, st: Option<&StyleSystem>, out: &mut Vec<Item>) {
    let is_key = |e: &str| is_key_em(e) || st.is_some_and(|st| e.starts_with("key") && st.pack.emphasis.contains_key(e));
    for s in &deck.slides {
        let mut objs = all_objects(s);
        objs.sort_by_key(|o| (o.frame.y, o.frame.x));
        let mut spans: Vec<Span> = vec![];
        for o in objs {
            for (prefix, b) in bodies(o) {
                for (pi, p) in b.paras.iter().enumerate() {
                    let mut cur: Option<Vec<usize>> = None;
                    for (ri, r) in p.runs.iter().enumerate() {
                        let k = r.em.as_deref().is_some_and(is_key);
                        if k {
                            cur.get_or_insert_with(Vec::new).push(ri);
                        } else if r.text.trim().is_empty() && cur.is_some() && r.kind == "text" {
                            // a space between key runs keeps the span open
                        } else if let Some(v) = cur.take() {
                            spans.push(Span { obj: o, prefix: prefix.clone(), para: pi, runs: v });
                        }
                    }
                    if let Some(v) = cur.take() {
                        spans.push(Span { obj: o, prefix: prefix.clone(), para: pi, runs: v });
                    }
                }
            }
        }
        if spans.len() <= 1 {
            continue;
        }
        let keep = spans.iter().position(|sp| is_takeaway(sp.obj)).unwrap_or(0);
        for (i, sp) in spans.iter().enumerate() {
            if i == keep {
                continue;
            }
            let b = bodies(sp.obj).into_iter().find(|(p, _)| *p == sp.prefix).map(|(_, b)| b).unwrap();
            let mut ch = vec![];
            let mut ok = true;
            for &ri in &sp.runs {
                let r = &b.paras[sp.para].runs[ri];
                let look: Option<Look> = match (st, &r.tag) {
                    (Some(st), Some(t)) => st.look(t, None),
                    _ => None,
                };
                let Some(mut look) = look else {
                    ok = false;
                    break;
                };
                look.size = r.look.size;
                ch.push(Change { slide: s.uid.clone(), id: sp.obj.id, path: run_path(&sp.prefix, sp.para, ri, "em"), before: json!(r.em), after: Value::Null });
                ch.push(Change { slide: s.uid.clone(), id: sp.obj.id, path: run_path(&sp.prefix, sp.para, ri, "look"), before: serde_json::to_value(&r.look).unwrap(), after: serde_json::to_value(&look).unwrap() });
            }
            let (first, last) = (sp.runs[0], *sp.runs.last().unwrap());
            let text: String = b.paras[sp.para].runs[first..=last].iter().map(|r| r.text.as_str()).collect::<String>().chars().take(20).collect();
            if ok {
                out.push(Item::new(5, &s.uid, Some(sp.obj.id), format!("一頁只留一個關鍵結果強調：「{text}」取消強調"), "applied", ch));
            } else {
                out.push(Item::new(5, &s.uid, Some(sp.obj.id), format!("一頁只留一個關鍵結果強調：「{text}」請手動取消"), "manual", vec![]));
            }
        }
    }
}

// ── Check 6: word precision ──────────────────────────────────────────────────

static FIG_TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\b(Figs?\.|figs?\.|Figures?|figures?)(\s*~?\s*\d)").unwrap());

/// Replace whole-token matches in every text run; one item per run.
fn replace_in_runs(deck: &Deck, check: u8, state: &str, what: &dyn Fn(&str) -> Option<(String, String)>, out: &mut Vec<Item>) {
    for s in &deck.slides {
        for o in all_objects(s) {
            if chrome_ph(o) {
                continue;
            }
            for (prefix, b) in bodies(o) {
                for (pi, p) in b.paras.iter().enumerate() {
                    for (ri, r) in p.runs.iter().enumerate() {
                        if r.kind != "text" || r.text.trim().is_empty() {
                            continue;
                        }
                        if let Some((new, msg)) = what(&r.text)
                            && new != r.text
                        {
                            out.push(Item::new(check, &s.uid, Some(o.id), msg, state, vec![Change { slide: s.uid.clone(), id: o.id, path: run_path(&prefix, pi, ri, "text"), before: json!(r.text), after: json!(new) }]));
                        }
                    }
                }
            }
        }
    }
}

fn deck_text(deck: &Deck) -> String {
    let mut t = String::new();
    for s in &deck.slides {
        for o in all_objects(s) {
            for (_, b) in bodies(o) {
                for p in &b.paras {
                    for r in p.runs.iter().filter(|r| r.kind == "text") {
                        t += &r.text;
                    }
                    t.push('\n');
                }
            }
        }
    }
    t
}

fn check_fig_form(deck: &Deck, profile: Option<&Profile>, out: &mut Vec<Item>) {
    let (a, f) = deckstyle::fig_counts(&deck_text(deck));
    if a + f == 0 {
        return;
    }
    let want_abbr = match profile.map(|p| p.captions.fig_form.as_str()) {
        Some("Fig.") => true,
        Some("Figure") => false,
        _ => a >= f,
    };
    let fix = |t: &str| -> Option<(String, String)> {
        let mut changed = false;
        let new = FIG_TOKEN.replace_all(t, |c: &regex::Captures| {
            let tok = &c[1];
            let plural = tok.starts_with("Figs") || tok.starts_with("figs") || tok.ends_with("res");
            let upper = tok.starts_with('F');
            let base = match (want_abbr, plural) {
                (true, false) => "Fig.",
                (true, true) => "Figs.",
                (false, false) => "Figure",
                (false, true) => "Figures",
            };
            let base = if upper { base.to_string() } else { base.to_lowercase() };
            if base != tok {
                changed = true;
            }
            format!("{base}{}", &c[2])
        });
        changed.then(|| (new.into_owned(), format!("圖的稱呼統一為「{}」", if want_abbr { "Fig." } else { "Figure" })))
    };
    replace_in_runs(deck, 6, "applied", &fix, out);
}

/// Spelling variants of a registered form: the same letters with other case (when the
/// form has capitals) or other separators (space / hyphen / none).
fn variant_regex(form: &str) -> Option<regex::Regex> {
    let words: Vec<&str> = form.split([' ', '-']).filter(|w| !w.is_empty()).collect();
    if words.is_empty() || form.chars().count() < 2 || !form.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let body = words.iter().map(|w| regex::escape(w)).collect::<Vec<_>>().join(r"[\s\-]?");
    regex::Regex::new(&format!(r"(?i)\b{body}\b")).ok()
}

fn check_notation(deck: &Deck, notation: &Notation, out: &mut Vec<Item>) {
    use lore_notation::model::Kind as NK;
    let mut forms: Vec<String> = vec![];
    for e in &notation.entries {
        if matches!(e.kind(), NK::Abbrev | NK::Term) {
            forms.extend([e.short.clone(), e.long.clone()].into_iter().flatten().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
        }
    }
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.dedup();
    let rx: Vec<(String, regex::Regex)> = forms.iter().filter_map(|f| variant_regex(f).map(|r| (f.clone(), r))).collect();
    if !rx.is_empty() {
        let fix = |t: &str| -> Option<(String, String)> {
            let mut s = t.to_string();
            let mut what = vec![];
            for (form, re) in &rx {
                let has_caps = form.chars().any(|c| c.is_uppercase());
                s = re
                    .replace_all(&s, |c: &regex::Captures| {
                        let m = &c[0];
                        let case_only = m.to_lowercase() == form.to_lowercase();
                        if m == form || (case_only && !has_caps) {
                            return m.to_string();
                        }
                        what.push(format!("{m} → {form}"));
                        form.clone()
                    })
                    .into_owned();
            }
            (!what.is_empty()).then(|| (s, format!("對照表的寫法：{}", what.join("、"))))
        };
        replace_in_runs(deck, 6, "applied", &fix, out);
    }
    // math symbols in a variant font of a registered one
    static FONT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\\(mathbf|boldsymbol|bm|mathrm|mathit)\s*\{([^{}]+)\}").unwrap());
    let reg: Vec<String> = notation.entries.iter().filter(|e| matches!(e.kind(), NK::Symbol)).filter_map(|e| e.latex.clone()).collect();
    let exact: BTreeSet<String> = reg.iter().map(|l| l.replace(' ', "")).collect();
    let base: BTreeMap<String, String> = reg.iter().filter_map(|l| FONT.captures(l).map(|c| (c[2].trim().to_string(), l.clone()))).collect();
    let mut seen = BTreeSet::new();
    for s in &deck.slides {
        for o in all_objects(s) {
            for (_, b) in bodies(o) {
                for p in &b.paras {
                    for r in p.runs.iter().filter(|r| r.kind == "math") {
                        let Some(l) = &r.latex else { continue };
                        for c in FONT.captures_iter(l) {
                            let tok = c[0].replace(' ', "");
                            if let Some(reg) = base.get(c[2].trim())
                                && !exact.contains(&tok)
                                && seen.insert(tok.clone())
                            {
                                out.push(Item::new(6, &s.uid, Some(o.id), format!("符號 {tok} 對照表登記的寫法是 {reg}"), "manual", vec![]));
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── The deterministic pass ───────────────────────────────────────────────────

/// What the checks need besides the deck.
pub struct Input<'a> {
    pub profile: Option<&'a Profile>,
    pub notation: &'a Notation,
    /// Roles per slide (`roles_for`).
    pub roles: Vec<String>,
}

/// Run the deterministic checks on the editor's deck, applying each fix as its own transaction
/// (in the order 6, 3, 4, 5, 1, 2, so sizes are measured on the final words). A fix the style
/// rules refuse becomes `manual`. Items come back sorted by check.
pub fn run_checks(editor: &mut Editor, inp: &Input) -> Report {
    let mut rep = Report::default();
    let st = editor.style.clone();
    let roles = |deck: &Deck| -> Vec<String> { if inp.roles.len() == deck.slides.len() { inp.roles.clone() } else { roles_for(deck, &[]) } };
    type Pass<'p> = Box<dyn Fn(&Deck, &mut Vec<Item>) + 'p>;
    let passes: Vec<Pass> = vec![
        Box::new(|d, o| check_fig_form(d, inp.profile, o)),
        Box::new(|d, o| check_notation(d, inp.notation, o)),
        Box::new(|d, o| check_titles(d, &roles(d), inp.profile, o)),
        Box::new(|d, o| check_captions_refs(d, &roles(d), inp.profile, o)),
        Box::new(|d, o| check_emphasis(d, st.as_ref(), o)),
        Box::new(|d, o| {
            if let Some(st) = st.as_ref() {
                check_min_size(d, st, o)
            }
        }),
        Box::new(|d, o| check_overflow(d, st.as_ref(), o)),
        Box::new(|d, o| check_bullets(d, &roles(d), inp.profile, o)),
    ];
    for pass in passes {
        let mut items = vec![];
        pass(&editor.deck, &mut items);
        for mut it in items {
            if it.state == "applied" {
                // pending → applied, or manual when refused
                let expected = std::mem::replace(&mut it.state, "pending".into());
                if let Err(e) = commit(editor, &mut it, true) {
                    it.state = "manual".into();
                    it.message = format!("{}（無法自動修正：{e}）", it.message);
                    it.changes.clear();
                } else {
                    it.state = expected;
                }
            }
            rep.push(it);
        }
    }
    rep.items.sort_by_key(|i| (i.check, i.id));
    rep
}

// ── The model step: one term per concept ─────────────────────────────────────

const STOP: [&str; 40] = [
    "the", "a", "an", "of", "and", "or", "in", "on", "for", "to", "with", "by", "is", "are", "was", "were", "be", "we", "our", "it", "its", "this", "that", "these", "those", "as", "at", "from", "than", "then",
    "can", "not", "no", "but", "into", "each", "per", "via", "vs", "which",
];

/// Candidate terms (1–3 word phrases without stop words at the edges, Latin script) with
/// counts; at most `cap`, most frequent first.
pub fn candidate_terms(deck: &Deck, cap: usize) -> Vec<(String, usize)> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for line in deck_text(deck).lines() {
        if deckstyle::ref_prefix(line).is_some() {
            continue;
        }
        for chunk in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ' ' || c == '\'')) {
            let words: Vec<&str> = chunk.split_whitespace().filter(|w| w.chars().any(|c| c.is_ascii_alphabetic())).collect();
            for n in 1..=3 {
                for win in words.windows(n) {
                    let (f, l) = (win[0].to_lowercase(), win[n - 1].to_lowercase());
                    if STOP.contains(&f.as_str()) || STOP.contains(&l.as_str()) || win.iter().any(|w| w.len() < 2) {
                        continue;
                    }
                    if n == 1 && win[0].len() < 4 && !win[0].chars().all(|c| c.is_ascii_uppercase()) {
                        continue;
                    }
                    *count.entry(win.join(" ")).or_default() += 1;
                }
            }
        }
    }
    let mut v: Vec<(String, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(cap);
    v
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Group {
    pub preferred: String,
    pub variants: Vec<String>,
}

pub fn terms_prompt(terms: &[(String, usize)]) -> (String, String, Value) {
    let system = "You check one slide deck for consistent terminology. You get the deck's terms with how often each occurs. \
Group terms that name the SAME concept in different words or spellings (\"segmentation mask\" / \"mask map\", \"fine-tuning\" / \"finetuning\"). \
For each group: preferred = the clearest, most standard form (one of the listed terms when possible); variants = the other listed terms of that group, copied exactly. \
Only groups you are sure about; never group a general term with a more specific one; never propose new wording for sentences. Reply with JSON {groups: [{preferred, variants}]}; {\"groups\": []} when the deck is consistent.".to_string();
    let user = terms.iter().map(|(t, n)| format!("{t} ({n})")).collect::<Vec<_>>().join("\n");
    let s = json!({ "type": "string" });
    let schema = json!({ "type": "object", "additionalProperties": false, "required": ["groups"], "properties": { "groups": { "type": "array", "items": {
        "type": "object", "additionalProperties": false, "required": ["preferred", "variants"],
        "properties": { "preferred": s, "variants": { "type": "array", "items": s } } } } } });
    (system, user, schema)
}

fn word_re(t: &str) -> Option<regex::Regex> {
    regex::Regex::new(&format!(r"\b{}\b", regex::escape(t))).ok()
}

/// Items for the model's groups: one `suggested` item per variant that really occurs as a
/// whole token, replacing it everywhere in one transaction.
pub fn term_items(deck: &Deck, groups: &[Group], terms: &[(String, usize)], rep: &mut Report) {
    let known: BTreeSet<&str> = terms.iter().map(|(t, _)| t.as_str()).collect();
    for g in groups {
        let pref = g.preferred.trim();
        if pref.is_empty() {
            continue;
        }
        for v in &g.variants {
            let v = v.trim();
            // a variant inside the preferred form would double words ("mask" → "segmentation mask")
            if v.is_empty() || v == pref || !known.contains(v) || word_re(v).is_some_and(|r| r.is_match(pref)) {
                continue;
            }
            let Some(re) = word_re(v) else { continue };
            let mut items = vec![];
            replace_in_runs(deck, 6, "suggested", &|t: &str| re.is_match(t).then(|| (re.replace_all(t, pref).into_owned(), String::new())), &mut items);
            if items.is_empty() {
                continue;
            }
            let first = items[0].slide.clone();
            let changes: Vec<Change> = items.into_iter().flat_map(|i| i.changes).collect();
            let n = changes.len();
            rep.push(Item::new(6, &first, None, format!("同一概念用同一個詞：「{v}」→「{pref}」（{n} 處）"), "suggested", changes));
        }
    }
    rep.items.sort_by_key(|i| (i.check, i.id));
}

/// Ask the model for term groups (the only model step). `Err` = a one-line reason.
pub async fn suggest_terms(llm: &crate::llm::Llm, deck: &Deck, engine: Option<&str>) -> Result<(Vec<Group>, Vec<(String, usize)>, String), String> {
    let terms = candidate_terms(deck, 300);
    if terms.len() < 2 {
        return Ok((vec![], terms, String::new()));
    }
    let (system, user, schema) = terms_prompt(&terms);
    let req = LlmRequest::prompt(system, user).json(schema).max_tokens(2000);
    let ans = llm.complete_json(crate::llm::TASK_PAPER_DECK, req, engine).await.map_err(|(m, _)| m)?;
    let groups: Vec<Group> = serde_json::from_value(ans.value["groups"].clone()).map_err(|e| format!("模型的回答無法使用：{e}"))?;
    Ok((groups, terms, ans.engine))
}

// ── The whole pass on a deck of the workspace ────────────────────────────────

pub fn report_path(deck_dir: &Path) -> std::path::PathBuf {
    deck_dir.join("polish.json")
}

pub fn load_report(deck_dir: &Path) -> Report {
    std::fs::read(report_path(deck_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_report(deck_dir: &Path, rep: &Report) -> Result<(), String> {
    sc_core::write_atomic(&report_path(deck_dir), &serde_json::to_vec_pretty(rep).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Options of a polish run.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Options {
    /// `local` / `claude` pins the engine of the term step.
    pub engine: Option<String>,
    /// false: skip the model step.
    pub terms: Option<bool>,
    /// Title → role hints (the plan's), see [`roles_for`].
    pub roles: Vec<(String, String)>,
}

/// Polish deck `name`: the deterministic checks (applied and saved), then the model's term
/// suggestions (without holding the deck while the model runs). The report is saved.
pub async fn polish(s: &Shared, name: &str, profile: Option<&Profile>, opt: &Options) -> Result<Report, String> {
    let sess = crate::session(s, name).map_err(|e| e.1)?;
    let (mut rep, deck, dir) = {
        let mut sess = sess.lock().unwrap();
        let (_, notation) = crate::notation::load(&sess.dir, None);
        let roles = roles_for(&sess.editor.deck, &opt.roles);
        let rep = run_checks(&mut sess.editor, &Input { profile, notation: &notation, roles });
        sess.save().map_err(|e| e.to_string())?;
        (rep, sess.editor.deck.clone(), sess.dir.clone())
    };
    if opt.terms != Some(false) {
        match suggest_terms(s.llm.get_or_init(|| crate::llm::Llm::new(s.home.clone())), &deck, opt.engine.as_deref()).await {
            Ok((groups, terms, engine)) => {
                // the deck may have changed meanwhile: items carry their before-values and are checked at 套用
                term_items(&deck, &groups, &terms, &mut rep);
                rep.engine = engine;
            }
            Err(e) => rep.notes.push(format!("用詞一致的建議沒有做：{e}")),
        }
    }
    save_report(&dir, &rep)?;
    Ok(rep)
}

// ── Routes ───────────────────────────────────────────────────────────────────

pub async fn get_report(State(s): State<Shared>, UrlPath(name): UrlPath<String>) -> R {
    let dir = s.ws.deck_dir(&name)?;
    Ok(Json(serde_json::to_value(load_report(&dir)).unwrap_or_default()))
}

pub async fn run(State(s): State<Shared>, UrlPath(name): UrlPath<String>, Json(opt): Json<Options>) -> R {
    let home = s.home.clone();
    let cache = s.ws.root.join("cache");
    let profile = tokio::task::spawn_blocking(move || deckstyle::for_home(&home, &cache)).await.ok().flatten();
    let rep = polish(&s, &name, profile.as_ref(), &opt).await.map_err(bad)?;
    Ok(Json(serde_json::to_value(rep).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct ItemBody {
    pub action: String,
}

pub async fn item(State(s): State<Shared>, UrlPath((name, id)): UrlPath<(String, usize)>, Json(b): Json<ItemBody>) -> R {
    let sess = crate::session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    let mut rep = load_report(&sess.dir);
    let it = rep.items.iter_mut().find(|i| i.id == id).ok_or_else(|| bad("找不到這一項"))?;
    let forward = match (b.action.as_str(), it.state.as_str()) {
        ("undo", "applied") => false,
        ("apply", "suggested" | "undone") => true,
        _ => return Err(bad("這一項現在不能這樣做")),
    };
    let a = commit(&mut sess.editor, it, forward).map_err(bad)?;
    let item = it.clone();
    save_report(&sess.dir, &rep).map_err(bad)?;
    let Json(mut v) = crate::changed(&mut sess, a)?;
    v["item"] = serde_json::to_value(item).unwrap_or_default();
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct ChangesBody {
    pub action: String,
    pub changes: Vec<Change>,
    #[serde(default)]
    pub label: String,
}

/// Undo / apply one fix from a report kept elsewhere (the 論文報告 job's `polish.json`): the
/// values are checked first, so a stale or edited fix is refused, never forced.
pub async fn changes(State(s): State<Shared>, UrlPath(name): UrlPath<String>, Json(b): Json<ChangesBody>) -> R {
    let forward = match b.action.as_str() {
        "undo" => false,
        "apply" => true,
        _ => return Err(bad("不認得的動作")),
    };
    let sess = crate::session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    let mut it = Item::new(0, "", None, if b.label.is_empty() { "排版用字".into() } else { b.label }, if forward { "suggested" } else { "applied" }, b.changes);
    let a = commit(&mut sess.editor, &mut it, forward).map_err(bad)?;
    crate::changed(&mut sess, a)
}
