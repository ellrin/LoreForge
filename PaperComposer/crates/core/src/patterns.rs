//! Situational sentence patterns (`style/patterns.toml`): sentence *structures* taken from
//! writing guides — "in this situation this structure fits". They are **not** the author's
//! voice: his voice is the phrase bank (`profile/phrases.csv`, `expressions.csv`,
//! `exemplars.csv`, from his own papers) and the liked rewrites of 潤飾. A pattern never goes
//! into any of those, into the derived style profile ([`crate::styleprofile`]) or into
//! 潤飾's 「我的風格」; it only reaches the prompts that draft a paragraph (寫作, 討論草稿), and
//! only the entries whose situation fits the paragraph ([`select`], deterministic, no model),
//! under a heading that says where they come from and that only the structure may be reused
//! ([`block`]).
//!
//! ```toml
//! [[pattern]]
//! id = "thesis-reason"
//! situation = "thesis_statement"
//! when = "The last sentence(s) of the introduction: the paper's main claim."
//! structure = "{topic} {claim} because {reason}."
//! example = "…"           # as the guide gives it
//! source = "guide"         # required: every entry is guide material
//! ```

use serde::Deserialize;
use std::path::Path;

pub const FILE: &str = "patterns.toml";
/// Entries one prompt gets at most, and per situation.
pub const MAX_SELECTED: usize = 6;
pub const PER_SITUATION: usize = 2;
/// Characters of one field at most (a guide entry is a line, not an essay).
const FIELD_CHARS: usize = 400;

/// The situations code knows how to pick (other situation names load but are never selected).
pub const SITUATIONS: &[&str] = &[
    "thesis_statement",
    "thesis_with_reasons",
    "purpose_to_thesis",
    "tentative_thesis_from_questions",
    "compare_two",
    "topic_sentence",
];

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Pattern {
    pub id: String,
    pub situation: String,
    #[serde(default)]
    pub when: String,
    pub structure: String,
    #[serde(default)]
    pub example: String,
    /// Always `guide`: where the entry comes from.
    pub source: String,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    pattern: Vec<Pattern>,
}

/// `style/patterns.toml` of a workspace; missing file = none. Entries without
/// `source = "guide"` are skipped (and reported): the file holds guide material only.
pub fn load(workspace: &Path) -> (Vec<Pattern>, Vec<String>) {
    match std::fs::read_to_string(workspace.join(crate::draft::STYLE_DIR).join(FILE)) {
        Ok(raw) => parse(&raw),
        Err(_) => (vec![], vec![]),
    }
}

pub fn parse(raw: &str) -> (Vec<Pattern>, Vec<String>) {
    let f: File = match toml::from_str(raw) {
        Ok(f) => f,
        Err(e) => return (vec![], vec![format!("{FILE}: {e}")]),
    };
    let mut problems = vec![];
    let out = f
        .pattern
        .into_iter()
        .filter(|p| {
            let ok = p.source.trim().eq_ignore_ascii_case("guide") && !p.structure.trim().is_empty();
            if !ok {
                problems.push(format!("{FILE}: {} skipped (needs source = \"guide\" and a structure)", p.id));
            }
            ok
        })
        .collect();
    (out, problems)
}

/// Where a paragraph sits: its section (any spelling; see [`canonical_section`]), its function
/// or paragraph type (free words, e.g. `contribution`, `Results comparison`), its index in the
/// section and how many paragraphs the section has (0 = unknown).
#[derive(Debug, Clone, Default)]
pub struct Situation<'a> {
    pub section: &'a str,
    pub function: &'a str,
    pub index: usize,
    pub count: usize,
}

/// A section title in the phrase bank's words: Abstract, Introduction, Related work, Method,
/// Result, Discussion, Conclusion; anything else as it is.
pub fn canonical_section(title: &str) -> String {
    let t = title.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| t.contains(x));
    let s = if has(&["abstract", "摘要"]) {
        "Abstract"
    } else if has(&["introduction", "緒論", "前言", "引言"]) {
        "Introduction"
    } else if has(&["related", "literature", "prior work", "文獻"]) {
        "Related work"
    } else if has(&["conclusion", "結論"]) {
        "Conclusion"
    } else if has(&["discussion", "討論"]) {
        "Discussion"
    } else if has(&["result", "experiment", "evaluation", "結果", "實驗"]) {
        "Result"
    } else if has(&["method", "material", "approach", "方法", "材料"]) {
        "Method"
    } else {
        return title.trim().to_string();
    };
    s.to_string()
}

