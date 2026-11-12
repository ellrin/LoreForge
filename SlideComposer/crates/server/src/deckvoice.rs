//! The author's voice (語氣) for 論文報告: example lines from the author's own earlier decks —
//! titles, bullets, take-home lines and captions per page role — that the outline prompt (and
//! the 建議 prompt) shows as "write like these", next to [`crate::deckstyle`]'s statistics.
//! Sending the author's own sentences to the model was approved by the author on 2026-11-13
//! (docs/paper-deck.md 「內容順序與風格」).
//!
//! - Source: exactly the decks `deckstyle` reads (lore-home's `private/SlideComposer/decks`,
//!   read-only), only decks imported from the author's own files ([`deckstyle::qualifies`]):
//!   decks SlideComposer generated never feed back.
//! - Left out: cover and thanks slides, footers and other chrome placeholders, text and
//!   pictures repeated across a deck (footers, confidential marks, presenter lines, logos),
//!   `Ref:` lines and citations (`et al.`), lines with e-mail addresses or links, confidential
//!   / copyright marks, presenter lines, the author's name and id (`shared/profile.toml`) and
//!   every word of `shared/deny-words.txt` (employer, course…), numbers-only lines.
//! - Stored in the author's private workspace, the tool-data tier (git-backed):
//!   `private/SlideComposer/voice/examples.json` (through lore-home, never the public repo, a
//!   cache or a code constant). Rebuilt when the decks change (same key as `deckstyle`'s
//!   cache, plus [`VERSION`]); the key lives in the cache tier (`cache/deck-style/voice-key.json`)
//!   and the examples file is rewritten only when its content changes, so the private repo
//!   sees a change only when the examples do.
//! - Use: [`Voice::block`] picks a few lines per role, most similar to the current paper first,
//!   bounded in characters.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lore_canvas::{Deck, Object};
use serde::{Deserialize, Serialize};

use crate::deckstyle::{self, ROLES, body_of, chrome_ph, norm_text, ref_prefix, title_object};

/// Bump when what is extracted changes (rebuilds the file).
pub const VERSION: u32 = 2;
const APP: &str = "SlideComposer";
const MAX_TITLES: usize = 12;
const MAX_BULLETS: usize = 30;
const MAX_CAPTIONS: usize = 10;
const MAX_CHARS: usize = 200;

/// Example lines of one page role.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoleVoice {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub titles: Vec<String>,
    /// Body lines (on the take-home role: the take-home messages).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bullets: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub captions: Vec<String>,
}

/// `voice/examples.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Voice {
    pub version: u32,
    /// Decks the lines come from.
    pub decks: usize,
    pub roles: BTreeMap<String, RoleVoice>,
}

impl Voice {
    pub fn count(&self) -> usize {
        self.roles.values().map(|r| r.titles.len() + r.bullets.len() + r.captions.len()).sum()
    }
}

// ── What is never an example ─────────────────────────────────────────────────

/// Words and names that never leave in an example line.
pub struct Exclude {
    words: Vec<regex::Regex>,
}

static MARKS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)confidential|機密|internal use|proprietary|all rights reserved|©|\(c\)\s*\d{4}|copyright|do not distribute|請勿外流|presented by|presenter|speaker:|報告人|講者|\bet al\.?|https?://|www\.|@").unwrap()
});

impl Exclude {
    /// The author's name and id (`profile.toml`) and the deny words (`deny-words.txt`, one
    /// regular expression per line, case-insensitive, whole word).
    pub fn for_home(home: &lore_home::Home) -> Exclude {
        let mut pats: Vec<String> = vec![];
        let p = home.prefs().profile;
        for n in [p.name, p.id].into_iter().flatten() {
            for part in std::iter::once(n.clone()).chain(n.split_whitespace().map(String::from)) {
                if part.chars().count() >= 2 {
                    pats.push(regex::escape(&part));
                }
            }
        }
        if let Some(f) = home.shared_file("deny-words.txt")
            && let Ok(t) = std::fs::read_to_string(f)
        {
            pats.extend(t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from));
        }
        Exclude::new(&pats)
    }

    pub fn new(pats: &[String]) -> Exclude {
        let words = pats
            .iter()
            .filter_map(|p| regex::Regex::new(&format!(r"(?i)(?:^|\b|\W)(?:{p})(?:\b|\W|$)")).or_else(|_| regex::Regex::new(&format!("(?i){}", regex::escape(p)))).ok())
            .collect();
        Exclude { words }
    }

    /// The line may be an example.
    pub fn allows(&self, line: &str) -> bool {
        let t = line.trim();
        let letters = t.chars().filter(|c| c.is_alphabetic()).count();
        letters >= 2 && t.chars().count() <= MAX_CHARS && ref_prefix(t).is_none() && !MARKS.is_match(t) && !self.words.iter().any(|w| w.is_match(t))
    }
}

