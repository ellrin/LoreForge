//! File helpers shared by every app: never overwrite a user's file.

use std::path::{Path, PathBuf};

/// `dir/stem.ext`, or `dir/stem-2.ext`… if taken. Never overwrites.
pub fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |n: usize| if ext.is_empty() { if n < 2 { stem.to_string() } else { format!("{stem}-{n}") } } else if n < 2 { format!("{stem}.{ext}") } else { format!("{stem}-{n}.{ext}") };
    let mut n = 1;
    let mut p = dir.join(name(n));
    while p.exists() || p.symlink_metadata().is_ok() {
        n = if n < 2 { 2 } else { n + 1 };
        p = dir.join(name(n));
    }
    p
}

/// Copy files that do not exist yet (recursively); never overwrite. Returns the number
/// of files copied.
pub fn copy_missing(from: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            n += copy_missing(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), target)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Member ids are folder names under `progress/`: lowercase ASCII letters, digits,
/// `-`, `_`; 1–40 chars; no dots (so never `..`).
pub fn valid_member(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Write through a temporary file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Expand a leading `~/` with the home folder.
pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") && let Some(home) = dirs::home_dir() {
        return home.join(rest);
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_path_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let a = free_path(d.path(), "card", "json");
        assert!(a.ends_with("card.json"));
        std::fs::write(&a, "x").unwrap();
        let b = free_path(d.path(), "card", "json");
        assert!(b.ends_with("card-2.json"));
        std::fs::write(&b, "x").unwrap();
        assert!(free_path(d.path(), "card", "json").ends_with("card-3.json"));
        assert!(free_path(d.path(), "dir", "").ends_with("dir"));
    }

    #[test]
    fn copy_missing_keeps_existing() {
        let d = tempfile::tempdir().unwrap();
        let from = d.path().join("from");
        let to = d.path().join("to");
        std::fs::create_dir_all(from.join("sub")).unwrap();
        std::fs::write(from.join("a.txt"), "template").unwrap();
        std::fs::write(from.join("sub/b.txt"), "template").unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("a.txt"), "mine").unwrap();
        assert_eq!(copy_missing(&from, &to).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(to.join("a.txt")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(to.join("sub/b.txt")).unwrap(), "template");
    }

    #[test]
    fn member_names() {
        assert!(valid_member("ellrin"));
        assert!(valid_member("daidai"));
        assert!(!valid_member(".."));
        assert!(!valid_member("a/b"));
        assert!(!valid_member(""));
        assert!(!valid_member("Ellrin"));
    }
}
