//! LaTeX the model must not touch: citations, references, labels, math, comments and the
//! registry's macros are replaced by locked placeholders (`⟦1⟧`, `⟦2⟧`, …) before a passage
//! goes out, and put back afterwards. A placeholder that is missing, comes back twice or is
//! made up, and any citation, reference, label, math or registered macro the model typed
//! itself, is a [`Problem`]: the proposal is shown but cannot be applied.
//!
//! The command lists match PaperComposer's `ui/context.js` (CITE, REF, math environments).

use serde::Serialize;

const CITE: &[&str] = &[
    "cite", "citep", "citet", "citealp", "citealt", "citeauthor", "citeyear", "parencite", "textcite", "autocite", "footcite", "supercite", "Citet", "Citep", "Citealp",
    "Citeauthor", "Parencite", "Textcite", "nocite",
];
const REF: &[&str] = &["ref", "eqref", "autoref", "cref", "Cref", "pageref", "vref", "subref", "label"];
const MATH_ENV: &[&str] = &["equation", "align", "alignat", "gather", "multline", "eqnarray", "flalign", "displaymath", "math", "dmath"];

const OPEN: char = '⟦';
const CLOSE: char = '⟧';

/// A passage with its locked parts taken out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Locked {
    /// The passage with `⟦n⟧` in place of each locked part.
    pub text: String,
    /// The locked parts, `slots[n - 1]` = what `⟦n⟧` stands for.
    pub slots: Vec<String>,
}

/// Why a rewrite cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Problem {
    /// A locked part did not come back.
    Missing { what: String },
    /// A locked part came back more than once.
    Twice { what: String },
    /// A placeholder that was never sent.
    Unknown { what: String },
    /// LaTeX of a locked kind the model wrote itself.
    Added { what: String },
    /// The app's own check (PaperComposer: lore-notation) found something new.
    Check { what: String },
}

impl Problem {
    /// One line for the page.
    pub fn message(&self) -> String {
        match self {
            Problem::Missing { what } => format!("少了 {what}"),
            Problem::Twice { what } => format!("{what} 出現了不只一次"),
            Problem::Unknown { what } => format!("多了不認得的記號 {what}"),
            Problem::Added { what } => format!("多了 {what}"),
            Problem::Check { what } => what.clone(),
        }
    }
}

pub fn token(n: usize) -> String {
    format!("{OPEN}{n}{CLOSE}")
}

/// End of the `{…}` group opening at `i` (exclusive), or None.
fn group(s: &[char], i: usize) -> Option<usize> {
    if s.get(i) != Some(&'{') {
        return None;
    }
    let mut d = 0;
    let mut k = i;
    while k < s.len() {
        match s[k] {
            '\\' => k += 1,
            '{' => d += 1,
            '}' => {
                d -= 1;
                if d == 0 {
                    return Some(k + 1);
                }
            }
            _ => {}
        }
        k += 1;
    }
    Some(s.len())
}

fn optional(s: &[char], i: usize) -> Option<usize> {
    if s.get(i) != Some(&'[') {
        return None;
    }
    let mut k = i + 1;
    while k < s.len() {
        match s[k] {
            '\\' => k += 1,
            ']' => return Some(k + 1),
            _ => {}
        }
        k += 1;
    }
    Some(s.len())
}

fn blank(s: &[char], mut i: usize) -> usize {
    while i < s.len() && (s[i] == ' ' || s[i] == '\t') {
        i += 1;
    }
    i
}

fn starts(s: &[char], i: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    s.len() >= i + p.len() && s[i..i + p.len()] == p[..]
}

/// Where `end` closes, searching from `i` (escapes skipped); the end of the text if never.
fn close(s: &[char], i: usize, end: &str) -> usize {
    let mut k = i;
    while k < s.len() {
        if starts(s, k, end) {
            return k + end.chars().count();
        }
        if s[k] == '\\' {
            k += 1;
        }
        k += 1;
    }
    s.len()
}

