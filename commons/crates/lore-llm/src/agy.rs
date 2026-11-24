//! Google models through the Antigravity CLI (`agy -p`) the user installed and signed into
//! with their own account (subscription), for ONE job only: rewriting a passage (task
//! `polish`; the router refuses `agy:` refs for any task without `agy = true`).
//!
//! - The official `agy` binary does the work; this crate never reads or forwards credentials.
//! - `agy -p` takes its prompt as the flag's own value (`-p=<prompt>`), not on stdin, so the
//!   prompt is one argv element (no shell). The system prompt is written in front of it.
//! - Every call runs in an empty temporary folder, with slash commands off, and the child
//!   starts from the same environment allowlist as the Claude engine.
//! - Every call is `--mode plan`: the CLI is an agent with tools, and this is what denies them
//!   (see [`Agy::args`]); a reply that tried a tool is an error, never a half answer.
//! - No streaming, no images, no web (capabilities say so).
//! - [`ModelLister`] / [`CliLister`]: `agy models`; [`Stored`]: the last list in the cache
//!   folder (`agy-models.json`, wipeable runtime data). Model ids are never hard-coded.
//!
//! Output of `agy -p --output-format json` (observed): `conversation_id`, `status`
//! (`SUCCESS`), `response`, `duration_seconds`, `num_turns`, `usage { input_tokens,
//! output_tokens, thinking_tokens, cache_read_tokens, total_tokens }`. With `--json-schema`
//! the shape was not observed; a `structured_output` object, else `response`, is used.
//!
//! Tested with a fake `agy` executable only ([`fake`]).

use crate::claude_code::ALLOW_ENV;
use crate::error::LlmError;
use crate::provider::{LlmProvider, check_request};
use crate::types::{Capabilities, LlmRequest, LlmResponse, Usage};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const NAME: &str = "Google 模型（agy）";

/// The prompt is an argument; keep well under the OS limit.
const MAX_PROMPT_BYTES: usize = 800_000;

/// Binary discovery: `LORE_AGY_BIN`, then PATH, then `~/.local/bin/agy`.
pub fn binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LORE_AGY_BIN").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let exe = if cfg!(windows) { "agy.exe" } else { "agy" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
    }
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

fn command(bin: &Path) -> Command {
    let mut c = Command::new(bin);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    c.env_clear();
    for k in ALLOW_ENV {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    c.stdin(Stdio::null()).kill_on_drop(true);
    c
}

fn unique() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn not_found() -> LlmError {
    LlmError::NotConfigured("找不到 Antigravity CLI（agy），請先安裝並登入".into())
}

pub struct Agy {
    /// Explicit binary (tests); `None` = [`binary`].
    pub bin: Option<PathBuf>,
    /// A model id as `agy models` lists it.
    pub model: String,
    pub effort: Option<String>,
    pub timeout: Duration,
    pub context: u32,
    pub temp_prefix: String,
}

impl Agy {
    pub fn new(model: impl Into<String>) -> Agy {
        Agy { bin: None, model: model.into(), effort: None, timeout: Duration::from_secs(600), context: 128_000, temp_prefix: "lore-llm-agy".into() }
    }

    /// Command-line arguments for one call. The prompt is the value of `-p=`.
    ///
    /// `--mode plan` is what makes this a model and not an agent: `agy -p` has tools (read and
    /// write files, shell commands, the web) and without it uses them unasked, on any path, also
    /// with `--sandbox`. In plan mode a print-mode call cannot ask for permission, so every tool
    /// is denied (`denied_actions` in the output, read by [`parse_output`]). Checked on the real
    /// CLI: reading a file outside the folder, writing one, listing a folder, a shell command and
    /// a web fetch were each denied, a plain answer and a `--json-schema` answer were not affected.
    pub fn args(&self, req: &LlmRequest) -> Vec<String> {
        let prompt = if req.system.trim().is_empty() { req.flatten() } else { format!("{}\n\n{}", req.system.trim(), req.flatten()) };
        let mut a = vec![format!("-p={prompt}"), "--output-format".into(), "json".into(), "--disable-slash-commands".into(), "--mode".into(), "plan".into(), "--model".into(), self.model.clone()];
        if let Some(e) = req.effort.as_ref().or(self.effort.as_ref()) {
            a.extend(["--effort".into(), e.clone()]);
        }
        if let Some(schema) = &req.json_schema {
            a.push(format!("--json-schema={schema}"));
        }
        a
    }
}

#[async_trait]
impl LlmProvider for Agy {
    fn id(&self) -> String {
        format!("agy:{}", self.model)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { json: true, vision: false, web: false, streaming: false, context: self.context, needs_network: true }
    }

    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        check_request(NAME, &self.capabilities(), req)?;
        let bin = self.bin.clone().filter(|p| p.is_file()).or_else(|| if self.bin.is_none() { binary() } else { None }).ok_or_else(not_found)?;
        let args = self.args(req);
        if args[0].len() > MAX_PROMPT_BYTES {
            return Err(LlmError::Unsupported(NAME.into(), "這麼長的文字".into()));
        }
        let dir = std::env::temp_dir().join(format!("{}-{}-{}", self.temp_prefix, std::process::id(), unique()));
        std::fs::create_dir_all(&dir).map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
        let mut c = command(&bin);
        c.current_dir(&dir).args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        let result = async {
            let child = c.spawn().map_err(|e| LlmError::Network(NAME.into(), format!("無法啟動 agy：{e}")))?;
            let out = tokio::time::timeout(self.timeout, child.wait_with_output())
                .await
                .map_err(|_| LlmError::Network(NAME.into(), format!("超過 {} 秒沒有回應", self.timeout.as_secs())))?
                .map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
            parse_output(&out.stdout, &out.stderr, out.status.success())
        }
        .await;
        let _ = std::fs::remove_dir_all(&dir);
        let (text, usage) = result?;
        Ok(LlmResponse { text, provider: self.id(), sources: vec![], usage })
    }
}

