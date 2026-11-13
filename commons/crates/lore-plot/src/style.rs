//! Style presets as data: public neutral defaults in `commons/ui/plot/styles/*.json`
//! (compiled in), the author's own in `private/shared/plot/styles/*.{json,toml}` (same id
//! wins), and `private/shared/plot/settings.toml` (default style, font families put in
//! front of every style of that kind). Styles name a `base`; [`resolved`] merges the chain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};

/// The public presets, in menu order.
pub const PUBLIC: &[(&str, &str)] = &[
    ("clean", include_str!("../../../ui/plot/styles/clean.json")),
    ("nature", include_str!("../../../ui/plot/styles/nature.json")),
    ("npj", include_str!("../../../ui/plot/styles/npj.json")),
    ("ieee", include_str!("../../../ui/plot/styles/ieee.json")),
    ("cvpr", include_str!("../../../ui/plot/styles/cvpr.json")),
    ("slide", include_str!("../../../ui/plot/styles/slide.json")),
    ("pastel", include_str!("../../../ui/plot/styles/pastel.json")),
];

/// `private/shared/plot/settings.toml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Settings {
    /// Style for a new figure (`mine`, `nature` …).
    #[serde(default)]
    pub default_style: Option<String>,
    /// Style for a new slide figure.
    #[serde(default)]
    pub slide_style: Option<String>,
    #[serde(default)]
    pub font: Fonts,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Fonts {
    /// Tried first in every serif style.
    #[serde(default)]
    pub serif: Vec<String>,
    /// Tried first in every sans style.
    #[serde(default)]
    pub sans: Vec<String>,
}

/// The `plot/` folder in the shared private folder.
pub fn private_dir(home: &lore_home::Home) -> Option<PathBuf> {
    home.shared_dir().map(|d| d.join("plot"))
}

pub fn settings(home: &lore_home::Home) -> Settings {
    private_dir(home).map(|d| read_settings(&d.join("settings.toml"))).unwrap_or_default()
}

pub fn read_settings(path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
            eprintln!("lore-plot: {}: {e} (using defaults)", path.display());
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

/// Raw styles by id: public, then the private folder's (same id replaces).
pub fn raw(private: Option<&Path>) -> Vec<Value> {
    let mut out: Vec<Value> = PUBLIC.iter().filter_map(|(_, s)| serde_json::from_str(s).ok()).collect();
    let Some(dir) = private.map(|d| d.join("styles")) else { return out };
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("json" | "toml"))).collect();
    files.sort();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let parsed: Result<Value, String> = if f.extension().and_then(|e| e.to_str()) == Some("toml") {
            toml::from_str::<toml::Value>(&text).map_err(|e| e.to_string()).and_then(|t| serde_json::to_value(t).map_err(|e| e.to_string()))
        } else {
            serde_json::from_str(&text).map_err(|e| e.to_string())
        };
        match parsed {
            Ok(mut v) if v.is_object() => {
                if v.get("id").and_then(Value::as_str).is_none() {
                    v["id"] = Value::from(f.file_stem().and_then(|s| s.to_str()).unwrap_or("mine"));
                }
                v["source"] = "private".into();
                let id = v["id"].as_str().unwrap_or_default().to_string();
                out.retain(|s| s.get("id").and_then(Value::as_str) != Some(id.as_str()));
                out.push(v);
            }
            Ok(_) => eprintln!("lore-plot: {}: not a table", f.display()),
            Err(e) => eprintln!("lore-plot: {}: {e}", f.display()),
        }
    }
    out
}

fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in y {
                match x.get_mut(k) {
                    Some(old) if old.is_object() && v.is_object() => merge(old, v),
                    _ => {
                        x.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}

/// Each style with its `base` chain merged in (no `base` left), private fonts in front.
pub fn resolved(raw: &[Value], fonts: &Fonts) -> Vec<Value> {
    let by: BTreeMap<&str, &Value> = raw.iter().filter_map(|s| Some((s.get("id")?.as_str()?, s))).collect();
    raw.iter()
        .filter_map(|s| {
            let id = s.get("id")?.as_str()?;
            let mut chain = vec![*by.get(id)?];
            let mut cur = *by.get(id)?;
            while let Some(b) = cur.get("base").and_then(Value::as_str) {
                let Some(next) = by.get(b) else { break };
                if chain.len() > 8 || chain.iter().any(|c| std::ptr::eq(*c, *next)) {
                    break;
                }
                chain.push(next);
                cur = next;
            }
            let mut out = Value::Object(Map::new());
            for c in chain.iter().rev() {
                merge(&mut out, c);
            }
            if let Some(o) = out.as_object_mut() {
                o.remove("base");
                o.insert("id".into(), id.into());
            }
            let kind = out.pointer("/font/kind").and_then(Value::as_str).unwrap_or("serif").to_string();
            let mine = if kind == "sans" { &fonts.sans } else { &fonts.serif };
            if !mine.is_empty()
                && let Some(fam) = out.pointer_mut("/font/family")
            {
                let rest: Vec<Value> = fam.as_array().cloned().unwrap_or_default().into_iter().filter(|f| !mine.iter().any(|m| Some(m.as_str()) == f.as_str())).collect();
                *fam = Value::Array(mine.iter().map(|m| Value::from(m.as_str())).chain(rest).collect());
            }
            Some(out)
        })
        .collect()
}

/// What `GET /plot/api/styles` answers: `{ styles, default, slide }`.
pub fn list(home: &lore_home::Home) -> Value {
    let dir = private_dir(home);
    let s = dir.as_deref().map(|d| read_settings(&d.join("settings.toml"))).unwrap_or_default();
    let styles = resolved(&raw(dir.as_deref()), &s.font);
    let has = |id: &str| styles.iter().any(|v| v["id"] == id);
    let default = s.default_style.filter(|d| has(d)).unwrap_or_else(|| "clean".into());
    let slide = s.slide_style.filter(|d| has(d)).unwrap_or_else(|| "slide".into());
    serde_json::json!({ "styles": styles, "default": default, "slide": slide })
}
