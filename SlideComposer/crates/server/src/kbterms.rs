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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as AxPath, State};
use kb_core::Node;
use kb_core::progress::{Progress, State as KbState};
use lore_canvas::style::{LEARN_EM, texts_mut};
use lore_canvas::{Object, Op, Para, Run, Slide, TextBody};
use lore_words::{Filters, is_acronym, is_hyphen, phrase_key, word_key};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{R, Shared, bad};

const APP: &str = "KnowledgeBase";

// ── KnowledgeBase (read-only) ────────────────────────────────────────────────

/// How well the author knows a term.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Familiar {
    /// 熟
    Known,
    /// 學習中
    Learning,
    /// 不熟 (no card, or one to learn)
    Unknown,
}

impl Familiar {
    pub fn label(self) -> &'static str {
        match self {
            Familiar::Known => "熟",
            Familiar::Learning => "學習中",
            Familiar::Unknown => "不熟",
        }
    }
}

/// One card name (title or alias) and the member's state for the card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KbName {
    pub name: String,
    pub card: String,
    pub title: String,
    pub state: Familiar,
}

/// A card's state for this member (see the module doc).
pub fn familiar(p: Option<&Progress>) -> Familiar {
    let Some(p) = p else { return Familiar::Learning };
    // the last answer (encounters and shares are no answer)
    let answer = p.log.iter().rev().find(|e| matches!(e.event.as_str(), "created" | "remembered" | "forgot" | "unfamiliar" | "known"));
    if answer.is_some_and(|e| e.event == "unfamiliar") {
        return Familiar::Unknown;
    }
    match p.state {
        KbState::Known => Familiar::Known,
        KbState::Learning | KbState::Stale => Familiar::Learning,
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn kb_app_dir() -> Option<PathBuf> {
    if let Some(d) = env("KB_HOME") {
        return Some(PathBuf::from(d));
    }
    let home = env("HOME").or_else(|| env("USERPROFILE")).map(PathBuf::from);
    let cfg = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA").map(PathBuf::from)
    } else {
        env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))
    };
    cfg.map(|d| d.join(APP))
}

fn kb_config() -> Option<toml_lite::Table> {
    let f = kb_app_dir()?.join("config.toml");
    toml_lite::parse(&std::fs::read_to_string(f).ok()?)
}

/// KnowledgeBase's spaces (the ones its server would mount, without seeding any).
pub fn space_dirs(home: &lore_home::Home) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = env("KB_SPACES").map(|s| s.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect()).unwrap_or_default();
    if v.is_empty()
        && let Some(t) = kb_config()
    {
        v = t.list("spaces").iter().map(|s| lore_store::expand_home(s)).collect();
    }
    if v.is_empty()
        && let Some(d) = home.app_dir(APP)
    {
        v.push(d);
    }
    v.retain(|d| d.join("space.toml").is_file());
    v
}

/// Whose learning state counts (what KnowledgeBase itself uses).
pub fn member(home: &lore_home::Home) -> String {
    env("KB_MEMBER").or_else(|| kb_config().and_then(|t| t.string("member"))).or_else(|| home.prefs().profile.id).unwrap_or_else(|| "me".into())
}

/// Every card title and alias with the member's state (read-only; unreadable files are skipped).
pub fn load_names(spaces: &[PathBuf], member: &str) -> Vec<KbName> {
    let mut out = vec![];
    for root in spaces {
        let Ok(space) = lore_store::Space::open(root) else { continue };
        let Ok(rd) = std::fs::read_dir(space.nodes_dir()) else { continue };
        let progress = space.progress_dir(member).ok();
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
        files.sort();
        for f in files {
            let Some(n) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<Node>(&b).ok()).filter(|n| !n.id.is_empty() && !n.title.trim().is_empty()) else { continue };
            let p = progress.as_ref().and_then(|d| std::fs::read(d.join(format!("{}.json", n.id))).ok()).and_then(|b| serde_json::from_slice::<Progress>(&b).ok());
            let state = familiar(p.as_ref());
            for name in std::iter::once(n.title.as_str()).chain(n.aliases.iter().map(String::as_str)) {
                if !name.trim().is_empty() {
                    out.push(KbName { name: name.trim().to_string(), card: n.id.clone(), title: n.title.trim().to_string(), state });
                }
            }
        }
    }
    out
}

