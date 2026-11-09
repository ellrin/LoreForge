//! Where the spaces are and who is using the app.
//!
//! Priority: command line (`--space <dir>` repeatable, `--member`, `--port`) > env
//! (`KB_SPACES` = dirs separated by `:`, `KB_MEMBER`, `PORT`) > `config.toml` in
//! `<app dir>` > LoreForge `private/KnowledgeBase` (seeded from `private-example/KnowledgeBase`,
//! see `lore-home`). The member defaults to `id` in `private/shared/profile.toml`. `<app dir>` is `$KB_HOME` or
//! `<OS config dir>/KnowledgeBase/` (also holds `reminders.toml`, see `reminders.rs`).
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
    /// App settings folder (reminders, state); `None` = nothing persists.
    pub app_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    member: Option<String>,
    spaces: Vec<String>,
    port: Option<u16>,
    intervals: Option<Intervals>,
}

/// This app's folder (`LoreForge/KnowledgeBase`).
pub fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn config_file() -> Option<PathBuf> {
    crate::reminders::app_dir().map(|d| d.join("config.toml"))
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
            spaces = file.spaces.iter().map(|s| lore_store::expand_home(s)).collect();
        }
        let home = lore_home::Home::discover(&repo_dir());
        if spaces.is_empty() {
            // LoreForge: private/KnowledgeBase, created from private-example/KnowledgeBase the first time.
            match home.seed_app("KnowledgeBase") {
                Ok(Some(private)) => {
                    let private = private.canonicalize().unwrap_or(private);
                    if private.join("space.toml").is_file() {
                        spaces.push(private);
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("kb-server: private/KnowledgeBase: {e}"),
            }
        }
        // Who: --member > KB_MEMBER > config.toml > private/shared/profile.toml (id).
        let member = member
            .or_else(|| std::env::var("KB_MEMBER").ok())
            .or(file.member)
            .or_else(|| home.prefs().profile.id)
            .unwrap_or_else(|| "me".into());
        let port = port.or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok())).or(file.port).unwrap_or(7700);
        Ok(Config { spaces, member, port, web: repo_dir().join("web"), intervals: file.intervals.unwrap_or_default(), app_dir: crate::reminders::app_dir() })
    }
}
