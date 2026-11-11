//! `polish` stage: layout and wording checks on the finished slides, no sentence rewritten
//! (docs/paper-deck.md 「排版用字微調」, rules 1–6), made by [`crate::deckpolish`].
//!
//! Contract (the runner calls exactly this; keep the signature):
//!
//! ```ignore
//! pub fn polish(slides: &mut Vec<Slide>, input: &PolishInput) -> PolishReport
//! ```
//!
//! - Runs on a blocking thread (`spawn_blocking`), no async, no model: the deterministic
//!   checks only. Rule 6's model step (one term per concept) runs afterwards from the deck
//!   (`POST /api/decks/{name}/polish`, `deckpolish::suggest_terms`).
//! - `slides` are the deck's slides as saved by the `slides` stage; changed in place. The
//!   stage applies them as one undoable transaction (「排版用字微調」); every fix carries its
//!   exact `changes`, so the page undoes one fix alone with
//!   `POST /api/decks/{name}/polish/changes {action: undo, changes}`.
//! - `checks` = how many checks ran (the progress shows 「檢查 n 項」); `items` = every finding,
//!   fixed or not (`manual`: listed only, `suggested`: offered).
//! - The stage stores the report as `polish.json` (`counts.checks`, `counts.fixes`).

use lore_canvas::{Deck, Editor, Slide, StyleSystem};
use serde::{Deserialize, Serialize};

use super::profile::StyleProfile;
use crate::deckpolish::{self, Change, Input, Item};

pub struct PolishInput<'a> {
    pub st: &'a StyleSystem,
    pub profile: &'a StyleProfile,
    /// The deck's notation (registered names and spellings, rule 6).
    pub notation: &'a lore_notation::Notation,
    pub lang: &'a str,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Fix {
    /// Slide uid.
    pub slide: String,
    /// Object id on the slide, when the fix is in one object.
    pub object: Option<u32>,
    /// Rule number (1–6).
    pub rule: u8,
    /// One line for the report.
    pub what: String,
    pub before: String,
    pub after: String,
    /// The exact values changed (undo one fix alone).
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishReport {
    pub checks: u32,
    pub fixes: Vec<Fix>,
    /// Every finding (applied, suggested, manual).
    pub items: Vec<Item>,
}

fn shown(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn polish(slides: &mut Vec<Slide>, input: &PolishInput) -> PolishReport {
    let pk = &input.st.pack.pack;
    let mut deck = Deck::new(pk.slide_cx, pk.slide_cy, Some(pk.id.clone()));
    deck.slides = std::mem::take(slides);
    let mut ed = Editor::new(deck, Some(input.st.clone()));
    let roles = deckpolish::roles_for(&ed.deck, &[]);
    let rep = deckpolish::run_checks(&mut ed, &Input { profile: input.profile.detail.as_ref(), notation: input.notation, roles });
    *slides = std::mem::take(&mut ed.deck.slides);
    let fixes = rep
        .items
        .iter()
        .filter(|i| i.state == "applied")
        .map(|i| Fix {
            slide: i.slide.clone(),
            object: i.object,
            rule: i.check,
            what: i.message.clone(),
            before: i.changes.first().map(|c| shown(&c.before)).unwrap_or_default(),
            after: i.changes.first().map(|c| shown(&c.after)).unwrap_or_default(),
            changes: i.changes.clone(),
        })
        .collect();
    PolishReport { checks: 6, fixes, items: rep.items }
}
