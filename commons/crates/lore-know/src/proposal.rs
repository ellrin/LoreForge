//! Proposals (docs/brain-hub.md §7, package K5). An AI or a tool may add, change or remove
//! knowledge only as a [`Proposal`]; nothing is written until the author approves, he may
//! approve only some of its changes, and an approved batch can be undone.
//!
//! * [`Proposals`]: the box (`<hub>/proposals/`, one JSON file per proposal).
//! * [`show`]: the before / after view for the author.
//! * [`apply`]: writes the chosen changes through a [`Sink`] and keeps an undo record.
//! * [`undo`]: reverses an applied batch (never over a later edit).

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::types::*;

const TEMP_PREFIX: &str = "new:";

fn is_temp(id: &str) -> bool {
    id.strip_prefix(TEMP_PREFIX).is_some_and(|n| !n.is_empty())
}

/// Write `bytes` to `path` through a temp file and a rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn other<E: std::fmt::Display>(e: E) -> KnowError {
    KnowError::Other(e.to_string())
}

// ── step 1: the box ─────────────────────────────────────────────────────────

/// The box of proposals: a folder with `<id>.json` per proposal (and `<id>.undo.json` after it
/// was applied).
#[derive(Debug, Clone)]
pub struct Proposals {
    dir: PathBuf,
}

impl Proposals {
    /// The box in `dir` (made when missing).
    pub fn open(dir: impl Into<PathBuf>) -> Result<Proposals> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Proposals { dir })
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty() || id.contains(['/', '\\', ':']) || id.starts_with('.') {
            return Err(KnowError::NotFound(id.into()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn undo_path(&self, id: &str) -> Result<PathBuf> {
        self.path(id)?;
        Ok(self.dir.join(format!("{id}.undo.json")))
    }

    fn save(&self, p: &Proposal) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(p).map_err(other)?;
        write_atomic(&self.path(&p.id)?, &bytes)
    }

    /// A new open proposal. Refuses an empty batch and malformed changes.
    pub fn create(&self, by: &str, title: &str, changes: Vec<Change>) -> Result<Proposal> {
        check_batch(&changes)?;
        let ms = chrono::Local::now().timestamp_millis().max(0) as u64;
        let changes = changes
            .into_iter()
            .map(|mut c| {
                c.result.clear();
                c.error.clear();
                c
            })
            .collect();
        let p = Proposal {
            id: format!("p{}", new_local(ms)),
            by: by.trim().into(),
            title: title.trim().into(),
            changes,
            state: "open".into(),
            created_at: now(),
        };
        self.save(&p)?;
        Ok(p)
    }

    pub fn get(&self, id: &str) -> Result<Proposal> {
        let path = self.path(id)?;
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { KnowError::NotFound(id.into()) } else { e.into() }
        })?;
        serde_json::from_slice(&bytes).map_err(|e| KnowError::Other(format!("提案 {id} 讀不了：{e}")))
    }

    /// Proposals (all, or those in `state`), newest first. Unreadable files are skipped.
    pub fn list(&self, state: Option<&str>) -> Vec<Proposal> {
        let Ok(rd) = fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Proposal> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name.ends_with(".undo.json") || !name.ends_with(".json") {
                    return None;
                }
                serde_json::from_slice::<Proposal>(&fs::read(e.path()).ok()?).ok()
            })
            .filter(|p| state.is_none_or(|s| p.state == s))
            .collect();
        // ids sort by creation time (new_local)
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out
    }

    /// The author turns it down: the file stays, the state is `dismissed`.
    pub fn dismiss(&self, id: &str) -> Result<Proposal> {
        let mut p = self.get(id)?;
        if p.state == "applied" {
            return Err(KnowError::Invalid("這一批已經套用，不能駁回".into()));
        }
        p.state = "dismissed".into();
        self.save(&p)?;
        Ok(p)
    }

    pub fn count_open(&self) -> usize {
        self.list(Some("open")).len()
    }
}

