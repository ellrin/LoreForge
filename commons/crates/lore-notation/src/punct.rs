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
    /// Preferred written forms (`softmax`, `cross entropy`, `X-ray`): a run of words that spells
    /// the same letters otherwise ("soft-max", "Cross-Entropy") is a [`Kind::Spelling`] violation.
    pub spellings: Vec<String>,
    /// Words per sentence at most.
    pub max_words: Option<usize>,
    /// Commas per sentence at most (numbers, maths, citations and statistics in parentheses
    /// do not count).
    pub max_commas: Option<usize>,
}

impl Rule {
    /// The punctuation marks are checked (`allowed` given).
    pub fn marks_on(&self) -> bool {
        !self.allowed.is_empty()
    }

    /// Hyphen compounds and spellings are checked: always with the marks, alone when a
    /// whitelist or preferred spellings are given.
    pub fn wording_on(&self) -> bool {
        self.marks_on() || !self.spellings.is_empty() || !self.hyphen_whitelist.is_empty()
    }

    /// Anything is checked at all.
    pub fn enabled(&self) -> bool {
        self.wording_on()
    }

    /// Only the wording: hyphen compounds and spellings. No punctuation marks, no sentence
    /// length or comma limits (slide text is not sentence prose).
    pub fn wording_only(&self) -> Rule {
        Rule { allowed: vec![], max_words: None, max_commas: None, ..self.clone() }
    }

    /// `self` is the shared rule, `ws` a workspace's: whitelists and spellings are the union,
    /// `allowed` and the limits come from the workspace when it has them.
    pub fn merged(&self, ws: &Rule) -> Rule {
        let union = |a: &[String], b: &[String]| {
            let mut v = a.to_vec();
            for x in b {
                if !v.iter().any(|y| y.eq_ignore_ascii_case(x)) {
                    v.push(x.clone());
                }
            }
            v
        };
        Rule {
            allowed: if ws.allowed.is_empty() { self.allowed.clone() } else { ws.allowed.clone() },
            hyphen_whitelist: union(&self.hyphen_whitelist, &ws.hyphen_whitelist),
            spellings: union(&self.spellings, &ws.spellings),
            max_words: ws.max_words.or(self.max_words),
            max_commas: ws.max_commas.or(self.max_commas),
        }
    }

