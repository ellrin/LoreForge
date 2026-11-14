//! Every proposal is one JSON line (PaperComposer: the paper's `draft/polish.jsonl`;
//! SlideComposer: `decks/<name>/polish-log.jsonl`). A line changes only when it is applied or
//! liked (the file is rewritten atomically); nothing is ever removed.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Earlier proposals of one passage shown in the dialog.
pub const HISTORY: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    /// The passage the conversation started from ([`passage_id`] of its first text).
    pub passage: String,
    pub time: String,
    pub instruction: String,
    pub before: String,
    pub after: String,
    pub applied: bool,
    pub liked: bool,
    /// The provider that wrote it (`claude-code:…`, `llama-server:…`, `mock…`).
    pub engine: String,
    /// Why it could not be applied (empty = it could).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

/// A short stable id of a passage's text.
pub fn passage_id(text: &str) -> String {
    // FNV-1a 64 over the trimmed text
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.trim().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let ms = chrono::Utc::now().timestamp_millis();
    format!("{ms:x}-{}", N.fetch_add(1, Ordering::Relaxed))
}

pub fn read(path: &Path) -> Vec<Entry> {
    std::fs::read_to_string(path).map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

pub fn append(path: &Path, e: &Entry) -> Result<(), String> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(f, "{}", serde_json::to_string(e).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))
}

/// Change the line with `id` (unknown lines are kept as they are); the changed entry.
pub fn update(path: &Path, id: &str, f: impl FnOnce(&mut Entry)) -> Result<Entry, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = String::new();
    let mut found = None;
    let mut f = Some(f);
    for line in raw.lines() {
        match serde_json::from_str::<Entry>(line) {
            Ok(mut e) if e.id == id && found.is_none() => {
                if let Some(f) = f.take() {
                    f(&mut e);
                }
                out += &serde_json::to_string(&e).map_err(|e| e.to_string())?;
                found = Some(e);
            }
            _ => out += line,
        }
        out.push('\n');
    }
    let Some(e) = found else { return Err("找不到這次的改寫".into()) };
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(e)
}

/// The passage a text belongs to: the conversation whose proposal (or start) it is, the
/// newest first; else a new passage.
pub fn passage_of(entries: &[Entry], text: &str) -> String {
    let t = text.trim();
    entries.iter().rev().find(|e| e.after.trim() == t || e.before.trim() == t).map(|e| e.passage.clone()).unwrap_or_else(|| passage_id(text))
}

/// The last [`HISTORY`] proposals of `passage`, oldest first.
pub fn history(entries: &[Entry], passage: &str) -> Vec<Entry> {
    let mine: Vec<&Entry> = entries.iter().filter(|e| e.passage == passage).collect();
    mine[mine.len().saturating_sub(HISTORY)..].iter().map(|e| (*e).clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_appended_marked_and_grouped_by_passage() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("draft/polish.jsonl");
        let first = "The loss drops quickly.";
        let pid = passage_id(first);
        let a = Entry { id: new_id(), passage: pid.clone(), before: first.into(), after: "The loss falls fast.".into(), instruction: "shorter".into(), ..Default::default() };
        let b = Entry { id: new_id(), passage: pid.clone(), before: first.into(), after: "Loss falls fast.".into(), instruction: "even shorter".into(), ..Default::default() };
        let other = Entry { id: new_id(), passage: passage_id("x"), before: "x".into(), after: "y".into(), ..Default::default() };
        for e in [&a, &b, &other] {
            append(&p, e).unwrap();
        }
        let e = update(&p, &a.id, |e| e.applied = true).unwrap();
        assert!(e.applied);
        let all = read(&p);
        assert_eq!(all.len(), 3);
        assert!(all[0].applied && !all[1].applied);
        assert_eq!(history(&all, &pid).iter().map(|e| e.after.as_str()).collect::<Vec<_>>(), ["The loss falls fast.", "Loss falls fast."]);
        // the applied text later selected again belongs to the same conversation
        assert_eq!(passage_of(&all, "The loss falls fast."), pid);
        assert_eq!(passage_of(&all, "Something new."), passage_id("Something new."));
        assert!(update(&p, "nope", |_| {}).is_err());
    }
}