fn thesis_function(f: &str) -> bool {
    ["contribution", "thesis", "purpose", "aim", "objective", "main claim", "this study", "key value"].iter().any(|w| f.contains(w))
}

fn compare_function(f: &str) -> bool {
    ["compar", "contrast", "versus", " vs", "differ"].iter().any(|w| f.contains(w))
}

/// The situations that fit a paragraph, in order. Deterministic:
/// - the Introduction's last paragraph, or a contribution / aim / purpose paragraph anywhere
///   but the Abstract → the thesis situations;
/// - a comparison paragraph → `compare_two`;
/// - every paragraph outside the Abstract → `topic_sentence` (its first sentence).
pub fn situations(s: &Situation) -> Vec<&'static str> {
    let section = canonical_section(s.section);
    let f = format!(" {}", s.function.to_lowercase());
    let mut out = vec![];
    if section == "Abstract" {
        return out;
    }
    let last_intro = section == "Introduction" && s.count > 0 && s.index + 1 == s.count;
    if last_intro || thesis_function(&f) {
        out.extend(["thesis_statement", "thesis_with_reasons", "purpose_to_thesis"]);
    }
    if compare_function(&f) {
        out.push("compare_two");
    }
    out.push("topic_sentence");
    out
}

/// The patterns for a paragraph: those of [`situations`], in that order, at most
/// [`PER_SITUATION`] each and [`MAX_SELECTED`] in all.
pub fn select<'a>(all: &'a [Pattern], s: &Situation) -> Vec<&'a Pattern> {
    situations(s).into_iter().flat_map(|sit| all.iter().filter(move |p| p.situation == sit).take(PER_SITUATION)).take(MAX_SELECTED).collect()
}

/// The patterns of some named situations (the key value check: the thesis ones).
pub fn of_situations<'a>(all: &'a [Pattern], which: &[&str]) -> Vec<&'a Pattern> {
    which.iter().flat_map(|sit| all.iter().filter(move |p| p.situation == *sit)).take(MAX_SELECTED).collect()
}

fn cut(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= FIELD_CHARS { s.to_string() } else { s.chars().take(FIELD_CHARS).collect::<String>() + "…" }
}

/// The heading every pattern block starts with.
pub const HEADING: &str = "Sentence structures from writing guides (NOT the author's voice: reuse only the structure where the situation fits; \
never copy the example's wording, subject or content, and never let them replace the author's own patterns)";

/// The selected patterns as a prompt block; nothing selected → empty.
pub fn block(selected: &[&Pattern]) -> String {
    if selected.is_empty() {
        return String::new();
    }
    let mut out = format!("\n{HEADING}:\n");
    for p in selected {
        out += &format!("- [{}] {}", p.situation, cut(&p.structure));
        if !p.when.trim().is_empty() {
            out += &format!(" — when: {}", cut(&p.when));
        }
        if !p.example.trim().is_empty() {
            out += &format!(" (guide's illustration, structure only: {})", cut(&p.example));
        }
        out += "\n";
    }
    out
}

/// The pattern block for a paragraph of the workspace (empty when there is no file or
/// nothing fits).
pub fn block_for(workspace: &Path, s: &Situation) -> String {
    let (all, _) = load(workspace);
    block(&select(&all, s))
}

