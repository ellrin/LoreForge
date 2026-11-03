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

/// A caption for a table (from its cells) or a figure (from its file name),
/// for the section it goes in.
#[allow(clippy::too_many_arguments)]
pub fn caption_prompt(kind: &str, grid: &[Vec<String>], file: &str, section: &str, key_value: &str, title: &str, language: &str) -> Prompt {
    let lang = if language.starts_with("zh") { "Traditional Chinese" } else { "English" };
    let what = if kind == "figure" { "figure" } else { "table" };
    let system = format!(
        "You write the caption of a {what} in an academic paper.\n\
Rules:\n\
1. One caption: a short title-like phrase, optionally followed by one sentence on what is compared or shown.\n\
2. Use only the information given (headers, row names, file name, section). Do not state results, rankings or numbers.\n\
3. Name metrics, methods and datasets exactly as given; expand nothing you are unsure of.\n\
4. Do not mention the section, the paper or its title; the section is context only.\n\
5. A figure is known only by its file name. If the name does not say what the figure shows, reply NONE.\n\
6. No \"Table 1:\" or \"Figure 1:\" prefix, no quotes, no LaTeX.\n\
7. Write in {lang}. Reply with the caption only."
    );
    let mut user = String::new();
    if !title.trim().is_empty() {
        user += &format!("Paper title: {}\n", title.trim());
    }
    if !key_value.trim().is_empty() {
        user += &format!("Paper key value: {}\n", key_value.trim());
    }
    if !section.trim().is_empty() {
        user += &format!("Section: {}\n", section.trim());
    }
    if kind == "figure" {
        user += &format!("Figure file name: {}\n", file.trim());
    } else {
        user += "Table (first row is the header; at most 12 rows shown):\n";
        for row in grid.iter().take(12) {
            user += &row.join(" | ");
            user += "\n";
        }
    }
    Prompt { system, user }
}

/// The caption out of a reply: first non-empty line, without a label or quotes.
pub fn clean_caption(reply: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut c = line.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '“' | '”' | '*' | '`')).trim();
    for p in ["Caption:", "caption:", "Table:", "Figure:"] {
        c = c.strip_prefix(p).unwrap_or(c).trim();
    }
    // "Table 2." / "Figure 1:" left at the front
    for w in ["Table ", "Figure ", "Fig. "] {
        if let Some(rest) = c.strip_prefix(w) {
            let rest = rest.trim_start_matches(|ch: char| ch.is_ascii_digit());
            if let Some(r) = rest.strip_prefix(['.', ':']) {
                c = r.trim();
            }
        }
    }
    (!c.is_empty() && !c.eq_ignore_ascii_case("none")).then(|| c.to_string())
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
    fn caption_prompt_and_cleanup() {
        let g = vec![vec!["Model".to_string(), "AUC".into()], vec!["ResNet50".into(), "0.91".into()]];
        let p = caption_prompt("table", &g, "", "Results", "提升可解釋性", "", "en");
        assert!(p.user.contains("Model | AUC\nResNet50 | 0.91"));
        assert!(p.system.contains("caption of a table"));
        assert!(caption_prompt("figure", &[], "fig1-roc.png", "", "", "", "en").user.contains("fig1-roc.png"));
        assert_eq!(clean_caption("\n\"Table 2: AUC of each model.\"\nmore").as_deref(), Some("AUC of each model."));
        assert_eq!(clean_caption("Caption: ROC curves").as_deref(), Some("ROC curves"));
        assert_eq!(clean_caption("  \n "), None);
        assert_eq!(clean_caption("NONE"), None);
    }

    #[test]
    fn reads_sample_section_titles() {
        let s = "\\section{Introduction}\\label{sec1}\n% \\section{Hidden}\n\\section*{Ack}\n\\section{Results with {\\LaTeX}}\n";
        assert_eq!(sample_sections(s), ["Introduction", "Results with {\\LaTeX}"]);
    }
}
