# ym-gitsync

Git sync for app workspaces through the system `git` (design §2.2, §3.2, §13.1). A plain
library — no axum; apps add their own routes. Merged from PaperComposer and JELearner
(`core/src/vcs.rs`, `server/src/git.rs`, JL `family.rs` sync).

```toml
ym-gitsync = { path = "../../commons/crates/ym-gitsync" }
```

## Rules

- `pull` is fast-forward only (`merge --ff-only --autostash`); both sides ahead →
  `ErrorKind::Diverged`.
- `sync` commits, fetches, replays local commits on top (`pull --rebase --autostash`) when
  the remote moved, pushes; a rebase conflict is aborted and reported
  (`ErrorKind::Conflict`, `files`), local work kept.
- Never force-push: the runner refuses `push -f/--force*/--mirror/+refspec`.
- Secret scan of the staged diff before every commit (`sk-ant-…`, GitHub tokens, AWS
  keys, private keys): a hit unstages and returns `ErrorKind::Secret` with the file.
- `SyncOptions::only` commits only those path prefixes (family mode
  `profiles/<member>/`, `knowledge/progress/<member>/`); other edits stay uncommitted.
- Errors carry `kind` (`offline`, `auth`, `conflict`, `diverged`, `secret`, `notrepo`,
  `nogit`, `identity`, `exists`, `other`), a user message (zh-TW), git's `detail`, and
  `committed` when a local commit was made before the failure (it goes up next time).
- No prompts ever (`GIT_TERMINAL_PROMPT=0`), `LC_ALL=C`, timeouts; no console windows on
  Windows.

## API

```rust
pub struct Repo { dir, ssh_command: Option<String>, config: Vec<(String, String)>, timeout }
Repo::new(dir).with_ssh_command(cmd).with_config("user.name", "…")
repo.status().await -> GitStatus { repo, branch, remote (credentials stripped), upstream, ahead, behind, changed }
repo.pull().await -> Result<bool, GitError>
repo.sync(&SyncOptions { message, only, author: Option<Author { name, email }>, group_roots }).await
    -> Result<SyncResult { committed, pushed, pulled, status }, GitError>
repo.init_and_push(remote: Option<&str>, first_message, author).await -> Result<String, GitError>
repo.git(&["…"]).await                                // raw call, same guards
clone(url, target, ssh_command: Option<&str>).await   // remembers core.sshCommand
ensure_deploy_key(path, comment).await -> Result<String /* public key */, String>

vcs::{commit_message(paths, group_roots), secret_like, scan_diff, strip_credentials,
      classify -> ErrorKind, ssh_url, ssh_command, deploy_key_path}
```

## Migration map

| App today | ym-gitsync |
|---|---|
| JL `jl_core::vcs::commit_message(paths)` | `vcs::commit_message(paths, &[])` |
| PC `pc_core::vcs::commit_message(paths)` (groups `papers/<paper>/…`) | `vcs::commit_message(paths, &["papers"])` |
| `vcs::secret_like` (both) | `vcs::secret_like` (+ `scan_diff`) |
| `git::GitStatus`, `git::SyncResult` (both) | same fields |
| `git::status(dir)` | `Repo::new(dir).status()` |
| `git::pull(dir) -> Result<bool, String>` | `Repo::new(dir).pull()` → `GitError` (`to_string()` gives the old message) |
| `git::sync(dir, message)` | `Repo::new(dir).sync(&SyncOptions { message, group_roots: vec!["papers".into()] /* PC */, ..Default::default() })` |
| JL `git::clone(url, target)` | `clone(url, target, None)` |
| JL `git::init_and_push(dir, remote)` | `Repo::new(dir).init_and_push(remote, "Create JELearner workspace", None)` |
| JL `family::sync_member(root, member) -> Result<Value, (reason, detail)>` | `Repo::new(root).sync(&SyncOptions { only: vec![format!("profiles/{member}")], author: Some(Author { name: member.into(), email: format!("{member}@jelearner.local") }), ..Default::default() })`; `(reason, detail)` = `(serde name of e.kind, e.detail)` — JL's `other` for conflicts becomes `conflict` |
| JL `family::{classify, ssh_url, ssh_command, key_path, ensure_key}` | `vcs::{classify, ssh_url, ssh_command, deploy_key_path}`, `ensure_deploy_key(path, &format!("JELearner {host}"))` (host name stays in the app) |
| JL `family::git(dir, args, ssh, secs)` | `Repo { ssh_command, timeout, .. }.git(args)` |
| JL `config user.name/email` written by `finish` | unchanged (app), or pass `author` per sync |

Behaviour differences to note in the refactor: `sync` now fetches and rebases **before**
pushing (JL/PC pushed first and rebased on rejection; same end state, one fewer failed
push), and still retries once if another device pushes in between.

## Tests

Real `git` against a local bare repository in a temp folder (no network): two devices,
fast-forward, divergence + rebase, conflict abort, secret scan, family-mode prefixes and
author, unreachable remote keeps the local commit, clone with deploy-key command,
`ssh-keygen` deploy key (skipped without OpenSSH). **Not tested against GitHub** (deploy
keys, HTTPS credential helpers).
