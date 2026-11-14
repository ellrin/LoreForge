//! Style checks from data: word lists in the workspace's `style/checks.toml`, turned on by
//! hard rows in rules.csv. Code knows the kinds of check; which phrases, adverbs or openers
//! count is the author's data.
//!
//! `checks.toml` holds named lists. An entry is a string, or `{ find, use }` with the
//! suggested replacement:
//!
//! ```toml
//! wordy = [{ find = "in order to", use = "to" }]
//! jargon = ["at the end of the day"]
//! comma_splice = ["however", "nevertheless"]
//! ```
//!
//! rules.csv `check` (type hard, `threshold` = the list's name):
//! - `phrases`             any entry found in a sentence (whole words, any case)
//! - `before_number`       an entry followed by a number ("over 300" → "more than 300")
//! - `comma_splice`        ", however," joining two clauses (a comma before the adverb and one
//!                         after it; "However," at the start and "; however," are fine)
//! - `sentence_start`      a sentence that starts with an entry ("And", "But")
//! - `end_preposition`     a sentence whose last word is an entry
//! - `double_comparative`  "more"/"most" before an entry ("more darker")
//! - `contractions`        it's, don't, we're… (no list)
//! - `exclamation`         "!" in prose (no list)
//!
//! Only prose is read: `%` comments, maths, verbatim and maths environments, tables, and
//! command arguments (`\cite`, `\ref`, `\label`…) are skipped; the text of `\emph`,
//! `\textit`, `\caption`, `\footnote` and the like is checked.

use crate::check::contains_word;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const FILE: &str = "checks.toml";

/// The check kinds this module runs, and whether each needs a list.
pub const KINDS: &[(&str, bool)] = &[
    ("phrases", true),
    ("before_number", true),
    ("comma_splice", true),
    ("sentence_start", true),
    ("end_preposition", true),
    ("double_comparative", true),
    ("contractions", false),
    ("exclamation", false),
];

pub fn is_kind(check: &str) -> bool {
    KINDS.iter().any(|(k, _)| *k == check)
}

pub fn needs_list(check: &str) -> bool {
    KINDS.iter().any(|(k, l)| *k == check && *l)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub find: String,
    /// The suggested replacement (empty: just avoid it).
    pub replace: String,
}

#[derive(Debug, Clone, Default)]
pub struct StyleChecks {
    pub lists: BTreeMap<String, Vec<Entry>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Plain(String),
    Pair {
        find: String,
        #[serde(default, rename = "use")]
        replace: String,
    },
}

impl StyleChecks {
    /// Read `checks.toml`; a missing file is no lists and no problem. Problems are returned,
    /// never fatal: a bad list is skipped.
    pub fn load(file: &Path) -> (StyleChecks, Vec<String>) {
        match std::fs::read_to_string(file) {
            Ok(raw) => StyleChecks::parse(&raw),
            Err(_) => (StyleChecks::default(), vec![]),
        }
    }

    pub fn parse(raw: &str) -> (StyleChecks, Vec<String>) {
        let mut out = StyleChecks::default();
        let mut problems = vec![];
        let table: toml::Table = match toml::from_str(raw) {
            Ok(t) => t,
            Err(e) => return (out, vec![e.to_string()]),
        };
        for (name, v) in table {
            match Vec::<RawEntry>::deserialize(v) {
                Ok(items) => {
                    let list = items
                        .into_iter()
                        .map(|e| match e {
                            RawEntry::Plain(f) => Entry { find: f.trim().to_string(), replace: String::new() },
                            RawEntry::Pair { find, replace } => Entry { find: find.trim().to_string(), replace: replace.trim().to_string() },
                        })
                        .filter(|e| !e.find.is_empty())
                        .collect();
                    out.lists.insert(name, list);
                }
                Err(_) => problems.push(format!("{name}: not a list of strings or {{ find, use }}")),
            }
        }
        (out, problems)
    }

