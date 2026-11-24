//! The whole hub in one object, for an app's server (docs/brain-hub.md §5).
//!
//! [`Hub::open`] reads the apps' manifests and the hub folder, gives every category the adapter
//! its `native` place asks for, and builds the index from the items. Every write goes through the
//! hub's own `put`, `remove`, `restore`, `import`, `apply_proposal` and `undo_proposal`, which keep
//! the index in step; writing through `hub.store` directly would leave the index behind (call
//! [`Hub::reindex`] after).
//!
//! | `native.kind` | format | adapter |
//! |---|---|---|
//! | `""` | any | the hub's own records |
//! | `csv` | pattern, expression, wording, rule, exemplar | [`CsvAdapter`] |
//! | `files` | note / layout | [`NoteFiles`] / [`LayoutFiles`] |
//! | `dir` | bundle / figure | [`BundleDirs`] / [`ReferenceDirs`] |
//! | `list` | layout | [`LayoutList`] |
//! | `flow` | figure | the flow tool's catalogue plus the saved drawings ([`FlowItems`]) |
//!
//! A folder or file that does not exist is an empty category; so is a category whose app folder
//! the caller did not give. A kind the hub has no reader for (`toml`, `cards`) reads as empty
//! with one problem ([`Store::problems`]), never as an error.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use lore_flow::figure::Catalogue;

use crate::formats::figure::{self, Drawings, ReferenceDirs, what};
use crate::formats::table::CsvAdapter;
use crate::formats::text::{BundleDirs, LayoutFiles, LayoutList, NoteFiles};
use crate::formats::{self, Formats};
use crate::index::{Count, Index, Query};
use crate::intake::{self, Batch, Report};
use crate::proposal::{self, Proposals};
use crate::registry::Registry;
use crate::store::Store;
use crate::types::*;

/// Everything an app's server needs.
pub struct Hub {
    pub store: Store,
    pub index: Index,
    pub proposals: Proposals,
    pub formats: Formats,
    flow_dirs: Vec<PathBuf>,
}

/// What is read when a category's data cannot be reached: nothing, and nothing can be written.
struct Nowhere(String);

impl Adapter for Nowhere {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        Ok((vec![], vec![]))
    }
    fn save(&self, _: &Item) -> Result<()> {
        Err(KnowError::ReadOnly(self.0.clone()))
    }
    fn remove(&self, id: &str) -> Result<Item> {
        Err(KnowError::NotFound(id.into()))
    }
}

/// The flow tool's own pieces as one category: every generator and every saved drawing of the
/// catalogue (read afresh each time, so a drawing saved by the tool shows at once). Generators are
/// the tool's own and read-only; a saved drawing is read and written by [`Drawings`].
pub struct FlowItems {
    category: String,
    source: Source,
    flow_dirs: Vec<PathBuf>,
    drawings: Drawings,
}

impl FlowItems {
    /// `save_dir` is where a drawing saved from the hub is written (`<save_dir>/modules/…`).
    pub fn new(category: &str, source: Source, flow_dirs: Vec<PathBuf>, save_dir: PathBuf) -> FlowItems {
        let drawings = Drawings { flow_dir: save_dir, category: category.into(), source: source.clone() };
        FlowItems { category: category.into(), source, flow_dirs, drawings }
    }

    fn catalogue(&self) -> Catalogue {
        let dirs: Vec<&Path> = self.flow_dirs.iter().map(PathBuf::as_path).collect();
        Catalogue::load_all(&dirs)
    }
}

impl Adapter for FlowItems {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let mut items = figure::from_catalogue(&self.catalogue(), &self.category, self.source.clone());
        // the saved drawings as the files say (they carry their level) win over the catalogue's copy
        let (saved, problems) = self.drawings.load()?;
        for d in saved {
            match items.iter().position(|i| i.id == d.id) {
                Some(at) => items[at] = d,
                None => items.push(d),
            }
        }
        Ok((items, problems))
    }

    fn save(&self, item: &Item) -> Result<()> {
        if item.field("what") != what::DRAWING {
            return Err(KnowError::ReadOnly("流程圖工具內建的畫法不能在這裡改，要改請另存成一個圖形".into()));
        }
        self.drawings.save(item)
    }

    fn remove(&self, id: &str) -> Result<Item> {
        match self.drawings.remove(id) {
            Err(KnowError::NotFound(_)) if self.load()?.0.iter().any(|i| i.id == id) => {
                Err(KnowError::ReadOnly("流程圖工具內建的畫法不能移除".into()))
            }
            other => other,
        }
    }
}

