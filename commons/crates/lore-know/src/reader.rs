//! What an app needs without opening the whole hub (it works with L-Brain closed): where the
//! hub is, the apps' declarations, the items outside applications aligned to one of its
//! categories, and the usage log. The signatures of `hub_dir`, `builtin_manifests` and `aligned`
//! are frozen.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::Item;

/// The hub folder of the author's personal space: `<private>/KnowledgeBase/hub`.
pub fn hub_dir(home: &lore_home::Home) -> Option<PathBuf> {
    home.app_dir("KnowledgeBase").map(|d| d.join("hub"))
}

/// Every Lore app's declaration of its categories (`manifests/*.toml` of this crate).
pub fn builtin_manifests() -> Vec<&'static str> {
    vec![
        include_str!("../manifests/pc.toml"),
        include_str!("../manifests/sc.toml"),
        include_str!("../manifests/lr.toml"),
        include_str!("../manifests/shared.toml"),
    ]
}

/// The items outside applications aligned to `category`, as the hub keeps them (never in the
/// app's own data): the records under `<hub>/items/<category>/` whose source is external, read
/// straight from the folder. Empty when the hub or the category has none; never an error;
/// unreadable files are skipped.
pub fn aligned(hub: &Path, category: &str) -> Vec<Item> {
    if category.is_empty() || category.contains(['/', '\\', '\0']) || category.starts_with('.') {
        return vec![];
    }
    let Ok(rd) = fs::read_dir(hub.join("items").join(category)) else { return vec![] };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    let mut out = vec![];
    for p in files {
        let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
        let Ok(text) = fs::read_to_string(&p) else { continue };
        let Ok(mut item) = serde_json::from_str::<Item>(&text) else { continue };
        if !item.source.is_external() {
            continue;
        }
        item.id = crate::item_id(category, &stem);
        item.category = category.to_string();
        out.push(item);
    }
    out
}

// ── the usage log ────────────────────────────────────────────────────────────

/// `<hub>/usage.jsonl`: one line per call of a step that took items into a prompt.
pub fn usage_path(hub: &Path) -> PathBuf {
    hub.join("usage.jsonl")
}

/// Append one line `{"day":"YYYY-MM-DD","task":"…","ids":[…]}` to the usage log of `hub`
/// (made when missing). For an app that has no [`crate::hub::Hub`] open; nothing else is touched.
pub fn record_use(hub: &Path, task: &str, ids: &[String], day: NaiveDate) -> crate::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let q = |s: &str| serde_json::Value::String(s.into()).to_string();
    let line = format!(
        "{{\"day\":{},\"task\":{},\"ids\":{}}}",
        q(&day.format("%Y-%m-%d").to_string()),
        q(task),
        serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
    );
    fs::create_dir_all(hub)?;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(usage_path(hub))?;
    // one write call, so lines of two processes do not interleave
    f.write_all(format!("{line}\n").as_bytes())?;
    Ok(())
}

/// The log's lines from `since` on (a line of that day included), oldest first. Lines that are
/// not in the shape above are skipped; a missing log is empty.
pub fn usage(hub: &Path, since: NaiveDate) -> Vec<(NaiveDate, String, Vec<String>)> {
    let Ok(text) = fs::read_to_string(usage_path(hub)) else { return vec![] };
    let mut out = vec![];
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        let Some(day) = v.get("day").and_then(|d| d.as_str()).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else { continue };
        let Some(task) = v.get("task").and_then(|t| t.as_str()) else { continue };
        let Some(ids) = v.get("ids").and_then(|i| i.as_array()) else { continue };
        if day < since {
            continue;
        }
        out.push((day, task.to_string(), ids.iter().filter_map(|i| i.as_str().map(String::from)).collect()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{format, Source};

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn aligned_reads_the_records_without_a_store() {
        let hub = tempfile::tempdir().unwrap();
        let dir = hub.path().join("items/pc.patterns");
        fs::create_dir_all(&dir).unwrap();
        let mk = |local: &str, source: Source| {
            let mut i = Item::new("pc.patterns", format::PATTERN, local, source);
            i.set("template", local);
            fs::write(dir.join(format!("{local}.json")), serde_json::to_string(&i).unwrap()).unwrap();
        };
        mk("acme.1", Source::external("acme"));
        mk("acme.2", Source::external("acme"));
        mk("own", Source::lore("pc"));
        fs::write(dir.join("broken.json"), "{ no").unwrap();
        fs::write(dir.join("note.txt"), "x").unwrap();
        let got = aligned(hub.path(), "pc.patterns");
        let ids: Vec<&str> = got.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["pc.patterns:acme.1", "pc.patterns:acme.2"], "external only, unreadable skipped");
        assert_eq!(got[0].source, Source::external("acme"));
        assert!(aligned(hub.path(), "pc.rules").is_empty());
        assert!(aligned(&hub.path().join("nope"), "pc.patterns").is_empty());
        assert!(aligned(hub.path(), "../x").is_empty());
    }

    #[test]
    fn use_is_logged_by_step_and_day() {
        let hub = tempfile::tempdir().unwrap();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        record_use(hub.path(), "draft", &ids(&["pc.patterns:a", "pc.rules:b"]), d("2026-11-01")).unwrap();
        record_use(hub.path(), "polish", &ids(&["pc.patterns:a"]), d("2026-11-20")).unwrap();
        record_use(hub.path(), "draft", &[], d("2026-11-21")).unwrap();
        let text = fs::read_to_string(usage_path(hub.path())).unwrap();
        assert_eq!(text.lines().count(), 2, "one line per call, none for no ids");
        assert!(text.starts_with(r#"{"day":"2026-11-01","task":"draft","ids":["pc.patterns:a","pc.rules:b"]}"#), "{text}");
        let all = usage(hub.path(), d("2026-01-01"));
        assert_eq!(all.len(), 2);
        assert_eq!((all[1].0, all[1].1.as_str(), all[1].2.clone()), (d("2026-11-20"), "polish", ids(&["pc.patterns:a"])));
        let late = usage(hub.path(), d("2026-11-20"));
        assert_eq!(late.len(), 1);
        assert!(usage(&hub.path().join("none"), d("2026-01-01")).is_empty());
    }

    #[test]
    fn bad_lines_are_skipped() {
        let hub = tempfile::tempdir().unwrap();
        let log = concat!(
            "{\"day\":\"2026-11-02\",\"task\":\"draft\",\"ids\":[\"a:1\"]}\n",
            "not json at all\n",
            "\n",
            "{\"day\":\"2026-13-40\",\"task\":\"draft\",\"ids\":[\"a:2\"]}\n",
            "{\"day\":\"2026-11-03\",\"ids\":[\"a:3\"]}\n",
            "{\"day\":\"2026-11-04\",\"task\":\"draft\"}\n",
            "{\"day\":\"2026-11-05\",\"task\":\"polish\",\"ids\":[\"a:4\",7]}\n",
        );
        fs::write(usage_path(hub.path()), log).unwrap();
        let got = usage(hub.path(), d("2026-11-01"));
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[1].2, ["a:4"]);
    }
}
