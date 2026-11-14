//! 我的風格 (writing stage 3): the author's own style per section, derived **only from his own
//! material** — the phrase bank rows from his published papers (`profile/phrases.csv`,
//! `expressions.csv`, `exemplars.csv`). Guide material never counts: a row tagged or sourced
//! `guide`, or one carrying a sentence of `style/patterns.toml` ([`crate::patterns`]), is left
//! out. Papers in the workspace are not read: nothing marks a paper as the author's own, and
//! generated drafts must never feed his style back.
//!
//! Per section: typical opening and closing templates, closing words (how his last sentences
//! start), connectives, the order of functions inside a paragraph (the phrase bank's
//! `position`: opening → body → closing), the share of a paragraph that states the main claim
//! and where it sits (a fixed marker list over his exemplars — deterministic, no model), and
//! typical lengths. Purely recomputable, so it lives in the cache tier (the app passes the
//! file); the file is rewritten only when the profile changes, and its key (the source files'
//! bytes + [`VERSION`]) skips the rebuild when nothing changed.

use crate::patterns::{is_guide_text, Pattern};
use crate::profile::{Expression, Phrase};
use crate::text::{sentences, strip_latex, word_count};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Bump when what is derived changes (rebuilds the cache).
pub const VERSION: u32 = 1;
const TEMPLATES: usize = 5;
const CONNECTIVES: usize = 8;
const CLOSING_WORDS: usize = 6;
/// Exemplars a prompt gets at most, and characters of each.
pub const EXEMPLARS: usize = 3;
pub const EXEMPLAR_CHARS: usize = 900;

/// A row of `exemplars.csv`: one of the author's own paragraphs.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Exemplar {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub function: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub source: String,
}

/// `exemplars.csv` of a profile folder (missing or broken rows: skipped).
pub fn read_exemplars(profile_dir: &Path) -> Vec<Exemplar> {
    let Ok(mut r) = csv::Reader::from_path(profile_dir.join("exemplars.csv")) else { return vec![] };
    r.deserialize().filter_map(Result::ok).filter(|e: &Exemplar| !e.text.trim().is_empty()).collect()
}

/// A tag or source that marks guide material (`guide`, `guide:…`, `writing guide`).
pub fn is_guide_mark(s: &str) -> bool {
    s.to_lowercase().split([';', ',', ' ', ':']).any(|w| w.trim() == "guide")
}

/// The author's own material, guide rows removed.
pub struct Sources<'a> {
    pub phrases: Vec<&'a Phrase>,
    pub expressions: Vec<&'a Expression>,
    pub exemplars: Vec<&'a Exemplar>,
}

pub fn own<'a>(phrases: &'a [Phrase], expressions: &'a [Expression], exemplars: &'a [Exemplar], guide: &[Pattern]) -> Sources<'a> {
    let guided = |texts: &[&str], marks: &[&str]| marks.iter().any(|m| is_guide_mark(m)) || texts.iter().any(|t| is_guide_text(guide, t));
    Sources {
        phrases: phrases.iter().filter(|p| !guided(&[&p.template, &p.example], &p.tags.iter().map(String::as_str).collect::<Vec<_>>())).collect(),
        expressions: expressions.iter().filter(|e| !guided(&[&e.text, &e.example], &e.tags.iter().map(String::as_str).collect::<Vec<_>>())).collect(),
        exemplars: exemplars.iter().filter(|e| !guided(&[&e.text], &[&e.source])).collect(),
    }
}

/// One section's style.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SectionStyle {
    pub openings: Vec<String>,
    pub closings: Vec<String>,
    /// How his paragraphs' last sentences start (first words, most frequent first).
    pub closing_words: Vec<String>,
    pub connectives: Vec<String>,
    /// Paragraph functions in the order they come (opening → body → closing).
    pub function_order: Vec<String>,
    /// Sentences stating the claim / all sentences, median over his paragraphs.
    pub claim_share: Option<f64>,
    /// Where the claim sentence sits most often: `first`, `last`, `first and last`, `middle`.
    pub claim_position: String,
    pub sentences_per_paragraph: Option<f64>,
    pub words_per_sentence: Option<f64>,
    pub words_per_paragraph: Option<f64>,
    pub paragraphs: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StyleProfile {
    pub version: u32,
    pub key: String,
    pub sections: BTreeMap<String, SectionStyle>,
}