/// The adapter a category's `native` place asks for. `None` = the hub's own records, or a kind
/// nothing reads yet.
fn adapter_for(cat: &Category, app_dirs: &BTreeMap<String, PathBuf>, flow_dirs: &[PathBuf]) -> Result<Option<Box<dyn Adapter>>> {
    let n = &cat.native;
    if n.kind.is_empty() {
        return Ok(None);
    }
    let source = Source::lore(&cat.app);
    let Some(root) = app_dirs.get(&n.app_dir) else {
        // the app's folder was not given: an empty category that takes nothing
        return Ok(Some(Box::new(Nowhere(format!("{} 的資料夾還沒有設定", cat.label)))));
    };
    let place = if n.path.is_empty() { root.clone() } else { root.join(&n.path) };
    let wrong = || KnowError::Invalid(format!("類別 {} 的宣告不對：資料放在 {}，格式卻是 {}", cat.id, n.kind, cat.format));
    let a: Box<dyn Adapter> = match (n.kind.as_str(), cat.format.as_str()) {
        ("csv", format::PATTERN | format::EXPRESSION | format::WORDING | format::RULE | format::EXEMPLAR) => {
            Box::new(CsvAdapter::new(place, &cat.id, &cat.format, source))
        }
        ("files", format::NOTE) => Box::new(NoteFiles::new(place, &cat.id, source)),
        ("files", format::LAYOUT) => Box::new(LayoutFiles { dir: place, category: cat.id.clone(), source }),
        ("dir", format::BUNDLE) => Box::new(BundleDirs { dir: place, category: cat.id.clone(), source }),
        ("dir", format::FIGURE) => Box::new(ReferenceDirs { dir: place, category: cat.id.clone(), source }),
        ("list", format::LAYOUT) => Box::new(LayoutList { file: place, category: cat.id.clone(), source }),
        ("flow", format::FIGURE) => Box::new(FlowItems::new(&cat.id, source, flow_dirs.to_vec(), place)),
        ("csv" | "files" | "dir" | "list" | "flow", _) => return Err(wrong()),
        // `toml`, `cards`: no reader yet; the store lists the category as not connected
        _ => return Ok(None),
    };
    Ok(Some(a))
}

impl Hub {
    /// Open the hub in `hub_dir`.
    ///
    /// * `manifests`: the apps' `know.toml` texts.
    /// * `app_dirs`: `Native::app_dir` → that app's private folder.
    /// * `flow_dirs`: the flow tool's folders (`layouts/`, `modules/` …) its catalogue reads.
    pub fn open(hub_dir: PathBuf, manifests: &[&str], app_dirs: &BTreeMap<String, PathBuf>, flow_dirs: &[PathBuf]) -> Result<Hub> {
        let registry = Registry::load(manifests, &hub_dir)?;
        let mut store = Store::open(hub_dir.clone(), registry)?;
        for cat in store.registry().all().to_vec() {
            if let Some(a) = adapter_for(&cat, app_dirs, flow_dirs)? {
                store.add_adapter(&cat.id, a);
            }
        }
        let items = store.all()?;
        let index = Index::build(&items, store.registry().all())?;
        let proposals = Proposals::open(hub_dir.join("proposals"))?;
        Ok(Hub { store, index, proposals, formats: formats::all(), flow_dirs: flow_dirs.to_vec() })
    }

    /// Read everything again into the index (after the files changed outside the hub).
    pub fn reindex(&mut self) -> Result<()> {
        let items = self.store.all()?;
        self.index.rebuild(&items, self.store.registry().all())
    }

    // ── writes, each keeping the index in step ───────────────────────────────

    /// Add or change an item ([`Sink::put`]); the item comes back with the id the app's data
    /// gives it now.
    pub fn put(&mut self, item: Item) -> Result<Item> {
        let given = item.id.clone();
        let stored = self.store.put(item)?;
        if !given.is_empty() && given != stored.id {
            self.index.delete(&given)?;
        }
        self.index.upsert(&stored)?;
        Ok(stored)
    }

