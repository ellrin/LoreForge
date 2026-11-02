//! Paper files on disk.
//!
//! A paper is a folder with `main.tex` and one file per section under
//! `sections/`. Section files hold one sentence per line and a blank line
//! between paragraphs, so Git diffs land on single sentences.

use crate::meta::{tex_text, OutlineSection};
use crate::text::sentences;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Section {
    /// File name under `sections/`, e.g. `introduction.tex`.
    pub file: String,
    pub title: String,
    pub paragraphs: Vec<String>,
    /// Whatever follows `\section{...}` on its line, e.g. `\label{sec1}`;
    /// kept verbatim so references survive a save.
    #[serde(default)]
    pub tail: String,
}

/// Accept plain names only: no separators, no parent references, no hidden files.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
}

/// Section files in reading order: the order of `\input{sections/...}` in
/// main.tex, then any remaining files in `sections/` alphabetically.
pub fn section_files(paper: &Path) -> Vec<String> {
    let mut order = Vec::new();
    if let Ok(main) = std::fs::read_to_string(paper.join("main.tex")) {
        for line in main.lines() {
            let line = line.split('%').next().unwrap_or("");
            for cmd in ["\\input{", "\\include{"] {
                if let Some(rest) = line.split(cmd).nth(1) {
                    if let Some(arg) = rest.split('}').next() {
                        if let Some(name) = arg.strip_prefix("sections/") {
                            let file = if name.ends_with(".tex") { name.to_string() } else { format!("{name}.tex") };
                            if !order.contains(&file) {
                                order.push(file);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut rest: Vec<String> = std::fs::read_dir(paper.join("sections"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".tex") && !order.contains(n))
        .collect();
    rest.sort();
    order.retain(|f| paper.join("sections").join(f).exists());
    order.extend(rest);
    order
}

/// Split a section file into its `\section{}` title and paragraphs.
pub fn parse_section(file: &str, text: &str) -> Section {
    let mut title = String::new();
    let mut tail = String::new();
    let mut body = String::new();
    let mut seen = false;
    for line in text.lines() {
        let t = line.trim_start();
        if !seen && t.starts_with("\\section") {
            if let Some((arg, rest)) = t.find('{').and_then(|i| braced(&t[i..])) {
                title = arg.to_string();
                tail = rest.trim().to_string();
                seen = true;
                continue;
            }
        }
        body.push_str(line);
        body.push('\n');
    }
    if title.is_empty() {
        title = file.trim_end_matches(".tex").replace(['-', '_'], " ");
    }
    Section { file: file.to_string(), title, paragraphs: crate::text::paragraphs(&body), tail }
}

/// `{...}rest` → (inside, rest), honouring nested braces.
fn braced(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((&s[1..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Write back: one sentence per line for prose; blocks with environments or
/// comments are kept exactly as typed.
pub fn render_section(s: &Section) -> String {
    let mut out = format!("\\section{{{}}}{}\n", s.title, s.tail);
    for p in &s.paragraphs {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        out.push('\n');
        // A real comment is an unescaped %; "12.4\%" is ordinary prose.
        let has_comment = p.lines().any(|l| strip_comment(l).len() != l.len());
        let keep = p.contains("\\begin") || has_comment || p.starts_with('\\');
        if keep {
            out.push_str(p);
        } else {
            out.push_str(&sentences(p).join("\n"));
        }
        out.push('\n');
    }
    out
}

/// `\newcommand`, `\renewcommand`, `\DeclareMathOperator` and simple `\def`
/// from LaTeX source, as name -> body, for the math previewer. Argument
/// counts are dropped: `#1` in the body works the same way in KaTeX.
pub fn extract_macros(src: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let src: String = src.lines().map(|l| strip_comment(l)).collect::<Vec<_>>().join("\n");
    let bytes: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '\\' {
            i += 1;
            continue;
        }
        let rest: String = bytes[i..bytes.len().min(i + 24)].iter().collect();
        let (kind, skip) = ["\\newcommand", "\\renewcommand", "\\providecommand", "\\DeclareMathOperator", "\\def"]
            .iter()
            .find(|k| rest.starts_with(**k))
            .map(|k| (*k, k.chars().count()))
            .unwrap_or(("", 0));
        if skip == 0 {
            i += 1;
            continue;
        }
        let mut j = i + skip;
        let starred = bytes.get(j) == Some(&'*');
        if starred {
            j += 1;
        }
        // Name: {\name} or \name
        let name;
        while bytes.get(j).is_some_and(|c| c.is_whitespace()) {
            j += 1;
        }
        if bytes.get(j) == Some(&'{') {
            let (n, k) = group(&bytes, j);
            name = n.trim().to_string();
            j = k;
        } else if bytes.get(j) == Some(&'\\') {
            let start = j;
            j += 1;
            while bytes.get(j).is_some_and(|c| c.is_alphabetic()) {
                j += 1;
            }
            name = bytes[start..j].iter().collect();
        } else {
            i += 1;
            continue;
        }
        // Skip [n] and [default] (newcommand) or #1#2 (def).
        loop {
            while bytes.get(j).is_some_and(|c| c.is_whitespace()) {
                j += 1;
            }
            match bytes.get(j) {
                Some('[') => {
                    while j < bytes.len() && bytes[j] != ']' {
                        j += 1;
                    }
                    j += 1;
                }
                Some('#') if kind == "\\def" => j += 2,
                _ => break,
            }
        }
        if bytes.get(j) == Some(&'{') && name.starts_with('\\') {
            let (body, k) = group(&bytes, j);
            // The starred form puts limits under the operator in display math.
            let body = match (kind, starred) {
                ("\\DeclareMathOperator", true) => format!("\\operatorname*{{{body}}}"),
                ("\\DeclareMathOperator", false) => format!("\\operatorname{{{body}}}"),
                _ => body,
            };
            out.insert(name, body);
            i = k;
        } else {
            i = j.max(i + 1);
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'%' && (i == 0 || b[i - 1] != b'\\') {
            return &line[..i];
        }
    }
    line
}

/// Content of a balanced `{...}` starting at `open`, and the index after it.
fn group(chars: &[char], open: usize) -> (String, usize) {
    let mut depth = 0;
    let mut j = open;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 1, // skip escaped char
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (chars[open + 1..j].iter().collect(), j + 1);
                }
            }
            _ => {}
        }
        j += 1;
    }
    (chars[open + 1..].iter().collect(), chars.len())
}

/// Macros for a paper: main.tex preamble plus any macros*.tex beside it.
pub fn paper_macros(paper: &Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut files = vec![paper.join("main.tex")];
    if let Ok(dir) = std::fs::read_dir(paper) {
        for e in dir.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("macros") && n.ends_with(".tex") {
                files.push(e.path());
            }
        }
    }
    for f in files {
        if let Ok(src) = std::fs::read_to_string(f) {
            out.extend(extract_macros(&src));
        }
    }
    out
}

pub fn load_paper(paper: &Path) -> Vec<Section> {
    section_files(paper)
        .into_iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(paper.join("sections").join(&f)).ok()?;
            Some(parse_section(&f, &text))
        })
        .collect()
}

/// Sections a blank paper starts with: (file stem, heading).
const BLANK_SECTIONS: &[(&str, &str)] = &[
    ("introduction", "Introduction"),
    ("related-work", "Related work"),
    ("method", "Method"),
    ("result", "Result"),
    ("discussion", "Discussion"),
    ("conclusion", "Conclusion"),
];

/// Create `papers/<name>/` with a plain article main.tex, empty sections and
/// an empty refs.bib. Refuses to touch an existing folder.
pub fn create_paper(papers: &Path, name: &str) -> Result<PathBuf, String> {
    create_paper_with(papers, name, None)
}

/// A blank paper; with an outline, its sections instead of the default ones.
pub fn create_paper_with(papers: &Path, name: &str, outline: Option<&[OutlineSection]>) -> Result<PathBuf, String> {
    let name = name.trim();
    if !safe_name(name) {
        return Err("名稱只能用文字、數字、空格、- _ .".into());
    }
    let dir = papers.join(name);
    if dir.exists() {
        return Err(format!("已經有一篇叫 {name} 的論文"));
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dir.join("sections")).map_err(io)?;
    let sections: Vec<Section> = match outline.filter(|o| !o.is_empty()) {
        Some(o) => o
            .iter()
            .enumerate()
            .map(|(i, s)| Section { file: format!("{:02}-{}.tex", i + 1, slug(&s.title)), title: tex_text(&s.title), paragraphs: s.paragraphs(), tail: String::new() })
            .collect(),
        None => BLANK_SECTIONS.iter().map(|(f, t)| Section { file: format!("{f}.tex"), title: (*t).into(), paragraphs: vec![], tail: String::new() }).collect(),
    };
    let inputs: String = sections.iter().map(|s| format!("\\input{{sections/{}}}\n", s.file.trim_end_matches(".tex"))).collect();
    let main = format!(
        "\\documentclass{{article}}\n\\usepackage{{amsmath,amssymb}}\n\n\\title{{{name}}}\n\\author{{}}\n\n\\begin{{document}}\n\\maketitle\n\n\\begin{{abstract}}\n\\end{{abstract}}\n\n{inputs}\n\\bibliographystyle{{plain}}\n\\bibliography{{refs}}\n\\end{{document}}\n"
    );
    write_atomic(&dir.join("main.tex"), &main).map_err(io)?;
    for s in &sections {
        write_atomic(&dir.join("sections").join(&s.file), &render_section(s)).map_err(io)?;
    }
    write_atomic(&dir.join("refs.bib"), "").map_err(io)?;
    Ok(dir)
}

/// Add `\usepackage{x}` for each package main.tex does not load yet, right
/// before `\begin{document}`. Returns the packages it added. A package the
/// class already loads is harmless to load again.
pub fn ensure_packages(paper: &Path, packages: &[String]) -> Result<Vec<String>, String> {
    let path = paper.join("main.tex");
    let main = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let loaded: Vec<String> = main
        .lines()
        .map(strip_comment)
        .filter_map(|l| {
            let i = l.find("\\usepackage")?;
            let rest = &l[i + "\\usepackage".len()..];
            let rest = rest.trim_start();
            let rest = if rest.starts_with('[') { &rest[rest.find(']')? + 1..] } else { rest };
            braced(rest.trim_start()).map(|(names, _)| names.to_string())
        })
        .flat_map(|names| names.split(',').map(|n| n.trim().to_string()).collect::<Vec<_>>())
        .collect();
    let missing: Vec<String> = packages.iter().filter(|p| !loaded.contains(p)).cloned().collect();
    if missing.is_empty() {
        return Ok(missing);
    }
    let at = main.find("\\begin{document}").ok_or("main.tex has no \\begin{document}")?;
    let lines: String = missing.iter().map(|p| format!("\\usepackage{{{p}}}\n")).collect();
    write_atomic(&path, &format!("{}{}{}", &main[..at], lines, &main[at..])).map_err(|e| e.to_string())?;
    Ok(missing)
}

/// Files in a publisher folder that are not copied into a paper.
const TEMPLATE_SKIP: &[&str] = &["sample.tex", "template.toml", "README", "README.md", "user-manual.pdf"];

/// Create `papers/<name>/` from a publisher template (`publishers/<x>/`):
/// the sample's preamble, front matter and back matter stay in main.tex,
/// each `\section` becomes a file in `sections/` (comment lines dropped),
/// and the class, style and support files are copied alongside so the paper
/// compiles on its own.
pub fn create_paper_from_template(papers: &Path, name: &str, template: &Path) -> Result<PathBuf, String> {
    create_paper_from_template_with(papers, name, template, None)
}

/// From a template; with an outline, its sections replace the sample's
/// (the sample's front and back matter stay).
pub fn create_paper_from_template_with(papers: &Path, name: &str, template: &Path, outline: Option<&[OutlineSection]>) -> Result<PathBuf, String> {
    let name = name.trim();
    if !safe_name(name) {
        return Err("名稱只能用文字、數字、空格、- _ .".into());
    }
    let bytes = std::fs::read(template.join("sample.tex")).map_err(|_| "模板裡沒有 sample.tex".to_string())?;
    // Some publishers still ship Latin-1 samples; read those byte for byte.
    let sample = String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
    let mut split = split_sample(&sample, false).ok_or("sample.tex 裡找不到 \\begin{document}")?;
    if let Some(o) = outline.filter(|o| !o.is_empty()) {
        replace_sections(&mut split, o);
    }
    let dir = papers.join(name);
    if dir.exists() {
        return Err(format!("已經有一篇叫 {name} 的論文"));
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(&dir).map_err(io)?;
    copy_support(template, &dir).map_err(io)?;
    // BibTeX only looks next to main.tex: lift styles kept in bst/ (Springer).
    if let Ok(rd) = std::fs::read_dir(template.join("bst")) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "bst") {
                std::fs::copy(e.path(), dir.join(e.file_name())).map_err(io)?;
            }
        }
    }

    write_split(&dir, &split)?;
    Ok(dir)
}

/// One file per section under `sections/`, and main.tex around them.
fn write_split(dir: &Path, split: &Sample) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dir.join("sections")).map_err(io)?;
    // main.tex keeps everything between sections in its place; each section
    // becomes an \\input of its own file.
    let mut doc = String::new();
    let mut i = 0;
    for piece in &split.pieces {
        match piece {
            Piece::Text(t) => doc += t,
            Piece::Section { title, tail, body } => {
                i += 1;
                let file = format!("{:02}-{}.tex", i, slug(title));
                let mut s = parse_section(&file, &format!("\\section{{{title}}}{tail}\n{body}"));
                s.file = file.clone();
                write_atomic(&dir.join("sections").join(&file), &render_section(&s)).map_err(io)?;
                doc += &format!("\\input{{sections/{}}}\n", file.trim_end_matches(".tex"));
            }
        }
    }
    let main = format!("{}\\begin{{document}}{}\\end{{document}}\n", split.preamble, doc.trim_end().to_string() + "\n");
    write_atomic(&dir.join("main.tex"), &main).map_err(io)
}