// ── Extraction ───────────────────────────────────────────────────────────────

static CITATION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r",\s*(19|20)\d\d\.?\s*$|\((19|20)\d\d\)").unwrap());

/// A body line worth imitating: a phrase (three words, or four CJK characters), not an
/// equation (math letters, `=`, `∈`) nor a citation.
pub fn sentence_like(line: &str) -> bool {
    let mathy = line.chars().any(|c| matches!(c as u32, 0x1D400..=0x1D7FF) || matches!(c, '=' | '∈' | 'ℝ' | '∑' | '∆'));
    let cjk = line.chars().filter(|c| deckstyle::mostly_cjk(&c.to_string())).count();
    let words = line.split_whitespace().filter(|w| w.chars().any(char::is_alphabetic)).count();
    !mathy && !CITATION.is_match(line) && (words >= 3 || cjk >= 4)
}

/// Private-use bullet glyphs (Wingdings arrows…) and spaces off the line.
fn clean_line(s: &str) -> String {
    let s: String = s.chars().filter(|c| !matches!(*c as u32, 0xE000..=0xF8FF)).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_caption(o: &Object, text: &str) -> bool {
    let tagged = body_of(o).is_some_and(|b| b.paras.iter().any(|p| p.runs.iter().any(|r| r.tag.as_deref() == Some("caption"))));
    let t = text.trim_start();
    o.name == "圖說" || o.slot.as_deref().is_some_and(|s| s.starts_with("caption")) || tagged || ((t.starts_with("Fig") || t.starts_with('圖')) && t.chars().count() <= 220)
}

#[derive(Default)]
struct Acc {
    titles: BTreeMap<String, Vec<(usize, String)>>,
    bullets: BTreeMap<String, Vec<(usize, String)>>,
    captions: BTreeMap<String, Vec<(usize, String)>>,
}

fn add_deck(acc: &mut Acc, ix: usize, deck: &Deck, ex: &Exclude) {
    let h = deck.size.h.max(1);
    let (bp_text, _) = deckstyle::boilerplate(deck);
    let titles: Vec<String> = deck.slides.iter().map(|s| deckstyle::title_text(s, h)).collect();
    let roles = deckstyle::classify(&titles);
    let ok = |t: &str| !t.trim().is_empty() && !bp_text.contains(&norm_text(t)) && ex.allows(t);
    for ((s, title), role) in deck.slides.iter().zip(&titles).zip(&roles) {
        if s.hidden || matches!(role.as_str(), "cover" | "thanks") {
            continue;
        }
        if ok(title) {
            acc.titles.entry(role.clone()).or_default().push((ix, title.trim().to_string()));
        }
        let title_id = title_object(s, h).map(|o| o.id);
        for o in s.objects.iter().filter(|o| !o.hidden && !chrome_ph(o) && Some(o.id) != title_id) {
            let Some(b) = body_of(o) else { continue };
            let whole = b.plain_text();
            if bp_text.contains(&norm_text(&whole)) {
                continue;
            }
            let caption = is_caption(o, &whole);
            for p in &b.paras {
                let line: String = p.runs.iter().filter(|r| r.em.as_deref() != Some(lore_canvas::style::GHOST_EM)).map(|r| r.text.as_str()).collect();
                let line = clean_line(&line);
                if !ok(&line) || !sentence_like(&line) {
                    continue;
                }
                let into = if caption { &mut acc.captions } else { &mut acc.bullets };
                into.entry(role.clone()).or_default().push((ix, line));
            }
        }
    }
}

/// Up to `cap` distinct lines, taking the decks in turn (one deck never fills a role alone).
fn spread(v: &[(usize, String)], cap: usize) -> Vec<String> {
    let mut by: BTreeMap<usize, Vec<&String>> = BTreeMap::new();
    for (d, t) in v {
        by.entry(*d).or_default().push(t);
    }
    let (mut out, mut seen) = (vec![], BTreeSet::new());
    let longest = by.values().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for list in by.values() {
            if let Some(t) = list.get(i)
                && out.len() < cap
                && seen.insert(norm_text(t))
            {
                out.push((*t).clone());
            }
        }
    }
    out
}

