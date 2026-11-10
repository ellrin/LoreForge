//! Where things are. Priority: environment > `config.toml` in the settings folder >
//! defaults. The settings folder is `$LOREREADER_HOME` or `<OS config dir>/LoreReader/`
//! (on macOS `~/Library/Application Support/LoreReader/`), never inside the repository.
//!
//! ```toml
//! library = "~/Documents/LoreReader/library"   # the PDFs (outside any git work tree)
//! exports = "~/Documents/LoreReader/exports"   # "Save as" copies
//! kb_url = "http://127.0.0.1:7700"             # KnowledgeBase server
//! claude_model = "claude-sonnet-…"             # claude:selected (else the catalog default)
//! [engine]                                     # local model (OpenAI-compatible server)
//! url = "http://127.0.0.1:8080/v1"
//! model = "qwen3.5-9b-q8"
//! ```
//!
//! Without `[engine]` the server uses the engine all LoreForge apps share,
//! `private/shared/engine.toml` (`lore_home::engine`).
//!
//! Env: `LOREREADER_LIBRARY`, `LOREREADER_EXPORTS`, `LOREREADER_KB_URL`,
//! `LOREREADER_LLM_URL`, `LOREREADER_LLM_MODEL`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const DEFAULT_KB_URL: &str = "http://127.0.0.1:7700";
pub const DEFAULT_PORT: u16 = 7890;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Base URL ending in /v1 (llama.cpp server, Ollama, LM Studio).
    pub url: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct File {
    library: Option<String>,
    exports: Option<String>,
    kb_url: Option<String>,
    claude_model: Option<String>,
    engine: EngineConfig,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub library: PathBuf,
    pub exports: PathBuf,
    pub kb_url: String,
    pub claude_model: Option<String>,
    pub engine: EngineConfig,
    /// The settings folder (config.toml, tasks.toml), when there is one.
    pub app_dir: Option<PathBuf>,
}

/// `$LOREREADER_HOME`, else `<OS config dir>/LoreReader`.
pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("LOREREADER_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join("LoreReader")))
}

/// Expand a leading `~/`.
pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(p)
}

fn documents() -> PathBuf {
    dirs::document_dir().or_else(|| dirs::home_dir().map(|h| h.join("Documents"))).unwrap_or_else(|| PathBuf::from("Documents")).join("LoreReader")
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

impl Config {
    /// From the environment and `<app dir>/config.toml`.
    pub fn load() -> Result<Config, String> {
        let dir = app_dir();
        Config::load_from(dir.as_deref())
    }

    /// From the environment and `dir/config.toml` (tests pass a temp dir).
    pub fn load_from(dir: Option<&Path>) -> Result<Config, String> {
        let file: File = match dir.map(|d| d.join("config.toml")).filter(|p| p.is_file()) {
            Some(p) => toml::from_str(&std::fs::read_to_string(&p).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", p.display()))?,
            None => File::default(),
        };
        let library = env("LOREREADER_LIBRARY").or(file.library).map(|s| expand_home(&s)).unwrap_or_else(|| documents().join("library"));
        let exports = env("LOREREADER_EXPORTS").or(file.exports).map(|s| expand_home(&s)).unwrap_or_else(|| documents().join("exports"));
        let kb_url = env("LOREREADER_KB_URL").or(file.kb_url).unwrap_or_else(|| DEFAULT_KB_URL.into()).trim_end_matches('/').to_string();
        let engine = EngineConfig { url: env("LOREREADER_LLM_URL").or(file.engine.url), model: env("LOREREADER_LLM_MODEL").or(file.engine.model) };
        Ok(Config { library, exports, kb_url, claude_model: file.claude_model, engine, app_dir: dir.map(Path::to_path_buf) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_values_and_defaults() {
        let d = tempfile::tempdir().unwrap();
        // Defaults (env vars are not set in tests).
        let c = Config::load_from(Some(d.path())).unwrap();
        assert!(c.library.ends_with("LoreReader/library"), "{}", c.library.display());
        assert_eq!(c.kb_url, DEFAULT_KB_URL);
        std::fs::write(d.path().join("config.toml"), "library = \"/x/lib\"\nkb_url = \"http://127.0.0.1:7999/\"\n[engine]\nurl = \"http://127.0.0.1:8080/v1\"\n").unwrap();
        let c = Config::load_from(Some(d.path())).unwrap();
        assert_eq!(c.library, PathBuf::from("/x/lib"));
        assert_eq!(c.kb_url, "http://127.0.0.1:7999");
        assert_eq!(c.engine.url.as_deref(), Some("http://127.0.0.1:8080/v1"));
        std::fs::write(d.path().join("config.toml"), "library = 3").unwrap();
        assert!(Config::load_from(Some(d.path())).is_err());
    }
}