/// Read a .tex file; some are still Latin-1, read those byte for byte.
fn read_tex(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect()))
}

/// The project's main file: a .tex with both \documentclass and
/// \begin{document} (main.tex or ms.tex first, then the largest).
pub fn find_main_tex(src: &Path) -> Option<PathBuf> {
    // arXiv names the file it compiles in 00README.json
    if let Some(top) = std::fs::read_to_string(src.join("00README.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|j| {
            j["sources"].as_array()?.iter().find(|s| s["usage"] == "toplevel")?["filename"].as_str().map(String::from)
        })
        .filter(|f| !f.contains("..") && !f.starts_with('/'))
    {
        let p = src.join(top);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut found: Vec<(bool, u64, PathBuf)> = Vec::new();
    let mut stack = vec![src.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "tex") {
                let Some(t) = read_tex(&p) else { continue };
                let code: String = t.lines().map(strip_comment).collect::<Vec<_>>().join("\n");
                if document_class(&code).is_some_and(|c| c != "subfiles" && c != "standalone") && code.contains("\\begin{document}") {
                    let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                    let size = t.len() as u64;
                    found.push((matches!(n.as_str(), "main.tex" | "ms.tex" | "paper.tex"), size, p));
                }
            }
        }
    }
    found.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    found.into_iter().next().map(|f| f.2)
}

