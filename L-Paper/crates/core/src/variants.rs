//! 說法 (`profile/variants.csv`): other ways to say one of the author's sentence patterns and
//! words that may be swapped in it, each confirmed by him in L-Brain (`lore_know::review`), each
//! with one line on when it fits. Drafting rotates through them so a routine sentence is not
//! worded the same way every time, and never sends the published sentence a pattern was cut from.
//!
//! ```csv
//! id,pattern,kind,text,slot,note
//! V001,P012,variant,"{approval} was obtained from {board}.",,shorter; when the board is the news
//! V002,P012,swap,granted,approved,more formal
//! ```
//!
//! `pattern` is a `phrases.csv` id. `kind = variant`: `text` is a whole template with the same
//! `{slots}` as the pattern. `kind = swap`: `text` may replace the word `slot` of the template.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::profile::{Issue, Phrase};

pub const FILE: &str = "variants.csv";
/// A run of this many words or more copied from one of the author's published sentences is flagged.
pub const COPIED_WORDS: usize = 7;
/// The same for a sentence of this paper's own earlier paragraphs (a long term of the paper is
/// repeated on purpose, so the run must be longer before it counts as the same sentence again).
pub const REPEATED_WORDS: usize = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Variant {
    pub id: String,
    pub pattern: String,
    /// `variant` or `swap`.
    pub kind: String,
    pub text: String,
    pub slot: String,
    pub note: String,
}

/// A swap offered for one word of a template.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Swap {
    pub slot: String,
    /// (word, when it fits), in file order.
    pub options: Vec<(String, String)>,
}

/// Words of a drafted sentence that repeat one of the author's published sentences.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Copied {
    /// Index of the drafted sentence.
    pub sentence: usize,
    /// The copied words as they stand in the draft.
    pub words: String,
    /// The `phrases.csv` id whose `example` they come from.
    pub from: String,
}

/// `variants.csv` of a profile folder. A missing file is no variants and no issue. A row without
/// `pattern` or `text`, with an unknown `kind`, a `swap` without `slot`, or a `pattern` that is
/// not in `phrases` is skipped with a warning.
pub fn load(root: &Path, phrases: &[Phrase], issues: &mut Vec<Issue>) -> Vec<Variant> {
    let path = root.join(FILE);
    if !path.exists() {
        return Vec::new();
    }
    let warn = |issues: &mut Vec<Issue>, line: u64, message: &str| {
        issues.push(Issue { file: FILE.to_string(), line, column: None, message: message.to_string(), error: false });
    };
    let mut reader = match csv::ReaderBuilder::new().trim(csv::Trim::All).from_path(&path) {
        Ok(r) => r,
        Err(e) => {
            warn(issues, 0, &e.to_string());
            return Vec::new();
        }
    };
    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            warn(issues, 0, &e.to_string());
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for rec in reader.records() {
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                let line = e.position().map(|p| p.line()).unwrap_or(0);
                warn(issues, line, &format!("row skipped: {e}"));
                continue;
            }
        };
        let line = rec.position().map(|p| p.line()).unwrap_or(0);
        let v: Variant = match rec.deserialize(Some(&headers)) {
            Ok(v) => v,
            Err(e) => {
                warn(issues, line, &format!("row skipped: {e}"));
                continue;
            }
        };
        let problem = if v.pattern.is_empty() {
            Some("row skipped: no pattern".to_string())
        } else if v.text.is_empty() {
            Some("row skipped: no text".to_string())
        } else if v.kind != "variant" && v.kind != "swap" {
            Some(format!("row skipped: unknown kind \"{}\"", v.kind))
        } else if v.kind == "swap" && v.slot.is_empty() {
            Some("row skipped: swap without slot".to_string())
        } else if !phrases.iter().any(|p| p.id == v.pattern) {
            Some(format!("row skipped: pattern {} is not in phrases.csv", v.pattern))
        } else {
            None
        };
        match problem {
            Some(m) => warn(issues, line, &m),
            None => out.push(v),
        }
    }
    out
}

/// Every wording of `phrase`: its own template first, then its `variant` rows in file order.
pub fn wordings<'a>(phrase: &'a Phrase, variants: &'a [Variant]) -> Vec<&'a str> {
    let mut out = vec![phrase.template.as_str()];
    out.extend(variants.iter().filter(|v| v.kind == "variant" && v.pattern == phrase.id).map(|v| v.text.as_str()));
    out
}

/// The wording for this `turn`: `wordings()[turn % len]`, so turn 0 is the author's own template
/// and successive turns walk through the variants.
pub fn pick<'a>(phrase: &'a Phrase, variants: &'a [Variant], turn: u64) -> &'a str {
    let w = wordings(phrase, variants);
    w[(turn % w.len() as u64) as usize]
}

