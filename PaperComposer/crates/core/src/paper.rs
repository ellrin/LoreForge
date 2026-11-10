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

/// Write back: one sentence per line for prose. Environments and display
/// math stay exactly as typed on their own lines, and so do lines with a `%`
/// comment; the prose around them is split into sentences. Only line breaks
/// between words change, so the compiled document is the same.
///
/// A paragraph holding a blank line (typed in the editor, as in LaTeX or
/// Word) becomes two paragraphs, split the way a file is read
/// (`text::paragraphs`: never inside an environment), so what is written
/// reads back as the same list.
pub fn render_section(s: &Section) -> String {
    let mut out = format!("\\section{{{}}}{}\n", s.title, s.tail);
    for p in s.paragraphs.iter().flat_map(|p| crate::text::paragraphs(p)) {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(&render_paragraph(p));
        out.push('\n');
    }
    out
}

// ── Section files: add, delete, restore, reorder, split ──
// The main file reads the sections with `\input{…}` (or `\include`):
// `sections/x` in a paper made here, the project's own paths in a paper
// opened as it is (open.rs; there a `\section{Title}` typed in the main file
// right before an `\input` is that file's title, and moves with it). Every
// operation edits those commands where they are: a new one goes on its own
// line after its neighbour's, a removed one takes its line with it when
// nothing else is on it, and a new order is written into the existing
// places in turn, so whatever sits between them (`\appendix`, a
// `\clearpage`) keeps its place. File names are never changed: review.json,
// the navigator and co-authors know a section by its file.

/// Where a section is read from, in each layout. Section ids are file names
/// under sections/ (standard), or paths relative to the paper folder (a
/// paper opened as it is).
struct Layout<'a> {
    paper: &'a Path,
    custom: bool,
    ids: Vec<String>,
}

impl<'a> Layout<'a> {
    fn of(paper: &'a Path) -> Self {
        let custom = crate::open::is_custom(paper);
        let ids = if custom { crate::open::sections(paper).into_iter().map(|s| s.file).collect() } else { section_files(paper) };
        Layout { paper, custom, ids }
    }
    fn path(&self, id: &str) -> PathBuf {
        if self.custom { self.paper.join(id) } else { self.paper.join("sections").join(id) }
    }
    /// The section an `\input` argument reads, if it is one.
    fn id_of(&self, arg: &str) -> Option<String> {
        let arg = arg.trim().trim_start_matches("./");
        if self.custom {
            let id = if arg.ends_with(".tex") { arg.to_string() } else { format!("{arg}.tex") };
            return self.ids.contains(&id).then_some(id);
        }
        let name = arg.strip_prefix("sections/")?;
        Some(if name.ends_with(".tex") { name.to_string() } else { format!("{name}.tex") })
    }
    fn arg(&self, id: &str, ext: bool) -> String {
        let stem = id.strip_suffix(".tex").unwrap_or(id);
        let stem = if ext { format!("{stem}.tex") } else { stem.to_string() };
        if self.custom { stem } else { format!("sections/{stem}") }
    }
    /// The file itself holds its `\section` (else its title is in the main file).
    fn titled(&self, id: &str) -> bool {
        std::fs::read_to_string(self.path(id)).is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("\\section")))
    }
    fn main_path(&self) -> PathBuf {
        crate::open::main_file(self.paper)
    }
    fn read_main(&self) -> Result<String, String> {
        let p = self.main_path();
        std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
    }
    fn write_main(&self, main: &str) -> Result<(), String> {
        write_atomic(&self.main_path(), main).map_err(|e| e.to_string())
    }
    /// Deleted sections wait here: sections/unused/, or the app's own folder
    /// in a project opened as it is (its folders stay the author's).
    fn unused(&self) -> PathBuf {
        if self.custom { self.paper.join(APP_DIR).join(crate::open::UNUSED_DIR) } else { self.paper.join("sections").join(crate::open::UNUSED_DIR) }
    }
    /// A section id this paper may have: a plain name (standard), or a
    /// relative .tex path inside the paper (opened as it is).
    fn valid_id(&self, id: &str) -> bool {
        if !id.ends_with(".tex") {
            return false;
        }
        if self.custom {
            !id.is_empty() && !id.starts_with('/') && !id.contains('\\') && id.split('/').all(|p| safe_name(p))
        } else {
            safe_name(id)
        }
    }
    /// A free id for a new section titled `title`, next to `near` (its folder).
    fn free_id(&self, title: &str, near: Option<&str>) -> String {
        let base = slug(&crate::text::strip_latex(title));
        let dir = if self.custom { near.and_then(|n| n.rsplit_once('/')).map(|(d, _)| format!("{d}/")).unwrap_or_default() } else { String::new() };
        let taken = |f: &str| self.path(f).exists() || self.unused().join(f.replace('/', "-")).exists();
        (1..).map(|n| if n == 1 { format!("{dir}{base}.tex") } else { format!("{dir}{base}-{n}.tex") }).find(|f| !taken(f)).unwrap()
    }
}

/// One `\input` / `\include` of a section in the main file, outside comments.
#[derive(Debug, Clone)]
struct SectionInput {
    /// The command.
    range: std::ops::Range<usize>,
    /// What belongs to the section here: the command, and before it a
    /// `\section{…}` typed in the main file when the file has none.
    slot: std::ops::Range<usize>,
    cmd: &'static str,
    id: String,
    /// Written with `.tex`.
    ext: bool,
}

fn section_inputs(main: &str, lay: &Layout) -> Vec<SectionInput> {
    let body = main.find("\\begin{document}").unwrap_or(0);
    let mut out: Vec<SectionInput> = Vec::new();
    let mut start = 0;
    for line in main.split_inclusive('\n') {
        let at = start;
        start += line.len();
        if at + line.len() <= body {
            continue;
        }
        let code = strip_comment(line);
        for cmd in ["\\input", "\\include"] {
            for (i, _) in code.match_indices(cmd) {
                let after = &code[i + cmd.len()..];
                if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                    continue; // \includegraphics, \inputencoding
                }
                let Some(arg) = after.strip_prefix('{').and_then(|a| a.split_once('}')).map(|(a, _)| a) else { continue };
                let Some(id) = lay.id_of(arg) else { continue };
                let range = at + i..at + i + cmd.len() + arg.len() + 2;
                out.push(SectionInput { slot: range.clone(), range, cmd: if cmd == "\\input" { "\\input" } else { "\\include" }, ext: arg.trim().ends_with(".tex"), id });
            }
        }
    }
    out.sort_by_key(|x| x.range.start);
    if lay.custom {
        // a title typed in the main file goes with the file after it
        let mut prev = body;
        for x in out.iter_mut() {
            if !lay.titled(&x.id) {
                if let Some(s) = main[prev..x.range.start].rfind("\\section").map(|i| prev + i) {
                    let ls = main[..s].rfind('\n').map_or(0, |i| i + 1);
                    x.slot.start = if main[ls..s].trim().is_empty() { ls } else { s };
                }
            }
            prev = x.range.end;
        }
    }
    out
}

fn line_start(s: &str, at: usize) -> usize {
    s[..at].rfind('\n').map_or(0, |i| i + 1)
}
fn line_end(s: &str, at: usize) -> usize {
    s[at..].find('\n').map_or(s.len(), |i| at + i + 1)
}

/// The main file with `text` on lines of its own: after the section `after`
/// reads, else before the first section, else before the bibliography /
/// `\end{document}`. Its lines take the indent of their neighbour.
fn insert_block(main: &str, lay: &Layout, text: &str, after: Option<&str>) -> String {
    let inputs = section_inputs(main, lay);
    let indent = |at: usize| main[line_start(main, at)..].chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>();
    let anchor = after.and_then(|a| inputs.iter().filter(|x| x.id == a).next_back()).or(if after.is_some() { inputs.last() } else { None });
    let (at, ind) = match (anchor, inputs.first()) {
        (Some(a), _) => (line_end(main, a.slot.end), indent(a.range.start)),
        (None, Some(first)) => (line_start(main, first.slot.start), indent(first.range.start)),
        (None, None) => {
            let body = main.find("\\begin{document}").unwrap_or(0);
            let end = ["\\bibliography{", "\\printbibliography", "\\appendix", "\\end{document}"]
                .iter()
                .filter_map(|m| main[body..].find(m).map(|i| body + i))
                .min()
                .unwrap_or(main.len());
            (line_start(main, end), String::new())
        }
    };
    let mut block: String = text.trim_matches('\n').lines().map(|l| format!("{ind}{}\n", l.trim_start())).collect();
    if at > 0 && !main[..at].ends_with('\n') {
        block.insert(0, '\n');
    }
    format!("{}{}{}", &main[..at], block, &main[at..])
}

/// A command reading section `id`, in the style of its neighbour `near`
/// (else of the main file's first).
fn input_line(main: &str, lay: &Layout, id: &str, near: Option<&str>) -> String {
    let inputs = section_inputs(main, lay);
    let like = near.and_then(|n| inputs.iter().find(|x| x.id == n)).or(inputs.first());
    let (cmd, ext) = like.map(|x| (x.cmd, x.ext)).unwrap_or(("\\input", false));
    format!("{cmd}{{{}}}", lay.arg(id, ext))
}

