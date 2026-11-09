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
use std::path::{Path, PathBuf};

pub const DRAFT_DIR: &str = "draft";
const SESSION_FILE: &str = "session.json";
const TRANSCRIPT_FILE: &str = "transcript.jsonl";
pub const RECORDS_DIR: &str = "records";
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DraftSection {
    pub section: String,
    pub paras: Vec<DraftPara>,
    /// Symbols the drafts use that the notation registry does not have.
    #[serde(default)]
    pub new_symbols: Vec<String>,
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
    paper.join(DRAFT_DIR)
}

impl Session {
    pub fn load(paper: &Path) -> Result<Session, String> {
        match std::fs::read_to_string(draft_dir(paper).join(SESSION_FILE)) {
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
            .map(|i| ItemState { id: i.id.clone(), missing: lens.missing(i), preferred: lens.preferred(i) })
            .collect();
        let g = self.group(group);
        let summary_complete = g.is_some_and(|g| g.summary.complete());
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
}

/// Append one turn to the transcript; its number is the line count + 1.
pub fn append_turn(paper: &Path, group: &str, who: Who, text: &str, options: Vec<String>) -> Result<Turn, String> {
    let dir = draft_dir(paper);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let n = read_turns(paper).len() + 1;
    let t = Turn { n, group: group.into(), who, text: text.into(), options };
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
}