    /// To the trash ([`Sink::remove`]).
    pub fn remove(&mut self, id: &str) -> Result<Item> {
        let gone = self.store.remove(id)?;
        self.index.delete(id)?;
        Ok(gone)
    }

    /// The latest trashed copy back ([`Sink::restore`]).
    pub fn restore(&mut self, id: &str) -> Result<Item> {
        let back = self.store.restore(id)?;
        self.index.upsert(&back)?;
        Ok(back)
    }

    /// Count one use of each item (what a step took into its prompt).
    pub fn record_use(&mut self, ids: &[String]) -> Result<()> {
        self.store.record_use(ids)?;
        for id in ids {
            if let Some(i) = self.store.get(id)? {
                self.index.upsert(&i)?;
            }
        }
        Ok(())
    }

    /// Count one use as [`Hub::record_use`] and log it: one line in `<hub>/usage.jsonl` with the
    /// step (`task`) and the day, which is what the dashboard's step figures are made from.
    pub fn record_use_by(&mut self, task: &str, ids: &[String], day: chrono::NaiveDate) -> Result<()> {
        self.record_use(ids)?;
        crate::reader::record_use(self.store.hub(), task, ids, day)
    }

    /// An outside application's batch ([`intake::import`]). An independent set is made first, so
    /// the store knows it. An empty `source` is the author's own file only when the caller says
    /// so (`own`); otherwise it is refused.
    pub fn import(&mut self, batch: &Batch, own: bool) -> Result<Report> {
        if batch.source.trim().is_empty() && !own {
            return Err(KnowError::Invalid("缺 source（應用程式的名字）；沒有名字的檔案只有作者自己的才收".into()));
        }
        if let intake::Target::Independent { name, format } = &batch.target {
            let source = batch.source.trim();
            if !self.formats.contains_key(format) {
                return Err(KnowError::Invalid(format!("沒有「{format}」這個格式；應用程式不能帶來新格式")));
            }
            if source.is_empty() {
                return Err(KnowError::Invalid("缺 source（應用程式的名字）".into()));
            }
            self.store.registry_mut().add_external(source, name.trim(), format)?;
        }
        // the import reads the category list while it writes through the store: it gets a copy
        let mut cats = self.store.registry().clone();
        let report = intake::import(&mut self.store, &mut cats, &self.formats, batch);
        self.reindex()?;
        report
    }

    /// A category (or one source's part of it) as a batch ([`intake::export`]).
    pub fn export(&self, category: &str, source: Option<&Source>) -> Result<Batch> {
        let cat = self.store.registry().get(category).ok_or_else(|| KnowError::NotFound(format!("類別 {category}")))?;
        intake::export(&self.store, cat, source)
    }

    /// Write the chosen changes (`None` = all) of an open proposal ([`proposal::apply`]).
    pub fn apply_proposal(&mut self, id: &str, chosen: Option<&[usize]>) -> Result<Proposal> {
        let r = proposal::apply(&mut self.store, &self.proposals, id, chosen);
        self.reindex()?;
        r
    }

    /// Reverse an applied proposal ([`proposal::undo`]); the lines say what could not be undone.
    pub fn undo_proposal(&mut self, id: &str) -> Result<Vec<String>> {
        let r = proposal::undo(&mut self.store, &self.proposals, id);
        self.reindex()?;
        r
    }

    // ── reads ────────────────────────────────────────────────────────────────

    /// The items a list asks for, in order, and how many match before `limit` / `offset`.
    pub fn query(&self, q: &Query) -> Result<(Vec<Item>, usize)> {
        let (ids, total) = self.index.query(q)?;
        // one read per category, however many of its items are asked for
        let mut read: HashMap<String, HashMap<String, Item>> = HashMap::new();
        let mut out = vec![];
        for id in ids {
            let Some((cat, _)) = split_id(&id) else { continue };
            if !read.contains_key(cat) {
                let items = self.store.list(cat)?;
                read.insert(cat.to_string(), items.into_iter().map(|i| (i.id.clone(), i)).collect());
            }
            if let Some(i) = read[cat].get(&id) {
                out.push(i.clone());
            }
        }
        Ok((out, total))
    }

    /// One [`Count`] per category.
    pub fn counts(&self) -> Result<Vec<Count>> {
        self.index.counts()
    }