/// The class in `\documentclass[opts]{class}`.
fn document_class(code: &str) -> Option<&str> {
    let rest = code[code.find("\\documentclass")? + "\\documentclass".len()..].trim_start();
    let rest = if rest.starts_with('[') { &rest[rest.find(']')? + 1..] } else { rest };
    braced(rest.trim_start()).map(|(c, _)| c.trim())
}

/// Replace `\input{x}`, `\include{x}` and `\subfile{x}` with the file's
/// text, recursively; `used` collects the files taken in. Commented-out
/// commands and files outside `root` are left alone.
pub fn flatten(root: &Path, base: &Path, text: &str, used: &mut Vec<PathBuf>, depth: usize) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let code = strip_comment(line);
        let mut rest = line;
        let mut done = 0;
        loop {
            let hit = ["\\input{", "\\include{", "\\subfile{"]
                .iter()
                .filter_map(|c| code[done..].find(c).map(|i| (done + i, *c)))
                .min_by_key(|h| h.0);
            let Some((at, cmd)) = hit else { break };
            let after = &line[at + cmd.len() - 1..];
            let Some((arg, _)) = braced(after) else { break };
            let end = at + cmd.len() + arg.len() + 1;
            let file = resolve_tex(root, base, arg.trim());
            match file.filter(|_| depth < 10).and_then(|f| read_tex(&f).map(|t| (f, t))) {
                Some((f, t)) => {
                    out += &line[line.len() - rest.len()..at];
                    let body = match (t.find("\\begin{document}"), t.rfind("\\end{document}")) {
                        // a subfile: only what is inside its document
                        (Some(b), Some(e)) if b < e => t[b + "\\begin{document}".len()..e].to_string(),
                        _ => t,
                    };
                    used.push(f.clone());
                    let inner = flatten(root, f.parent().unwrap_or(root), &body, used, depth + 1);
                    out += inner.trim_end_matches('\n');
                    rest = &line[end..];
                }
                None => {}
            }
            done = end;
        }
        out += rest;
        out.push('\n');
    }
    out
}