/// The `--output-format json` result.
pub fn parse_output(stdout: &[u8], stderr: &[u8], ok: bool) -> Result<(String, Option<Usage>), LlmError> {
    let err_text = || {
        let e = String::from_utf8_lossy(stderr);
        let e = e.trim();
        if e.is_empty() { String::from_utf8_lossy(stdout).trim().chars().take(300).collect::<String>() } else { e.chars().take(300).collect() }
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        let e = err_text();
        let low = e.to_lowercase();
        if low.contains("login") || low.contains("sign in") || low.contains("sign-in") || low.contains("auth") {
            return Err(LlmError::NotConfigured("Antigravity CLI（agy）尚未登入".into()));
        }
        return Err(LlmError::Parse(NAME.into(), if e.is_empty() { "agy 沒有回傳結果".into() } else { e }));
    };
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("SUCCESS");
    let response = v.get("response").and_then(|r| r.as_str()).unwrap_or_default().to_string();
    if !ok || !status.eq_ignore_ascii_case("SUCCESS") {
        let msg = if response.is_empty() { v.get("error").and_then(|e| e.as_str()).map(String::from).unwrap_or_else(|| status.to_string()) } else { response };
        return Err(LlmError::Api(NAME.into(), 0, msg));
    }
    // the model tried a tool (a file, a command, the web) and was refused: no answer of its own
    if let Some(denied) = v.get("denied_actions").and_then(|d| d.as_array()).filter(|d| !d.is_empty()) {
        let what: Vec<&str> = denied.iter().filter_map(|a| a.get("action").and_then(|x| x.as_str())).collect();
        return Err(LlmError::Api(NAME.into(), 0, format!("模型想使用工具（{}），已拒絕；Google 模型在這裡只能回答文字", what.join("、"))));
    }
    let usage = v.get("usage").map(|u| Usage {
        input_tokens: u.get("input_tokens").and_then(|n| n.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0),
    });
    if let Some(s) = v.get("structured_output").filter(|s| s.is_object() || s.is_array()) {
        return Ok((s.to_string(), usage));
    }
    if response.trim().is_empty() {
        return Err(LlmError::Parse(NAME.into(), "沒有文字內容".into()));
    }
    Ok((response, usage))
}

// ── the model list ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
}

/// `<id>\t<label>` lines; anything else (the "Fetching…" line) is skipped.
pub fn parse_models(stdout: &str) -> Vec<ModelInfo> {
    let mut out: Vec<ModelInfo> = Vec::new();
    for line in stdout.lines() {
        let Some((id, label)) = line.split_once('\t') else { continue };
        let (id, label) = (id.trim(), label.trim());
        if id.is_empty() || id.contains(char::is_whitespace) || out.iter().any(|m| m.id == id) {
            continue;
        }
        out.push(ModelInfo { id: id.into(), label: if label.is_empty() { id.into() } else { label.into() } });
    }
    out
}

/// Runs `agy models` (replaced by a fake in tests).
#[async_trait]
pub trait ModelLister: Send + Sync {
    /// Is the CLI installed.
    fn installed(&self) -> bool;
    async fn list(&self) -> Result<Vec<ModelInfo>, String>;
}

/// The real CLI.
#[derive(Debug, Clone, Default)]
pub struct CliLister {
    /// Explicit binary; `None` = [`binary`].
    pub bin: Option<PathBuf>,
}