/// The main file without the places reading `id` (and their lines, when
/// nothing else is on them), the text taken out (for a restore), and, when
/// it was one place, where and exactly what was cut.
fn remove_input(main: &str, lay: &Layout, id: &str) -> (String, String, Option<(usize, String)>) {
    let mut out = main.to_string();
    let mut taken = Vec::new();
    let mut cut = Vec::new();
    for x in section_inputs(main, lay).into_iter().rev().filter(|x| x.id == id) {
        let ls = line_start(&out, x.slot.start);
        let le = line_end(&out, x.slot.end);
        taken.push(out[x.slot.clone()].to_string());
        let range = if out[ls..x.slot.start].trim().is_empty() && out[x.slot.end..le].trim().is_empty() { ls..le } else { x.slot.clone() };
        cut.push((range.start, out[range.clone()].to_string()));
        out.replace_range(range, "");
    }
    taken.reverse();
    let exact = if cut.len() == 1 { cut.pop() } else { None };
    (out, taken.join("\n"), exact)
}

fn text_hash(t: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Every `\label{…}` in the main file and the section files.
fn paper_labels(paper: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for f in crate::open::source_files(paper) {
        let t = std::fs::read_to_string(f).unwrap_or_default();
        for l in t.lines().map(strip_comment) {
            for (i, _) in l.match_indices("\\label{") {
                if let Some((arg, _)) = braced(&l[i + "\\label".len()..]) {
                    out.push(arg.trim().to_string());
                }
            }
        }
    }
    out
}

/// `\label{sec:<slug>}` not used anywhere in the paper yet.
fn free_section_label(paper: &Path, title: &str) -> String {
    let base = slug(&crate::text::strip_latex(title));
    let labels = paper_labels(paper);
    let label = (1..).map(|n| if n == 1 { format!("sec:{base}") } else { format!("sec:{base}-{n}") }).find(|l| !labels.contains(l)).unwrap();
    format!("\\label{{{label}}}")
}

/// A section title as typed: one line, braces balanced.
pub fn valid_title(title: &str) -> Result<&str, String> {
    let t = title.trim();
    if t.is_empty() {
        return Err("章節名稱不能空白".into());
    }
    if t.contains('\n') || braced(&format!("{{{t}}}")).is_none_or(|(inner, rest)| inner != t || !rest.is_empty()) {
        return Err("章節名稱的大括號沒有成對".into());
    }
    Ok(t)
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    write_atomic(path, text).map_err(|e| e.to_string())
}

/// A new, empty section after `after` (None: first), with a `\label{sec:…}`,
/// in a file of its own (in a project opened as it is: next to the section
/// it follows). Returns its id.
pub fn add_section(paper: &Path, title: &str, after: Option<&str>) -> Result<String, String> {
    let title = valid_title(title)?;
    let lay = Layout::of(paper);
    let main = lay.read_main()?;
    let id = lay.free_id(title, after.or(lay.ids.first().map(String::as_str)));
    let tail = free_section_label(paper, title);
    let s = Section { file: id.clone(), title: title.to_string(), paragraphs: vec![], tail };
    write_new(&lay.path(&id), &render_section(&s))?;
    let line = input_line(&main, &lay, &id, after);
    let lay = Layout { ids: [lay.ids.clone(), vec![id.clone()]].concat(), ..lay };
    lay.write_main(&insert_block(&main, &lay, &line, after))?;
    Ok(id)
}

/// A deleted section: where its file waits, and what to put back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parked {
    /// File name in the unused folder.
    pub parked: String,
    /// Its id before.
    pub file: String,
    /// The section it followed.
    pub after: Option<String>,
    /// The main file's text that read it (with a `\section` typed there).
    pub slot: String,
    /// Where the main file was cut and what came out, valid while the main
    /// file is still as the delete left it (`main`): an undo puts it back
    /// exactly (after an `\appendix`, say).
    #[serde(default)]
    pub at: Option<usize>,
    #[serde(default)]
    pub cut: String,
    #[serde(default)]
    pub main: String,
}

/// Take a section out of the paper: its file goes to the unused folder (so
/// it can come back) and the main file no longer reads it.
pub fn delete_section(paper: &Path, file: &str) -> Result<Parked, String> {
    let lay = Layout::of(paper);
    let at = lay.ids.iter().position(|f| f == file).ok_or("找不到這個章節")?;
    let main = lay.read_main()?;
    let unused = lay.unused();
    std::fs::create_dir_all(&unused).map_err(|e| e.to_string())?;
    let flat = file.replace('/', "-");
    let stem = flat.trim_end_matches(".tex");
    let parked = (1..).map(|n| if n == 1 { flat.clone() } else { format!("{stem}-{n}.tex") }).find(|f| !unused.join(f).exists()).unwrap();
    let (rest, slot, exact) = remove_input(&main, &lay, file);
    std::fs::rename(lay.path(file), unused.join(&parked)).map_err(|e| e.to_string())?;
    lay.write_main(&rest)?;
    let (cut_at, cut) = exact.map(|(a, c)| (Some(a), c)).unwrap_or((None, String::new()));
    Ok(Parked { parked, file: file.to_string(), after: at.checked_sub(1).map(|i| lay.ids[i].clone()), slot, at: cut_at, cut, main: text_hash(&rest) })
}

/// Bring a deleted section back as it was, after `p.after` (or first). Its
/// old id unless that was taken since. Returns its id.
pub fn restore_section(paper: &Path, p: &Parked) -> Result<String, String> {
    let lay = Layout::of(paper);
    if !safe_name(&p.parked) || !lay.valid_id(&p.file) {
        return Err("invalid section file".into());
    }
    let from = lay.unused().join(&p.parked);
    if !from.is_file() {
        return Err("找不到刪除的章節".into());
    }
    let main = lay.read_main()?;
    let id = if lay.path(&p.file).exists() { lay.free_id(p.file.trim_end_matches(".tex").rsplit('/').next().unwrap_or("section"), Some(&p.file)) } else { p.file.clone() };
    let to = lay.path(&id);
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
    // the text that read it, when it still reads this file; else a plain \input
    let lay = Layout { ids: [lay.ids.clone(), vec![id.clone()]].concat(), ..lay };
    let block = if id == p.file && section_inputs(&format!("\\begin{{document}}\n{}\n", p.slot), &lay).iter().any(|x| x.id == id) { p.slot.clone() } else { input_line(&main, &lay, &id, p.after.as_deref()) };
    let after = p.after.as_deref().filter(|a| lay.ids.iter().any(|f| f == a));
    let exact = p.at.filter(|&a| id == p.file && !p.cut.is_empty() && text_hash(&main) == p.main && main.is_char_boundary(a) && a <= main.len());
    let out = match exact {
        Some(a) => format!("{}{}{}", &main[..a], p.cut, &main[a..]),
        None => insert_block(&main, &lay, &block, after),
    };
    lay.write_main(&out)?;
    Ok(id)
}

/// Put the sections in `order` (every section once). The main file's own
/// places take the new order in turn; sections it did not read yet are
/// added after the last one.
pub fn reorder_sections(paper: &Path, order: &[String]) -> Result<(), String> {
    let lay = Layout::of(paper);
    let mut a = lay.ids.clone();
    let mut b = order.to_vec();
    a.sort();
    b.sort();
    if a != b {
        return Err("章節清單和磁碟上的不同，請重新載入".into());
    }
    let main = lay.read_main()?;
    // the first place reading each section is its slot
    let mut seen = Vec::new();
    let slots: Vec<SectionInput> = section_inputs(&main, &lay).into_iter().filter(|x| !seen.contains(&x.id) && { seen.push(x.id.clone()); true }).collect();
    let text_of = |id: &str| slots.iter().find(|x| x.id == id).map(|x| main[x.slot.clone()].to_string()).unwrap_or_else(|| input_line(&main, &lay, id, None));
    let mut out = main.clone();
    for (x, id) in slots.iter().zip(order).rev() {
        out.replace_range(x.slot.clone(), &text_of(id));
    }
    let mut last = slots.len().checked_sub(1).map(|i| order[i].clone());
    for id in order.iter().skip(slots.len()) {
        out = insert_block(&out, &lay, &input_line(&main, &lay, id, last.as_deref()), last.as_deref());
        last = Some(id.clone());
    }
    if out != main {
        lay.write_main(&out)?;
    }
    Ok(())
}

/// A paragraph that is only `\section{Title}` (and a `\label`): (title, tail).
pub fn section_heading(p: &str) -> Option<(String, String)> {
    let t = strip_comment(p.trim()).trim();
    if t.contains('\n') {
        return None;
    }
    let rest = t.strip_prefix("\\section")?;
    let (title, tail) = braced(rest.strip_prefix('{').map(|_| rest)?)?;
    let tail = tail.trim();
    let label_only = tail.is_empty() || (tail.starts_with("\\label{") && braced(&tail["\\label".len()..]).is_some_and(|(_, r)| r.trim().is_empty()));
    label_only.then(|| (title.trim().to_string(), tail.to_string()))
}

/// The section file as a split reads it: (path, holds only the body, section).
/// A file whose title is typed in the main file holds only the body: there
/// every `\section` in it is a new one (read as a section, the file would
/// take the first for its own title).
fn split_source(lay: &Layout, file: &str) -> Result<(PathBuf, bool, Section), String> {
    if !lay.ids.iter().any(|f| f == file) {
        return Err("找不到這個章節".into());
    }
    let path = lay.path(file);
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let body_only = lay.custom && !text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('%')).is_some_and(|l| l.starts_with("\\section"));
    let sec = if body_only {
        Section { file: file.to_string(), title: String::new(), paragraphs: crate::text::paragraphs(&text), tail: String::new() }
    } else {
        parse_section(file, &text)
    };
    Ok((path, body_only, sec))
}

