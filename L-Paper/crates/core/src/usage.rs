//! Which of the author's sentences his draft has already used (`usage.json` in the workspace).
//! His sentences are filed by section (Introduction, Method …), so the question is only whether
//! the section being written has said this sentence already: a wording used there is not
//! offered again while another is left, and a sentence whose every wording is used there falls
//! down the list and goes out marked "reword". The pattern itself is not held against him, only
//! the sentence: the same frame in another wording is a different sentence.
//!
//! A wording is named by its key: the pattern id for the author's own template (`P012`), the
//! pattern id and the 說法 id for a confirmed variant (`P012#V001`). The record is kept per paper
//! and paragraph (`Methods/p3`) and replaced when the paragraph is drafted again.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::profile::Phrase;
use crate::variants::Variant;

pub const FILE: &str = "usage.json";
/// What each time a sentence was already said in this section, in every wording it has, takes
/// off its retrieval score (a full word overlap is 10).
pub const WORN_PENALTY: f32 = 3.0;

/// paper → paragraph (`section/id`) → the wording keys its sentences used (one per sentence).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Usage {
    pub papers: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// What one paragraph sees as already used: counts per wording key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    /// In the other paragraphs of its section.
    pub here: BTreeMap<String, u32>,
    /// What the paragraph itself had the last time it was drafted: its sentences are not pushed
    /// down the list (they fit it), but it does not get the same wording again.
    pub again: BTreeMap<String, u32>,
}

fn section_of(para: &str) -> &str {
    para.split('/').next().unwrap_or(para)
}

impl Usage {
    /// The record in `ws`. A missing or unreadable file is an empty record.
    pub fn load(ws: &Path) -> Usage {
        std::fs::read_to_string(ws.join(FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, ws: &Path) -> std::io::Result<()> {
        let tmp = ws.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, ws.join(FILE))
    }

    /// The wordings `para` of `paper` used, replacing what it had. No keys: the entry is removed.
    pub fn record(&mut self, paper: &str, para: &str, keys: Vec<String>) {
        if keys.is_empty() {
            if let Some(p) = self.papers.get_mut(paper) {
                p.remove(para);
                if p.is_empty() {
                    self.papers.remove(paper);
                }
            }
        } else {
            self.papers.entry(paper.to_string()).or_default().insert(para.to_string(), keys);
        }
    }

    /// Drop the records of `section` whose paragraph is not in `ids` (removed from the outline:
    /// what it said no longer stands in the section).
    pub fn keep(&mut self, paper: &str, section: &str, ids: &[&str]) {
        if let Some(p) = self.papers.get_mut(paper) {
            p.retain(|para, _| section_of(para) != section || para.split_once('/').is_some_and(|(_, id)| ids.contains(&id)));
            if p.is_empty() {
                self.papers.remove(paper);
            }
        }
    }

    /// The counts as paragraph `para` (`section/id`) of `paper` sees them: its own section only.
    pub fn seen(&self, paper: &str, para: &str) -> Seen {
        let mut s = Seen::default();
        for (id, keys) in self.papers.get(paper).into_iter().flatten().filter(|(id, _)| section_of(id) == section_of(para)) {
            let into = if id == para { &mut s.again } else { &mut s.here };
            for k in keys {
                *into.entry(k.clone()).or_default() += 1;
            }
        }
        s
    }
}

impl Seen {
    /// How often one wording was already used.
    pub fn weight(&self, key: &str) -> u32 {
        self.here.get(key).copied().unwrap_or(0) + self.again.get(key).copied().unwrap_or(0)
    }

    /// What is taken off the retrieval score of `phrase`: nothing while one of its wordings is
    /// unused in the section, [`WORN_PENALTY`] for each time all of them have been said there.
    pub fn penalty(&self, phrase: &Phrase, variants: &[Variant]) -> f32 {
        let said = keyed(phrase, variants).iter().map(|(k, _)| self.here.get(k).copied().unwrap_or(0)).min().unwrap_or(0);
        said as f32 * WORN_PENALTY
    }
}

/// The wording of a pattern chosen for one draft.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice<'a> {
    pub key: String,
    pub wording: &'a str,
    /// Every wording of it has been said in this section: the frame goes out to be reworded.
    pub worn: bool,
}

/// Every wording of `phrase` with its key: his own template, then the confirmed variants.
pub fn keyed<'a>(phrase: &'a Phrase, variants: &'a [Variant]) -> Vec<(String, &'a str)> {
    let mut out = vec![(phrase.id.clone(), phrase.template.as_str())];
    for (n, v) in variants.iter().filter(|v| v.kind == "variant" && v.pattern == phrase.id).enumerate() {
        let name = if v.id.is_empty() { format!("{}", n + 1) } else { v.id.clone() };
        out.push((format!("{}#{name}", phrase.id), v.text.as_str()));
    }
    out
}

/// The least used wording of `phrase`. Among equally used ones `seed` decides, so the first
/// wording is not the one that always comes first.
pub fn choose<'a>(phrase: &'a Phrase, variants: &'a [Variant], seen: &Seen, seed: u64) -> Choice<'a> {
    let all = keyed(phrase, variants);
    let least = all.iter().map(|(k, _)| seen.weight(k)).min().unwrap_or(0);
    let ties: Vec<&(String, &str)> = all.iter().filter(|(k, _)| seen.weight(k) == least).collect();
    let (key, wording) = ties[(seed % ties.len() as u64) as usize];
    Choice { key: key.clone(), wording, worn: least > 0 }
}