fn resolve_tex(root: &Path, base: &Path, arg: &str) -> Option<PathBuf> {
    if arg.contains("..") || arg.starts_with('/') {
        return None;
    }
    for dir in [base, root] {
        for name in [arg.to_string(), format!("{arg}.tex")] {
            let p = dir.join(&name);
            if p.is_file() && p.extension().is_some_and(|x| x == "tex") {
                return Some(p);
            }
        }
    }
    None
}

/// Build output, not sources.
pub const BUILD_EXT: &[&str] = &["aux", "log", "out", "blg", "fls", "fdb_latexmk", "synctex.gz", "toc", "lof", "lot", "nav", "snm", "vrb"];

/// Create `papers/<name>/` from an existing LaTeX project in `src`: the main
/// file and everything it inputs become main.tex plus one file per section
/// (comments kept), and every other file (figures, styles, .bib) is copied.
/// A .bbl next to the main file comes along as main.bbl.
pub fn import_paper(papers: &Path, name: &str, src: &Path) -> Result<PathBuf, String> {
    let name = name.trim();
    if !safe_name(name) {
        return Err("名稱只能用文字、數字、空格、- _ .".into());
    }
    let main_path = find_main_tex(src).ok_or("找不到主檔（有 \\documentclass 與 \\begin{document} 的 .tex）")?;
    let base = main_path.parent().unwrap_or(src).to_path_buf();
    let mut used = vec![main_path.clone()];
    let text = flatten(&base, &base, &read_tex(&main_path).unwrap_or_default(), &mut used, 0);
    let split = split_sample(&text, true).ok_or("主檔裡找不到 \\begin{document}")?;
    let dir = papers.join(name);
    if dir.exists() {
        return Err(format!("已經有一篇叫 {name} 的論文"));
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(&dir).map_err(io)?;
    copy_project(&base, &dir, &used, false).map_err(io)?;
    let bbl = main_path.with_extension("bbl");
    if bbl.is_file() {
        std::fs::copy(&bbl, dir.join("main.bbl")).map_err(io)?;
    }
    write_split(&dir, &split)?;
    Ok(dir)
}

/// Copy a project's files except the .tex files merged into main.tex, build
/// output, and any .tex under sections/ (that folder belongs to the app).
fn copy_project(from: &Path, to: &Path, used: &[PathBuf], in_sections: bool) -> std::io::Result<()> {
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let p = e.path();
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.') || used.contains(&p) || BUILD_EXT.iter().any(|x| n.ends_with(&format!(".{x}"))) {
            continue;
        }
        if e.file_type()?.is_dir() {
            let d = to.join(&n);
            std::fs::create_dir_all(&d)?;
            copy_project(&p, &d, used, in_sections || n == "sections")?;
        } else if !(in_sections && n.ends_with(".tex")) && !n.ends_with(".bbl") {
            std::fs::copy(&p, to.join(&n))?;
        }
    }
    Ok(())
}

