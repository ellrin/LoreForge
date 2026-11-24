//! Categories: the apps' manifests (read-only, origin `lore`), the author's own categories and
//! the independent sets of outside applications (both kept in `<hub>/categories.toml`).
//!
//! Order of [`Registry::all`] is stable: manifest order, then manual, then external.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::*;

/// Every format id a category may use.
const FORMATS: &[&str] = &[
    format::CARD,
    format::PATTERN,
    format::EXPRESSION,
    format::WORDING,
    format::RULE,
    format::EXEMPLAR,
    format::NOTE,
    format::BUNDLE,
    format::LAYOUT,
    format::FIGURE,
];

/// Write `bytes` to `path` atomically (temp file in the same folder, then rename).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    fs::write(&tmp, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn rank(origin: &str) -> u8 {
    match origin {
        "lore" => 0,
        "manual" => 1,
        _ => 2,
    }
}

fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok && !id.starts_with('.') && !id.ends_with('.') && !id.contains("..") {
        Ok(())
    } else {
        Err(KnowError::Invalid(format!("類別代號「{id}」不合格：只能用小寫英文、數字與 . _ -")))
    }
}

fn check_format(f: &str) -> Result<()> {
    if FORMATS.contains(&f) { Ok(()) } else { Err(KnowError::Invalid(format!("不認得的格式「{f}」"))) }
}

fn section(id: &str) -> String {
    id.split('.').next().unwrap_or("").to_string()
}

/// A lowercase file-safe name (`safe_local`, lowercased); `None` when nothing of the text is usable.
fn lower_local(s: &str) -> Option<String> {
    let t = safe_local(s).to_ascii_lowercase();
    // `safe_local` turns an unusable text into "item"; a text that really is "item" stays.
    if t == "item" && !s.trim().eq_ignore_ascii_case("item") { None } else { Some(t) }
}

/// The category list.
#[derive(Debug, Clone)]
pub struct Registry {
    file: PathBuf,
    cats: Vec<Category>,
}

impl Registry {
    /// Categories of the apps' manifests (TOML `[[category]]`, forced `origin = "lore"`) plus the
    /// hand-made and outside ones in `<hub>/categories.toml` (missing file = none).
    pub fn load(manifests: &[&str], hub: &Path) -> Result<Registry> {
        let mut cats: Vec<Category> = vec![];
        for text in manifests {
            let m: Manifest = toml::from_str(text).map_err(|e| KnowError::Invalid(format!("類別宣告讀不了：{e}")))?;
            for mut c in m.category {
                check_id(&c.id)?;
                check_format(&c.format)?;
                c.origin = "lore".into();
                if c.app.is_empty() {
                    c.app = section(&c.id);
                }
                if c.label.is_empty() {
                    c.label = c.id.clone();
                }
                push_unique(&mut cats, c)?;
            }
        }
        let file = hub.join("categories.toml");
        if file.exists() {
            let text = fs::read_to_string(&file)?;
            let m: Manifest = toml::from_str(&text).map_err(|e| KnowError::Invalid(format!("categories.toml 讀不了：{e}")))?;
            for mut c in m.category {
                check_id(&c.id)?;
                check_format(&c.format)?;
                let origin = match c.origin.as_str() {
                    "manual" | "external" => c.origin.clone(),
                    "" if c.id.starts_with("my.") => "manual".into(),
                    "" if c.id.starts_with("x.") => "external".into(),
                    o => return Err(KnowError::Invalid(format!("categories.toml 裡的「{}」來源「{o}」不對", c.id))),
                };
                c.origin = origin;
                normalise(&mut c)?;
                push_unique(&mut cats, c)?;
            }
        }
        let mut r = Registry { file, cats };
        r.cats.sort_by_key(|c| rank(&c.origin));
        Ok(r)
    }

    pub fn all(&self) -> &[Category] {
        &self.cats
    }

    pub fn get(&self, id: &str) -> Option<&Category> {
        self.cats.iter().find(|c| c.id == id)
    }

    /// Where the hand-made and outside categories are kept.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Rewrite `categories.toml` with `cats` (only manual and external ones), then adopt them.
    fn commit(&mut self, mut cats: Vec<Category>) -> Result<()> {
        cats.sort_by_key(|c| rank(&c.origin));
        let own = Manifest { category: cats.iter().filter(|c| c.origin != "lore").cloned().collect() };
        let text = toml::to_string_pretty(&own).map_err(|e| KnowError::Other(format!("類別存不了：{e}")))?;
        write_atomic(&self.file, text.as_bytes())?;
        self.cats = cats;
        Ok(())
    }

