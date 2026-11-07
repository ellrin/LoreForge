//! One-shot paragraph composition, engine-agnostic.
//!
//! The same request, the same retrieved phrases and the same prompt go to
//! whichever engine runs it (local model, home server, Claude). The output is
//! parsed and validated here, so every engine is held to the user's phrase
//! bank in the same way ("tone lock").

use crate::profile::Profile;
use crate::retrieve::{search, Hit, Query};
use crate::text::{strip_latex, tokens, word_count};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ComposeRequest {
    /// One point per line. A leading `!` marks it as emphasised.
    pub points: String,
    pub section: String,
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default = "default_min")]
    pub min_words: usize,
    #[serde(default = "default_max")]
    pub max_words: usize,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    /// What the whole paper argues; every paragraph must serve it.
    #[serde(default)]
    pub key_value: Option<String>,
}

fn default_min() -> usize {
    120
}
fn default_max() -> usize {
    180
}
fn default_language() -> String {
    "en".into()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Point {
    pub text: String,
    pub emphasis: bool,
}

/// Parse the user's bullet list. Accepts `-`, `*`, `•` and numbering; a
/// leading `!` (or full-width `！`) marks emphasis.
pub fn parse_points(raw: &str) -> Vec<Point> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l.trim_start_matches(['-', '*', '•']).trim_start();
            let l = l
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches(['.', ')'])
                .trim_start();
            let (emphasis, text) = match l.strip_prefix('!').or_else(|| l.strip_prefix('！')) {
                Some(rest) => (true, rest.trim()),
                None => (false, l),
            };
            Point { text: text.to_string(), emphasis }
        })
        .filter(|p| !p.text.is_empty())
        .collect()
}

