//! Sync tests with the real `git` against a local bare repository (no
//! network): two devices, divergence, conflicts, secrets, family mode.

use super::*;
use std::path::Path;

/// A repo with a test identity and no signing or hooks from the developer's
/// global git configuration.
fn repo(dir: &Path) -> Repo {
    Repo::new(dir)
        .with_config("user.name", "ellrin")
        .with_config("user.email", "ellrin@example.com")
        .with_config("commit.gpgsign", "false")
        .with_config("core.hooksPath", "/dev/null")
}

fn write(dir: &Path, rel: &str, s: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// A bare remote, device `a` that created the workspace, device `b` cloned from it.
async fn two_devices(root: &Path) -> (Repo, Repo, PathBuf) {
    let bare = root.join("remote.git");
    std::fs::create_dir_all(&bare).unwrap();
    Repo::new(&bare).git(&["init", "--quiet", "--bare", "-b", "main"]).await.unwrap();
    let a = root.join("a");
    write(&a, "workspace.toml", "name = \"test\"\n");
    write(&a, "notes/first.md", "hello\n");
    let ra = repo(&a);
    let msg = ra.init_and_push(Some(bare.to_str().unwrap()), "Create workspace", None).await.unwrap();
    assert!(msg.contains("remote.git"), "{msg}");
    let b = root.join("b");
    clone(bare.to_str().unwrap(), &b, None).await.unwrap();
    (ra, repo(&b), bare)
}

#[tokio::test]
async fn sync_commits_pushes_and_the_other_device_fast_forwards() {
    let t = tempfile::tempdir().unwrap();
    let (a, b, _) = two_devices(t.path()).await;
    let st = a.status().await;
    assert!(st.repo && st.upstream && st.changed.is_empty(), "{st:?}");
    assert_eq!(st.branch.as_deref(), Some("main"));

    write(&a.dir, "notes/second.md", "more\n");
    write(&a.dir, "vocab/my-words.csv", "word\n");
    let r = a.sync(&SyncOptions::default()).await.unwrap();
    assert_eq!(r.committed.as_deref(), Some("Update notes (second); vocab (my-words)"));
    assert!(r.pushed && !r.pulled);
    assert_eq!(r.status.ahead, 0);

    assert!(b.pull().await.unwrap(), "b fast-forwards");
    assert!(b.dir.join("notes/second.md").exists());
    assert!(!b.pull().await.unwrap(), "nothing new the second time");

    // Nothing to do: no commit, no push.
    let r = a.sync(&SyncOptions::default()).await.unwrap();
    assert!(r.committed.is_none() && !r.pushed);
}

#[tokio::test]
async fn diverged_devices_rebase_on_sync() {
    let t = tempfile::tempdir().unwrap();
    let (a, b, _) = two_devices(t.path()).await;
    write(&a.dir, "notes/from-a.md", "a\n");
    a.sync(&SyncOptions::default()).await.unwrap();
    write(&b.dir, "notes/from-b.md", "b\n");
    b.git(&["add", "-A"]).await.unwrap();
    b.git(&["commit", "-q", "-m", "local"]).await.unwrap();
    assert_eq!(b.pull().await.unwrap_err().kind, ErrorKind::Diverged, "pull never merges");

    let r = b.sync(&SyncOptions { message: Some("Mine".into()), ..Default::default() }).await.unwrap();
    assert!(r.pulled && r.pushed);
    assert!(r.committed.is_none(), "already committed by hand");
    assert!(b.dir.join("notes/from-a.md").exists() && b.dir.join("notes/from-b.md").exists());
    let log = b.git(&["log", "--format=%s"]).await.unwrap();
    assert_eq!(log.lines().take(2).collect::<Vec<_>>(), vec!["local", "Update notes (from-a)"], "linear history, ours on top");
    assert!(a.pull().await.unwrap());
}

#[tokio::test]
async fn conflicts_abort_and_keep_local_work() {
    let t = tempfile::tempdir().unwrap();
    let (a, b, _) = two_devices(t.path()).await;
    write(&a.dir, "notes/first.md", "from a\n");
    a.sync(&SyncOptions::default()).await.unwrap();
    write(&b.dir, "notes/first.md", "from b\n");
    let e = b.sync(&SyncOptions::default()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict, "{e:?}");
    assert_eq!(e.files, vec!["notes/first.md".to_string()]);
    assert!(e.committed.is_some(), "the local commit is reported");
    assert!(!b.dir.join(".git/rebase-merge").exists() && !b.dir.join(".git/rebase-apply").exists(), "rebase aborted");
    assert_eq!(std::fs::read_to_string(b.dir.join("notes/first.md")).unwrap(), "from b\n", "local work kept");
    // The remote still has a's version: nothing was forced.
    assert!(b.git(&["push", "--force"]).await.is_err());
    assert!(b.git(&["push", "origin", "+main"]).await.is_err());
    assert!(a.pull().await.is_ok_and(|p| !p));
}

#[tokio::test]
async fn secrets_are_never_committed() {
    let t = tempfile::tempdir().unwrap();
    let (a, _, _) = two_devices(t.path()).await;
    write(&a.dir, "notes/ok.md", "fine\n");
    write(&a.dir, "config/keys.toml", "anthropic = \"sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123\"\n");
    let e = a.sync(&SyncOptions::default()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Secret);
    assert_eq!(e.files, vec!["config/keys.toml".to_string()]);
    assert!(e.message.contains("Anthropic API key"));
    assert_eq!(a.git(&["diff", "--cached", "--name-only"]).await.unwrap(), "", "unstaged again");
    assert_eq!(a.git(&["log", "--format=%s"]).await.unwrap().lines().count(), 1, "no new commit");
}

#[tokio::test]
async fn family_mode_commits_only_the_members_folder() {
    let t = tempfile::tempdir().unwrap();
    let (admin, member, _) = two_devices(t.path()).await;
    let daidai = Author { name: "daidai".into(), email: "daidai@example.com".into() };
    let only = SyncOptions { only: vec!["profiles/daidai/".into(), "knowledge/progress/daidai".into()], author: Some(daidai), ..Default::default() };

    // Nothing of daidai's yet: nothing to commit, shared edits stay local.
    write(&member.dir, "workspace.toml", "name = \"changed on daidai's computer\"\n");
    let r = member.sync(&only).await.unwrap();
    assert!(r.committed.is_none() && !r.pushed);

    write(&member.dir, "profiles/daidai/progress.csv", "word,known\n");
    write(&member.dir, "profiles/ellrin/progress.csv", "not mine\n");
    // Meanwhile the admin changed something else.
    write(&admin.dir, "topics.toml", "x = 1\n");
    admin.sync(&SyncOptions::default()).await.unwrap();

    let r = member.sync(&only).await.unwrap();
    assert_eq!(r.committed.as_deref(), Some("Update profiles (progress)"));
    assert!(r.pulled && r.pushed);
    let show = member.git(&["show", "--name-only", "--format=%an", "HEAD"]).await.unwrap();
    let lines: Vec<&str> = show.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines, vec!["daidai", "profiles/daidai/progress.csv"], "only daidai's folder, authored by daidai");
    let st = member.status().await;
    assert!(st.changed.contains(&"workspace.toml".to_string()) && st.changed.contains(&"profiles/ellrin/progress.csv".to_string()), "shared edits kept, uncommitted: {st:?}");
    assert!(member.dir.join("topics.toml").exists(), "admin's work came in");

    assert!(admin.pull().await.unwrap());
    assert!(admin.dir.join("profiles/daidai/progress.csv").exists());
    assert!(!admin.dir.join("profiles/ellrin/progress.csv").exists());
}

