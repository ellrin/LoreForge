//! Claude subscription engine: runs the Claude Code CLI (`claude -p`) that the
//! user installed and signed into with their own Claude account, so usage
//! counts against their subscription instead of API billing. Merged from
//! PaperComposer and JELearner.
//!
//! - The official, unmodified `claude` binary does the work; sign-in happens
//!   in Anthropic's own browser flow (`claude auth login --claudeai`). This
//!   crate never reads, stores or forwards account credentials or tokens.
//! - Every call runs in an empty temporary folder with all tools turned off
//!   and all customisations skipped (`--safe-mode`, or the equivalent flags on
//!   older CLIs, read once from `claude --help`), so it only answers the
//!   prompt and never touches the user's files.
//! - The child starts from an empty environment plus an allowlist
//!   ([`ALLOW_ENV`]): nothing that could route billing to an API key, another
//!   provider or someone else's login ([`STRIP_ENV`]) gets through.
//! - `CLAUDE_CONFIG_DIR` per user (JELearner: each family member signs in to
//!   their own Claude). **Not tested on macOS with a real account.**
//! - Structured output uses `--json-schema`; the prompt goes on stdin.
//! - Web search (optional, [`ClaudeCode::allow_web`]): only the `WebSearch`
//!   tool is enabled for that call (`--tools WebSearch --allowedTools
//!   WebSearch`). **Not tested with the real CLI.**
//! - No images and no token streaming here (capabilities say so); the router
//!   sends vision tasks elsewhere.
//!
//! Tested with a fake `claude` executable only ([`fake`]).

use crate::error::LlmError;
use crate::provider::{LlmProvider, check_request};
use crate::types::{Capabilities, LlmRequest, LlmResponse};
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const NAME: &str = "Claude 訂閱";

/// The only environment variables Claude Code inherits: what a program needs
/// to find files, the network and the locale.
pub const ALLOW_ENV: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TERM", "TMPDIR", "TMP", "TEMP",
    "LANG", "LC_ALL", "LC_CTYPE", "LC_MESSAGES", "TZ",
    "HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "https_proxy", "http_proxy", "no_proxy",
    "SSL_CERT_FILE", "SSL_CERT_DIR", "NODE_EXTRA_CA_CERTS",
    "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR",
    // Windows
    "USERPROFILE", "APPDATA", "LOCALAPPDATA", "SYSTEMROOT", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT", "HOMEDRIVE", "HOMEPATH",
];

/// Variables that would route Claude Code to API billing, another provider or
/// someone else's login. Never passed (they are not in [`ALLOW_ENV`]).
pub const STRIP_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_OAUTH_TOKEN",
];

/// Official install command shown in settings.
pub const INSTALL_HINT: &str = "curl -fsSL https://claude.ai/install.sh | bash";
pub const SETUP_PAGE: &str = "https://code.claude.com/docs/en/setup";

/// Which Claude Code to run and with whose sign-in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Cli {
    /// Explicit binary (settings or an app env var); `None` = search.
    pub bin: Option<PathBuf>,
    /// Passed as `CLAUDE_CONFIG_DIR`: one sign-in per user.
    pub config_dir: Option<PathBuf>,
}

