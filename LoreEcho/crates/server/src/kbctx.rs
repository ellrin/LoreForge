//! 知道你知道什麼 (design §3): the author's KnowledgeBase cards in the conversation.
//!
//! Cards (`nodes/<id>.json`) and this member's learning state (`progress/<member>/<id>.json`)
//! are read **straight from disk**, read-only, so this works when KnowledgeBase is not
//! running; nothing is ever written there. The only write is a review outcome
//! (`POST /api/recall/{id}` on KnowledgeBase's own server), and only while it runs.
//!
//! Each chat turn ([`Pre::run`], called by `chat::stream` before `start`):
//! 1. a KnowledgeBase request in the message ([`intent`]: 抽／隨機／複習／考我／有哪些卡／找…的卡,
//!    「記住這個」, English too) is answered by code: the cards are chosen here
//!    ([`draw`] = due first by the schedule, then random weighted to 學習中／尚無紀錄;
//!    [`list`]; a quiz card), the model only phrases it;
//! 2. otherwise [`retrieve`]: cards whose title or alias occurs in the message (then in the last
//!    few turns), plus key-word hits on title, aliases and summary; a few, within
//!    [`BUDGET_CHARS`];
//! 3. the cards go to the model as labelled lines (title, kind, state, summary, source) with a
//!    note that they are the author's own cards, and to the page as SSE `search` then `cards`.
//!
//! The server keeps nothing between turns: a quiz is carried by the page (`kb.quiz` = the card
//! asked about, `kb.cited` = the cards of the last reply) and graded by the author's own 記得／忘了
//! (`POST /api/kb/recall`).
//!
//! Where the spaces are: like KnowledgeBase itself (`KB_SPACES`, `spaces` in its `config.toml`,
//! lore-home's `private/KnowledgeBase`, never seeded); who: `KB_MEMBER`, `member` in that
//! config, `private/shared/profile.toml`, `me`. [`AppState::new`](crate::AppState::new) only looks
//! at its own home (tests, the isolated example); [`AppState::system`](crate::AppState::system)
//! turns the rest on. The matching follows PaperComposer's `kb.rs` (same rules).

use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use kb_core::{Clock, Node, Progress, SystemClock};
use lore_llm::LlmRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Shared;
use crate::chat::Turn;

const APP: &str = "KnowledgeBase";
/// Cards given to the model in one turn.
pub const MAX_CARDS: usize = 5;
/// The cards' lines together (the local model's context is small).
pub const BUDGET_CHARS: usize = 1200;
/// A summary in the prompt.
const SUMMARY_CHARS: usize = 160;
/// Earlier turns looked at for titles (besides the new message).
const RECENT_TURNS: usize = 4;
/// KnowledgeBase's port in browser mode (the desktop app records its own in ports.json).
const KB_DEFAULT_PORT: u16 = 7700;

/// One card as the model and the page see it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Card {
    pub id: String,
    pub space: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub kind: String,
    pub summary: String,
    /// learning | known | stale | "" (no learning state yet)
    pub state: String,
    /// The schedule says check it now.
    pub due: bool,
    /// `next_check_at` (unix seconds), for due order.
    #[serde(skip)]
    pub next: Option<i64>,
    /// The first source (a title or reference), "" when none.
    pub source: String,
}

impl Card {
    /// 「學習中」 etc.: the page's and the prompt's word for the state.
    pub fn state_label(&self) -> &'static str {
        match self.state.as_str() {
            "learning" => "學習中",
            "known" => "已懂",
            "stale" => "久未複習",
            _ => "尚無紀錄",
        }
    }
}

// ── where the cards are ──

/// Fixed spaces, member and KnowledgeBase address (tests, the isolated example).
#[derive(Debug, Clone)]
pub struct Pin {
    pub spaces: Vec<PathBuf>,
    pub member: String,
    /// KnowledgeBase's server (`None` = it is not running).
    pub url: Option<String>,
}

/// The KnowledgeBase this server reads (one per [`crate::AppState`]).
#[derive(Debug, Default)]
pub struct Kb {
    /// Also `KB_SPACES`/`KB_MEMBER`, KnowledgeBase's config.toml and its running server.
    system: AtomicBool,
    pin: RwLock<Option<Pin>>,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// KnowledgeBase's own `config.toml` (`$KB_HOME` or `<config dir>/KnowledgeBase`).
fn kb_config() -> Option<toml::Table> {
    let dir = env("KB_HOME").map(PathBuf::from).or_else(|| home_config_dir().map(|d| d.join(APP)))?;
    std::fs::read_to_string(dir.join("config.toml")).ok()?.parse().ok()
}

fn home_config_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support"))
    } else {
        Some(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")))
    }
}

impl Kb {
    /// The real machine: KnowledgeBase's own settings and its running server count too.
    pub fn use_system(&self) {
        self.system.store(true, Ordering::Relaxed);
    }

    /// Exactly these spaces, member and server.
    pub fn pin(&self, p: Pin) {
        *self.pin.write().unwrap_or_else(|e| e.into_inner()) = Some(p);
    }

    fn pinned(&self) -> Option<Pin> {
        self.pin.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The spaces (with a `space.toml`) and the member, as KnowledgeBase would pick them.
    pub fn locate(&self, home: &lore_home::Home) -> (Vec<PathBuf>, String) {
        if let Some(p) = self.pinned() {
            return (p.spaces.into_iter().filter(|d| d.join("space.toml").is_file()).collect(), p.member);
        }
        let system = self.system.load(Ordering::Relaxed);
        let cfg = if system { kb_config() } else { None };
        let mut v: Vec<PathBuf> = vec![];
        if system && let Some(s) = env("KB_SPACES") {
            v = s.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        }
        if v.is_empty() && let Some(t) = &cfg {
            v = t.get("spaces").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|s| s.as_str()).map(lore_store::expand_home).collect();
        }
        if v.is_empty() && let Some(d) = home.app_dir(APP) {
            v.push(d);
        }
        v.retain(|d| d.join("space.toml").is_file());
        let member = (if system { env("KB_MEMBER") } else { None })
            .or_else(|| cfg.as_ref().and_then(|t| t.get("member")?.as_str().map(str::to_string)))
            .or_else(|| home.prefs().profile.id)
            .unwrap_or_else(|| "me".into());
        (v, member)
    }

