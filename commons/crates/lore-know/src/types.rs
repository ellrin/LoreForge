//! The frozen contract of the hub (docs/brain-hub.md §4). Packages build against these types
//! and do not change them; a needed change is reported, not made.
//!
//! Words: a **format** is a fixed shape; a **category** is a named set of items in one format,
//! owned by a Lore app or made by the author by hand; a **source** is who supplied an item. An
//! outside application is only ever a source.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ── ids ──────────────────────────────────────────────────────────────────────

/// Format ids (`Item::format`, `Category::format`).
pub mod format {
    pub const CARD: &str = "card";
    pub const PATTERN: &str = "pattern";
    pub const EXPRESSION: &str = "expression";
    pub const WORDING: &str = "wording";
    pub const RULE: &str = "rule";
    pub const EXEMPLAR: &str = "exemplar";
    /// Another way to say one of the author's own sentence patterns, or a word that may be swapped
    /// in it. Never the author's own writing: it enters only through [`crate::review`].
    pub const VARIANT: &str = "variant";
    pub const NOTE: &str = "note";
    pub const BUNDLE: &str = "bundle";
    pub const LAYOUT: &str = "layout";
    pub const FIGURE: &str = "figure";
}

/// Tag facets (`Item::tags` keys). More may be added by the author.
pub mod facet {
    /// The field (machine learning, web …): the knowledge cards' domains, borrowed read-only.
    pub const DOMAIN: &str = "domain";
    /// What kind of structure (architecture, data flow, timeline, network, comparison …).
    pub const KIND: &str = "kind";
    /// What it is for (paper, slide, book …).
    pub const USE: &str = "use";
    /// Where the item sits on its family's tree (`段落/小節/使用時機` paths; the first is the main one, order kept).
    pub const PLACE: &str = "place";
}

/// Sizes of a figure item (`Item::level`).
pub mod level {
    pub const COMPONENT: &str = "component";
    pub const MODULE: &str = "module";
    pub const FIGURE: &str = "figure";
}

/// An item id is `<category id>:<local id>`; the local id never contains `:` or `/`.
pub fn item_id(category: &str, local: &str) -> String {
    format!("{category}:{local}")
}

/// `(category id, local id)` of an item id.
pub fn split_id(id: &str) -> Option<(&str, &str)> {
    let (c, l) = id.rsplit_once(':')?;
    (!c.is_empty() && !l.is_empty() && !l.contains('/')).then_some((c, l))
}

/// A local id safe as a file name: ASCII letters, digits, `-`, `_`, `.`; anything else `-`.
/// A text with nothing ASCII to keep, but with letters of its own (Chinese …), gets `u` + 12 hex
/// characters of a stable hash of the text, so two such names never meet; any other empty text is `item`.
pub fn safe_local(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    let t = t.trim_matches(|c| c == '-' || c == '.').to_string();
    if !t.is_empty() {
        t
    } else if s.chars().any(|c| !c.is_ascii() && c.is_alphanumeric()) {
        stable_key(s.trim())
    } else {
        "item".into()
    }
}

/// `u` + 12 hex characters (FNV-1a, 64 bit) of `s`: the same text gives the same key on every run.
pub fn stable_key(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("u{h:016x}")[..13].to_string()
}

/// A new local id: `k` + 26 characters that sort by creation time (48-bit ms + 80 random bits).
pub fn new_local(ms: u64) -> String {
    const C: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut buf = [0u8; 10];
    getrandom::getrandom(&mut buf).expect("system randomness");
    let rand = buf.iter().fold(0u128, |a, b| (a << 8) | *b as u128);
    let mut v = ((ms as u128 & ((1u128 << 48) - 1)) << 80) | rand;
    let mut out = [0u8; 26];
    for i in (0..26).rev() {
        out[i] = C[(v & 31) as usize];
        v >>= 5;
    }
    format!("k{}", String::from_utf8_lossy(&out))
}

/// Now as RFC 3339 (`Item::created_at`, `updated_at`).
pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

