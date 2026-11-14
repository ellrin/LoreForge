//! LaTeX fragment → plain text (for term scanners, notation extractors,
//! captions). Inline math is kept as `$…$` (or dropped), citations are
//! dropped, `\ref` becomes the printed number when known.

use super::scan::{find_env_end, group_text, read_group, skip_optionals};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct PlainOpts<'a> {
    /// Keep inline math as `$…$` and display math as `$$…$$`.
    pub keep_math: bool,
    /// `\label` key → printed number, for `\ref`.
    pub labels: Option<&'a HashMap<String, String>>,
}

impl Default for PlainOpts<'_> {
    fn default() -> Self {
        PlainOpts {
            keep_math: true,
            labels: None,
        }
    }
}

const CITE: &[&str] = &[
    "cite",
    "citep",
    "citet",
    "citealp",
    "citealt",
    "citeauthor",
    "citeyear",
    "parencite",
    "textcite",
    "autocite",
    "nocite",
    "Cite",
    "Citep",
    "Citet",
];
const DROP1: &[&str] = &[
    "label",
    "footnote",
    "thanks",
    "vspace",
    "hspace",
    "includegraphics",
    "bibliographystyle",
    "bibliography",
    "color",
    "pagestyle",
    "thispagestyle",
    "captionsetup",
    "IEEEmembership",
    "footnotetext",
    "markboth",
    "graphicspath",
    "input",
    "include",
    "linenumbers",
    "tnotetext",
    "fntext",
    "cortext",
    "ead",
    "address",
    "affiliation",
    "orcidlink",
    "newcommand",
    "renewcommand",
    "def",
];
const DROP2: &[&str] = &["setlength", "addtolength", "setcounter", "addtocounter"];
const KEEP: &[&str] = &[
    "textbf",
    "textit",
    "emph",
    "underline",
    "texttt",
    "textsc",
    "textrm",
    "textsf",
    "textup",
    "textnormal",
    "textmd",
    "mbox",
    "text",
    "hbox",
    "uline",
    "ul",
    "textsl",
    "mathrm",
    "mathbf",
    "mathit",
    "boldsymbol",
    "bm",
    "textsuperscript",
    "textsubscript",
    "makebox",
    "fbox",
    "framebox",
    "small",
    "footnotesize",
    "large",
];
const REF: &[&str] = &[
    "ref", "eqref", "autoref", "cref", "Cref", "pageref", "nameref", "subref",
];
const MATH_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "eqnarray",
    "eqnarray*",
    "flalign",
    "flalign*",
    "alignat",
    "alignat*",
    "displaymath",
    "math",
];
const SKIP_ENVS: &[&str] = &[
    "figure",
    "figure*",
    "table",
    "table*",
    "sidewaystable",
    "sidewaysfigure",
    "wrapfigure",
    "wraptable",
    "tikzpicture",
    "algorithm",
    "algorithm*",
    "thebibliography",
    "comment",
    "lstlisting",
    "verbatim",
];

pub fn is_math_env(name: &str) -> bool {
    MATH_ENVS.contains(&name)
}

/// Convert a LaTeX fragment to plain text.
pub fn to_plain(s: &str, o: &PlainOpts) -> String {
    let mut out = String::with_capacity(s.len());
    conv(s, o, &mut out);
    tidy(&out)
}

fn accent(cmd: char, base: char) -> char {
    let table: &[(char, char, char)] = &[
        ('\'', 'e', 'é'),
        ('\'', 'a', 'á'),
        ('\'', 'o', 'ó'),
        ('\'', 'i', 'í'),
        ('\'', 'u', 'ú'),
        ('\'', 'E', 'É'),
        ('`', 'e', 'è'),
        ('`', 'a', 'à'),
        ('"', 'o', 'ö'),
        ('"', 'u', 'ü'),
        ('"', 'a', 'ä'),
        ('"', 'O', 'Ö'),
        ('"', 'U', 'Ü'),
        ('"', 'A', 'Ä'),
        ('~', 'n', 'ñ'),
        ('^', 'o', 'ô'),
        ('^', 'e', 'ê'),
        ('c', 'c', 'ç'),
    ];
    table
        .iter()
        .find(|(c, b, _)| *c == cmd && *b == base)
        .map(|t| t.2)
        .unwrap_or(base)
}