fn heading_cuts(sec: &Section) -> Vec<usize> {
    sec.paragraphs.iter().enumerate().filter(|(_, p)| section_heading(p).is_some()).map(|(i, _)| i).collect()
}

/// Where `split_section` would cut: the heading paragraphs' places.
pub fn split_cuts(paper: &Path, file: &str) -> Vec<usize> {
    split_source(&Layout::of(paper), file).map(|(_, _, s)| heading_cuts(&s)).unwrap_or_default()
}

/// A `\section{…}` typed inside a section becomes a section file of its own:
/// it and what follows it move to a new file read right after this one
/// (with a `\label{sec:…}` when it had none). Every such heading in the
/// section is split off. Returns the new ids in order.
pub fn split_section(paper: &Path, file: &str) -> Result<Vec<String>, String> {
    let lay = Layout::of(paper);
    let (path, body_only, mut sec) = split_source(&lay, file)?;
    let mut cuts = heading_cuts(&sec);
    if cuts.is_empty() {
        return Ok(vec![]);
    }
    let mut parts: Vec<Section> = Vec::new();
    cuts.push(sec.paragraphs.len());
    for w in cuts.windows(2) {
        let (title, tail) = section_heading(&sec.paragraphs[w[0]]).unwrap();
        parts.push(Section { file: String::new(), title, paragraphs: sec.paragraphs[w[0] + 1..w[1]].to_vec(), tail });
    }
    sec.paragraphs.truncate(cuts[0]);
    let mut main = lay.read_main()?;
    let mut lay = lay;
    let mut after = file.to_string();
    let mut made = Vec::new();
    for mut part in parts {
        part.file = lay.free_id(&part.title, Some(file));
        if part.tail.is_empty() {
            part.tail = free_section_label(paper, &part.title);
        }
        write_new(&lay.path(&part.file), &render_section(&part))?;
        let line = input_line(&main, &lay, &part.file, Some(file));
        lay.ids.push(part.file.clone());
        main = insert_block(&main, &lay, &line, Some(&after));
        after = part.file.clone();
        made.push(part.file);
    }
    let rendered = render_section(&sec);
    let own = if body_only { rendered.split_once('\n').map(|x| x.1).unwrap_or("").trim_start_matches('\n').to_string() } else { rendered };
    write_atomic(&path, &own).map_err(|e| e.to_string())?;
    lay.write_main(&main)?;
    Ok(made)
}

/// Commands that start running text rather than a block: drop caps,
/// run-in heads, emphasis, a citation or reference opening a sentence.
const PROSE_COMMANDS: &[&str] = &[
    "IEEEPARstart", "PARstart", "noindent", "indent", "textit", "emph", "textbf", "textsc", "textsl", "textsf", "textrm", "texttt", "underline",
    "paragraph", "cite", "citet", "citep", "Citet", "Citep", "citeauthor", "Citeauthor", "ref", "eqref", "cref", "Cref", "autoref", "url", "href", "mbox",
];

/// Running text: it starts with a word, or with one of `PROSE_COMMANDS`.
fn is_prose(text: &str) -> bool {
    let Some(rest) = text.trim_start().strip_prefix('\\') else { return true };
    let name: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    PROSE_COMMANDS.contains(&name.as_str())
}

/// A line holding only commands that are not prose and their arguments,
/// such as `\\enlargethispage{-\\baselineskip}` or `\\vspace{2mm}\\noindent`.
fn command_only(t: &str) -> bool {
    let mut rest = t.trim();
    if rest.is_empty() || is_prose(rest) {
        return false;
    }
    while let Some(r) = rest.strip_prefix('\\') {
        let name_len = r.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        if name_len == 0 {
            return false;
        }
        rest = r[name_len..].trim_start_matches('*').trim_start();
        loop {
            if rest.starts_with('{') {
                match braced(rest) {
                    Some((_, after)) => rest = after.trim_start(),
                    None => return false,
                }
            } else if rest.starts_with('[') {
                match rest.find(']') {
                    Some(e) => rest = rest[e + 1..].trim_start(),
                    None => return false,
                }
            } else {
                break;
            }
        }
    }
    rest.is_empty()
}

/// One paragraph in file form: prose one sentence per line; environments,
/// display math, comment lines and lines of layout commands verbatim on
/// their own lines.
fn render_paragraph(p: &str) -> String {
    // \verb keeps its spaces: leave such a paragraph alone.
    if p.contains("\\verb") {
        return p.to_string();
    }
    let lines: Vec<&str> = p.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut prose: Vec<&str> = Vec::new();
    let flush = |prose: &mut Vec<&str>, out: &mut Vec<String>| {
        if prose.is_empty() {
            return;
        }
        let text = prose.join("\n");
        if is_prose(&text) {
            out.extend(sentences(&text));
        } else {
            out.push(text);
        }
        prose.clear();
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let code = strip_comment(line);
        let t = code.trim_start();
        let end = if code.contains("\\begin{") {
            // to the line that closes every environment opened here
            let mut depth = 0i32;
            let mut j = i;
            loop {
                let c = strip_comment(lines[j]);
                depth += c.matches("\\begin{").count() as i32 - c.matches("\\end{").count() as i32;
                j += 1;
                if depth <= 0 || j == lines.len() {
                    break j;
                }
            }
        } else if t.starts_with("\\[") {
            let close = |c: &str| c.contains("\\]");
            (i..lines.len()).find(|&j| close(if j == i { &t[2..] } else { strip_comment(lines[j]) })).map_or(lines.len(), |j| j + 1)
        } else if t.starts_with("$$") {
            let close = |c: &str| c.contains("$$");
            (i..lines.len()).find(|&j| close(if j == i { &t[2..] } else { strip_comment(lines[j]) })).map_or(lines.len(), |j| j + 1)
        } else if code.len() != line.len() {
            i + 1 // a line with a comment: the comment also swallows the line break
        } else if command_only(code) {
            i + 1
        } else {
            prose.push(line);
            i += 1;
            continue;
        };
        flush(&mut prose, &mut out);
        out.push(split_items(&lines[i..end]).unwrap_or_else(|| lines[i..end].join("\n")));
        i = end;
    }
    flush(&mut prose, &mut out);
    out.join("\n")
}

/// A simple list (`itemize` / `enumerate` / `description` with no nested
/// environment and no comment) with each `\item` one sentence per line, like
/// prose; `None` leaves the block as typed. Only line breaks between words
/// change. Captions and other environments are never split.
fn split_items(block: &[&str]) -> Option<String> {
    let (first, last) = (block.first()?.trim(), block.last()?.trim());
    let env = ["itemize", "enumerate", "description"].into_iter().find(|e| first.starts_with(&format!("\\begin{{{e}}}")))?;
    let head_rest = first[format!("\\begin{{{env}}}").len()..].trim();
    if block.len() < 3 || last != format!("\\end{{{env}}}") || !(head_rest.is_empty() || head_rest.starts_with('[') && head_rest.ends_with(']')) {
        return None;
    }
    let inner = &block[1..block.len() - 1];
    if inner.iter().any(|l| l.contains('%') || l.contains("\\begin{") || l.contains("\\end{") || l.trim().is_empty() || l.contains("\\verb")) {
        return None;
    }
    let mut items: Vec<(String, Vec<&str>)> = Vec::new();
    for l in inner {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("\\item").filter(|r| !r.starts_with(|c: char| c.is_ascii_alphabetic())) {
            // \item[label] keeps its label with the first sentence
            let (label, body) = if rest.trim_start().starts_with('[') {
                let r = rest.trim_start();
                let e = r.find(']')?;
                (r[..=e].to_string(), &r[e + 1..])
            } else {
                (String::new(), rest)
            };
            items.push((format!("\\item{label}"), vec![body.trim()]));
        } else {
            items.last_mut()?.1.push(t);
        }
    }
    let mut out = vec![block[0].trim_end().to_string()];
    for (head, body) in items {
        let text = body.join("\n");
        let sents = if text.trim().is_empty() { vec![] } else { crate::text::sentences(&text) };
        match sents.split_first() {
            Some((one, rest)) => {
                out.push(format!("{head} {one}"));
                out.extend(rest.iter().cloned());
            }
            None => out.push(head),
        }
    }
    out.push(block[block.len() - 1].trim_end().to_string());
    Some(out.join("\n"))
}

/// Layout parameters and document parts a preamble redefines; none of them
/// is math, and KaTeX would only choke on them.
const LAYOUT_MACROS: &[&str] = &[
    "\\topfraction", "\\bottomfraction", "\\textfraction", "\\floatpagefraction", "\\dbltopfraction", "\\dblfloatpagefraction",
    "\\baselinestretch", "\\arraystretch", "\\thebibliography", "\\bibsection", "\\bibfont", "\\maketitle", "\\section", "\\subsection",
    "\\subsubsection", "\\paragraph", "\\thesection", "\\thesubsection", "\\thefigure", "\\thetable", "\\footnoterule", "\\headrule",
];

/// A macro the math previewer cannot use: a layout setting, or an
/// internal one (`@` in its name or body only works between \makeatletter
/// and \makeatother).
fn layout_macro(name: &str, body: &str) -> bool {
    LAYOUT_MACROS.contains(&name) || name.contains('@') || body.contains('@')
}

