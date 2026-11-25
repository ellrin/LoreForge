//! 確認 (review): what a model wrote for the author's own sentences (another way to say a
//! pattern, a word that may be swapped in it) waits here, one candidate at a time. Nothing is in
//! his data until he accepts that one candidate; a candidate he refused is remembered and never
//! offered again; either decision can be taken back on its own.
//!
//! ```text
//! <hub>/review/<set id>.json     one Set: its candidates with their state
//! ```
//!
//! Accepting writes one item of the set's category (format `variant`) through [`Sink`]; taking
//! it back removes that item. A proposal (`proposal.rs`) is a batch decided once; a set is
//! decided candidate by candidate over many sittings.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::*;
use crate::util::write_atomic;

/// `Candidate::state`.
pub const OPEN: &str = "open";
pub const ACCEPTED: &str = "accepted";
pub const REJECTED: &str = "rejected";

/// One thing the author did not write, waiting for his decision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Candidate {
    /// Unique in its set: `c<n>` in the order given (set by [`Reviews::create`]).
    pub id: String,
    /// The item id of the author's own item it belongs to (`pc.patterns:P001`).
    pub of: String,
    /// `variant` (the whole sentence said another way) or `swap` (one word for `slot`).
    pub kind: String,
    pub text: String,
    /// `swap` only: the word of the author's sentence that `text` may replace.
    pub slot: String,
    /// One short line: when this one fits (emphasis, length, context).
    pub note: String,
    /// [`OPEN`], [`ACCEPTED`] or [`REJECTED`].
    pub state: String,
    /// The stored item's id while accepted, "" otherwise.
    pub item: String,
    /// RFC 3339, "" while open.
    pub decided_at: String,
}

/// A batch of candidates from one run of a tool.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Set {
    /// `r` + a time-sortable id (set by [`Reviews::create`]).
    pub id: String,
    /// Who made it: a Lore app id or a tool's name (`relabel`).
    pub by: String,
    pub title: String,
    /// The category accepted candidates are written to (`pc.variants`); its format is `variant`.
    pub category: String,
    pub created_at: String,
    pub candidates: Vec<Candidate>,
}

/// How many candidates of a set are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub open: usize,
    pub accepted: usize,
    pub rejected: usize,
}

/// The author's own item with the candidates that belong to it, in the order they were given.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// `Candidate::of`.
    pub of: String,
    /// The stored item now; `None` when it no longer exists (its candidates cannot be accepted).
    pub own: Option<Item>,
    pub candidates: Vec<Candidate>,
}

/// The box of sets: a folder with `<id>.json` per set.
#[derive(Debug, Clone)]
pub struct Reviews {
    dir: PathBuf,
}