/// Shape of a batch: not empty; op in add/update/remove; add/update carry an item, remove an id;
/// an `add` item's id is empty or a `new:<n>` unique in the batch.
fn check_batch(changes: &[Change]) -> Result<()> {
    if changes.is_empty() {
        return Err(KnowError::Invalid("提案沒有任何一條改動".into()));
    }
    let mut temps = BTreeSet::new();
    for (i, c) in changes.iter().enumerate() {
        let n = i + 1;
        match c.op.as_str() {
            "add" => {
                let item = c.item.as_ref().ok_or_else(|| KnowError::Invalid(format!("第 {n} 條：新增沒有內容")))?;
                if item.category.trim().is_empty() {
                    return Err(KnowError::Invalid(format!("第 {n} 條：新增沒有類別")));
                }
                if !item.id.is_empty() {
                    if !is_temp(&item.id) {
                        return Err(KnowError::Invalid(format!("第 {n} 條：新增的編號要留空或用 new:<數字>（{}）", item.id)));
                    }
                    if !temps.insert(item.id.clone()) {
                        return Err(KnowError::Invalid(format!("第 {n} 條：暫時編號 {} 重複", item.id)));
                    }
                }
            }
            "update" => {
                let item = c.item.as_ref().ok_or_else(|| KnowError::Invalid(format!("第 {n} 條：修改沒有內容")))?;
                if item.id.is_empty() || is_temp(&item.id) {
                    return Err(KnowError::Invalid(format!("第 {n} 條：修改要指明既有的一條")));
                }
            }
            "remove" => {
                if c.id.trim().is_empty() || is_temp(&c.id) {
                    return Err(KnowError::Invalid(format!("第 {n} 條：移除沒有指明哪一條")));
                }
            }
            op => return Err(KnowError::Invalid(format!("第 {n} 條：不認得的動作「{op}」"))),
        }
    }
    Ok(())
}

// ── step 2: the view ────────────────────────────────────────────────────────

/// One change as the author sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shown {
    /// Position in `Proposal::changes`.
    pub index: usize,
    pub op: String,
    pub note: String,
    /// The stored item now (update, remove).
    pub before: Option<Item>,
    /// The item as proposed (add, update).
    pub after: Option<Item>,
    /// What differs: field names, `title`, `tags.<facet>`, `triggers`, `contains`.
    pub changed: Vec<String>,
    /// One line, Traditional Chinese; "" when the change can be applied as it is.
    pub problem: String,
}

fn differences(a: &Item, b: &Item) -> Vec<String> {
    let mut out = vec![];
    if a.title != b.title {
        out.push("title".to_string());
    }
    let names: BTreeSet<&String> = a.fields.keys().chain(b.fields.keys()).collect();
    for n in names {
        if a.fields.get(n) != b.fields.get(n) {
            out.push(n.clone());
        }
    }
    let facets: BTreeSet<&String> = a.tags.keys().chain(b.tags.keys()).collect();
    for f in facets {
        if a.tags.get(f) != b.tags.get(f) {
            out.push(format!("tags.{f}"));
        }
    }
    if a.triggers != b.triggers {
        out.push("triggers".into());
    }
    if a.contains != b.contains {
        out.push("contains".into());
    }
    out
}

/// Every change of `p` with what is stored now beside it.
pub fn show(sink: &dyn Sink, p: &Proposal) -> Result<Vec<Shown>> {
    let mut out = vec![];
    for (index, c) in p.changes.iter().enumerate() {
        let mut s = Shown { index, op: c.op.clone(), note: c.note.clone(), ..Default::default() };
        match c.op.as_str() {
            "add" => s.after = c.item.clone(),
            "update" => {
                s.after = c.item.clone();
                let id = c.item.as_ref().map(|i| i.id.as_str()).unwrap_or("");
                s.before = sink.get(id)?;
                match (&s.before, &s.after) {
                    (None, _) => s.problem = "這一條已經不存在".into(),
                    (Some(b), Some(a)) => {
                        s.changed = differences(b, a);
                        if b.revision != a.revision {
                            s.problem = format!("這一條在提案之後被改過（現在第 {} 版，提案依據第 {} 版）", b.revision, a.revision);
                        }
                    }
                    _ => {}
                }
            }
            "remove" => {
                s.before = sink.get(&c.id)?;
                if s.before.is_none() {
                    s.problem = "這一條已經不存在".into();
                }
            }
            op => s.problem = format!("不認得的動作「{op}」"),
        }
        out.push(s);
    }
    Ok(out)
}