fn conv(s: &str, o: &PlainOpts, out: &mut String) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'$' => {
                let display = s[i..].starts_with("$$");
                let open = if display { 2 } else { 1 };
                let close = if display { "$$" } else { "$" };
                let mut j = i + open;
                let end = loop {
                    match s[j..].find(close) {
                        Some(k) if j + k > 0 && b[j + k - 1] == b'\\' => j = j + k + 1,
                        Some(k) => break Some(j + k),
                        None => break None,
                    }
                };
                let Some(end) = end else {
                    i += open;
                    continue;
                };
                if o.keep_math {
                    let m = s[i + open..end].trim();
                    if display {
                        out.push_str(&format!(" $${m}$$ "));
                    } else {
                        out.push_str(&format!("${m}$"));
                    }
                }
                i = end + open;
            }
            b'\\' => {
                let rest = &s[i + 1..];
                let Some(nc) = rest.chars().next() else { break };
                if nc.is_ascii_alphabetic() {
                    let name_len = rest.bytes().take_while(|x| x.is_ascii_alphabetic()).count();
                    let name = &rest[..name_len];
                    let mut p = i + 1 + name_len;
                    if b.get(p) == Some(&b'*') {
                        p += 1;
                    }
                    i = command(s, name, p, o, out);
                } else {
                    i += 1 + nc.len_utf8();
                    match nc {
                        '\\' => {
                            out.push(' ');
                            // `\\[2pt]`
                            if let Some((_, _, after)) = super::scan::read_optional(s, i)
                                && s[i..].trim_start().starts_with('[')
                            {
                                i = after;
                            }
                        }
                        '%' | '&' | '_' | '#' | '$' | '{' | '}' => out.push(nc),
                        ' ' | ',' | ';' | ':' | '!' => out.push(' '),
                        '(' => {
                            let end = s[i..].find("\\)").map(|k| i + k);
                            if let Some(end) = end {
                                if o.keep_math {
                                    out.push_str(&format!("${}$", s[i..end].trim()));
                                }
                                i = end + 2;
                            }
                        }
                        '[' => {
                            let end = s[i..].find("\\]").map(|k| i + k);
                            if let Some(end) = end {
                                if o.keep_math {
                                    out.push_str(&format!(" $${}$$ ", s[i..end].trim()));
                                }
                                i = end + 2;
                            }
                        }
                        '\'' | '`' | '"' | '^' | '~' | '=' | '.' => {
                            // accent: \'e or \'{e}
                            if let Some((t, after)) =
                                group_text(s, i).filter(|_| b.get(i) == Some(&b'{'))
                            {
                                if let Some(base) = t.chars().next() {
                                    out.push(accent(nc, base));
                                }
                                i = after;
                            } else if let Some(base) = s[i..].chars().next() {
                                out.push(accent(nc, base));
                                i += base.len_utf8();
                            }
                        }
                        _ => {}
                    }
                }
            }
            b'{' | b'}' => i += 1,
            b'~' => {
                out.push(' ');
                i += 1;
            }
            b'-' if s[i..].starts_with("---") => {
                out.push('—');
                i += 3;
            }
            b'-' if s[i..].starts_with("--") => {
                out.push('–');
                i += 2;
            }
            b'`' if s[i..].starts_with("``") => {
                out.push('“');
                i += 2;
            }
            b'\'' if s[i..].starts_with("''") => {
                out.push('”');
                i += 2;
            }
            _ => {
                let ch = s[i..].chars().next().unwrap();
                out.push(if ch == '\n' || ch == '\t' { ' ' } else { ch });
                i += ch.len_utf8();
            }
        }
    }
}

