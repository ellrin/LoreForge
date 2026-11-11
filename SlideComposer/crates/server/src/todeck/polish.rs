//! `polish` stage: layout and wording checks on the finished slides, no sentence rewritten
//! (docs/paper-deck.md 「排版用字微調」, rules 1–6). **Owned by the polish agent**; this file
//! is a pass-through stub (nothing checked, nothing changed) until it is filled in.
//!
//! Contract (the runner calls exactly this; keep the signature):
//!
//! ```ignore
//! pub fn polish(slides: &mut Vec<Slide>, input: &PolishInput) -> PolishReport
//! ```
//!
//! - Runs on a blocking thread (`spawn_blocking`), no async, no model.
//! - `slides` are the deck's slides as saved by the `slides` stage; change them in place.
//!   When `fixes` is not empty the stage applies the changed slides as one undoable
//!   transaction (「排版用字微調」); every fix is listed so the page can undo it alone.
//! - `checks` = how many checks ran (the progress shows 「檢查 n 項」).
//! - The stage stores the report as `polish.json` (`counts.checks`, `counts.fixes`).

use lore_canvas::{Slide, StyleSystem};
use serde::{Deserialize, Serialize};

use super::profile::StyleProfile;

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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishReport {
    pub checks: u32,
    pub fixes: Vec<Fix>,
}

/// Stub: nothing checked, nothing changed.
pub fn polish(_slides: &mut Vec<Slide>, _input: &PolishInput) -> PolishReport {
    PolishReport::default()
}
