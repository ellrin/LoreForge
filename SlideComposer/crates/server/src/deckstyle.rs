//! The author's deck style profile (論文報告's `profile` stage): read the author's decks
//! read-only (lore-home's `private/SlideComposer/decks/*/deck.json`, SlideComposer's own
//! deck format) and reduce them to a compact summary — page share per role, title length
//! and patterns per role, bullets per slide, how take-home messages are phrased, caption
//! and `Ref:` habits, emphasis use. The outline prompt receives [`Profile::prompt_text`],
//! never a deck.
//!
//! What it keeps: counts, shares, lengths and pattern names. What it never keeps: bullet,
//! caption or cover text, footers, confidential marks, logos (text or pictures repeated on
//! many slides, footer placeholders, small pictures) and names. Two exceptions, both
//! boilerplate: the fixed title of a closing role ("Thanks") when the decks agree on it,
//! and common title opening words that recur across several decks.
//!
//! Only decks imported from the author's own files count (most slides carry `src`):
//! decks SlideComposer generated (論文報告) never feed the profile back.
//!
//! Cached in the workspace's cache tier (`cache/deck-style/profile.json`), keyed by the
//! decks' folder names and `deck.json` mtimes/sizes and [`VERSION`]; any change rebuilds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lore_canvas::{Bullet, Deck, Kind, Object, Slide, TextBody};
use serde::{Deserialize, Serialize};

/// Bump when the profile's fields or rules change (invalidates the cache).
pub const VERSION: u32 = 2;

/// Page roles of a paper report, in the author's order (design §7.1; same names as todeck).
pub const ROLES: [&str; 13] = ["cover", "background", "challenge", "overview", "method", "loss", "training", "results", "ablation", "visual", "opinion", "takeaway", "thanks"];

/// Roles whose title is boilerplate (may be stored verbatim when the decks agree).
pub const FIXED_ROLES: [&str; 3] = ["opinion", "takeaway", "thanks"];

/// Decks shorter than this are samples or snippets, not talks.
const MIN_SLIDES: usize = 6;

/// median / p90 / max of a count or length.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    pub n: usize,
    pub median: f64,
    pub p90: f64,
    pub max: f64,
}

impl Stat {
    pub fn of(v: &[f64]) -> Stat {
        if v.is_empty() {
            return Stat::default();
        }
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
        Stat { n: s.len(), median: q(0.5), p90: q(0.9), max: *s.last().unwrap() }
    }
}

