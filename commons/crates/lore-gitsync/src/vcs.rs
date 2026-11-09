//! Pure helpers: commit messages, the secret check, error classification,
//! remote URLs and deploy-key paths. No git process runs here.

use crate::ErrorKind;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A readable commit message from the changed paths, e.g.
/// `Update lessons (2026-11-02-ai-chips); vocab (my-words-ja)`.
///
/// Paths are grouped by their top folder. A folder in `group_roots` holds
/// named collections and groups by the collection instead
/// (`papers/example-paper/sections/method.tex` with root `papers` →
/// `example-paper (method)`). Five or more items are shortened to three and
/// a count.
pub fn commit_message(paths: &[String], group_roots: &[&str]) -> String {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in paths {
        let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
        let stem = |s: &str| s.rsplit_once('.').map(|(a, _)| a).unwrap_or(s).to_string();
        let (group, item) = match parts.as_slice() {
            [root, name, .., file] if group_roots.contains(root) => (name.to_string(), stem(file)),
            [top, .., file] => (top.to_string(), stem(file)),
            [file] => ("workspace".into(), stem(file)),
            [] => continue,
        };
        let items = groups.entry(group).or_default();
        if !item.is_empty() && !items.contains(&item) {
            items.push(item);
        }
    }
    if groups.is_empty() {
        return "Update workspace".into();
    }
    let body = groups
        .into_iter()
        .map(|(g, items)| {
            if items.is_empty() {
                g
            } else if items.len() > 4 {
                format!("{g} ({}, +{})", items[..3].join(", "), items.len() - 3)
            } else {
                format!("{g} ({})", items.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("Update {body}")
}

/// A label when a line looks like it contains a credential.
pub fn secret_like(line: &str) -> Option<&'static str> {
    let run_after = |marker: &str, min: usize| {
        line.match_indices(marker).any(|(i, _)| {
            line[i + marker.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').count() >= min
        })
    };
    if run_after("sk-ant-", 20) {
        return Some("Anthropic API key");
    }
    if run_after("ghp_", 30) || run_after("github_pat_", 20) || run_after("gho_", 30) {
        return Some("GitHub token");
    }
    if run_after("AKIA", 16) && line.match_indices("AKIA").any(|(i, _)| line[i..].chars().take(20).all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())) {
        return Some("AWS access key");
    }
    if line.contains("PRIVATE KEY-----") {
        return Some("private key");
    }
    None
}

/// The first added line in a `git diff -U0` that looks like a secret:
/// `(file, kind)`.
pub fn scan_diff(diff: &str) -> Option<(String, &'static str)> {
    let mut file = String::new();
    for line in diff.lines() {
        if let Some(f) = line.strip_prefix("+++ b/") {
            file = f.to_string();
        } else if line.starts_with('+')
            && !line.starts_with("+++")
            && let Some(kind) = secret_like(line)
        {
            return Some((file, kind));
        }
    }
    None
}

/// A remote URL with any user/token removed (safe to show and log).
pub fn strip_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host_end = rest.find('/').unwrap_or(rest.len());
            match rest[..host_end].rfind('@') {
                Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
                None => url.to_string(),
            }
        }
        None => url.to_string(),
    }
}

/// What kind of trouble a git error message is.
pub fn classify(err: &str) -> ErrorKind {
    let e = err.to_lowercase();
    if ["could not resolve host", "network is unreachable", "timed out", "connection refused", "operation timed out", "no route to host", "connection reset", "temporary failure in name resolution"]
        .iter()
        .any(|x| e.contains(x))
    {
        ErrorKind::Offline
    } else if ["permission denied", "repository not found", "authentication failed", "could not read from remote", "access denied", "key is already in use", "invalid username or password", "error: 403"]
        .iter()
        .any(|x| e.contains(x))
    {
        ErrorKind::Auth
    } else if e.contains("user.email") || e.contains("author identity") {
        ErrorKind::Identity
    } else if e.contains("conflict") {
        ErrorKind::Conflict
    } else {
        ErrorKind::Other
    }
}

/// A GitHub repo URL in the SSH form deploy keys need, plus `(owner, repo)`.
/// Other URLs (another host, a local path in tests) pass through unchanged.
pub fn ssh_url(url: &str) -> (String, Option<(String, String)>) {
    let u = url.trim();
    let rest = u.strip_prefix("https://github.com/").or_else(|| u.strip_prefix("git@github.com:")).or_else(|| u.strip_prefix("ssh://git@github.com/"));
    if let Some(rest) = rest {
        let mut it = rest.trim_end_matches('/').trim_end_matches(".git").splitn(3, '/');
        if let (Some(o), Some(r)) = (it.next(), it.next())
            && !o.is_empty()
            && !r.is_empty()
        {
            return (format!("git@github.com:{o}/{r}.git"), Some((o.to_string(), r.to_string())));
        }
    }
    (u.to_string(), None)
}