/// The locked spans of `s` (char indices, in order): `macros` are the registry's macro names
/// (with or without the backslash), `literal` any other strings to keep as they are
/// (SlideComposer: a line break inside a paragraph).
pub fn spans(text: &str, macros: &[String], literal: &[String]) -> Vec<(usize, usize)> {
    let s: Vec<char> = text.chars().collect();
    let macros: Vec<&str> = macros.iter().map(|m| m.trim().trim_start_matches('\\')).filter(|m| !m.is_empty()).collect();
    let lits: Vec<&String> = literal.iter().filter(|l| !l.is_empty()).collect();
    let n = s.len();
    let mut out = vec![];
    let mut i = 0;
    while i < n {
        if let Some(l) = lits.iter().find(|l| starts(&s, i, l)) {
            let z = i + l.chars().count();
            out.push((i, z));
            i = z;
            continue;
        }
        let c = s[i];
        if c == '%' {
            let z = (i..n).find(|&k| s[k] == '\n').unwrap_or(n);
            out.push((i, z));
            i = z;
            continue;
        }
        if c == '$' {
            let z = if s.get(i + 1) == Some(&'$') { close(&s, i + 2, "$$") } else { close(&s, i + 1, "$") };
            out.push((i, z));
            i = z;
            continue;
        }
        if c != '\\' {
            i += 1;
            continue;
        }
        let d = s.get(i + 1).copied();
        if d == Some('(') || d == Some('[') {
            let z = close(&s, i + 2, if d == Some('(') { "\\)" } else { "\\]" });
            out.push((i, z));
            i = z;
            continue;
        }
        if !d.is_some_and(|d| d.is_ascii_alphabetic()) {
            i += 2; // \$ \% \\ …
            continue;
        }
        let mut j = i + 1;
        while j < n && s[j].is_ascii_alphabetic() {
            j += 1;
        }
        let name: String = s[i + 1..j].iter().collect();
        let star = s.get(j) == Some(&'*');
        let after = if star { j + 1 } else { j };
        if CITE.contains(&name.as_str()) || REF.contains(&name.as_str()) {
            let mut k = after;
            while let Some(o) = optional(&s, blank(&s, k)) {
                k = o;
            }
            if let Some(g) = group(&s, blank(&s, k)) {
                out.push((i, g));
                i = g;
                continue;
            }
        }
        if name == "begin"
            && let Some(g) = group(&s, blank(&s, j))
        {
            let env: String = s[blank(&s, j) + 1..g - 1].iter().collect();
            if MATH_ENV.contains(&env.trim_end_matches('*')) {
                let z = close(&s, g, &format!("\\end{{{env}}}"));
                out.push((i, z));
                i = z;
                continue;
            }
        }
        if macros.contains(&name.as_str()) {
            // the macro and its arguments ({} included)
            let mut k = j;
            while let Some(g) = group(&s, k) {
                k = g;
            }
            out.push((i, k));
            i = k;
            continue;
        }
        i = j;
    }
    out
}

/// Take the locked parts out of `text`.
pub fn lock(text: &str, macros: &[String], literal: &[String]) -> Locked {
    let s: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut slots: Vec<String> = vec![];
    let mut at = 0;
    for (a, b) in spans(text, macros, literal) {
        out.extend(&s[at..a]);
        let part: String = s[a..b].iter().collect();
        // the same part twice in the passage gets two placeholders: each must come back
        slots.push(part);
        out += &token(slots.len());
        at = b;
    }
    out.extend(&s[at..]);
    Locked { text: out, slots }
}

