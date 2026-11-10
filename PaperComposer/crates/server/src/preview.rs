//! What the editor needs to show a paper the way LaTeX prints it: the
//! numbers of labels and citations, and figures as images.
//!
//! Numbers come from the .aux files of the last export (exactly what the PDF
//! shows); citations fall back to the order of `\bibitem`s in the paper's
//! .bbl or `thebibliography`.

use crate::texmeasure::tool;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct Cite {
    /// The printed label: "3", or for author–year styles the number natbib
    /// keeps alongside.
    pub n: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
}

#[derive(Serialize, Default, Debug)]
pub struct Numbers {
    pub labels: BTreeMap<String, String>,
    pub cites: BTreeMap<String, Cite>,
    /// True when they come from a compile, not from guessing.
    pub compiled: bool,
}

/// `{...}` at the start of `s` (after spaces): (inside, rest).
fn group(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[1..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Printed text of an .aux value: `\mbox {II-A}` → `II-A`, `Smith et~al.` → `Smith et al.`.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphabetic() || n == '@' {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
            }
            '{' | '}' => {}
            '~' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Read `\newlabel` and `\bibcite` lines.
pub fn parse_aux(aux: &str, n: &mut Numbers) {
    for line in aux.lines() {
        if let Some(rest) = line.strip_prefix("\\newlabel") {
            let Some((key, rest)) = group(rest) else { continue };
            if key.ends_with("@cref") {
                continue;
            }
            let Some((val, _)) = group(rest) else { continue };
            if let Some((num, _)) = group(val) {
                n.labels.insert(key.to_string(), plain(num));
            }
        } else if let Some(rest) = line.strip_prefix("\\bibcite") {
            let Some((key, rest)) = group(rest) else { continue };
            let Some((val, _)) = group(rest) else { continue };
            // natbib: {{num}{year}{{short authors}}{{long authors}}}
            let cite = match group(val) {
                Some((num, more)) => {
                    let (year, more) = group(more).unwrap_or(("", ""));
                    let author = group(more).map(|(a, _)| plain(a)).filter(|a| !a.is_empty());
                    Cite { n: plain(num), author, year: Some(plain(year)).filter(|y| !y.is_empty()) }
                }
                None => Cite { n: plain(val), ..Default::default() },
            };
            n.cites.insert(key.to_string(), cite);
        }
    }
}

/// Citations from `\bibitem[opt]{key}` in reading order.
pub fn parse_bibitems(src: &str, n: &mut Numbers) {
    let mut i = 0;
    for part in src.split("\\bibitem").skip(1) {
        let part = part.trim_start();
        let (opt, rest) = match part.strip_prefix('[') {
            Some(r) => {
                // balanced [...]: natbib labels hold braces, not brackets
                let mut depth = 0;
                let end = r.char_indices().find(|&(_, c)| {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        ']' if depth == 0 => return true,
                        _ => {}
                    }
                    false
                });
                match end {
                    Some((e, _)) => (Some(&r[..e]), &r[e + 1..]),
                    None => continue,
                }
            }
            None => (None, part),
        };
        let Some((key, _)) = group(rest) else { continue };
        i += 1;
        let mut c = Cite { n: i.to_string(), ..Default::default() };
        // natbib: "Smith et~al.(2020)Smith, Jones, and Lee"
        if let Some(o) = opt.map(plain) {
            match (o.find('('), o.find(')')) {
                (Some(a), Some(b)) if a < b => {
                    c.author = Some(o[..a].trim().to_string());
                    c.year = Some(o[a + 1..b].trim().to_string());
                }
                _ => c.n = o,
            }
        }
        n.cites.entry(key.trim().to_string()).or_insert(c);
    }
}

pub fn numbers(paper: &Path, build: &Path) -> Numbers {
    let mut n = Numbers::default();
    let mut stack = vec![build.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "aux") {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    parse_aux(&t, &mut n);
                }
            }
        }
    }
    n.compiled = !n.labels.is_empty() || !n.cites.is_empty();
    if n.cites.is_empty() {
        for src in own_sources(paper, build) {
            if let Ok(t) = std::fs::read_to_string(src) {
                parse_bibitems(&t, &mut n);
                if !n.cites.is_empty() {
                    break;
                }
            }
        }
    }
    n
}

