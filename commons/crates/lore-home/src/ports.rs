//! Which port each running Lore desktop app serves on.
//!
//! The desktop apps bind a random free port, so the others cannot guess it. Each app, when
//! its server binds, records `{port, pid}` under its short name (`pc`, `sc`, `lr`, `kb`) in
//! `<OS cache dir>/LoreForge/ports.json`. This is runtime data (wiped with the cache, never
//! private data, never in a repo). Entries of dead processes are ignored and replaced.
//! `LOREFORGE_CACHE` moves the folder (tests).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub const FILE: &str = "ports.json";

/// `<OS cache dir>/LoreForge` (or `LOREFORGE_CACHE`).
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LOREFORGE_CACHE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".cache")))?
    };
    Some(base.join("LoreForge"))
}

/// The default ports file.
pub fn default_path() -> Option<PathBuf> {
    cache_dir().map(|d| d.join(FILE))
}

/// Is a process with this id running?
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output();
        return out.map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string())).unwrap_or(false);
    }
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn read_raw(path: &Path) -> BTreeMap<String, (u16, u32)> {
    let Ok(text) = std::fs::read_to_string(path) else { return BTreeMap::new() };
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&text) else { return BTreeMap::new() };
    m.into_iter()
        .filter_map(|(app, v)| {
            let port = u16::try_from(v.get("port")?.as_u64()?).ok().filter(|p| *p != 0)?;
            let pid = u32::try_from(v.get("pid")?.as_u64()?).ok()?;
            Some((app, (port, pid)))
        })
        .collect()
}

/// The apps whose process is still running, `app → port`. A missing or broken file is empty.
pub fn live_at(path: &Path) -> BTreeMap<String, u16> {
    read_raw(path).into_iter().filter(|(_, (_, pid))| pid_alive(*pid)).map(|(app, (port, _))| (app, port)).collect()
}

/// Record `app` on `port` for process `pid`; entries of dead processes are dropped.
pub fn register_at(path: &Path, app: &str, port: u16, pid: u32) -> std::io::Result<()> {
    let mut all = read_raw(path);
    all.retain(|a, (_, p)| a != app && pid_alive(*p));
    all.insert(app.to_string(), (port, pid));
    let doc: serde_json::Map<String, Value> = all.into_iter().map(|(a, (port, pid))| (a, json!({ "port": port, "pid": pid }))).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{pid}"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&Value::Object(doc))?)?;
    std::fs::rename(&tmp, path)
}

/// [`register_at`] the default file, for this process. Failure is only reported on stderr:
/// the app works without it, its links just fall back to the default ports.
pub fn register(app: &str, port: u16) {
    let Some(p) = default_path() else { return };
    if let Err(e) = register_at(&p, app, port, std::process::id()) {
        eprintln!("ports: {e}");
    }
}

/// [`live_at`] the default file.
pub fn live() -> BTreeMap<String, u16> {
    default_path().map(|p| live_at(&p)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_and_dead_pids() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sub").join(FILE);
        assert!(live_at(&f).is_empty());
        let me = std::process::id();
        register_at(&f, "sc", 51234, me).unwrap();
        register_at(&f, "pc", 51235, me).unwrap();
        assert_eq!(live_at(&f), BTreeMap::from([("pc".to_string(), 51235), ("sc".to_string(), 51234)]));
        // a dead process is ignored, and replaced by the next registration
        register_at(&f, "lr", 7890, u32::MAX - 1).unwrap();
        assert!(!live_at(&f).contains_key("lr"));
        register_at(&f, "kb", 7700, me).unwrap();
        assert!(!read_raw(&f).contains_key("lr"), "dead entry dropped on write");
        // same app again: replaced
        register_at(&f, "sc", 60000, me).unwrap();
        assert_eq!(live_at(&f)["sc"], 60000);
        std::fs::write(&f, "not json").unwrap();
        assert!(live_at(&f).is_empty());
    }
}
