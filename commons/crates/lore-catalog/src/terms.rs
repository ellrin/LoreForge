//! The term pool: words and short phrases in the author's files that his KnowledgeBase does not
//! have yet, with the files and sentences they come from — candidates, never cards. Built from the
//! whole text (`fulltext`) without a model; a model later says which are real terms
//! (`Term::verdict`). Kept in the cache tier (`<cache>/LoreForge/shelf/terms.json`, rebuilt from
//! the files at any time); only the author's decisions about them go into the catalog's edits.
//!
//! This first pass reads Latin script only (words, acronyms, two-word phrases); Chinese terms
//! need a model to be cut out and come with the model pass.

use crate::fulltext::FullText;
use lore_words::{Filters, is_acronym, phrase_key, word_key};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Terms kept in the pool (the best by score).
pub const MAX_TERMS: usize = 4_000;
/// Every file also keeps its own best terms, however the pool ranks them overall (a tissue in one
/// set of notes would never make the overall cut).
pub const PER_FILE: usize = 25;
/// Example sentences kept per term.
pub const EXAMPLES: usize = 3;
/// Characters of an example sentence.
pub const EXAMPLE_CHARS: usize = 220;
/// Known cards remembered per term (the ones it appears next to most).
pub const NEAR: usize = 5;
/// How much more often than chance a card must appear next to a term to count as near it.
pub const NEAR_LIFT: f64 = 3.0;
pub const VERSION: u32 = 1;

/// A KnowledgeBase card, as the pool needs it.
#[derive(Debug, Clone, Default)]
pub struct Known {
    pub title: String,
    pub aliases: Vec<String>,
    pub domains: Vec<String>,
    /// `known`, `learning`, `stale`, `na`, or "" (no learning state).
    pub state: String,
}

/// A file to read: its content id, and whether a cloud model may read it.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub cloud_ok: bool,
    /// In a folder the author has said he knows all of (his own seminar talks and their papers).
    pub mastered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Example {
    /// The file's content id.
    pub id: String,
    /// Its page, slide or block.
    pub n: u32,
    pub text: String,
    /// From a file a cloud model may read.
    #[serde(default)]
    pub cloud_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Term {
    pub key: String,
    /// The way it is written most often.
    pub label: String,
    /// Files it is in, and how often in all.
    pub files: usize,
    pub count: usize,
    pub score: f64,
    /// Some file it was found in may go to a cloud model (papers, slide decks): then those
    /// examples (`Example::cloud_ok`) may be shown to one.
    pub cloud_ok: bool,
    pub examples: Vec<Example>,
    /// Known cards it appears next to (same page / slide / block), most often first.
    pub near: Vec<String>,
    /// Of those, how many the author already knows (`known`): a term among things he knows.
    pub near_known: usize,
    /// The domains of those cards, most often first.
    pub domains: Vec<String>,
    /// Found in a file the author knows all of (`Source::mastered`): a known card once it is a term.
    #[serde(default)]
    pub mastered: bool,
    /// The model's word on it (`term`, `general`, `name`, `junk`), once asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pool {
    pub version: u32,
    pub built: String,
    /// Files read, and how many had text.
    pub files: usize,
    pub terms: Vec<Term>,
}

impl Pool {
    pub fn file(cache_dir: &Path) -> PathBuf {
        cache_dir.join("terms.json")
    }

    pub fn load(cache_dir: &Path) -> Option<Pool> {
        serde_json::from_slice(&std::fs::read(Self::file(cache_dir)).ok()?).ok()
    }

    pub fn save(&self, cache_dir: &Path) -> Result<(), crate::CatalogError> {
        let p = Self::file(cache_dir);
        let b = serde_json::to_vec(self).map_err(|e| crate::CatalogError::io(&p, std::io::Error::other(e)))?;
        crate::store::write_atomic(&p, &b)
    }
}

/// A Latin word: letters, inner hyphens and apostrophes; digits only inside (`T2`, `3D` stay whole).
fn words(sentence: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let ok = |c: char| c.is_ascii_alphanumeric() || (c.is_alphabetic() && !cjk(c));
    let b = sentence;
    for (i, c) in b.char_indices().chain([(b.len(), ' ')]) {
        let inner = (c == '-' || c == '\'') && start.is_some() && b[i + 1..].chars().next().is_some_and(ok);
        if ok(c) || inner {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            out.push((s, &b[s..i]));
        }
    }
    out
}

fn cjk(c: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&c) || ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{ac00}'..='\u{d7af}').contains(&c)
}