/// Words that mark a sentence stating the paragraph's main claim (fixed, deterministic).
const CLAIM_MARKERS: &[&str] = &[
    "we propose", "we present", "we introduce", "we show", "we demonstrate", "we found", "we find", "we conclude", "we argue",
    "this study", "this work", "this paper", "our results", "our findings", "these results", "these findings", "the results",
    "indicate that", "suggest that", "show that", "demonstrate that", "demonstrated that", "confirm", "therefore", "thus,",
    "in conclusion", "in summary", "overall,", "taken together", "improved", "improves", "outperform", "enhances", "enables",
];

pub fn states_claim(sentence: &str) -> bool {
    let s = format!(" {} ", strip_latex(sentence).to_lowercase());
    CLAIM_MARKERS.iter().any(|m| s.contains(m))
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let m = if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 };
    Some((m * 100.0).round() / 100.0)
}

fn position_rank(p: &str) -> f64 {
    match p.trim().to_lowercase().as_str() {
        "opening" => 0.0,
        "closing" => 2.0,
        _ => 1.0,
    }
}

fn first_words(s: &str, n: usize) -> String {
    strip_latex(s).split_whitespace().take(n).collect::<Vec<_>>().join(" ")
}

fn section_of(s: &str) -> String {
    crate::patterns::canonical_section(s)
}