    /// `[punctuation]` of a TOML file: `allowed = [".", ","]`, `hyphen_whitelist = […]`.
    /// `spellings = […]` too. None when the table is missing or has none of the three (the guard
    /// stays off); without `allowed` the marks stay off but the wording checks run.
    pub fn from_toml(raw: &str) -> Option<Rule> {
        let t: toml::Table = toml::from_str(raw).ok()?;
        let p = t.get("punctuation")?.as_table()?;
        let strings = |k: &str| -> Vec<String> { p.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let allowed: Vec<char> = strings("allowed").iter().filter_map(|s| s.trim().chars().next()).collect();
        let r = Rule { allowed, hyphen_whitelist: strings("hyphen_whitelist"), spellings: strings("spellings"), max_words: None, max_commas: None };
        r.enabled().then_some(r)
    }

    /// The prompt line (short: the code check is the guarantee).
    pub fn prompt_line(&self) -> String {
        if !self.enabled() {
            return String::new();
        }
        let mut wording = " Use few compound nouns, prefer plain words, never coin a compound.".to_string();
        if !self.spellings.is_empty() {
            let list: Vec<String> = self.spellings.iter().take(20).map(|x| format!("\"{x}\"")).collect();
            wording += &format!(" Spellings to use: {}.", list.join(", "));
        }
        if !self.marks_on() {
            return format!("{}\n", wording.trim());
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
        s + &wording + "\n"
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
    /// A hyphenated or spaced spelling of a preferred form (`soft-max` for `softmax`).
    Spelling,
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
    /// For [`Kind::Spelling`]: the written form to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<String>,
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
            Kind::Spelling => format!("\"{}\": write \"{}\"", self.found, self.preferred.as_deref().unwrap_or("")),
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
    // a name, not a compound: a digit or a capital inside a part (ResNet-50, U-Net, BERT-base,
    // t-SNE). Opening it ("ResNet 50") would change the name.
    if token.split('-').any(|p| p.chars().any(|c| c.is_ascii_digit()) || p.chars().skip(1).any(|c| c.is_ascii_uppercase()) || (p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase()))) {
        return true;
    }
    let t = token.to_lowercase();
    rule.hyphen_whitelist.iter().chain(rule.spellings.iter().filter(|x| x.contains('-'))).chain(exempt.iter()).any(|w| {
        let w = w.to_lowercase();
        w == t || w.split(|c: char| !(c.is_alphanumeric() || c == '-')).any(|x| x == t)
    })
}

/// One finding before it is tied to a sentence: byte range, kind, what.
fn marks_in(text: &str, m: &[u8], rule: &Rule, exempt: &[String]) -> Vec<(usize, usize, Kind, String, Option<String>)> {
    let sp = spellings_in(text, m, rule);
    let mut out: Vec<_> = marks_only(text, m, rule, exempt).into_iter().filter(|(s, e, ..)| !sp.iter().any(|(a, b, ..)| s < b && a < e)).map(|(s, e, k, f)| (s, e, k, f, None)).collect();
    out.extend(sp.into_iter().map(|(s, e, found, pref)| (s, e, Kind::Spelling, found, Some(pref))));
    out.sort_by_key(|x| x.0);
    out
}

/// A run of words, hyphens and single spaces that spells a preferred form's letters and
/// digits (ignoring case) but is written otherwise: byte range, what was found, what to write.
fn spellings_in(text: &str, m: &[u8], rule: &Rule) -> Vec<(usize, usize, String, String)> {
    let key = |s: &str| -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() };
    let forms: Vec<(String, &String)> = rule.spellings.iter().filter(|f| !key(f).is_empty()).map(|f| (key(f), f)).collect();
    if forms.is_empty() {
        return vec![];
    }
    // the words of the masked text: byte ranges of ASCII letter / digit runs
    let mut ws: Vec<(usize, usize)> = vec![];
    let mut i = 0;
    while i < m.len() {
        if is_word_char(m[i]) {
            let e = m[i..].iter().position(|x| !is_word_char(*x)).map(|k| i + k).unwrap_or(m.len());
            ws.push((i, e));
            i = e;
        } else {
            i += 1;
        }
    }
    let mut out = vec![];
    let mut a = 0;
    while a < ws.len() {
        let mut best: Option<(usize, &String)> = None;
        let mut acc = String::new();
        for b in a..ws.len().min(a + 6) {
            if b > a {
                let gap = &m[ws[b - 1].1..ws[b].0];
                if gap != b"-" && gap != b" " {
                    break;
                }
            }
            acc += &text[ws[b].0..ws[b].1].to_lowercase();
            if let Some((_, f)) = forms.iter().find(|(k, _)| *k == acc) {
                best = Some((b, f));
            }
        }
        let Some((b, form)) = best else {
            a += 1;
            continue;
        };
        let (s, e) = (ws[a].0, ws[b].1);
        // never inside a masked span; never part of a longer hyphenated word
        let joined = |at: Option<usize>, dir: i32| at.is_some_and(|k| m[k] == b'-' && if dir < 0 { k > 0 && is_word_char(m[k - 1]) } else { m.get(k + 1).is_some_and(|x| is_word_char(*x)) });
        let clean = m[s..e] == text.as_bytes()[s..e] && !joined(s.checked_sub(1), -1) && !joined((e < m.len()).then_some(e), 1);
        let found = &text[s..e];
        let upper = found.chars().next().is_some_and(char::is_uppercase);
        let mut want = (*form).clone();
        if upper {
            let mut c = want.chars();
            want = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
        }
        // an all capitals word is an acronym, left alone
        let acronym = !found.chars().any(char::is_lowercase);
        if clean && !acronym && found != want {
            out.push((s, e, found.to_string(), want));
        }
        a = b + 1;
    }
    out
}

fn marks_only(text: &str, m: &[u8], rule: &Rule, exempt: &[String]) -> Vec<(usize, usize, Kind, String)> {
    let mut out = vec![];
    let marks = rule.marks_on();
    let allowed = |c: char| rule.allowed.contains(&c);
    let mut i = 0;
    while i < m.len() {
        let c = m[i];
        // multi-byte marks (— – “ ” ‘ ’)
        if c >= 0x80 {
            let ch = text[i..].chars().next().unwrap_or(' ');
            let len = ch.len_utf8();
            if marks && MARKS.contains(&ch) && !allowed(ch) && m[i] != b'X' {
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
            b':' | b';' | b'!' | b'?' | b'"' if marks && !allowed(c as char) => {
                let kind = match c {
                    b':' => Kind::Colon,
                    b';' => Kind::Semicolon,
                    b'!' => Kind::Exclamation,
                    b'?' => Kind::Question,
                    _ => Kind::Quote,
                };
                out.push((i, i + 1, kind, (c as char).to_string()));
            }
            b'`' | b'\'' if marks && !allowed('"') => {
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
                    if !marks {
                        i += run;
                        continue;
                    }
                    let kind = if run == 2 && digit_before && digit_after { Kind::Range } else { Kind::Dash };
                    if !allowed('—') {
                        let found = if kind == Kind::Range { range_text(text, m, i, i + run) } else { "-".repeat(run) };
                        out.push((i, i + run, kind, found));
                    }
                } else if marks && spaced && !allowed('—') {
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
                    if numeric && !marks {
                        // a number range is not wording
                    } else if numeric && digit_before && digit_after {
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
            b'(' if marks => {
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
        for (s, _, kind, found, preferred) in marks_in(text, &m, rule, exempt) {
            let k = sent_of(s);
            let (a, b) = sents.get(k).copied().unwrap_or((0, text.len()));
            out.push(Violation { kind, found, sentence: k + 1, text: plain_cut(text, a, b), preferred });
        }
    }
    for (k, (a, b)) in sents.iter().enumerate() {
        let w = words(&m[*a..*b]);
        if rule.max_words.is_some_and(|x| w > x) {
            out.push(Violation { kind: Kind::LongSentence, found: format!("{w} words (at most {})", rule.max_words.unwrap_or(0)), sentence: k + 1, text: plain_cut(text, *a, *b), preferred: None });
        }
        let c = commas(&m[*a..*b]);
        if rule.max_commas.is_some_and(|x| c > x) {
            out.push(Violation { kind: Kind::Commas, found: format!("{c} commas (at most {})", rule.max_commas.unwrap_or(0)), sentence: k + 1, text: plain_cut(text, *a, *b), preferred: None });
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
    for (s, e, kind, found, preferred) in marks_in(text, &m, rule, exempt) {
        let rep = match kind {
            Kind::Spelling => preferred.unwrap_or(found),
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
    if !rule.marks_on() {
        return out;
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
        Rule { allowed: vec!['.', ','], hyphen_whitelist: vec!["X-ray".into(), "COVID-19".into(), "Grad-CAM".into()], max_words: Some(35), max_commas: Some(3), ..Default::default() }
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
    fn names_with_digits_or_inner_capitals_are_not_compounds() {
        let r = Rule { hyphen_whitelist: vec![], ..rule() };
        for t in ["We train ResNet-50 and U-Net.", "BERT-base and t-SNE are used.", "GPT-4 reads it."] {
            assert!(kinds(t, &[]).is_empty(), "{t}");
            assert_eq!(fix(t, &r, &[]), t);
        }
        assert_eq!(kinds("A data-driven model. Data-driven models work.", &[]), [Kind::Hyphen, Kind::Hyphen]);
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

    fn spell_rule() -> Rule {
        Rule { spellings: vec!["softmax".into(), "cross entropy".into(), "X-ray".into()], ..Default::default() }
    }

    #[test]
    fn spellings_are_found_and_fixed() {
        let r = spell_rule();
        assert!(r.enabled() && !r.marks_on());
        let t = "The soft-max and soft max and Soft-Max layers use cross-entropy, Cross-Entropy and crossentropy.";
        let v = check(t, &r, &[]);
        assert_eq!(v.len(), 6, "{v:?}");
        assert!(v.iter().all(|x| x.kind == Kind::Spelling));
        assert!(v[0].describe().contains("\"soft-max\": write \"softmax\""), "{}", v[0].describe());
        let f = fix(t, &r, &[]);
        assert_eq!(f, "The softmax and softmax and Softmax layers use cross entropy, Cross entropy and cross entropy.");
        assert!(check(&f, &r, &[]).is_empty());
        // capital at a sentence start is kept; the right form and longer words are fine
        assert_eq!(fix("Soft max is used. Softmaxes differ. softmax is fine.", &r, &[]), "Softmax is used. Softmaxes differ. softmax is fine.");
        // a hyphenated preferred form is allowed, and a spaced one is corrected
        assert!(check("A chest X-ray.", &r, &[]).is_empty());
        assert_eq!(fix("A chest x ray and an x-ray.", &r, &[]), "A chest X-ray and an X-ray.");
    }

    #[test]
    fn spellings_skip_masked_spans_and_neighbours() {
        let r = spell_rule();
        let t = "See $soft-max$ and \\cite{soft-max} and \\ref{cross-entropy}, \\label{soft max}.\n\\begin{equation}soft-max\\end{equation}";
        assert!(check(t, &r, &[]).is_empty(), "{:?}", check(t, &r, &[]));
        assert_eq!(fix(t, &r, &[]), t);
        // part of a longer hyphenated word: the hyphen rule's business
        assert_eq!(check("A cross-entropy-based loss.", &r, &[]).iter().map(|v| v.kind).collect::<Vec<_>>(), [Kind::Hyphen]);
        assert_eq!(fix("A cross-entropy-based loss.", &r, &[]), "A cross entropy based loss.");
    }

    #[test]
    fn spelling_beats_the_hyphen_fix() {
        let r = Rule { allowed: vec!['.', ','], spellings: vec!["softmax".into()], ..Default::default() };
        let t = "A soft-max head, data-driven: fast.";
        assert_eq!(fix(t, &r, &[]), "A softmax head, data driven, fast.");
        let v = check("A soft-max head.", &r, &[]);
        assert_eq!(v.iter().map(|x| x.kind).collect::<Vec<_>>(), [Kind::Spelling], "no second Hyphen finding");
    }

    #[test]
    fn wording_only_ignores_marks_and_length() {
        let full = Rule { allowed: vec!['.', ','], spellings: vec!["softmax".into()], hyphen_whitelist: vec!["X-ray".into()], max_words: Some(5), max_commas: Some(1), ..Default::default() };
        let w = full.wording_only();
        assert!(w.enabled() && !w.marks_on() && w.max_words.is_none());
        let t = "Note: the soft-max is data-driven — see 0.70–0.75 and 2010-2022, one two three four five six seven eight nine.";
        let kinds: Vec<Kind> = check(t, &w, &[]).iter().map(|v| v.kind).collect();
        assert_eq!(kinds, [Kind::Spelling, Kind::Hyphen]);
        assert_eq!(fix(t, &w, &[]), "Note: the softmax is data driven — see 0.70–0.75 and 2010-2022, one two three four five six seven eight nine.");
        assert!(check(t, &full, &[]).iter().any(|v| v.kind == Kind::Colon));
        // nothing given: off
        assert!(!Rule::default().wording_only().enabled());
    }

    #[test]
    fn old_toml_without_spellings() {
        let r = Rule::from_toml("[punctuation]\nallowed = [\".\", \",\"]\n").unwrap();
        assert!(r.spellings.is_empty() && r.marks_on());
        assert_eq!(fix("A soft-max.", &r, &[]), "A soft max.", "behaviour unchanged");
        assert!(r.prompt_line().contains("only \".\" and \",\"") && !r.prompt_line().contains("Spellings to use"));
        assert!(r.prompt_line().contains("few compound nouns"));
        // a shared file with only spellings and a whitelist: wording rule, no marks
        let s = Rule::from_toml("[punctuation]\nspellings = [\"softmax\"]\nhyphen_whitelist = [\"X-ray\"]\n").unwrap();
        assert!(!s.marks_on() && s.wording_on());
        assert!(s.prompt_line().contains("\"softmax\"") && !s.prompt_line().contains("Punctuation in prose"));
        assert!(Rule::from_toml("[punctuation]\nallowed = []\nspellings = []\n").is_none());
        // merge: union, workspace marks win
        let m = s.merged(&Rule::from_toml("[punctuation]\nallowed = [\".\"]\nspellings = [\"Softmax\", \"cross entropy\"]\n").unwrap());
        assert_eq!((m.allowed.clone(), m.spellings.len(), m.hyphen_whitelist.len()), (vec!['.'], 2, 1));
    }
}
