//! What the dashboard shows (docs/brain-hub.md §8, the chosen cover): the whole and how it
//! splits into families, what each family holds, what was added week by week, what the steps
//! used, where the gaps are. The shapes here are the FROZEN contract between the hub, L-Brain's
//! server and its page; `Hub::stats` fills them.
//!
//! A **family** is a row of the dashboard: the categories that share a `Category::group` under
//! one app section (a category without a group is its own family). Knowledge cards are a family
//! the server adds itself (they live in kb-core, not in the hub).

use serde::{Deserialize, Serialize};

/// A named count inside a family (`句型 440`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    /// The category id to open (or "" when the part is not a category of its own, e.g. a level).
    pub category: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Family {
    /// Stable key: `<app>/<group>` or the category id.
    pub id: String,
    pub label: String,
    /// The dashboard section: a Lore app id, `my`, `x`, `kb`.
    pub app: String,
    /// The category ids in it, in registry order (the first is what the row opens).
    pub categories: Vec<String>,
    pub count: usize,
    pub parts: Vec<Part>,
    /// Items created in each of the last 12 weeks (Monday to Sunday), oldest first; the last is this week.
    pub weeks: Vec<usize>,
    /// Created in the last 30 days.
    pub added_30d: usize,
    /// Items a step used in the last 30 days (distinct items).
    pub used_30d: usize,
    /// One short line when something is missing (Traditional Chinese), else "": references still
    /// to make, no step uses it, not shared with Lore.
    pub gap: String,
    /// An outside application's independent set.
    pub independent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepUse {
    /// lore-llm task id.
    pub task: String,
    /// Distinct items it used in the last 30 days.
    pub items: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub total: usize,
    pub families: Vec<Family>,
    /// The Monday (`YYYY-MM-DD`) of each of the 12 weeks in `Family::weeks`, oldest first.
    pub week_starts: Vec<String>,
    pub steps: Vec<StepUse>,
    /// Open proposals waiting for the author.
    pub proposals: usize,
    pub untagged: usize,
    /// Lore and hand-made categories no step uses.
    pub unconnected: Vec<String>,
    /// Outside applications and how many items each gave.
    pub sources: Vec<Part>,
}