// ── source ───────────────────────────────────────────────────────────────────

/// Who supplied an item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    /// `author` (by hand in L-Brain) · `lore` (a Lore app's own data or tool) · `external`.
    pub kind: String,
    /// The Lore app id (`pc`, `sc`, `lr`, `le`, `kb`) or the outside application's name; "" for the author.
    pub name: String,
}

impl Source {
    pub fn author() -> Source {
        Source { kind: "author".into(), name: String::new() }
    }
    pub fn lore(app: &str) -> Source {
        Source { kind: "lore".into(), name: app.into() }
    }
    pub fn external(name: &str) -> Source {
        Source { kind: "external".into(), name: name.trim().into() }
    }
    pub fn is_external(&self) -> bool {
        self.kind == "external"
    }
}

// ── item ─────────────────────────────────────────────────────────────────────

/// One piece of knowledge.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    /// `<category>:<local>` ([`item_id`]).
    pub id: String,
    pub category: String,
    pub format: String,
    /// What a list row shows first.
    pub title: String,
    /// The format's fields ([`FormatDef::fields`]).
    pub fields: Map<String, Value>,
    /// Facet → tags ([`facet`]). Sorted, no duplicates ([`Item::tidy`]).
    pub tags: BTreeMap<String, Vec<String>>,
    /// Words or structures whose presence in the content makes this item apply.
    pub triggers: Vec<String>,
    pub source: Source,
    /// Figures only: [`level`]. "" otherwise.
    pub level: String,
    /// Figures only: `true` = drawable (an entity), `false` = a reference to make. Other formats: `true`.
    pub entity: bool,
    /// Ids of the smaller items this one is made of (figure → modules → components).
    pub contains: Vec<String>,
    /// A picture or drawing to show: a path relative to the hub folder, or "" (figures: see
    /// `formats::figure` for thumbnails drawn on demand).
    pub preview: String,
    pub created_at: String,
    pub updated_at: String,
    /// Bumped by every write; a write with a stale revision is refused.
    pub revision: u64,
    /// How often a step used it, and when last (K7 records it).
    pub used: u64,
    pub used_at: String,
}

impl Item {
    /// A new item of `category` / `format` by `source` (id, times and revision are set by the store).
    pub fn new(category: &str, format: &str, title: &str, source: Source) -> Item {
        Item { category: category.into(), format: format.into(), title: title.trim().into(), source, entity: true, ..Default::default() }
    }

    pub fn field(&self, name: &str) -> &str {
        self.fields.get(name).and_then(Value::as_str).unwrap_or("")
    }

    pub fn set(&mut self, name: &str, value: impl Into<Value>) -> &mut Item {
        self.fields.insert(name.into(), value.into());
        self
    }

    pub fn tag(&mut self, facet: &str, tag: &str) -> &mut Item {
        let t = tag.trim();
        if !t.is_empty() {
            self.tags.entry(facet.into()).or_default().push(t.into());
            self.tidy();
        }
        self
    }

    /// All tags of all facets.
    pub fn all_tags(&self) -> Vec<&str> {
        self.tags.values().flatten().map(String::as_str).collect()
    }

    /// Trimmed, sorted, no duplicates, no empty facet; triggers and `contains` likewise (order kept).
    pub fn tidy(&mut self) {
        for (facet, v) in self.tags.iter_mut() {
            let mut t: Vec<String> = v.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if facet == facet::PLACE {
                // where the item sits on its family's tree: the order is the author's (the first is the main place)
                let mut seen: Vec<String> = vec![];
                for x in t {
                    if !seen.contains(&x) {
                        seen.push(x);
                    }
                }
                t = seen;
            } else {
                t.sort();
                t.dedup();
            }
            *v = t;
        }
        self.tags.retain(|_, v| !v.is_empty());
        let once = |v: &mut Vec<String>| {
            let mut seen: Vec<String> = vec![];
            for s in v.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                if !seen.contains(&s) {
                    seen.push(s);
                }
            }
            *v = seen;
        };
        once(&mut self.triggers);
        once(&mut self.contains);
        self.title = self.title.trim().to_string();
    }
}

