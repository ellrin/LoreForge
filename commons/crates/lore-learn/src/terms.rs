//! 「不熟」: a paper's domain terms against the author's KnowledgeBase (docs/paper-deck.md
//! 「不熟」標示, docs/book.md `terms`). Lifted from SlideComposer's `kbterms.rs`, which keeps
//! the slide marks and re-exports this module; PaperComposer's 寫書 uses it for its `terms` stage.
//!
//! * **Terms** ([`find_terms`], pure): acronyms, multi-word phrases (an expansion before its
//!   acronym in parentheses, "Spectral Boundary Loss (SBL)"; or a 2–4-word phrase used at least
//!   twice whose first word is not general English) and KnowledgeBase titles /
//!   aliases (1–4 words). Common words never count: `lore-words`, the filter LoreReader uses.
//! * **State** per term: 熟 = card `known`; 學習中 = card `learning` / `stale` / no learning
//!   state for this member; 不熟 = no card, or a card whose last event is 標為不熟 (要學).
//! * **KnowledgeBase**, read-only from disk (`nodes/*.json` + `progress/<member>/*.json`; spaces:
//!   `KB_SPACES` > KnowledgeBase's `config.toml` `spaces` > lore-home `private/KnowledgeBase`;
//!   member: `KB_MEMBER` > `config.toml` `member` > `private/shared/profile.toml` `id`).
//! * [`prerequisites`]: the cards' `prerequisite` links (a card → what it needs first).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use kb_core::Node;
use kb_core::progress::{Progress, State as KbState};
use lore_words::{Filters, is_acronym, is_hyphen, phrase_key, word_key};
use serde::{Deserialize, Serialize};

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
/// (`(section, text)`), filtered like LoreReader: the built-in lists plus the author's
/// `private/LoreReader/ignore-words.txt` (read-only).
pub fn terms_for(home: &lore_home::Home, items: &[(String, String)]) -> TermsResult {
    let kb = load_names(&space_dirs(home), &member(home));
    let filters = home.app_dir("LoreReader").map(|d| Filters::with_file(&d.join("ignore-words.txt"))).unwrap_or_default();
    find_terms(items, &kb, &filters)
}

/// Page roles (todeck's `ROLES`) whose sentences are marked whole: background and method narration.

// ── finding terms in text ────────────────────────────────────────────────────

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


/// The cards' `prerequisite` links, read-only: `(card, needs)` pairs, `needs` to be learnt
/// before `card`. Unreadable files are skipped.
pub fn prerequisites(spaces: &[PathBuf]) -> Vec<(String, String)> {
    let mut out = vec![];
    for root in spaces {
        let Ok(space) = lore_store::Space::open(root) else { continue };
        let Ok(rd) = std::fs::read_dir(space.nodes_dir()) else { continue };
        for f in rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")) {
            let Some(n) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<Node>(&b).ok()) else { continue };
            for l in n.links.iter().filter(|l| l.rel == "prerequisite" && !l.to.is_empty()) {
                out.push((n.id.clone(), l.to.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}
