//! 刪除論文: a workspace paper's folder goes to the system Trash; a paper
//! opened where it is only leaves the list (opened.toml), its folder untouched.
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/papers/{name}/delete | → `{action: "trashed" \| "unlisted"}`; the server decides which, never the page |
//!
//! Only a real folder directly in the workspace's `papers/` is trashed (no
//! symlink, nothing that resolves elsewhere), never removed outright. The
//! paper's build cache goes too, so a later paper of the same name starts clean.

use super::*;
use opened::Registry;

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/api/papers/{name}/delete", post(delete))
}

#[derive(Debug, PartialEq)]
pub enum Removed {
    /// The workspace folder went to the Trash.
    Trashed(PathBuf),
    /// Only the opened.toml entry goes.
    Unlisted,
}

#[derive(Debug, PartialEq)]
pub enum Refused {
    BadName,
    NotFound,
    /// In papers/ but not a plain folder there (a symlink, or resolves elsewhere).
    Outside,
    Trash(String),
}

/// `papers/<name>` when it is a real folder directly inside `papers/`.
fn workspace_folder(papers: &Path, name: &str) -> Result<Option<PathBuf>, Refused> {
    let dir = papers.join(name);
    let Ok(meta) = std::fs::symlink_metadata(&dir) else { return Ok(None) };
    if meta.file_type().is_symlink() {
        return Err(Refused::Outside);
    }
    if !meta.is_dir() {
        return Ok(None);
    }
    let (Ok(d), Ok(p)) = (dir.canonicalize(), papers.canonicalize()) else { return Err(Refused::Outside) };
    if d.parent() != Some(p.as_path()) || d == p {
        return Err(Refused::Outside);
    }
    Ok(Some(d))
}

/// Decide and do: trash a workspace paper (through `trash`), or say the
/// opened paper `name` is only to be unlisted. `papers` is the workspace's
/// `papers/` (none without a workspace).
pub fn remove_paper(papers: Option<&Path>, reg: &Registry, name: &str, trash: &dyn Fn(&Path) -> Result<(), String>) -> Result<Removed, Refused> {
    if !paper::safe_name(name) {
        return Err(Refused::BadName);
    }
    if let Some(papers) = papers {
        if let Some(dir) = workspace_folder(papers, name)? {
            trash(&dir).map_err(Refused::Trash)?;
            return Ok(Removed::Trashed(dir));
        }
    }
    match reg.get(name) {
        Some(_) => Ok(Removed::Unlisted),
        None => Err(Refused::NotFound),
    }
}

