//! A document as one comment-free text in reading order, with a map back to
//! (file, line, column). `\input` / `\include` are followed from the root
//! file, so a PaperComposer paper (`main.tex` + `sections/*.tex`) and a
//! single-file paper look the same to the scanner.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct LineInfo {
    start: usize,
    file: usize,
    line: u32,
}

#[derive(Debug, Clone)]
pub struct Document {
    /// File names relative to the document root (`main.tex`, `sections/method.tex`).
    pub files: Vec<String>,
    /// Flattened text, comments removed, line structure kept.
    pub text: String,
    lines: Vec<LineInfo>,
    /// Paragraph index (0-based, blank-line separated) of every source line, per file.
    paras: Vec<Vec<u32>>,
    /// Range of the body (after `\begin{document}`, before `\end{document}`).
    pub body: std::ops::Range<usize>,
}

/// A location the UI can jump to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Loc {
    pub file: String,
    /// 1-based.
    pub line: u32,
    /// 1-based, in characters.
    pub col: u32,
    /// Paragraph (0-based) in the file, blank-line separated.
    pub para: u32,
    /// Sentence number (1-based) in the file, headings and captions included.
    pub sentence: u32,
    /// Sentence number (1-based) within the paragraph.
    pub para_sentence: u32,
    /// body | caption | table | heading | abstract | equation
    pub context: String,
    /// Offset in reading order (for sorting).
    pub ord: usize,
    /// The source line, trimmed and shortened.
    pub snippet: String,
}

/// Remove an unescaped `%` comment from one line.
pub fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'%' => return &line[..i],
            _ => i += 1,
        }
    }
    line
}

struct Builder<'a> {
    root: Option<&'a Path>,
    doc: Document,
    seen: Vec<PathBuf>,
}

impl Document {
    /// One in-memory file (tests, editors).
    pub fn from_str(name: &str, text: &str) -> Self {
        Self::from_files(&[(name.to_string(), text.to_string())])
    }

    /// Several in-memory files: the first is the root; `\input{x}` pulls in
    /// the file named `x` or `x.tex` from the list.
    pub fn from_files(files: &[(String, String)]) -> Self {
        let mut b = Builder::new(None);
        if let Some((name, text)) = files.first() {
            b.add(name, text, 0, &mut |n: &str| {
                files.iter().find(|(f, _)| f == n || *f == format!("{n}.tex")).map(|(f, t)| (f.clone(), t.clone()))
            });
        }
        b.finish()
    }

    /// Load from disk. `path` is a `.tex` file or a folder with `main.tex`
    /// (or exactly one `.tex` holding `\begin{document}`).
    pub fn load(path: &Path) -> Result<Self, String> {
        let main = if path.is_dir() { find_root(path).ok_or_else(|| format!("{}: 找不到 main.tex", path.display()))? } else { path.to_path_buf() };
        let root = main.parent().unwrap_or(Path::new(".")).to_path_buf();
        let text = std::fs::read_to_string(&main).map_err(|e| format!("{}: {e}", main.display()))?;
        let name = main.file_name().and_then(|n| n.to_str()).unwrap_or("main.tex").to_string();
        let mut b = Builder::new(Some(&root));
        b.seen.push(main.clone());
        let root2 = root.clone();
        b.add(&name, &text, 0, &mut |n: &str| {
            let mut p = root2.join(n);
            if p.extension().is_none() {
                p.set_extension("tex");
            }
            let rel = p.strip_prefix(&root2).ok()?.to_string_lossy().replace('\\', "/");
            if rel.split('/').any(|s| s == "..") {
                return None;
            }
            std::fs::read_to_string(&p).ok().map(|t| (rel, t))
        });
        Ok(b.finish())
    }

    /// (file index, 1-based line, 1-based column) of a text offset.
    pub fn position(&self, off: usize) -> (usize, u32, u32) {
        let i = match self.lines.binary_search_by(|l| l.start.cmp(&off)) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let Some(l) = self.lines.get(i) else { return (0, 1, 1) };
        let col = self.text.get(l.start..off.min(self.text.len())).map(|s| s.chars().count()).unwrap_or(0) as u32 + 1;
        (l.file, l.line, col)
    }

    pub fn para_of(&self, file: usize, line: u32) -> u32 {
        self.paras.get(file).and_then(|p| p.get(line.saturating_sub(1) as usize)).copied().unwrap_or(0)
    }

    /// The flattened line containing `off`, trimmed and shortened around it.
    pub fn snippet(&self, off: usize) -> String {
        let off = off.min(self.text.len());
        let start = self.text[..off].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = self.text[off..].find('\n').map(|i| off + i).unwrap_or(self.text.len());
        let line = &self.text[start..end];
        let rel = off - start;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let ci = chars.iter().position(|(b, _)| *b >= rel).unwrap_or(chars.len());
        let from = ci.saturating_sub(50);
        let to = (ci + 70).min(chars.len());
        let s: String = chars[from..to].iter().map(|(_, c)| c).collect();
        let mut s = s.trim().to_string();
        if from > 0 {
            s = format!("…{s}");
        }
        if to < chars.len() {
            s.push('…');
        }
        s
    }

