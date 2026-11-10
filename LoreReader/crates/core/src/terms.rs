//! Offline, deterministic detector of specialist-term candidates (the "red" terms).
//!
//! KnowledgeBase's `/api/scan` finds terms that already have a card; its `candidates`
//! output comes from `MockAssistant` until lore-llm is wired in, so LoreReader uses this
//! detector instead (no model calls). Candidates, in priority order:
//!
//! 1. `acronym` / `expansion`: an acronym in parentheses and the words before it whose
//!    initials spell it — "Spectral Boundary Loss (SBL)"; later uses of that acronym too.
//! 2. `acronym`: other tokens with two or more capitals (`BERT`, `ResNet`, `U-Net`).
//! 3. `phrase`: capitalised multi-word phrases ("Variational Mask Prior"); connectors
//!    (of, and, for, the, …) may sit inside; a common sentence-initial word is dropped.
//!    Title-case lines (headings) are skipped.
//! 4. `hyphenated`: hyphenated words with a part outside the common list, or long ones
//!    ("cross-entropy", "label-smoothing"), except everyday ones ("state-of-the-art").
//! 5. `rare`: words of 5+ letters that are not in the common-English list after simple
//!    inflection stripping (plural, -ed, -ing, -ly, -er, -est).
//!
//! Text after a "References" / "Bibliography" line is ignored. Offsets are UTF-16 (what
//! JavaScript uses), like KnowledgeBase's scanner.
//!
//! Known limits: no part-of-speech or context — proper names of people and places
//! (Hausdorff, Sobel) count as rare words; a capitalised word at a sentence start is
//! judged by its lowercase form; British/American spellings outside the list, numbers
//! with units and Greek letters are not handled; ligatures or broken hyphenation in the
//! PDF's text layer split words; the common list is short (≈1,300 words), so ordinary
//! but less frequent words also show up. The author's own additions go in
//! `private/LoreReader/known-words.txt`.

use std::collections::HashSet;

use serde::Serialize;

const COMMON: &str = include_str!("common_words.txt");

const CONNECTORS: &[&str] = &["of", "and", "for", "the", "in", "on", "with", "to", "a", "an", "de", "von", "van", "der"];
const EVERYDAY_HYPHENATED: &[&str] = &["state-of-the-art", "so-called", "well-known", "real-world", "end-to-end", "one-to-one", "on-the-fly", "trade-off", "follow-up", "long-term", "short-term", "large-scale", "small-scale", "high-quality", "low-cost", "e-mail", "non-zero", "re-use"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub text: String,
    /// UTF-16 offsets into the scanned text.
    pub start: usize,
    pub end: usize,
    /// acronym | expansion | phrase | hyphenated | rare
    pub kind: &'static str,
}

pub struct Detector {
    common: HashSet<String>,
}

#[derive(Debug, Clone)]
struct Tok {
    s: usize,
    e: usize,
    text: String,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (b, c) = chars[i];
        if !c.is_alphanumeric() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() {
            let c = chars[j].1;
            if c.is_alphanumeric() {
                j += 1;
            } else if (c == '-' || c == '\'' || c == '’') && j + 1 < chars.len() && chars[j + 1].1.is_alphanumeric() && chars[j - 1].1.is_alphanumeric() {
                j += 2;
            } else {
                break;
            }
        }
        let e = chars.get(j).map(|x| x.0).unwrap_or(text.len());
        out.push(Tok { s: b, e, text: text[b..e].to_string() });
        i = j;
    }
    out
}

fn is_capitalised(w: &str) -> bool {
    let mut cs = w.chars();
    cs.next().is_some_and(|c| c.is_uppercase()) && cs.all(|c| c.is_lowercase() || c == '-' || c == '\'')
}

fn capitals(w: &str) -> usize {
    w.chars().filter(|c| c.is_uppercase()).count()
}

fn is_roman(w: &str) -> bool {
    w.len() <= 5 && w.chars().all(|c| "IVXLC".contains(c))
}

/// Byte offset of the first line that is just "References" / "Bibliography" (optionally numbered).
fn references_start(text: &str) -> Option<usize> {
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c.is_whitespace()).to_lowercase();
        if matches!(t.as_str(), "references" | "bibliography" | "literature cited" | "參考文獻") {
            return Some(off);
        }
        off += line.len();
    }
    None
}

impl Default for Detector {
    fn default() -> Self {
        Detector::new(&[])
    }
}

impl Detector {
    /// The built-in common list plus `extra` known words (lowercased; `#` comments ignored).
    pub fn new(extra: &[String]) -> Detector {
        let mut common: HashSet<String> = HashSet::new();
        for line in COMMON.lines().chain(extra.iter().map(String::as_str)) {
            let line = line.split('#').next().unwrap_or("");
            for w in line.split_whitespace() {
                common.insert(w.to_lowercase());
            }
        }
        Detector { common }
    }

    /// Lines of a known-words file (one or more words per line, `#` comments).
    pub fn with_file(path: &std::path::Path) -> Detector {
        let extra: Vec<String> = std::fs::read_to_string(path).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default();
        Detector::new(&extra)
    }

