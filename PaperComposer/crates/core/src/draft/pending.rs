//! 待補: results the paper still waits for (the last experiments whose data has
//! not come in). The author marks an experiment, or cells / rows / columns of a
//! results table, as 等資料 in the record step; explains each item in the
//! interview group 「待補項目」 ([`GROUP_ID`]); the draft writes the expected
//! result in the expected direction with lore-notation's draft marks
//! (`\pending{P1}{…}`, `\pendingcell`, `\pendingpara`, `\pendingdep`), so the
//! paper source itself says where each item and every sentence that depends on
//! it are ([`locate`]); the reviewer pass leaves marks alone and opens with the
//! 待補清單 ([`report`]); a submission export is refused while marks remain
//! ([`blockers`]). When the result comes in, a 修訂 request is made from the item
//! ([`request_text`], [`turn_context`]) and the numbers are filled in by code
//! ([`fill_changes`]); only a result against the expectation needs the model,
//! and every sentence that depended on it is flagged to rewrite.

use super::revise::{Change, Revision};
use super::session::{Finding, Group, GroupStatus, Item, ResultTable, Session};
use lore_notation::pending::{self as marks, Mark, MarkKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The interview group where the author explains every pending item.
pub const GROUP_ID: &str = "wp";
pub const GROUP_TITLE: &str = "待補項目";
/// The role of the items in [`GROUP_ID`].
pub const ROLE: &str = "pending";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PendingStatus {
    /// 等資料
    #[default]
    Waiting,
    /// The result came in (a 修訂 fills it in).
    Filled,
}

/// How the result compares with what the author expected.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// As expected: only the numbers change.
    Matches,
    /// The other way: every dependent sentence is rewritten.
    Contradicts,
    /// Not sure yet: discussed in the 修訂 conversation.
    Unsure,
}

/// The author's explanation from the interview, condensed in their words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PendingNote {
    /// What it is.
    #[serde(default)]
    pub what: String,
    /// Why the paper needs it.
    #[serde(default)]
    pub why: String,
    /// What they expect.
    #[serde(default)]
    pub expect: String,
    /// What changes if it comes out the other way.
    #[serde(default)]
    pub if_not: String,
}

impl PendingNote {
    pub fn complete(&self) -> bool {
        [&self.what, &self.why, &self.expect, &self.if_not].iter().all(|s| !s.trim().is_empty())
    }
}

/// Cells of a results table that wait: a row (`col` None), a column (`row`
/// None) or one cell. Rows count data rows (0 = the first row under the header).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Hole {
    #[serde(default)]
    pub row: Option<usize>,
    #[serde(default)]
    pub col: Option<usize>,
}

/// One result the paper waits for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PendingItem {
    /// P1, P2, … (given by the code; the marks in the paper carry it).
    #[serde(default)]
    pub id: String,
    /// What is measured (metric, comparison).
    pub what: String,
    /// The data it waits for.
    #[serde(default)]
    pub waits_for: String,
    /// The expected direction / hypothesis.
    #[serde(default)]
    pub expected: String,
    /// Expected date, free text (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub due: String,
    /// The experiment it belongs to (E3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<String>,
    /// The experiment's title, to find it again after the record is read again (ids are renumbered).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub experiment_title: String,
    /// The results table its cells are in (R2), with the caption to find it
    /// again after the record is read again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub table_caption: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Hole>,
    #[serde(default)]
    pub note: PendingNote,
    #[serde(default)]
    pub status: PendingStatus,
    /// The result as the author gave it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
    /// Values per cell (`Ours / Dice` → `0.74`), and per number mark (its argument).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// The 修訂 that fills it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl PendingItem {
    /// The result as the author gave it, else its values (`Ours / Dice = 0.74; …`).
    pub fn shown_result(&self) -> String {
        if !self.result.trim().is_empty() {
            return self.result.trim().to_string();
        }
        self.values.iter().map(|(k, v)| format!("{k} = {v}")).collect::<Vec<_>>().join("; ")
    }
}

/// The next free id (P1, P2, …).
pub fn next_id(items: &[PendingItem]) -> String {
    let n = items.iter().filter_map(|i| i.id.strip_prefix('P')?.parse::<usize>().ok()).max().unwrap_or(0);
    format!("P{}", n + 1)
}

/// Give new items an id, keep what the code owns (the interview note, the
/// result, the revision) on items that exist. Items with no `what` are dropped.
pub fn merge_edit(old: &[PendingItem], edit: Vec<PendingItem>) -> Vec<PendingItem> {
    let mut out: Vec<PendingItem> = vec![];
    for mut e in edit.into_iter().filter(|e| !e.what.trim().is_empty()) {
        match old.iter().find(|o| !e.id.is_empty() && o.id == e.id) {
            Some(o) => {
                e.note = o.note.clone();
                e.status = o.status;
                e.result = o.result.clone();
                e.values = o.values.clone();
                e.direction = o.direction;
                e.revision = o.revision.clone();
            }
            None => {
                let mut all = old.to_vec();
                all.extend(out.iter().cloned());
                e.id = next_id(&all);
                e.note = PendingNote::default();
                e.status = PendingStatus::Waiting;
                e.revision = None;
            }
        }
        out.push(e);
    }
    out
}

/// Whether a table cell (data row `r`, column `c`) waits for `item`.
fn in_hole(h: &Hole, r: usize, c: usize) -> bool {
    // a row keeps its label in the first column
    h.row.is_none_or(|x| x == r) && h.col.is_none_or(|x| x == c) && !(h.col.is_none() && c == 0)
}

/// The item a table cell waits for, if any.
pub fn cell_item<'a>(t: &ResultTable, items: &'a [PendingItem], r: usize, c: usize) -> Option<&'a PendingItem> {
    items.iter().find(|i| i.status == PendingStatus::Waiting && i.table.as_deref() == Some(t.id.as_str()) && i.cells.iter().any(|h| in_hole(h, r, c)))
}

/// The name of a cell: `row label / column header`.
pub fn cell_name(t: &ResultTable, r: usize, c: usize) -> String {
    let row = t.rows.get(r).and_then(|x| x.first()).map(String::as_str).unwrap_or("");
    let col = t.columns.get(c).map(String::as_str).unwrap_or("");
    format!("{} / {}", row.trim(), col.trim()).trim_matches([' ', '/']).to_string()
}

/// The table as the table importer reads it, pending cells as `\pendingcell`.
pub fn table_tsv(t: &ResultTable, items: &[PendingItem]) -> String {
    let mut lines = vec![t.columns.join("\t")];
    for (r, row) in t.rows.iter().enumerate() {
        let cells: Vec<String> = (0..t.columns.len().max(row.len()))
            .map(|c| match cell_item(t, items, r, c) {
                Some(i) => format!("\\pendingcell{{{}}}{{{}}}", i.id, cell_name(t, r, c).replace(['{', '}', '\t'], "")),
                None => row.get(c).cloned().unwrap_or_default(),
            })
            .collect();
        lines.push(cells.join("\t"));
    }
    lines.join("\n")
}

/// After the record is read again: experiments get new ids; follow them by title.
pub fn rebind_experiments(items: &mut [PendingItem], experiments: &[super::session::Experiment]) {
    for i in items.iter_mut() {
        if i.experiment_title.is_empty() {
            if let Some(e) = i.experiment.as_deref().and_then(|e| experiments.iter().find(|x| x.id == e)) {
                i.experiment_title = e.title.clone();
            }
            continue;
        }
        i.experiment = experiments.iter().find(|x| x.title == i.experiment_title).map(|x| x.id.clone());
    }
}

