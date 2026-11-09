//! lore-fonts — what the fonts a document names become on this machine.
//!
//! A deck or a paper names fonts (Calibri, 微軟正黑體, a company font). This
//! machine may not have them. For every name this crate gives a chain of
//! substitutes and the one actually used:
//!
//! 1. the author's own rules (`private/shared/fonts.toml` `[fallback]`; company
//!    or licensed fonts are named only there),
//! 2. the bundled defaults (`defaults.toml`: metric-compatible substitutes
//!    first, e.g. Calibri → Carlito, so lines break the same way),
//! 3. a guessed category (sans, serif, mono, math, CJK sans / serif / kai) and
//!    that category's chain.
//!
//! "Has" means: installed on the system, or a font file in the author's
//! private font folder (`private/shared/fonts/`), which the apps serve to
//! their pages with `@font-face` — no installation needed, never committed to
//! a public repository.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DEFAULTS: &str = include_str!("defaults.toml");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Sans,
    Serif,
    Mono,
    Math,
    CjkSans,
    CjkSerif,
    CjkKai,
}

impl Category {
    /// The CSS generic family that ends a stack.
    pub fn generic(self) -> &'static str {
        match self {
            Category::Serif | Category::Math | Category::CjkSerif | Category::CjkKai => "serif",
            Category::Mono => "monospace",
            Category::Sans | Category::CjkSans => "sans-serif",
        }
    }

    pub fn is_cjk(self) -> bool {
        matches!(self, Category::CjkSans | Category::CjkSerif | Category::CjkKai)
    }
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}

/// A category for a font no table knows, from its name.
pub fn guess_category(name: &str) -> Category {
    let l = name.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| l.contains(w));
    let cjk = name.chars().any(is_cjk_char) || has(&["jhenghei", "yahei", "mingliu", "kai", "song", "ming", "hei", "gothic", "pingfang", " tc", " sc", " jp", " kr", "cjk"]);
    if cjk {
        if has(&["楷", "kai"]) {
            Category::CjkKai
        } else if has(&["明", "宋", "ming", "song", "serif"]) {
            Category::CjkSerif
        } else {
            Category::CjkSans
        }
    } else if has(&["math"]) {
        Category::Math
    } else if has(&["mono", "courier", "consol", "code", "menlo", "typewriter"]) {
        Category::Mono
    } else if has(&["serif", "times", "roman", "garamond", "georgia", "cambria", "book", "minion", "palatino", "caslon"]) && !l.contains("sans") {
        Category::Serif
    } else {
        Category::Sans
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DefaultFile {
    category: BTreeMap<Category, Vec<String>>,
    #[serde(default)]
    font: BTreeMap<String, DefaultFont>,
}

#[derive(Debug, Clone, Deserialize)]
struct DefaultFont {
    category: Category,
    chain: Vec<String>,
}

/// The substitution rules: the author's first, then the defaults.
#[derive(Debug, Clone)]
pub struct Table {
    categories: BTreeMap<Category, Vec<String>>,
    /// lower-case name → (category, chain)
    fonts: BTreeMap<String, (Category, Vec<String>)>,
    /// Names the author gave a rule for (shown as such in reports).
    user: BTreeSet<String>,
}

impl Table {
    pub fn defaults() -> Table {
        let d: DefaultFile = toml::from_str(DEFAULTS).expect("bundled defaults.toml is valid");
        Table {
            categories: d.category,
            fonts: d.font.into_iter().map(|(k, f)| (k.to_lowercase(), (f.category, f.chain))).collect(),
            user: BTreeSet::new(),
        }
    }

    /// The defaults with the author's `[fallback]` rules on top (each rule's
    /// chain replaces the default chain; the category is kept or guessed).
    pub fn with_user(user: &BTreeMap<String, Vec<String>>) -> Table {
        let mut t = Table::defaults();
        for (name, chain) in user {
            let key = name.to_lowercase();
            let cat = t.fonts.get(&key).map(|(c, _)| *c).unwrap_or_else(|| guess_category(name));
            let chain: Vec<String> = chain.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            t.fonts.insert(key.clone(), (cat, chain));
            t.user.insert(key);
        }
        t
    }

    /// Category and full chain for a name: its own chain, then its
    /// category's, without repeats.
    pub fn chain(&self, name: &str) -> (Category, Vec<String>) {
        let key = name.trim().to_lowercase();
        let (cat, own) = match self.fonts.get(&key) {
            Some((c, ch)) => (*c, ch.clone()),
            None => (guess_category(name), vec![name.trim().to_string()]),
        };
        let mut out: Vec<String> = vec![];
        for f in own.iter().chain(self.categories.get(&cat).into_iter().flatten()) {
            if !out.iter().any(|x| x.eq_ignore_ascii_case(f)) {
                out.push(f.clone());
            }
        }
        (cat, out)
    }

    /// Every rule as name → chain (category chains included), for a page
    /// that resolves fonts itself (CSS font stacks).
    pub fn stacks(&self) -> BTreeMap<String, Vec<String>> {
        self.fonts.keys().map(|k| (k.clone(), self.chain(k).1)).collect()
    }

    pub fn category_chains(&self) -> &BTreeMap<Category, Vec<String>> {
        &self.categories
    }
}

/// A font file in the author's private font folder.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrivateFace {
    /// Every family name the file carries (e.g. "Iansui" and "芫荽").
    pub families: Vec<String>,
    /// File name inside the folder.
    pub file: String,
    pub weight: u16,
    pub italic: bool,
}

