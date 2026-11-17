//! 開始面板 (docs/book.md §統一入口): the one place a book starts. A `Setup` names the starting
//! points (L-Reader papers, PDF files, a folder of PDFs, typed keywords, L-Brain cards) and the
//! book's options. Another app (L-Reader's 寫 review) or the panel itself keeps one here for an
//! hour (`POST /api/book/setup`), the panel reads and saves it, 搜尋檔案 adds files to it, and
//! `POST /api/book` turns it into the job's [`super::pipeline::Params`].
//!
//! Paths of files and folders stay in this memory store and in the job's cache (its params);
//! everything shown elsewhere names a file by its file name only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::source::Extras;

/// One starting point. `lr`: `sha` (+ `title`); `file` / `folder`: `path`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Start {
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sha: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl Start {
    /// What the page shows: the title, else the file or folder name (never the path).
    pub fn label(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.trim().to_string();
        }
        match &self.path {
            Some(p) => name_of(p),
            None => self.sha.chars().take(8).collect(),
        }
    }
}

pub fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

pub const FORMATS: &[&str] = &["notes", "review1", "review2"];

/// The panel's state (docs/book.md `Setup`). Every field may be missing in a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Setup {
    pub sources: Vec<Start>,
    pub keywords: Vec<String>,
    /// L-Brain card ids.
    pub cards: Vec<String>,
    pub extras: Extras,
    /// `new` or an opened.toml id (加進這本書).
    pub book: String,
    /// A new book's title; "" = the planner names it.
    pub title: String,
    pub folder: Option<PathBuf>,
    /// notes · review1 · review2
    pub format: String,
    /// b5 · a4 (notes only)
    pub page: String,
    pub length: String,
    pub lang: String,
    pub make_cards: bool,
    /// review · direct
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup {
            sources: vec![],
            keywords: vec![],
            cards: vec![],
            extras: Extras::default(),
            book: "new".into(),
            title: String::new(),
            folder: None,
            format: "notes".into(),
            page: "b5".into(),
            length: "standard".into(),
            lang: "en-zh".into(),
            make_cards: true,
            mode: "review".into(),
            engine: None,
        }
    }
}

impl Setup {
    /// A body of `POST /api/book`: a `Setup`, or the older `{source: {kind: "lr", sha}, …}`.
    pub fn from_body(mut v: Value) -> Result<Setup, String> {
        if !v.is_object() {
            return Err("要送 JSON 物件".into());
        }
        if v.get("sources").is_none()
            && let Some(src) = v.get("source").filter(|s| s.is_object()).cloned()
        {
            v["sources"] = json!([src]);
        }
        serde_json::from_value(v).map_err(|e| format!("看不懂的設定：{e}"))
    }

    /// Keywords trimmed, empty ones and repeats out; card ids likewise.
    pub fn tidy(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.keywords = self.keywords.iter().map(|k| k.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|k| !k.is_empty() && seen.insert(k.to_lowercase())).collect();
        let mut seen = std::collections::HashSet::new();
        self.cards = self.cards.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && seen.insert(c.clone())).collect();
        let mut seen: Vec<Start> = vec![];
        for s in std::mem::take(&mut self.sources) {
            if !seen.iter().any(|x| x.kind == s.kind && x.sha == s.sha && x.path == s.path) {
                seen.push(s);
            }
        }
        self.sources = seen;
        if self.book.trim().is_empty() {
            self.book = "new".into();
        }
    }

    /// Anything to start from at all.
    pub fn has_start(&self) -> bool {
        !self.sources.is_empty() || !self.keywords.is_empty() || !self.cards.is_empty()
    }

    /// Every starting point is one the job can read: a known kind, a valid sha, an absolute path
    /// to a PDF / a folder that is there.
    pub fn check_sources(&self) -> Result<(), String> {
        for s in &self.sources {
            match s.kind.as_str() {
                "lr" if lore_learn::reader::valid_sha(&s.sha) => {}
                "lr" => return Err("不認得的論文".into()),
                "file" => match &s.path {
                    Some(p) if p.is_absolute() && p.is_file() && is_pdf(p) => {}
                    Some(p) => return Err(format!("找不到這個 PDF：{}", name_of(p))),
                    None => return Err("檔案沒有路徑".into()),
                },
                "folder" => match &s.path {
                    Some(p) if p.is_absolute() && p.is_dir() => {}
                    Some(p) => return Err(format!("找不到這個資料夾：{}", name_of(p))),
                    None => return Err("資料夾沒有路徑".into()),
                },
                _ => return Err("不認得的來源".into()),
            }
        }
        Ok(())
    }

    /// What `GET /api/book/setup/{id}` answers: the setup, each source with its `label`.
    pub fn view(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(json!({}));
        if let Some(a) = v["sources"].as_array_mut() {
            for (x, s) in a.iter_mut().zip(&self.sources) {
                x["label"] = s.label().into();
            }
        }
        v
    }
}

pub fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

// ── the store (memory, one hour) ─────────────────────────────────────────────

const KEEP: Duration = Duration::from_secs(3600);

static STORE: Mutex<Option<HashMap<String, (Instant, Setup)>>> = Mutex::new(None);