    /// KnowledgeBase's server, when it runs: its port in ports.json, else the browser-mode
    /// port when that answers like KnowledgeBase.
    pub async fn server(&self) -> Option<String> {
        if let Some(p) = self.pinned() {
            return p.url;
        }
        if !self.system.load(Ordering::Relaxed) {
            return None;
        }
        let candidates: Vec<u16> = match lore_home::ports::live().get("kb") {
            Some(p) => vec![*p, KB_DEFAULT_PORT],
            None => vec![KB_DEFAULT_PORT],
        };
        for port in candidates {
            let url = format!("http://127.0.0.1:{port}");
            if is_kb(&url).await {
                return Some(url);
            }
        }
        None
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(3)).no_proxy().build().unwrap_or_default()
}

/// Does `url` answer `/api/health` like kb-server (`member` and `assistant`)?
async fn is_kb(url: &str) -> bool {
    let Ok(r) = client().get(format!("{url}/api/health")).timeout(Duration::from_millis(600)).send().await else { return false };
    r.json::<Value>().await.is_ok_and(|v| v.get("member").is_some_and(Value::is_string) && v.get("assistant").is_some())
}

/// A review outcome through KnowledgeBase's API (never its files).
pub async fn recall(url: &str, id: &str, outcome: &str) -> Result<Value, String> {
    let r = client().post(format!("{url}/api/recall/{id}")).json(&json!({ "outcome": outcome, "via": "loreecho" })).send().await.map_err(|e| e.to_string())?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if ok { Ok(v) } else { Err(v.get("error").and_then(Value::as_str).unwrap_or("KnowledgeBase 沒有記下").to_string()) }
}

// ── reading ──

fn first_para(body: &str) -> String {
    body.split("\n\n").map(str::trim).find(|p| !p.is_empty() && !p.starts_with('#')).unwrap_or("").chars().take(300).collect()
}

fn json_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    v.sort();
    v
}

/// Every card of `spaces` with `member`'s state (read-only; unreadable files are skipped).
pub fn load(spaces: &[PathBuf], member: &str, clock: &dyn Clock) -> Vec<Card> {
    let now = clock.now();
    let iv = kb_core::Intervals::default();
    let mut out = vec![];
    for root in spaces {
        let Ok(space) = lore_store::Space::open(root) else { continue };
        let progress_dir = space.progress_dir(member).ok();
        for f in json_files(&space.nodes_dir()) {
            let Some(n) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<Node>(&b).ok()).filter(|n| !n.id.is_empty() && !n.title.trim().is_empty()) else { continue };
            let p = progress_dir.as_ref().map(|d| d.join(format!("{}.json", n.id))).and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Progress>(&b).ok()).map(|mut p| {
                p.refresh(now, &iv); // stale is computed on read, as KnowledgeBase does
                p
            });
            let summary = if n.summary.trim().is_empty() { first_para(&n.body) } else { n.summary.trim().to_string() };
            let source = n.sources.first().map(|s| s.extra.get("title").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| s.reference.clone())).unwrap_or_default();
            out.push(Card {
                id: n.id,
                space: space.id().to_string(),
                title: n.title.trim().to_string(),
                aliases: n.aliases.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect(),
                kind: n.kind,
                summary,
                state: p.as_ref().map(|p| p.state.as_str().to_string()).unwrap_or_default(),
                due: p.as_ref().is_some_and(|p| p.is_due(now)),
                next: p.as_ref().and_then(|p| p.next_check_at).map(|t| t.timestamp()),
                source,
            });
        }
    }
    out
}

// ── matching (PaperComposer's kb.rs rules) ──

fn is_word(c: char) -> bool {
    c.is_alphanumeric() && c.is_ascii()
}

fn cjk(c: char) -> bool {
    !c.is_ascii() && c.is_alphanumeric()
}

/// Does `name` (lowercase) occur in `text` (lowercase): Latin names only as whole words (a
/// plural s/es is fine), CJK anywhere; at least two characters.
pub fn occurs(text: &str, name: &str) -> bool {
    if name.chars().count() < 2 {
        return false;
    }
    let latin = name.chars().any(is_word) && !name.chars().any(cjk);
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let (a, z) = (from + at, from + at + name.len());
        if !latin {
            return true;
        }
        let before = text[..a].chars().next_back();
        let rest = &text[z..];
        let after = rest.strip_prefix("es").or_else(|| rest.strip_prefix('s')).filter(|r| !r.starts_with(is_word)).unwrap_or(rest);
        if !before.is_some_and(is_word) && !after.chars().next().is_some_and(is_word) {
            return true;
        }
        from = z;
    }
    false
}

/// The longest title or alias of `c` in `text`.
fn named_in(c: &Card, text: &str) -> Option<usize> {
    std::iter::once(&c.title).chain(c.aliases.iter()).map(|n| n.to_lowercase()).filter(|n| occurs(text, n)).map(|n| n.chars().count()).max()
}

const STOP_EN: &[&str] = &["the", "and", "for", "what", "how", "why", "this", "that", "with", "about", "you", "can", "are", "was", "does", "have", "from", "into", "card", "cards", "tell", "explain", "please"];
/// Two-character pieces that say nothing about a topic.
const STOP_ZH: &[&str] = &["什麼", "怎麼", "為什", "麼是", "可以", "知道", "一下", "我們", "你們", "他們", "這個", "那個", "就是", "還是", "因為", "所以", "如果", "然後", "但是", "不是", "是什", "的是", "一個", "有沒", "沒有", "現在", "今天", "覺得", "幫我", "告訴", "說明", "解釋", "知識", "識庫", "卡片"];

/// The message's key words: Latin words of 3+ letters, and two-character pieces of CJK runs.
pub fn key_words(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let mut out: Vec<String> = vec![];
    for w in text.split(|c: char| !is_word(c)).filter(|w| w.len() >= 3 && !STOP_EN.contains(w)) {
        out.push(w.to_string());
    }
    let mut run: Vec<char> = vec![];
    let flush = |run: &mut Vec<char>, out: &mut Vec<String>| {
        for p in run.windows(2) {
            let s: String = p.iter().collect();
            if !STOP_ZH.contains(&s.as_str()) && !p.iter().any(|c| "的了嗎呢吧啊是在有和與我你他她它".contains(*c)) {
                out.push(s);
            }
        }
        run.clear();
    };
    for c in text.chars() {
        if cjk(c) { run.push(c) } else { flush(&mut run, &mut out) }
    }
    flush(&mut run, &mut out);
    out.sort();
    out.dedup();
    out
}