struct Sample {
    preamble: String,
    /// The document in order: text that stays in main.tex, and sections.
    pieces: Vec<Piece>,
}

enum Piece {
    Text(String),
    /// Title, rest of the \\section line, body (comment lines dropped for templates).
    Section { title: String, tail: String, body: String },
}

/// Split a single-file document into preamble and pieces. A section runs to
/// the next \\section or to back matter (bibliography, \\appendix, ...);
/// sections after back matter, such as appendices, are sections too.
fn split_sample(src: &str, keep_comments: bool) -> Option<Sample> {
    let begin = src.find("\\begin{document}")?;
    let preamble = src[..begin].to_string();
    let after = &src[begin + "\\begin{document}".len()..];
    let end = after.rfind("\\end{document}").unwrap_or(after.len());
    let doc = &after[..end];

    let back_marks = ["\\appendix", "\\appendices", "\\begin{appendices}", "\\end{appendices}", "\\bibliography", "\\printbibliography", "\\begin{thebibliography}", "\\backmatter", "\\bibliographystyle", "\\bmhead", "\\begin{IEEEbiography", "\\EOD"];
    let mut pieces: Vec<Piece> = vec![Piece::Text(String::new())];
    let mut in_bib = false;
    for line in doc.lines() {
        let code = strip_comment(line).trim();
        if in_bib {
            if code.contains("\\end{thebibliography}") {
                in_bib = false;
            }
        } else if code.starts_with("\\section") && !code.starts_with("\\section*") {
            if let Some((t, rest)) = code.find('{').and_then(|i| braced(&code[i..])) {
                pieces.push(Piece::Section { title: t.to_string(), tail: rest.trim().to_string(), body: String::new() });
                continue;
            }
        } else if back_marks.iter().any(|m| code.starts_with(m)) {
            in_bib = code.starts_with("\\begin{thebibliography}");
            if let Some(Piece::Section { body, .. }) = pieces.last_mut() {
                // A group opened at the end of the section for the back
                // matter (CVPR's `{\small \bibliography...}`) belongs to it.
                let carry = open_tail(body);
                let moved = body.split_off(carry);
                pieces.push(Piece::Text(moved));
            }
        }
        match pieces.last_mut() {
            Some(Piece::Section { body, .. }) => {
                // Drop pure comment lines: samples are full of instructions.
                if !keep_comments && line.trim_start().starts_with('%') {
                    continue;
                }
                body.push_str(line);
                body.push('\n');
            }
            Some(Piece::Text(t)) => {
                t.push_str(line);
                t.push('\n');
            }
            None => {}
        }
    }
    Some(Sample { preamble, pieces })
}

