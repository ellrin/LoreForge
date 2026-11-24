//! Intake (K6): the one door for outside applications (docs/brain-hub.md §2 rule 8, §9).
//!
//! An application outside Lore never creates a kind of knowledge. It either **aligns** to an
//! existing Lore category (its items join that category and are used with it) or stays
//! **independent** (a named set in one existing format that no Lore step reads). There is one
//! file format ([`Batch`], also the API body) and no per-application code. Adding the same source
//! to the same target again replaces only what that source gave before; items that do not fit
//! the format are listed in the [`Report`], never coerced and never silently dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::*;

/// The version of the batch file this module reads and writes.
pub const VERSION: u32 = 1;

// ── step 1: the file format ─────────────────────────────────────────────────

/// Where a batch goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    /// Join an existing Lore (or hand-made) category, in that category's format.
    Align { category: String },
    /// A set of its own, named by the author, in one of the existing formats.
    Independent { name: String, format: String },
}

impl Default for Target {
    fn default() -> Target {
        Target::Align { category: String::new() }
    }
}

/// One piece of knowledge as an application writes it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// The application's own id (optional). With it, adding again replaces the same item.
    pub id: String,
    pub title: String,
    pub fields: Map<String, Value>,
    pub tags: BTreeMap<String, Vec<String>>,
    pub triggers: Vec<String>,
}

/// The whole file: who it is from, where it goes, what it holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Batch {
    pub lore_know: u32,
    pub source: String,
    pub target: Target,
    pub items: Vec<Entry>,
}

/// Read a batch file (JSON). Errors say in Traditional Chinese what is wrong.
pub fn parse(text: &str) -> Result<Batch> {
    let v: Value = serde_json::from_str(text).map_err(|e| KnowError::Invalid(format!("不是 JSON 檔：{e}")))?;
    let obj = v.as_object().ok_or_else(|| KnowError::Invalid("檔案最外層要是一個物件".into()))?;
    match obj.get("lore_know").and_then(Value::as_u64) {
        Some(n) if n == VERSION as u64 => {}
        Some(n) => return Err(KnowError::Invalid(format!("不認得的版本 lore_know = {n}，這裡讀的是 {VERSION}"))),
        None => return Err(KnowError::Invalid(format!("缺 lore_know（版本，要寫 {VERSION}）"))),
    }
    // `source` must be written. Empty is the author's own file (an export of a Lore or hand-made
    // category) and is only valid for aligning; an application always names itself.
    let Some(source) = obj.get("source").and_then(Value::as_str) else {
        return Err(KnowError::Invalid("缺 source（應用程式的名字；作者自己的檔案寫成空字串）".into()));
    };
    let Some(t) = obj.get("target").filter(|t| t.is_object()) else {
        return Err(KnowError::Invalid("缺 target（要對齊哪一類，或獨立一組）".into()));
    };
    let target: Target = serde_json::from_value(t.clone()).map_err(|_| {
        KnowError::Invalid("target 的寫法不對：要 {\"kind\":\"align\",\"category\":…} 或 {\"kind\":\"independent\",\"name\":…,\"format\":…}".into())
    })?;
    match &target {
        Target::Align { category } if category.trim().is_empty() => return Err(KnowError::Invalid("target 沒有寫 category".into())),
        Target::Independent { name, format } if name.trim().is_empty() || format.trim().is_empty() => {
            return Err(KnowError::Invalid("target 的獨立一組要寫 name 和 format".into()));
        }
        _ => {}
    }
    if source.trim().is_empty() && matches!(target, Target::Independent { .. }) {
        return Err(KnowError::Invalid("缺 source（獨立的一組一定要寫應用程式的名字）".into()));
    }
    serde_json::from_value(v).map_err(|e| KnowError::Invalid(format!("內容的寫法不對：{e}")))
}

// ── step 2: the seam to the registry ────────────────────────────────────────

