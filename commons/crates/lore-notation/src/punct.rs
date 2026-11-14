//! A punctuation and sentence guard for generated English prose (LaTeX source).
//!
//! Off unless a rule is given (the author's choice lives in their private style folder):
//! with a [`Rule`], only the allowed marks may appear in prose (periods and commas, say),
//! sentences stay under a word and a comma count, and hyphenated compounds are written as
//! open words unless the term is the document's own (its references, its notation) or on
//! a short whitelist. Maths, `\cite`/`\ref`/`\label` and the other reference commands, the
//! pending marks, numbers (decimals, thousands), `\%`, possessive apostrophes and
//! parentheses for abbreviations and statistics are never violations.
//!
//! [`check`] finds the violations sentence by sentence, [`repair_note`] names them for one
//! repair call, [`fix`] applies the safe mechanical fixes (a semicolon becomes a full stop,
//! a number range "0.70–0.75" becomes "0.70 to 0.75", "data-driven" becomes "data driven",
//! quotation marks go); long sentences and asides have no mechanical fix and stay flagged.

use serde::Serialize;

/// Marks the guard knows. A mark in [`Rule::allowed`] is never a violation.
const MARKS: &[char] = &[':', ';', '—', '–', '"', '“', '”', '‘', '’', '!', '?'];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rule {
    /// Punctuation allowed in prose besides what is always fine (parentheses, apostrophes of
    /// possession). Empty = the guard is off.
    pub allowed: Vec<char>,
    /// Hyphenated terms always allowed (case-insensitive), e.g. `X-ray`, `COVID-19`.
    pub hyphen_whitelist: Vec<String>,
    /// Words per sentence at most.
    pub max_words: Option<usize>,
    /// Commas per sentence at most (numbers, maths, citations and statistics in parentheses
    /// do not count).
    pub max_commas: Option<usize>,
}

impl Rule {
    pub fn enabled(&self) -> bool {
        !self.allowed.is_empty()
    }

