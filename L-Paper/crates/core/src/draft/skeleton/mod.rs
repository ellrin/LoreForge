//! 骨架 (skeleton first, `docs/long-paper.md` §G; the page contract is `docs/skeleton-api.md`).
//!
//! The paper is planned as sections → blocks (a frame around a few paragraphs) → paragraphs,
//! each with one line of what it says, before any text exists. Questions hang on paragraphs as
//! threads in the margin; an answer grows the paragraph's points in the background; text is
//! written paragraph by paragraph at the end.
//!
//! Stored inside the draft session ([`super::session::Session::skeleton`]); a session from before
//! it has none and keeps the old steps. Every edit is an [`Op`] on a fixed id applied by the
//! server under its lock (the page never writes the whole skeleton back), so a background fill
//! that lands between two edits is never overwritten. Ids are stable and never reused (the
//! counters are kept here). Undo restores the structure, or the fields an edit changed, never
//! what a background task wrote meanwhile.
//!
//! - [`inventory`]: 素材清單, what the paper could present.
//! - [`convert`]: an old session (groups, outline, drafts, interview) → a skeleton.
//! - [`plan`]: the planner, the repair at 暫存, the fill-in of an answer (prompts, schemas,
//!   checked parsing).
//! - [`style`]: 風格對照, the program's numbers of the written text next to the author's.

