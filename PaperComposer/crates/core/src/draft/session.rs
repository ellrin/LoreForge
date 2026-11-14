//! The state of one paper's draft interview, kept in the paper folder:
//!
//! ```text
//! papers/<name>/draft/
//!   session.json      phase, experiments, timeline, groups, ledger, claims,
//!                     review, outline, drafts, audit
//!   transcript.jsonl  every question and answer, append only
//!   records/          the experiment records the author uploaded
//! ```
//!
//! Everything is plain JSON so the author's private repository keeps the
//! whole conversation, and a session can be resumed on another machine.

use super::lens::{Graded, Lens, Missing};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DRAFT_DIR: &str = "draft";
const SESSION_FILE: &str = "session.json";
const TRANSCRIPT_FILE: &str = "transcript.jsonl";
pub const RECORDS_DIR: &str = "records";
/// Earlier sessions (重新開始) and snapshots taken before a rewind or a
/// template change, one folder per time stamp.
pub const ARCHIVE_DIR: &str = "archive";
pub const SECTIONS: [&str; 5] = ["Introduction", "Methods", "Results", "Discussion", "Conclusion"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// 1 key value and the "interesting" hypothesis
    #[default]
    Topic,
    /// 2 experiment record → experiments and timeline
    Record,
    /// 3 question groups
    Plan,
    /// 4 the conversation
    Interview,
    /// 4.5 claim register
    Claims,
    /// 5 order and narrative check
    Review,
    /// ★ the journal template (the paper starts on the plain article)
    Template,
    /// 6 paragraph-level outline
    Outline,
    /// 7 drafting
    Draft,
    /// 8 reviewer pass
    Audit,
}

/// Step 1: what readers already accept, and the twist this paper adds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Hook {
    #[serde(default)]
    pub reader_accepts: String,
    #[serde(default)]
    pub twist: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RecordFile {
    pub file: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Experiment {
    pub id: String,
    pub title: String,
    /// What was changed or compared.
    #[serde(default)]
    pub change: String,
    #[serde(default)]
    pub metrics: String,
    #[serde(default)]
    pub result: String,
    /// Guessed purpose; `purpose_sure` false = ask the author.
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub purpose_sure: bool,
    /// Guessed lens mode (e.g. routine cross-validation vs an own ablation).
    #[serde(default)]
    pub mode: Option<String>,
}

/// Step 2: what was tried, in order — where the thinking shows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TimelineStep {
    pub step: usize,
    pub what: String,
    #[serde(default)]
    pub observed: String,
    #[serde(default)]
    pub changed: String,
}

/// What the author wrote down about their own thinking in the record
/// ("tried X because…", "suspect Y") — evidence for their stance and a
/// source of options when the interview asks why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Thought {
    #[serde(default)]
    pub id: String,
    /// Close to the author's own words.
    pub text: String,
    /// Experiment ids (E2) or timeline steps (T3) it is about.
    #[serde(default)]
    pub about: Vec<String>,
}

/// A results table built from the experiment record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ResultTable {
    #[serde(default)]
    pub id: String,
    pub caption: String,
    /// Header row, then data rows; numbers copied from the record.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// What the table is meant to show (for the author).
    #[serde(default)]
    pub note: String,
    /// Experiment ids it draws on.
    #[serde(default)]
    pub from: Vec<String>,
    /// Put into the paper already.
    #[serde(default)]
    pub inserted: bool,
}