impl CliLister {
    fn bin(&self) -> Option<PathBuf> {
        self.bin.clone().filter(|p| p.is_file()).or_else(|| if self.bin.is_none() { binary() } else { None })
    }
}

#[async_trait]
impl ModelLister for CliLister {
    fn installed(&self) -> bool {
        self.bin().is_some()
    }

    async fn list(&self) -> Result<Vec<ModelInfo>, String> {
        let bin = self.bin().ok_or("找不到 Antigravity CLI（agy）")?;
        let mut c = command(&bin);
        c.arg("models").stdout(Stdio::piped()).stderr(Stdio::piped());
        let out = tokio::time::timeout(Duration::from_secs(60), c.output())
            .await
            .map_err(|_| "agy models 超過 60 秒沒有回應".to_string())?
            .map_err(|e| format!("無法啟動 agy：{e}"))?;
        let models = parse_models(&String::from_utf8_lossy(&out.stdout));
        if !out.status.success() || models.is_empty() {
            let e = String::from_utf8_lossy(&out.stderr);
            return Err(if e.trim().is_empty() { "agy 沒有列出模型，請確認已登入".into() } else { e.trim().chars().take(200).collect() });
        }
        Ok(models)
    }
}

/// The last listing, kept in the cache folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    /// Unix seconds of the listing.
    pub checked: u64,
    pub models: Vec<ModelInfo>,
    /// Ids that were not in the list before this one.
    #[serde(default)]
    pub new: Vec<String>,
}

pub const FILE: &str = "agy-models.json";

