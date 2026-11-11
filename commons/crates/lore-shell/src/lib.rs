//! The shared LoreForge shell (`docs/shell.md`): `commons/ui/icons.js`, `base.css`,
//! `shell.js` and `makedeck.js`, compiled in like `lore_home::theme`, so an app server only needs
//! these routes:
//!
//! ```ignore
//! for (path, mime, body) in lore_shell::FILES { router = router.route(path, get(move || async move { ([("content-type", *mime)], *body) })); }
//! ```
//!
//! The pages load them after theme.css / theme.js:
//! `theme.css → base.css → styles.css` and `theme.js → icons.js → shell.js → app.js`.
//! The app's own copy of icons.js goes away (both declare `ICONS`).

/// `commons/ui/icons.js`: the union icon set (`ICONS`, `icon`, `paintIcons`, `iconButton`).
pub const ICONS_JS: &str = include_str!("../../../ui/icons.js");
/// `commons/ui/base.css`: buttons, menus, dialogs, toast, blocks, top bar, drawers.
pub const BASE_CSS: &str = include_str!("../../../ui/base.css");
/// `commons/ui/shell.js`: `LoreShell` (top bar, settings, toast, ask, engine chip) and `LoreLinks`.
pub const SHELL_JS: &str = include_str!("../../../ui/shell.js");
/// `commons/ui/makedeck.js`: `LoreMakeDeck`, the 做成簡報 button and card (apps with papers mount it).
pub const MAKEDECK_JS: &str = include_str!("../../../ui/makedeck.js");

/// `(route, content type, body)` for each file.
pub const FILES: &[(&str, &str, &str)] = &[
    ("/icons.js", "text/javascript; charset=utf-8", ICONS_JS),
    ("/base.css", "text/css; charset=utf-8", BASE_CSS),
    ("/shell.js", "text/javascript; charset=utf-8", SHELL_JS),
    ("/makedeck.js", "text/javascript; charset=utf-8", MAKEDECK_JS),
];

/// The names in `const ICONS = { … }` of an icons.js, in order (duplicates kept).
pub fn icon_names(src: &str) -> Vec<String> {
    icon_entries(src).into_iter().map(|(n, _)| n).collect()
}

/// `(name, svg body)` of every `  name: '…',` line in `const ICONS = { … };`.
fn icon_entries(src: &str) -> Vec<(String, String)> {
    let Some(start) = src.find("const ICONS = {") else { return Vec::new() };
    let rest = &src[start..];
    let end = rest.find("\n};").unwrap_or(rest.len());
    rest[..end]
        .lines()
        .skip(1)
        .filter_map(|l| {
            let l = l.trim();
            let (name, body) = l.split_once(": '")?;
            let body = body.strip_suffix("',")?;
            name.chars().all(|c| c.is_ascii_alphanumeric()).then(|| (name.to_string(), body.to_string()))
        })
        .collect()
}