/// After the record is read again: tables get new ids; follow them by caption.
pub fn rebind_tables(items: &mut [PendingItem], tables: &[ResultTable]) {
    for i in items.iter_mut() {
        if i.table_caption.is_empty() {
            continue;
        }
        match tables.iter().find(|t| t.caption == i.table_caption) {
            Some(t) => i.table = Some(t.id.clone()),
            None => {
                i.table = None;
                i.cells.clear();
            }
        }
    }
}

/// The record step's convention: a number the record does not have yet is `?`.
/// Such table cells and experiments become pending items (or join the item
/// of their experiment).
pub fn from_record(items: &mut Vec<PendingItem>, experiments: &[super::session::Experiment], tables: &[ResultTable]) {
    let unknown = |s: &str| matches!(s.trim(), "?" | "？" | "TBD" | "待補");
    for e in experiments.iter().filter(|e| unknown(&e.result)) {
        if items.iter().any(|i| i.experiment.as_deref() == Some(e.id.as_str())) {
            continue;
        }
        let id = next_id(items);
        items.push(PendingItem { id, what: format!("{} {}", e.title, e.metrics).trim().to_string(), experiment: Some(e.id.clone()), experiment_title: e.title.clone(), ..Default::default() });
    }
    for t in tables {
        let holes: Vec<Hole> = t
            .rows
            .iter()
            .enumerate()
            .flat_map(|(r, row)| row.iter().enumerate().filter(|(_, v)| unknown(v)).map(move |(c, _)| Hole { row: Some(r), col: Some(c) }))
            .collect();
        if holes.is_empty() {
            continue;
        }
        let at = items
            .iter()
            .position(|i| i.table.as_deref() == Some(t.id.as_str()))
            .or_else(|| items.iter().position(|i| i.table.is_none() && i.experiment.as_ref().is_some_and(|e| t.from.contains(e))));
        let i = match at {
            Some(k) => k,
            None => {
                // named by its waiting cells, not by the table: the table's other cells are known,
                // and an item named after the whole table made every sentence about it "dependent"
                let id = next_id(items);
                let cells: Vec<String> = holes.iter().map(|h| cell_name(t, h.row.unwrap_or(0), h.col.unwrap_or(0))).collect();
                items.push(PendingItem { id, what: format!("{} (in \"{}\")", cells.join(", "), t.caption.trim()), ..Default::default() });
                items.len() - 1
            }
        };
        let it = &mut items[i];
        it.table = Some(t.id.clone());
        it.table_caption = t.caption.clone();
        for h in holes {
            if !it.cells.contains(&h) {
                it.cells.push(h);
            }
        }
    }
}

// ── where the marks are ──────────────────────────────────

/// One mark in the paper.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Place {
    pub id: String,
    pub kind: MarkKind,
    pub arg: String,
    /// Section file (as the editor names it), or the main file for the abstract.
    pub file: String,
    /// The section's title, or `Abstract`.
    pub section: String,
    /// Paragraph index in the section; None in the main file.
    pub para: Option<usize>,
    /// The sentence the mark is in (dependents: the sentence that depends).
    pub sentence: String,
}

/// The sentence of `text` around byte `pos`.
pub fn sentence_at(text: &str, pos: usize) -> String {
    let ends = |b: &[u8], i: usize| matches!(b[i], b'.' | b'?' | b'!') && b.get(i + 1).is_none_or(|c| c.is_ascii_whitespace());
    let b = text.as_bytes();
    let mut s = pos.min(b.len());
    while s > 0 && !ends(b, s - 1) {
        s -= 1;
    }
    let mut e = pos.min(b.len());
    while e < b.len() && !ends(b, e) {
        e += 1;
    }
    let e = (e + 1).min(b.len());
    let mut s = s;
    while !text.is_char_boundary(s) {
        s -= 1;
    }
    let mut e = e;
    while !text.is_char_boundary(e) {
        e += 1;
    }
    let one: String = text[s..e].split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 320 { one.chars().take(318).collect::<String>() + "…" } else { one }
}

fn places_in(text: &str, file: &str, section: &str, para: Option<usize>) -> Vec<Place> {
    marks::marks(text)
        .into_iter()
        .map(|m: Mark| Place { sentence: sentence_at(text, m.start), id: m.id, kind: m.kind, arg: m.arg, file: file.into(), section: section.into(), para })
        .collect()
}

/// The main file's own text (front matter, abstract), its `\input`s left out.
fn main_places(paper: &Path) -> Vec<Place> {
    let main = crate::open::main_file(paper);
    let name = main.file_name().and_then(|n| n.to_str()).unwrap_or("main.tex").to_string();
    let text = std::fs::read_to_string(&main).unwrap_or_default();
    let mut out = vec![];
    for (i, block) in text.split("\n\n").enumerate() {
        let _ = i;
        let sec = if block.contains("abstract") { "Abstract" } else { "" };
        out.extend(places_in(block, &name, sec, None));
    }
    out
}

/// Every mark in the paper: the main file (abstract) first, then each section.
pub fn locate(paper: &Path) -> Vec<Place> {
    let mut out = main_places(paper);
    for s in crate::open::sections(paper) {
        let title = crate::text::strip_latex(&s.title);
        for (i, p) in s.paragraphs.iter().enumerate() {
            out.extend(places_in(p, &s.file, &title, Some(i)));
        }
    }
    out
}

/// What blocks a submission export: every mark, in paper order.
pub fn blockers(paper: &Path) -> Vec<Place> {
    locate(paper)
}

/// The marks of one paragraph, for an export's refusal (one line per paragraph).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Blocker {
    pub file: String,
    pub para: Option<usize>,
    pub message: String,
    /// The paragraph's first marked sentence, marks spelled out.
    pub context: String,
}

/// Marks grouped by paragraph: `待補 P1（數字・依賴句）：external Dice`.
pub fn blocker_groups(places: &[Place]) -> Vec<Blocker> {
    let mut out: Vec<(String, Option<usize>, Vec<&Place>)> = vec![];
    for p in places {
        // a paragraph of a section, or (the main file) the same sentence block
        let same = |f: &String, k: &Option<usize>, v: &Vec<&Place>| *f == p.file && *k == p.para && (k.is_some() || v.iter().any(|x| x.sentence == p.sentence));
        match out.iter_mut().find(|(f, k, v)| same(f, k, v)) {
            Some((_, _, v)) => v.push(p),
            None => out.push((p.file.clone(), p.para, vec![p])),
        }
    }
    out.into_iter()
        .map(|(file, para, ps)| {
            let mut ids: Vec<&str> = ps.iter().map(|p| p.id.as_str()).collect();
            ids.dedup();
            let mut kinds: Vec<&str> = vec![];
            let mut args: Vec<String> = vec![];
            for p in &ps {
                let k = kind_zh(p.kind);
                if !kinds.contains(&k) {
                    kinds.push(k);
                }
                let arg = plain_arg(&p.arg);
                if !arg.is_empty() && !args.contains(&arg) {
                    args.push(arg);
                }
            }
            let what = if args.is_empty() { String::new() } else { format!("：{}", args.join("、")) };
            // the sentence, unless it is only the planned paragraph's frame (already in the message)
            let context = ps.iter().find(|p| !matches!(p.kind, MarkKind::Cell | MarkKind::Para)).map(|p| marks::to_plain(&p.sentence)).unwrap_or_default();
            Blocker { file, para, message: format!("待補 {}（{}）{what}", ids.join("、"), kinds.join("・")), context }
        })
        .collect()
}

