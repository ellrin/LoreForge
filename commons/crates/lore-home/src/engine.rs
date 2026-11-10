//! The local model every LoreForge app uses, chosen once: `private/shared/engine.toml`
//!
//! ```toml
//! [engine]
//! url = "http://127.0.0.1:8080/v1"   # an OpenAI-compatible server (llama-server, Ollama, LM Studio)
//! model = "qwen3.5-9b-q8"            # optional: the catalog id or the server's model name
//! ```
//!
//! An app's own `[engine]` (its config.toml or environment) wins; this is the default
//! when it has none, so the author does not configure the same server in every app.
//! The app that manages a local model (PaperComposer) writes it with
//! [`Home::set_engine`] when the author picks or starts one. Read it on every use: the
//! server may move to another port. Never holds keys.

use crate::Home;
use serde::Deserialize;
use std::path::PathBuf;

/// The file in `private/shared` that holds the shared engine.
pub const FILE: &str = "engine.toml";

/// `[engine]` of [`FILE`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SharedEngine {
    /// Base URL ending in `/v1`.
    pub url: Option<String>,
    pub model: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EngineFile {
    engine: SharedEngine,
}

impl Home {
    fn engine_file(&self) -> Option<PathBuf> {
        self.shared_dir().map(|d| d.join(FILE))
    }

    /// The shared local engine, when one is set (a non-empty `url`).
    pub fn engine(&self) -> Option<SharedEngine> {
        let s = std::fs::read_to_string(self.engine_file()?).ok()?;
        let f: EngineFile = toml::from_str(&s).map_err(|e| eprintln!("lore-home: {FILE}: {e}")).ok()?;
        let clean = |v: Option<String>| v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let url = clean(f.engine.url)?;
        Some(SharedEngine { url: Some(url), model: clean(f.engine.model) })
    }

    /// Keep `url` / `model` as the shared engine (`url = None` clears it; other tables
    /// of the file stay).
    pub fn set_engine(&self, url: Option<&str>, model: Option<&str>) -> std::io::Result<()> {
        let path = self.engine_file().ok_or_else(|| std::io::Error::other("no shared settings folder"))?;
        let mut t: toml::Table = std::fs::read_to_string(&path).ok().and_then(|s| s.parse().ok()).unwrap_or_default();
        match url.map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => {
                let mut e = toml::Table::new();
                e.insert("url".into(), toml::Value::String(u.into()));
                if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
                    e.insert("model".into(), toml::Value::String(m.into()));
                }
                t.insert("engine".into(), toml::Value::Table(e));
            }
            None => {
                t.remove("engine");
            }
        }
        let dir = path.parent().ok_or_else(|| std::io::Error::other("no shared settings folder"))?;
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, t.to_string())?;
        std::fs::rename(&tmp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_round_trip_and_empty_values() {
        let d = tempfile::tempdir().unwrap();
        let home = Home::at(d.path());
        assert_eq!(home.engine(), None);
        home.set_engine(Some("http://127.0.0.1:8123/v1"), Some("qwen3.5-4b-q8")).unwrap();
        assert_eq!(home.engine(), Some(SharedEngine { url: Some("http://127.0.0.1:8123/v1".into()), model: Some("qwen3.5-4b-q8".into()) }));
        let f = d.path().join("private/shared/engine.toml");
        std::fs::write(&f, "note = \"kept\"\n[engine]\nurl = \"  \"\nmodel = \"x\"\n").unwrap();
        assert_eq!(home.engine(), None, "an empty url is no engine");
        home.set_engine(Some("http://127.0.0.1:9/v1"), None).unwrap();
        assert!(std::fs::read_to_string(&f).unwrap().contains("note = \"kept\""));
        assert_eq!(home.engine().unwrap().model, None);
        home.set_engine(None, None).unwrap();
        assert_eq!(home.engine(), None);
        std::fs::write(&f, "[engine]\nurl = 3\n").unwrap();
        assert_eq!(home.engine(), None, "a broken file is no engine");
    }
}
