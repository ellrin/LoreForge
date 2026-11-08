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

/// Append rows given as `column → value`, each with the next free id for
/// `prefix`; `default` fills columns a row leaves empty. A row whose `key`
/// column equals an existing row's (trimmed) is skipped, so saving the same
/// sentence twice adds it once. Returns the ids of the rows added.
pub fn append_rows(dir: &Path, name: &str, prefix: &str, key: &str, default: &[(&str, &str)], rows: &[Vec<(&str, String)>]) -> Result<Vec<String>, String> {
    let mut t = read_table(dir, name)?;
    let col = |c: &str| t.headers.iter().position(|h| h == c);
    let (id_col, key_col) = (col("id"), col(key));
    let mut added = Vec::new();
    for values in rows {
        let key_val = values.iter().find(|(c, _)| *c == key).map(|(_, v)| v.trim()).unwrap_or("");
        if let Some(k) = key_col {
            if !key_val.is_empty() && t.rows.iter().any(|r| r.get(k).is_some_and(|v| v.trim() == key_val)) {
                continue;
            }
        }
        let mut row = vec![String::new(); t.headers.len()];
        for (c, v) in default.iter().map(|(c, v)| (*c, v.to_string())).chain(values.iter().cloned()) {
            if let Some(i) = col(c) {
                if !v.is_empty() {
                    row[i] = v;
                }
            }
        }
        if let Some(i) = id_col {
            let id = next_id(t.rows.iter().filter_map(|r| r.get(i).map(String::as_str)), prefix);
            row[i] = id.clone();
            added.push(id);
        }
        t.rows.push(row);
    }
    if !added.is_empty() {
        write_table(dir, name, &t)?;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_numbers_rows_fills_defaults_and_skips_duplicates() {
        let dir = std::env::temp_dir().join(format!("pc-append-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("phrases.csv"), "id,section,function,position,template,example,tags,priority\nP007,Results,finding,body,We found {x}.,,,5\n").unwrap();
        let row = |tpl: &str| vec![("section", "Introduction".to_string()), ("template", tpl.to_string()), ("tags", "novelty;first".to_string())];
        let ids = append_rows(&dir, "phrases.csv", "P", "template", &[("priority", "3")], &[row("To our knowledge, {claim}."), row("We found {x}."), row("To our knowledge, {claim}.")]).unwrap();
        assert_eq!(ids, vec!["P008"]);
        let t = read_table(&dir, "phrases.csv").unwrap();
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1], vec!["P008", "Introduction", "", "", "To our knowledge, {claim}.", "", "novelty;first", "3"]);
        assert!(append_rows(&dir, "phrases.csv", "P", "template", &[], &[row("We found {x}.")]).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

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
