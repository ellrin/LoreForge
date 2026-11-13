//! LoreReader's files, read-only, without LoreReader running (the same places SlideComposer's
//! `reader.rs` reads; `docs/book.md` `source`):
//!
//! - per paper: `private/LoreReader/papers/<sha256>/{paper,symbols,quotes,annotations,reading,chat}.json`
//!   (`LOREREADER_WORKSPACE`, else lore-home's `private/LoreReader`; never created or seeded);
//! - the PDF: `<library>/<sha[..2]>/<sha>.pdf`, `<library>` = `LOREREADER_LIBRARY` →
//!   `library = "…"` in `<LOREREADER_HOME or OS config dir/LoreReader>/config.toml` →
//!   `~/Documents/LoreReader/library` (the order of `lr_core::config`).
//!
//! Nothing here writes into LoreReader's folders.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn user_home() -> Option<PathBuf> {
    env("HOME").or_else(|| env("USERPROFILE")).map(PathBuf::from)
}

fn expand_home(p: &str, home: Option<&Path>) -> PathBuf {
    match (p.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(p),
    }
}

/// The OS settings folder (what `dirs::config_dir` returns).
fn config_dir(home: Option<&Path>) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA").map(PathBuf::from)
    } else {
        env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))
    }
}

/// `library = "…"` at the top level of LoreReader's config.toml (before any `[table]`).
pub fn config_library(toml: &str) -> Option<String> {
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            break;
        }
        let Some(rest) = l.strip_prefix("library") else { continue };
        let Some(v) = rest.trim_start().strip_prefix('=') else { continue };
        let v = v.trim();
        let q = v.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = v[1..].find(q)?;
        return Some(v[1..1 + end].to_string()).filter(|s| !s.is_empty());
    }
    None
}

/// LoreReader's PDF library. `getenv` and `home` are parameters so tests can pass their own.
pub fn library_dir(getenv: &dyn Fn(&str) -> Option<String>, home: Option<&Path>) -> PathBuf {
    if let Some(l) = getenv("LOREREADER_LIBRARY") {
        return expand_home(&l, home);
    }
    let app = getenv("LOREREADER_HOME").map(PathBuf::from).or_else(|| config_dir(home).map(|d| d.join("LoreReader")));
    if let Some(l) = app.and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok()).and_then(|t| config_library(&t)) {
        return expand_home(&l, home);
    }
    home.map(|h| h.join("Documents")).unwrap_or_else(|| PathBuf::from("Documents")).join("LoreReader/library")
}

pub fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// LoreReader's folders (read-only).
#[derive(Debug, Clone, PartialEq)]
pub struct Reader {
    /// `private/LoreReader/papers`
    pub papers: PathBuf,
    /// The PDF library.
    pub library: PathBuf,
}

/// One question the author asked about a paper (問這篇, `chat.json`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Question {
    pub text: String,
    /// The passage it was about, when the question had one.
    #[serde(default)]
    pub about: String,
}

impl Reader {
    /// `LOREREADER_WORKSPACE`, else lore-home's `private/LoreReader`; the library as LoreReader finds it.
    pub fn discover(home: &lore_home::Home) -> Option<Reader> {
        let ws = env("LOREREADER_WORKSPACE").map(PathBuf::from).or_else(|| home.app_dir("LoreReader"))?;
        Some(Reader { papers: ws.join("papers"), library: library_dir(&env, user_home().as_deref()) })
    }

    fn json(&self, sha: &str, file: &str) -> Option<Value> {
        if !valid_sha(sha) {
            return None;
        }
        serde_json::from_slice(&std::fs::read(self.papers.join(sha).join(file)).ok()?).ok()
    }

    /// The paper's PDF in the library, when it is there.
    pub fn pdf(&self, sha: &str) -> Option<PathBuf> {
        if !valid_sha(sha) {
            return None;
        }
        Some(self.library.join(&sha[..2]).join(format!("{sha}.pdf"))).filter(|p| p.is_file())
    }

    pub fn paper_json(&self, sha: &str) -> Option<Value> {
        self.json(sha, "paper.json")
    }

    pub fn symbols_json(&self, sha: &str) -> Option<Value> {
        self.json(sha, "symbols.json")
    }

    /// Paragraph ids the author marked 懂了 (`reading.json` `understood`).
    pub fn understood(&self, sha: &str) -> Vec<String> {
        self.json(sha, "reading.json").and_then(|v| v.get("understood").and_then(Value::as_array).cloned()).unwrap_or_default().iter().filter_map(Value::as_str).map(String::from).collect()
    }

    /// The author's own questions in 問這篇 (`chat.json`, role `user`), oldest first.
    pub fn questions(&self, sha: &str) -> Vec<Question> {
        let Some(v) = self.json(sha, "chat.json") else { return vec![] };
        v.get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("user"))
            .filter_map(|m| {
                let text = m.get("text").and_then(Value::as_str)?.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                let about = m.get("attachments").and_then(Value::as_array).and_then(|a| a.first()).and_then(|a| a.get("text")).and_then(Value::as_str).unwrap_or("").trim().to_string();
                Some(Question { text, about })
            })
            .collect()
    }

    /// Every paper with a paper.json, newest first.
    pub fn list(&self) -> Vec<Value> {
        let mut out: Vec<Value> = std::fs::read_dir(&self.papers)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let sha = e.file_name().to_string_lossy().to_string();
                self.json(&sha, "paper.json").map(|mut m| {
                    m["sha256"] = sha.into();
                    m
                })
            })
            .collect();
        out.sort_by(|a, b| b["added_at"].as_str().unwrap_or("").cmp(a["added_at"].as_str().unwrap_or("")));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_follows_lorereader_config_order() {
        let home = Path::new("/home/ellrin");
        let none = |_: &str| None;
        assert_eq!(library_dir(&none, Some(home)), home.join("Documents/LoreReader/library"));
        let env = |k: &str| (k == "LOREREADER_LIBRARY").then(|| "~/lib".to_string());
        assert_eq!(library_dir(&env, Some(home)), home.join("lib"));
        assert_eq!(config_library("library = \"/x/y\"\n[engine]\nlibrary = \"/no\""), Some("/x/y".into()));
        assert_eq!(config_library("[engine]\nlibrary = \"/no\""), None);
    }

    #[test]
    fn reading_and_questions_from_disk() {
        let d = tempfile::tempdir().unwrap();
        let sha = "a".repeat(64);
        let p = d.path().join("papers").join(&sha);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("reading.json"), r#"{"schema":"x","understood":["p1","p3"]}"#).unwrap();
        std::fs::write(
            p.join("chat.json"),
            r#"{"schema":"lorereader.chat/1","messages":[{"role":"user","text":"Why a temperature?","attachments":[{"text":"we divide by tau","page":2,"context":""}]},{"role":"assistant","text":"Because…"},{"role":"user","text":"  "}]}"#,
        )
        .unwrap();
        let r = Reader { papers: d.path().join("papers"), library: d.path().join("lib") };
        assert_eq!(r.understood(&sha), ["p1", "p3"]);
        assert_eq!(r.questions(&sha), [Question { text: "Why a temperature?".into(), about: "we divide by tau".into() }]);
        assert!(r.pdf(&sha).is_none() && r.understood("nope").is_empty());
    }
}
