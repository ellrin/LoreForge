//! The engines every LoreForge app uses, chosen once: `private/shared/engine.toml`
//!
//! ```toml
//! [engine]                           # the local model (written by the app that runs it)
//! url = "http://127.0.0.1:8080/v1"   # an OpenAI-compatible server (llama-server, Ollama, LM Studio)
//! model = "qwen3.5-9b-q8"            # optional: the catalog id or the server's model name
//!
//! [claude]                           # the Claude connection (never a key: keys are in the keychain)
//! via = "subscription"               # subscription | api
//! model = "sonnet"                   # catalog alias or id
//!
//! [route]                            # per task class: auto | cloud | local
//! create = "auto"                    # 創作與審稿
//! tidy = "auto"                      # 整理與格式
//! ask = "auto"                       # 問答
//!
//! [overrides]                        # per task id, wins over its class
//! pc-draft-turn = "local"
//! ```
//!
//! An app's own `[engine]` (its config.toml or environment) wins; this is the default
//! when it has none, so the author does not configure the same server in every app.
//! The app that manages a local model (PaperComposer) writes it with
//! [`Home::set_engine`] when the author picks or starts one. Read it on every use: the
//! server may move to another port. Never holds keys. Every setter keeps the other
//! tables (and unknown keys) of the file.

use crate::Home;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The file in `private/shared` that holds the shared engine.
pub const FILE: &str = "engine.toml";

/// Ways into Claude (`[claude] via`).
pub const VIAS: [&str; 2] = ["subscription", "api"];
/// Task classes (`[route]` keys): 創作與審稿, 整理與格式, 問答.
pub const CLASSES: [&str; 3] = ["create", "tidy", "ask"];
/// Routes of a class or a task: `auto` (the class order), `cloud` (Claude only), `local` (local only).
pub const ROUTES: [&str; 3] = ["auto", "cloud", "local"];

/// `[engine]` of [`FILE`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SharedEngine {
    /// Base URL ending in `/v1`.
    pub url: Option<String>,
    pub model: Option<String>,
}

/// `[claude]` of [`FILE`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ClaudeChoice {
    /// `subscription` | `api`; `None` = the app decides.
    pub via: Option<String>,
    /// Catalog alias or id; `None` = the app's own pick.
    pub model: Option<String>,
}

/// Everything in [`FILE`], checked: unknown values are dropped (with a note on stderr).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineSettings {
    pub engine: Option<SharedEngine>,
    pub claude: ClaudeChoice,
    /// Class → route; a missing class is `auto`.
    pub route: BTreeMap<String, String>,
    /// Task id → route (`auto` is never kept: it means "follow the class").
    pub overrides: BTreeMap<String, String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EngineFile {
    engine: SharedEngine,
    claude: ClaudeChoice,
    route: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
}

fn clean(v: Option<String>) -> Option<String> {
    v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn routes(m: BTreeMap<String, String>, table: &str, keys: Option<&[&str]>) -> BTreeMap<String, String> {
    m.into_iter()
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, v)| {
            let ok = ROUTES.contains(&v.as_str()) && keys.is_none_or(|ks| ks.contains(&k.as_str()));
            if !ok {
                eprintln!("lore-home: {FILE}: [{table}] {k} = \"{v}\" ignored");
            }
            ok
        })
        .collect()
}

impl Home {
    fn engine_file(&self) -> Option<PathBuf> {
        self.shared_dir().map(|d| d.join(FILE))
    }

    fn read_engine_file(&self) -> Option<EngineFile> {
        let s = std::fs::read_to_string(self.engine_file()?).ok()?;
        toml::from_str(&s).map_err(|e| eprintln!("lore-home: {FILE}: {e}")).ok()
    }

    /// The shared local engine, when one is set (a non-empty `url`).
    pub fn engine(&self) -> Option<SharedEngine> {
        let f = self.read_engine_file()?;
        let url = clean(f.engine.url)?;
        Some(SharedEngine { url: Some(url), model: clean(f.engine.model) })
    }

