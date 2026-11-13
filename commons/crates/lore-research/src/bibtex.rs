//! BibTeX for records. Keys: `doi:<doi>` when there is a DOI, else `arXiv:<id>`, else
//! `<first author's last name><year><first title word>`; characters BibTeX / `\cite` cannot take
//! become `-`; a key already used in the set gets `a`, `b`, … Field values escape LaTeX specials.
//! No macros are written: escaping only.

use std::collections::HashSet;

use crate::Record;

/// Words skipped as "first title word".
const STOP: &[&str] = &["a", "an", "the", "on", "of", "in", "for", "to", "and", "with", "towards", "toward", "via", "from", "by", "at", "is", "are"];

fn key_safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '/' | '-' | '_') { c } else { '-' }).collect()
}

fn ascii_word(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// The last name of "First Last" or "Last, First".
fn last_name(author: &str) -> &str {
    match author.split_once(',') {
        Some((last, _)) => last.trim(),
        None => author.split_whitespace().last().unwrap_or(""),
    }
}

/// The stable key of one record (before de-duplication within a set).
pub fn key(r: &Record) -> String {
    if let Some(d) = r.doi.as_deref().filter(|d| !d.is_empty()) {
        return format!("doi:{}", key_safe(&d.to_ascii_lowercase()));
    }
    if let Some(a) = r.arxiv.as_deref().filter(|a| !a.is_empty()) {
        return format!("arXiv:{}", key_safe(a));
    }
    let author = r.authors.first().map(|a| ascii_word(last_name(a))).filter(|a| !a.is_empty()).unwrap_or_else(|| "anon".into());
    let year = r.year.map(|y| y.to_string()).unwrap_or_default();
    let word = r.title.split_whitespace().map(ascii_word).find(|w| !w.is_empty() && !STOP.contains(&w.as_str())).unwrap_or_default();
    format!("{author}{year}{word}")
}

/// Keys for a set, in order, none repeated (`x`, `xa`, `xb`, …).
pub fn keys(records: &[Record]) -> Vec<String> {
    let mut used = HashSet::new();
    records
        .iter()
        .map(|r| {
            let base = key(r);
            let mut k = base.clone();
            let mut n = 0u32;
            while !used.insert(k.clone()) {
                k = format!("{base}{}", suffix(n));
                n += 1;
            }
            k
        })
        .collect()
}

fn suffix(mut n: u32) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        if n < 26 {
            return s;
        }
        n = n / 26 - 1;
    }
}

/// Escape LaTeX specials in a field value (`& % $ # _ { } ~ ^ \`).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '\\' => out.push_str("\\textbackslash{}"),
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// One entry: `@article` when there is a venue, else `@misc` (arXiv: `eprint`, `archivePrefix`).
pub fn entry(r: &Record, key: &str) -> String {
    let kind = if r.venue.is_some() { "article" } else { "misc" };
    let mut f: Vec<(&str, String)> = Vec::new();
    // braces keep the title's capitals
    f.push(("title", format!("{{{}}}", escape(&r.title))));
    if !r.authors.is_empty() {
        f.push(("author", r.authors.iter().map(|a| escape(a)).collect::<Vec<_>>().join(" and ")));
    }
    if let Some(y) = r.year {
        f.push(("year", y.to_string()));
    }
    if let Some(v) = &r.venue {
        f.push(("journal", escape(v)));
    }
    if let Some(d) = &r.doi {
        f.push(("doi", escape(d)));
    }
    if let Some(a) = &r.arxiv {
        f.push(("eprint", escape(a)));
        f.push(("archivePrefix", "arXiv".into()));
    }
    if let Some(u) = r.doi.as_ref().map(|d| format!("https://doi.org/{d}")).or_else(|| r.url.clone()) {
        // url fields are verbatim in biblatex / url.sty: only braces and % need care
        f.push(("url", u.replace(['{', '}'], "").replace('%', "\\%")));
    }
    let mut out = format!("@{kind}{{{key},\n");
    for (k, v) in f {
        out.push_str(&format!("  {k} = {{{v}}},\n"));
    }
    out.push_str("}\n");
    out
}

/// A `.bib` for a set of records with unique keys; returns `(keys, text)`.
pub fn bib(records: &[Record]) -> (Vec<String>, String) {
    let ks = keys(records);
    let text = records.iter().zip(&ks).map(|(r, k)| entry(r, k)).collect::<Vec<_>>().join("\n");
    (ks, text)
}