/// The cards for this turn, best first: a title or alias in the new message, then in the last
/// few turns, then key-word hits (title and aliases count double; a summary-only hit needs two
/// words). At most `limit`.
pub fn retrieve(cards: &[Card], turns: &[Turn], limit: usize) -> Vec<Card> {
    let Some((last, before)) = turns.split_last() else { return vec![] };
    let now = last.text.to_lowercase();
    let recent = before.iter().rev().take(RECENT_TURNS).map(|t| t.text.to_lowercase()).collect::<Vec<_>>().join("\n");
    let words = key_words(&last.text);
    let mut hits: Vec<(usize, &Card)> = vec![];
    for c in cards {
        let score = if let Some(n) = named_in(c, &now) {
            10_000 + n
        } else if let Some(n) = named_in(c, &recent) {
            5_000 + n
        } else {
            let names = std::iter::once(&c.title).chain(c.aliases.iter()).map(|s| s.to_lowercase()).collect::<Vec<_>>().join(" ");
            let summary = c.summary.to_lowercase();
            let in_names = words.iter().filter(|w| names.contains(w.as_str())).count();
            let in_summary = words.iter().filter(|w| summary.contains(w.as_str())).count();
            let s = in_names * 2 + in_summary;
            if s < 2 {
                continue;
            }
            s
        };
        hits.push((score, c));
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
    hits.into_iter().take(limit).map(|(_, c)| c.clone()).collect()
}

/// Cards about `topic` (title and aliases first, then summary); every card when it is empty
/// (due first, then by title).
pub fn list(cards: &[Card], topic: &str, limit: usize) -> (usize, Vec<Card>) {
    let t = topic.trim().to_lowercase();
    let mut hits: Vec<(u8, &Card)> = cards
        .iter()
        .filter_map(|c| {
            if t.is_empty() {
                return Some((if c.due { 0 } else { 1 }, c));
            }
            let names = std::iter::once(&c.title).chain(c.aliases.iter()).map(|s| s.to_lowercase()).collect::<Vec<_>>().join(" ");
            if names.contains(&t) {
                Some((0, c))
            } else if c.summary.to_lowercase().contains(&t) || c.kind.to_lowercase() == t {
                Some((1, c))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    (hits.len(), hits.into_iter().take(limit).map(|(_, c)| c.clone()).collect())
}

/// `n` cards: due ones first (oldest check first), then a random pick weighted to 尚無紀錄 and
/// 學習中 (3), 久未複習 (2), 已懂 (1). `seed` makes the pick repeatable.
pub fn draw(cards: &[Card], n: usize, seed: u64) -> Vec<Card> {
    let mut due: Vec<&Card> = cards.iter().filter(|c| c.due).collect();
    due.sort_by(|a, b| a.next.cmp(&b.next).then_with(|| a.title.cmp(&b.title)));
    let mut out: Vec<Card> = due.into_iter().take(n).cloned().collect();
    let mut rest: Vec<(u64, &Card)> = cards
        .iter()
        .filter(|c| !c.due)
        .map(|c| (match c.state.as_str() { "known" => 1, "stale" => 2, _ => 3 }, c))
        .collect();
    let mut x = seed | 1;
    let mut next = || {
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    while out.len() < n && !rest.is_empty() {
        let total: u64 = rest.iter().map(|r| r.0).sum();
        let mut r = next() % total;
        let i = rest.iter().position(|(w, _)| if r < *w { true } else { r -= w; false }).unwrap_or(0);
        out.push(rest.remove(i).1.clone());
    }
    out
}

// ── what the message asks of KnowledgeBase ──

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// 抽 N 張
    Draw(usize),
    /// 今天要複習的: due ones (then a draw)
    Review(usize),
    /// 考我 (on a topic, or a drawn card)
    Quiz(String),
    /// 有哪些…的卡: topic, how many to name
    List(String, usize),
    /// 記住這個: the last reply's cards as remembered
    Remember,
    /// 這張卡寫錯了: correct a card (the page opens 修正知識卡; author, 2026-11-14)
    Fix,
}

fn zh_digit(c: char) -> Option<usize> {
    "零一二三四五六七八九十".chars().position(|d| d == c).or_else(|| (c == '兩').then_some(2))
}

/// The number asked for (digits, 一…十／兩 before a counter word, English one…ten).
pub fn count(text: &str) -> Option<usize> {
    let digits: String = text.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
    if let Ok(n) = digits.parse::<usize>() {
        return Some(n);
    }
    let cs: Vec<char> = text.chars().collect();
    for (i, c) in cs.iter().enumerate() {
        if let Some(d) = zh_digit(*c) {
            // 十二, 二十 are rare here; one numeral before 張／個／題／則 (or before 出來)
            if cs.get(i + 1).is_some_and(|n| "張個題則條".contains(*n)) || (*c == '十' && d == 10) {
                return Some(d);
            }
        }
    }
    let lower = text.to_lowercase();
    for (i, w) in ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"].iter().enumerate() {
        if lower.split(|c: char| !c.is_ascii_alphabetic()).any(|x| x == *w) {
            return Some(i + 1);
        }
    }
    None
}

fn has_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

fn strip(text: &str, words: &[&str]) -> String {
    let mut t = format!(" {text} ");
    for w in words {
        t = t.replace(w, " ");
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

const FILLER: &[&str] = &[
    "從知識庫", "我有", "知識庫裡面", "知識庫裡", "知識庫中", "知識庫", "我的卡片", "我的卡", "的卡片", "的卡", "卡片", "有哪些", "有什麼", "哪些", "關於", "有關", "相關", "考考我", "考我",
    "幫我", "給我", "請", "找出", "找", "列出", "一下", "吧", "嗎", "呢", "？", "?", "。", "！", "!", "，", ",", "我有", "裡面", "裡",
    " quiz me ", " test me ", " cards ", " card ", " my ", " the ", " about ", " on ", " list ", " find ", " which ", " do i have ", " have ", " show ", " me ", " in ", " kb ", " knowledge base ", " any ", " some ",
];

fn topic_of(text: &str) -> String {
    let t = strip(&text.to_lowercase(), FILLER);
    let t = t.split_whitespace().filter(|w| !matches!(*w, "卡" | "我" | "的")).collect::<Vec<_>>().join(" ");
    t.trim_matches(|c: char| c == '的' || c.is_whitespace()).to_string()
}

/// A request about the KnowledgeBase itself (Traditional Chinese or English), or `None` for a
/// normal turn.
pub fn intent(text: &str) -> Option<Intent> {
    let t = text.trim();
    let lower = t.to_lowercase();
    let n = count(t);
    if t.chars().count() <= 12 && has_any(t, &["記住這個", "記住這張", "記住它", "記住了"]) || has_any(&lower, &["remember this", "remember that card"]) {
        return Some(Intent::Remember);
    }
    // a card is wrong: 「這張卡寫錯了」「幫我修正〈Dice〉那張卡」「fix this card」
    // 卡 as the card (not 卡住／卡關／卡頓), or 這張／那張 right before it
    let cs: Vec<char> = t.chars().collect();
    let card_noun = cs.iter().enumerate().any(|(i, &c)| c == '卡' && !matches!(cs.get(i + 1), Some('住' | '關' | '頓' | '到' | '在')));
    let about_card = card_noun || lower.contains("card");
    if about_card && (has_any(t, &["寫錯", "有錯", "錯了", "不對", "修正", "更正", "改正", "改一下", "幫我改", "改掉"]) || has_any(&lower, &["fix this card", "fix the card", "card is wrong", "correct this card", "correct the card", "wrong card"])) {
        return Some(Intent::Fix);
    }
    if has_any(t, &["考我", "考考我", "抽考"]) || has_any(&lower, &["quiz me", "test me"]) {
        return Some(Intent::Quiz(topic_of(&strip(t, &["抽考"]))));
    }
    // 「我昨天複習了卷積」 is a normal turn; 「幫我複習 X」 is a quiz on X
    if has_any(t, &["幫我複習", "考考我複習", "陪我複習"]) && !has_any(t, &["今天", "什麼", "哪些", "哪幾"]) {
        let topic = topic_of(&strip(t, &["幫我複習", "陪我複習", "考考我複習"]));
        if !topic.is_empty() {
            return Some(Intent::Quiz(topic));
        }
    }
    if has_any(t, &["今天要複習", "今天該複習", "要複習什麼", "該複習什麼", "複習什麼", "要複習哪", "複習哪", "待複習", "到期", "有哪些要複習", "幫我複習", "陪我複習"]) || has_any(&lower, &["to review", "review today", "due today", "cards due", "what's due", "due cards"]) {
        return Some(Intent::Review(n.unwrap_or(5).clamp(1, 10)));
    }
    let listing = has_any(t, &["有哪些卡", "哪些卡", "有什麼卡", "有幾張"]) || (t.contains('找') && t.contains('卡')) || (t.contains("關於") && t.contains("的卡"))
        || has_any(&lower, &["cards about", "cards on", "list cards", "list my cards", "which cards", "find cards", "find card"]);
    if listing {
        return Some(Intent::List(topic_of(t), n.unwrap_or(8).clamp(1, 10)));
    }
    let drawing = (has_any(t, &["抽", "隨機", "挑"]) && (n.is_some() || has_any(t, &["卡", "張", "知識庫"])))
        || (has_any(&lower, &["draw", "pick", "random"]) && (lower.contains("card") || n.is_some()));
    if drawing {
        return Some(Intent::Draw(n.unwrap_or(2).clamp(1, 10)));
    }
    None
}

// ── the turn ──

/// What the page sends with a message (`kb` in the chat body).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Ask {
    /// The card the last reply quizzed the author on: this message is the answer.
    pub quiz: Option<String>,
    /// The cards the last reply used (「記住這個」 means these).
    pub cited: Vec<String>,
}

/// The turn's plan: the cards, what was asked, and the note for the model.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    pub cards: Vec<Card>,
    /// "" (context) | draw | review | list | quiz | answer | remember
    pub intent: &'static str,
    /// quiz: the card asked about (the page sends it back with the answer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiz: Option<String>,
    /// answer: the card the author grades himself on (記得／忘了).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    /// remember: the cards recorded as remembered.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recorded: Vec<String>,
    /// One line for the page (e.g. KnowledgeBase is not running).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub notice: String,
    #[serde(skip)]
    pub system: String,
}

const OWN_CARDS: &str = "下面是使用者自己在 KnowledgeBase 寫的知識卡（他自己的筆記，不是外部資料）。\
用到某張卡時，要說出是他的卡，例如「你的卡〈標題〉說…」。熟悉度：已懂＝不必從頭解釋；學習中＝可以順便考考他；\
久未複習＝提醒他該複習了；尚無紀錄＝當成新東西講。不要編造卡片裡沒有的內容。";

fn clip(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s }
}

/// The cards as labelled lines, within [`BUDGET_CHARS`] (the rest are left out).
pub fn card_lines(cards: &[Card]) -> (String, usize) {
    let mut out = String::new();
    let mut used = 0;
    for c in cards {
        let mut line = format!("- 〈{}〉（{}，{}{}）：{}", c.title, c.kind, c.state_label(), if c.due { "，該複習了" } else { "" }, clip(&c.summary, SUMMARY_CHARS));
        if !c.aliases.is_empty() {
            line.push_str(&format!("〔也叫：{}〕", c.aliases.join("、")));
        }
        if !c.source.is_empty() {
            line.push_str(&format!("〔來源：{}〕", clip(&c.source, 60)));
        }
        if used > 0 && out.chars().count() + line.chars().count() > BUDGET_CHARS {
            break;
        }
        out.push_str(&line);
        out.push('\n');
        used += 1;
    }
    (out, used)
}

/// The plan for a turn over already loaded `cards` (no I/O: the KnowledgeBase write for
/// 「記住這個」 is done by the caller).
pub fn plan(cards: &[Card], turns: &[Turn], ask: &Ask, seed: u64) -> Plan {
    let Some(last) = turns.last() else { return Plan::default() };
    let find = |id: &str| cards.iter().find(|c| c.id == id).cloned();
    let mut p = Plan::default();
    // right after a reply that used his cards, 「修正一下」「加點中文解釋」 need not say 卡
    let wanted = intent(&last.text).or_else(|| (!ask.cited.is_empty() && fix_words(&last.text)).then_some(Intent::Fix));
    if wanted.is_none() && let Some(card) = ask.quiz.as_deref().and_then(find) {
        p.intent = "answer";
        p.grade = Some(card.id.clone());
        let (lines, _) = card_lines(std::slice::from_ref(&card));
        p.system = format!("{OWN_CARDS}\n你上一句用這張卡考了使用者，他最新的訊息是他的回答：\n{lines}\
把他的回答和卡片比較：說他對在哪裡、漏了或錯了什麼（照卡片的內容）。不要問他記不記得，畫面上會讓他自己選。");
        p.cards = vec![card];
        return p;
    }
    let Some(want) = wanted else {
        p.cards = retrieve(cards, turns, MAX_CARDS);
        let (lines, used) = card_lines(&p.cards);
        p.cards.truncate(used);
        if used > 0 {
            p.system = format!("{OWN_CARDS}\n{lines}只在跟話題有關時才用。");
        }
        return p;
    };
    let empty = cards.is_empty();
    match want {
        Intent::Draw(n) => {
            p.intent = "draw";
            p.cards = draw(cards, n, seed);
            p.system = if empty { "使用者要你從他的知識庫抽卡，但知識庫裡沒有卡。直接這樣告訴他。".into() } else {
                let (lines, used) = card_lines(&p.cards);
                p.cards.truncate(used);
                format!("{OWN_CARDS}\n使用者要你從他的知識庫抽 {n} 張卡。程式已經抽好下面這 {used} 張（不要換、不要加、不要說別的卡）：\n{lines}\
依序報出卡名，每張用一句話帶到重點或問他一句；該複習的說一聲。")
            };
        }
        Intent::Review(n) => {
            p.intent = "review";
            let due: Vec<Card> = draw(cards, n, seed).into_iter().filter(|c| c.due).collect();
            if due.is_empty() {
                p.cards = draw(cards, n.min(3), seed);
                let (lines, used) = card_lines(&p.cards);
                p.cards.truncate(used);
                p.system = if empty { "使用者問今天要複習什麼，但他的知識庫裡沒有卡。直接這樣告訴他。".into() } else {
                    format!("{OWN_CARDS}\n使用者問今天要複習什麼：排程上今天沒有到期的卡。先這樣告訴他，再提議看看這幾張（程式挑的，不要換）：\n{lines}")
                };
            } else {
                let (lines, used) = card_lines(&due);
                p.cards = due;
                p.cards.truncate(used);
                p.system = format!("{OWN_CARDS}\n使用者問今天要複習什麼。排程上到期的是這 {used} 張（程式挑的，不要換、不要加）：\n{lines}報出卡名，問他要不要從第一張開始考。");
            }
        }
        Intent::List(topic, n) => {
            p.intent = "list";
            let (total, found) = list(cards, &topic, n);
            let (lines, used) = card_lines(&found);
            p.cards = found;
            p.cards.truncate(used);
            let what = if topic.is_empty() { "他的知識庫".to_string() } else { format!("跟「{topic}」有關的卡") };
            p.system = if total == 0 {
                format!("使用者問{what}：知識庫裡沒有符合的卡。直接這樣告訴他，不要編卡名。")
            } else {
                format!("{OWN_CARDS}\n使用者問{what}：程式找到 {total} 張，下面是前 {used} 張（只說這些卡名，不要加）：\n{lines}")
            };
        }
        Intent::Quiz(topic) => {
            p.intent = "quiz";
            let on_topic = (!topic.is_empty()).then(|| {
                let probe = [Turn { role: "user".into(), text: topic.clone() }];
                retrieve(cards, &probe, 1).into_iter().next().or_else(|| list(cards, &topic, 1).1.into_iter().next())
            });
            let card = on_topic.flatten().or_else(|| draw(cards, 1, seed).into_iter().next());
            match card {
                Some(c) => {
                    let (lines, _) = card_lines(std::slice::from_ref(&c));
                    p.quiz = Some(c.id.clone());
                    p.system = format!("{OWN_CARDS}\n使用者要你考他。程式選了這張卡：\n{lines}根據卡片內容問他一個問題（只問這張卡），不要說出答案，也不要給提示。");
                    p.cards = vec![c];
                }
                None => {
                    p.system = if empty { "使用者要你考他，但他的知識庫裡沒有卡。直接這樣告訴他。".into() } else { format!("使用者要你考他「{topic}」，但知識庫裡沒有這方面的卡。直接這樣告訴他。") };
                }
            }
        }
        Intent::Remember => {
            p.intent = "remember";
            p.cards = ask.cited.iter().filter_map(|id| find(id)).collect();
        }
        Intent::Fix => {
            // the card he names, else the ones the last reply used; the page opens 修正知識卡 on the first
            p.intent = "fix";
            p.cards = fix_targets(cards, turns, ask);
            p.system = match p.cards.first() {
                Some(c) => format!("使用者要你修改他的知識卡〈{}〉，程式已經開始處理了。只回一句：好，你會上網查證後改好給他看（他按「套用」才會改）。不要問他任何問題，不要自己講改法，也不要說你做不到。", c.title),
                None => "使用者要修正一張知識卡，但沒說是哪一張。用一句話請他說出卡名（或先讓你引用那張卡）。不要說你做不到。".into(),
            };
        }
    }
    p
}

/// Letters and digits only, lower case: 「[CLS] Token」 = 「cls token」 = 「CLS-token」.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Words that ask for a card to be changed (said right after a reply that used it).
pub fn fix_words(text: &str) -> bool {
    has_any(text, &["修正", "更正", "改正", "改一下", "幫我改", "改成", "改寫", "寫錯", "有錯", "補充", "加一點", "加點", "加上"]) || has_any(&text.to_lowercase(), &["fix it", "fix that", "correct it", "rewrite it"])
}

/// The cards 「修正」 may mean, most likely first: the ones his message names (ignoring brackets,
/// spaces and case; the longest name first), then the last reply's, then the turns' best matches.
pub fn fix_targets(cards: &[Card], turns: &[Turn], ask: &Ask) -> Vec<Card> {
    let text = turns.last().map(|t| squash(&t.text)).unwrap_or_default();
    let mut named: Vec<(usize, &Card)> = cards
        .iter()
        .filter_map(|c| std::iter::once(&c.title).chain(c.aliases.iter()).map(|n| squash(n)).filter(|n| n.chars().count() >= 2 && text.contains(n.as_str())).map(|n| n.chars().count()).max().map(|l| (l, c)))
        .collect();
    named.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out: Vec<Card> = vec![];
    let cited = ask.cited.iter().filter_map(|id| cards.iter().find(|c| &c.id == id));
    for c in named.into_iter().map(|(_, c)| c).chain(cited) {
        if !out.iter().any(|x| x.id == c.id) {
            out.push(c.clone());
        }
    }
    if out.is_empty() {
        out = retrieve(cards, turns, 3);
    }
    out.truncate(MAX_CARDS);
    out
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

/// Retrieval for one chat turn, run by `chat::stream` before `start`.
pub struct Pre {
    shared: Shared,
    turns: Vec<Turn>,
    ask: Ask,
}

impl Pre {
    pub fn new(shared: &Shared, turns: &[Turn], ask: Option<Ask>) -> Pre {
        Pre { shared: shared.clone(), turns: turns.to_vec(), ask: ask.unwrap_or_default() }
    }

    /// `search` (when there is a KnowledgeBase), the plan as `cards`, and `req` with the note.
    pub async fn run(self, mut req: LlmRequest, send: impl Fn(&str, Value)) -> LlmRequest {
        let (spaces, member) = self.shared.kb.locate(&self.shared.home);
        if spaces.is_empty() {
            if self.turns.last().is_some_and(|t| intent(&t.text).is_some()) {
                req.system.push_str("\n\n使用者提到他的知識庫，但這台電腦上找不到 KnowledgeBase。用一句話這樣告訴他。");
                send("cards", json!({ "cards": [], "intent": "", "available": false, "notice": "找不到 KnowledgeBase" }));
            }
            return req;
        }
        send("search", json!({}));
        let (turns, ask) = (self.turns, self.ask);
        let mut p = tokio::task::spawn_blocking(move || {
            let cards = load(&spaces, &member, &SystemClock);
            plan(&cards, &turns, &ask, seed())
        })
        .await
        .unwrap_or_default();
        if p.intent == "remember" {
            remember(&self.shared, &mut p).await;
        }
        if !p.system.is_empty() {
            req.system.push_str("\n\n");
            req.system.push_str(&p.system);
        }
        let mut v = serde_json::to_value(&p).unwrap_or_else(|_| json!({}));
        v["available"] = json!(true);
        send("cards", v);
        req
    }
}

/// 「記住這個」: the last reply's cards recorded as remembered through KnowledgeBase's API.
async fn remember(shared: &Shared, p: &mut Plan) {
    if p.cards.is_empty() {
        p.system = "使用者說「記住這個」，但上一句沒有用到他的知識卡。用一句話告訴他：要記的卡要先在對話裡出現（新卡之後用「做成知識卡」）。".into();
        return;
    }
    let names = p.cards.iter().map(|c| format!("〈{}〉", c.title)).collect::<Vec<_>>().join("、");
    let Some(url) = shared.kb.server().await else {
        p.notice = "KnowledgeBase 沒開，沒有記下".into();
        p.system = format!("使用者要你把{names}記成「記得」，但 KnowledgeBase 沒有開，所以沒有記下。用一句話告訴他，請他打開 KnowledgeBase 再說一次。");
        return;
    };
    for c in &p.cards {
        if recall(&url, &c.id, "remembered").await.is_ok() {
            p.recorded.push(c.id.clone());
        }
    }
    p.system = if p.recorded.len() == p.cards.len() {
        format!("已經在使用者的 KnowledgeBase 把{names}記成「記得」（複習紀錄）。用一句話確認。")
    } else {
        p.notice = "有些卡沒有記下".into();
        format!("想在 KnowledgeBase 把{names}記成「記得」，但有的沒有記下。用一句話告訴他。")
    };
}

// ── routes ──

#[derive(Deserialize)]
pub struct RecallBody {
    id: String,
    /// remembered | forgot | unfamiliar
    outcome: String,
}

/// `POST /api/kb/recall {id, outcome}` → `{ ok, progress? }` or `{ error }`: through
/// KnowledgeBase's own API, only while it runs.
pub async fn recall_post(State(s): State<Shared>, Json(b): Json<RecallBody>) -> Response {
    if !matches!(b.outcome.as_str(), "remembered" | "forgot" | "unfamiliar") || !kb_core::id::valid_node_id(&b.id) {
        return (axum::http::StatusCode::BAD_REQUEST, Json(json!({ "error": "不認得的卡片或結果" }))).into_response();
    }
    let Some(url) = s.kb.server().await else { return Json(json!({ "error": "KnowledgeBase 沒開，沒有記下" })).into_response() };
    match recall(&url, &b.id, &b.outcome).await {
        Ok(v) => Json(json!({ "ok": true, "progress": v.get("progress") })).into_response(),
        Err(e) => Json(json!({ "error": format!("沒有記下：{e}") })).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(role: &str, text: &str) -> Turn {
        Turn { role: role.into(), text: text.into() }
    }

    fn card(id: &str, title: &str, aliases: &[&str], summary: &str, state: &str, due: bool) -> Card {
        Card { id: id.into(), space: "me".into(), title: title.into(), aliases: aliases.iter().map(|s| s.to_string()).collect(), kind: "concept".into(), summary: summary.into(), state: state.into(), due, next: due.then_some(100), source: String::new() }
    }

    fn deck() -> Vec<Card> {
        vec![
            card("a", "梯度", &["gradient"], "函數增加最快的方向。", "learning", false),
            card("b", "Gated flow", &["gating"], "A flow whose edges are switched by a learned gate.", "known", false),
            card("c", "傅立葉轉換", &["Fourier transform"], "把訊號拆成不同頻率的正弦波。", "", false),
            card("d", "卷積", &[], "兩個函數的一種乘積，訊號處理常用；和傅立葉轉換有關。", "stale", false),
            card("e", "熵", &["entropy"], "不確定性的量度。", "learning", true),
        ]
    }

    #[test]
    fn ranking_title_in_message_then_recent_then_key_words() {
        let cards = deck();
        let turns = [t("user", "我們剛剛聊 gating"), t("assistant", "嗯"), t("user", "那傅立葉轉換跟頻率的關係呢？")];
        let got: Vec<_> = retrieve(&cards, &turns, 5).into_iter().map(|c| c.title).collect();
        assert_eq!(got[0], "傅立葉轉換", "named in the message: {got:?}");
        assert_eq!(got[1], "Gated flow", "named a few turns back: {got:?}");
        assert!(got.contains(&"卷積".to_string()), "key words 傅立/立葉/轉換 in its summary: {got:?}");
        assert!(!got.contains(&"熵".to_string()));
        assert!(retrieve(&cards, &[t("user", "你今天好嗎？可以聊聊什麼嗎")], 5).is_empty(), "function words cite nothing");
        assert_eq!(retrieve(&cards, &[t("user", "what is the gradient here")], 5)[0].title, "梯度", "an alias as a whole word");
        assert!(retrieve(&cards, &[t("user", "gradients everywhere, regradient")], 5).len() == 1);
        assert_eq!(retrieve(&cards, &turns, 1).len(), 1);
    }

    #[test]
    fn the_prompt_lines_stay_within_the_budget() {
        let mut cards = deck();
        for c in &mut cards {
            c.summary = "長".repeat(500);
        }
        let many: Vec<Card> = (0..20).map(|i| { let mut c = cards[0].clone(); c.id = format!("x{i}"); c }).collect();
        let (lines, used) = card_lines(&many);
        assert!((1..20).contains(&used));
        assert!(lines.chars().count() <= BUDGET_CHARS + SUMMARY_CHARS);
        assert!(lines.contains("〈梯度〉（concept，學習中）") && lines.contains("〔也叫：gradient〕"), "{lines}");
    }

    #[test]
    fn intents_in_chinese_and_english_with_counts() {
        assert_eq!(intent("從知識庫抽兩個出來"), Some(Intent::Draw(2)));
        assert_eq!(intent("隨機抽 3 張卡給我"), Some(Intent::Draw(3)));
        assert_eq!(intent("抽卡"), Some(Intent::Draw(2)));
        assert_eq!(intent("draw five cards"), Some(Intent::Draw(5)));
        assert_eq!(intent("pick 4 random cards"), Some(Intent::Draw(4)));
        assert_eq!(intent("抽 50 張"), Some(Intent::Draw(10)), "at most ten");
        assert_eq!(intent("今天要複習什麼？"), Some(Intent::Review(5)));
        assert_eq!(intent("what's due today"), Some(Intent::Review(5)));
        assert_eq!(intent("該複習哪幾張？"), Some(Intent::Review(5)));
        assert_eq!(intent("幫我複習傅立葉轉換"), Some(Intent::Quiz("傅立葉轉換".into())));
        assert_eq!(intent("幫我複習"), Some(Intent::Review(5)));
        assert_eq!(intent("我昨天複習了卷積，還是不懂"), None);
        assert_eq!(intent("考我"), Some(Intent::Quiz(String::new())));
        assert_eq!(intent("考考我傅立葉轉換"), Some(Intent::Quiz("傅立葉轉換".into())));
        assert_eq!(intent("quiz me on entropy"), Some(Intent::Quiz("entropy".into())));
        assert_eq!(intent("我有哪些卡？"), Some(Intent::List(String::new(), 8)));
        assert_eq!(intent("幫我找梯度的卡"), Some(Intent::List("梯度".into(), 8)));
        assert_eq!(intent("知識庫裡有哪些關於訊號的卡"), Some(Intent::List("訊號".into(), 8)));
        assert_eq!(intent("which cards about gating do I have"), Some(Intent::List("gating".into(), 8)));
        assert_eq!(intent("記住這個"), Some(Intent::Remember));
        assert_eq!(intent("這張卡寫錯了"), Some(Intent::Fix));
        assert_eq!(intent("幫我修正 Dice 那張卡，分母不對"), Some(Intent::Fix));
        assert_eq!(intent("fix this card please"), Some(Intent::Fix));
        assert_eq!(intent("我修正了論文的第二節"), None, "not about a card");
        assert_eq!(intent("程式卡住了，不對"), None);
        assert_eq!(intent("這張圖不對"), None);
        assert_eq!(intent("改一下那張投影片"), None);
        assert_eq!(intent("卡關了，錯了好幾次"), None);
        assert_eq!(intent("這張卡不對"), Some(Intent::Fix));
        assert!(fix_words("保留公式加一點自然語言的中文解釋") && fix_words("幫我修正 CLS") && !fix_words("CLS 是什麼？"));
        assert_eq!(intent("remember this"), Some(Intent::Remember));
        for none in ["今天天氣如何", "我想抽空讀書", "幫我想想傅立葉轉換", "I picked a topic", "考試好難"] {
            assert_eq!(intent(none), None, "{none}");
        }
        assert_eq!(count("給我三張"), Some(3));
        assert_eq!(count("抽兩個"), Some(2));
        assert_eq!(count("12 cards"), Some(12));
        assert_eq!(count("隨便"), None);
    }

    #[test]
    fn draw_takes_due_cards_first_then_weighted() {
        let mut cards = deck();
        cards[3].due = true;
        cards[3].next = Some(10);
        cards[4].next = Some(50);
        for seed in 1..50 {
            let got = draw(&cards, 3, seed);
            assert_eq!(got.len(), 3);
            assert_eq!((got[0].title.as_str(), got[1].title.as_str()), ("卷積", "熵"), "due, oldest check first");
            assert!(!["卷積", "熵"].contains(&got[2].title.as_str()), "no repeats");
        }
        // the known card comes up less often than the learning ones
        let only: Vec<Card> = deck().into_iter().filter(|c| !c.due).collect();
        let mut known = 0;
        let mut learning = 0;
        for seed in 1..2000 {
            match draw(&only, 1, seed * 7919)[0].state.as_str() {
                "known" => known += 1,
                "learning" => learning += 1,
                _ => {}
            }
        }
        assert!(learning > known * 2, "learning {learning}, known {known}");
        assert_eq!(draw(&only, 10, 3).len(), only.len());
        assert!(draw(&[], 3, 3).is_empty());
    }

    #[test]
    fn plans_are_chosen_in_code() {
        let cards = deck();
        let p = plan(&cards, &[t("user", "從知識庫抽兩個出來")], &Ask::default(), 9);
        assert_eq!((p.intent, p.cards.len()), ("draw", 2));
        assert_eq!(p.cards[0].title, "熵", "the due card first");
        assert!(p.system.contains("不要換") && p.system.contains("〈熵〉"), "{}", p.system);
        let p = plan(&cards, &[t("user", "有哪些關於訊號的卡？")], &Ask::default(), 9);
        assert_eq!(p.intent, "list");
        assert_eq!(p.cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["傅立葉轉換", "卷積"]);
        let p = plan(&cards, &[t("user", "找量子的卡")], &Ask::default(), 9);
        assert!(p.cards.is_empty() && p.system.contains("沒有符合"));
        let p = plan(&cards, &[t("user", "今天要複習什麼")], &Ask::default(), 9);
        assert_eq!(p.cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["熵"]);
        let p = plan(&[], &[t("user", "抽三張")], &Ask::default(), 9);
        assert!(p.cards.is_empty() && p.system.contains("沒有卡"));
        let p = plan(&cards, &[t("user", "傅立葉轉換怎麼用？")], &Ask::default(), 9);
        assert_eq!((p.intent, p.cards[0].title.as_str()), ("", "傅立葉轉換"));
        assert!(p.system.contains("他自己的筆記") && p.system.contains("你的卡"));
    }

    #[test]
    fn quiz_asks_then_the_answer_is_compared_with_the_card() {
        let cards = deck();
        let p = plan(&cards, &[t("user", "考考我 gating")], &Ask::default(), 1);
        assert_eq!((p.intent, p.quiz.as_deref()), ("quiz", Some("b")));
        assert!(p.system.contains("不要說出答案") && p.system.contains("switched by a learned gate"));
        let p = plan(&cards, &[t("user", "考我")], &Ask::default(), 1);
        assert_eq!(p.quiz.as_deref(), Some("e"), "no topic: the due card");
        let turns = [t("user", "考我"), t("assistant", "熵是什麼？"), t("user", "是不確定性的量度")];
        let p = plan(&cards, &turns, &Ask { quiz: Some("e".into()), cited: vec![] }, 1);
        assert_eq!((p.intent, p.grade.as_deref()), ("answer", Some("e")));
        assert!(p.system.contains("比較") && p.system.contains("不確定性的量度"));
        // a new request instead of an answer ends the quiz
        let p = plan(&cards, &[t("user", "抽一張")], &Ask { quiz: Some("e".into()), cited: vec![] }, 1);
        assert_eq!(p.intent, "draw");
    }

    #[test]
    fn reads_a_space_without_writing() {
        let d = tempfile::tempdir().unwrap();
        let root = test_space(d.path());
        let before = snapshot(&root);
        let cards = load(std::slice::from_ref(&root), "ellrin", &kb_core::FixedClock::at("2026-11-10T09:00:00+00:00"));
        assert_eq!(cards.len(), 3, "broken and hidden files skipped");
        let g = cards.iter().find(|c| c.title == "梯度").unwrap();
        assert_eq!((g.state.as_str(), g.due), ("learning", true));
        let f = cards.iter().find(|c| c.title == "傅立葉轉換").unwrap();
        assert_eq!((f.state.as_str(), f.due, f.source.as_str()), ("", false, "Signals, a primer"));
        assert_eq!(cards.iter().find(|c| c.title == "Entropy").unwrap().summary, "How uncertain a variable is.", "the body's first paragraph");
        // someone else's progress is not ours
        assert!(load(std::slice::from_ref(&root), "daidai", &kb_core::FixedClock::at("2026-11-10T09:00:00+00:00")).iter().all(|c| c.state.is_empty()));
        assert_eq!(before, snapshot(&root), "nothing was written into the space");
    }

    pub(crate) fn snapshot(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = vec![];
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.path().is_dir() { stack.push(e.path()) } else { out.push((e.path(), std::fs::read(e.path()).unwrap())) }
            }
        }
        out.sort();
        out
    }

    /// A synthetic space: three cards, one learning and due for `ellrin`.
    pub(crate) fn test_space(dir: &std::path::Path) -> PathBuf {
        let root = dir.join("kb");
        std::fs::create_dir_all(root.join("nodes")).unwrap();
        std::fs::create_dir_all(root.join("progress/ellrin")).unwrap();
        std::fs::write(root.join("space.toml"), "id = \"me\"\nname = \"Me\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
        let node = |id: &str, v: Value| {
            let mut n = json!({ "id": id, "kind": "concept", "created_at": "2026-11-01T09:00:00Z", "updated_at": "2026-11-01T09:00:00Z" });
            n.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
            std::fs::write(root.join("nodes").join(format!("{id}.json")), serde_json::to_vec_pretty(&n).unwrap()).unwrap();
        };
        node("nAAAAAAAAAAAAAAAAAAAAAAAA1", json!({ "title": "梯度", "aliases": ["gradient"], "summary": "函數增加最快的方向。" }));
        node("nAAAAAAAAAAAAAAAAAAAAAAAA2", json!({ "title": "傅立葉轉換", "summary": "把訊號拆成不同頻率的正弦波。", "sources": [{ "type": "book", "ref": "isbn:0", "title": "Signals, a primer" }] }));
        node("nAAAAAAAAAAAAAAAAAAAAAAAA3", json!({ "title": "Entropy", "body": "# Entropy\n\nHow uncertain a variable is.\n\nMore." }));
        std::fs::write(root.join("nodes/.hidden.json"), "{}").unwrap();
        std::fs::write(root.join("nodes/broken.json"), "not json").unwrap();
        std::fs::write(
            root.join("progress/ellrin/nAAAAAAAAAAAAAAAAAAAAAAAA1.json"),
            r#"{ "state": "learning", "streak": 0, "next_check_at": "2026-11-02T09:00:00Z", "log": [] }"#,
        )
        .unwrap();
        root
    }

    #[test]
    fn the_card_to_fix_is_found_without_its_brackets_then_from_the_last_reply() {
        let mut cls = card("nC", "[CLS] Token", &[], "聚合特徵", "learning", false);
        cls.aliases = vec![];
        let cards = vec![cls, card("nD", "Dice", &[], "重疊", "known", false)];
        let ask = Ask { quiz: None, cited: vec!["nD".into()] };
        let got = fix_targets(&cards, &[t("user", "幫我修正一下 CLS token的那張知識卡")], &ask);
        assert_eq!(got.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["nC", "nD"]);
        // a follow-up without 卡 right after a reply that used the card
        let p = plan(&cards, &[t("user", "保留公式加一點中文解釋")], &Ask { quiz: None, cited: vec!["nC".into()] }, 1);
        assert_eq!(p.intent, "fix");
        assert_eq!(p.cards[0].id, "nC");
        // without cited cards it stays a normal turn
        assert_eq!(plan(&cards, &[t("user", "保留公式加一點中文解釋")], &Ask::default(), 1).intent, "");
    }

}
