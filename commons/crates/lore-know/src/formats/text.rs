//! K3: text-shaped formats — note, bundle (a whole folder), layout (one arrangement) — and the
//! adapters that read an app's existing files as items of those formats (docs/brain-hub.md §4, §12).
//!
//! Paths inside an [`Item`] are never absolute: file names are relative to the adapter's `dir`.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::types::{
    format, item_id, safe_local, split_id, Adapter, FieldDef, Format, FormatDef, Item, KnowError, Problem, Result, Source,
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// `s` on one line, cut to at most `n` characters (the last one `…` when something was cut).
fn clip(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n {
        return one;
    }
    let mut t: String = one.chars().take(n.saturating_sub(1)).collect();
    t.push('…');
    t
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Write beside the target and rename, so a reader never sees half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.tmp{}", std::process::id()));
    let r = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(r?)
}

/// Directory entries sorted by name; a missing directory is an empty one (the app has not made it yet).
fn entries(dir: &Path) -> Result<Vec<fs::DirEntry>> {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut v: Vec<fs::DirEntry> = rd.filter_map(|e| e.ok()).collect();
    v.sort_by_key(|e| e.file_name());
    Ok(v)
}

fn stem_of(name: &str) -> &str {
    Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name)
}

/// The first non-empty string under one of `keys`.
fn str_of<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())).unwrap_or("")
}

fn is_file(e: &fs::DirEntry) -> bool {
    e.file_type().map(|t| t.is_file()).unwrap_or(false)
}

// ── formats ──────────────────────────────────────────────────────────────────

/// A markdown note.
pub struct Note;
/// A whole folder opened in its app (a journal's template, a slide style pack).
pub struct Bundle;
/// One arrangement of a slide or a figure.
pub struct Layout;

impl Format for Note {
    fn def(&self) -> FormatDef {
        FormatDef {
            id: format::NOTE.into(),
            label: "筆記".into(),
            fields: vec![FieldDef::new("body", "內容", "markdown", true), FieldDef::new("file", "檔名", "text", false)],
            shape: "text".into(),
            form: true,
        }
    }
    fn line(&self, item: &Item) -> String {
        let first = item.field("body").lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("");
        clip(first, 80)
    }
    fn prompt(&self, item: &Item) -> String {
        format!("## {}\n{}", item.title, item.field("body").trim())
    }
}

impl Format for Bundle {
    fn def(&self) -> FormatDef {
        FormatDef {
            id: format::BUNDLE.into(),
            label: "模板".into(),
            fields: vec![
                FieldDef::new("about", "說明", "text", false),
                FieldDef::new("files", "檔案", "list", false),
                FieldDef::new("for", "用途", "text", false),
            ],
            shape: "bundle".into(),
            form: false,
        }
    }
    fn line(&self, item: &Item) -> String {
        let n = item.fields.get("files").and_then(Value::as_array).map_or(0, Vec::len);
        let what = match item.field("for") {
            "" => item.field("about"),
            f => f,
        };
        let what = clip(what, 60);
        if what.is_empty() {
            format!("{n} 個檔案")
        } else {
            format!("{what} · {n} 個檔案")
        }
    }
    fn prompt(&self, item: &Item) -> String {
        format!("{}: {}", item.title, item.field("about"))
    }
}

impl Format for Layout {
    fn def(&self) -> FormatDef {
        FormatDef {
            id: format::LAYOUT.into(),
            label: "版型".into(),
            fields: vec![
                FieldDef::new("group", "分類", "text", false),
                FieldDef::new("description", "說明", "text", false),
                FieldDef::new("data", "資料", "text", false),
            ],
            shape: "figure".into(),
            form: false,
        }
    }
    fn line(&self, item: &Item) -> String {
        let (g, d) = (item.field("group"), item.field("description"));
        clip(&if g.is_empty() { d.to_string() } else if d.is_empty() { g.to_string() } else { format!("{g} · {d}") }, 80)
    }
    fn prompt(&self, item: &Item) -> String {
        let (g, d) = (item.field("group"), item.field("description"));
        let mut s = item.title.clone();
        if !g.is_empty() {
            s.push_str(&format!(" ({g})"));
        }
        if !d.is_empty() {
            s.push_str(&format!(": {d}"));
        }
        s
    }
}

