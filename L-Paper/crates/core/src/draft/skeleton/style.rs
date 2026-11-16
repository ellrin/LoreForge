//! 風格對照 (no model): numbers of the written text per section next to the author's, and the
//! paragraphs out of range; and the refinements (細修) as proposed changes on a paragraph's text.
//!
//! The author's side comes only from what the code already has: the paragraph ranges of
//! `style/checks.toml` (`[paragraph_words.<Section>]`: words, sentences), the style profile
//! derived from his own published paragraphs ([`crate::styleprofile`]: words per paragraph and
//! per sentence, sentences per paragraph, the share of claim sentences, his connectives), the
//! words he banned (interview notes, the profile's vocabulary), the punctuation guard. A metric
//! with no author number is shown with `mine` null, never a made-up range. A single typical value
//! (the profile's median) is shown as `[v, v]` and counts as in range within a quarter of it.

use super::*;
use crate::styleprofile::{states_claim, SectionStyle};
use crate::text::{sentences, strip_latex, word_count};
use lore_notation::punct::{Guard, ParagraphRange};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Row {
    pub id: String,
    pub label: String,
    pub mine: Option<[f64; 2]>,
    pub draft: f64,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ParaIssues {
    pub id: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SectionReport {
    pub id: String,
    pub title: String,
    pub rows: Vec<Row>,
    pub paras: Vec<ParaIssues>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StyleReport {
    pub sections: Vec<SectionReport>,
}

/// The author's side for one section.
#[derive(Default)]
pub struct Mine<'a> {
    pub range: Option<ParagraphRange>,
    pub profile: Option<&'a SectionStyle>,
}

/// Connectives counted when the profile has none of the author's (a fixed, plain list).
const CONNECTIVES: &[&str] = &[
    "however", "therefore", "thus", "moreover", "furthermore", "in contrast", "in addition", "consequently", "nevertheless", "similarly", "finally", "first", "second", "hence", "instead", "meanwhile", "overall",
];

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Sentence lengths (words) of a paragraph's prose.
fn lengths(text: &str) -> Vec<f64> {
    sentences(&strip_latex(text)).iter().map(|s| word_count(s) as f64).filter(|n| *n > 0.0).collect()
}

/// Mean change between neighbouring sentence lengths, relative to the mean length.
fn change_rate(l: &[f64]) -> Option<f64> {
    if l.len() < 2 {
        return None;
    }
    let mean = l.iter().sum::<f64>() / l.len() as f64;
    let d: f64 = l.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (l.len() - 1) as f64;
    (mean > 0.0).then(|| d / mean)
}

/// Interquartile range of sentence lengths.
fn spread(l: &[f64]) -> f64 {
    if l.len() < 2 {
        return 0.0;
    }
    let mut v = l.to_vec();
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    q(0.75) - q(0.25)
}

fn row(id: &str, label: &str, mine: Option<[f64; 2]>, draft: f64) -> Row {
    let ok = match mine {
        None => true,
        Some([a, b]) if a == b => (draft - a).abs() <= a.abs() * 0.25 + 0.5,
        Some([a, b]) => draft >= a && draft <= b,
    };
    Row { id: id.into(), label: label.into(), mine, draft: r1(draft), ok }
}

fn one(v: Option<f64>) -> Option<[f64; 2]> {
    v.map(|x| [r1(x), r1(x)])
}

/// Sentences that open with a connective (the author's list when he has one).
fn connective_openings(text: &str, list: &[String]) -> usize {
    let own: Vec<String> = if list.is_empty() { CONNECTIVES.iter().map(|c| c.to_string()).collect() } else { list.iter().map(|c| c.to_lowercase()) .collect() };
    sentences(&strip_latex(text))
        .iter()
        .filter(|s| {
            let low = s.trim().to_lowercase();
            own.iter().any(|c| low.starts_with(c.trim_end_matches(',')) && low[c.trim_end_matches(',').len()..].starts_with([',', ' ']))
        })
        .count()
}

/// The report of every section with written text. `mine` gives the author's side per section;
/// `banned` the words he never wants; `guard` the punctuation rules.
pub fn report<'a>(sk: &Skeleton, mine: &dyn Fn(&Section) -> Mine<'a>, banned: &[String], guard: &Guard) -> StyleReport {
    let mut out = StyleReport::default();
    for s in &sk.sections {
        let paras: Vec<&Para> = s.blocks.iter().flat_map(|b| b.paras.iter()).filter(|p| !p.text.trim().is_empty()).collect();
        if paras.is_empty() {
            continue;
        }
        let m = mine(s);
        let words: Vec<f64> = paras.iter().map(|p| word_count(&strip_latex(&p.text)) as f64).collect();
        let lens: Vec<Vec<f64>> = paras.iter().map(|p| lengths(&p.text)).collect();
        let all: Vec<f64> = lens.iter().flatten().copied().collect();
        let n_sent: Vec<f64> = lens.iter().map(|l| l.len() as f64).collect();
        let rates: Vec<f64> = lens.iter().filter_map(|l| change_rate(l)).collect();
        let claims = paras.iter().flat_map(|p| sentences(&strip_latex(&p.text))).filter(|x| states_claim(x)).count() as f64;
        let conn_list: Vec<String> = m.profile.map(|p| p.connectives.clone()).unwrap_or_default();
        let conn: f64 = paras.iter().map(|p| connective_openings(&p.text, &conn_list) as f64).sum::<f64>() / paras.len() as f64;
        let banned_hits: Vec<Vec<String>> = paras.iter().map(|p| banned_in(&p.text, banned)).collect();
        let punct: Vec<usize> = paras.iter().map(|p| guard.check(&p.text).len()).collect();
        let prof = m.profile;
        let range_words = m.range.map(|r| [r.target.0 as f64, r.target.1 as f64]).or_else(|| one(prof.and_then(|p| p.words_per_paragraph)));
        let range_sent = m.range.and_then(|r| r.sentences).map(|(a, b)| [a as f64, b as f64]).or_else(|| one(prof.and_then(|p| p.sentences_per_paragraph)));
        let mut rows = vec![
            row("paragraph_words", "每段字數", range_words, median(words.clone())),
            row("paragraph_sentences", "每段句數", range_sent, median(n_sent.clone())),
            row("sentence_words", "每句字數", one(prof.and_then(|p| p.words_per_sentence)), median(all.clone())),
            row("sentence_spread", "句長離散（四分位距）", None, spread(&all)),
            row("length_change", "相鄰句長變化率", None, if rates.is_empty() { 0.0 } else { rates.iter().sum::<f64>() / rates.len() as f64 }),
            row("claim_share", "結論句比例", one(prof.and_then(|p| p.claim_share)), if all.is_empty() { 0.0 } else { claims / all.len() as f64 }),
            row("connectives", "每段連接詞開頭的句數", None, conn),
            row("banned", "禁用字", Some([0.0, 0.0]), banned_hits.iter().map(|b| b.len()).sum::<usize>() as f64),
        ];
        if guard.active() {
            rows.push(row("punctuation", "標點與句長", Some([0.0, 0.0]), punct.iter().sum::<usize>() as f64));
        }
        let mut issues = vec![];
        for (k, p) in paras.iter().enumerate() {
            let mut v: Vec<String> = vec![];
            if let Some([a, b]) = range_words.filter(|[a, b]| a != b) {
                if words[k] < a || words[k] > b {
                    v.push(format!("字數 {}，範圍 {}–{}", words[k], a, b));
                }
            }
            if let Some([a, b]) = range_sent.filter(|[a, b]| a != b) {
                if n_sent[k] < a || n_sent[k] > b {
                    v.push(format!("句數 {}，範圍 {}–{}", n_sent[k], a, b));
                }
            }
            // three sentences or more all within three words of each other
            let l = &lens[k];
            if l.len() >= 3 && l.iter().cloned().fold(f64::MIN, f64::max) - l.iter().cloned().fold(f64::MAX, f64::min) <= 3.0 {
                v.push("句長變化太小".into());
            }
            if !banned_hits[k].is_empty() {
                v.push(format!("用了不要的字：{}", banned_hits[k].join("、")));
            }
            if punct[k] > 0 {
                v.push(format!("標點或句長 {} 處", punct[k]));
            }
            if !v.is_empty() {
                issues.push(ParaIssues { id: p.id.clone(), issues: v });
            }
        }
        out.sections.push(SectionReport { id: s.id.clone(), title: s.title.clone(), rows, paras: issues });
    }
    out
}

/// Banned words a text uses (whole words, any case).
fn banned_in(text: &str, banned: &[String]) -> Vec<String> {
    let plain = format!(" {} ", strip_latex(text).to_lowercase());
    let mut out: Vec<String> = vec![];
    for b in banned {
        let w = b.trim().to_lowercase();
        if w.is_empty() {
            continue;
        }
        if crate::check::contains_word(&plain, &w) && !out.contains(b) {
            out.push(b.clone());
        }
    }
    out
}

// ── 細修 ─────────────────────────────────────────────────

/// A rewrite of a paragraph as proposed changes, sentence by sentence when the sentences line up
/// (each `before` is a passage of the text as it is), else one change of the whole text.
pub fn changes_from_rewrite(text: &str, after: &str, why: &str) -> Vec<Change> {
    if text.trim() == after.trim() || after.trim().is_empty() {
        return vec![];
    }
    let a = sentences(text);
    let b = sentences(after);
    let mut out = vec![];
    if a.len() == b.len() && a.iter().all(|s| text.contains(s.as_str())) {
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                out.push(Change { id: String::new(), target: String::new(), field: "text".into(), before: x.clone(), after: y.clone(), why: why.into(), state: "proposed".into() });
            }
        }
    } else {
        out.push(Change { id: String::new(), target: String::new(), field: "text".into(), before: text.to_string(), after: after.trim().to_string(), why: why.into(), state: "proposed".into() });
    }
    out
}