    /// `[punctuation]` of a TOML file: `allowed = [".", ","]`, `hyphen_whitelist = […]`.
    /// None when the table is missing or empty (the guard stays off).
    pub fn from_toml(raw: &str) -> Option<Rule> {
        let t: toml::Table = toml::from_str(raw).ok()?;
        let p = t.get("punctuation")?.as_table()?;
        let strings = |k: &str| -> Vec<String> { p.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let allowed: Vec<char> = strings("allowed").iter().filter_map(|s| s.trim().chars().next()).collect();
        if allowed.is_empty() {
            return None;
        }
        Some(Rule { allowed, hyphen_whitelist: strings("hyphen_whitelist"), max_words: None, max_commas: None })
    }

    /// The prompt line (short: the code check is the guarantee).
    pub fn prompt_line(&self) -> String {
        if !self.enabled() {
            return String::new();
        }
        let marks: String = self.allowed.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(" and ");
        let mut s = format!(
            "Punctuation in prose: only {marks} (parentheses only for an abbreviation or statistics). No colons, semicolons, dashes, quotation marks or exclamation marks; a range is written \"0.70 to 0.75\"; write compounds as open words (\"data driven\", not \"data-driven\") unless the term is a name."
        );
        if let Some(w) = self.max_words {
            s += &format!(" At most {w} words per sentence.");
        }
        if let Some(c) = self.max_commas {
            s += &format!(" At most {c} commas per sentence.");
        }
        s + "\n"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Colon,
    Semicolon,
    Dash,
    /// A number range written with a dash or hyphen (`0.70–0.75`, `2010-2022`).
    Range,
    Quote,
    Exclamation,
    Question,
    /// A hyphenated compound that is not the document's own term.
    Hyphen,
    /// A parenthesis that is an aside (more than a few words), not an abbreviation or statistics.
    Aside,
    LongSentence,
    Commas,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Violation {
    pub kind: Kind,
    /// What was found (the mark, the compound, "41 words", "5 commas").
    pub found: String,
    /// 1-based sentence number in the text.
    pub sentence: usize,
    /// The sentence, as plain words, cut.
    pub text: String,
}

impl Violation {
    pub fn describe(&self) -> String {
        let what = match self.kind {
            Kind::Colon => "colon".to_string(),
            Kind::Semicolon => "semicolon".into(),
            Kind::Dash => format!("dash \"{}\"", self.found),
            Kind::Range => format!("range \"{}\": write \"A to B\"", self.found),
            Kind::Quote => "quotation mark".into(),
            Kind::Exclamation => "exclamation mark".into(),
            Kind::Question => "question mark".into(),
            Kind::Hyphen => format!("hyphenated \"{}\": write it as open words", self.found),
            Kind::Aside => format!("parenthetical aside \"{}\"", self.found),
            Kind::LongSentence => format!("{}: split the sentence", self.found),
            Kind::Commas => format!("{}: split the sentence", self.found),
        };
        format!("sentence {} (\"{}\"): {what}", self.sentence, self.text)
    }
}

/// Commands whose braced arguments are never prose.
const OPAQUE: &[&str] = &[
    "cite", "citep", "citet", "citealp", "parencite", "textcite", "ref", "eqref", "cref", "Cref", "autoref", "pageref", "label", "url", "href", "input",
    "include", "includegraphics", "pending", "pendingdep", "pendingcell", "pendingpara", "begin", "end", "SI", "si", "num",
];
/// Environments whose body is never prose.
const MATH_ENVS: &[&str] = &["equation", "equation*", "align", "align*", "gather", "gather*", "multline", "multline*", "eqnarray", "eqnarray*", "displaymath", "math"];

/// `text` with everything that is not prose replaced by `X` (letters, so nothing reads as a
/// mark), byte for byte the same length: maths, opaque commands with their arguments, other
/// command names, `\%`, comments.
fn masked(text: &str) -> Vec<u8> {
    let b = text.as_bytes();
    let mut m = b.to_vec();
    let fill = |m: &mut Vec<u8>, from: usize, to: usize| {
        for x in m.iter_mut().take(to.min(b.len())).skip(from) {
            if !x.is_ascii_whitespace() {
                *x = b'X';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i == 0 || b[i - 1] != b'\\' => {
                let e = text[i..].find('\n').map(|k| i + k).unwrap_or(b.len());
                fill(&mut m, i, e);
                i = e;
            }
            b'$' => {
                let dbl = b.get(i + 1) == Some(&b'$');
                let open = if dbl { 2 } else { 1 };
                let close = if dbl { "$$" } else { "$" };
                let e = text[i + open..].find(close).map(|k| i + open + k + open).unwrap_or(b.len());
                fill(&mut m, i, e);
                i = e;
            }
            b'\\' => {
                let rest = &text[i + 1..];
                if rest.starts_with('(') || rest.starts_with('[') {
                    let close = if rest.starts_with('(') { "\\)" } else { "\\]" };
                    let e = text[i + 2..].find(close).map(|k| i + 2 + k + 2).unwrap_or(b.len());
                    fill(&mut m, i, e);
                    i = e;
                    continue;
                }
                let name: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                if name.is_empty() {
                    // \% \& \, \\ …: one escaped character
                    let e = (i + 2).min(b.len());
                    fill(&mut m, i, e);
                    i = e;
                    continue;
                }
                let mut e = i + 1 + name.len();
                if name == "begin" {
                    let env = rest[name.len()..].trim_start().strip_prefix('{').and_then(|r| r.split('}').next()).unwrap_or("");
                    if MATH_ENVS.contains(&env) {
                        let end_tag = format!("\\end{{{env}}}");
                        let stop = text[e..].find(&end_tag).map(|k| e + k + end_tag.len()).unwrap_or(b.len());
                        fill(&mut m, i, stop);
                        i = stop;
                        continue;
                    }
                }
                if OPAQUE.contains(&name.as_str()) {
                    // every following [..] and {..}
                    loop {
                        let mut k = e;
                        while k < b.len() && b[k] == b' ' {
                            k += 1;
                        }
                        let (open, close) = match b.get(k) {
                            Some(b'{') => (b'{', b'}'),
                            Some(b'[') => (b'[', b']'),
                            _ => break,
                        };
                        let mut depth = 0;
                        let mut j = k;
                        while j < b.len() {
                            if b[j] == open {
                                depth += 1;
                            } else if b[j] == close {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            j += 1;
                        }
                        e = (j + 1).min(b.len());
                    }
                }
                fill(&mut m, i, e);
                i = e;
            }
            // a note for the author ([Author to confirm: …]) is not prose
            b'[' if text[i..].starts_with("[Author") => {
                let e = text[i..].find(']').map(|k| i + k + 1).unwrap_or(b.len());
                // blank, not X: its words do not count toward the sentence
                for x in m.iter_mut().take(e).skip(i) {
                    *x = b' ';
                }
                i = e;
            }
            b'{' | b'}' | b'~' => {
                m[i] = b' ';
                i += 1;
            }
            _ => i += 1,
        }
    }
    m
}

/// Byte ranges of the sentences of a masked text.
fn sentence_ranges(m: &[u8]) -> Vec<(usize, usize)> {
    const ABBR: &[&str] = &["e.g", "i.e", "al", "Fig", "Figs", "Eq", "Eqs", "vs", "approx", "cf", "No", "Ref", "Refs", "etc"];
    let mut out = vec![];
    let mut start = 0;
    let mut i = 0;
    while i < m.len() {
        if matches!(m[i], b'.' | b'?' | b'!') && m.get(i + 1).is_none_or(|c| c.is_ascii_whitespace()) {
            let word_start = m[..i].iter().rposition(|c| c.is_ascii_whitespace()).map(|k| k + 1).unwrap_or(0);
            let word = String::from_utf8_lossy(&m[word_start..i]).trim_start_matches('(').to_string();
            let mut k = i + 1;
            while k < m.len() && m[k].is_ascii_whitespace() {
                k += 1;
            }
            let next_ok = k >= m.len() || m[k].is_ascii_uppercase() || m[k] == b'X' || !m[k].is_ascii_alphabetic() && m[k] != b'(';
            if !ABBR.contains(&word.as_str()) && next_ok {
                out.push((start, i + 1));
                start = k;
            }
        }
        i += 1;
    }
    if start < m.len() && m[start..].iter().any(|c| !c.is_ascii_whitespace()) {
        out.push((start, m.len()));
    }
    out
}

fn plain_cut(text: &str, from: usize, to: usize) -> String {
    let mut f = from.min(text.len());
    while !text.is_char_boundary(f) {
        f -= 1;
    }
    let mut t = to.min(text.len());
    while !text.is_char_boundary(t) {
        t += 1;
    }
    let s: String = text[f..t].split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > 90 { s.chars().take(88).collect::<String>() + "…" } else { s }
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// Is the hyphenated token one the document or the whitelist names?
fn hyphen_ok(token: &str, rule: &Rule, exempt: &[String]) -> bool {
    let t = token.to_lowercase();
    rule.hyphen_whitelist.iter().chain(exempt.iter()).any(|w| {
        let w = w.to_lowercase();
        w == t || w.split(|c: char| !(c.is_alphanumeric() || c == '-')).any(|x| x == t)
    })
}

/// One finding before it is tied to a sentence: byte range, kind, what.
fn marks_in(text: &str, m: &[u8], rule: &Rule, exempt: &[String]) -> Vec<(usize, usize, Kind, String)> {
    let mut out = vec![];
    let allowed = |c: char| rule.allowed.contains(&c);
    let mut i = 0;
    while i < m.len() {
        let c = m[i];
        // multi-byte marks (— – “ ” ‘ ’)
        if c >= 0x80 {
            let ch = text[i..].chars().next().unwrap_or(' ');
            let len = ch.len_utf8();
            if MARKS.contains(&ch) && !allowed(ch) && m[i] != b'X' {
                let digit_before = i > 0 && m[i - 1].is_ascii_digit();
                let digit_after = m.get(i + len).is_some_and(|x| x.is_ascii_digit());
                let kind = match ch {
                    '—' => Kind::Dash,
                    '–' if digit_before && digit_after => Kind::Range,
                    '–' => Kind::Dash,
                    '’' if i > 0 && m[i - 1].is_ascii_alphabetic() && (m.get(i + len) == Some(&b's') || m[i - 1] == b's') => {
                        i += len;
                        continue;
                    }
                    _ => Kind::Quote,
                };
                let found = if kind == Kind::Range { range_text(text, m, i, i + len) } else { ch.to_string() };
                out.push((i, i + len, kind, found));
            }
            i += len;
            continue;
        }
        match c {
            // a ratio or a time (8:2, 10:30) is a number, not a colon
            b':' if i > 0 && m[i - 1].is_ascii_digit() && m.get(i + 1).is_some_and(|x| x.is_ascii_digit()) => {}
            b':' | b';' | b'!' | b'?' | b'"' if !allowed(c as char) => {
                let kind = match c {
                    b':' => Kind::Colon,
                    b';' => Kind::Semicolon,
                    b'!' => Kind::Exclamation,
                    b'?' => Kind::Question,
                    _ => Kind::Quote,
                };
                out.push((i, i + 1, kind, (c as char).to_string()));
            }
            b'`' | b'\'' if !allowed('"') => {
                // ``…'' quotes; an apostrophe of possession or a contraction is not a quote
                let possessive = c == b'\'' && i > 0 && m[i - 1].is_ascii_alphabetic() && (m.get(i + 1).is_some_and(|x| x.is_ascii_alphabetic()) || m[i - 1] == b's');
                if !possessive {
                    let e = if m.get(i + 1) == Some(&c) { i + 2 } else { i + 1 };
                    out.push((i, e, Kind::Quote, text[i..e].to_string()));
                    i = e;
                    continue;
                }
            }
            b'-' => {
                let run = m[i..].iter().take_while(|x| **x == b'-').count();
                let before = i > 0 && is_word_char(m[i - 1]);
                let after = m.get(i + run).is_some_and(|x| is_word_char(*x));
                let digit_before = i > 0 && m[i - 1].is_ascii_digit();
                let digit_after = m.get(i + run).is_some_and(|x| x.is_ascii_digit());
                let spaced = i > 0 && m[i - 1] == b' ' && m.get(i + run) == Some(&b' ');
                if run >= 2 {
                    let kind = if run == 2 && digit_before && digit_after { Kind::Range } else { Kind::Dash };
                    if !allowed('—') {
                        let found = if kind == Kind::Range { range_text(text, m, i, i + run) } else { "-".repeat(run) };
                        out.push((i, i + run, kind, found));
                    }
                } else if spaced && !allowed('—') {
                    out.push((i, i + 1, Kind::Dash, " - ".into()));
                } else if before && after {
                    // the whole token: letters, digits and hyphens
                    let s = m[..i].iter().rposition(|x| !(is_word_char(*x) || *x == b'-')).map(|k| k + 1).unwrap_or(0);
                    let e = m[i..].iter().position(|x| !(is_word_char(*x) || *x == b'-')).map(|k| i + k).unwrap_or(m.len());
                    let token = &text[s..e];
                    let masked_part = m[s..e].contains(&b'X') && !token.contains('X');
                    let numeric = token.split('-').all(|p| p.chars().all(|c| c.is_ascii_digit() || c == '.'));
                    // 1.2E-5, 3e-4: a number in E notation
                    let e_notation = m[..i].ends_with(b"E") || m[..i].ends_with(b"e");
                    let e_notation = e_notation && i >= 2 && m[i - 2].is_ascii_digit() && digit_after;
                    if e_notation {
                        i = e;
                        continue;
                    }
                    if numeric && digit_before && digit_after {
                        out.push((i, i + 1, Kind::Range, range_text(text, m, i, i + 1)));
                    } else if !masked_part && !hyphen_ok(token, rule, exempt) && !allowed('-') {
                        out.push((s, e, Kind::Hyphen, token.to_string()));
                    }
                    i = e;
                    continue;
                }
                i += run;
                continue;
            }
            b'(' => {
                // an aside: more than six words with no digit (abbreviations and statistics are short or numeric)
                if let Some(k) = m[i..].iter().position(|x| *x == b')') {
                    let inner = String::from_utf8_lossy(&m[i + 1..i + k]).to_string();
                    let words = inner.split_whitespace().count();
                    if words > 6 && !inner.chars().any(|c| c.is_ascii_digit()) {
                        out.push((i, i + k + 1, Kind::Aside, plain_cut(text, i, i + k + 1)));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn range_text(text: &str, m: &[u8], s: usize, e: usize) -> String {
    let a = m[..s].iter().rposition(|x| !(x.is_ascii_digit() || *x == b'.')).map(|k| k + 1).unwrap_or(0);
    let b = m[e..].iter().position(|x| !(x.is_ascii_digit() || *x == b'.')).map(|k| e + k).unwrap_or(m.len());
    text[a..b].trim_end_matches('.').to_string()
}

/// Commas that count in a masked sentence: not inside a number (65,133), not inside a
/// parenthesis that holds a number (statistics).
fn commas(m: &[u8]) -> usize {
    let mut n = 0;
    let mut depth_num: Vec<bool> = vec![];
    for (i, c) in m.iter().enumerate() {
        match c {
            b'(' => {
                let close = m[i..].iter().position(|x| *x == b')').map(|k| i + k).unwrap_or(m.len());
                depth_num.push(m[i..close].iter().any(|x| x.is_ascii_digit()));
            }
            b')' => {
                depth_num.pop();
            }
            b',' => {
                let in_number = i > 0 && m[i - 1].is_ascii_digit() && m.get(i + 1).is_some_and(|x| x.is_ascii_digit());
                if !in_number && !depth_num.iter().any(|x| *x) {
                    n += 1;
                }
            }
            _ => {}
        }
    }
    n
}

/// The commas of one sentence that count (see [`Rule::max_commas`]).
pub fn sentence_commas(sentence: &str) -> usize {
    commas(&masked(sentence))
}

fn words(m: &[u8]) -> usize {
    String::from_utf8_lossy(m).split_whitespace().filter(|w| w.chars().any(|c| c.is_alphanumeric())).count()
}

/// Every violation of `rule` in `text` (LaTeX prose), in order. `exempt` = the document's own
/// hyphenated terms (reference titles and keys, notation names). Off rule: nothing.
pub fn check(text: &str, rule: &Rule, exempt: &[String]) -> Vec<Violation> {
    let enforce_sentences = rule.max_words.is_some() || rule.max_commas.is_some();
    if !rule.enabled() && !enforce_sentences {
        return vec![];
    }
    let m = masked(text);
    let sents = sentence_ranges(&m);
    let sent_of = |pos: usize| sents.iter().position(|(s, e)| pos >= *s && pos < *e).unwrap_or(sents.len().saturating_sub(1));
    let mut out = vec![];
    if rule.enabled() {
        for (s, _, kind, found) in marks_in(text, &m, rule, exempt) {
            let k = sent_of(s);
            let (a, b) = sents.get(k).copied().unwrap_or((0, text.len()));
            out.push(Violation { kind, found, sentence: k + 1, text: plain_cut(text, a, b) });
        }
    }
    for (k, (a, b)) in sents.iter().enumerate() {
        let w = words(&m[*a..*b]);
        if rule.max_words.is_some_and(|x| w > x) {
            out.push(Violation { kind: Kind::LongSentence, found: format!("{w} words (at most {})", rule.max_words.unwrap_or(0)), sentence: k + 1, text: plain_cut(text, *a, *b) });
        }
        let c = commas(&m[*a..*b]);
        if rule.max_commas.is_some_and(|x| c > x) {
            out.push(Violation { kind: Kind::Commas, found: format!("{c} commas (at most {})", rule.max_commas.unwrap_or(0)), sentence: k + 1, text: plain_cut(text, *a, *b) });
        }
    }
    out.sort_by_key(|v| v.sentence);
    out
}

/// The repair call's instruction: every violation by sentence.
pub fn repair_note(v: &[Violation]) -> String {
    if v.is_empty() {
        return String::new();
    }
    let mut s = "Punctuation and sentence rules broken; fix exactly these and change nothing else:\n".to_string();
    for x in v {
        s += &format!("- {}\n", x.describe());
    }
    s
}

/// The safe mechanical fixes (see the module text). Long sentences, too many commas, asides
/// and question marks are left as they are: [`check`] still reports them.
pub fn fix(text: &str, rule: &Rule, exempt: &[String]) -> String {
    if !rule.enabled() {
        return text.to_string();
    }
    let m = masked(text);
    let mut edits: Vec<(usize, usize, String)> = vec![];
    for (s, e, kind, found) in marks_in(text, &m, rule, exempt) {
        let rep = match kind {
            Kind::Semicolon | Kind::Exclamation => {
                // a full stop; the next word starts the new sentence
                let rest = &text[e..];
                let ws = rest.len() - rest.trim_start().len();
                let next = rest.trim_start().chars().next();
                match next {
                    Some(c) if c.is_lowercase() => {
                        edits.push((s, e + ws + c.len_utf8(), format!(". {}", c.to_uppercase())));
                        continue;
                    }
                    _ => ".".to_string(),
                }
            }
            Kind::Colon => ",".into(),
            Kind::Dash => {
                // " — " / "—" / " -- " → ", "
                let mut a = s;
                while a > 0 && text.as_bytes()[a - 1] == b' ' {
                    a -= 1;
                }
                let mut b = e;
                while b < text.len() && text.as_bytes()[b] == b' ' {
                    b += 1;
                }
                edits.push((a, b, ", ".into()));
                continue;
            }
            Kind::Range => " to ".into(),
            Kind::Quote => String::new(),
            Kind::Hyphen => found.replace('-', " "),
            _ => continue,
        };
        edits.push((s, e, rep));
    }
    edits.sort_by_key(|x| std::cmp::Reverse(x.0));
    let mut out = text.to_string();
    let mut last = usize::MAX;
    for (s, e, r) in edits {
        if e > last {
            continue;
        }
        out.replace_range(s..e, &r);
        last = s;
    }
    // ", ," and ",." left by a dash at a clause end
    out.replace(", ,", ",").replace(",.", ".").replace(" ,", ",")
}

/// The hyphenated words of a text (reference titles, notation names): the document's own terms.
pub fn hyphenated_terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .map(|w| w.trim_matches('-'))
        .filter(|w| w.contains('-') && w.chars().any(|c| c.is_alphabetic()))
        .map(str::to_string)
        .collect();
    out.sort_by_key(|w| w.to_lowercase());
    out.dedup_by_key(|w| w.to_lowercase());
    out
}

/// A rule with the document's own terms: what every generation path runs its text through.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Guard {
    pub rule: Rule,
    /// The document's hyphenated terms (reference titles and keys, notation names).
    pub exempt: Vec<String>,
}

impl Guard {
    pub fn active(&self) -> bool {
        self.rule.enabled()
    }
    pub fn check(&self, text: &str) -> Vec<Violation> {
        if !self.active() {
            return vec![];
        }
        check(text, &self.rule, &self.exempt)
    }
    pub fn fix(&self, text: &str) -> String {
        fix(text, &self.rule, &self.exempt)
    }
    /// After the repair call: the mechanical fixes, then what is still wrong (to flag, never hidden).
    pub fn finish(&self, text: &str) -> (String, Vec<Violation>) {
        if !self.active() {
            return (text.to_string(), vec![]);
        }
        let t = self.fix(text);
        let v = self.check(&t);
        (t, v)
    }
}

/// A section's paragraph lengths, in words: the range to aim for and the bounds to check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParagraphRange {
    pub target: (usize, usize),
    pub bounds: (usize, usize),
    pub sentences: Option<(usize, usize)>,
}

impl ParagraphRange {
    /// The words to aim for in paragraph `k` of `n`: a slice of the target that changes from one
    /// paragraph to the next (neighbours never get the same slice).
    pub fn aim(&self, k: usize) -> (usize, usize) {
        let (lo, hi) = self.target;
        let span = hi.saturating_sub(lo).max(3);
        let third = span / 3;
        match k % 3 {
            0 => (lo + third, hi),
            1 => (lo, lo + 2 * third),
            _ => (lo + third / 2, hi - third / 2),
        }
    }

    /// What is wrong with a written paragraph, for the repair call (None: within the bounds).
    pub fn problem(&self, text: &str) -> Option<String> {
        let m = masked(text);
        let w = words(&m);
        let n = sentence_ranges(&m).len();
        let (lo, hi) = self.bounds;
        if w > hi {
            return Some(format!("{w} words (at most {hi}): split it at the turn between two of its points, or cut what repeats"));
        }
        if w < lo {
            return Some(format!("{w} words (at least {lo}): add the reasoning the points give (never invent content), or say it is meant to merge with its neighbour"));
        }
        match self.sentences {
            Some((a, _)) if n < a => Some(format!("{n} sentences (at least {a})")),
            Some((_, b)) if n > b => Some(format!("{n} sentences (at most {b}): split the paragraph")),
            _ => None,
        }
    }
}

/// `[paragraph_words.<Section>]` tables: `target = [85, 150]`, `bounds = [75, 185]`,
/// `sentences = [4, 10]` (optional). Section names as the file writes them.
pub fn paragraph_ranges(raw: &str) -> std::collections::BTreeMap<String, ParagraphRange> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(t) = toml::from_str::<toml::Table>(raw) else { return out };
    let Some(p) = t.get("paragraph_words").and_then(|v| v.as_table()) else { return out };
    let pair = |v: Option<&toml::Value>| -> Option<(usize, usize)> {
        let a = v?.as_array()?;
        let x = a.first()?.as_integer()?;
        let y = a.get(1)?.as_integer()?;
        (x >= 0 && y >= x).then_some((x as usize, y as usize))
    };
    for (sec, v) in p {
        let Some(tb) = v.as_table() else { continue };
        let Some(target) = pair(tb.get("target")) else { continue };
        let bounds = pair(tb.get("bounds")).unwrap_or((target.0 * 2 / 3, target.1 * 3 / 2));
        out.insert(sec.clone(), ParagraphRange { target, bounds, sentences: pair(tb.get("sentences")) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Rule {
        Rule { allowed: vec!['.', ','], hyphen_whitelist: vec!["X-ray".into(), "COVID-19".into(), "Grad-CAM".into()], max_words: Some(35), max_commas: Some(3) }
    }

    fn kinds(t: &str, ex: &[String]) -> Vec<Kind> {
        check(t, &rule(), ex).into_iter().map(|v| v.kind).collect()
    }

    #[test]
    fn off_by_default() {
        assert!(Rule::from_toml("wordy = [\"a\"]").is_none());
        assert!(check("A: b; c — d.", &Rule::default(), &[]).is_empty());
        assert_eq!(fix("A: b.", &Rule::default(), &[]), "A: b.");
        let r = Rule::from_toml("[punctuation]\nallowed = [\".\", \",\"]\nhyphen_whitelist = [\"X-ray\"]\n").unwrap();
        assert_eq!((r.allowed.clone(), r.hyphen_whitelist.clone()), (vec!['.', ','], vec!["X-ray".to_string()]));
        assert!(r.prompt_line().contains("only \".\" and \",\""));
    }

    #[test]
    fn finds_the_banned_marks() {
        let t = "The model has two parts: an encoder and a decoder; both are data-driven — and \"robust\". Results improved!";
        assert_eq!(kinds(t, &[]), [Kind::Colon, Kind::Semicolon, Kind::Hyphen, Kind::Dash, Kind::Quote, Kind::Quote, Kind::Exclamation]);
        assert_eq!(kinds("AUC was 0.70–0.75 in 2010-2022 and 0.5--0.6.", &[]), [Kind::Range, Kind::Range, Kind::Range]);
        assert_eq!(check("AUC was 0.70–0.75.", &rule(), &[])[0].found, "0.70–0.75");
        assert_eq!(kinds("Results were good -- mostly.", &[]), [Kind::Dash]);
        assert_eq!(kinds("Results were good - mostly.", &[]), [Kind::Dash]);
        // numbers stay numbers: a ratio, a time, E notation
        assert!(kinds("Split 8:2 at 10:30 with p = 1.2E-5 and 3e-4.", &[]).is_empty());
        assert_eq!(fix("Split 8:2, p = 5.34E-5.", &rule(), &[]), "Split 8:2, p = 5.34E-5.");
    }

    #[test]
    fn maths_citations_numbers_and_own_terms_are_exempt() {
        let ok = "We set $a: b; c - d$ and \\(x - y\\) as in~\\cite{lee-2020,kim2019} and Table~\\ref{tab:main-auc}. \
The model's AUC (95\\% CI, 0.70 to 0.75) on 65,133 patients improved, as a region of interest (ROI) showed. \
Chest X-ray, COVID-19 and Grad-CAM maps were used \\pending{P1}{site-B AUC}\\pendingdep{P1}.\n\\begin{equation}f(x) = x; y\\end{equation}\n";
        assert_eq!(check(ok, &rule(), &[]), vec![], "{:?}", check(ok, &rule(), &[]));
        // the document's own terms (reference titles, notation)
        assert_eq!(kinds("A self-supervised encoder.", &[]), [Kind::Hyphen]);
        assert!(kinds("A self-supervised encoder.", &hyphenated_terms("Self-Supervised Learning of Chest Images")).is_empty());
        assert_eq!(hyphenated_terms("Self-Supervised x-ray, a-b-c; -x"), ["a-b-c", "Self-Supervised", "x-ray"]);
        // e.g. and et al. do not end a sentence; a long aside is flagged
        assert!(kinds("As in Lee et al. (2020), e.g. the encoder works.", &[]).is_empty());
        assert!(kinds("We trained it [Author to confirm: epochs; batch size].", &[]).is_empty(), "a note for the author is not prose");
        let note = format!("We trained it [Author to confirm: {}].", vec!["word"; 40].join(" "));
        assert!(kinds(&note, &[]).is_empty(), "its words do not count");
        assert_eq!(kinds("The encoder (which we trained for a long time on many images) works.", &[]), [Kind::Aside]);
    }

    #[test]
    fn long_sentences_and_commas() {
        let long = format!("{}.", vec!["word"; 41].join(" "));
        let v = check(&long, &rule(), &[]);
        assert_eq!((v[0].kind, v[0].found.as_str()), (Kind::LongSentence, "41 words (at most 35)"));
        assert_eq!(kinds("We used A, B, C, D, and E. Short.", &[]), [Kind::Commas]);
        assert!(kinds("We used 65,133 images (AUC 0.81, 95\\% CI 0.79, 0.83), A, B and C~\\cite{a,b,c}.", &[]).is_empty(), "numbers, statistics and citations do not count");
        let note = repair_note(&check("We used A, B, C, D, and E. Short: yes.", &rule(), &[]));
        assert!(note.contains("sentence 1 (\"We used A, B, C, D, and E.\"): 4 commas (at most 3): split the sentence"), "{note}");
        assert!(note.contains("sentence 2") && note.contains("colon"));
        // only the sentence limits (no punctuation rule): still checked
        let only = Rule { max_words: Some(3), ..Default::default() };
        assert_eq!(check("One two three four. Ok: fine.", &only, &[]).iter().map(|v| v.kind).collect::<Vec<_>>(), [Kind::LongSentence]);
    }

    #[test]
    fn paragraph_lengths() {
        let r = paragraph_ranges("[paragraph_words.Methods]\ntarget = [55, 120]\nbounds = [35, 170]\nsentences = [2, 8]\n[paragraph_words.Results]\ntarget = [65, 125]\n");
        let m = r["Methods"];
        assert_eq!((m.target, m.bounds, m.sentences), ((55, 120), (35, 170), Some((2, 8))));
        assert_eq!(r["Results"].bounds, (43, 187), "bounds default around the target");
        let aims: Vec<_> = (0..4).map(|k| m.aim(k)).collect();
        assert!(aims.windows(2).all(|w| w[0] != w[1]) && aims.iter().all(|(a, b)| *a >= 55 && *b <= 120 && a < b), "{aims:?}");
        let long = format!("{}.", vec!["word"; 200].join(" "));
        assert!(m.problem(&long).unwrap().starts_with("200 words (at most 170): split"));
        assert!(m.problem("Too short. Really.").unwrap().contains("never invent content"));
        let ok = format!("{}. Then {}.", vec!["word"; 30].join(" "), vec!["word"; 30].join(" "));
        assert_eq!(m.problem(&ok), None);
        assert!(paragraph_ranges("x = 1").is_empty());
        let g = Guard { rule: rule(), exempt: vec![] };
        assert_eq!(g.finish("A: b; c.").0, "A, b. C.");
        assert!(Guard::default().check("A: b.").is_empty());
    }

    #[test]
    fn mechanical_fixes() {
        let r = rule();
        let t = "The model has two parts: an encoder and a decoder; both are data-driven — and \"robust\". AUC was 0.70–0.75.";
        let f = fix(t, &r, &[]);
        assert_eq!(f, "The model has two parts, an encoder and a decoder. Both are data driven, and robust. AUC was 0.70 to 0.75.");
        assert!(check(&f, &r, &[]).is_empty());
        assert_eq!(fix("Gains were large — about 0.2 — overall.", &r, &[]), "Gains were large, about 0.2, overall.");
        assert_eq!(fix("See $a:b$ and \\cite{x-y}; done.", &r, &[]), "See $a:b$ and \\cite{x-y}. Done.");
        assert_eq!(fix("Use X-ray and self-supervised maps.", &r, &["self-supervised".into()]), "Use X-ray and self-supervised maps.");
    }
}