fn words_of(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The swaps of `phrase` that apply to `wording` (the slot word occurs in it as a whole word,
/// case-insensitive), grouped by slot in file order.
pub fn swaps(phrase: &Phrase, variants: &[Variant], wording: &str) -> Vec<Swap> {
    // the fixed words only: a `{slot}` name is not a word of the sentence
    let mut fixed = String::new();
    let mut depth = 0usize;
    for c in wording.chars() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                fixed.push(' ');
            }
            _ if depth == 0 => fixed.push(c),
            _ => {}
        }
    }
    let have = words_of(&fixed);
    let mut out: Vec<Swap> = Vec::new();
    for v in variants.iter().filter(|v| v.kind == "swap" && v.pattern == phrase.id) {
        let want = words_of(&v.slot);
        if want.is_empty() || !have.windows(want.len()).any(|w| w == want.as_slice()) {
            continue;
        }
        let opt = (v.text.clone(), v.note.clone());
        match out.iter_mut().find(|s| s.slot.to_lowercase() == v.slot.to_lowercase()) {
            Some(s) => s.options.push(opt),
            None => out.push(Swap { slot: v.slot.clone(), options: vec![opt] }),
        }
    }
    out
}

/// Runs of at least `min_words` consecutive words that a drafted sentence shares with the
/// `example` of a phrase (words compared lower-case, punctuation ignored; `{slots}` do not
/// exist in examples). The longest run per (sentence, phrase), sentences in order.
pub fn copied(sentences: &[String], phrases: &[Phrase], min_words: usize) -> Vec<Copied> {
    let sources: Vec<(&str, &str)> = phrases.iter().map(|p| (p.id.as_str(), p.example.as_str())).collect();
    copied_from(sentences, &sources, min_words)
}