/// `<cache>/LoreForge/agy-models.json` (`LOREFORGE_CACHE` moves it).
pub fn default_path() -> Option<PathBuf> {
    lore_home::ports::cache_dir().map(|d| d.join(FILE))
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Stored {
    pub fn load(path: &Path) -> Option<Stored> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().ok_or_else(|| std::io::Error::other("no cache folder"))?;
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    /// The listing after `fresh` came back: `new` = ids absent from `previous` (nothing is
    /// new when there was no previous list).
    pub fn after(previous: Option<&Stored>, fresh: Vec<ModelInfo>, at: u64) -> Stored {
        let new = match previous {
            Some(p) => fresh.iter().filter(|m| !p.models.iter().any(|o| o.id == m.id)).map(|m| m.id.clone()).collect(),
            None => vec![],
        };
        Stored { checked: at, models: fresh, new }
    }

    /// Older than `max_age` seconds at `at`.
    pub fn stale(&self, at: u64, max_age: u64) -> bool {
        at.saturating_sub(self.checked) >= max_age
    }
}

/// A stand-in `agy` for offline tests (Unix shells only). `models` prints `models.txt` (or fails
/// without it); `-p` records its arguments in `last-args` and the working folder's entry count in
/// `cwd-entries`, then prints `reply.json` (default: an `OK` reply).
#[cfg(unix)]
pub mod fake {
    use std::path::{Path, PathBuf};

    pub fn write(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let d = dir.display();
        let script = format!(
            r#"#!/bin/sh
D="{d}"
if [ "$1" = "models" ]; then
  [ -f "$D/models.txt" ] || {{ echo "not signed in" >&2; exit 1; }}
  echo "Fetching available models..."; cat "$D/models.txt"; exit 0
fi
printf '%s\n' "$@" > "$D/last-args"
ls -A | wc -l | tr -d ' ' > "$D/cwd-entries"
env | cut -d= -f1 | sort > "$D/last-env-names"
if [ -f "$D/reply.json" ]; then cat "$D/reply.json"; else
  printf '{{"conversation_id":"x","status":"SUCCESS","response":"OK\\n","num_turns":1,"usage":{{"input_tokens":3,"output_tokens":1}}}}'
fi
"#
        );
        let path = dir.join("agy");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_reply_that_tried_a_tool_is_an_error() {
        let out = br#"{"status":"SUCCESS","response":"","denied_actions":[{"action":"read_file","display_name":"ReadFile"}]}"#;
        let e = parse_output(out, b"", true).unwrap_err().to_string();
        assert!(e.contains("read_file") && e.contains("已拒絕"), "{e}");
        let ok = br#"{"status":"SUCCESS","response":"fine","denied_actions":[]}"#;
        assert_eq!(parse_output(ok, b"", true).unwrap().0, "fine");
    }

    #[test]
    fn args_pass_the_prompt_as_one_argument_and_the_schema() {
        let mut p = Agy::new("some-model");
        p.effort = Some("low".into());
        let req = LlmRequest::prompt("sys", "text with $(rm -rf x) and \"quotes\"").json(json!({"type":"object"}));
        let a = p.args(&req);
        assert_eq!(a[0], "-p=sys\n\ntext with $(rm -rf x) and \"quotes\"");
        let has = |k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(has("--output-format", "json") && has("--model", "some-model") && has("--effort", "low"));
        assert!(a.contains(&"--disable-slash-commands".to_string()));
        // regression guard: without plan mode the CLI reads files and runs commands unasked
        assert!(has("--mode", "plan"), "every call denies the CLI's tools");
        assert!(!a.iter().any(|x| x == "--dangerously-skip-permissions" || x == "--add-dir" || x.starts_with("--add-dir=")));
        assert!(a.contains(&r#"--json-schema={"type":"object"}"#.to_string()));
        assert!(!p.args(&LlmRequest::prompt("", "u")).iter().any(|x| x.starts_with("--json-schema")));
    }

    #[test]
    fn capabilities_say_no_to_images_web_and_streaming() {
        let c = Agy::new("m").capabilities();
        assert!(c.json && !c.vision && !c.web && !c.streaming && c.needs_network);
    }

    #[test]
    fn reads_the_observed_output_shape_and_errors() {
        let ok = br#"{"conversation_id":"c","status":"SUCCESS","response":"OK\n","duration_seconds":2.6,"num_turns":1,"usage":{"input_tokens":9406,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":9407}}"#;
        let (t, u) = parse_output(ok, b"", true).unwrap();
        assert_eq!(t, "OK\n");
        assert_eq!(u, Some(Usage { input_tokens: 9406, output_tokens: 1 }));
        assert!(parse_output(br#"{"status":"ERROR","response":"quota"}"#, b"", true).unwrap_err().to_string().contains("quota"));
        assert!(matches!(parse_output(b"", b"please sign in first", false), Err(LlmError::NotConfigured(_))));
        assert!(matches!(parse_output(b"garbage", b"boom", false), Err(LlmError::Parse(..))));
        assert_eq!(parse_output(br#"{"status":"SUCCESS","response":"","structured_output":{"a":1}}"#, b"", true).unwrap().0, r#"{"a":1}"#);
    }

    #[test]
    fn parses_the_model_list() {
        let m = parse_models("Fetching available models...\nalpha-1\tAlpha One\nbeta-2\tBeta (High)\nalpha-1\tdup\n\n");
        assert_eq!(m, vec![ModelInfo { id: "alpha-1".into(), label: "Alpha One".into() }, ModelInfo { id: "beta-2".into(), label: "Beta (High)".into() }]);
    }

    #[test]
    fn new_ids_are_those_missing_from_the_previous_list() {
        let m = |id: &str| ModelInfo { id: id.into(), label: id.into() };
        let first = Stored::after(None, vec![m("a")], 10);
        assert!(first.new.is_empty(), "the first list has no previous one");
        let second = Stored::after(Some(&first), vec![m("a"), m("b")], 20);
        assert_eq!(second.new, ["b"]);
        assert!(second.stale(20 + 86_400, 86_400) && !second.stale(21, 86_400));
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join(FILE);
        second.save(&p).unwrap();
        assert_eq!(Stored::load(&p), Some(second));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_cli_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake::write(dir.path());
        let lister = CliLister { bin: Some(bin.clone()) };
        assert!(lister.installed());
        assert!(lister.list().await.is_err(), "not signed in: no list");
        std::fs::write(dir.path().join("models.txt"), "m-1\tModel One\nm-2\tModel Two\n").unwrap();
        assert_eq!(lister.list().await.unwrap().len(), 2);

        let mut p = Agy::new("m-1");
        p.bin = Some(bin);
        let r = p.send(&LlmRequest::prompt("sys", "the passage")).await.unwrap();
        assert_eq!((r.text.as_str(), r.provider.as_str()), ("OK\n", "agy:m-1"));
        assert_eq!(r.usage, Some(Usage { input_tokens: 3, output_tokens: 1 }));
        assert_eq!(std::fs::read_to_string(dir.path().join("cwd-entries")).unwrap().trim(), "0", "runs in an empty folder");
        let names = std::fs::read_to_string(dir.path().join("last-env-names")).unwrap();
        for n in names.lines() {
            assert!(ALLOW_ENV.contains(&n) || ["PWD", "SHLVL", "_", "OLDPWD"].contains(&n), "{n} reached agy");
        }
    }

    #[tokio::test]
    async fn missing_cli_and_unsupported_requests_are_refused_before_spawning() {
        let mut p = Agy::new("m");
        p.bin = Some("/nonexistent/agy".into());
        assert!(matches!(p.send(&LlmRequest::prompt("s", "u")).await, Err(LlmError::NotConfigured(_))));
        assert!(matches!(p.send(&LlmRequest::prompt("s", "u").web(true)).await, Err(LlmError::Unsupported(..))));
    }
}