/// The real Trash: NSFileManager on macOS (no Finder automation prompt).
fn system_trash(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

async fn delete(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    let (ws, demo) = {
        let l = s.loaded.read().await;
        (l.workspace.clone(), l.demo)
    };
    // the bundled demo workspace is part of the app: nothing in it goes to the Trash
    let papers = ws.filter(|_| !demo).map(|w| w.join("papers"));
    let reg = opened::registry();
    let n = name.clone();
    let done = tokio::task::spawn_blocking(move || remove_paper(papers.as_deref(), &reg, &n, &system_trash)).await;
    let action = match done {
        Ok(Ok(Removed::Trashed(dir))) => {
            // an opened.toml entry for the same folder would list it again
            let _ = opened::update(|r| r.papers.retain(|e| !Registry::same(&e.path, &dir) && e.path != dir));
            "trashed"
        }
        Ok(Ok(Removed::Unlisted)) => {
            if let Err(e) = opened::update(|r| r.forget(&name)) {
                return bad(&e);
            }
            "unlisted"
        }
        Ok(Err(Refused::BadName)) => return bad("invalid paper name"),
        Ok(Err(Refused::NotFound)) => return (StatusCode::NOT_FOUND, "paper not found").into_response(),
        Ok(Err(Refused::Outside)) => return bad("不在工作區的 papers/ 裡，不刪除"),
        Ok(Err(Refused::Trash(e))) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("沒能移到垃圾桶：{e}")).into_response(),
        Err(e) => return bad(&e.to_string()),
    };
    // the build cache (thumbnail, numbers, bibliography) is keyed by the name
    for key in [name.clone(), format!("{name}-blind")] {
        let _ = std::fs::remove_dir_all(export::cache_dir(&key));
    }
    Json(serde_json::json!({ "action": action })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-remove-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn paper_at(dir: &Path) {
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        std::fs::write(dir.join("main.tex"), "\\title{x}\n").unwrap();
    }

    /// A trash that moves the folder into `bin/` (nothing touches the real Trash).
    fn mover<'a>(bin: &'a Path, seen: &'a RefCell<Vec<PathBuf>>) -> impl Fn(&Path) -> Result<(), String> + 'a {
        move |p: &Path| {
            seen.borrow_mut().push(p.to_path_buf());
            std::fs::create_dir_all(bin).unwrap();
            std::fs::rename(p, bin.join(p.file_name().unwrap())).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn a_workspace_paper_moves_to_the_trash() {
        let root = tmp("ws");
        let papers = root.join("ws/papers");
        paper_at(&papers.join("ellrin study"));
        paper_at(&papers.join("daidai notes"));
        let bin = root.join("bin");
        let (bin_, seen) = (&bin, RefCell::new(vec![]));
        let trash = mover(bin_, &seen);
        let r = remove_paper(Some(&papers), &Registry::default(), "ellrin study", &trash).unwrap();
        assert!(matches!(r, Removed::Trashed(_)));
        assert!(!papers.join("ellrin study").exists(), "moved out of papers/");
        assert!(bin.join("ellrin study/main.tex").is_file(), "kept whole in the trash");
        assert!(papers.join("daidai notes/main.tex").is_file(), "the other paper stays");
        assert_eq!(seen.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_names_and_traversal_are_refused() {
        let root = tmp("bad");
        let papers = root.join("ws/papers");
        paper_at(&papers.join("ellrin"));
        paper_at(&root.join("ws/profile"));
        let (bin_, seen) = (&root.join("bin"), RefCell::new(vec![]));
        let trash = mover(bin_, &seen);
        for name in ["", "..", ".", "../profile", "a/b", "/etc", ".git", "ellrin/.."] {
            assert_eq!(remove_paper(Some(&papers), &Registry::default(), name, &trash), Err(Refused::BadName), "{name:?}");
        }
        assert!(seen.borrow().is_empty(), "nothing was trashed");
        assert!(root.join("ws/profile/main.tex").is_file());
        assert_eq!(remove_paper(Some(&papers), &Registry::default(), "nobody", &trash), Err(Refused::NotFound));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_papers_is_not_followed() {
        let root = tmp("link");
        let papers = root.join("ws/papers");
        std::fs::create_dir_all(&papers).unwrap();
        let elsewhere = root.join("Documents/daidai paper");
        paper_at(&elsewhere);
        std::os::unix::fs::symlink(&elsewhere, papers.join("daidai")).unwrap();
        let (bin_, seen) = (&root.join("bin"), RefCell::new(vec![]));
        let trash = mover(bin_, &seen);
        assert_eq!(remove_paper(Some(&papers), &Registry::default(), "daidai", &trash), Err(Refused::Outside));
        assert!(seen.borrow().is_empty());
        assert!(elsewhere.join("main.tex").is_file(), "the target is untouched");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_paper_opened_elsewhere_is_only_unlisted() {
        let root = tmp("ext");
        let papers = root.join("ws/papers");
        std::fs::create_dir_all(&papers).unwrap();
        let ext = root.join("Projects/ellrin draft");
        paper_at(&ext);
        let mut reg = Registry::default();
        let id = reg.add(&ext, "latex", None, &[]);
        let (bin_, seen) = (&root.join("bin"), RefCell::new(vec![]));
        let trash = mover(bin_, &seen);
        assert_eq!(remove_paper(Some(&papers), &reg, &id, &trash), Ok(Removed::Unlisted));
        // without a workspace too
        assert_eq!(remove_paper(None, &reg, &id, &trash), Ok(Removed::Unlisted));
        assert!(seen.borrow().is_empty(), "never trashed");
        assert!(ext.join("main.tex").is_file(), "the author's own folder stays");
        let _ = std::fs::remove_dir_all(&root);
    }
}