/// Where the trailing lines of `body` that leave a `{` group open begin:
/// the end of the last line at which every group was closed.
fn open_tail(body: &str) -> usize {
    let mut depth = 0i32;
    let mut cut = 0;
    let mut pos = 0;
    for line in body.split_inclusive('\n') {
        let b = strip_comment(line).as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 1,
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        pos += line.len();
        if depth <= 0 {
            depth = 0;
            cut = pos;
        }
    }
    cut
}

/// Put the outline's sections where the sample's body sections were; the
/// sample's later sections (appendices) go too, its back matter stays.
fn replace_sections(split: &mut Sample, outline: &[OutlineSection]) {
    let ours = outline.iter().map(|s| Piece::Section { title: tex_text(&s.title), tail: String::new(), body: s.paragraphs().join("\n\n") + "\n" });
    let pieces = std::mem::take(&mut split.pieces);
    let Some(first) = pieces.iter().position(|p| matches!(p, Piece::Section { .. })) else {
        split.pieces = pieces;
        split.pieces.extend(ours);
        return;
    };
    let mut end = first;
    while end < pieces.len()
        && match &pieces[end] {
            Piece::Section { .. } => true,
            Piece::Text(t) => t.trim().is_empty(),
        }
    {
        end += 1;
    }
    let mut out = Vec::new();
    let mut rest = pieces.into_iter();
    out.extend(rest.by_ref().take(first));
    out.extend(ours);
    out.extend(rest.skip(end - first).filter(|p| matches!(p, Piece::Text(_))));
    split.pieces = out;
}

fn slug(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "section".into() } else { s.chars().take(40).collect() }
}

fn copy_support(from: &Path, to: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if n.starts_with('.') || TEMPLATE_SKIP.contains(&n.as_ref()) {
            continue;
        }
        let dst = to.join(&name);
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_support(&e.path(), &dst)?;
        } else {
            std::fs::copy(e.path(), dst)?;
        }
    }
    Ok(())
}