/// A few lines of TOML are all we read from KnowledgeBase's config (`spaces`, `member`).
mod toml_lite {
    pub struct Table(Vec<(String, String)>);
    pub fn parse(text: &str) -> Option<Table> {
        let mut v = vec![];
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                break; // top-level keys only
            }
            if let Some((k, val)) = line.split_once('=') {
                v.push((k.trim().to_string(), val.trim().to_string()));
            }
        }
        Some(Table(v))
    }
    fn strings(s: &str) -> Vec<String> {
        s.split('"').skip(1).step_by(2).map(str::to_string).collect()
    }
    impl Table {
        pub fn string(&self, k: &str) -> Option<String> {
            self.0.iter().find(|(a, _)| a == k).and_then(|(_, v)| strings(v).into_iter().next())
        }
        pub fn list(&self, k: &str) -> Vec<String> {
            self.0.iter().find(|(a, _)| a == k).map(|(_, v)| strings(v)).unwrap_or_default()
        }
    }
}

// ── tokens ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Tok {
    b0: usize,
    b1: usize,
    text: String,
    key: String,
    /// only spaces or one hyphen up to the next word: phrases may continue
    open_next: bool,
}

fn word_char(c: char) -> bool {
    (c.is_alphanumeric() && !matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)) || c == '\'' || c == '’'
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = vec![];
    let mut start: Option<usize> = None;
    let push = |a: usize, b: usize, out: &mut Vec<Tok>| {
        let w = text[a..b].trim_matches(|c| c == '\'' || c == '’');
        if w.is_empty() {
            return;
        }
        let a = a + text[a..b].find(w).unwrap_or(0);
        out.push(Tok { b0: a, b1: a + w.len(), text: w.to_string(), key: word_key(w), open_next: false });
    };
    for (i, c) in text.char_indices() {
        if word_char(c) {
            start.get_or_insert(i);
        } else if let Some(a) = start.take() {
            push(a, i, &mut out);
        }
    }
    if let Some(a) = start {
        push(a, text.len(), &mut out);
    }
    for i in 0..out.len().saturating_sub(1) {
        let gap = &text[out[i].b1..out[i + 1].b0];
        let hy = gap.chars().filter(|c| is_hyphen(*c)).count();
        out[i].open_next = gap.chars().all(|c| c.is_whitespace() || is_hyphen(c)) && hy <= 1 && !gap.contains('\n');
    }
    out
}

fn letters(w: &str) -> usize {
    w.chars().filter(|c| c.is_alphabetic()).count()
}

fn is_roman(w: &str) -> bool {
    w.len() <= 5 && w.chars().all(|c| "IVXLC".contains(c))
}

/// An acronym worth listing: not a roman numeral, not an all-caps common word.
fn acronym(w: &str, f: &Filters) -> bool {
    is_acronym(w) && !is_roman(w) && letters(w) >= 2 && !f.is_function(w) && !(letters(w) >= 4 && f.is_general(w))
}

/// A word that can be part of a phrase.
fn content(t: &Tok, f: &Filters) -> bool {
    letters(&t.text) >= 2 && !f.is_function(&t.text) && !t.text.chars().all(|c| c.is_ascii_digit())
}

// ── terms ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Where {
    /// The section / slide the text came from.
    pub at: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Term {
    /// Normalised key (a phrase key; `card:<id>` for a KB card).
    pub key: String,
    /// The most frequent spelling.
    pub text: String,
    /// kb | expansion | acronym | phrase
    pub kind: String,
    /// The acronym of an expansion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub state: Familiar,
    /// 熟 / 學習中 / 不熟
    pub label: String,
    pub count: usize,
    pub at: Vec<Where>,
    /// Every spelling seen (what the marks look for).
    pub forms: Vec<String>,
    /// 我懂 (the author cleared it): never marked.
    #[serde(default)]
    pub understood: bool,
}

