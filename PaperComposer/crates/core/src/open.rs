//! Opening a LaTeX project where it is (開啟 LaTeX…): the folder becomes a
//! PaperComposer paper in place, the way an import would make it (body
//! `\input`s merged, one `sections/*.tex` per `\section`, one sentence per
//! line), but only after the author has seen every file that changes.
//!
//! `plan` stages the import in a scratch folder and compares it with the
//! project: nothing in the project is written. `apply` stages again, checks
//! that the project has not changed since the plan was shown, then writes
//! the new files (atomically, keeping permissions), moves stray section
//! files aside and removes the files merged into others. Hidden files (.git),
//! build output and everything the paper does not use are left alone.

use crate::paper::{self, APP_DIR};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where stray `sections/*.tex` (not read by the main file) are moved, so
/// they do not show up as sections.
pub const UNUSED_DIR: &str = "unused";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// The paper folder: the folder of the main file (inside the opened one).
    pub dir: PathBuf,
    /// The main file, relative to `dir` (becomes main.tex).
    pub main: String,
    /// New files, relative to `dir`.
    pub created: Vec<String>,
    /// Files rewritten (main.tex).
    pub changed: Vec<String>,
    /// Files merged into main.tex or a section, then removed: (file, into).
    pub merged: Vec<(String, String)>,
    /// Section files nothing reads: (from, to).
    pub moved: Vec<(String, String)>,
    /// What the project looked like when planned (see `fingerprint`).
    pub stamp: String,
}

/// Already a PaperComposer paper: opened as it is. Either it has the app's
/// folder, or main.tex reads every `sections/*.tex` directly and nothing
/// else is merged anywhere.
pub fn is_shaped(dir: &Path) -> bool {
    if dir.join(APP_DIR).is_dir() {
        return true;
    }
    let Ok(f) = paper::flatten_project(dir) else { return false };
    if f.main != dir.join("main.tex") {
        return false;
    }
    let sections = dir.join("sections");
    let Ok(rd) = std::fs::read_dir(&sections) else { return false };
    let files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "tex")).collect();
    if files.is_empty() {
        return false;
    }
    // every section file is read, nothing else is, and each holds one \section
    let used: Vec<&PathBuf> = f.used.iter().skip(1).collect();
    used.len() == files.len()
        && files.iter().all(|p| used.contains(&p))
        && files.iter().all(|p| {
            let t = std::fs::read_to_string(p).unwrap_or_default();
            t.lines().filter(|l| l.trim_start().starts_with("\\section")).count() == 1
        })
}

fn rel(base: &Path, p: &Path) -> String {
    p.strip_prefix(base).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Every file under `dir` (relative paths), hidden entries skipped.
fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(rel(dir, &p));
            }
        }
    }
    out.sort();
    out
}

/// Names, sizes and contents of the project's files: a change after the
/// preview (another editor, a sync) makes `apply` refuse.
fn fingerprint(dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in files(dir) {
        f.hash(&mut h);
        std::fs::read(dir.join(&f)).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn scratch() -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("papercomposer-open-{}-{n}", std::process::id()))
}

/// The paper as an import would make it, in `stage`, and the plan.
fn stage(folder: &Path, stage: &Path) -> Result<Plan, String> {
    let f = paper::flatten_project(folder)?;
    let dir = f.base.clone();
    let main_rel = rel(&dir, &f.main);
    // a main.tex that is not the main file and is not merged would be lost
    if main_rel != "main.tex" && dir.join("main.tex").is_file() && !f.used.contains(&dir.join("main.tex")) {
        return Err(format!("{main_rel} 是主檔，但資料夾裡另有一個 main.tex"));
    }
    paper::import_into(stage, &dir)?;
    let mut plan = Plan { dir: dir.clone(), main: main_rel, ..Default::default() };
    let staged = files(stage);
    for r in &staged {
        match std::fs::read(dir.join(r)) {
            Ok(old) if std::fs::read(stage.join(r)).is_ok_and(|new| new == old) => {}
            Ok(_) => plan.changed.push(r.clone()),
            Err(_) => plan.created.push(r.clone()),
        }
    }
    // files merged into others: their content is in main.tex or a section now
    for u in &f.used {
        let r = rel(&dir, u);
        if staged.contains(&r) {
            continue;
        }
        let into = if u == &f.main { "main.tex".to_string() } else { "sections/".to_string() };
        plan.merged.push((r, into));
    }
    // section files that nothing reads would show up as sections
    for e in std::fs::read_dir(dir.join("sections")).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let r = format!("sections/{n}");
        if n.ends_with(".tex") && !n.starts_with('.') && !f.used.contains(&e.path()) && !plan.merged.iter().any(|(m, _)| *m == r) {
            let to = (1..)
                .map(|k| if k == 1 { format!("{UNUSED_DIR}/{n}") } else { format!("{UNUSED_DIR}/{k}-{n}") })
                .find(|t| !dir.join(t).exists())
                .expect("a free name");
            plan.moved.push((r, to));
        }
    }
    // a stray section file with the name of a new one is moved first, so it is not "changed"
    plan.changed.retain(|c| !plan.moved.iter().any(|(m, _)| m == c));
    for (m, _) in &plan.moved {
        if staged.contains(m) && !plan.created.contains(m) {
            plan.created.push(m.clone());
        }
    }
    plan.created.sort();
    plan.stamp = fingerprint(&dir);
    Ok(plan)
}