// ── step 3: applying ────────────────────────────────────────────────────────

/// What is needed to reverse one applied change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct UndoStep {
    /// `added` · `updated` · `removed`.
    kind: String,
    id: String,
    /// `updated`: the item as it was before.
    before: Option<Item>,
    /// `added`, `updated`: the revision our write left; a different one now means a later edit.
    revision: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct UndoRecord {
    steps: Vec<UndoStep>,
}

fn read_undo(box_: &Proposals, id: &str) -> Result<UndoRecord> {
    let path = box_.undo_path(id)?;
    let bytes = fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { KnowError::Invalid("這一批沒有可以復原的紀錄".into()) } else { e.into() }
    })?;
    serde_json::from_slice(&bytes).map_err(|e| KnowError::Other(format!("復原紀錄讀不了：{e}")))
}

fn write_undo(box_: &Proposals, id: &str, rec: &UndoRecord) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(rec).map_err(other)?;
    write_atomic(&box_.undo_path(id)?, &bytes)
}

/// Write the chosen changes (`None` = all) of the open proposal `id`. Order: adds, then the
/// links between new items (`contains`, temporary ids to stored ids), then updates, then removes.
/// A change that cannot be written is `refused` with its error and the rest go on. The proposal
/// becomes `applied` and an undo record is kept beside it.
pub fn apply(sink: &mut dyn Sink, box_: &Proposals, id: &str, chosen: Option<&[usize]>) -> Result<Proposal> {
    let mut p = box_.get(id)?;
    if p.state != "open" {
        return Err(KnowError::Invalid(format!("這一批不是待決定的（現在：{}）", p.state)));
    }
    check_batch(&p.changes)?;
    let n = p.changes.len();
    if let Some(ch) = chosen {
        if let Some(bad) = ch.iter().find(|&&i| i >= n) {
            return Err(KnowError::Invalid(format!("沒有第 {} 條", bad + 1)));
        }
        if ch.is_empty() {
            return Err(KnowError::Invalid("沒有選任何一條".into()));
        }
    }
    let is_chosen = |i: usize| chosen.is_none_or(|c| c.contains(&i));

    let ops: Vec<String> = p.changes.iter().map(|c| c.op.clone()).collect();
    let mut steps: Vec<UndoStep> = vec![];
    let mut mapped: HashMap<String, String> = HashMap::new(); // temporary id -> stored id
    let mut added: Vec<(usize, Item)> = vec![]; // stored adds, in order

    for i in 0..n {
        p.changes[i].result = if is_chosen(i) { String::new() } else { "skipped".into() };
        p.changes[i].error.clear();
    }

    // 1. adds (temporary ids in `contains` wait until every add has its stored id)
    for i in (0..n).filter(|&i| is_chosen(i) && ops[i] == "add") {
        let mut item = p.changes[i].item.clone().unwrap_or_default();
        let temp = std::mem::take(&mut item.id);
        item.revision = 0;
        item.contains.retain(|c| !is_temp(c));
        match sink.put(item) {
            Ok(stored) => {
                if !temp.is_empty() {
                    mapped.insert(temp, stored.id.clone());
                }
                steps.push(UndoStep { kind: "added".into(), id: stored.id.clone(), before: None, revision: stored.revision });
                p.changes[i].result = "applied".into();
                added.push((i, stored));
            }
            Err(e) => {
                p.changes[i].result = "refused".into();
                p.changes[i].error = e.to_string();
            }
        }
    }

    // 2. `contains`: temporary ids to stored ids; a temporary id nobody stored is dropped
    let relink = |list: &[String]| -> Vec<String> {
        list.iter().filter_map(|c| if is_temp(c) { mapped.get(c).cloned() } else { Some(c.clone()) }).collect()
    };
    for (i, stored) in &added {
        let wanted = relink(&p.changes[*i].item.as_ref().map(|x| x.contains.clone()).unwrap_or_default());
        if wanted != stored.contains {
            let mut again = stored.clone();
            again.contains = wanted;
            match sink.put(again) {
                Ok(s2) => {
                    if let Some(step) = steps.iter_mut().find(|s| s.kind == "added" && s.id == s2.id) {
                        step.revision = s2.revision;
                    }
                }
                Err(e) => p.changes[*i].error = format!("連結沒有寫入：{e}"),
            }
        }
    }

    // 3. updates
    for i in (0..n).filter(|&i| is_chosen(i) && ops[i] == "update") {
        let mut item = p.changes[i].item.clone().unwrap_or_default();
        item.contains = relink(&item.contains);
        let before = match sink.get(&item.id) {
            Ok(Some(b)) => b,
            Ok(None) => {
                p.changes[i].result = "refused".into();
                p.changes[i].error = "這一條已經不存在".into();
                continue;
            }
            Err(e) => {
                p.changes[i].result = "refused".into();
                p.changes[i].error = e.to_string();
                continue;
            }
        };
        match sink.put(item) {
            Ok(stored) => {
                steps.push(UndoStep { kind: "updated".into(), id: stored.id.clone(), before: Some(before), revision: stored.revision });
                p.changes[i].result = "applied".into();
            }
            Err(e) => {
                p.changes[i].result = "refused".into();
                p.changes[i].error = e.to_string();
            }
        }
    }

    // 4. removes
    for i in (0..n).filter(|&i| is_chosen(i) && ops[i] == "remove") {
        let rid = p.changes[i].id.clone();
        match sink.remove(&rid) {
            Ok(_) => {
                steps.push(UndoStep { kind: "removed".into(), id: rid, before: None, revision: 0 });
                p.changes[i].result = "applied".into();
            }
            Err(e) => {
                p.changes[i].result = "refused".into();
                p.changes[i].error = e.to_string();
            }
        }
    }

    p.state = "applied".into();
    // the undo record first: a proposal that says `applied` always has what reverses it
    write_undo(box_, id, &UndoRecord { steps })?;
    box_.save(&p)?;
    Ok(p)
}