/// Where a printed bibliography may be read from: the last compile's .bbl,
/// the paper's own .bbl, then its main file (which may hold the bibliography).
fn own_sources(paper: &Path, build: &Path) -> Vec<PathBuf> {
    let main = pc_core::open::main_file(paper);
    let bbl = main.with_extension("bbl");
    vec![build.join("main.bbl"), paper.join("main.bbl"), bbl, main]
}

/// main.tex around the sections: what comes before the first section
/// (title, authors, abstract) and what comes after (appendix marks,
/// acknowledgments, bibliography commands), in order.
pub fn front_back(main: &str) -> (String, String) {
    front_back_in(main, false)
}

/// `front_back` for a paper of either kind. In a paper opened as it is, a
/// section is a `\section`, `\input` or `\include` line of its main file.
pub fn front_back_in(main: &str, custom: bool) -> (String, String) {
    let body = match main.find("\\begin{document}") {
        Some(i) => &main[i + "\\begin{document}".len()..],
        None => return (String::new(), String::new()),
    };
    let body = &body[..body.rfind("\\end{document}").unwrap_or(body.len())];
    let (mut front, mut back, mut seen) = (String::new(), String::new(), false);
    for line in body.lines() {
        let code = line.split('%').next().unwrap_or("");
        if code.contains("\\input{sections/") || (custom && ["\\section", "\\input{", "\\include{"].iter().any(|k| code.contains(k))) {
            seen = true;
            continue;
        }
        let t = if seen { &mut back } else { &mut front };
        t.push_str(line);
        t.push('\n');
    }
    // the title block may sit in the preamble
    let pre = &main[..main.find("\\begin{document}").unwrap_or(0)];
    let meta: String = ["\\title", "\\author", "\\affil", "\\address", "\\email", "\\institute", "\\IEEEauthorblockN", "\\IEEEauthorblockA"]
        .iter()
        .flat_map(|cmd| command_blocks(pre, cmd))
        .collect::<Vec<_>>()
        .join("\n");
    (if meta.is_empty() { front } else { format!("{meta}\n{front}") }, back)
}

