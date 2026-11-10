//! Opening a LaTeX project where it is (開啟 LaTeX…): the folder becomes a
//! PaperComposer paper in place.
//!
//! The default keeps the author's files as they are: the main file keeps its
//! name, `\input`/`\include`d files stay where they are, nothing is merged,
//! renamed or moved. PaperComposer only adds `.papercomposer/` with a
//! `layout.toml` naming the main file; the editor reads the sections from the
//! main file's own `\input`s (see `Layout`).
//!
//! Restructuring (body `\input`s merged, one `sections/*.tex` per `\section`,
//! one sentence per line, main file renamed to main.tex) is an explicit choice
//! (`apply(.., restructure = true)`) and copies every file it touches to
//! `.papercomposer/original/` first.
//!
//! `plan` stages the restructuring in a scratch folder and compares it with
//! the project: nothing in the project is written. `apply` stages again,
//! checks that the project has not changed since the plan was shown, then
//! writes. Hidden files (.git), build output and everything the paper does
//! not use are left alone.

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
    /// The main file, relative to `dir`. It keeps its name unless the
    /// author chooses to restructure.
    pub main: String,
    /// The sections the editor shows when the project is opened as it is:
    /// (file relative to `dir`, title).
    #[serde(default)]
    pub sections: Vec<(String, String)>,
    /// `\section`s typed inside the main file itself: kept, not shown as sections.
    #[serde(default)]
    pub inline_sections: usize,
    /// The restructuring below (opt-in): new files, relative to `dir`.
    pub created: Vec<String>,
    /// Why restructuring is not offered (e.g. another main.tex is in the way).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restructure_error: Option<String>,
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

/// What opening `folder` in place would do; nothing is written.
/// `None`: it is already a PaperComposer paper.
pub fn plan(folder: &Path) -> Result<Option<Plan>, String> {
    if is_shaped(folder) {
        return Ok(None);
    }
    let f = paper::flatten_project(folder)?;
    let dir = f.base.clone();
    let main = rel(&dir, &f.main);
    let found = read_layout(&dir, &main);
    let sections: Vec<(String, String)> = found.entries.iter().map(|e| (e.file.clone(), e.title.clone())).collect();
    let tmp = scratch();
    let mut plan = match stage(folder, &tmp) {
        Ok(p) => p,
        Err(e) => Plan { dir: dir.clone(), main: main.clone(), restructure_error: Some(e), stamp: fingerprint(&dir), ..Default::default() },
    };
    let _ = std::fs::remove_dir_all(&tmp);
    plan.sections = sections;
    plan.inline_sections = found.inline;
    Ok(Some(plan))
}

/// Copy the `rels` files of `dir` to `.papercomposer/original[-n]/`, same
/// relative paths. Returns that folder's name under `dir`.
fn backup(dir: &Path, rels: &[String]) -> Result<String, String> {
    let io = |e: std::io::Error| e.to_string();
    let name = (1..)
        .map(|k| if k == 1 { format!("{APP_DIR}/original") } else { format!("{APP_DIR}/original-{k}") })
        .find(|n| !dir.join(n).exists())
        .expect("a free name");
    for r in rels {
        let from = dir.join(r);
        if !from.is_file() {
            continue;
        }
        let to = dir.join(&name).join(r);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(io)?;
        }
        std::fs::copy(&from, &to).map_err(io)?;
    }
    Ok(name)
}

