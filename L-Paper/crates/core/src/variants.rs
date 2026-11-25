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
pub fn load(_root: &Path, _phrases: &[Phrase], _issues: &mut Vec<Issue>) -> Vec<Variant> {
    todo!("variants package")
}

/// Every wording of `phrase`: its own template first, then its `variant` rows in file order.
pub fn wordings<'a>(_phrase: &'a Phrase, _variants: &'a [Variant]) -> Vec<&'a str> {
    todo!("variants package")
}

/// The wording for this `turn`: `wordings()[turn % len]`, so turn 0 is the author's own template
/// and successive turns walk through the variants.
pub fn pick<'a>(_phrase: &'a Phrase, _variants: &'a [Variant], _turn: u64) -> &'a str {
    todo!("variants package")
}

/// The swaps of `phrase` that apply to `wording` (the slot word occurs in it as a whole word,
/// case-insensitive), grouped by slot in file order.
pub fn swaps(_phrase: &Phrase, _variants: &[Variant], _wording: &str) -> Vec<Swap> {
    todo!("variants package")
}

/// Runs of at least `min_words` consecutive words that a drafted sentence shares with the
/// `example` of a phrase (words compared lower-case, punctuation ignored; `{slots}` do not
/// exist in examples). The longest run per (sentence, phrase), sentences in order.
pub fn copied(_sentences: &[String], _phrases: &[Phrase], _min_words: usize) -> Vec<Copied> {
    todo!("variants package")
}