/// `\newcommand`, `\renewcommand`, `\DeclareMathOperator` and simple `\def`
/// from LaTeX source, as name -> body, for the math previewer. Argument
/// counts are dropped: `#1` in the body works the same way in KaTeX. Layout
/// macros (`\topfraction`, `\thebibliography`, anything with `@`) are left out.
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
            if layout_macro(&name, &body) {
                i = k;
                continue;
            }
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
    let main_path = crate::open::main_file(paper);
    let mut files = vec![main_path.clone()];
    if let Ok(dir) = std::fs::read_dir(paper) {
        for e in dir.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("macros") && n.ends_with(".tex") {
                files.push(e.path());
            }
        }
    }
    // files the preamble \inputs (notation.tex, a co-author's defs.tex)
    let main = std::fs::read_to_string(&main_path).unwrap_or_default();
    let preamble = &main[..main.find("\\begin{document}").unwrap_or(main.len())];
    for line in preamble.lines().map(strip_comment) {
        for (i, _) in line.match_indices("\\input{") {
            if let Some((arg, _)) = braced(&line[i + "\\input".len()..]) {
                if let Some(f) = resolve_tex(paper, paper, arg.trim()).filter(|f| !files.contains(f)) {
                    files.push(f);
                }
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

/// The paper's sections; a paper opened as it is reads them from its own
/// main file's structure (`open::sections`).
pub fn load_paper(paper: &Path) -> Vec<Section> {
    if crate::open::is_custom(paper) {
        return crate::open::sections(paper);
    }
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
    write_atomic(&dir.join("main.tex"), &blank_main(name, &inputs)).map_err(io)?;
    for s in &sections {
        write_atomic(&dir.join("sections").join(&s.file), &render_section(s)).map_err(io)?;
    }
    write_atomic(&dir.join("refs.bib"), "").map_err(io)?;
    Ok(dir)
}

/// The plain article: the neutral template a paper has until one is chosen.
fn blank_main(title: &str, inputs: &str) -> String {
    format!(
        "\\documentclass{{article}}\n\\usepackage{{amsmath,amssymb}}\n\n\\title{{{title}}}\n\\author{{}}\n\n\\begin{{document}}\n\\maketitle\n\n\\begin{{abstract}}\n\\end{{abstract}}\n\n{inputs}\n\\bibliographystyle{{plain}}\n\\bibliography{{refs}}\n\\end{{document}}\n"
    )
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

/// Packages a paper's content may need, carried over when the template
/// changes. Class-specific packages are left behind on purpose.
const CONTENT_PACKAGES: &[&str] = &[
    "amsmath", "amssymb", "amsthm", "mathtools", "bm", "graphicx", "booktabs", "multirow", "makecell", "array", "tabularx",
    "threeparttable", "siunitx", "xcolor", "subcaption", "algorithm", "algorithmic", "algpseudocode", "url",
];

/// Put an existing paper on another template (`None` = the plain article).
/// main.tex is rebuilt around the paper's own section files, which are not
/// touched; the title, abstract and keywords move over, and so do the
/// preamble's `\input` of files in the paper (notation.tex, macros), its
/// one-line macro definitions, its content packages and its own .bib files.
/// The template's support files are copied without overwriting anything.
/// Authors are left to the caller (they come from the profile). Returns the
/// old main.tex.
pub fn apply_template(paper: &Path, template: Option<&Path>) -> Result<String, String> {
    let io = |e: std::io::Error| e.to_string();
    let main_path = crate::open::main_file(paper);
    let old = read_tex(&main_path).ok_or("這篇論文沒有主檔")?;
    // the body's sections as the paper reads them: its own file names, and a
    // title typed in the main file stays in front of its file
    let inputs: String = if crate::open::is_custom(paper) {
        crate::open::sections(paper)
            .iter()
            .map(|s| {
                let own = std::fs::read_to_string(paper.join(&s.file)).is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("\\section")));
                let cmd = format!("\\input{{{}}}\n", s.file.trim_end_matches(".tex"));
                if own { cmd } else { format!("\\section{{{}}}\n{cmd}", s.title) }
            })
            .collect()
    } else {
        section_files(paper).iter().map(|f| format!("\\input{{sections/{}}}\n", f.trim_end_matches(".tex"))).collect()
    };
    let mut main = match template {
        None => blank_main("", &inputs),
        Some(t) => {
            let sample = read_tex(&t.join("sample.tex")).ok_or("模板裡沒有 sample.tex")?;
            let mut split = split_sample(&sample, false).ok_or("sample.tex 裡找不到 \\begin{document}")?;
            replace_section_pieces(&mut split, vec![Piece::Text(inputs)]);
            let doc: String = split.pieces.iter().map(|p| if let Piece::Text(t) = p { t.as_str() } else { "" }).collect();
            copy_new(t, paper).map_err(io)?;
            if let Ok(rd) = std::fs::read_dir(t.join("bst")) {
                for e in rd.flatten() {
                    if e.path().extension().is_some_and(|x| x == "bst") && !paper.join(e.file_name()).exists() {
                        std::fs::copy(e.path(), paper.join(e.file_name())).map_err(io)?;
                    }
                }
            }
            format!("{}\\begin{{document}}{}\\end{{document}}\n", split.preamble, doc.trim_end().to_string() + "\n")
        }
    };
    // the title block
    let had = crate::front::fields(&old);
    for f in ["title", "abstract", "keywords"] {
        if let Some(v) = had.iter().find(|x| x.field == f && x.index == 0).filter(|x| !x.value.trim().is_empty()) {
            if let Ok(m) = crate::front::set_field(&main, f, 0, &v.value) {
                main = m;
            }
        }
    }
    // the preamble's own lines
    let old_pre = &old[..old.find("\\begin{document}").unwrap_or(old.len())];
    let at = main.find("\\begin{document}").ok_or("新的 main.tex 沒有 \\begin{document}")?;
    let new_macros = extract_macros(&main[..at]);
    let mut carried = String::new();
    let mut packages: Vec<String> = Vec::new();
    for line in old_pre.lines() {
        let code = strip_comment(line).trim();
        if let Some(arg) = ["\\input{", "\\include{"].iter().find_map(|c| code.strip_prefix(c)).and_then(|r| r.split('}').next()) {
            let file = if arg.ends_with(".tex") { arg.to_string() } else { format!("{arg}.tex") };
            if safe_rel(&file) && paper.join(&file).is_file() && !main[..at].contains(code) {
                carried += code;
                carried.push('\n');
            }
        } else if let Some(rest) = code.strip_prefix("\\usepackage") {
            let rest = rest.trim_start();
            let rest = if rest.starts_with('[') { rest.find(']').map_or("", |e| &rest[e + 1..]) } else { rest };
            if let Some((names, _)) = braced(rest.trim_start()) {
                packages.extend(names.split(',').map(str::trim).filter(|n| CONTENT_PACKAGES.contains(n)).map(String::from));
            }
        } else if ["\\newcommand", "\\renewcommand", "\\providecommand", "\\DeclareMathOperator"].iter().any(|c| code.starts_with(c)) && balanced_line(code) {
            let defs = extract_macros(code);
            if !defs.is_empty() && defs.keys().all(|k| !new_macros.contains_key(k)) {
                carried += code;
                carried.push('\n');
            }
        }
    }
    let mut pre = main[..at].to_string();
    packages.dedup();
    let loaded = loaded_packages(&pre);
    for p in packages.iter().filter(|p| !loaded.contains(p)) {
        pre += &format!("\\usepackage{{{p}}}\n");
    }
    pre += &carried;
    main = pre + &main[at..];
    // the paper's own .bib files (an empty one is a placeholder)
    let own: Vec<String> = bibliography_names(&old).into_iter().filter(|b| std::fs::metadata(paper.join(format!("{b}.bib"))).is_ok_and(|m| m.len() > 0)).collect();
    main = add_bibliographies(&main, &own);
    write_atomic(&main_path, &main).map_err(io)?;
    Ok(old)
}

fn balanced_line(s: &str) -> bool {
    let mut depth = 0i32;
    let mut esc = false;
    for c in s.chars() {
        match c {
            _ if esc => esc = false,
            '\\' => esc = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// Package names a preamble loads.
fn loaded_packages(pre: &str) -> Vec<String> {
    pre.lines()
        .map(strip_comment)
        .filter_map(|l| {
            let i = l.find("\\usepackage")?;
            let rest = l[i + "\\usepackage".len()..].trim_start();
            let rest = if rest.starts_with('[') { &rest[rest.find(']')? + 1..] } else { rest };
            braced(rest.trim_start()).map(|(names, _)| names.to_string())
        })
        .flat_map(|names| names.split(',').map(|n| n.trim().to_string()).collect::<Vec<_>>())
        .collect()
}

/// The names in `\bibliography{a,b}`.
fn bibliography_names(main: &str) -> Vec<String> {
    main.lines()
        .map(strip_comment)
        .filter_map(|l| braced(&l[l.find("\\bibliography{")? + "\\bibliography".len()..]).map(|(n, _)| n.to_string()))
        .flat_map(|n| n.split(',').map(|x| x.trim().trim_end_matches(".bib").to_string()).collect::<Vec<_>>())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Add names to main.tex's `\bibliography{…}` (nothing when it has none).
fn add_bibliographies(main: &str, names: &[String]) -> String {
    let have = bibliography_names(main);
    let missing: Vec<&String> = names.iter().filter(|n| !have.contains(n)).collect();
    if missing.is_empty() {
        return main.to_string();
    }
    let mut done = false;
    let lines: Vec<String> = main
        .lines()
        .map(|l| {
            let code = strip_comment(l);
            if let (false, Some(i)) = (done, code.find("\\bibliography{")) {
                if let Some(j) = code[i..].find('}') {
                    done = true;
                    let add: String = missing.iter().map(|n| format!(",{n}")).collect();
                    let head = &l[..i + j];
                    let head = if head.ends_with('{') { format!("{head}{}", &add[1..]) } else { format!("{head}{add}") };
                    return format!("{head}{}", &l[i + j..]);
                }
            }
            l.to_string()
        })
        .collect();
    lines.join("\n") + if main.ends_with('\n') { "\n" } else { "" }
}

/// Like `copy_support`, but never over a file that is already there.
fn copy_new(from: &Path, to: &Path) -> std::io::Result<()> {
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
            copy_new(&e.path(), &dst)?;
        } else if !dst.exists() {
            std::fs::copy(e.path(), dst)?;
        }
    }
    Ok(())
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
    let dir = papers.join(name);
    if dir.exists() {
        return Err(format!("已經有一篇叫 {name} 的論文"));
    }
    import_into(&dir, src)?;
    Ok(dir)
}

/// The project in `src` as it would be imported: its main file, the folder
/// it sits in (the paper folder), the .tex files merged into it, and the
/// document split into sections.
pub struct Flattened {
    pub main: PathBuf,
    pub base: PathBuf,
    /// The main file and every file its body takes in.
    pub used: Vec<PathBuf>,
    split: Sample,
}

/// Read and flatten a project. Only the document body's `\input`s are
/// merged: the preamble's (notation.tex, macros) stay files of their own.
pub fn flatten_project(src: &Path) -> Result<Flattened, String> {
    let main = find_main_tex(src).ok_or("找不到主檔（有 \\documentclass 與 \\begin{document} 的 .tex）")?;
    let base = main.parent().unwrap_or(src).to_path_buf();
    let mut used = vec![main.clone()];
    let raw = read_tex(&main).unwrap_or_default();
    let at = raw.find("\\begin{document}").ok_or("主檔裡找不到 \\begin{document}")?;
    let text = format!("{}{}", &raw[..at], flatten(&base, &base, &raw[at..], &mut used, 0));
    let split = split_sample(&text, true).ok_or("主檔裡找不到 \\begin{document}")?;
    Ok(Flattened { main, base, used, split })
}

/// Import the project in `src` into the folder `dir` (created when missing;
/// it must not hold a main.tex yet): main.tex plus one file per section,
/// every other file copied.
pub fn import_into(dir: &Path, src: &Path) -> Result<(), String> {
    let f = flatten_project(src)?;
    if dir.join("main.tex").exists() {
        return Err(format!("{} 已經有 main.tex", dir.display()));
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dir).map_err(io)?;
    copy_project(&f.base, dir, &f.used, false).map_err(io)?;
    let bbl = f.main.with_extension("bbl");
    if bbl.is_file() {
        std::fs::copy(&bbl, dir.join("main.bbl")).map_err(io)?;
    }
    write_split(dir, &f.split)
}

/// A LaTeX document (one file, \input already merged) split the way an
/// import splits it: (section file name, section) in order. Used to compare
/// two versions of the same document.
pub fn split_sections(text: &str) -> Vec<Section> {
    let Some(split) = split_sample(text, true) else { return vec![] };
    let mut out = Vec::new();
    for piece in &split.pieces {
        if let Piece::Section { title, tail, body } = piece {
            let file = format!("{:02}-{}.tex", out.len() + 1, slug(title));
            let mut s = parse_section(&file, &format!("\\section{{{title}}}{tail}\n{body}"));
            s.file = file;
            out.push(s);
        }
    }
    out
}

/// Optional clean-up after an import, offered to the user, never automatic.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tidy {
    /// Figures whose file names have spaces → `figures/figN.ext` (N = the
    /// figure's place in the document), paths relative to the paper.
    pub renames: Vec<(String, String)>,
    /// Class files the paper does not load.
    pub remove: Vec<String>,
    /// Lines to add to the paper's `.gitignore`: `*-eps-converted-to.pdf`
    /// when the paper has .eps figures. Those PDFs are build output that
    /// epstopdf writes next to the .eps on every compile, so they are kept
    /// (deleting them only makes the next export write them again) but
    /// not committed.
    #[serde(default)]
    pub gitignore: Vec<String>,
}

/// Build output that LaTeX (epstopdf) writes into the paper folder itself.
pub const CONVERTED_PATTERN: &str = "*-eps-converted-to.pdf";

impl Tidy {
    pub fn is_empty(&self) -> bool {
        self.renames.is_empty() && self.remove.is_empty() && self.gitignore.is_empty()
    }
}

/// `.gitignore` lines of a folder (trimmed).
fn gitignore_lines(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(".gitignore")).unwrap_or_default().lines().map(|l| l.trim().to_string()).collect()
}

/// The paper's own .tex files: main.tex and its sections.
fn paper_tex_files(paper: &Path) -> Vec<PathBuf> {
    let mut v = vec![paper.join("main.tex")];
    v.extend(section_files(paper).into_iter().map(|f| paper.join("sections").join(f)));
    v
}

/// `\includegraphics[..]{arg}` arguments in reading order, with the byte
/// range of each `{arg}` (comments skipped).
fn graphics_args(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let code = strip_comment(line);
        let mut from = 0;
        while let Some(i) = code[from..].find("\\includegraphics") {
            let mut j = from + i + "\\includegraphics".len();
            let rest = &code[j..];
            let r = rest.trim_start_matches('*').trim_start();
            j += rest.len() - r.len();
            let r = if r.starts_with('[') {
                match r.find(']') {
                    Some(e) => {
                        j += e + 1;
                        &r[e + 1..]
                    }
                    None => break,
                }
            } else {
                r
            };
            let lead = r.len() - r.trim_start().len();
            j += lead;
            match braced(&code[j..]) {
                Some((arg, _)) => {
                    out.push((at + j..at + j + arg.len() + 2, arg.to_string()));
                    from = j + arg.len() + 2;
                }
                None => break,
            }
        }
        at += line.len();
    }
    out
}

const FIGURE_EXT: &[&str] = &["pdf", "png", "jpg", "jpeg", "eps"];

/// The figure file an `\includegraphics` argument names (graphicx tries the
/// usual extensions when there is none).
fn figure_file(paper: &Path, arg: &str) -> Option<PathBuf> {
    if arg.contains("..") || arg.starts_with('/') {
        return None;
    }
    let p = paper.join(arg);
    if p.is_file() {
        return Some(p);
    }
    FIGURE_EXT.iter().map(|x| paper.join(format!("{arg}.{x}"))).find(|p| p.is_file())
}

/// What `apply_tidy` would do to this paper.
pub fn tidy_plan(paper: &Path) -> Tidy {
    let mut plan = Tidy::default();
    // figures, numbered by their first \includegraphics
    let mut seen: Vec<String> = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    for f in paper_tex_files(paper) {
        let Some(text) = read_tex(&f) else { continue };
        for (_, arg) in graphics_args(&text) {
            if seen.contains(&arg) {
                continue;
            }
            seen.push(arg.clone());
            let Some(file) = figure_file(paper, &arg) else { continue };
            let Ok(rel) = file.strip_prefix(paper) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !rel.contains(' ') || plan.renames.iter().any(|(from, _)| *from == rel) {
                continue;
            }
            let ext = file.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            let n = seen.len();
            let to = (1..)
                .map(|k| if k == 1 { format!("figures/fig{n}.{ext}") } else { format!("figures/fig{n}-{k}.{ext}") })
                .find(|t| !paper.join(t).exists() && !taken.contains(t))
                .expect("a free name");
            taken.push(to.clone());
            plan.renames.push((rel, to));
        }
    }
    // class files nobody loads
    let used: Vec<PathBuf> = crate::journal::used_style_files(paper);
    let mut classes: Vec<String> = used.iter().filter(|p| p.extension().is_some_and(|x| x == "cls")).filter_map(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).collect();
    // a class may load another (\LoadClass)
    let mut i = 0;
    while i < classes.len() {
        if let Some(src) = read_tex(&paper.join(format!("{}.cls", classes[i]))) {
            for line in src.lines() {
                let code = strip_comment(line);
                for cmd in ["\\LoadClassWithOptions", "\\LoadClass"] {
                    if let Some(r) = code.find(cmd).map(|k| &code[k + cmd.len()..]) {
                        let r = r.trim_start();
                        let r = if r.starts_with('[') { r.find(']').map_or("", |e| &r[e + 1..]) } else { r };
                        if let Some((c, _)) = braced(r.trim_start()) {
                            if !classes.iter().any(|x| x == c.trim()) {
                                classes.push(c.trim().to_string());
                            }
                        }
                        break;
                    }
                }
            }
        }
        i += 1;
    }
    let names: Vec<String> = std::fs::read_dir(paper).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    let mut remove: Vec<String> = Vec::new();
    for n in &names {
        if let Some(stem) = n.strip_suffix(".cls") {
            // Only when the main class is found here: otherwise nothing is known to be unused.
            if !classes.is_empty() && !classes.iter().any(|c| c == stem) {
                remove.push(n.clone());
            }
        }
    }
    remove.sort();
    plan.remove = remove;
    // .eps figures: epstopdf writes x-eps-converted-to.pdf beside them on each compile
    let has_eps = names.iter().any(|n| n.to_lowercase().ends_with(".eps") || n.ends_with("-eps-converted-to.pdf"))
        || paper.join("figures").read_dir().into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".eps"));
    if has_eps && !gitignore_lines(paper).iter().any(|l| l == CONVERTED_PATTERN) {
        plan.gitignore.push(CONVERTED_PATTERN.to_string());
    }
    plan
}