/// One page role across the decks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleStyle {
    /// Share of all pages (0–1).
    pub share: f64,
    /// Mean pages per deck.
    pub per_deck: f64,
    pub title_words: Stat,
    pub title_chars: Stat,
    /// Title pattern → share (`colon` "Prefix: X", `question`, `sentence`, `noun-phrase`, `cjk`).
    pub patterns: BTreeMap<String, f64>,
    /// Share of titles in Title Case (vs sentence case), Latin titles only.
    pub title_case: f64,
    /// Share of titles ending with a period.
    pub period: f64,
    /// The decks agree on this title (closing roles only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_title: Option<String>,
    /// Opening words recurring across decks (generic words only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openers: Vec<String>,
    /// Bullet paragraphs per page.
    pub bullets: Stat,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TakeHome {
    pub slides: usize,
    /// Share of take-home slides with numbered items.
    pub numbered: f64,
    pub items: Stat,
    pub item_words: Stat,
    /// Shares of items: starting with a verb, ending with a period, mostly CJK, using we/our.
    pub verb_start: f64,
    pub period: f64,
    pub cjk: f64,
    pub we: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Captions {
    /// Content pages with a sizeable picture.
    pub figure_slides: usize,
    /// Share of those with a caption.
    pub with_caption: f64,
    /// `Fig.` or `Figure` (the form the decks use more), "" when neither appears.
    pub fig_form: String,
    pub fig_abbrev: usize,
    pub fig_full: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Refs {
    /// Share of content pages with a reference line.
    pub share: f64,
    /// Role → share of its pages with a reference line.
    pub by_role: BTreeMap<String, f64>,
    /// The prefix the decks use (`Ref:`), "" when none.
    pub prefix: String,
    /// Share of reference lines in the bottom quarter of the slide.
    pub bottom: f64,
    /// Reference lines seen (the prefix counts from 3 on).
    pub lines: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Emphasis {
    /// Emphasis tag → mean spans per content page.
    pub per_page: BTreeMap<String, f64>,
    /// Share of content pages with a key-result emphasis.
    pub key_pages: f64,
    /// Most key-result spans on one page.
    pub key_max: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub decks: usize,
    pub pages: usize,
    pub pages_per_deck: Stat,
    pub roles: BTreeMap<String, RoleStyle>,
    /// Bullet paragraphs per content page (all roles).
    pub bullets: Stat,
    pub take_home: TakeHome,
    pub captions: Captions,
    pub refs: Refs,
    pub emphasis: Emphasis,
}

// ── Reading a deck ───────────────────────────────────────────────────────────

/// Placeholders that are never content.
pub fn chrome_ph(o: &Object) -> bool {
    matches!(o.ph.as_deref(), Some("ftr" | "sldNum" | "dt" | "hdr"))
}

pub fn body_of(o: &Object) -> Option<&TextBody> {
    match &o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => sp.text.as_ref(),
        _ => None,
    }
}

pub fn norm_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Text and pictures repeated on many slides of one deck (footers, confidential marks,
/// presenter lines, logos): normalised text and picture assets.
pub fn boilerplate(deck: &Deck) -> (BTreeSet<String>, BTreeSet<String>) {
    let n = deck.slides.len();
    let mut texts: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut pics: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for (i, s) in deck.slides.iter().enumerate() {
        s.walk(&mut |o| {
            if let Some(b) = body_of(o) {
                let t = norm_text(&b.plain_text());
                if !t.is_empty() {
                    texts.entry(t).or_default().insert(i);
                }
            }
            if let Kind::Picture(p) = &o.kind {
                pics.entry(p.asset.clone()).or_default().insert(i);
            }
        });
    }
    let many = |c: usize| n >= 3 && c * 10 >= n * 4;
    // a title repeated by a reveal is not boilerplate: those slides also share the title
    let tx = texts.into_iter().filter(|(_, v)| many(v.len())).map(|(k, _)| k).collect();
    let px = pics.into_iter().filter(|(_, v)| many(v.len())).map(|(k, _)| k).collect();
    (tx, px)
}

/// The title object of a slide.
pub fn title_object(s: &Slide, slide_h: i64) -> Option<&Object> {
    let txt = |o: &&Object| body_of(o).is_some_and(|b| !b.plain_text().trim().is_empty()) && !o.hidden;
    s.objects
        .iter()
        .filter(txt)
        .find(|o| matches!(o.ph.as_deref(), Some("title" | "ctrTitle")))
        .or_else(|| s.objects.iter().filter(txt).find(|o| o.slot.as_deref().is_some_and(|x| x.starts_with("title")) || o.name == "Title"))
        .or_else(|| {
            s.objects
                .iter()
                .filter(txt)
                .filter(|o| !chrome_ph(o) && o.frame.y < slide_h / 4 && o.name != "kicker")
                .filter(|o| body_of(o).is_some_and(|b| b.paras.iter().any(|p| p.runs.iter().any(|r| r.tag.as_deref() == Some("title")))))
                .min_by_key(|o| o.frame.y)
        })
}

pub fn title_text(s: &Slide, slide_h: i64) -> String {
    title_object(s, slide_h).and_then(body_of).map(|b| b.plain_text().split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

fn has(t: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| t.contains(k))
}

/// Role of one title by its words alone (None: no keyword).
pub fn role_of_title(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    if t.trim().is_empty() {
        return None;
    }
    let rules: [(&str, &[&str]); 12] = [
        ("thanks", &["thank", "謝謝", "感謝", "q&a", "q & a", "any question"]),
        ("takeaway", &["take home", "take-home", "takeaway", "take away", "key message", "lessons", "conclusion", "結論", "總結", "summary"]),
        ("opinion", &["opinion", "discussion", "comment", "my view", "thought", "limitation", "心得", "討論", "看法"]),
        ("ablation", &["ablation", "消融"]),
        ("visual", &["visuali", "qualitative", "case stud", "failure case", "examples", "視覺化"]),
        ("loss", &["loss", "objective", "損失"]),
        ("training", &["training detail", "implementation", "experimental setup", "setup", "dataset", "training", "訓練", "實作", "資料集"]),
        ("results", &["result", "experiment", "performance", "comparison", "evaluation", "benchmark", "結果", "實驗", "比較"]),
        ("overview", &["overview", "framework", "architecture", "pipeline", "big picture", "proposed", "our approach", "架構", "流程"]),
        ("challenge", &["challenge", "problem", "motivation", "intuition", "挑戰", "動機", "問題"]),
        ("background", &["background", "introduction", "related work", "prior work", "背景", "簡介", "介紹"]),
        ("method", &["method", "module", "step", "方法", "模組"]),
    ];
    rules.iter().find(|(_, k)| has(&t, k)).map(|(r, _)| *r)
}

/// Roles of a deck's slides from their titles: the first slide is the cover, the same title
/// as the previous slide keeps its role (reveal steps), keywords decide, and an untitled /
/// unmatched slide is background before the method part and keeps the running role after.
pub fn classify(titles: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(titles.len());
    let mut seen_method = false;
    for (i, t) in titles.iter().enumerate() {
        let role = if i == 0 {
            "cover".to_string()
        } else if i > 0 && !t.trim().is_empty() && norm_text(t) == norm_text(&titles[i - 1]) {
            out[i - 1].clone()
        } else if let Some(r) = role_of_title(t) {
            r.to_string()
        } else if i + 1 == titles.len() && t.trim().is_empty() {
            "thanks".to_string()
        } else if !seen_method {
            "background".to_string()
        } else {
            match out.last().map(String::as_str) {
                Some("cover" | "background" | "challenge" | "thanks") | None => "method".to_string(),
                Some("overview") => "method".to_string(),
                Some(r) => r.to_string(),
            }
        };
        if matches!(role.as_str(), "overview" | "method" | "loss" | "training" | "results" | "ablation") {
            seen_method = true;
        }
        out.push(role);
    }
    // keyword-less slides between the last background page and the first loss / training /
    // results page are the method's modules ("Encoder", "Fusion")
    let first_late = out.iter().position(|r| matches!(r.as_str(), "loss" | "training" | "results" | "ablation"));
    let last_bg = (0..first_late.unwrap_or(0)).rev().find(|&i| matches!(out[i].as_str(), "challenge" | "overview") || (out[i] == "background" && role_of_title(&titles[i]) == Some("background")));
    if let (Some(end), Some(start)) = (first_late, last_bg) {
        for i in start + 1..end {
            if out[i] == "background" && role_of_title(&titles[i]).is_none() {
                out[i] = "method".into();
            }
        }
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

pub fn mostly_cjk(s: &str) -> bool {
    let c = s.chars().filter(|c| is_cjk(*c)).count();
    let l = s.chars().filter(|c| c.is_ascii_alphabetic()).count();
    c > 0 && c * 2 >= l
}

/// Words of a title (CJK: characters count as words / 2).
pub fn word_count(s: &str) -> usize {
    if mostly_cjk(s) {
        (s.chars().filter(|c| is_cjk(*c)).count() + 1) / 2
    } else {
        s.split_whitespace().count()
    }
}

const SENTENCE_VERBS: [&str; 16] = ["is", "are", "was", "were", "can", "does", "do", "improves", "outperforms", "shows", "helps", "works", "matters", "fails", "beats", "makes"];

/// Pattern name of a title: `colon`, `question`, `sentence`, `cjk`, `noun-phrase`.
pub fn title_pattern(t: &str) -> &'static str {
    let t = t.trim();
    if let Some((a, b)) = t.split_once([':', '：'])
        && !a.trim().is_empty()
        && !b.trim().is_empty()
    {
        return "colon";
    }
    if t.ends_with('?') || t.ends_with('？') {
        return "question";
    }
    if mostly_cjk(t) {
        return "cjk";
    }
    let words: Vec<String> = t.split_whitespace().map(|w| w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()).to_string()).collect();
    if t.ends_with('.') || words.iter().skip(1).any(|w| SENTENCE_VERBS.contains(&w.as_str())) {
        return "sentence";
    }
    "noun-phrase"
}

const SMALL: [&str; 14] = ["a", "an", "the", "of", "and", "or", "in", "on", "for", "to", "with", "by", "vs", "via"];

/// Title Case: most longer words after the first start upper case.
pub fn is_title_case(t: &str) -> bool {
    let words: Vec<&str> = t.split_whitespace().skip(1).filter(|w| w.chars().next().is_some_and(|c| c.is_alphabetic()) && !SMALL.contains(&w.to_lowercase().as_str())).collect();
    if words.is_empty() {
        return false;
    }
    let up = words.iter().filter(|w| w.chars().next().is_some_and(|c| c.is_uppercase())).count();
    up * 10 >= words.len() * 7
}

const VERBS: [&str; 48] = [
    "use", "uses", "try", "consider", "keep", "avoid", "combine", "combines", "improve", "improves", "show", "shows", "help", "helps", "make", "makes", "reduce", "reduces", "learn", "learns",
    "add", "adds", "need", "needs", "remember", "check", "choose", "pick", "start", "build", "builds", "train", "fuse", "fuses", "beat", "beats", "outperform", "outperforms", "achieve",
    "achieves", "enable", "enables", "provide", "provides", "matter", "matters", "work", "works",
];

fn starts_with_verb(s: &str) -> bool {
    s.split_whitespace().next().is_some_and(|w| VERBS.contains(&w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric())))
}

/// Non-empty paragraphs counted as bullets of a body object.
pub fn bullet_count(o: &Object) -> usize {
    let Some(b) = body_of(o) else { return 0 };
    let body_ph = matches!(o.ph.as_deref(), Some("body" | "obj"));
    b.paras
        .iter()
        .filter(|p| p.runs.iter().any(|r| !r.text.trim().is_empty() && r.em.as_deref() != Some(lore_canvas::style::GHOST_EM)))
        .filter(|p| body_ph || !matches!(p.bullet, Bullet::None))
        .count()
}

static REF_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^\s*(ref|refs|reference|references|參考(?:文獻|資料)?)\s*[:：]").unwrap());
static FIG_ABBR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\bFigs?\.").unwrap());
static FIG_FULL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\b[Ff]igures?\b").unwrap());

/// The reference-line prefix of a text (`Ref:`), if it starts with one.
pub fn ref_prefix(text: &str) -> Option<String> {
    REF_RE.find(text).map(|m| m.as_str().trim().to_string())
}

pub fn fig_counts(text: &str) -> (usize, usize) {
    (FIG_ABBR.find_iter(text).count(), FIG_FULL.find_iter(text).count())
}

/// Visible pictures of at least `frac` of the slide area (not boilerplate, not ghosted).
pub fn big_pictures<'a>(s: &'a Slide, area: f64, frac: f64, skip: &BTreeSet<String>) -> Vec<&'a Object> {
    s.objects
        .iter()
        .filter(|o| !o.hidden && !o.is_empty_slot())
        .filter(|o| matches!(&o.kind, Kind::Picture(p) if p.alpha >= 0.5 && !skip.contains(&p.asset)))
        .filter(|o| (o.frame.w as f64 * o.frame.h as f64) >= area * frac)
        .collect()
}

/// A caption for picture `p`: a caption-tagged run anywhere on the slide, a `圖說` box, or a
/// short text box right under or beside it.
pub fn has_caption(s: &Slide, p: &Object, h: i64) -> bool {
    s.objects.iter().filter(|o| o.id != p.id && !o.hidden).any(|o| {
        let Some(b) = body_of(o) else { return false };
        let t = b.plain_text();
        if t.trim().is_empty() || ref_prefix(&t).is_some() {
            return false;
        }
        if o.name == "圖說" || o.slot.as_deref().is_some_and(|x| x.starts_with("caption")) || b.paras.iter().any(|q| q.runs.iter().any(|r| r.tag.as_deref() == Some("caption"))) {
            return true;
        }
        let short = t.chars().count() <= 220 && b.paras.len() <= 3;
        let (pf, of) = (&p.frame, &o.frame);
        let overlap_x = of.x < pf.x + pf.w && of.x + of.w > pf.x;
        let under = overlap_x && of.y >= pf.y + pf.h - h / 50 && of.y <= pf.y + pf.h + h / 8;
        let starts = t.trim_start().starts_with("Fig") || t.trim_start().starts_with('圖');
        short && (under || starts)
    })
}

/// Key-result emphasis spans on a slide: (object id, para, first run) per span.
pub fn key_spans(s: &Slide, is_key: &dyn Fn(&str) -> bool) -> Vec<(u32, usize, usize)> {
    let mut out = vec![];
    s.walk(&mut |o| {
        let Some(b) = body_of(o) else { return };
        for (pi, p) in b.paras.iter().enumerate() {
            let mut prev = false;
            for (ri, r) in p.runs.iter().enumerate() {
                let k = r.em.as_deref().is_some_and(is_key) && !r.text.trim().is_empty();
                if k && !prev {
                    out.push((o.id, pi, ri));
                }
                // a whitespace run between two key runs does not split the span
                prev = k || (prev && r.text.trim().is_empty());
            }
        }
    });
    out
}

pub fn is_key_em(e: &str) -> bool {
    e == "key" || e.starts_with("key-")
}

// ── Building the profile ─────────────────────────────────────────────────────

#[derive(Default)]
struct Acc {
    decks: usize,
    pages: usize,
    pages_per_deck: Vec<f64>,
    role_pages: BTreeMap<String, usize>,
    role_decks: BTreeMap<String, usize>,
    title_words: BTreeMap<String, Vec<f64>>,
    title_chars: BTreeMap<String, Vec<f64>>,
    patterns: BTreeMap<String, BTreeMap<String, usize>>,
    titles_n: BTreeMap<String, usize>,
    latin_n: BTreeMap<String, usize>,
    title_case: BTreeMap<String, usize>,
    period: BTreeMap<String, usize>,
    fixed: BTreeMap<String, BTreeMap<String, (String, BTreeSet<usize>)>>,
    openers: BTreeMap<String, BTreeMap<String, BTreeSet<usize>>>,
    role_bullets: BTreeMap<String, Vec<f64>>,
    bullets: Vec<f64>,
    th: TakeHome,
    th_items: Vec<f64>,
    th_words: Vec<f64>,
    th_counts: [usize; 5],
    fig_slides: usize,
    fig_captioned: usize,
    fig: (usize, usize),
    content_pages: usize,
    ref_pages: usize,
    ref_role: BTreeMap<String, usize>,
    prefixes: BTreeMap<String, usize>,
    ref_bottom: usize,
    ref_lines: usize,
    em_spans: BTreeMap<String, usize>,
    key_pages: usize,
    key_max: usize,
}

fn add_deck(acc: &mut Acc, deck_ix: usize, deck: &Deck) {
    let (w, h) = (deck.size.w.max(1), deck.size.h.max(1));
    let area = w as f64 * h as f64;
    let (bp_text, bp_pics) = boilerplate(deck);
    let titles: Vec<String> = deck.slides.iter().map(|s| title_text(s, h)).collect();
    let roles = classify(&titles);
    acc.decks += 1;
    acc.pages_per_deck.push(deck.slides.len() as f64);
    let mut roles_here = BTreeSet::new();
    for ((s, title), role) in deck.slides.iter().zip(&titles).zip(&roles) {
        if s.hidden {
            continue;
        }
        acc.pages += 1;
        *acc.role_pages.entry(role.clone()).or_default() += 1;
        roles_here.insert(role.clone());
        let title_id = title_object(s, h).map(|o| o.id);
        // titles: never the cover's (paper title, presenter)
        if role != "cover" && !title.is_empty() && !bp_text.contains(&norm_text(title)) {
            *acc.titles_n.entry(role.clone()).or_default() += 1;
            acc.title_words.entry(role.clone()).or_default().push(word_count(title) as f64);
            acc.title_chars.entry(role.clone()).or_default().push(title.chars().count() as f64);
            *acc.patterns.entry(role.clone()).or_default().entry(title_pattern(title).into()).or_default() += 1;
            if !mostly_cjk(title) {
                *acc.latin_n.entry(role.clone()).or_default() += 1;
                if is_title_case(title) {
                    *acc.title_case.entry(role.clone()).or_default() += 1;
                }
            }
            if title.ends_with('.') || title.ends_with('。') {
                *acc.period.entry(role.clone()).or_default() += 1;
            }
            if FIXED_ROLES.contains(&role.as_str()) {
                let e = acc.fixed.entry(role.clone()).or_default().entry(norm_text(title)).or_insert_with(|| (title.clone(), BTreeSet::new()));
                e.1.insert(deck_ix);
            } else if role != "thanks"
                && let Some(w) = title.split_whitespace().next()
            {
                let w = w.trim_matches(|c: char| !c.is_alphanumeric());
                if (2..=12).contains(&w.len()) && w.chars().all(|c| c.is_ascii_alphabetic()) {
                    acc.openers.entry(role.clone()).or_default().entry(w.to_string()).or_default().insert(deck_ix);
                }
            }
        }
        if matches!(role.as_str(), "cover" | "thanks") {
            continue;
        }
        acc.content_pages += 1;
        // bullets (body objects only)
        let content: Vec<&Object> = s.objects.iter().filter(|o| !o.hidden && !chrome_ph(o) && Some(o.id) != title_id).filter(|o| body_of(o).is_some_and(|b| !bp_text.contains(&norm_text(&b.plain_text())))).collect();
        let n_b: usize = content.iter().filter(|o| body_of(o).is_some_and(|b| ref_prefix(&b.plain_text()).is_none())).map(|o| bullet_count(o)).sum();
        acc.bullets.push(n_b as f64);
        acc.role_bullets.entry(role.clone()).or_default().push(n_b as f64);
        // take home phrasing
        if role == "takeaway" {
            acc.th.slides += 1;
            let mut items = 0;
            let mut numbered = false;
            for o in &content {
                let Some(b) = body_of(o) else { continue };
                for p in &b.paras {
                    let t: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    let t = t.trim();
                    if t.is_empty() {
                        continue;
                    }
                    items += 1;
                    numbered |= matches!(p.bullet, Bullet::Num { .. }) || t.chars().next().is_some_and(|c| c.is_ascii_digit());
                    acc.th_words.push(word_count(t) as f64);
                    let c = &mut acc.th_counts;
                    c[0] += 1;
                    c[1] += starts_with_verb(t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ')) as usize;
                    c[2] += (t.ends_with('.') || t.ends_with('。')) as usize;
                    c[3] += mostly_cjk(t) as usize;
                    let lw = format!(" {} ", t.to_lowercase());
                    c[4] += (lw.contains(" we ") || lw.contains(" our ")) as usize;
                }
            }
            acc.th_items.push(items as f64);
            acc.th.numbered += numbered as usize as f64;
        }
        // captions and Fig./Figure
        let pics = big_pictures(s, area, 0.08, &bp_pics);
        if !pics.is_empty() {
            acc.fig_slides += 1;
            if pics.iter().any(|p| has_caption(s, p, h)) {
                acc.fig_captioned += 1;
            }
        }
        let mut has_ref = false;
        for o in &content {
            let t = body_of(o).map(|b| b.plain_text()).unwrap_or_default();
            let (a, f) = fig_counts(&t);
            acc.fig.0 += a;
            acc.fig.1 += f;
            for line in t.lines() {
                if let Some(p) = ref_prefix(line) {
                    has_ref = true;
                    acc.ref_lines += 1;
                    *acc.prefixes.entry(p).or_default() += 1;
                    if o.frame.y >= h * 3 / 4 {
                        acc.ref_bottom += 1;
                    }
                }
            }
        }
        if has_ref {
            acc.ref_pages += 1;
            *acc.ref_role.entry(role.clone()).or_default() += 1;
        }
        // emphasis
        let mut ems: BTreeMap<String, usize> = BTreeMap::new();
        for o in &content {
            let Some(b) = body_of(o) else { continue };
            for p in &b.paras {
                let mut prev: Option<&str> = None;
                for r in &p.runs {
                    let e = r.em.as_deref().filter(|e| *e != lore_canvas::style::GHOST_EM && !r.text.trim().is_empty());
                    if let Some(e) = e
                        && prev != Some(e)
                    {
                        *ems.entry(e.to_string()).or_default() += 1;
                    }
                    prev = e.or(if r.text.trim().is_empty() { prev } else { None });
                }
            }
        }
        let keys: usize = ems.iter().filter(|(e, _)| is_key_em(e)).map(|(_, n)| n).sum();
        if keys > 0 {
            acc.key_pages += 1;
        }
        acc.key_max = acc.key_max.max(keys);
        for (e, n) in ems {
            *acc.em_spans.entry(e).or_default() += n;
        }
    }
    for r in roles_here {
        *acc.role_decks.entry(r).or_default() += 1;
    }
}

fn share(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { ((a as f64 / b as f64) * 100.0).round() / 100.0 }
}

fn finish(acc: Acc) -> Profile {
    let mut roles = BTreeMap::new();
    let min_decks = if acc.decks >= 3 { 3 } else { acc.decks.max(1) };
    for r in ROLES {
        let pages = acc.role_pages.get(r).copied().unwrap_or(0);
        if pages == 0 {
            continue;
        }
        let tn = acc.titles_n.get(r).copied().unwrap_or(0);
        let patterns = acc.patterns.get(r).map(|m| m.iter().map(|(k, v)| (k.clone(), share(*v, tn))).collect()).unwrap_or_default();
        let fixed_title = acc.fixed.get(r).and_then(|m| {
            let (orig, decks) = m.values().max_by_key(|(_, d)| d.len())?;
            let total: usize = acc.role_decks.get(r).copied().unwrap_or(0);
            let generic = !orig.chars().any(|c| c.is_ascii_digit()) && word_count(orig) <= 5 && orig.chars().count() <= 40;
            (generic && decks.len() >= 2.min(total) && decks.len() * 2 > total).then(|| orig.clone())
        });
        let mut openers: Vec<(usize, String)> = acc.openers.get(r).map(|m| m.iter().filter(|(_, d)| d.len() >= min_decks).map(|(w, d)| (d.len(), w.clone())).collect()).unwrap_or_default();
        openers.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        roles.insert(
            r.to_string(),
            RoleStyle {
                share: share(pages, acc.pages),
                per_deck: ((pages as f64 / acc.decks.max(1) as f64) * 10.0).round() / 10.0,
                title_words: Stat::of(acc.title_words.get(r).map(Vec::as_slice).unwrap_or(&[])),
                title_chars: Stat::of(acc.title_chars.get(r).map(Vec::as_slice).unwrap_or(&[])),
                patterns,
                title_case: share(acc.title_case.get(r).copied().unwrap_or(0), acc.latin_n.get(r).copied().unwrap_or(0)),
                period: share(acc.period.get(r).copied().unwrap_or(0), tn),
                fixed_title,
                openers: openers.into_iter().take(5).map(|(_, w)| w).collect(),
                bullets: Stat::of(acc.role_bullets.get(r).map(Vec::as_slice).unwrap_or(&[])),
            },
        );
    }
    let c = acc.th_counts;
    let take_home = TakeHome {
        slides: acc.th.slides,
        numbered: share(acc.th.numbered as usize, acc.th.slides),
        items: Stat::of(&acc.th_items),
        item_words: Stat::of(&acc.th_words),
        verb_start: share(c[1], c[0]),
        period: share(c[2], c[0]),
        cjk: share(c[3], c[0]),
        we: share(c[4], c[0]),
    };
    let fig_form = match acc.fig {
        (0, 0) => String::new(),
        (a, f) if a >= f => "Fig.".into(),
        _ => "Figure".into(),
    };
    let prefix = acc.prefixes.iter().max_by_key(|(_, n)| **n).map(|(p, _)| p.clone()).unwrap_or_default();
    let by_role = acc.ref_role.iter().map(|(r, n)| (r.clone(), share(*n, acc.role_pages.get(r).copied().unwrap_or(0)))).collect();
    Profile {
        version: VERSION,
        decks: acc.decks,
        pages: acc.pages,
        pages_per_deck: Stat::of(&acc.pages_per_deck),
        roles,
        bullets: Stat::of(&acc.bullets),
        take_home,
        captions: Captions { figure_slides: acc.fig_slides, with_caption: share(acc.fig_captioned, acc.fig_slides), fig_form, fig_abbrev: acc.fig.0, fig_full: acc.fig.1 },
        refs: Refs { share: share(acc.ref_pages, acc.content_pages), by_role, prefix, bottom: share(acc.ref_bottom, acc.ref_lines), lines: acc.ref_lines },
        emphasis: Emphasis {
            per_page: acc.em_spans.iter().map(|(e, n)| (e.clone(), share(*n, acc.content_pages))).collect(),
            key_pages: share(acc.key_pages, acc.content_pages),
            key_max: acc.key_max,
        },
    }
}

/// A deck counts when it is a talk imported from the author's own file.
pub fn qualifies(deck: &Deck) -> bool {
    let n = deck.slides.len();
    n >= MIN_SLIDES && deck.slides.iter().filter(|s| s.src.is_some()).count() * 2 >= n
}

/// The profile of these decks (None when none qualifies). Identical decks count once.
pub fn profile_of(decks: &[Deck]) -> Option<Profile> {
    let mut acc = Acc::default();
    let mut seen = BTreeSet::new();
    for d in decks.iter().filter(|d| qualifies(d)) {
        let sig = lore_canvas::fp(&d.slides);
        if !seen.insert(sig) {
            continue;
        }
        add_deck(&mut acc, seen.len() - 1, d);
    }
    (acc.decks > 0).then(|| finish(acc))
}

// ── Cache ────────────────────────────────────────────────────────────────────

/// `deck.json` files of a decks folder, sorted, with (mtime ns, size).
fn deck_files(decks_dir: &Path) -> Vec<(String, u128, u64)> {
    let mut v = vec![];
    for e in std::fs::read_dir(decks_dir).into_iter().flatten().flatten() {
        let p = e.path().join("deck.json");
        if let Ok(m) = std::fs::metadata(&p) {
            let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            v.push((e.file_name().to_string_lossy().into_owned(), t, m.len()));
        }
    }
    v.sort();
    v
}

/// Cache key of a decks folder: [`VERSION`] and every deck.json's name, mtime and size.
pub fn cache_key(decks_dir: &Path) -> String {
    format!("v{VERSION}-{:016x}", lore_canvas::fp(&deck_files(decks_dir)))
}

#[derive(Serialize, Deserialize)]
struct Cached {
    key: String,
    profile: Option<Profile>,
}

/// Where the cached profile lives in a workspace's cache tier.
pub fn cache_file(cache_root: &Path) -> PathBuf {
    cache_root.join("deck-style").join("profile.json")
}

/// The profile of `decks_dir`, from the cache when its key still matches. Read-only on the
/// decks; the only write is the cache file.
pub fn load_or_build(decks_dir: &Path, cache_root: &Path) -> Option<Profile> {
    let key = cache_key(decks_dir);
    let file = cache_file(cache_root);
    if let Some(c) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Cached>(&b).ok())
        && c.key == key
    {
        return c.profile;
    }
    let decks: Vec<Deck> = deck_files(decks_dir)
        .iter()
        .filter_map(|(name, _, _)| std::fs::read(decks_dir.join(name).join("deck.json")).ok())
        .filter_map(|b| serde_json::from_slice::<Deck>(&b).ok())
        .collect();
    let profile = profile_of(&decks);
    if let Some(parent) = file.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let _ = sc_core::write_atomic(&file, &serde_json::to_vec_pretty(&Cached { key, profile: profile.clone() }).unwrap_or_default());
    }
    profile
}

