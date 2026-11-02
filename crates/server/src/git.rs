//! Git sync for the private workspace, through the system `git`.
//!
//! Using the installed git means the user's own sign-in (Git Credential
//! Manager on Windows, Keychain on macOS, SSH keys) handles authentication;
//! the app never stores a Git password.
//!
//! Rules: pull fast-forward only; never force-push; when histories diverge
//! and a rebase conflicts, abort and report instead of merging blindly.

use pc_core::vcs::{commit_message, secret_like};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Serialize, Default)]
pub struct GitStatus {
    /// The workspace is a git repository.
    pub repo: bool,
    pub branch: Option<String>,
    /// Remote URL with any credentials stripped.
    pub remote: Option<String>,
    pub upstream: bool,
    pub ahead: u32,
    pub behind: u32,
    /// Files with uncommitted changes.
    pub changed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncResult {
    pub committed: Option<String>,
    pub pushed: bool,
    pub pulled: bool,
    pub status: GitStatus,
}

async fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let run = crate::proc::cmd("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0") // never hang waiting for a password
        .env("LC_ALL", "C")
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .map_err(|_| format!("git {} timed out", args.first().unwrap_or(&"")))?
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "找不到 git，請先安裝 Git".to_string() } else { e.to_string() })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn strip_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_, host)) if !rest[..rest.find('/').unwrap_or(rest.len())].is_empty() && rest.find('@') < rest.find('/') => {
                format!("{scheme}://{host}")
            }
            _ => url.to_string(),
        },
        None => url.to_string(),
    }
}

pub async fn status(dir: &Path) -> GitStatus {
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).await.as_deref() != Ok("true") {
        return GitStatus::default();
    }
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await.ok();
    let remote = git(dir, &["remote", "get-url", "origin"]).await.ok().map(|u| strip_credentials(&u));
    let upstream = git(dir, &["rev-parse", "--abbrev-ref", "@{u}"]).await.is_ok();
    let (mut ahead, mut behind) = (0, 0);
    if upstream {
        if let Ok(s) = git(dir, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]).await {
            let mut it = s.split_whitespace().filter_map(|n| n.parse().ok());
            ahead = it.next().unwrap_or(0);
            behind = it.next().unwrap_or(0);
        }
    }
    let changed = git(dir, &["status", "--porcelain=v1", "--untracked-files=all"])
        .await
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|p| p.trim_matches('"').to_string())
        .collect();
    GitStatus { repo: true, branch, remote, upstream, ahead, behind, changed }
}

/// On open: bring in remote work if it is a fast-forward.
pub async fn pull(dir: &Path) -> Result<bool, String> {
    let before = status(dir).await;
    if !before.repo || before.remote.is_none() {
        return Ok(false);
    }
    git(dir, &["fetch", "--quiet", "origin"]).await.map_err(|e| format!("無法連線到遠端：{e}"))?;
    let st = status(dir).await;
    if st.behind == 0 {
        return Ok(false);
    }
    if st.ahead > 0 {
        return Err("本機與遠端都有新的 commit，請按同步讓兩邊合併".into());
    }
    git(dir, &["merge", "--ff-only", "--autostash", "--quiet", "@{u}"])
        .await
        .map_err(|e| format!("無法更新：{e}"))?;
    Ok(true)
}

/// On save: commit everything, then push; rebase onto remote work if needed.
pub async fn sync(dir: &Path, message: Option<String>) -> Result<SyncResult, String> {
    let st = status(dir).await;
    if !st.repo {
        return Err("工作區不是 Git repository".into());
    }
    let mut committed = None;
    if !st.changed.is_empty() {
        git(dir, &["add", "-A"]).await?;
        // Last line of defence: never commit something that looks like a key.
        let diff = git(dir, &["diff", "--cached", "-U0", "--no-color"]).await?;
        let mut file = String::new();
        for line in diff.lines() {
            if let Some(f) = line.strip_prefix("+++ b/") {
                file = f.to_string();
            } else if line.starts_with('+') && !line.starts_with("+++") {
                if let Some(kind) = secret_like(line) {
                    let _ = git(dir, &["reset", "-q"]).await;
                    return Err(format!("{file} 看起來含有 {kind}，已取消 commit"));
                }
            }
        }
        let names: Vec<String> = git(dir, &["diff", "--cached", "--name-only"]).await?.lines().map(str::to_string).collect();
        let msg = message.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| commit_message(&names));
        git(dir, &["commit", "-q", "-m", &msg]).await.map_err(|e| {
            if e.contains("user.email") || e.contains("Author identity") {
                "Git 尚未設定作者，請執行 git config --global user.name 與 user.email".to_string()
            } else {
                e
            }
        })?;
        committed = Some(msg);
    }

    let mut pulled = false;
    let mut pushed = false;
    if st.remote.is_some() {
        let push = if st.upstream { git(dir, &["push", "-q"]).await } else { git(dir, &["push", "-q", "-u", "origin", "HEAD"]).await };
        match push {
            Ok(_) => pushed = true,
            Err(e) if e.contains("rejected") || e.contains("fetch first") || e.contains("non-fast-forward") => {
                // Someone (another device) pushed first: replay ours on top.
                if git(dir, &["pull", "--rebase", "--autostash", "-q"]).await.is_err() {
                    let files = git(dir, &["diff", "--name-only", "--diff-filter=U"]).await.unwrap_or_default();
                    let _ = git(dir, &["rebase", "--abort"]).await;
                    let files = files.lines().collect::<Vec<_>>().join("、");
                    return Err(if files.is_empty() {
                        "與遠端的修改衝突，已還原到同步前的狀態".into()
                    } else {
                        format!("{files} 在兩台裝置都被修改，已還原到同步前的狀態，你的修改仍保留在本機")
                    });
                }
                pulled = true;
                git(dir, &["push", "-q"]).await.map_err(|e| format!("推送失敗：{e}"))?;
                pushed = true;
            }
            Err(e) => return Err(format!("推送失敗：{e}")),
        }
    }
    Ok(SyncResult { committed, pushed, pulled, status: status(dir).await })
}

#[cfg(test)]
mod tests {
    use super::strip_credentials;

    #[test]
    fn strips_tokens_from_remote_urls() {
        assert_eq!(strip_credentials("https://user:tok@github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_credentials("https://github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_credentials("git@github.com:a/b.git"), "git@github.com:a/b.git");
    }
}