pub mod convert;
pub mod inventory;
pub mod plan;
pub mod project;
pub mod style;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Undo steps kept.
pub const UNDO_MAX: usize = 50;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Outline,
    Points,
    Text,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Point {
    pub text: String,
    /// The thread it grew from; None = the planner's.
    #[serde(default)]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Jump {
    pub why: String,
    pub fix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Para {
    pub id: String,
    /// A paragraph type of the author's style, or "".
    #[serde(default)]
    pub kind: String,
    /// What it says, one line (Traditional Chinese).
    #[serde(default)]
    pub say: String,
    #[serde(default)]
    pub points: Vec<Point>,
    #[serde(default)]
    pub link_prev: String,
    #[serde(default)]
    pub link_next: String,
    /// Too far from the paragraph before.
    #[serde(default)]
    pub jump: Option<Jump>,
    #[serde(default)]
    pub floats: Vec<String>,
    #[serde(default)]
    pub cites: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub pending: Vec<String>,
    /// LaTeX prose once written.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub state: State,
    /// `fill`, `write`, `refine`, `repair` while a background task works on it.
    #[serde(default)]
    pub busy: Option<String>,
    /// Checks on `text`.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The author's reason for adding or moving it, until the next 暫存.
    #[serde(default)]
    pub reason: String,
    /// Edited since the last 暫存.
    #[serde(default)]
    pub touched: bool,
}

impl Para {
    /// The state its content gives (text → text, points → points, else outline); `done` stays
    /// while there is text.
    pub fn settle_state(&mut self) {
        self.state = match (self.text.trim().is_empty(), self.state) {
            (false, State::Done) => State::Done,
            (false, _) => State::Text,
            (true, _) if !self.points.is_empty() => State::Points,
            _ => State::Outline,
        };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Block {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub purpose: String,
    /// The block or paragraph it expands.
    #[serde(default)]
    pub of: Option<String>,
    #[serde(default)]
    pub paras: Vec<Para>,
}

impl Block {
    /// The lowest state of its paragraphs (an empty block: outline).
    pub fn state(&self) -> State {
        self.paras.iter().map(|p| p.state).min().unwrap_or_default()
    }
}

/// `introduction|related|methods|results|discussion|conclusion|other`.
pub const ROLES: &[&str] = &["introduction", "related", "methods", "results", "discussion", "conclusion", "other"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Section {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

/// The role of a section title (the journal's words: "Materials and Methods" → methods).
pub fn role_of(title: &str) -> String {
    match crate::venueprofile::canonical_role(title) {
        "introduction" | "background" => "introduction",
        "related" => "related",
        "methods" | "data" | "experiments" => "methods",
        "results" => "results",
        "discussion" | "limitations" => "discussion",
        "conclusion" => "conclusion",
        _ => "other",
    }
    .to_string()
}

/// The old steps' section (one of [`super::session::SECTIONS`]) a skeleton section stands for:
/// related work goes with the Introduction; `other` by its title, else Discussion.
pub fn old_section(s: &Section) -> &'static str {
    use super::session::SECTIONS;
    match s.role.as_str() {
        "introduction" | "related" => "Introduction",
        "methods" => "Methods",
        "results" => "Results",
        "discussion" => "Discussion",
        "conclusion" => "Conclusion",
        _ => super::prompts::section_index(&s.title).map(|i| SECTIONS[i]).unwrap_or("Discussion"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Opt {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Msg {
    /// `ai` or `me`.
    pub who: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Change {
    pub id: String,
    pub target: String,
    /// `say|link_prev|link_next|purpose|text|points`.
    pub field: String,
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub after: String,
    #[serde(default)]
    pub why: String,
    /// `proposed|applied|reverted|rejected`.
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Thread {
    pub id: String,
    /// A paragraph or a block.
    pub anchor: String,
    /// `conflict|choice|missing|note|edit|refine`.
    pub kind: String,
    /// `open|answered|skipped|done`.
    pub status: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub options: Vec<Opt>,
    #[serde(default)]
    pub messages: Vec<Msg>,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub changes: Vec<Change>,
}

/// One thing the paper could present (素材清單).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Inv {
    pub id: String,
    /// `finding|table|figure|flow|equation|algorithm`.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
    /// Record ids that exist.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// What the text `\ref`s ("" for a finding).
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub keep: bool,
    #[serde(default)]
    pub by_author: bool,
    #[serde(default)]
    pub pending: bool,
    /// `table|plot|flow|image|equation|algorithm|`.
    #[serde(default)]
    pub tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Job {
    /// `plan|save|write|convert`.
    pub kind: String,
    pub label: String,
    pub done: usize,
    pub total: usize,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Targets {
    pub figure: [usize; 2],
    pub table: [usize; 2],
    pub equation: [usize; 2],
}

/// One undo step: the sections as they were and what the op changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Undo {
    pub sections: Vec<Section>,
    /// Paragraphs or blocks added, removed or moved: the structure comes back.
    #[serde(default)]
    pub structural: bool,
    /// Otherwise the fields an edit changed, by id.
    #[serde(default)]
    pub fields: Vec<(String, Vec<String>)>,
    /// The seams and touched blocks as they were.
    #[serde(default)]
    pub seams: Vec<String>,
    #[serde(default)]
    pub touched_blocks: Vec<String>,
    /// Threads a removal dropped (open questions on a removed paragraph), back on undo.
    #[serde(default)]
    pub dropped: Vec<Thread>,
    /// Threads a removal moved to another anchor: (thread, the anchor it had).
    #[serde(default)]
    pub reanchored: Vec<(String, String)>,
}

/// The skeleton as stored in the session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Skeleton {
    /// Grows with every change.
    #[serde(default)]
    pub rev: u64,
    #[serde(default)]
    pub sections: Vec<Section>,
    /// In the order they are asked.
    #[serde(default)]
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub inventory: Vec<Inv>,
    /// Where each inventory item came from (`fig:F1`, `tab:R1`, `exp:E1`, `eq:eq:loss`), to merge a
    /// fresh list into the author's choices; author items have none.
    #[serde(default)]
    pub inv_src: BTreeMap<String, String>,
    /// The author chose what to present (POST /inventory).
    #[serde(default)]
    pub inventory_set: bool,
    /// The last id given per prefix (s, b, p, q, e, r, n, v).
    #[serde(default)]
    pub counters: BTreeMap<String, u64>,
    #[serde(default)]
    pub undo: Vec<Undo>,
    /// Edits since the last 暫存.
    #[serde(default)]
    pub edits: usize,
    /// Paragraphs that got a new neighbour (a removal or a move next to them): repaired at 暫存.
    #[serde(default)]
    pub seams: Vec<String>,
    /// Blocks added, moved or edited since the last 暫存.
    #[serde(default)]
    pub touched_blocks: Vec<String>,
    /// A follow-up thread → the thread it follows (never a second follow-up).
    #[serde(default)]
    pub follow_of: BTreeMap<String, String>,
    #[serde(default)]
    pub job: Option<Job>,
    /// Answered threads whose paragraph was removed while their fill-in ran: the fill-in lands
    /// nowhere (the thread still closes).
    #[serde(default)]
    pub orphaned: Vec<String>,
}

/// What the page gets (the contract's `View`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct View {
    pub rev: u64,
    pub exists: bool,
    pub can_convert: bool,
    pub job: Option<Job>,
    pub dirty: bool,
    pub can_undo: bool,
    pub targets: Option<Targets>,
    pub inventory: Vec<Inv>,
    pub sections: Vec<Section>,
    pub threads: Vec<Thread>,
    pub next: Option<String>,
}

/// One edit from the page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AddPara {
        block: String,
        #[serde(default)]
        at: Option<usize>,
        #[serde(default)]
        say: String,
        #[serde(default)]
        reason: String,
    },
    AddBlock {
        section: String,
        #[serde(default)]
        at: Option<usize>,
        #[serde(default)]
        title: String,
        #[serde(default)]
        of: Option<String>,
    },
    Remove {
        id: String,
    },
    Move {
        id: String,
        to: String,
        #[serde(default)]
        at: Option<usize>,
        #[serde(default)]
        reason: String,
    },
    Edit {
        id: String,
        fields: serde_json::Map<String, serde_json::Value>,
    },
    Reason {
        id: String,
        reason: String,
    },
    Done {
        id: String,
        done: bool,
    },
    Undo,
}

/// Where a paragraph is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    pub s: usize,
    pub b: usize,
    pub p: usize,
}

impl Skeleton {
    /// A planned skeleton exists (not only an inventory).
    pub fn exists(&self) -> bool {
        !self.sections.is_empty()
    }

    /// The next id with `prefix`, never one given before.
    pub fn new_id(&mut self, prefix: &str) -> String {
        let n = self.counters.entry(prefix.to_string()).or_insert(0);
        *n += 1;
        format!("{prefix}{n}")
    }

    /// Raise the counters past every id in use (after a conversion or a plan that kept ids).
    pub fn sync_counters(&mut self) {
        let mut seen: Vec<String> = vec![];
        for s in &self.sections {
            seen.push(s.id.clone());
            for b in &s.blocks {
                seen.push(b.id.clone());
                seen.extend(b.paras.iter().map(|p| p.id.clone()));
            }
        }
        seen.extend(self.threads.iter().map(|t| t.id.clone()));
        seen.extend(self.inventory.iter().map(|i| i.id.clone()));
        for id in seen {
            let k = id.trim_start_matches(|c: char| c.is_ascii_alphabetic()).len();
            let (prefix, num) = id.split_at(id.len() - k);
            if let Ok(n) = num.parse::<u64>() {
                let c = self.counters.entry(prefix.to_string()).or_insert(0);
                *c = (*c).max(n);
            }
        }
    }

    pub fn find(&self, id: &str) -> Option<At> {
        for (si, s) in self.sections.iter().enumerate() {
            for (bi, b) in s.blocks.iter().enumerate() {
                if let Some(pi) = b.paras.iter().position(|p| p.id == id) {
                    return Some(At { s: si, b: bi, p: pi });
                }
            }
        }
        None
    }

    pub fn para(&self, id: &str) -> Option<&Para> {
        self.find(id).map(|a| &self.sections[a.s].blocks[a.b].paras[a.p])
    }

    pub fn para_mut(&mut self, id: &str) -> Option<&mut Para> {
        let a = self.find(id)?;
        Some(&mut self.sections[a.s].blocks[a.b].paras[a.p])
    }

    /// (section index, block index) of a block.
    pub fn find_block(&self, id: &str) -> Option<(usize, usize)> {
        self.sections.iter().enumerate().find_map(|(si, s)| s.blocks.iter().position(|b| b.id == id).map(|bi| (si, bi)))
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.find_block(id).map(|(s, b)| &self.sections[s].blocks[b])
    }

    pub fn block_mut(&mut self, id: &str) -> Option<&mut Block> {
        let (s, b) = self.find_block(id)?;
        Some(&mut self.sections[s].blocks[b])
    }

    /// The block holding a paragraph.
    pub fn block_of(&self, para: &str) -> Option<&Block> {
        self.find(para).map(|a| &self.sections[a.s].blocks[a.b])
    }

    /// The section holding a paragraph or a block.
    pub fn section_of(&self, id: &str) -> Option<&Section> {
        self.find(id).map(|a| a.s).or_else(|| self.find_block(id).map(|x| x.0)).map(|s| &self.sections[s])
    }

    /// Every paragraph id in reading order.
    pub fn order(&self) -> Vec<String> {
        self.paras().map(|p| p.id.clone()).collect()
    }

    pub fn paras(&self) -> impl Iterator<Item = &Para> {
        self.sections.iter().flat_map(|s| s.blocks.iter().flat_map(|b| b.paras.iter()))
    }

    pub fn paras_mut(&mut self) -> impl Iterator<Item = &mut Para> {
        self.sections.iter_mut().flat_map(|s| s.blocks.iter_mut().flat_map(|b| b.paras.iter_mut()))
    }

    /// The paragraphs before and after `id` in reading order.
    pub fn neighbours(&self, id: &str) -> (Option<String>, Option<String>) {
        let o = self.order();
        match o.iter().position(|x| x == id) {
            Some(k) => (k.checked_sub(1).map(|j| o[j].clone()), o.get(k + 1).cloned()),
            None => (None, None),
        }
    }

    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }

    pub fn thread_mut(&mut self, id: &str) -> Option<&mut Thread> {
        self.threads.iter_mut().find(|t| t.id == id)
    }

    /// The open thread to show now: the first open one in asking order.
    pub fn next(&self) -> Option<String> {
        self.threads.iter().find(|t| t.status == "open" && !t.question.is_empty()).map(|t| t.id.clone())
    }

    pub fn dirty(&self) -> bool {
        self.edits > 0
    }

    /// Paragraph labels and floats the planner may use: the kept inventory's labels.
    pub fn kept_labels(&self) -> Vec<String> {
        self.inventory.iter().filter(|i| i.keep && !i.label.is_empty()).map(|i| i.label.clone()).collect()
    }

    /// A paragraph id or a block id that exists.
    pub fn has(&self, id: &str) -> bool {
        self.find(id).is_some() || self.find_block(id).is_some()
    }

    /// The page's view of it.
    pub fn view(&self, can_convert: bool, targets: Option<Targets>) -> View {
        View {
            rev: self.rev,
            exists: self.exists(),
            can_convert,
            job: self.job.clone(),
            dirty: self.dirty(),
            can_undo: !self.undo.is_empty(),
            targets,
            inventory: self.inventory.clone(),
            sections: self.sections.clone(),
            threads: self.threads.clone(),
            next: self.next(),
        }
    }

    fn mark_seams_around(&mut self, id: &str) {
        let (a, b) = self.neighbours(id);
        for x in [a, b].into_iter().flatten() {
            if !self.seams.contains(&x) {
                self.seams.push(x);
            }
        }
    }

    fn touch_block(&mut self, id: &str) {
        if !self.touched_blocks.iter().any(|b| b == id) {
            self.touched_blocks.push(id.to_string());
        }
    }

    fn push_undo(&mut self, structural: bool, fields: Vec<(String, Vec<String>)>) {
        self.undo.push(Undo { sections: self.sections.clone(), structural, fields, seams: self.seams.clone(), touched_blocks: self.touched_blocks.clone(), ..Default::default() });
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
    }

    /// Paragraphs (or a block) `gone`: their open questions are dropped, the rest of their
    /// threads hang on `to` (the block, or the section when the block went). Kept on the last
    /// undo step so undo brings them back.
    fn unanchor(&mut self, gone: &[String], to: &str) {
        let mut dropped = vec![];
        let mut moved = vec![];
        let mut keep = vec![];
        for mut t in std::mem::take(&mut self.threads) {
            if !gone.contains(&t.anchor) {
                keep.push(t);
            } else if t.status == "open" {
                dropped.push(t);
            } else {
                moved.push((t.id.clone(), t.anchor.clone()));
                if t.status == "answered" && !self.orphaned.contains(&t.id) {
                    self.orphaned.push(t.id.clone());
                }
                t.anchor = to.to_string();
                keep.push(t);
            }
        }
        self.threads = keep;
        if let Some(u) = self.undo.last_mut() {
            u.dropped = dropped;
            u.reanchored = moved;
        }
    }

    /// Apply one op. Returns the id it made (add_para, add_block). Never calls a model.
    pub fn apply(&mut self, op: Op) -> Result<Option<String>, String> {
        let made = match op {
            Op::Undo => {
                self.undo_last()?;
                return Ok(None);
            }
            Op::AddPara { block, at, say, reason } => {
                let Some((si, bi)) = self.find_block(&block) else { return Err(format!("沒有這個框：{block}")) };
                self.push_undo(true, vec![]);
                let id = self.new_id("p");
                let len = self.sections[si].blocks[bi].paras.len();
                let at = at.unwrap_or(len).min(len);
                self.sections[si].blocks[bi].paras.insert(at, Para { id: id.clone(), say: say.trim().to_string(), reason: reason.trim().to_string(), touched: true, ..Default::default() });
                Some(id)
            }
            Op::AddBlock { section, at, title, of } => {
                let Some(si) = self.sections.iter().position(|s| s.id == section) else { return Err(format!("沒有這一章：{section}")) };
                let of = of.filter(|o| self.has(o));
                self.push_undo(true, vec![]);
                let id = self.new_id("b");
                let len = self.sections[si].blocks.len();
                let at = at.unwrap_or(len).min(len);
                self.sections[si].blocks.insert(at, Block { id: id.clone(), title: title.trim().to_string(), of, ..Default::default() });
                self.touch_block(&id);
                Some(id)
            }
            Op::Remove { id } => {
                if let Some(a) = self.find(&id) {
                    self.push_undo(true, vec![]);
                    self.mark_seams_around(&id);
                    let block = self.sections[a.s].blocks[a.b].id.clone();
                    self.sections[a.s].blocks[a.b].paras.remove(a.p);
                    self.unanchor(&[id.clone()], &block);
                } else if let Some((si, bi)) = self.find_block(&id) {
                    self.push_undo(true, vec![]);
                    let ids: Vec<String> = self.sections[si].blocks[bi].paras.iter().map(|p| p.id.clone()).collect();
                    let mut gone = ids.clone();
                    gone.push(id.clone());
                    let section = self.sections[si].id.clone();
                    self.unanchor(&gone, &section);
                    if let (Some(first), Some(last)) = (ids.first(), ids.last()) {
                        let (before, _) = self.neighbours(first);
                        let (_, after) = self.neighbours(last);
                        for x in [before, after].into_iter().flatten() {
                            if !self.seams.contains(&x) {
                                self.seams.push(x);
                            }
                        }
                    }
                    self.sections[si].blocks.remove(bi);
                    // a block that expanded the removed one now expands nothing
                    for b in self.sections.iter_mut().flat_map(|s| s.blocks.iter_mut()) {
                        if b.of.as_deref().is_some_and(|o| o == id || ids.iter().any(|p| p == o)) {
                            b.of = None;
                        }
                    }
                } else {
                    return Err(format!("沒有這一段或這個框：{id}"));
                }
                self.seams.retain(|s| s != &id);
                None
            }
            Op::Move { id, to, at, reason } => {
                if let Some(a) = self.find(&id) {
                    let Some((ts, tb)) = self.find_block(&to) else { return Err(format!("沒有這個框：{to}")) };
                    self.push_undo(true, vec![]);
                    self.mark_seams_around(&id);
                    let mut p = self.sections[a.s].blocks[a.b].paras.remove(a.p);
                    p.touched = true;
                    if !reason.trim().is_empty() {
                        p.reason = reason.trim().to_string();
                    }
                    let len = self.sections[ts].blocks[tb].paras.len();
                    self.sections[ts].blocks[tb].paras.insert(at.unwrap_or(len).min(len), p);
                    self.seams.retain(|s| s != &id);
                } else if let Some((si, bi)) = self.find_block(&id) {
                    let Some(ts) = self.sections.iter().position(|s| s.id == to) else { return Err(format!("沒有這一章：{to}")) };
                    self.push_undo(true, vec![]);
                    let ids: Vec<String> = self.sections[si].blocks[bi].paras.iter().map(|p| p.id.clone()).collect();
                    if let (Some(first), Some(last)) = (ids.first().cloned(), ids.last().cloned()) {
                        self.mark_seams_around(&first);
                        self.mark_seams_around(&last);
                    }
                    let b = self.sections[si].blocks.remove(bi);
                    let len = self.sections[ts].blocks.len();
                    self.sections[ts].blocks.insert(at.unwrap_or(len).min(len), b);
                    self.seams.retain(|s| !ids.contains(s));
                    for p in ids.iter() {
                        if let Some(x) = self.para_mut(p) {
                            x.touched = true;
                            if !reason.trim().is_empty() && x.reason.is_empty() {
                                x.reason = reason.trim().to_string();
                            }
                        }
                    }
                    self.touch_block(&id);
                } else {
                    return Err(format!("沒有這一段或這個框：{id}"));
                }
                None
            }
            Op::Edit { id, fields } => {
                self.edit(&id, fields)?;
                None
            }
            Op::Reason { id, reason } => {
                if self.para(&id).is_none() {
                    return Err(format!("沒有這一段：{id}"));
                }
                self.push_undo(false, vec![(id.clone(), vec!["reason".into()])]);
                if let Some(p) = self.para_mut(&id) {
                    p.reason = reason.trim().to_string();
                }
                None
            }
            Op::Done { id, done } => {
                let Some(p) = self.para(&id) else { return Err(format!("沒有這一段：{id}")) };
                if p.text.trim().is_empty() {
                    return Err("這一段還沒有文字".into());
                }
                self.push_undo(false, vec![(id.clone(), vec!["state".into()])]);
                if let Some(p) = self.para_mut(&id) {
                    p.state = if done { State::Done } else { State::Text };
                }
                None
            }
        };
        self.edits += 1;
        Ok(made)
    }

    fn edit(&mut self, id: &str, fields: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
        let str_of = |k: &str, v: &serde_json::Value| -> Result<String, String> { v.as_str().map(|s| s.to_string()).ok_or_else(|| format!("{k} 要是文字")) };
        if self.para(id).is_some() {
            let mut names: Vec<String> = vec![];
            // check every field before changing anything
            let mut set: Vec<(String, serde_json::Value)> = vec![];
            for (k, v) in fields {
                match k.as_str() {
                    "say" | "kind" | "text" => {
                        str_of(&k, &v)?;
                    }
                    "points" => {
                        points_of(&v)?;
                    }
                    _ => return Err(format!("段落不能改 {k}")),
                }
                names.push(k.clone());
                set.push((k, v));
            }
            let mut fields_undo = names.clone();
            fields_undo.push("touched".to_string());
            self.push_undo(false, vec![(id.to_string(), fields_undo)]);
            let p = self.para_mut(id).expect("checked");
            for (k, v) in set {
                match k.as_str() {
                    "say" => p.say = v.as_str().unwrap_or("").trim().to_string(),
                    "kind" => p.kind = v.as_str().unwrap_or("").trim().to_string(),
                    "text" => p.text = v.as_str().unwrap_or("").to_string(),
                    "points" => p.points = points_of(&v).unwrap_or_default(),
                    _ => {}
                }
            }
            p.touched = true;
            p.settle_state();
            return Ok(());
        }
        if self.block(id).is_some() {
            let mut set: Vec<(String, serde_json::Value)> = vec![];
            for (k, v) in fields {
                match k.as_str() {
                    "title" | "purpose" => {
                        str_of(&k, &v)?;
                    }
                    "of" => {
                        if !(v.is_null() || v.is_string()) {
                            return Err("of 要是文字或 null".into());
                        }
                    }
                    _ => return Err(format!("框不能改 {k}")),
                }
                set.push((k, v));
            }
            let names: Vec<String> = set.iter().map(|(k, _)| k.clone()).collect();
            self.push_undo(false, vec![(id.to_string(), names)]);
            let ok_of: Option<String> = set.iter().find(|(k, _)| k == "of").and_then(|(_, v)| v.as_str()).map(str::to_string).filter(|o| o != id && self.has(o));
            let b = self.block_mut(id).expect("checked");
            for (k, v) in set {
                match k.as_str() {
                    "title" => b.title = v.as_str().unwrap_or("").trim().to_string(),
                    "purpose" => b.purpose = v.as_str().unwrap_or("").trim().to_string(),
                    "of" => b.of = ok_of.clone(),
                    _ => {}
                }
            }
            self.touch_block(id);
            return Ok(());
        }
        Err(format!("沒有這一段或這個框：{id}"))
    }

    /// Undo the last op: the structure as it was (paragraphs keep what was written into them
    /// since: points, text, problems), or the fields an edit changed.
    fn undo_last(&mut self) -> Result<(), String> {
        let Some(u) = self.undo.pop() else { return Err("沒有可以復原的".into()) };
        if u.structural {
            let current: BTreeMap<String, Para> = self.paras().map(|p| (p.id.clone(), p.clone())).collect();
            let blocks: BTreeMap<String, (String, String, Option<String>)> =
                self.sections.iter().flat_map(|s| s.blocks.iter()).map(|b| (b.id.clone(), (b.title.clone(), b.purpose.clone(), b.of.clone()))).collect();
            let mut secs = u.sections;
            for b in secs.iter_mut().flat_map(|s| s.blocks.iter_mut()) {
                if let Some((t, p, o)) = blocks.get(&b.id) {
                    b.title = t.clone();
                    b.purpose = p.clone();
                    b.of = o.clone();
                }
                for p in b.paras.iter_mut() {
                    if let Some(now) = current.get(&p.id) {
                        // the content as it is now; only where it sits, and the marks of the edit, come back
                        let (touched, reason) = (p.touched, p.reason.clone());
                        *p = now.clone();
                        p.touched = touched;
                        p.reason = reason;
                    }
                }
            }
            self.sections = secs;
        } else {
            for (id, names) in &u.fields {
                let old_para = u.sections.iter().flat_map(|s| s.blocks.iter().flat_map(|b| b.paras.iter())).find(|p| &p.id == id).cloned();
                let old_block = u.sections.iter().flat_map(|s| s.blocks.iter()).find(|b| &b.id == id).cloned();
                if let (Some(old), Some(p)) = (old_para, self.para_mut(id)) {
                    for n in names {
                        match n.as_str() {
                            "say" => p.say = old.say.clone(),
                            "kind" => p.kind = old.kind.clone(),
                            "text" => p.text = old.text.clone(),
                            "points" => p.points = old.points.clone(),
                            "reason" => p.reason = old.reason.clone(),
                            "touched" => p.touched = old.touched,
                            "state" => p.state = old.state,
                            _ => {}
                        }
                    }
                    if !names.iter().any(|n| n == "state") {
                        p.settle_state();
                    }
                } else if let (Some(old), Some(b)) = (old_block, self.block_mut(id)) {
                    for n in names {
                        match n.as_str() {
                            "title" => b.title = old.title.clone(),
                            "purpose" => b.purpose = old.purpose.clone(),
                            "of" => b.of = old.of.clone(),
                            _ => {}
                        }
                    }
                }
            }
        }
        // threads the op dropped or moved come back where they were
        for (t, anchor) in &u.reanchored {
            if let Some(x) = self.thread_mut(t) {
                x.anchor = anchor.clone();
            }
            self.orphaned.retain(|o| o != t);
        }
        for t in u.dropped {
            if self.thread(&t.id).is_none() {
                self.threads.push(t);
            }
        }
        self.seams = u.seams;
        self.touched_blocks = u.touched_blocks;
        self.edits = self.edits.saturating_sub(1);
        Ok(())
    }

    /// Apply the page's ops in order; stops at the first that fails (those before stay).
    /// Returns the ids made. `rev` grows once.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> Result<Vec<String>, String> {
        let mut made = vec![];
        let mut err = None;
        for op in ops {
            match self.apply(op) {
                Ok(Some(id)) => made.push(id),
                Ok(None) => {}
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        self.rev += 1;
        match err {
            Some(e) => Err(e),
            None => Ok(made),
        }
    }

    /// What 暫存 repairs: paragraphs touched or next to a seam, and their neighbours; blocks touched
    /// or holding a touched paragraph. Empty when nothing changed.
    pub fn repair_scope(&self) -> (Vec<String>, Vec<String>) {
        let order = self.order();
        let core: Vec<&String> = order.iter().filter(|id| self.para(id).is_some_and(|p| p.touched) || self.seams.contains(id)).collect();
        let mut paras: Vec<String> = vec![];
        for (k, id) in order.iter().enumerate() {
            let near = |j: usize| order.get(j).is_some_and(|x| core.contains(&x));
            if core.contains(&id) || near(k + 1) || k.checked_sub(1).is_some_and(near) {
                paras.push(id.clone());
            }
        }
        let mut blocks: Vec<String> = self.touched_blocks.iter().filter(|b| self.block(b).is_some()).cloned().collect();
        for id in &core {
            if let Some(b) = self.block_of(id) {
                if !blocks.contains(&b.id) {
                    blocks.push(b.id.clone());
                }
            }
        }
        (paras, blocks)
    }

    /// After 暫存: nothing is pending repair.
    pub fn clear_marks(&mut self) {
        for p in self.paras_mut() {
            p.touched = false;
            p.reason.clear();
        }
        self.seams.clear();
        self.touched_blocks.clear();
        self.edits = 0;
    }

    /// Take or drop one change of a thread. `edit` changes are applied already: false reverts,
    /// true applies again; `refine` changes are proposed: true applies (only while `before` is
    /// still in the text), false rejects.
    pub fn decide(&mut self, thread: &str, change: &str, accept: bool) -> Result<(), String> {
        let Some(t) = self.thread(thread) else { return Err("沒有這一串".into()) };
        let Some(c) = t.changes.iter().find(|c| c.id == change).cloned() else { return Err("沒有這項修改".into()) };
        let new_state = match (c.state.as_str(), accept) {
            ("proposed", false) => "rejected",
            ("applied", true) | ("rejected", false) | ("reverted", false) => return Ok(()),
            ("proposed" | "reverted" | "rejected", true) => {
                self.set_field(&c.target, &c.field, &c.before, &c.after)?;
                "applied"
            }
            ("applied", false) => {
                self.set_field(&c.target, &c.field, &c.after, &c.before)?;
                "reverted"
            }
            _ => return Err("這項修改的狀態不明".into()),
        };
        if let Some(t) = self.thread_mut(thread) {
            if let Some(x) = t.changes.iter_mut().find(|x| x.id == change) {
                x.state = new_state.into();
            }
            if t.kind == "refine" && t.changes.iter().all(|x| x.state != "proposed") {
                t.status = "done".into();
            }
        }
        self.rev += 1;
        Ok(())
    }

    /// `field` of `target` from `from` to `to`. Text changes replace a passage that must still be
    /// there exactly once; the other fields must still read `from`.
    fn set_field(&mut self, target: &str, field: &str, from: &str, to: &str) -> Result<(), String> {
        if field == "purpose" {
            let b = self.block_mut(target).ok_or("那個框已經不在了")?;
            if b.purpose != from {
                return Err("框的目的已經改過，不能套用".into());
            }
            b.purpose = to.to_string();
            return Ok(());
        }
        let p = self.para_mut(target).ok_or("那一段已經不在了")?;
        match field {
            "text" => {
                if from.is_empty() {
                    if !p.text.is_empty() {
                        return Err("文字已經改過，不能套用".into());
                    }
                    p.text = to.to_string();
                } else {
                    let n = p.text.matches(from).count();
                    if n != 1 {
                        return Err(if n == 0 { "文字已經改過，原句不在了".into() } else { "原句出現不只一次，請手動改".into() });
                    }
                    p.text = p.text.replacen(from, to, 1);
                }
                p.settle_state();
            }
            "say" | "link_prev" | "link_next" => {
                let cur = match field {
                    "say" => &mut p.say,
                    "link_prev" => &mut p.link_prev,
                    _ => &mut p.link_next,
                };
                if cur != from {
                    return Err("這一欄已經改過，不能套用".into());
                }
                *cur = to.to_string();
            }
            "points" => {
                let cur = serde_json::to_string(&p.points.iter().map(|x| x.text.clone()).collect::<Vec<_>>()).unwrap_or_default();
                if cur != from {
                    return Err("要點已經改過，不能套用".into());
                }
                let texts: Vec<String> = serde_json::from_str(to).unwrap_or_default();
                p.points = texts.into_iter().map(|text| Point { text, from: None }).collect();
                p.settle_state();
            }
            _ => return Err(format!("不能改 {field}")),
        }
        Ok(())
    }

    /// A new thread (its id from the kind: q for questions, n notes, e edits, r refinements).
    pub fn add_thread(&mut self, mut t: Thread) -> String {
        let prefix = match t.kind.as_str() {
            "edit" => "e",
            "refine" => "r",
            "note" => "n",
            _ => "q",
        };
        t.id = self.new_id(prefix);
        let id = t.id.clone();
        self.threads.push(t);
        id
    }
}

/// `points` from the page: `[{text, from}]` or `["…"]`.
fn points_of(v: &serde_json::Value) -> Result<Vec<Point>, String> {
    let arr = v.as_array().ok_or("points 要是陣列")?;
    let mut out = vec![];
    for x in arr {
        match x {
            serde_json::Value::String(s) => out.push(Point { text: s.trim().to_string(), from: None }),
            serde_json::Value::Object(_) => out.push(serde_json::from_value::<Point>(x.clone()).map_err(|e| format!("points：{e}"))?),
            _ => return Err("points 的每一項要是文字".into()),
        }
    }
    out.retain(|p| !p.text.trim().is_empty());
    Ok(out)
}

/// Option ids o1, o2 … for a question's choices.
pub fn options(texts: Vec<String>) -> Vec<Opt> {
    texts.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).enumerate().map(|(k, text)| Opt { id: format!("o{}", k + 1), text }).collect()
}

#[cfg(test)]
mod tests;