impl Term {
    /// Marked on slides: 不熟 and not cleared.
    pub fn unfamiliar(&self) -> bool {
        self.state == Familiar::Unknown && !self.understood
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TermsResult {
    pub terms: Vec<Term>,
    /// 名詞 n
    pub total: usize,
    /// 不熟 m
    pub unfamiliar: usize,
}

impl TermsResult {
    fn recount(&mut self) {
        self.total = self.terms.len();
        self.unfamiliar = self.terms.iter().filter(|t| t.unfamiliar()).count();
    }

    /// 我懂: the term (by key, text or a form) is no longer marked. False if unknown.
    pub fn understood(&mut self, term: &str) -> bool {
        let k = phrase_key(term);
        let mut hit = false;
        for t in &mut self.terms {
            if t.key == term || t.text == term || t.forms.iter().any(|f| phrase_key(f) == k) || t.short.as_deref().is_some_and(|s| word_key(s) == k) {
                t.understood = true;
                hit = true;
            }
        }
        self.recount();
        hit
    }

    /// What the marks look for: every form (and acronym) of the unfamiliar terms.
    pub fn matcher(&self) -> Matcher {
        Matcher::new(self.terms.iter().filter(|t| t.unfamiliar()).flat_map(|t| t.forms.iter().cloned().chain(t.short.clone())))
    }
}

#[derive(Default)]
struct Acc {
    kind: String,
    short: Option<String>,
    card: Option<String>,
    state: Option<Familiar>,
    forms: BTreeMap<String, usize>,
    at: Vec<(String, usize)>,
}

impl Acc {
    fn hit(&mut self, surface: &str, at: &str) {
        *self.forms.entry(surface.to_string()).or_default() += 1;
        match self.at.iter_mut().find(|(a, _)| a == at) {
            Some((_, n)) => *n += 1,
            None => self.at.push((at.to_string(), 1)),
        }
    }
}

fn span(text: &str, toks: &[Tok], i: usize, n: usize) -> String {
    text[toks[i].b0..toks[i + n - 1].b1].to_string()
}

fn chain_open(toks: &[Tok], i: usize, n: usize) -> bool {
    (i..i + n - 1).all(|j| toks[j].open_next)
}

/// An expansion before `(ACR)`: the last words whose initials spell the acronym
/// (function words inside are skipped). Returns the first word's index.
fn expansion_start(text: &str, toks: &[Tok], i: usize, f: &Filters) -> Option<usize> {
    let t = &toks[i];
    if i == 0 || !text[..t.b0].trim_end().ends_with('(') || !text[t.b1..].trim_start().starts_with(')') {
        return None;
    }
    let acr: Vec<char> = t.text.trim_end_matches('s').chars().filter(|c| c.is_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    let mut need = acr.len();
    let mut j = i;
    let mut initials = vec![];
    while need > 0 && j > 0 {
        j -= 1;
        if j + 1 < i && !toks[j].open_next {
            return None;
        }
        let w = &toks[j].text;
        if f.is_function(w) && !initials.is_empty() {
            continue;
        }
        initials.push(w.chars().next()?.to_ascii_lowercase());
        need -= 1;
    }
    initials.reverse();
    (need == 0 && initials == acr).then_some(j)
}

/// The paper's domain terms, each with its state and where it appears. `items` are
/// `(where, text)` pairs (sections or slides).
pub fn find_terms(items: &[(String, String)], kb: &[KbName], f: &Filters) -> TermsResult {
    // KB names by phrase key; names that are only common words do not count
    let mut kbk: HashMap<String, &KbName> = HashMap::new();
    for n in kb {
        let words: Vec<&str> = n.name.split(|c: char| c.is_whitespace() || is_hyphen(c)).filter(|w| !w.is_empty()).collect();
        if words.is_empty() || words.len() > 4 || words.iter().all(|w| f.is_general(w) || f.is_function(w)) && !words.iter().any(|w| is_acronym(w)) {
            continue;
        }
        kbk.entry(phrase_key(&n.name)).or_insert(n);
    }
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    // acronym key → term key of its expansion
    let mut short_of: HashMap<String, String> = HashMap::new();
    let card_key = |n: &KbName| format!("card:{}", n.card);
    // pass 1: expansions (so an acronym used before its definition elsewhere still groups)
    for (_, text) in items {
        let toks = tokens(text);
        for i in 0..toks.len() {
            if !acronym(&toks[i].text, f) {
                continue;
            }
            if let Some(j) = expansion_start(text, &toks, i, f) {
                let exp = span(text, &toks, j, i - j);
                let ek = phrase_key(&exp);
                let ak = toks[i].key.clone();
                let key = kbk.get(&ek).or_else(|| kbk.get(&ak)).map(|n| card_key(n)).unwrap_or_else(|| ek.clone());
                short_of.entry(ak).or_insert(key);
            }
        }
    }
    let mut phrases: HashMap<Vec<String>, (usize, Vec<(String, String)>)> = HashMap::new();
    for (at, text) in items {
        let toks = tokens(text);
        // a KB name starting at i, longest first
        let kb_at = |i: usize| -> Option<(usize, &KbName)> {
            (1..=4.min(toks.len() - i)).rev().filter(|&n| chain_open(&toks, i, n)).find_map(|n| {
                let k = toks[i..i + n].iter().map(|t| t.key.as_str()).collect::<Vec<_>>().join(" ");
                kbk.get(&k).map(|kn| (n, *kn))
            })
        };
        let mut i = 0;
        while i < toks.len() {
            let mut done = false;
            if let Some((n, kn)) = kb_at(i) {
                {
                    let key = card_key(kn);
                    let a = acc.entry(key).or_insert_with(|| Acc { kind: "kb".into(), card: Some(kn.card.clone()), state: Some(kn.state), ..Default::default() });
                    a.hit(&span(text, &toks, i, n), at);
                    // "Spectral Boundary Loss (SBL)": the acronym belongs to this card
                    if n > 1 && i + n < toks.len() && acronym(&toks[i + n].text, f) && short_of.get(&toks[i + n].key).is_some_and(|s| s == &format!("card:{}", kn.card)) {
                        a.short.get_or_insert(toks[i + n].text.trim_end_matches('s').to_string());
                    }
                    i += n;
                    done = true;
                }
            }
            if done {
                continue;
            }
            let t = &toks[i];
            if acronym(&t.text, f) {
                let (key, kind) = match short_of.get(&t.key) {
                    Some(k) => (k.clone(), "expansion"),
                    None => (t.key.clone(), "acronym"),
                };
                let a = acc.entry(key).or_insert_with(|| Acc { kind: kind.into(), ..Default::default() });
                if kind == "expansion" {
                    a.short.get_or_insert(t.text.trim_end_matches('s').to_string());
                } else {
                    a.hit(&t.text, at);
                }
                // the acronym counts as a use of the expansion, but the expansion's own
                // words are counted below as a phrase occurrence
                if kind == "expansion" {
                    *a.forms.entry(t.text.clone()).or_default() += 0;
                    match a.at.iter_mut().find(|(x, _)| x == at) {
                        Some((_, n)) => *n += 1,
                        None => a.at.push((at.clone(), 1)),
                    }
                }
                i += 1;
                continue;
            }
            // phrase candidates: n-grams of content words in one open chain
            if content(t, f) {
                let mut j = i;
                while j + 1 < toks.len() && toks[j].open_next && content(&toks[j + 1], f) && !acronym(&toks[j + 1].text, f) && kb_at(j + 1).is_none() {
                    j += 1;
                }
                for a in i..=j {
                    for n in 2..=4 {
                        if a + n - 1 > j {
                            break;
                        }
                        let ws: Vec<String> = toks[a..a + n].iter().map(|t| t.key.clone()).collect();
                        let e = phrases.entry(ws).or_default();
                        e.0 += 1;
                        e.1.push((at.clone(), span(text, &toks, a, n)));
                    }
                }
                i = j + 1;
                continue;
            }
            i += 1;
        }
    }
    // expansions: their own words are phrase occurrences
    let exp_keys: HashSet<String> = short_of.values().filter(|k| !k.starts_with("card:")).cloned().collect();
    for (ws, (_, occ)) in &phrases {
        let k = ws.join(" ");
        if exp_keys.contains(&k)
            && let Some(a) = acc.get_mut(&k)
        {
            for (at, s) in occ {
                a.hit(s, at);
            }
        }
    }
    // phrases: used twice, the first word not general English, longest wins
    let general = |w: &str| f.is_general(w) || f.is_function(w);
    let first = |s: &str| s.split(|c: char| c.is_whitespace() || is_hyphen(c)).next().unwrap_or("").to_string();
    let mut chosen: Vec<(&Vec<String>, &(usize, Vec<(String, String)>))> = phrases
        .iter()
        .filter(|(ws, (n, occ))| *n >= 2 && !exp_keys.contains(&ws.join(" ")) && !general(&first(&occ[0].1)))
        .collect();
    chosen.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
    let mut kept: Vec<(&Vec<String>, usize)> = vec![];
    for (ws, (n, occ)) in chosen {
        // inside a longer kept phrase every time it occurs
        if kept.iter().any(|(l, ln)| l.len() > ws.len() && *ln >= *n && l.windows(ws.len()).any(|w| w == ws.as_slice())) {
            continue;
        }
        kept.push((ws, *n));
        let a = acc.entry(ws.join(" ")).or_insert_with(|| Acc { kind: "phrase".into(), ..Default::default() });
        for (at, s) in occ {
            a.hit(s, at);
        }
    }
    let mut terms: Vec<Term> = acc
        .into_iter()
        .filter(|(_, a)| !a.forms.is_empty() && a.at.iter().map(|x| x.1).sum::<usize>() > 0)
        .filter(|(k, _)| !f.is_ignored(k))
        .map(|(key, a)| {
            let state = a.state.unwrap_or(Familiar::Unknown);
            let text = a.forms.iter().filter(|(s, _)| a.short.as_deref() != Some(s.trim_end_matches('s'))).max_by_key(|(_, n)| **n).map(|(s, _)| s.clone()).unwrap_or_default();
            let count = a.at.iter().map(|x| x.1).sum();
            Term {
                key,
                text,
                kind: a.kind,
                short: a.short,
                card: a.card,
                state,
                label: state.label().into(),
                count,
                at: a.at.into_iter().map(|(at, count)| Where { at, count }).collect(),
                forms: a.forms.into_keys().collect(),
                understood: false,
            }
        })
        .filter(|t| !t.text.is_empty())
        .collect();
    terms.sort_by(|a, b| (b.state == Familiar::Unknown).cmp(&(a.state == Familiar::Unknown)).then(b.count.cmp(&a.count)).then(a.text.cmp(&b.text)));
    let mut r = TermsResult { terms, ..Default::default() };
    r.recount();
    r
}

/// The `terms` stage in one call: the author's KnowledgeBase (read-only) against `items`
/// (`(section, text)`), filtered with the built-in lists.
pub fn terms_for(home: &lore_home::Home, items: &[(String, String)]) -> TermsResult {
    let kb = load_names(&space_dirs(home), &member(home));
    find_terms(items, &kb, &Filters::default())
}

/// Page roles (todeck's `ROLES`) whose sentences are marked whole: background and method narration.
pub fn sentence_role(role: &str) -> bool {
    matches!(role, "background" | "challenge" | "overview" | "method" | "loss")
}

// ── marks ────────────────────────────────────────────────────────────────────

/// Term spellings to find in slide text (by word keys, so plurals and hyphens match).
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    keys: HashSet<String>,
    max: usize,
}

impl Matcher {
    pub fn new(forms: impl IntoIterator<Item = String>) -> Matcher {
        let mut m = Matcher::default();
        for f in forms {
            let k = phrase_key(&f);
            if k.is_empty() {
                continue;
            }
            m.max = m.max.max(k.split(' ').count());
            m.keys.insert(k);
        }
        m
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Byte ranges of the terms in `text`.
    pub fn find(&self, text: &str) -> Vec<(usize, usize)> {
        let toks = tokens(text);
        let mut out = vec![];
        let mut i = 0;
        'outer: while i < toks.len() {
            for n in (1..=self.max.min(toks.len() - i)).rev() {
                if !chain_open(&toks, i, n) {
                    continue;
                }
                let k = toks[i..i + n].iter().map(|t| t.key.as_str()).collect::<Vec<_>>().join(" ");
                if self.keys.contains(&k) {
                    out.push((toks[i].b0, toks[i + n - 1].b1));
                    i += n;
                    continue 'outer;
                }
            }
            i += 1;
        }
        out
    }
}

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

/// Create a minimal card in state 記得 through KnowledgeBase's API. `Err` = a line for the author.
pub async fn save_known(base: &str, term: &str) -> Result<String, String> {
    let down = || "KnowledgeBase 沒有開啟，沒有存進知識庫".to_string();
    let http = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(8)).build().map_err(|e| e.to_string())?;
    let spaces: Value = http.get(format!("{base}/api/spaces")).send().await.map_err(|_| down())?.json().await.map_err(|_| down())?;
    let list = spaces["spaces"].as_array().cloned().unwrap_or_default();
    let space = list
        .iter()
        .find(|s| s["kind"] == "personal" && s["is_member"] == true)
        .or_else(|| list.iter().find(|s| s["is_member"] == true))
        .and_then(|s| s["id"].as_str())
        .ok_or("知識庫沒有你能寫入的空間")?
        .to_string();
    let body = json!({ "space": space, "via": "slidecomposer", "kind": "concept", "title": term.trim(), "initial_state": "known" });
    let res = http.post(format!("{base}/api/nodes")).json(&body).send().await.map_err(|_| down())?;
    let ok = res.status().is_success();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if !ok {
        return Err(format!("知識庫：{}", v["error"].as_str().unwrap_or("沒有存進去")));
    }
    Ok(v["node"]["id"].as_str().or(v["id"].as_str()).unwrap_or_default().to_string())
}

/// `PUT /api/decks/{name}/learn`: 我懂 — clear the term's marks (one undoable step).
pub async fn understood(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<UnderstoodBody>) -> R {
    let term = b.term.trim().to_string();
    if term.is_empty() {
        return Err(bad("缺少名詞"));
    }
    let kb = if b.kb {
        match save_known(&kb_url(), &term).await {
            Ok(id) => json!({ "saved": true, "id": id, "message": "已存進知識庫（記得）" }),
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