/// Where this computer's deploy key for `owner/repo` lives (in the app's
/// settings folder, never in the workspace).
pub fn deploy_key_path(app_dir: &Path, owner: &str, repo: &str) -> PathBuf {
    let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect::<String>();
    app_dir.join("deploy").join(format!("{}__{}", safe(owner), safe(repo))).join("id_ed25519")
}

/// The ssh command that uses only this key. Forward slashes work on Windows too.
pub fn ssh_command(key: &Path) -> String {
    let k = key.display().to_string().replace('\\', "/");
    format!("ssh -i \"{k}\" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o BatchMode=yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_paths_like_both_apps() {
        // JELearner: top folders.
        let m = commit_message(
            &["lessons/ja/2026-11-02-ai.md".into(), "vocab/my-words-ja.csv".into(), "vocab/my-words-ja.csv".into(), "workspace.toml".into()],
            &[],
        );
        assert_eq!(m, "Update lessons (2026-11-02-ai); vocab (my-words-ja); workspace (workspace)");
        // PaperComposer: papers/<paper>/… groups by paper.
        let m = commit_message(
            &[
                "papers/example-paper/sections/result.tex".into(),
                "papers/example-paper/sections/method.tex".into(),
                "profile/phrases.csv".into(),
            ],
            &["papers"],
        );
        assert_eq!(m, "Update example-paper (result, method); profile (phrases)");
        let many: Vec<String> = (1..=6).map(|i| format!("notes/n{i}.md")).collect();
        assert_eq!(commit_message(&many, &[]), "Update notes (n1, n2, n3, +3)");
        assert_eq!(commit_message(&[], &[]), "Update workspace");
    }

    #[test]
    fn detects_keys_but_not_prose() {
        assert!(secret_like("key = sk-ant-api03-abcdefghijklmnopqrstuvwxyz").is_some());
        assert!(secret_like("ghp_0123456789abcdefghijklmnopqrstuvwxyz").is_some());
        assert!(secret_like("-----BEGIN OPENSSH PRIVATE KEY-----").is_some());
        assert!(secret_like("aws = AKIAABCDEFGHIJKLMNOP").is_some());
        assert!(secret_like("We ask the model (sk-ant-) nothing.").is_none());
        assert!(secret_like("AKIA is a prefix").is_none());
        let diff = "diff --git a/a.txt b/a.txt\n+++ b/a.txt\n+hello\n+++ b/conf/k.toml\n+key = \"sk-ant-api03-abcdefghijklmnopqrstuvwxyz\"\n";
        assert_eq!(scan_diff(diff), Some(("conf/k.toml".into(), "Anthropic API key")));
        assert_eq!(scan_diff("+++ b/a\n-sk-ant-api03-abcdefghijklmnopqrstuvwxyz\n"), None, "removed lines are fine");
    }

    #[test]
    fn strips_tokens_from_remote_urls() {
        assert_eq!(strip_credentials("https://user:tok@github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_credentials("https://github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_credentials("https://github.com/a/b@c.git"), "https://github.com/a/b@c.git");
        assert_eq!(strip_credentials("git@github.com:a/b.git"), "git@github.com:a/b.git");
    }

    #[test]
    fn errors_are_sorted() {
        assert_eq!(classify("ssh: Could not resolve host: github.com"), ErrorKind::Offline);
        assert_eq!(classify("git@github.com: Permission denied (publickey)."), ErrorKind::Auth);
        assert_eq!(classify("ERROR: Repository not found."), ErrorKind::Auth);
        assert_eq!(classify("Author identity unknown"), ErrorKind::Identity);
        assert_eq!(classify("something else"), ErrorKind::Other);
    }

    #[test]
    fn deploy_keys_need_the_ssh_url() {
        assert_eq!(ssh_url("https://github.com/ellrin/JELearner-private").0, "git@github.com:ellrin/JELearner-private.git");
        assert_eq!(ssh_url("https://github.com/ellrin/JELearner-private.git/").1, Some(("ellrin".into(), "JELearner-private".into())));
        assert_eq!(ssh_url("git@github.com:ellrin/x.git").0, "git@github.com:ellrin/x.git");
        assert_eq!(ssh_url("/tmp/bare.git"), ("/tmp/bare.git".into(), None));
        let p = deploy_key_path(Path::new("/app"), "ellrin", "a/b");
        assert_eq!(p, Path::new("/app/deploy/ellrin__a_b/id_ed25519"));
        assert!(ssh_command(&p).contains("IdentitiesOnly=yes"));
    }
}