impl Locked {
    /// `text` (an earlier rewrite of the same passage) with these slots locked again, so a
    /// conversation shows the model the same placeholders every turn.
    pub fn encode(&self, text: &str, macros: &[String], literal: &[String]) -> String {
        let s: Vec<char> = text.chars().collect();
        let mut used = vec![false; self.slots.len()];
        let mut out = String::new();
        let mut at = 0;
        for (a, b) in spans(text, macros, literal) {
            out.extend(&s[at..a]);
            let part: String = s[a..b].iter().collect();
            match (0..self.slots.len()).find(|&k| !used[k] && self.slots[k] == part) {
                Some(k) => {
                    used[k] = true;
                    out += &token(k + 1);
                }
                None => out += &part,
            }
            at = b;
        }
        out.extend(&s[at..]);
        out
    }

    /// Put the locked parts back into `reply`. Returns the text (placeholders that cannot be
    /// resolved left as they are) and what is wrong with it; empty problems = safe to apply.
    pub fn restore(&self, reply: &str, macros: &[String], literal: &[String]) -> (String, Vec<Problem>) {
        let mut problems = vec![];
        // LaTeX of a locked kind the model typed itself (placeholders are not LaTeX)
        for (a, b) in spans(reply, macros, literal) {
            let s: Vec<char> = reply.chars().collect();
            let part: String = s[a..b].iter().collect();
            if literal.contains(&part) {
                continue; // a literal (a line break) the model kept typed out is harmless
            }
            problems.push(Problem::Added { what: part });
        }
        let mut seen = vec![0usize; self.slots.len()];
        let mut out = String::new();
        let mut rest = reply;
        while let Some(p) = rest.find(OPEN) {
            out += &rest[..p];
            let after = &rest[p + OPEN.len_utf8()..];
            let Some(q) = after.find(CLOSE) else {
                out += &rest[p..];
                rest = "";
                break;
            };
            let inner = &after[..q];
            let whole = &rest[p..p + OPEN.len_utf8() + q + CLOSE.len_utf8()];
            match inner.trim().parse::<usize>().ok().filter(|n| *n >= 1 && *n <= self.slots.len()) {
                Some(n) => {
                    seen[n - 1] += 1;
                    out += &self.slots[n - 1];
                }
                None => {
                    problems.push(Problem::Unknown { what: whole.to_string() });
                    out += whole;
                }
            }
            rest = &after[q + CLOSE.len_utf8()..];
        }
        out += rest;
        for (k, n) in seen.iter().enumerate() {
            match n {
                0 => problems.push(Problem::Missing { what: self.slots[k].clone() }),
                1 => {}
                _ => problems.push(Problem::Twice { what: self.slots[k].clone() }),
            }
        }
        (out, problems)
    }
}