/// The examples of these decks (None when none qualifies). Identical decks count once.
pub fn voice_of(decks: &[Deck], ex: &Exclude) -> Option<Voice> {
    let mut acc = Acc::default();
    let mut seen = BTreeSet::new();
    for d in decks.iter().filter(|d| deckstyle::qualifies(d)) {
        if !seen.insert(lore_canvas::fp(&d.slides)) {
            continue;
        }
        add_deck(&mut acc, seen.len() - 1, d, ex);
    }
    if seen.is_empty() {
        return None;
    }
    let mut roles = BTreeMap::new();
    for r in ROLES {
        let get = |m: &BTreeMap<String, Vec<(usize, String)>>, cap| m.get(r).map(|v| spread(v, cap)).unwrap_or_default();
        let rv = RoleVoice { titles: get(&acc.titles, MAX_TITLES), bullets: get(&acc.bullets, MAX_BULLETS), captions: get(&acc.captions, MAX_CAPTIONS) };
        if rv != RoleVoice::default() {
            roles.insert(r.to_string(), rv);
        }
    }
    Some(Voice { version: VERSION, decks: seen.len(), roles })
}

// ── Storage ──────────────────────────────────────────────────────────────────

/// `private/SlideComposer/voice/examples.json` (through lore-home).
pub fn examples_file(home: &lore_home::Home) -> Option<PathBuf> {
    home.app_dir(APP).map(|d| d.join("voice").join("examples.json"))
}

fn key_file(cache_root: &Path) -> PathBuf {
    cache_root.join("deck-style").join("voice-key.json")
}

#[derive(Serialize, Deserialize)]
struct Key {
    key: String,
}

/// The examples in the private workspace (no rebuild).
pub fn load(home: &lore_home::Home) -> Option<Voice> {
    let v: Voice = serde_json::from_slice(&std::fs::read(examples_file(home)?).ok()?).ok()?;
    (v.version == VERSION && v.count() > 0).then_some(v)
}

/// The examples, rebuilt first when the author's decks (or the exclusions) changed. Read-only on
/// the decks; writes only `voice/examples.json` in the private workspace (when its content
/// changes) and the key in `cache_root`.
pub fn refresh(home: &lore_home::Home, cache_root: &Path) -> Option<Voice> {
    let decks_dir = deckstyle::author_decks(home)?;
    let file = examples_file(home)?;
    let deny = home.shared_file("deny-words.txt").and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
    let who = format!("{:?}", home.prefs().profile);
    let key = format!("voice{VERSION}-{}-{:016x}", deckstyle::cache_key(&decks_dir), lore_canvas::fp(&(deny, who)));
    let kf = key_file(cache_root);
    let stored = std::fs::read(&kf).ok().and_then(|b| serde_json::from_slice::<Key>(&b).ok()).map(|k| k.key);
    if stored.as_deref() == Some(key.as_str())
        && let Some(v) = load(home)
    {
        return Some(v);
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&decks_dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    dirs.sort();
    let decks: Vec<Deck> = dirs
        .iter()
        .filter_map(|d| std::fs::read(d.join("deck.json")).ok())
        .filter_map(|b| serde_json::from_slice::<Deck>(&b).ok())
        .collect();
    let voice = voice_of(&decks, &Exclude::for_home(home))?;
    let bytes = serde_json::to_vec_pretty(&voice).unwrap_or_default();
    if std::fs::read(&file).ok().as_deref() != Some(bytes.as_slice()) {
        if let Some(dir) = file.parent()
            && std::fs::create_dir_all(dir).is_err()
        {
            return Some(voice);
        }
        if let Err(e) = sc_core::write_atomic(&file, &bytes) {
            eprintln!("deckvoice: {}: {e}", file.display());
        }
    }
    if let Some(dir) = kf.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let _ = sc_core::write_atomic(&kf, &serde_json::to_vec(&Key { key }).unwrap_or_default());
    }
    Some(voice)
}

// ── For the prompts ──────────────────────────────────────────────────────────