/// Every format of this module.
pub fn all() -> Vec<Box<dyn Format>> {
    vec![Box::new(Note), Box::new(Bundle), Box::new(Layout)]
}

// ── notes ────────────────────────────────────────────────────────────────────

/// One note per text file of `dir` (not recursive).
#[derive(Debug, Clone)]
pub struct NoteFiles {
    pub dir: PathBuf,
    pub category: String,
    pub source: Source,
    /// Extensions read, without the dot (default `["md"]`).
    pub ext: Vec<String>,
    /// File names starting with one of these are not notes (e.g. `_`).
    pub skip_prefix: Vec<String>,
}

impl NoteFiles {
    /// Reads `*.md` files, skipping none by prefix.
    pub fn new(dir: impl Into<PathBuf>, category: &str, source: Source) -> NoteFiles {
        NoteFiles { dir: dir.into(), category: category.into(), source, ext: vec!["md".into()], skip_prefix: vec![] }
    }

    /// The note files of `dir`: (file name, local id), sorted by file name.
    fn files(&self) -> Result<Vec<(String, String)>> {
        let mut out = vec![];
        for e in entries(&self.dir)? {
            let name = e.file_name().to_string_lossy().to_string();
            let ext = Path::new(&name).extension().and_then(|x| x.to_str()).unwrap_or("");
            if !is_file(&e)
                || is_hidden(&name)
                || !self.ext.iter().any(|x| x.eq_ignore_ascii_case(ext))
                || self.skip_prefix.iter().any(|p| !p.is_empty() && name.starts_with(p.as_str()))
            {
                continue;
            }
            let local = safe_local(stem_of(&name));
            out.push((name, local));
        }
        Ok(out)
    }

    fn read(&self, name: &str, local: &str) -> Result<Item> {
        let text = fs::read_to_string(self.dir.join(name))?;
        let title = heading_of(&text).unwrap_or_else(|| stem_of(name).to_string());
        let mut it = Item::new(&self.category, format::NOTE, &title, self.source.clone());
        it.id = item_id(&self.category, local);
        it.set("body", text).set("file", name);
        Ok(it)
    }

    fn file_of(&self, id: &str) -> Result<Option<String>> {
        let Some((cat, local)) = split_id(id) else { return Ok(None) };
        if cat != self.category {
            return Ok(None);
        }
        Ok(self.files()?.into_iter().find(|(_, l)| l == local).map(|(n, _)| n))
    }

    /// A file name for a new note that no existing file (any extension, any case) already uses.
    fn free_name(&self, wanted: &str) -> Result<String> {
        let taken: HashSet<String> = entries(&self.dir)?.iter().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
        let ext = self.ext.first().map(String::as_str).unwrap_or("md");
        let stem = safe_local(wanted);
        let mut n = 1;
        loop {
            let s = if n == 1 { stem.clone() } else { format!("{stem}-{n}") };
            let low = s.to_lowercase();
            if !taken.iter().any(|t| t == &low || stem_of(t) == low) {
                return Ok(format!("{s}.{ext}"));
            }
            n += 1;
        }
    }
}

/// The first `# heading` of a markdown text (code fences skipped).
fn heading_of(text: &str) -> Option<String> {
    let mut fence = false;
    for t in text.lines().map(str::trim) {
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
        } else if let Some(h) = t.strip_prefix("# ").map(str::trim).filter(|h| !fence && !h.is_empty()) {
            return Some(h.to_string());
        }
    }
    None
}