/// A seed from names (a paper, a paragraph): the same names give the same seed on every run.
pub fn seed(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0xff)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    // the low bits of FNV are weak for a small modulus: mix once more
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^ (h >> 33)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrase(id: &str, template: &str) -> Phrase {
        Phrase { id: id.into(), section: "Any".into(), function: "f".into(), position: "body".into(), template: template.into(), example: String::new(), tags: vec![], priority: 3 }
    }

    fn var(id: &str, pattern: &str, text: &str) -> Variant {
        Variant { id: id.into(), pattern: pattern.into(), kind: "variant".into(), text: text.into(), slot: String::new(), note: String::new() }
    }

    fn bank() -> (Phrase, Vec<Variant>) {
        (phrase("P1", "{a} was approved by {b}."), vec![var("V1", "P1", "{b} approved {a}."), var("V2", "P1", "Approval of {a} came from {b}."), var("V9", "P2", "other pattern")])
    }

    #[test]
    fn a_sentence_said_in_the_section_is_not_chosen_while_another_wording_is_left() {
        let (p, vs) = bank();
        let mut u = Usage::default();
        u.record("paper", "Methods/p1", vec!["P1".into()]);
        for seed in 0..20 {
            let c = choose(&p, &vs, &u.seen("paper", "Methods/p2"), seed);
            assert_ne!(c.key, "P1", "seed {seed}");
            assert!(!c.worn);
        }
        u.record("paper", "Methods/p2", vec!["P1#V1".into()]);
        for seed in 0..20 {
            assert_eq!(choose(&p, &vs, &u.seen("paper", "Methods/p3"), seed).key, "P1#V2");
        }
    }

    #[test]
    fn only_the_section_being_written_counts() {
        let (p, vs) = bank();
        let mut u = Usage::default();
        u.record("paper", "Methods/p1", vec!["P1".into(), "P1#V1".into(), "P1#V2".into()]);
        u.record("other paper", "Results/p1", vec!["P1".into()]);
        assert_eq!(u.seen("paper", "Results/p1"), Seen::default(), "another section of the paper");
        assert_eq!(u.seen("third paper", "Methods/p1"), Seen::default(), "another paper");
        assert_eq!(u.seen("paper", "Results/p1").penalty(&p, &vs), 0.0);
    }

    #[test]
    fn the_first_wording_is_not_always_first() {
        let (p, vs) = bank();
        let seen = Seen::default();
        let keys: std::collections::BTreeSet<String> = ["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"].iter().map(|para| choose(&p, &vs, &seen, seed(&["paper", para, "P1"])).key).collect();
        assert_eq!(keys.len(), 3, "every wording opens some paragraph: {keys:?}");
    }

    #[test]
    fn drafting_the_same_paragraph_again_gets_another_wording() {
        let (p, vs) = bank();
        let mut u = Usage::default();
        let s = seed(&["paper", "Methods/p1", "P1"]);
        let first = choose(&p, &vs, &u.seen("paper", "Methods/p1"), s).key;
        u.record("paper", "Methods/p1", vec![first.clone()]);
        let again = u.seen("paper", "Methods/p1");
        let second = choose(&p, &vs, &again, s).key;
        assert_ne!(first, second);
        // its own sentence still fits it: not pushed down for the redraft
        assert_eq!(again.penalty(&p, &vs), 0.0);
        // and the redraft replaces the record: one use, not two
        u.record("paper", "Methods/p1", vec![second.clone()]);
        assert_eq!(u.seen("paper", "Methods/p2").here.values().sum::<u32>(), 1);
    }

    #[test]
    fn a_sentence_said_in_every_wording_is_worn_and_falls_down_the_list() {
        let (p, vs) = bank();
        let mut u = Usage::default();
        u.record("paper", "Methods/a", vec!["P1".into(), "P1#V1".into()]);
        let s = u.seen("paper", "Methods/c");
        assert_eq!(s.penalty(&p, &vs), 0.0, "one wording is still unused: the frame is not held against him");
        u.record("paper", "Methods/b", vec!["P1#V2".into(), "P1".into()]);
        let s = u.seen("paper", "Methods/c");
        let c = choose(&p, &vs, &s, 0);
        assert!(c.worn);
        assert_ne!(c.key, "P1", "the one said twice is the last to come back");
        assert_eq!(s.penalty(&p, &vs), WORN_PENALTY);
        // a pattern with one wording is worn after one use
        let single = phrase("P5", "{a} is set to {b}.");
        u.record("paper", "Methods/b", vec!["P5".into()]);
        assert_eq!(u.seen("paper", "Methods/c").penalty(&single, &vs), WORN_PENALTY);
    }

    #[test]
    fn a_paragraph_taken_out_of_the_section_no_longer_counts() {
        let mut u = Usage::default();
        u.record("paper", "Methods/p1", vec!["P1".into()]);
        u.record("paper", "Methods/p2", vec!["P1#V1".into()]);
        u.record("paper", "Results/p2", vec!["P1".into()]);
        u.keep("paper", "Methods", &["p2", "p3"]);
        assert_eq!(u.papers["paper"].keys().collect::<Vec<_>>(), ["Methods/p2", "Results/p2"]);
    }

    #[test]
    fn the_record_is_kept_on_disk_and_an_empty_paragraph_leaves_it() {
        let d = std::env::temp_dir().join(format!("pc-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(Usage::load(&d), Usage::default());
        let mut u = Usage::default();
        u.record("paper", "Methods/p1", vec!["P1#V1".into()]);
        u.save(&d).unwrap();
        assert_eq!(Usage::load(&d), u);
        u.record("paper", "Methods/p1", vec![]);
        assert!(u.papers.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
