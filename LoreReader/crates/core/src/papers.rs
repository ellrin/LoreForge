//! Per-paper metadata in the private workspace: `private/LoreReader/papers/<sha256>/`.
//! The PDF itself is in the library (outside git); these files are small JSON, written
//! atomically. Formats (`schema` names a version; unknown fields are kept):
//!
//! `paper.json`
//! ```json
//! { "schema": "lorereader.paper/1", "sha256": "…", "title": "…", "original_name": "x.pdf",
//!   "doi": "10.…" | null, "pages": 12, "added_at": "2026-11-10T09:00:00+08:00",
//!   "derived_from": { "op": "compose", "from": ["<sha256>", …] } | null }
//! ```
//! `annotations.json`
//! ```json
//! { "schema": "lorereader.annotations/1", "annotations": [
//!   { "id": "a…", "page": 3, "kind": "highlight" | "note", "color": "yellow",
//!     "quote": { "exact": "…", "prefix": "…", "suffix": "…" },
//!     "rects": [[x0, y0, x1, y1], …],          // fractions of the page box, top-left origin
//!     "note": "…", "created_at": "…", "updated_at": "…" } ] }
//! ```
//! `quotes.json` (citable sentences; PaperComposer will read this format)
//! ```json
//! { "schema": "lorereader.quotes/1", "quotes": [
//!   { "id": "q…", "text": "…", "page": 3, "note": "…" | "", "created_at": "…" } ] }
//! ```
//! `chat.json`
//! ```json
//! { "schema": "lorereader.chat/1", "messages": [
//!   { "role": "user" | "assistant", "text": "…", "at": "…", "engine": "…"?,
//!     "attachments": [ { "text": "…", "page": 3, "context": "…" } ],
//!     "paragraph": "p…"?, "chip": { "kind": "symbol", "id": "w", "label": "w_i" }?,
//!     "context": "…"? } ] }
//! ```

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::library::valid_sha;
use crate::{Error, Result};

