//! Decomposition: turn a sentence or paragraph the user likes into reusable
//! parts for the phrase bank. The model only proposes; the user picks what to
//! keep in the database editor.

use crate::compose::Prompt;
use crate::profile::{Profile, POSITIONS, SECTIONS};
use crate::table::next_id;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProposedPhrase {
    #[serde(default)]
    pub id: String,
    pub section: String,
    pub function: String,
    pub position: String,
    pub template: String,
    #[serde(default)]
    pub example: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProposedExpression {
    #[serde(default)]
    pub id: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub usage: String,
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub example: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Proposals {
    #[serde(default)]
    pub phrases: Vec<ProposedPhrase>,
    #[serde(default)]
    pub expressions: Vec<ProposedExpression>,
}

/// The reply `build_prompt` asks for.
pub fn reply_schema() -> serde_json::Value {
    use crate::compose::{json_array, json_object, json_string};
    let s = json_string;
    let phrase = json_object(&[("section", s()), ("function", s()), ("position", s()), ("template", s()), ("example", s()), ("tags", json_array(s()))]);
    let expression = json_object(&[("kind", s()), ("text", s()), ("usage", s()), ("section", s()), ("example", s())]);
    json_object(&[("phrases", json_array(phrase)), ("expressions", json_array(expression))])
}

pub fn build_prompt(profile: &Profile, text: &str, section: Option<&str>) -> Prompt {
    let mut functions: Vec<&str> = profile.phrases.iter().map(|p| p.function.as_str()).collect();
    functions.sort();
    functions.dedup();
    let system = format!(
        "You turn academic writing the author admires into reusable parts for the author's phrase bank.\n\
For every sentence, write a TEMPLATE: keep the rhetorical frame and the fixed wording; replace \
content-specific words (topics, methods, datasets, numbers, citations, symbols) with {{slot_name}} \
placeholders in snake_case. Keep LaTeX commands that belong to the frame.\n\
Also extract EXPRESSIONS worth reusing on their own: opening connectives (kind \"connective\"), \
collocations (kind \"collocation\"), and short phrases (kind \"phrase\"). Give each a one-line usage note.\n\
Use only what is in the text; do not invent wording.\n\
section is one of: {sections}. position is one of: {positions}.\n\
function is a short snake_case label; reuse one of these when it fits: {functions}.\n\
Reply with JSON only:\n\
{{\"phrases\":[{{\"section\":\"\",\"function\":\"\",\"position\":\"\",\"template\":\"\",\"example\":\"<the original sentence>\",\"tags\":[\"\"]}}],\
\"expressions\":[{{\"kind\":\"\",\"text\":\"\",\"usage\":\"\",\"section\":\"\",\"example\":\"\"}}]}}",
        sections = SECTIONS.join(", "),
        positions = POSITIONS.join(", "),
        functions = if functions.is_empty() { "none yet".into() } else { functions.join(", ") },
    );
    let mut user = String::new();
    if let Some(s) = section.filter(|s| !s.is_empty()) {
        user += &format!("This text is from a {s} section.\n\n");
    }
    user += text.trim();
    Prompt { system, user }
}

/// Parse the reply and give every proposal the next free id in the profile.
pub fn parse_proposals(profile: &Profile, reply: &str) -> Result<Proposals, String> {
    let start = reply.find('{').ok_or("no JSON object in reply")?;
    let end = reply.rfind('}').ok_or("no JSON object in reply")?;
    let mut p: Proposals = serde_json::from_str(&reply[start..=end]).map_err(|e| e.to_string())?;
    p.phrases.retain(|x| !x.template.trim().is_empty());
    p.expressions.retain(|x| !x.text.trim().is_empty());

    let mut phrase_ids: Vec<String> = profile.phrases.iter().map(|x| x.id.clone()).collect();
    for x in &mut p.phrases {
        x.id = next_id(phrase_ids.iter().map(String::as_str), "P");
        phrase_ids.push(x.id.clone());
        if !POSITIONS.contains(&x.position.as_str()) {
            x.position = "body".into();
        }
        if !SECTIONS.contains(&x.section.as_str()) {
            x.section = "Any".into();
        }
    }
    let mut expr_ids: Vec<String> = profile.expressions.iter().map(|x| x.id.clone()).collect();
    for x in &mut p.expressions {
        x.id = next_id(expr_ids.iter().map(String::as_str), "X");
        expr_ids.push(x.id.clone());
    }
    if p.phrases.is_empty() && p.expressions.is_empty() {
        return Err("nothing reusable found".into());
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Manifest, Phrase};

    fn profile() -> Profile {
        Profile {
            root: Default::default(),
            authors: vec![],
            affiliations: vec![],
            manifest: Manifest { name: "t".into(), schema_version: 1, language: "en".into() },
            phrases: vec![Phrase {
                id: "P017".into(),
                section: "Result".into(),
                function: "finding".into(),
                position: "opening".into(),
                template: "x".into(),
                example: String::new(),
                tags: vec![],
                priority: 3,
            }],
            expressions: vec![],
            vocabulary: vec![],
            rules: vec![],
        }
    }

    #[test]
    fn prompt_offers_existing_functions() {
        let pr = build_prompt(&profile(), "Some text.", Some("Result"));
        assert!(pr.system.contains("finding"));
        assert!(pr.user.starts_with("This text is from a Result section."));
    }

    #[test]
    fn assigns_ids_and_fixes_bad_enums() {
        let reply = r#"{"phrases":[{"section":"Results","function":"finding","position":"start","template":"Compared with {baseline}, {method} {verb} {metric}.","example":"..."}],
                       "expressions":[{"kind":"connective","text":"Notably,","usage":"Flags a striking result"}]}"#;
        let p = parse_proposals(&profile(), reply).unwrap();
        assert_eq!(p.phrases[0].id, "P018");
        assert_eq!(p.phrases[0].section, "Any");
        assert_eq!(p.phrases[0].position, "body");
        assert_eq!(p.expressions[0].id, "X001");
    }
}