/// Rename figures (and rewrite their `\includegraphics`) and remove the
/// files in `plan`. Never overwrites a file; returns what was done.
pub fn apply_tidy(paper: &Path, plan: &Tidy) -> Result<Tidy, String> {
    let io = |e: std::io::Error| e.to_string();
    let mut done = Tidy::default();
    let mut moved: Vec<(String, String)> = Vec::new();
    for (from, to) in &plan.renames {
        let (src, dst) = (paper.join(from), paper.join(to));
        if !safe_rel(from) || !safe_rel(to) || !src.is_file() || dst.exists() {
            continue;
        }
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(io)?;
        }
        std::fs::rename(&src, &dst).map_err(io)?;
        moved.push((from.clone(), to.clone()));
        done.renames.push((from.clone(), to.clone()));
    }
    if !moved.is_empty() {
        for f in paper_tex_files(paper) {
            let Some(text) = read_tex(&f) else { continue };
            let mut out = String::new();
            let mut last = 0;
            for (range, arg) in graphics_args(&text) {
                let Some((_, to)) = moved.iter().find(|(from, _)| *from == arg || from.rsplit_once('.').is_some_and(|(stem, _)| stem == arg)) else { continue };
                // keep the argument's form: with an extension only if it had one
                let new = if moved.iter().any(|(from, _)| *from == arg) { to.clone() } else { to.rsplit_once('.').map_or(to.clone(), |(stem, _)| stem.to_string()) };
                out += &text[last..range.start];
                out += &format!("{{{new}}}");
                last = range.end;
            }
            if last > 0 {
                out += &text[last..];
                write_atomic(&f, &out).map_err(io)?;
            }
        }
    }
    for n in &plan.remove {
        let p = paper.join(n);
        if safe_name(n) && p.is_file() && n.ends_with(".cls") {
            std::fs::remove_file(&p).map_err(io)?;
            done.remove.push(n.clone());
        }
    }
    let have = gitignore_lines(paper);
    let add: Vec<&String> = plan.gitignore.iter().filter(|l| !l.trim().is_empty() && !l.contains('\n') && !have.contains(l)).collect();
    if !add.is_empty() {
        let mut text = std::fs::read_to_string(paper.join(".gitignore")).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        for l in add {
            text.push_str(l);
            text.push('\n');
            done.gitignore.push(l.clone());
        }
        write_atomic(&paper.join(".gitignore"), &text).map_err(io)?;
    }
    Ok(done)
}

