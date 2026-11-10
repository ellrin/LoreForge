//! lore-home — where a LoreForge app finds its private data.
//!
//! ```text
//! LoreForge/                    (public repo)
//!   private-example/            public, minimal: enough for every app to open
//!     shared/  <App>/
//!   private/                    ignored by LoreForge; each folder is its own private repo
//!     shared/                   personal settings every app may read (LoreForge-private)
//!       profile.toml  fonts.toml  deny-words.txt  docs/  …
//!     <App>/                    one app's data (e.g. KnowledgeBase-private)
//! ```
//!
//! * [`Home::discover`] finds the LoreForge folder from a path inside it (an
//!   app passes its `CARGO_MANIFEST_DIR` or executable path). `LOREFORGE_HOME`
//!   names it explicitly; `LOREFORGE_PRIVATE` moves the private folder elsewhere.
//! * [`Home::app_dir`] is `private/<App>`; [`Home::seed_app`] creates it from
//!   `private-example/<App>` on first use (missing files only, never overwrites).
//!   Nothing is ever written into `private-example/`.
//! * [`Home::shared_file`] reads `private/shared/<file>`, else the example's
//!   copy — so a new app gets the user's personal settings with one call.
//! * [`Prefs`] are the typed shared settings: [`Profile`] and [`Fonts`].
//!   Fonts may name licensed fonts installed on this machine; they stay in the
//!   private repo and only reach the apps' pages at run time.
//!
//! * [`theme`] is the colour theme all apps share (`private/shared/ui.toml`),
//!   with the stylesheet and script every server sends.
//!
//! Keys and passwords never live here: they are in the OS keychain (see
//! `lore-secrets`), and local models in `LOCAL_LLM_HOME` (see `lore-llm`).

pub mod engine;
pub mod theme;

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PRIVATE: &str = "private";
pub const EXAMPLE: &str = "private-example";
pub const SHARED: &str = "shared";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Home {
    /// The LoreForge folder, when the app runs from one.
    pub root: Option<PathBuf>,
    /// `private/` (or `LOREFORGE_PRIVATE`).
    pub private: Option<PathBuf>,
}

/// Does `dir` look like the LoreForge folder?
fn is_root(dir: &Path) -> bool {
    dir.join(EXAMPLE).is_dir() && dir.join("commons").is_dir()
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Home {
    /// From the environment, else the first ancestor of `start` that is a LoreForge folder.
    pub fn discover(start: &Path) -> Home {
        let root = env_path("LOREFORGE_HOME").or_else(|| start.ancestors().find(|a| is_root(a)).map(Path::to_path_buf));
        let private = env_path("LOREFORGE_PRIVATE").or_else(|| root.as_ref().map(|r| r.join(PRIVATE)));
        Home { root, private }
    }

    /// A fixed layout (tests, or an app with its own settings).
    pub fn at(root: impl Into<PathBuf>) -> Home {
        let root = root.into();
        Home { private: Some(root.join(PRIVATE)), root: Some(root) }
    }

    /// `private/<app>` (may not exist yet).
    pub fn app_dir(&self, app: &str) -> Option<PathBuf> {
        self.private.as_ref().map(|p| p.join(app))
    }

    /// `private-example/<app>`, when present.
    pub fn example_dir(&self, app: &str) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(EXAMPLE).join(app)).filter(|d| d.is_dir())
    }

    /// `private/<app>`, created from `private-example/<app>` the first time:
    /// copies files that are missing, never overwrites one.
    pub fn seed_app(&self, app: &str) -> std::io::Result<Option<PathBuf>> {
        let Some(dir) = self.app_dir(app) else { return Ok(None) };
        if !dir.exists() {
            match self.example_dir(app) {
                Some(ex) => copy_missing(&ex, &dir)?,
                None => std::fs::create_dir_all(&dir)?,
            }
        }
        Ok(Some(dir))
    }

    /// `private/shared` (may not exist).
    pub fn shared_dir(&self) -> Option<PathBuf> {
        self.private.as_ref().map(|p| p.join(SHARED))
    }

    /// `private/shared/<rel>` if it exists, else `private-example/shared/<rel>`.
    pub fn shared_file(&self, rel: &str) -> Option<PathBuf> {
        let mine = self.shared_dir().map(|d| d.join(rel)).filter(|p| p.exists());
        mine.or_else(|| self.root.as_ref().map(|r| r.join(EXAMPLE).join(SHARED).join(rel)).filter(|p| p.exists()))
    }

    /// Profile and fonts from the shared folder (defaults when missing or unreadable).
    pub fn prefs(&self) -> Prefs {
        Prefs { profile: self.read_toml("profile.toml"), fonts: self.read_toml("fonts.toml") }
    }

    fn read_toml<T: for<'de> Deserialize<'de> + Default>(&self, rel: &str) -> T {
        let Some(p) = self.shared_file(rel) else { return T::default() };
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("lore-home: {}: {e} (using defaults)", p.display());
                T::default()
            }
        }
    }
}