    /// The SVG of a drawable figure item, drawn by the flow tool (a reference is not drawn: its
    /// picture is a file the caller serves).
    pub fn thumbnail(&self, id: &str) -> Result<String> {
        let item = self.store.get(id)?.ok_or_else(|| KnowError::NotFound(id.into()))?;
        if item.format != format::FIGURE {
            return Err(KnowError::Invalid("這一條不是圖形，沒有縮圖可畫".into()));
        }
        let dirs: Vec<&Path> = self.flow_dirs.iter().map(PathBuf::as_path).collect();
        figure::thumbnail(&item, &Catalogue::load_all(&dirs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::{Entry, Target};

    const DRAWING: &str = include_str!("../../../../private-example/SlideComposer/flow/modules/document/page.json");

    const MANIFEST: &str = r#"
[[category]]
id = "t.words"
label = "用詞"
format = "wording"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "words.csv"

[[category]]
id = "t.notes"
label = "筆記"
format = "note"
writable = true
tasks = ["draft"]
[category.native]
kind = "files"
app_dir = "App"
path = "notes"

[[category]]
id = "t.layouts"
label = "版型檔"
format = "layout"
[category.native]
kind = "files"
app_dir = "App"
path = "layouts"

[[category]]
id = "t.packs"
label = "整包"
format = "bundle"
[category.native]
kind = "dir"
app_dir = "App"
path = "packs"

[[category]]
id = "t.refs"
label = "參考圖"
format = "figure"
writable = true
[category.native]
kind = "dir"
app_dir = "App"
path = "refs"

[[category]]
id = "t.list"
label = "版型表"
format = "layout"
[category.native]
kind = "list"
app_dir = "App"
path = "list.json"

[[category]]
id = "t.flow"
label = "流程圖"
format = "figure"
writable = true
[category.native]
kind = "flow"
app_dir = "App"
path = "flow"

[[category]]
id = "t.exprs"
label = "常用詞"
format = "expression"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "exprs.csv"

[[category]]
id = "t.own"
label = "自己的"
format = "rule"
writable = true
"#;

    fn app(dir: &Path) {
        let w = |p: &str, text: &str| {
            let p = dir.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        w("words.csv", "avoid,prefer,note\nutilize,use,\n");
        w("exprs.csv", "id,kind,text,usage,section,example,tags\nE1,phrase,in contrast,,,,\n");
        w("notes/Plan.md", "# Plan\n\nfirst note\n");
        w("layouts/grid.json", r#"{"name":"Grid","group":"basic","description":"three columns"}"#);
        w("packs/journal-a/template.toml", "name = \"Journal A\"\ndescription = \"a template\"\n");
        w("packs/journal-a/main.tex", "\\documentclass{article}\n");
        w("refs/f01/desc.md", "# f01 Overview\n\nTwo rows of boxes.\n");
        w("list.json", r#"[{"id":"a","name":"Alpha","group":"g"},{"id":"b","name":"Beta"}]"#);
        w("flow/modules/document/page.json", DRAWING);
    }

    fn open(hub: &Path, app_root: &Path) -> Hub {
        let dirs = BTreeMap::from([("App".to_string(), app_root.to_path_buf())]);
        Hub::open(hub.to_path_buf(), &[MANIFEST], &dirs, &[app_root.join("flow")]).unwrap()
    }

    fn ids(h: &Hub, q: Query) -> Vec<String> {
        h.query(&q).unwrap().0.into_iter().map(|i| i.id).collect()
    }

    fn in_cat(c: &str) -> Query {
        Query { category: Some(c.into()), ..Default::default() }
    }

    #[test]
    fn a_hub_opens_every_kind_of_category() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        app(root.path());
        let h = open(hub.path(), root.path());
        let counts: BTreeMap<String, usize> = h.counts().unwrap().into_iter().map(|c| (c.category, c.total)).collect();
        for (c, n) in [("t.words", 1), ("t.exprs", 1), ("t.notes", 1), ("t.layouts", 1), ("t.packs", 1), ("t.refs", 1), ("t.list", 2), ("t.own", 0)] {
            assert_eq!(counts[c], n, "{c}");
        }
        assert!(counts["t.flow"] > 90, "the flow tool's own pieces and the saved drawing: {}", counts["t.flow"]);
        // each kind reads as its format says
        let one = |c: &str| h.query(&in_cat(c)).unwrap().0.remove(0);
        assert_eq!((one("t.words").id.as_str(), one("t.words").field("prefer")), ("t.words:utilize", "use"));
        assert_eq!(one("t.notes").title, "Plan");
        assert_eq!(one("t.layouts").title, "Grid");
        let pack = one("t.packs");
        assert_eq!((pack.title.as_str(), pack.fields["files"].as_array().unwrap().len()), ("Journal A", 2));
        let r = one("t.refs");
        assert_eq!((r.title.as_str(), r.entity, r.level.as_str()), ("Overview", false, "figure"));
        assert_eq!(h.query(&in_cat("t.list")).unwrap().0.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Alpha", "Beta"]);
        // source: the app, never the author
        assert!(h.query(&Query::default()).unwrap().0.iter().all(|i| i.source == Source::lore("t")));
        // a saved drawing is an entity with a level; the tool's own pieces draw
        let flow = ids(&h, Query { category: Some("t.flow".into()), text: "document".into(), ..Default::default() });
        assert!(flow.contains(&"t.flow:document.page".to_string()), "{flow:?}");
        assert!(h.thumbnail("t.flow:document.page").unwrap().starts_with("<svg"));
        assert!(h.thumbnail("t.flow:unet.bars").unwrap().contains("viewBox"));
        // a reference has no drawing, a note is not a figure, an unknown id is unknown
        assert!(h.thumbnail("t.refs:f01").is_err() && h.thumbnail("t.notes:Plan").is_err());
        assert!(matches!(h.thumbnail("t.flow:nope.nope"), Err(KnowError::NotFound(_))));
        // sorting and filtering go through the index
        let (page, total) = h.query(&Query { category: Some("t.flow".into()), level: Some("component".into()), limit: 2, ..Default::default() }).unwrap();
        assert!(page.len() == 2 && total > 2 && page.iter().all(|i| i.level == "component"));
    }

    #[test]
    fn writes_keep_the_index_in_step() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        app(root.path());
        let mut h = open(hub.path(), root.path());
        let text = |t: &str| Query { text: t.into(), ..Default::default() };
        // add: found by its words at once, with the id the files give
        let mut n = Item::new("t.notes", format::NOTE, "Roadmap", Source::author());
        n.set("body", "zebra crossing ideas");
        let n = h.put(n).unwrap();
        assert_eq!(n.id, "t.notes:Roadmap");
        assert_eq!(ids(&h, text("zebra")), std::slice::from_ref(&n.id));
        // change: the old words go, the new ones come
        let mut n2 = n.clone();
        n2.set("body", "# Roadmap\n\nquokka plans");
        n2.tag(facet::USE, "paper");
        let n2 = h.put(n2).unwrap();
        assert!(ids(&h, text("zebra")).is_empty());
        assert_eq!(ids(&h, text("quokka")), std::slice::from_ref(&n.id));
        let by_tag = Query { tags: BTreeMap::from([(facet::USE.to_string(), vec!["paper".to_string()])]), ..Default::default() };
        assert_eq!(ids(&h, by_tag), std::slice::from_ref(&n.id));
        // a wording row renamed has a new id; the old one is out of the index
        let mut w = h.query(&in_cat("t.words")).unwrap().0.remove(0);
        w.set("avoid", "utilise");
        let w = h.put(w).unwrap();
        assert_eq!(w.id, "t.words:utilise");
        assert_eq!(ids(&h, in_cat("t.words")), ["t.words:utilise"]);
        // remove: gone from the list and the counts; restore: back
        let before = h.counts().unwrap().into_iter().find(|c| c.category == "t.notes").unwrap().total;
        h.remove(&n2.id).unwrap();
        assert!(ids(&h, text("quokka")).is_empty());
        assert_eq!(h.counts().unwrap().into_iter().find(|c| c.category == "t.notes").unwrap().total, before - 1);
        h.restore(&n2.id).unwrap();
        assert_eq!(ids(&h, text("quokka")), std::slice::from_ref(&n.id));
        // use is counted and sorts
        h.record_use(std::slice::from_ref(&n.id)).unwrap();
        let top = h.query(&Query { sort: "used".into(), desc: true, limit: 1, ..Default::default() }).unwrap().0;
        assert_eq!((top[0].id.as_str(), top[0].used), (n.id.as_str(), 1));
        // the index is what a fresh open would build
        let fresh = open(hub.path(), root.path());
        let all = |h: &Hub| ids(h, Query { sort: "created".into(), ..Default::default() });
        assert_eq!(all(&h), all(&fresh));
        assert_eq!(h.counts().unwrap(), fresh.counts().unwrap());
        // a refused write changes nothing
        let mut g = h.query(&in_cat("t.layouts")).unwrap().0.remove(0);
        g.title = "Other".into();
        assert!(matches!(h.put(g), Err(KnowError::ReadOnly(_))));
        assert!(matches!(h.remove("t.flow:unet.bars"), Err(KnowError::ReadOnly(_)) | Err(KnowError::Invalid(_))));
        assert_eq!(h.counts().unwrap(), fresh.counts().unwrap());
    }

    #[test]
    fn an_outside_batch_and_a_proposal_keep_the_index_in_step_too() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        app(root.path());
        let mut h = open(hub.path(), root.path());
        let entry = |id: &str, text: &str| {
            let mut e = Entry { id: id.into(), title: text.into(), ..Default::default() };
            e.fields.insert("text".into(), text.into());
            e
        };
        let batch = |items: Vec<Entry>| Batch { lore_know: 1, source: "wordbox".into(), target: Target::Align { category: "t.exprs".into() }, items };
        // aligned into a CSV-backed category: it keeps its outside source, and a second batch replaces only that source's rows
        assert_eq!(h.import(&batch(vec![entry("1", "however"), entry("2", "thus")]), false).unwrap().added, 2);
        assert_eq!(ids(&h, in_cat("t.exprs")).len(), 3);
        let r = h.import(&batch(vec![entry("1", "nevertheless")]), false).unwrap();
        assert_eq!((r.added, r.replaced, r.removed), (0, 1, 1));
        let rows = h.query(&in_cat("t.exprs")).unwrap().0;
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().any(|i| i.id == "t.exprs:E1" && i.source == Source::lore("t")), "his own row stays");
        assert!(rows.iter().any(|i| i.title == "nevertheless" && i.source == Source::external("wordbox")));
        // a wording row is known by its `avoid` text, so the id an outside batch gives does not come back from the file
        let word = |avoid: &str, prefer: &str| {
            let mut e = Entry { id: "1".into(), title: avoid.into(), ..Default::default() };
            e.fields.insert("avoid".into(), avoid.into());
            e.fields.insert("prefer".into(), prefer.into());
            e
        };
        let wb = |items: Vec<Entry>| Batch { target: Target::Align { category: "t.words".into() }, ..batch(items) };
        assert_eq!(h.import(&wb(vec![word("commence", "start")]), false).unwrap().added, 1);
        // the same file again changes nothing
        let again = h.import(&wb(vec![word("commence", "start")]), false).unwrap();
        assert_eq!((again.added, again.replaced, again.removed, again.problems.len()), (0, 1, 0, 0), "{again:?}");
        assert_eq!(ids(&h, in_cat("t.words")).len(), 2);
        // a changed answer replaces the row, and his own row stays
        let changed = h.import(&wb(vec![word("commence", "begin")]), false).unwrap();
        assert_eq!((changed.added, changed.replaced, changed.removed, changed.problems.len()), (0, 1, 0, 0), "{changed:?}");
        let words = h.query(&in_cat("t.words")).unwrap().0;
        assert_eq!(words.len(), 2);
        assert!(words.iter().any(|i| i.field("avoid") == "commence" && i.field("prefer") == "begin" && i.source == Source::external("wordbox")));
        assert!(words.iter().any(|i| i.id == "t.words:utilize" && i.source == Source::lore("t")));
        // an independent set shows up in the counts
        let ind = Batch {
            lore_know: 1,
            source: "notesapp".into(),
            target: Target::Independent { name: "Ideas".into(), format: format::NOTE.into() },
            items: vec![Entry { id: "1".into(), title: "x".into(), fields: serde_json::from_value(serde_json::json!({"body": "y"})).unwrap(), ..Default::default() }],
        };
        let rep = h.import(&ind, false).unwrap();
        assert_eq!(h.counts().unwrap().into_iter().find(|c| c.category == rep.category).unwrap().total, 1);
        // a proposal applied is in the index, and undone is out
        let mut add = Item::new("t.own", format::RULE, "Short", Source::lore("le"));
        add.set("message", "Keep it short.");
        let p = h.proposals.create("le", "one rule", vec![Change { op: "add".into(), item: Some(add), ..Default::default() }]).unwrap();
        h.apply_proposal(&p.id, None).unwrap();
        assert_eq!(ids(&h, in_cat("t.own")).len(), 1);
        assert!(h.undo_proposal(&p.id).unwrap().is_empty());
        assert!(ids(&h, in_cat("t.own")).is_empty());
    }

    #[test]
    fn a_missing_folder_is_an_empty_category() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        // nothing was made in the app's folder at all
        let mut h = open(hub.path(), root.path());
        let counts = h.counts().unwrap();
        assert_eq!(counts.len(), 9);
        for c in &counts {
            // the flow tool's own pieces are there without any folder
            if c.category != "t.flow" {
                assert_eq!(c.total, 0, "{}", c.category);
            }
        }
        assert!(h.store.problems("t.notes").is_empty());
        // an app folder that was not given: empty, nothing to write into, no error
        let h2 = Hub::open(hub.path().join("two"), &[MANIFEST], &BTreeMap::new(), &[]).unwrap();
        assert!(h2.counts().unwrap().iter().filter(|c| c.category != "t.flow").all(|c| c.total == 0));
        assert!(h2.store.list("t.notes").unwrap().is_empty());
        // writing into a missing folder makes it
        let mut n = Item::new("t.notes", format::NOTE, "First", Source::author());
        n.set("body", "hello");
        h.put(n).unwrap();
        assert!(root.path().join("notes/First.md").is_file());
        // the hub's own records need no folder either
        let mut r = Item::new("t.own", format::RULE, "R", Source::author());
        r.set("message", "m");
        h.put(r).unwrap();
        assert_eq!(ids(&h, in_cat("t.own")).len(), 1);
        // a declaration that contradicts itself is an error that says so
        let bad = "[[category]]\nid = \"t.bad\"\nformat = \"figure\"\n[category.native]\nkind = \"csv\"\napp_dir = \"App\"\npath = \"x.csv\"\n";
        let dirs = BTreeMap::from([("App".to_string(), root.path().to_path_buf())]);
        assert!(matches!(Hub::open(hub.path().join("three"), &[bad], &dirs, &[]), Err(KnowError::Invalid(_))));
    }

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_builtin_manifests_load_into_one_registry() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::load(&crate::reader::builtin_manifests(), dir.path()).unwrap();
        let ids: Vec<&str> = reg.all().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "pc.patterns", "pc.expressions", "pc.rules", "pc.wording", "pc.exemplars", "pc.notes", "pc.templates", "pc.venues", "pc.stylerefs",
                "sc.flow", "sc.flowrefs", "sc.voice", "sc.styles", "sc.arrangements", "shared.plotstyles"
            ]
        );
        let c = |id: &str| reg.get(id).unwrap().clone();
        assert_eq!((c("pc.patterns").format.as_str(), c("pc.patterns").native.path.as_str(), c("pc.patterns").writable), ("pattern", "profile/phrases.csv", true));
        assert_eq!((c("pc.templates").group.as_str(), c("pc.templates").writable, c("pc.templates").native.kind.as_str()), ("論文模板", false, "dir"));
        assert_eq!((c("sc.flowrefs").native.path.as_str(), c("sc.flowrefs").native.app_dir.as_str()), ("flow/examples", "SlideComposer"));
        assert_eq!((c("shared.plotstyles").app.as_str(), c("shared.plotstyles").native.app_dir.as_str()), ("shared", "shared"));
        assert!(reg.all().iter().all(|x| x.origin == "lore"));
        // the steps known to read a category are declared (L-Paper's were read from its prompts)
        assert!(reg.get("pc.patterns").unwrap().tasks.contains(&"pc-compose-paragraph".to_string()));
    }

    #[test]
    fn a_hub_opens_on_the_builtin_manifests_with_empty_folders() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dirs: BTreeMap<String, PathBuf> =
            ["PaperComposer", "SlideComposer", "shared"].iter().map(|n| (n.to_string(), root.path().join(n))).collect();
        let h = Hub::open(hub.path().to_path_buf(), &crate::reader::builtin_manifests(), &dirs, &[]).unwrap();
        let counts = h.counts().unwrap();
        assert_eq!(counts.len(), 15);
        // the flow tool's own generators are there without any folder; everything else is empty
        for c in counts.iter().filter(|c| c.category != "sc.flow") {
            assert_eq!(c.total, 0, "{}", c.category);
        }
        for c in h.store.registry().all() {
            if c.id != "sc.flow" {
                assert!(h.store.problems(&c.id).is_empty(), "{}", c.id);
            }
        }
    }

    #[test]
    fn a_chinese_note_and_a_chinese_set_name_work() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        app(root.path());
        let mut h = open(hub.path(), root.path());
        let mut n = Item::new("t.notes", format::NOTE, "中文筆記", Source::author());
        n.set("body", "內容一");
        let n = h.put(n).unwrap();
        assert!(n.id.starts_with("t.notes:k"), "{}", n.id);
        assert_eq!(n.title, "中文筆記");
        let mut m = Item::new("t.notes", format::NOTE, "中文筆記", Source::author());
        m.set("body", "內容二");
        let m = h.put(m).unwrap();
        assert_ne!(m.id, n.id);
        let fresh = open(hub.path(), root.path());
        assert_eq!(fresh.store.get(&n.id).unwrap().unwrap().title, "中文筆記");
        // an outside application and its set, named in Chinese
        let entry = |id: &str, t: &str| {
            let mut e = Entry { id: id.into(), title: t.into(), ..Default::default() };
            e.fields.insert("body".into(), t.into());
            e
        };
        let ind = |source: &str, name: &str, items: Vec<Entry>| Batch {
            lore_know: 1,
            source: source.into(),
            target: Target::Independent { name: name.into(), format: format::NOTE.into() },
            items,
        };
        let a = h.import(&ind("某應用", "想法", vec![entry("甲", "第一"), entry("乙", "第二")]), false).unwrap();
        assert_eq!((a.added, a.problems.len()), (2, 0), "{a:?}");
        let b = h.import(&ind("另一個應用", "想法", vec![entry("甲", "第三")]), false).unwrap();
        assert_ne!(a.category, b.category, "two Chinese sources do not meet");
        let cat = h.store.registry().get(&a.category).unwrap().clone();
        assert_eq!((cat.label.as_str(), cat.group.as_str()), ("想法", "某應用"));
        let items = h.store.list(&a.category).unwrap();
        assert_eq!(items.len(), 2);
        assert_ne!(items[0].id, items[1].id, "two Chinese ids stay two");
        // the same source again replaces only its own
        let again = h.import(&ind("某應用", "想法", vec![entry("甲", "第一改")]), false).unwrap();
        assert_eq!((again.category.as_str(), again.replaced, again.removed), (a.category.as_str(), 1, 1));
        assert_eq!(h.store.list(&b.category).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_source_needs_the_authors_own_flag() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        app(root.path());
        let mut h = open(hub.path(), root.path());
        let mut e = Entry { id: "1".into(), title: "Short".into(), ..Default::default() };
        e.fields.insert("message".into(), "Keep it short.".into());
        let batch = Batch { lore_know: 1, source: String::new(), target: Target::Align { category: "t.own".into() }, items: vec![e] };
        assert!(matches!(h.import(&batch, false), Err(KnowError::Invalid(_))));
        assert!(ids(&h, in_cat("t.own")).is_empty());
        let r = h.import(&batch, true).unwrap();
        assert_eq!((r.added, r.problems.len()), (1, 0), "{r:?}");
        assert_eq!(h.query(&in_cat("t.own")).unwrap().0[0].source, Source::author());
    }

    #[test]
    fn the_hub_logs_use_by_step() {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        app(root.path());
        let mut h = open(hub.path(), root.path());
        let ids_ = vec!["t.words:utilize".to_string()];
        h.record_use_by("draft", &ids_, day("2026-11-24")).unwrap();
        assert_eq!(h.store.get(&ids_[0]).unwrap().unwrap().used, 1);
        let log = crate::reader::usage(hub.path(), day("2026-11-01"));
        assert_eq!(log, vec![(day("2026-11-24"), "draft".to_string(), ids_)]);
    }
}
