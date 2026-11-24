//! The hub folder: records, overlays and trash behind [`Sink`].
//!
//! ```text
//! <hub>/items/<category>/<local>.json          one Item per file (categories with native.kind "", and the
//!                                              items an outside application gave to an app's category:
//!                                              those are never written into the app's own data)
//! <hub>/meta/<category>/<local>.json           bookkeeping beside an app's own data (adapter categories)
//! <hub>/trash/<category>/<local>.<unix ms>.json  removed items
//! <hub>/trash/<category>/<local>.<unix ms>.<ext> the picture of a removed reference, beside its JSON
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;

use crate::registry::Registry;
use crate::util::write_atomic;
use crate::types::*;

/// What the hub keeps beside an item of an adapter category.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Meta {
    #[serde(flatten)]
    overlay: Overlay,
    revision: u64,
    created_at: String,
    updated_at: String,
    /// Who supplied the item, when the app's own data cannot say (a CSV row has no source column).
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Source>,
}

fn ms_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| KnowError::Other(format!("{} 讀不了：{e}", path.display())))
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| KnowError::Other(format!("存不了：{e}")))?;
    write_atomic(path, &bytes)
}

/// A local id must be usable as a file name.
fn check_local(local: &str) -> Result<()> {
    if local.is_empty() || local.starts_with('.') || local.contains(['/', '\\', '\0']) {
        return Err(KnowError::Invalid(format!("代號「{local}」不能當檔名")));
    }
    Ok(())
}

/// Entries of `a` not already in `b`, per facet.
fn tags_minus(a: &BTreeMap<String, Vec<String>>, b: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for (f, v) in a {
        let have = b.get(f);
        let rest: Vec<String> = v.iter().filter(|t| !have.is_some_and(|h| h.contains(t))).cloned().collect();
        if !rest.is_empty() {
            out.insert(f.clone(), rest);
        }
    }
    out
}

fn list_minus(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|x| !b.contains(x)).cloned().collect()
}