impl Reviews {
    /// The box in `dir` (made when missing).
    pub fn open(dir: impl Into<PathBuf>) -> Result<Reviews> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Reviews { dir })
    }

    /// The folder (for the implementation and tests).
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        let ok = id.starts_with('r') && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
        if !ok {
            return Err(KnowError::NotFound(id.into()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn save(&self, set: &Set) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(set).map_err(|e| KnowError::Other(e.to_string()))?;
        write_atomic(&self.path(&set.id)?, &bytes)
    }

    /// A new set, every candidate [`OPEN`] with ids `c1`, `c2` … Refused: no candidate, an empty
    /// `category`, a candidate without `of` or `text`, a `kind` other than `variant` / `swap`, a
    /// `swap` without `slot`. A candidate whose (`of`, `kind`, `slot`, `text`) was already
    /// [`REJECTED`] or is already waiting or accepted in any earlier set is left out (he is never
    /// asked twice); when nothing is left the set is not made and `Ok(None)` comes back.
    pub fn create(&self, by: &str, title: &str, category: &str, candidates: Vec<Candidate>) -> Result<Option<Set>> {
        let category = category.trim();
        if category.is_empty() {
            return Err(KnowError::Invalid("確認沒有指明存到哪一類".into()));
        }
        if candidates.is_empty() {
            return Err(KnowError::Invalid("確認沒有任何一條".into()));
        }
        let mut wanted = vec![];
        for (i, c) in candidates.into_iter().enumerate() {
            let n = i + 1;
            if c.of.trim().is_empty() {
                return Err(KnowError::Invalid(format!("第 {n} 條：沒有指明是哪一句的")));
            }
            if c.text.trim().is_empty() {
                return Err(KnowError::Invalid(format!("第 {n} 條：沒有內容")));
            }
            match c.kind.as_str() {
                "variant" => {}
                "swap" => {
                    if c.slot.trim().is_empty() {
                        return Err(KnowError::Invalid(format!("第 {n} 條：換字沒有指明換哪個字")));
                    }
                }
                k => return Err(KnowError::Invalid(format!("第 {n} 條：不認得的種類「{k}」"))),
            }
            wanted.push(Candidate {
                id: String::new(),
                of: c.of.trim().into(),
                kind: c.kind,
                text: c.text.trim().into(),
                slot: c.slot.trim().into(),
                note: c.note.trim().into(),
                state: OPEN.into(),
                item: String::new(),
                decided_at: String::new(),
            });
        }
        let key = |c: &Candidate| (c.of.clone(), c.kind.clone(), c.slot.clone(), c.text.clone());
        let mut known: BTreeSet<_> = self.list().iter().flat_map(|s| s.candidates.iter().map(key)).collect();
        let mut kept = vec![];
        for c in wanted {
            if known.insert(key(&c)) {
                kept.push(c);
            }
        }
        if kept.is_empty() {
            return Ok(None);
        }
        for (i, c) in kept.iter_mut().enumerate() {
            c.id = format!("c{}", i + 1);
        }
        let ms = chrono::Local::now().timestamp_millis().max(0) as u64;
        let set = Set {
            id: format!("r{}", &new_local(ms)[1..]),
            by: by.trim().into(),
            title: title.trim().into(),
            category: category.into(),
            created_at: now(),
            candidates: kept,
        };
        self.save(&set)?;
        Ok(Some(set))
    }

    /// [`KnowError::NotFound`] for an unknown or malformed id.
    pub fn get(&self, id: &str) -> Result<Set> {
        let path = self.path(id)?;
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { KnowError::NotFound(id.into()) } else { e.into() }
        })?;
        serde_json::from_slice(&bytes).map_err(|e| KnowError::Other(format!("確認 {id} 讀不了：{e}")))
    }

    /// Every set, oldest first. A file that cannot be read is skipped.
    pub fn list(&self) -> Vec<Set> {
        let Ok(rd) = fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Set> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !name.ends_with(".json") {
                    return None;
                }
                serde_json::from_slice::<Set>(&fs::read(e.path()).ok()?).ok()
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Candidates still open, over every set.
    pub fn count_open(&self) -> usize {
        self.list().iter().map(|s| counts(s).open).sum()
    }
}

/// The counts of one set.
pub fn counts(set: &Set) -> Counts {
    let mut n = Counts::default();
    for c in &set.candidates {
        match c.state.as_str() {
            ACCEPTED => n.accepted += 1,
            REJECTED => n.rejected += 1,
            _ => n.open += 1,
        }
    }
    n
}

/// The candidates of `set` grouped by the item they belong to, groups in the order of their
/// first candidate, each with that item as stored now.
pub fn groups(sink: &dyn Sink, set: &Set) -> Result<Vec<Group>> {
    let mut out: Vec<Group> = vec![];
    for c in &set.candidates {
        match out.iter_mut().find(|g| g.of == c.of) {
            Some(g) => g.candidates.push(c.clone()),
            None => out.push(Group { of: c.of.clone(), own: sink.get(&c.of)?, candidates: vec![c.clone()] }),
        }
    }
    Ok(out)
}

/// The set and the position of one candidate in it.
fn find(box_: &Reviews, set: &str, candidate: &str) -> Result<(Set, usize)> {
    let s = box_.get(set)?;
    let at = s.candidates.iter().position(|c| c.id == candidate).ok_or_else(|| KnowError::NotFound(candidate.into()))?;
    Ok((s, at))
}

/// Accept one open candidate: one new item of `set.category` (format `variant`; title = `text`;
/// fields `pattern` = the local id of `of`, `kind`, `text`, `slot`, `note`; source
/// `Source::lore(set.by)`) is written through `sink`, the candidate becomes [`ACCEPTED`] with
/// the stored id in `item`. Refused when the candidate is not open, or the item `of` no longer
/// exists. The set file is saved only after the item was stored. Returns the candidate.
pub fn accept(sink: &mut dyn Sink, box_: &Reviews, set: &str, candidate: &str) -> Result<Candidate> {
    let (mut s, at) = find(box_, set, candidate)?;
    let c = s.candidates[at].clone();
    if c.state != OPEN {
        return Err(KnowError::Invalid("這一條已經決定過了".into()));
    }
    if sink.get(&c.of)?.is_none() {
        return Err(KnowError::Invalid("原來的那一句已經不在了，這一條不能收".into()));
    }
    let local = split_id(&c.of).map(|(_, l)| l).unwrap_or(&c.of).to_string();
    let mut item = Item::new(&s.category, format::VARIANT, &c.text, Source::lore(&s.by));
    item.set("pattern", local.as_str());
    item.set("kind", c.kind.as_str());
    item.set("text", c.text.as_str());
    item.set("slot", c.slot.as_str());
    item.set("note", c.note.as_str());
    let stored = sink.put(item)?;
    s.candidates[at].state = ACCEPTED.into();
    s.candidates[at].item = stored.id.clone();
    s.candidates[at].decided_at = now();
    if let Err(e) = box_.save(&s) {
        let _ = sink.remove(&stored.id);
        return Err(e);
    }
    Ok(s.candidates[at].clone())
}