/// Latin words (3+ letters) and CJK character pairs, lower-cased.
pub fn tokens(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let low = s.to_lowercase();
    for w in low.split(|c: char| !c.is_ascii_alphanumeric()) {
        if w.len() >= 3 && !matches!(w, "the" | "and" | "for" | "with" | "from" | "this" | "that" | "are" | "was" | "our") {
            out.insert(w.to_string());
        }
    }
    let cjk: Vec<char> = low.chars().collect();
    for w in cjk.windows(2) {
        if w.iter().all(|c| deckstyle::mostly_cjk(&c.to_string())) {
            out.insert(w.iter().collect());
        }
    }
    out
}

/// The `n` lines most similar to `query` (shared tokens; ties keep the stored order), never two
/// near-copies of one line (the same bullet reused in several talks).
fn best<'a>(lines: &'a [String], query: &BTreeSet<String>, n: usize) -> Vec<&'a String> {
    let mut v: Vec<(usize, usize, &String, BTreeSet<String>)> = lines.iter().enumerate().map(|(i, l)| { let t = tokens(l); (t.intersection(query).count(), i, l, t) }).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<(&String, BTreeSet<String>)> = vec![];
    for (_, _, l, t) in v {
        let near = out.iter().any(|(_, u)| { let common = t.intersection(u).count(); common * 10 >= t.union(u).count() * 6 });
        if !near {
            out.push((l, t));
        }
        if out.len() == n {
            break;
        }
    }
    out.into_iter().map(|(l, _)| l).collect()
}

fn quoted(lines: &[&String]) -> String {
    lines.iter().map(|l| format!("\"{}\"", crate::todeck::plan::clip(l, 120))).collect::<Vec<_>>().join(" · ")
}