    /// The whole file: local engine, Claude choice, class routes and task overrides.
    /// A missing or broken file is the defaults (nothing chosen, every route `auto`).
    pub fn engine_settings(&self) -> EngineSettings {
        let Some(f) = self.read_engine_file() else { return EngineSettings::default() };
        let url = clean(f.engine.url);
        let engine = url.map(|u| SharedEngine { url: Some(u), model: clean(f.engine.model) });
        let via = clean(f.claude.via).filter(|v| {
            let ok = VIAS.contains(&v.as_str());
            if !ok {
                eprintln!("lore-home: {FILE}: [claude] via = \"{v}\" ignored");
            }
            ok
        });
        let mut overrides = routes(f.overrides, "overrides", None);
        overrides.retain(|_, v| v != "auto");
        EngineSettings {
            engine,
            claude: ClaudeChoice { via, model: clean(f.claude.model) },
            route: routes(f.route, "route", Some(&CLASSES)),
            overrides,
        }
    }

    /// Change the file as a TOML table and write it back atomically.
    fn edit_engine_file(&self, f: impl FnOnce(&mut toml::Table)) -> std::io::Result<()> {
        let path = self.engine_file().ok_or_else(|| std::io::Error::other("no shared settings folder"))?;
        let mut t: toml::Table = std::fs::read_to_string(&path).ok().and_then(|s| s.parse().ok()).unwrap_or_default();
        f(&mut t);
        let dir = path.parent().ok_or_else(|| std::io::Error::other("no shared settings folder"))?;
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, t.to_string())?;
        std::fs::rename(&tmp, &path)
    }

    /// Keep `url` / `model` as the shared engine (`url = None` clears it; other tables
    /// of the file stay).
    pub fn set_engine(&self, url: Option<&str>, model: Option<&str>) -> std::io::Result<()> {
        self.edit_engine_file(|t| match url.map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => {
                let mut e = toml::Table::new();
                e.insert("url".into(), toml::Value::String(u.into()));
                if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
                    e.insert("model".into(), toml::Value::String(m.into()));
                }
                t.insert("engine".into(), toml::Value::Table(e));
            }
            None => {
                t.remove("engine");
            }
        })
    }

    /// Set `[claude] via` and/or `model`: `None` leaves a value as it is, `Some("")`
    /// removes it. `via` must be one of [`VIAS`].
    pub fn set_claude(&self, via: Option<&str>, model: Option<&str>) -> std::io::Result<()> {
        if let Some(v) = via.map(str::trim).filter(|v| !v.is_empty())
            && !VIAS.contains(&v)
        {
            return Err(std::io::Error::other(format!("unknown Claude connection: {v}")));
        }
        self.edit_engine_file(|t| {
            for (k, v) in [("via", via), ("model", model)] {
                match v.map(str::trim) {
                    Some("") => set_in(t, "claude", k, None),
                    Some(v) => set_in(t, "claude", k, Some(v)),
                    None => {}
                }
            }
        })
    }

    /// `[route] <class> = <route>` (one of [`CLASSES`] and of [`ROUTES`]).
    pub fn set_route(&self, class: &str, route: &str) -> std::io::Result<()> {
        if !CLASSES.contains(&class) || !ROUTES.contains(&route) {
            return Err(std::io::Error::other(format!("unknown class or route: {class} = {route}")));
        }
        self.edit_engine_file(|t| set_in(t, "route", class, Some(route)))
    }

    /// `[overrides] <task> = <route>`; `auto` or `None` removes the task's line (back to its class).
    pub fn set_override(&self, task: &str, route: Option<&str>) -> std::io::Result<()> {
        let task = task.trim();
        if task.is_empty() || route.is_some_and(|r| !ROUTES.contains(&r)) {
            return Err(std::io::Error::other(format!("unknown task or route: {task} = {route:?}")));
        }
        self.edit_engine_file(|t| set_in(t, "overrides", task, route.filter(|r| *r != "auto")))
    }
}