// ── step 4: undo ────────────────────────────────────────────────────────────

/// Reverse an applied proposal, last write first. Returns one line per thing that could not be
/// undone (those stay in the undo record; the proposal stays `applied` until everything is back).
pub fn undo(sink: &mut dyn Sink, box_: &Proposals, id: &str) -> Result<Vec<String>> {
    let mut p = box_.get(id)?;
    if p.state != "applied" {
        return Err(KnowError::Invalid(format!("這一批沒有套用過（現在：{}）", p.state)));
    }
    let rec = read_undo(box_, id)?;
    let mut left: Vec<UndoStep> = vec![];
    let mut lines: Vec<String> = vec![];

    for step in rec.steps.iter().rev() {
        let done: std::result::Result<(), String> = match step.kind.as_str() {
            "added" => match sink.get(&step.id) {
                Ok(Some(cur)) if cur.revision != step.revision => Err(format!("{}：新增之後又被改過，沒有移除", step.id)),
                Ok(Some(_)) => sink.remove(&step.id).map(|_| ()).map_err(|e| format!("{}：{e}", step.id)),
                Ok(None) => Ok(()), // already gone
                Err(e) => Err(format!("{}：{e}", step.id)),
            },
            "updated" => match (sink.get(&step.id), &step.before) {
                (Ok(Some(cur)), Some(before)) => {
                    if cur.revision != step.revision {
                        Err(format!("{}：修改之後又被改過，沒有蓋掉", step.id))
                    } else {
                        let mut back = before.clone();
                        back.revision = cur.revision;
                        sink.put(back).map(|_| ()).map_err(|e| format!("{}：{e}", step.id))
                    }
                }
                (Ok(None), _) => Err(format!("{}：這一條現在不存在，沒有改回", step.id)),
                (Err(e), _) => Err(format!("{}：{e}", step.id)),
                (_, None) => Err(format!("{}：復原紀錄缺少原本的內容", step.id)),
            },
            "removed" => match sink.get(&step.id) {
                Ok(Some(_)) => Err(format!("{}：現在又有同一編號的一條，沒有還原", step.id)),
                Ok(None) => sink.restore(&step.id).map(|_| ()).map_err(|e| format!("{}：{e}", step.id)),
                Err(e) => Err(format!("{}：{e}", step.id)),
            },
            k => Err(format!("{}：不認得的紀錄「{k}」", step.id)),
        };
        if let Err(line) = done {
            lines.push(line);
            left.push(step.clone());
        }
    }

    if left.is_empty() {
        for c in &mut p.changes {
            c.result.clear();
            c.error.clear();
        }
        p.state = "open".into();
        box_.save(&p)?;
        let _ = fs::remove_file(box_.undo_path(id)?);
    } else {
        left.reverse(); // keep the original order in the record
        write_undo(box_, id, &UndoRecord { steps: left })?;
    }
    Ok(lines)
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn note(title: &str) -> Item {
        let mut i = Item::new("my.notes", format::NOTE, title, Source::lore("le"));
        i.set("body", "text");
        i
    }

    fn add(title: &str, temp: &str) -> Change {
        let mut i = note(title);
        i.id = temp.into();
        Change { op: "add".into(), item: Some(i), note: "why".into(), ..Default::default() }
    }

    fn stored(s: &mut MemSink, title: &str) -> Item {
        s.put(note(title)).unwrap()
    }

    fn boxed() -> (tempfile::TempDir, Proposals) {
        let d = tempfile::tempdir().unwrap();
        let b = Proposals::open(d.path().join("proposals")).unwrap();
        (d, b)
    }

    fn update(item: Item) -> Change {
        Change { op: "update".into(), item: Some(item), ..Default::default() }
    }

    fn remove(id: &str) -> Change {
        Change { op: "remove".into(), id: id.into(), ..Default::default() }
    }

    // step 1

    #[test]
    fn a_proposal_is_kept_until_decided() {
        let (d, b) = boxed();
        let p = b.create("le", " 補三條 ", vec![add("A", "")]).unwrap();
        assert!(p.id.starts_with("pk") && p.state == "open" && !p.created_at.is_empty());
        assert_eq!(p.title, "補三條");
        let again = Proposals::open(d.path().join("proposals")).unwrap();
        assert_eq!(again.get(&p.id).unwrap(), p);
        assert_eq!(again.count_open(), 1);
        assert!(matches!(again.get("pnone"), Err(KnowError::NotFound(_))));
        assert!(matches!(again.get("../x"), Err(KnowError::NotFound(_))));
        // newest first
        std::thread::sleep(std::time::Duration::from_millis(3));
        let q = b.create("le", "後來的", vec![add("B", "")]).unwrap();
        let ids: Vec<String> = b.list(None).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![q.id, p.id]);
        let names: Vec<String> =
            fs::read_dir(d.path().join("proposals")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| n.ends_with(".json") && !n.starts_with('.')), "{names:?}");
    }

    #[test]
    fn malformed_batches_are_refused() {
        let (_d, b) = boxed();
        let bad = |c: Vec<Change>| matches!(b.create("le", "t", c), Err(KnowError::Invalid(_)));
        assert!(bad(vec![]));
        assert!(bad(vec![Change { op: "rename".into(), ..Default::default() }]));
        assert!(bad(vec![Change { op: "add".into(), ..Default::default() }]));
        assert!(bad(vec![add("A", "my.notes:k1")]));
        assert!(bad(vec![add("A", "new:")]));
        assert!(bad(vec![add("A", "new:1"), add("B", "new:1")]));
        assert!(bad(vec![Change { op: "update".into(), ..Default::default() }]));
        assert!(bad(vec![update(note("no id"))]));
        assert!(bad(vec![Change { op: "remove".into(), ..Default::default() }]));
        let mut nocat = add("A", "");
        nocat.item.as_mut().unwrap().category.clear();
        assert!(bad(vec![nocat]));
        assert_eq!(b.list(None).len(), 0, "nothing was kept");
        assert!(b.create("le", "t", vec![add("A", ""), add("B", ""), add("C", "new:1"), add("D", "new:2")]).is_ok());
        assert_eq!(b.list(None).len(), 1);
    }

    #[test]
    fn dismissed_ones_are_not_open() {
        let (_d, b) = boxed();
        let p = b.create("le", "a", vec![add("A", "")]).unwrap();
        let q = b.create("le", "b", vec![add("B", "")]).unwrap();
        assert_eq!(b.count_open(), 2);
        assert_eq!(b.dismiss(&p.id).unwrap().state, "dismissed");
        assert_eq!(b.count_open(), 1);
        assert_eq!(b.list(Some("dismissed")).len(), 1);
        assert_eq!(b.list(Some("open"))[0].id, q.id);
        assert_eq!(b.list(None).len(), 2, "the file is kept");
        let mut s = MemSink::default();
        assert!(matches!(apply(&mut s, &b, &p.id, None), Err(KnowError::Invalid(_))));
    }

    // step 2

    #[test]
    fn updates_show_what_differs() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let mut a = note("Old");
        a.tag(facet::USE, "paper");
        let a = s.put(a).unwrap();
        let mut now_a = a.clone();
        now_a.title = "New".into();
        now_a.set("body", "other");
        now_a.tag(facet::KIND, "timeline");
        now_a.triggers.push("softmax".into());
        now_a.contains.push("my.notes:x".into());
        let p = b.create("le", "t", vec![update(now_a), add("Z", ""), remove(&a.id)]).unwrap();
        let v = show(&s, &p).unwrap();
        assert_eq!(v.len(), 3);
        for k in ["title", "body", "tags.kind", "triggers", "contains"] {
            assert!(v[0].changed.iter().any(|c| c == k), "{k} missing in {:?}", v[0].changed);
        }
        assert_eq!(v[0].changed.len(), 5);
        assert_eq!(v[0].before.as_ref().unwrap().title, "Old");
        assert_eq!(v[0].after.as_ref().unwrap().title, "New");
        assert!(v[0].problem.is_empty());
        assert!(v[1].before.is_none() && v[1].after.is_some() && v[1].problem.is_empty());
        assert_eq!((v[2].op.as_str(), v[2].before.as_ref().unwrap().id.as_str()), ("remove", a.id.as_str()));
        assert_eq!(v[2].index, 2);
    }

    #[test]
    fn a_change_to_a_changed_item_is_flagged() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let a = stored(&mut s, "A");
        let c = stored(&mut s, "C");
        let mut up = a.clone();
        up.title = "A2".into();
        let p = b.create("le", "t", vec![update(up), remove(&c.id), remove("my.notes:gone")]).unwrap();
        s.put(a.clone()).unwrap(); // A moves on after the proposal
        let v = show(&s, &p).unwrap();
        assert!(v[0].problem.contains("被改過"), "{}", v[0].problem);
        assert!(v[1].problem.is_empty());
        assert!(v[2].problem.contains("不存在") && v[2].before.is_none());
        let mut gone = a.clone();
        gone.id = "my.notes:gone".into();
        let p2 = b.create("le", "t", vec![update(gone)]).unwrap();
        let v2 = show(&s, &p2).unwrap();
        assert!(v2[0].problem.contains("不存在") && v2[0].before.is_none());
    }

    // step 3

    #[test]
    fn approving_all_writes_every_change() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let a = stored(&mut s, "A");
        let c = stored(&mut s, "C");
        let mut up = a.clone();
        up.title = "A2".into();
        let p = b.create("le", "t", vec![add("N", ""), update(up), remove(&c.id)]).unwrap();
        let r = apply(&mut s, &b, &p.id, None).unwrap();
        assert_eq!(r.state, "applied");
        assert!(r.changes.iter().all(|c| c.result == "applied" && c.error.is_empty()));
        assert_eq!(s.items[&a.id].title, "A2");
        assert!(s.get(&c.id).unwrap().is_none());
        assert_eq!(s.list("my.notes").unwrap().len(), 2);
        assert_eq!(b.get(&p.id).unwrap(), r);
        assert_eq!(b.count_open(), 0);
        assert!(b.undo_path(&p.id).unwrap().exists());
        assert_eq!(b.list(None).len(), 1, "the undo record is not a proposal");
    }

    #[test]
    fn approving_some_skips_the_rest() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let c = stored(&mut s, "C");
        let p = b.create("le", "t", vec![add("N1", ""), add("N2", ""), remove(&c.id)]).unwrap();
        let r = apply(&mut s, &b, &p.id, Some(&[1])).unwrap();
        let results: Vec<&str> = r.changes.iter().map(|c| c.result.as_str()).collect();
        assert_eq!(results, ["skipped", "applied", "skipped"]);
        assert_eq!(r.state, "applied");
        let titles: Vec<String> = s.list("my.notes").unwrap().into_iter().map(|i| i.title).collect();
        assert!(titles.contains(&"N2".into()) && titles.contains(&"C".into()) && !titles.contains(&"N1".into()));
        let q = b.create("le", "t", vec![add("M", "")]).unwrap();
        assert!(matches!(apply(&mut s, &b, &q.id, Some(&[5])), Err(KnowError::Invalid(_))));
        assert!(matches!(apply(&mut s, &b, &q.id, Some(&[])), Err(KnowError::Invalid(_))));
        assert_eq!(b.get(&q.id).unwrap().state, "open");
    }

    #[test]
    fn temporary_ids_link_the_new_items_together() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let old = stored(&mut s, "Old");
        let mut fig = add("figure", "new:1");
        let mut m1 = add("module", "new:2");
        let m2 = add("dropped module", "new:3");
        fig.item.as_mut().unwrap().contains = vec!["new:2".into(), "new:3".into(), old.id.clone()];
        m1.item.as_mut().unwrap().contains = vec!["new:1".into()];
        let mut up = old.clone();
        up.contains = vec!["new:1".into(), "new:3".into()];
        let p = b.create("le", "t", vec![fig, m1, m2, update(up)]).unwrap();
        let r = apply(&mut s, &b, &p.id, Some(&[0, 1, 3])).unwrap();
        assert!(r.changes.iter().map(|c| c.result.as_str()).eq(["applied", "applied", "skipped", "applied"]));
        let by_title = |t: &str| s.items.values().find(|i| i.title == t).unwrap().clone();
        let (f, m) = (by_title("figure"), by_title("module"));
        assert!(!f.id.contains("new:") && !m.id.contains("new:"));
        assert_eq!(f.contains, vec![m.id.clone(), old.id.clone()], "new:3 was not chosen and is dropped");
        assert_eq!(m.contains, vec![f.id.clone()]);
        assert_eq!(s.items[&old.id].contains, vec![f.id.clone()]);
        assert!(s.items.values().all(|i| i.contains.iter().all(|c| !c.starts_with("new:"))));
        // undo takes the linked new items out again even though the link was a second write
        assert!(undo(&mut s, &b, &p.id).unwrap().is_empty());
        assert_eq!(s.items.len(), 1);
        assert!(s.items[&old.id].contains.is_empty());
    }

    #[test]
    fn a_stale_update_is_refused_and_the_others_still_apply() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let a = stored(&mut s, "A");
        let c = stored(&mut s, "C");
        let mut ua = a.clone();
        ua.title = "A2".into();
        let mut uc = c.clone();
        uc.title = "C2".into();
        let p = b.create("le", "t", vec![update(ua), add("N", ""), update(uc)]).unwrap();
        s.put(a.clone()).unwrap(); // A moves on
        let r = apply(&mut s, &b, &p.id, None).unwrap();
        assert_eq!(r.state, "applied");
        assert_eq!(r.changes[0].result, "refused");
        assert!(r.changes[0].error.contains("改過"), "{}", r.changes[0].error);
        assert_eq!((r.changes[1].result.as_str(), r.changes[2].result.as_str()), ("applied", "applied"));
        assert_eq!(s.items[&a.id].title, "A");
        assert_eq!(s.items[&c.id].title, "C2");
        assert_eq!(read_undo(&b, &p.id).unwrap().steps.len(), 2, "the refused update is not in the undo record");
        // an update of an item that is gone is refused and does not re-create it
        let q = b.create("le", "t", vec![update(c.clone())]).unwrap();
        s.remove(&c.id).unwrap();
        let r = apply(&mut s, &b, &q.id, None).unwrap();
        assert_eq!(r.changes[0].result, "refused");
        assert!(s.get(&c.id).unwrap().is_none());
    }

    #[test]
    fn a_decided_proposal_cannot_be_applied_again() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let p = b.create("le", "t", vec![add("N", "")]).unwrap();
        apply(&mut s, &b, &p.id, None).unwrap();
        assert!(matches!(apply(&mut s, &b, &p.id, None), Err(KnowError::Invalid(_))));
        assert_eq!(s.items.len(), 1);
        assert!(matches!(apply(&mut s, &b, "pnone", None), Err(KnowError::NotFound(_))));
    }

    // step 4

    #[test]
    fn undo_puts_everything_back() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let a = stored(&mut s, "A");
        let c = stored(&mut s, "C");
        let mut up = a.clone();
        up.title = "A2".into();
        up.set("body", json!("changed"));
        let p = b.create("le", "t", vec![add("N", ""), update(up), remove(&c.id)]).unwrap();
        apply(&mut s, &b, &p.id, None).unwrap();
        let lines = undo(&mut s, &b, &p.id).unwrap();
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(s.items.len(), 2);
        let (ra, rc) = (&s.items[&a.id], &s.items[&c.id]);
        assert_eq!((ra.title.as_str(), ra.field("body")), ("A", "text"));
        assert_eq!(rc.title, "C");
        let back = b.get(&p.id).unwrap();
        assert_eq!(back.state, "open");
        assert!(back.changes.iter().all(|c| c.result.is_empty()));
        assert!(!b.undo_path(&p.id).unwrap().exists());
        // it can be decided again
        assert_eq!(apply(&mut s, &b, &p.id, Some(&[0])).unwrap().state, "applied");
        assert!(matches!(undo(&mut s, &b, "pnone"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn undo_does_not_overwrite_a_later_edit() {
        let (_d, b) = boxed();
        let mut s = MemSink::default();
        let a = stored(&mut s, "A");
        let c = stored(&mut s, "C");
        let mut up = a.clone();
        up.title = "A2".into();
        let p = b.create("le", "t", vec![update(up), remove(&c.id), add("N", "")]).unwrap();
        apply(&mut s, &b, &p.id, None).unwrap();
        // the author edits A again
        let mut later = s.items[&a.id].clone();
        later.title = "A by hand".into();
        s.put(later).unwrap();
        let lines = undo(&mut s, &b, &p.id).unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(&a.id) && lines[0].contains("被改過"));
        assert_eq!(s.items[&a.id].title, "A by hand");
        assert!(s.items.contains_key(&c.id), "the rest was undone");
        assert_eq!(s.items.len(), 2);
        assert_eq!(b.get(&p.id).unwrap().state, "applied");
        // only the step that could not be undone is kept
        assert_eq!(read_undo(&b, &p.id).unwrap().steps.len(), 1);
    }
}