/// The fonts this machine has: system families plus private files.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// lower-case family → display name
    families: BTreeMap<String, String>,
    pub private: Vec<PrivateFace>,
}

fn file_ext_ok(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2"))
}

impl Installed {
    /// System fonts (fontdb's search: macOS Library folders, Windows Fonts,
    /// fontconfig dirs) plus every font file directly in `private_dir`.
    pub fn scan(private_dir: Option<&Path>) -> Installed {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut me = Installed::default();
        for f in db.faces() {
            for (name, _) in &f.families {
                me.families.entry(name.to_lowercase()).or_insert_with(|| name.clone());
            }
        }
        if let Some(dir) = private_dir {
            me.add_private(dir);
        }
        me
    }

    /// A set of family names (tests, or a machine description).
    pub fn from_names<I: IntoIterator<Item = S>, S: AsRef<str>>(names: I) -> Installed {
        Installed { families: names.into_iter().map(|n| (n.as_ref().to_lowercase(), n.as_ref().to_string())).collect(), private: vec![] }
    }

    /// Fonts under `dir` and its sub-folders counted as installed (not
    /// private), e.g. a TeX distribution's `fonts/opentype` tree, which
    /// XeTeX and LuaTeX find by name but the system font list may not show.
    pub fn add_system_dir(&mut self, dir: &Path) {
        let mut db = fontdb::Database::new();
        db.load_fonts_dir(dir);
        for f in db.faces() {
            for (name, _) in &f.families {
                self.families.entry(name.to_lowercase()).or_insert_with(|| name.clone());
            }
        }
    }

