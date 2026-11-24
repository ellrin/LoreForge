//! The inbox store (docs/brain-hub.md §16, package I1): `<hub>/inbox/<id>.json`, one file per
//! [`Entry`]. Three sides use it:
//!
//! * hand-over: [`Inbox::add`] cuts words into sentences, one waiting entry each;
//! * the worker: [`Inbox::next`], [`Inbox::ready`], [`Inbox::failed`], [`Inbox::recover`];
//! * the author: [`Inbox::accept`], [`Inbox::skip`], [`Inbox::retry`], [`Inbox::clear`].
//!
//! Read-modify-write steps are serialised by one process-wide lock, so two threads never take
//! the same entry; two processes on one folder are not guarded against each other.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::inbox::{Counts, Entry, From as Origin, Made, sentences, state};
use crate::types::*;
use crate::util::write_atomic;

static LOCK: Mutex<()> = Mutex::new(());
static LAST_MS: AtomicU64 = AtomicU64::new(0);

fn other<E: std::fmt::Display>(e: E) -> KnowError {
    KnowError::Other(e.to_string())
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A millisecond clock that never repeats or goes back inside this process, so ids made one
/// after another sort in the order they were made.
fn next_ms() -> u64 {
    let now = chrono::Local::now().timestamp_millis().max(0) as u64;
    let mut last = LAST_MS.load(Ordering::SeqCst);
    loop {
        let v = now.max(last + 1);
        match LAST_MS.compare_exchange(last, v, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return v,
            Err(l) => last = l,
        }
    }
}

/// Lower case, whitespace collapsed: how two sentences are compared.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn bad_state(e: &Entry, wanted: &str) -> KnowError {
    KnowError::Invalid(format!("這一筆現在是「{}」，不能這樣做（需要：{wanted}）", e.state))
}

/// The inbox folder.
#[derive(Debug, Clone)]
pub struct Inbox {
    dir: PathBuf,
}