/// 檢查: the program's findings as proposed changes: a word with a suggestion → that sentence with
/// it replaced; the guard's mechanical fixes → the sentences they change.
pub fn changes_from_checks(text: &str, words: &[(String, String, String)], guard_fixed: Option<&str>) -> Vec<Change> {
    let mut out: Vec<Change> = vec![];
    for s in sentences(text).iter().filter(|s| text.contains(s.as_str())) {
        let mut after = s.clone();
        let mut why: Vec<String> = vec![];
        for (found, suggest, message) in words {
            if suggest.trim().is_empty() || found.trim().is_empty() {
                continue;
            }
            // as written, or capitalised at the start of the sentence
            let cap: String = found.chars().take(1).flat_map(|c| c.to_uppercase()).chain(found.chars().skip(1)).collect();
            let hit = after.find(found.as_str()).map(|i| (i, suggest.clone())).or_else(|| {
                let sug: String = suggest.chars().take(1).flat_map(|c| c.to_uppercase()).chain(suggest.chars().skip(1)).collect();
                after.find(cap.as_str()).map(|i| (i, sug))
            });
            if let Some((i, rep)) = hit {
                after.replace_range(i..i + found.len(), &rep);
                why.push(message.clone());
            }
        }
        if after != *s {
            out.push(Change { id: String::new(), target: String::new(), field: "text".into(), before: s.clone(), after, why: why.join("；"), state: "proposed".into() });
        }
    }
    if let Some(fixed) = guard_fixed {
        // applied on top of the word changes: proposed only for sentences they do not touch
        for c in changes_from_rewrite(text, fixed, "標點與句長") {
            if !out.iter().any(|x| x.before.contains(&c.before) || c.before.contains(&x.before)) {
                out.push(c);
            }
        }
    }
    out
}

/// A `refine` thread for a paragraph (changes numbered c1, c2 …; none = done at once).
pub fn refine_thread(para: &str, mut changes: Vec<Change>, note: &str) -> Thread {
    for (k, c) in changes.iter_mut().enumerate() {
        c.id = format!("c{}", k + 1);
        c.target = para.to_string();
    }
    let status = if changes.is_empty() { "done" } else { "open" };
    Thread { id: String::new(), anchor: para.into(), kind: "refine".into(), status: status.into(), question: String::new(), options: vec![], messages: vec![], conclusion: note.into(), changes }
}