/// Lower-case letters and digits only, single-spaced: how guide text is compared.
pub fn normalise(s: &str) -> String {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Does `text` carry a guide example (or a guide structure)? Used to keep guide sentences out
/// of the author's phrase bank and style profile.
pub fn is_guide_text(all: &[Pattern], text: &str) -> bool {
    let t = normalise(text);
    if t.split(' ').count() < 4 {
        return false;
    }
    all.iter().any(|p| {
        [&p.example, &p.structure].iter().any(|g| {
            let g = normalise(g);
            g.split(' ').count() >= 4 && (t == g || t.contains(&g) || g.contains(&t))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"
[[pattern]]
id = "t1"
situation = "thesis_statement"
when = "closes the introduction"
structure = "{topic} {claim} because {reason}."
example = "Quiet libraries help students focus because they remove interruptions."
source = "guide"

[[pattern]]
id = "c1"
situation = "compare_two"
structure = "Although {A} and {B} differ in {x}, both {shared claim}."
example = "Although trains and buses differ in speed, both reduce traffic in city centres."
source = "guide"

[[pattern]]
id = "s1"
situation = "topic_sentence"
structure = "{topic} {key words: the claim about it}."
example = "Cycling is a practical way to commute for several reasons."
source = "guide"

[[pattern]]
id = "mine"
situation = "topic_sentence"
structure = "{x} is {y}."
source = "author"
"#;

    #[test]
    fn only_guide_entries_load() {
        let (p, problems) = parse(SAMPLE);
        assert_eq!(p.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["t1", "c1", "s1"]);
        assert!(problems[0].contains("mine"));
        assert!(parse("not toml [").1[0].contains(FILE));
    }

    #[test]
    fn selection_follows_section_function_and_position() {
        let (p, _) = parse(SAMPLE);
        let ids = |s: Situation| select(&p, &s).iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        // the Introduction's last paragraph: the thesis, then the topic sentence
        assert_eq!(ids(Situation { section: "1 Introduction", function: "", index: 3, count: 4 }), ["t1", "s1"]);
        // an earlier Introduction paragraph: only the topic sentence
        assert_eq!(ids(Situation { section: "Introduction", function: "background", index: 0, count: 4 }), ["s1"]);
        // a contribution paragraph anywhere
        assert_eq!(ids(Situation { section: "Conclusion", function: "contribution", ..Default::default() }), ["t1", "s1"]);
        // a comparison paragraph
        assert_eq!(ids(Situation { section: "Results", function: "Results comparison", index: 1, count: 3 }), ["c1", "s1"]);
        assert_eq!(ids(Situation { section: "Materials and Methods", function: "dataset", ..Default::default() }), ["s1"]);
        // the Abstract gets none
        assert!(ids(Situation { section: "Abstract", function: "contribution", ..Default::default() }).is_empty());
        // many thesis entries never crowd out the topic sentence
        let many: Vec<Pattern> = (0..5).map(|k| Pattern { id: format!("t{k}"), situation: "thesis_statement".into(), when: String::new(), structure: "x".into(), example: String::new(), source: "guide".into() }).chain(p.iter().filter(|p| p.id == "s1").cloned()).collect();
        let got: Vec<String> = select(&many, &Situation { section: "Introduction", index: 1, count: 2, ..Default::default() }).iter().map(|p| p.id.clone()).collect();
        assert_eq!(got, ["t0", "t1", "s1"]);
    }

    #[test]
    fn the_block_says_guide_not_voice() {
        let (p, _) = parse(SAMPLE);
        let b = block(&select(&p, &Situation { section: "Introduction", index: 1, count: 2, ..Default::default() }));
        assert!(b.contains(HEADING) && b.contains("NOT the author's voice") && b.contains("structure only"));
        assert!(b.contains("[thesis_statement]") && b.contains("[topic_sentence]") && !b.contains("compare_two"));
        assert_eq!(block(&[]), "");
    }

    #[test]
    fn guide_text_is_recognised() {
        let (p, _) = parse(SAMPLE);
        assert!(is_guide_text(&p, "Although trains and buses differ in speed, both reduce traffic in city centres."));
        assert!(is_guide_text(&p, "  quiet libraries help students focus BECAUSE they remove interruptions "));
        assert!(!is_guide_text(&p, "The proposed loss improves recall on long inputs."));
        assert!(!is_guide_text(&p, "both"), "short text never matches");
    }

    #[test]
    fn sections_are_canonical() {
        for (t, want) in [("2. Materials and Methods", "Method"), ("Experimental results", "Result"), ("5 Conclusions", "Conclusion"), ("Related Work", "Related work"), ("緒論", "Introduction"), ("Appendix", "Appendix")] {
            assert_eq!(canonical_section(t), want, "{t}");
        }
    }
}
