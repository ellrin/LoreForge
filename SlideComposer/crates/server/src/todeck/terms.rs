//! `terms` stage: the paper's domain terms against the author's KnowledgeBase (熟／不熟,
//! docs/paper-deck.md 「不熟」標示). The work is `crate::kbterms` (its own owner); this is the
//! stage's call into it.
//!
//! ```ignore
//! pub fn terms(input: &TermsInput) -> kbterms::TermsResult
//! ```
//!
//! - Runs on a blocking thread (`spawn_blocking`): reads the KnowledgeBase from disk.
//! - The stage stores the result as `terms.json` and shows 「名詞 n → 不熟 m」 (`counts.terms`
//!   = `total`, `counts.unknown` = `unfamiliar`). The review window lists the unfamiliar ones;
//!   我懂 there marks them `understood` in this checkpoint before the slides are made, and the
//!   `slides` stage marks the rest (`kbterms::mark_slide`) and saves them next to the deck.

pub use crate::kbterms::TermsResult;

use super::source::Material;

pub struct TermsInput<'a> {
    pub material: &'a Material,
    pub notation: &'a lore_notation::Notation,
    pub home: &'a lore_home::Home,
}

pub fn terms(input: &TermsInput) -> TermsResult {
    crate::kbterms::terms_for(input.home, &input.material.sections)
}
