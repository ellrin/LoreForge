//! Raw table access to the profile's CSV files for the database editor.
//!
//! The editor works on strings, not typed rows, so columns the app does not
//! know about survive a round trip. Validation happens by reloading the
//! profile after a save, which reports problems per file and line.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Editable files and their default columns (used when the file is missing).
pub const TABLES: &[(&str, &[&str])] = &[
    ("phrases.csv", &["id", "section", "function", "position", "template", "example", "tags", "priority"]),
    ("expressions.csv", &["id", "kind", "text", "usage", "section", "example", "tags"]),
    ("vocabulary.csv", &["avoid", "prefer", "note"]),
    ("rules.csv", &["id", "type", "scope", "check", "threshold", "severity", "message"]),
    ("exemplars.csv", &["id", "section", "function", "text", "source"]),
    ("equations.csv", &["id", "name", "latex", "tags"]),
    ("authors.csv", &["id", "given", "family", "email", "orcid", "affiliations", "role", "note"]),
    ("affiliations.csv", &["id", "department", "organization", "city", "country"]),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn default_headers(name: &str) -> Option<Vec<String>> {
    TABLES.iter().find(|(n, _)| *n == name).map(|(_, h)| h.iter().map(|s| s.to_string()).collect())
}

pub fn read_table(dir: &Path, name: &str) -> Result<Table, String> {
    let headers = default_headers(name).ok_or_else(|| format!("not an editable table: {name}"))?;
    let path = dir.join(name);
    if !path.exists() {
        return Ok(Table { headers, rows: vec![] });
    }
    let mut r = csv::ReaderBuilder::new().flexible(true).from_path(&path).map_err(|e| e.to_string())?;
    let headers: Vec<String> = r.headers().map_err(|e| e.to_string())?.iter().map(str::to_string).collect();
    let mut rows = Vec::new();
    for rec in r.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let mut row: Vec<String> = rec.iter().map(str::to_string).collect();
        row.resize(headers.len(), String::new());
        rows.push(row);
    }
    Ok(Table { headers, rows })
}

pub fn write_table(dir: &Path, name: &str, t: &Table) -> Result<(), String> {
    default_headers(name).ok_or_else(|| format!("not an editable table: {name}"))?;
    if t.headers.is_empty() {
        return Err("table has no columns".into());
    }
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    w.write_record(&t.headers).map_err(|e| e.to_string())?;
    for row in &t.rows {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue; // drop blank rows
        }
        let mut row = row.clone();
        row.resize(t.headers.len(), String::new());
        w.write_record(row.iter().map(|c| c.trim())).map_err(|e| e.to_string())?;
    }
    let bytes = w.into_inner().map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    crate::paper::write_atomic(&dir.join(name), &text).map_err(|e| e.to_string())
}

/// Next free id with the given prefix, e.g. `P018` after `P017`.
pub fn next_id<'a>(existing: impl IntoIterator<Item = &'a str>, prefix: &str) -> String {
    let max = existing
        .into_iter()
        .filter_map(|id| id.strip_prefix(prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{:03}", max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_unknown_columns_and_drops_blank_rows() {
        let dir = std::env::temp_dir().join(format!("pc-table-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vocabulary.csv"), "avoid,prefer,note,extra\nutilize,use,,x\n").unwrap();
        let mut t = read_table(&dir, "vocabulary.csv").unwrap();
        assert_eq!(t.headers.len(), 4);
        t.rows.push(vec!["".into(); 4]);
        t.rows.push(vec!["a lot of".into(), "many".into()]);
        write_table(&dir, "vocabulary.csv", &t).unwrap();
        let back = read_table(&dir, "vocabulary.csv").unwrap();
        assert_eq!(back.rows.len(), 2);
        assert_eq!(back.rows[0][3], "x");
        assert!(read_table(&dir, "../etc/passwd").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn next_id_counts_up() {
        assert_eq!(next_id(["P001", "P017", "X003"], "P"), "P018");
        assert_eq!(next_id([], "X"), "X001");
    }
}
