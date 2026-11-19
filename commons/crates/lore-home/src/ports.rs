//! Which port each running Lore desktop app serves on.
//!
//! The desktop apps bind a random free port, so the others cannot guess it. Each app, when
//! its server binds, records `{port, pid}` under its short name (`pc`, `sc`, `lr`, `kb`) in
//! `<OS cache dir>/LoreForge/ports.json`. This is runtime data (wiped with the cache, never
//! private data, never in a repo). Entries of dead processes are ignored and replaced.
//! `LOREFORGE_CACHE` moves the folder (tests).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

// ── starting another app ──────────────────────────────────────────

/// How long [`ensure`] waits for an app it launched.
pub const WAIT: Duration = Duration::from_secs(20);
/// After a launch that did not come up, [`ensure`] does not launch the same app again for this
/// long (a missing app does not make every request wait [`WAIT`]).
pub const COOLDOWN: Duration = Duration::from_secs(60);

/// The application name (`open -a`) of a short name.
pub fn app_name(id: &str) -> &str {
    match id {
        "pc" => "L-Paper",
        "sc" => "L-Slide",
        "lr" => "L-Reader",
        "kb" => "L-Brain",
        "le" => "L-Echo",
        "ls" => "L-Shelf",
        other => other,
    }
}

/// Does something listen on `127.0.0.1:port`?
fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(400)).is_ok()
}

/// Start the app in the background (macOS `open -g -a`).
fn os_launch(name: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(format!("{name} 沒有開"));
    }
    let ok = std::process::Command::new("open").args(["-g", "-a", name]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(format!("找不到 {name} App")) }
}

/// A Lore app's port, starting the app in the background when it is not running: a request
/// the author made never stops at "please open it first". `fallback` = the port its browser
/// mode serves on (taken when something answers there). Blocking (up to [`WAIT`]); one launch
/// at a time per process, none again within [`COOLDOWN`] of one that did not come up.
/// Callers decide when this is allowed: never from tests, isolated examples or polls.
pub fn ensure(id: &str, fallback: Option<u16>) -> Result<u16, String> {
    static FAILED: Mutex<BTreeMap<String, Instant>> = Mutex::new(BTreeMap::new());
    let path = default_path().ok_or_else(|| format!("{} 沒有開", app_name(id)))?;
    ensure_with(&path, id, fallback, &FAILED, WAIT, &os_launch)
}

/// [`ensure`] over a given ports file, failure record, wait and launcher (tests).
pub fn ensure_with(path: &Path, id: &str, fallback: Option<u16>, failed: &Mutex<BTreeMap<String, Instant>>, wait: Duration, launch: &dyn Fn(&str) -> Result<(), String>) -> Result<u16, String> {
    let name = app_name(id);
    let up = || live_at(path).get(id).copied().or_else(|| fallback.filter(|p| answers(*p)));
    if let Some(p) = up() {
        return Ok(p);
    }
    let mut failed = failed.lock().unwrap_or_else(|e| e.into_inner());
    // another request may have started it while this one waited for the lock
    if let Some(p) = up() {
        return Ok(p);
    }
    let no_answer = format!("{name} 沒有回應（打開 {name} 再試一次）");
    if failed.get(id).is_some_and(|t| t.elapsed() < COOLDOWN) {
        return Err(no_answer);
    }
    if let Err(e) = launch(name) {
        failed.insert(id.to_string(), Instant::now());
        return Err(e);
    }
    let until = Instant::now() + wait;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(p) = up() {
            failed.remove(id);
            return Ok(p);
        }
    }
    failed.insert(id.to_string(), Instant::now());
    Err(no_answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_launches_once_waits_and_cools_down() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(FILE);
        let failed = Mutex::new(BTreeMap::new());
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let me = std::process::id();
        // running: no launch
        register_at(&f, "sc", 51234, me).unwrap();
        let never = |_: &str| -> Result<(), String> { panic!("launched a running app") };
        assert_eq!(ensure_with(&f, "sc", None, &failed, Duration::from_millis(10), &never), Ok(51234));
        // browser mode: something answers on the fallback port
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert_eq!(ensure_with(&f, "kb", Some(port), &failed, Duration::from_millis(10), &never), Ok(port));
        drop(l);
        // not running: launched by its app name, and found once it records its port
        let starts = |name: &str| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(name, "L-Brain");
            register_at(&f, "kb", 7701, me).map_err(|e| e.to_string())
        };
        assert_eq!(ensure_with(&f, "kb", None, &failed, Duration::from_secs(2), &starts), Ok(7701));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        // launched but never comes: an error, and no second launch within the cooldown
        let silent = |_: &str| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        };
        assert!(ensure_with(&f, "lr", None, &failed, Duration::from_millis(50), &silent).unwrap_err().contains("L-Reader"));
        assert!(ensure_with(&f, "lr", None, &failed, Duration::from_millis(50), &silent).is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2, "one launch per cooldown");
    }

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