/// The registry's macro names (`\ribmask` → `ribmask`), and the 待補 draft marks
/// (`\pending{P1}{…}`, lore-notation's `pending::MACROS`): a rewrite never drops or moves one.
pub fn macros_of(n: &lore_notation::Notation) -> Vec<String> {
    let mut v: Vec<String> = n.entries.iter().filter_map(|e| e.macro_name().map(str::to_string)).collect();
    v.extend(lore_notation::pending::MACROS.iter().map(|m| m.to_string()));
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "As shown in Fig.~\\ref{fig:a} and by \\citet[p.~3]{smith2020}, the loss $\\mathcal{L}=\\sum_i x_i$ of \\ribmask drops \\(x\\); see Eq.~\\eqref{eq:loss}.\\label{sec:x} 100\\% sure % note\nmore";

    fn macros() -> Vec<String> {
        vec!["\\ribmask".into()]
    }

    #[test]
    fn locks_citations_references_math_macros_and_comments() {
        let l = lock(P, &macros(), &[]);
        assert_eq!(
            l.slots,
            ["\\ref{fig:a}", "\\citet[p.~3]{smith2020}", "$\\mathcal{L}=\\sum_i x_i$", "\\ribmask", "\\(x\\)", "\\eqref{eq:loss}", "\\label{sec:x}", "% note"]
        );
        assert!(l.text.starts_with("As shown in Fig.~⟦1⟧ and by ⟦2⟧, the loss ⟦3⟧ of ⟦4⟧ drops ⟦5⟧;"), "{}", l.text);
        assert!(l.text.contains("100\\% sure ⟦8⟧\nmore"), "an escaped % is prose: {}", l.text);
        // nothing changed: the same text back, nothing wrong
        assert_eq!(l.restore(&l.text, &macros(), &[]), (P.to_string(), vec![]));
    }

    #[test]
    fn a_prefix_of_a_macro_is_not_the_macro_and_environments_lock_whole() {
        let l = lock("\\ribmasks and \\begin{align*} a &= b \\end{align*} done", &macros(), &[]);
        assert_eq!(l.slots, ["\\begin{align*} a &= b \\end{align*}"]);
    }

    #[test]
    fn a_dropped_placeholder_is_refused_and_named() {
        let l = lock(P, &macros(), &[]);
        let reply = "Fig.~⟦1⟧ and ⟦2⟧ show that ⟦3⟧ of ⟦4⟧ falls ⟦5⟧ (Eq.~⟦6⟧).⟦7⟧ ⟦8⟧\n";
        let (_, problems) = l.restore(reply, &macros(), &[]);
        assert!(problems.is_empty(), "{problems:?}");
        let dropped = reply.replace("⟦2⟧", "earlier work");
        let (text, problems) = l.restore(&dropped, &macros(), &[]);
        assert_eq!(problems, [Problem::Missing { what: "\\citet[p.~3]{smith2020}".into() }]);
        assert_eq!(problems[0].message(), "少了 \\citet[p.~3]{smith2020}");
        assert!(!text.contains("smith2020"));
    }

    #[test]
    fn twice_unknown_and_typed_latex_are_refused() {
        let l = lock("We use $x$ \\cite{a}.", &[], &[]);
        let (_, p) = l.restore("We use ⟦1⟧ and ⟦1⟧ ⟦2⟧.", &[], &[]);
        assert_eq!(p, [Problem::Twice { what: "$x$".into() }]);
        let (_, p) = l.restore("We use ⟦1⟧ ⟦2⟧ ⟦3⟧.", &[], &[]);
        assert_eq!(p, [Problem::Unknown { what: "⟦3⟧".into() }]);
        let (_, p) = l.restore("We use ⟦1⟧ ⟦2⟧ and $y$ \\cite{b}.", &[], &[]);
        assert_eq!(p, [Problem::Added { what: "$y$".into() }, Problem::Added { what: "\\cite{b}".into() }]);
    }

    #[test]
    fn earlier_rewrites_are_locked_with_the_same_placeholders() {
        let l = lock("A $x$ and $x$ \\cite{a}.", &[], &[]);
        assert_eq!(l.slots, ["$x$", "$x$", "\\cite{a}"]);
        assert_eq!(l.encode("\\cite{a} gives $x$, then $x$ again.", &[], &[]), "⟦3⟧ gives ⟦1⟧, then ⟦2⟧ again.");
    }

    #[test]
    fn pending_marks_are_locked() {
        let m = macros_of(&lore_notation::Notation::default());
        let t = "Dice reached \\pending{P1}{Dice on the external set}, above the baseline\\pendingdep{P1}.";
        let l = lock(t, &m, &[]);
        assert_eq!(l.slots, ["\\pending{P1}{Dice on the external set}", "\\pendingdep{P1}"]);
        let (_, problems) = l.restore("Dice reached ⟦1⟧, above the baseline.", &m, &[]);
        assert_eq!(problems, [Problem::Missing { what: "\\pendingdep{P1}".into() }], "a dropped dependency is refused");
    }

    #[test]
    fn literals_are_locked_too() {
        let br = vec!["\n".to_string()];
        let l = lock("First line\nsecond $y$", &[], &br);
        assert_eq!(l.slots, ["\n", "$y$"]);
        assert_eq!(l.restore("First ⟦1⟧ second ⟦2⟧", &[], &br).0, "First \n second $y$");
    }
}