impl Adapter for NoteFiles {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let (mut items, mut problems) = (vec![], vec![]);
        let mut seen = HashSet::new();
        for (name, local) in self.files()? {
            if !seen.insert(local.clone()) {
                problems.push(Problem { at: name, field: "file".into(), message: "檔名轉成編號後和別的檔案相同，略過".into() });
                continue;
            }
            match self.read(&name, &local) {
                Ok(i) => items.push(i),
                Err(e) => problems.push(Problem { at: name, field: "file".into(), message: format!("讀不了：{e}") }),
            }
        }
        Ok((items, problems))
    }

    fn save(&self, item: &Item) -> Result<()> {
        let body = item.field("body");
        if body.trim().is_empty() {
            return Err(KnowError::Invalid("筆記沒有內容".into()));
        }
        fs::create_dir_all(&self.dir)?;
        if let Some(name) = self.file_of(&item.id)? {
            return write_atomic(&self.dir.join(name), body.as_bytes());
        }
        // A new note: its own free file named from the title. The title is written as a heading
        // when the body has none, so the note reads back with the same title.
        let name = self.free_name(&item.title)?;
        let text = if heading_of(body).is_some() || item.title.trim().is_empty() {
            body.to_string()
        } else {
            format!("# {}\n\n{}", item.title.trim(), body)
        };
        write_atomic(&self.dir.join(name), text.as_bytes())
    }

    fn remove(&self, id: &str) -> Result<Item> {
        let name = self.file_of(id)?.ok_or_else(|| KnowError::NotFound(id.into()))?;
        let (_, local) = split_id(id).ok_or_else(|| KnowError::NotFound(id.into()))?;
        let item = self.read(&name, local)?;
        fs::remove_file(self.dir.join(&name))?;
        Ok(item)
    }
}

// ── bundles ──────────────────────────────────────────────────────────────────

/// One bundle per sub-folder of `dir`.
#[derive(Debug, Clone)]
pub struct BundleDirs {
    pub dir: PathBuf,
    pub category: String,
    pub source: Source,
}

const MANIFESTS: [&str; 3] = ["template.toml", "style.toml", "pack.toml"];
const MAX_FILES: usize = 200;

/// Relative file names under `root` (`/` separators), sorted, hidden entries skipped, at most [`MAX_FILES`].
fn bundle_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.filter_map(|e| e.ok()) {
            if is_hidden(&e.file_name().to_string_lossy()) {
                continue;
            }
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                walk(root, &e.path(), out);
            } else if let (true, Ok(rel)) = (t.is_file(), e.path().strip_prefix(root)) {
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out.sort();
    out.truncate(MAX_FILES);
    out
}

/// The first paragraph of a README (headings skipped).
fn readme_about(text: &str) -> String {
    let mut para: Vec<&str> = vec![];
    for l in text.lines().map(str::trim) {
        if l.is_empty() || l.starts_with('#') {
            if !para.is_empty() {
                break;
            }
        } else {
            para.push(l);
        }
    }
    para.join(" ")
}

/// A string at the top of the table or inside its `[pack]` / `[template]` / `[style]` table.
fn toml_str(t: &toml::Table, keys: &[&str]) -> String {
    let look = |tab: &toml::Table| {
        keys.iter().find_map(|k| tab.get(*k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from))
    };
    look(t)
        .or_else(|| ["pack", "template", "style"].iter().find_map(|s| t.get(*s).and_then(|v| v.as_table()).and_then(look)))
        .unwrap_or_default()
}

impl Adapter for BundleDirs {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let (mut items, mut problems) = (vec![], vec![]);
        let mut seen = HashSet::new();
        for e in entries(&self.dir)? {
            let name = e.file_name().to_string_lossy().to_string();
            if is_hidden(&name) || !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let local = safe_local(&name);
            if !seen.insert(local.clone()) {
                problems.push(Problem { at: name, field: "folder".into(), message: "資料夾名稱轉成編號後和別的相同，略過".into() });
                continue;
            }
            let path = e.path();
            // The first manifest that parses; an unreadable one is the same as none.
            let t = MANIFESTS
                .iter()
                .find_map(|m| fs::read_to_string(path.join(m)).ok().and_then(|s| s.parse::<toml::Table>().ok()))
                .unwrap_or_default();
            let title = Some(toml_str(&t, &["name", "title"])).filter(|s| !s.is_empty()).unwrap_or_else(|| name.clone());
            let about = Some(toml_str(&t, &["description", "about", "notes"]))
                .filter(|s| !s.is_empty())
                .or_else(|| fs::read_to_string(path.join("README.md")).ok().map(|r| readme_about(&r)))
                .unwrap_or_default();
            let mut it = Item::new(&self.category, format::BUNDLE, &title, self.source.clone());
            it.id = item_id(&self.category, &local);
            it.set("about", about).set("for", toml_str(&t, &["journal", "venue", "for"]));
            it.set("files", Value::Array(bundle_files(&path).into_iter().map(Value::from).collect()));
            items.push(it);
        }
        Ok((items, problems))
    }
    fn save(&self, _: &Item) -> Result<()> {
        Err(KnowError::ReadOnly("整包的資料夾在它自己的程式裡改".into()))
    }
    fn remove(&self, _: &str) -> Result<Item> {
        Err(KnowError::ReadOnly("整包的資料夾在它自己的程式裡移除".into()))
    }
}

