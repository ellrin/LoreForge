//! The two small files LoreEcho keeps in `private/LoreEcho/` (never a conversation):
//!
//! ```toml
//! # state.toml — where the bubble is (written by the app)
//! [window]
//! x = 1600.0          # the bubble's top-left corner, logical pixels; missing = bottom-right
//! y = 940.0
//! expanded = false
//! w = 620.0           # the open window's size (420..1000), missing = 620 × 620
//! h = 620.0
//!
//! # settings.toml — the author's choices (the gear's 一般 tab)
//! character = "echo"
//! shortcut = "Alt+Space"
//! ```
//!
//! Writes are atomic (a temporary file, then a rename); keys this version does not know are
//! kept. A missing or broken file reads as the defaults.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const STATE_FILE: &str = "state.toml";
pub const SETTINGS_FILE: &str = "settings.toml";
pub const DEFAULT_SHORTCUT: &str = "Alt+Space";
/// Positions further out than this are nonsense (a broken file, a bad request).
const MAX_COORD: f64 = 100_000.0;
/// The open window's size limits (logical pixels) and its default.
pub const MIN_WIN: f64 = 420.0;
pub const MAX_WIN: f64 = 1000.0;
pub const DEFAULT_WIN: f64 = 620.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    pub expanded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
}

/// A change to the window state; missing fields stay as they are.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowPatch {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub expanded: Option<bool>,
    pub w: Option<f64>,
    pub h: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The character's folder name; `None` = the first one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    /// Shows / hides LoreEcho from anywhere ("" = no shortcut).
    pub shortcut: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { character: None, shortcut: DEFAULT_SHORTCUT.into() }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SettingsPatch {
    pub character: Option<String>,
    pub shortcut: Option<String>,
}

fn read_table(path: &Path) -> toml::Table {
    let Ok(s) = std::fs::read_to_string(path) else { return toml::Table::new() };
    s.parse().unwrap_or_else(|e| {
        eprintln!("le-server: {}: {e} (using defaults)", path.display());
        toml::Table::new()
    })
}

fn write_table(path: &Path, t: &toml::Table) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, t.to_string())?;
    std::fs::rename(&tmp, path)
}

/// `t` with `value`'s fields put over it (other keys kept; `None` fields removed).
fn merge<T: Serialize>(t: &mut toml::Table, value: &T, keys: &[&str]) {
    let new = toml::Table::try_from(value).unwrap_or_default();
    for k in keys {
        match new.get(*k) {
            Some(v) => {
                t.insert((*k).into(), v.clone());
            }
            None => {
                t.remove(*k);
            }
        }
    }
}

fn finite(v: f64) -> Result<f64, String> {
    if v.is_finite() && v.abs() <= MAX_COORD { Ok(v) } else { Err(format!("位置不對：{v}")) }
}

/// A window size, kept inside the limits.
fn size(v: f64) -> Result<f64, String> {
    if v.is_finite() { Ok(v.clamp(MIN_WIN, MAX_WIN)) } else { Err(format!("大小不對：{v}")) }
}

impl WindowState {
    /// The open window's size: the saved one inside the limits, else 620 × 620.
    pub fn size(&self) -> (f64, f64) {
        (self.w.unwrap_or(DEFAULT_WIN).clamp(MIN_WIN, MAX_WIN), self.h.unwrap_or(DEFAULT_WIN).clamp(MIN_WIN, MAX_WIN))
    }

    fn clean(mut self) -> WindowState {
        self.w = self.w.filter(|v| v.is_finite()).map(|v| v.clamp(MIN_WIN, MAX_WIN));
        self.h = self.h.filter(|v| v.is_finite()).map(|v| v.clamp(MIN_WIN, MAX_WIN));
        self.x = self.x.filter(|v| finite(*v).is_ok());
        self.y = self.y.filter(|v| finite(*v).is_ok());
        if self.x.is_none() || self.y.is_none() {
            (self.x, self.y) = (None, None);
        }
        self
    }
}

pub fn window(dir: &Path) -> WindowState {
    let t = read_table(&dir.join(STATE_FILE));
    t.get("window").and_then(|w| w.clone().try_into::<WindowState>().ok()).unwrap_or_default().clean()
}