    /// Font files directly in `dir` (not sub-folders).
    pub fn add_private(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && file_ext_ok(p)).collect();
        files.sort();
        for path in files {
            let mut db = fontdb::Database::new();
            if db.load_font_file(&path).is_err() {
                continue;
            }
            let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            for f in db.faces() {
                let families: Vec<String> = f.families.iter().map(|(n, _)| n.clone()).collect();
                for n in &families {
                    self.families.entry(n.to_lowercase()).or_insert_with(|| n.clone());
                }
                self.private.push(PrivateFace { families, file: file.clone(), weight: f.weight.0, italic: f.style != fontdb::Style::Normal });
            }
        }
    }

    pub fn has(&self, family: &str) -> bool {
        self.families.contains_key(&family.trim().to_lowercase())
    }

    fn is_private(&self, family: &str) -> bool {
        self.private.iter().any(|p| p.families.iter().any(|f| f.eq_ignore_ascii_case(family)))
    }

    pub fn count(&self) -> usize {
        self.families.len()
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    /// The font itself (or one of its own names) is installed.
    Installed,
    /// The font itself, from the private font folder.
    Private,
    /// Another font from the chain stands in.
    Substitute,
    /// Nothing in the chain is here; the generic family is used.
    Missing,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Resolution {
    pub name: String,
    pub category: Category,
    pub chain: Vec<String>,
    /// The first name in the chain this machine has.
    pub used: Option<String>,
    pub how: How,
    /// The author's own rule decided the chain.
    pub user_rule: bool,
}

/// What `name` becomes here.
pub fn resolve(table: &Table, installed: &Installed, name: &str) -> Resolution {
    let (category, chain) = table.chain(name);
    let used = chain.iter().find(|f| installed.has(f)).cloned();
    // "Itself": the requested name, or another name of the same font listed
    // before the first substitute in its own rule (微軟正黑體 = Microsoft JhengHei).
    let own_names: Vec<&String> = match table.fonts.get(&name.trim().to_lowercase()) {
        Some((_, ch)) => ch.iter().take_while(|f| f.eq_ignore_ascii_case(name) || !table.fonts.contains_key(&f.to_lowercase()) || aliases(name, f)).collect(),
        None => vec![],
    };
    let how = match &used {
        None => How::Missing,
        Some(u) if u.eq_ignore_ascii_case(name) || own_names.iter().any(|o| o.eq_ignore_ascii_case(u)) && aliases(name, u) => {
            if installed.is_private(u) { How::Private } else { How::Installed }
        }
        Some(_) => How::Substitute,
    };
    Resolution { name: name.trim().to_string(), category, chain, used, how, user_rule: table.user.contains(&name.trim().to_lowercase()) }
}

/// Two names of the same font (a localized and an English family name).
fn aliases(a: &str, b: &str) -> bool {
    const PAIRS: &[(&str, &str)] = &[
        ("微軟正黑體", "Microsoft JhengHei"),
        ("新細明體", "PMingLiU"),
        ("細明體", "MingLiU"),
        ("標楷體", "DFKai-SB"),
        ("芫荽", "Iansui"),
        ("微軟雅黑", "Microsoft YaHei"),
    ];
    a.eq_ignore_ascii_case(b) || PAIRS.iter().any(|(x, y)| (a == *x && b.eq_ignore_ascii_case(y)) || (b == *x && a.eq_ignore_ascii_case(y)))
}

/// Resolve several names (e.g. every font a deck uses), sorted, no repeats.
pub fn report<I: IntoIterator<Item = S>, S: AsRef<str>>(table: &Table, installed: &Installed, names: I) -> Vec<Resolution> {
    let mut seen = BTreeSet::new();
    let mut out: Vec<Resolution> = names
        .into_iter()
        .map(|n| n.as_ref().trim().to_string())
        // Theme placeholders (+mj-lt) and CSS keywords are not fonts.
        .filter(|n| !n.is_empty() && !n.starts_with('+') && !is_css_keyword(n) && seen.insert(n.to_lowercase()))
        .map(|n| resolve(table, installed, &n))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn is_css_keyword(n: &str) -> bool {
    matches!(n.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset" | "revert" | "serif" | "sans-serif" | "monospace" | "system-ui" | "cursive" | "fantasy")
}

/// `@font-face` rules for the private font folder, served at `url_prefix`
/// (e.g. `/fonts/`). Each family name of each file gets a rule, so a deck
/// naming "芫荽" or "Iansui" finds the same file.
pub fn font_faces_css(installed: &Installed, url_prefix: &str) -> String {
    let mut css = String::new();
    for p in &installed.private {
        let fmt = match p.file.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
            Some("woff2") => "woff2",
            Some("woff") => "woff",
            Some("otf") | Some("otc") => "opentype",
            _ => "truetype",
        };
        for fam in &p.families {
            css += &format!(
                "@font-face {{ font-family: \"{}\"; src: url(\"{}{}\") format(\"{fmt}\"); font-weight: {}; font-style: {}; font-display: swap; }}\n",
                fam.replace('"', ""),
                url_prefix,
                p.file.replace('"', "%22"),
                p.weight,
                if p.italic { "italic" } else { "normal" }
            );
        }
    }
    css
}

/// A font file name that may be served from the private folder.
pub fn safe_font_file(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']) && file_ext_ok(Path::new(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_are_guessed_from_names() {
        assert_eq!(guess_category("Brand Sans"), Category::Sans);
        assert_eq!(guess_category("Source Serif 4"), Category::Serif);
        assert_eq!(guess_category("JetBrains Mono"), Category::Mono);
        assert_eq!(guess_category("STIX Two Math"), Category::Math);
        assert_eq!(guess_category("華康楷書"), Category::CjkKai);
        assert_eq!(guess_category("思源宋體"), Category::CjkSerif);
        assert_eq!(guess_category("蘋方-繁"), Category::CjkSans);
    }

    #[test]
    fn office_fonts_on_a_mac_without_office() {
        let t = Table::defaults();
        let mac = Installed::from_names(["PT Sans", "Helvetica Neue", "Arial", "Times New Roman", "PingFang TC", "BiauKai", "STIX Two Math", "Menlo"]);
        let r = resolve(&t, &mac, "Calibri");
        assert_eq!((r.used.as_deref(), r.how), (Some("PT Sans"), How::Substitute), "a narrow face, like Calibri");
        let r = resolve(&t, &mac, "Some Company Sans");
        assert_eq!(r.used.as_deref(), Some("PT Sans"));
        let r = resolve(&t, &mac, "微軟正黑體");
        assert_eq!((r.used.as_deref(), r.how, r.category), (Some("PingFang TC"), How::Substitute, Category::CjkSans));
        let r = resolve(&t, &mac, "標楷體");
        assert_eq!((r.used.as_deref(), r.how), (Some("BiauKai"), How::Substitute));
        let r = resolve(&t, &mac, "Times New Roman");
        assert_eq!(r.how, How::Installed);
        let r = resolve(&t, &mac, "Cambria Math");
        assert_eq!(r.used.as_deref(), Some("STIX Two Math"));
    }

    #[test]
    fn metric_compatible_substitutes_come_first() {
        let t = Table::defaults();
        let linux = Installed::from_names(["Carlito", "Liberation Sans", "DejaVu Sans"]);
        assert_eq!(resolve(&t, &linux, "Calibri").used.as_deref(), Some("Carlito"));
        assert_eq!(resolve(&t, &linux, "Arial").used.as_deref(), Some("Liberation Sans"));
    }

    #[test]
    fn localized_names_count_as_the_font_itself() {
        let t = Table::defaults();
        let win = Installed::from_names(["Microsoft JhengHei"]);
        assert_eq!(resolve(&t, &win, "微軟正黑體").how, How::Installed);
    }

    #[test]
    fn the_authors_rules_win_and_unknown_fonts_get_a_category() {
        let mut user = BTreeMap::new();
        user.insert("Brand Sans".to_string(), vec!["Brand Sans".to_string(), "Calibri".to_string(), "Carlito".to_string()]);
        let t = Table::with_user(&user);
        let here = Installed::from_names(["Carlito", "Arial"]);
        let r = resolve(&t, &here, "Brand Sans");
        assert_eq!((r.used.as_deref(), r.how, r.user_rule), (Some("Carlito"), How::Substitute, true));
        let r = resolve(&t, &here, "Some Unknown Grotesk");
        assert_eq!((r.category, r.used.as_deref()), (Category::Sans, Some("Carlito")));
        let r = resolve(&t, &Installed::default(), "Nothing Here");
        assert_eq!((r.how, r.used), (How::Missing, None));
        assert_eq!(r.category.generic(), "sans-serif");
    }

    #[test]
    fn reports_skip_theme_placeholders_and_repeats() {
        let t = Table::defaults();
        let r = report(&t, &Installed::default(), ["Arial", "arial", "+mj-lt", "", "inherit", "Calibri"]);
        assert_eq!(r.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Arial", "Calibri"]);
    }

    #[test]
    fn private_files_become_font_faces() {
        let i = Installed {
            families: BTreeMap::new(),
            private: vec![PrivateFace { families: vec!["Iansui".into(), "芫荽".into()], file: "Iansui-Regular.ttf".into(), weight: 400, italic: false }],
        };
        let css = font_faces_css(&i, "/fonts/");
        assert!(css.contains("font-family: \"芫荽\"; src: url(\"/fonts/Iansui-Regular.ttf\") format(\"truetype\")"));
        assert!(css.contains("font-family: \"Iansui\""));
        assert!(safe_font_file("Iansui-Regular.ttf") && !safe_font_file("../x.ttf") && !safe_font_file("notes.txt"));
    }

    #[test]
    fn scans_real_files_in_a_private_folder() {
        // Any font file on this machine will do; skip when none is found.
        let candidates = ["/usr/share/fonts/truetype/crosextra/Carlito-Regular.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial Unicode.ttf"];
        let Some(src) = candidates.iter().map(Path::new).find(|p| p.exists()) else { return };
        let dir = std::env::temp_dir().join(format!("lore-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(src, dir.join("face.ttf")).unwrap();
        std::fs::write(dir.join("readme.txt"), "not a font").unwrap();
        let mut i = Installed::default();
        i.add_private(&dir);
        assert_eq!(i.private.len(), 1);
        assert!(i.has(&i.private[0].families[0].clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_dirs_are_read_with_sub_folders_and_count_as_installed() {
        let candidates = ["/usr/share/fonts/truetype/crosextra/Carlito-Regular.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial Unicode.ttf"];
        let Some(src) = candidates.iter().map(Path::new).find(|p| p.exists()) else { return };
        let dir = std::env::temp_dir().join(format!("lore-fonts-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("opentype/public/x")).unwrap();
        std::fs::copy(src, dir.join("opentype/public/x/face.ttf")).unwrap();
        let mut probe = Installed::default();
        probe.add_private(&dir.join("opentype/public/x"));
        let family = probe.private[0].families[0].clone();
        let mut i = Installed::default();
        i.add_system_dir(&dir);
        assert!(i.has(&family) && i.private.is_empty());
        assert_eq!(resolve(&Table::defaults(), &i, &family).how, How::Installed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stacks_end_with_the_category_chain() {
        let t = Table::defaults();
        let s = t.stacks();
        let c = &s["calibri"];
        assert_eq!(&c[..2], ["Calibri", "Carlito"]);
        assert!(c.contains(&"Helvetica Neue".to_string()));
    }
}