/// Build the profile from the author's own rows.
pub fn build(src: &Sources) -> StyleProfile {
    let mut names: Vec<String> = src.phrases.iter().map(|p| section_of(&p.section)).chain(src.exemplars.iter().map(|e| section_of(&e.section))).filter(|s| !s.is_empty() && s != "Any").collect();
    names.sort();
    names.dedup();
    let mut sections = BTreeMap::new();
    for name in names {
        let mut st = SectionStyle::default();
        let phr: Vec<&&Phrase> = src.phrases.iter().filter(|p| section_of(&p.section) == name).collect();
        let pick = |pos: &str| {
            let mut v: Vec<&&Phrase> = phr.iter().copied().filter(|p| p.position.trim().eq_ignore_ascii_case(pos)).collect();
            v.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
            v.into_iter().take(TEMPLATES).map(|p| p.template.trim().to_string()).collect::<Vec<_>>()
        };
        st.openings = pick("opening");
        st.closings = pick("closing");
        // functions by their mean position in a paragraph
        let mut fpos: BTreeMap<String, (f64, usize)> = BTreeMap::new();
        for p in &phr {
            let f = p.function.trim().to_lowercase();
            if f.is_empty() || f == "transition" {
                continue;
            }
            let e = fpos.entry(f).or_default();
            e.0 += position_rank(&p.position);
            e.1 += 1;
        }
        let mut order: Vec<(String, f64, usize)> = fpos.into_iter().map(|(f, (s, n))| (f, s / n as f64, n)).collect();
        order.sort_by(|a, b| a.1.total_cmp(&b.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        st.function_order = order.into_iter().map(|(f, _, _)| f).collect();
        let mut conn: Vec<&&Expression> = src.expressions.iter().filter(|e| e.kind.trim().eq_ignore_ascii_case("connective") && (section_of(&e.section) == name || e.section.trim().is_empty() || e.section.trim() == "Any")).collect();
        conn.sort_by_key(|e| (section_of(&e.section) != name, e.id.clone()));
        let mut seen = vec![];
        for e in conn {
            let t = e.text.trim().to_string();
            if !seen.contains(&t) {
                seen.push(t);
            }
        }
        st.connectives = seen.into_iter().take(CONNECTIVES).collect();
        // his paragraphs: lengths, the claim, closing words
        let ex: Vec<&&Exemplar> = src.exemplars.iter().filter(|e| section_of(&e.section) == name).collect();
        st.paragraphs = ex.len();
        let (mut shares, mut sents, mut wps, mut wpp) = (vec![], vec![], vec![], vec![]);
        let mut at: BTreeMap<&str, usize> = BTreeMap::new();
        let mut closing: BTreeMap<String, usize> = BTreeMap::new();
        for e in &ex {
            let ss = sentences(&e.text);
            if ss.is_empty() {
                continue;
            }
            let words: Vec<usize> = ss.iter().map(|s| word_count(s)).collect();
            let claims: Vec<bool> = ss.iter().map(|s| states_claim(s)).collect();
            shares.push(claims.iter().filter(|c| **c).count() as f64 / ss.len() as f64);
            sents.push(ss.len() as f64);
            wpp.push(words.iter().sum::<usize>() as f64);
            wps.extend(words.iter().map(|w| *w as f64));
            let (first, last) = (claims[0], ss.len() > 1 && claims[ss.len() - 1]);
            let pos = match (first, last) {
                (true, true) => "first and last",
                (true, false) => "first",
                (false, true) => "last",
                _ if claims.iter().any(|c| *c) => "middle",
                _ => "",
            };
            if !pos.is_empty() {
                *at.entry(pos).or_default() += 1;
            }
            if ss.len() > 1 {
                let w = first_words(&ss[ss.len() - 1], 3);
                if !w.is_empty() {
                    *closing.entry(w).or_default() += 1;
                }
            }
        }
        st.claim_share = median(shares);
        st.sentences_per_paragraph = median(sents);
        st.words_per_sentence = median(wps);
        st.words_per_paragraph = median(wpp);
        st.claim_position = at.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(p, _)| p.to_string()).unwrap_or_default();
        let mut cw: Vec<(String, usize)> = closing.into_iter().collect();
        cw.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        st.closing_words = cw.into_iter().take(CLOSING_WORDS).map(|(w, _)| w).collect();
        sections.insert(name, st);
    }
    StyleProfile { version: VERSION, key: String::new(), sections }
}

/// FNV-1a over the bytes (stable across builds, unlike std's hasher).
fn fnv(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.iter().chain(&[0xff]) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}

/// The cache file of a workspace under `cache_root` (one per workspace path).
pub fn cache_file(cache_root: &Path, workspace: &Path) -> std::path::PathBuf {
    cache_root.join("style-profile").join(format!("{}.json", fnv(&[workspace.to_string_lossy().as_bytes()])))
}

/// The cache key: the source files' bytes (phrases, expressions, exemplars, patterns) and
/// [`VERSION`].
pub fn key(profile_dir: &Path, workspace: &Path) -> String {
    let read = |p: &Path| std::fs::read(p).unwrap_or_default();
    let files = [
        read(&profile_dir.join("phrases.csv")),
        read(&profile_dir.join("expressions.csv")),
        read(&profile_dir.join("exemplars.csv")),
        read(&workspace.join(crate::draft::STYLE_DIR).join(crate::patterns::FILE)),
    ];
    let v = VERSION.to_le_bytes();
    let mut parts: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    parts.push(&v);
    fnv(&parts)
}

/// The profile for a workspace: from `cache` when its key matches, else rebuilt from the
/// author's own rows; the file is written only when its content changes.
pub fn load_or_build(phrases: &[Phrase], expressions: &[Expression], profile_dir: &Path, workspace: &Path, cache: &Path) -> StyleProfile {
    let k = key(profile_dir, workspace);
    let old: Option<StyleProfile> = std::fs::read_to_string(cache).ok().and_then(|t| serde_json::from_str(&t).ok());
    if let Some(p) = &old {
        if p.key == k && p.version == VERSION {
            return p.clone();
        }
    }
    let exemplars = read_exemplars(profile_dir);
    let (guide, _) = crate::patterns::load(workspace);
    let mut p = build(&own(phrases, expressions, &exemplars, &guide));
    p.key = k;
    if old.as_ref() != Some(&p) {
        if let Some(dir) = cache.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(&p) {
            let tmp = cache.with_extension("json.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, cache);
            }
        }
    }
    p
}

impl StyleProfile {
    /// The section's style as a prompt block (empty when the section is unknown).
    pub fn prompt_text(&self, section: &str) -> String {
        let name = section_of(section);
        let Some(s) = self.sections.get(&name) else { return String::new() };
        let mut out = format!("The author's own style in {name} paragraphs (derived from his published papers):\n");
        let list = |v: &[String]| v.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join("; ");
        if !s.openings.is_empty() {
            out += &format!("- typical openings: {}\n", list(&s.openings));
        }
        if !s.closings.is_empty() {
            out += &format!("- typical closings: {}\n", list(&s.closings));
        }
        if !s.closing_words.is_empty() {
            out += &format!("- his last sentences start with: {}\n", list(&s.closing_words));
        }
        if !s.connectives.is_empty() {
            out += &format!("- connectives he uses: {}\n", s.connectives.join(", "));
        }
        if !s.function_order.is_empty() {
            out += &format!("- order of the argument inside a paragraph: {}\n", s.function_order.join(" → "));
        }
        if let Some(c) = s.claim_share {
            out += &format!("- the main claim takes about {:.0}% of a paragraph's sentences", c * 100.0);
            if !s.claim_position.is_empty() {
                out += &format!(", most often in the {} sentence", s.claim_position);
            }
            out += "\n";
        }
        if let (Some(n), Some(w)) = (s.sentences_per_paragraph, s.words_per_sentence) {
            out += &format!("- typical length: {n:.0} sentences, {w:.0} words per sentence");
            if let Some(p) = s.words_per_paragraph {
                out += &format!(", {p:.0} words per paragraph");
            }
            out += "\n";
        }
        out
    }
}

/// Up to [`EXEMPLARS`] of the author's own paragraphs of `section` for a passage: the same
/// function first when given, then the most similar wording; each cut to [`EXEMPLAR_CHARS`].
pub fn exemplars_for<'a>(own: &[&'a Exemplar], section: &str, function: &str, text: &str) -> Vec<&'a Exemplar> {
    let name = section_of(section);
    let words = |s: &str| strip_latex(s).to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 3).map(str::to_string).collect::<std::collections::BTreeSet<_>>();
    let want = words(text);
    let f = function.trim().to_lowercase();
    let mut v: Vec<(bool, usize, &'a Exemplar)> = own
        .iter()
        .filter(|e| section_of(&e.section) == name)
        .map(|e| (!f.is_empty() && e.function.trim().to_lowercase() == f, words(&e.text).intersection(&want).count(), *e))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.id.cmp(&b.2.id)));
    v.into_iter().take(EXEMPLARS).map(|(_, _, e)| e).collect()
}

