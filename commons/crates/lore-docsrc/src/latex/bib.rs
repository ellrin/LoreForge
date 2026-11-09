//! BibTeX entries and `\bibitem` lists.

use super::plain::{PlainOpts, to_plain};
use super::scan::{find_command, group_text, read_group, read_optional, skip_ws};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct BibEntry {
    pub entry_type: String,
    pub key: String,
    /// Lower-case field names; values as written, outer braces/quotes removed.
    pub fields: BTreeMap<String, String>,
}

/// Parse a `.bib` file (unknown syntax is skipped, never fatal).
pub fn parse_bibtex(src: &str) -> Vec<BibEntry> {
    let b = src.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while let Some(at) = src[i..].find('@') {
        let start = i + at + 1;
        let ty_len = src[start..]
            .bytes()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
        let ty = src[start..start + ty_len].to_lowercase();
        let p = skip_ws(src, start + ty_len);
        if p >= b.len() || (b[p] != b'{' && b[p] != b'(') || ty.is_empty() {
            i = start;
            continue;
        }
        let Some((inner_s, inner_e, after)) = read_group(src, p).or_else(|| paren_group(src, p))
        else {
            i = start;
            continue;
        };
        i = after;
        if matches!(ty.as_str(), "comment" | "string" | "preamble") {
            continue;
        }
        let inner = &src[inner_s..inner_e];
        let Some(comma) = inner.find(',') else {
            continue;
        };
        let key = inner[..comma].trim().to_string();
        let fields = parse_fields(&inner[comma + 1..]);
        out.push(BibEntry {
            entry_type: ty,
            key,
            fields,
        });
    }
    out
}

fn paren_group(s: &str, p: usize) -> Option<(usize, usize, usize)> {
    let mut depth = 0;
    for (k, c) in s[p..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((p + 1, p + k, p + k + 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_fields(s: &str) -> BTreeMap<String, String> {
    let b = s.as_bytes();
    let mut out = BTreeMap::new();
    let mut i = 0;
    loop {
        i = skip_ws(s, i);
        while i < b.len() && b[i] == b',' {
            i = skip_ws(s, i + 1);
        }
        let name_len = s[i..]
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
            .count();
        if name_len == 0 {
            break;
        }
        let name = s[i..i + name_len].to_lowercase();
        i = skip_ws(s, i + name_len);
        if i >= b.len() || b[i] != b'=' {
            break;
        }
        i = skip_ws(s, i + 1);
        let mut value = String::new();
        loop {
            if i >= b.len() {
                break;
            }
            match b[i] {
                b'{' => {
                    let Some((a, e, after)) = read_group(s, i) else {
                        break;
                    };
                    value.push_str(&s[a..e]);
                    i = after;
                }
                b'"' => {
                    let mut j = i + 1;
                    let mut depth = 0;
                    while j < b.len() {
                        match b[j] {
                            b'{' => depth += 1,
                            b'}' => depth -= 1,
                            b'"' if depth == 0 && b[j - 1] != b'\\' => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    value.push_str(&s[i + 1..j.min(b.len())]);
                    i = j + 1;
                }
                _ => {
                    let len = s[i..]
                        .bytes()
                        .take_while(|c| {
                            !matches!(c, b',' | b'#' | b'}') && !c.is_ascii_whitespace()
                        })
                        .count();
                    value.push_str(&s[i..i + len]);
                    i += len;
                }
            }
            i = skip_ws(s, i);
            if i < b.len() && b[i] == b'#' {
                i = skip_ws(s, i + 1);
                continue;
            }
            break;
        }
        out.insert(name, value.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    out
}

/// Format an entry as one plain-text line: `Authors. Title. Venue, Year.`
pub fn format_entry(e: &BibEntry) -> String {
    let o = PlainOpts {
        keep_math: true,
        labels: None,
    };
    let f = |k: &str| {
        e.fields
            .get(k)
            .map(|v| to_plain(v, &o))
            .filter(|v| !v.is_empty())
    };
    let authors = f("author").map(|a| {
        a.split(" and ")
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(", ")
    });
    let venue = f("journal")
        .or_else(|| f("booktitle"))
        .or_else(|| f("publisher"))
        .or_else(|| f("howpublished"));
    let mut parts = vec![];
    if let Some(a) = authors {
        parts.push(a);
    }
    if let Some(t) = f("title") {
        parts.push(t);
    }
    match (venue, f("year")) {
        (Some(v), Some(y)) => parts.push(format!("{v}, {y}")),
        (Some(v), None) => parts.push(v),
        (None, Some(y)) => parts.push(y),
        _ => {}
    }
    let mut s = parts
        .into_iter()
        .map(|p| p.trim_end_matches('.').to_string())
        .collect::<Vec<_>>()
        .join(". ");
    if !s.is_empty() {
        s.push('.');
    }
    s
}

/// `\bibitem[label]{key} text` entries (from a `.bbl` or `thebibliography`).
pub fn parse_bibitems(src: &str) -> Vec<(String, String)> {
    let o = PlainOpts {
        keep_math: true,
        labels: None,
    };
    let mut out = vec![];
    let mut starts = vec![];
    let mut i = 0;
    while let Some((s, e, _)) = find_command(src, "bibitem", i) {
        starts.push((s, e));
        i = e;
    }
    for (n, (_, e)) in starts.iter().enumerate() {
        let mut p = *e;
        if let Some((_, _, after)) = read_optional(src, p) {
            p = after;
        }
        let Some((key, after)) = group_text(src, p) else {
            continue;
        };
        let end = starts.get(n + 1).map(|x| x.0).unwrap_or(src.len());
        let mut body = &src[after..end];
        if let Some(k) = body.find("\\end{thebibliography}") {
            body = &body[..k];
        }
        let body = body.replace("\\newblock", " ");
        out.push((key.trim().to_string(), to_plain(&body, &o)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bibtex_entries() {
        let src = r#"
@comment{ignore me}
@string{mia = "Medical Image Analysis"}
@article{smith2020,
  author = {Smith, Ann and Doe, Bo},
  title = {Deep {X-ray} Things},
  journal = "Med. Image Anal.",
  year = 2020,
}
@inproceedings( lee21 , title={A}, booktitle={MICCAI}, year={2021})
"#;
        let es = parse_bibtex(src);
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].key, "smith2020");
        assert_eq!(es[0].fields["title"], "Deep {X-ray} Things");
        assert_eq!(es[0].fields["year"], "2020");
        assert_eq!(
            format_entry(&es[0]),
            "Smith, Ann, Doe, Bo. Deep X-ray Things. Med. Image Anal., 2020."
        );
        assert_eq!(es[1].entry_type, "inproceedings");
        assert_eq!(es[1].fields["booktitle"], "MICCAI");
    }

    #[test]
    fn bibitems() {
        let src = "\\begin{thebibliography}{9}\n\\bibitem{a} A.~Author, ``Title,'' \\emph{J.}, 2020.\n\\bibitem[B(2)]{b}\nB. Two.\n\\end{thebibliography}";
        let items = parse_bibitems(src);
        assert_eq!(
            items,
            vec![
                ("a".into(), "A. Author, “Title,” J., 2020.".into()),
                ("b".into(), "B. Two.".into())
            ]
        );
    }
}