/// Find the CLI. GUI apps on macOS do not get the shell's PATH, so the
/// installers' default locations are checked too. `explicit` wins.
pub fn binary(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

fn command(bin: &Path, config_dir: Option<&Path>) -> Command {
    let mut c = Command::new(bin);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // no console window from a GUI app
    c.env_clear();
    for k in ALLOW_ENV {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    if let Some(d) = config_dir {
        let _ = std::fs::create_dir_all(d);
        c.env("CLAUDE_CONFIG_DIR", d);
    }
    c.stdin(Stdio::null()).kill_on_drop(true);
    c
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Status {
    pub installed: bool,
    pub version: Option<String>,
    pub logged_in: bool,
    /// From `claude auth status`: `claude.ai` / `oauth_token` = subscription.
    pub auth_method: Option<String>,
    pub account: Option<String>,
    /// Installed, signed in, and billing a subscription.
    pub ready: bool,
}

pub async fn status(cli: &Cli) -> Status {
    let Some(bin) = binary(cli.bin.as_deref()) else { return Status::default() };
    let run = |args: &'static [&'static str]| {
        let mut c = command(&bin, cli.config_dir.as_deref());
        c.args(args);
        async move { tokio::time::timeout(Duration::from_secs(20), c.output()).await.ok().and_then(|r| r.ok()) }
    };
    let version = run(&["--version"])
        .await
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_end_matches("(Claude Code)").trim().to_string());
    let auth = run(&["auth", "status", "--json"]).await;
    let json: serde_json::Value = auth.as_ref().and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or_default();
    let logged_in = auth.as_ref().is_some_and(|o| o.status.success()) && json.get("loggedIn").and_then(|v| v.as_bool()).unwrap_or(true);
    let auth_method = json.get("authMethod").and_then(|v| v.as_str()).map(String::from);
    let account = ["email", "emailAddress", "account", "orgName"]
        .iter()
        .find_map(|k| json.get(k).and_then(|v| v.as_str()))
        .map(String::from);
    let ready = logged_in && matches!(auth_method.as_deref(), Some("claude.ai") | Some("oauth_token"));
    Status { installed: true, version, logged_in, auth_method, account, ready }
}

/// `claude auth status` spawns a process and UIs poll; keep each sign-in
/// folder's answer for a minute unless a fresh one is asked for.
#[derive(Default)]
pub struct Cache(tokio::sync::Mutex<HashMap<Cli, (Instant, Status)>>);

impl Cache {
    pub async fn get(&self, cli: &Cli, fresh: bool) -> Status {
        let mut c = self.0.lock().await;
        if let (false, Some((at, s))) = (fresh, c.get(cli))
            && at.elapsed() < Duration::from_secs(60)
        {
            return s.clone();
        }
        let s = status(cli).await;
        c.insert(cli.clone(), (Instant::now(), s.clone()));
        s
    }
}

/// Start `claude auth login --claudeai`: Claude Code opens the browser and
/// sign-in completes on Anthropic's pages. The UI then polls [`status`].
pub fn start_login(cli: &Cli) -> Result<(), String> {
    let bin = binary(cli.bin.as_deref()).ok_or("找不到 Claude Code，請先安裝")?;
    let mut c = command(&bin, cli.config_dir.as_deref());
    c.args(["auth", "login", "--claudeai"]).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(false);
    let mut child = c.spawn().map_err(|e| format!("無法啟動 Claude Code 登入：{e}"))?;
    tokio::spawn(async move {
        if tokio::time::timeout(Duration::from_secs(600), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
    });
    Ok(())
}

/// Flags that skip the user's customisations. `--safe-mode` does it in one
/// flag on newer CLIs; older ones get the same from no setting sources, no
/// MCP servers and no slash commands.
pub fn pick_isolation(help: &str) -> &'static [&'static str] {
    if help.contains("--safe-mode") {
        &["--safe-mode"]
    } else if help.contains("--disable-slash-commands") {
        &["--strict-mcp-config", "--setting-sources", "", "--disable-slash-commands"]
    } else {
        &["--strict-mcp-config", "--setting-sources", ""]
    }
}

/// Read once per binary from `claude --help`.
async fn isolation_flags(bin: &Path) -> &'static [&'static str] {
    static SEEN: Mutex<Option<HashMap<PathBuf, &'static [&'static str]>>> = Mutex::new(None);
    if let Some(f) = SEEN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(bin) {
        return f;
    }
    let mut c = command(bin, None);
    c.arg("--help");
    let help = tokio::time::timeout(Duration::from_secs(20), c.output())
        .await
        .ok()
        .and_then(|r| r.ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let flags = pick_isolation(&help);
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(bin.to_path_buf(), flags);
    flags
}

pub struct ClaudeCode {
    pub cli: Cli,
    /// A model id or alias from the catalog.
    pub model: String,
    pub effort: Option<String>,
    pub timeout: Duration,
    /// Let `web_search` requests use Claude Code's WebSearch tool.
    pub allow_web: bool,
    /// Context window reported to the router (from the catalog).
    pub context: u32,
    /// Prefix of the empty temp folder each call runs in (app name).
    pub temp_prefix: String,
}

impl ClaudeCode {
    pub fn new(cli: Cli, model: impl Into<String>, effort: Option<String>) -> ClaudeCode {
        ClaudeCode {
            cli,
            model: model.into(),
            effort,
            timeout: Duration::from_secs(600),
            allow_web: false,
            context: 1_000_000,
            temp_prefix: "ym-llm-claude".into(),
        }
    }

    /// Command-line arguments for one call (the prompt goes on stdin).
    pub fn args(&self, req: &LlmRequest, isolation: &[&str]) -> Vec<String> {
        let mut a: Vec<String> = ["-p", "--output-format", "json", "--no-session-persistence"].iter().map(|s| s.to_string()).collect();
        a.extend(isolation.iter().map(|s| s.to_string()));
        if req.web_search && self.allow_web {
            a.extend(["--tools", "WebSearch", "--allowedTools", "WebSearch"].map(String::from));
        } else {
            a.extend(["--tools".into(), String::new()]);
        }
        a.extend(["--model".into(), self.model.clone(), "--system-prompt".into(), req.system.clone()]);
        if let Some(e) = req.effort.as_ref().or(self.effort.as_ref()) {
            a.extend(["--effort".into(), e.clone()]);
        }
        if let Some(schema) = &req.json_schema {
            a.extend(["--json-schema".into(), schema.to_string()]);
        }
        a
    }
}

