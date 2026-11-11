//! Characters (design §6): `private/LoreEcho/characters/<id>/`
//!
//! ```text
//! character.toml   name, persona (the system prompt's personality), voice (later), language
//! avatar/          idle | listening | thinking | speaking  .png .svg .webp .jpg
//! ```
//!
//! A state without its own picture shows `idle`. With no character at all the app still
//! talks, as [`Character::builtin`] (no picture: the page shows the app icon).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DIR: &str = "characters";
pub const FILE: &str = "character.toml";
pub const STATES: [&str; 4] = ["idle", "listening", "thinking", "speaking"];
/// Picture formats, in the order they are looked for.
const EXTS: [(&str, &str); 5] = [("svg", "image/svg+xml"), ("png", "image/png"), ("webp", "image/webp"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg")];
/// Id of [`Character::builtin`].
pub const BUILTIN: &str = "default";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct File {
    name: String,
    persona: String,
    voice: String,
    language: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    /// Not sent to the page (it is the system prompt's).
    #[serde(skip)]
    pub persona: String,
    /// Placeholder for the voice phase (design §7).
    pub voice: String,
    pub language: String,
    /// The states that have their own picture.
    pub avatars: Vec<String>,
}

impl Character {
    /// Used when no character folder can be read.
    pub fn builtin() -> Character {
        Character {
            id: BUILTIN.into(),
            name: "LoreEcho".into(),
            persona: "你是使用者的學習夥伴：安靜、好奇、有耐心。你陪他把想法說清楚，必要時反問一句，幫他找出還不懂的地方。".into(),
            voice: String::new(),
            language: "zh-TW".into(),
            avatars: vec![],
        }
    }
}

/// Folder names that can be a character id (no paths, no dots).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn folder(app_dir: &Path, id: &str) -> PathBuf {
    app_dir.join(DIR).join(id)
}

fn picture(dir: &Path, state: &str) -> Option<(PathBuf, &'static str)> {
    EXTS.iter().map(|(ext, mime)| (dir.join("avatar").join(format!("{state}.{ext}")), *mime)).find(|(p, _)| p.is_file())
}

/// One character, or why it cannot be read.
pub fn load(app_dir: &Path, id: &str) -> Result<Character, String> {
    if !valid_id(id) {
        return Err(format!("人物名稱不對：「{id}」"));
    }
    let dir = folder(app_dir, id);
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let f: File = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = f.name.trim();
    Ok(Character {
        id: id.into(),
        name: if name.is_empty() { id.into() } else { name.into() },
        persona: f.persona.trim().into(),
        voice: f.voice.trim().into(),
        language: Some(f.language.trim()).filter(|l| !l.is_empty()).unwrap_or("zh-TW").into(),
        avatars: STATES.iter().filter(|s| picture(&dir, s).is_some()).map(|s| s.to_string()).collect(),
    })
}

/// Every readable character, by id (a broken one is reported on stderr and left out).
pub fn list(app_dir: &Path) -> Vec<Character> {
    let Ok(rd) = std::fs::read_dir(app_dir.join(DIR)) else { return vec![] };
    let mut ids: Vec<String> = rd.flatten().filter(|e| e.path().join(FILE).is_file()).filter_map(|e| e.file_name().into_string().ok()).filter(|id| valid_id(id)).collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| match load(app_dir, &id) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("le-server: {e}");
                None
            }
        })
        .collect()
}

/// The character to talk as: `wanted` if it loads, else the first one, else the built-in.
pub fn pick(app_dir: Option<&Path>, wanted: Option<&str>) -> Character {
    let Some(dir) = app_dir else { return Character::builtin() };
    if let Some(id) = wanted.filter(|w| *w != BUILTIN)
        && let Ok(c) = load(dir, id)
    {
        return c;
    }
    list(dir).into_iter().next().unwrap_or_else(Character::builtin)
}

/// The picture for `state` (`idle`'s when it has none): path and content type. `None` for
/// an unknown state, id or character without pictures.
pub fn avatar(app_dir: &Path, id: &str, state: &str) -> Option<(PathBuf, &'static str)> {
    if !valid_id(id) || !STATES.contains(&state) {
        return None;
    }
    let dir = folder(app_dir, id);
    picture(&dir, state).or_else(|| picture(&dir, "idle"))
}
