//! The colour theme every LoreForge app shows: one choice, kept in
//! `private/shared/ui.toml` (`theme = "<id>"`), so changing it in one app
//! changes it in all of them.
//!
//! The palettes live in `commons/ui/theme.css` and the page side (apply before
//! first paint, the 外觀 picker, follow the choice on focus) in
//! `commons/ui/theme.js`; both are compiled in here so a server only needs
//! three routes: `/theme.css` ([`CSS`]), `/theme.js` ([`script`]) and
//! `GET`/`PUT /api/theme` ([`Home::theme`], [`Home::set_theme`]).

use crate::Home;
use std::path::PathBuf;

/// `commons/ui/theme.css`.
pub const CSS: &str = include_str!("../../../ui/theme.css");
/// `commons/ui/theme.js` (without the choice; see [`script`]).
pub const JS: &str = include_str!("../../../ui/theme.js");
/// The file in `private/shared` that holds interface choices.
pub const FILE: &str = "ui.toml";

/// Every theme id, in the order of `theme.css`: dark single-hue, dark
/// combinations, light single-hue, light combinations.
pub const THEMES: [&str; 20] = [
    "hacker", "amber", "ice", "ultraviolet", "redalert",
    "neon", "shibuya", "hongkong", "synthwave", "aurora",
    "lavender", "lemon", "mint", "blueberry", "strawberry",
    "giftbox", "peachlavender", "sakuramatcha", "blueberrylemon", "caramelpistachio",
];
/// What a fresh install shows (also the stylesheet's fallback).
pub const DEFAULT: &str = "giftbox";

/// A current id as it is; a palette of the first version (`orange` / `night` /
/// `terminal`, with `light` true for its light mode) as the nearest theme.
pub fn nearest(id: &str, light: bool) -> Option<&'static str> {
    if let Some(t) = THEMES.iter().find(|t| **t == id) {
        return Some(t);
    }
    Some(match (id, light) {
        ("orange", false) => "amber",
        ("orange", true) => "peachlavender",
        ("night", false) => "neon",
        ("night", true) => "lavender",
        ("terminal", false) => "hacker",
        ("terminal", true) => "mint",
        _ => return None,
    })
}

#[derive(Debug)]
pub enum ThemeError {
    /// Not one of [`THEMES`].
    Unknown(String),
    /// The app does not run from a LoreForge folder: there is nowhere shared to keep it.
    NoHome,
    Io(std::io::Error),
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::Unknown(id) => write!(f, "unknown theme: {id}"),
            ThemeError::NoHome => write!(f, "no shared settings folder"),
            ThemeError::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ThemeError {}

impl Home {
    fn ui_file(&self) -> Option<PathBuf> {
        self.shared_dir().map(|d| d.join(FILE))
    }

    fn ui_table(&self) -> toml::Table {
        self.ui_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.parse().ok()).unwrap_or_default()
    }

    /// The shared choice; `None` until one was made (pages then show [`DEFAULT`]).
    pub fn theme(&self) -> Option<&'static str> {
        let t = self.ui_table();
        let light = t.get("mode").and_then(|m| m.as_str()) == Some("light");
        nearest(t.get("theme")?.as_str()?, light)
    }

    /// Keep `id` as the shared choice (other keys of `ui.toml` stay).
    pub fn set_theme(&self, id: &str) -> Result<&'static str, ThemeError> {
        let id = *THEMES.iter().find(|t| **t == id).ok_or_else(|| ThemeError::Unknown(id.to_string()))?;
        let path = self.ui_file().ok_or(ThemeError::NoHome)?;
        let mut t = self.ui_table();
        t.insert("theme".into(), toml::Value::String(id.into()));
        t.remove("mode");
        let dir = path.parent().ok_or(ThemeError::NoHome)?;
        std::fs::create_dir_all(dir).map_err(ThemeError::Io)?;
        let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, t.to_string()).and_then(|_| std::fs::rename(&tmp, &path)).map_err(ThemeError::Io)?;
        Ok(id)
    }
}