/// [`copied`] against any texts: `sources` = (name, text), `from` of a finding is the name.
pub fn copied_from(sentences: &[String], sources: &[(&str, &str)], min_words: usize) -> Vec<Copied> {
    let min_words = min_words.max(1);
    let sources: Vec<(&str, Vec<String>)> = sources.iter().map(|(n, t)| (*n, words_of(t))).filter(|(_, w)| w.len() >= min_words).collect();
    let mut out = Vec::new();
    for (si, sentence) in sentences.iter().enumerate() {
        let shown: Vec<&str> = sentence.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        let low: Vec<String> = shown.iter().map(|w| w.to_lowercase()).collect();
        for (name, ex) in &sources {
            // longest common run of words
            let (mut best, mut end) = (0usize, 0usize);
            let mut prev = vec![0usize; ex.len() + 1];
            for i in 1..=low.len() {
                let mut cur = vec![0usize; ex.len() + 1];
                for j in 1..=ex.len() {
                    if low[i - 1] == ex[j - 1] {
                        cur[j] = prev[j - 1] + 1;
                        if cur[j] > best {
                            best = cur[j];
                            end = i;
                        }
                    }
                }
                prev = cur;
            }
            if best >= min_words {
                out.push(Copied { sentence: si, words: shown[end - best..end].join(" "), from: name.to_string() });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrase(id: &str, template: &str, example: &str) -> Phrase {
        Phrase {
            id: id.into(),
            section: "Any".into(),
            function: "f".into(),
            position: "body".into(),
            template: template.into(),
            example: example.into(),
            tags: vec![],
            priority: 3,
        }
    }

    fn var(id: &str, pattern: &str, kind: &str, text: &str, slot: &str, note: &str) -> Variant {
        Variant { id: id.into(), pattern: pattern.into(), kind: kind.into(), text: text.into(), slot: slot.into(), note: note.into() }
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pc-variants-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_missing_file_is_no_variants_and_no_issue() {
        let d = dir("missing");
        let mut issues = vec![];
        assert!(load(&d, &[phrase("P1", "t", "")], &mut issues).is_empty());
        assert!(issues.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rows_are_read_in_file_order() {
        let d = dir("order");
        std::fs::write(
            d.join(FILE),
            "id,pattern,kind,text,slot,note\nV1,P1,variant,\"{a} was seen by {b}.\",,short\nV2,P1,swap,granted,approved,formal\nV3,P1,variant,{b} saw {a}.,,\n",
        )
        .unwrap();
        let mut issues = vec![];
        let v = load(&d, &[phrase("P1", "t", "")], &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(v.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["V1", "V2", "V3"]);
        assert_eq!(v[0].text, "{a} was seen by {b}.");
        assert_eq!(v[1].slot, "approved");
        assert_eq!(v[0].note, "short");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_rows_are_skipped_with_a_warning() {
        let d = dir("bad");
        std::fs::write(
            d.join(FILE),
            "id,pattern,kind,text,slot,note\nA,,variant,x y,,\nB,P1,variant,,,\nC,P1,other,x,,\nD,P1,swap,x,,\nE,P9,variant,x,,\nF,P1,variant,ok text,,\n",
        )
        .unwrap();
        let mut issues = vec![];
        let v = load(&d, &[phrase("P1", "t", "")], &mut issues);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "F");
        assert_eq!(issues.len(), 5);
        assert!(issues.iter().all(|i| i.file == FILE && !i.error));
        let lines: Vec<u64> = issues.iter().map(|i| i.line).collect();
        assert_eq!(lines, [2, 3, 4, 5, 6]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_own_template_comes_first_then_the_variants() {
        let p = phrase("P1", "own {a}.", "");
        let vs = vec![
            var("V1", "P1", "variant", "first {a}.", "", ""),
            var("V2", "P1", "swap", "w", "s", ""),
            var("V3", "P2", "variant", "other", "", ""),
            var("V4", "P1", "variant", "second {a}.", "", ""),
        ];
        assert_eq!(wordings(&p, &vs), ["own {a}.", "first {a}.", "second {a}."]);
    }

    #[test]
    fn pick_walks_through_the_wordings_by_turn() {
        let p = phrase("P1", "own", "");
        let vs = vec![var("V1", "P1", "variant", "one", "", ""), var("V2", "P1", "variant", "two", "", "")];
        let got: Vec<&str> = (0..5).map(|t| pick(&p, &vs, t)).collect();
        assert_eq!(got, ["own", "one", "two", "own", "one"]);
        let q = phrase("P9", "alone", "");
        assert!((0..4).all(|t| pick(&q, &vs, t) == "alone"));
    }

    #[test]
    fn swaps_are_grouped_by_slot_and_only_when_the_word_is_in_the_wording() {
        let p = phrase("P1", "The {sample} was approved by {board}.", "");
        let vs = vec![
            var("V1", "P1", "swap", "granted", "approved", "formal"),
            var("V2", "P1", "swap", "cleared", "Approved", ""),
            var("V3", "P1", "swap", "by", "approve", "not a whole word"),
            var("V4", "P1", "swap", "other", "rejected", "not in wording"),
            var("V5", "P1", "swap", "committee", "board", "when a group"),
            var("V6", "P2", "swap", "x", "approved", ""),
        ];
        let got = swaps(&p, &vs, "The {sample} was approved by {board}.");
        // "board" is only the name of a {slot} here, not a word of the sentence
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].slot, "approved");
        assert_eq!(got[0].options, vec![("granted".to_string(), "formal".to_string()), ("cleared".to_string(), String::new())]);
        assert!(swaps(&p, &vs, "{board} gave consent.").is_empty());
        assert_eq!(swaps(&p, &vs, "The board approved it.").len(), 2);
        assert!(swaps(&p, &vs, "Nothing here.").is_empty());
    }

    fn ex() -> Vec<Phrase> {
        vec![phrase("P1", "t", "The committee reviewed every sample and approved the protocol before work began.")]
    }

    #[test]
    fn a_long_copied_run_is_found_with_its_phrase() {
        let s = vec!["Short one.".to_string(), "Later, the Committee reviewed every sample and approved the protocol, as planned.".to_string()];
        let c = copied(&s, &ex(), COPIED_WORDS);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].sentence, c[0].from.as_str()), (1, "P1"));
        assert_eq!(c[0].words, "the Committee reviewed every sample and approved the protocol");
    }

    #[test]
    fn a_short_shared_run_is_not_flagged() {
        let s = vec!["Yes, the committee reviewed every sample and then stopped.".to_string()];
        // shared: the committee reviewed every sample and = 6 words
        assert!(copied(&s, &ex(), COPIED_WORDS).is_empty());
        assert_eq!(copied(&s, &ex(), 6).len(), 1);
    }

    #[test]
    fn punctuation_and_case_do_not_hide_a_copy() {
        let s = vec!["THE committee, reviewed; every sample (and) approved -- the protocol!".to_string()];
        assert_eq!(copied(&s, &ex(), COPIED_WORDS).len(), 1);
    }

    #[test]
    fn a_phrase_without_example_is_ignored() {
        let s = vec!["The committee reviewed every sample and approved the protocol.".to_string()];
        assert!(copied(&s, &[phrase("P2", "t", "")], 1).is_empty());
    }
}