/// Sentences of a part: cut at . ! ? 。！？ and line ends.
fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '\n')).map(str::trim).filter(|s| s.chars().count() >= 3)
}

/// Parts of addresses and citations, never terms.
const NOISE: [&str; 14] = ["https", "http", "www", "doi", "org", "com", "etal", "et", "al", "pdf", "html", "fig", "eq", "pp"];

/// A word's tail on its own (`tion`): a line break inside a word, not a word.
const TAILS: [&str; 9] = ["tion", "tions", "ment", "ments", "ness", "ing", "ally", "ity", "ies"];

/// A word broken by a PDF's ligatures (`signicant`, `dierence`, `eect`) or a tail on its own.
fn broken(w: &str, lost: &HashSet<String>) -> bool {
    let l = w.to_lowercase();
    TAILS.contains(&l.as_str()) || lost.contains(&l)
}

/// Is this word worth keeping on its own?
fn term_word(w: &str, f: &Filters, lost: &HashSet<String>) -> bool {
    let letters = w.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 || w.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if is_acronym(w) {
        return letters >= 2 && letters <= 8;
    }
    letters >= 4 && !f.is_function(w) && !f.is_general(w) && !f.is_familiar(w) && !w.chars().any(|c| c.is_ascii_digit()) && !NOISE.contains(&w.to_lowercase().as_str()) && !broken(w, lost)
}

/// Can this word be part of a two-word phrase?
fn phrase_word(w: &str, f: &Filters, lost: &HashSet<String>) -> bool {
    w.chars().filter(|c| c.is_alphabetic()).count() >= 3 && !f.is_function(w) && !w.chars().all(|c| c.is_ascii_digit()) && !NOISE.contains(&w.to_lowercase().as_str()) && !broken(w, lost)
}

/// The word checks, remembered per written form (they run for every occurrence of every word;
/// `broken` tries dozens of spellings).
struct Words<'a> {
    f: &'a Filters,
    lost: HashSet<String>,
    term: HashMap<String, bool>,
    phrase: HashMap<String, bool>,
}

impl Words<'_> {
    fn term(&mut self, w: &str) -> bool {
        if let Some(&v) = self.term.get(w) {
            return v;
        }
        let v = term_word(w, self.f, &self.lost);
        self.term.insert(w.to_string(), v);
        v
    }
    fn phrase(&mut self, w: &str) -> bool {
        if let Some(&v) = self.phrase.get(w) {
            return v;
        }
        let v = phrase_word(w, self.f, &self.lost);
        self.phrase.insert(w.to_string(), v);
        v
    }
}

struct Acc {
    labels: HashMap<String, usize>,
    /// Occurrences written with a capital first letter and nothing else (a name: `Wang`, `Taiwan`).
    titlecase: usize,
    sentence_start: usize,
    parts: usize,
    per_file: HashMap<usize, usize>,
    examples: Vec<Example>,
    near: HashMap<usize, usize>,
    any_cloud: bool,
    mastered: bool,
}