    /// Is `w` (any case) an everyday word, allowing for simple inflections?
    pub fn is_common(&self, w: &str) -> bool {
        let w = w.to_lowercase().replace('’', "'");
        let w = w.strip_suffix("'s").unwrap_or(&w).to_string();
        if self.common.contains(&w) {
            return true;
        }
        let mut forms: Vec<String> = Vec::new();
        let n = w.len();
        let strip = |suf: &str| w.strip_suffix(suf).filter(|r| r.len() >= 2).map(str::to_string);
        if let Some(r) = strip("ies") {
            forms.push(format!("{r}y"));
        }
        for suf in ["es", "s", "ed", "d", "ing", "ly", "er", "est", "ness", "ment", "al", "ity"] {
            if let Some(r) = strip(suf) {
                forms.push(r.clone());
                forms.push(format!("{r}e"));
                // stopped → stop, running → run
                let b = r.as_bytes();
                if b.len() >= 3 && b[b.len() - 1] == b[b.len() - 2] {
                    forms.push(r[..r.len() - 1].to_string());
                }
            }
        }
        if let Some(r) = strip("ily") {
            forms.push(format!("{r}y"));
        }
        if let Some(r) = strip("ically") {
            forms.push(format!("{r}ic"));
        }
        let _ = n;
        forms.iter().any(|f| self.common.contains(f))
    }

