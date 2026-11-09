//! lore-gitsync — Git sync for app workspaces through the system `git`
//! (merged from PaperComposer and JELearner `git.rs` / `vcs.rs`, and
//! JELearner's family-mode sync). A plain library: apps add their own routes.
//!
//! Using the installed git means the user's own sign-in (Keychain, Git
//! Credential Manager, SSH keys, or a per-clone deploy key through
//! `core.sshCommand`) handles authentication; this crate never stores a Git
//! password.
//!
//! Rules:
//! - pull is fast-forward only; when both sides have new commits, [`Repo::sync`]
//!   replays local commits on top (`pull --rebase --autostash`) and, on a
//!   conflict, aborts and reports instead of merging blindly;
//! - never force-push (the runner refuses any forced push);
//! - before every commit the staged diff is scanned for keys and tokens
//!   ([`vcs::secret_like`]); a hit unstages and stops;
//! - [`SyncOptions::only`] commits only some path prefixes (family mode:
//!   `profiles/<member>/`, `knowledge/progress/<member>/`): shared files edited
//!   on that computer are never committed from there;
//! - failures carry an [`ErrorKind`] (`offline`, `auth`, `conflict`, …) so a
//!   UI can stay quiet when offline and speak up when it matters; a local
//!   commit made before a failed push is reported in [`GitError::committed`]
//!   (the commit stays and goes up next time).

pub mod vcs;

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// No network / remote unreachable: retry later, quietly.
    Offline,
    /// Credentials or deploy key rejected, repository not found.
    Auth,
    /// Both sides changed the same lines; local work kept, rebase aborted.
    Conflict,
    /// `pull` found commits on both sides (sync would rebase).
    Diverged,
    /// The staged changes look like they contain a key; nothing committed.
    Secret,
    NotRepo,
    /// git is not installed.
    NoGit,
    /// No `user.name` / `user.email`.
    Identity,
    /// Target folder already exists (clone).
    Exists,
    Other,
}

/// A failed git step, worded for the user, with git's own words in `detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct GitError {
    pub kind: ErrorKind,
    pub message: String,
    /// git's stderr (for "給幫忙的人看").
    pub detail: String,
    /// A local commit made before the failure (it stays and is pushed next time).
    pub committed: Option<String>,
    /// Conflicting files, or the file that looked like it holds a secret.
    pub files: Vec<String>,
}

impl GitError {
    fn new(kind: ErrorKind, message: impl Into<String>, detail: impl Into<String>) -> GitError {
        GitError { kind, message: message.into(), detail: detail.into(), committed: None, files: vec![] }
    }

    /// From git's stderr, classified, with a short message for the step.
    fn from_git(step: &str, stderr: String) -> GitError {
        let kind = vcs::classify(&stderr);
        let message = match kind {
            ErrorKind::Offline => format!("{step}：無法連線到遠端"),
            ErrorKind::Auth => format!("{step}：遠端拒絕存取（金鑰或權限）"),
            ErrorKind::Identity => "Git 尚未設定作者，請執行 git config --global user.name 與 user.email".into(),
            _ => format!("{step}：{}", stderr.lines().last().unwrap_or("")),
        };
        GitError::new(kind, message, stderr)
    }

    fn with_commit(mut self, c: &Option<String>) -> GitError {
        self.committed = c.clone();
        self
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// The folder is a git work tree.
    pub repo: bool,
    pub branch: Option<String>,
    /// Remote `origin` URL with any credentials stripped.
    pub remote: Option<String>,
    pub upstream: bool,
    pub ahead: u32,
    pub behind: u32,
    /// Files with uncommitted changes.
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncResult {
    pub committed: Option<String>,
    pub pushed: bool,
    pub pulled: bool,
    pub status: GitStatus,
}

/// Commit author for this sync only (family members' computers often have
/// no Git identity at all).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    /// Commit message; `None` = generated from the changed paths.
    pub message: Option<String>,
    /// Commit only these path prefixes (relative to the repo); empty = everything.
    pub only: Vec<String>,
    pub author: Option<Author>,
    /// Folders that hold named collections, for the generated message
    /// (PaperComposer: `papers`).
    pub group_roots: Vec<String>,
}

/// A workspace repository.
#[derive(Debug, Clone)]
pub struct Repo {
    pub dir: PathBuf,
    /// `GIT_SSH_COMMAND` for this repo's calls (a deploy key before the clone
    /// remembers it in `core.sshCommand`).
    pub ssh_command: Option<String>,
    /// Extra `-c key=value` for every call.
    pub config: Vec<(String, String)>,
    /// Network steps (fetch, push, pull) may take this long.
    pub timeout: Duration,
}

impl Repo {
    pub fn new(dir: impl Into<PathBuf>) -> Repo {
        Repo { dir: dir.into(), ssh_command: None, config: vec![], timeout: Duration::from_secs(120) }
    }