/// Handle `\name` whose arguments start at `p`; returns the position after it.
fn command(s: &str, name: &str, p: usize, o: &PlainOpts, out: &mut String) -> usize {
    let one = |p: usize| -> (Option<&str>, usize) {
        let q = skip_optionals(s, p);
        match group_text(s, q) {
            Some((t, after)) => (Some(t), after),
            None => (None, q),
        }
    };
    if CITE.contains(&name) {
        let q = skip_optionals(s, p);
        return read_group(s, q).map(|g| g.2).unwrap_or(q);
    }
    if DROP1.contains(&name) {
        return one(p).1;
    }
    if DROP2.contains(&name) {
        let (_, a) = one(p);
        return one(a).1;
    }
    if REF.contains(&name) {
        let (key, after) = one(p);
        let num = key
            .and_then(|k| {
                // \cref{a,b}
                let first = k.split(',').next().unwrap_or(k).trim();
                o.labels.and_then(|m| m.get(first)).cloned()
            })
            .unwrap_or_else(|| "??".into());
        if name == "eqref" {
            out.push_str(&format!("({num})"));
        } else {
            out.push_str(&num);
        }
        return after;
    }
    if KEEP.contains(&name) {
        let (t, after) = one(p);
        if let Some(t) = t {
            conv(t, o, out);
        }
        return after;
    }
    match name {
        // 待補 draft marks (lore-notation `pending`): shown as marks, never as a number
        "pending" | "pendingpara" => {
            let (_, a) = one(p);
            let (t, after) = one(a);
            let mut what = String::new();
            conv(t.unwrap_or(""), o, &mut what);
            let what = tidy(&what);
            out.push_str(&if what.is_empty() { "[pending]".to_string() } else { format!("[pending: {what}]") });
            after
        }
        "pendingcell" => {
            let (_, a) = one(p);
            out.push_str("[pending]");
            one(a).1
        }
        // a sentence that depends on a pending result says so
        "pendingdep" => {
            let kept = out.trim_end().len();
            out.truncate(kept);
            out.push_str(" [pending]");
            one(p).1
        }
        "begin" => {
            let Some((env, body_start)) = group_text(s, p) else {
                return p;
            };
            let env = env.trim();
            let Some((body_end, end)) = find_env_end(s, env, body_start) else {
                return body_start;
            };
            let body = &s[body_start..body_end];
            if is_math_env(env) {
                if o.keep_math {
                    let m = super::extract::strip_labels(body);
                    out.push_str(&format!(" $${}$$ ", m.trim()));
                }
            } else if SKIP_ENVS.contains(&env) {
            } else {
                // drop the env's own arguments, e.g. {tabular}{lcr}, {minipage}{0.5\linewidth}
                let mut q = skip_optionals(body, 0);
                if matches!(
                    env,
                    "minipage"
                        | "tabular"
                        | "tabularx"
                        | "tabular*"
                        | "array"
                        | "subfigure"
                        | "wrapfigure"
                        | "multicols"
                ) && let Some((_, _, after)) = read_group(body, q)
                {
                    q = after;
                    if env == "tabularx" || env == "tabular*" {
                        q = read_group(body, q).map(|g| g.2).unwrap_or(q);
                    }
                }
                out.push(' ');
                conv(&body[q..], o, out);
                out.push(' ');
            }
            end
        }
        "end" => one(p).1,
        "item" => {
            out.push_str("\n- ");

            super::scan::read_optional(s, p).map(|g| g.2).unwrap_or(p)
        }
        "textcolor" | "colorbox" => {
            let (_, a) = one(p);
            let (t, after) = one(a);
            if let Some(t) = t {
                conv(t, o, out);
            }
            after
        }
        "href" => {
            let (_, a) = one(p);
            let (t, after) = one(a);
            if let Some(t) = t {
                conv(t, o, out);
            }
            after
        }
        "url" => {
            let (t, after) = one(p);
            out.push_str(t.unwrap_or(""));
            after
        }
        "IEEEPARstart" => {
            let (a, x) = one(p);
            let (b2, after) = one(x);
            out.push_str(a.unwrap_or(""));
            out.push_str(b2.unwrap_or(""));
            after
        }
        "LaTeX" => {
            out.push_str("LaTeX");
            p
        }
        "TeX" => {
            out.push_str("TeX");
            p
        }
        "ldots" | "dots" | "cdots" => {
            out.push('…');
            p
        }
        "textendash" => {
            out.push('–');
            p
        }
        "textemdash" => {
            out.push('—');
            p
        }
        "ie" => {
            out.push_str("i.e.");
            p
        }
        "eg" => {
            out.push_str("e.g.");
            p
        }
        "etal" => {
            out.push_str("et al.");
            p
        }
        "par" | "newline" | "linebreak" | "noindent" | "centering" | "raggedright" | "and"
        | "And" | "AND" => {
            out.push(' ');
            p
        }
        _ => {
            // unknown: drop the name and its optional args, keep brace contents
            skip_optionals(s, p)
        }
    }
}

