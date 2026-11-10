//! The PDF library: a folder **outside** any git work tree. PDFs are stored by content
//! (`<library>/ab/abcd….pdf`, sha256 of the bytes, like KnowledgeBase assets) and are never
//! modified or overwritten. Exports go to their own folder with free names.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Library {
    pub root: PathBuf,
    pub exports: PathBuf,
}

/// The git work tree that contains `p` (its top folder), if any. `p` need not exist:
/// the nearest existing ancestor is checked.
pub fn git_work_tree(p: &Path) -> Option<PathBuf> {
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().ok()?.join(p) };
    let existing = abs.ancestors().find(|a| a.exists())?;
    let real = existing.canonicalize().unwrap_or_else(|_| existing.to_path_buf());
    real.ancestors().find(|a| a.join(".git").exists()).map(Path::to_path_buf)
}

/// 64 lowercase hex digits.
pub fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Does this look like a PDF (`%PDF-` near the start)?
pub fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.len() > 8 && bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

impl Library {
    /// Open (and create) the library and the exports folder. Refuses either one inside
    /// a git work tree: PDFs never go into a repository.
    pub fn open(root: impl Into<PathBuf>, exports: impl Into<PathBuf>) -> Result<Library> {
        let (root, exports) = (root.into(), exports.into());
        for (what, p) in [("書庫", &root), ("匯出資料夾", &exports)] {
            if let Some(repo) = git_work_tree(p) {
                return Err(Error::Invalid(format!("{what} {} 在 git 工作目錄 {} 裡；PDF 不能放進 repo，請改到 repo 外（config.toml 的 library / exports，或 LOREREADER_LIBRARY）", p.display(), repo.display())));
            }
        }
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(&exports)?;
        Ok(Library { root, exports })
    }

    pub fn path(&self, sha: &str) -> PathBuf {
        self.root.join(&sha[..2]).join(format!("{sha}.pdf"))
    }

    pub fn has(&self, sha: &str) -> bool {
        valid_sha(sha) && self.path(sha).is_file()
    }

    pub fn read(&self, sha: &str) -> Result<Vec<u8>> {
        if !self.has(sha) {
            return Err(Error::NotFound(format!("書庫裡沒有 {sha}")));
        }
        Ok(std::fs::read(self.path(sha))?)
    }

    /// Store `bytes` (a PDF). Returns the sha256 and whether the file is new.
    /// An existing file with that hash is left alone.
    pub fn add(&self, bytes: &[u8]) -> Result<(String, bool)> {
        if !looks_like_pdf(bytes) {
            return Err(Error::Invalid("不是 PDF 檔".into()));
        }
        let sha = sha256(bytes);
        let p = self.path(&sha);
        if p.is_file() {
            return Ok((sha, false));
        }
        crate::write_atomic(&p, bytes)?;
        Ok((sha, true))
    }

    /// Copy `bytes` to the exports folder as `<stem>.pdf` (or `<stem>-2.pdf`, …).
    pub fn export(&self, bytes: &[u8], stem: &str) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.exports)?;
        let p = crate::free_path(&self.exports, &crate::safe_stem(stem), "pdf");
        // create_new: never replace a file that appeared in the meantime.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&p)?;
        f.write_all(bytes)?;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_addressed_and_never_in_git() {
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(d.path().join("lib"), d.path().join("out")).unwrap();
        let pdf = crate::synth::text_pdf(&[vec!["Hello".into()]]);
        let (sha, new) = lib.add(&pdf).unwrap();
        assert!(new && valid_sha(&sha));
        assert_eq!(lib.path(&sha), d.path().join("lib").join(&sha[..2]).join(format!("{sha}.pdf")));
        assert!(!lib.add(&pdf).unwrap().1, "same bytes, same file");
        assert_eq!(lib.read(&sha).unwrap(), pdf);
        assert!(lib.add(b"hello world, not a pdf").is_err());
        let a = lib.export(&pdf, "x").unwrap();
        let b = lib.export(&pdf, "x").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), pdf);

        // A library inside a git work tree is refused (also when it does not exist yet).
        std::fs::create_dir_all(d.path().join("repo/.git")).unwrap();
        let e = Library::open(d.path().join("repo/a/lib"), d.path().join("out")).unwrap_err();
        assert!(e.to_string().contains("git"), "{e}");
        assert!(Library::open(d.path().join("lib"), d.path().join("repo/exports")).is_err());
        assert!(!d.path().join("repo/a").exists());
    }
}