impl ResultTable {
    /// Tab-separated cells, the format the table importer reads.
    pub fn tsv(&self) -> String {
        std::iter::once(&self.columns).chain(self.rows.iter()).map(|r| r.join("\t")).collect::<Vec<_>>().join("\n")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GroupStatus {
    #[default]
    Pending,
    Active,
    Done,
    Skipped,
}

/// What a group has to deliver: explanation, motive, conclusion; plus the
/// two halves of "interesting".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GroupSummary {
    #[serde(default)]
    pub explain: String,
    #[serde(default)]
    pub motive: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub reader_accepts: String,
    #[serde(default)]
    pub twist: String,
}

impl GroupSummary {
    pub fn complete(&self) -> bool {
        [&self.explain, &self.motive, &self.conclusion].iter().all(|s| !s.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Group {
    pub id: String,
    /// One of SECTIONS.
    pub section: String,
    pub title: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub status: GroupStatus,
    #[serde(default)]
    pub summary: GroupSummary,
    /// Answers given in this group.
    #[serde(default)]
    pub turns: usize,
    /// 預想選項 for the group's items: being prepared, ready, or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prep: Option<super::candidates::Prep>,
}

/// One entry in the ledger: a decision, an experiment, a finding, a claim
/// in the making — graded by the lens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Item {
    pub id: String,
    pub group: String,
    pub label: String,
    /// e.g. core-contribution, design, experiment, key-result, result, background.
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub flags: Vec<String>,
    /// The number that backs it, as said or found in the record.
    #[serde(default)]
    pub number: Option<String>,
    /// The author's own view on it, condensed (Chinese).
    #[serde(default)]
    pub stance: String,
    /// Where it comes from: turn numbers, experiment ids, timeline steps.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Reference keys its motive rests on.
    #[serde(default)]
    pub refs: Vec<String>,
    /// The pending result it stands for (items of the 待補項目 group).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<String>,
    /// 預想選項: possible answers prepared from the record before the interview.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<super::candidates::Candidate>,
    /// What the record lacks for more candidates (said by the model).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub candidates_missing: String,
    /// Candidates the author chose (their answer may edit or combine them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<String>,
    /// Candidates were asked for once (kept or not): never again on their own,
    /// only when the author asks (重試).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub candidates_tried: bool,
    /// 這題不用寫進論文: never asked again, never in the ledger, the claims or the draft.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub excluded: bool,
}

impl Graded for Item {
    fn role(&self) -> &str {
        &self.role
    }
    fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }
    fn depth(&self) -> Option<usize> {
        self.depth
    }
    fn has_flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    fn has_number(&self) -> bool {
        self.number.as_deref().is_some_and(|n| !n.trim().is_empty())
    }
}

/// Step 4.5: every claim once, with where it is first said in full.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Claim {
    /// Given by the code (c1, c2, …), not by the model.
    #[serde(default)]
    pub id: String,
    pub zh: String,
    #[serde(default)]
    pub en: String,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub reader_accepts: String,
    #[serde(default)]
    pub twist: String,
    /// Section where it is said in full; elsewhere only called back.
    #[serde(default)]
    pub home: String,
    #[serde(default)]
    pub items: Vec<String>,
    /// Reference keys that back it or that it builds on.
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChainLink {
    pub group: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub follows: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Suggestion {
    pub id: String,
    /// move | split | merge | resize | table | other
    pub kind: String,
    #[serde(default)]
    pub groups: Vec<String>,
    pub detail: String,
    #[serde(default)]
    pub reason: String,
    /// None = not decided yet.
    #[serde(default)]
    pub accepted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Review {
    pub chain: Vec<ChainLink>,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutlinePara {
    pub id: String,
    /// Paragraph type from the author's style, e.g. "Methods custom decision".
    #[serde(default)]
    pub kind: String,
    /// Unit plan, e.g. "fact → L2 reason → consequence".
    #[serde(default)]
    pub plan: String,
    /// What the paragraph says, in Chinese, for discussion.
    pub zh: String,
    /// The same as English points (one per line), for the phrase bank.
    #[serde(default)]
    pub en: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    /// Reference keys to cite in this paragraph.
    #[serde(default)]
    pub cites: Vec<String>,
    /// Pending results (P1, …) the paragraph reports or depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    /// Planned figures and tables (labels) the paragraph presents ([`super::figures`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutlineSection {
    pub section: String,
    #[serde(default)]
    pub subsection: Option<String>,
    pub paras: Vec<OutlinePara>,
    /// The author agreed with this section's outline.
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DraftPara {
    pub para: String,
    pub text: String,
    #[serde(default)]
    pub problems: Vec<String>,
    /// Pending results marked in it (`\pending…{P1}`): what depends on what.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DraftSection {
    pub section: String,
    pub paras: Vec<DraftPara>,
    /// Symbols the drafts use that the notation registry does not have.
    #[serde(default)]
    pub new_symbols: Vec<String>,
    /// The engine that wrote it (`本機`, `Claude`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine: String,
    /// 放進論文 wrote it into the paper (a new 生成 clears it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub placed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Finding {
    pub id: String,
    /// contradiction | overclaim | generic | missing-number | repeat | buried-origin | other
    pub kind: String,
    #[serde(default)]
    pub para: String,
    #[serde(default)]
    pub quote: String,
    pub why: String,
    /// A question to put back to the author, when the fix needs them.
    #[serde(default)]
    pub ask: String,
    #[serde(default)]
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Session {
    #[serde(default)]
    pub lens: String,
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub hook: Hook,
    #[serde(default)]
    pub records: Vec<RecordFile>,
    #[serde(default)]
    pub record_summary: String,
    #[serde(default)]
    pub experiments: Vec<Experiment>,
    #[serde(default)]
    pub timeline: Vec<TimelineStep>,
    #[serde(default)]
    pub thoughts: Vec<Thought>,
    /// Results tables built from the record.
    #[serde(default)]
    pub tables: Vec<ResultTable>,
    /// Where the record and the paper's own tables disagree.
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub review: Option<Review>,
    #[serde(default)]
    pub outline: Vec<OutlineSection>,
    #[serde(default)]
    pub drafts: Vec<DraftSection>,
    #[serde(default)]
    pub audit: Vec<Finding>,
    /// 修訂 requests on the existing paper.
    #[serde(default)]
    pub revisions: Vec<super::revise::Revision>,
    /// 待補: results the paper still waits for ([`super::pending`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<super::pending::PendingItem>,
    /// Which engine produced each step's result (`record`, `plan`, `claims`,
    /// `review`, `outline`, `audit`), shown next to the step's button.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub engines: BTreeMap<String, EngineUsed>,
    /// 深度討論: talks about one question, kept apart from the interview — never material
    /// for the paper; only an answer the author confirms goes back through the interview.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discussions: Vec<super::discuss::Discussion>,
    /// 圖表規劃 ([`super::figures`]): proposed with the question groups, kept by the author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figures: Option<super::figures::FigurePlan>,
    /// 寫法筆記: how the author said to write, from the interview ([`super::notes`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writing_notes: Vec<super::notes::WritingNote>,
}

/// The engine that ran a step: a short label (`Opus`, `Sonnet`, `本機`) and,
/// when it was not the first choice, why (for a tooltip).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EngineUsed {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The phase a step key in [`Session::engines`] belongs to.
pub fn engine_step_phase(step: &str) -> Phase {
    match step {
        "record" => Phase::Record,
        "plan" => Phase::Plan,
        "claims" => Phase::Claims,
        "review" => Phase::Review,
        "outline" => Phase::Outline,
        "draft" => Phase::Draft,
        _ => Phase::Audit,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    Ai,
    Me,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    pub n: usize,
    pub group: String,
    pub who: Who,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// The engine that asked (AI turns), e.g. `Haiku`, `本機`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine: String,
    /// The item the question is about (AI turns): its candidates are the choices.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// Candidates the author chose for this answer (`i3:k2`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<String>,
}

/// Changes the model proposes for one item (by id, or a new one by label).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ItemUpdate {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default)]
    pub stance: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub refs: Vec<String>,
}

/// One item's standing in a group, for the UI and the next prompt.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemState {
    pub id: String,
    pub missing: Vec<Missing>,
    /// A deeper level worth asking for, never required.
    pub preferred: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupState {
    pub group: String,
    pub items: Vec<ItemState>,
    pub summary_complete: bool,
    /// Every item done and the summary written.
    pub ready: bool,
    /// The lens's turn budget is spent.
    pub over_budget: bool,
}

pub fn draft_dir(paper: &Path) -> PathBuf {
    crate::paper::app_dir(paper).join(DRAFT_DIR)
}

impl Session {
    pub fn load(paper: &Path) -> Result<Session, String> {
        Session::load_file(&draft_dir(paper).join(SESSION_FILE))
    }

    /// A session file anywhere (an archived one); missing = a new session.
    pub fn load_file(path: &Path) -> Result<Session, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| format!("draft/session.json: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Session::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Write through a temporary file so a crash never leaves half a session.
    pub fn save(&self, paper: &Path) -> Result<(), String> {
        let dir = draft_dir(paper);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(".session.json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, json + "\n").map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join(SESSION_FILE)).map_err(|e| e.to_string())
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    pub fn items_of<'a>(&'a self, group: &'a str) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |i| i.group == group)
    }

    fn next_id(&self, prefix: &str, existing: impl Iterator<Item = String>) -> String {
        let n = existing.filter_map(|id| id.strip_prefix(prefix)?.parse::<usize>().ok()).max().unwrap_or(0);
        format!("{prefix}{}", n + 1)
    }

    pub fn new_item_id(&self) -> String {
        self.next_id("i", self.items.iter().map(|i| i.id.clone()))
    }

    /// Replace the plan's groups and items (step 3). Items keep no answers yet.
    pub fn set_plan(&mut self, groups: Vec<(Group, Vec<Item>)>) {
        self.groups.clear();
        self.items.clear();
        self.active = None;
        for (gi, (mut g, items)) in groups.into_iter().enumerate() {
            g.id = format!("g{}", gi + 1);
            g.status = GroupStatus::Pending;
            g.turns = 0;
            for mut it in items {
                it.id = self.new_item_id();
                it.group = g.id.clone();
                self.items.push(it);
            }
            self.groups.push(g);
        }
    }

    /// Apply the model's grading of the latest answer to a group's items.
    /// Unknown modes are dropped (the lens decides what modes exist), depths
    /// are clamped to the scale, flags must be the lens's own.
    /// `known_refs` are the paper's reference keys; others are dropped.
    pub fn apply(&mut self, lens: &Lens, group: &str, known_refs: &[String], updates: Vec<ItemUpdate>) {
        let max_depth = lens.scale.levels.len() - 1;
        for u in updates {
            let pos = u.id.as_deref().and_then(|id| self.items.iter().position(|i| i.id == id && i.group == group));
            let idx = match pos {
                Some(p) => p,
                None => {
                    let Some(label) = u.label.clone().filter(|l| !l.trim().is_empty()) else { continue };
                    let id = self.new_item_id();
                    self.items.push(Item { id, group: group.into(), label, ..Default::default() });
                    self.items.len() - 1
                }
            };
            let it = &mut self.items[idx];
            if let Some(l) = u.label.filter(|l| !l.trim().is_empty()) {
                it.label = l;
            }
            if let Some(r) = u.role.filter(|r| !r.trim().is_empty()) {
                it.role = r;
            }
            if let Some(m) = u.mode.filter(|m| lens.mode(m).is_some()) {
                it.mode = Some(m);
            }
            if let Some(d) = u.depth {
                it.depth = Some(d.min(max_depth));
            }
            for f in u.flags {
                if lens.flags.iter().any(|x| x.id == f) && !it.flags.contains(&f) {
                    it.flags.push(f);
                }
            }
            if let Some(n) = u.number.filter(|n| !n.trim().is_empty()) {
                it.number = Some(n);
            }
            if let Some(s) = u.stance.filter(|s| !s.trim().is_empty()) {
                it.stance = s;
            }
            for e in u.evidence {
                if !it.evidence.contains(&e) {
                    it.evidence.push(e);
                }
            }
            for r in u.refs {
                if known_refs.contains(&r) && !it.refs.contains(&r) {
                    it.refs.push(r);
                }
            }
        }
    }

    pub fn group_state(&self, lens: &Lens, group: &str) -> GroupState {
        let items: Vec<ItemState> = self
            .items_of(group)
            .map(|i| match i.pending.as_deref().filter(|_| i.role == super::pending::ROLE) {
                // left out of the paper by the author: nothing to ask
                _ if i.excluded => ItemState { id: i.id.clone(), missing: vec![], preferred: None },
                // a pending result is done when the author explained it
                Some(p) => {
                    let done = self.pending.iter().find(|x| x.id == p).is_none_or(|x| x.note.complete());
                    ItemState { id: i.id.clone(), missing: if done { vec![] } else { vec![Missing::Explain] }, preferred: None }
                }
                None => ItemState { id: i.id.clone(), missing: lens.missing(i), preferred: lens.preferred(i) },
            })
            .collect();
        let g = self.group(group);
        // 待補項目 has no summary of its own: it is done when every result is explained
        let summary_complete = if group == super::pending::GROUP_ID { items.iter().all(|i| i.missing.is_empty()) } else { g.is_some_and(|g| g.summary.complete()) };
        let ready = summary_complete && items.iter().all(|i| i.missing.is_empty());
        let over_budget = g.is_some_and(|g| g.turns >= lens.stop.max_turns_per_group);
        GroupState { group: group.into(), items, summary_complete, ready, over_budget }
    }

    /// The group to talk about next: the active one, else the first pending.
    pub fn current_group(&self) -> Option<&Group> {
        self.active
            .as_deref()
            .and_then(|a| self.group(a))
            .filter(|g| g.status == GroupStatus::Active)
            .or_else(|| self.groups.iter().find(|g| g.status == GroupStatus::Pending))
    }

    pub fn start_group(&mut self, id: &str) -> bool {
        if self.group(id).is_none() {
            return false;
        }
        for g in &mut self.groups {
            if g.status == GroupStatus::Active {
                g.status = GroupStatus::Pending;
            }
        }
        if let Some(g) = self.group_mut(id) {
            g.status = GroupStatus::Active;
        }
        self.active = Some(id.into());
        true
    }

    pub fn close_group(&mut self, id: &str, skipped: bool) {
        if let Some(g) = self.group_mut(id) {
            g.status = if skipped { GroupStatus::Skipped } else { GroupStatus::Done };
        }
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
    }

    /// All groups done or skipped.
    pub fn interview_finished(&self) -> bool {
        !self.groups.is_empty() && self.groups.iter().all(|g| matches!(g.status, GroupStatus::Done | GroupStatus::Skipped))
    }

    /// 從這步重來: back to `to`, clearing what every later step produced.
    /// `to`'s own results stay (its inputs too: the uploaded records, the
    /// hook, the lens). Groups opened by the reviewer pass (`a…`) and by
    /// 修訂 (`v…`) belong to those steps and go with them. Returns the groups
    /// whose conversation is gone (their turns are dropped from the transcript).
    pub fn rewind(&mut self, to: Phase) -> RewindGone {
        let before: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        if to < Phase::Record {
            self.record_summary.clear();
            self.experiments.clear();
            self.timeline.clear();
            self.thoughts.clear();
            self.tables.clear();
            self.conflicts.clear();
        }
        if to < Phase::Plan {
            self.groups.clear();
            self.items.clear();
            self.figures = None;
        }
        // Later steps' own groups.
        self.revisions.clear();
        for p in &mut self.pending {
            p.revision = None;
            if to < Phase::Interview {
                p.note = Default::default();
            }
        }
        let audit_too = to < Phase::Audit;
        self.groups.retain(|g| !(g.id.starts_with('v') || audit_too && g.id.starts_with('a')));
        let keep: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        self.items.retain(|i| keep.contains(&i.group));
        let all_turns = to < Phase::Interview;
        if all_turns {
            for g in &mut self.groups {
                g.status = GroupStatus::Pending;
                g.turns = 0;
                g.summary = GroupSummary::default();
            }
            for i in &mut self.items {
                // the plan's own fields stay: the pending result it stands for, the options prepared from the record
                *i = Item {
                    id: i.id.clone(),
                    group: i.group.clone(),
                    label: i.label.clone(),
                    role: i.role.clone(),
                    pending: i.pending.clone(),
                    candidates: std::mem::take(&mut i.candidates),
                    candidates_missing: std::mem::take(&mut i.candidates_missing),
                    candidates_tried: i.candidates_tried,
                    ..Default::default()
                };
            }
        }
        for g in &mut self.groups {
            if g.prep == Some(super::candidates::Prep::Running) {
                g.prep = None;
            }
        }
        if all_turns || self.active.as_ref().is_some_and(|a| !keep.contains(a)) {
            self.active = None;
        }
        // a discussion belongs to its question: gone with the conversation it is about
        self.discussions.retain(|d| !all_turns && keep.contains(&d.group));
        // the notes come from the conversation: they go with the groups they were said in
        self.writing_notes.retain(|n| n.kept || !all_turns && keep.contains(&n.group));
        if to < Phase::Claims {
            self.claims.clear();
            for it in self.figures.iter_mut().flat_map(|p| p.items.iter_mut()) {
                it.claims.clear();
                it.warnings.retain(|w| !w.starts_with("沒有主張"));
            }
        }
        if let Some(p) = self.figures.as_mut() {
            p.running = false;
        }
        if to < Phase::Review {
            self.review = None;
        }
        if to < Phase::Outline {
            self.outline.clear();
        }
        if to < Phase::Draft {
            self.drafts.clear();
        }
        if to < Phase::Audit {
            self.audit.clear();
        }
        self.engines.retain(|k, _| engine_step_phase(k) <= to);
        self.phase = to;
        let groups = before.into_iter().filter(|g| all_turns || !keep.contains(g)).collect();
        RewindGone { all_turns, groups }
    }
}

/// What a rewind removed from the conversation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RewindGone {
    /// Every turn (back before the interview).
    pub all_turns: bool,
    /// Groups whose turns go.
    pub groups: Vec<String>,
}

/// A new, empty `draft/archive/<UTC time stamp>/` folder.
pub fn new_archive(paper: &Path) -> Result<PathBuf, String> {
    let base = draft_dir(paper).join(ARCHIVE_DIR);
    let stamp = stamp(std::time::SystemTime::now());
    let mut dir = base.join(&stamp);
    let mut k = 2;
    while dir.exists() {
        dir = base.join(format!("{stamp}-{k}"));
        k += 1;
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// `20261109-153012` (UTC) for a moment.
pub fn stamp(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// 重新開始: move everything in `draft/` (session, transcript, records)
/// into a new archive folder; the next load starts a fresh session.
pub fn restart(paper: &Path) -> Result<PathBuf, String> {
    let dir = draft_dir(paper);
    let to = new_archive(paper)?;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        if e.file_name() == ARCHIVE_DIR {
            continue;
        }
        std::fs::rename(e.path(), to.join(e.file_name())).map_err(|e| e.to_string())?;
    }
    Ok(to)
}

/// 從這步重來 on disk: a copy of the session and transcript goes to a new
/// archive folder first, then the session is rewound and the transcript
/// keeps only the conversation that is still there.
pub fn rewind(paper: &Path, to: Phase) -> Result<Session, String> {
    let mut sess = Session::load(paper)?;
    let snap = new_archive(paper)?;
    for f in [SESSION_FILE, TRANSCRIPT_FILE] {
        let from = draft_dir(paper).join(f);
        if from.is_file() {
            std::fs::copy(&from, snap.join(f)).map_err(|e| e.to_string())?;
        }
    }
    let gone = sess.rewind(to);
    if gone.all_turns || !gone.groups.is_empty() {
        let kept: Vec<Turn> = if gone.all_turns { vec![] } else { read_turns(paper).into_iter().filter(|t| !gone.groups.contains(&t.group)).collect() };
        let mut text = String::new();
        for t in &kept {
            text += &serde_json::to_string(t).map_err(|e| e.to_string())?;
            text.push('\n');
        }
        let path = draft_dir(paper).join(TRANSCRIPT_FILE);
        if path.exists() {
            std::fs::write(&path, text).map_err(|e| e.to_string())?;
        }
    }
    sess.save(paper)?;
    Ok(sess)
}

/// Append one turn to the transcript; its number is the line count + 1.
pub fn append_turn(paper: &Path, group: &str, who: Who, text: &str, options: Vec<String>) -> Result<Turn, String> {
    append_turn_by(paper, group, who, text, options, "")
}

/// [`append_turn`] noting the engine that wrote it.
pub fn append_turn_by(paper: &Path, group: &str, who: Who, text: &str, options: Vec<String>, engine: &str) -> Result<Turn, String> {
    append_turn_with(paper, Turn { n: 0, group: group.into(), who, text: text.into(), options, engine: engine.into(), about: String::new(), picked: vec![] })
}

/// Take turn `n` out of the interview (a question back moves to 深度討論, never an answer).
pub fn remove_turn(paper: &Path, n: usize) -> Result<(), String> {
    let mut turns = read_turns(paper);
    let before = turns.len();
    turns.retain(|t| t.n != n);
    if turns.len() == before {
        return Ok(());
    }
    let mut text = String::new();
    for t in &turns {
        text += &serde_json::to_string(t).map_err(|e| e.to_string())?;
        text.push('\n');
    }
    let path = draft_dir(paper).join(TRANSCRIPT_FILE);
    let tmp = draft_dir(paper).join(".transcript.jsonl.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Append `t` (its number is given here).
pub fn append_turn_with(paper: &Path, mut t: Turn) -> Result<Turn, String> {
    let dir = draft_dir(paper);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // after the max, not the count: a rewind may have dropped turns in between
    t.n = read_turns(paper).iter().map(|t| t.n).max().unwrap_or(0) + 1;
    let mut line = serde_json::to_string(&t).map_err(|e| e.to_string())?;
    line.push('\n');
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(TRANSCRIPT_FILE))
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| e.to_string())?;
    Ok(t)
}

/// Every turn so far (unreadable lines are skipped, never fatal).
pub fn read_turns(paper: &Path) -> Vec<Turn> {
    std::fs::read_to_string(draft_dir(paper).join(TRANSCRIPT_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Store an uploaded record under `records/`, with a safe, unique name.
pub fn save_record(paper: &Path, name: &str, data: &[u8]) -> Result<RecordFile, String> {
    let dir = draft_dir(paper).join(RECORDS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let clean: String = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("record.txt")
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let clean = clean.trim_start_matches('.').to_string();
    let clean = if clean.is_empty() { "record.txt".to_string() } else { clean };
    let (stem, ext) = match clean.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (clean.clone(), String::new()),
    };
    let mut file = clean.clone();
    let mut k = 2;
    while dir.join(&file).exists() {
        file = format!("{stem}-{k}{ext}");
        k += 1;
    }
    std::fs::write(dir.join(&file), data).map_err(|e| e.to_string())?;
    Ok(RecordFile { file, bytes: data.len() })
}

/// The text of every record, each under its file name (for the prompt).
pub fn records_text(paper: &Path, files: &[RecordFile], max_chars: usize) -> String {
    let mut out = String::new();
    for f in files {
        let Ok(bytes) = std::fs::read(draft_dir(paper).join(RECORDS_DIR).join(&f.file)) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        out += &format!("=== {} ===\n{}\n\n", f.file, text.trim());
    }
    if out.chars().count() > max_chars {
        let cut: String = out.chars().take(max_chars).collect();
        return cut + "\n[… record truncated …]\n";
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens() -> Lens {
        Lens::parse(
            r#"
name = "t"
[scale]
label = "d"
levels = [{ id = "L0", label = "a" }, { id = "L1", label = "b" }, { id = "L2", label = "c" }]
[[flag]]
id = "origin"
label = "o"
[[mode]]
id = "comply"
label = "c"
ask = "confirm"
[[mode]]
id = "custom"
label = "x"
min_depth = 2
[[target]]
role = "core-contribution"
min_depth = 2
flags = ["origin"]
[stop]
max_turns_per_group = 2
"#,
        )
        .unwrap()
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-draft-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn planned() -> Session {
        let mut s = Session::default();
        let g = Group { section: "Methods".into(), title: "Loss".into(), ..Default::default() };
        let items = vec![
            Item { label: "BCE".into(), role: "design".into(), ..Default::default() },
            Item { label: "fuzzy weight".into(), role: "core-contribution".into(), ..Default::default() },
        ];
        s.set_plan(vec![(g, items)]);
        s
    }

    #[test]
    fn grading_moves_items_to_done() {
        let l = lens();
        let mut s = planned();
        assert_eq!(s.groups[0].id, "g1");
        assert_eq!(s.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["i1", "i2"]);
        let st = s.group_state(&l, "g1");
        assert!(!st.ready && st.items.iter().all(|i| i.missing == vec![Missing::Mode]));

        // "BCE for two classes, no plan to try others" → routine, done.
        // The core contribution gets a reason but not yet its origin; an
        // unknown mode and flag are ignored, depth is clamped.
        s.apply(&l, "g1", &[], vec![
            ItemUpdate { id: Some("i1".into()), mode: Some("comply".into()), ..Default::default() },
            ItemUpdate { id: Some("i2".into()), mode: Some("custom".into()), depth: Some(9), flags: vec!["fame".into()], ..Default::default() },
            ItemUpdate { id: Some("i2".into()), mode: Some("vibes".into()), ..Default::default() },
        ]);
        assert_eq!(s.items[1].mode.as_deref(), Some("custom"));
        assert_eq!(s.items[1].depth, Some(2));
        assert!(s.items[1].flags.is_empty());
        let st = s.group_state(&l, "g1");
        assert!(st.items[0].missing.is_empty());
        assert_eq!(st.items[1].missing, vec![Missing::Flag("origin".into())]);

        s.apply(&l, "g1", &[], vec![ItemUpdate { id: Some("i2".into()), flags: vec!["origin".into()], stance: Some("看到醫師的分級方式才想到".into()), ..Default::default() }]);
        assert!(!s.group_state(&l, "g1").ready, "items done but no summary yet");
        s.groups[0].summary = GroupSummary { explain: "a".into(), motive: "b".into(), conclusion: "c".into(), ..Default::default() };
        assert!(s.group_state(&l, "g1").ready);
    }

    #[test]
    fn only_the_papers_references_are_kept() {
        let l = lens();
        let mut s = planned();
        let known = vec!["kim2019".to_string()];
        s.apply(&l, "g1", &known, vec![ItemUpdate { id: Some("i2".into()), refs: vec!["kim2019".into(), "invented2024".into()], ..Default::default() }]);
        assert_eq!(s.items[1].refs, ["kim2019"]);
        let t = ResultTable { columns: vec!["Method".into(), "Dice".into()], rows: vec![vec!["Fuzzy".into(), "0.720".into()]], ..Default::default() };
        assert_eq!(t.tsv(), "Method\tDice\nFuzzy\t0.720");
    }

    #[test]
    fn new_items_come_from_answers() {
        let l = lens();
        let mut s = planned();
        s.apply(&l, "g1", &[], vec![ItemUpdate { label: Some("pretrained weights".into()), mode: Some("comply".into()), ..Default::default() }]);
        assert_eq!(s.items.last().unwrap().id, "i3");
        assert_eq!(s.items.last().unwrap().group, "g1");
        s.apply(&l, "g1", &[], vec![ItemUpdate { id: Some("nope".into()), ..Default::default() }]);
        assert_eq!(s.items.len(), 3, "an update with an unknown id and no label is dropped");
    }

    #[test]
    fn groups_open_close_and_finish() {
        let l = lens();
        let mut s = planned();
        assert_eq!(s.current_group().unwrap().id, "g1");
        assert!(s.start_group("g1"));
        s.groups[0].turns = 2;
        assert!(s.group_state(&l, "g1").over_budget);
        s.close_group("g1", true);
        assert!(s.interview_finished());
        assert!(s.current_group().is_none());
    }

    #[test]
    fn session_and_transcript_round_trip() {
        let p = tmp("rt");
        assert_eq!(Session::load(&p).unwrap(), Session::default());
        let mut s = planned();
        s.phase = Phase::Interview;
        s.save(&p).unwrap();
        assert_eq!(Session::load(&p).unwrap(), s);
        append_turn(&p, "g1", Who::Ai, "為什麼用 BCE？", vec!["常規".into(), "有特別原因".into()]).unwrap();
        let t = append_turn(&p, "g1", Who::Me, "二分類，沒打算試別的", vec![]).unwrap();
        assert_eq!(t.n, 2);
        assert_eq!(read_turns(&p).len(), 2);
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn an_old_session_file_still_loads() {
        // written before the template step existed: no "template" phase, and
        // fields added since then are missing
        let p = tmp("old");
        std::fs::create_dir_all(p.join(DRAFT_DIR)).unwrap();
        let old = r#"{"lens":"default","phase":"outline","hook":{"reader_accepts":"a","twist":"b"},"records":[],"experiments":[],"timeline":[],
"groups":[{"id":"g1","section":"Methods","title":"Loss","status":"done","turns":2}],"items":[],"claims":[{"id":"c1","zh":"主張"}],
"outline":[{"section":"Methods","paras":[{"id":"p1","zh":"段落"}]}],"drafts":[],"audit":[]}"#;
        std::fs::write(p.join(DRAFT_DIR).join(SESSION_FILE), old).unwrap();
        let s = Session::load(&p).unwrap();
        assert_eq!(s.phase, Phase::Outline);
        assert!(s.phase > Phase::Template && Phase::Template > Phase::Review);
        assert_eq!(s.groups[0].status, GroupStatus::Done);
        assert_eq!(s.outline[0].paras[0].id, "p1");
        assert!(s.revisions.is_empty() && s.tables.is_empty());
        assert!(s.pending.is_empty() && s.items.is_empty() && s.groups[0].prep.is_none(), "待補 and 預想選項 came later");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("pending") && !json.contains("candidates") && !json.contains("prep"), "nothing new is written until used");
        assert_eq!(serde_json::to_string(&Phase::Template).unwrap(), "\"template\"");
        let _ = std::fs::remove_dir_all(&p);
    }

    /// A session that went all the way to the reviewer pass, with a
    /// reviewer group and a revision group.
    fn finished(p: &Path) -> Session {
        let l = lens();
        let mut s = planned();
        s.hook.twist = "twist".into();
        s.records.push(RecordFile { file: "log.md".into(), bytes: 3 });
        s.experiments.push(Experiment { id: "E1".into(), title: "run".into(), ..Default::default() });
        s.apply(&l, "g1", &[], vec![ItemUpdate { id: Some("i1".into()), mode: Some("comply".into()), stance: Some("常規".into()), ..Default::default() }]);
        s.start_group("g1");
        s.groups[0].turns = 1;
        s.groups[0].summary.explain = "e".into();
        s.close_group("g1", false);
        append_turn(p, "g1", Who::Ai, "為什麼？", vec![]).unwrap();
        append_turn(p, "g1", Who::Me, "因為", vec![]).unwrap();
        s.claims.push(Claim { id: "c1".into(), zh: "主張".into(), ..Default::default() });
        s.review = Some(Review::default());
        s.outline.push(OutlineSection { section: "Methods".into(), paras: vec![OutlinePara { id: "p1".into(), zh: "x".into(), ..Default::default() }], ..Default::default() });
        s.drafts.push(DraftSection { section: "Methods".into(), paras: vec![], new_symbols: vec![], engine: String::new(), ..Default::default() });
        s.audit.push(Finding { id: "f1".into(), kind: "generic".into(), why: "w".into(), ..Default::default() });
        for gid in ["af1", "v1"] {
            s.groups.push(Group { id: gid.into(), section: "Results".into(), title: gid.into(), ..Default::default() });
            s.items.push(Item { id: s.new_item_id(), group: gid.into(), label: gid.into(), ..Default::default() });
            append_turn(p, gid, Who::Ai, "追問", vec![]).unwrap();
        }
        s.revisions.push(super::super::revise::Revision { id: "v1".into(), request: "r".into(), group: "v1".into(), ..Default::default() });
        s.phase = Phase::Audit;
        s.save(p).unwrap();
        s
    }

    #[test]
    fn rewinding_clears_only_later_steps() {
        let p = tmp("rw");
        let s = finished(&p);
        // back to 6 大綱: the outline stays, drafts, the reviewer pass and 修訂 go
        let r = rewind(&p, Phase::Outline).unwrap();
        assert_eq!(r.phase, Phase::Outline);
        assert_eq!(r.outline, s.outline);
        assert!(r.drafts.is_empty() && r.audit.is_empty() && r.revisions.is_empty());
        assert_eq!(r.claims, s.claims);
        assert_eq!(r.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["g1"]);
        assert!(r.items.iter().all(|i| i.group == "g1"));
        assert_eq!(read_turns(&p).iter().map(|t| t.group.as_str()).collect::<Vec<_>>(), ["g1", "g1"]);
        assert_eq!(Session::load(&p).unwrap(), r);
        // a snapshot of what was there before
        let snaps: Vec<_> = std::fs::read_dir(p.join(DRAFT_DIR).join(ARCHIVE_DIR)).unwrap().flatten().collect();
        assert_eq!(snaps.len(), 1);
        assert_eq!(Session::load_file(&snaps[0].path().join(SESSION_FILE)).unwrap(), s);
        // the next turn is numbered after the ones kept (never a number still in use)
        assert_eq!(append_turn(&p, "g1", Who::Ai, "再問", vec![]).unwrap().n, 3);

        // back to 3 題組: the plan stays, the conversation goes
        let r = rewind(&p, Phase::Plan).unwrap();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].status, GroupStatus::Pending);
        assert_eq!((r.groups[0].turns, r.groups[0].summary.clone()), (0, GroupSummary::default()));
        assert_eq!(r.items.iter().map(|i| (i.label.as_str(), i.mode.is_none(), i.stance.is_empty())).collect::<Vec<_>>(), [("BCE", true, true), ("fuzzy weight", true, true)]);
        assert!(r.claims.is_empty() && r.review.is_none() && r.outline.is_empty());
        assert!(read_turns(&p).is_empty());
        assert_eq!(r.experiments, s.experiments, "step 2 is before the target");

        // back to 1 主題: the hook and the uploaded records stay, the rest goes
        let r = rewind(&p, Phase::Topic).unwrap();
        assert_eq!(r.hook, s.hook);
        assert_eq!(r.records, s.records);
        assert!(r.experiments.is_empty() && r.groups.is_empty() && r.items.is_empty());
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn restarting_archives_the_whole_session() {
        let p = tmp("rs");
        let s = finished(&p);
        save_record(&p, "log.md", b"run 1").unwrap();
        let to = restart(&p).unwrap();
        assert!(to.starts_with(p.join(DRAFT_DIR).join(ARCHIVE_DIR)));
        assert_eq!(Session::load(&p).unwrap(), Session::default());
        assert!(read_turns(&p).is_empty());
        assert_eq!(Session::load_file(&to.join(SESSION_FILE)).unwrap(), s);
        assert!(to.join(TRANSCRIPT_FILE).is_file() && to.join(RECORDS_DIR).join("log.md").is_file());
        // a second restart keeps the first archive
        Session::default().save(&p).unwrap();
        let again = restart(&p).unwrap();
        assert_ne!(again, to);
        assert!(to.join(SESSION_FILE).is_file());
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn time_stamps_are_utc_dates() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_227_012);
        assert_eq!(stamp(t), "20261109-122332");
        assert_eq!(stamp(std::time::UNIX_EPOCH + std::time::Duration::from_secs(951_868_799)), "20000229-235959");
        assert_eq!(stamp(std::time::UNIX_EPOCH), "19700101-000000");
    }

    #[test]
    fn records_get_safe_unique_names() {
        let p = tmp("rec");
        let a = save_record(&p, "../../exp log.md", b"run 1: dice 0.70").unwrap();
        let b = save_record(&p, "exp-log.md", b"run 2").unwrap();
        assert_eq!(a.file, "exp-log.md");
        assert_eq!(b.file, "exp-log-2.md");
        let text = records_text(&p, &[a, b], 1000);
        assert!(text.contains("run 1") && text.contains("=== exp-log-2.md ==="));
        assert!(records_text(&p, &[RecordFile { file: "exp-log.md".into(), bytes: 0 }], 5).contains("truncated"));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn rewinding_forgets_the_engines_of_later_steps() {
        let mut s = Session::default();
        for k in ["record", "plan", "claims", "outline", "audit"] {
            s.engines.insert(k.into(), EngineUsed { label: "本機".into(), note: String::new() });
        }
        s.rewind(Phase::Plan);
        assert_eq!(s.engines.keys().map(String::as_str).collect::<Vec<_>>(), ["plan", "record"]);
        let json = serde_json::to_string(&Session::default()).unwrap();
        assert!(!json.contains("engines"), "nothing written until a step has run");
    }
}