/// The author's decks folder through lore-home (`private/SlideComposer/decks`).
pub fn author_decks(home: &lore_home::Home) -> Option<PathBuf> {
    home.app_dir("SlideComposer").map(|d| d.join("decks")).filter(|d| d.is_dir())
}

/// The profile for a workspace: the author's decks via lore-home, cached under `cache_root`
/// (the workspace's `cache/`).
pub fn for_home(home: &lore_home::Home, cache_root: &Path) -> Option<Profile> {
    load_or_build(&author_decks(home)?, cache_root)
}

// ── For the outline prompt ───────────────────────────────────────────────────

fn pct(x: f64) -> String {
    format!("{}%", (x * 100.0).round() as i64)
}

fn role_label(r: &str) -> &'static str {
    match r {
        "cover" => "cover",
        "background" => "background",
        "challenge" => "challenge",
        "overview" => "big picture",
        "method" => "method",
        "loss" => "loss",
        "training" => "training details",
        "results" => "results",
        "ablation" => "ablation",
        "visual" => "visualisation",
        "opinion" => "opinions",
        "takeaway" => "take home",
        "thanks" => "thanks",
        _ => "other",
    }
}

impl Profile {
    /// Bullet limit for a role: the role's max when it has a few pages, else the overall p90.
    pub fn bullet_limit(&self, role: &str) -> usize {
        let r = self.roles.get(role).map(|r| &r.bullets);
        let v = match r {
            Some(s) if s.n >= 3 => s.max.max(self.bullets.p90),
            _ => self.bullets.p90,
        };
        if v < 1.0 { 5 } else { v.round() as usize }
    }