pub fn cut(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= EXEMPLAR_CHARS { s.to_string() } else { s.chars().take(EXEMPLAR_CHARS).collect::<String>() + "…" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ph(id: &str, section: &str, function: &str, position: &str, t: &str, tags: &[&str]) -> Phrase {
        Phrase { id: id.into(), section: section.into(), function: function.into(), position: position.into(), template: t.into(), example: String::new(), tags: tags.iter().map(|s| s.to_string()).collect(), priority: 3 }
    }

    fn ex(id: &str, section: &str, function: &str, text: &str, source: &str) -> Exemplar {
        Exemplar { id: id.into(), section: section.into(), function: function.into(), text: text.into(), source: source.into() }
    }

    fn guide() -> Vec<Pattern> {
        crate::patterns::parse("[[pattern]]\nid = \"g\"\nsituation = \"compare_two\"\nstructure = \"Although {A} and {B} differ in {x}, both {y}.\"\nexample = \"Although trains and buses differ in speed, both reduce traffic in city centres.\"\nsource = \"guide\"\n").0
    }

    fn rows() -> (Vec<Phrase>, Vec<Expression>, Vec<Exemplar>) {
        let phrases = vec![
            ph("P1", "Introduction", "background", "opening", "{condition} is a major problem in {population}.", &["paper1"]),
            ph("P2", "Introduction", "gap", "body", "However, previous models {limitation}.", &["paper1"]),
            ph("P3", "Introduction", "contribution", "closing", "This study proposes {method} to {aim}.", &["paper1"]),
            // a guide sentence tagged as such, and one that slipped in untagged
            ph("G1", "Introduction", "thesis", "opening", "{topic} is superior because {reason}.", &["guide"]),
            ph("G2", "Introduction", "comparison", "opening", "Although trains and buses differ in speed, both reduce traffic in city centres.", &[]),
        ];
        let expressions = vec![
            Expression { id: "X1".into(), kind: "connective".into(), text: "Additionally,".into(), usage: String::new(), section: "Introduction".into(), example: String::new(), tags: vec!["paper1".into()] },
            Expression { id: "X2".into(), kind: "connective".into(), text: "Nevertheless,".into(), usage: String::new(), section: "Introduction".into(), example: String::new(), tags: vec!["guide".into()] },
        ];
        let exemplars = vec![
            ex("E1", "Introduction", "gap", "Fractures are common in older adults. However, earlier models look at the wrong regions. This study proposes a loss that keeps attention on the lesion.", "Author, Journal (2024)"),
            ex("E2", "Introduction", "background", "Screening needs many readers. Readers disagree on borderline cases. These results indicate that consensus labels are needed.", "Author, Journal (2026)"),
            ex("E9", "Introduction", "thesis", "Although trains and buses differ in speed, both reduce traffic in city centres. Cities should fund both.", "writing guide"),
        ];
        (phrases, expressions, exemplars)
    }

    #[test]
    fn only_the_authors_rows_count() {
        let (p, x, e) = rows();
        let g = guide();
        let src = own(&p, &x, &e, &g);
        assert_eq!(src.phrases.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["P1", "P2", "P3"]);
        assert_eq!(src.expressions.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["X1"]);
        assert_eq!(src.exemplars.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["E1", "E2"]);
        let prof = build(&src);
        let s = &prof.sections["Introduction"];
        assert_eq!(s.openings, ["{condition} is a major problem in {population}."]);
        assert_eq!(s.closings, ["This study proposes {method} to {aim}."]);
        assert_eq!(s.function_order, ["background", "gap", "contribution"]);
        assert_eq!(s.connectives, ["Additionally,"]);
        assert_eq!(s.paragraphs, 2);
        assert_eq!(s.claim_share, Some(0.33));
        assert_eq!(s.claim_position, "last");
        assert_eq!(s.sentences_per_paragraph, Some(3.0));
        assert!(s.closing_words.contains(&"This study proposes".to_string()));
        let text = prof.prompt_text("1. Introduction");
        for want in ["typical openings", "background → gap → contribution", "about 33%", "last sentence", "Additionally,"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        for never in ["trains and buses", "superior because", "Nevertheless", "Cities should"] {
            assert!(!text.contains(never), "guide material in the profile: {never}");
        }
        assert_eq!(prof.prompt_text("Appendix"), "");
        let picked = exemplars_for(&src.exemplars, "Introduction", "gap", "earlier models attention");
        assert_eq!(picked[0].id, "E1");
        assert!(picked.iter().all(|e| e.id != "E9"));
    }

    #[test]
    fn the_cache_is_rewritten_only_when_the_sources_change() {
        let root = std::env::temp_dir().join(format!("pc-styleprofile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (ws, prof_dir, cache) = (root.join("ws"), root.join("ws/profile"), root.join("cache/style-profile.json"));
        std::fs::create_dir_all(&prof_dir).unwrap();
        std::fs::write(prof_dir.join("exemplars.csv"), "id,section,function,text,source\nE1,Result,finding,\"Accuracy rose. These results indicate that the loss helps.\",\"Author (2024)\"\nE2,Result,finding,\"Although trains and buses differ in speed, both reduce traffic in city centres.\",guide\n").unwrap();
        let (p, x, _) = rows();
        let a = load_or_build(&p, &x, &prof_dir, &ws, &cache);
        assert_eq!(a.sections["Result"].paragraphs, 1, "the guide row is not his");
        let t1 = std::fs::metadata(&cache).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = load_or_build(&p, &x, &prof_dir, &ws, &cache);
        assert_eq!(a, b);
        assert_eq!(std::fs::metadata(&cache).unwrap().modified().unwrap(), t1, "unchanged sources: not rewritten");
        // a changed source rebuilds it
        std::fs::write(prof_dir.join("exemplars.csv"), "id,section,function,text,source\nE1,Result,finding,\"Accuracy rose. Recall rose. These results indicate that the loss helps.\",\"Author (2024)\"\n").unwrap();
        let c = load_or_build(&p, &x, &prof_dir, &ws, &cache);
        assert_ne!(c.key, a.key);
        assert_eq!(c.sections["Result"].sentences_per_paragraph, Some(3.0));
        assert_ne!(std::fs::metadata(&cache).unwrap().modified().unwrap(), t1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