/// Build the pool from the cached text of `sources` (`fulltext_dir`), leaving out what `known`
/// already has (titles and aliases) and what `ignore` lists (keys).
pub fn build(fulltext_dir: &Path, sources: &[Source], known: &[Known], ignore: &HashSet<String>, filters: &Filters) -> Pool {
    let known_keys: HashMap<String, usize> = known
        .iter()
        .enumerate()
        .flat_map(|(i, k)| std::iter::once(k.title.as_str()).chain(k.aliases.iter().map(String::as_str)).map(move |s| (phrase_key(s), i)))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    let longest = known_keys.keys().map(|k| k.split(' ').count()).max().unwrap_or(1).min(6);
    let mut acc: HashMap<String, Acc> = HashMap::new();
    let mut ww = Words { f: filters, lost: filters.ligature_losses(), term: HashMap::new(), phrase: HashMap::new() };
    let mut keyc: HashMap<String, String> = HashMap::new();
    let mut with_text = 0;
    let (mut parts_total, mut card_parts): (usize, HashMap<usize, usize>) = (0, HashMap::new());
    for (fi, src) in sources.iter().enumerate() {
        let Some(ft) = FullText::load(fulltext_dir, &src.id) else { continue };
        if ft.parts.is_empty() {
            continue;
        }
        with_text += 1;
        for part in &ft.parts {
            let mut in_part: Vec<(String, String, String, bool)> = Vec::new(); // (key, label, sentence, first word of it)
            let mut near_here: HashSet<usize> = HashSet::new();
            for s in sentences(&part.text) {
                let ws = words(s);
                let keys: Vec<String> = ws.iter().map(|(_, w)| keyc.entry(w.to_string()).or_insert_with(|| word_key(w)).clone()).collect();
                // known cards named here (their titles may run over several words)
                for i in 0..keys.len() {
                    for len in 1..=longest.min(keys.len() - i) {
                        if let Some(&k) = known_keys.get(&keys[i..i + len].join(" ")) {
                            near_here.insert(k);
                        }
                    }
                }
                for (j, (_, w)) in ws.iter().enumerate() {
                    if ww.term(w) {
                        in_part.push((keys[j].clone(), w.to_string(), s.to_string(), j == 0));
                    }
                    if let Some((_, w2)) = ws.get(j + 1)
                        && ww.phrase(w)
                        && ww.phrase(w2)
                        && (ww.term(w) || ww.term(w2))
                        // a card's word and an everyday word (`reported accuracy`) is not a new term
                        && !(known_keys.contains_key(&keys[j]) && !ww.term(w2) || known_keys.contains_key(&keys[j + 1]) && !ww.term(w))
                    {
                        in_part.push((format!("{} {}", keys[j], keys[j + 1]), format!("{w} {w2}"), s.to_string(), j == 0));
                    }
                }
            }
            parts_total += 1;
            for &k in &near_here {
                *card_parts.entry(k).or_default() += 1;
            }
            let mut counted: HashSet<String> = HashSet::new();
            for (key, label, sentence, first) in in_part {
                if known_keys.contains_key(&key) || ignore.contains(&key) {
                    continue;
                }
                let a = acc.entry(key.clone()).or_insert_with(|| Acc { labels: HashMap::new(), titlecase: 0, sentence_start: 0, parts: 0, per_file: HashMap::new(), examples: Vec::new(), near: HashMap::new(), any_cloud: false, mastered: false });
                // a capital at the start of a sentence says nothing; anywhere else it marks a name
                let mut cs = label.chars();
                if first {
                    a.sentence_start += 1;
                } else if !label.contains(' ') && cs.next().is_some_and(char::is_uppercase) && cs.all(|c| !c.is_uppercase()) {
                    a.titlecase += 1;
                }
                *a.labels.entry(label).or_default() += 1;
                *a.per_file.entry(fi).or_default() += 1;
                a.any_cloud |= src.cloud_ok;
                a.mastered |= src.mastered;
                if a.examples.len() < EXAMPLES && !a.examples.iter().any(|e| e.id == src.id) {
                    a.examples.push(Example { id: src.id.clone(), n: part.n, text: sentence.chars().take(EXAMPLE_CHARS).collect(), cloud_ok: src.cloud_ok });
                }
                if counted.insert(key) {
                    a.parts += 1;
                    for &k in &near_here {
                        *a.near.entry(k).or_default() += 1;
                    }
                }
            }
        }
    }
    let n = with_text.max(1) as f64;
    let mut per_file_counts: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut terms: Vec<Term> = acc
        .into_iter()
        .filter(|(k, a)| {
            let count: usize = a.per_file.values().sum();
            // a single word seen once in one file is noise; a phrase must repeat; a single word
            // nearly always written as a name (`Wang`, `Taiwan`) is a name, not a term
            let phrase = k.contains(' ');
            let inside = count - a.sentence_start.min(count);
            let name = !phrase && inside > 0 && a.titlecase * 10 >= inside * 9;
            !name && if phrase { count >= 3 } else { count >= 2 || a.per_file.len() >= 2 }
        })
        .map(|(key, a)| {
            per_file_counts.insert(key.clone(), a.per_file.iter().map(|(&f, &c)| (f, c)).collect());
            let files = a.per_file.len();
            let idf = ((n + 1.0) / (files as f64 + 0.5)).ln().max(0.1);
            let score = a.per_file.values().map(|&c| (1.0 + c as f64).ln()).sum::<f64>() * idf;
            let label = a.labels.into_iter().max_by(|x, y| x.1.cmp(&y.1).then_with(|| y.0.cmp(&x.0))).map(|x| x.0).unwrap_or_default();
            // the cards it is really next to: together in 2+ parts, and far more often than chance
            // (a card named on every page — `Accuracy` — is next to everything and says nothing)
            let lift = |k: usize, c: usize| c as f64 * parts_total as f64 / (card_parts.get(&k).copied().unwrap_or(1) as f64 * a.parts.max(1) as f64);
            let mut near: Vec<(usize, usize)> = a.near.into_iter().filter(|&(k, c)| c >= 2 && lift(k, c) >= NEAR_LIFT).collect();
            near.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
            near.truncate(NEAR);
            let near_known = near.iter().filter(|(k, _)| known[*k].state == "known").count();
            let mut doms: BTreeMap<&str, usize> = BTreeMap::new();
            for (k, c) in &near {
                for d in &known[*k].domains {
                    *doms.entry(d.as_str()).or_default() += c;
                }
            }
            let mut domains: Vec<(&str, usize)> = doms.into_iter().collect();
            domains.sort_by(|x, y| y.1.cmp(&x.1));
            Term {
                key,
                label,
                files,
                count: a.per_file.values().sum(),
                score: (score * 100.0).round() / 100.0,
                cloud_ok: a.any_cloud,
                mastered: a.mastered,
                examples: a.examples,
                near: near.iter().map(|(k, _)| known[*k].title.clone()).collect(),
                near_known,
                domains: domains.into_iter().take(3).map(|(d, _)| d.to_string()).collect(),
                verdict: None,
                field: None,
            }
        })
        .collect();
    // a phrase that is the same word twice adds nothing
    terms.retain(|t| !t.key.split(' ').collect::<Vec<_>>().windows(2).any(|w| w[0] == w[1]));
    terms.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.key.cmp(&b.key)));
    // kept: the best overall, and each file's own best (by how often there × how rare overall)
    // every term of a file he knows all of stays (they become known cards)
    let mut keep: HashSet<usize> = (0..terms.len().min(MAX_TERMS)).chain(terms.iter().enumerate().filter(|(_, t)| t.mastered).map(|(i, _)| i)).collect();
    let mut by_file: HashMap<usize, Vec<(f64, usize)>> = HashMap::new();
    for (ti, t) in terms.iter().enumerate() {
        let idf = ((n + 1.0) / (t.files as f64 + 0.5)).ln().max(0.1);
        for &(fi, c) in per_file_counts.get(&t.key).into_iter().flatten() {
            by_file.entry(fi).or_default().push(((1.0 + c as f64).ln() * idf, ti));
        }
    }
    for mut v in by_file.into_values() {
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
        keep.extend(v.into_iter().take(PER_FILE).map(|x| x.1));
    }
    let terms: Vec<Term> = terms.into_iter().enumerate().filter(|(i, _)| keep.contains(i)).map(|x| x.1).collect();
    Pool { version: VERSION, built: chrono::Utc::now().to_rfc3339(), files: with_text, terms }
}

