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
    /// Where the new paragraph goes in its section (0-based) and how many paragraphs the
    /// section will have: picks the guide's sentence structures ([`crate::patterns`]).
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub count: Option<usize>,
}

impl ComposeRequest {
    /// The paragraph's situation for [`crate::patterns::select`].
    pub fn situation(&self) -> crate::patterns::Situation<'_> {
        crate::patterns::Situation { section: &self.section, function: self.function.as_deref().unwrap_or(""), index: self.index.unwrap_or(0), count: self.count.unwrap_or(0) }
    }
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
    // the author's voice only: a row marked as guide material is never offered as his pattern
    hits.retain(|h| !h.phrase.tags.iter().any(|t| crate::styleprofile::is_guide_mark(t)));
    hits
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

/// `style` = the author's style notes (`draft::style_text`; empty: none), as rules to follow.
pub fn build_prompt(profile: &Profile, req: &ComposeRequest, points: &[Point], refs: &[Hit], style: &str) -> Prompt {
    let lang = if req.language.starts_with("zh") { "Traditional Chinese" } else { "English" };
    let system = format!(
        "You write one paragraph of an academic paper in the author's own voice.\n\
Rules:\n\
1. The points are the only source of content: they are what the author wants to convey (his motivation, the facts, his own view), and every claim, number and finding comes from them.\n\
2. Write in the author's voice: phrase as many points as you can with the author's patterns below, filling the {{slots}} from the point and keeping the pattern wording. \
You may join related points in one sentence and add the transitions the patterns use. \
A pattern is a sentence frame, never a source of content: its example shows the wording only, \
and the example's subject, methods, numbers and findings belong to another paper and must never appear. \
A pattern that fits no point is not used.\n\
3. Do not polish, embellish, or add rhetoric. Do not add claims, numbers, methods, experiments or citations that are not in the points.\n\
4. Cover every point: keep the author's motivation and his own view recognisable, not only the facts. Points marked EMPHASIS must appear in the first or the last sentence.\n\
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
    crate::draft::with_style(Prompt { system, user }, style)
}

/// `prompt` with the guide's sentence structures for this paragraph (a [`crate::patterns::block`];
/// empty: unchanged). They go after the author's own patterns and say they are not his voice.
pub fn with_patterns(mut prompt: Prompt, block: &str) -> Prompt {
    if !block.trim().is_empty() {
        prompt.user += block;
    }
    prompt
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

/// A JSON Schema object with these fields, all required and nothing else:
/// the shape every engine's structured output accepts (Claude, Claude Code,
/// llama-server `json_schema`).
pub fn json_object(fields: &[(&str, serde_json::Value)]) -> serde_json::Value {
    let props: serde_json::Map<String, serde_json::Value> = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let required: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
    serde_json::json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

pub fn json_array(items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "type": "array", "items": items })
}

pub fn json_string() -> serde_json::Value {
    serde_json::json!({ "type": "string" })
}

/// A string, or null when there is none.
pub fn json_string_or_null() -> serde_json::Value {
    serde_json::json!({ "anyOf": [{ "type": "string" }, { "type": "null" }] })
}

/// The reply `build_prompt` asks for.
pub fn reply_schema() -> serde_json::Value {
    json_object(&[("sentences", json_array(json_object(&[("text", json_string()), ("phrase_id", json_string_or_null())])))])
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
            style_checks: Default::default(),
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
            index: None,
            count: None,
        }
    }

    #[test]
    fn key_value_becomes_a_rule() {
        let pr = profile();
        let mut r = req();
        let pts = parse_points(&r.points);
        assert!(!build_prompt(&pr, &r, &pts, &[], "").system.contains("key value"));
        r.key_value = Some("提升骨折判讀的可解釋性".into());
        assert!(build_prompt(&pr, &r, &pts, &[], "").system.contains("8. The whole paper argues one key value: \"提升骨折判讀的可解釋性\""));
    }

    #[test]
    fn examples_are_marked_as_wording_only() {
        // Gemma copied example content (other experiments, other numbers)
        // into drafts while padding to length; the prompt must say not to.
        let pr = profile();
        let r = req();
        let pts = parse_points(&r.points);
        let s = build_prompt(&pr, &r, &pts, &[], "").system;
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
        let prompt = build_prompt(&pr, &r, &pts, &refs, "");
        assert!(prompt.user.contains("P1") && prompt.user.contains("EMPHASIS: recall improves"));
        assert!(prompt.system.contains("\"utilize\""));
        assert!(!prompt.system.contains("style rules"), "no notes, nothing added");
        // the workspace's style notes reach the 寫作 prompt as rules
        let ws = std::env::temp_dir().join(format!("pc-compose-style-{}", std::process::id()));
        std::fs::create_dir_all(ws.join("style")).unwrap();
        std::fs::write(ws.join("style/10-writing.md"), "Never use contractions.").unwrap();
        let styled = build_prompt(&pr, &r, &pts, &refs, &crate::draft::style_text(&ws));
        assert!(styled.system.contains("style rules: follow them") && styled.system.contains("Never use contractions."));
        assert_eq!(styled.user, prompt.user);
        let _ = std::fs::remove_dir_all(&ws);
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

    #[test]
    fn guide_structures_fit_the_paragraph_and_stay_apart_from_the_voice() {
        let root = std::env::temp_dir().join(format!("pc-compose-guide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = &root;
        std::fs::create_dir_all(dir.join("style")).unwrap();
        std::fs::write(
            dir.join("style/patterns.toml"),
            "[[pattern]]\nid = \"t\"\nsituation = \"thesis_statement\"\nstructure = \"{topic} {claim} because {reason}.\"\nexample = \"Quiet rooms help people focus because they remove interruptions.\"\nsource = \"guide\"\n\n\
[[pattern]]\nid = \"s\"\nsituation = \"topic_sentence\"\nstructure = \"{topic} {key words}.\"\nexample = \"Cycling is a practical way to commute for several reasons.\"\nsource = \"guide\"\n",
        )
        .unwrap();
        let pr = profile();
        let mut r = req();
        let pts = parse_points(&r.points);
        // the Introduction's last paragraph: thesis and topic sentence
        r.section = "Introduction".into();
        (r.index, r.count) = (Some(2), Some(3));
        let p = with_patterns(build_prompt(&pr, &r, &pts, &[], ""), &crate::patterns::block_for(dir, &r.situation()));
        let (mine, guide) = p.user.split_once(crate::patterns::HEADING).expect("the guide block is there");
        assert!(guide.contains("Quiet rooms") && guide.contains("Cycling"));
        assert!(!mine.contains("Quiet rooms") && !mine.contains("Cycling"), "guide examples are never listed as the author's patterns");
        assert!(!p.system.contains("Quiet rooms"));
        // a Method paragraph: the topic sentence only
        r.section = "Method".into();
        (r.index, r.count) = (Some(0), Some(3));
        let p = with_patterns(build_prompt(&pr, &r, &pts, &[], ""), &crate::patterns::block_for(dir, &r.situation()));
        assert!(p.user.contains("Cycling") && !p.user.contains("Quiet rooms"));
        // no file: the prompt is unchanged
        let bare = build_prompt(&pr, &r, &pts, &[], "");
        assert_eq!(with_patterns(bare.clone(), &crate::patterns::block_for(&dir.join("none"), &r.situation())).user, bare.user);
        assert!(bare.system.contains("his motivation, the facts, his own view"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
