//! Which wordings of the author's patterns his drafts have already used (`usage.json` in the
//! workspace): a wording that was used is offered far less the next time, a pattern used in this
//! paper is pushed down the list, and one with no unused wording left goes out marked "reword".
//!
//! A wording is named by its key: the pattern id for the author's own template (`P012`), the
//! pattern id and the 說法 id for a confirmed variant (`P012#V001`). The record is kept per paper
//! and paragraph and replaced when the paragraph is drafted again, so a redraft never counts twice.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::profile::Phrase;
use crate::variants::Variant;

pub const FILE: &str = "usage.json";
/// One use in this paper weighs as much as this many uses in the author's other papers.
pub const HERE: u32 = 4;
/// What one use in this paper takes off a pattern's retrieval score (a full word overlap is 10).
pub const HERE_PENALTY: f32 = 4.0;
/// The same for one use in another paper, and the most that other papers can take off.
pub const ELSEWHERE_PENALTY: f32 = 0.5;
pub const ELSEWHERE_CAP: f32 = 3.0;

/// paper → paragraph → the wording keys its sentences used (one entry per sentence).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Usage {
    pub papers: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// What is already used, seen from one paper: counts per wording key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    pub here: BTreeMap<String, u32>,
    pub elsewhere: BTreeMap<String, u32>,
    /// What the paragraph being drafted had the last time: its patterns are not pushed down
    /// (they fit it), but it does not get the same wording again.
    pub again: BTreeMap<String, u32>,
}

/// The pattern id of a wording key.
pub fn pattern_of(key: &str) -> &str {
    key.split('#').next().unwrap_or(key)
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

    /// The counts as paragraph `para` of `paper` sees them.
    pub fn seen(&self, paper: &str, para: &str) -> Seen {
        let mut s = Seen::default();
        for (name, paras) in &self.papers {
            for (id, keys) in paras {
                let into = if name != paper {
                    &mut s.elsewhere
                } else if id == para {
                    &mut s.again
                } else {
                    &mut s.here
                };
                for k in keys {
                    *into.entry(k.clone()).or_default() += 1;
                }
            }
        }
        s
    }
}

impl Seen {
    /// How used one wording is: uses in this paper count [`HERE`] times.
    pub fn weight(&self, key: &str) -> u32 {
        let n = |m: &BTreeMap<String, u32>| m.get(key).copied().unwrap_or(0);
        (n(&self.here) + n(&self.again)) * HERE + n(&self.elsewhere)
    }

    /// Uses of any wording of `pattern`: (in this paper, in the other papers).
    pub fn pattern(&self, pattern: &str) -> (u32, u32) {
        let sum = |m: &BTreeMap<String, u32>| m.iter().filter(|(k, _)| pattern_of(k) == pattern).map(|(_, n)| *n).sum();
        (sum(&self.here), sum(&self.elsewhere))
    }

    /// What the uses of `pattern` take off its retrieval score.
    pub fn penalty(&self, pattern: &str) -> f32 {
        let (here, elsewhere) = self.pattern(pattern);
        here as f32 * HERE_PENALTY + (elsewhere as f32 * ELSEWHERE_PENALTY).min(ELSEWHERE_CAP)
    }
}

/// The wording of a pattern chosen for one draft.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice<'a> {
    pub key: String,
    pub wording: &'a str,
    /// Every wording of the pattern has been used: the frame goes out to be reworded.
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
    fn a_used_wording_is_not_chosen_while_an_unused_one_is_left() {
        let (p, vs) = bank();
        let mut u = Usage::default();
        u.record("paper-a", "Methods/p1", vec!["P1".into()]);
        for seed in 0..20 {
            let c = choose(&p, &vs, &u.seen("paper-b", "x"), seed);
            assert_ne!(c.key, "P1", "seed {seed}");
            assert!(!c.worn);
        }
        u.record("paper-a", "Methods/p2", vec!["P1#V1".into()]);
        for seed in 0..20 {
            assert_eq!(choose(&p, &vs, &u.seen("paper-b", "x"), seed).key, "P1#V2");
        }
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
        // its own pattern still fits it: not pushed down for the redraft
        assert_eq!(again.penalty("P1"), 0.0);
        // and the redraft replaces the record: one use, not two
        u.record("paper", "Methods/p1", vec![second.clone()]);
        assert_eq!(u.seen("paper", "Methods/p2").pattern("P1"), (1, 0));
    }

    #[test]
    fn a_pattern_with_every_wording_used_is_worn_and_the_least_used_goes_out() {
        let (p, vs) = bank();
        let mut u = Usage::default();
        u.record("old", "a", vec!["P1".into(), "P1#V1".into(), "P1#V2".into()]);
        u.record("this", "b", vec!["P1".into()]);
        let c = choose(&p, &vs, &u.seen("this", "x"), 0);
        assert!(c.worn);
        assert_ne!(c.key, "P1", "the one used in this paper is the most used");
    }

    #[test]
    fn this_paper_weighs_more_than_the_others() {
        let mut u = Usage::default();
        u.record("this", "a", vec!["P1".into()]);
        u.record("old", "a", vec!["P2".into(), "P2#V9".into()]);
        let s = u.seen("this", "x");
        assert_eq!(s.pattern("P1"), (1, 0));
        assert_eq!(s.pattern("P2"), (0, 2));
        assert!(s.penalty("P1") > s.penalty("P2"));
        assert_eq!(s.penalty("P3"), 0.0);
        // many old uses never outweigh the cap
        u.record("old", "b", vec!["P2".into(); 40]);
        assert_eq!(u.seen("this", "x").penalty("P2"), ELSEWHERE_CAP);
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
