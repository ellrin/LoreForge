//! The title block in main.tex: title, authors, affiliations, abstract and
//! keywords, found the way each class writes them, and edited in place.
//! Only the content of a field is replaced; the rest of main.tex is kept
//! byte for byte.

use serde::Serialize;
use std::ops::Range;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Field {
    /// "title" | "author" | "affil" | "abstract" | "keywords"
    pub field: String,
    /// Which one, for fields that repeat (Springer has one \author per person).
    pub index: usize,
    pub value: String,
}

/// main.tex with comments blanked out (same byte offsets), so searches skip them.
pub fn masked(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.split_inclusive('\n') {
        let b = line.as_bytes();
        let cut = (0..b.len()).find(|&i| b[i] == b'%' && (i == 0 || b[i - 1] != b'\\'));
        match cut {
            Some(i) => {
                out.push_str(&line[..i]);
                let rest = &line[i..];
                out.extend(rest.bytes().map(|c| if c == b'\n' { '\n' } else { ' ' }));
            }
            None => out.push_str(line),
        }
    }
    out
}

/// End (exclusive) of the `{...}` group opening at `open`.
pub(crate) fn group_end(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The part of the main file that holds the title block: up to the first
/// section (an `\input{sections/…}`, an `\include`, or a `\section` typed in
/// a paper opened as it is).
fn front_limit(m: &str) -> usize {
    let body = m.find("\\begin{document}").unwrap_or(0);
    // only at the top level: `\abstract{\input{sections/01-abstract}}` is still the title block
    let top = |at: usize| depth_at(m, at) == 0;
    ["\\input{sections/", "\\include{", "\\section"]
        .iter()
        .filter_map(|k| m[body..].match_indices(k).map(|(i, _)| body + i).find(|&i| top(i)))
        .min()
        .unwrap_or(m.len())
}

/// Brace depth of `m` at byte `at` (escaped braces skipped; comments are masked already).
pub(crate) fn depth_at(m: &str, at: usize) -> i32 {
    let b = m.as_bytes();
    // from the document's start when `at` is in it: a stray brace in the preamble never hides the body
    let from = m.find("\\begin{document}").filter(|&d| d <= at).unwrap_or(0);
    let (mut depth, mut i) = (0i32, from);
    while i < at.min(b.len()) {
        match b[i] {
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    depth.max(0)
}

/// Content ranges of `\cmd*[opt]{content}`.
fn command_ranges(m: &str, cmd: &str, limit: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < limit
        && let Some(i) = m[at..limit].find(cmd)
    {
        let start = at + i;
        let mut j = start + cmd.len();
        at = j;
        if m[j..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let b = m.as_bytes();
        while j < limit && (b[j] == b'*' || b[j] == b' ') {
            j += 1;
        }
        if j < limit && b[j] == b'[' {
            match m[j..limit].find(']') {
                Some(e) => j += e + 1,
                None => continue,
            }
        }
        while j < limit && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < limit && b[j] == b'{' {
            if let Some(end) = group_end(m, j) {
                out.push(j + 1..end - 1);
                at = end;
            }
        }
    }
    out
}

/// Content ranges of `\begin{env}content\end{env}`.
fn env_ranges(m: &str, env: &str, limit: usize) -> Vec<Range<usize>> {
    let (open, close) = (format!("\\begin{{{env}}}"), format!("\\end{{{env}}}"));
    let mut out = Vec::new();
    let mut at = 0;
    while at < limit
        && let Some(i) = m[at..limit].find(&open)
    {
        let start = at + i + open.len();
        match m[start..].find(&close) {
            Some(e) => {
                out.push(start..start + e);
                at = start + e + close.len();
            }
            None => break,
        }
    }
    out
}

fn ranges(m: &str, field: &str) -> Vec<Range<usize>> {
    let limit = front_limit(m);
    let mut v = match field {
        "title" => command_ranges(m, "\\title", limit),
        "author" => [command_ranges(m, "\\author", limit), command_ranges(m, "\\IEEEauthorblockN", limit)].concat(),
        "affil" => ["\\affil", "\\address", "\\institute", "\\IEEEauthorblockA"].iter().flat_map(|c| command_ranges(m, c, limit)).collect(),
        "abstract" => [env_ranges(m, "abstract", limit), command_ranges(m, "\\abstract", limit)].concat(),
        "keywords" => [env_ranges(m, "IEEEkeywords", limit), env_ranges(m, "keywords", limit), command_ranges(m, "\\keywords", limit)].concat(),
        _ => vec![],
    };
    v.sort_by_key(|r| r.start);
    v
}

pub const FIELDS: &[&str] = &["title", "author", "affil", "abstract", "keywords"];

/// Every title-block field in main.tex, in order of appearance per kind.
pub fn fields(main: &str) -> Vec<Field> {
    let m = masked(main);
    FIELDS
        .iter()
        .flat_map(|f| {
            ranges(&m, f).into_iter().enumerate().map(|(index, r)| Field { field: f.to_string(), index, value: main[r].trim().to_string() })
        })
        .collect()
}

fn balanced(s: &str) -> bool {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    depth == 0
}

/// main.tex with one field's content replaced. A missing abstract is added
/// after \maketitle (or at the start of the document).
pub fn set_field(main: &str, field: &str, index: usize, value: &str) -> Result<String, String> {
    if !FIELDS.contains(&field) {
        return Err("unknown field".into());
    }
    let value = value.trim();
    if !balanced(value) {
        return Err("大括號 { } 沒有成對".into());
    }
    if value.contains("\\end{") && !value.contains("\\begin{") {
        return Err("內容不能有多出來的 \\end{…}".into());
    }
    let m = masked(main);
    let found = ranges(&m, field);
    if let Some(r) = found.get(index) {
        // multi-line blocks keep their own lines; one-liners stay on one line
        let inner = if main[r.clone()].contains('\n') { format!("\n{value}\n") } else { value.to_string() };
        return Ok(format!("{}{}{}", &main[..r.start], inner, &main[r.end..]));
    }
    if field == "abstract" && found.is_empty() {
        let at = m.find("\\maketitle").map(|i| i + "\\maketitle".len())
            .or_else(|| m.find("\\begin{document}").map(|i| i + "\\begin{document}".len()))
            .ok_or("main.tex 沒有 \\begin{document}")?;
        return Ok(format!("{}\n\n\\begin{{abstract}}\n{value}\n\\end{{abstract}}{}", &main[..at], &main[at..]));
    }
    Err("main.tex 裡找不到這個欄位".into())
}

/// Text a class's sample puts in a field (never the author's): Springer's showcase abstract,
/// `keyword1, Keyword2, …`, Elsevier's "Lorem ipsum".
pub fn is_sample_text(v: &str) -> bool {
    let t = v.to_lowercase();
    ["the abstract serves both as a general introduction", "keyword1", "lorem ipsum", "this is the abstract", "abstract text goes here"].iter().any(|k| t.contains(k))
}

/// The file a field reads, when its whole content is `\input{sections/01-abstract}` (safe paths only).
pub fn input_file(value: &str) -> Option<String> {
    let f = value.trim().strip_prefix("\\input{")?.strip_suffix('}')?.trim();
    let rel = if f.ends_with(".tex") { f.to_string() } else { format!("{f}.tex") };
    (!rel.is_empty() && !rel.contains("..") && !rel.starts_with('/') && !rel.contains('\\')).then_some(rel)
}

/// [`fields`], with a field that only `\input`s a file shown as that file's text.
pub fn fields_in(dir: &std::path::Path, main: &str) -> Vec<Field> {
    fields(main)
        .into_iter()
        .map(|mut f| {
            if let Some(rel) = input_file(&f.value) {
                f.value = std::fs::read_to_string(dir.join(rel)).map(|t| t.trim().to_string()).unwrap_or_default();
            }
            f
        })
        .collect()
}

/// [`set_field`] for a paper folder: a field that `\input`s a file gets the file written instead
/// (main.tex unchanged, the author's layout kept). Returns the new main.tex.
pub fn set_field_in(dir: &std::path::Path, main: &str, field: &str, index: usize, value: &str) -> Result<String, String> {
    let current = fields(main).into_iter().filter(|f| f.field == field).nth(index);
    if let Some(rel) = current.and_then(|f| input_file(&f.value)) {
        if !balanced(value.trim()) {
            return Err("大括號 { } 沒有成對".into());
        }
        std::fs::write(dir.join(rel), format!("{}\n", value.trim())).map_err(|e| e.to_string())?;
        return Ok(main.to_string());
    }
    set_field(main, field, index, value)
}

// ── 雙盲：盲審版 ──────────────────────────────────────────

/// The convention of a hand-set double-blind main.tex: `\blinded{full}{blinded}` picks one text,
/// `\blindmode` (defined before the class) picks the blinded build.
pub const BLINDED_MACRO: &str = "\\providecommand{\\blinded}[2]{\\ifdefined\\blindmode #2\\else #1\\fi}";

/// Is the author block already switched by `\blindmode` (`\ifdefined\blindmode\else … \fi`)?
pub fn has_blind_switch(main: &str) -> bool {
    // in the document, not the `\blinded` macro's definition in the preamble
    let m = masked(main);
    let body = m.find("\\begin{document}").unwrap_or(0);
    m[body..].contains("\\ifdefined\\blindmode")
}

/// The commands of the title block that name people (cut whole in the blinded build).
const IDENTIFYING: &[&str] = &["\\author", "\\affil", "\\address", "\\institute", "\\email", "\\equalcont", "\\IEEEauthorblockN", "\\IEEEauthorblockA", "\\orcid", "\\corref", "\\ead", "\\fntext", "\\cortext", "\\thanks"];

/// Ranges of whole commands `\cmd*[opt]{…}` (with any further `{…}` groups) in the title block.
fn whole_commands(m: &str, cmd: &str, limit: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let b = m.as_bytes();
    let mut at = 0;
    while at < limit
        && let Some(i) = m[at..limit].find(cmd)
    {
        let start = at + i;
        let mut j = start + cmd.len();
        at = j;
        if m[j..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        while j < limit && (b[j] == b'*' || b[j] == b' ') {
            j += 1;
        }
        while j < limit && b[j] == b'[' {
            match m[j..limit].find(']') {
                Some(e) => j += e + 1,
                None => break,
            }
        }
        let mut end = None;
        while j < limit && b[j] == b'{' {
            match group_end(m, j) {
                Some(e) => {
                    end = Some(e);
                    j = e;
                }
                None => break,
            }
        }
        if let Some(e) = end {
            out.push(start..e);
            at = e;
        }
    }
    out
}

/// main.tex for the blinded build: `\blindmode` on, the `\blinded` macro available, and, unless the
/// author block is already switched by `\blindmode`, every author / affiliation / e-mail command of the
/// title block cut out. The rest of the file is kept byte for byte.
pub fn blind_main(main: &str) -> String {
    let mut out = main.to_string();
    if !has_blind_switch(main) {
        let m = masked(main);
        let limit = front_limit(&m);
        let mut cuts: Vec<Range<usize>> = IDENTIFYING.iter().flat_map(|c| whole_commands(&m, c, limit)).collect();
        // the title block only: from \begin{document} or the first \title (classes that take authors in the preamble)
        cuts.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in cuts {
            match merged.last_mut() {
                Some(l) if r.start < l.end => l.end = l.end.max(r.end),
                _ => merged.push(r),
            }
        }
        for r in merged.into_iter().rev() {
            out.replace_range(r, "");
        }
    }
    let defs = if masked(&out).contains("\\blinded") && !masked(&out).contains("\\newcommand{\\blinded}") { format!("\\def\\blindmode{{}}\n{BLINDED_MACRO}\n") } else { "\\def\\blindmode{}\n".to_string() };
    defs + &out
}

/// Strings of `people` (names, e-mails, organisations) that a blinded PDF's text still has.
pub fn identity_leaks(pdf_text: &str, strings: &[String]) -> Vec<String> {
    let flat: String = pdf_text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let mut out: Vec<String> = strings
        .iter()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| s.chars().filter(|c| c.is_alphanumeric()).count() >= 4 && flat.contains(&s.to_lowercase()))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const IEEE: &str = "\\documentclass[journal]{IEEEtran}\n\\begin{document}\n\\title{Old {\\LaTeX} Title}\n%\\title{commented}\n\\author{A.~One, \\IEEEmembership{Member, IEEE}\\thanks{T.}}\n\\maketitle\n\\begin{abstract}\nFirst.\nSecond.\n\\end{abstract}\n\\begin{IEEEkeywords}\nx, y\n\\end{IEEEkeywords}\n\\input{sections/01-intro}\n\\begin{abstract}not front\\end{abstract}\n\\end{document}\n";

    #[test]
    fn finds_ieee_fields_skipping_comments_and_body() {
        let f = fields(IEEE);
        let get = |k: &str| f.iter().filter(|x| x.field == k).map(|x| x.value.as_str()).collect::<Vec<_>>();
        assert_eq!(get("title"), ["Old {\\LaTeX} Title"]);
        assert_eq!(get("author"), ["A.~One, \\IEEEmembership{Member, IEEE}\\thanks{T.}"]);
        assert_eq!(get("abstract"), ["First.\nSecond."]);
        assert_eq!(get("keywords"), ["x, y"]);
    }

    #[test]
    fn edits_one_field_and_nothing_else() {
        let out = set_field(IEEE, "title", 0, "New Title").unwrap();
        assert_eq!(out, IEEE.replace("Old {\\LaTeX} Title", "New Title"));
        let out = set_field(IEEE, "abstract", 0, "Only one.").unwrap();
        assert!(out.contains("\\begin{abstract}\nOnly one.\n\\end{abstract}"));
        assert!(out.contains("not front"), "the body is untouched");
        assert!(set_field(IEEE, "title", 0, "Bad {").is_err());
    }

    #[test]
    fn springer_commands_and_repeated_authors() {
        let sn = "\\documentclass{sn-jnl}\n\\begin{document}\n\\title[Short]{Long Title}\n\\author*[1]{\\fnm{A} \\sur{One}}\\email{a@x}\n\\author[2]{\\fnm{B} \\sur{Two}}\n\\affil*[1]{\\orgname{U1}}\n\\abstract{Abs {text}.}\n\\keywords{k1, k2}\n\\maketitle\n\\input{sections/01-a}\n\\end{document}\n";
        let f = fields(sn);
        assert_eq!(f.iter().filter(|x| x.field == "author").count(), 2);
        assert_eq!(f.iter().find(|x| x.field == "title").unwrap().value, "Long Title");
        assert_eq!(f.iter().find(|x| x.field == "abstract").unwrap().value, "Abs {text}.");
        let out = set_field(sn, "author", 1, "\\fnm{C} \\sur{Three}").unwrap();
        assert!(out.contains("\\author[2]{\\fnm{C} \\sur{Three}}") && out.contains("\\author*[1]{\\fnm{A} \\sur{One}}"));
    }

    #[test]
    fn an_abstract_that_inputs_a_section_file_is_front_matter() {
        // a hand-set Springer main.tex: this used to panic (byte range past the front limit)
        let sn = "\\documentclass{sn-jnl}\n\\begin{document}\n\\title{T}\n\\abstract{\\input{sections/01-abstract}}\n\\keywords{k1, k2}\n\\maketitle\n\\input{sections/02-introduction}\n\\end{document}\n";
        let f = fields(sn);
        assert_eq!(f.iter().find(|x| x.field == "abstract").unwrap().value, "\\input{sections/01-abstract}");
        assert_eq!(f.iter().find(|x| x.field == "keywords").unwrap().value, "k1, k2");
        assert!(set_field(sn, "keywords", 0, "a, b").unwrap().contains("\\keywords{a, b}"));
    }

    #[test]
    fn adds_a_missing_abstract_after_maketitle() {
        let blank = "\\documentclass{article}\n\\title{T}\n\\author{}\n\\begin{document}\n\\maketitle\n\n\\input{sections/introduction}\n\\end{document}\n";
        let out = set_field(blank, "abstract", 0, "We show.").unwrap();
        assert!(out.contains("\\maketitle\n\n\\begin{abstract}\nWe show.\n\\end{abstract}\n\n\\input"), "{out}");
        // title in the preamble is found too
        assert_eq!(fields(blank).iter().find(|x| x.field == "title").unwrap().value, "T");
        assert_eq!(set_field(blank, "author", 0, "Me").unwrap(), blank.replace("\\author{}", "\\author{Me}"));
    }

    #[test]
    fn the_blinded_build_drops_people_and_keeps_the_rest() {
        let sn = "\\documentclass{sn-jnl}\n\\begin{document}\n\\title[Short]{Long Title}\n\\author*[1]{\\fnm{A} \\sur{One}}\\email{a@x.org}\n\\author[2]{\\fnm{B} \\sur{Two}}\n\\equalcont{Equal.}\n\\affil*[1]{\\orgdiv{D}, \\orgname{U1}}\n\\abstract{Abs.}\n\\keywords{k1}\n\\maketitle\n\\input{sections/01-a}\n\\bmhead{Acknowledgements}\n\\blinded{Thanks B.}{Removed.}\n\\end{document}\n";
        let b = blind_main(sn);
        assert!(b.starts_with("\\def\\blindmode{}\n\\providecommand{\\blinded}"), "{b}");
        for gone in ["\\author", "\\affil", "\\email", "\\equalcont", "One", "U1"] {
            assert!(!b.contains(gone), "{gone} in {b}");
        }
        for kept in ["\\title[Short]{Long Title}", "\\abstract{Abs.}", "\\keywords{k1}", "\\input{sections/01-a}", "\\blinded{Thanks B.}{Removed.}"] {
            assert!(b.contains(kept), "{kept} missing in {b}");
        }
        // the author's own convention: the author block is switched already, nothing is cut
        let own = "\\documentclass{sn-jnl}\n\\newcommand{\\blinded}[2]{\\ifdefined\\blindmode #2\\else #1\\fi}\n\\begin{document}\n\\ifdefined\\blindmode\\else\n\\author[1]{\\fnm{A} \\sur{One}}\n\\fi\n\\maketitle\n\\end{document}\n";
        assert_eq!(blind_main(own), format!("\\def\\blindmode{{}}\n{own}"));
        // the \blinded macro in the preamble is not a switched author block
        let made = format!("\\documentclass{{sn-jnl}}\n{BLINDED_MACRO}\n\\begin{{document}}\n\\author{{\\fnm{{A}} \\sur{{One}}}}\n\\maketitle\n\\end{{document}}\n");
        assert!(!has_blind_switch(&made) && !blind_main(&made).contains("One"));
        // IEEE: the block with its \thanks
        let ieee = blind_main(IEEE);
        assert!(!ieee.contains("A.~One") && ieee.contains("\\begin{abstract}"), "{ieee}");
        assert_eq!(identity_leaks("by A. One\nExample U  City", &["A. One".into(), "Nowhere U".into(), "B".into()]), ["A. One"]);
    }
}