#[tokio::test]
async fn offline_remote_keeps_the_local_commit() {
    let t = tempfile::tempdir().unwrap();
    let (a, _, bare) = two_devices(t.path()).await;
    std::fs::rename(&bare, t.path().join("gone.git")).unwrap();
    write(&a.dir, "notes/offline.md", "x\n");
    let e = a.sync(&SyncOptions::default()).await.unwrap_err();
    assert_eq!(e.committed.as_deref(), Some("Update notes (offline)"), "{e:?}");
    assert!(a.status().await.changed.is_empty(), "committed locally");
    std::fs::rename(t.path().join("gone.git"), &bare).unwrap();
    let r = a.sync(&SyncOptions::default()).await.unwrap();
    assert!(r.pushed && r.committed.is_none(), "goes up next time");
}

#[tokio::test]
async fn not_a_repo_and_clone_guards() {
    let t = tempfile::tempdir().unwrap();
    let r = repo(t.path());
    assert!(!r.status().await.repo);
    assert_eq!(r.sync(&SyncOptions::default()).await.unwrap_err().kind, ErrorKind::NotRepo);
    assert!(!r.pull().await.unwrap());
    assert_eq!(clone("/nowhere.git", t.path(), None).await.unwrap_err().kind, ErrorKind::Exists);
    let e = clone(t.path().join("missing.git").to_str().unwrap(), &t.path().join("c"), None).await.unwrap_err();
    assert_ne!(e.kind, ErrorKind::Exists);
}

#[tokio::test]
async fn clone_remembers_the_deploy_key_command() {
    let t = tempfile::tempdir().unwrap();
    let (_, _, bare) = two_devices(t.path()).await;
    let key = vcs::deploy_key_path(t.path(), "ellrin", "ws");
    let cmd = vcs::ssh_command(&key);
    let c = t.path().join("c");
    clone(bare.to_str().unwrap(), &c, Some(&cmd)).await.unwrap();
    assert_eq!(repo(&c).git(&["config", "core.sshCommand"]).await.unwrap(), cmd);
}

#[tokio::test]
async fn deploy_key_is_made_once() {
    if std::process::Command::new("ssh-keygen").arg("-?").output().is_err() {
        return; // no OpenSSH on this machine
    }
    let t = tempfile::tempdir().unwrap();
    let key = vcs::deploy_key_path(t.path(), "ellrin", "ws");
    let first = ensure_deploy_key(&key, "ym-gitsync test").await.unwrap();
    assert!(first.starts_with("ssh-ed25519 ") && first.ends_with("ym-gitsync test"));
    assert_eq!(ensure_deploy_key(&key, "other").await.unwrap(), first);
}

#[tokio::test]
async fn deploy_keys_are_never_made_inside_a_git_repo() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join(".git")).unwrap();
    let key = vcs::deploy_key_path(&t.path().join("app"), "ellrin", "ws");
    let err = crate::ensure_deploy_key(&key, "test").await.unwrap_err();
    assert!(err.contains("git repository"), "{err}");
    assert!(!key.exists());
}