    /// The compact text the outline prompt receives.
    pub fn prompt_text(&self) -> String {
        let mut o = format!("The presenter's earlier paper-report decks ({} decks, about {} slides each) follow this style; match it:\n", self.decks, self.pages_per_deck.median.round() as i64);
        let shares: Vec<String> = ROLES.iter().filter_map(|r| self.roles.get(*r).map(|s| format!("{} {}", role_label(r), pct(s.share)))).collect();
        o += &format!("- Pages per role: {}.\n", shares.join(", "));
        for r in ROLES {
            let Some(s) = self.roles.get(r) else { continue };
            if s.title_words.n == 0 {
                continue;
            }
            let mut parts = vec![format!("median {} words, at most {}", s.title_words.median.round() as i64, s.title_words.max.round() as i64)];
            let mut pats: Vec<(&String, &f64)> = s.patterns.iter().filter(|(_, v)| **v >= 0.2).collect();
            pats.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
            if !pats.is_empty() {
                parts.push(pats.iter().map(|(k, v)| format!("{} {}", k.replace('-', " "), pct(**v))).collect::<Vec<_>>().join(", "));
            }
            if s.title_case >= 0.7 {
                parts.push("Title Case".into());
            } else if s.title_case <= 0.3 && s.patterns.get("cjk").copied().unwrap_or(0.0) < 0.5 {
                parts.push("sentence case".into());
            }
            if s.period == 0.0 {
                parts.push("no final period".into());
            }
            if let Some(f) = &s.fixed_title {
                parts.push(format!("always \"{f}\""));
            }
            if !s.openers.is_empty() {
                parts.push(format!("often starts with {}", s.openers.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(" / ")));
            }
            o += &format!("- {} titles: {}.\n", role_label(r), parts.join("; "));
        }
        let mut per_role: Vec<String> = vec![];
        for r in ROLES {
            if let Some(s) = self.roles.get(r).filter(|s| s.bullets.n >= 3 && !matches!(r, "cover" | "thanks")) {
                per_role.push(format!("{} ≤{}", role_label(r), s.bullets.max.round() as i64));
            }
        }
        o += &format!("- Bullets per slide: median {}, at most {}{}.\n", self.bullets.median.round() as i64, self.bullet_limit(""), if per_role.is_empty() { String::new() } else { format!(" ({})", per_role.join(", ")) });
        let t = &self.take_home;
        if t.slides > 0 {
            o += &format!(
                "- Take home: {} items{}, about {} words each; start with a verb {}, end with a period {}, use we/our {}.\n",
                t.items.median.round() as i64,
                if t.numbered >= 0.5 { ", numbered" } else { "" },
                t.item_words.median.round() as i64,
                pct(t.verb_start),
                pct(t.period),
                pct(t.we)
            );
        }
        let c = &self.captions;
        if c.figure_slides > 0 {
            o += &format!("- Captions under {} of figures{}.\n", pct(c.with_caption), if c.fig_form.is_empty() { String::new() } else { format!("; write \"{}\"", c.fig_form) });
        }
        let r = &self.refs;
        if !r.prefix.is_empty() && r.lines >= 3 {
            let mut top: Vec<(&String, &f64)> = r.by_role.iter().filter(|(_, v)| **v >= 0.3).collect();
            top.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
            let roles = top.iter().take(3).map(|(k, v)| format!("{} {}", role_label(k), pct(**v))).collect::<Vec<_>>().join(", ");
            o += &format!("- \"{}\" line{} on {} of content slides{}.\n", r.prefix, if r.bottom >= 0.5 { " at the bottom" } else { "" }, pct(r.share), if roles.is_empty() { String::new() } else { format!(" ({roles})") });
        }
        let e = &self.emphasis;
        o += &format!("- Key-result emphasis (red bold) on {} of slides, never more than one per slide.\n", pct(e.key_pages));
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_from_titles() {
        let t: Vec<String> = ["Paper", "Why grading matters", "Overview", "Encoder", "Fusion", "Fusion", "Loss function", "Training details", "Results", "Ablation study", "Some opinions", "Take home messages", "Thanks"].iter().map(|s| s.to_string()).collect();
        let r = classify(&t);
        assert_eq!(r, ["cover", "background", "overview", "method", "method", "method", "loss", "training", "results", "ablation", "opinion", "takeaway", "thanks"]);
    }

    #[test]
    fn patterns() {
        assert_eq!(title_pattern("Sampling method: two stages"), "colon");
        assert_eq!(title_pattern("Does fusion help?"), "question");
        assert_eq!(title_pattern("Fusion improves grading"), "sentence");
        assert_eq!(title_pattern("Results of ablation study"), "noun-phrase");
        assert_eq!(title_pattern("合成病灶分級"), "cjk");
        assert!(is_title_case("Results of the Ablation Study"));
        assert!(!is_title_case("Results of the ablation study"));
        assert_eq!(ref_prefix("Reference: Ellrin et al."), Some("Reference:".into()));
        assert_eq!(fig_counts("Fig. 2 and Figure 3"), (1, 1));
    }

    #[test]
    fn stats() {
        let s = Stat::of(&[1.0, 2.0, 3.0, 4.0, 10.0]);
        assert_eq!((s.median, s.max, s.n), (3.0, 10.0, 5));
    }

    /// Manual look at real decks (never in CI): `SC_PROFILE_DECKS=<decks dir> SC_PROFILE_CACHE=<scratch dir>
    /// cargo test -p sc-server --lib deckstyle::tests::real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real() {
        let (Ok(d), Ok(c)) = (std::env::var("SC_PROFILE_DECKS"), std::env::var("SC_PROFILE_CACHE")) else { return };
        let p = load_or_build(Path::new(&d), Path::new(&c)).expect("a profile");
        println!("{}", p.prompt_text());
    }
}
