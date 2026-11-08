//! A LaTeX project on disk: find the main file, follow `\input` / `\include`,
//! strip comments, and map offsets in the expanded text back to file:line.

use super::scan::{find_command, group_text, strip_comments};
use crate::error::{Error, Result};
use crate::model::SourceRef;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct Span {
    start: usize,
    end: usize,
    file: String,
    /// Line (1-based) of the span's first byte in `file`.
    line0: usize,
}

/// The expanded, comment-free text of a LaTeX project.
#[derive(Debug, Clone)]
pub struct LatexProject {
    pub root: PathBuf,
    /// Main file, relative to `root`.
    pub main: String,
    /// Whole document with inputs expanded and comments removed.
    pub text: String,
    spans: Vec<Span>,
    /// Files that `\input` named but were not found.
    pub missing: Vec<String>,
}

const MAIN_NAMES: &[&str] = &[
    "main.tex",
    "ms.tex",
    "paper.tex",
    "article.tex",
    "manuscript.tex",
];

fn tex_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if depth < 3 {
                tex_files(root, &p, depth + 1, out);
            }
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tex")) {
            out.push(p.strip_prefix(root).unwrap_or(&p).to_path_buf());
        }
    }
}

/// The main `.tex` file under `root`: has `\documentclass` and `\begin{document}`.
/// Several candidates: a conventional name wins, else the largest.
pub fn find_main(root: &Path) -> Result<String> {
    let mut files = vec![];
    tex_files(root, root, 0, &mut files);
    let mut cands: Vec<(PathBuf, usize)> = vec![];
    for f in files {
        let Ok(text) = std::fs::read_to_string(root.join(&f)) else {
            continue;
        };
        let t = strip_comments(&text);
        if t.contains("\\documentclass") && t.contains("\\begin{document}") {
            cands.push((f, t.len()));
        }
    }
    if let Some((f, _)) = cands
        .iter()
        .find(|(f, _)| MAIN_NAMES.iter().any(|n| f.as_os_str() == *n))
    {
        return Ok(slash(f));
    }
    cands.sort_by_key(|c| std::cmp::Reverse(c.1));
    cands
        .first()
        .map(|(f, _)| slash(f))
        .ok_or_else(|| Error::NoMainTex(root.to_path_buf()))
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

impl LatexProject {
    /// Open a source directory and find its main file.
    pub fn open(root: &Path) -> Result<LatexProject> {
        let main = find_main(root)?;
        LatexProject::open_main(root, &main)
    }

    /// Open with a known main file (relative to `root`).
    pub fn open_main(root: &Path, main: &str) -> Result<LatexProject> {
        let mut p = LatexProject {
            root: root.to_path_buf(),
            main: main.to_string(),
            text: String::new(),
            spans: vec![],
            missing: vec![],
        };
        p.expand(main, 0)?;
        Ok(p)
    }

    /// Build a project from text alone (tests, snippets).
    pub fn from_text(name: &str, text: &str) -> LatexProject {
        let t = strip_comments(text);
        LatexProject {
            root: PathBuf::from("."),
            main: name.into(),
            spans: vec![Span {
                start: 0,
                end: t.len(),
                file: name.into(),
                line0: 1,
            }],
            text: t,
            missing: vec![],
        }
    }

    fn resolve(&self, name: &str, from_dir: &str) -> Option<String> {
        let name = name.trim();
        let mut tries = vec![];
        for base in [from_dir, ""] {
            let p = if base.is_empty() {
                name.to_string()
            } else {
                format!("{base}/{name}")
            };
            tries.push(p.clone());
            if !p.ends_with(".tex") {
                tries.push(format!("{p}.tex"));
            }
        }
        tries.into_iter().find(|t| self.root.join(t).is_file())
    }

    fn expand(&mut self, file: &str, depth: usize) -> Result<()> {
        let raw = std::fs::read_to_string(self.root.join(file))?;
        let src = strip_comments(&raw);
        let dir = Path::new(file).parent().map(slash).unwrap_or_default();
        let mut i = 0;
        let mut last = 0;
        loop {
            // the next \input / \include / \subfile, whichever first
            let next = ["input", "include", "subfile"]
                .iter()
                .filter_map(|c| find_command(&src, c, i).map(|(s, e, _)| (s, e)))
                .min_by_key(|x| x.0);
            let Some((start, after)) = next else { break };
            let Some((name, end)) = group_text(&src, after) else {
                i = after;
                continue;
            };
            self.push(&src, last, start, file);
            match self.resolve(name, &dir) {
                Some(rel) if depth < 12 => self.expand(&rel, depth + 1)?,
                _ => self.missing.push(name.trim().to_string()),
            }
            last = end;
            i = end;
        }
        self.push(&src, last, src.len(), file);
        Ok(())
    }

    fn push(&mut self, src: &str, from: usize, to: usize, file: &str) {
        if to <= from {
            return;
        }
        let line0 = src[..from].matches('\n').count() + 1;
        let start = self.text.len();
        self.text.push_str(&src[from..to]);
        self.spans.push(Span {
            start,
            end: self.text.len(),
            file: file.to_string(),
            line0,
        });
    }

    /// File and line of an offset in [`LatexProject::text`].
    pub fn locate(&self, offset: usize) -> SourceRef {
        let Some(sp) = self
            .spans
            .iter()
            .find(|s| offset >= s.start && offset < s.end.max(s.start + 1))
        else {
            return SourceRef {
                file: Some(self.main.clone()),
                ..Default::default()
            };
        };
        let line = sp.line0
            + self.text[sp.start..offset.min(sp.end)]
                .matches('\n')
                .count();
        SourceRef {
            file: Some(sp.file.clone()),
            line: Some(line),
            page: None,
        }
    }

    /// `(preamble, body, body offset)`: around `\begin{document}` … `\end{document}`.
    pub fn split_document(&self) -> (&str, &str, usize) {
        let t = &self.text;
        match t.find("\\begin{document}") {
            Some(b) => {
                let start = b + "\\begin{document}".len();
                let end = t[start..]
                    .find("\\end{document}")
                    .map(|e| start + e)
                    .unwrap_or(t.len());
                (&t[..b], &t[start..end], start)
            }
            None => ("", t.as_str(), 0),
        }
    }

    /// Find a graphics file named by `\includegraphics` (extension optional,
    /// `\graphicspath` folders tried). Returns the path relative to the root.
    pub fn find_graphic(&self, name: &str, graphicspath: &[String]) -> Option<String> {
        let name = name.trim();
        let exts = [
            "", ".pdf", ".png", ".jpg", ".jpeg", ".eps", ".PNG", ".JPG", ".PDF", ".svg",
        ];
        let mut dirs: Vec<String> = vec![String::new()];
        dirs.extend(graphicspath.iter().cloned());
        for d in &dirs {
            for e in exts {
                let rel = format!("{d}{name}{e}");
                let rel = rel.trim_start_matches("./").to_string();
                if self.root.join(&rel).is_file() {
                    return Some(rel);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_inputs_and_locates_lines() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("sec")).unwrap();
        std::fs::write(r.join("paper.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sec/intro}\n% \\input{gone}\nEnd\n\\end{document}\n").unwrap();
        std::fs::write(r.join("sec/intro.tex"), "Line one\n\\include{sec/deep}\n").unwrap();
        std::fs::write(r.join("sec/deep.tex"), "\n\nDeep text\n").unwrap();
        std::fs::write(r.join("other.tex"), "\\section{x}").unwrap();
        let p = LatexProject::open(r).unwrap();
        assert_eq!(p.main, "paper.tex");
        assert!(p.text.contains("Line one"));
        assert!(!p.text.contains("gone"));
        let off = p.text.find("Deep text").unwrap();
        let loc = p.locate(off);
        assert_eq!(loc.file.as_deref(), Some("sec/deep.tex"));
        assert_eq!(loc.line, Some(3));
        let off = p.text.find("End").unwrap();
        assert_eq!(p.locate(off).line, Some(5));
        assert!(p.missing.is_empty());
    }

    #[test]
    fn no_main_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.tex"), "\\section{x}").unwrap();
        assert!(matches!(
            LatexProject::open(dir.path()),
            Err(Error::NoMainTex(_))
        ));
    }
}
