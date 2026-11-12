//! The catalog directory and its local-only guard.
//!
//! The catalog holds file names and paths, so it must never reach a remote:
//! [`Guard::check`] refuses a location inside iCloud Drive (or another synced
//! cloud folder), inside the LoreForge repository, or inside any git work tree
//! that has a remote. [`Catalog::open`] runs the guard, then on first use
//! `git init`s the folder (local history for later tag edits), writes the
//! [`LOCAL_ONLY_MARKER`] file and a `pre-push` hook that always fails (with a
//! repo-local `core.hooksPath`, so a global hooks path cannot bypass it).

use crate::CatalogError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Marker file at the root of a repository that must never be pushed or
/// synced. `lore-gitsync` refuses network steps in a repo that has it
/// (same constant: `lore_gitsync::LOCAL_ONLY_MARKER`).
pub const LOCAL_ONLY_MARKER: &str = ".lore-local-only";

const MARKER_TEXT: &str = "This folder is a local-only LoreShelf catalog (file names and paths).\n\
It must never get a git remote, be pushed, or be synced to any cloud.\n";

/// The local git versions only the author's edits (`edits/`); scan output
/// (`files/`, `projects/`, `packages/`, `datasets/`, `reports/`, anything
/// later) is ignored and never committed.
pub const GITIGNORE: &str = "# LoreShelf: only the author's edits are versioned; scan output is rebuilt by every scan.\n\
/*\n\
!/.gitignore\n\
!/.lore-local-only\n\
!/edits/\n";

const PRE_PUSH: &str = "#!/bin/sh\n\
echo \"LoreShelf: this catalog is local-only (.lore-local-only); pushing is not allowed.\" >&2\n\
exit 1\n";

/// Default catalog directory: `$LORESHELF_HOME`, else
/// `<data dir>/LoreForge/LoreShelf` (`~/Library/Application Support` on macOS,
/// `$XDG_DATA_HOME` or `~/.local/share` on Linux, `%APPDATA%` on Windows).
/// Only the CLI / app calls this; library functions take explicit paths.
pub fn default_catalog_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LORESHELF_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    dirs::data_dir().map(|d| d.join("LoreForge").join("LoreShelf"))
}

/// Default hash-cache directory: `<cache>/LoreForge/shelf/` (cache tier,
/// wipeable; `$LOREFORGE_CACHE` overrides the base like every app).
pub fn default_cache_dir() -> Option<PathBuf> {
    lore_home::ports::cache_dir().map(|d| d.join("shelf"))
}

/// Where a catalog may not live.
#[derive(Debug, Clone)]
pub struct Guard {
    /// Synced cloud folders (iCloud Drive's `~/Library/Mobile Documents`,
    /// `~/Library/CloudStorage`).
    pub cloud: Vec<PathBuf>,
    /// Folders that are public or synced by design (the LoreForge repo).
    pub forbidden: Vec<PathBuf>,
}

impl Default for Guard {
    fn default() -> Guard {
        let mut cloud = Vec::new();
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            let home = PathBuf::from(home);
            cloud.push(home.join("Library/Mobile Documents"));
            cloud.push(home.join("Library/CloudStorage"));
        }
        // This crate lives at <LoreForge>/commons/crates/lore-catalog.
        let mut forbidden = Vec::new();
        if let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3) {
            forbidden.push(repo.to_path_buf());
        }
        Guard { cloud, forbidden }
    }
}

/// `p` made absolute and resolved as far as it exists (symlinks such as
/// `/tmp` → `/private/tmp` resolved), the missing tail appended.
pub fn resolve(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf()) };
    let mut tail = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return out;
        }
        match (cur.file_name(), cur.parent()) {
            (Some(n), Some(parent)) => {
                tail.push(n.to_os_string());
                cur = parent;
            }
            _ => return abs,
        }
    }
}

fn under(p: &Path, base: &Path) -> bool {
    p.starts_with(base) || p.starts_with(resolve(base))
}