/// Open `folder`, if the project is still as `plan` showed it.
/// `restructure` false (the default): only `.papercomposer/` is added, the
/// author's files are not touched. true: the project is restructured after
/// every file that changes was copied to `.papercomposer/original/`.
/// Returns the paper folder.
pub fn apply(folder: &Path, seen: &str, restructure: bool) -> Result<PathBuf, String> {
    if is_shaped(folder) {
        return paper::flatten_project(folder).map(|f| f.base);
    }
    let io = |e: std::io::Error| e.to_string();
    if !restructure {
        let f = paper::flatten_project(folder)?;
        let dir = f.base.clone();
        if fingerprint(&dir) != seen {
            return Err("資料夾在預覽之後有變動，請重新開啟".to_string());
        }
        let main = rel(&dir, &f.main);
        if read_layout(&dir, &main).entries.is_empty() {
            return Err("主檔沒有分檔的章節，請選「整理成章節檔」".to_string());
        }
        std::fs::create_dir_all(dir.join(APP_DIR)).map_err(io)?;
        write_layout(&dir, &main)?;
        return Ok(dir);
    }
    let tmp = scratch();
    let res = (|| {
        let plan = stage(folder, &tmp)?;
        if plan.stamp != seen {
            return Err("資料夾在預覽之後有變動，請重新開啟".to_string());
        }
        let dir = &plan.dir;
        // the originals first: everything rewritten, merged or moved
        let touched: Vec<String> = plan.changed.iter().cloned().chain(plan.merged.iter().map(|m| m.0.clone())).chain(plan.moved.iter().map(|m| m.0.clone())).collect();
        backup(dir, &touched)?;
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

// ── A project opened as it is ───────────────────────────
//
// `.papercomposer/layout.toml` (`main = "ms.tex"`) marks a paper whose files
// are the author's own. Its sections are the main file's body `\input` /
// `\include`s: a file holding a `\section` is a section (title from the
// file); a file right after a `\section{Title}` typed in the main file is the
// body of that section (its title is edited in the main file). Any other
// `\input` (an abstract, a table) is not a section and is never written.

pub const LAYOUT_FILE: &str = "layout.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct LayoutFile {
    main: String,
}

fn write_layout(dir: &Path, main: &str) -> Result<(), String> {
    let t = toml::to_string(&LayoutFile { main: main.to_string() }).map_err(|e| e.to_string())?;
    paper::write_atomic(&dir.join(APP_DIR).join(LAYOUT_FILE), &t).map_err(|e| e.to_string())
}

/// The main file's name when the paper is opened as it is.
fn layout_main(dir: &Path) -> Option<String> {
    let t = std::fs::read_to_string(dir.join(APP_DIR).join(LAYOUT_FILE)).ok()?;
    toml::from_str::<LayoutFile>(&t).ok().map(|l| l.main).filter(|m| !m.is_empty() && !m.contains("..") && !m.starts_with('/'))
}

/// A paper opened as it is (its own main file name and `\input` structure).
pub fn is_custom(dir: &Path) -> bool {
    layout_main(dir).is_some()
}

/// The main file of a paper: main.tex unless it was opened as it is.
pub fn main_file(dir: &Path) -> PathBuf {
    dir.join(layout_main(dir).unwrap_or_else(|| "main.tex".into()))
}

/// Whether the folder is a paper PaperComposer can read.
pub fn is_paper(dir: &Path) -> bool {
    main_file(dir).is_file()
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    /// Relative to the paper folder, `/` separated.
    file: String,
    title: String,
    /// The title is typed in the main file: its byte range there.
    title_in_main: Option<(usize, usize)>,
}

struct Found {
    entries: Vec<Entry>,
    /// `\section`s typed in the main file with no file after them.
    inline: usize,
}

fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'%' && (i == 0 || b[i - 1] != b'\\') {
            return &line[..i];
        }
    }
    line
}

/// `\cmd{arg}` at the start of `s`: the argument (nested braces) and its
/// byte offset in `s`.
fn arg_of<'a>(s: &'a str, cmd: &str) -> Option<(&'a str, usize)> {
    let rest = s.strip_prefix(cmd)?;
    let rest = rest.strip_prefix('*').unwrap_or(rest);
    let rest = match rest.strip_prefix('[') {
        Some(r) => &r[r.find(']')? + 1..],
        None => rest,
    };
    let rest = rest.strip_prefix('{')?;
    let start = s.len() - rest.len();
    let mut depth = 1;
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&rest[..i], start));
                }
            }
            _ => {}
        }
    }
    None
}

fn tex_file(dir: &Path, arg: &str) -> Option<String> {
    let arg = arg.trim().trim_start_matches("./");
    if arg.is_empty() || arg.contains("..") || arg.starts_with('/') || arg.contains('\\') {
        return None;
    }
    [arg.to_string(), format!("{arg}.tex")].into_iter().find(|n| n.ends_with(".tex") && dir.join(n).is_file())
}