/// Collapse whitespace (newlines from `\item` kept), fix spaces before punctuation.
pub fn tidy(s: &str) -> String {
    let mut lines = vec![];
    for line in s.split('\n') {
        let mut t = String::new();
        let mut prev_space = false;
        for ch in line.chars() {
            if ch.is_whitespace() {
                if !prev_space && !t.is_empty() {
                    t.push(' ');
                }
                prev_space = true;
            } else {
                if prev_space && matches!(ch, ',' | '.' | ';' | ':' | ')' | '!') && t.ends_with(' ')
                {
                    t.pop();
                }
                t.push(ch);
                prev_space = false;
            }
        }
        let t = t.trim().replace("( ", "(");
        if !t.is_empty() {
            lines.push(t);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_basics() {
        let mut labels = HashMap::new();
        labels.insert("fig:a".to_string(), "2".to_string());
        labels.insert("eq:l".to_string(), "5".to_string());
        let o = PlainOpts {
            keep_math: true,
            labels: Some(&labels),
        };
        let s = r"As shown in Fig.~\ref{fig:a} and \eqref{eq:l}, the \textbf{encoder} $E$ maps~\cite{smith2020} images --- see \url{x.org}\footnote{note}.";
        assert_eq!(
            to_plain(s, &o),
            "As shown in Fig. 2 and (5), the encoder $E$ maps images — see x.org."
        );
        let o2 = PlainOpts {
            keep_math: false,
            labels: None,
        };
        assert_eq!(
            to_plain(r"Let $x$ be \emph{given} \ref{q}.", &o2),
            "Let be given ??."
        );
    }

    #[test]
    fn pending_marks_stay_marks() {
        let s = r"Dice is \pending{P1}{Dice on the \textbf{external} set}, above the baseline\pendingdep{P1}. Cell \pendingcell{P1}{Ours / Dice}. \pendingpara{P2}{Compare the sites.}";
        assert_eq!(
            tidy(&to_plain(s, &PlainOpts::default())),
            "Dice is [pending: Dice on the external set], above the baseline [pending]. Cell [pending]. [pending: Compare the sites.]"
        );
    }

    #[test]
    fn lists_and_ieee_start() {
        let s = r"\IEEEPARstart{T}{his} works: \begin{itemize}\item one \item two\end{itemize}";
        assert_eq!(
            to_plain(s, &PlainOpts::default()),
            "This works:\n- one\n- two"
        );
    }

    #[test]
    fn display_math_and_accents() {
        let s = "G\\\"odel said \\begin{equation}a=b\\label{e}\\end{equation} done 50\\%";
        assert_eq!(
            to_plain(s, &PlainOpts::default()),
            "Gödel said $$a=b$$ done 50%"
        );
    }
}