fn unique() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

#[async_trait]
impl LlmProvider for ClaudeCode {
    fn id(&self) -> String {
        format!("claude-subscription:{}", self.model)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { json: true, vision: false, web: self.allow_web, streaming: false, context: self.context, needs_network: true }
    }

    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        check_request(NAME, &self.capabilities(), req)?;
        let bin = binary(self.cli.bin.as_deref()).ok_or_else(|| LlmError::NotConfigured("找不到 Claude Code，請先安裝並登入".into()))?;
        let isolation = isolation_flags(&bin).await;
        // An empty folder of its own: Claude Code has nothing to read there.
        let dir = std::env::temp_dir().join(format!("{}-{}-{}", self.temp_prefix, std::process::id(), unique()));
        std::fs::create_dir_all(&dir).map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
        let mut c = command(&bin, self.cli.config_dir.as_deref());
        c.current_dir(&dir).args(self.args(req, isolation)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let prompt = req.flatten();
        let result = async {
            let mut child = c.spawn().map_err(|e| LlmError::Network(NAME.into(), format!("無法啟動 Claude Code：{e}")))?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(prompt.as_bytes()).await.map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
            }
            let out = tokio::time::timeout(self.timeout, child.wait_with_output())
                .await
                .map_err(|_| LlmError::Network(NAME.into(), format!("超過 {} 秒沒有回應", self.timeout.as_secs())))?
                .map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
            parse_output(&out.stdout, &out.stderr)
        }
        .await;
        let _ = std::fs::remove_dir_all(&dir);
        Ok(LlmResponse { text: result?, provider: self.id(), sources: vec![], usage: None })
    }
}

/// The `--output-format json` result: `structured_output` when a schema was
/// given, otherwise the text in `result`; failures are flagged by `is_error`.
pub fn parse_output(stdout: &[u8], stderr: &[u8]) -> Result<String, LlmError> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        let err = String::from_utf8_lossy(stderr);
        let err = err.trim();
        return Err(LlmError::Parse(NAME.into(), if err.is_empty() { "Claude Code 沒有回傳結果".into() } else { err.to_string() }));
    };
    let text = v.get("result").and_then(|r| r.as_str()).unwrap_or_default().to_string();
    if v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false) {
        let msg = if text.is_empty() { v.get("subtype").and_then(|s| s.as_str()).unwrap_or("執行失敗").to_string() } else { text };
        return Err(LlmError::Api(NAME.into(), 0, msg));
    }
    if let Some(s) = v.get("structured_output").filter(|s| s.is_object() || s.is_array()) {
        return Ok(s.to_string());
    }
    if text.trim().is_empty() {
        return Err(LlmError::Parse(NAME.into(), "沒有文字內容".into()));
    }
    Ok(text)
}

/// A stand-in `claude` executable for offline tests (Unix shells only).
///
/// Answers `--version`, `--help` (lists `--safe-mode`), `auth status`,
/// `auth login` and `-p`. Sign-in is kept per `CLAUDE_CONFIG_DIR` (or the
/// fake's own folder): signed in when that folder has `fake-login`; the
/// account name comes from `fake-account`. Evidence is left in the fake's
/// folder: `last-args`, `last-stdin`, `cwd-entries`, `last-config-dir`,
/// `last-env-names`, `login-called`. Put a JSON reply in `reply.json` for
/// `--json-schema` calls and a text reply in `reply.txt` for the rest. If
/// any variable from [`STRIP_ENV`] reaches it, it fails loudly.
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
S="${{CLAUDE_CONFIG_DIR:-$D}}"
case "$1" in
  --version) echo "9.9.9 (Claude Code)"; exit 0 ;;
  --help) echo "  --safe-mode   Skip customisations"; exit 0 ;;
  auth)
    if [ "$2" = "login" ]; then mkdir -p "$S"; touch "$S/fake-login" "$D/login-called"; exit 0; fi
    if [ ! -f "$S/fake-login" ]; then echo '{{"loggedIn":false}}'; exit 1; fi
    A="ellrin@example.com"; [ -f "$S/fake-account" ] && A="$(cat "$S/fake-account")"
    echo "{{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\"email\":\"$A\"}}"; exit 0 ;;
