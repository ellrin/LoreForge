//! Typed references in LaTeX written by pandoc from Word: `[1]`, `[2, 3]`,
//! `[4–6]` (pandoc writes `{[}1{]}`) become `\cite{ref1}`; "Fig. 1",
//! "Table 2", "Eq. (3)", "Section 2.1" become `\ref` / `\eqref` to the
//! label of that number when the conversion made one. A number with no
//! label stays typed, so the notation registry's check still points at it.

/// The reference words: (word as typed, label kind).
const WORDS: &[(&str, &str)] = &[
    ("Figures", "fig"),
    ("Figure", "fig"),
    ("Figs.", "fig"),
    ("Fig.", "fig"),
    ("Tables", "tab"),
    ("Table", "tab"),
    ("Tab.", "tab"),
    ("Equations", "eq"),
    ("Equation", "eq"),
    ("Eqs.", "eq"),
    ("Eq.", "eq"),
    ("Sections", "sec"),
    ("Section", "sec"),
    ("Sec.", "sec"),
];

/// `1, 2`, `4--6`, `4–6` → [1, 2] / [4, 5, 6]; None when it is not a citation.
fn cite_numbers(inner: &str) -> Option<Vec<usize>> {
    let inner = inner.replace("--", "-").replace(['–', '—'], "-");
    let mut out = Vec::new();
    for part in inner.split([',', ';']) {
        let part = part.trim();
        if let Some((a, b)) = part.split_once('-') {
            let (a, b): (usize, usize) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
            if b < a || b - a > 50 {
                return None;
            }
            out.extend(a..=b);
        } else {
            out.push(part.parse().ok()?);
        }
    }
    (!out.is_empty()).then_some(out)
}

fn citations(text: &str, refs: usize) -> String {
    let (open, close) = ("{[}", "{]}");
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(close).filter(|j| *j < 40) else {
            out += &rest[..i + open.len()];
            rest = after;
            continue;
        };
        match cite_numbers(&after[..j]).filter(|ns| refs > 0 && ns.iter().all(|n| (1..=refs).contains(n))) {
            Some(ns) => {
                out += &rest[..i];
                out += &format!("\\cite{{{}}}", ns.iter().map(|n| format!("ref{n}")).collect::<Vec<_>>().join(","));
                rest = &after[j + close.len()..];
            }
            None => {
                out += &rest[..i + open.len()];
                rest = after;
            }
        }
    }
    out + rest
}

fn typed(text: &str, labels: &[String]) -> String {
    let mut out = String::new();
    let mut i = 0;
    let b = text.as_bytes();
    'scan: while i < text.len() {
        let prev = text[..i].chars().next_back();
        if prev.is_none_or(|c| !c.is_alphanumeric() && c != '\\') {
            for (word, kind) in WORDS {
                if !text[i..].starts_with(word) {
                    continue;
                }
                let mut j = i + word.len();
                let gap = j;
                while j < text.len() && (b[j] == b' ' || b[j] == b'~') {
                    j += 1;
                }
                if j == gap {
                    continue;
                }
                let paren = *kind == "eq" && b.get(j) == Some(&b'(');
                let start = if paren { j + 1 } else { j };
                let mut k = start;
                while k < text.len() && (b[k].is_ascii_digit() || (*kind == "sec" && b[k] == b'.' && b.get(k + 1).is_some_and(|c| c.is_ascii_digit()))) {
                    k += 1;
                }
                if k == start || b.get(k).is_some_and(|c| c.is_ascii_alphanumeric()) {
                    continue;
                }
                if paren {
                    if b.get(k) != Some(&b')') {
                        continue;
                    }
                }
                let label = format!("{kind}:{}", &text[start..k]);
                if !labels.contains(&label) {
                    continue;
                }
                let cmd = if *kind == "eq" { "eqref" } else { "ref" };
                out += &format!("{word}~\\{cmd}{{{label}}}");
                i = if paren { k + 1 } else { k };
                continue 'scan;
            }
        }
        let c = text[i..].chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Citations and typed references of a whole document.
pub fn rewrite(text: &str, labels: &[String], refs: usize) -> String {
    typed(&citations(text, refs), labels)
}

/// The typed reference list as BibTeX: `ref1`, `ref2`, … holding the text
/// as written (a `note`), until each is matched to its record.
pub fn bibtex(refs: &[String]) -> String {
    let esc = |s: &str| -> String {
        let mut o = String::new();
        let mut depth = 0i32;
        for c in s.chars() {
            match c {
                '&' | '%' | '$' | '#' | '_' => {
                    o.push('\\');
                    o.push(c);
                }
                '{' => {
                    depth += 1;
                    o.push(c);
                }
                '}' if depth > 0 => {
                    depth -= 1;
                    o.push(c);
                }
                '}' | '\\' => {}
                _ => o.push(c),
            }
        }
        o + &"}".repeat(depth.max(0) as usize)
    };
    refs.iter().enumerate().map(|(i, r)| format!("@misc{{ref{},\n  note = {{{}}}\n}}\n\n", i + 1, esc(r))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_citations_become_cite() {
        assert_eq!(citations("common {[}1{]}. Also {[}2, 3{]} and {[}4--6{]}.", 6), "common \\cite{ref1}. Also \\cite{ref2,ref3} and \\cite{ref4,ref5,ref6}.");
        assert_eq!(citations("a {[}7{]} b", 6), "a {[}7{]} b", "no reference 7");
        assert_eq!(citations("x {[}see{]} y", 6), "x {[}see{]} y");
        assert_eq!(citations("{[}1{]}", 0), "{[}1{]}", "no reference list: left typed");
    }

    #[test]
    fn typed_numbers_become_refs_when_labelled() {
        let labels: Vec<String> = ["fig:1", "tab:2", "eq:3", "sec:2", "sec:2.1"].map(String::from).to_vec();
        assert_eq!(
            typed("Fig. 1 and Table 2 follow Eq. (3); Section 2.1 and Section 2. Fig.~1.", &labels),
            "Fig.~\\ref{fig:1} and Table~\\ref{tab:2} follow Eq.~\\eqref{eq:3}; Section~\\ref{sec:2.1} and Section~\\ref{sec:2}. Fig.~\\ref{fig:1}."
        );
        assert_eq!(typed("Table 9 and Figure 1a; \\Table 2", &labels), "Table 9 and Figure 1a; \\Table 2");
    }

    #[test]
    fn reference_list_as_bibtex() {
        let b = bibtex(&["A. Writer, R&D of widgets, J. 3 (2020) 1-10.".into()]);
        assert_eq!(b, "@misc{ref1,\n  note = {A. Writer, R\\&D of widgets, J. 3 (2020) 1-10.}\n}\n\n");
    }
}
