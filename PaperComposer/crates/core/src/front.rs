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
pub(crate) fn masked(src: &str) -> String {
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
    ["\\input{sections/", "\\include{", "\\section"].iter().filter_map(|k| m[body..].find(k).map(|i| body + i)).min().unwrap_or(m.len())
}

/// Content ranges of `\cmd*[opt]{content}`.
fn command_ranges(m: &str, cmd: &str, limit: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = m[at..limit].find(cmd) {
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
    while let Some(i) = m[at..limit].find(&open) {
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
    fn adds_a_missing_abstract_after_maketitle() {
        let blank = "\\documentclass{article}\n\\title{T}\n\\author{}\n\\begin{document}\n\\maketitle\n\n\\input{sections/introduction}\n\\end{document}\n";
        let out = set_field(blank, "abstract", 0, "We show.").unwrap();
        assert!(out.contains("\\maketitle\n\n\\begin{abstract}\nWe show.\n\\end{abstract}\n\n\\input"), "{out}");
        // title in the preamble is found too
        assert_eq!(fields(blank).iter().find(|x| x.field == "title").unwrap().value, "T");
        assert_eq!(set_field(blank, "author", 0, "Me").unwrap(), blank.replace("\\author{}", "\\author{Me}"));
    }
}