/// `{ old: "new", … }` of `const ICON_ALIASES = { … };`.
pub fn icon_aliases(src: &str) -> Vec<(String, String)> {
    let Some(start) = src.find("const ICON_ALIASES = {") else { return Vec::new() };
    let rest = &src[start + "const ICON_ALIASES = {".len()..];
    let end = rest.find('}').unwrap_or(rest.len());
    rest[..end]
        .split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    const APPS: [(&str, &str); 4] = [("PaperComposer", "ui"), ("SlideComposer", "web"), ("LoreReader", "web"), ("KnowledgeBase", "web")];

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }
    fn union() -> BTreeSet<String> {
        icon_names(ICONS_JS).into_iter().collect()
    }
    fn known() -> BTreeSet<String> {
        let mut k = union();
        k.extend(icon_aliases(ICONS_JS).into_iter().map(|(a, _)| a));
        k
    }
    /// The app's own JS and HTML (not icons.js, not vendor/).
    fn app_sources(dir: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else { return out };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name == "icons.js" || !(name.ends_with(".js") || name.ends_with(".html")) {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(&p) {
                out.push((name, s));
            }
        }
        out
    }
    /// Identifiers right after each prefix, up to the closing quote.
    fn after(src: &str, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(i) = src[at..].find(prefix) {
            let s = at + i + prefix.len();
            let id: String = src[s..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            let close = src[s + id.len()..].chars().next();
            if !id.is_empty() && matches!(close, Some('"') | Some('\'') | Some('\\')) {
                out.push(id);
            }
            at = s;
        }
        out
    }
    const CALLS: [&str; 13] = [
        "data-icon=\"", "data-icon=\\\"", "data-icon='", "\"data-icon\": \"", "icon(\"", "icon('", "iconButton(\"", "iconButton('",
        "dataset.icon = \"", "icon: \"", "ib(\"", "icoBtn(\"", "ICONS.",
    ];

    #[test]
    fn icons_parse_without_duplicates() {
        let entries = icon_entries(ICONS_JS);
        assert!(entries.len() >= 140, "only {} icons parsed", entries.len());
        let body = &ICONS_JS[ICONS_JS.find("const ICONS = {").unwrap()..];
        let body = &body[..body.find("\n};").unwrap()];
        for l in body.lines().skip(1) {
            let t = l.trim();
            if t.is_empty() || t.starts_with("//") {
                continue;
            }
            assert!(entries.iter().any(|(n, _)| t.starts_with(&format!("{n}: '"))), "line not an icon: {t}");
        }
        let mut seen = BTreeSet::new();
        for (n, svg) in &entries {
            assert!(seen.insert(n.clone()), "{n} defined twice");
            assert!(svg.starts_with('<') && svg.ends_with('>'), "{n}: not SVG");
            assert!(!svg.contains('\''), "{n}: quote inside");
            assert_eq!(svg.matches('<').count(), svg.matches('>').count(), "{n}: unbalanced tags");
        }
    }

    #[test]
    fn aliases_and_renames_point_at_icons() {
        let u = union();
        let aliases = icon_aliases(ICONS_JS);
        assert!(!aliases.is_empty());
        for (old, new) in aliases {
            assert!(u.contains(&new), "alias {old} → {new}: no such icon");
            assert!(!u.contains(&old), "alias {old} is also an icon");
        }
        let renames = &ICONS_JS[ICONS_JS.find("const ICON_RENAMES").unwrap()..ICONS_JS.find("const ICONS = {").unwrap()];
        for n in after(renames, "name: \"") {
            assert!(u.contains(&n), "rename target {n}: no such icon");
        }
        for n in ["home", "chevron", "book", "download", "printer", "save", "folder", "spark", "gear", "undo", "redo", "search", "cloud", "cloudOff", "panelLeft", "panel", "x"] {
            assert!(u.contains(n), "shared slot icon {n} missing");
        }
    }

    /// Every icon the four apps had, and every name they use, still draws.
    #[test]
    fn covers_every_app() {
        let k = known();
        let mut missing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut apps = 0;
        for (app, web) in APPS {
            let dir = repo().join(app).join(web);
            if !dir.is_dir() {
                continue;
            }
            apps += 1;
            if let Ok(old) = std::fs::read_to_string(dir.join("icons.js")) {
                for n in icon_names(&old) {
                    if !k.contains(&n) {
                        missing.entry(format!("{app}/icons.js")).or_default().insert(n);
                    }
                }
            }
            for (file, src) in app_sources(&dir) {
                for p in CALLS {
                    for n in after(&src, p) {
                        if !k.contains(&n) && !(p == "ICONS." && n == "ICONS") {
                            missing.entry(format!("{app}/{web}/{file}")).or_default().insert(n);
                        }
                    }
                }
            }
        }
        assert!(apps > 0, "no app folders next to commons");
        assert!(missing.is_empty(), "icons not in commons/ui/icons.js: {missing:?}");
    }

    /// base.css uses theme.css tokens (and its own four) only, no new colours.
    #[test]
    fn base_css_uses_theme_tokens() {
        let theme = include_str!("../../../ui/theme.css");
        let defined: BTreeSet<String> = after(theme, "--").into_iter().chain(["sans", "mono", "r", "top-h"].map(String::from)).collect();
        let mut used = BTreeSet::new();
        let mut at = 0;
        while let Some(i) = BASE_CSS[at..].find("var(--") {
            let s = at + i + 6;
            let id: String = BASE_CSS[s..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            used.insert(id);
            at = s;
        }
        let unknown: Vec<_> = used.iter().filter(|u| !defined.contains(*u) && !theme.contains(&format!("--{u}:"))).collect();
        assert!(unknown.is_empty(), "base.css uses tokens theme.css does not define: {unknown:?}");
        // literal colours: only the black shadows / backdrop (#000…)
        for (i, _) in BASE_CSS.match_indices('#') {
            let hex: String = BASE_CSS[i + 1..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() >= 3 {
                assert!(hex.starts_with("000"), "literal colour #{hex} in base.css");
            }
        }
        assert!(!BASE_CSS.contains("rgb("), "rgb() in base.css");
    }

    #[test]
    fn settings_tabs_and_slots() {
        let tabs = &SHELL_JS[SHELL_JS.find("const SHARED_TABS").unwrap()..];
        let tabs = &tabs[..tabs.find("];").unwrap()];
        let labels: Vec<String> = tabs.split("label: \"").skip(1).map(|s| s[..s.find('"').unwrap()].to_string()).collect();
        assert_eq!(labels, ["一般", "外觀", "模型", "同步", "快捷鍵"]);
        assert!(labels.iter().all(|l| l.chars().count() <= 3));
        // right side: notation · export · gear, gear last
        let r = SHELL_JS.find("right: [undo redo]").unwrap();
        let (n, e, g) = (SHELL_JS[r..].find("\"對照表\"").unwrap(), SHELL_JS[r..].find("\"匯出\"").unwrap(), SHELL_JS[r..].find("\"設定 ⌘,\"").unwrap());
        assert!(n < e && e < g);
        assert!(!SHELL_JS.contains("confirm("), "no native confirm");
    }

    #[test]
    fn files_are_served() {
        assert_eq!(FILES.len(), 4);
        assert!(FILES.iter().all(|(p, _, b)| p.starts_with('/') && !b.is_empty()));
        assert!(SHELL_JS.contains("window.LoreShell") && SHELL_JS.contains("window.LoreLinks"));
        assert!(MAKEDECK_JS.contains("window.LoreMakeDeck") && MAKEDECK_JS.contains("/api/todeck"));
    }
}