/// A relative path inside the paper: plain names joined by `/`.
fn safe_rel(p: &str) -> bool {
    !p.is_empty() && p.split('/').all(safe_name)
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
    let ours = outline.iter().map(|s| Piece::Section { title: tex_text(&s.title), tail: String::new(), body: s.paragraphs().join("\n\n") + "\n" }).collect();
    replace_section_pieces(split, ours);
}

/// `ours` where the sample's body sections were; its later sections go too.
fn replace_section_pieces(split: &mut Sample, ours: Vec<Piece>) {
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
/// A file that exists keeps its permissions.
pub fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    write_atomic_bytes(path, content.as_bytes())
}

pub fn write_atomic_bytes(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp: PathBuf = path.with_file_name(format!(".{name}.tmp~"));
    std::fs::write(&tmp, content)?;
    if let Ok(m) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, m.permissions());
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// PaperComposer's own files of a paper (paper.toml, review.json,
/// citations.csv, notation.json, draft/, reference.docx) live in this
/// hidden folder when the paper has one. A paper opened where it is (a
/// co-author's project) gets it, so the project's top level shows only
/// LaTeX; papers made in the workspace keep these files at their top level.
pub const APP_DIR: &str = ".papercomposer";

pub fn app_dir(paper: &Path) -> PathBuf {
    let d = paper.join(APP_DIR);
    if d.is_dir() { d } else { paper.to_path_buf() }
}

/// What each reference is used for in this paper.
pub fn citations_file(paper: &Path) -> PathBuf {
    app_dir(paper).join("citations.csv")
}