    pub fn detect(&self, text: &str) -> Vec<Candidate> {
        let body = &text[..references_start(text).unwrap_or(text.len())];
        let toks = tokens(body);
        // UTF-16 offset of every byte boundary.
        let mut u16_at = vec![0usize; body.len() + 1];
        let mut u = 0;
        for (i, c) in body.char_indices() {
            for k in 0..c.len_utf8() {
                u16_at[i + k] = u;
            }
            u += c.len_utf16();
        }
        u16_at[body.len()] = u;

        let mut taken = vec![false; toks.len()];
        let mut found: Vec<(usize, usize, &'static str)> = Vec::new(); // token ranges [a, b)
        let sentence_start = |i: usize| -> bool {
            if i == 0 {
                return true;
            }
            let between = &body[toks[i - 1].e..toks[i].s];
            between.contains(['.', '!', '?', ':']) || between.contains("\n\n")
        };
        let same_sentence = |a: usize, b: usize| -> bool { (a + 1..=b).all(|k| !body[toks[k - 1].e..toks[k].s].contains(['.', '!', '?', ';', ':', '(', ')'])) };

        // 1. "(ACR)" and its expansion.
        let mut acronyms: HashSet<String> = HashSet::new();
        for i in 0..toks.len() {
            let t = &toks[i].text;
            let open = body[..toks[i].s].ends_with('(');
            let close = body[toks[i].e..].starts_with(')');
            let letters: Vec<char> = t.trim_end_matches('s').chars().filter(|c| c.is_alphabetic()).collect();
            if !(open && close && capitals(t) >= 2 && (2..=10).contains(&t.len()) && letters.len() >= 2) {
                continue;
            }
            acronyms.insert(t.clone());
            let want: Vec<char> = letters.iter().map(|c| c.to_ascii_lowercase()).collect();
            // Words before "(", skipping connectors, whose initials spell the acronym.
            let mut k = i;
            let mut idx = want.len();
            let mut start = None;
            while k > 0 && idx > 0 && i - k < want.len() + 4 {
                k -= 1;
                if !same_sentence(k, i - 1) {
                    break;
                }
                let w = toks[k].text.to_lowercase();
                if CONNECTORS.contains(&w.as_str()) && idx < want.len() {
                    continue;
                }
                // hyphenated words may give several letters ("U-Net" → u, n)
                let initials: Vec<char> = w.split('-').filter_map(|p| p.chars().next()).collect();
                if initials.len() <= idx && initials[..] == want[idx - initials.len()..idx] {
                    idx -= initials.len();
                    start = Some(k);
                } else if w.starts_with(want[idx - 1]) {
                    idx -= 1;
                    start = Some(k);
                } else {
                    break;
                }
            }
            if idx == 0
                && let Some(s) = start
            {
                found.push((s, i, "expansion"));
                taken[s..i].iter_mut().for_each(|x| *x = true);
            }
        }
        // 1b + 2. acronyms anywhere.
        for i in 0..toks.len() {
            let t = &toks[i].text;
            let mixed = capitals(t) >= 2 && t.len() <= 12 && t.chars().next().is_some_and(|c| c.is_uppercase()) && !is_roman(t) && t.chars().all(|c| c.is_alphanumeric() || c == '-');
            if !taken[i] && (acronyms.contains(t) || mixed) {
                found.push((i, i + 1, "acronym"));
                taken[i] = true;
            }
        }
        // 3. Capitalised multi-word phrases (not in title-case lines).
        let title_line = |i: usize| -> bool {
            let ls = body[..toks[i].s].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let le = body[toks[i].s..].find('\n').map(|p| toks[i].s + p).unwrap_or(body.len());
            let words: Vec<&Tok> = toks.iter().filter(|t| t.s >= ls && t.e <= le && t.text.chars().any(char::is_alphabetic)).collect();
            let content: Vec<&&Tok> = words.iter().filter(|t| !CONNECTORS.contains(&t.text.to_lowercase().as_str())).collect();
            content.len() >= 2 && content.iter().filter(|t| t.text.chars().next().is_some_and(char::is_uppercase)).count() * 10 >= content.len() * 7
        };
        let mut i = 0;
        while i < toks.len() {
            if taken[i] || !is_capitalised(&toks[i].text) {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let mut last_cap = i;
            while j < toks.len() && !taken[j] && same_sentence(j - 1, j) {
                let w = &toks[j].text;
                if is_capitalised(w) {
                    last_cap = j;
                } else if !CONNECTORS.contains(&w.as_str()) {
                    break;
                }
                j += 1;
            }
            let mut a = i;
            if sentence_start(a) && self.is_common(&toks[a].text) {
                a += 1;
            }
            let caps = (a..=last_cap).filter(|&k| is_capitalised(&toks[k].text)).count();
            if last_cap > a && caps >= 2 && !title_line(i) {
                found.push((a, last_cap + 1, "phrase"));
                taken[a..=last_cap].iter_mut().for_each(|x| *x = true);
            }
            i = last_cap.max(i) + 1;
        }
        // 4 + 5. Hyphenated and rare single words.
        for i in 0..toks.len() {
            if taken[i] {
                continue;
            }
            let t = &toks[i].text;
            if !t.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'' || c == '’') {
                continue;
            }
            if t.contains('-') {
                let lower = t.to_lowercase();
                if EVERYDAY_HYPHENATED.contains(&lower.as_str()) {
                    continue;
                }
                let parts: Vec<&str> = lower.split('-').collect();
                if parts.iter().any(|p| p.len() >= 3 && !self.is_common(p)) || lower.len() >= 12 {
                    found.push((i, i + 1, "hyphenated"));
                }
                continue;
            }
            if t.chars().count() >= 5 && !self.is_common(t) {
                found.push((i, i + 1, "rare"));
            }
        }
        found.sort_by_key(|f| f.0);
        found
            .into_iter()
            .map(|(a, b, kind)| {
                let (s, e) = (toks[a].s, toks[b - 1].e);
                Candidate { text: body[s..e].to_string(), start: u16_at[s], end: u16_at[e], kind }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(c: &[Candidate], kind: &str) -> Vec<String> {
        c.iter().filter(|x| x.kind == kind).map(|x| x.text.clone()).collect()
    }

    #[test]
    fn finds_the_four_kinds() {
        let d = Detector::default();
        let t = "We introduce a Spectral Boundary Loss (SBL) for thin structures. The SBL term helps.\n\
                 Our Variational Mask Prior uses a cross-entropy baseline with label-smoothing and anisotropic diffusion.\n\
                 The state-of-the-art results are well-known. Results improved on BERT and ResNet.";
        let c = d.detect(t);
        assert_eq!(texts(&c, "expansion"), ["Spectral Boundary Loss"]);
        assert_eq!(texts(&c, "acronym"), ["SBL", "SBL", "BERT", "ResNet"]);
        assert_eq!(texts(&c, "phrase"), ["Variational Mask Prior"]);
        assert_eq!(texts(&c, "hyphenated"), ["cross-entropy", "label-smoothing"]);
        assert_eq!(texts(&c, "rare"), ["anisotropic", "diffusion"]);
        // Offsets are UTF-16 and point at the text.
        let u: Vec<u16> = t.encode_utf16().collect();
        for x in &c {
            assert_eq!(String::from_utf16(&u[x.start..x.end]).unwrap(), x.text);
        }
    }

    #[test]
    fn inflections_headings_references_and_extra_words() {
        let d = Detector::default();
        assert!(d.is_common("Studies") && d.is_common("improved") && d.is_common("running") && d.is_common("usually"));
        assert!(!d.is_common("heteroscedastic"));
        // Title-case heading lines are not phrases; references are ignored.
        let t = "A Synthetic Study of Overlap Metrics\nThe text goes on with plain words.\nReferences\n[1] Quaternion Wavelet Transforms. Journal of Examples.";
        let c = d.detect(t);
        assert!(texts(&c, "phrase").is_empty(), "{c:?}");
        assert!(!c.iter().any(|x| x.text.contains("Quaternion")));
        // Sentence-initial "The" is dropped from a phrase; non-ASCII text keeps UTF-16 offsets.
        let c = d.detect("The Hidden Markov Model works. 中文 heteroscedastic");
        assert_eq!(texts(&c, "phrase"), ["Hidden Markov Model"]);
        let r = c.iter().find(|x| x.kind == "rare").unwrap();
        assert_eq!((r.start, r.end), (34, 49));
        let d = Detector::new(&["heteroscedastic # mine".into()]);
        assert!(d.detect("heteroscedastic noise").is_empty());
    }
}
