//! The new-paper wizard's model step: from the author's bullet outline and
//! key value, title suggestions and a section skeleton (headings, plus at
//! most two sentences on what each part will say). No paper text is written.

use pc_core::compose::Prompt;
use pc_core::meta::{OutlineSection, OutlineSub};
use serde::{Deserialize, Serialize};

pub fn outline_prompt(outline: &str, key_value: &str, template_sections: &[String], language: &str) -> Prompt {
    let lang = if language.starts_with("zh") { "Traditional Chinese" } else { "English" };
    let system = format!(
        "You plan an academic paper before it is written. From the author's bullet outline and key value, \
propose titles and the section skeleton.\n\
Rules:\n\
1. The key value is the paper's single message. Every title must express it; every section must serve it.\n\
2. Follow the outline's content and order. Do not add claims, results, numbers or citations that are not in it.\n\
3. When the journal's sample sections are given, use the paper sections the journal expects, in its order; \
skip sample sections that only demonstrate formatting (equations, tables, figures, algorithms, cross referencing and the like).\n\
4. Add subsections only where the outline has distinct parts.\n\
5. `intent` is at most two sentences on what that part will say: a placeholder the author will rewrite, not finished prose.\n\
6. Give exactly 3 titles. Write titles, headings and intents in {lang}.\n\
7. Reply with JSON only: {{\"titles\":[\"...\"],\"sections\":[{{\"title\":\"Introduction\",\"intent\":\"...\",\"subsections\":[{{\"title\":\"...\",\"intent\":\"...\"}}]}}]}}"
    );
    let mut user = format!("Key value: {}\n", key_value.trim());
    if !template_sections.is_empty() {
        user += &format!("Journal sample sections, in order: {}\n", template_sections.join("; "));
    }
    user += &format!("\nOutline:\n{}\n", outline.trim());
    Prompt { system, user }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Plan {
    pub titles: Vec<String>,
    pub sections: Vec<OutlineSection>,
}

/// At most two sentences.
fn two_sentences(s: &str) -> String {
    pc_core::text::sentences(s.trim()).into_iter().take(2).collect::<Vec<_>>().join(" ")
}

pub fn parse_plan(reply: &str) -> Result<Plan, String> {
    let start = reply.find('{').ok_or("回覆裡沒有 JSON")?;
    let end = reply.rfind('}').ok_or("回覆裡沒有 JSON")?;
    let raw: Plan = serde_json::from_str(&reply[start..=end]).map_err(|e| format!("看不懂回覆：{e}"))?;
    let titles: Vec<String> = raw.titles.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).take(3).collect();
    let sections: Vec<OutlineSection> = raw
        .sections
        .into_iter()
        .filter(|s| !s.title.trim().is_empty())
        .map(|s| OutlineSection {
            title: s.title.trim().to_string(),
            intent: two_sentences(&s.intent),
            subsections: s
                .subsections
                .into_iter()
                .filter(|x| !x.title.trim().is_empty())
                .map(|x| OutlineSub { title: x.title.trim().to_string(), intent: two_sentences(&x.intent) })
                .collect(),
        })
        .collect();
    if sections.is_empty() {
        return Err("回覆裡沒有章節".into());
    }
    Ok(Plan { titles, sections })
}

/// `\section{...}` titles of a template's sample, in order.
pub fn sample_sections(sample: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in sample.lines() {
        let code = line.split('%').next().unwrap_or("").trim();
        if let Some(rest) = code.strip_prefix("\\section{") {
            let mut depth = 1;
            let mut title = String::new();
            for c in rest.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                title.push(c);
            }
            if !title.trim().is_empty() {
                out.push(title.trim().to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_key_value_outline_and_sample_sections() {
        let p = outline_prompt("- 背景\n- 方法", "提升可解釋性", &["Introduction".into(), "Methods".into()], "en");
        assert!(p.user.starts_with("Key value: 提升可解釋性\nJournal sample sections, in order: Introduction; Methods\n"));
        assert!(p.user.contains("Outline:\n- 背景\n- 方法"));
        assert!(p.system.contains("in English"));
    }

    #[test]
    fn parses_and_trims_the_plan() {
        let reply = "Here:\n```json\n{\"titles\":[\"A\",\" \",\"B\",\"C\",\"D\"],\"sections\":[{\"title\":\"Intro\",\"intent\":\"One. Two. Three.\",\"subsections\":[{\"title\":\"Gap\",\"intent\":\"Why.\"},{\"title\":\"\"}]},{\"title\":\" \"}]}\n```";
        let p = parse_plan(reply).unwrap();
        assert_eq!(p.titles, ["A", "B", "C"]);
        assert_eq!(p.sections.len(), 1);
        assert_eq!(p.sections[0].intent, "One. Two.");
        assert_eq!(p.sections[0].subsections.len(), 1);
        assert!(parse_plan("{\"titles\":[],\"sections\":[]}").is_err());
    }

    #[test]
    fn reads_sample_section_titles() {
        let s = "\\section{Introduction}\\label{sec1}\n% \\section{Hidden}\n\\section*{Ack}\n\\section{Results with {\\LaTeX}}\n";
        assert_eq!(sample_sections(s), ["Introduction", "Results with {\\LaTeX}"]);
    }
}