impl Inbox {
    /// The inbox in `dir` (made when missing).
    pub fn open(dir: impl Into<PathBuf>) -> Result<Inbox> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Inbox { dir })
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty() || id.contains(['/', '\\', ':']) || id.starts_with('.') {
            return Err(KnowError::NotFound(id.into()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn save(&self, e: &Entry) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(e).map_err(other)?;
        write_atomic(&self.path(&e.id)?, &bytes)
    }

    fn all(&self) -> Vec<Entry> {
        let Ok(rd) = fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !name.ends_with(".json") {
                    return None;
                }
                serde_json::from_slice::<Entry>(&fs::read(e.path()).ok()?).ok()
            })
            .filter(|e| !e.id.is_empty())
            .collect();
        // ids sort by creation time
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    // ── hand-over ────────────────────────────────────────────────────────────

    /// One waiting entry per sentence of `text` ([`sentences`]), oldest first. A sentence that
    /// is already waiting, working or ready in the same category (compared ignoring case and
    /// spacing) is not added again and is not returned. Nothing to add is not an error.
    pub fn add(&self, category: &str, text: &str, own: bool, from: Origin) -> Result<Vec<Entry>> {
        let _g = lock();
        let category = category.trim();
        if category.is_empty() {
            return Err(KnowError::Invalid("沒有類別".into()));
        }
        let mut seen: Vec<String> = self
            .all()
            .into_iter()
            .filter(|e| e.category == category && matches!(e.state.as_str(), state::WAITING | state::WORKING | state::READY))
            .map(|e| norm(&e.text))
            .collect();
        let mut out = vec![];
        for s in sentences(text) {
            let key = norm(&s);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let t = now();
            let e = Entry {
                id: format!("i{}", new_local(next_ms())),
                category: category.into(),
                text: s,
                own,
                from: from.clone(),
                state: state::WAITING.into(),
                created_at: t.clone(),
                updated_at: t,
                ..Default::default()
            };
            self.save(&e)?;
            out.push(e);
        }
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Entry> {
        let path = self.path(id)?;
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { KnowError::NotFound(id.into()) } else { e.into() }
        })?;
        serde_json::from_slice(&bytes).map_err(|e| KnowError::Other(format!("收件 {id} 讀不了：{e}")))
    }

    /// Entries in any of `states` (empty = all), oldest first. Unreadable files are skipped.
    pub fn list(&self, states: &[&str]) -> Vec<Entry> {
        self.all().into_iter().filter(|e| states.is_empty() || states.contains(&e.state.as_str())).collect()
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for e in self.all() {
            match e.state.as_str() {
                state::WAITING => c.waiting += 1,
                state::WORKING => c.working += 1,
                state::READY => c.ready += 1,
                state::FAILED => c.failed += 1,
                _ => {}
            }
        }
        c
    }

    // ── the worker's side ────────────────────────────────────────────────────

    /// The oldest waiting entry, marked `working` and saved; two calls never return the same one.
    pub fn next(&self) -> Result<Option<Entry>> {
        let _g = lock();
        let Some(mut e) = self.all().into_iter().find(|e| e.state == state::WAITING) else { return Ok(None) };
        e.state = state::WORKING.into();
        e.updated_at = now();
        self.save(&e)?;
        Ok(Some(e))
    }

    /// The worker's result: state `ready`, with `result` (its id emptied), `similar` and
    /// `engine`. Allowed from `working` (or `waiting`, when a recovery put it back meanwhile).
    /// When the entry is not `own`, the result must not keep the author's wording: if
    /// `result.fields["example"]` equals the entry's text (ignoring case and spacing) it is set
    /// to "".
    pub fn ready(&self, id: &str, made: Made, engine: &str) -> Result<Entry> {
        let _g = lock();
        let mut e = self.get(id)?;
        if !matches!(e.state.as_str(), state::WORKING | state::WAITING) {
            return Err(bad_state(&e, "處理中"));
        }
        let mut item = made.item;
        item.id.clear();
        if !e.own && item.fields.get("example").and_then(|v| v.as_str()).is_some_and(|x| norm(x) == norm(&e.text)) {
            item.fields.insert("example".into(), serde_json::Value::String(String::new()));
        }
        e.result = Some(item);
        e.similar = made.similar;
        e.engine = engine.into();
        e.error.clear();
        e.state = state::READY.into();
        e.updated_at = now();
        self.save(&e)?;
        Ok(e)
    }

    /// The worker could not: state `failed`, `error` = one line why.
    pub fn failed(&self, id: &str, error: &str) -> Result<Entry> {
        let _g = lock();
        let mut e = self.get(id)?;
        if !matches!(e.state.as_str(), state::WORKING | state::WAITING) {
            return Err(bad_state(&e, "處理中"));
        }
        e.error = error.lines().next().unwrap_or("").trim().into();
        e.state = state::FAILED.into();
        e.updated_at = now();
        self.save(&e)?;
        Ok(e)
    }

    /// Entries a stopped worker left `working` go back to `waiting` (called at start). Returns how many.
    pub fn recover(&self) -> Result<usize> {
        let _g = lock();
        let mut n = 0;
        for mut e in self.all().into_iter().filter(|e| e.state == state::WORKING) {
            e.state = state::WAITING.into();
            e.updated_at = now();
            self.save(&e)?;
            n += 1;
        }
        Ok(n)
    }

    // ── the author's side ────────────────────────────────────────────────────

    /// 加入: only from `ready`. Writes `edited` (when given) or the result through `sink` as
    /// `Source::author()` in the entry's category with an empty id; the entry becomes `added`
    /// with the stored id. A refused write leaves the entry `ready` and returns the error.
    pub fn accept(&self, sink: &mut dyn Sink, id: &str, edited: Option<Item>) -> Result<(Entry, Item)> {
        let _g = lock();
        let mut e = self.get(id)?;
        if e.state != state::READY {
            return Err(bad_state(&e, "待確認"));
        }
        let mut item = edited.or_else(|| e.result.clone()).ok_or_else(|| KnowError::Invalid("這一筆沒有結果".into()))?;
        item.id.clear();
        item.revision = 0;
        item.created_at.clear();
        item.updated_at.clear();
        item.source = Source::author();
        item.category = e.category.clone();
        let stored = sink.put(item)?;
        e.state = state::ADDED.into();
        e.added = stored.id.clone();
        e.updated_at = now();
        self.save(&e)?;
        Ok((e, stored))
    }

    /// 略過: from `ready`, `failed` or `waiting`.
    pub fn skip(&self, id: &str) -> Result<Entry> {
        let _g = lock();
        let mut e = self.get(id)?;
        if !matches!(e.state.as_str(), state::READY | state::FAILED | state::WAITING) {
            return Err(bad_state(&e, "待確認、失敗或等待"));
        }
        e.state = state::SKIPPED.into();
        e.updated_at = now();
        self.save(&e)?;
        Ok(e)
    }

    /// 重試: from `failed` back to `waiting`, the error cleared.
    pub fn retry(&self, id: &str) -> Result<Entry> {
        let _g = lock();
        let mut e = self.get(id)?;
        if e.state != state::FAILED {
            return Err(bad_state(&e, "失敗"));
        }
        e.state = state::WAITING.into();
        e.error.clear();
        e.updated_at = now();
        self.save(&e)?;
        Ok(e)
    }

    /// Deletes `added` and `skipped` entries last changed more than `older_than_days` days
    /// before `today`; never others. Returns how many were deleted.
    pub fn clear(&self, older_than_days: i64, today: chrono::NaiveDate) -> usize {
        let _g = lock();
        let mut n = 0;
        for e in self.all() {
            if !matches!(e.state.as_str(), state::ADDED | state::SKIPPED) {
                continue;
            }
            let Ok(t) = chrono::DateTime::parse_from_rfc3339(&e.updated_at) else { continue };
            if (today - t.date_naive()).num_days() > older_than_days && self.path(&e.id).is_ok_and(|p| fs::remove_file(p).is_ok()) {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const S1: &str = "We train the model for ten epochs.";
    const S2: &str = "Does the loss fall below the baseline?";
    const S3: &str = "The learning rate is halved after each plateau.";

    fn boxed() -> (tempfile::TempDir, Inbox) {
        let d = tempfile::tempdir().unwrap();
        let b = Inbox::open(d.path().join("inbox")).unwrap();
        (d, b)
    }

    fn from() -> Origin {
        Origin { via: "voice".into(), app: String::new(), doc: String::new() }
    }

    fn made(example: &str) -> Made {
        let mut item = Item::new("pc.patterns", "pattern", "We train X for N epochs.", Source::lore("kb"));
        item.id = "stale-id".into();
        item.fields.insert("pattern".into(), json!("We train {model} for {n} epochs."));
        item.fields.insert("example".into(), json!(example));
        Made { item, similar: vec!["pc.patterns:a".into()] }
    }

    fn one(b: &Inbox, text: &str, own: bool) -> Entry {
        b.add("pc.patterns", text, own, from()).unwrap().remove(0)
    }

    #[test]
    fn a_hand_over_becomes_one_entry_per_sentence() {
        let (_d, b) = boxed();
        let es = b.add("pc.patterns", &format!("{S1} {S2}\n{S3}"), true, from()).unwrap();
        assert_eq!(es.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec![S1, S2, S3]);
        assert!(es.windows(2).all(|w| w[0].id < w[1].id));
        for e in &es {
            assert!(e.id.starts_with('i') && e.state == state::WAITING && e.own && e.from.via == "voice");
            assert!(!e.created_at.is_empty() && e.created_at == e.updated_at);
            assert_eq!(b.get(&e.id).unwrap(), *e);
        }
        assert!(b.add("pc.patterns", "Yes.", false, from()).unwrap().is_empty());
        assert!(b.get("../x").is_err() && b.get("").is_err() && b.get("a/b").is_err());
        assert!(matches!(b.get("inope"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn the_same_sentence_is_not_queued_twice() {
        let (_d, b) = boxed();
        assert_eq!(b.add("pc.patterns", S1, false, from()).unwrap().len(), 1);
        // same words, other case and spacing; also twice inside one hand-over
        let again = b.add("pc.patterns", &format!("we  TRAIN the model for ten epochs. {S2} {S2}"), false, from()).unwrap();
        assert_eq!(again.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec![S2]);
        // another category takes it
        assert_eq!(b.add("pc.terms", S1, false, from()).unwrap().len(), 1);
        // a finished one may come again
        let id = b.list(&[]).iter().find(|e| e.text == S1 && e.category == "pc.patterns").unwrap().id.clone();
        b.skip(&id).unwrap();
        assert_eq!(b.add("pc.patterns", S1, false, from()).unwrap().len(), 1);
    }

    #[test]
    fn entries_list_oldest_first_and_count_by_state() {
        let (d, b) = boxed();
        let es = b.add("pc.patterns", &format!("{S1} {S2} {S3}"), false, from()).unwrap();
        let w = b.next().unwrap().unwrap();
        assert_eq!(w.id, es[0].id);
        b.failed(&es[0].id, "壞了\n第二行").unwrap();
        let w = b.next().unwrap().unwrap();
        b.ready(&w.id, made("x"), "m").unwrap();
        assert_eq!(b.counts(), Counts { waiting: 1, working: 0, ready: 1, failed: 1 });
        assert_eq!(b.list(&[]).iter().map(|e| e.id.clone()).collect::<Vec<_>>(), es.iter().map(|e| e.id.clone()).collect::<Vec<_>>());
        assert_eq!(b.list(&[state::READY, state::FAILED]).len(), 2);
        assert_eq!(b.list(&[state::WAITING])[0].id, es[2].id);
        // an unreadable file and a stray file are skipped
        fs::write(d.path().join("inbox/ibroken.json"), "{ nope").unwrap();
        fs::write(d.path().join("inbox/notes.txt"), "x").unwrap();
        assert_eq!(b.list(&[]).len(), 3);
    }

    #[test]
    fn the_worker_takes_each_waiting_entry_once() {
        let (_d, b) = boxed();
        assert!(b.next().unwrap().is_none());
        let es = b.add("pc.patterns", &format!("{S1} {S2}"), false, from()).unwrap();
        let a = b.next().unwrap().unwrap();
        let c = b.next().unwrap().unwrap();
        assert_eq!((a.id.as_str(), c.id.as_str()), (es[0].id.as_str(), es[1].id.as_str()));
        assert_eq!(a.state, state::WORKING);
        assert_eq!(b.get(&a.id).unwrap().state, state::WORKING);
        assert!(b.next().unwrap().is_none());
    }

    #[test]
    fn a_result_makes_it_ready() {
        let (_d, b) = boxed();
        let e = one(&b, S1, true);
        b.next().unwrap();
        let r = b.ready(&e.id, made(S1), "claude:opus").unwrap();
        assert_eq!((r.state.as_str(), r.engine.as_str()), (state::READY, "claude:opus"));
        assert_eq!(r.similar, vec!["pc.patterns:a".to_string()]);
        let item = r.result.clone().unwrap();
        assert!(item.id.is_empty());
        assert_eq!(item.field("example"), S1, "own words are kept");
        assert_eq!(b.get(&e.id).unwrap(), r);
        // not twice
        assert!(b.ready(&e.id, made(S1), "x").is_err());
        assert!(matches!(b.ready("inope", made(S1), "x"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn others_sentences_do_not_keep_their_wording() {
        let (_d, b) = boxed();
        let e = one(&b, S1, false);
        b.next().unwrap();
        let r = b.ready(&e.id, made(&format!("  {}  ", S1.to_uppercase())), "m").unwrap();
        let item = r.result.unwrap();
        assert_eq!(item.field("example"), "");
        assert_eq!(item.field("pattern"), "We train {model} for {n} epochs.");
        // a different example of the pattern is not the author's wording and stays
        let e2 = one(&b, S2, false);
        b.next().unwrap();
        let r2 = b.ready(&e2.id, made("They train it for three epochs."), "m").unwrap();
        assert_eq!(r2.result.unwrap().field("example"), "They train it for three epochs.");
    }

    #[test]
    fn a_failure_keeps_its_reason() {
        let (_d, b) = boxed();
        let e = one(&b, S1, false);
        b.next().unwrap();
        let f = b.failed(&e.id, "模型沒有回應").unwrap();
        assert_eq!((f.state.as_str(), f.error.as_str()), (state::FAILED, "模型沒有回應"));
        assert_eq!(b.get(&e.id).unwrap().error, "模型沒有回應");
        assert!(b.failed(&e.id, "again").is_err());
    }

    #[test]
    fn entries_left_working_are_recovered() {
        let (_d, b) = boxed();
        b.add("pc.patterns", &format!("{S1} {S2} {S3}"), false, from()).unwrap();
        let a = b.next().unwrap().unwrap();
        b.next().unwrap();
        assert_eq!(b.recover().unwrap(), 2);
        assert_eq!(b.counts(), Counts { waiting: 3, working: 0, ready: 0, failed: 0 });
        assert_eq!(b.next().unwrap().unwrap().id, a.id, "order is kept");
        assert_eq!(b.recover().unwrap(), 1);
        assert_eq!(b.recover().unwrap(), 0);
    }

    fn ready_entry(b: &Inbox, text: &str) -> Entry {
        let e = one(b, text, true);
        b.next().unwrap();
        b.ready(&e.id, made(text), "m").unwrap()
    }

    #[test]
    fn accepting_writes_the_item_and_marks_the_entry() {
        let (_d, b) = boxed();
        let e = ready_entry(&b, S1);
        let mut sink = MemSink::default();
        let (done, item) = b.accept(&mut sink, &e.id, None).unwrap();
        assert_eq!(done.state, state::ADDED);
        assert_eq!(done.added, item.id);
        assert!(!item.id.is_empty() && item.id.starts_with("pc.patterns"));
        assert_eq!((item.source.kind.as_str(), item.category.as_str()), ("author", "pc.patterns"));
        assert_eq!(sink.get(&item.id).unwrap().unwrap().field("pattern"), "We train {model} for {n} epochs.");
        assert_eq!(b.get(&e.id).unwrap().state, state::ADDED);
        // not twice
        assert!(b.accept(&mut sink, &e.id, None).is_err());
        assert_eq!(sink.items.len(), 1);
        // not from waiting
        let w = one(&b, S2, false);
        assert!(b.accept(&mut sink, &w.id, None).is_err());
    }

    #[test]
    fn an_edited_result_is_what_gets_written() {
        let (_d, b) = boxed();
        let e = ready_entry(&b, S1);
        let mut edited = e.result.clone().unwrap();
        edited.fields.insert("pattern".into(), json!("We train {model} for {n} steps."));
        edited.id = "pc.patterns:other".into();
        edited.source = Source::lore("x");
        edited.category = "wrong".into();
        let mut sink = MemSink::default();
        let (_, item) = b.accept(&mut sink, &e.id, Some(edited)).unwrap();
        assert_eq!(item.field("pattern"), "We train {model} for {n} steps.");
        assert_eq!((item.source.kind.as_str(), item.category.as_str()), ("author", "pc.patterns"));
        assert_ne!(item.id, "pc.patterns:other");
    }

    /// A sink that refuses every write.
    struct Refusing;
    impl Sink for Refusing {
        fn get(&self, _: &str) -> Result<Option<Item>> {
            Ok(None)
        }
        fn list(&self, _: &str) -> Result<Vec<Item>> {
            Ok(vec![])
        }
        fn put(&mut self, _: Item) -> Result<Item> {
            Err(KnowError::ReadOnly("pc.patterns".into()))
        }
        fn remove(&mut self, id: &str) -> Result<Item> {
            Err(KnowError::NotFound(id.into()))
        }
        fn restore(&mut self, id: &str) -> Result<Item> {
            Err(KnowError::NotFound(id.into()))
        }
    }

    #[test]
    fn a_refused_write_leaves_it_ready() {
        let (_d, b) = boxed();
        let e = ready_entry(&b, S1);
        assert!(matches!(b.accept(&mut Refusing, &e.id, None), Err(KnowError::ReadOnly(_))));
        let again = b.get(&e.id).unwrap();
        assert_eq!((again.state.as_str(), again.added.as_str()), (state::READY, ""));
        assert!(b.accept(&mut MemSink::default(), &e.id, None).is_ok());
    }

    #[test]
    fn skip_and_retry_move_states_only_where_allowed() {
        let (_d, b) = boxed();
        let w = one(&b, S1, false);
        assert!(b.retry(&w.id).is_err(), "only failed ones retry");
        let f = one(&b, S2, false);
        b.next().unwrap();
        b.next().unwrap();
        b.failed(&f.id, "壞了").unwrap();
        let r = b.retry(&f.id).unwrap();
        assert_eq!((r.state.as_str(), r.error.as_str()), (state::WAITING, ""));
        b.next().unwrap();
        assert!(b.skip(&f.id).is_err(), "working cannot be skipped");
        b.failed(&f.id, "壞了").unwrap();
        assert_eq!(b.skip(&f.id).unwrap().state, state::SKIPPED);
        assert!(b.skip(&f.id).is_err() && b.retry(&f.id).is_err());
        // ready and waiting can be skipped
        let r = ready_entry(&b, S3);
        assert_eq!(b.skip(&r.id).unwrap().state, state::SKIPPED);
        let w2 = one(&b, "Another sentence waits here.", false);
        assert_eq!(b.skip(&w2.id).unwrap().state, state::SKIPPED);
        // an added one cannot be skipped
        let a = ready_entry(&b, "A fourth sentence is ready now.");
        b.accept(&mut MemSink::default(), &a.id, None).unwrap();
        assert!(b.skip(&a.id).is_err());
        let _ = w;
    }

    #[test]
    fn old_finished_entries_are_cleared_others_never() {
        let (d, b) = boxed();
        let mk = |t: &str, st: &str, upd: &str| {
            let mut e = one(&b, t, false);
            e.state = st.into();
            e.updated_at = upd.into();
            fs::write(d.path().join(format!("inbox/{}.json", e.id)), serde_json::to_vec(&e).unwrap()).unwrap();
            e.id
        };
        let old = "2026-01-01T10:00:00+00:00";
        let recent = "2026-11-20T10:00:00+00:00";
        let a = mk(S1, state::ADDED, old);
        let s = mk(S2, state::SKIPPED, old);
        let young = mk(S3, state::ADDED, recent);
        let ready = mk("An old ready one stays put.", state::READY, old);
        let failed = mk("An old failed one stays put.", state::FAILED, old);
        let waiting = mk("An old waiting one stays put.", state::WAITING, old);
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        assert_eq!(b.clear(30, today), 2);
        assert!(b.get(&a).is_err() && b.get(&s).is_err());
        for id in [&young, &ready, &failed, &waiting] {
            assert!(b.get(id).is_ok());
        }
        assert_eq!(b.clear(30, today), 0);
    }
}
