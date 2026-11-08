//! `config.toml` at the workspace root: what the UI remembers between
//! sessions (light or dark, colour, the paper last open, ...). It lives with
//! the user's own data, so it syncs through git like the rest.

use serde_json::{Map, Value};
use std::path::Path;

const FILE: &str = "config.toml";

/// The `[ui]` table as JSON; empty when there is no file yet.
pub fn read(ws: &Path) -> Map<String, Value> {
    let Ok(text) = std::fs::read_to_string(ws.join(FILE)) else { return Map::new() };
    let Ok(doc) = text.parse::<toml::Table>() else { return Map::new() };
    match doc.get("ui").and_then(|u| serde_json::to_value(u).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// Replace the `[ui]` table, keeping anything else in the file.
pub fn write(ws: &Path, ui: &Map<String, Value>) -> Result<(), String> {
    let path = ws.join(FILE);
    let mut doc = std::fs::read_to_string(&path).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default();
    let mut table = toml::Table::new();
    for (k, v) in ui {
        if let Some(v) = to_toml(v) {
            table.insert(k.clone(), v);
        }
    }
    doc.insert("ui".into(), toml::Value::Table(table));
    let text = format!("# PaperComposer remembers these between sessions. Safe to edit or delete.\n\n{}", toml::to_string(&doc).map_err(|e| e.to_string())?);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    pc_core::paper::write_atomic(&path, &text).map_err(|e| e.to_string())
}

/// Scalars only; TOML has no null.
fn to_toml(v: &Value) -> Option<toml::Value> {
    match v {
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Number(n) => n.as_i64().map(toml::Value::Integer).or_else(|| n.as_f64().map(toml::Value::Float)),
        Value::String(s) => Some(toml::Value::String(s.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_keeps_other_tables() {
        let dir = std::env::temp_dir().join(format!("pc-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "[other]\nx = 1\n").unwrap();
        let ui: Map<String, Value> = serde_json::from_str(r#"{"theme":"blue","dim":true,"zoom":1.25,"paper":null}"#).unwrap();
        write(&dir, &ui).unwrap();
        let back = read(&dir);
        assert_eq!(back.get("theme"), Some(&Value::String("blue".into())));
        assert_eq!(back.get("dim"), Some(&Value::Bool(true)));
        assert!(back.get("paper").is_none());
        assert!(std::fs::read_to_string(dir.join(FILE)).unwrap().contains("[other]"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