impl Voice {
    /// The "write like these" block: per role a few titles, lines and captions, most similar to
    /// `query` (the paper) first, at most `budget` characters. `lang`: the deck's language.
    pub fn block(&self, query: &str, lang: &str, budget: usize) -> String {
        if self.count() == 0 {
            return String::new();
        }
        let q = tokens(query);
        let language = if lang == "en" { "English" } else { "Traditional Chinese (technical terms in English)" };
        let mut o = format!(
            "Write like the presenter. The lines below come from the presenter's own earlier decks about OTHER papers: imitate their tone — wording, length, how Chinese and English are mixed, verb-first bullets, punctuation, how take-home lines are phrased — never their content, and never copy them. \
Some are in another language than this deck; they still show the tone, but write this deck in {language}.\n"
        );
        let head = o.chars().count();
        let mut captions = 0;
        for r in ROLES {
            let Some(rv) = self.roles.get(r) else { continue };
            let mut parts = vec![];
            if !rv.titles.is_empty() {
                parts.push(format!("titles {}", quoted(&best(&rv.titles, &q, 2))));
            }
            if !rv.bullets.is_empty() {
                let label = if r == "takeaway" { "take-home lines" } else { "lines" };
                parts.push(format!("{label} {}", quoted(&best(&rv.bullets, &q, 3))));
            }
            if !rv.captions.is_empty() && captions < 2 {
                captions += 1;
                parts.push(format!("caption {}", quoted(&best(&rv.captions, &q, 1))));
            }
            let line = format!("- {r}: {}\n", parts.join("; "));
            if o.chars().count() + line.chars().count() > budget {
                break;
            }
            o += &line;
        }
        if o.chars().count() == head { String::new() } else { o }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_canvas::{Bullet, Frame, Geom, Kind, Look, Para, Run, Slide, SlideSrc, SpObj, TextBody};

    const W: i64 = 12_192_000;
    const H: i64 = 6_858_000;

    fn text(id: u32, f: Frame, lines: &[&str], ph: Option<&str>, name: &str) -> Object {
        let paras = lines
            .iter()
            .map(|t| {
                let mut r = Run::text(t, "body", Look { size: 18.0, ..Look::default() });
                r.tag = None;
                let mut p = Para::new(vec![r]);
                p.bullet = Bullet::Char { ch: "•".into() };
                p
            })
            .collect();
        Object { id, name: name.into(), frame: f, hidden: false, ph: ph.map(String::from), slot: None, kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(TextBody { paras, ..Default::default() }), equation: false }), src: None }
    }
    use lore_canvas::Paint;

    /// A synthetic talk of the author's: footer, confidential mark and presenter line on every
    /// slide, a `Ref:` line, an e-mail, the author's name, a deny word, a caption.
    fn talk(tag: &str, imported: bool) -> Deck {
        let pages: [(&str, &[&str]); 8] = [
            ("Cover talk by Ellrin Example", &["Ellrin Example · lab meeting"]),
            ("Background", &["臨床上 grading 很花時間", "Experts disagree on borderline cases", "Contact: ellrin@example.org"]),
            ("Overview", &["Split the task into detect and grade"]),
            ("Encoder", &["Use a small CNN to find lesion candidates", "Built at Example-Company labs"]),
            ("Results", &["Beats the baseline on every metric", "Ref: Daidai et al., Synthetic Lesions, 2025"]),
            ("Ablation study", &["Removing fusion costs the most"]),
            ("Take home messages", &["1. Split hard tasks into small steps", "2. Check every stage by hand"]),
            ("Thanks", &["Questions?"]),
        ];
        let mut d = Deck::new(W, H, Some("blank".into()));
        for (i, (title, body)) in pages.iter().enumerate() {
            let mut s = Slide::new(d.new_uid(), None);
            if imported {
                s.src = Some(SlideSrc { part: format!("ppt/slides/slide{i}.xml"), k: i as u64, notes_k: 0, anims_k: 0 });
            }
            s.objects.push(text(2, Frame::new(W / 25, H / 20, W * 9 / 10, H / 8), &[title], Some("title"), "Title"));
            let mine: Vec<String> = body.iter().map(|b| format!("{b}{tag}")).collect();
            let mine: Vec<&str> = mine.iter().map(String::as_str).collect();
            s.objects.push(text(3, Frame::new(W / 25, H / 4, W * 9 / 10, H / 2), &mine, None, "Body"));
            s.objects.push(text(4, Frame::new(0, H * 9 / 10, W / 2, H / 20), &["Footer of the series"], Some("ftr"), "Footer"));
            s.objects.push(text(5, Frame::new(W / 2, H * 9 / 10, W / 2, H / 20), &["Strictly Confidential"], None, "mark"));
            s.objects.push(text(6, Frame::new(W / 2, H * 8 / 10, W / 2, H / 20), &["Lab logo text"], None, "logo"));
            if i == 2 {
                s.objects.push(text(7, Frame::new(W / 25, H * 3 / 4, W / 2, H / 20), &[&format!("Fig. 1 The two stages{tag}")], None, "圖說"));
            }
            d.slides.push(s);
        }
        d
    }

    fn home_with_decks(root: &Path) -> lore_home::Home {
        let home = lore_home::Home::at(root);
        let shared = home.shared_dir().unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("profile.toml"), "id = \"ellrin\"\nname = \"Ellrin Example\"\n").unwrap();
        std::fs::write(shared.join("deny-words.txt"), "# synthetic\nexample-company\n").unwrap();
        let decks = home.app_dir(APP).unwrap().join("decks");
        for (n, tag, imported) in [("a", "", true), ("b", " again", true), ("made", " generated", false)] {
            std::fs::create_dir_all(decks.join(n)).unwrap();
            std::fs::write(decks.join(n).join("deck.json"), serde_json::to_vec(&talk(tag, imported)).unwrap()).unwrap();
        }
        home
    }

    fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(p: &Path, m: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                let q = e.path();
                if q.is_dir() { walk(&q, m) } else { m.insert(q.clone(), std::fs::read(&q).unwrap()); }
            }
        }
        let mut m = BTreeMap::new();
        walk(root, &mut m);
        m
    }

    #[test]
    fn examples_leave_out_boilerplate_names_and_marks() {
        let tmp = tempfile::tempdir().unwrap();
        let home = home_with_decks(tmp.path());
        let cache = tmp.path().join("ws/cache");
        let before = files(tmp.path());
        let v = refresh(&home, &cache).expect("examples");
        assert_eq!(v.decks, 2, "the generated deck (no src) never counts");
        let all = serde_json::to_string(&v).unwrap();
        for never in ["Footer", "Confidential", "logo", "Ref:", "et al", "@", "Ellrin", "Example-Company", "Questions", "Cover talk", "generated"] {
            assert!(!all.contains(never), "{never} in {all}");
        }
        let bg = &v.roles["background"];
        assert_eq!(bg.titles, ["Background"]);
        assert!(bg.bullets.contains(&"臨床上 grading 很花時間".to_string()) && bg.bullets.contains(&"Experts disagree on borderline cases again".to_string()));
        assert_eq!(v.roles["takeaway"].bullets[0], "1. Split hard tasks into small steps");
        assert_eq!(v.roles["overview"].captions, ["Fig. 1 The two stages", "Fig. 1 The two stages again"]);
        assert!(!v.roles.contains_key("cover") && !v.roles.contains_key("thanks"));

        // stored in the private workspace; nothing else written but the key in the cache
        let file = home.app_dir(APP).unwrap().join("voice/examples.json");
        assert_eq!(file, examples_file(&home).unwrap());
        assert_eq!(load(&home).as_ref(), Some(&v));
        let after = files(tmp.path());
        let new: Vec<&PathBuf> = after.keys().filter(|k| !before.contains_key(*k)).collect();
        assert_eq!(new, [&file, &cache.join("deck-style/voice-key.json")]);
        assert!(before.iter().all(|(k, b)| after.get(k) == Some(b)), "the decks are read-only");

        // unchanged decks: the file is not rewritten; a changed deck rebuilds it
        let t0 = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        refresh(&home, &cache).unwrap();
        std::fs::remove_file(cache.join("deck-style/voice-key.json")).unwrap();
        refresh(&home, &cache).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), t0, "same content, same file");
        let decks = home.app_dir(APP).unwrap().join("decks");
        std::fs::create_dir_all(decks.join("c")).unwrap();
        std::fs::write(decks.join("c/deck.json"), serde_json::to_vec(&talk(" third", true)).unwrap()).unwrap();
        assert_eq!(refresh(&home, &cache).unwrap().decks, 3);
        assert!(std::fs::read_to_string(&file).unwrap().contains("third"));
    }

    #[test]
    fn equations_citations_and_labels_are_no_examples() {
        assert!(sentence_like("Use a small CNN to find lesions") && sentence_like("臨床上分級很花時間"));
        for no in ["Goal", "Node:", "T=BERT(W), T∈ℝ", "( 𝐸𝑠𝐼𝑥, 𝐸𝑠𝐼𝑑 )", "A survey of synthetic tests, Journal of Tests, 2021", "Some dataset of ribs (2021) for tests"] {
            assert!(!sentence_like(no), "{no}");
        }
        assert_eq!(clean_line("\u{f0e0} Information  dilution"), "Information dilution");
    }

    #[test]
    fn no_decks_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = lore_home::Home::at(tmp.path());
        assert!(refresh(&home, &tmp.path().join("cache")).is_none());
        assert!(!tmp.path().join("private").exists());
    }

    #[test]
    fn block_is_bounded_and_prefers_similar_lines() {
        let mut v = Voice { version: VERSION, decks: 2, roles: BTreeMap::new() };
        v.roles.insert("background".into(), RoleVoice { titles: vec!["Background".into(), "Why lesion grading is slow".into()], bullets: vec!["Unrelated words here".into(), "Retinal images need experts".into(), "Grading lesions by hand takes hours".into(), "其他".into()], captions: vec![] });
        v.roles.insert("takeaway".into(), RoleVoice { bullets: vec!["1. Split hard tasks into small steps".into()], ..Default::default() });
        let b = v.block("Two-stage grading of synthetic lesions", "zh", 2000);
        assert!(b.starts_with("Write like the presenter") && b.contains("Traditional Chinese") && b.contains("another language"));
        assert!(b.contains("- background: titles \"Why lesion grading is slow\" · \"Background\""), "{b}");
        assert!(b.contains("lines \"Grading lesions by hand takes hours\""), "{b}");
        assert!(b.contains("take-home lines \"1. Split hard tasks into small steps\""));
        let full = v.block("x", "en", 5000);
        let first = full.find("\n- takeaway").unwrap() + 1;
        let budget = full[..first].chars().count() + 10;
        let small = v.block("x", "en", budget);
        assert!(small.chars().count() <= budget && small.contains("background") && !small.contains("takeaway"), "{small}");
        assert_eq!(v.block("x", "en", 300), "", "not even one role fits");
        assert_eq!(Voice::default().block("x", "en", 1000), "");
    }

    /// The author's real decks (never in CI): writes `private/SlideComposer/voice/examples.json`
    /// (the intended place) and prints counts only.
    /// `SC_VOICE_CACHE=<scratch dir> cargo test -p sc-server --lib deckvoice::tests::real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real() {
        let Ok(c) = std::env::var("SC_VOICE_CACHE") else { return };
        let home = lore_home::Home::discover(&crate::repo_dir());
        let v = refresh(&home, Path::new(&c)).expect("examples");
        println!("decks {} · lines {}", v.decks, v.count());
        for (r, rv) in &v.roles {
            println!("{r}: titles {} · lines {} · captions {}", rv.titles.len(), rv.bullets.len(), rv.captions.len());
        }
    }

}