esac
printf '%s\n' "$@" > "$D/last-args"
printf '%s' "$CLAUDE_CONFIG_DIR" > "$D/last-config-dir"
env | cut -d= -f1 | sort > "$D/last-env-names"
cat > "$D/last-stdin"
ls -A | wc -l | tr -d ' ' > "$D/cwd-entries"
if [ -n "$ANTHROPIC_API_KEY$ANTHROPIC_AUTH_TOKEN$ANTHROPIC_BASE_URL$CLAUDE_CODE_USE_BEDROCK$CLAUDE_CODE_OAUTH_TOKEN" ]; then
  echo '{{"type":"result","is_error":true,"result":"billing variable leaked"}}'; exit 1
fi
if [ ! -f "$S/fake-login" ]; then echo '{{"type":"result","is_error":true,"result":"Not logged in"}}'; exit 1; fi
case " $* " in
  *" --json-schema "*) printf '{{"type":"result","is_error":false,"result":"","structured_output":%s}}' "$(cat "$D/reply.json")" ;;
  *) R="OK"; [ -f "$D/reply.txt" ] && R="$(cat "$D/reply.txt")"; printf '{{"type":"result","is_error":false,"result":"%s"}}' "$R" ;;
esac
"#
        );
        let path = dir.join("claude");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Sign a sign-in folder in as `account` (as if the browser login ran).
    pub fn sign_in(state_dir: &Path, account: &str) {
        std::fs::create_dir_all(state_dir).unwrap();
        std::fs::write(state_dir.join("fake-login"), "").unwrap();
        std::fs::write(state_dir.join("fake-account"), account).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Image, Message};
    use serde_json::json;

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_cli_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake::write(dir.path());
        std::fs::write(dir.path().join("reply.json"), r#"{"ok":true}"#).unwrap();
        let me = Cli { bin: Some(bin.clone()), config_dir: Some(dir.path().join("ellrin")) };

        let st = status(&me).await;
        assert!(st.installed && !st.ready, "{st:?}");
        start_login(&me).unwrap();
        for _ in 0..100 {
            if dir.path().join("login-called").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let st = status(&me).await;
        assert!(st.ready, "login lands: {st:?}");
        assert_eq!(st.version.as_deref(), Some("9.9.9"));

        let p = ClaudeCode::new(me.clone(), "claude-sonnet-5-5", None);
        let req = LlmRequest::prompt("sys", "the article").json(json!({"type":"object"}));
        assert_eq!(p.complete(&req).await.unwrap(), r#"{"ok":true}"#);
        assert_eq!(std::fs::read_to_string(dir.path().join("last-stdin")).unwrap(), "the article");
        assert_eq!(std::fs::read_to_string(dir.path().join("cwd-entries")).unwrap().trim(), "0", "runs in an empty folder");
        let args = std::fs::read_to_string(dir.path().join("last-args")).unwrap();
        assert!(args.contains("--safe-mode"), "isolation read from --help: {args}");
        assert_eq!(std::fs::read_to_string(dir.path().join("last-config-dir")).unwrap(), dir.path().join("ellrin").display().to_string());

        // A transcript is written into the one prompt.
        let chat = LlmRequest::chat("s", vec![Message::user("hi"), Message::assistant("hello"), Message::user("bye")]);
        assert_eq!(p.complete(&chat).await.unwrap(), "OK");
        assert!(std::fs::read_to_string(dir.path().join("last-stdin")).unwrap().contains("<turn role=\"assistant\">\nhello"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn each_user_has_their_own_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake::write(dir.path());
        let a = Cli { bin: Some(bin.clone()), config_dir: Some(dir.path().join("ellrin")) };
        let b = Cli { bin: Some(bin.clone()), config_dir: Some(dir.path().join("daidai")) };
        fake::sign_in(&dir.path().join("ellrin"), "ellrin@example.com");
        let cache = Cache::default();
        assert_eq!(cache.get(&a, true).await.account.as_deref(), Some("ellrin@example.com"));
        assert!(!cache.get(&b, true).await.ready, "daidai is not signed in by ellrin's login");
        let p = ClaudeCode::new(b.clone(), "m", None);
        assert!(p.complete(&LlmRequest::prompt("s", "u")).await.is_err(), "daidai cannot use ellrin's sign-in");
        fake::sign_in(&dir.path().join("daidai"), "daidai@example.com");
        assert_eq!(cache.get(&b, true).await.account.as_deref(), Some("daidai@example.com"));
        assert_eq!(cache.get(&a, false).await.account.as_deref(), Some("ellrin@example.com"));
    }

    #[test]
    fn args_turn_everything_off_and_pass_schema() {
        let p = ClaudeCode::new(Cli::default(), "claude-opus-5-5", Some("medium".into()));
        let req = LlmRequest::prompt("sys", "u").json(json!({"type":"object"}));
        let a = p.args(&req, &["--safe-mode"]);
        let has = |k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(a.contains(&"--safe-mode".to_string()) && a.contains(&"--no-session-persistence".to_string()));
        assert!(has("--tools", ""));
        assert!(has("--model", "claude-opus-5-5"));
        assert!(has("--effort", "medium"));
        assert!(has("--json-schema", r#"{"type":"object"}"#));
        assert!(!a.contains(&"u".to_string()), "the prompt goes on stdin, not argv");
    }

    #[test]
    fn web_search_only_when_allowed() {
        let mut p = ClaudeCode::new(Cli::default(), "m", None);
        let req = LlmRequest::prompt("s", "u").web(true);
        let has = |a: &[String], k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(has(&p.args(&req, &[]), "--tools", ""), "web off by default");
        p.allow_web = true;
        let a = p.args(&req, &[]);
        assert!(has(&a, "--tools", "WebSearch") && has(&a, "--allowedTools", "WebSearch"));
        assert!(has(&p.args(&LlmRequest::prompt("s", "u"), &[]), "--tools", ""), "other calls keep all tools off");
    }

    #[tokio::test]
    async fn images_and_disallowed_web_are_refused_before_spawning() {
        let p = ClaudeCode::new(Cli { bin: Some("/nonexistent/claude".into()), config_dir: None }, "m", None);
        let img = LlmRequest::prompt("s", "u").image(Image::from_bytes("image/png", b"x"));
        assert!(matches!(p.send(&img).await, Err(LlmError::Unsupported(..))));
        assert!(matches!(p.send(&LlmRequest::prompt("s", "u").web(true)).await, Err(LlmError::Unsupported(..))));
        assert!(matches!(p.send(&LlmRequest::prompt("s", "u")).await, Err(LlmError::NotConfigured(_))));
    }

    #[test]
    fn isolation_follows_the_cli_version() {
        assert_eq!(pick_isolation("  --safe-mode   Skip\n  --disable-slash-commands"), &["--safe-mode"]);
        assert_eq!(pick_isolation("  --disable-slash-commands\n"), &["--strict-mcp-config", "--setting-sources", "", "--disable-slash-commands"]);
        assert!(!pick_isolation("").contains(&"--safe-mode"));
    }

    #[test]
    fn reads_structured_output_result_and_errors() {
        assert_eq!(parse_output(br#"{"type":"result","is_error":false,"result":"hi"}"#, b"").unwrap(), "hi");
        assert_eq!(parse_output(br#"{"is_error":false,"result":"","structured_output":{"a":1}}"#, b"").unwrap(), r#"{"a":1}"#);
        assert!(parse_output(br#"{"is_error":true,"result":"Not logged in"}"#, b"").unwrap_err().to_string().contains("Not logged in"));
        assert!(parse_output(b"", b"error: unknown option").unwrap_err().to_string().contains("unknown option"));
    }

    #[test]
    fn child_environment_is_only_the_allowlist() {
        let c = command(Path::new("claude"), Some(&std::env::temp_dir().join("ym-llm-test-claude")));
        for (k, v) in c.as_std().get_envs() {
            let k = k.to_string_lossy();
            assert!(v.is_some());
            assert!(ALLOW_ENV.contains(&k.as_ref()) || k == "CLAUDE_CONFIG_DIR", "{k} passed through");
        }
        for v in STRIP_ENV {
            assert!(!ALLOW_ENV.contains(v));
        }
    }

    /// Whatever CLAUDE_* / ANTHROPIC_* variables this machine has, the child
    /// sees none of them except its own sign-in folder.
    #[cfg(unix)]
    #[tokio::test]
    async fn nothing_from_this_process_leaks_to_claude_code() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake::write(dir.path());
        let cli = Cli { bin: Some(bin), config_dir: Some(dir.path().join("ellrin")) };
        fake::sign_in(&dir.path().join("ellrin"), "ellrin@example.com");
        ClaudeCode::new(cli, "m", None).complete(&LlmRequest::prompt("s", "u")).await.unwrap();
        let names = std::fs::read_to_string(dir.path().join("last-env-names")).unwrap();
        for n in names.lines() {
            let ok = ALLOW_ENV.contains(&n) || ["CLAUDE_CONFIG_DIR", "PWD", "SHLVL", "_", "OLDPWD"].contains(&n);
            assert!(ok, "{n} reached Claude Code");
        }
    }
}