/// What opening `folder` in place would change; nothing is written.
/// `None`: it is already a PaperComposer paper.
pub fn plan(folder: &Path) -> Result<Option<Plan>, String> {
    if is_shaped(folder) {
        return Ok(None);
    }
    let tmp = scratch();
    let res = stage(folder, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    res.map(Some)
}

/// Make the change `plan` showed, if the project is still as it was.
/// Returns the paper folder.
pub fn apply(folder: &Path, seen: &str) -> Result<PathBuf, String> {
    if is_shaped(folder) {
        return paper::flatten_project(folder).map(|f| f.base);
    }
    let tmp = scratch();
    let res = (|| {
        let plan = stage(folder, &tmp)?;
        if plan.stamp != seen {
            return Err("資料夾在預覽之後有變動，請重新開啟".to_string());
        }
        let dir = &plan.dir;
        let io = |e: std::io::Error| e.to_string();
        for (from, to) in &plan.moved {
            let dst = dir.join(to);
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d).map_err(io)?;
            }
            std::fs::rename(dir.join(from), dst).map_err(io)?;
        }
        for r in plan.created.iter().chain(&plan.changed) {
            let dst = dir.join(r);
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d).map_err(io)?;
            }
            paper::write_atomic_bytes(&dst, &std::fs::read(tmp.join(r)).map_err(io)?).map_err(io)?;
        }
        // only once everything new is written
        for (m, _) in &plan.merged {
            let p = dir.join(m);
            if p.is_file() {
                std::fs::remove_file(p).map_err(io)?;
            }
        }
        std::fs::create_dir_all(dir.join(APP_DIR)).map_err(io)?;
        Ok(dir.clone())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-open-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A co-author's project: ms.tex inputs intro.tex and sections/method.tex,
    /// a preamble \input{notation}, a stray section file, a figure, .git.
    fn project(root: &Path) {
        std::fs::create_dir_all(root.join("sections")).unwrap();
        std::fs::create_dir_all(root.join("figs")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            root.join("ms.tex"),
            "\\documentclass{article}\n\\input{notation}\n\\begin{document}\n\\section{Intro}\n\\input{intro}\n\\section{Method}\n\\input{sections/method}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(root.join("notation.tex"), "% generated by lore-notation\n\\newcommand{\\vw}{\\mathbf{w}}\n").unwrap();
        std::fs::write(root.join("intro.tex"), "First sentence. Second sentence.\n").unwrap();
        std::fs::write(root.join("sections/method.tex"), "We use $\\vw$.\n").unwrap();
        std::fs::write(root.join("sections/old.tex"), "\\section{Old}\nunused\n").unwrap();
        std::fs::write(root.join("figs/a.png"), "png").unwrap();
        std::fs::write(root.join("ms.aux"), "").unwrap();
        std::fs::write(root.join("refs.bib"), "").unwrap();
    }

    #[test]
    fn previews_without_writing_then_restructures_in_place() {
        let root = tmp("plan");
        project(&root);
        let before = fingerprint(&root);
        let plan = plan(&root).unwrap().expect("needs restructuring");
        assert_eq!(fingerprint(&root), before, "planning writes nothing");
        assert!(!root.join(APP_DIR).exists() && !root.join("main.tex").exists());
        assert_eq!(plan.main, "ms.tex");
        assert_eq!(plan.created, ["main.tex", "sections/01-intro.tex", "sections/02-method.tex"]);
        assert!(plan.changed.is_empty());
        assert_eq!(plan.merged, [("ms.tex".to_string(), "main.tex".to_string()), ("intro.tex".into(), "sections/".into()), ("sections/method.tex".into(), "sections/".into())]);
        assert_eq!(plan.moved, [("sections/old.tex".to_string(), "unused/old.tex".to_string())]);

        // a change after the preview is refused
        std::fs::write(root.join("intro.tex"), "Edited elsewhere.\n").unwrap();
        assert!(apply(&root, &plan.stamp).is_err());
        assert!(!root.join("main.tex").exists());
        let plan = super::plan(&root).unwrap().unwrap();

        let dir = apply(&root, &plan.stamp).unwrap();
        assert_eq!(dir, root);
        let main = std::fs::read_to_string(root.join("main.tex")).unwrap();
        assert!(main.contains("\\input{notation}") && main.contains("\\input{sections/01-intro}"), "{main}");
        assert!(std::fs::read_to_string(root.join("notation.tex")).unwrap().starts_with("% generated by lore-notation"), "notation.tex kept");
        assert!(paper::paper_macros(&root).contains_key("\\vw"), "its macros still reach the page");
        assert!(!root.join("ms.tex").exists() && !root.join("intro.tex").exists() && !root.join("sections/method.tex").exists());
        assert!(root.join("unused/old.tex").is_file() && root.join("figs/a.png").is_file());
        assert!(root.join(".git/HEAD").is_file() && root.join("ms.aux").is_file(), "hidden files and build output untouched");
        assert!(root.join(APP_DIR).is_dir());
        assert_eq!(paper::section_files(&root), ["01-intro.tex", "02-method.tex"]);
        let intro = std::fs::read_to_string(root.join("sections/01-intro.tex")).unwrap();
        assert!(intro.contains("Edited elsewhere."), "{intro}");
        // now it opens as it is
        assert!(is_shaped(&root));
        assert_eq!(super::plan(&root).unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_paper_made_by_papercomposer_opens_as_it_is() {
        let root = tmp("shaped");
        let dir = paper::create_paper(&root, "P").unwrap();
        assert!(is_shaped(&dir));
        assert_eq!(plan(&dir).unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keeps_permissions_on_rewrite() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let root = tmp("perm");
            let f = root.join("a.tex");
            std::fs::write(&f, "x").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
            paper::write_atomic(&f, "y").unwrap();
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::read_to_string(&f).unwrap(), "y");
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no temp file left");
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