    pub fn list(&self, name: &str) -> &[Entry] {
        self.lists.get(name.trim()).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// One thing a check found in a sentence.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub found: String,
    pub suggest: String,
}

/// Run one kind on one sentence of prose (see [`prose_sentences`]).
pub fn run(kind: &str, list: &[Entry], prose: &str) -> Vec<Hit> {
    let low = format!(" {} ", prose.to_lowercase());
    let hit = |e: &Entry| Hit { found: e.find.clone(), suggest: e.replace.clone() };
    match kind {
        "phrases" => list.iter().filter(|e| contains_word(&low, &e.find.to_lowercase())).map(hit).collect(),
        "before_number" => list
            .iter()
            .filter(|e| {
                let t = e.find.to_lowercase();
                word_matches(&low, &t).any(|end| low[end..].trim_start().starts_with(|c: char| c.is_ascii_digit()))
            })
            .map(hit)
            .collect(),
        "comma_splice" => list
            .iter()
            .filter(|e| {
                let t = e.find.to_lowercase();
                word_matches(&low, &t).any(|end| {
                    let start = end - t.len();
                    low[..start].trim_end().ends_with(',') && low[end..].trim_start().starts_with(',')
                })
            })
            .map(|e| Hit { found: format!(", {},", e.find), suggest: format!("; {},", e.find) })
            .collect(),
        "sentence_start" => {
            let first = low.trim_start().trim_start_matches(|c: char| !c.is_alphanumeric() && c != '∎');
            list.iter()
                .filter(|e| {
                    let t = e.find.to_lowercase();
                    first.starts_with(&t) && !first[t.len()..].starts_with(char::is_alphanumeric)
                })
                .map(hit)
                .collect()
        }
        "end_preposition" => {
            let last = low.trim_end().trim_end_matches(|c: char| !c.is_alphanumeric() && c != '∎').rsplit(|c: char| !c.is_alphanumeric() && c != '∎').next().unwrap_or("");
            list.iter().filter(|e| e.find.eq_ignore_ascii_case(last)).map(hit).collect()
        }
        "double_comparative" => {
            let words: Vec<&str> = low.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
            list.iter()
                .filter(|e| words.windows(2).any(|w| matches!(w[0], "more" | "most") && e.find.eq_ignore_ascii_case(w[1])))
                .map(|e| Hit { found: format!("more {}", e.find), suggest: if e.replace.is_empty() { e.find.clone() } else { e.replace.clone() } })
                .collect()
        }
        "contractions" => contractions(prose),
        "exclamation" => {
            if prose.contains('!') {
                vec![Hit { found: "!".into(), suggest: ".".into() }]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// Ends (byte offsets) of whole-word matches of `term` in `hay`.
fn word_matches<'a>(hay: &'a str, term: &'a str) -> impl Iterator<Item = usize> + 'a {
    hay.match_indices(term).filter_map(move |(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + term.len()..].chars().next();
        (!term.is_empty() && !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)).then_some(i + term.len())
    })
}

const PRONOUN_S: &[(&str, &str)] = &[
    ("it", "it is"),
    ("that", "that is"),
    ("there", "there is"),
    ("here", "here is"),
    ("what", "what is"),
    ("who", "who is"),
    ("he", "he is"),
    ("she", "she is"),
    ("let", "let us"),
];

fn contractions(prose: &str) -> Vec<Hit> {
    let mut out: Vec<Hit> = vec![];
    for raw in prose.split_whitespace() {
        let w: String = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’').replace('’', "'");
        let w = w.trim_matches('\'');
        let lw = w.to_lowercase();
        let Some(ap) = lw.find('\'') else { continue };
        let (stem, tail) = (&lw[..ap], &lw[ap + 1..]);
        let full = if lw.ends_with("n't") {
            match lw.as_str() {
                "can't" => Some("cannot".to_string()),
                "won't" => Some("will not".into()),
                "shan't" => Some("shall not".into()),
                _ => Some(format!("{} not", &lw[..lw.len() - 3])),
            }
        } else {
            match tail {
                "re" => Some(format!("{stem} are")),
                "ve" => Some(format!("{stem} have")),
                "ll" => Some(format!("{stem} will")),
                "d" => Some(format!("{stem} would")),
                "m" => Some(format!("{stem} am")),
                "s" => PRONOUN_S.iter().find(|(p, _)| *p == stem).map(|(_, f)| f.to_string()),
                _ => None,
            }
        };
        let Some(mut full) = full else { continue };
        if w.starts_with(char::is_uppercase) {
            full = full[..1].to_uppercase() + &full[1..];
        }
        if !out.iter().any(|h| h.found == w) {
            out.push(Hit { found: w.to_string(), suggest: full });
        }
    }
    out
}

// ── prose only ────────────────────────────────────────────────────────────────

/// Environments read as no prose at all.
const SKIP_ENVS: &[&str] = &[
    "verbatim", "Verbatim", "lstlisting", "minted", "comment", "equation", "align", "gather", "multline", "eqnarray", "displaymath",
    "math", "split", "tabular", "tabularx", "tabulary", "longtable", "array", "tikzpicture", "algorithmic", "algorithm", "thebibliography",
];

/// Commands whose `{…}` text is prose; every other command's arguments are skipped.
const TEXT_CMDS: &[&str] = &[
    "emph", "textit", "textbf", "textsc", "textrm", "textsf", "texttt", "textsl", "underline", "mbox", "text", "footnote", "caption",
    "section", "subsection", "subsubsection", "paragraph", "subparagraph", "chapter", "item", "enquote", "term",
];

/// Stands for a piece of maths: a word that is no number and no English.
const MATH: &str = " ∎ ";

/// The sentences of a paragraph as prose, with the index of the checker's own sentence when
/// they line up one to one (they do unless a comment or a skipped block holds a full stop).
pub fn prose_sentences(paragraph: &str, raw_sentences: usize) -> Vec<(Option<usize>, String)> {
    let blocks = mask_blocks(paragraph);
    let ss = crate::text::sentences(&blocks);
    let aligned = ss.len() == raw_sentences;
    ss.iter()
        .enumerate()
        .map(|(i, s)| (aligned.then_some(i), mask_inline(s).split_whitespace().collect::<Vec<_>>().join(" ")))
        .filter(|(_, s)| !s.is_empty())
        .collect()
}

fn mask_blocks(text: &str) -> String {
    // comments
    let mut out: String = text
        .lines()
        .map(|l| {
            let b = l.as_bytes();
            let cut = (0..b.len()).find(|&i| b[i] == b'%' && (i == 0 || b[i - 1] != b'\\')).unwrap_or(b.len());
            &l[..cut]
        })
        .collect::<Vec<_>>()
        .join("\n");
    // skipped environments
    loop {
        let found = SKIP_ENVS.iter().flat_map(|e| [e.to_string(), format!("{e}*")]).filter_map(|e| out.find(&format!("\\begin{{{e}}}")).map(|i| (i, e))).min();
        let Some((i, e)) = found else { break };
        let end = format!("\\end{{{e}}}");
        let stop = out[i..].find(&end).map(|j| i + j + end.len()).unwrap_or(out.len());
        out.replace_range(i..stop, MATH);
    }
    // display maths
    for (open, close) in [("$$", "$$"), ("\\[", "\\]")] {
        while let Some(i) = out.find(open) {
            let stop = out[i + open.len()..].find(close).map(|j| i + open.len() + j + close.len()).unwrap_or(out.len());
            out.replace_range(i..stop, MATH);
        }
    }
    out
}

fn mask_inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '$' => {
                let j = (i + 1..chars.len()).find(|&j| chars[j] == '$' && chars[j - 1] != '\\').unwrap_or(chars.len());
                out += MATH;
                i = j + 1;
            }
            '\\' if chars.get(i + 1) == Some(&'(') => {
                let j = (i + 2..chars.len()).find(|&j| chars[j] == ')' && chars[j - 1] == '\\').unwrap_or(chars.len());
                out += MATH;
                i = j + 1;
            }
            '\\' if chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()) => {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let name: String = chars[i + 1..j].iter().collect();
                if name == "verb" && j < chars.len() {
                    let d = chars[j];
                    j = (j + 1..chars.len()).find(|&k| chars[k] == d).map(|k| k + 1).unwrap_or(chars.len());
                    out.push(' ');
                    i = j;
                    continue;
                }
                if chars.get(j) == Some(&'*') {
                    j += 1;
                }
                let keep = TEXT_CMDS.contains(&name.as_str());
                out.push(' ');
                // arguments: [..] always skipped, {..} kept as prose for text commands
                loop {
                    let k = (j..chars.len()).find(|&k| !chars[k].is_whitespace()).unwrap_or(chars.len());
                    let (open, close) = match chars.get(k) {
                        Some('[') => ('[', ']'),
                        Some('{') => ('{', '}'),
                        _ => break,
                    };
                    let mut depth = 0;
                    let mut e = k;
                    while e < chars.len() {
                        if chars[e] == open {
                            depth += 1;
                        } else if chars[e] == close {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        e += 1;
                    }
                    if keep && open == '{' {
                        let inner: String = chars[k + 1..e.min(chars.len())].iter().collect();
                        out += &mask_inline(&inner);
                        out.push(' ');
                    }
                    j = (e + 1).min(chars.len());
                }
                i = j;
            }
            '\\' => {
                // \%, \&, \_ … as the character; \\ and \, as a space
                match chars.get(i + 1) {
                    Some(&ch) if "%&_#$".contains(ch) => out.push(ch),
                    _ => out.push(' '),
                }
                i += 2;
            }
            '~' | '{' | '}' => {
                out.push(' ');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(find: &str, replace: &str) -> Entry {
        Entry { find: find.into(), replace: replace.into() }
    }

    fn found(kind: &str, list: &[Entry], text: &str) -> Vec<String> {
        prose_sentences(text, usize::MAX).iter().flat_map(|(_, s)| run(kind, list, s)).map(|h| h.found).collect()
    }

    #[test]
    fn the_data_file_has_named_lists_of_strings_or_pairs() {
        let (c, p) = StyleChecks::parse("wordy = [{ find = \"in order to\", use = \"to\" }, \"a number of\"]\nadverbs = [\"however\"]\nbad = 3\n");
        assert_eq!(c.list("wordy"), &[e("in order to", "to"), e("a number of", "")]);
        assert_eq!(c.list("adverbs").len(), 1);
        assert!(c.list("missing").is_empty());
        assert_eq!(p.len(), 1, "a value that is no list is reported: {p:?}");
        assert!(StyleChecks::parse("x = [").1.len() == 1);
        assert!(StyleChecks::load(Path::new("/nonexistent/checks.toml")).1.is_empty());
    }

    #[test]
    fn phrases_whole_words_any_case_with_the_suggestion() {
        let l = [e("in order to", "to"), e("is comprised of", "comprises"), e("not uncommon", "common")];
        let h = run("phrases", &l, "In order to see, the set is comprised of parts.");
        assert_eq!(h.iter().map(|h| h.suggest.as_str()).collect::<Vec<_>>(), ["to", "comprises"]);
        assert!(run("phrases", &l, "This is not uncommonly seen.").is_empty(), "whole words only");
        assert_eq!(found("phrases", &l, "Such failures are not uncommon."), ["not uncommon"]);
    }

    #[test]
    fn latex_is_never_read_as_prose() {
        let l = [e("in order to", "to"), e("prior to", "before")];
        for t in [
            "We follow \\cite[in order to]{in order to} closely.",
            "We follow Section~\\ref{sec:in-order-to} closely.",
            "We fix $x$ in order\\label{prior to}.",
            "The value $\\text{in order to}$ is set.",
            "We use it. % in order to check this later",
            "We use it.\n\\begin{verbatim}\nin order to\n\\end{verbatim}",
            "We use it \\[ \\text{prior to} \\] here.",
            "Run \\verb|in order to| now.",
        ] {
            assert!(found("phrases", &l, t).is_empty(), "{t}");
        }
        assert_eq!(found("phrases", &l, "We wait \\emph{prior to} the test."), ["prior to"], "text commands are prose");
        assert_eq!(found("phrases", &l, "Data were cleaned in order to\\footnote{x} help."), ["in order to"]);
    }

    #[test]
    fn comma_splice_only_between_commas() {
        let l = [e("however", ""), e("nevertheless", "")];
        let h = run("comma_splice", &l, "The model converged, however, the loss stayed high.");
        assert_eq!(h, [Hit { found: ", however,".into(), suggest: "; however,".into() }]);
        assert!(run("comma_splice", &l, "However, the loss stayed high.").is_empty(), "sentence start is fine");
        assert!(run("comma_splice", &l, "The model converged; however, the loss stayed high.").is_empty());
        assert!(run("comma_splice", &l, "The model converged, however slowly.").is_empty());
        assert_eq!(found("comma_splice", &l, "It ran \\cite{a}, nevertheless, it failed."), [", nevertheless,"]);
    }

    #[test]
    fn openers_endings_numbers_and_comparatives() {
        let starts = [e("And", ""), e("But", "However,")];
        assert_eq!(run("sentence_start", &starts, "But the loss rose.")[0].suggest, "However,");
        assert!(run("sentence_start", &starts, "Andrew agreed.").is_empty());
        assert!(run("sentence_start", &starts, "The loss rose, but not much.").is_empty());
        let preps = [e("with", ""), e("of", "")];
        assert_eq!(run("end_preposition", &preps, "These are the tools we work with.").len(), 1);
        assert!(run("end_preposition", &preps, "We work with these tools.").is_empty());
        assert!(found("end_preposition", &preps, "It is the size of $X$.").is_empty(), "maths is a word");
        let over = [e("over", "more than"), e("in excess of", "more than")];
        assert_eq!(run("before_number", &over, "We used over 300 samples.")[0].suggest, "more than");
        assert_eq!(run("before_number", &over, "Costs were in excess of 5 units.").len(), 1);
        assert!(run("before_number", &over, "We averaged over folds.").is_empty());
        let cmp = [e("darker", ""), e("better", "")];
        assert_eq!(run("double_comparative", &cmp, "The image is more darker.")[0].suggest, "darker");
        assert!(run("double_comparative", &cmp, "The image is darker and better.").is_empty());
    }

    #[test]
    fn contractions_and_exclamations() {
        let h = run("contractions", &[], "It's clear that we don't know; they’re late and Smith's model can't run.");
        let pairs: Vec<(String, String)> = h.into_iter().map(|h| (h.found, h.suggest)).collect();
        assert_eq!(
            pairs,
            [("It's".into(), "It is".into()), ("don't".into(), "do not".into()), ("they're".into(), "they are".into()), ("can't".into(), "cannot".into())]
        );
        assert!(run("contractions", &[], "The model's output and its loss.").is_empty(), "possessives are fine");
        assert_eq!(run("exclamation", &[], "It works!").len(), 1);
        assert!(found("exclamation", &[], "Spacing $a\\!b$ is tight.").is_empty());
    }

    #[test]
    fn sentence_indices_line_up_or_are_left_out() {
        let s = prose_sentences("One here. Two $a. b$ there.", 2);
        assert_eq!(s.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [Some(0), Some(1)]);
        let s = prose_sentences("One here. % a. b. c.\nTwo there.", 5);
        assert!(s.iter().all(|(i, _)| i.is_none()));
    }
}
