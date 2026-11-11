//! PaperComposer notes (筆記) and ideas (靈感): one LaTeX file per note in
//! `<PaperComposer workspace>/notes/`, a `% ---` front matter block on top.
//! PaperComposer serves them (`/api/notes`); LoreEcho writes the same files
//! through this crate when PaperComposer is closed. Every write runs the
//! lore-notation checks first and refuses to save on a blocking finding.
//! See the README for the format and the blocking rule.

use lore_notation::{check_document, CheckOptions, Document, Notation, Severity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Findings that stop a save: PaperComposer's own blocking pair for written
/// text (`hardcoded_reference`, `ref_undefined`) plus what breaks a fragment.
/// A `\ref{…}` is defined when the note or the registry has the label.
pub const BLOCKING: &[&str] = &["hardcoded_reference", "ref_undefined", "label_duplicate", "registry_invalid", "note_structure"];

/// The trash folder inside the notes folder.
pub const TRASH: &str = ".trash";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// 筆記: a worked-out note.
    #[default]
    Note,
    /// 靈感: a one-line idea, listed apart.
    Idea,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Idea => "idea",
        }
    }
    fn parse(s: &str) -> Kind {
        if s.trim().eq_ignore_ascii_case("idea") { Kind::Idea } else { Kind::Note }
    }
}

/// One note as read from (or written to) its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Note {
    /// The file stem (`2026-11-12-loss-weights`); empty before the first save.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: Kind,
    /// `loreecho` (學習夥伴) or `manual` (手寫); anything else is kept as is.
    #[serde(default)]
    pub source: String,
    /// ISO 8601 with offset (`2026-11-12T10:04:00+08:00`).
    #[serde(default)]
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// Related KnowledgeBase card ids.
    #[serde(default)]
    pub kb: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Front matter keys this crate does not know, kept on rewrite.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
    /// LaTeX body (no preamble), trailing whitespace trimmed.
    #[serde(default)]
    pub body: String,
    /// File modification time, Unix milliseconds (0 when unknown); not stored in the file.
    #[serde(default)]
    pub modified: u64,
}

/// What a caller sends to create a note (`POST /api/notes`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NewNote {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub kb: Vec<String>,
    /// Defaults to `manual`.
    #[serde(default)]
    pub source: Option<String>,
}

/// A change to an existing note (`PUT /api/notes/{id}`); missing fields stay.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Edit {
    pub title: Option<String>,
    pub body: Option<String>,
    pub kind: Option<Kind>,
    pub tags: Option<Vec<String>>,
    pub kb: Option<Vec<String>>,
}

/// One lore-notation finding on a note body (lines count from the body's first line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    /// error | warning | info
    pub severity: String,
    pub message: String,
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// A saved note: its id, file and the (non-blocking) findings.
#[derive(Debug, Clone, Serialize)]
pub struct Saved {
    pub id: String,
    pub path: PathBuf,
    pub findings: Vec<Finding>,
}