/// Copy `from` into `to`, creating folders; existing files are left alone.
pub fn copy_missing(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_missing(&e.path(), &dst)?;
        } else if !dst.exists() {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prefs {
    pub profile: Profile,
    pub fonts: Fonts,
}

/// `shared/profile.toml`: who uses this machine's apps.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// User id in spaces and progress folders (lower-case letters, digits, - or _).
    pub id: Option<String>,
    /// Display name.
    pub name: Option<String>,
}

/// `shared/fonts.toml`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Fonts {
    /// Interface fonts of every app page (CSS stacks, first = preferred).
    pub ui: UiFonts,
    /// Slide editor: what a font named in a deck falls back to on this machine,
    /// e.g. `"Brand Sans" = ["Brand Sans", "Calibri", "Arial"]`.
    pub fallback: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct UiFonts {
    pub sans: Vec<String>,
    pub serif: Vec<String>,
    pub mono: Vec<String>,
}

fn css_family(name: &str) -> String {
    const GENERIC: &[&str] = &["serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui", "ui-monospace", "ui-serif", "ui-sans-serif", "-apple-system"];
    if GENERIC.contains(&name) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace(['"', '\\', ';', '{', '}', '<', '>'], ""))
    }
}

impl Fonts {
    /// CSS that overrides the pages' `--sans` / `--serif` / `--mono` (empty when nothing is set).
    /// Served by each app at `/prefs.css`, linked after its own stylesheet.
    pub fn css(&self) -> String {
        let mut vars = Vec::new();
        for (var, stack) in [("--sans", &self.ui.sans), ("--serif", &self.ui.serif), ("--mono", &self.ui.mono)] {
            if !stack.is_empty() {
                vars.push(format!("  {var}: {};", stack.iter().map(|f| css_family(f)).collect::<Vec<_>>().join(", ")));
            }
        }
        if vars.is_empty() {
            return "/* no font preferences (private/shared/fonts.toml) */\n".into();
        }
        format!("/* from private/shared/fonts.toml */\n:root {{\n{}\n}}\n", vars.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> (tempfile::TempDir, Home) {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        for d in ["commons", "private-example/shared", "private-example/App/notes", "KnowledgeBase/crates/server"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        std::fs::write(r.join("private-example/App/notes/a.txt"), "example").unwrap();
        std::fs::write(r.join("private-example/shared/profile.toml"), "id = \"ellrin\"\n").unwrap();
        let h = Home::at(r);
        (t, h)
    }

    #[test]
    fn discovers_the_root_from_inside() {
        let (t, _) = layout();
        let h = Home::discover(&t.path().join("KnowledgeBase/crates/server"));
        assert_eq!(h.root.as_deref(), Some(t.path()));
        assert_eq!(h.app_dir("KnowledgeBase").unwrap(), t.path().join("private/KnowledgeBase"));
        assert_eq!(Home::discover(Path::new("/")).root, None);
    }

    #[test]
    fn seeds_from_the_example_without_overwriting_or_touching_it() {
        let (t, h) = layout();
        let d = h.seed_app("App").unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(d.join("notes/a.txt")).unwrap(), "example");
        std::fs::write(d.join("notes/a.txt"), "mine").unwrap();
        h.seed_app("App").unwrap();
        assert_eq!(std::fs::read_to_string(d.join("notes/a.txt")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(t.path().join("private-example/App/notes/a.txt")).unwrap(), "example");
        // an app without an example still gets an empty folder
        assert!(h.seed_app("New").unwrap().unwrap().is_dir());
    }

    #[test]
    fn shared_files_prefer_private_over_example() {
        let (t, h) = layout();
        assert_eq!(h.prefs().profile.id.as_deref(), Some("ellrin"));
        std::fs::create_dir_all(t.path().join("private/shared")).unwrap();
        std::fs::write(t.path().join("private/shared/profile.toml"), "id = \"daidai\"\nname = \"D\"\n").unwrap();
        assert_eq!(h.prefs().profile.id.as_deref(), Some("daidai"));
        assert_eq!(h.shared_file("nope.toml"), None);
    }

    #[test]
    fn font_preferences_become_css_variables() {
        let (t, h) = layout();
        assert!(!h.prefs().fonts.css().contains(":root"));
        std::fs::create_dir_all(t.path().join("private/shared")).unwrap();
        std::fs::write(
            t.path().join("private/shared/fonts.toml"),
            "[ui]\nsans = [\"Brand Sans\", \"sans-serif\"]\n[fallback]\n\"Brand Sans\" = [\"Brand Sans\", \"Calibri\"]\n",
        )
        .unwrap();
        let f = h.prefs().fonts;
        assert_eq!(f.fallback["Brand Sans"], vec!["Brand Sans", "Calibri"]);
        let css = f.css();
        assert!(css.contains("--sans: \"Brand Sans\", sans-serif;"), "{css}");
        assert!(!css.contains("--mono"));
    }

    #[test]
    fn bad_files_fall_back_to_defaults() {
        let (t, h) = layout();
        std::fs::create_dir_all(t.path().join("private/shared")).unwrap();
        std::fs::write(t.path().join("private/shared/fonts.toml"), "ui = 3").unwrap();
        assert_eq!(h.prefs().fonts, Fonts::default());
        assert_eq!(css_family("x\"; } body { color: red"), "\"x  body  color: red\"");
    }
}