/// `t[table][key] = value` (`None` removes the key), making `table` a table if it is not one.
fn set_in(t: &mut toml::Table, table: &str, key: &str, value: Option<&str>) {
    let e = t.entry(table).or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !e.is_table() {
        *e = toml::Value::Table(toml::Table::new());
    }
    let e = e.as_table_mut().expect("a table");
    match value {
        Some(v) => {
            e.insert(key.into(), toml::Value::String(v.into()));
        }
        None => {
            e.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_round_trip_and_empty_values() {
        let d = tempfile::tempdir().unwrap();
        let home = Home::at(d.path());
        assert_eq!(home.engine(), None);
        home.set_engine(Some("http://127.0.0.1:8123/v1"), Some("qwen3.5-4b-q8")).unwrap();
        assert_eq!(home.engine(), Some(SharedEngine { url: Some("http://127.0.0.1:8123/v1".into()), model: Some("qwen3.5-4b-q8".into()) }));
        let f = d.path().join("private/shared/engine.toml");
        std::fs::write(&f, "note = \"kept\"\n[engine]\nurl = \"  \"\nmodel = \"x\"\n").unwrap();
        assert_eq!(home.engine(), None, "an empty url is no engine");
        home.set_engine(Some("http://127.0.0.1:9/v1"), None).unwrap();
        assert!(std::fs::read_to_string(&f).unwrap().contains("note = \"kept\""));
        assert_eq!(home.engine().unwrap().model, None);
        home.set_engine(None, None).unwrap();
        assert_eq!(home.engine(), None);
        std::fs::write(&f, "[engine]\nurl = 3\n").unwrap();
        assert_eq!(home.engine(), None, "a broken file is no engine");
    }

    #[test]
    fn claude_routes_and_overrides_round_trip_and_keep_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let home = Home::at(d.path());
        assert_eq!(home.engine_settings(), EngineSettings::default());
        home.set_engine(Some("http://127.0.0.1:8123/v1"), Some("qwen3.5-4b-q8")).unwrap();
        home.set_claude(Some("api"), Some("sonnet")).unwrap();
        home.set_route("create", "cloud").unwrap();
        home.set_route("ask", "local").unwrap();
        home.set_override("pc-draft-turn", Some("local")).unwrap();
        home.set_override("lr-chat", Some("cloud")).unwrap();
        let s = home.engine_settings();
        assert_eq!(s.engine.unwrap().model.as_deref(), Some("qwen3.5-4b-q8"));
        assert_eq!(s.claude, ClaudeChoice { via: Some("api".into()), model: Some("sonnet".into()) });
        assert_eq!(s.route.get("create").map(String::as_str), Some("cloud"));
        assert_eq!(s.overrides.len(), 2);

        // auto removes an override; the engine table survives the other setters
        home.set_override("lr-chat", Some("auto")).unwrap();
        home.set_override("pc-draft-turn", None).unwrap();
        home.set_claude(None, Some("")).unwrap();
        home.set_engine(Some("http://127.0.0.1:8124/v1"), None).unwrap();
        let s = home.engine_settings();
        assert!(s.overrides.is_empty());
        assert_eq!(s.claude, ClaudeChoice { via: Some("api".into()), model: None });
        assert_eq!(s.route.len(), 2, "set_engine keeps the routes");

        // bad values are refused when written and dropped when read
        assert!(home.set_claude(Some("carrier-pigeon"), None).is_err());
        assert!(home.set_route("draw", "auto").is_err() && home.set_route("tidy", "sky").is_err());
        assert!(home.set_override("x", Some("nope")).is_err());
        let f = d.path().join("private/shared/engine.toml");
        std::fs::write(&f, "[overrides]\nc = 3\n").unwrap();
        assert_eq!(home.engine_settings(), EngineSettings::default(), "a broken file is the defaults");
        std::fs::write(&f, "[claude]\nvia = \"pigeon\"\n[route]\ncreate = \"sky\"\ntidy = \"local\"\nother = \"cloud\"\n[overrides]\na = \"auto\"\nb = \"cloud\"\n").unwrap();
        let s = home.engine_settings();
        assert_eq!(s.claude.via, None);
        assert_eq!(s.route.into_iter().collect::<Vec<_>>(), vec![("tidy".to_string(), "local".to_string())]);
        assert_eq!(s.overrides.into_iter().collect::<Vec<_>>(), vec![("b".to_string(), "cloud".to_string())]);
    }
}