/// A mark's argument as words: `\\ref{tab:site}` keeps its key, other LaTeX goes.
fn plain_arg(a: &str) -> String {
    let mut t = a.to_string();
    for cmd in ["\\eqref{", "\\ref{", "\\cref{", "\\Cref{"] {
        while let Some(i) = t.find(cmd) {
            let Some(e) = t[i..].find('}') else { break };
            let key = t[i + cmd.len()..i + e].to_string();
            t.replace_range(i..i + e + 1, &key);
        }
    }
    crate::text::strip_latex(&t).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn kind_zh(k: MarkKind) -> &'static str {
    match k {
        MarkKind::Number => "數字",
        MarkKind::Cell => "表格",
        MarkKind::Para => "段落",
        MarkKind::Dep => "依賴句",
    }
}

/// One line per mark for an export's refusal.
pub fn blocker_message(p: &Place) -> String {
    let what = if p.arg.is_empty() { String::new() } else { format!("：{}", p.arg) };
    format!("待補 {}（{}）{what}", p.id, kind_zh(p.kind))
}

/// One item of the 待補清單.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReportEntry {
    pub id: String,
    pub what: String,
    pub status: PendingStatus,
    /// Numbers, cells and planned paragraphs.
    pub places: Vec<Place>,
    /// Sentences that depend on it.
    pub dependents: Vec<Place>,
    /// Marks in the paper with no item in the session (made by hand, or the
    /// session was started again).
    pub unknown: bool,
}

/// The 待補清單: each item (session order, then ids only the paper has), where
/// it appears and what depends on it — the abstract, discussion and conclusion included.
pub fn report(paper: &Path, sess: &Session) -> Vec<ReportEntry> {
    report_from(&locate(paper), sess)
}

pub fn report_from(all: &[Place], sess: &Session) -> Vec<ReportEntry> {
    let mut out: Vec<ReportEntry> = sess
        .pending
        .iter()
        .map(|i| ReportEntry { id: i.id.clone(), what: i.what.clone(), status: i.status, places: vec![], dependents: vec![], unknown: false })
        .collect();
    for p in all {
        let at = match out.iter().position(|e| e.id == p.id) {
            Some(k) => k,
            None => {
                out.push(ReportEntry { id: p.id.clone(), what: p.arg.clone(), status: PendingStatus::Waiting, places: vec![], dependents: vec![], unknown: true });
                out.len() - 1
            }
        };
        if p.kind == MarkKind::Dep { &mut out[at].dependents } else { &mut out[at].places }.push(p.clone());
    }
    out
}

/// The marks in the session's draft (before it is copied into the paper), as places.
pub fn draft_places(sess: &Session) -> Vec<Place> {
    let mut out = vec![];
    for d in &sess.drafts {
        for p in &d.paras {
            for mut x in places_in(&p.text, "", &d.section, None) {
                x.file = format!("draft:{}", p.para);
                out.push(x);
            }
        }
    }
    out
}

// ── the paper compiles with marks ────────────────────────

/// Write `pending.tex` (lore-notation's definitions) next to the main file and
/// `\input` it in the preamble when the paper has marks. True when main changed.
pub fn ensure_tex(paper: &Path) -> Result<bool, String> {
    if locate(paper).is_empty() {
        return Ok(false);
    }
    let main_path = crate::open::main_file(paper);
    let dir = main_path.parent().unwrap_or(paper);
    let file = marks::write_tex(dir).map_err(|e| e.to_string())?;
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("pending").to_string();
    let main = std::fs::read_to_string(&main_path).map_err(|e| e.to_string())?;
    let has = main.lines().map(|l| l.split('%').next().unwrap_or("")).any(|l| l.contains(&format!("\\input{{{stem}}}")) || l.contains(&format!("\\input{{{stem}.tex}}")));
    if has {
        return Ok(false);
    }
    let at = main.find("\\begin{document}").ok_or("main.tex has no \\begin{document}")?;
    crate::paper::write_atomic(&main_path, &format!("{}\\input{{{stem}}}\n{}", &main[..at], &main[at..])).map_err(|e| e.to_string())?;
    Ok(true)
}

// ── the interview ────────────────────────────────────────

/// Keep the 待補項目 group in step with the items: one item per waiting result.
/// Only once a plan exists (the group joins the other groups); a group already
/// talked about keeps its conversation.
pub fn sync_group(sess: &mut Session) {
    let waiting: Vec<&PendingItem> = sess.pending.iter().filter(|i| i.status == PendingStatus::Waiting).collect();
    let planned = sess.groups.iter().any(|g| g.id.starts_with('g'));
    if !planned || waiting.is_empty() {
        let talked = sess.group(GROUP_ID).is_some_and(|g| g.turns > 0);
        if !talked {
            sess.groups.retain(|g| g.id != GROUP_ID);
            sess.items.retain(|i| i.group != GROUP_ID);
            if sess.active.as_deref() == Some(GROUP_ID) {
                sess.active = None;
            }
        }
        if waiting.is_empty() || !planned {
            return;
        }
    }
    let ids: Vec<(String, String)> = waiting.iter().map(|i| (i.id.clone(), i.what.clone())).collect();
    if sess.group(GROUP_ID).is_none() {
        sess.groups.push(Group {
            id: GROUP_ID.into(),
            section: "Results".into(),
            title: GROUP_TITLE.into(),
            purpose: "每一項：是什麼、為什麼必要、預期結果、結果相反時要改什麼".into(),
            status: GroupStatus::Pending,
            ..Default::default()
        });
    }
    sess.items.retain(|i| i.group != GROUP_ID || i.pending.as_ref().is_some_and(|p| ids.iter().any(|(x, _)| x == p)));
    for (id, what) in ids {
        match sess.items.iter_mut().find(|i| i.group == GROUP_ID && i.pending.as_deref() == Some(id.as_str())) {
            Some(it) => it.label = format!("{id} {what}"),
            None => {
                let iid = sess.new_item_id();
                sess.items.push(Item { id: iid, group: GROUP_ID.into(), label: format!("{id} {what}"), role: ROLE.into(), pending: Some(id), ..Default::default() });
            }
        }
    }
}

/// The interview block for the 待補項目 group.
pub fn group_context(sess: &Session) -> String {
    let mut s = "\n## This group is 待補項目: results the paper still waits for\n\
The data for these is not in yet, but the paper needs them. For each item get, in the author's own words: `what` it \
is (what is measured, on what), `why` the paper needs it (which claim rests on it), `expect` (the direction and \
rough size they expect, and why), `if_not` (what in the paper changes if it comes out the other way). Ask one item \
at a time, one part at a time; offer options built from the record and the experiments when they are unsure. \
Record what they said in `pending_notes` ({id, what, why, expect, if_not}, Traditional Chinese, condensed, empty \
strings for parts not said yet; earlier parts are kept). Grade the group's ledger items `comply` once their item's \
four parts are said. A table cell or row that waits is explained like any item: which cells, and why the table \
needs them.\n"
        .to_string();
    for i in sess.pending.iter().filter(|i| i.status == PendingStatus::Waiting) {
        s += &item_line(i);
    }
    s
}

fn item_line(i: &PendingItem) -> String {
    let mut s = format!("- {} {}", i.id, i.what);
    if !i.waits_for.is_empty() {
        s += &format!("; waits for: {}", i.waits_for);
    }
    if !i.expected.is_empty() {
        s += &format!("; expected: {}", i.expected);
    }
    if let Some(e) = &i.experiment {
        s += &format!("; experiment {e}");
    }
    if let Some(t) = &i.table {
        s += &format!("; table {t}: only {} cell(s) wait, every other value of that table is known", i.cells.len());
    }
    let n = &i.note;
    if !(n.what.is_empty() && n.why.is_empty() && n.expect.is_empty() && n.if_not.is_empty()) {
        s += &format!(" | author: what={} / why={} / expect={} / if not={}", n.what, n.why, n.expect, n.if_not);
    }
    s + "\n"
}

