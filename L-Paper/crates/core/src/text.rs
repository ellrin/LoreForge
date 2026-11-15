//! Text helpers: paragraph and sentence splitting, tokens for retrieval.
//!
//! A paragraph is a block separated by one or more blank lines, matching the
//! one-sentence-per-line, blank-line-between-paragraphs file format.

/// Split text into paragraphs (non-empty blocks between blank lines). A
/// blank line inside an open environment (`\begin{table}` ... `\end{table}`)
/// does not split, so tables and equations stay whole.
pub fn paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut depth: i32 = 0;
    for line in text.lines() {
        let code = line.split('%').next().unwrap_or("");
        depth += code.matches("\\begin{").count() as i32 - code.matches("\\end{").count() as i32;
        if depth > 0 && line.trim().is_empty() {
            cur.push(line);
            continue;
        }
        depth = depth.max(0);
        // A heading on its own line is a paragraph of its own, so the text or
        // table after it is recognised on its own (LaTeX output is the same).
        if depth == 0 && is_heading_line(code) {
            if !cur.is_empty() {
                out.push(cur.join("\n"));
                cur.clear();
            }
            out.push(line.to_string());
            continue;
        }
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(cur.join("\n"));
                cur.clear();
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

/// `\\subsection{Title}`, optionally followed by `\\label{...}`, and nothing else.
fn is_heading_line(code: &str) -> bool {
    let t = code.trim();
    let Some(rest) = ["\\subsubsection", "\\subsection", "\\section"].iter().find_map(|h| t.strip_prefix(h)) else { return false };
    let rest = rest.strip_prefix('*').unwrap_or(rest);
    let Some(rest) = rest.strip_prefix('{') else { return false };
    let mut depth = 1;
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let after = rest[i + 1..].trim();
                    return after.is_empty() || (after.starts_with("\\label{") && after.ends_with('}') && after.matches('}').count() == 1);
                }
            }
            _ => {}
        }
    }
    false
}

/// Abbreviations that end with a period but do not end a sentence.
const ABBREVIATIONS: &[&str] = &[
    "e.g.", "i.e.", "et al.", "etc.", "vs.", "fig.", "figs.", "eq.", "eqs.", "sec.", "ref.",
    "refs.", "no.", "cf.", "approx.", "resp.", "tab.",
];

/// Split a paragraph into sentences. LaTeX-aware enough for prose: periods
/// inside `$...$` math and after common abbreviations do not split.
pub fn sentences(paragraph: &str) -> Vec<String> {
    let flat = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_math = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' && (i == 0 || chars[i - 1] != '\\') {
            in_math = !in_math;
        }
        let ends = matches!(c, '.' | '?' | '!' | '。' | '？' | '！');
        let next_is_break = i + 1 >= chars.len() || chars[i + 1] == ' ';
        if ends && !in_math && next_is_break {
            let candidate: String = chars[start..=i].iter().collect();
            let lower = candidate.to_lowercase();
            let is_abbrev = c == '.' && ABBREVIATIONS.iter().any(|a| lower.ends_with(a));
            if !is_abbrev {
                let s = candidate.trim().to_string();
                if !s.is_empty() {
                    out.push(s);
                }
                start = i + 1;
            }
        }
        i += 1;
    }
    let rest: String = chars[start..].iter().collect();
    if !rest.trim().is_empty() {
        out.push(rest.trim().to_string());
    }
    out
}

/// Count words, ignoring inline math and LaTeX commands such as `\cite{...}`.
pub fn word_count(sentence: &str) -> usize {
    strip_latex(sentence)
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count()
}

/// Remove inline math and `\command{...}` so word counts and matching see prose.
pub fn strip_latex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_math = false;
    while let Some(c) = chars.next() {
        match c {
            '$' => in_math = !in_math,
            _ if in_math => {}
            '\\' => {
                // Skip the command name and one following {...} group.
                while chars.peek().is_some_and(|c| c.is_alphabetic()) {
                    chars.next();
                }
                if chars.peek() == Some(&'{') {
                    let mut depth = 0;
                    for c in chars.by_ref() {
                        match c {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                out.push(' ');
            }
            '~' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

/// Lowercase word tokens for keyword matching.
pub fn tokens(s: &str) -> Vec<String> {
    strip_latex(s)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2 && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect()
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "are", "was", "were", "from", "our", "its",
    "has", "have", "been", "which", "into", "than", "can", "not", "but", "also",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_stand_alone() {
        let t = "\\subsection{Encoder {and} Decoder} \\label{s:e}\n\\begin{table}\nx\n\\end{table}\nText \\subsection{inline} stays.\n\\paragraph{Run-in} text.";
        assert_eq!(
            paragraphs(t),
            ["\\subsection{Encoder {and} Decoder} \\label{s:e}", "\\begin{table}\nx\n\\end{table}\nText \\subsection{inline} stays.\n\\paragraph{Run-in} text."]
        );
    }

    #[test]
    fn splits_paragraphs_on_blank_lines() {
        let t = "First line.\nSecond line.\n\n\nNext para.\n";
        assert_eq!(paragraphs(t), vec!["First line.\nSecond line.", "Next para."]);
    }

    #[test]
    fn keeps_abbreviations_and_math_together() {
        let p = "We follow Smith et al. in this. The error is $e = 0.5$ here. Done!";
        let s = sentences(p);
        assert_eq!(s.len(), 3, "{s:?}");
        assert!(s[0].contains("et al. in this."));
        assert!(s[1].contains("$e = 0.5$"));
    }

    #[test]
    fn word_count_ignores_latex() {
        assert_eq!(word_count(r"As shown in \cite{smith2020}, $x^2$ grows fast."), 5);
    }
}