/// What intake needs from the registry of categories (the registry implements it).
pub trait Categories {
    fn get(&self, id: &str) -> Option<Category>;
    /// The independent set `name` of `source` in `format`: made now, or the existing one.
    fn add_external(&mut self, source: &str, name: &str, format: &str) -> Result<Category>;
}

pub use crate::formats::Formats;

// ── step 3: importing ───────────────────────────────────────────────────────

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub category: String,
    pub added: usize,
    /// Same application id as last time.
    pub replaced: usize,
    /// Given last time, absent now (moved to the trash).
    pub removed: usize,
    /// Entries not written, and why.
    pub problems: Vec<Problem>,
}

fn problem(at: &str, field: &str, message: &str) -> Problem {
    Problem { at: at.into(), field: field.into(), message: message.into() }
}

/// FNV-1a, 64 bit.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The same value with every object's keys in sorted order (stable whatever serde_json's features).
fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&m[k]))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// 12 hex characters from the title and the fields: the same content gives the same id on every run.
fn content_hash(e: &Entry) -> String {
    let fields = serde_json::to_string(&sorted(&Value::Object(e.fields.clone()))).unwrap_or_default();
    let h = fnv1a(format!("{}\u{0}{}", e.title.trim(), fields).as_bytes());
    format!("{h:016x}")[..12].to_string()
}

fn prefix(source: &str) -> String {
    format!("{}.", safe_local(source))
}

fn local_id(source: &str, e: &Entry) -> String {
    let key = if e.id.trim().is_empty() { content_hash(e) } else { safe_local(&e.id) };
    if source.is_empty() { key } else { format!("{}{}", prefix(source), key) }
}