    /// A category made by hand. `app_section` is the dashboard section ("" = `my`).
    pub fn add_manual(&mut self, label: &str, format: &str, app_section: &str, group: &str) -> Result<Category> {
        check_format(format)?;
        let section_id = if app_section.trim().is_empty() { "my".to_string() } else { app_section.trim().to_string() };
        check_id(&section_id)?;
        let local = lower_local(label).unwrap_or_else(|| format!("c{}", chrono::Local::now().timestamp_millis()));
        let base = format!("my.{local}");
        let mut id = base.clone();
        let mut n = 2;
        while self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        check_id(&id)?;
        let label = if label.trim().is_empty() { local } else { label.trim().to_string() };
        let c = Category {
            id,
            label,
            app: section_id,
            group: group.trim().into(),
            format: format.into(),
            origin: "manual".into(),
            tasks: vec![],
            native: Native::default(),
            writable: true,
        };
        let mut cats = self.cats.clone();
        cats.push(c.clone());
        self.commit(cats)?;
        Ok(c)
    }

    /// An outside application's independent set. The same source and name with the same format
    /// returns the existing one; with another format it is refused.
    pub fn add_external(&mut self, source_name: &str, set_name: &str, format: &str) -> Result<Category> {
        check_format(format)?;
        let (Some(s), Some(n)) = (lower_local(source_name), lower_local(set_name)) else {
            return Err(KnowError::Invalid("來源與這一組都要有英文或數字的名字".into()));
        };
        let id = format!("x.{s}.{n}");
        check_id(&id)?;
        if let Some(have) = self.get(&id) {
            return if have.format == format {
                Ok(have.clone())
            } else {
                Err(KnowError::Invalid(format!("「{id}」已經存在，格式是 {}，不能改成 {format}", have.format)))
            };
        }
        let c = Category {
            id,
            label: set_name.trim().into(),
            app: "x".into(),
            group: source_name.trim().into(),
            format: format.into(),
            origin: "external".into(),
            tasks: vec![],
            native: Native::default(),
            writable: true,
        };
        let mut cats = self.cats.clone();
        cats.push(c.clone());
        self.commit(cats)?;
        Ok(c)
    }

    fn position(&self, id: &str) -> Result<usize> {
        self.cats.iter().position(|c| c.id == id).ok_or_else(|| KnowError::NotFound(id.into()))
    }

    /// Steps that use a hand-made category. Lore categories' steps come from the manifest.
    pub fn set_tasks(&mut self, id: &str, tasks: Vec<String>) -> Result<()> {
        let at = self.position(id)?;
        match self.cats[at].origin.as_str() {
            "manual" => {}
            "lore" => return Err(KnowError::ReadOnly(format!("{id} 的步驟由程式的宣告決定"))),
            _ => return Err(KnowError::Invalid("外部應用程式的獨立組不接任何步驟".into())),
        }
        let mut clean: Vec<String> = vec![];
        for t in tasks.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            if !clean.iter().any(|c| c == t) {
                clean.push(t.to_string());
            }
        }
        let mut cats = self.cats.clone();
        cats[at].tasks = clean;
        self.commit(cats)
    }

    /// Change the label of a hand-made or outside category.
    pub fn rename(&mut self, id: &str, label: &str) -> Result<()> {
        let at = self.position(id)?;
        if self.cats[at].origin == "lore" {
            return Err(KnowError::ReadOnly(format!("{id} 是程式宣告的，名稱不能在這裡改")));
        }
        if label.trim().is_empty() {
            return Err(KnowError::Invalid("名稱不能空白".into()));
        }
        let mut cats = self.cats.clone();
        cats[at].label = label.trim().into();
        self.commit(cats)
    }

    /// Remove a hand-made or outside category; refused while it still has items (`has_items`).
    pub fn remove(&mut self, id: &str, has_items: bool) -> Result<Category> {
        let at = self.position(id)?;
        if self.cats[at].origin == "lore" {
            return Err(KnowError::ReadOnly(format!("{id} 是程式宣告的，不能移除")));
        }
        if has_items {
            return Err(KnowError::Invalid(format!("「{}」還有內容，要先清空才能移除", self.cats[at].label)));
        }
        let mut cats = self.cats.clone();
        let gone = cats.remove(at);
        self.commit(cats)?;
        Ok(gone)
    }
}