/// Pending results for the shared context of every later step, and the ones
/// that came in (with how they compare with the expectation).
pub fn context_block(sess: &Session) -> String {
    let waiting: Vec<&PendingItem> = sess.pending.iter().filter(|i| i.status == PendingStatus::Waiting).collect();
    let filled: Vec<&PendingItem> = sess.pending.iter().filter(|i| i.status == PendingStatus::Filled).collect();
    let mut s = String::new();
    if !waiting.is_empty() {
        s += "\nPending results (the data is not in yet; the paper is drafted around them, ids are fixed):\n";
        for i in waiting {
            s += &item_line(i);
        }
    }
    if !filled.is_empty() {
        s += "\nResults that came in after the draft:\n";
        for i in filled {
            let dir = match i.direction {
                Some(Direction::Matches) => "as expected",
                Some(Direction::Contradicts) => "AGAINST the expectation",
                _ => "not yet compared",
            };
            s += &format!("- {} {}: {}{} ({dir}; expected: {})\n", i.id, i.what, i.result, if i.values.is_empty() { String::new() } else { format!(" [{}]", i.values.iter().map(|(k, v)| format!("{k} = {v}")).collect::<Vec<_>>().join("; ")) }, i.expected);
        }
    }
    s
}

/// A filled result goes back into the record: its experiment's `?` result
/// and its table cells (by cell name, else the result as given).
pub fn write_back(sess: &mut Session, id: &str) {
    let Some(i) = sess.pending.iter().find(|x| x.id == id).cloned() else { return };
    let unknown = |s: &str| matches!(s.trim(), "" | "?" | "？" | "TBD" | "待補");
    if let Some(e) = i.experiment.as_deref().and_then(|e| sess.experiments.iter_mut().find(|x| x.id == e)) {
        if unknown(&e.result) && !i.shown_result().is_empty() {
            e.result = i.shown_result();
        }
    }
    let Some(t) = i.table.as_deref().and_then(|t| sess.tables.iter_mut().find(|x| x.id == t)) else { return };
    let snapshot = t.clone();
    for r in 0..snapshot.rows.len() {
        for c in 0..snapshot.columns.len() {
            if !i.cells.iter().any(|h| in_hole(h, r, c)) {
                continue;
            }
            let v = i.values.get(&cell_name(&snapshot, r, c)).cloned().or_else(|| (i.cells.len() == 1 && i.cells[0].row.is_some() && i.cells[0].col.is_some()).then(|| i.result.trim().to_string()));
            if let (Some(v), Some(cell)) = (v.filter(|v| !v.is_empty()), t.rows.get_mut(r).and_then(|row| row.get_mut(c))) {
                *cell = v;
            }
        }
    }
}

/// Waiting items whose experiment the record now gives a result for (the
/// author re-read an updated record): indexes into `sess.pending`.
pub fn arrived(sess: &Session) -> Vec<usize> {
    let unknown = |s: &str| matches!(s.trim(), "" | "?" | "？" | "TBD" | "待補");
    sess.pending
        .iter()
        .enumerate()
        .filter(|(_, i)| i.status == PendingStatus::Waiting)
        .filter(|(_, i)| i.experiment.as_deref().and_then(|e| sess.experiments.iter().find(|x| x.id == e)).is_some_and(|e| !unknown(&e.result)))
        .map(|(k, _)| k)
        .collect()
}

/// How an item's holes read: `External 欄`, `Weighted 列`, `Weighted / Small`.
pub fn holes_text(i: &PendingItem, t: &ResultTable) -> String {
    i.cells
        .iter()
        .map(|h| match (h.row, h.col) {
            (None, Some(c)) => format!("{} 欄", t.columns.get(c).map(String::as_str).unwrap_or("?")),
            (Some(r), None) => format!("{} 列", t.rows.get(r).and_then(|x| x.first()).map(String::as_str).unwrap_or("?")),
            (Some(r), Some(c)) => cell_name(t, r, c),
            (None, None) => "整張表".into(),
        })
        .collect::<Vec<_>>()
        .join("、")
}