/// What the hub keeps beside an item whose words live in an app's own data (a CSV row, a
/// template folder): the things that data has no place for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub tags: BTreeMap<String, Vec<String>>,
    pub triggers: Vec<String>,
    pub contains: Vec<String>,
    pub used: u64,
    pub used_at: String,
}

// ── format ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldDef {
    pub name: String,
    /// Shown in L-Brain (Traditional Chinese).
    pub label: String,
    /// `text` · `markdown` · `latex` · `number` · `list` · `url` · `choice`.
    pub kind: String,
    pub required: bool,
    /// `choice` only.
    pub choices: Vec<String>,
}

impl FieldDef {
    pub fn new(name: &str, label: &str, kind: &str, required: bool) -> FieldDef {
        FieldDef { name: name.into(), label: label.into(), kind: kind.into(), required, choices: vec![] }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FormatDef {
    pub id: String,
    pub label: String,
    pub fields: Vec<FieldDef>,
    /// `row` (one line of text) · `text` (a document) · `bundle` (a folder, opened in its app) · `figure`.
    pub shape: String,
    /// Items can be edited in L-Brain as a form (`row`, `text`); bundles and figures open their tool.
    pub form: bool,
}

/// Something wrong with an item for its format (never coerced, never dropped: listed).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Problem {
    /// The item's title or its place in a batch (`#3`).
    pub at: String,
    pub field: String,
    /// One line, Traditional Chinese.
    pub message: String,
}

/// A fixed shape of knowledge: its fields, how a list shows it, how it is written into a prompt.
pub trait Format: Send + Sync {
    fn def(&self) -> FormatDef;
    /// What is wrong with `item` for this format (empty = fine). The default checks required fields.
    fn validate(&self, item: &Item) -> Vec<Problem> {
        let mut out = vec![];
        if item.title.trim().is_empty() {
            out.push(Problem { at: item.id.clone(), field: "title".into(), message: "沒有名稱".into() });
        }
        for f in self.def().fields.iter().filter(|f| f.required) {
            let empty = match item.fields.get(&f.name) {
                None | Some(Value::Null) => true,
                Some(Value::String(s)) => s.trim().is_empty(),
                Some(Value::Array(a)) => a.is_empty(),
                _ => false,
            };
            if empty {
                out.push(Problem { at: item.title.clone(), field: f.name.clone(), message: format!("缺「{}」", f.label) });
            }
        }
        out
    }
    /// The second thing a list row shows after the title (one line, no markup).
    fn line(&self, item: &Item) -> String;
    /// The item as it is written into a prompt (plain text, English labels, no trailing newline).
    fn prompt(&self, item: &Item) -> String;
}

// ── category ─────────────────────────────────────────────────────────────────

/// Where a category's words live.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Native {
    /// "" = the hub's own records · `csv` · `toml` · `dir` (one item per sub-folder) ·
    /// `files` (one item per file) · `flow` (the flow tool's catalogue) · `cards` (L-Brain's nodes).
    pub kind: String,
    /// The app's private folder name (`PaperComposer`, `SlideComposer`, `shared` …).
    pub app_dir: String,
    /// Relative to that folder; never absolute.
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Category {
    /// `<app>.<name>` for a Lore app's (`pc.patterns`), `my.<name>` for one made by hand,
    /// `x.<source>.<name>` for an outside application's independent set.
    pub id: String,
    pub label: String,
    /// The dashboard section: a Lore app id, `my`, or `x`.
    pub app: String,
    /// A heading inside the section (`流程圖`, `寫作準則` …); "" = none.
    pub group: String,
    pub format: String,
    /// `lore` (declared by an app's manifest) · `manual` · `external` (independent set).
    pub origin: String,
    /// lore-llm task ids that use this category. Empty = 未接上. Always empty for `external`.
    pub tasks: Vec<String>,
    pub native: Native,
    /// Items can be added / changed / removed through the hub (false: shown and tagged only).
    pub writable: bool,
}

impl Category {
    pub fn connected(&self) -> bool {
        !self.tasks.is_empty()
    }
    pub fn independent(&self) -> bool {
        self.origin == "external"
    }
}

/// An app's declaration (`know.toml`, included in its code): `[[category]]` tables of [`Category`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub category: Vec<Category>,
}

// ── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum KnowError {
    #[error("找不到：{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    /// The item changed since it was read.
    #[error("這一條已經被改過（第 {have} 版，送來的是第 {given} 版）")]
    Stale { have: u64, given: u64 },
    #[error("這一類不能在這裡改：{0}")]
    ReadOnly(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, KnowError>;

// ── reading and writing ──────────────────────────────────────────────────────

/// An app's existing data as items of one category (K2–K4). The hub's overlay (tags, triggers,
/// `contains`, usage) is added by the store, not here.
pub trait Adapter: Send + Sync {
    /// The items in the data now. Unreadable entries are skipped and named in the second value.
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)>;
    /// Add `item` or replace the one with its id, in the data's own format. [`KnowError::ReadOnly`]
    /// when this kind of data is not written from the hub.
    fn save(&self, item: &Item) -> Result<()>;
    /// Take the item out of the data and return it (the store keeps it in the trash).
    fn remove(&self, id: &str) -> Result<Item>;
    /// The picture file of the item now in the data, if it has one (the store keeps a copy of it
    /// in the trash when the item is removed). Default: none.
    fn picture_of(&self, _id: &str) -> Option<std::path::PathBuf> {
        None
    }
    /// Put the picture the trash kept (`from`) back as `name` beside the restored item. Default: nothing.
    fn restore_picture(&self, _id: &str, _name: &str, _from: &std::path::Path) -> Result<()> {
        Ok(())
    }
}

/// The one way to read and write items (the store implements it; [`MemSink`] in tests).
pub trait Sink {
    fn get(&self, id: &str) -> Result<Option<Item>>;
    fn list(&self, category: &str) -> Result<Vec<Item>>;
    /// Add (`item.id` empty: a new id in `item.category`) or replace (`item.revision` must be the
    /// stored one, else [`KnowError::Stale`]). Returns the stored item (id, times, revision set).
    fn put(&mut self, item: Item) -> Result<Item>;
    /// To the trash. Returns what was removed.
    fn remove(&mut self, id: &str) -> Result<Item>;
    /// The latest trashed copy back.
    fn restore(&mut self, id: &str) -> Result<Item>;
}

/// A [`Sink`] in memory, with the store's rules (ids, revisions, trash), for unit tests.
#[derive(Debug, Default)]
pub struct MemSink {
    pub items: BTreeMap<String, Item>,
    pub trash: Vec<Item>,
    next: u64,
}

impl MemSink {
    pub fn with(items: impl IntoIterator<Item = Item>) -> MemSink {
        let mut s = MemSink::default();
        for i in items {
            s.items.insert(i.id.clone(), i);
        }
        s
    }
}

impl Sink for MemSink {
    fn get(&self, id: &str) -> Result<Option<Item>> {
        Ok(self.items.get(id).cloned())
    }
    fn list(&self, category: &str) -> Result<Vec<Item>> {
        Ok(self.items.values().filter(|i| i.category == category).cloned().collect())
    }
    fn put(&mut self, mut item: Item) -> Result<Item> {
        item.tidy();
        if item.category.is_empty() {
            return Err(KnowError::Invalid("沒有類別".into()));
        }
        if item.id.is_empty() {
            self.next += 1;
            item.id = item_id(&item.category, &format!("m{:04}", self.next));
            item.created_at = now();
            item.revision = 0;
        } else if let Some(have) = self.items.get(&item.id) {
            if have.revision != item.revision {
                return Err(KnowError::Stale { have: have.revision, given: item.revision });
            }
            item.created_at = have.created_at.clone();
        } else if item.created_at.is_empty() {
            item.created_at = now();
        }
        item.revision += 1;
        item.updated_at = now();
        self.items.insert(item.id.clone(), item.clone());
        Ok(item)
    }
    fn remove(&mut self, id: &str) -> Result<Item> {
        let i = self.items.remove(id).ok_or_else(|| KnowError::NotFound(id.into()))?;
        self.trash.push(i.clone());
        Ok(i)
    }
    fn restore(&mut self, id: &str) -> Result<Item> {
        let at = self.trash.iter().rposition(|i| i.id == id).ok_or_else(|| KnowError::NotFound(id.into()))?;
        let i = self.trash.remove(at);
        self.items.insert(i.id.clone(), i.clone());
        Ok(i)
    }
}