/// Every `\\cmd*[..]{...}` (with its braces) in `src`, comments ignored.
fn command_blocks(src: &str, cmd: &str) -> Vec<String> {
    let code: String = src.lines().map(|l| strip_comment(l)).collect::<Vec<_>>().join("\n");
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = code[at..].find(cmd) {
        let start = at + i;
        let mut j = start + cmd.len();
        if code[j..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            at = j;
            continue;
        }
        let rest = &code[j..];
        let mut k = rest.len() - rest.trim_start_matches(['*', ' ']).len();
        if rest[k..].starts_with('[') {
            k += rest[k..].find(']').map(|e| e + 1).unwrap_or(0);
        }
        match group(&rest[k..]) {
            Some((_, after)) => {
                j += rest.len() - after.len();
                out.push(code[start..j].to_string());
                at = j;
            }
            None => at = j,
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

#[derive(Serialize, Debug, PartialEq)]
pub struct BibEntry {
    pub key: String,
    /// natbib's [label], when given
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub text: String,
}

/// The entries of a thebibliography (a .bbl, or written in main.tex).
pub fn bib_entries(src: &str) -> Vec<BibEntry> {
    let Some(i) = src.find("\\begin{thebibliography}") else { return vec![] };
    let body = &src[i..];
    let body = &body[..body.find("\\end{thebibliography}").unwrap_or(body.len())];
    let mut out = Vec::new();
    for part in body.split("\\bibitem").skip(1) {
        let part = part.trim_start();
        let (label, rest) = match part.strip_prefix('[') {
            Some(r) => {
                let mut depth = 0;
                let end = r.char_indices().find(|&(_, c)| {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        ']' if depth == 0 => return true,
                        _ => {}
                    }
                    false
                });
                match end {
                    Some((e, _)) => (Some(r[..e].to_string()), &r[e + 1..]),
                    None => continue,
                }
            }
            None => (None, part),
        };
        let Some((key, text)) = group(rest) else { continue };
        let text: String = text.lines().map(strip_comment).collect::<Vec<_>>().join("\n");
        out.push(BibEntry { key: key.trim().to_string(), label, text: text.trim().to_string() });
    }
    out
}

/// The bibliography as printed: the last compile's .bbl, else the paper's own.
pub fn bibliography(paper: &Path, build: &Path) -> Vec<BibEntry> {
    for src in own_sources(paper, build) {
        if let Ok(t) = std::fs::read_to_string(src) {
            let v = bib_entries(&t);
            if !v.is_empty() {
                return v;
            }
        }
    }
    vec![]
}

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "svg", "webp"];
const CONVERT_EXT: &[&str] = &["pdf", "eps", "ps"];

/// `\graphicspath{{figures/}{img/}}` folders.
fn graphics_paths(main: &str) -> Vec<String> {
    let mut v = vec![String::new()];
    for line in main.lines() {
        let code = line.split('%').next().unwrap_or("");
        if let Some(i) = code.find("\\graphicspath") {
            if let Some((inner, _)) = group(&code[i + "\\graphicspath".len()..]) {
                let mut rest = inner;
                while let Some((d, r)) = group(rest) {
                    v.push(d.trim().to_string());
                    rest = r;
                }
            }
        }
    }
    v
}

/// The file `\includegraphics{name}` means, inside the paper folder.
pub fn resolve_figure(paper: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.contains("..") || name.starts_with('/') || name.contains('\\') {
        return None;
    }
    let main = std::fs::read_to_string(paper.join("main.tex")).unwrap_or_default();
    let has_ext = Path::new(name).extension().is_some_and(|x| {
        let x = x.to_string_lossy().to_lowercase();
        IMAGE_EXT.contains(&x.as_str()) || CONVERT_EXT.contains(&x.as_str())
    });
    for dir in graphics_paths(&main) {
        let base = paper.join(&dir).join(name);
        if has_ext && base.is_file() {
            return Some(base);
        }
        // LaTeX's own order of preference for pdflatex
        for ext in ["pdf", "png", "jpg", "jpeg", "eps"] {
            let p = PathBuf::from(format!("{}.{ext}", base.display()));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// An image the browser can show: the file itself, or a PNG of the first
/// page of a PDF/EPS figure (Ghostscript, or sips on macOS), cached.
pub async fn figure_image(file: &Path, cache: &Path) -> Option<(PathBuf, &'static str)> {
    let ext = file.extension()?.to_string_lossy().to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "",
    };
    if !mime.is_empty() {
        return Some((file.to_path_buf(), mime));
    }
    if !CONVERT_EXT.contains(&ext.as_str()) {
        return None;
    }
    let stamp = std::fs::metadata(file).and_then(|m| m.modified()).ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(file, stamp), &mut h);
    let out = cache.join(format!("{:016x}.png", std::hash::Hasher::finish(&h)));
    if out.is_file() {
        return Some((out, "image/png"));
    }
    std::fs::create_dir_all(cache).ok()?;
    let mut ok = false;
    if let Some(gs) = tool("gs") {
        let mut c = crate::proc::cmd(gs);
        c.args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=png16m", "-r144", "-dTextAlphaBits=4", "-dGraphicsAlphaBits=4", "-dFirstPage=1", "-dLastPage=1"]);
        if ext != "pdf" {
            c.arg("-dEPSCrop");
        }
        c.arg(format!("-sOutputFile={}", out.display())).arg(file);
        ok = c.kill_on_drop(true).output().await.is_ok_and(|o| o.status.success());
    }
    if !ok && cfg!(target_os = "macos") && ext == "pdf" {
        ok = crate::proc::cmd("sips")
            .args(["-s", "format", "png"])
            .arg(file)
            .arg("--out")
            .arg(&out)
            .output()
            .await
            .is_ok_and(|o| o.status.success());
    }
    (ok && out.is_file()).then_some((out, "image/png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_labels_and_citations_from_aux() {
        let aux = "\\newlabel{eq:loss}{{3}{2}{}{equation.2.3}{}}\n\\newlabel{sec:m}{{\\mbox {II-A}}{2}{}{}{}}\n\\newlabel{eq:loss@cref}{{[equation][3][]3}{[1][2][]2}}\n\\bibcite{smith}{4}\n\\bibcite{lee}{{2}{2020}{{Lee et~al.}}{{Lee, Kim}}}\n";
        let mut n = Numbers::default();
        parse_aux(aux, &mut n);
        assert_eq!(n.labels["eq:loss"], "3");
        assert_eq!(n.labels["sec:m"], "II-A");
        assert!(!n.labels.contains_key("eq:loss@cref"));
        assert_eq!(n.cites["smith"], Cite { n: "4".into(), ..Default::default() });
        assert_eq!(n.cites["lee"], Cite { n: "2".into(), author: Some("Lee et al.".into()), year: Some("2020".into()) });
    }

    #[test]
    fn numbers_bibitems_in_order() {
        let bbl = "\\begin{thebibliography}{10}\n\\bibitem{a} A.\n\\bibitem[{Smith et~al.(2020)Smith, Jones}]{b} B.\n\\bibitem[7]{c} C.\n";
        let mut n = Numbers::default();
        parse_bibitems(bbl, &mut n);
        assert_eq!(n.cites["a"].n, "1");
        assert_eq!(n.cites["b"].author.as_deref(), Some("Smith et al."));
        assert_eq!(n.cites["b"].year.as_deref(), Some("2020"));
        assert_eq!(n.cites["c"].n, "7");
    }

    #[test]
    fn splits_main_around_sections() {
        let main = "\\documentclass{x}\n\\title{T}\n\\author[1]{A \\and B}\n\\begin{document}\n\\maketitle\n\\begin{abstract}Ab\\end{abstract}\n\\input{sections/01-a}\n\\appendix\n\\input{sections/02-b}\n\\bibliography{r}\n\\end{document}\n";
        let (front, back) = front_back(main);
        assert!(front.starts_with("\\title{T}\n\\author[1]{A \\and B}\n"), "{front}");
        assert!(front.contains("\\begin{abstract}Ab"));
        assert_eq!(back.trim(), "\\appendix\n\\bibliography{r}");
    }

    #[test]
    fn reads_bibliography_entries() {
        let bbl = "\\begin{thebibliography}{10}\n\\bibitem{a}\nA.~Author.\n\\newblock Title.\n\n\\bibitem[{Lee(2020)}]{b} B.\n\\end{thebibliography}";
        let v = bib_entries(bbl);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].key, "a");
        assert!(v[0].text.contains("\\newblock Title."));
        assert_eq!(v[1].label.as_deref(), Some("{Lee(2020)}"));
    }

    #[test]
    fn finds_figures_like_latex() {
        let d = std::env::temp_dir().join(format!("pc-fig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("img")).unwrap();
        std::fs::write(d.join("main.tex"), "\\graphicspath{{img/}}\n").unwrap();
        std::fs::write(d.join("img/a.png"), "x").unwrap();
        std::fs::write(d.join("b.pdf"), "x").unwrap();
        assert_eq!(resolve_figure(&d, "a"), Some(d.join("img/a.png")));
        assert_eq!(resolve_figure(&d, "b.pdf"), Some(d.join("b.pdf")));
        assert_eq!(resolve_figure(&d, "../x"), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