fn union(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = a.to_vec();
    for x in b {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

/// The hub's store.
pub struct Store {
    hub: PathBuf,
    registry: Registry,
    adapters: BTreeMap<String, Box<dyn Adapter>>,
}

impl Store {
    pub fn open(hub: PathBuf, registry: Registry) -> Result<Store> {
        fs::create_dir_all(&hub)?;
        Ok(Store { hub, registry, adapters: BTreeMap::new() })
    }

    /// The adapter that reads and writes the app's own data of `category`.
    pub fn add_adapter(&mut self, category: &str, a: Box<dyn Adapter>) {
        self.adapters.insert(category.into(), a);
    }

    pub fn hub(&self) -> &Path {
        &self.hub
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    fn dir(&self, kind: &str, category: &str) -> PathBuf {
        self.hub.join(kind).join(category)
    }

    fn file(&self, kind: &str, id: &str) -> Result<PathBuf> {
        let (c, l) = split_id(id).ok_or_else(|| KnowError::Invalid(format!("代號不對：{id}")))?;
        check_local(l)?;
        Ok(self.dir(kind, c).join(format!("{l}.json")))
    }

    fn category(&self, id: &str) -> Result<Category> {
        self.registry.get(id).cloned().ok_or_else(|| KnowError::NotFound(format!("類別 {id}")))
    }

    /// Is `id` a hub record (an item file under `items/`)?
    fn is_record(&self, id: &str) -> bool {
        self.file("items", id).map(|p| p.exists()).unwrap_or(false)
    }

    fn read_meta(&self, id: &str) -> Meta {
        self.file("meta", id).ok().filter(|p| p.exists()).and_then(|p| read_json(&p).ok()).unwrap_or_default()
    }

    fn write_meta(&self, id: &str, m: &Meta) -> Result<()> {
        write_json(&self.file("meta", id)?, m)
    }

    /// The adapter's item merged with the hub's bookkeeping.
    fn merge(cat: &Category, mut item: Item, meta: Meta) -> Item {
        item.category = cat.id.clone();
        if item.format.is_empty() {
            item.format = cat.format.clone();
        }
        for (f, tags) in &meta.overlay.tags {
            let e = item.tags.entry(f.clone()).or_default();
            *e = union(e, tags);
        }
        item.tidy();
        item.triggers = union(&item.triggers, &meta.overlay.triggers);
        item.contains = union(&item.contains, &meta.overlay.contains);
        if let Some(src) = meta.source {
            item.source = src;
        }
        item.used = meta.overlay.used;
        item.used_at = meta.overlay.used_at;
        item.revision = meta.revision.max(1);
        if !meta.created_at.is_empty() {
            item.created_at = meta.created_at;
        }
        if !meta.updated_at.is_empty() {
            item.updated_at = meta.updated_at;
        }
        item
    }

    fn adapter(&self, cat: &Category) -> Option<&dyn Adapter> {
        self.adapters.get(&cat.id).map(|b| b.as_ref())
    }

    fn read_records(&self, cat: &Category) -> (Vec<Item>, Vec<Problem>) {
        let mut items = vec![];
        let mut problems = vec![];
        let Ok(rd) = fs::read_dir(self.dir("items", &cat.id)) else { return (items, problems) };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for p in files {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match read_json::<Item>(&p) {
                Ok(mut i) => {
                    i.id = item_id(&cat.id, &stem);
                    i.category = cat.id.clone();
                    items.push(i);
                }
                Err(e) => problems.push(Problem { at: stem, field: String::new(), message: format!("讀不了這一條：{e}") }),
            }
        }
        (items, problems)
    }

    fn load_category(&self, cat: &Category) -> Result<(Vec<Item>, Vec<Problem>)> {
        if cat.native.kind.is_empty() {
            return Ok(self.read_records(cat));
        }
        // the app's own data, then the hub records of outside sources
        let (mut items, mut problems) = match self.adapter(cat) {
            Some(a) => {
                let (items, problems) = a.load()?;
                let merged = items
                    .into_iter()
                    .map(|i| {
                        let meta = self.read_meta(&i.id);
                        Self::merge(cat, i, meta)
                    })
                    .collect();
                (merged, problems)
            }
            None => (vec![], vec![Problem { at: cat.id.clone(), field: String::new(), message: "這一類還沒有接上讀取程式".into() }]),
        };
        let (records, rp) = self.read_records(cat);
        problems.extend(rp);
        for r in records {
            if items.iter().any(|i| i.id == r.id) {
                problems.push(Problem { at: r.id.clone(), field: "id".into(), message: "中樞裡的一條和這一類自己資料裡的一條同編號，略過中樞的".into() });
            } else {
                items.push(r);
            }
        }
        Ok((items, problems))
    }

    /// What `adapter.load` skipped (adapter categories) or what could not be read (records).
    pub fn problems(&self, category: &str) -> Vec<Problem> {
        match self.category(category).and_then(|c| self.load_category(&c)) {
            Ok((_, p)) => p,
            Err(e) => vec![Problem { at: category.into(), field: String::new(), message: e.to_string() }],
        }
    }

    /// Every item of every category.
    pub fn all(&self) -> Result<Vec<Item>> {
        let mut out = vec![];
        for c in self.registry.all() {
            out.extend(self.list(&c.id)?);
        }
        Ok(out)
    }

    /// Count one use of each item (no new revision).
    pub fn record_use(&mut self, ids: &[String]) -> Result<()> {
        let stamp = now();
        for id in ids {
            let Some((c, _)) = split_id(id) else { continue };
            let Some(cat) = self.registry.get(c).cloned() else { continue };
            if cat.native.kind.is_empty() || self.is_record(id) {
                let path = self.file("items", id)?;
                if let Ok(mut i) = read_json::<Item>(&path) {
                    i.used += 1;
                    i.used_at = stamp.clone();
                    write_json(&path, &i)?;
                }
            } else if self.get(id)?.is_some() {
                let mut m = self.read_meta(id);
                m.overlay.used += 1;
                m.overlay.used_at = stamp.clone();
                self.write_meta(id, &m)?;
            }
        }
        Ok(())
    }

    /// Write the bookkeeping of an adapter item: only what the adapter's own item does not carry.
    fn write_meta_for(&self, cat: &Category, item: &Item) -> Result<()> {
        let own = match self.adapter(cat) {
            Some(a) => a.load()?.0.into_iter().find(|i| i.id == item.id),
            None => None,
        };
        // the app's data keeps no source: the hub keeps it when it differs from what the adapter reports
        let source = own.as_ref().filter(|o| o.source != item.source).map(|_| item.source.clone());
        let (otags, otrig, ocont) = own.map(|o| (o.tags, o.triggers, o.contains)).unwrap_or_default();
        let m = Meta {
            overlay: Overlay {
                tags: tags_minus(&item.tags, &otags),
                triggers: list_minus(&item.triggers, &otrig),
                contains: list_minus(&item.contains, &ocont),
                used: item.used,
                used_at: item.used_at.clone(),
            },
            revision: item.revision,
            created_at: item.created_at.clone(),
            updated_at: item.updated_at.clone(),
            source,
        };
        self.write_meta(&item.id, &m)
    }

    fn trash_put(&self, item: &Item) -> Result<PathBuf> {
        let (c, l) = split_id(&item.id).ok_or_else(|| KnowError::Invalid(item.id.clone()))?;
        let mut ms = ms_now();
        loop {
            let p = self.dir("trash", c).join(format!("{l}.{ms}.json"));
            if !p.exists() {
                write_json(&p, item)?;
                return Ok(p);
            }
            ms += 1;
        }
    }

    /// The picture the trash kept beside the JSON copy `json` (`<local>.<ms>.<ext>`), if any.
    fn trash_picture(json: &Path) -> Option<PathBuf> {
        let stem = json.file_stem()?.to_string_lossy().into_owned();
        let prefix = format!("{stem}.");
        fs::read_dir(json.parent()?)
            .ok()?
            .filter_map(|e| e.ok())
            .find(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with(&prefix) && !n.ends_with(".json") && !n[prefix.len()..].contains('.')
            })
            .map(|e| e.path())
    }

    fn trash_latest(&self, id: &str) -> Result<(PathBuf, Item)> {
        let (c, l) = split_id(id).ok_or_else(|| KnowError::Invalid(id.into()))?;
        let mut best: Option<(u64, PathBuf)> = None;
        if let Ok(rd) = fs::read_dir(self.dir("trash", c)) {
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(stem) = name.strip_suffix(".json") else { continue };
                let Some((local, ms)) = stem.rsplit_once('.') else { continue };
                let Ok(ms) = ms.parse::<u64>() else { continue };
                if local == l && best.as_ref().is_none_or(|(b, _)| ms >= *b) {
                    best = Some((ms, e.path()));
                }
            }
        }
        let (_, p) = best.ok_or_else(|| KnowError::NotFound(format!("垃圾桶裡沒有 {id}")))?;
        let item = read_json(&p)?;
        Ok((p, item))
    }
}

impl Sink for Store {
    fn get(&self, id: &str) -> Result<Option<Item>> {
        let Some((c, l)) = split_id(id) else { return Ok(None) };
        let Some(cat) = self.registry.get(c).cloned() else { return Ok(None) };
        if check_local(l).is_err() {
            return Ok(None);
        }
        // a hub record: the category's own, or an outside source's in an app's category
        let p = self.file("items", id)?;
        if p.exists() {
            let mut i: Item = read_json(&p)?;
            i.id = id.into();
            i.category = cat.id;
            return Ok(Some(i));
        }
        if cat.native.kind.is_empty() {
            return Ok(None);
        }
        let Some(a) = self.adapter(&cat) else { return Ok(None) };
        let found = a.load()?.0.into_iter().find(|i| i.id == id);
        Ok(found.map(|i| Self::merge(&cat, i, self.read_meta(id))))
    }

    fn list(&self, category: &str) -> Result<Vec<Item>> {
        let cat = self.category(category)?;
        Ok(self.load_category(&cat)?.0)
    }

    fn put(&mut self, mut item: Item) -> Result<Item> {
        let cat = self.category(&item.category)?;
        if item.format.is_empty() {
            item.format = cat.format.clone();
        } else if item.format != cat.format {
            return Err(KnowError::Invalid(format!("「{}」收的是 {} 格式，不是 {}", cat.label, cat.format, item.format)));
        }
        item.tidy();
        let adapter_cat = !cat.native.kind.is_empty();
        let cur = if item.id.is_empty() {
            item.id = item_id(&cat.id, &new_local(ms_now()));
            None
        } else {
            match split_id(&item.id) {
                Some((c, l)) if c == cat.id => check_local(l)?,
                _ => return Err(KnowError::Invalid(format!("代號 {} 不屬於 {}", item.id, cat.id))),
            }
            self.get(&item.id)?
        };
        // Where it goes: an outside source's item is a hub record, never a row of the app's own
        // data; an id lives on one side only.
        let external = item.source.is_external();
        let to_record = if !adapter_cat {
            true
        } else {
            match &cur {
                Some(_) if self.is_record(&item.id) => {
                    if !external {
                        return Err(KnowError::Invalid(format!("{} 是外部來源放在中樞裡的一條，不能改成別的來源", item.id)));
                    }
                    true
                }
                Some(c) => {
                    if external && !c.source.is_external() {
                        return Err(KnowError::Invalid(format!("{} 已經是「{}」自己資料裡的一條，外部來源不能用同一個編號", item.id, cat.label)));
                    }
                    false
                }
                None => external,
            }
        };
        // A new title is the note's first heading too.
        if item.format == format::NOTE && cur.as_ref().is_some_and(|c| c.title != item.title) && !item.title.is_empty() {
            let body = crate::formats::text::retitle(item.field("body"), &item.title);
            item.set("body", body);
        }
        // A read-only native category still takes tags, triggers and `contains` (overlay only).
        let overlay_only = !cat.writable && adapter_cat && !to_record && cur.as_ref().is_some_and(|c| c.title == item.title && c.fields == item.fields);
        if !cat.writable && !overlay_only {
            return Err(KnowError::ReadOnly(cat.label.clone()));
        }
        if let Some(c) = cur.as_ref().filter(|c| c.revision != item.revision) {
            return Err(KnowError::Stale { have: c.revision, given: item.revision });
        }
        let stamp = now();
        item.revision = cur.as_ref().map_or(0, |c| c.revision) + 1;
        item.created_at = cur.as_ref().map(|c| c.created_at.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| stamp.clone());
        item.updated_at = stamp;
        // Usage is the store's, not the caller's.
        if let Some(c) = &cur {
            item.used = c.used;
            item.used_at = c.used_at.clone();
        }
        if to_record {
            write_json(&self.file("items", &item.id)?, &item)?;
            return Ok(item);
        }
        if overlay_only {
            let mut merged = cur.expect("overlay-only change has a current item");
            merged.tags = item.tags;
            merged.triggers = item.triggers;
            merged.contains = item.contains;
            merged.revision = item.revision;
            merged.updated_at = item.updated_at;
            item = merged;
        } else {
            let a = self.adapter(&cat).ok_or_else(|| KnowError::Other(format!("{} 還沒有接上讀取程式", cat.id)))?;
            let before: Vec<String> = a.load()?.0.into_iter().map(|i| i.id).collect();
            a.save(&item)?;
            // The app's data may name the item differently from the id given (a note's id comes
            // from its file name, a wording row's from its `avoid`): the id it reports now is the
            // stable one.
            let (after, _) = a.load()?;
            if !after.iter().any(|i| i.id == item.id) {
                let fresh: Vec<&Item> = after.iter().filter(|i| !before.contains(&i.id)).collect();
                let found = match fresh.len() {
                    1 => Some(fresh[0]),
                    _ => fresh.iter().copied().find(|i| i.title == item.title),
                };
                let found = found.ok_or_else(|| KnowError::Other(format!("「{}」存進去了，但是在 {} 裡找不到它", item.title, cat.label)))?;
                let old = item.id.clone();
                item.id = found.id.clone();
                // bookkeeping written under the old id would be orphaned
                if let Ok(p) = self.file("meta", &old)
                    && p.exists()
                {
                    let _ = fs::remove_file(p);
                }
            }
        }
        self.write_meta_for(&cat, &item)?;
        Ok(self.get(&item.id)?.unwrap_or(item))
    }

    fn remove(&mut self, id: &str) -> Result<Item> {
        let item = self.get(id)?.ok_or_else(|| KnowError::NotFound(id.into()))?;
        let cat = self.category(&item.category)?;
        if !cat.writable {
            return Err(KnowError::ReadOnly(cat.label));
        }
        let trashed = self.trash_put(&item)?;
        if cat.native.kind.is_empty() || self.is_record(id) {
            if let Err(e) = fs::remove_file(self.file("items", id)?) {
                let _ = fs::remove_file(&trashed);
                return Err(e.into());
            }
        } else {
            let a = self.adapter(&cat).ok_or_else(|| KnowError::Other(format!("{} 還沒有接上讀取程式", cat.id)))?;
            // the picture of a reference goes to the trash with it
            let mut kept: Option<PathBuf> = None;
            if let Some(pic) = a.picture_of(id) {
                let ext = pic.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "img".into());
                let dest = trashed.with_extension(ext);
                match fs::copy(&pic, &dest) {
                    Ok(_) => kept = Some(dest),
                    Err(e) => {
                        let _ = fs::remove_file(&trashed);
                        return Err(e.into());
                    }
                }
            }
            if let Err(e) = a.remove(id) {
                let _ = fs::remove_file(&trashed);
                if let Some(k) = kept {
                    let _ = fs::remove_file(k);
                }
                return Err(e);
            }
            let mp = self.file("meta", id)?;
            if mp.exists() {
                fs::remove_file(mp)?;
            }
        }
        Ok(item)
    }

    fn restore(&mut self, id: &str) -> Result<Item> {
        let (c, _) = split_id(id).ok_or_else(|| KnowError::Invalid(id.into()))?;
        let cat = self.category(c)?;
        if !cat.writable {
            return Err(KnowError::ReadOnly(cat.label));
        }
        if self.get(id)?.is_some() {
            return Err(KnowError::Invalid(format!("{id} 現在已經有一條了，不能再復原一條")));
        }
        let (path, item) = self.trash_latest(id)?;
        let picture = Self::trash_picture(&path);
        // an outside source's item goes back as a hub record, never into the app's data
        if cat.native.kind.is_empty() || item.source.is_external() {
            write_json(&self.file("items", id)?, &item)?;
        } else {
            let a = self.adapter(&cat).ok_or_else(|| KnowError::Other(format!("{} 還沒有接上讀取程式", cat.id)))?;
            a.save(&item)?;
            if let Some(pic) = &picture {
                a.restore_picture(id, item.field("image"), pic)?;
            }
            self.write_meta_for(&cat, &item)?;
        }
        fs::remove_file(path)?;
        if let Some(pic) = picture {
            let _ = fs::remove_file(pic);
        }
        Ok(self.get(id)?.unwrap_or(item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const MANIFEST: &str = r#"
[[category]]
id = "pc.patterns"
label = "句型"
format = "pattern"
writable = true
[category.native]
kind = "csv"
app_dir = "PaperComposer"
path = "patterns.csv"

[[category]]
id = "pc.layouts"
label = "版型"
format = "layout"
writable = false
[category.native]
kind = "dir"
app_dir = "SlideComposer"
path = "layouts"
"#;

    /// An adapter over memory that keeps only title and fields (like a CSV that has no tag column)
    /// unless `whole` is set.
    #[derive(Clone, Default)]
    struct Fake {
        rows: Arc<Mutex<Vec<Item>>>,
        whole: bool,
        skipped: Arc<Mutex<Vec<Problem>>>,
    }

    impl Adapter for Fake {
        fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
            Ok((self.rows.lock().unwrap().clone(), self.skipped.lock().unwrap().clone()))
        }
        fn save(&self, item: &Item) -> Result<()> {
            let mut row = item.clone();
            if !self.whole {
                row.tags.clear();
                row.triggers.clear();
                row.contains.clear();
                row.revision = 0;
                row.created_at.clear();
                row.updated_at.clear();
                row.used = 0;
                row.used_at.clear();
            }
            let mut rows = self.rows.lock().unwrap();
            match rows.iter_mut().find(|r| r.id == row.id) {
                Some(r) => *r = row,
                None => rows.push(row),
            }
            Ok(())
        }
        fn remove(&self, id: &str) -> Result<Item> {
            let mut rows = self.rows.lock().unwrap();
            let at = rows.iter().position(|r| r.id == id).ok_or_else(|| KnowError::NotFound(id.into()))?;
            Ok(rows.remove(at))
        }
    }

    fn open(d: &Path) -> Store {
        let mut r = Registry::load(&[MANIFEST], d).unwrap();
        r.add_manual("Notes", format::NOTE, "", "").unwrap();
        Store::open(d.to_path_buf(), r).unwrap()
    }

    fn note(title: &str) -> Item {
        let mut i = Item::new("my.notes", format::NOTE, title, Source::author());
        i.set("body", "hello");
        i
    }

    #[test]
    fn records_round_trip_with_ids_times_and_revisions() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let a = s.put(note("A")).unwrap();
        assert!(a.id.starts_with("my.notes:k") && a.revision == 1 && !a.created_at.is_empty() && !a.updated_at.is_empty());
        assert!(d.path().join("items/my.notes").join(format!("{}.json", split_id(&a.id).unwrap().1)).exists());
        let mut b = a.clone();
        b.title = "B".into();
        b.tag(facet::USE, "paper");
        let b = s.put(b).unwrap();
        assert_eq!((b.revision, b.created_at.clone()), (2, a.created_at.clone()));
        assert_eq!(s.get(&a.id).unwrap().unwrap(), b);
        // a second store over the same folder sees the same thing
        let s2 = open(d.path());
        assert_eq!(s2.list("my.notes").unwrap(), vec![b.clone()]);
        assert_eq!(s2.all().unwrap().len(), 1);
        // wrong category, wrong format, unknown category
        let mut bad = note("X");
        bad.format = format::RULE.into();
        assert!(matches!(s.put(bad), Err(KnowError::Invalid(_))));
        let mut bad = note("X");
        bad.category = "my.none".into();
        assert!(matches!(s.put(bad), Err(KnowError::NotFound(_))));
        let mut bad = note("X");
        bad.id = "my.notes:../evil".into();
        assert!(s.put(bad).is_err());
    }

    #[test]
    fn a_stale_write_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let a = s.put(note("A")).unwrap();
        let _b = s.put(a.clone()).unwrap();
        assert!(matches!(s.put(a), Err(KnowError::Stale { have: 2, given: 1 })));
        // adapter categories too
        let f = Fake::default();
        s.add_adapter("pc.patterns", Box::new(f));
        let p = s.put(Item::new("pc.patterns", format::PATTERN, "P", Source::lore("pc"))).unwrap();
        let _p2 = s.put(p.clone()).unwrap();
        assert!(matches!(s.put(p), Err(KnowError::Stale { have: 2, given: 1 })));
    }

    #[test]
    fn adapter_items_merge_with_their_overlay() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let f = Fake::default();
        let mut own = Item::new("pc.patterns", format::PATTERN, "Own", Source::lore("pc"));
        own.id = "pc.patterns:P1".into();
        own.tag(facet::DOMAIN, "ml");
        f.rows.lock().unwrap().push(own);
        s.add_adapter("pc.patterns", Box::new(f.clone()));
        let got = s.list("pc.patterns").unwrap();
        assert_eq!((got.len(), got[0].revision, got[0].created_at.as_str()), (1, 1, ""));
        // the hub adds tags, triggers and contains; the app's own tag stays
        let mut it = got[0].clone();
        it.tag(facet::USE, "paper").tag(facet::DOMAIN, "web");
        it.triggers.push("softmax".into());
        it.contains.push("pc.patterns:P2".into());
        it.title = "Own 2".into();
        let saved = s.put(it).unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.tags[facet::DOMAIN], ["ml", "web"]);
        assert_eq!(saved.tags[facet::USE], ["paper"]);
        // the adapter has the new title but (this one) no tags; the overlay file holds what it lacks
        assert_eq!(f.rows.lock().unwrap()[0].title, "Own 2");
        let meta: Meta = read_json(&d.path().join("meta/pc.patterns/P1.json")).unwrap();
        // this adapter drops tags on save, so the overlay keeps both
        assert_eq!(meta.overlay.tags[facet::DOMAIN], ["ml", "web"]);
        assert_eq!((meta.overlay.triggers.clone(), meta.revision), (vec!["softmax".to_string()], 2));
        let again = s.get("pc.patterns:P1").unwrap().unwrap();
        assert_eq!(again, saved);
        assert!(!d.path().join("items/pc.patterns").exists());
    }

    #[test]
    fn tags_on_a_read_only_category_go_to_the_overlay_only() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let f = Fake::default();
        let mut own = Item::new("pc.layouts", format::LAYOUT, "Grid", Source::lore("pc"));
        own.id = "pc.layouts:grid".into();
        own.set("cols", 3);
        f.rows.lock().unwrap().push(own.clone());
        s.add_adapter("pc.layouts", Box::new(f.clone()));
        let mut it = s.get("pc.layouts:grid").unwrap().unwrap();
        it.tag(facet::KIND, "grid");
        it.triggers.push("table".into());
        let saved = s.put(it.clone()).unwrap();
        assert_eq!((saved.revision, saved.tags[facet::KIND].clone(), saved.triggers.clone()), (2, vec!["grid".to_string()], vec!["table".to_string()]));
        assert_eq!(f.rows.lock().unwrap()[0], own, "the app's data is untouched");
        // a change of words is refused, a new item and a remove too
        let mut edit = saved.clone();
        edit.title = "Other".into();
        assert!(matches!(s.put(edit), Err(KnowError::ReadOnly(_))));
        assert!(matches!(s.put(Item::new("pc.layouts", format::LAYOUT, "New", Source::author())), Err(KnowError::ReadOnly(_))));
        assert!(matches!(s.remove("pc.layouts:grid"), Err(KnowError::ReadOnly(_))));
    }

    #[test]
    fn remove_goes_to_the_trash_and_restore_brings_it_back() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let mut a = s.put(note("A")).unwrap();
        a.tag(facet::USE, "book");
        let a = s.put(a).unwrap();
        let gone = s.remove(&a.id).unwrap();
        assert_eq!(gone, a);
        assert!(s.get(&a.id).unwrap().is_none() && s.list("my.notes").unwrap().is_empty());
        let local = split_id(&a.id).unwrap().1.to_string();
        let trashed: Vec<_> = fs::read_dir(d.path().join("trash/my.notes")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(trashed.len(), 1);
        assert!(trashed[0].starts_with(&format!("{local}.")) && trashed[0].ends_with(".json"));
        assert_eq!(s.restore(&a.id).unwrap(), a);
        assert_eq!(s.get(&a.id).unwrap().unwrap(), a);
        assert_eq!(fs::read_dir(d.path().join("trash/my.notes")).unwrap().count(), 0);
        assert!(matches!(s.restore(&a.id), Err(KnowError::Invalid(_))));
        assert!(matches!(s.remove("my.notes:nope"), Err(KnowError::NotFound(_))));

        // adapter category
        let f = Fake::default();
        s.add_adapter("pc.patterns", Box::new(f.clone()));
        let mut p = Item::new("pc.patterns", format::PATTERN, "P", Source::lore("pc"));
        p.tag(facet::USE, "slide");
        let p = s.put(p).unwrap();
        s.record_use(std::slice::from_ref(&p.id)).unwrap();
        let p = s.get(&p.id).unwrap().unwrap();
        let removed = s.remove(&p.id).unwrap();
        assert_eq!(removed, p);
        assert!(f.rows.lock().unwrap().is_empty() && s.get(&p.id).unwrap().is_none());
        let local = split_id(&p.id).unwrap().1;
        assert!(!d.path().join(format!("meta/pc.patterns/{local}.json")).exists());
        let back = s.restore(&p.id).unwrap();
        assert_eq!(back, p);
        assert_eq!((back.used, back.revision), (1, 1));
        assert_eq!(f.rows.lock().unwrap().len(), 1);
    }

    #[test]
    fn use_is_recorded_without_a_new_revision() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let a = s.put(note("A")).unwrap();
        s.add_adapter("pc.patterns", Box::new(Fake::default()));
        let p = s.put(Item::new("pc.patterns", format::PATTERN, "P", Source::lore("pc"))).unwrap();
        s.record_use(&[a.id.clone(), p.id.clone(), a.id.clone(), "my.notes:missing".into(), "junk".into()]).unwrap();
        let a2 = s.get(&a.id).unwrap().unwrap();
        let p2 = s.get(&p.id).unwrap().unwrap();
        assert_eq!((a2.used, a2.revision, a2.updated_at.clone()), (2, 1, a.updated_at.clone()));
        assert!(!a2.used_at.is_empty());
        assert_eq!((p2.used, p2.revision), (1, 1));
        // the item read before the uses can still be written; the count stays
        let a3 = s.put(a).unwrap();
        assert_eq!((a3.used, a3.revision), (2, 2));
    }

    #[test]
    fn unreadable_records_are_listed_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let a = s.put(note("A")).unwrap();
        fs::write(d.path().join("items/my.notes/broken.json"), "{ not json").unwrap();
        let list = s.list("my.notes").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, a.id);
        let p = s.problems("my.notes");
        assert_eq!((p.len(), p[0].at.as_str()), (1, "broken"));
        assert_eq!(s.all().unwrap().len(), 1);
        // adapter problems come through
        let f = Fake::default();
        f.skipped.lock().unwrap().push(Problem { at: "row 3".into(), field: "".into(), message: "壞列".into() });
        s.add_adapter("pc.patterns", Box::new(f));
        assert_eq!(s.problems("pc.patterns")[0].at, "row 3");
        // an adapter category with no adapter reads as empty with a problem
        assert!(s.list("pc.layouts").unwrap().is_empty());
        assert_eq!(s.problems("pc.layouts").len(), 1);
    }

    const REAL: &str = r#"