    pub fn with_ssh_command(mut self, cmd: impl Into<String>) -> Repo {
        self.ssh_command = Some(cmd.into());
        self
    }

    pub fn with_config(mut self, key: &str, value: &str) -> Repo {
        self.config.push((key.into(), value.into()));
        self
    }

    /// Run git without prompts. Refuses any forced push.
    pub async fn git(&self, args: &[&str]) -> Result<String, GitError> {
        run(&self.dir, args, &self.config, self.ssh_command.as_deref(), self.timeout).await
    }

    async fn git_as(&self, args: &[&str], author: Option<&Author>) -> Result<String, GitError> {
        let mut config = self.config.clone();
        if let Some(a) = author {
            config.push(("user.name".into(), a.name.clone()));
            config.push(("user.email".into(), a.email.clone()));
        }
        run(&self.dir, args, &config, self.ssh_command.as_deref(), self.timeout).await
    }

    pub async fn status(&self) -> GitStatus {
        if self.git(&["rev-parse", "--is-inside-work-tree"]).await.as_deref() != Ok("true") {
            return GitStatus::default();
        }
        let branch = self.git(&["rev-parse", "--abbrev-ref", "HEAD"]).await.ok();
        let remote = self.git(&["remote", "get-url", "origin"]).await.ok().map(|u| vcs::strip_credentials(&u));
        let upstream = self.git(&["rev-parse", "--abbrev-ref", "@{u}"]).await.is_ok();
        let (mut ahead, mut behind) = (0, 0);
        if upstream && let Ok(s) = self.git(&["rev-list", "--left-right", "--count", "HEAD...@{u}"]).await {
            let mut it = s.split_whitespace().filter_map(|n| n.parse().ok());
            ahead = it.next().unwrap_or(0);
            behind = it.next().unwrap_or(0);
        }
        let changed = self
            .git(&["status", "--porcelain=v1", "--untracked-files=all"])
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.get(3..))
            .map(|p| p.rsplit(" -> ").next().unwrap_or(p).trim_matches('"').to_string())
            .collect();
        GitStatus { repo: true, branch, remote, upstream, ahead, behind, changed }
    }

    /// On open: bring in remote work when it is a fast-forward. `Ok(true)` =
    /// something came in. Both sides ahead → [`ErrorKind::Diverged`] (sync
    /// replays local work on top).
    pub async fn pull(&self) -> Result<bool, GitError> {
        let before = self.status().await;
        if !before.repo || before.remote.is_none() {
            return Ok(false);
        }
        self.git(&["fetch", "--quiet", "origin"]).await.map_err(|e| GitError::from_git("更新", e.detail))?;
        let st = self.status().await;
        if st.behind == 0 {
            return Ok(false);
        }
        if st.ahead > 0 {
            return Err(GitError::new(ErrorKind::Diverged, "本機與遠端都有新的 commit，請按同步讓兩邊合併", ""));
        }
        self.git(&["merge", "--ff-only", "--autostash", "--quiet", "@{u}"]).await.map_err(|e| GitError::from_git("無法更新", e.detail))?;
        Ok(true)
    }

    /// Paths in `only` that exist or are tracked (a pathspec that matches
    /// nothing would make git fail).
    async fn live_paths(&self, only: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for p in only {
            let p = p.trim_matches('/');
            if p.is_empty() {
                continue;
            }
            let tracked = self.git(&["ls-files", "--", p]).await.is_ok_and(|s| !s.trim().is_empty());
            if self.dir.join(p).exists() || tracked {
                out.push(p.to_string());
            }
        }
        out
    }

    /// On save: commit (everything, or only `opts.only`), bring in remote
    /// work (rebasing local commits on top), push.
    pub async fn sync(&self, opts: &SyncOptions) -> Result<SyncResult, GitError> {
        let st = self.status().await;
        if !st.repo {
            return Err(GitError::new(ErrorKind::NotRepo, "工作區不是 Git repository", ""));
        }
        let author = opts.author.as_ref();
        let scoped = !opts.only.is_empty();
        let paths = if scoped { self.live_paths(&opts.only).await } else { vec![] };
        let with_paths = |base: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = base.iter().map(|s| s.to_string()).collect();
            if scoped {
                v.push("--".into());
                v.extend(paths.iter().cloned());
            }
            v
        };