/// After the last mark is filled: the preamble's `\input{pending}` goes (the
/// generated file stays, harmless). True when main changed.
pub fn release_tex(paper: &Path) -> Result<bool, String> {
    if !locate(paper).is_empty() {
        return Ok(false);
    }
    let main_path = crate::open::main_file(paper);
    let Ok(main) = std::fs::read_to_string(&main_path) else { return Ok(false) };
    let kept: Vec<&str> = main.split_inclusive('\n').filter(|l| !matches!(l.trim(), "\\input{pending}" | "\\input{pending.tex}")).collect();
    let new = kept.concat();
    if new == main {
        return Ok(false);
    }
    crate::paper::write_atomic(&main_path, &new).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Outline ids a drafted paragraph should mark but does not (a pending result
/// written as a plain number would be a made-up number).
pub fn missing_marks(sess: &Session, wanted: &[String], text: &str) -> Vec<String> {
    let have = marks::ids(text);
    wanted.iter().filter(|w| sess.pending.iter().any(|p| &p.id == *w && p.status == PendingStatus::Waiting) && !have.contains(*w)).cloned().collect()
}

/// The drafting rules for pending results (paragraph prompts, outline).
pub fn draft_rules(sess: &Session, only: &[String]) -> String {
    let waiting: Vec<&PendingItem> = sess.pending.iter().filter(|i| i.status == PendingStatus::Waiting && (only.is_empty() || only.contains(&i.id))).collect();
    if waiting.is_empty() {
        return String::new();
    }
    let mut s = "Pending results (their data is not in yet). Write them as if the expected result came in — plainly, in \
the expected direction, no hedging (\"may\", \"might\", \"is expected to\") — and mark them so they are found later:\n\
- the number itself: \\pending{ID}{short English name of what is measured} (never a made-up number, never \
\"[Author to confirm]\" for these);\n\
- a sentence that states this result, compares with it or draws a conclusion from it (also in the abstract, \
discussion and conclusion): end it with \\pendingdep{ID} right before its full stop. Only those: a sentence that \
uses only known numbers (other cells of the same table, other experiments) does not depend on it, even when it is \
about the same table or experiment;\n\
- a paragraph whose whole content waits: write only \\pendingpara{ID}{frame: what the paragraph will say and which \
figure or table (\\ref{…}) it holds}.\n\
Use only these ids:\n"
        .to_string();
    for i in waiting {
        s += &item_line(i);
    }
    s
}

/// For a paragraph that depends on no pending result: never a number for one, no marks.
pub fn not_here(sess: &Session) -> String {
    let waiting: Vec<&PendingItem> = sess.pending.iter().filter(|i| i.status == PendingStatus::Waiting).collect();
    if waiting.is_empty() {
        return String::new();
    }
    let ids: Vec<String> = waiting.iter().map(|i| format!("{} {}", i.id, i.what)).collect();
    format!("Results not in yet (never write a number for them; this paragraph uses no pending marks): {}.\n", ids.join("; "))
}

/// The code's side of 「依賴句只算直接用到的」: a `\pendingdep` of an id that is not waiting
/// (filled, or unknown) or that this paragraph does not depend on (`allowed`, from the outline) is
/// removed, unless its own sentence states the value (`\pending{ID}{…}`).
///
/// Also removed: the mark of a sentence whose numbers are all values the record already has
/// (`known`): it reports known values, not the pending one ("In men, both findings differed
/// (p = 0.004)…" was marked as depending on a missing cell). The caller passes no `known` for
/// the abstract and the conclusion: a sentence there that concludes about the pending result
/// while quoting a known number must keep its mark, so a contradicting result flags it (回填).
pub fn prune_deps(sess: &Session, allowed: &[String], text: &str, known: &str) -> String {
    let waiting = |id: &str| sess.pending.iter().any(|p| p.id == id && p.status == PendingStatus::Waiting);
    // stated in the same sentence (`… is \\pending{P1}{…}, which keeps the ranking\\pendingdep{P1}.`)
    let stated_here = |m: &Mark| marks::marks(&sentence_at(text, m.start)).iter().any(|x| x.id == m.id && x.kind != MarkKind::Dep);
    let mut out = text.to_string();
    for m in marks::marks(text).into_iter().rev().filter(|m| m.kind == MarkKind::Dep) {
        let only_known = || {
            let nums = numbers(&marks::to_plain(&sentence_at(text, m.start)));
            !nums.is_empty() && nums.iter().all(|n| has_number(known, n))
        };
        let keep = waiting(&m.id) && (stated_here(&m) || allowed.contains(&m.id) && !only_known());
        if !keep {
            // "known 0.72 \\pendingdep{P1}." → "known 0.72."
            let mut start = m.start;
            while start > 0 && out.as_bytes()[start - 1] == b' ' {
                start -= 1;
            }
            out.replace_range(start..m.end, "");
        }
    }
    out
}

/// The numbers written in `s` (`0.078`, `5,159`, `93.31`), with no unit.
pub fn numbers(s: &str) -> Vec<String> {
    let b: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_digit() || matches!(b[j], '.' | ',') && b.get(j + 1).is_some_and(|c| c.is_ascii_digit())) {
                j += 1;
            }
            // a lone digit (two groups, step 3) says nothing about which value it is
            if j < b.len() && b[j].is_ascii_alphabetic() || j - i < 2 {
                i = j.max(i + 1);
                continue;
            }
            out.push(b[i..j].iter().collect());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// `n` written in `known` as a number of its own (not inside a longer one).
fn has_number(known: &str, n: &str) -> bool {
    known.match_indices(n).any(|(at, _)| {
        let before = known[..at].chars().next_back();
        let after = known[at + n.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit() || c == '.') && !after.is_some_and(|c| c.is_ascii_digit()) && !(after == Some('.') && known[at + n.len() + 1..].chars().next().is_some_and(|c| c.is_ascii_digit()))
    })
}

/// At most this many dependent sentences of one result in one paragraph; more means the
/// paragraph marked sentences that only use known values.
pub const MAX_DEPS: usize = 2;

/// Ids with more than [`MAX_DEPS`] dependent sentences in `text`, with their count.
pub fn overmarked(text: &str) -> Vec<(String, usize)> {
    let mut n: BTreeMap<String, usize> = BTreeMap::new();
    for m in marks::marks(text).into_iter().filter(|m| m.kind == MarkKind::Dep) {
        *n.entry(m.id).or_default() += 1;
    }
    n.into_iter().filter(|(_, k)| *k > MAX_DEPS).collect()
}

/// The reviewer pass's rule for marks.
pub const AUDIT_RULE: &str = "Text marked \\pending{…}{…}, \\pendingcell, \\pendingpara or \\pendingdep is a result the \
author still waits for, written in the expected direction on purpose: never report it (or a sentence that ends in \
\\pendingdep) as missing-number or overclaim.";

/// Findings about marked text that the reviewer pass must not raise
/// (missing-number, overclaim) are dropped by the code as well.
pub fn filter_findings(findings: Vec<Finding>, sess: &Session) -> Vec<Finding> {
    let text_of = |para: &str| sess.drafts.iter().flat_map(|d| &d.paras).find(|p| p.para == para).map(|p| p.text.clone());
    let mut out: Vec<Finding> = findings
        .into_iter()
        .filter(|f| {
            if !matches!(f.kind.as_str(), "missing-number" | "overclaim") {
                return true;
            }
            if !marks::marks(&f.quote).is_empty() || f.quote.contains("\\pending") {
                return false;
            }
            let Some(text) = text_of(&f.para) else { return true };
            let q = f.quote.trim();
            let Some(at) = (!q.is_empty()).then(|| text.find(q)).flatten() else { return true };
            marks::marks(&sentence_at(&text, at)).is_empty()
        })
        .collect();
    for (i, f) in out.iter_mut().enumerate() {
        f.id = format!("f{}", i + 1);
    }
    out
}

// ── 回填: the result came in ─────────────────────────────

/// The 修訂 request made from a filled item: what it was, the author's
/// explanation, the result and what depends on it.
pub fn request_text(i: &PendingItem, rep: Option<&ReportEntry>) -> String {
    let mut s = format!("回填 {}：{}。結果：{}", i.id, i.what, i.shown_result());
    match i.direction {
        Some(Direction::Matches) => s += "（符合預期）",
        Some(Direction::Contradicts) => s += "（與預期相反）",
        _ => {}
    }
    if let Some(r) = rep {
        let n = r.places.len() + r.dependents.len();
        if n > 0 {
            s += &format!("。論文裡 {n} 處要改");
        }
    }
    s
}

/// The 修訂 conversation's block for a fill-in.
pub fn turn_context(i: &PendingItem, rep: Option<&ReportEntry>) -> String {
    let mut s = format!(
        "\n## This revision FILLS IN pending result {} — {}\n\
Expected: {} | the author's explanation: what={} / why={} / expect={} / if not={}\n\
Result now: {}{}\n",
        i.id,
        i.what,
        i.expected,
        i.note.what,
        i.note.why,
        i.note.expect,
        i.note.if_not,
        i.result,
        if i.values.is_empty() { String::new() } else { format!(" (values: {})", i.values.iter().map(|(k, v)| format!("{k} = {v}")).collect::<Vec<_>>().join("; ")) }
    );
    s += match i.direction {
        Some(Direction::Matches) => "The author says it matches the expectation: only the numbers change. Confirm the values and whether anything else needs a word; do not reopen settled reasoning.\n",
        Some(Direction::Contradicts) => "The author says it goes AGAINST the expectation: go through the dependent sentences below one at a time — what each should say now, and whether the claim it served still stands (the author's `if not` above is the starting point).\n",
        _ => "First ask what came in (the author may say it aloud) and whether it matches the expectation; then settle the dependent sentences below.\n",
    };
    if let Some(r) = rep {
        if !r.places.is_empty() {
            s += "Where the result goes:\n";
            for p in &r.places {
                s += &format!("- {} {} ¶{}: {}\n", p.file, p.section, p.para.map(|x| x.to_string()).unwrap_or("-".into()), p.sentence);
            }
        }
        if !r.dependents.is_empty() {
            s += "Sentences that depend on it:\n";
            for p in &r.dependents {
                s += &format!("- {} {} ¶{}: {}\n", p.file, p.section, p.para.map(|x| x.to_string()).unwrap_or("-".into()), p.sentence);
            }
        }
    }
    s
}

/// The plan rule for a fill-in (the model only writes what the code cannot).
pub fn plan_rule(i: &PendingItem) -> String {
    format!(
        "\nThis revision fills in pending result {id}. The code puts the numbers in: keep every \\pending{{{id}}}{{…}} and \
\\pendingcell{{{id}}}{{…}} mark as it is in your text. Write a change only for a paragraph that must be rewritten: a \
\\pendingpara{{{id}}}{{frame}} (write the paragraph the frame describes, with the result), and{} every sentence that \
ends in \\pendingdep{{{id}}} — drop that mark and say what the result shows. Never other ids' marks.",
        if i.direction == Some(Direction::Contradicts) { " — the result goes against the expectation —" } else { ", when the result does not support it," },
        id = i.id
    )
}

/// The value a mark of `i` gets, if the author gave one.
fn value_for(i: &PendingItem, m: &Mark) -> Option<String> {
    // a cell needs its own value; a number takes its own, else the result as given
    let own = i.values.get(m.arg.trim()).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    own.or_else(|| (m.kind == MarkKind::Number).then(|| i.result.trim().to_string()).filter(|v| !v.is_empty()))
}

/// Fill the marks of `i` in `text` (numbers and cells; dependency marks go).
pub fn fill_text(i: &PendingItem, text: &str) -> String {
    marks::fill(text, &i.id, &|m| value_for(i, m))
}

/// The code's own changes for a fill-in: every paragraph (and the main file's
/// abstract) with marks of `i`, its numbers filled and dependency marks gone.
/// Under a contradicting result, a paragraph with a dependent sentence is
/// flagged to rewrite. A planned paragraph, or a number with no value, is
/// left for the model or the author (an effect says so).
pub fn fill_changes(paper: &Path, i: &PendingItem) -> (Vec<Change>, Vec<String>) {
    let mut changes = vec![];
    let mut effects = vec![];
    let contra = i.direction == Some(Direction::Contradicts);
    let mut one = |file: &str, index: Option<usize>, action: &str, text: &str, where_: String| {
        let ms: Vec<Mark> = marks::marks(text).into_iter().filter(|m| m.id == i.id).collect();
        if ms.is_empty() {
            return;
        }
        for m in ms.iter().filter(|m| matches!(m.kind, MarkKind::Number | MarkKind::Cell) && value_for(i, m).is_none()) {
            effects.push(format!("{where_}：「{}」還沒有數值", if m.arg.is_empty() { &i.what } else { &m.arg }));
        }
        if ms.iter().any(|m| m.kind == MarkKind::Para) {
            effects.push(format!("{where_}：預留的段落要寫成內容"));
        }
        let new = fill_text(i, text);
        if new == text {
            return;
        }
        let dep = ms.iter().any(|m| m.kind == MarkKind::Dep);
        let rewrite = contra && dep;
        changes.push(Change {
            id: String::new(),
            file: file.into(),
            action: action.into(),
            index,
            before: text.to_string(),
            text: new,
            reason: if rewrite { format!("填入 {} 的結果；與預期相反，這段的說法要改寫", i.id) } else { format!("填入 {} 的結果", i.id) },
            applied: None,
            rewrite,
        });
    };
    let main = crate::open::main_file(paper);
    let main_name = main.file_name().and_then(|n| n.to_str()).unwrap_or("main.tex").to_string();
    let main_text = std::fs::read_to_string(&main).unwrap_or_default();
    for block in main_text.split("\n\n") {
        one(&main_name, None, "replace-text", block, format!("{main_name}（摘要）"));
    }
    for s in crate::open::sections(paper) {
        for (k, p) in s.paragraphs.iter().enumerate() {
            one(&s.file, Some(k), "replace", p, format!("{} ¶{k}", s.file));
        }
    }
    (changes, effects)
}

/// Merge the code's fill-in changes with a model's plan: the model's text gets
/// the numbers too; a paragraph the model rewrote is not changed twice; a
/// rewritten dependent paragraph is flagged when the result contradicts.
pub fn merge_plan(i: &PendingItem, model: Vec<Change>, code: Vec<Change>) -> Vec<Change> {
    let contra = i.direction == Some(Direction::Contradicts);
    let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: Vec<Change> = model
        .into_iter()
        .map(|mut c| {
            c.text = fill_text(i, &c.text);
            if contra && c.before.contains(&format!("\\pendingdep{{{}}}", i.id)) {
                c.rewrite = true;
            }
            c
        })
        .collect();
    for c in code {
        let taken = out.iter().any(|m| m.action == "replace" && m.file == c.file && norm(&m.before) == norm(&c.before));
        if !taken {
            out.push(c);
        }
    }
    for (k, c) in out.iter_mut().enumerate() {
        c.id = format!("x{}", k + 1);
    }
    out
}

/// A 修訂 for the item (its request, the 修訂 group made by the caller).
pub fn revision_for(i: &PendingItem, id: &str, group: &str, rep: Option<&ReportEntry>) -> Revision {
    Revision { id: id.into(), request: request_text(i, rep), group: group.into(), pending: Some(i.id.clone()), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::session::{DraftPara, DraftSection, Experiment};

    fn paper() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pc-pending-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(
            d.join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\\begin{abstract}\nWeighting holds on a second site\\pendingdep{P1}.\n\\end{abstract}\n\n\\input{sections/results}\n\\input{sections/conclusion}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            d.join("sections/results.tex"),
            "\\section{Results}\nInternal Dice is 0.72.\n\nExternal Dice is \\pending{P1}{external Dice}, which keeps the ranking\\pendingdep{P1}. The gap is small.\n\n\\pendingpara{P2}{Per-site comparison in \\ref{tab:site}.}\n",
        )
        .unwrap();
        std::fs::write(d.join("sections/conclusion.tex"), "\\section{Conclusion}\nThe gain carries over\\pendingdep{P1}.\n").unwrap();
        d
    }

    fn item(dir: Option<Direction>) -> PendingItem {
        PendingItem { id: "P1".into(), what: "external Dice".into(), expected: "略低於內部但排名不變".into(), result: "0.70".into(), direction: dir, status: PendingStatus::Filled, ..Default::default() }
    }

    #[test]
    fn the_report_lists_places_and_dependents_everywhere() {
        let p = paper();
        let mut s = Session::default();
        s.pending.push(PendingItem { id: "P1".into(), what: "external Dice".into(), ..Default::default() });
        let r = report(&p, &s);
        assert_eq!(r.iter().map(|e| (e.id.as_str(), e.places.len(), e.dependents.len(), e.unknown)).collect::<Vec<_>>(), [("P1", 1, 3, false), ("P2", 1, 0, true)]);
        let deps: Vec<&str> = r[0].dependents.iter().map(|d| d.section.as_str()).collect();
        assert_eq!(deps, ["Abstract", "Results", "Conclusion"], "abstract, results and conclusion");
        assert_eq!(r[0].places[0].para, Some(1));
        assert!(r[0].dependents[1].sentence.starts_with("External Dice is") && !r[0].dependents[1].sentence.contains("gap"), "{}", r[0].dependents[1].sentence);
        assert_eq!(blockers(&p).len(), 5);
        assert!(blocker_message(&r[0].places[0]).contains("待補 P1（數字）：external Dice"));
        let g = blocker_groups(&blockers(&p));
        assert_eq!(g.iter().map(|b| b.message.as_str()).collect::<Vec<_>>(), ["待補 P1（依賴句）", "待補 P1（數字・依賴句）：external Dice", "待補 P2（段落）：Per-site comparison in tab:site.", "待補 P1（依賴句）"]);
        assert_eq!(g[1].context, "External Dice is [pending: external Dice], which keeps the ranking [pending].");
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn matching_results_change_only_numbers() {
        let p = paper();
        let (ch, eff) = fill_changes(&p, &item(Some(Direction::Matches)));
        assert_eq!(ch.len(), 3, "abstract, results ¶1, conclusion");
        assert!(ch.iter().all(|c| !c.rewrite));
        let res = ch.iter().find(|c| c.file == "results.tex").unwrap();
        assert_eq!(res.text, "External Dice is 0.70, which keeps the ranking. The gap is small.");
        assert_eq!((res.action.as_str(), res.index), ("replace", Some(1)));
        assert_eq!(ch[0].action, "replace-text");
        assert!(eff.is_empty(), "{eff:?}");
        // applying every change leaves no P1 mark, P2 stays
        for c in &ch {
            crate::draft::revise::apply(&p, c).unwrap();
        }
        assert!(locate(&p).iter().all(|x| x.id == "P2"));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn a_contradicting_result_flags_every_dependent_paragraph() {
        let p = paper();
        let i = item(Some(Direction::Contradicts));
        let (code, _) = fill_changes(&p, &i);
        assert!(code.iter().all(|c| c.rewrite), "every P1 paragraph has a dependent sentence");
        // the model rewrote the results paragraph (and left the number mark for the code)
        let before = std::fs::read_to_string(p.join("sections/results.tex")).unwrap();
        let para = before.split("\n\n").nth(1).unwrap().to_string();
        let model = vec![Change { id: "x1".into(), file: "results.tex".into(), action: "replace".into(), index: Some(1), before: para, text: "External Dice is \\pending{P1}{external Dice}, and the ranking flips.".into(), reason: "改寫".into(), ..Default::default() }];
        let all = merge_plan(&i, model, code);
        assert_eq!(all.len(), 3, "the model's change replaces the code's for that paragraph");
        assert_eq!(all[0].text, "External Dice is 0.70, and the ranking flips.", "numbers come from the code");
        assert!(all[0].rewrite && all.iter().map(|c| c.id.as_str()).eq(["x1", "x2", "x3"]));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn missing_values_and_planned_paragraphs_are_effects() {
        let p = paper();
        let mut i = item(Some(Direction::Matches));
        i.id = "P2".into();
        let (ch, eff) = fill_changes(&p, &i);
        assert!(ch.is_empty() && eff.iter().any(|e| e.contains("預留的段落")), "{eff:?}");
        let mut i = item(None);
        i.result.clear();
        let (_, eff) = fill_changes(&p, &i);
        assert!(eff.iter().any(|e| e.contains("external Dice") && e.contains("還沒有數值")), "{eff:?}");
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn tables_carry_pending_cells_rows_and_columns() {
        let t = ResultTable {
            id: "R1".into(),
            caption: "Dice".into(),
            columns: vec!["Method".into(), "Internal".into(), "External".into()],
            rows: vec![vec!["Base".into(), "0.69".into(), "?".into()], vec!["Ours".into(), "0.72".into(), "".into()]],
            ..Default::default()
        };
        let mut items = vec![];
        from_record(&mut items, &[], std::slice::from_ref(&t));
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].table.as_deref(), items[0].cells.as_slice()), (Some("R1"), [Hole { row: Some(0), col: Some(2) }].as_slice()));
        // the author makes the whole column wait, and adds a row
        items[0].cells = vec![Hole { row: None, col: Some(2) }];
        assert_eq!(table_tsv(&t, &items), "Method\tInternal\tExternal\nBase\t0.69\t\\pendingcell{P1}{Base / External}\nOurs\t0.72\t\\pendingcell{P1}{Ours / External}");
        items[0].cells = vec![Hole { row: Some(1), col: None }];
        assert_eq!(table_tsv(&t, &items).lines().nth(2).unwrap(), "Ours\t\\pendingcell{P1}{Ours / Internal}\t\\pendingcell{P1}{Ours / External}", "a row keeps its label");
        // after the record is read again the item follows its table by caption
        let t2 = ResultTable { id: "R3".into(), ..t.clone() };
        rebind_tables(&mut items, &[t2]);
        assert_eq!(items[0].table.as_deref(), Some("R3"));
        rebind_tables(&mut items, &[]);
        assert!(items[0].table.is_none() && items[0].cells.is_empty());
        // experiments without a result yet
        let e = Experiment { id: "E4".into(), title: "External site".into(), metrics: "Dice".into(), result: "?".into(), ..Default::default() };
        from_record(&mut items, &[e], &[]);
        assert_eq!(items[1].experiment.as_deref(), Some("E4"));
        assert_eq!(items[1].id, "P2");
    }

    #[test]
    fn a_filled_result_goes_back_into_the_record() {
        let mut s = Session::default();
        s.experiments.push(Experiment { id: "E2".into(), result: "?".into(), ..Default::default() });
        s.tables.push(ResultTable { id: "R1".into(), columns: vec!["Method".into(), "External".into()], rows: vec![vec!["Base".into(), "?".into()], vec!["Ours".into(), "?".into()]], ..Default::default() });
        s.pending.push(PendingItem {
            id: "P1".into(),
            what: "external Dice".into(),
            experiment: Some("E2".into()),
            table: Some("R1".into()),
            cells: vec![Hole { row: None, col: Some(1) }],
            result: "0.70".into(),
            values: [("Base / External".to_string(), "0.66".to_string()), ("Ours / External".to_string(), "0.70".to_string())].into(),
            status: PendingStatus::Filled,
            direction: Some(Direction::Matches),
            ..Default::default()
        });
        assert_eq!(holes_text(&s.pending[0], &s.tables[0]), "External 欄");
        write_back(&mut s, "P1");
        assert_eq!(s.experiments[0].result, "0.70");
        assert_eq!(s.tables[0].rows, [["Base", "0.66"], ["Ours", "0.70"]]);
        let c = context_block(&s);
        assert!(c.contains("Results that came in") && c.contains("P1 external Dice: 0.70") && c.contains("as expected"), "{c}");
        assert_eq!(missing_marks(&s, &["P1".into()], "Dice is 0.70."), Vec::<String>::new(), "filled: no mark expected");
        s.pending[0].status = PendingStatus::Waiting;
        assert_eq!(missing_marks(&s, &["P1".into()], "Dice is 0.70."), ["P1"]);
        assert!(missing_marks(&s, &["P1".into()], "Dice is \\pending{P1}{external Dice}.").is_empty());
    }

    #[test]
    fn a_rewind_keeps_the_link_and_an_updated_record_is_noticed() {
        let mut s = Session::default();
        s.pending.push(PendingItem { id: "P1".into(), what: "external Dice".into(), experiment: Some("E2".into()), note: PendingNote { why: "推廣".into(), ..Default::default() }, ..Default::default() });
        s.set_plan(vec![(Group { section: "Results".into(), title: "主表".into(), ..Default::default() }, vec![])]);
        sync_group(&mut s);
        s.group_mut(GROUP_ID).unwrap().prep = Some(crate::draft::candidates::Prep::Running);
        s.rewind(crate::draft::session::Phase::Plan);
        let it = s.items_of(GROUP_ID).next().unwrap();
        assert_eq!(it.pending.as_deref(), Some("P1"), "the item still stands for P1");
        assert!(s.group(GROUP_ID).unwrap().prep.is_none(), "a preparation in flight is forgotten");
        assert!(s.pending[0].note.why.is_empty(), "the interview's words go with the interview");
        // the record read again now has E2's number
        s.experiments.push(Experiment { id: "E2".into(), result: "?".into(), ..Default::default() });
        assert!(arrived(&s).is_empty());
        s.experiments[0].result = "0.70".into();
        assert_eq!(arrived(&s), [0]);
        // read again: the model numbered the experiments differently; the item follows its title
        s.experiments[0].title = "External site".into();
        rebind_experiments(&mut s.pending, &s.experiments.clone());
        assert_eq!(s.pending[0].experiment_title, "External site");
        let renumbered = vec![Experiment { id: "E1".into(), title: "Internal".into(), result: "0.72".into(), ..Default::default() }, Experiment { id: "E5".into(), title: "External site".into(), result: "?".into(), ..Default::default() }];
        rebind_experiments(&mut s.pending, &renumbered);
        assert_eq!(s.pending[0].experiment.as_deref(), Some("E5"), "never E1's number");
    }

    #[test]
    fn edits_keep_what_the_code_owns() {
        let old = vec![PendingItem { id: "P1".into(), what: "a".into(), note: PendingNote { why: "因為".into(), ..Default::default() }, ..Default::default() }];
        let edit = vec![
            PendingItem { id: "P1".into(), what: "a2".into(), ..Default::default() },
            PendingItem { what: "new".into(), ..Default::default() },
            PendingItem { what: " ".into(), ..Default::default() },
        ];
        let m = merge_edit(&old, edit);
        assert_eq!(m.iter().map(|i| (i.id.as_str(), i.what.as_str())).collect::<Vec<_>>(), [("P1", "a2"), ("P2", "new")]);
        assert_eq!(m[0].note.why, "因為");
    }

    #[test]
    fn the_group_follows_the_items() {
        let mut s = Session::default();
        s.pending.push(PendingItem { id: "P1".into(), what: "external Dice".into(), ..Default::default() });
        sync_group(&mut s);
        assert!(s.group(GROUP_ID).is_none(), "no plan yet");
        s.set_plan(vec![(Group { section: "Results".into(), title: "主表".into(), ..Default::default() }, vec![])]);
        sync_group(&mut s);
        assert_eq!(s.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["g1", GROUP_ID]);
        assert_eq!(s.items_of(GROUP_ID).map(|i| (i.pending.as_deref(), i.role.as_str())).collect::<Vec<_>>(), [(Some("P1"), ROLE)]);
        sync_group(&mut s);
        assert_eq!(s.items_of(GROUP_ID).count(), 1, "not twice");
        s.pending[0].status = PendingStatus::Filled;
        sync_group(&mut s);
        assert!(s.group(GROUP_ID).is_none(), "nothing waits, the untouched group goes");
    }

    #[test]
    fn findings_on_marked_text_are_dropped() {
        let mut s = Session::default();
        s.drafts.push(DraftSection {
            section: "Results".into(),
            paras: vec![DraftPara { para: "p3".into(), text: "Dice is \\pending{P1}{external Dice}, clearly better\\pendingdep{P1}. Recall improved.".into(), ..Default::default() }],
            ..Default::default()
        });
        let f = |k: &str, q: &str| Finding { id: "f".into(), kind: k.into(), para: "p3".into(), quote: q.into(), why: "w".into(), ..Default::default() };
        let out = filter_findings(vec![f("overclaim", "clearly better"), f("missing-number", "Recall improved"), f("generic", "clearly better"), f("missing-number", "\\pending{P1}")], &s);
        assert_eq!(out.iter().map(|x| (x.id.as_str(), x.kind.as_str())).collect::<Vec<_>>(), [("f1", "missing-number"), ("f2", "generic")]);
    }

    #[test]
    fn only_sentences_that_use_the_pending_value_are_dependent() {
        // regression (e2e: 11–27 dependent sentences per item): the item was named after the whole
        // table, every paragraph got every item's rules, and nothing pruned the marks
        let t = ResultTable {
            id: "R1".into(),
            caption: "Cross-site AUC".into(),
            columns: vec!["Finding".into(), "Site A".into(), "Site B".into()],
            rows: vec![vec!["Edema".into(), "0.83".into(), "0.79".into()], vec!["Devices".into(), "0.88".into(), "?".into()]],
            ..Default::default()
        };
        let mut s = Session::default();
        from_record(&mut s.pending, &[], std::slice::from_ref(&t));
        assert_eq!(s.pending[0].what, "Devices / Site B (in \"Cross-site AUC\")", "named by its waiting cell");
        s.pending.push(PendingItem { id: "P2".into(), what: "other".into(), status: PendingStatus::Filled, ..Default::default() });
        let r = draft_rules(&s, &["P1".into()]);
        assert!(r.contains("only 1 cell(s) wait, every other value of that table is known"));
        assert!(r.contains("a sentence that uses only known numbers") && !r.contains("every sentence that depends"));
        assert!(!draft_rules(&s, &["P9".into()]).contains("P1"), "only the paragraph's own items");
        assert!(not_here(&s).contains("P1") && !not_here(&s).contains("pendingdep"));
        // the code drops marks of results the paragraph does not depend on, and of filled ones
        let text = "Edema fell from 0.83 to 0.79 \\pendingdep{P1}. Devices reached \\pending{P1}{Devices Site B AUC}\\pendingdep{P1}. Old\\pendingdep{P2}.";
        assert_eq!(prune_deps(&s, &[], text, ""), "Edema fell from 0.83 to 0.79. Devices reached \\pending{P1}{Devices Site B AUC}\\pendingdep{P1}. Old.", "stated here: kept; filled: gone");
        assert_eq!(prune_deps(&s, &[], "A 0.79\\pendingdep{P1}.", ""), "A 0.79.", "neither depended on nor stated: gone");
        // …but an abstract or conclusion sentence that states the outcome stays marked (回填 rewrites it)
        assert_eq!(prune_deps(&s, &["P1".into()], "The gain carries over\\pendingdep{P1}.", "0.83 0.79"), "The gain carries over\\pendingdep{P1}.");
        // a sentence of known values only (even in a paragraph about the result) is not dependent
        let record = "| Edema | 0.83 | 0.79 |\nIn men p = 0.003671 (E6)";
        let para = "In men, devices differed (p = 0.003671)\\pendingdep{P1}. Edema fell from 0.83 to 0.80\\pendingdep{P1}. Devices reached \\pending{P1}{x}, so the gap is broad\\pendingdep{P1}.";
        assert_eq!(prune_deps(&s, &["P1".into()], para, record), "In men, devices differed (p = 0.003671). Edema fell from 0.83 to 0.80\\pendingdep{P1}. Devices reached \\pending{P1}{x}, so the gap is broad\\pendingdep{P1}.", "0.80 is not in the record: kept");
        assert_eq!(numbers("p = 5.34, n = 5,159 in 2 groups, CT3 and 0.2."), ["5.34", "5,159", "0.2"]);
        // the abstract / conclusion (no `known`): a conclusion quoting a known number keeps its mark
        let concl = "Only Edema fell, by 0.04, so the shift is narrow\\pendingdep{P1}.";
        assert_eq!(prune_deps(&s, &["P1".into()], concl, ""), concl);
        assert_eq!(prune_deps(&s, &["P1".into()], concl, "0.04"), "Only Edema fell, by 0.04, so the shift is narrow.", "elsewhere it is pruned");
        assert!(overmarked("a\\pendingdep{P1}. b\\pendingdep{P1}.").is_empty());
        assert_eq!(overmarked("a\\pendingdep{P1}. b\\pendingdep{P1}. c\\pendingdep{P1}."), [("P1".to_string(), 3)]);
    }

    #[test]
    fn the_paper_compiles_with_its_marks() {
        let p = paper();
        assert!(ensure_tex(&p).unwrap());
        let main = std::fs::read_to_string(p.join("main.tex")).unwrap();
        assert!(main.contains("\\input{pending}\n\\begin{document}"), "{main}");
        assert!(p.join("pending.tex").is_file());
        assert!(!ensure_tex(&p).unwrap(), "once");
        assert!(!release_tex(&p).unwrap(), "marks left: the input stays");
        std::fs::write(p.join("sections/results.tex"), "\\section{Results}\nDone.\n").unwrap();
        std::fs::write(p.join("sections/conclusion.tex"), "\\section{Conclusion}\nDone.\n").unwrap();
        let main = std::fs::read_to_string(p.join("main.tex")).unwrap().replace("\\pendingdep{P1}", "");
        std::fs::write(p.join("main.tex"), main).unwrap();
        assert!(release_tex(&p).unwrap());
        assert!(!std::fs::read_to_string(p.join("main.tex")).unwrap().contains("\\input{pending}"));
        let _ = std::fs::remove_dir_all(&p);
    }
}