// ── proposals ────────────────────────────────────────────────────────────────

/// One change of a [`Proposal`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Change {
    /// `add` · `update` · `remove`.
    pub op: String,
    /// `add`: the new item (id empty, or a temporary `new:<n>` other changes of the batch may
    /// name in `contains`). `update`: the whole item as it should be (its `revision` = the one read).
    pub item: Option<Item>,
    /// `remove`: the item's id.
    pub id: String,
    /// One line for the author: why.
    pub note: String,
    /// Set when applied: `applied` · `skipped` (not chosen) · `refused` (with `error`).
    pub result: String,
    pub error: String,
}

/// A batch an AI or a tool proposes. Nothing is written until the author approves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Proposal {
    pub id: String,
    /// Who proposes: `le` (L-Echo), a tool's name …
    pub by: String,
    /// One line for the author: what this batch is.
    pub title: String,
    pub changes: Vec<Change>,
    /// `open` · `applied` · `dismissed`.
    pub state: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_split_and_local_ids_are_safe_and_sorted() {
        assert_eq!(split_id("pc.patterns:P012"), Some(("pc.patterns", "P012")));
        assert_eq!(split_id("x.app.words:k01"), Some(("x.app.words", "k01")));
        assert_eq!(split_id("nocolon"), None);
        assert_eq!(safe_local(" a/b:c d "), "a-b-c-d");
        assert_eq!(safe_local("///"), "item");
        let (a, b) = (new_local(1), new_local(2));
        assert!(a.len() == 27 && a < b, "{a} {b}");
    }

    #[test]
    fn places_keep_the_authors_order_other_tags_are_sorted() {
        let mut i = Item::new("pc.patterns", format::PATTERN, "x", Source::author());
        i.tags.insert(facet::PLACE.into(), vec!["Method/訓練/超參數設定".into(), "Method/訓練/訓練環境與硬體".into(), "Method/訓練/超參數設定".into(), "Abstract/摘要/目標".into()]);
        i.tags.insert(facet::KIND.into(), vec!["b".into(), "a".into()]);
        i.tidy();
        assert_eq!(i.tags[facet::PLACE], ["Method/訓練/超參數設定", "Method/訓練/訓練環境與硬體", "Abstract/摘要/目標"]);
        assert_eq!(i.tags[facet::KIND], ["a", "b"]);
    }

    #[test]
    fn the_memory_sink_keeps_the_stores_rules() {
        let mut s = MemSink::default();
        let mut i = Item::new("my.notes", format::NOTE, " A ", Source::author());
        i.tag(facet::USE, "paper").tag(facet::USE, "paper").tag(facet::KIND, " timeline ");
        let a = s.put(i).unwrap();
        assert_eq!((a.title.as_str(), a.revision, a.tags[facet::USE].len()), ("A", 1, 1));
        let mut stale = a.clone();
        stale.revision = 0;
        assert!(matches!(s.put(stale), Err(KnowError::Stale { have: 1, given: 0 })));
        let b = s.put(a.clone()).unwrap();
        assert_eq!(b.revision, 2);
        s.remove(&b.id).unwrap();
        assert!(s.get(&b.id).unwrap().is_none());
        assert_eq!(s.restore(&b.id).unwrap().id, b.id);
        assert_eq!(s.list("my.notes").unwrap().len(), 1);
    }
}