/// Phrases offered to the engine as the allowed patterns.
pub fn reference_phrases(profile: &Profile, req: &ComposeRequest, points: &[Point]) -> Vec<Hit> {
    let text = points.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
    let mut hits = search(
        &profile.phrases,
        &Query {
            section: Some(req.section.clone()),
            function: req.function.clone(),
            position: None,
            text: Some(text.clone()),
            limit: Some(8),
        },
    );
    // Always offer some transitions so sentences can be linked in the user's voice.
    let transitions = search(
        &profile.phrases,
        &Query { section: Some("Any".into()), function: Some("transition".into()), text: Some(text), limit: Some(3), ..Default::default() },
    );
    for t in transitions {
        if !hits.iter().any(|h| h.phrase.id == t.phrase.id) {
            hits.push(t);
        }
    }
    hits
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

pub fn build_prompt(profile: &Profile, req: &ComposeRequest, points: &[Point], refs: &[Hit]) -> Prompt {
    let lang = if req.language.starts_with("zh") { "Traditional Chinese" } else { "English" };
    let system = format!(
        "You write one paragraph of an academic paper in the author's own voice.\n\
Rules:\n\
1. The points are the only source of content: every claim, number and finding comes from them.\n\
2. Write in the author's voice: phrase as many points as you can with the author's patterns below, filling the {{slots}} from the point and keeping the pattern wording. \
You may join related points in one sentence and add the transitions the patterns use. \
A pattern is a sentence frame, never a source of content: its example shows the wording only, \
and the example's subject, methods, numbers and findings belong to another paper and must never appear. \
A pattern that fits no point is not used.\n\
3. Do not polish, embellish, or add rhetoric. Do not add claims, numbers, methods, experiments or citations that are not in the points.\n\
4. Cover every point. Points marked EMPHASIS must appear in the first or the last sentence.\n\
5. Length: aim for {min} to {max} words, but never pad: a shorter paragraph is right when the points are few. Language: {lang}.\n\
6. Avoid these words: {avoid}.\n\
7. Reply with JSON only: {{\"sentences\":[{{\"text\":\"...\",\"phrase_id\":\"P001\"}}]}}. \
Use \"phrase_id\": null for a sentence that uses no pattern.",
        min = req.min_words,
        max = req.max_words,
        avoid = if profile.vocabulary.is_empty() {
            "none".to_string()
        } else {
            profile.vocabulary.iter().map(|v| format!("\"{}\"", v.avoid)).collect::<Vec<_>>().join(", ")
        },
    );

    let mut system = system;
    if let Some(k) = req.key_value.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        system += &format!("\n8. The whole paper argues one key value: \"{k}\". Every sentence must serve it; do not drift to other topics.");
    }

    let mut user = format!("Section: {}\n", req.section);
    if let Some(f) = req.function.as_deref().filter(|f| !f.is_empty()) {
        user += &format!("Paragraph function: {f}\n");
    }
    user += "\nAuthor's patterns:\n";
    for h in refs {
        let p = &h.phrase;
        user += &format!("- {} [{} / {}] {}", p.id, p.function, p.position, p.template);
        if !p.example.is_empty() {
            user += &format!("  (wording example from another paper: {})", p.example);
        }
        user += "\n";
    }
    user += "\nPoints:\n";
    for p in points {
        user += &format!("- {}{}\n", if p.emphasis { "EMPHASIS: " } else { "" }, p.text);
    }
    if let Some(b) = req.before.as_deref().filter(|s| !s.trim().is_empty()) {
        user += &format!("\nPrevious paragraph (link to it, do not repeat it):\n{b}\n");
    }
    if let Some(a) = req.after.as_deref().filter(|s| !s.trim().is_empty()) {
        user += &format!("\nNext paragraph (lead into it):\n{a}\n");
    }
    Prompt { system, user }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sentence {
    pub text: String,
    #[serde(default)]
    pub phrase_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub sentences: Vec<Sentence>,
}

/// Parse the engine's reply. Tolerates code fences and text around the JSON.
pub fn parse_draft(reply: &str) -> Result<Draft, String> {
    let start = reply.find('{').ok_or("no JSON object in reply")?;
    let end = reply.rfind('}').ok_or("no JSON object in reply")?;
    let draft: Draft = serde_json::from_str(&reply[start..=end]).map_err(|e| e.to_string())?;
    if draft.sentences.is_empty() {
        return Err("reply has no sentences".into());
    }
    Ok(draft)
}

#[derive(Debug, Clone, Serialize)]
pub struct DraftCheck {
    pub words: usize,
    /// Sentences that use one of the offered patterns.
    pub covered: usize,
    pub total: usize,
    pub problems: Vec<String>,
}

impl DraftCheck {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub fn check_draft(profile: &Profile, req: &ComposeRequest, points: &[Point], refs: &[Hit], draft: &mut Draft) -> DraftCheck {
    let mut problems = Vec::new();
    // Unknown ids are cleared, not trusted: coverage counts only real patterns.
    let mut covered = 0;
    for s in &mut draft.sentences {
        match &s.phrase_id {
            Some(id) if refs.iter().any(|h| &h.phrase.id == id) => covered += 1,
            Some(id) => {
                problems.push(format!("unknown pattern id {id}"));
                s.phrase_id = None;
            }
            None => {}
        }
    }
    let words: usize = draft.sentences.iter().map(|s| word_count(&s.text)).sum();
    // Word counts only make sense for space-separated languages.
    if !req.language.starts_with("zh") {
        let slack = |n: usize| n / 5; // 20% tolerance
        if words + slack(req.min_words) < req.min_words || words > req.max_words + slack(req.max_words) {
            problems.push(format!("length {words} words, asked {}-{}", req.min_words, req.max_words));
        }
    }
    let n = draft.sentences.len();
    for p in points.iter().filter(|p| p.emphasis) {
        let want: Vec<String> = tokens(&p.text);
        let edge = [draft.sentences.first(), draft.sentences.last()]
            .into_iter()
            .flatten()
            .map(|s| tokens(&s.text))
            .any(|have| !want.is_empty() && want.iter().filter(|w| have.contains(w)).count() * 2 >= want.len());
        if !edge {
            problems.push(format!("emphasised point not in first or last sentence: {}", p.text));
        }
    }
    for s in &draft.sentences {
        let plain = format!(" {} ", strip_latex(&s.text).to_lowercase());
        for v in &profile.vocabulary {
            if crate::check::contains_word(&plain, &v.avoid.to_lowercase()) {
                problems.push(format!("avoided word used: {}", v.avoid));
            }
        }
    }
    DraftCheck { words, covered, total: n, problems }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Manifest, Phrase, VocabEntry};

    fn profile() -> Profile {
        let ph = |id: &str, section: &str, function: &str, t: &str| Phrase {
            id: id.into(),
            section: section.into(),
            function: function.into(),
            position: "opening".into(),
            template: t.into(),
            example: String::new(),
            tags: vec![],
            priority: 3,
        };
        Profile {
            root: Default::default(),
            authors: vec![],
            affiliations: vec![],
            manifest: Manifest { name: "t".into(), schema_version: 1, language: "en".into() },
            phrases: vec![
                ph("P1", "Result", "finding", "Compared with {baseline}, {method} improves {metric} by {value}."),
                ph("P2", "Result", "evidence", "These results indicate that {claim}."),
                ph("T1", "Any", "transition", "In contrast, {x}."),
            ],
            expressions: vec![],
            vocabulary: vec![VocabEntry { avoid: "utilize".into(), prefer: "use".into(), note: String::new() }],
            rules: vec![],
        }
    }

    fn req() -> ComposeRequest {
        ComposeRequest {
            points: "- ! recall improves by 12%\n- gain largest on long inputs".into(),
            section: "Result".into(),
            function: None,
            min_words: 10,
            max_words: 60,
            language: "en".into(),
            before: None,
            after: None,
            key_value: None,
        }
    }

    #[test]
    fn key_value_becomes_a_rule() {
        let pr = profile();
        let mut r = req();
        let pts = parse_points(&r.points);
        assert!(!build_prompt(&pr, &r, &pts, &[]).system.contains("key value"));
        r.key_value = Some("提升骨折判讀的可解釋性".into());
        assert!(build_prompt(&pr, &r, &pts, &[]).system.contains("8. The whole paper argues one key value: \"提升骨折判讀的可解釋性\""));
    }

    #[test]
    fn examples_are_marked_as_wording_only() {
        // Gemma copied example content (other experiments, other numbers)
        // into drafts while padding to length; the prompt must say not to.
        let pr = profile();
        let r = req();
        let pts = parse_points(&r.points);
        let s = build_prompt(&pr, &r, &pts, &[]).system;
        assert!(s.contains("A pattern that fits no point is not used"));
        assert!(s.contains("phrase as many points as you can with the author's patterns"));
        assert!(s.contains("must never appear"));
        assert!(s.contains("never pad"));
    }

    #[test]
    fn parses_points_and_emphasis() {
        let p = parse_points("1. first\n- ! second\n\n•  ！third ");
        assert_eq!(p.len(), 3);
        assert!(!p[0].emphasis && p[1].emphasis && p[2].emphasis);
        assert_eq!(p[2].text, "third");
    }

    #[test]
    fn prompt_lists_patterns_points_and_avoid_words() {
        let pr = profile();
        let r = req();
        let pts = parse_points(&r.points);
        let refs = reference_phrases(&pr, &r, &pts);
        assert!(refs.iter().any(|h| h.phrase.id == "T1"), "transitions are always offered");
        let prompt = build_prompt(&pr, &r, &pts, &refs);
        assert!(prompt.user.contains("P1") && prompt.user.contains("EMPHASIS: recall improves"));
        assert!(prompt.system.contains("\"utilize\""));
    }

    #[test]
    fn validates_a_draft() {
        let pr = profile();
        let r = req();
        let pts = parse_points(&r.points);
        let refs = reference_phrases(&pr, &r, &pts);
        let reply = "```json\n{\"sentences\":[{\"text\":\"Compared with BM25, our method improves recall by 12%.\",\"phrase_id\":\"P1\"},{\"text\":\"The gain is largest on long inputs, which we utilize.\",\"phrase_id\":\"P9\"}]}\n```";
        let mut d = parse_draft(reply).unwrap();
        let c = check_draft(&pr, &r, &pts, &refs, &mut d);
        assert_eq!((c.covered, c.total), (1, 2));
        assert_eq!(d.sentences[1].phrase_id, None, "unknown id cleared");
        assert!(c.problems.iter().any(|p| p.contains("P9")));
        assert!(c.problems.iter().any(|p| p.contains("utilize")));
        assert!(!c.problems.iter().any(|p| p.contains("emphasised")), "{:?}", c.problems);
    }
}