        // 1. Commit.
        let mut committed = None;
        let has_work = if scoped { !paths.is_empty() } else { !st.changed.is_empty() };
        if has_work {
            self.git(&strs(&with_paths(&["add", "-A"]))).await?;
            let staged: Vec<String> = self.git(&strs(&with_paths(&["diff", "--cached", "--name-only"]))).await?.lines().map(str::to_string).collect();
            if !staged.is_empty() {
                // Last line of defence: never commit something that looks like a key.
                let diff = self.git(&strs(&with_paths(&["diff", "--cached", "-U0", "--no-color"]))).await?;
                if let Some((file, kind)) = vcs::scan_diff(&diff) {
                    let _ = self.git(&strs(&with_paths(&["reset", "-q"]))).await;
                    let mut e = GitError::new(ErrorKind::Secret, format!("{file} 看起來含有 {kind}，已取消 commit"), "");
                    e.files = vec![file];
                    return Err(e);
                }
                let roots: Vec<&str> = opts.group_roots.iter().map(String::as_str).collect();
                let msg = opts.message.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| vcs::commit_message(&staged, &roots));
                let mut commit = vec!["commit".to_string(), "-q".into(), "-m".into(), msg.clone()];
                if scoped {
                    commit.push("--".into());
                    commit.extend(paths.iter().cloned());
                }
                self.git_as(&strs(&commit), author).await.map_err(|e| GitError::from_git("commit 失敗", e.detail))?;
                committed = Some(msg);
            }
        }

        // 2. Remote work, with local commits replayed on top.
        if st.remote.is_none() {
            return Ok(SyncResult { committed, pushed: false, pulled: false, status: self.status().await });
        }
        self.git(&["fetch", "--quiet", "origin"]).await.map_err(|e| GitError::from_git("同步", e.detail).with_commit(&committed))?;
        let upstream = self.git(&["rev-parse", "--abbrev-ref", "@{u}"]).await.is_ok();
        let mut pulled = false;
        if upstream && self.count("HEAD..@{u}").await > 0 {
            self.rebase(author, &committed).await?;
            pulled = true;
        }

        // 3. Up.
        let ahead = if upstream { self.count("@{u}..HEAD").await } else { u32::from(self.git(&["rev-parse", "--verify", "-q", "HEAD"]).await.is_ok()) };
        let mut pushed = false;
        if ahead > 0 {
            let push: &[&str] = if upstream { &["push", "-q"] } else { &["push", "-q", "-u", "origin", "HEAD"] };
            match self.git(push).await {
                Ok(_) => pushed = true,
                Err(e) if ["rejected", "fetch first", "non-fast-forward"].iter().any(|m| e.detail.contains(m)) => {
                    // Another device pushed in between: replay once more, push again.
                    self.rebase(author, &committed).await?;
                    pulled = true;
                    self.git(push).await.map_err(|e| GitError::from_git("推送失敗", e.detail).with_commit(&committed))?;
                    pushed = true;
                }
                Err(e) => return Err(GitError::from_git("推送失敗", e.detail).with_commit(&committed)),
            }
        }
        Ok(SyncResult { committed, pushed, pulled, status: self.status().await })
    }

    async fn count(&self, range: &str) -> u32 {
        self.git(&["rev-list", "--count", range]).await.ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
    }

    /// `pull --rebase --autostash`; on failure abort and report the files.
    async fn rebase(&self, author: Option<&Author>, committed: &Option<String>) -> Result<(), GitError> {
        if let Err(e) = self.git_as(&["pull", "--rebase", "--autostash", "-q"], author).await {
            let files: Vec<String> = self.git(&["diff", "--name-only", "--diff-filter=U"]).await.unwrap_or_default().lines().map(str::to_string).collect();
            let _ = self.git(&["rebase", "--abort"]).await;
            if matches!(vcs::classify(&e.detail), ErrorKind::Offline | ErrorKind::Auth) {
                return Err(GitError::from_git("同步", e.detail).with_commit(committed));
            }
            let message = if files.is_empty() {
                "與遠端的修改衝突，已還原到同步前的狀態".to_string()
            } else {
                format!("{} 在兩台裝置都被修改，已還原到同步前的狀態，你的修改仍保留在本機", files.join("、"))
            };
            let mut err = GitError::new(ErrorKind::Conflict, message, e.detail).with_commit(committed);
            err.files = files;
            return Err(err);
        }
        Ok(())
    }

    /// `git init` a new workspace, make the first commit, and push it to the
    /// user's empty private repo when a remote URL is given.
    pub async fn init_and_push(&self, remote: Option<&str>, first_message: &str, author: Option<&Author>) -> Result<String, GitError> {
        if self.status().await.repo {
            return Ok("已是 Git repository".into());
        }
        self.git(&["init", "--quiet", "-b", "main"]).await?;
        self.git(&["add", "-A"]).await?;
        let diff = self.git(&["diff", "--cached", "-U0", "--no-color"]).await?;
        if let Some((file, kind)) = vcs::scan_diff(&diff) {
            let _ = self.git(&["reset", "-q"]).await;
            let mut e = GitError::new(ErrorKind::Secret, format!("{file} 看起來含有 {kind}，已取消 commit"), "");
            e.files = vec![file];
            return Err(e);
        }
        self.git_as(&["commit", "-q", "-m", first_message], author).await.map_err(|e| GitError::from_git("commit 失敗", e.detail))?;
        let Some(url) = remote else { return Ok("已建立本機 Git repository".into()) };
        self.git(&["remote", "add", "origin", url]).await?;
        self.git(&["push", "-q", "-u", "origin", "main"]).await.map_err(|e| GitError::from_git("推送失敗", e.detail))?;
        Ok(format!("已推送到 {}", vcs::strip_credentials(url)))
    }
}

fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

async fn run(dir: &Path, args: &[&str], config: &[(String, String)], ssh: Option<&str>, timeout: Duration) -> Result<String, GitError> {
    if args.first() == Some(&"push") && args.iter().any(|a| *a == "-f" || a.starts_with("--force") || a.starts_with('+') || *a == "--mirror") {
        return Err(GitError::new(ErrorKind::Other, "不允許 force push", args.join(" ")));
    }
    let mut c = Command::new("git");
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // no console window from a GUI app
    for (k, v) in config {
        c.arg("-c").arg(format!("{k}={v}"));
    }
    c.args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0") // never hang waiting for a password
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(s) = ssh {
        c.env("GIT_SSH_COMMAND", s);
    }
    let out = tokio::time::timeout(timeout, c.output())
        .await
        .map_err(|_| GitError::new(ErrorKind::Offline, format!("git {} 逾時", args.first().unwrap_or(&"")), "timed out"))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::new(ErrorKind::NoGit, "找不到 git，請先安裝 Git", e.to_string())
            } else {
                GitError::new(ErrorKind::Other, e.to_string(), e.to_string())
            }
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let kind = vcs::classify(&stderr);
        Err(GitError::new(kind, format!("git {}：{}", args.first().unwrap_or(&""), stderr.lines().last().unwrap_or("")), stderr))
    }
}

/// Clone a repo into `target` (must not exist yet). `ssh_command` = a deploy
/// key for this clone; it is remembered in the clone's `core.sshCommand`.
pub async fn clone(url: &str, target: &Path, ssh_command: Option<&str>) -> Result<(), GitError> {
    if target.exists() {
        return Err(GitError::new(ErrorKind::Exists, format!("{} 已存在", target.display()), ""));
    }
    let parent = target.parent().ok_or_else(|| GitError::new(ErrorKind::Other, "位置無效", ""))?;
    std::fs::create_dir_all(parent).map_err(|e| GitError::new(ErrorKind::Other, e.to_string(), ""))?;
    let t = target.to_string_lossy().to_string();
    run(parent, &["clone", "--quiet", url, &t], &[], ssh_command, Duration::from_secs(600))
        .await
        .map_err(|e| GitError::from_git("clone 失敗", e.detail))?;
    if let Some(s) = ssh_command {
        run(target, &["config", "core.sshCommand", s], &[], None, Duration::from_secs(10)).await?;
    }
    Ok(())
}

/// Make an ed25519 deploy key at `path` once (no passphrase, the comment
/// names the computer); later calls return the same public key.
/// **Not tested against real GitHub** here.
pub async fn ensure_deploy_key(path: &Path, comment: &str) -> Result<String, String> {
    let public = path.with_extension("pub");
    if !path.exists() {
        // A private key must never be able to end up in a commit.
        let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(path) };
        if let Some(repo) = abs.ancestors().skip(1).find(|a| a.join(".git").exists()) {
            return Err(format!("部署金鑰不能放在 git repository 裡（{}）；請把 App 的設定資料夾放在 repository 外面。", repo.display()));
        }
        std::fs::create_dir_all(path.parent().ok_or("位置無效")?).map_err(|e| e.to_string())?;
        let out = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-q", "-C", comment, "-f"])
            .arg(path)
            .output()
            .await
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "這台電腦沒有 ssh-keygen（Windows：設定 → 選用功能 → OpenSSH 用戶端）".to_string() } else { e.to_string() })?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    }
    std::fs::read_to_string(&public).map(|s| s.trim().to_string()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
