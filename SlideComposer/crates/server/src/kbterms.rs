//! 「不熟」: the paper's domain terms against the author's KnowledgeBase, and the `learn`
//! marks they put on a deck (docs/paper-deck.md 「不熟」標示).
//!
//! * **Terms** ([`find_terms`], pure): acronyms, multi-word phrases (an expansion before its
//!   acronym in parentheses, "Spectral Boundary Loss (SBL)"; or a 2–4-word phrase used at least
//!   twice whose first word is not general English) and KnowledgeBase titles /
//!   aliases (1–4 words). Common words never count: `lore-words`, the filter LoreReader uses.
//! * **State** per term: 熟 = card `known`; 學習中 = card `learning` / `stale` / no learning
//!   state for this member; 不熟 = no card, or a card whose last event is 標為不熟 (要學).
//! * **KnowledgeBase**, read-only from disk like PaperComposer's `kb.rs` (`nodes/*.json` +
//!   `progress/<member>/*.json`; spaces: `KB_SPACES` > KnowledgeBase's `config.toml` `spaces` >
//!   lore-home `private/KnowledgeBase`; member: `KB_MEMBER` > `config.toml` `member` >
//!   `private/shared/profile.toml` `id`). The only write is 「同時存進知識庫（記得）」, through
//!   KnowledgeBase's own `POST /api/nodes` when it is running (`KB_URL` > the shared ports file
//!   > 127.0.0.1:7700); not running = nothing written, nothing queued.
//! * **Marks** ([`mark_body`], [`mark_slide`], [`unmark_body`], pure): runs get the editor-only
//!   emphasis `learn` (`lore_canvas::style::LEARN_EM`) on the term, or on the whole sentence
//!   for background / method text. Only plain generated runs are touched (no other emphasis,
//!   no imported source), so `learn` never replaces a key-result mark.
//!
//! | Method | Path | |
//! |---|---|---|
//! | PUT | /api/decks/{name}/learn | `{term, forms?, kb?: bool}` 我懂: clears the term's marks (one undoable step); `kb` also creates a card in state 記得 → `{…changed, kb: {saved, message}}` |

use std::path::Path;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as AxPath, State};
use lore_canvas::style::{LEARN_EM, texts_mut};
use lore_canvas::{Object, Op, Para, Run, Slide, TextBody};
use lore_words::phrase_key;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{R, Shared, bad};

pub use lore_learn::terms::*;

// The terms, their KnowledgeBase states and `Matcher` live in commons (`lore-learn`), shared with
// PaperComposer's 寫書; this file keeps the slide marks, 我懂 and the KnowledgeBase write.

