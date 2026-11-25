//! The author's know-how about figures, as a model is shown it (`docs/flow-figures.md` §8).
//!
//! The flow tool draws; what to draw comes from knowledge the author keeps in L-Brain: every
//! component, module and whole figure there says what it is and when to use it (its prompt),
//! which variables it takes and gives, and how its parts connect. An app reads that from the
//! hub and hands it in as a [`Knowledge`] (this crate does not know the hub). Drafting a new
//! figure then picks from it by the variables and words of the description ([`pick_modules`],
//! [`pick_examples`]) and every pick says why it was picked, so the author can see and correct
//! what the model was told.
//!
//! The types are FROZEN (the seam between this crate, lore-know's figure format and the apps).

use serde::{Deserialize, Serialize};

/// The words a variable's kind may be. A variable of an unknown kind is `other`.
pub const VAR_KINDS: [&str; 12] = ["image", "volume", "video", "signal", "mask", "label", "vector", "feature", "scalar", "text", "loss", "other"];

/// One variable a piece takes or gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Var {
    /// A notation id (`x`, `y_hat`) or a plain name when it has none.
    pub id: String,
    /// One of [`VAR_KINDS`].
    pub kind: String,
    /// What it is, in a few words.
    pub meaning: String,
}

/// A component or module the model may put in a figure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModuleNote {
    /// The module kind of the DSL (`encoder`, `unet`, a saved drawing's kind).
    pub kind: String,
    /// The art style; empty = the kind as a whole.
    pub style: String,
    pub title: String,
    /// The author's words: what it is and when to use it. Empty = he has not said.
    pub prompt: String,
    pub inputs: Vec<Var>,
    pub outputs: Vec<Var>,
    /// Domain and kind tags (`segmentation`, `attention` …).
    pub tags: Vec<String>,
}

/// A whole figure the author made: a worked example for the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Example {
    /// The hub item's id (shown to the author, never to the model).
    pub id: String,
    pub title: String,
    /// What the figure shows (the description it was made from, or the author's prompt).
    pub description: String,
    /// The figure's L1 text.
    pub dsl: String,
    pub inputs: Vec<Var>,
    pub outputs: Vec<Var>,
    pub tags: Vec<String>,
}

/// Everything an app hands in for one draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Knowledge {
    pub modules: Vec<ModuleNote>,
    pub examples: Vec<Example>,
}

/// Something picked, with the reason in one short line the author can read
/// (`takes an image and gives a mask, like the description`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Picked<T> {
    pub item: T,
    pub score: f32,
    pub why: String,
}

/// What the description is about, as far as code can tell: the variable kinds it names.
/// Pure and cheap (word lists, no model): `CT volume` → `volume`, `segmentation mask` → `mask`.
pub fn kinds_in(description: &str) -> Vec<String> {
    let _ = description;
    vec![]
}

/// The made figures closest to the description, best first, at most `limit`: ranked by the
/// variable kinds they share with it (inputs and outputs), then by shared tags and words. A
/// figure that shares nothing is not picked. Deterministic.
pub fn pick_examples<'a>(description: &str, k: &'a Knowledge, limit: usize) -> Vec<Picked<&'a Example>> {
    let _ = (description, k, limit);
    vec![]
}

/// The modules worth a line of their own in the prompt, best first, at most `limit`: those
/// whose prompt is not empty and whose variables, tags or words meet the description.
pub fn pick_modules<'a>(description: &str, k: &'a Knowledge, limit: usize) -> Vec<Picked<&'a ModuleNote>> {
    let _ = (description, k, limit);
    vec![]
}

/// `id | kind | meaning`, one variable per line: the text form the hub stores. Lines that are
/// empty or start with `#` are skipped; a missing kind is `other`.
pub fn vars_from_text(text: &str) -> Vec<Var> {
    let _ = text;
    vec![]
}

/// The inverse of [`vars_from_text`].
pub fn vars_to_text(vars: &[Var]) -> String {
    let _ = vars;
    String::new()
}
