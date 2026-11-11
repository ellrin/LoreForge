//! Characters (design §6): `private/LoreEcho/characters/<id>/`
//!
//! ```text
//! character.toml   name, persona (the system prompt's personality), voice (later), language,
//!                  figure_scale, figure_offset_y (how big the full body stands in the open window)
//! avatar/          idle | listening | thinking | speaking  .png .svg .webp .jpg   (portrait: the bubble)
//! full/            the same states, full body (head to feet, transparent background): the open window
//! ```
//!
//! A state without its own picture shows `idle`. A character without `full/` shows its
//! portrait in the open window too. With no character at all the app still talks, as
//! [`Character::builtin`] (no picture: the page shows the app icon).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DIR: &str = "characters";
pub const FILE: &str = "character.toml";
pub const STATES: [&str; 4] = ["idle", "listening", "thinking", "speaking"];
/// Picture formats, in the order they are looked for.
const EXTS: [(&str, &str); 5] = [("svg", "image/svg+xml"), ("png", "image/png"), ("webp", "image/webp"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg")];
/// The portrait folder (bubble) and the full-body folder (open window).
pub const AVATAR: &str = "avatar";
pub const FULL: &str = "full";
/// Id of [`Character::builtin`].
pub const BUILTIN: &str = "default";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct File {
    name: String,
    persona: String,
    voice: String,
    language: String,
    figure_scale: Option<f64>,
    figure_offset_y: Option<f64>,
}

/// How the full body stands in the open window (a visual-novel sprite): `scale` 1.0 = head
/// to feet fit the window height, more = bigger, the legs run off the window's bottom edge;
/// `offset_y` moves it down by that fraction of the window height (negative = up).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Figure {
    pub scale: f64,
    pub offset_y: f64,
}

impl Default for Figure {
    fn default() -> Figure {
        Figure { scale: 1.0, offset_y: 0.0 }
    }
}

impl Figure {
    pub const SCALE: (f64, f64) = (0.5, 3.0);
    pub const OFFSET_Y: (f64, f64) = (-0.5, 0.5);

    /// From character.toml: missing or not a number → default, out of range → clamped.
    fn from(scale: Option<f64>, offset_y: Option<f64>) -> Figure {
        let pick = |v: Option<f64>, d: f64, (lo, hi): (f64, f64)| v.filter(|v| v.is_finite()).map_or(d, |v| v.clamp(lo, hi));
        Figure { scale: pick(scale, 1.0, Figure::SCALE), offset_y: pick(offset_y, 0.0, Figure::OFFSET_Y) }
    }
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
    /// The states that have their own portrait (`avatar/`).
    pub avatars: Vec<String>,
    /// How the full body stands in the open window.
    pub figure: Figure,
    /// The states that have their own full-body picture (`full/`); empty = no full body
    /// (the open window then shows the portrait).
    pub full: Vec<String>,
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
            figure: Figure::default(),
            full: vec![],
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

/// `dir/<kind>/<state>.<ext>` (kind = [`AVATAR`] or [`FULL`]), the first format found.
fn picture(dir: &Path, kind: &str, state: &str) -> Option<(PathBuf, &'static str)> {
    EXTS.iter().map(|(ext, mime)| (dir.join(kind).join(format!("{state}.{ext}")), *mime)).find(|(p, _)| p.is_file())
}

fn states_with(dir: &Path, kind: &str) -> Vec<String> {
    STATES.iter().filter(|s| picture(dir, kind, s).is_some()).map(|s| s.to_string()).collect()
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
        avatars: states_with(&dir, AVATAR),
        figure: Figure::from(f.figure_scale, f.figure_offset_y),
        full: states_with(&dir, FULL),
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
    picture(&dir, AVATAR, state).or_else(|| picture(&dir, AVATAR, "idle"))
}

/// The full-body picture for `state`: `full/<state>`, else `full/idle`; a character with no
/// full body at all gets its portrait ([`avatar`]) instead. `None` as for [`avatar`].
pub fn full_body(app_dir: &Path, id: &str, state: &str) -> Option<(PathBuf, &'static str)> {
    if !valid_id(id) || !STATES.contains(&state) {
        return None;
    }
    let dir = folder(app_dir, id);
    picture(&dir, FULL, state).or_else(|| picture(&dir, FULL, "idle")).or_else(|| avatar(app_dir, id, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(toml: &str) -> Character {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(DIR).join("x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), toml).unwrap();
        load(tmp.path(), "x").unwrap()
    }

    #[test]
    fn figure_defaults_and_limits() {
        assert_eq!(one("name = \"x\"\n").figure, Figure { scale: 1.0, offset_y: 0.0 });
        assert_eq!(one("figure_scale = 1.35\nfigure_offset_y = -0.1\n").figure, Figure { scale: 1.35, offset_y: -0.1 });
        assert_eq!(one("figure_scale = 2\n").figure.scale, 2.0, "an integer is fine");
        assert_eq!(one("figure_scale = 9.0\nfigure_offset_y = 3.0\n").figure, Figure { scale: 3.0, offset_y: 0.5 });
        assert_eq!(one("figure_scale = 0.1\nfigure_offset_y = -3.0\n").figure, Figure { scale: 0.5, offset_y: -0.5 });
        assert_eq!(one("figure_scale = nan\n").figure.scale, 1.0);
        assert_eq!(Character::builtin().figure, Figure::default());
    }

    #[test]
    fn a_wrong_type_is_an_error_like_any_other_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(DIR).join("x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "figure_scale = \"big\"\n").unwrap();
        assert!(load(tmp.path(), "x").is_err());
    }
}
