//! `profile` stage: a short style summary of the author's own decks (docs/paper-deck.md
//! 「內容順序與風格」), made by [`crate::deckstyle`].
//!
//! Contract (the runner calls exactly this; keep the signature):
//!
//! ```ignore
//! pub fn profile(input: &ProfileInput) -> StyleProfile
//! ```
//!
//! - Runs on a blocking thread (`spawn_blocking`): file reads are fine, no async, no model.
//! - Reads the author's decks read-only through lore-home (`input.home`); never sends a deck
//!   itself anywhere. Footers, confidential marks, logos and names are left out. Cached in
//!   `input.ws.root/cache/deck-style/` (wipeable tier), keyed by the decks' mtimes.
//! - `summary` is pasted into the `outline` prompt as is (when not empty). `detail` is the
//!   whole profile, for `polish`.
//! - The stage stores the output as `profile.json`; it has no counts.

use serde::{Deserialize, Serialize};

use crate::deckstyle;

pub struct ProfileInput<'a> {
    pub home: &'a lore_home::Home,
    pub ws: &'a sc_core::Workspace,
    /// The base (style folder id) the deck is made with.
    pub style: &'a str,
    /// `zh` or `en`.
    pub lang: &'a str,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleProfile {
    /// For the outline prompt: slides per kind, title length and wording, bullets per slide,
    /// the tone of Take home, captions and `Ref:` habits.
    pub summary: String,
    /// Most bullets on one slide (polish rule 2).
    pub max_bullets: Option<u32>,
    /// `Fig.` or `Figure` (polish rule 6).
    pub figure_word: Option<String>,
    /// The whole profile (`deckstyle::Profile`), none without the author's decks.
    pub detail: Option<deckstyle::Profile>,
}

impl From<deckstyle::Profile> for StyleProfile {
    fn from(p: deckstyle::Profile) -> Self {
        StyleProfile {
            summary: p.prompt_text(),
            max_bullets: Some(p.bullet_limit("") as u32),
            figure_word: (!p.captions.fig_form.is_empty()).then(|| p.captions.fig_form.clone()),
            detail: Some(p),
        }
    }
}

/// The author's style, or an empty profile when there are no decks of theirs to learn from.
pub fn profile(input: &ProfileInput) -> StyleProfile {
    deckstyle::for_home(input.home, &input.ws.root.join("cache")).map(StyleProfile::from).unwrap_or_default()
}
