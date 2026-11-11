//! Where LoreEcho runs from and what it serves.
//!
//! Priority for the port: command line (`--port`) > env (`LOREECHO_PORT`, `PORT`) > 7860
//! (browser mode; the desktop app binds a free port). Data: `private/LoreEcho`, created from
//! `private-example/LoreEcho` on first run (lore-home `seed_app`).

use std::path::{Path, PathBuf};

pub const APP: &str = "LoreEcho";
/// Short name in `ports.json` (no `lore-le://` link: other apps never open LoreEcho).
pub const SHORT: &str = "le";
pub const DEFAULT_PORT: u16 = 7860;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub web: PathBuf,
    pub home: lore_home::Home,
}

/// This app's folder (`LoreForge/LoreEcho`).
pub fn repo_dir() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    here.canonicalize().unwrap_or(here)
}

impl Config {
    pub fn load(args: &[String]) -> Result<Config, String> {
        let mut port = None;
        let mut i = 0;
        while i < args.len() {
            let val = args.get(i + 1).cloned();
            match args[i].as_str() {
                "--port" => port = Some(val.ok_or("--port needs a number")?.parse::<u16>().map_err(|e| e.to_string())?),
                other => return Err(format!("unknown argument `{other}`")),
            }
            i += 2;
        }
        let env_port = || ["LOREECHO_PORT", "PORT"].iter().find_map(|k| std::env::var(k).ok()?.trim().parse().ok());
        Ok(Config { port: port.or_else(env_port).unwrap_or(DEFAULT_PORT), web: repo_dir().join("web"), home: lore_home::Home::discover(&repo_dir()) })
    }
}
