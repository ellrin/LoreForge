//! The two sets, parsed once from the embedded Iconify JSON.

use super::Look;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Raw {
    body: String,
    width: Option<f64>,
    height: Option<f64>,
}

#[derive(Deserialize)]
struct Alias {
    parent: String,
    // any other key (hFlip, vFlip, rotate, body, size) = a transform: not followed
    #[serde(flatten)]
    rest: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct File {
    width: f64,
    height: f64,
    icons: BTreeMap<String, Raw>,
    #[serde(default)]
    aliases: BTreeMap<String, Alias>,
}

/// One entry a search can offer.
pub struct Entry {
    pub name: String,
    pub title: String,
    pub look: Look,
    /// Tokens of the name, a trailing `s` stripped.
    pub tokens: Vec<String>,
    pub has_off: bool,
}

pub struct Set {
    pub id: &'static str,
    pub name: &'static str,
    pub license: &'static str,
    file: File,
    pub entries: Vec<Entry>,
}

/// What an id names: the drawing and its grid.
pub struct Found<'a> {
    pub body: &'a str,
    pub w: f64,
    pub h: f64,
}

/// A word as compared in a search: lower case, one trailing `s` stripped.
pub fn norm(w: &str) -> String {
    let w = w.to_lowercase();
    match w.strip_suffix('s') {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => w,
    }
}

fn entry(set: &str, name: &str) -> Option<Entry> {
    let (title, look) = if set == "tabler" {
        match name.strip_suffix("-filled") {
            Some(t) => (t, Look::Filled),
            None => (name, Look::Line),
        }
    } else {
        if name.ends_with("-negative") || name.ends_with("-24px") {
            return None;
        }
        match name.strip_suffix("-outline") {
            Some(t) => (t, Look::Line),
            None => (name, Look::Filled),
        }
    };
    let parts: Vec<&str> = name.split('-').collect();
    Some(Entry {
        name: name.to_string(),
        title: title.to_string(),
        look,
        tokens: parts.iter().map(|p| norm(p)).collect(),
        has_off: set == "tabler" && parts.contains(&"off"),
    })
}

fn load(id: &'static str, name: &'static str, license: &'static str, json: &str) -> Set {
    let file: File = serde_json::from_str(json).expect("icon set json");
    let entries = file.icons.keys().filter_map(|n| entry(id, n)).collect();
    Set { id, name, license, file, entries }
}

pub fn all() -> &'static [Set] {
    static S: OnceLock<Vec<Set>> = OnceLock::new();
    S.get_or_init(|| {
        vec![
            load("tabler", "Tabler Icons", "MIT", include_str!("../../../icons/tabler.json")),
            load("healthicons", "Health Icons", "CC0 1.0", include_str!("../../../icons/healthicons.json")),
        ]
    })
}

impl Set {
    pub fn get(&self, name: &str) -> Option<Found<'_>> {
        let mut n = name;
        for _ in 0..4 {
            if let Some(r) = self.file.icons.get(n) {
                return Some(Found {
                    body: &r.body,
                    w: r.width.unwrap_or(self.file.width),
                    h: r.height.unwrap_or(self.file.height),
                });
            }
            match self.file.aliases.get(n) {
                Some(a) if a.rest.is_empty() => n = &a.parent,
                _ => return None,
            }
        }
        None
    }

    /// Every real icon name (not aliases), sorted.
    #[cfg(test)]
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.file.icons.keys()
    }
}

pub fn find(id: &str) -> Option<Found<'static>> {
    let (set, name) = id.split_once(':')?;
    all().iter().find(|s| s.id == set)?.get(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sets_load_and_count() {
        let s = crate::sets();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].id, s[1].id), ("tabler", "healthicons"));
        assert_eq!(s[0].count, 6199);
        // 2797 minus the -negative and -24px spellings
        assert!(s[1].count > 800 && s[1].count < 2797, "{}", s[1].count);
        assert_eq!(s[0].license, "MIT");
    }

    #[test]
    fn an_id_names_one_icon_and_aliases_resolve() {
        assert!(crate::exists("tabler:server"));
        assert!(crate::exists("healthicons:doctor-outline"));
        assert!(crate::exists("healthicons:crutches-negative"));
        assert!(!crate::exists("tabler:no-such-icon"));
        assert!(!crate::exists("nosuch:server"));
        assert!(!crate::exists("server"));
        // alias with only a parent
        let a = find("tabler:12-hours").unwrap();
        let b = find("tabler:hours-12").unwrap();
        assert_eq!(a.body, b.body);
    }

    #[test]
    fn svg_wraps_the_sets_own_drawing() {
        let s = crate::svg("tabler:server").unwrap();
        assert!(s.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">"));
        assert!(s.ends_with("</svg>"));
        assert!(s.contains("M3 7a3 3 0 0 1 3-3"));
        assert!(crate::svg("healthicons:doctor").unwrap().contains("viewBox=\"0 0 48 48\""));
        assert!(crate::svg("tabler:nope").is_none());
    }
}