/// Add or replace what `batch.source` gives to its target; remove what it gave before and no longer gives.
pub fn import(sink: &mut dyn Sink, cats: &mut dyn Categories, formats: &Formats, batch: &Batch) -> Result<Report> {
    let source = batch.source.trim();
    // An empty source is the author's own file: his items, added and never replacing anything.
    let author = source.is_empty();
    if author && !matches!(batch.target, Target::Align { .. }) {
        return Err(KnowError::Invalid("缺 source（應用程式的名字）".into()));
    }
    let cat = match &batch.target {
        Target::Align { category } => {
            let c = cats.get(category).ok_or_else(|| KnowError::NotFound(format!("沒有「{category}」這一類，沒辦法對齊")))?;
            if c.independent() {
                return Err(KnowError::Invalid(format!("「{}」是別的應用程式的獨立一組，不能對齊", c.label)));
            }
            if !c.writable {
                return Err(KnowError::ReadOnly(format!("「{}」不接受從這裡加入", c.label)));
            }
            c
        }
        Target::Independent { name, format } => {
            if !formats.contains_key(format) {
                return Err(KnowError::Invalid(format!("沒有「{format}」這個格式；應用程式不能帶來新格式")));
            }
            if name.trim().is_empty() {
                return Err(KnowError::Invalid("獨立一組要有名字".into()));
            }
            let c = cats.add_external(source, name.trim(), format)?;
            if !c.independent() {
                return Err(KnowError::Invalid(format!("「{name}」不是獨立的一組")));
            }
            c
        }
    };
    let fmt = formats.get(&cat.format).ok_or_else(|| KnowError::Invalid(format!("這一類的格式「{}」不認得", cat.format)))?;
    let src = if author { Source::author() } else { Source::external(source) };

    let all = sink.list(&cat.id)?;
    // the author's file never replaces: everything that is there already is "somebody else's"
    let mine: BTreeMap<String, Item> =
        if author { BTreeMap::new() } else { all.iter().filter(|i| i.source == src).map(|i| (i.id.clone(), i.clone())).collect() };
    let others: BTreeSet<&str> = all.iter().filter(|i| author || i.source != src).map(|i| i.id.as_str()).collect();

    let mut rep = Report { category: cat.id.clone(), ..Default::default() };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut ready: Vec<Item> = vec![];
    // Ids of this source that stay: written now, or whose new entry did not fit (the old one is kept).
    let mut keep: BTreeSet<String> = BTreeSet::new();
    // Where an app's data names an item by its own words (a wording row by its `avoid`), the id
    // given here does not come back. An entry that has no item under its id but says exactly what
    // an item of this source already says is that item, so adding the same file again changes nothing.
    let wanted: BTreeSet<String> = batch.items.iter().map(|e| item_id(&cat.id, &local_id(source, e))).collect();
    let mut paired: BTreeSet<String> = BTreeSet::new();
    for (n, e) in batch.items.iter().enumerate() {
        let at = format!("#{} {}", n + 1, e.title.trim());
        let mut id = item_id(&cat.id, &local_id(source, e));
        if !mine.contains_key(&id)
            && let Some(m) = mine.values().find(|m| {
                !wanted.contains(&m.id) && !paired.contains(&m.id) && m.title.trim() == e.title.trim() && e.fields.iter().all(|(k, v)| m.fields.get(k) == Some(v))
            })
        {
            paired.insert(m.id.clone());
            id = m.id.clone();
        }
        let mut item = Item::new(&cat.id, &cat.format, &e.title, src.clone());
        item.id = id.clone();
        item.fields = e.fields.clone();
        item.tags = e.tags.clone();
        item.triggers = e.triggers.clone();
        item.tidy();
        let mut ps: Vec<Problem> = fmt
            .validate(&item)
            .into_iter()
            .map(|mut p| {
                p.at = at.clone();
                p
            })
            .collect();
        if ps.is_empty() && !seen.insert(id.clone()) {
            ps.push(problem(&at, "id", "同一個檔案裡有重複的項目（id 或內容相同）"));
        }
        if ps.is_empty() && others.contains(id.as_str()) {
            let why = if author { "這一類已經有同一個 id 的一條，沒有蓋掉它" } else { "和別的來源的項目撞到同一個 id" };
            ps.push(problem(&at, "id", why));
        }
        if ps.is_empty() {
            keep.insert(id);
            ready.push(item);
        } else {
            keep.insert(id);
            rep.problems.extend(ps);
        }
    }

    // What this source gave before and no longer gives goes first: a row changed in the words that
    // name it (a wording) is then written as a new one without meeting its old self.
    for id in mine.keys().filter(|id| !keep.contains(*id)) {
        sink.remove(id)?;
        rep.removed += 1;
    }
    for item in ready {
        let at = item.title.clone();
        // a write the category refuses (a read-only place, a row the app's data cannot hold) is
        // listed with the entry; it does not stop the others
        let written = if let Some(have) = mine.get(&item.id) {
            let mut next = have.clone();
            next.title = item.title;
            next.fields = item.fields;
            next.tags = item.tags;
            next.triggers = item.triggers;
            next.format = item.format;
            sink.put(next).map(|_| rep.replaced += 1)
        } else {
            sink.put(item).map(|_| rep.added += 1)
        };
        if let Err(e) = written {
            rep.problems.push(problem(&at, "", &format!("沒有寫入：{e}")));
        }
    }
    Ok(rep)
}

// ── step 4: writing out ─────────────────────────────────────────────────────

