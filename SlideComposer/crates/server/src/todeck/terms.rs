//! `terms` stage: the paper's domain terms compared with the author's knowledge base
//! (熟／不熟, docs/paper-deck.md 「不熟」標示). **Owned by the unknown-terms agent**; this
//! file is a pass-through stub (no terms) until it is filled in.
//!
//! Contract (the runner calls exactly this; keep the signature):
//!
//! ```ignore
//! pub fn terms(input: &TermsInput) -> TermsOutput
//! ```
//!
//! - Runs on a blocking thread (`spawn_blocking`): file reads are fine, no async, no model.
//! - `input.material`: the paper's text (title, abstract, sections, captions…);
//!   `input.notation`: the entries the deck will register (symbols, abbreviations);
//!   `input.home`: lore-home, for the knowledge base (read-only).
//! - Return every domain term found; `unknown = true` for the ones to mark (not in the
//!   knowledge base, or its card is 「要學」). `n` = occurrences in the paper. Common words
//!   never appear.
//! - The stage stores the output as `terms.json` and shows 「名詞 n → 不熟 m」
//!   (`counts.terms`, `counts.unknown`). The review window lists the unknown ones; the
//!   author's 「我懂」 and 「記得」 come back in the review output (`known`, `remember`).

use serde::{Deserialize, Serialize};

use super::source::Material;

pub struct TermsInput<'a> {
    pub material: &'a Material,
    pub notation: &'a lore_notation::Notation,
    pub home: &'a lore_home::Home,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Term {
    pub term: String,
    pub unknown: bool,
    /// Occurrences in the paper.
    pub n: u32,
    /// `abbr` · `phrase` · `alias` (a knowledge card's alias).
    pub kind: String,
    /// The knowledge card's id, when there is one.
    pub card: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TermsOutput {
    pub terms: Vec<Term>,
}

/// Stub: no terms.
pub fn terms(_input: &TermsInput) -> TermsOutput {
    TermsOutput::default()
}
