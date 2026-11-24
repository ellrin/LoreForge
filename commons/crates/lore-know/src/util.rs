//! Small helpers shared by every module that writes.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::Result;

/// Write `bytes` to `path` atomically: a temp file in the same folder (unique per process and
/// call), synced, then renamed over the target. The folder is made when missing. A reader sees
/// the old file or the new one, never half of one; on failure the temp file is removed.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let r = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if let Err(e) = r {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/b/c.txt");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
        // a target that is a folder cannot be replaced: an error, and no temp file stays
        let dir_target = d.path().join("a/b/dir");
        fs::create_dir_all(dir_target.join("x")).unwrap();
        assert!(write_atomic(&dir_target, b"x").is_err());
        assert_eq!(fs::read_dir(p.parent().unwrap()).unwrap().count(), 2);
    }
}