fn read_layout(dir: &Path, main: &str) -> Found {
    let text = std::fs::read(dir.join(main)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let body_at = text.find("\\begin{document}").map(|i| i + "\\begin{document}".len()).unwrap_or(0);
    let mut found = Found { entries: vec![], inline: 0 };
    let mut pending: Option<(String, (usize, usize))> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        if at + line.len() <= body_at {
            continue;
        }
        let code = strip_comment(line);
        let mut i = 0;
        while i < code.len() {
            let rest = &code[i..];
            if !rest.starts_with('\\') {
                i += rest.chars().next().map_or(1, char::len_utf8);
                continue;
            }
            if let Some((arg, start)) = arg_of(rest, "\\section") {
                if pending.take().is_some() {
                    found.inline += 1;
                }
                pending = Some((arg.to_string(), (at + i + start, at + i + start + arg.len())));
                i += start + arg.len();
                continue;
            }
            if let Some((arg, start)) = ["\\input", "\\include"].iter().find_map(|c| arg_of(rest, c)) {
                if let Some(file) = tex_file(dir, arg) {
                    let own = std::fs::read_to_string(dir.join(&file)).ok().and_then(|t| {
                        t.lines().any(|l| l.trim_start().starts_with("\\section")).then(|| paper::parse_section(&file, &t).title)
                    });
                    if !found.entries.iter().any(|e| e.file == file) {
                        match (own, pending.take()) {
                            (Some(title), p) => {
                                found.inline += usize::from(p.is_some());
                                found.entries.push(Entry { file, title, title_in_main: None });
                            }
                            (None, Some((title, range))) => found.entries.push(Entry { file, title, title_in_main: Some(range) }),
                            (None, None) => {}
                        }
                    }
                }
                i += start + arg.len();
                continue;
            }
            i += 1;
        }
    }
    found.inline += usize::from(pending.is_some());
    found
}

fn entries(dir: &Path) -> Vec<Entry> {
    read_layout(dir, &layout_main(dir).unwrap_or_default()).entries
}

fn read_entry(dir: &Path, e: &Entry) -> Option<paper::Section> {
    let text = std::fs::read_to_string(dir.join(&e.file)).ok()?;
    let mut s = paper::parse_section(&e.file, &text);
    s.title = e.title.clone();
    Some(s)
}

/// The sections the editor shows.
pub fn sections(dir: &Path) -> Vec<paper::Section> {
    if !is_custom(dir) {
        return paper::load_paper(dir);
    }
    entries(dir).iter().filter_map(|e| read_entry(dir, e)).collect()
}

/// The section `file` (the id the page uses) of the paper.
pub fn read_section(dir: &Path, file: &str) -> Option<paper::Section> {
    if !is_custom(dir) {
        if !paper::safe_name(file) {
            return None;
        }
        return std::fs::read_to_string(dir.join("sections").join(file)).ok().map(|t| paper::parse_section(file, &t));
    }
    let e = entries(dir).into_iter().find(|e| e.file == file)?;
    read_entry(dir, &e)
}

/// Whether `file` may be written as a section of this paper: a section
/// file of the standard layout, or a file the main file inputs.
pub fn is_section_file(dir: &Path, file: &str) -> bool {
    if is_custom(dir) {
        entries(dir).iter().any(|e| e.file == file)
    } else {
        paper::safe_name(file) && file.ends_with(".tex")
    }
}

/// Write a section back. A section whose title is typed in the main file
/// keeps it there: its file holds only the body.
pub fn write_section(dir: &Path, s: &paper::Section) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let Some(e) = is_custom(dir).then(|| entries(dir).into_iter().find(|e| e.file == s.file)).flatten() else {
        std::fs::create_dir_all(dir.join("sections")).map_err(io)?;
        return paper::write_atomic(&dir.join("sections").join(&s.file), &paper::render_section(s)).map_err(io);
    };
    let path = dir.join(&e.file);
    let Some((a, b)) = e.title_in_main else {
        return paper::write_atomic(&path, &paper::render_section(s)).map_err(io);
    };
    let text = paper::render_section(s);
    let body = text.split_once('\n').map(|x| x.1).unwrap_or("").trim_start_matches('\n');
    paper::write_atomic(&path, body).map_err(io)?;
    if s.title != e.title {
        let main = main_file(dir);
        let mut t = std::fs::read_to_string(&main).map_err(io)?;
        if t.get(a..b) == Some(e.title.as_str()) {
            t.replace_range(a..b, &s.title);
            paper::write_atomic(&main, &t).map_err(io)?;
        }
    }
    Ok(())
}