fn with_store<T>(f: impl FnOnce(&mut HashMap<String, (Instant, Setup)>) -> T) -> T {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    m.retain(|_, (t, _)| t.elapsed() < KEEP);
    f(m)
}

/// A random id (letters and digits; the store is reachable from any local page).
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out += &format!("{:016x}", h.finish());
    }
    out
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Keep `s`; its new id.
pub fn put_new(s: Setup) -> String {
    let id = new_id();
    with_store(|m| m.insert(id.clone(), (Instant::now(), s)));
    id
}

/// Replace a kept setup (false: no such id, or it expired).
pub fn replace(id: &str, s: Setup) -> bool {
    with_store(|m| match m.get_mut(id) {
        Some(e) => {
            *e = (Instant::now(), s);
            true
        }
        None => false,
    })
}

pub fn get(id: &str) -> Option<Setup> {
    with_store(|m| m.get_mut(id).map(|e| {
        e.0 = Instant::now();
        e.1.clone()
    }))
}

/// 搜尋檔案 for 寫書 (`lore_shell::findfile`, context `{setup}`): a PDF the author chose in
/// L-Echo joins the setup's sources (once). Only under `home`. `Ok` = what L-Echo shows.
pub fn picked(id: &str, path: &Path, home: &Path) -> Result<String, String> {
    if !valid_id(id) {
        return Err("開始面板已經關閉，請重新打開".into());
    }
    let real = crate::draft::record_path(&path.to_string_lossy(), home)?;
    if !real.is_file() || !is_pdf(&real) {
        return Err("寫書只收 PDF".into());
    }
    with_store(|m| {
        let (t, s) = m.get_mut(id).ok_or("開始面板已經關閉，請重新打開")?;
        *t = Instant::now();
        if !s.sources.iter().any(|x| x.kind == "file" && x.path.as_deref() == Some(real.as_path())) {
            s.sources.push(Start { kind: "file".into(), path: Some(real.clone()), ..Default::default() });
        }
        Ok(name_of(&real))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setup_from_old_and_new_bodies() {
        let sha = "ab".repeat(32);
        let old = Setup::from_body(json!({ "source": { "kind": "lr", "sha": sha }, "length": "short" })).unwrap();
        assert_eq!((old.sources.len(), old.length.as_str(), old.format.as_str(), old.make_cards), (1, "short", "notes", true));
        let mut s = Setup::from_body(json!({ "keywords": [" contrastive  learning ", "Contrastive learning", ""], "cards": ["k1", "k1"] })).unwrap();
        s.tidy();
        assert_eq!(s.keywords, ["contrastive learning"]);
        assert_eq!(s.cards, ["k1"]);
        assert!(s.has_start() && !Setup::default().has_start());
        assert!(Setup::from_body(json!([1])).is_err());
    }

    #[test]
    fn sources_are_checked_and_labelled_without_paths() {
        let d = std::env::temp_dir().join(format!("pc-book-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("papers")).unwrap();
        std::fs::write(d.join("papers/a.pdf"), b"%PDF").unwrap();
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        let file = |p: PathBuf| Start { kind: "file".into(), path: Some(p), ..Default::default() };
        let mut s = Setup { sources: vec![file(d.join("papers/a.pdf")), Start { kind: "folder".into(), path: Some(d.join("papers")), ..Default::default() }], ..Default::default() };
        assert!(s.check_sources().is_ok());
        let v = s.view();
        assert_eq!(v["sources"][0]["label"], "a.pdf");
        assert_eq!(v["sources"][1]["label"], "papers");
        s.sources.push(file(d.join("notes.txt")));
        assert!(s.check_sources().is_err(), "not a PDF");
        s.sources = vec![Start { kind: "lr".into(), sha: "x".into(), ..Default::default() }];
        assert!(s.check_sources().is_err());
        s.sources = vec![Start { kind: "web".into(), ..Default::default() }];
        assert!(s.check_sources().is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_store_keeps_replaces_and_takes_picked_files_once() {
        let home = std::env::temp_dir().join(format!("pc-book-picked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("a.pdf"), b"%PDF").unwrap();
        std::fs::write(home.join("b.docx"), b"x").unwrap();
        let id = put_new(Setup::default());
        assert!(valid_id(&id) && get(&id).is_some());
        assert_ne!(put_new(Setup::default()), id, "a fresh id each time");
        assert!(replace(&id, Setup { keywords: vec!["softmax".into()], ..Default::default() }));
        assert!(!replace(&"0".repeat(32), Setup::default()));
        assert_eq!(picked(&id, &home.join("a.pdf"), &home).unwrap(), "a.pdf");
        picked(&id, &home.join("a.pdf"), &home).unwrap();
        let s = get(&id).unwrap();
        assert_eq!((s.keywords.len(), s.sources.len()), (1, 1), "the same file once");
        assert!(picked(&id, &home.join("b.docx"), &home).is_err());
        assert!(picked(&"0".repeat(32), &home.join("a.pdf"), &home).is_err(), "an expired setup");
        assert!(picked(&id, Path::new("/etc/hosts"), &home).is_err(), "outside home");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