/// Keep the model's verdicts of an older pool for the terms that are still there.
pub fn carry_verdicts(new: &mut Pool, old: &Pool) {
    let by: HashMap<&str, &Term> = old.terms.iter().map(|t| (t.key.as_str(), t)).collect();
    for t in &mut new.terms {
        if let Some(o) = by.get(t.key.as_str()) {
            t.verdict.clone_from(&o.verdict);
            t.field.clone_from(&o.field);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fulltext::Part;

    fn save(dir: &Path, id: &str, parts: &[(u32, &str)]) {
        FullText { version: 1, unit: "page".into(), parts: parts.iter().map(|(n, t)| Part { n: *n, text: t.to_string() }).collect(), note: None }.save(dir, id).unwrap();
    }

    #[test]
    fn unknown_terms_with_their_sentences_and_the_cards_next_to_them() {
        let t = tempfile::tempdir().unwrap();
        save(t.path(), "notes", &[(30, "The Achilles tendon stores elastic energy during the gait cycle. Ground reaction force peaks at heel strike."), (31, "Plantar fascia and the Achilles tendon. Ground reaction force again. Ground reaction force once more.")]);
        save(t.path(), "paper", &[(2, "We measure the Achilles tendon with ultrasound. The data were collected in 2024.")]);
        let known = vec![
            Known { title: "Gait cycle".into(), domains: vec!["biomechanics".into()], state: "known".into(), ..Known::default() },
            Known { title: "Ultrasound".into(), aliases: vec!["sonography".into()], domains: vec!["imaging".into()], state: "learning".into() },
        ];
        let sources = vec![Source { id: "notes".into(), cloud_ok: false, mastered: false }, Source { id: "paper".into(), cloud_ok: true, mastered: false }, Source { id: "gone".into(), cloud_ok: true, mastered: false }];
        let pool = build(t.path(), &sources, &known, &HashSet::new(), &Filters::default());
        assert_eq!(pool.files, 2);
        let get = |k: &str| pool.terms.iter().find(|x| x.key == phrase_key(k));
        let achilles = get("achilles tendon").expect("a repeated phrase");
        assert_eq!((achilles.files, achilles.count, achilles.label.as_str()), (2, 3, "Achilles tendon"));
        assert!(achilles.cloud_ok, "one of its files is a paper");
        assert_eq!(achilles.examples.iter().map(|e| e.cloud_ok).collect::<Vec<_>>(), vec![false, true]);
        assert_eq!(achilles.examples.len(), 2);
        assert!(achilles.examples[0].text.starts_with("The Achilles tendon stores") && achilles.examples[0].n == 30);
        assert!(achilles.near.is_empty(), "together only once each: not near");
        assert!(get("ground reaction").is_none(), "a phrase of everyday words: left to the model pass");
        assert!(get("ultrasound").is_none() && get("gait cycle").is_none(), "already cards");
        assert!(get("during").is_none() && get("energy").is_none() && get("2024").is_none(), "everyday words and numbers");
        assert!(get("plantar fascia").is_none(), "a phrase seen once");
        // ignored keys stay out
        let ig: HashSet<String> = ["achilles tendon".to_string()].into();
        let ig: HashSet<String> = ig.iter().map(|k| phrase_key(k)).collect();
        assert!(build(t.path(), &sources, &known, &ig, &Filters::default()).terms.iter().all(|x| x.key != phrase_key("achilles tendon")));
    }

    #[test]
    fn names_broken_words_and_citations_stay_out_and_near_cards_need_more_than_chance() {
        let t = tempfile::tempdir().unwrap();
        let mut parts: Vec<(u32, String)> = Vec::new();
        for n in 1..=20 {
            // Accuracy is on every page; Gait cycle only where tendinopathy is
            let extra = if n <= 4 { " Tendinopathy changes the gait cycle." } else { "" };
            parts.push((n, format!("As Wang et al reported, accuracy at https doi org. The signicant dierence was clear.{extra}")));
        }
        let parts: Vec<(u32, &str)> = parts.iter().map(|(n, s)| (*n, s.as_str())).collect();
        save(t.path(), "f", &parts);
        let known = vec![Known { title: "Accuracy".into(), state: "known".into(), ..Known::default() }, Known { title: "Gait cycle".into(), domains: vec!["biomechanics".into()], state: "known".into(), ..Known::default() }];
        let pool = build(t.path(), &[Source { id: "f".into(), cloud_ok: false, mastered: false }], &known, &HashSet::new(), &Filters::default());
        let keys: Vec<&str> = pool.terms.iter().map(|x| x.key.as_str()).collect();
        for bad in ["wang", "https", "signicant", "dierence"] {
            assert!(!keys.contains(&bad), "{bad} in {keys:?}");
        }
        let tend = pool.terms.iter().find(|x| x.key == word_key("Tendinopathy")).expect("the term");
        assert_eq!((tend.near.clone(), tend.near_known, tend.domains.clone()), (vec!["Gait cycle".to_string()], 1, vec!["biomechanics".to_string()]));
    }

    #[test]
    fn words_keep_hyphens_and_acronyms() {
        let w: Vec<&str> = words("Drug-induced sleep endoscopy (DISE), T2-weighted 肌腱 MRIs.").into_iter().map(|x| x.1).collect();
        assert_eq!(w, vec!["Drug-induced", "sleep", "endoscopy", "DISE", "T2-weighted", "MRIs"]);
        let f = Filters::default();
        let lost = f.ligature_losses();
        assert!(lost.contains("signicant") && lost.contains("dierence") && !lost.contains("significant"));
        assert!(term_word("DISE", &f, &lost) && term_word("endoscopy", &f, &lost) && !term_word("sleep", &f, &lost) && !term_word("T2-weighted", &f, &lost) && !term_word("signicant", &f, &lost));
    }

    #[test]
    fn a_file_keeps_its_own_terms_whatever_the_overall_ranking() {
        let t = tempfile::tempdir().unwrap();
        // 2,100 other files each with two distinct frequent terms crowd the overall ranking
        let mut sources = Vec::new();
        for i in 0..2100 {
            // letters only (a word with digits is not a term): i written in base 26
            let a: String = format!("zq{}", (0..3).map(|d| (b'a' + (i / 26usize.pow(d)) as u8 % 26) as char).collect::<String>());
            save(t.path(), &format!("f{i}"), &[(1, &format!("{a}thing {a}thing {a}thing. {a}other {a}other {a}other."))]);
            sources.push(Source { id: format!("f{i}"), cloud_ok: false, mastered: false });
        }
        save(t.path(), "notes", &[(30, "The talocrural joint. The talocrural joint again.")]);
        sources.push(Source { id: "notes".into(), cloud_ok: false, mastered: false });
        let pool = build(t.path(), &sources, &[], &HashSet::new(), &Filters::default());
        assert!(pool.terms.len() > MAX_TERMS);
        assert!(pool.terms.iter().any(|x| x.key == word_key("talocrural")), "the notes keep their term");
    }

    #[test]
    fn verdicts_survive_a_rebuild() {
        let term = |k: &str, v: Option<&str>| Term { key: k.into(), label: k.into(), files: 1, count: 2, score: 1.0, cloud_ok: false, examples: vec![], near: vec![], near_known: 0, domains: vec![], mastered: false, verdict: v.map(str::to_string), field: None };
        let old = Pool { terms: vec![term("a", Some("term")), term("gone", Some("junk"))], ..Pool::default() };
        let mut new = Pool { terms: vec![term("a", None), term("b", None)], ..Pool::default() };
        carry_verdicts(&mut new, &old);
        assert_eq!((new.terms[0].verdict.as_deref(), new.terms[1].verdict.as_deref()), (Some("term"), None));
    }
}
