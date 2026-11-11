//! `profile` stage: a short style summary of the author's own decks (docs/paper-deck.md
//! 「內容順序與風格」). **Owned by the style-profile agent**; this file is a pass-through
//! stub (an empty profile) until it is filled in.
//!
//! Contract (the runner calls exactly this; keep the signature):
//!
//! ```ignore
//! pub fn profile(input: &ProfileInput) -> StyleProfile
//! ```
//!
//! - Runs on a blocking thread (`spawn_blocking`): file reads are fine, no async, no model.
//! - Read the author's decks read-only through lore-home (`input.home`); never send a deck
//!   itself anywhere. Footer slogans, logos and the like are left out. A cache across jobs
//!   may live in `input.ws.root.join("cache/…")` (wipeable tier), never in the repo.
//! - `summary` is pasted into the `outline` prompt as is (when not empty), so keep it short
//!   (a few hundred words, English). The structured fields are for `polish`.
//! - The stage stores the output as `profile.json`; it has no counts.

use serde::{Deserialize, Serialize};

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
}

/// Stub: an empty profile.
pub fn profile(_input: &ProfileInput) -> StyleProfile {
    StyleProfile::default()
}