/// Apply `p` and write `state.toml`; the new state.
pub fn set_window(dir: &Path, p: &WindowPatch) -> Result<WindowState, String> {
    let mut w = window(dir);
    if let Some(x) = p.x {
        w.x = Some(finite(x)?);
    }
    if let Some(y) = p.y {
        w.y = Some(finite(y)?);
    }
    if let Some(e) = p.expanded {
        w.expanded = e;
    }
    if let Some(v) = p.w {
        w.w = Some(size(v)?);
    }
    if let Some(v) = p.h {
        w.h = Some(size(v)?);
    }
    let w = w.clean();
    let path = dir.join(STATE_FILE);
    let mut t = read_table(&path);
    let mut wt = t.get("window").and_then(|v| v.as_table()).cloned().unwrap_or_default();
    merge(&mut wt, &w, &["x", "y", "expanded", "w", "h"]);
    t.insert("window".into(), toml::Value::Table(wt));
    write_table(&path, &t).map_err(|e| format!("無法寫入 {STATE_FILE}：{e}"))?;
    Ok(w)
}

pub fn settings(dir: &Path) -> Settings {
    let t = read_table(&dir.join(SETTINGS_FILE));
    toml::Value::Table(t).try_into::<Settings>().unwrap_or_default()
}

/// Apply `p` and write `settings.toml`. The shortcut is checked here ([`check_shortcut`]);
/// the character by the caller (it must exist).
pub fn set_settings(dir: &Path, p: &SettingsPatch) -> Result<Settings, String> {
    let mut s = settings(dir);
    if let Some(c) = &p.character {
        s.character = Some(c.trim().to_string()).filter(|c| !c.is_empty());
    }
    if let Some(k) = &p.shortcut {
        s.shortcut = check_shortcut(k)?;
    }
    let path = dir.join(SETTINGS_FILE);
    let mut t = read_table(&path);
    merge(&mut t, &s, &["character", "shortcut"]);
    write_table(&path, &t).map_err(|e| format!("無法寫入 {SETTINGS_FILE}：{e}"))?;
    Ok(s)
}

const MODIFIERS: [&str; 10] = ["alt", "option", "ctrl", "control", "shift", "cmd", "command", "super", "meta", "cmdorctrl"];

/// `Alt+Space`, `CmdOrCtrl+Shift+E`, `F9`…: modifiers joined with `+`, then one key (a
/// letter, a digit, F1–F24 or a named key). Returns it trimmed; "" = no shortcut. The
/// desktop app parses it again with the global-shortcut plugin (it knows the exact list).
pub fn check_shortcut(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let bad = || format!("快捷鍵的寫法不對：「{s}」（例如 Alt+Space）");
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or_else(bad)?;
    let mut seen = Vec::new();
    for m in mods {
        let m = m.to_ascii_lowercase();
        if !MODIFIERS.contains(&m.as_str()) || seen.contains(&m) {
            return Err(bad());
        }
        seen.push(m);
    }
    let k = key.to_ascii_lowercase();
    let named = ["space", "enter", "tab", "escape", "backquote", "comma", "period", "slash", "semicolon", "quote", "minus", "equal", "up", "down", "left", "right"];
    let fkey = k.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n));
    let single = k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric());
    if !(single || fkey || named.contains(&k.as_str())) {
        return Err(bad());
    }
    // a bare letter or space would swallow typing everywhere
    if mods.is_empty() && !fkey {
        return Err(format!("快捷鍵要加上修飾鍵（Alt、Ctrl、Shift、Cmd）：「{s}」"));
    }
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts() {
        for ok in ["Alt+Space", "CmdOrCtrl+Shift+E", "F9", "ctrl+alt+1", ""] {
            assert!(check_shortcut(ok).is_ok(), "{ok}");
        }
        for bad in ["Space", "e", "Alt+", "Alt+Alt+E", "Hyper+E", "Alt+Space+E", "Alt+Home2"] {
            assert!(check_shortcut(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn half_a_position_is_none() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(STATE_FILE), "[window]\nx = 10.0\nexpanded = true\n").unwrap();
        assert_eq!(window(d.path()), WindowState { x: None, y: None, expanded: true, w: None, h: None });
        std::fs::write(d.path().join(STATE_FILE), "not toml [").unwrap();
        assert_eq!(window(d.path()), WindowState::default());
    }

    #[test]
    fn unknown_keys_are_kept() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(SETTINGS_FILE), "later = 1\n").unwrap();
        set_settings(d.path(), &SettingsPatch { character: Some("echo".into()), shortcut: None }).unwrap();
        let s = std::fs::read_to_string(d.path().join(SETTINGS_FILE)).unwrap();
        assert!(s.contains("later = 1") && s.contains("character = \"echo\"") && s.contains("Alt+Space"), "{s}");
    }
}