/// Refuse one open candidate: it becomes [`REJECTED`]; nothing is written to the data.
pub fn reject(box_: &Reviews, set: &str, candidate: &str) -> Result<Candidate> {
    let (mut s, at) = find(box_, set, candidate)?;
    if s.candidates[at].state != OPEN {
        return Err(KnowError::Invalid("這一條已經決定過了".into()));
    }
    s.candidates[at].state = REJECTED.into();
    s.candidates[at].decided_at = now();
    box_.save(&s)?;
    Ok(s.candidates[at].clone())
}

/// Take a decision back: an accepted candidate's item is removed through `sink` (already gone:
/// fine) and a rejected one is simply reopened; the candidate is [`OPEN`] again with `item` and
/// `decided_at` empty. Refused when the candidate is open.
pub fn undo(sink: &mut dyn Sink, box_: &Reviews, set: &str, candidate: &str) -> Result<Candidate> {
    let (mut s, at) = find(box_, set, candidate)?;
    let c = &s.candidates[at];
    match c.state.as_str() {
        ACCEPTED => {
            if !c.item.is_empty() {
                match sink.remove(&c.item) {
                    Ok(_) | Err(KnowError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        REJECTED => {}
        _ => return Err(KnowError::Invalid("這一條還沒有決定，不用收回".into())),
    }
    let c = &mut s.candidates[at];
    c.state = OPEN.into();
    c.item.clear();
    c.decided_at.clear();
    box_.save(&s)?;
    Ok(s.candidates[at].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> (tempfile::TempDir, Reviews) {
        let d = tempfile::tempdir().unwrap();
        let b = Reviews::open(d.path().join("review")).unwrap();
        (d, b)
    }

    fn cand(of: &str, kind: &str, text: &str, slot: &str) -> Candidate {
        Candidate { of: of.into(), kind: kind.into(), text: text.into(), slot: slot.into(), note: "when it fits".into(), ..Default::default() }
    }

    fn variant(of: &str, text: &str) -> Candidate {
        cand(of, "variant", text, "")
    }

    fn pattern(s: &mut MemSink, local: &str) -> Item {
        let mut i = Item::new("pc.patterns", format::PATTERN, "The {thing} was measured with {tool}.", Source::author());
        i.id = item_id("pc.patterns", local);
        s.put(i).unwrap()
    }

    fn one_set(b: &Reviews, cs: Vec<Candidate>) -> Set {
        b.create("relabel", "t", "pc.variants", cs).unwrap().unwrap()
    }

    #[test]
    fn a_set_is_made_and_read_back() {
        let (_d, b) = boxed();
        let s = one_set(&b, vec![variant("pc.patterns:P001", "We measured the {thing} with {tool}."), cand("pc.patterns:P001", "swap", "assessed", "measured")]);
        assert!(s.id.starts_with('r'));
        assert_eq!(s.category, "pc.variants");
        assert_eq!(s.candidates.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["c1", "c2"]);
        assert!(s.candidates.iter().all(|c| c.state == OPEN && c.item.is_empty()));
        assert_eq!(b.get(&s.id).unwrap(), s);
        assert_eq!(b.list(), vec![s.clone()]);
        assert_eq!(counts(&s), Counts { open: 2, accepted: 0, rejected: 0 });
        assert!(matches!(b.get("rnope"), Err(KnowError::NotFound(_))));
        assert!(matches!(b.get("../x"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn malformed_candidates_are_refused() {
        let (_d, b) = boxed();
        let ok = || variant("pc.patterns:P001", "text");
        let cases: Vec<(&str, Vec<Candidate>)> = vec![
            ("pc.variants", vec![]),
            ("", vec![ok()]),
            ("pc.variants", vec![variant("", "text")]),
            ("pc.variants", vec![variant("pc.patterns:P001", " ")]),
            ("pc.variants", vec![cand("pc.patterns:P001", "other", "text", "")]),
            ("pc.variants", vec![cand("pc.patterns:P001", "swap", "text", "")]),
        ];
        for (cat, cs) in cases {
            assert!(matches!(b.create("x", "t", cat, cs), Err(KnowError::Invalid(_))));
        }
        assert!(b.list().is_empty());
    }

    #[test]
    fn a_candidate_decided_or_waiting_earlier_is_not_offered_again() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        pattern(&mut mem, "P001");
        let s1 = one_set(&b, vec![variant("pc.patterns:P001", "one"), variant("pc.patterns:P001", "two")]);
        reject(&b, &s1.id, "c1").unwrap();
        // c1 rejected, c2 still open in set 1: both left out, a new one stays
        let s2 = b.create("x", "t", "pc.variants", vec![variant("pc.patterns:P001", "one"), variant("pc.patterns:P001", "two"), variant("pc.patterns:P001", "three")]).unwrap().unwrap();
        assert_eq!(s2.candidates.len(), 1);
        assert_eq!(s2.candidates[0].text, "three");
        assert_eq!(s2.candidates[0].id, "c1");
        // everything already known: no set, no file
        let before = std::fs::read_dir(b.dir()).unwrap().count();
        assert!(b.create("x", "t", "pc.variants", vec![variant("pc.patterns:P001", "one"), variant("pc.patterns:P001", "three")]).unwrap().is_none());
        assert_eq!(std::fs::read_dir(b.dir()).unwrap().count(), before);
        // accepted ones are not offered again either
        accept(&mut mem, &b, &s2.id, "c1").unwrap();
        assert!(b.create("x", "t", "pc.variants", vec![variant("pc.patterns:P001", "three")]).unwrap().is_none());
        // the same text as a swap of another slot is another candidate
        assert!(b.create("x", "t", "pc.variants", vec![cand("pc.patterns:P001", "swap", "one", "word")]).unwrap().is_some());
    }

    #[test]
    fn an_unreadable_file_is_skipped() {
        let (_d, b) = boxed();
        let s = one_set(&b, vec![variant("pc.patterns:P001", "one")]);
        std::fs::write(b.dir().join("rbroken.json"), b"{ not json").unwrap();
        std::fs::write(b.dir().join("notes.txt"), b"x").unwrap();
        assert_eq!(b.list(), vec![s]);
        assert_eq!(b.count_open(), 1);
    }

    #[test]
    fn count_open_counts_over_every_set() {
        let (_d, b) = boxed();
        assert_eq!(b.count_open(), 0);
        let s1 = one_set(&b, vec![variant("pc.patterns:P001", "a"), variant("pc.patterns:P001", "b")]);
        let _s2 = one_set(&b, vec![variant("pc.patterns:P002", "c")]);
        assert_eq!(b.count_open(), 3);
        reject(&b, &s1.id, "c1").unwrap();
        assert_eq!(b.count_open(), 2);
        assert_eq!(b.list().len(), 2);
    }

    #[test]
    fn groups_keep_the_order_and_carry_the_own_item() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        let own = pattern(&mut mem, "P001");
        let s = one_set(&b, vec![variant("pc.patterns:P002", "a"), variant("pc.patterns:P001", "b"), variant("pc.patterns:P002", "c")]);
        let g = groups(&mem, &s).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].of, "pc.patterns:P002");
        assert!(g[0].own.is_none());
        assert_eq!(g[0].candidates.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        assert_eq!(g[1].of, "pc.patterns:P001");
        assert_eq!(g[1].own.as_ref().unwrap().id, own.id);
        assert_eq!(g[1].candidates.len(), 1);
    }

    #[test]
    fn accepting_writes_one_item_and_marks_the_candidate() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        pattern(&mut mem, "P001");
        let s = one_set(&b, vec![cand("pc.patterns:P001", "swap", "assessed", "measured")]);
        let c = accept(&mut mem, &b, &s.id, "c1").unwrap();
        assert_eq!(c.state, ACCEPTED);
        assert!(!c.decided_at.is_empty());
        let item = mem.get(&c.item).unwrap().unwrap();
        assert_eq!(item.category, "pc.variants");
        assert_eq!(item.format, format::VARIANT);
        assert_eq!(item.title, "assessed");
        assert_eq!(item.field("pattern"), "P001");
        assert_eq!(item.field("kind"), "swap");
        assert_eq!(item.field("text"), "assessed");
        assert_eq!(item.field("slot"), "measured");
        assert_eq!(item.field("note"), "when it fits");
        assert_eq!(item.source, Source::lore("relabel"));
        assert_eq!(mem.list("pc.variants").unwrap().len(), 1);
        assert_eq!(b.get(&s.id).unwrap().candidates[0], c);
    }

    #[test]
    fn accepting_twice_or_a_missing_own_item_is_refused() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        pattern(&mut mem, "P001");
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a"), variant("pc.patterns:P009", "b")]);
        accept(&mut mem, &b, &s.id, "c1").unwrap();
        assert!(matches!(accept(&mut mem, &b, &s.id, "c1"), Err(KnowError::Invalid(_))));
        assert!(matches!(accept(&mut mem, &b, &s.id, "c2"), Err(KnowError::Invalid(_))));
        assert_eq!(mem.list("pc.variants").unwrap().len(), 1);
        assert_eq!(b.get(&s.id).unwrap().candidates[1].state, OPEN);
        assert!(matches!(accept(&mut mem, &b, &s.id, "c7"), Err(KnowError::NotFound(_))));
        assert!(matches!(accept(&mut mem, &b, "rnone", "c1"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn refusing_writes_nothing() {
        let (_d, b) = boxed();
        let mem = MemSink::default();
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a")]);
        let c = reject(&b, &s.id, "c1").unwrap();
        assert_eq!(c.state, REJECTED);
        assert!(!c.decided_at.is_empty());
        assert!(mem.items.is_empty());
        assert!(matches!(reject(&b, &s.id, "c1"), Err(KnowError::Invalid(_))));
        assert_eq!(counts(&b.get(&s.id).unwrap()), Counts { open: 0, accepted: 0, rejected: 1 });
    }

    #[test]
    fn undo_of_an_accepted_one_removes_its_item_and_reopens() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        pattern(&mut mem, "P001");
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a")]);
        let c = accept(&mut mem, &b, &s.id, "c1").unwrap();
        let back = undo(&mut mem, &b, &s.id, "c1").unwrap();
        assert_eq!(back.state, OPEN);
        assert!(back.item.is_empty() && back.decided_at.is_empty());
        assert!(mem.get(&c.item).unwrap().is_none());
        assert_eq!(b.get(&s.id).unwrap().candidates[0], back);
        accept(&mut mem, &b, &s.id, "c1").unwrap();
        assert_eq!(mem.list("pc.variants").unwrap().len(), 1);
    }

    #[test]
    fn undo_of_a_refused_one_reopens() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a")]);
        reject(&b, &s.id, "c1").unwrap();
        let back = undo(&mut mem, &b, &s.id, "c1").unwrap();
        assert_eq!(back.state, OPEN);
        assert!(back.decided_at.is_empty());
    }

    #[test]
    fn undo_of_an_open_one_is_refused() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a")]);
        assert!(matches!(undo(&mut mem, &b, &s.id, "c1"), Err(KnowError::Invalid(_))));
        assert!(matches!(undo(&mut mem, &b, &s.id, "c9"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn undo_when_the_item_is_already_gone_still_reopens() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        pattern(&mut mem, "P001");
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a")]);
        let c = accept(&mut mem, &b, &s.id, "c1").unwrap();
        mem.remove(&c.item).unwrap();
        assert_eq!(undo(&mut mem, &b, &s.id, "c1").unwrap().state, OPEN);
    }

    #[test]
    fn one_decision_does_not_touch_the_others() {
        let (_d, b) = boxed();
        let mut mem = MemSink::default();
        pattern(&mut mem, "P001");
        let s = one_set(&b, vec![variant("pc.patterns:P001", "a"), variant("pc.patterns:P001", "b"), variant("pc.patterns:P001", "c")]);
        accept(&mut mem, &b, &s.id, "c1").unwrap();
        reject(&b, &s.id, "c2").unwrap();
        assert_eq!(b.get(&s.id).unwrap().candidates[2].state, OPEN);
        undo(&mut mem, &b, &s.id, "c2").unwrap();
        let got = b.get(&s.id).unwrap();
        assert_eq!(got.candidates[0].state, ACCEPTED);
        assert_eq!(got.candidates[1].state, OPEN);
        assert_eq!(counts(&got), Counts { open: 2, accepted: 1, rejected: 0 });
        assert_eq!(mem.list("pc.variants").unwrap().len(), 1);
    }
}