#[derive(Debug)]
pub enum Error {
    /// A blocking finding: nothing was written.
    Blocked(Vec<Finding>),
    NotFound(String),
    /// An id with a path separator, `..` or a leading dot.
    BadId(String),
    /// No title and no body.
    Empty,
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Blocked(v) => write!(f, "對照表檢查沒過（{} 項）", v.iter().filter(|x| x.blocking).count()),
            Error::NotFound(id) => write!(f, "找不到筆記 {id}"),
            Error::BadId(id) => write!(f, "筆記名稱不合法：{id}"),
            Error::Empty => write!(f, "筆記是空的"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ── file format ──────────────────────────────────────────

const FENCE: &str = "% ---";

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn list_value(v: &[String]) -> String {
    v.iter().map(|t| one_line(&t.replace(',', " "))).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", ")
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

fn clean_list(v: Vec<String>) -> Vec<String> {
    split_list(&list_value(&v))
}

/// The file text of a note: front matter, then the body.
pub fn to_tex(n: &Note) -> String {
    let mut out = String::from(FENCE);
    out.push('\n');
    let mut line = |k: &str, v: &str| {
        out.push_str(&format!("% {k}: {}\n", one_line(v)));
    };
    line("source", &n.source);
    line("created", &n.created);
    if let Some(u) = &n.updated {
        line("updated", u);
    }
    line("kind", n.kind.as_str());
    line("kb", &list_value(&n.kb));
    line("tags", &list_value(&n.tags));
    line("title", &n.title);
    for (k, v) in &n.extra {
        line(k, v);
    }
    out.push_str(FENCE);
    out.push('\n');
    let body = n.body.trim_end();
    if !body.is_empty() {
        out.push_str(body);
        out.push('\n');
    }
    out
}

/// Read a note file's text. A file without front matter (written by hand)
/// is a note whose title is its id and whose body is the whole text.
pub fn parse(id: &str, text: &str) -> Note {
    let mut n = Note { id: id.to_string(), kind: Kind::Note, ..Default::default() };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    let mut body_start = 0;
    if first.trim_end() == FENCE {
        let mut off = first.len();
        let mut closed = false;
        for l in lines {
            off += l.len();
            let t = l.trim_end();
            if t == FENCE {
                closed = true;
                break;
            }
            let Some(kv) = t.strip_prefix('%') else { break };
            let Some((k, v)) = kv.split_once(':') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "title" => n.title = v.to_string(),
                "source" => n.source = v.to_string(),
                "created" => n.created = v.to_string(),
                "updated" => n.updated = Some(v.to_string()).filter(|s| !s.is_empty()),
                "kind" => n.kind = Kind::parse(v),
                "kb" => n.kb = split_list(v),
                "tags" => n.tags = split_list(v),
                _ if !k.is_empty() => {
                    n.extra.insert(k.to_string(), v.to_string());
                }
                _ => {}
            }
        }
        if closed {
            body_start = off;
        } else {
            // not our block after all: the whole text is the body
            n = Note { id: id.to_string(), ..Default::default() };
        }
    }
    n.body = text[body_start..].trim_end().to_string();
    if n.title.is_empty() && body_start == 0 {
        n.title = id.to_string();
    }
    n
}

/// A file-name part from a title: letters and digits of any script kept
/// (lower-cased), everything else one `-`, at most 40 characters.
pub fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s: String = s.trim_matches('-').chars().take(40).collect();
    s.trim_end_matches('-').to_string()
}

/// Is this a safe note id (a plain file stem)?
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains(['/', '\\', '\0']) && !id.contains("..") && id.len() <= 200
}

/// `dir/<stem>.tex`, or `<stem>-2.tex`, `-3` … when taken.
pub fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.tex"));
    if !first.exists() {
        return first;
    }
    (2..).map(|i| dir.join(format!("{stem}-{i}.tex"))).find(|p| !p.exists()).unwrap()
}

/// Now, local time with offset, to the second.
pub fn now() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Local::now().fixed_offset()
}

