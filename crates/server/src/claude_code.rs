//! Claude subscription engine: runs the Claude Code CLI (`claude -p`) the
//! user installed and logged into with their claude.ai account, so usage
//! counts against the subscription instead of API billing.
//!
//! Each call runs in an empty temporary folder with every tool turned off and
//! all customisations skipped (`--safe-mode`, or its equivalent on older
//! CLIs; see `isolation_flags`), so it only answers the prompt
//! and never reads or writes the user's files. ANTHROPIC_API_KEY is removed
//! from the child's environment: with it set, Claude Code would bill the API.
//! `--bare` is not used because bare mode ignores the subscription login.

use pc_core::compose::Prompt;
use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Variables that would make Claude Code bill an API key or another provider
/// instead of the subscription.
const STRIP_ENV: &[&str] = &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"];

/// The CLI binary. GUI apps on macOS do not get the shell's PATH, so the
/// installers' default locations are checked as well.
pub fn binary() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

fn command(bin: &PathBuf) -> Command {
    let mut c = crate::proc::cmd(bin);
    for v in STRIP_ENV {
        c.env_remove(v);
    }
    c.stdin(Stdio::null()).kill_on_drop(true);
    c
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    pub installed: bool,
    pub version: Option<String>,
    pub logged_in: bool,
    /// From `claude auth status`: "claude.ai" means the subscription.
    pub auth_method: Option<String>,
    pub account: Option<String>,
    /// Installed, logged in, and billing the subscription.
    pub ready: bool,
}

pub async fn status() -> Status {
    let Some(bin) = binary() else { return Status::default() };
    let version = command(&bin)
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_end_matches("(Claude Code)").trim().to_string());
    let auth = command(&bin).args(["auth", "status"]).output().await.ok();
    let json: serde_json::Value = auth
        .as_ref()
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or_default();
    let logged_in = auth.is_some_and(|o| o.status.success());
    let auth_method = json.get("authMethod").and_then(|v| v.as_str()).map(String::from);
    let account = ["email", "emailAddress", "account", "orgName"]
        .iter()
        .find_map(|k| json.get(k).and_then(|v| v.as_str()))
        .map(String::from);
    let ready = logged_in && matches!(auth_method.as_deref(), Some("claude.ai") | Some("oauth_token"));
    Status { installed: true, version, logged_in, auth_method, account, ready }
}

/// `claude auth status` spawns a process; the UI polls engine status, so
/// keep the answer for a little while.
#[derive(Default)]
pub struct Cache(tokio::sync::Mutex<Option<(Instant, Status)>>);

impl Cache {
    pub async fn get(&self, fresh: bool) -> Status {
        let mut c = self.0.lock().await;
        if let (false, Some((at, s))) = (fresh, c.as_ref()) {
            if at.elapsed() < Duration::from_secs(60) {
                return s.clone();
            }
        }
        let s = status().await;
        *c = Some((Instant::now(), s.clone()));
        s
    }
}

/// Start `claude auth login` (claude.ai account, not Console). It opens the
/// browser; the UI then polls status until the login lands.
pub fn start_login() -> Result<(), String> {
    let bin = binary().ok_or("找不到 Claude Code，請先安裝")?;
    let mut c = command(&bin);
    c.args(["auth", "login"]).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(false);
    let mut child = c.spawn().map_err(|e| format!("無法啟動 Claude Code 登入：{e}"))?;
    // Reap it in the background; give up after ten minutes.
    tokio::spawn(async move {
        if tokio::time::timeout(Duration::from_secs(600), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
    });
    Ok(())
}

pub async fn complete(prompt: &Prompt, model: &str, extra: Option<&str>) -> Result<String, String> {
    let bin = binary().ok_or("找不到 Claude Code，請先安裝")?;
    let dir = std::env::temp_dir().join(format!("papercomposer-claude-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut user = prompt.user.clone();
    if let Some(e) = extra {
        user += "\n";
        user += e;
    }
    let isolation = isolation_flags(&bin).await;
    let mut c = command(&bin);
    c.current_dir(&dir)
        .args(["-p", "--output-format", "json", "--no-session-persistence", "--tools", ""])
        .args(isolation)
        .args(["--model", model, "--system-prompt", &prompt.system])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("無法啟動 Claude Code：{e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(user.as_bytes()).await.map_err(|e| e.to_string())?;
    }
    let out = tokio::time::timeout(Duration::from_secs(300), child.wait_with_output())
        .await
        .map_err(|_| "Claude Code 超過 5 分鐘沒有回應".to_string())?
        .map_err(|e| e.to_string())?;
    parse_output(&out.stdout, &out.stderr)
}

/// Flags that skip the user's customisations. `--safe-mode` does it in one
/// flag but only newer CLIs have it; older ones get the same effect from no
/// setting sources (so no hooks or settings), no MCP servers and no slash
/// commands. Read once from `claude --help`.
async fn isolation_flags(bin: &PathBuf) -> &'static [&'static str] {
    static FLAGS: tokio::sync::OnceCell<&'static [&'static str]> = tokio::sync::OnceCell::const_new();
    *FLAGS
        .get_or_init(|| async {
            let help = command(bin).arg("--help").output().await.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
            pick_isolation(&help)
        })
        .await
}

fn pick_isolation(help: &str) -> &'static [&'static str] {
    if help.contains("--safe-mode") {
        &["--safe-mode"]
    } else if help.contains("--disable-slash-commands") {
        &["--strict-mcp-config", "--setting-sources", "", "--disable-slash-commands"]
    } else {
        &["--strict-mcp-config", "--setting-sources", ""]
    }
}

/// The `--output-format json` result: text in `result`, failures flagged by
/// `is_error` (and then described in `result`).
fn parse_output(stdout: &[u8], stderr: &[u8]) -> Result<String, String> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        let err = String::from_utf8_lossy(stderr);
        let err = err.trim();
        return Err(if err.is_empty() { "Claude Code 沒有回傳結果".into() } else { format!("Claude Code：{err}") });
    };
    let text = v.get("result").and_then(|r| r.as_str()).unwrap_or_default().to_string();
    if v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false) {
        return Err(format!("Claude Code：{}", if text.is_empty() { "執行失敗" } else { &text }));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{parse_output, pick_isolation};

    #[test]
    fn isolation_follows_the_cli_version() {
        assert_eq!(pick_isolation("  --safe-mode   Skip customisations\n  --disable-slash-commands"), &["--safe-mode"]);
        assert_eq!(pick_isolation("  --disable-slash-commands\n  --strict-mcp-config"), &["--strict-mcp-config", "--setting-sources", "", "--disable-slash-commands"]);
        assert!(!pick_isolation("").contains(&"--safe-mode"));
    }

    #[test]
    fn reads_result_and_errors() {
        assert_eq!(parse_output(br#"{"type":"result","is_error":false,"result":"{\"a\":1}"}"#, b"").unwrap(), r#"{"a":1}"#);
        let e = parse_output(br#"{"is_error":true,"result":"Not logged in"}"#, b"").unwrap_err();
        assert!(e.contains("Not logged in"));
        let e = parse_output(b"", b"error: unknown option").unwrap_err();
        assert!(e.contains("unknown option"));
    }
}