pub const PAPER_SCHEMA: &str = "lorereader.paper/1";
pub const ANNOTATIONS_SCHEMA: &str = "lorereader.annotations/1";
pub const QUOTES_SCHEMA: &str = "lorereader.quotes/1";
pub const CHAT_SCHEMA: &str = "lorereader.chat/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Derived {
    pub op: String,
    pub from: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperMeta {
    #[serde(default = "paper_schema")]
    pub schema: String,
    pub sha256: String,
    pub title: String,
    #[serde(default)]
    pub original_name: String,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub pages: u32,
    pub added_at: String,
    #[serde(default)]
    pub derived_from: Option<Derived>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn paper_schema() -> String {
    PAPER_SCHEMA.into()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TextQuote {
    pub exact: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    #[serde(default)]
    pub id: String,
    pub page: u32,
    #[serde(default = "highlight")]
    pub kind: String,
    #[serde(default = "yellow")]
    pub color: String,
    #[serde(default)]
    pub quote: TextQuote,
    #[serde(default)]
    pub rects: Vec<[f64; 4]>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn highlight() -> String {
    "highlight".into()
}
fn yellow() -> String {
    "yellow".into()
}

pub const COLORS: [&str; 4] = ["yellow", "green", "blue", "pink"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitableQuote {
    #[serde(default)]
    pub id: String,
    pub text: String,
    pub page: u32,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub text: String,
    pub page: u32,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// The paragraph this exchange is filed under (track B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paragraph: Option<String>,
    /// The stuck point it was about: `{kind: term|symbol|equation, id, label}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<Value>,
    /// What was sent to the model before the question (paper, section, paragraph,
    /// definitions, the author's KB states), so the history replays exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnnotationsFile {
    schema: String,
    annotations: Vec<Annotation>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuotesFile {
    schema: String,
    quotes: Vec<CitableQuote>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatFile {
    schema: String,
    messages: Vec<ChatMessage>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Changes to an annotation (only the fields that are set).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AnnotationPatch {
    pub color: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Papers {
    /// `private/LoreReader/papers`
    pub dir: PathBuf,
}

fn read_json<T: DeserializeOwned>(p: &Path) -> Result<Option<T>> {
    match std::fs::read(p) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| Error::Invalid(format!("{}: {e}", p.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    let mut b = serde_json::to_vec_pretty(v)?;
    b.push(b'\n');
    crate::write_atomic(p, &b)?;
    Ok(())
}

fn check_color(c: &str) -> Result<()> {
    if COLORS.contains(&c) { Ok(()) } else { Err(Error::Invalid(format!("顏色只能是 {}", COLORS.join("、")))) }
}

impl Papers {
    pub fn new(dir: impl Into<PathBuf>) -> Papers {
        Papers { dir: dir.into() }
    }

    fn folder(&self, sha: &str) -> Result<PathBuf> {
        if !valid_sha(sha) {
            return Err(Error::Invalid(format!("不是 sha256：{sha}")));
        }
        Ok(self.dir.join(sha))
    }

    pub fn meta(&self, sha: &str) -> Result<Option<PaperMeta>> {
        read_json(&self.folder(sha)?.join("paper.json"))
    }

    /// Write `paper.json` the first time a PDF is added. An existing one (the user may
    /// have edited the title) is kept and returned.
    pub fn register(&self, meta: PaperMeta) -> Result<PaperMeta> {
        if let Some(m) = self.meta(&meta.sha256)? {
            return Ok(m);
        }
        let p = self.folder(&meta.sha256)?.join("paper.json");
        write_json(&p, &meta)?;
        Ok(meta)
    }

    /// Rename a paper (the only edit to paper.json from the UI).
    pub fn set_title(&self, sha: &str, title: &str) -> Result<PaperMeta> {
        let mut m = self.meta(sha)?.ok_or_else(|| Error::NotFound(sha.into()))?;
        let t = title.trim();
        if t.is_empty() {
            return Err(Error::Invalid("標題不能是空的".into()));
        }
        m.title = t.to_string();
        write_json(&self.folder(sha)?.join("paper.json"), &m)?;
        Ok(m)
    }

    /// 「我的論文」: the author's own (published or co-authored) paper.
    pub fn set_mine(&self, sha: &str, on: bool) -> Result<PaperMeta> {
        let mut m = self.meta(sha)?.ok_or_else(|| Error::NotFound(sha.into()))?;
        if on {
            m.extra.insert("mine".into(), Value::Bool(true));
        } else {
            m.extra.remove("mine");
        }
        write_json(&self.folder(sha)?.join("paper.json"), &m)?;
        Ok(m)
    }

    /// Every paper with a `paper.json`, newest first.
    pub fn list(&self) -> Result<Vec<PaperMeta>> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Ok(out) };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if valid_sha(&name)
                && let Ok(Some(m)) = self.meta(&name)
            {
                out.push(m);
            }
        }
        out.sort_by(|a, b| b.added_at.cmp(&a.added_at));
        Ok(out)
    }

    // ── annotations ──

    pub fn annotations(&self, sha: &str) -> Result<Vec<Annotation>> {
        Ok(read_json::<AnnotationsFile>(&self.folder(sha)?.join("annotations.json"))?.map(|f| f.annotations).unwrap_or_default())
    }

    fn save_annotations(&self, sha: &str, list: Vec<Annotation>) -> Result<()> {
        let p = self.folder(sha)?.join("annotations.json");
        let extra = read_json::<AnnotationsFile>(&p)?.map(|f| f.extra).unwrap_or_default();
        write_json(&p, &AnnotationsFile { schema: ANNOTATIONS_SCHEMA.into(), annotations: list, extra })
    }

    pub fn add_annotation(&self, sha: &str, mut a: Annotation) -> Result<Annotation> {
        if a.page == 0 {
            return Err(Error::Invalid("頁碼從 1 開始".into()));
        }
        if !matches!(a.kind.as_str(), "highlight" | "note") {
            return Err(Error::Invalid("kind 只能是 highlight 或 note".into()));
        }
        check_color(&a.color)?;
        if a.quote.exact.trim().is_empty() && a.rects.is_empty() {
            return Err(Error::Invalid("註記需要選取的文字或位置".into()));
        }
        let now = crate::now();
        a.id = crate::new_id("a");
        a.created_at = now.clone();
        a.updated_at = now;
        let mut list = self.annotations(sha)?;
        list.push(a.clone());
        self.save_annotations(sha, list)?;
        Ok(a)
    }

    pub fn update_annotation(&self, sha: &str, id: &str, patch: AnnotationPatch) -> Result<Annotation> {
        let mut list = self.annotations(sha)?;
        let a = list.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::NotFound(id.into()))?;
        if let Some(c) = patch.color {
            check_color(&c)?;
            a.color = c;
        }
        if let Some(n) = patch.note {
            a.note = n;
        }
        a.updated_at = crate::now();
        let out = a.clone();
        self.save_annotations(sha, list)?;
        Ok(out)
    }

    pub fn delete_annotation(&self, sha: &str, id: &str) -> Result<()> {
        let mut list = self.annotations(sha)?;
        let n = list.len();
        list.retain(|a| a.id != id);
        if list.len() == n {
            return Err(Error::NotFound(id.into()));
        }
        self.save_annotations(sha, list)
    }

    // ── citable sentences ──

    pub fn quotes(&self, sha: &str) -> Result<Vec<CitableQuote>> {
        Ok(read_json::<QuotesFile>(&self.folder(sha)?.join("quotes.json"))?.map(|f| f.quotes).unwrap_or_default())
    }

    fn save_quotes(&self, sha: &str, list: Vec<CitableQuote>) -> Result<()> {
        let p = self.folder(sha)?.join("quotes.json");
        let extra = read_json::<QuotesFile>(&p)?.map(|f| f.extra).unwrap_or_default();
        write_json(&p, &QuotesFile { schema: QUOTES_SCHEMA.into(), quotes: list, extra })
    }

    pub fn add_quote(&self, sha: &str, text: &str, page: u32, note: &str) -> Result<CitableQuote> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || page == 0 {
            return Err(Error::Invalid("引用句需要文字和頁碼".into()));
        }
        let note = note.lines().next().unwrap_or("").trim().to_string();
        let q = CitableQuote { id: crate::new_id("q"), text, page, note, created_at: crate::now(), extra: Map::new() };
        let mut list = self.quotes(sha)?;
        list.push(q.clone());
        self.save_quotes(sha, list)?;
        Ok(q)
    }

    pub fn delete_quote(&self, sha: &str, id: &str) -> Result<()> {
        let mut list = self.quotes(sha)?;
        let n = list.len();
        list.retain(|q| q.id != id);
        if list.len() == n {
            return Err(Error::NotFound(id.into()));
        }
        self.save_quotes(sha, list)
    }

    /// Citable sentences of every paper, with the paper's sha and title.
    pub fn all_quotes(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for m in self.list()? {
            for q in self.quotes(&m.sha256)? {
                let mut v = serde_json::to_value(&q)?;
                v["sha256"] = m.sha256.clone().into();
                v["paper_title"] = m.title.clone().into();
                v["doi"] = m.doi.clone().map(Value::from).unwrap_or(Value::Null);
                out.push(v);
            }
        }
        Ok(out)
    }

    // ── other per-paper files (nouns.json, symbols.json, paragraphs.json, reading.json) ──

    /// Read `papers/<sha>/<file>` (`None` when it does not exist yet).
    pub fn load<T: DeserializeOwned>(&self, sha: &str, file: &str) -> Result<Option<T>> {
        read_json(&self.folder(sha)?.join(file))
    }

    /// Write `papers/<sha>/<file>` atomically.
    pub fn save<T: Serialize>(&self, sha: &str, file: &str, v: &T) -> Result<()> {
        if file.contains(['/', '\\']) || !file.ends_with(".json") {
            return Err(Error::Invalid(format!("檔名：{file}")));
        }
        write_json(&self.folder(sha)?.join(file), v)
    }

    // ── chat ──

    pub fn chat(&self, sha: &str) -> Result<Vec<ChatMessage>> {
        Ok(read_json::<ChatFile>(&self.folder(sha)?.join("chat.json"))?.map(|f| f.messages).unwrap_or_default())
    }

    pub fn push_chat(&self, sha: &str, msgs: &[ChatMessage]) -> Result<()> {
        let p = self.folder(sha)?.join("chat.json");
        let (mut list, extra) = read_json::<ChatFile>(&p)?.map(|f| (f.messages, f.extra)).unwrap_or_default();
        list.extend_from_slice(msgs);
        write_json(&p, &ChatFile { schema: CHAT_SCHEMA.into(), messages: list, extra })
    }

    pub fn clear_chat(&self, sha: &str) -> Result<()> {
        let p = self.folder(sha)?.join("chat.json");
        if p.exists() {
            let extra = read_json::<ChatFile>(&p)?.map(|f| f.extra).unwrap_or_default();
            write_json(&p, &ChatFile { schema: CHAT_SCHEMA.into(), messages: vec![], extra })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(sha: &str) -> PaperMeta {
        PaperMeta { schema: PAPER_SCHEMA.into(), sha256: sha.into(), title: "T".into(), original_name: "t.pdf".into(), doi: None, pages: 2, added_at: crate::now(), derived_from: None, extra: Map::new() }
    }

    #[test]
    fn files_round_trip_and_keep_unknown_fields() {
        let d = tempfile::tempdir().unwrap();
        let ps = Papers::new(d.path());
        let sha = "ab".repeat(32);
        ps.register(meta(&sha)).unwrap();
        // register never overwrites
        let mut other = meta(&sha);
        other.title = "Other".into();
        assert_eq!(ps.register(other).unwrap().title, "T");
        assert_eq!(ps.set_title(&sha, " New ").unwrap().title, "New");
        assert!(ps.meta("nope").is_err());

        let a = ps
            .add_annotation(&sha, Annotation { id: String::new(), page: 2, kind: "highlight".into(), color: "green".into(), quote: TextQuote { exact: "Dice".into(), prefix: "the ".into(), suffix: " score".into() }, rects: vec![[0.1, 0.2, 0.3, 0.25]], note: String::new(), created_at: String::new(), updated_at: String::new(), extra: Map::new() })
            .unwrap();
        assert!(a.id.starts_with('a'));
        let u = ps.update_annotation(&sha, &a.id, AnnotationPatch { note: Some("重要".into()), color: None }).unwrap();
        assert_eq!(u.note, "重要");
        assert!(ps.update_annotation(&sha, &a.id, AnnotationPatch { color: Some("red".into()), note: None }).is_err());

        // An unknown top-level field survives our writes.
        let p = d.path().join(&sha).join("annotations.json");
        let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        v["future"] = 1.into();
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        ps.delete_annotation(&sha, &a.id).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!((v["future"].as_i64(), v["annotations"].as_array().unwrap().len()), (Some(1), 0));
        assert_eq!(v["schema"], ANNOTATIONS_SCHEMA);

        let q = ps.add_quote(&sha, "  Overlap   metrics\nhide errors. ", 3, "for intro\nignored").unwrap();
        assert_eq!((q.text.as_str(), q.note.as_str()), ("Overlap metrics hide errors.", "for intro"));
        let all = ps.all_quotes().unwrap();
        assert_eq!(all[0]["sha256"], sha);
        assert_eq!(all[0]["paper_title"], "New");
        ps.delete_quote(&sha, &q.id).unwrap();
        assert!(ps.quotes(&sha).unwrap().is_empty());

        ps.push_chat(&sha, &[ChatMessage { role: "user".into(), text: "?".into(), attachments: vec![Attachment { text: "x".into(), page: 1, context: "y".into() }], at: crate::now(), engine: None, paragraph: None, chip: None, context: None }]).unwrap();
        assert_eq!(ps.chat(&sha).unwrap().len(), 1);
        ps.clear_chat(&sha).unwrap();
        assert!(ps.chat(&sha).unwrap().is_empty());
        // No temp files left behind.
        let left: Vec<_> = std::fs::read_dir(d.path().join(&sha)).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().ends_with(".tmp")).collect();
        assert!(left.is_empty());
    }
}