[[category]]
id = "pc.notes"
label = "筆記"
format = "note"
writable = true
[category.native]
kind = "files"
app_dir = "PaperComposer"
path = "notes"

[[category]]
id = "pc.wording"
label = "用詞"
format = "wording"
writable = true
[category.native]
kind = "csv"
app_dir = "PaperComposer"
path = "wording.csv"
"#;

    #[test]
    fn a_new_note_and_a_new_wording_row_come_back_with_their_stable_ids() {
        use crate::formats::table::CsvAdapter;
        use crate::formats::text::NoteFiles;
        let hub = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        let open_real = || {
            let mut s = Store::open(hub.path().to_path_buf(), Registry::load(&[REAL], hub.path()).unwrap()).unwrap();
            s.add_adapter("pc.notes", Box::new(NoteFiles::new(app.path().join("notes"), "pc.notes", Source::lore("pc"))));
            s.add_adapter("pc.wording", Box::new(CsvAdapter::new(app.path().join("wording.csv"), "pc.wording", format::WORDING, Source::lore("pc"))));
            s
        };
        let mut s = open_real();

        let mut n = Item::new("pc.notes", format::NOTE, "Plan A", Source::author());
        n.set("body", "hello");
        n.tag(facet::USE, "paper");
        let n = s.put(n).unwrap();
        assert_eq!(n.id, "pc.notes:Plan-A", "the id the file name gives, not the temporary one");
        assert_eq!((n.revision, n.source.clone(), n.tags[facet::USE].clone()), (1, Source::author(), vec!["paper".to_string()]));
        assert_eq!(s.get(&n.id).unwrap().unwrap(), n);
        // a fresh store over the same folders finds the same item with the same id
        let s2 = open_real();
        assert_eq!(s2.list("pc.notes").unwrap(), vec![n.clone()]);
        // no bookkeeping is left under a temporary id
        let metas: Vec<_> = fs::read_dir(hub.path().join("meta/pc.notes")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(metas, ["Plan-A.json"]);
        // the id it came back with can be written again
        let mut again = n.clone();
        again.set("body", "# Plan A\n\nhello again");
        assert_eq!(s.put(again).unwrap().revision, 2);
        // a second note with the same title is a second note
        let mut m = Item::new("pc.notes", format::NOTE, "Plan A", Source::author());
        m.set("body", "other");
        assert_eq!(s.put(m).unwrap().id, "pc.notes:Plan-A-2");

        let mut w = Item::new("pc.wording", format::WORDING, "utilize", Source::lore("pc"));
        w.set("avoid", "utilize").set("prefer", "use");
        w.tag(facet::KIND, "verb");
        let w = s.put(w).unwrap();
        assert_eq!(w.id, "pc.wording:utilize");
        assert_eq!((w.revision, w.field("prefer")), (1, "use"));
        assert_eq!(open_real().get(&w.id).unwrap().unwrap().id, w.id);
        // the same word twice is refused and the file stays as it was
        let csv = fs::read_to_string(app.path().join("wording.csv")).unwrap();
        let mut dup = Item::new("pc.wording", format::WORDING, "utilize", Source::lore("pc"));
        dup.set("avoid", "utilize").set("prefer", "employ");
        assert!(matches!(s.put(dup), Err(KnowError::Invalid(_))));
        assert_eq!(fs::read_to_string(app.path().join("wording.csv")).unwrap(), csv);
        // renaming `avoid` gives the row its new id, and the old bookkeeping goes
        let mut renamed = w.clone();
        renamed.set("avoid", "utilise");
        let renamed = s.put(renamed).unwrap();
        assert_eq!(renamed.id, "pc.wording:utilise");
        assert!(s.get("pc.wording:utilize").unwrap().is_none());
        assert!(!hub.path().join("meta/pc.wording/utilize.json").exists());
    }

    #[test]
    fn the_source_of_an_item_in_an_apps_data_is_kept() {
        use crate::formats::table::CsvAdapter;
        let hub = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        let mut s = Store::open(hub.path().to_path_buf(), Registry::load(&[REAL], hub.path()).unwrap()).unwrap();
        s.add_adapter("pc.wording", Box::new(CsvAdapter::new(app.path().join("wording.csv"), "pc.wording", format::WORDING, Source::lore("pc"))));
        let mut w = Item::new("pc.wording", format::WORDING, "utilize", Source::external("acme"));
        w.set("avoid", "utilize").set("prefer", "use");
        let w = s.put(w).unwrap();
        assert_eq!(w.source, Source::external("acme"));
        assert_eq!(s.list("pc.wording").unwrap()[0].source, Source::external("acme"));
    }

    const PATTERNS: &str = r#"
[[category]]
id = "pc.patterns"
label = "句型"
format = "pattern"
writable = true
[category.native]
kind = "csv"
app_dir = "PaperComposer"
path = "phrases.csv"
"#;

    fn pattern(template: &str, source: Source) -> Item {
        let mut p = Item::new("pc.patterns", format::PATTERN, template, source);
        p.set("template", template);
        p
    }

    fn patterns_store(hub: &Path, csv: &Path) -> Store {
        use crate::formats::table::CsvAdapter;
        let mut s = Store::open(hub.to_path_buf(), Registry::load(&[PATTERNS], hub).unwrap()).unwrap();
        s.add_adapter("pc.patterns", Box::new(CsvAdapter::new(csv, "pc.patterns", format::PATTERN, Source::lore("pc"))));
        s
    }

    #[test]
    fn an_outside_item_in_an_apps_category_is_a_hub_record_not_a_row() {
        let (hub, app) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let csv = app.path().join("phrases.csv");
        let mut s = patterns_store(hub.path(), &csv);
        s.put(pattern("Little is known about X.", Source::lore("pc"))).unwrap();
        let before = fs::read(&csv).unwrap();
        let mut o = pattern("We propose X.", Source::external("acme"));
        o.id = "pc.patterns:acme.1".into();
        let o = s.put(o).unwrap();
        assert_eq!(o.id, "pc.patterns:acme.1");
        assert_eq!(fs::read(&csv).unwrap(), before, "the app's own file is byte-identical");
        assert!(hub.path().join("items/pc.patterns/acme.1.json").is_file());
        assert!(!hub.path().join("meta/pc.patterns/acme.1.json").exists());
        // a new outside item without an id is a record too
        let n = s.put(pattern("Other.", Source::external("acme"))).unwrap();
        assert!(hub.path().join("items/pc.patterns").join(format!("{}.json", split_id(&n.id).unwrap().1)).is_file());
        assert_eq!(fs::read(&csv).unwrap(), before);
        // it reads back, changes, is used, goes to the trash and comes back: all as a record
        let mut c = s.get(&o.id).unwrap().unwrap();
        c.set("example", "x");
        let c = s.put(c).unwrap();
        assert_eq!(c.revision, 2);
        s.record_use(std::slice::from_ref(&o.id)).unwrap();
        assert_eq!(s.get(&o.id).unwrap().unwrap().used, 1);
        s.remove(&o.id).unwrap();
        assert!(s.get(&o.id).unwrap().is_none());
        assert!(!hub.path().join("items/pc.patterns/acme.1.json").exists());
        s.restore(&o.id).unwrap();
        assert!(hub.path().join("items/pc.patterns/acme.1.json").is_file());
        assert_eq!(fs::read(&csv).unwrap(), before, "still untouched");
        // an id the app's own data has is refused, and the other way round
        let own = s.list("pc.patterns").unwrap().into_iter().find(|i| i.source == Source::lore("pc")).unwrap();
        let mut clash = pattern("Clash", Source::external("acme"));
        clash.id = own.id.clone();
        assert!(matches!(s.put(clash), Err(KnowError::Invalid(_))));
        let mut back = s.get(&o.id).unwrap().unwrap();
        back.source = Source::author();
        assert!(matches!(s.put(back), Err(KnowError::Invalid(_))));
        assert_eq!(fs::read(&csv).unwrap(), before);
    }

    #[test]
    fn the_category_lists_both() {
        let (hub, app) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut s = patterns_store(hub.path(), &app.path().join("phrases.csv"));
        s.put(pattern("Own one.", Source::lore("pc"))).unwrap();
        s.put(pattern("Outside one.", Source::external("acme"))).unwrap();
        let all = s.list("pc.patterns").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().filter(|i| i.source.is_external()).count(), 1);
        assert_eq!(s.all().unwrap().len(), 2);
        let outside = all.iter().find(|i| i.source.is_external()).unwrap();
        assert_eq!(s.get(&outside.id).unwrap().unwrap().title, "Outside one.");
        // an unreadable record is a problem, not an error
        fs::write(hub.path().join("items/pc.patterns/broken.json"), "{ no").unwrap();
        assert_eq!(s.list("pc.patterns").unwrap().len(), 2);
        assert_eq!(s.problems("pc.patterns").len(), 1);
    }

    #[test]
    fn adding_again_replaces_only_the_outside_sources_records() {
        use crate::intake::{self, Batch, Entry, Target};
        let (hub, app) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let csv = app.path().join("phrases.csv");
        let mut s = patterns_store(hub.path(), &csv);
        s.put(pattern("Own one.", Source::lore("pc"))).unwrap();
        let before = fs::read(&csv).unwrap();
        let e = |id: &str, t: &str| {
            let mut e = Entry { id: id.into(), title: t.into(), ..Default::default() };
            e.fields.insert("template".into(), t.into());
            e
        };
        let batch = |source: &str, items: Vec<Entry>| Batch { lore_know: 1, source: source.into(), target: Target::Align { category: "pc.patterns".into() }, items };
        let mut cats = s.registry().clone();
        let f = crate::formats::all();
        intake::import(&mut s, &mut cats, &f, &batch("acme", vec![e("1", "A one."), e("2", "A two.")])).unwrap();
        intake::import(&mut s, &mut cats, &f, &batch("beta", vec![e("1", "B one.")])).unwrap();
        let r = intake::import(&mut s, &mut cats, &f, &batch("acme", vec![e("1", "A one changed.")])).unwrap();
        assert_eq!((r.added, r.replaced, r.removed), (0, 1, 1), "{r:?}");
        let titles = |s: &Store| {
            let mut t: Vec<String> = s.list("pc.patterns").unwrap().into_iter().map(|i| i.title).collect();
            t.sort();
            t
        };
        assert_eq!(titles(&s), ["A one changed.", "B one.", "Own one."]);
        assert_eq!(fs::read(&csv).unwrap(), before);
    }

    #[test]
    fn a_new_title_rewrites_the_heading() {
        let d = tempfile::tempdir().unwrap();
        let mut s = open(d.path());
        let mut n = Item::new("my.notes", format::NOTE, "Old", Source::author());
        n.set("body", "# Old\n\ntext\n\n## Sub\n");
        let n = s.put(n).unwrap();
        let mut c = n.clone();
        c.title = "新的名稱".into();
        let c = s.put(c).unwrap();
        assert_eq!(c.field("body"), "# 新的名稱\n\ntext\n\n## Sub\n");
        assert_eq!(s.get(&c.id).unwrap().unwrap().field("body"), c.field("body"));
        // no heading: one is added on top; a heading inside a code fence is not the one
        let mut m = Item::new("my.notes", format::NOTE, "Plain", Source::author());
        m.set("body", "```\n# not me\n```\nline");
        let m = s.put(m).unwrap();
        let mut c = m.clone();
        c.title = "Named".into();
        let c = s.put(c).unwrap();
        assert_eq!(c.field("body"), "# Named\n\n```\n# not me\n```\nline");
        // a write that keeps the title leaves the body alone
        let mut same = c.clone();
        same.set("body", "# Named\n\nedited");
        assert_eq!(s.put(same).unwrap().field("body"), "# Named\n\nedited");
    }

    #[test]
    fn a_removed_reference_comes_back_with_its_picture() {
        use crate::formats::figure::ReferenceDirs;
        let (hub, app) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        const M: &str = "[[category]]\nid = \"sc.flowrefs\"\nlabel = \"參考圖\"\nformat = \"figure\"\nwritable = true\n[category.native]\nkind = \"dir\"\napp_dir = \"SlideComposer\"\npath = \"refs\"\n";
        let folder = app.path().join("refs/f01");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("desc.md"), "# f01 Overview\n\nTwo rows.\n").unwrap();
        fs::write(folder.join("ref.png"), b"\x89PNG-bytes").unwrap();
        let mut s = Store::open(hub.path().to_path_buf(), Registry::load(&[M], hub.path()).unwrap()).unwrap();
        s.add_adapter("sc.flowrefs", Box::new(ReferenceDirs { dir: app.path().join("refs"), category: "sc.flowrefs".into(), source: Source::lore("sc") }));
        s.remove("sc.flowrefs:f01").unwrap();
        assert!(!folder.exists());
        let kept: Vec<String> = fs::read_dir(hub.path().join("trash/sc.flowrefs")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(kept.iter().any(|n| n.starts_with("f01.") && n.ends_with(".png")) && kept.iter().any(|n| n.ends_with(".json")));
        let back = s.restore("sc.flowrefs:f01").unwrap();
        assert_eq!(back.title, "Overview");
        assert_eq!(fs::read(folder.join("ref.png")).unwrap(), b"\x89PNG-bytes");
        assert_eq!(s.get("sc.flowrefs:f01").unwrap().unwrap().field("image"), "ref.png");
        assert_eq!(fs::read_dir(hub.path().join("trash/sc.flowrefs")).unwrap().count(), 0, "nothing left in the trash");
    }
}