fn push_unique(cats: &mut Vec<Category>, c: Category) -> Result<()> {
    if cats.iter().any(|o| o.id == c.id) {
        return Err(KnowError::Invalid(format!("類別代號重複：{}", c.id)));
    }
    cats.push(c);
    Ok(())
}

/// The rules of the hand-made and outside categories read from `categories.toml`.
fn normalise(c: &mut Category) -> Result<()> {
    if c.origin == "manual" {
        if !c.id.starts_with("my.") {
            return Err(KnowError::Invalid(format!("手動類別的代號要以 my. 開頭：{}", c.id)));
        }
        c.native = Native::default();
        c.writable = true;
        if c.app.is_empty() {
            c.app = "my".into();
        }
    } else {
        let parts: Vec<&str> = c.id.split('.').collect();
        if parts.len() < 3 || parts[0] != "x" || parts[1].is_empty() || parts[2].is_empty() {
            return Err(KnowError::Invalid(format!("外部類別的代號要是 x.<來源>.<名稱>：{}", c.id)));
        }
        c.tasks.clear();
        c.native = Native::default();
        c.writable = true;
        c.app = "x".into();
    }
    if c.label.is_empty() {
        c.label = c.id.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PC: &str = r#"
[[category]]
id = "pc.patterns"
label = "句型"
app = "pc"
group = "寫作"
format = "pattern"
origin = "manual"
tasks = ["draft"]
writable = true
[category.native]
kind = "csv"
app_dir = "PaperComposer"
path = "patterns.csv"

[[category]]
id = "pc.rules"
label = "規則"
format = "rule"
"#;
    const SC: &str = r#"
[[category]]
id = "sc.layouts"
label = "版型"
app = "sc"
format = "layout"
"#;

    #[test]
    fn manifests_and_hand_made_categories_load_in_order() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::load(&[PC, SC], d.path()).unwrap();
        assert!(!d.path().join("categories.toml").exists());
        let m = r.add_manual("我的 notes", format::NOTE, "", "雜").unwrap();
        assert_eq!(m.id, "my.notes");
        let e = r.add_external("Acme", "Words", format::EXPRESSION).unwrap();
        assert_eq!(e.id, "x.acme.words");
        // a manual one made later still sorts before the external one
        let m2 = r.add_manual("", format::RULE, "pc", "").unwrap();
        assert!(m2.id.starts_with("my.c"));
        let ids: Vec<&str> = r.all().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["pc.patterns", "pc.rules", "sc.layouts", "my.notes", &m2.id, "x.acme.words"]);
        let pc = r.get("pc.patterns").unwrap();
        assert_eq!((pc.origin.as_str(), pc.tasks.len(), pc.native.kind.as_str()), ("lore", 1, "csv"));
        assert_eq!(r.get("pc.rules").unwrap().app, "pc");
        assert!(r.get("pc.rules").unwrap().tasks.is_empty());
        // lore categories are never written to categories.toml
        let text = std::fs::read_to_string(d.path().join("categories.toml")).unwrap();
        assert!(!text.contains("pc.patterns") && text.contains("my.notes"));
        let again = Registry::load(&[PC, SC], d.path()).unwrap();
        assert_eq!(again.all().iter().map(|c| &c.id).collect::<Vec<_>>(), r.all().iter().map(|c| &c.id).collect::<Vec<_>>());
    }

    #[test]
    fn an_external_set_never_has_tasks() {
        let d = tempfile::tempdir().unwrap();
        // a hand-edited file that tries to give an outside set tasks and a native place
        std::fs::write(
            d.path().join("categories.toml"),
            "[[category]]\nid = \"x.acme.words\"\nlabel = \"W\"\nformat = \"expression\"\norigin = \"external\"\ntasks = [\"draft\"]\n[category.native]\nkind = \"csv\"\npath = \"a.csv\"\n",
        )
        .unwrap();
        let mut r = Registry::load(&[], d.path()).unwrap();
        let c = r.get("x.acme.words").unwrap();
        assert!(c.tasks.is_empty() && c.native.kind.is_empty() && c.independent() && !c.connected());
        assert!(matches!(r.set_tasks("x.acme.words", vec!["draft".into()]), Err(KnowError::Invalid(_))));
        let m = r.add_manual("Mine", format::NOTE, "", "").unwrap();
        r.set_tasks(&m.id, vec!["a".into(), " a ".into(), "b".into()]).unwrap();
        assert_eq!(r.get(&m.id).unwrap().tasks, ["a", "b"]);
    }

    #[test]
    fn bad_ids_formats_and_duplicates_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let bad_id = "[[category]]\nid = \"PC.Bad\"\nformat = \"rule\"\n";
        assert!(matches!(Registry::load(&[bad_id], d.path()), Err(KnowError::Invalid(_))));
        let bad_fmt = "[[category]]\nid = \"pc.a\"\nformat = \"spaceship\"\n";
        assert!(matches!(Registry::load(&[bad_fmt], d.path()), Err(KnowError::Invalid(_))));
        assert!(matches!(Registry::load(&[PC, PC], d.path()), Err(KnowError::Invalid(_))));
        assert!(Registry::load(&["not = [toml"], d.path()).is_err());
        let mut r = Registry::load(&[PC], d.path()).unwrap();
        assert!(matches!(r.add_manual("x", "nope", "", ""), Err(KnowError::Invalid(_))));
        assert!(matches!(r.add_manual("x", format::NOTE, "Bad Section", ""), Err(KnowError::Invalid(_))));
        let a = r.add_manual("Same", format::NOTE, "", "").unwrap();
        let b = r.add_manual("Same", format::NOTE, "", "").unwrap();
        assert_eq!((a.id.as_str(), b.id.as_str()), ("my.same", "my.same-2"));
        r.add_external("acme", "w", format::RULE).unwrap();
        assert_eq!(r.add_external("ACME", "W", format::RULE).unwrap().id, "x.acme.w");
        assert!(matches!(r.add_external("acme", "w", format::NOTE), Err(KnowError::Invalid(_))));
        assert!(matches!(r.add_external("", "w", format::NOTE), Err(KnowError::Invalid(_))));
        // a hand file whose manual id lacks the prefix, and a duplicate of a manifest id
        std::fs::write(d.path().join("categories.toml"), "[[category]]\nid = \"oops\"\nformat = \"note\"\norigin = \"manual\"\n").unwrap();
        assert!(Registry::load(&[PC], d.path()).is_err());
        std::fs::write(d.path().join("categories.toml"), "[[category]]\nid = \"pc.rules\"\nformat = \"rule\"\norigin = \"manual\"\n").unwrap();
        assert!(Registry::load(&[PC], d.path()).is_err());
    }

    #[test]
    fn changes_survive_a_reload() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::load(&[PC], d.path()).unwrap();
        let m = r.add_manual("Mine", format::NOTE, "", "g").unwrap();
        let e = r.add_external("acme", "w", format::RULE).unwrap();
        r.set_tasks(&m.id, vec!["t1".into()]).unwrap();
        r.rename(&m.id, "Renamed").unwrap();
        r.rename(&e.id, "Outside").unwrap();
        let r2 = Registry::load(&[PC], d.path()).unwrap();
        let m2 = r2.get(&m.id).unwrap();
        assert_eq!((m2.label.as_str(), m2.group.as_str(), m2.tasks.clone()), ("Renamed", "g", vec!["t1".to_string()]));
        assert_eq!(r2.get(&e.id).unwrap().label, "Outside");
        // lore categories: read-only
        let mut r2 = r2;
        assert!(matches!(r2.rename("pc.rules", "x"), Err(KnowError::ReadOnly(_))));
        assert!(matches!(r2.set_tasks("pc.rules", vec![]), Err(KnowError::ReadOnly(_))));
        assert!(matches!(r2.remove("pc.rules", false), Err(KnowError::ReadOnly(_))));
        assert!(matches!(r2.remove(&m.id, true), Err(KnowError::Invalid(_))));
        assert!(r2.get(&m.id).is_some());
        r2.remove(&m.id, false).unwrap();
        assert!(matches!(r2.remove("my.none", false), Err(KnowError::NotFound(_))));
        let r3 = Registry::load(&[PC], d.path()).unwrap();
        assert!(r3.get(&m.id).is_none() && r3.get(&e.id).is_some());
        assert!(std::fs::read_dir(d.path()).unwrap().all(|f| !f.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }
}