/// The paper's sources as they are on disk now (main.tex and the section
/// files), by content: another editor or a `git pull` changes it.
pub fn stamp(paper: &Path) -> String {
    if crate::open::is_custom(paper) {
        return crate::open::stamp(paper);
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::fs::read(paper.join("main.tex")).unwrap_or_default().hash(&mut h);
    let mut files: Vec<String> = std::fs::read_dir(paper.join("sections"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".tex"))
        .collect();
    files.sort();
    for f in files {
        f.hash(&mut h);
        std::fs::read(paper.join("sections").join(&f)).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
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
    fn run_in_heads_and_text_around_environments_are_split() {
        let text = "\\section{Method}\n\n\\IEEEPARstart{W}{e} study a toy problem. It has two\nparts.\n\n\
\\textit{Data.} We use 120 synthetic images. Each one is $64\\times64$.\n\n\
\\noindent Only the first part is new. The rest follows [1].\n\n\
The loss is\n\\begin{equation}\nL = a. b = c.\n\\end{equation}\nwhere $a$ is small. It works\nwell. %keep this note\nThen we stop. Done.\n\n\
\\vspace{2mm} Kept as typed. Really.\n";
        let s = parse_section("method.tex", text);
        let out = render_section(&s);
        let want = "\\section{Method}\n\n\\IEEEPARstart{W}{e} study a toy problem.\nIt has two parts.\n\n\
\\textit{Data.} We use 120 synthetic images.\nEach one is $64\\times64$.\n\n\
\\noindent Only the first part is new.\nThe rest follows [1].\n\n\
The loss is\n\\begin{equation}\nL = a. b = c.\n\\end{equation}\nwhere $a$ is small.\nwell. %keep this note\nThen we stop.\nDone.\n\n\
\\vspace{2mm} Kept as typed. Really.\n";
        // The line with a comment stays whole, so the sentence before it
        // ends at that line's start.
        let want = want.replace("where $a$ is small.\nwell.", "where $a$ is small.\nIt works\nwell.");
        assert_eq!(out, want);
        // Saving again changes nothing, and only line breaks between words changed.
        assert_eq!(render_section(&parse_section("method.tex", &out)), out);
        let words = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(words(&out), words(text));
        assert_eq!(render_paragraph("a  \\verb|x  y|. B."), "a  \\verb|x  y|. B.", "\\verb keeps its spaces");
        // simple lists: each \item one sentence per line too; anything unusual stays as typed
        let list = "We list:\n\\begin{itemize}\n  \\item First point. It has two\n  parts.\n  \\item[(b)] Second one.\n\\end{itemize}";
        let got = render_paragraph(list);
        assert_eq!(got, "We list:\n\\begin{itemize}\n\\item First point.\nIt has two parts.\n\\item[(b)] Second one.\n\\end{itemize}");
        assert_eq!(render_paragraph(&got), got, "stable");
        assert_eq!(words(&got), words(list));
        let nested = "\\begin{itemize}\n\\item A. B.\n\\begin{enumerate}\n\\item C.\n\\end{enumerate}\n\\end{itemize}";
        assert_eq!(render_paragraph(nested), nested, "nested lists are left alone");
        let fig = "\\begin{figure}\n\\caption{One. Two.}\n\\end{figure}";
        assert_eq!(render_paragraph(fig), fig, "captions are not split");
        assert_eq!(render_paragraph("\\[ x. y \\]\nThen. Now."), "\\[ x. y \\]\nThen.\nNow.");
        assert_eq!(render_paragraph("\\enlargethispage{-\\baselineskip}\nOne. Two."), "\\enlargethispage{-\\baselineskip}\nOne.\nTwo.");
        assert_eq!(render_paragraph("\\vspace{2mm}\\noindent\nOne. Two."), "\\vspace{2mm}\\noindent\nOne.\nTwo.");
        assert_eq!(render_paragraph("\\label{x} One. Two."), "\\label{x} One. Two.", "a command followed by text on its line stays as typed");
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
    fn tidy_renames_spaced_figures_and_drops_unused_classes() {
        let root = std::env::temp_dir().join(format!("pc-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.tex"), "\\documentclass{wrapper}\n\\begin{document}\n\\section{Intro}\nSee the plot.\n\\begin{figure}\n\\includegraphics[width=\\linewidth]{Plot A}\n\\end{figure}\n\\begin{figure}\n\\includegraphics{logo}\\includegraphics*{Scan B.png}\n% \\includegraphics{Old C}\n\\end{figure}\n\\end{document}\n").unwrap();
        std::fs::write(src.join("wrapper.cls"), "\\LoadClass[journal]{base}\n").unwrap();
        std::fs::write(src.join("base.cls"), "% base").unwrap();
        std::fs::write(src.join("spare.cls"), "% spare").unwrap();
        std::fs::write(src.join("Plot A.pdf"), "pdf").unwrap();
        std::fs::write(src.join("Scan B.png"), "png").unwrap();
        std::fs::write(src.join("Old C.png"), "png").unwrap();
        std::fs::write(src.join("logo.eps"), "eps").unwrap();
        std::fs::write(src.join("logo-eps-converted-to.pdf"), "pdf").unwrap();
        std::fs::write(src.join("chart-eps-converted-to.pdf"), "no source: keep").unwrap();
        let dir = import_paper(&root.join("papers"), "P", &src).unwrap();
        let plan = tidy_plan(&dir);
        assert_eq!(plan.renames, [("Plot A.pdf".to_string(), "figures/fig1.pdf".to_string()), ("Scan B.png".into(), "figures/fig3.png".into())]);
        // the converted PDF is build output: kept, and ignored by git instead
        assert_eq!(plan.remove, ["spare.cls"]);
        assert_eq!(plan.gitignore, [CONVERTED_PATTERN]);
        let done = apply_tidy(&dir, &plan).unwrap();
        assert_eq!(done, plan);
        let sec = std::fs::read_to_string(dir.join("sections/01-intro.tex")).unwrap();
        assert!(sec.contains("\\includegraphics[width=\\linewidth]{figures/fig1}\n"), "{sec}");
        assert!(sec.contains("\\includegraphics{logo}\\includegraphics*{figures/fig3.png}"), "{sec}");
        assert!(sec.contains("% \\includegraphics{Old C}"), "comments untouched");
        assert!(dir.join("figures/fig1.pdf").is_file() && !dir.join("Plot A.pdf").exists());
        assert!(dir.join("base.cls").is_file() && dir.join("wrapper.cls").is_file() && !dir.join("spare.cls").exists());
        assert!(dir.join("chart-eps-converted-to.pdf").is_file() && dir.join("logo-eps-converted-to.pdf").is_file());
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), format!("{CONVERTED_PATTERN}\n"));
        assert!(tidy_plan(&dir).is_empty(), "nothing left to do");
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
    fn leaves_layout_macros_out() {
        let src = r"\makeatletter
\renewcommand{\thebibliography}[1]{%
  \section*{References}%
  \def\bibitem{\@mybibitem}%
  \list{\@biblabel{\@arabic\c@enumiv}}{}}
\makeatother
\renewcommand{\topfraction}{0.9}
\renewcommand{\floatpagefraction}{0.85}
\def\journalname{Journal of Examples}
\newcommand{\R}{\mathbb{R}}";
        let m = extract_macros(src);
        assert_eq!(m.keys().collect::<Vec<_>>(), ["\\R", "\\journalname"]);
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

    #[test]
    fn changing_the_template_keeps_every_written_word() {
        let root = std::env::temp_dir().join(format!("pc-retpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // a paper started on the plain article, written in, with the
        // author's notation and a table that needs booktabs
        let dir = create_paper(&root.join("papers"), "P").unwrap();
        std::fs::write(dir.join("notation.tex"), "% generated by lore-notation\n\\newcommand{\\vw}{\\mathbf{w}}\n").unwrap();
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        let main = main.replace("\\usepackage{amsmath,amssymb}\n", "\\usepackage{amsmath,amssymb}\n\\usepackage{booktabs}\n\\usepackage{fancyhdr}\n\\input{notation}\n\\newcommand{\\R}{\\mathbb{R}}\n");
        let main = crate::front::set_field(&main, "title", 0, "Weighted masks for uncertain labels").unwrap();
        let main = crate::front::set_field(&main, "abstract", 0, "We weight, not drop.").unwrap();
        std::fs::write(dir.join("main.tex"), main).unwrap();
        std::fs::write(dir.join("sections/method.tex"), "\\section{Method}\n\nThe weight $\\vw \\in \\R$ follows Eq.~\\eqref{eq:w}.\n\n\\begin{table}[t]\n\\centering\n\\begin{tabular}{ll}\\toprule\na & b\\\\\\bottomrule\n\\end{tabular}\n\\caption{Results.}\\label{tab:res}\n\\end{table}\n").unwrap();
        std::fs::write(dir.join("refs.bib"), "@article{kim2019,\n  title = {T},\n  year = {2019}\n}\n").unwrap();
        let read_secs = |d: &Path| section_files(d).iter().map(|f| (f.clone(), std::fs::read_to_string(d.join("sections").join(f)).unwrap())).collect::<Vec<_>>();
        let before = read_secs(&dir);
        assert_eq!(before.len(), 6);

        let tpl = root.join("publishers/demo");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("demo.cls"), "% class").unwrap();
        std::fs::write(tpl.join("refs.bib"), "@article{sample1, title={Sample}}\n").unwrap();
        std::fs::write(tpl.join("sample-refs.bib"), "@article{sample2, title={Sample}}\n").unwrap();
        std::fs::write(
            tpl.join("sample.tex"),
            "\\documentclass{demo}\n\\usepackage{cite}\n\\begin{document}\n\\title{Sample title}\\maketitle\n\\begin{abstract}Sample abstract.\\end{abstract}\n\\begin{keywords}k\\end{keywords}\n\
\\section{Introduction}\nSample intro.\n\n\\section{Methods}\nSample methods.\n\n\\bibliographystyle{demo}\n\\bibliography{sample-refs}\n\\appendix\n\\section{Extra}\nSample appendix.\n\\end{document}\n",
        )
        .unwrap();
        let old = apply_template(&dir, Some(&tpl)).unwrap();
        assert!(old.starts_with("\\documentclass{article}"));
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.starts_with("\\documentclass{demo}\n\\usepackage{cite}\n"), "{main}");
        assert!(!main.contains("Sample"), "the sample's text is gone: {main}");
        assert!(main.contains("\\title{Weighted masks for uncertain labels}") && main.contains("We weight, not drop."), "{main}");
        let inputs: String = before.iter().map(|(f, _)| format!("\\input{{sections/{}}}\n", f.trim_end_matches(".tex"))).collect();
        assert!(main.contains(&inputs), "the paper's sections, in order: {main}");
        assert!(main.contains("\\usepackage{amsmath}\n\\usepackage{amssymb}\n\\usepackage{booktabs}\n\\input{notation}\n\\newcommand{\\R}{\\mathbb{R}}\n\\begin{document}"), "{main}");
        assert!(!main.contains("fancyhdr"), "layout packages stay with the old template");
        assert!(main.contains("\\bibliography{sample-refs,refs}"), "{main}");
        assert!(main.find("\\bibliography").unwrap() < main.find("\\appendix").unwrap());
        assert_eq!(read_secs(&dir), before, "section files are not touched");
        assert!(std::fs::read_to_string(dir.join("notation.tex")).unwrap().starts_with("% generated by lore-notation"));
        assert!(std::fs::read_to_string(dir.join("refs.bib")).unwrap().contains("kim2019"), "the paper's .bib is never overwritten");
        assert!(dir.join("demo.cls").exists() && dir.join("sample-refs.bib").exists());
        assert_eq!(paper_macros(&dir).get("\\R").map(String::as_str), Some("\\mathbb{R}"));

        // and back to the plain article
        apply_template(&dir, None).unwrap();
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.starts_with("\\documentclass{article}") && main.contains(&inputs) && main.contains("\\input{notation}"), "{main}");
        assert!(main.contains("\\title{Weighted masks for uncertain labels}") && main.contains("We weight, not drop."));
        assert!(main.contains("\\bibliography{refs,") && !main.contains("\\usepackage{cite}"), "{main}");
        assert_eq!(read_secs(&dir), before);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_blank_line_in_a_paragraph_makes_two_paragraphs() {
        let s = Section {
            file: "a.tex".into(),
            title: "A".into(),
            paragraphs: vec![
                "First one. Still first.\n\nSecond one.\n\n\n".into(),
                "\\begin{equation}\nx = 1\n\nnot split inside\n\\end{equation}".into(),
                "Plain.".into(),
            ],
            tail: String::new(),
        };
        let text = render_section(&s);
        assert_eq!(text, "\\section{A}\n\nFirst one.\nStill first.\n\nSecond one.\n\n\\begin{equation}\nx = 1\n\nnot split inside\n\\end{equation}\n\nPlain.\n");
        let back = parse_section("a.tex", &text);
        assert_eq!(back.paragraphs.len(), 4);
        assert_eq!(render_section(&back), text, "reads back as written");
    }

    #[test]
    fn existing_section_files_render_unchanged() {
        let text = "\\section{Intro}\\label{sec:intro}\n\nOne.\nTwo.\n\n\\subsection{Sub}\n\n\\begin{itemize}\n\\item A.\n\\end{itemize}\n\n% a note\nText.\n";
        assert_eq!(render_section(&parse_section("i.tex", text)), text);
    }

    fn paper_with(main_body: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pc-secops-{}-{}", std::process::id(), files.len() * 31 + main_body.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        std::fs::write(dir.join("main.tex"), format!("\\documentclass{{article}}\n\\begin{{document}}\n{main_body}\\bibliography{{refs}}\n\\end{{document}}\n")).unwrap();
        for (f, t) in files {
            std::fs::write(dir.join("sections").join(f), format!("\\section{{{t}}}\n\nText of {t}.\n")).unwrap();
        }
        dir
    }
    fn main_of(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("main.tex")).unwrap()
    }

    #[test]
    fn sections_are_added_deleted_restored_and_reordered_in_main_tex() {
        let dir = paper_with(
            "\\input{sections/intro}\n\\input{sections/method}\n\\appendix\n  \\input{sections/extra}\n",
            &[("intro.tex", "Intro"), ("method.tex", "Method"), ("extra.tex", "Extra")],
        );
        // add after intro: own line, \label{sec:…}, empty
        let f = add_section(&dir, "Related work", Some("intro.tex")).unwrap();
        assert_eq!(f, "related-work.tex");
        assert!(main_of(&dir).contains("\\input{sections/intro}\n\\input{sections/related-work}\n\\input{sections/method}\n"), "{}", main_of(&dir));
        let s = &load_paper(&dir)[1];
        assert_eq!((s.title.as_str(), s.tail.as_str(), s.paragraphs.len()), ("Related work", "\\label{sec:related-work}", 0));
        // the same title again: new file and label
        assert_eq!(add_section(&dir, "Related work", None).unwrap(), "related-work-2.tex");
        assert!(main_of(&dir).contains("\\begin{document}\n\\input{sections/related-work-2}\n\\input{sections/intro}"), "first: {}", main_of(&dir));
        assert!(std::fs::read_to_string(dir.join("sections/related-work-2.tex")).unwrap().starts_with("\\section{Related work}\\label{sec:related-work-2}\n"));
        assert!(add_section(&dir, "Bad {title", None).is_err() && add_section(&dir, "  ", None).is_err());

        // delete: parked in sections/unused/, its line gone
        let parked = delete_section(&dir, "related-work-2.tex").unwrap();
        assert_eq!((parked.parked.as_str(), parked.after.as_deref()), ("related-work-2.tex", None));
        assert!(dir.join("sections/unused/related-work-2.tex").is_file() && !main_of(&dir).contains("related-work-2"));
        assert_eq!(section_files(&dir), ["intro.tex", "related-work.tex", "method.tex", "extra.tex"]);
        // and back where it was
        assert_eq!(restore_section(&dir, &parked).unwrap(), "related-work-2.tex");
        assert_eq!(section_files(&dir)[0], "related-work-2.tex");
        delete_section(&dir, "related-work-2.tex").unwrap();
        // a section after the appendix keeps its indent and place
        let before = main_of(&dir);
        let p2 = delete_section(&dir, "extra.tex").unwrap();
        assert!(main_of(&dir).contains("\\appendix\n\\bibliography"), "{}", main_of(&dir));
        restore_section(&dir, &p2).unwrap();
        assert_eq!(main_of(&dir), before, "an undo puts it back exactly, after the appendix");
        // the main file changed since: back after the section it followed
        let p3 = delete_section(&dir, "extra.tex").unwrap();
        std::fs::write(dir.join("main.tex"), main_of(&dir).replace("\\appendix\n", "\\appendix\n\\clearpage\n")).unwrap();
        restore_section(&dir, &p3).unwrap();
        assert!(main_of(&dir).contains("\\input{sections/method}\n\\input{sections/extra}\n\\appendix\n"), "restored after method: {}", main_of(&dir));

        // reorder: commands take the new order in turn, \appendix stays between slots
        let dir = paper_with(
            "\\input{sections/intro}\n\\input{sections/method}\n\\appendix\n  \\input{sections/extra.tex}\n",
            &[("intro.tex", "Intro"), ("method.tex", "Method"), ("extra.tex", "Extra"), ("zzz.tex", "Stray")],
        );
        let order: Vec<String> = ["method.tex", "intro.tex", "zzz.tex", "extra.tex"].map(String::from).to_vec();
        reorder_sections(&dir, &order).unwrap();
        assert!(main_of(&dir).contains("\\input{sections/method}\n\\input{sections/intro}\n\\appendix\n  \\input{sections/zzz}\n  \\input{sections/extra}\n"), "{}", main_of(&dir));
        assert_eq!(section_files(&dir), order);
        assert!(reorder_sections(&dir, &order[..2]).is_err(), "every section once");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_typed_section_heading_becomes_its_own_file() {
        let dir = paper_with("\\input{sections/intro}\n\\input{sections/end}\n", &[("intro.tex", "Intro"), ("end.tex", "End")]);
        let mut s = load_paper(&dir).remove(0);
        s.paragraphs.push("More intro.\n\n\\section{Methods}\nWe do it.\n\n\\section{Data}\\label{sec:data}\n\nRows.".into());
        write_atomic(&dir.join("sections/intro.tex"), &render_section(&s)).unwrap();
        assert_eq!(split_section(&dir, "intro.tex").unwrap(), ["methods.tex", "data.tex"]);
        let secs = load_paper(&dir);
        let names: Vec<_> = secs.iter().map(|s| (s.file.as_str(), s.title.as_str(), s.tail.as_str())).collect();
        assert_eq!(names, [("intro.tex", "Intro", ""), ("methods.tex", "Methods", "\\label{sec:methods}"), ("data.tex", "Data", "\\label{sec:data}"), ("end.tex", "End", "")]);
        assert_eq!(secs[0].paragraphs, ["Text of Intro.", "More intro."]);
        assert_eq!(secs[1].paragraphs, ["We do it."]);
        assert_eq!(secs[2].paragraphs, ["Rows."]);
        assert!(split_section(&dir, "intro.tex").unwrap().is_empty(), "nothing left to split");
        assert!(section_heading("\\section*{X}").is_none() && section_heading("\\section{X} and text").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn new_sections_pass_the_notation_checks() {
        use lore_notation::{check_document, CheckOptions, Document, Notation};
        let dir = paper_with("\\input{sections/intro}\n", &[("intro.tex", "Intro")]);
        add_section(&dir, "Method", Some("intro.tex")).unwrap();
        let mut s = load_paper(&dir).remove(1);
        s.paragraphs.push("As Section~\\ref{sec:method} shows, it works.".into());
        write_atomic(&dir.join("sections").join(&s.file), &render_section(&s)).unwrap();
        let mut files = vec![("main.tex".to_string(), main_of(&dir))];
        for f in section_files(&dir) {
            files.push((format!("sections/{f}"), std::fs::read_to_string(dir.join("sections").join(&f)).unwrap()));
        }
        let issues = check_document(Document::from_files(&files), &Notation::default(), &CheckOptions::default()).issues;
        let mut wrong = files.clone();
        wrong[2].1.push_str("\nSee Section~\\ref{sec:nope}.\n");
        let caught = check_document(Document::from_files(&wrong), &Notation::default(), &CheckOptions::default()).issues;
        assert!(caught.iter().any(|i| i.code.starts_with("ref_undefined")), "the check does look at section labels");
        assert!(!issues.iter().any(|i| i.code.starts_with("ref_undefined") || i.code.starts_with("label_")), "{:?}", issues.iter().map(|i| &i.code).collect::<Vec<_>>());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn section_operations_keep_an_opened_projects_own_files() {
        let dir = std::env::temp_dir().join(format!("pc-secops-custom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("chapters")).unwrap();
        std::fs::create_dir_all(dir.join(APP_DIR)).unwrap();
        std::fs::write(dir.join(APP_DIR).join(crate::open::LAYOUT_FILE), "main = \"ms.tex\"\n").unwrap();
        let ms = "\\documentclass{article}\n\\begin{document}\n\\input{abstract}\n\\section{Intro}\\label{sec:intro}\n\\input{intro}\n\\input{chapters/results.tex}\n\\appendix\n\\input{chapters/extra}\n\\end{document}\n";
        std::fs::write(dir.join("ms.tex"), ms).unwrap();
        std::fs::write(dir.join("abstract.tex"), "An abstract.\n").unwrap();
        std::fs::write(dir.join("intro.tex"), "Intro text.\n").unwrap();
        std::fs::write(dir.join("chapters/results.tex"), "\\section{Results}\\label{sec:r}\n\nWe found it.\n").unwrap();
        std::fs::write(dir.join("chapters/extra.tex"), "\\section{Extra}\n\nMore.\n").unwrap();
        let ms_of = || std::fs::read_to_string(dir.join("ms.tex")).unwrap();
        let ids = || load_paper(&dir).into_iter().map(|s| s.file).collect::<Vec<_>>();
        assert_eq!(ids(), ["intro.tex", "chapters/results.tex", "chapters/extra.tex"]);

        // add: next to the section it follows, in ms.tex's own \input style
        let id = add_section(&dir, "Method", Some("chapters/results.tex")).unwrap();
        assert_eq!(id, "chapters/method.tex");
        assert!(ms_of().contains("\\input{chapters/results.tex}\n\\input{chapters/method.tex}\n\\appendix"), "{}", ms_of());
        assert!(!dir.join("main.tex").exists() && !dir.join("sections").exists(), "no main.tex, no sections/ in the author's project");

        // reorder: the title typed in ms.tex moves with its file
        reorder_sections(&dir, &["chapters/results.tex", "intro.tex", "chapters/method.tex", "chapters/extra.tex"].map(String::from)).unwrap();
        assert!(ms_of().contains("\\input{abstract}\n\\input{chapters/results.tex}\n\\section{Intro}\\label{sec:intro}\n\\input{intro}\n\\input{chapters/method.tex}\n\\appendix\n\\input{chapters/extra}\n"), "{}", ms_of());
        assert_eq!(load_paper(&dir)[1].title, "Intro");

        // delete waits in .papercomposer/unused/, and an undo puts back the title line too
        let before = ms_of();
        let p = delete_section(&dir, "intro.tex").unwrap();
        assert!(!ms_of().contains("Intro") && dir.join(APP_DIR).join("unused/intro.tex").is_file() && !dir.join("intro.tex").exists());
        assert!(p.slot.starts_with("\\section{Intro}"), "{}", p.slot);
        restore_section(&dir, &p).unwrap();
        assert_eq!(ms_of(), before);
        assert_eq!(load_paper(&dir)[1].title, "Intro");

        // a typed \section in a file whose title is in ms.tex: the body file stays body-only
        let mut s = load_paper(&dir).remove(1);
        s.paragraphs.push("\\section{Scope}".into());
        s.paragraphs.push("Scope text.".into());
        crate::open::write_section(&dir, &s).unwrap();
        assert_eq!(split_section(&dir, "intro.tex").unwrap(), ["scope.tex"]);
        assert_eq!(std::fs::read_to_string(dir.join("intro.tex")).unwrap().trim(), "Intro text.");
        assert!(ms_of().contains("\\input{intro}\n\\input{scope}\n"), "{}", ms_of());
        assert_eq!(load_paper(&dir).iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Results", "Intro", "Scope", "Method", "Extra"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