/// Page roles (todeck's `ROLES`) whose sentences are marked whole: background and method narration.
pub fn sentence_role(role: &str) -> bool {
    matches!(role, "background" | "challenge" | "overview" | "method" | "loss")
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

// ── marks ────────────────────────────────────────────────────────────────────

/// Sentences of a paragraph as byte ranges (trimmed).
fn sentences(text: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = 0;
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in cs.iter().enumerate() {
        let end = i + c.len_utf8();
        let next_space = cs.get(k + 1).is_none_or(|(_, n)| n.is_whitespace());
        if matches!(c, '。' | '！' | '？' | '；') || (matches!(c, '.' | '!' | '?' | ';') && next_space) {
            out.push((start, end));
            start = end;
        }
    }
    if start < text.len() {
        out.push((start, text.len()));
    }
    out.into_iter()
        .filter_map(|(a, b)| {
            let s = &text[a..b];
            let lead = s.len() - s.trim_start().len();
            let t = s.trim();
            (!t.is_empty()).then(|| (a + lead, a + lead + t.len()))
        })
        .collect()
}

/// A run the marks may split and tag: generated text with no emphasis of its own.
fn markable(r: &Run) -> bool {
    r.kind == "text" && r.src.is_none() && r.em.is_none() && !r.text.is_empty()
}

fn mergeable(a: &Run, b: &Run) -> bool {
    a.kind == "text" && b.kind == "text" && a.src.is_none() && b.src.is_none() && a.tag == b.tag && a.em == b.em && a.look == b.look && a.field.is_none() && b.field.is_none()
}

/// Mark one paragraph; returns the number of marked spans.
fn mark_para(p: &mut Para, m: &Matcher, sentence: bool) -> usize {
    // the paragraph's text; non-text runs break words
    let mut text = String::new();
    let mut ranges = vec![];
    for r in &p.runs {
        let a = text.len();
        if r.kind == "text" {
            text.push_str(&r.text);
        } else {
            text.push('\u{FFFC}');
        }
        ranges.push((a, text.len()));
    }
    let hits = m.find(&text);
    if hits.is_empty() {
        return 0;
    }
    let spans: Vec<(usize, usize)> = if sentence {
        let ss = sentences(&text);
        let mut v: Vec<(usize, usize)> = ss.into_iter().filter(|(a, b)| hits.iter().any(|(x, y)| x < b && y > a)).collect();
        // a term crossing a sentence end still marks itself
        let crossing: Vec<(usize, usize)> = hits.iter().copied().filter(|(x, y)| !v.iter().any(|(a, b)| a <= x && y <= b)).collect();
        v.extend(crossing);
        v
    } else {
        hits
    };
    let mut out: Vec<Run> = vec![];
    for (r, (a, b)) in p.runs.drain(..).zip(ranges) {
        if !markable(&r) {
            out.push(r);
            continue;
        }
        // cut points inside this run
        let mut cuts: Vec<usize> = spans.iter().flat_map(|(x, y)| [*x, *y]).filter(|c| *c > a && *c < b).map(|c| c - a).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut from = 0;
        for c in cuts.into_iter().chain(std::iter::once(b - a)) {
            if c <= from || !r.text.is_char_boundary(c) {
                continue;
            }
            let mut piece = r.clone();
            piece.text = r.text[from..c].to_string();
            let mid = a + from;
            if spans.iter().any(|(x, y)| *x <= mid && mid < *y) {
                piece.em = Some(LEARN_EM.into());
            }
            out.push(piece);
            from = c;
        }
    }
    // join neighbours that ended up alike
    let mut merged: Vec<Run> = vec![];
    for r in out {
        match merged.last_mut() {
            Some(l) if mergeable(l, &r) => l.text.push_str(&r.text),
            _ => merged.push(r),
        }
    }
    p.runs = merged;
    p.runs.iter().filter(|r| r.em.as_deref() == Some(LEARN_EM)).count()
}

const TITLE_TAGS: &[&str] = &["title", "cover-title", "subtitle", "venue"];

/// Mark a text body: terms, or whole sentences (`sentence`, background / method text)
/// except in titles. Returns the number of marked spans.
pub fn mark_body(t: &mut TextBody, m: &Matcher, sentence: bool) -> usize {
    if m.is_empty() {
        return 0;
    }
    t.paras
        .iter_mut()
        .map(|p| {
            let title = p.runs.iter().any(|r| r.tag.as_deref().is_some_and(|g| TITLE_TAGS.contains(&g)));
            mark_para(p, m, sentence && !title)
        })
        .sum()
}

/// Mark every text of a slide (generation: `sentence` for background / method slides).
pub fn mark_slide(s: &mut Slide, m: &Matcher, sentence: bool) -> usize {
    let mut n = 0;
    for o in &mut s.objects {
        texts_mut(o, &mut |t| n += mark_body(t, m, sentence));
    }
    n
}

/// Clear marks and mark again with `m` (what is still unfamiliar). A paragraph whose
/// marks covered whole sentences keeps sentence marks. Returns whether anything changed.
pub fn unmark_body(t: &mut TextBody, m: &Matcher) -> bool {
    let before = t.clone();
    for p in &mut t.paras {
        let learn: Vec<&Run> = p.runs.iter().filter(|r| r.em.as_deref() == Some(LEARN_EM)).collect();
        if learn.is_empty() {
            continue;
        }
        let sentence = learn.iter().any(|r| r.text.split_whitespace().count() > 4 || r.text.trim_end().ends_with(['.', '。', '!', '?']));
        for r in &mut p.runs {
            if r.em.as_deref() == Some(LEARN_EM) {
                r.em = None;
            }
        }
        let mut merged: Vec<Run> = vec![];
        for r in p.runs.drain(..) {
            match merged.last_mut() {
                Some(l) if mergeable(l, &r) => l.text.push_str(&r.text),
                _ => merged.push(r),
            }
        }
        p.runs = merged;
        mark_para(p, m, sentence);
    }
    *t != before
}

/// Ops that clear `term` from a slide's marks, keeping the other marks (`keep`).
pub fn unmark_ops(s: &Slide, keep: &Matcher) -> Vec<Op> {
    let mut ops = vec![];
    for o in &s.objects {
        let mut n: Object = o.clone();
        let mut changed = false;
        texts_mut(&mut n, &mut |t| changed |= unmark_body(t, keep));
        if changed {
            ops.push(Op::SetObj { slide: s.uid.clone(), id: o.id, path: String::new(), value: serde_json::to_value(&n).unwrap_or(Value::Null) });
        }
    }
    ops
}

/// The deck's terms (`decks/<name>/terms.json`, written when the deck was made from a paper).
pub const TERMS_FILE: &str = "terms.json";

pub fn load_terms(deck_dir: &Path) -> Option<TermsResult> {
    serde_json::from_slice(&std::fs::read(deck_dir.join(TERMS_FILE)).ok()?).ok()
}

/// Keep the terms next to the deck (atomic), so 我懂 knows which marks stay.
pub fn save_terms(deck_dir: &Path, t: &TermsResult) -> std::io::Result<()> {
    let tmp = deck_dir.join(format!(".{TERMS_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(t)?)?;
    std::fs::rename(tmp, deck_dir.join(TERMS_FILE))
}

/// Without terms.json: the short learn runs (a term each) other than `term`.
fn marked_terms(deck: &lore_canvas::Deck, term: &Matcher) -> Vec<String> {
    let mut out = vec![];
    let mut d = deck.clone();
    for s in &mut d.slides {
        for o in &mut s.objects {
            texts_mut(o, &mut |t| {
                for r in t.paras.iter().flat_map(|p| &p.runs).filter(|r| r.em.as_deref() == Some(LEARN_EM)) {
                    let w = r.text.trim();
                    if (1..=4).contains(&w.split_whitespace().count()) && term.find(w).is_empty() {
                        out.push(w.to_string());
                    }
                }
            });
        }
    }
    out
}

// ── 我懂 ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UnderstoodBody {
    pub term: String,
    /// Other spellings of the same term (acronym, plural …).
    #[serde(default)]
    pub forms: Vec<String>,
    /// 同時存進知識庫（記得）
    #[serde(default)]
    pub kb: bool,
}

/// Where KnowledgeBase answers: `KB_URL`, the shared ports file, else its default port.
pub fn kb_url() -> String {
    if let Some(u) = env("KB_URL") {
        return u.trim_end_matches('/').to_string();
    }
    let port = lore_home::ports::live().get("kb").copied().unwrap_or(7700);
    format!("http://127.0.0.1:{port}")
}

/// 同時存進知識庫（記得）through KnowledgeBase's API: a new minimal card in state 記得, or, for a
/// term that already has a card (`card`), the answer 記得 on that card (no duplicate).
/// `Err` = a line for the author; KnowledgeBase not running = nothing written, nothing queued.
pub async fn save_known(base: &str, term: &str, card: Option<&str>) -> Result<String, String> {
    let down = || "KnowledgeBase 沒有開啟，沒有存進知識庫".to_string();
    let http = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(8)).build().map_err(|e| e.to_string())?;
    let (url, body) = match card {
        Some(id) => (format!("{base}/api/recall/{id}"), json!({ "outcome": "remembered", "via": "slidecomposer" })),
        None => {
            let spaces: Value = http.get(format!("{base}/api/spaces")).send().await.map_err(|_| down())?.json().await.map_err(|_| down())?;
            let list = spaces["spaces"].as_array().cloned().unwrap_or_default();
            let space = list
                .iter()
                .find(|s| s["kind"] == "personal" && s["is_member"] == true)
                .or_else(|| list.iter().find(|s| s["is_member"] == true))
                .and_then(|s| s["id"].as_str())
                .ok_or("知識庫沒有你能寫入的空間")?
                .to_string();
            (format!("{base}/api/nodes"), json!({ "space": space, "via": "slidecomposer", "kind": "concept", "title": term.trim(), "initial_state": "known" }))
        }
    };
    let res = http.post(url).json(&body).send().await.map_err(|_| down())?;
    let ok = res.status().is_success();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if !ok {
        return Err(format!("知識庫：{}", v["error"].as_str().unwrap_or("沒有存進去")));
    }
    Ok(card.map(str::to_string).or_else(|| v["node"]["id"].as_str().or(v["id"].as_str()).map(str::to_string)).unwrap_or_default())
}

/// `PUT /api/decks/{name}/learn`: 我懂 — clear the term's marks (one undoable step).
pub async fn understood(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<UnderstoodBody>) -> R {
    let term = b.term.trim().to_string();
    if term.is_empty() {
        return Err(bad("缺少名詞"));
    }
    // the deck first (no card for a deck that is not there), and the term's card if it has one
    let dir = crate::session(&s, &name)?.lock().unwrap().dir.clone();
    let card = load_terms(&dir).and_then(|t| {
        let k = phrase_key(&term);
        t.terms.into_iter().find(|x| x.text == term || x.key == term || x.forms.iter().any(|f| phrase_key(f) == k)).and_then(|x| x.card)
    });
    let kb = if b.kb {
        match save_known(&kb_url(), &term, card.as_deref()).await {
            Ok(id) => json!({ "saved": true, "id": id, "message": if card.is_some() { "知識庫的卡片記為「記得」" } else { "已存進知識庫（記得）" } }),
            Err(m) => json!({ "saved": false, "message": m }),
        }
    } else {
        Value::Null
    };
    let sess = crate::session(&s, &name)?;
    let mut sess = sess.lock().unwrap();
    let this = Matcher::new(std::iter::once(term.clone()).chain(b.forms.iter().cloned()));
    let keep = match load_terms(&sess.dir) {
        Some(mut t) => {
            t.understood(&term);
            for f in &b.forms {
                t.understood(f);
            }
            if let Err(e) = save_terms(&sess.dir, &t) {
                eprintln!("terms.json: {e}");
            }
            t.matcher()
        }
        None => Matcher::new(marked_terms(&sess.editor.deck, &this)),
    };
    let ops: Vec<Op> = sess.editor.deck.slides.iter().flat_map(|sl| unmark_ops(sl, &keep)).collect();
    let mut out = if ops.is_empty() {
        json!({ "slides": [], "label": "我懂" })
    } else {
        let a = sess.editor.apply(ops, &format!("我懂：{term}"))?;
        crate::changed(&mut sess, a)?.0
    };
    out["kb"] = kb;
    Ok(Json(out))
}