fn iso(t: &chrono::DateTime<chrono::FixedOffset>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

// ── checks ───────────────────────────────────────────────

/// Unbalanced braces, `$`, `\begin`/`\end`, or a whole document where a
/// fragment belongs.
fn structure(body: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |msg: &str| {
        out.push(Finding { code: "note_structure".into(), severity: "error".into(), message: msg.into(), blocking: true, subject: None, line: None, snippet: None });
    };
    let text: String = body.lines().map(lore_notation::source::strip_comment).collect::<Vec<_>>().join("\n");
    if !lore_notation::macros::balanced(&text) {
        push("大括號 { } 沒有成對");
    }
    let b = text.as_bytes();
    let (mut dollars, mut i) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'$' => {
                dollars += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if dollars % 2 == 1 {
        push("$ 沒有成對");
    }
    let mut stack: Vec<&str> = Vec::new();
    let mut bad_env = false;
    let mut rest = text.as_str();
    while let Some(p) = rest.find('\\') {
        rest = &rest[p + 1..];
        let (open, tail) = if let Some(t) = rest.strip_prefix("begin{") {
            (true, t)
        } else if let Some(t) = rest.strip_prefix("end{") {
            (false, t)
        } else {
            continue;
        };
        let Some(close) = tail.find('}') else { break };
        let env = &tail[..close];
        if open {
            stack.push(env);
        } else if stack.pop() != Some(env) {
            bad_env = true;
        }
    }
    if bad_env || !stack.is_empty() {
        push("\\begin 與 \\end 沒有成對");
    }
    if text.contains("\\documentclass") || text.contains("\\begin{document}") {
        push("筆記只放內文，不放 \\documentclass 或 \\begin{document}");
    }
    out
}

/// Labels the registry knows (equations, figures, tables): a `\ref` to one
/// is fine in a note.
fn registry_labels(n: &Notation) -> std::collections::BTreeSet<String> {
    n.entries.iter().filter_map(|e| e.label.clone()).collect()
}

/// Run the lore-notation checks on a note body against the registry the
/// caller loaded (the author's global `notation.json`). Blocking findings
/// (see [`BLOCKING`]) come first.
pub fn check(body: &str, notation: &Notation) -> Vec<Finding> {
    let mut out = structure(body);
    let known = registry_labels(notation);
    let report = check_document(Document::from_str("note.tex", body), notation, &CheckOptions::default());
    for i in report.issues {
        if i.code == "ref_undefined" && i.subject.as_deref().is_some_and(|s| known.contains(s)) {
            continue;
        }
        // about the whole registry, not this note (a note uses a few entries at most)
        if i.code == "unused_entry" || i.category == "registry" && i.severity != Severity::Error {
            continue;
        }
        let blocking = BLOCKING.contains(&i.code.as_str());
        let severity = match i.severity {
            _ if blocking => "error",
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let loc = i.locations.first();
        out.push(Finding {
            blocking,
            code: i.code,
            severity: severity.into(),
            message: i.message,
            subject: i.subject,
            line: loc.map(|l| l.line),
            snippet: loc.map(|l| l.snippet.clone()).filter(|s| !s.is_empty()),
        });
    }
    out.sort_by_key(|f| !f.blocking);
    out
}

/// Does any finding stop the save?
pub fn is_blocked(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.blocking)
}

// ── the folder ───────────────────────────────────────────

/// The notes folder (`<PaperComposer workspace>/notes`). Created on the first write.
#[derive(Debug, Clone)]
pub struct Notes {
    dir: PathBuf,
}

fn mtime_ms(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn title_from_body(body: &str) -> String {
    let first = body.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('%')).unwrap_or("");
    let t: String = one_line(first).chars().take(40).collect();
    t
}

impl Notes {
    pub fn new(dir: impl Into<PathBuf>) -> Notes {
        Notes { dir: dir.into() }
    }

    /// `<workspace>/notes`.
    pub fn in_workspace(workspace: &Path) -> Notes {
        Notes::new(workspace.join("notes"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::BadId(id.to_string()));
        }
        let p = self.dir.join(format!("{id}.tex"));
        if !p.is_file() {
            return Err(Error::NotFound(id.to_string()));
        }
        Ok(p)
    }

    /// Every note and idea, newest first (by `created`, then file time).
    /// Skips `.trash/`, hidden files and anything not `.tex`.
    pub fn list(&self) -> Vec<Note> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Note> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name()?.to_str()?;
                if name.starts_with('.') || !p.is_file() {
                    return None;
                }
                let id = name.strip_suffix(".tex")?;
                let text = std::fs::read_to_string(&p).ok()?;
                let mut n = parse(id, &text);
                n.modified = mtime_ms(&p);
                Some(n)
            })
            .collect();
        out.sort_by(|a, b| {
            let key = |n: &Note| chrono::DateTime::parse_from_rfc3339(&n.created).map(|t| t.timestamp_millis() as u64).unwrap_or(n.modified);
            key(b).cmp(&key(a)).then_with(|| b.id.cmp(&a.id))
        });
        out
    }

    pub fn get(&self, id: &str) -> Result<Note> {
        let p = self.path(id)?;
        let mut n = parse(id, &std::fs::read_to_string(&p)?);
        n.modified = mtime_ms(&p);
        Ok(n)
    }

    /// Check, then write a new file `<YYYY-MM-DD>-<slug>.tex` (never
    /// overwriting: `-2`, `-3` … when taken).
    pub fn create(&self, new: NewNote, notation: &Notation) -> Result<Saved> {
        self.create_at(new, notation, now())
    }

    /// [`Notes::create`] with a given time (tests).
    pub fn create_at(&self, new: NewNote, notation: &Notation, at: chrono::DateTime<chrono::FixedOffset>) -> Result<Saved> {
        let body = new.body.trim_end().to_string();
        let mut title = one_line(&new.title);
        if title.is_empty() {
            title = title_from_body(&body);
        }
        if title.is_empty() && body.trim().is_empty() {
            return Err(Error::Empty);
        }
        let findings = check(&body, notation);
        if is_blocked(&findings) {
            return Err(Error::Blocked(findings));
        }
        let mut stem_part = slug(&title);
        if stem_part.is_empty() {
            stem_part = new.kind.as_str().to_string();
        }
        std::fs::create_dir_all(&self.dir)?;
        let stem = format!("{}-{stem_part}", at.format("%Y-%m-%d"));
        let mut note = Note {
            id: String::new(),
            title,
            kind: new.kind,
            source: new.source.map(|s| one_line(&s)).filter(|s| !s.is_empty()).unwrap_or_else(|| "manual".into()),
            created: iso(&at),
            updated: None,
            kb: clean_list(new.kb),
            tags: clean_list(new.tags),
            extra: BTreeMap::new(),
            body,
            modified: 0,
        };
        // another writer (LoreEcho, PaperComposer) may take the free name first: try the next
        for _ in 0..100 {
            let path = unique_path(&self.dir, &stem);
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            note.id = id.clone();
            match write_new(&path, &to_tex(&note)) {
                Ok(()) => return Ok(Saved { id, path, findings }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(Error::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free note name")))
    }

    /// Check, then rewrite an existing note in place (the id stays when the title changes).
    pub fn update(&self, id: &str, edit: Edit, notation: &Notation) -> Result<Saved> {
        self.update_at(id, edit, notation, now())
    }

    pub fn update_at(&self, id: &str, edit: Edit, notation: &Notation, at: chrono::DateTime<chrono::FixedOffset>) -> Result<Saved> {
        let path = self.path(id)?;
        let mut n = parse(id, &std::fs::read_to_string(&path)?);
        if let Some(t) = edit.title {
            n.title = one_line(&t);
        }
        if let Some(b) = edit.body {
            n.body = b.trim_end().to_string();
        }
        if let Some(k) = edit.kind {
            n.kind = k;
        }
        if let Some(t) = edit.tags {
            n.tags = clean_list(t);
        }
        if let Some(k) = edit.kb {
            n.kb = clean_list(k);
        }
        if n.title.is_empty() {
            n.title = title_from_body(&n.body);
        }
        let findings = check(&n.body, notation);
        if is_blocked(&findings) {
            return Err(Error::Blocked(findings));
        }
        if n.source.is_empty() {
            n.source = "manual".into();
        }
        if n.created.is_empty() {
            n.created = iso(&at);
        }
        n.updated = Some(iso(&at));
        lore_notation::write_atomic(&path, &to_tex(&n))?;
        Ok(Saved { id: id.to_string(), path, findings })
    }

    /// Move a note to `.trash/` (a free name there); returns where it went.
    pub fn delete(&self, id: &str) -> Result<PathBuf> {
        let from = self.path(id)?;
        let trash = self.dir.join(TRASH);
        std::fs::create_dir_all(&trash)?;
        let to = unique_path(&trash, id);
        std::fs::rename(&from, &to)?;
        Ok(to)
    }
}

/// Write a file that must not exist yet (another writer may race us for the name).
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()
}

#[cfg(test)]
mod tests;