/// Atomic write (temp file + rename) so a crash never leaves half a file.
pub fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp: PathBuf = path.with_extension("tmp~");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_section_one_sentence_per_line() {
        let text = "\\section{Result}\n\nCompared with A, B wins by 12.4\\%. It is fast.\n\n\\begin{equation}\nx = 1. y = 2.\n\\end{equation}\n";
        let s = parse_section("result.tex", text);
        assert_eq!(s.title, "Result");
        assert_eq!(s.paragraphs.len(), 2);
        let out = render_section(&s);
        assert!(out.contains("Compared with A, B wins by 12.4\\%.\nIt is fast.\n"), "{out}");
        assert!(out.contains("x = 1. y = 2."), "environments kept verbatim");
        // Saving again changes nothing.
        assert_eq!(render_section(&parse_section("result.tex", &out)), out);
    }

    #[test]
    fn creates_a_blank_paper_that_loads() {
        let tmp = std::env::temp_dir().join(format!("pc-blank-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dir = create_paper(&tmp, "我的 論文-1").unwrap();
        let secs = load_paper(&dir);
        assert_eq!(secs.len(), BLANK_SECTIONS.len());
        assert_eq!(secs[0].title, "Introduction");
        assert!(secs.iter().all(|s| s.paragraphs.is_empty()));
        assert!(create_paper(&tmp, "我的 論文-1").is_err(), "never overwrites");
        assert!(create_paper(&tmp, "../escape").is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn section_title_with_label_round_trips() {
        let s = parse_section("a.tex", "\\section{Results for $\\mathbb{R}^{n}$}\\label{sec:res}\n\nOne. Two.\n");
        assert_eq!(s.title, "Results for $\\mathbb{R}^{n}$");
        assert_eq!(s.tail, "\\label{sec:res}");
        let out = render_section(&s);
        assert!(out.starts_with("\\section{Results for $\\mathbb{R}^{n}$}\\label{sec:res}\n"), "{out}");
    }

    #[test]
    fn appendices_become_sections_in_place() {
        let src = "\\documentclass{article}\n\\begin{document}\n\\maketitle\n\\section{Intro}\nA.\n\\appendices\n\\section{Proof}\\label{app:a}\nB.\n\\bibliographystyle{plain}\n\\begin{thebibliography}{1}\n\\bibitem{x} X.\n\\end{thebibliography}\n\\section*{Extra}\n\\section{Late}\nC.\n\\end{document}\n";
        let root = std::env::temp_dir().join(format!("pc-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.tex"), src).unwrap();
        let dir = import_paper(&root.join("papers"), "P", &root.join("src")).unwrap();
        let secs = load_paper(&dir);
        assert_eq!(secs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Intro", "Proof", "Late"]);
        assert_eq!(secs[1].tail, "\\label{app:a}");
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        let order: Vec<usize> = ["\\maketitle", "sections/01-intro", "\\appendices", "sections/02-proof", "\\bibitem{x}", "\\section*{Extra}", "sections/03-late", "\\end{document}"]
            .iter()
            .map(|m| main.find(m).unwrap_or_else(|| panic!("{m} missing in {main}")))
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{main}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn imports_a_multi_file_project() {
        let root = std::env::temp_dir().join(format!("pc-import-{}", std::process::id()));
        let src = root.join("src");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(src.join("figs")).unwrap();
        std::fs::create_dir_all(src.join("sections")).unwrap();
        std::fs::write(src.join("ms.tex"), "\\documentclass{article}\n\\begin{document}\n\\title{T}\n\\section{Intro}\n\\input{intro}\n\\section{Method} \\label{sec:m}\n\\input{sections/method}\n% \\input{unused}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
        std::fs::write(src.join("intro.tex"), "First % a note\nparagraph.\n\nSecond.\n").unwrap();
        std::fs::write(src.join("sections/method.tex"), "We \\input{eq} here.\n").unwrap();
        std::fs::write(src.join("sections/eq.tex"), "$x$").unwrap();
        std::fs::write(src.join("sections/old.tex"), "unused").unwrap();
        std::fs::write(src.join("unused.tex"), "x").unwrap();
        std::fs::write(src.join("ms.bbl"), "\\begin{thebibliography}{1}\\end{thebibliography}").unwrap();
        std::fs::write(src.join("figs/a.png"), "png").unwrap();
        std::fs::write(src.join("ms.aux"), "").unwrap();
        // a decoy with a bigger body is ignored when arXiv names the top-level file
        std::fs::write(src.join("old.tex"), format!("\\documentclass{{article}}\\begin{{document}}{}\\end{{document}}", "x".repeat(5000))).unwrap();
        std::fs::write(src.join("00README.json"), r#"{"sources":[{"usage":"toplevel","filename":"ms.tex"}]}"#).unwrap();
        let dir = import_paper(&root.join("papers"), "P", &src).unwrap();
        let secs = load_paper(&dir);
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].paragraphs, ["First % a note\nparagraph.", "Second."], "comments are kept");
        assert_eq!(secs[1].tail, "\\label{sec:m}");
        assert_eq!(secs[1].paragraphs, ["We $x$ here.\n% \\input{unused}"], "nested input relative to its file");
        assert!(dir.join("figs/a.png").is_file() && dir.join("main.bbl").is_file() && dir.join("unused.tex").is_file());
        assert!(!dir.join("intro.tex").exists() && !dir.join("ms.aux").exists() && !dir.join("sections/old.tex").exists());
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.contains("\\bibliography{refs}"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_a_paper_from_a_single_file_template() {
        let root = std::env::temp_dir().join(format!("pc-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tpl = root.join("publishers/demo");
        std::fs::create_dir_all(tpl.join("bst")).unwrap();
        std::fs::write(tpl.join("demo.cls"), "% class").unwrap();
        std::fs::write(tpl.join("bst/x.bst"), "").unwrap();
        std::fs::write(tpl.join("README"), "skip me").unwrap();
        std::fs::write(
            tpl.join("sample.tex"),
            "\\documentclass{demo}\n\\begin{document}\n\\title{T}\\maketitle\n\\begin{abstract}A.\\end{abstract}\n\
% instructions\n\\section{Introduction}\\label{sec1}\n% delete me\nFirst idea. Second idea.\n\n\\section{Methods}\nWe did it.\n\
\\bibliography{refs}\n\\end{document}\n",
        )
        .unwrap();
        let dir = create_paper_from_template(&root.join("papers"), "My paper", &tpl).unwrap();
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.starts_with("\\documentclass{demo}"));
        assert!(main.contains("\\begin{abstract}A.\\end{abstract}"));
        assert!(main.contains("\\input{sections/01-introduction}\n\\input{sections/02-methods}"), "{main}");
        assert!(main.contains("\\bibliography{refs}"));
        assert!(dir.join("demo.cls").exists() && dir.join("bst/x.bst").exists());
        assert!(!dir.join("README").exists() && !dir.join("sample.tex").exists());
        let secs = load_paper(&dir);
        assert_eq!(secs.len(), 2);
        assert_eq!((secs[0].title.as_str(), secs[0].tail.as_str()), ("Introduction", "\\label{sec1}"));
        assert_eq!(secs[0].paragraphs, vec!["First idea.\nSecond idea.".to_string()], "one sentence per line, comments dropped");
        assert!(create_paper_from_template(&root.join("papers"), "My paper", &tpl).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adds_only_missing_packages() {
        let dir = std::env::temp_dir().join(format!("pc-pkg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\usepackage[T1]{fontenc}\n\\usepackage{amsmath, booktabs}\n% \\usepackage{siunitx}\n\\begin{document}\nx\n\\end{document}\n").unwrap();
        let pk = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ensure_packages(&dir, &pk(&["booktabs", "siunitx", "multirow"])).unwrap(), pk(&["siunitx", "multirow"]));
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.contains("\\usepackage{siunitx}\n\\usepackage{multirow}\n\\begin{document}"));
        assert!(ensure_packages(&dir, &pk(&["booktabs", "siunitx"])).unwrap().is_empty(), "second call adds nothing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracts_macros() {
        let src = r"\newcommand{\R}{\mathbb{R}} % reals
\newcommand\norm[1]{\left\lVert #1 \right\rVert}
\renewcommand*{\vec}[1]{\mathbf{#1}}
\DeclareMathOperator*{\argmax}{arg\,max}
\def\eps{\varepsilon}
% \newcommand{\hidden}{x}";
        let m = extract_macros(src);
        assert_eq!(m["\\R"], "\\mathbb{R}");
        assert_eq!(m["\\norm"], "\\left\\lVert #1 \\right\\rVert");
        assert_eq!(m["\\vec"], "\\mathbf{#1}");
        assert_eq!(m["\\argmax"], "\\operatorname*{arg\\,max}");
        assert_eq!(m["\\eps"], "\\varepsilon");
        assert!(!m.contains_key("\\hidden"));
    }

    #[test]
    fn rejects_path_tricks() {
        assert!(safe_name("introduction.tex"));
        assert!(!safe_name("../secrets"));
        assert!(!safe_name("a/b.tex"));
        assert!(!safe_name(".git"));
    }

    const SAMPLE: &str = "\\documentclass{article}\n\\begin{document}\n\\title{T}\n\\maketitle\n\\section{Intro}\nSample intro.\n\\section{Equations}\nSample math.\n{\n  \\small\n  \\bibliographystyle{plain}\n  \\bibliography{refs}\n}\n\\appendix\n\\section{Proofs}\nSample proof.\n\\end{document}\n";

    #[test]
    fn a_group_opened_for_the_bibliography_stays_with_it() {
        let s = split_sample(SAMPLE, false).unwrap();
        let bodies: Vec<&str> = s.pieces.iter().filter_map(|p| match p { Piece::Section { body, .. } => Some(body.as_str()), _ => None }).collect();
        assert_eq!(bodies[1], "Sample math.\n");
        let texts: String = s.pieces.iter().filter_map(|p| match p { Piece::Text(t) => Some(t.as_str()), _ => None }).collect();
        assert!(texts.contains("{\n  \\small\n  \\bibliographystyle{plain}"), "{texts}");
    }

    #[test]
    fn an_outline_replaces_the_sample_sections() {
        let root = std::env::temp_dir().join(format!("pc-outline-{}", std::process::id()));
        let tpl = root.join("tpl");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("sample.tex"), SAMPLE).unwrap();
        let outline = vec![
            OutlineSection { title: "Introduction".into(), intent: "Why it matters.".into(), subsections: vec![] },
            OutlineSection { title: "Methods & data".into(), intent: "How.".into(), subsections: vec![crate::meta::OutlineSub { title: "Cohort".into(), intent: "Who.".into() }] },
        ];
        let dir = create_paper_from_template_with(&root.join("papers"), "P", &tpl, Some(&outline)).unwrap();
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.contains("\\maketitle\n\\input{sections/01-introduction}\n\\input{sections/02-methods-data}\n{\n  \\small"), "{main}");
        assert!(main.contains("\\appendix") && !main.contains("Proofs") && !main.contains("Sample"));
        let s = load_paper(&dir);
        assert_eq!(s.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["Introduction", "Methods \\& data"]);
        assert_eq!(s[1].paragraphs, ["How.", "\\subsection{Cohort}", "Who."]);
        let blank = create_paper_with(&root.join("papers"), "B", Some(&outline)).unwrap();
        assert_eq!(load_paper(&blank).len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