/// `/theme.js`: the shared choice in front of [`JS`], so the page has its
/// theme before the first paint. Send it `no-cache`.
pub fn script(home: &Home) -> String {
    let id = home.theme().map(|t| format!("\"{t}\"")).unwrap_or_else(|| "null".into());
    format!("window.LORE_THEME = {id};\n{JS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, Home) {
        let t = tempfile::tempdir().unwrap();
        let h = Home::at(t.path());
        (t, h)
    }

    /// The palette block of one theme.
    fn block(id: &str) -> &'static str {
        let start = CSS.find(&format!(":root[data-theme=\"{id}\"] {{")).unwrap_or_else(|| panic!("{id} missing in theme.css"));
        &CSS[start..start + CSS[start..].find("\n}").unwrap()]
    }

    fn token<'a>(id: &str, block: &'a str, name: &str) -> &'a str {
        let key = format!("--{name}: #");
        let i = block.find(&key).unwrap_or_else(|| panic!("{id}: no --{name}")) + key.len() - 1;
        &block[i..i + 7]
    }

    /// WCAG contrast ratio of two `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        let lum = |h: &str| {
            let c: Vec<f64> = (0..3)
                .map(|i| {
                    let v = u8::from_str_radix(&h[1 + 2 * i..3 + 2 * i], 16).unwrap() as f64 / 255.0;
                    if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
                })
                .collect();
            0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// The rules in the header of theme.css.
    #[test]
    fn themes_meet_the_contrast_rules() {
        for id in THEMES {
            let b = block(id);
            let tok = |name: &str| token(id, b, name);
            let (bg, panel, surface) = (tok("bg"), tok("panel"), tok("surface"));
            for s in [bg, panel, surface, tok("paper")] {
                assert!(!["#000000", "#ffffff"].contains(&s), "{id}: pure black or white");
            }
            assert!(contrast(tok("text"), bg) >= 7.0 && contrast(tok("text"), panel) >= 7.0, "{id}: text");
            assert!(contrast(tok("muted"), panel) >= 4.5, "{id}: muted on panel");
            assert!(contrast(tok("faint"), panel) >= 3.0, "{id}: faint on panel");
            assert!(contrast(tok("paper-ink"), tok("paper")) >= 7.0, "{id}: paper-ink on paper");
            for a in ["accent", "accent-2", "accent-3"] {
                let t = tok(&format!("{a}-text"));
                assert!(contrast(t, bg) >= 4.5 && contrast(t, panel) >= 4.5, "{id}: {a}-text {t}");
                let (fill, ink) = (tok(a), tok(&format!("{a}-ink")));
                assert!(contrast(ink, fill) >= 4.5, "{id}: {a}-ink {ink} on {fill}");
            }
            for s in ["ok", "warn", "err"] {
                assert!(contrast(tok(s), panel) >= 3.0, "{id}: {s} on panel");
            }
        }
    }

    #[test]
    fn theme_ids_match_the_stylesheet_and_the_script() {
        for id in THEMES {
            block(id);
            assert!(JS.contains(&format!("{{ id: \"{id}\", name: ")), "{id} missing in theme.js");
        }
        assert_eq!(CSS.matches(":root[data-theme=").count(), THEMES.len(), "theme.css has a theme not in THEMES");
        assert_eq!(JS.matches("{ id: \"").count(), THEMES.len(), "theme.js has a theme not in THEMES");
        for token in ["--accent:", "--accent-2:", "--accent-3:", "--accent-2-ink:", "--accent-3-ink:", "--accent-text:", "--accent-2-text:", "--accent-3-text:", "--paper:", "--ok:"] {
            assert_eq!(CSS.matches(&format!(" {token} #")).count(), THEMES.len(), "every theme defines {token}");
        }
        // dark / light in the script agrees with the stylesheet, and so does the first swatch
        for id in THEMES {
            let b = block(id);
            let dark = b.contains("color-scheme: dark");
            let line = JS.lines().find(|l| l.contains(&format!("{{ id: \"{id}\","))).unwrap();
            assert!(line.contains(&format!("dark: {dark},")), "{id}: dark/light differs");
            assert!(line.contains(&format!("sw: [\"{}\"", token(id, b, "bg"))), "{id}: swatch does not start with --bg");
        }
        // the default is the stylesheet's fallback and the script's default
        assert!(CSS.contains(&format!(":root,\n:root[data-theme=\"{DEFAULT}\"] {{")));
        assert!(JS.contains(&format!("const DEFAULT = \"{DEFAULT}\";")));
        assert!(THEMES.contains(&DEFAULT));
    }

    #[test]
    fn old_palettes_map_to_the_nearest_theme() {
        assert_eq!(nearest("orange", false), Some("amber"));
        assert_eq!(nearest("night", true), Some("lavender"));
        assert_eq!(nearest("terminal", false), Some("hacker"));
        assert_eq!(nearest("mint", true), Some("mint"));
        assert_eq!(nearest("blue", false), None);
        for (old, dark, light) in [("orange", "amber", "peachlavender"), ("night", "neon", "lavender"), ("terminal", "hacker", "mint")] {
            assert!(JS.contains(&format!("{old}: {{ dark: \"{dark}\", light: \"{light}\" }}")), "{old} differs in theme.js");
        }
    }

    #[test]
    fn the_choice_is_shared_and_keeps_other_settings() {
        let (t, h) = home();
        assert_eq!(h.theme(), None);
        assert!(script(&h).starts_with("window.LORE_THEME = null;\n"));
        assert!(matches!(h.set_theme("orange"), Err(ThemeError::Unknown(_))));
        assert_eq!(h.theme(), None);

        assert_eq!(h.set_theme("shibuya").unwrap(), "shibuya");
        assert_eq!(Home::at(t.path()).theme(), Some("shibuya"));
        assert!(script(&h).starts_with("window.LORE_THEME = \"shibuya\";\n"));

        let f = t.path().join("private/shared/ui.toml");
        std::fs::write(&f, "theme = \"night\"\nmode = \"light\"\nzoom = 1.25\n").unwrap();
        assert_eq!(h.theme(), Some("lavender"), "a hand-written old value still reads");
        h.set_theme("mint").unwrap();
        let s = std::fs::read_to_string(&f).unwrap();
        assert!(s.contains("theme = \"mint\"") && s.contains("zoom = 1.25") && !s.contains("mode"), "{s}");

        std::fs::write(&f, "theme = 3").unwrap();
        assert_eq!(h.theme(), None);
        assert!(matches!(Home::default().set_theme("mint"), Err(ThemeError::NoHome)));
    }
}