/// The paper's sources as they are on disk now (see `paper::stamp`).
pub fn stamp(dir: &Path) -> String {
    if !is_custom(dir) {
        return paper::stamp(dir);
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::fs::read(main_file(dir)).unwrap_or_default().hash(&mut h);
    for e in entries(dir) {
        e.file.hash(&mut h);
        std::fs::read(dir.join(&e.file)).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// The main file and the section files: what the paper's text is read from.
pub fn source_files(dir: &Path) -> Vec<PathBuf> {
    if !is_custom(dir) {
        return std::iter::once(dir.join("main.tex")).chain(paper::section_files(dir).into_iter().map(|f| dir.join("sections").join(f))).collect();
    }
    std::iter::once(main_file(dir)).chain(entries(dir).into_iter().map(|e| dir.join(e.file))).collect()
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

    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        files(root).into_iter().map(|f| (f.clone(), std::fs::read(root.join(&f)).unwrap())).collect()
    }

    #[test]
    fn opens_as_it_is_by_default() {
        let root = tmp("keep");
        project(&root);
        let before = snapshot(&root);
        let plan = plan(&root).unwrap().expect("not a paper yet");
        assert_eq!(snapshot(&root), before, "planning writes nothing");
        assert_eq!(plan.main, "ms.tex");
        // the sections come from the main file's own \input structure
        assert_eq!(plan.sections, [("intro.tex".to_string(), "Intro".to_string()), ("sections/method.tex".into(), "Method".into())]);
        assert_eq!(plan.inline_sections, 0);

        // a change after the preview is refused
        std::fs::write(root.join("intro.tex"), "Edited elsewhere.\n").unwrap();
        assert!(apply(&root, &plan.stamp, false).is_err());
        assert!(!root.join(APP_DIR).exists());
        let plan = super::plan(&root).unwrap().unwrap();
        let before = snapshot(&root);

        let dir = apply(&root, &plan.stamp, false).unwrap();
        assert_eq!(dir, root);
        // not renamed, not merged, not moved: the only addition is .papercomposer/
        let after = snapshot(&root);
        assert_eq!(after.iter().filter(|(f, _)| !f.starts_with(APP_DIR)).cloned().collect::<Vec<_>>(), before, "the author's files are as they were");
        assert!(!root.join("main.tex").exists() && root.join("ms.tex").is_file() && root.join("intro.tex").is_file());
        assert!(root.join("sections/old.tex").is_file(), "nothing moved to unused/");
        assert!(is_custom(&root) && is_shaped(&root));
        assert_eq!(main_file(&root), root.join("ms.tex"));
        assert_eq!(super::plan(&root).unwrap(), None);

        // the editor reads and writes those files
        let secs = sections(&root);
        assert_eq!(secs.iter().map(|s| (s.file.as_str(), s.title.as_str())).collect::<Vec<_>>(), [("intro.tex", "Intro"), ("sections/method.tex", "Method")]);
        assert_eq!(secs[0].paragraphs, ["Edited elsewhere."]);
        let st = stamp(&root);
        let mut intro = secs[0].clone();
        intro.paragraphs = vec!["Rewritten.".into()];
        intro.title = "Introduction".into();
        write_section(&root, &intro).unwrap();
        // the title lives in ms.tex, the file keeps only the body
        assert_eq!(std::fs::read_to_string(root.join("intro.tex")).unwrap().trim(), "Rewritten.");
        let ms = std::fs::read_to_string(root.join("ms.tex")).unwrap();
        assert!(ms.contains("\\section{Introduction}\n\\input{intro}") && ms.contains("\\input{notation}"), "{ms}");
        assert_ne!(stamp(&root), st);
        assert_eq!(read_section(&root, "intro.tex").unwrap().title, "Introduction");
        assert!(is_section_file(&root, "sections/method.tex") && !is_section_file(&root, "notation.tex") && !is_section_file(&root, "../x.tex"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_section_in_its_own_file_keeps_its_heading_there() {
        let root = tmp("ownfile");
        std::fs::create_dir_all(root.join("chapters")).unwrap();
        std::fs::write(root.join("paper.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{abstract}\n\\section{Typed here}\nInline text.\n\\input{chapters/results.tex} % ok\n\\end{document}\n").unwrap();
        std::fs::write(root.join("abstract.tex"), "An abstract.\n").unwrap();
        std::fs::write(root.join("chapters/results.tex"), "\\section{Results}\\label{sec:r}\nWe found it.\n").unwrap();
        let p = plan(&root).unwrap().unwrap();
        // the abstract is no section; the heading typed in paper.tex with text after it (not a file) is inline
        assert_eq!(p.sections, [("chapters/results.tex".to_string(), "Results".to_string())]);
        assert_eq!(p.inline_sections, 1);
        apply(&root, &p.stamp, false).unwrap();
        let mut s = read_section(&root, "chapters/results.tex").unwrap();
        assert_eq!(s.tail, "\\label{sec:r}");
        s.paragraphs = vec!["Changed.".into()];
        write_section(&root, &s).unwrap();
        let t = std::fs::read_to_string(root.join("chapters/results.tex")).unwrap();
        assert!(t.starts_with("\\section{Results}\\label{sec:r}") && t.contains("Changed."), "{t}");
        assert!(std::fs::read_to_string(root.join("abstract.tex")).unwrap().starts_with("An abstract."));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restructuring_is_a_choice_and_keeps_the_originals() {
        let root = tmp("plan");
        project(&root);
        let before = snapshot(&root);
        let plan = plan(&root).unwrap().expect("needs restructuring");
        assert_eq!(snapshot(&root), before, "planning writes nothing");
        assert!(!root.join(APP_DIR).exists() && !root.join("main.tex").exists());
        assert_eq!(plan.main, "ms.tex");
        assert_eq!(plan.created, ["main.tex", "sections/01-intro.tex", "sections/02-method.tex"]);
        assert!(plan.changed.is_empty());
        assert_eq!(plan.merged, [("ms.tex".to_string(), "main.tex".to_string()), ("intro.tex".into(), "sections/".into()), ("sections/method.tex".into(), "sections/".into())]);
        assert_eq!(plan.moved, [("sections/old.tex".to_string(), "unused/old.tex".to_string())]);

        // a change after the preview is refused
        std::fs::write(root.join("intro.tex"), "Edited elsewhere.\n").unwrap();
        assert!(apply(&root, &plan.stamp, true).is_err());
        assert!(!root.join("main.tex").exists());
        let plan = super::plan(&root).unwrap().unwrap();
        let originals = [
            ("ms.tex", std::fs::read(root.join("ms.tex")).unwrap()),
            ("intro.tex", std::fs::read(root.join("intro.tex")).unwrap()),
            ("sections/method.tex", std::fs::read(root.join("sections/method.tex")).unwrap()),
            ("sections/old.tex", std::fs::read(root.join("sections/old.tex")).unwrap()),
        ];

        let dir = apply(&root, &plan.stamp, true).unwrap();
        assert_eq!(dir, root);
        let main = std::fs::read_to_string(root.join("main.tex")).unwrap();
        assert!(main.contains("\\input{notation}") && main.contains("\\input{sections/01-intro}"), "{main}");
        assert!(std::fs::read_to_string(root.join("notation.tex")).unwrap().starts_with("% generated by lore-notation"), "notation.tex kept");
        assert!(paper::paper_macros(&root).contains_key("\\vw"), "its macros still reach the page");
        assert!(!root.join("ms.tex").exists() && !root.join("intro.tex").exists() && !root.join("sections/method.tex").exists());
        assert!(root.join("unused/old.tex").is_file() && root.join("figs/a.png").is_file());
        assert!(root.join(".git/HEAD").is_file() && root.join("ms.aux").is_file(), "hidden files and build output untouched");
        // every original is in the backup, byte for byte
        for (f, bytes) in &originals {
            assert_eq!(&std::fs::read(root.join(APP_DIR).join("original").join(f)).unwrap(), bytes, "{f}");
        }
        assert!(!is_custom(&root));
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