    pub fn file_of(&self, off: usize) -> usize {
        self.position(off).0
    }
}

impl<'a> Builder<'a> {
    fn new(root: Option<&'a Path>) -> Self {
        Builder { root, doc: Document { files: Vec::new(), text: String::new(), lines: Vec::new(), paras: Vec::new(), body: 0..0 }, seen: Vec::new() }
    }

    fn add(&mut self, name: &str, text: &str, depth: usize, resolve: &mut dyn FnMut(&str) -> Option<(String, String)>) {
        let fi = self.doc.files.len();
        self.doc.files.push(name.to_string());
        // paragraphs as pc-core counts them: blank-line separated, the
        // file's leading \section line not counted
        let mut paras = Vec::new();
        let mut para = 0u32;
        let (mut prev_blank, mut content_seen, mut first) = (false, false, true);
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw);
            let blank = raw.trim().is_empty();
            if !blank {
                if prev_blank && content_seen {
                    para += 1;
                }
                if !(first && raw.trim_start().starts_with("\\section")) {
                    content_seen = true;
                }
                first = false;
            }
            prev_blank = blank;
            paras.push(para);
            self.doc.lines.push(LineInfo { start: self.doc.text.len(), file: fi, line: n as u32 + 1 });
            // \input{x} / \include{x}: pull the file in after this line
            let inc = include_arg(line);
            match &inc {
                Some((cmd_start, cmd_end, _)) => {
                    self.doc.text.push_str(&line[..*cmd_start]);
                    self.doc.text.push_str(&" ".repeat(cmd_end - cmd_start));
                    self.doc.text.push_str(&line[*cmd_end..]);
                }
                None => self.doc.text.push_str(line),
            }
            self.doc.text.push('\n');
            if let Some((_, _, arg)) = inc
                && depth < 8 {
                    // a generated notation.tex is not part of the text: its
                    // macros come from the registry itself
                    if let Some((rel, body)) = resolve(&arg).filter(|(_, b)| !crate::macros::is_generated(b) && !crate::pending::is_generated(b)) {
                        let key = self.root.map(|r| r.join(&rel)).unwrap_or_else(|| PathBuf::from(&rel));
                        if !self.seen.contains(&key) {
                            self.seen.push(key);
                            self.add(&rel, &body, depth + 1, resolve);
                        }
                    }
                }
        }
        // keep the per-file paragraph table at the file's index
        if self.doc.paras.len() <= fi {
            self.doc.paras.resize(fi + 1, Vec::new());
        }
        self.doc.paras[fi] = paras;
    }

    fn finish(mut self) -> Document {
        let t = &self.doc.text;
        let start = t.find("\\begin{document}").map(|i| i + "\\begin{document}".len()).unwrap_or(0);
        let end = t.find("\\end{document}").unwrap_or(t.len());
        self.doc.body = start..end.max(start);
        self.doc
    }
}

/// `\input{x}` or `\include{x}` on a line: (start, end, argument).
fn include_arg(line: &str) -> Option<(usize, usize, String)> {
    for cmd in ["\\input", "\\include"] {
        let mut from = 0;
        while let Some(i) = line[from..].find(cmd) {
            let s = from + i;
            let rest = &line[s + cmd.len()..];
            let after = rest.trim_start();
            if after.starts_with('{') && !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
                let open = s + cmd.len() + (rest.len() - after.len());
                if let Some(close) = line[open..].find('}') {
                    let arg = line[open + 1..open + close].trim().to_string();
                    return Some((s, open + close + 1, arg));
                }
            }
            from = s + cmd.len();
        }
    }
    None
}

fn find_root(dir: &Path) -> Option<PathBuf> {
    let main = dir.join("main.tex");
    if main.exists() {
        return Some(main);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tex"))
        .filter(|p| std::fs::read_to_string(p).map(|t| t.contains("\\begin{document}")).unwrap_or(false))
        .collect();
    found.sort();
    found.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_inputs_and_maps_lines_back() {
        let files = vec![
            ("main.tex".to_string(), "\\begin{document}\n\\input{sections/a}\nEnd. % note\n\\end{document}\n".to_string()),
            ("sections/a.tex".to_string(), "First line.\n\nSecond $x$ here.\n".to_string()),
        ];
        let d = Document::from_files(&files);
        assert_eq!(d.files, vec!["main.tex", "sections/a.tex"]);
        let off = d.text.find("$x$").unwrap();
        let (f, line, col) = d.position(off);
        assert_eq!((d.files[f].as_str(), line, col), ("sections/a.tex", 3, 8));
        assert_eq!(d.para_of(f, line), 1);
        let d2 = Document::from_str("s.tex", "\\section{A}\\label{a}\n\nOne.\n\nTwo.\n");
        assert_eq!((d2.para_of(0, 3), d2.para_of(0, 5)), (0, 1));
        assert!(!d.text.contains("note"));
        let end = d.text.find("End.").unwrap();
        assert_eq!(d.position(end), (0, 3, 1));
    }

    #[test]
    fn escaped_percent_is_text() {
        assert_eq!(strip_comment("12.4\\% of cases % c"), "12.4\\% of cases ");
    }
}