/// The items of `category` (all, or those of one source) as a batch of the same format.
pub fn export(sink: &dyn Sink, category: &Category, source: Option<&Source>) -> Result<Batch> {
    let mut items = sink.list(&category.id)?;
    items.retain(|i| source.is_none_or(|s| &i.source == s));
    items.sort_by(|a, b| a.id.cmp(&b.id));
    let entries = items
        .iter()
        .map(|i| {
            let local = split_id(&i.id).map(|(_, l)| l).unwrap_or(&i.id);
            let from_it = i.source.is_external() && source.is_some_and(|s| s == &i.source);
            let id = if from_it { local.strip_prefix(prefix(&i.source.name).as_str()).unwrap_or(local) } else { local };
            Entry { id: id.to_string(), title: i.title.clone(), fields: i.fields.clone(), tags: i.tags.clone(), triggers: i.triggers.clone() }
        })
        .collect();
    let target = if category.independent() {
        Target::Independent { name: category.label.clone(), format: category.format.clone() }
    } else {
        Target::Align { category: category.id.clone() }
    };
    // The file says who gave it. The author's and Lore's own items have no outside source: "" (it
    // imports back as his own). An independent set is always an outside application's.
    let name = match source {
        Some(s) if s.is_external() => s.name.clone(),
        Some(_) => String::new(),
        None if category.independent() => {
            let mut names: Vec<&str> = items.iter().filter(|i| i.source.is_external()).map(|i| i.source.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            match names.as_slice() {
                [one] => (*one).to_string(),
                // several (or none left): the application the set was made for, from its id `x.<source>.<name>`
                _ if !category.group.trim().is_empty() => category.group.trim().to_string(),
                _ => category.id.split('.').nth(1).unwrap_or("export").to_string(),
            }
        }
        None => String::new(),
    };
    Ok(Batch { lore_know: VERSION, source: name, target, items: entries })
}

/// The batch as pretty JSON, keys in a stable order.
pub fn to_text(batch: &Batch) -> String {
    let v = serde_json::to_value(batch).unwrap_or(Value::Null);
    serde_json::to_string_pretty(&sorted(&v)).unwrap_or_default()
}

// ── step 5: who brought what ────────────────────────────────────────────────

/// One outside application and where its items are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceInfo {
    pub name: String,
    /// (category id, number of its items there, aligned to a Lore category (true) or independent (false)).
    pub sets: Vec<(String, usize, bool)>,
}