impl Guard {
    /// Refuse a catalog location that could leak paths to a remote or cloud.
    /// Does not create anything.
    pub fn check(&self, dir: &Path) -> Result<(), CatalogError> {
        let dir = resolve(dir);
        if let Some(c) = self.cloud.iter().find(|c| under(&dir, c)) {
            return Err(CatalogError::Guard(format!("目錄不能放在雲端同步資料夾裡（{}）：{}", c.display(), dir.display())));
        }
        if dir.components().any(|c| c.as_os_str() == "Mobile Documents") {
            return Err(CatalogError::Guard(format!("目錄不能放在 iCloud Drive 裡：{}", dir.display())));
        }
        if let Some(f) = self.forbidden.iter().find(|f| under(&dir, f)) {
            return Err(CatalogError::Guard(format!("目錄不能放在 LoreForge repository 裡（{}）", f.display())));
        }
        for a in dir.ancestors() {
            if a.join(".git").exists() {
                let remotes = git_remotes(a);
                if !remotes.is_empty() {
                    return Err(CatalogError::Guard(format!(
                        "目錄位於有遠端（{}）的 git 工作目錄 {} 裡；目錄只能存在本機",
                        remotes.join(", "),
                        a.display()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Remote names of the repository at `dir` (`git remote`; falls back to
/// reading `.git/config` when git cannot run). Any error that leaves the
/// answer unknown counts as a remote (refuse rather than guess).
pub fn git_remotes(dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["remote"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect(),
        _ => {
            let cfg = dir.join(".git").join("config");
            match std::fs::read_to_string(&cfg) {
                Ok(s) => s
                    .lines()
                    .filter_map(|l| l.trim().strip_prefix("[remote \"").and_then(|r| r.strip_suffix("\"]")))
                    .map(str::to_string)
                    .collect(),
                Err(_) if dir.join(".git").is_dir() => vec![],
                Err(_) => vec!["(unknown)".into()],
            }
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, CatalogError> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| CatalogError::Guard(format!("找不到 git（{e}）；目錄需要 git 保存本機修改紀錄")))?;
    if !o.status.success() {
        return Err(CatalogError::Guard(format!("git {}：{}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// An opened (guarded, initialised) catalog directory.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub dir: PathBuf,
}

impl Catalog {
    /// Guard, then create / initialise the catalog: `git init`, local
    /// `core.hooksPath`, refusing `pre-push` hook, marker file. Re-checked on
    /// every open (a remote added by hand stops the next scan).
    pub fn open(dir: &Path, guard: &Guard) -> Result<Catalog, CatalogError> {
        guard.check(dir)?;
        std::fs::create_dir_all(dir).map_err(|e| CatalogError::io(dir, e))?;
        let dir = resolve(dir);
        if !dir.join(".git").exists() {
            git(&dir, &["init", "-q", "-b", "main"])?;
        }
        let hooks = dir.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).map_err(|e| CatalogError::io(&hooks, e))?;
        let hook = hooks.join("pre-push");
        if std::fs::read_to_string(&hook).ok().as_deref() != Some(PRE_PUSH) {
            write_atomic(&hook, PRE_PUSH.as_bytes())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).map_err(|e| CatalogError::io(&hook, e))?;
        }
        let hooks_s = hooks.to_string_lossy().to_string();
        if git(&dir, &["config", "--local", "--get", "core.hooksPath"]).ok().as_deref() != Some(hooks_s.as_str()) {
            git(&dir, &["config", "--local", "core.hooksPath", &hooks_s])?;
        }
        let gi = dir.join(".gitignore");
        if !std::fs::read_to_string(&gi).is_ok_and(|s| s.lines().any(|l| l.trim() == "/*") && s.lines().any(|l| l.trim() == "!/edits/")) {
            write_atomic(&gi, GITIGNORE.as_bytes())?;
        }
        let marker = dir.join(LOCAL_ONLY_MARKER);
        if !marker.exists() {
            write_atomic(&marker, MARKER_TEXT.as_bytes())?;
        }
        // Again after init: the catalog's own repo must have no remote.
        let remotes = git_remotes(&dir);
        if !remotes.is_empty() {
            return Err(CatalogError::Guard(format!("目錄的 git 有遠端（{}）；請移除遠端，目錄只能存在本機", remotes.join(", "))));
        }
        Ok(Catalog { dir })
    }

    pub fn files_dir(&self) -> PathBuf {
        self.dir.join("files")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.dir.join("projects")
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.dir.join("reports")
    }

    pub fn edits_dir(&self) -> PathBuf {
        self.dir.join("edits")
    }

    /// The author's edits of one record (`edits/<id>.json`), if any.
    pub fn edit(&self, id: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        valid_id(id).ok()?;
        serde_json::from_slice(&std::fs::read(self.edits_dir().join(format!("{id}.json"))).ok()?).ok()
    }

    /// Set `field` (e.g. `tags`) of record `id` (a sha256 or a unit id) in
    /// `edits/<id>.json` and commit that file in the local repo with the
    /// message `<field>: <id>`. Returns the new commit, or `None` when the
    /// value was already there. Only the author's edits are ever committed;
    /// scans never commit.
    pub fn record_edit(&self, id: &str, field: &str, value: serde_json::Value) -> Result<Option<String>, CatalogError> {
        valid_id(id)?;
        if field.is_empty() || field.len() > 40 || !field.chars().all(|c| c.is_ascii_lowercase() || c == '_') || field == "id" {
            return Err(CatalogError::Guard(format!("欄位名稱不合法：{field}")));
        }
        let rel = format!("edits/{id}.json");
        let path = self.dir.join(&rel);
        let mut m = self.edit(id).unwrap_or_default();
        m.insert("id".into(), id.into());
        m.insert(field.into(), value);
        let mut b = serde_json::to_vec_pretty(&m).map_err(|e| CatalogError::io(&path, std::io::Error::other(e)))?;
        b.push(b'\n');
        write_atomic(&path, &b)?;

        let paths = [rel.as_str(), ".gitignore", LOCAL_ONLY_MARKER];
        git(&self.dir, &[&["add", "--"][..], &paths].concat())?;
        let staged = git(&self.dir, &[&["diff", "--cached", "--name-only", "--"][..], &paths].concat())?;
        if staged.is_empty() {
            return Ok(None);
        }
        let mut args: Vec<String> = vec!["-c".into(), "commit.gpgsign=false".into()];
        if git(&self.dir, &["config", "user.name"]).is_err() || git(&self.dir, &["config", "user.email"]).is_err() {
            args.extend(["-c".into(), "user.name=LoreShelf".into(), "-c".into(), "user.email=loreshelf@localhost".into()]);
        }
        args.extend(["commit".into(), "-q".into(), "-m".into(), format!("{field}: {id}"), "--".into()]);
        args.extend(paths.iter().map(|p| p.to_string()));
        git(&self.dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
        git(&self.dir, &["rev-parse", "HEAD"]).map(Some)
    }

    /// The most recently written report (`reports/*-scan.md`).
    pub fn latest_report(dir: &Path) -> Option<PathBuf> {
        std::fs::read_dir(dir.join("reports"))
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .max()
            .map(|(_, p)| p)
    }
}

/// Record ids: lower-case hex sha256 or a unit id (`proj-…`, `pkg-…`, `data-…`).
fn valid_id(id: &str) -> Result<(), CatalogError> {
    if !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        Ok(())
    } else {
        Err(CatalogError::Guard(format!("紀錄 id 不合法：{id}")))
    }
}

/// Write via a temporary file in the same folder, then rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CatalogError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| CatalogError::io(parent, e))?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| CatalogError::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        CatalogError::io(path, e)
    })
}