// ── layouts ──────────────────────────────────────────────────────────────────

/// One layout per `*.json` file of `dir`.
#[derive(Debug, Clone)]
pub struct LayoutFiles {
    pub dir: PathBuf,
    pub category: String,
    pub source: Source,
}

/// One layout per entry of a list in one JSON file (a list, or an object with the list under
/// `layouts` or `recipes`).
#[derive(Debug, Clone)]
pub struct LayoutList {
    pub file: PathBuf,
    pub category: String,
    pub source: Source,
}

fn layout_item(category: &str, source: &Source, local: &str, fallback_title: &str, data: &Value) -> Item {
    let title = Some(str_of(data, &["name", "title"])).filter(|s| !s.is_empty()).unwrap_or(fallback_title);
    let mut it = Item::new(category, format::LAYOUT, title, source.clone());
    it.id = item_id(category, local);
    it.set("group", str_of(data, &["group", "class"])).set("description", str_of(data, &["description"]));
    it.set("data", data.clone());
    it
}

fn layouts_read_only() -> KnowError {
    KnowError::ReadOnly("版型在它自己的程式裡改".into())
}

impl Adapter for LayoutFiles {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let (mut items, mut problems) = (vec![], vec![]);
        let mut seen = HashSet::new();
        for e in entries(&self.dir)? {
            let name = e.file_name().to_string_lossy().to_string();
            let json = Path::new(&name).extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("json"));
            if is_hidden(&name) || !json || !is_file(&e) {
                continue;
            }
            let local = safe_local(stem_of(&name));
            if !seen.insert(local.clone()) {
                problems.push(Problem { at: name, field: "file".into(), message: "檔名轉成編號後和別的檔案相同，略過".into() });
                continue;
            }
            let read = fs::read_to_string(e.path()).map_err(|x| x.to_string()).and_then(|s| serde_json::from_str::<Value>(&s).map_err(|x| x.to_string()));
            match read {
                Ok(v) => items.push(layout_item(&self.category, &self.source, &local, stem_of(&name), &v)),
                Err(x) => problems.push(Problem { at: name, field: "file".into(), message: format!("讀不了：{x}") }),
            }
        }
        Ok((items, problems))
    }
    fn save(&self, _: &Item) -> Result<()> {
        Err(layouts_read_only())
    }
    fn remove(&self, _: &str) -> Result<Item> {
        Err(layouts_read_only())
    }
}

impl Adapter for LayoutList {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let text = match fs::read_to_string(&self.file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((vec![], vec![])),
            Err(e) => return Err(e.into()),
        };
        let root: Value = serde_json::from_str(&text).map_err(|e| KnowError::Invalid(format!("版型清單讀不了：{e}")))?;
        let list: &Value = match &root {
            Value::Array(_) => &root,
            Value::Object(o) => ["layouts", "recipes"]
                .iter()
                .find_map(|k| o.get(*k).filter(|v| v.is_array()))
                .ok_or_else(|| KnowError::Invalid("版型清單裡找不到 layouts 或 recipes 的列表".into()))?,
            _ => return Err(KnowError::Invalid("版型清單要是列表".into())),
        };
        let (mut items, mut problems) = (vec![], vec![]);
        let mut seen = HashSet::new();
        for (n, entry) in list.as_array().into_iter().flatten().enumerate() {
            let id = match entry.get("id") {
                Some(Value::String(s)) => s.trim().to_string(),
                Some(Value::Number(x)) => x.to_string(),
                _ => String::new(),
            };
            if id.is_empty() {
                let at = match str_of(entry, &["name", "title"]) {
                    "" => format!("#{}", n + 1),
                    s => format!("#{} {s}", n + 1),
                };
                problems.push(Problem { at, field: "id".into(), message: "沒有 id".into() });
                continue;
            }
            let local = safe_local(&id);
            if !seen.insert(local.clone()) {
                problems.push(Problem { at: id, field: "id".into(), message: "id 重複，略過後面的".into() });
                continue;
            }
            items.push(layout_item(&self.category, &self.source, &local, &id, entry));
        }
        Ok((items, problems))
    }
    fn save(&self, _: &Item) -> Result<()> {
        Err(layouts_read_only())
    }
    fn remove(&self, _: &str) -> Result<Item> {
        Err(layouts_read_only())
    }
}