/// Every external source in `cats` and where its items are, for the dashboard.
pub fn sources(sink: &dyn Sink, cats: &[Category]) -> Result<Vec<SourceInfo>> {
    let mut by: BTreeMap<String, BTreeMap<String, (usize, bool)>> = BTreeMap::new();
    for c in cats {
        for i in sink.list(&c.id)?.iter().filter(|i| i.source.is_external()) {
            by.entry(i.source.name.clone()).or_default().entry(c.id.clone()).or_insert((0, !c.independent())).0 += 1;
        }
    }
    Ok(by.into_iter().map(|(name, m)| SourceInfo { name, sets: m.into_iter().map(|(c, (n, a))| (c, n, a)).collect() }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fmt(&'static str);
    impl Format for Fmt {
        fn def(&self) -> FormatDef {
            FormatDef {
                id: self.0.into(),
                label: self.0.into(),
                fields: vec![FieldDef::new("body", "內容", "text", true), FieldDef::new("note", "備註", "text", false)],
                shape: "row".into(),
                form: true,
            }
        }
        fn line(&self, i: &Item) -> String {
            i.field("body").into()
        }
        fn prompt(&self, i: &Item) -> String {
            i.field("body").into()
        }
    }

    fn formats() -> Formats {
        let mut f = Formats::new();
        f.insert("note".into(), Box::new(Fmt("note")));
        f.insert("card".into(), Box::new(Fmt("card")));
        f
    }

    #[derive(Default)]
    struct Cats(BTreeMap<String, Category>);
    impl Cats {
        fn with(cs: Vec<Category>) -> Cats {
            Cats(cs.into_iter().map(|c| (c.id.clone(), c)).collect())
        }
    }
    impl Categories for Cats {
        fn get(&self, id: &str) -> Option<Category> {
            self.0.get(id).cloned()
        }
        fn add_external(&mut self, source: &str, name: &str, format: &str) -> Result<Category> {
            let id = format!("x.{}.{}", safe_local(source), safe_local(name));
            if let Some(c) = self.0.get(&id) {
                if c.format == format {
                    return Ok(c.clone());
                }
                return Err(KnowError::Invalid("這一組已經是別的格式".into()));
            }
            let c = Category {
                id: id.clone(),
                label: name.into(),
                app: "x".into(),
                format: format.into(),
                origin: "external".into(),
                writable: true,
                ..Default::default()
            };
            self.0.insert(id, c.clone());
            Ok(c)
        }
    }

    fn lore_cat(id: &str, origin: &str, writable: bool) -> Category {
        Category { id: id.into(), label: id.into(), app: "pc".into(), format: "card".into(), origin: origin.into(), tasks: vec!["t".into()], writable, ..Default::default() }
    }
    fn cats() -> Cats {
        Cats::with(vec![lore_cat("pc.words", "lore", true), lore_cat("pc.locked", "lore", false), lore_cat("my.mine", "manual", true)])
    }

    fn entry(id: &str, title: &str, body: &str) -> Entry {
        let mut fields = Map::new();
        if !body.is_empty() {
            fields.insert("body".into(), body.into());
        }
        Entry { id: id.into(), title: title.into(), fields, ..Default::default() }
    }
    fn align(source: &str, cat: &str, items: Vec<Entry>) -> Batch {
        Batch { lore_know: 1, source: source.into(), target: Target::Align { category: cat.into() }, items }
    }
    fn indep(source: &str, name: &str, items: Vec<Entry>) -> Batch {
        Batch { lore_know: 1, source: source.into(), target: Target::Independent { name: name.into(), format: "note".into() }, items }
    }

    // step 1
    #[test]
    fn a_batch_file_parses() {
        let text = r#"{"lore_know":1,"source":"wordbox","target":{"kind":"align","category":"pc.words"},
            "items":[{"id":"7","title":"alpha","fields":{"body":"first"},"tags":{"use":["paper"]},"triggers":["alpha"]},{"title":"beta"}]}"#;
        let b = parse(text).unwrap();
        assert_eq!(b.source, "wordbox");
        assert_eq!(b.target, Target::Align { category: "pc.words".into() });
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.items[0].tags["use"], vec!["paper"]);
        assert_eq!(b.items[1].id, "");
        let i = parse(r#"{"lore_know":1,"source":"notesapp","target":{"kind":"independent","name":"Ideas","format":"note"}}"#).unwrap();
        assert_eq!(i.target, Target::Independent { name: "Ideas".into(), format: "note".into() });
        assert!(i.items.is_empty());
    }

    #[test]
    fn bad_files_say_what_is_wrong() {
        let msg = |t: &str| parse(t).unwrap_err().to_string();
        assert!(msg("not json").contains("不是 JSON"));
        assert!(msg("[1]").contains("物件"));
        assert!(msg(r#"{"source":"a","target":{"kind":"align","category":"c"}}"#).contains("lore_know"));
        assert!(msg(r#"{"lore_know":2,"source":"a","target":{"kind":"align","category":"c"}}"#).contains("版本"));
        assert!(msg(r#"{"lore_know":1,"target":{"kind":"align","category":"c"}}"#).contains("source"));
        assert!(msg(r#"{"lore_know":1,"source":" ","target":{"kind":"independent","name":"x","format":"note"}}"#).contains("source"));
        assert!(msg(r#"{"lore_know":1,"source":"a"}"#).contains("target"));
        assert!(msg(r#"{"lore_know":1,"source":"a","target":{"kind":"merge"}}"#).contains("target"));
        assert!(msg(r#"{"lore_know":1,"source":"a","target":{"kind":"independent","name":"x","format":""}}"#).contains("format"));
    }

    // step 3
    #[test]
    fn aligned_items_join_the_category_with_their_source() {
        let mut s = MemSink::default();
        let mut c = cats();
        let r = import(&mut s, &mut c, &formats(), &align("wordbox", "pc.words", vec![entry("7", "alpha", "first"), entry("8", "beta", "second")])).unwrap();
        assert_eq!((r.category.as_str(), r.added, r.replaced, r.removed), ("pc.words", 2, 0, 0));
        let a = s.get("pc.words:wordbox.7").unwrap().unwrap();
        assert_eq!((a.source.clone(), a.format.as_str(), a.field("body")), (Source::external("wordbox"), "card", "first"));
        assert!(c.0.len() == 3, "aligning makes no category");
    }

    #[test]
    fn an_independent_set_gets_its_own_category_and_no_tasks() {
        let mut s = MemSink::default();
        let mut c = cats();
        let r = import(&mut s, &mut c, &formats(), &indep("notesapp", "Ideas", vec![entry("1", "x", "y")])).unwrap();
        assert_eq!(r.category, "x.notesapp.Ideas");
        let cat = c.get(&r.category).unwrap();
        assert!(cat.independent() && cat.tasks.is_empty() && !cat.connected());
        assert_eq!(s.list(&r.category).unwrap().len(), 1);
    }

    #[test]
    fn adding_again_replaces_only_this_sources_items() {
        let mut s = MemSink::default();
        let mut c = cats();
        let f = formats();
        let mut mine = Item::new("pc.words", "card", "by hand", Source::author());
        mine.id = "pc.words:h1".into();
        mine.set("body", "x");
        let mut lore = Item::new("pc.words", "card", "by lore", Source::lore("pc"));
        lore.id = "pc.words:l1".into();
        let mut other = Item::new("pc.words", "card", "other app", Source::external("otherapp"));
        other.id = "pc.words:otherapp.1".into();
        let (mine, lore, other) = (s.put(mine).unwrap(), s.put(lore).unwrap(), s.put(other).unwrap());

        import(&mut s, &mut c, &f, &align("wordbox", "pc.words", vec![entry("1", "a", "A"), entry("2", "b", "B"), entry("3", "c", "C")])).unwrap();
        let before = s.get("pc.words:wordbox.1").unwrap().unwrap();
        let r = import(&mut s, &mut c, &f, &align("wordbox", "pc.words", vec![entry("1", "a2", "A2"), entry("3", "c", "C"), entry("4", "d", "D")])).unwrap();
        assert_eq!((r.added, r.replaced, r.removed), (1, 2, 1));
        let after = s.get("pc.words:wordbox.1").unwrap().unwrap();
        assert_eq!((after.title.as_str(), after.revision), ("a2", before.revision + 1));
        assert!(s.get("pc.words:wordbox.2").unwrap().is_none());
        assert_eq!(s.trash.len(), 1);
        for i in [&mine, &lore, &other] {
            assert_eq!(&s.get(&i.id).unwrap().unwrap(), i, "{} untouched", i.id);
        }
        // an empty list takes the source out
        let r = import(&mut s, &mut c, &f, &align("wordbox", "pc.words", vec![])).unwrap();
        assert_eq!(r.removed, 3);
        assert_eq!(s.list("pc.words").unwrap().len(), 3);
    }

    #[test]
    fn entries_that_do_not_fit_are_listed_and_not_written() {
        let mut s = MemSink::default();
        let mut c = cats();
        let f = formats();
        let b = align("wordbox", "pc.words", vec![entry("1", "ok", "A"), entry("2", "no body", ""), entry("3", "", "B"), entry("1", "dup", "C")]);
        let r = import(&mut s, &mut c, &f, &b).unwrap();
        assert_eq!((r.added, r.problems.len()), (1, 3));
        assert!(r.problems.iter().any(|p| p.at == "#2 no body" && p.field == "body"));
        assert!(r.problems.iter().any(|p| p.at.starts_with("#3") && p.field == "title"));
        assert!(r.problems.iter().any(|p| p.at == "#4 dup" && p.field == "id"));
        assert_eq!(s.list("pc.words").unwrap().len(), 1);
        // a bad entry does not take away what the same id gave last time
        let r = import(&mut s, &mut c, &f, &align("wordbox", "pc.words", vec![entry("1", "ok", "")])).unwrap();
        assert_eq!((r.problems.len(), r.removed, r.replaced), (1, 0, 0));
        assert_eq!(s.get("pc.words:wordbox.1").unwrap().unwrap().field("body"), "A");
    }

    #[test]
    fn an_application_cannot_align_to_a_missing_or_external_category() {
        let mut s = MemSink::default();
        let mut c = cats();
        let f = formats();
        let r = import(&mut s, &mut c, &f, &indep("notesapp", "Ideas", vec![])).unwrap();
        for target in ["pc.nothing", r.category.as_str()] {
            assert!(import(&mut s, &mut c, &f, &align("wordbox", target, vec![entry("1", "a", "A")])).is_err(), "{target}");
        }
        assert!(matches!(import(&mut s, &mut c, &f, &align("wordbox", "pc.locked", vec![entry("1", "a", "A")])), Err(KnowError::ReadOnly(_))));
        assert!(s.items.is_empty());
        // a hand-made category is fine
        assert!(import(&mut s, &mut c, &f, &align("wordbox", "my.mine", vec![entry("1", "a", "A")])).is_ok());
    }

    #[test]
    fn an_unknown_format_is_refused() {
        let mut s = MemSink::default();
        let mut c = cats();
        let mut b = indep("notesapp", "Ideas", vec![entry("1", "a", "A")]);
        b.target = Target::Independent { name: "Ideas".into(), format: "my-own-shape".into() };
        let e = import(&mut s, &mut c, &formats(), &b).unwrap_err();
        assert!(matches!(e, KnowError::Invalid(_)));
        assert!(c.0.len() == 3 && s.items.is_empty());
    }

    #[test]
    fn ids_are_stable_without_an_application_id() {
        let mut s = MemSink::default();
        let mut c = cats();
        let f = formats();
        let b = align("wordbox", "pc.words", vec![entry("", "alpha", "A"), entry("", "beta", "B")]);
        import(&mut s, &mut c, &f, &b).unwrap();
        let ids: Vec<String> = s.items.keys().cloned().collect();
        let local = ids[0].strip_prefix("pc.words:wordbox.").unwrap();
        assert!(local.len() == 12 && local.chars().all(|ch| ch.is_ascii_hexdigit()), "{local}");
        // again: the same ids, replaced and not duplicated
        let r = import(&mut s, &mut c, &f, &b).unwrap();
        assert_eq!((r.added, r.replaced, r.removed), (0, 2, 0));
        assert_eq!(s.items.keys().cloned().collect::<Vec<_>>(), ids);
        // key order of the fields does not matter
        let mut e1 = entry("", "t", "A");
        e1.fields.insert("note".into(), "n".into());
        let mut e2 = entry("", "t", "");
        e2.fields.insert("note".into(), "n".into());
        e2.fields.insert("body".into(), "A".into());
        assert_eq!(content_hash(&e1), content_hash(&e2));
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
    }

    // step 4
    #[test]
    fn what_is_written_out_imports_back_the_same() {
        let f = formats();
        for aligned in [true, false] {
            let mut s = MemSink::default();
            let mut c = cats();
            let mut e1 = entry("7", "alpha", "first");
            e1.tags.insert("use".into(), vec!["paper".into()]);
            e1.triggers = vec!["alpha".into()];
            let e2 = entry("", "beta", "second");
            let b = if aligned { align("wordbox", "pc.words", vec![e1, e2]) } else { indep("notesapp", "Ideas", vec![e1, e2]) };
            let r = import(&mut s, &mut c, &f, &b).unwrap();
            let cat = c.get(&r.category).unwrap();
            let src = Source::external(&b.source);
            let out = export(&s, &cat, Some(&src)).unwrap();
            assert_eq!(out.target, b.target);
            assert_eq!(out.items.iter().find(|e| e.title == "alpha").unwrap().id, "7");
            let text = to_text(&out);
            assert_eq!(text, to_text(&parse(&text).unwrap()));
            let mut s2 = MemSink::default();
            let mut c2 = cats();
            import(&mut s2, &mut c2, &f, &parse(&text).unwrap()).unwrap();
            let strip = |s: &MemSink| {
                let mut v: Vec<_> = s.items.values().map(|i| (i.title.clone(), i.fields.clone(), i.tags.clone(), i.triggers.clone())).collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            };
            assert_eq!(strip(&s), strip(&s2));
            assert_eq!(strip(&s).len(), 2);
            assert_eq!(export(&s, &cat, None).unwrap().items.len(), 2);
        }
    }

    // step 5
    #[test]
    fn sources_are_listed_with_their_sets() {
        let mut s = MemSink::default();
        let mut c = cats();
        let f = formats();
        import(&mut s, &mut c, &f, &align("wordbox", "pc.words", vec![entry("1", "a", "A"), entry("2", "b", "B")])).unwrap();
        import(&mut s, &mut c, &f, &align("wordbox", "my.mine", vec![entry("1", "a", "A")])).unwrap();
        import(&mut s, &mut c, &f, &indep("notesapp", "Ideas", vec![entry("1", "a", "A")])).unwrap();
        let mut by_hand = Item::new("pc.words", "card", "h", Source::author());
        by_hand.set("body", "x");
        s.put(by_hand).unwrap();
        let all: Vec<Category> = c.0.values().cloned().collect();
        let got = sources(&s, &all).unwrap();
        assert_eq!(
            got,
            vec![
                SourceInfo { name: "notesapp".into(), sets: vec![("x.notesapp.Ideas".into(), 1, false)] },
                SourceInfo { name: "wordbox".into(), sets: vec![("my.mine".into(), 1, true), ("pc.words".into(), 2, true)] },
            ]
        );
    }

    #[test]
    fn the_authors_own_export_imports_as_his_not_as_an_application() {
        let f = formats();
        let mut s = MemSink::default();
        let mut c = cats();
        let mut mine = Item::new("pc.words", "card", "by hand", Source::author());
        mine.id = "pc.words:h1".into();
        mine.set("body", "x");
        let mut lore = Item::new("pc.words", "card", "by lore", Source::lore("pc"));
        lore.id = "pc.words:l1".into();
        lore.set("body", "y");
        s.put(mine).unwrap();
        s.put(lore).unwrap();
        // no source asked for: the file does not name an application
        let out = export(&s, &c.get("pc.words").unwrap(), None).unwrap();
        assert_eq!(out.source, "");
        let text = to_text(&out);
        let back = parse(&text).unwrap();
        assert_eq!(back.source, "");
        // into another category: his own items, no source prefix, not external
        let into = Batch { target: Target::Align { category: "my.mine".into() }, ..back.clone() };
        let r = import(&mut s, &mut c, &f, &into).unwrap();
        assert_eq!((r.added, r.replaced, r.removed, r.problems.len()), (2, 0, 0, 0));
        let got = s.list("my.mine").unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|i| i.source == Source::author() && !i.id.contains("export")), "{got:?}");
        assert!(crate::intake::sources(&s, &c.0.values().cloned().collect::<Vec<_>>()).unwrap().is_empty(), "no outside application appears");
        // adding it again replaces nothing and removes nothing: each item is listed as already there
        let again = import(&mut s, &mut c, &f, &into).unwrap();
        assert_eq!((again.added, again.replaced, again.removed, again.problems.len()), (0, 0, 0, 2));
        assert_eq!(s.list("my.mine").unwrap().len(), 2);
        // an empty batch of his own removes nothing, and an outside source's items beside his stay
        import(&mut s, &mut c, &f, &align("wordbox", "my.mine", vec![entry("1", "a", "A")])).unwrap();
        let none = Batch { items: vec![], ..into.clone() };
        assert_eq!(import(&mut s, &mut c, &f, &none).unwrap().removed, 0);
        assert_eq!(s.list("my.mine").unwrap().len(), 3);
        // an independent set always names its application; with several, the id says which
        let r = import(&mut s, &mut c, &f, &indep("notesapp", "Ideas", vec![entry("1", "x", "y")])).unwrap();
        let ind = c.get(&r.category).unwrap();
        assert_eq!(export(&s, &ind, None).unwrap().source, "notesapp");
        // an independent set cannot be the author's
        let no_source = Batch { source: "".into(), ..indep("a", "Ideas", vec![]) };
        assert!(import(&mut s, &mut c, &f, &no_source).is_err());
        assert!(parse(&to_text(&no_source)).is_err());
    }
}
