//! Where the spaces are and who is using the app.
//!
//! Priority: command line (`--space <dir>` repeatable, `--member`, `--port`) > env
//! (`KB_SPACES` = dirs separated by `:`, `KB_MEMBER`, `PORT`) > `config.toml` in
//! `<config dir>/KnowledgeBase/` > the sibling `../private` of this repo.
//!
//! ```toml
//! member = "ellrin"
//! spaces = ["/path/to/KnowledgeBase/private"]
//! [intervals]
//! first_check_days = 1
//! maintenance = false
//! ```

use std::path::{Path, PathBuf};

use kb_core::Intervals;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub spaces: Vec<PathBuf>,
    pub member: String,
    pub port: u16,
    pub web: PathBuf,
    pub intervals: Intervals,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    member: Option<String>,
    spaces: Vec<String>,
    port: Option<u16>,
    intervals: Option<Intervals>,
}

/// `KnowledgeBase/public` (this repo).
pub fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn config_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("KnowledgeBase").join("config.toml"))
}

impl Config {
    pub fn load(args: &[String]) -> Result<Config, String> {
        let file: File = match config_file().filter(|p| p.is_file()) {
            Some(p) => toml::from_str(&std::fs::read_to_string(&p).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", p.display()))?,
            None => File::default(),
        };
        let mut spaces: Vec<PathBuf> = Vec::new();
        let mut member = None;
        let mut port = None;
        let mut i = 0;
        while i < args.len() {
            let val = args.get(i + 1).cloned();
            match args[i].as_str() {
                "--space" => spaces.push(PathBuf::from(val.ok_or("--space needs a folder")?)),
                "--member" => member = Some(val.ok_or("--member needs an id")?),
                "--port" => port = Some(val.ok_or("--port needs a number")?.parse::<u16>().map_err(|e| e.to_string())?),
                other => return Err(format!("unknown argument `{other}`")),
            }
            i += 2;
        }
        if spaces.is_empty() && let Ok(v) = std::env::var("KB_SPACES") {
            spaces = v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        }
        if spaces.is_empty() {
            spaces = file.spaces.iter().map(|s| ym_store::expand_home(s)).collect();
        }
        if spaces.is_empty() {
            let private = repo_dir().join("../private");
            let private = private.canonicalize().unwrap_or(private);
            if private.join("space.toml").is_file() {
                spaces.push(private);
            }
        }
        let member = member.or_else(|| std::env::var("KB_MEMBER").ok()).or(file.member).unwrap_or_else(|| "ellrin".into());
        let port = port.or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok())).or(file.port).unwrap_or(7700);
        Ok(Config { spaces, member, port, web: repo_dir().join("web"), intervals: file.intervals.unwrap_or_default() })
    }
}