// ── previews ─────────────────────────────────────────────────────────────────

/// What a detail pane shows for a text-shaped item, at most about `max` characters: a note's body
/// cut at a paragraph end, a bundle's `about` and its file list (one per line), a layout's description.
pub fn preview_text(item: &Item, max: usize) -> String {
    match item.format.as_str() {
        format::NOTE => {
            let mut out = String::new();
            for p in item.field("body").trim().split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
                let len = p.chars().count() + if out.is_empty() { 0 } else { out.chars().count() + 2 };
                if len > max {
                    if out.is_empty() {
                        out = p.chars().take(max.saturating_sub(1)).collect::<String>() + "…";
                    } else {
                        out.push_str("\n\n…");
                    }
                    return out;
                }
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(p);
            }
            out
        }
        format::BUNDLE => {
            let mut out = item.field("about").trim().to_string();
            let files = item.fields.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
            for (i, f) in files.iter().filter_map(Value::as_str).enumerate() {
                if i > 0 && out.chars().count() + f.chars().count() + 1 > max {
                    out.push_str("\n…");
                    break;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(f);
            }
            out
        }
        format::LAYOUT => clip(item.field("description"), max),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn put(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn notes(dir: &Path) -> NoteFiles {
        NoteFiles::new(dir, "my.notes", Source::author())
    }

    #[test]
    fn formats_declare_their_fields() {
        let all = all();
        let d: Vec<FormatDef> = all.iter().map(|f| f.def()).collect();
        let ids: Vec<_> = d.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["note", "bundle", "layout"]);
        assert_eq!((d[0].label.as_str(), d[0].shape.as_str(), d[0].form), ("筆記", "text", true));
        assert_eq!((d[1].label.as_str(), d[1].shape.as_str(), d[1].form), ("模板", "bundle", false));
        assert_eq!((d[2].label.as_str(), d[2].shape.as_str(), d[2].form), ("版型", "figure", false));
        let names = |i: usize| d[i].fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names(0), ["body", "file"]);
        assert_eq!(names(1), ["about", "files", "for"]);
        assert_eq!(names(2), ["group", "description", "data"]);
        assert!(d[0].fields[0].required && !d[0].fields[1].required);
        assert_eq!(d[0].fields[0].kind, "markdown");
        // The default validation: a note without a body is refused, a bundle needs nothing.
        assert_eq!(Note.validate(&Item::new("my.notes", format::NOTE, "A", Source::author())).len(), 1);
        assert!(Bundle.validate(&Item::new("c", format::BUNDLE, "A", Source::author())).is_empty());
    }

    #[test]
    fn lines_and_prompts_read_well() {
        let mut n = Item::new("my.notes", format::NOTE, "Warm-up", Source::author());
        n.set("body", "# Warm-up\n\n\nStart with five minutes of easy spinning.\nThen stretch.");
        assert_eq!(Note.line(&n), "Start with five minutes of easy spinning.");
        assert_eq!(Note.prompt(&n), "## Warm-up\n# Warm-up\n\n\nStart with five minutes of easy spinning.\nThen stretch.");
        n.set("body", "x".repeat(100));
        let l = Note.line(&n);
        assert_eq!(l.chars().count(), 80);
        assert!(l.ends_with('…'));

        let mut b = Item::new("pc.templates", format::BUNDLE, "Review", Source::lore("pc"));
        b.set("about", "A two column review.").set("files", json!(["a.tex", "b.bib"]));
        assert_eq!(Bundle.line(&b), "A two column review. · 2 個檔案");
        b.set("for", "Journal of Examples");
        assert_eq!(Bundle.line(&b), "Journal of Examples · 2 個檔案");
        assert_eq!(Bundle.prompt(&b), "Review: A two column review.");

        let mut y = Item::new("sc.layouts", format::LAYOUT, "Title only", Source::lore("sc"));
        y.set("group", "bisect").set("description", "Half and half.");
        assert_eq!(Layout.line(&y), "bisect · Half and half.");
        assert_eq!(Layout.prompt(&y), "Title only (bisect): Half and half.");
    }

    #[test]
    fn notes_load_with_titles_from_headings() {
        let t = tempdir().unwrap();
        put(t.path(), "alpha.md", "# First Note\n\nbody a\n");
        put(t.path(), "beta.md", "no heading here\n");
        put(t.path(), "_hidden.md", "# skipped\n");
        put(t.path(), ".dot.md", "# skipped\n");
        put(t.path(), "other.txt", "not markdown\n");
        put(t.path(), "sub/deep.md", "# not recursive\n");
        let mut a = notes(t.path());
        a.skip_prefix = vec!["_".into()];
        let (items, problems) = a.load().unwrap();
        assert!(problems.is_empty());
        let got: Vec<_> = items.iter().map(|i| (i.id.as_str(), i.title.as_str(), i.field("file"))).collect();
        assert_eq!(got, [("my.notes:alpha", "First Note", "alpha.md"), ("my.notes:beta", "beta", "beta.md")]);
        assert_eq!(items[0].field("body"), "# First Note\n\nbody a\n");
        assert_eq!(items[0].source, Source::author());
        // A heading inside a code fence is not the title.
        put(t.path(), "gamma.md", "```\n# not me\n```\n# Me\n");
        let (items, _) = notes(t.path()).load().unwrap();
        assert_eq!(items.iter().find(|i| i.id.ends_with(":gamma")).unwrap().title, "Me");
    }

    #[test]
    fn saving_a_note_rewrites_only_its_file() {
        let t = tempdir().unwrap();
        put(t.path(), "alpha.md", "# A\n\none\n");
        put(t.path(), "beta.md", "# B\n\ntwo\n");
        let a = notes(t.path());
        let (items, _) = a.load().unwrap();
        let mut x = items[0].clone();
        x.title = "Renamed".into();
        x.set("body", "# Renamed\n\nchanged\n");
        a.save(&x).unwrap();
        assert_eq!(fs::read_to_string(t.path().join("alpha.md")).unwrap(), "# Renamed\n\nchanged\n");
        assert_eq!(fs::read_to_string(t.path().join("beta.md")).unwrap(), "# B\n\ntwo\n");
        let mut names: Vec<_> = fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, ["alpha.md", "beta.md"], "no rename, no temp file left");
        let (again, _) = a.load().unwrap();
        assert_eq!((again[0].id.as_str(), again[0].title.as_str()), ("my.notes:alpha", "Renamed"));
    }

    #[test]
    fn a_new_note_gets_a_free_file_name() {
        let t = tempdir().unwrap();
        put(t.path(), "My-Idea.md", "# Old\n\nkeep me\n");
        let a = notes(t.path());
        let mut n = Item::new("my.notes", format::NOTE, "My Idea", Source::author());
        n.set("body", "fresh");
        a.save(&n).unwrap();
        a.save(&n).unwrap();
        assert_eq!(fs::read_to_string(t.path().join("My-Idea.md")).unwrap(), "# Old\n\nkeep me\n");
        assert_eq!(fs::read_to_string(t.path().join("My-Idea-2.md")).unwrap(), "# My Idea\n\nfresh");
        assert!(t.path().join("My-Idea-3.md").exists());
        let (items, _) = a.load().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items.iter().filter(|i| i.title == "My Idea").count(), 2);
    }

    #[test]
    fn removing_a_note_returns_it() {
        let t = tempdir().unwrap();
        put(t.path(), "alpha.md", "# A\n\none\n");
        put(t.path(), "beta.md", "# B\n");
        let a = notes(t.path());
        let gone = a.remove("my.notes:alpha").unwrap();
        assert_eq!((gone.title.as_str(), gone.field("body")), ("A", "# A\n\none\n"));
        assert!(!t.path().join("alpha.md").exists() && t.path().join("beta.md").exists());
        assert!(matches!(a.remove("my.notes:alpha"), Err(KnowError::NotFound(_))));
        assert!(matches!(a.remove("other.cat:beta"), Err(KnowError::NotFound(_))));
        assert!(t.path().join("beta.md").exists());
    }

    fn bundle_tree(root: &Path) {
        put(root, "review/template.toml", "name = \"Review paper\"\njournal = \"Journal of Examples\"\ndescription = \"Single column.\"\n");
        put(root, "review/main.tex", "x");
        put(root, "review/img/logo.png", "x");
        put(root, "review/.secret", "x");
        put(root, "plain/README.md", "# Plain\n\nFirst paragraph\nsecond line.\n\nSecond paragraph.\n");
        put(root, "plain/a.txt", "x");
        put(root, ".hidden/x.txt", "x");
        put(root, "file.txt", "x");
    }

    fn bundles(dir: &Path) -> BundleDirs {
        BundleDirs { dir: dir.into(), category: "pc.templates".into(), source: Source::lore("pc") }
    }

    #[test]
    fn folders_become_bundles_with_their_files() {
        let t = tempdir().unwrap();
        bundle_tree(t.path());
        let (items, problems) = bundles(t.path()).load().unwrap();
        assert!(problems.is_empty());
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["pc.templates:plain", "pc.templates:review"]);
        assert_eq!(items[0].fields["files"], json!(["README.md", "a.txt"]));
        assert_eq!(items[0].field("about"), "First paragraph second line.");
        assert_eq!(items[1].fields["files"], json!(["img/logo.png", "main.tex", "template.toml"]));
        assert_eq!((items[1].field("about"), items[1].field("for")), ("Single column.", "Journal of Examples"));
        // At most 200 files.
        for i in 0..230 {
            put(t.path(), &format!("big/f{i:03}.txt"), "x");
        }
        let (items, _) = bundles(t.path()).load().unwrap();
        let big = items.iter().find(|i| i.id.ends_with(":big")).unwrap();
        assert_eq!(big.fields["files"].as_array().unwrap().len(), 200);
        assert_eq!(big.fields["files"][0], "f000.txt");
    }

    #[test]
    fn a_bundle_is_read_only() {
        let t = tempdir().unwrap();
        bundle_tree(t.path());
        let a = bundles(t.path());
        let (items, _) = a.load().unwrap();
        assert!(matches!(a.save(&items[0]), Err(KnowError::ReadOnly(_))));
        assert!(matches!(a.remove(&items[0].id), Err(KnowError::ReadOnly(_))));
        assert!(t.path().join("plain/a.txt").exists());
    }

    #[test]
    fn titles_come_from_the_folders_own_toml() {
        let t = tempdir().unwrap();
        put(t.path(), "one/style.toml", "# comment\n[pack]\nid = \"one\"\nname = \"空白\"\n");
        put(t.path(), "two/pack.toml", "title = \"Two\"\nvenue = \"Conf X\"\nabout = \"Dark.\"\n");
        put(t.path(), "three/template.toml", "this is = not [valid\n");
        put(t.path(), "four/notes.txt", "x");
        let (items, _) = bundles(t.path()).load().unwrap();
        let by = |n: &str| items.iter().find(|i| i.id.ends_with(n)).unwrap().clone();
        assert_eq!(by(":one").title, "空白");
        let two = by(":two");
        assert_eq!((two.title.as_str(), two.field("for"), two.field("about")), ("Two", "Conf X", "Dark."));
        assert_eq!(by(":three").title, "three");
        assert_eq!(by(":four").title, "four");
    }

    #[test]
    fn layout_files_become_items() {
        let t = tempdir().unwrap();
        put(t.path(), "01-title.json", r#"{"id":"01-title","name":"只有標題","class":"academic","blocks":[]}"#);
        put(t.path(), "02-text.json", r#"{"title":"Text","group":"g","description":"d"}"#);
        put(t.path(), "03-plain.json", r#"{"blocks":[1]}"#);
        put(t.path(), "bad.json", "{oops");
        put(t.path(), "note.txt", "x");
        let a = LayoutFiles { dir: t.path().into(), category: "sc.arrangements".into(), source: Source::lore("sc") };
        let (items, problems) = a.load().unwrap();
        let got: Vec<_> = items.iter().map(|i| (i.id.as_str(), i.title.as_str(), i.field("group"), i.field("description"))).collect();
        assert_eq!(
            got,
            [
                ("sc.arrangements:01-title", "只有標題", "academic", ""),
                ("sc.arrangements:02-text", "Text", "g", "d"),
                ("sc.arrangements:03-plain", "03-plain", "", ""),
            ]
        );
        assert_eq!(items[2].fields["data"], json!({"blocks":[1]}));
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].at, "bad.json");
        assert!(matches!(a.save(&items[0]), Err(KnowError::ReadOnly(_))));
        assert!(matches!(a.remove(&items[0].id), Err(KnowError::ReadOnly(_))));
    }

    fn list(file: &Path) -> LayoutList {
        LayoutList { file: file.into(), category: "sl.recipes".into(), source: Source::lore("sl") }
    }

    #[test]
    fn a_list_file_becomes_one_item_per_entry() {
        let t = tempdir().unwrap();
        let f = t.path().join("recipes.json");
        let entries = json!([
            {"id": "bisect/left-right", "group": "bisect", "name": "Left and right", "description": "Half and half."},
            {"id": "solo", "name": "Solo"}
        ]);
        fs::write(&f, entries.to_string()).unwrap();
        let (items, problems) = list(&f).load().unwrap();
        assert!(problems.is_empty());
        assert_eq!(items[0].id, "sl.recipes:bisect-left-right");
        assert_eq!((items[0].title.as_str(), items[0].field("group"), items[0].field("description")), ("Left and right", "bisect", "Half and half."));
        assert_eq!(items[0].fields["data"]["id"], "bisect/left-right", "the entry is kept as given");
        assert_eq!((items[1].title.as_str(), items[1].field("group")), ("Solo", ""));
        // An object holding the list under `recipes` (the shape of L-Slide's file) or `layouts`.
        for key in ["recipes", "layouts"] {
            fs::write(&f, json!({"source": "x", "compositions": {}, key: entries}).to_string()).unwrap();
            assert_eq!(list(&f).load().unwrap().0.len(), 2, "{key}");
        }
        assert!(matches!(list(&f).save(&items[0]), Err(KnowError::ReadOnly(_))));
        assert!(matches!(list(&f).remove(&items[0].id), Err(KnowError::ReadOnly(_))));
    }

    #[test]
    fn entries_without_an_id_are_listed() {
        let t = tempdir().unwrap();
        let f = t.path().join("l.json");
        fs::write(&f, json!([{"id":"a"}, {"name":"No id"}, 5, {"id":"  "}, {"id":"a"}]).to_string()).unwrap();
        let (items, problems) = list(&f).load().unwrap();
        assert_eq!(items.len(), 1);
        let at: Vec<_> = problems.iter().map(|p| p.at.as_str()).collect();
        assert_eq!(at, ["#2 No id", "#3", "#4", "a"]);
        assert!(problems[0].message.contains("id"));
    }

    #[test]
    fn previews_are_clipped_at_a_paragraph() {
        let mut n = Item::new("my.notes", format::NOTE, "N", Source::author());
        n.set("body", "First paragraph.\n\nSecond one is here.\n\nThird.");
        assert_eq!(preview_text(&n, 1000), "First paragraph.\n\nSecond one is here.\n\nThird.");
        assert_eq!(preview_text(&n, 40), "First paragraph.\n\nSecond one is here.\n\n…");
        assert_eq!(preview_text(&n, 10), "First par…");
        let mut b = Item::new("c", format::BUNDLE, "B", Source::author());
        b.set("about", "About it.").set("files", json!(["a.tex", "b.bib", "c.sty"]));
        assert_eq!(preview_text(&b, 1000), "About it.\na.tex\nb.bib\nc.sty");
        assert_eq!(preview_text(&b, 20), "About it.\na.tex\n…");
        let mut y = Item::new("c", format::LAYOUT, "L", Source::author());
        y.set("description", "Half\nand half.");
        assert_eq!(preview_text(&y, 100), "Half and half.");
    }
}
