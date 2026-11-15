//! Phrase retrieval.
//!
//! Stage 1 (this file): filter by the structured columns, then rank by keyword
//! overlap and priority. This works on every device, with no model at all.
//! Stage 2 (later): embedding search + reranker on top of the same filter.

use crate::profile::Phrase;
use crate::text::tokens;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Query {
    pub section: Option<String>,
    pub function: Option<String>,
    pub position: Option<String>,
    /// Free text, e.g. the user's bullet points.
    pub text: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub phrase: Phrase,
    pub score: f32,
}

fn matches(field: &str, want: &Option<String>) -> bool {
    match want.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(w) => field.eq_ignore_ascii_case(w) || field.eq_ignore_ascii_case("Any"),
    }
}

pub fn search(phrases: &[Phrase], q: &Query) -> Vec<Hit> {
    let want: HashSet<String> = q.text.as_deref().map(tokens).unwrap_or_default().into_iter().collect();
    let mut hits: Vec<Hit> = phrases
        .iter()
        .filter(|p| {
            matches(&p.section, &q.section)
                && matches(&p.function, &q.function)
                && matches(&p.position, &q.position)
        })
        .map(|p| {
            let overlap = if want.is_empty() {
                0.0
            } else {
                let have: HashSet<String> = tokens(&format!(
                    "{} {} {} {}",
                    p.template,
                    p.example,
                    p.function.replace('_', " "),
                    p.tags.join(" ")
                ))
                .into_iter()
                .collect();
                want.intersection(&have).count() as f32 / want.len() as f32
            };
            // Overlap dominates; priority (1-5) breaks ties.
            let score = overlap * 10.0 + p.priority as f32 * 0.2;
            Hit { phrase: p.clone(), score }
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.phrase.id.cmp(&b.phrase.id)));
    hits.truncate(q.limit.unwrap_or(10).clamp(1, 50));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, section: &str, function: &str, template: &str, priority: u8) -> Phrase {
        Phrase {
            id: id.into(),
            section: section.into(),
            function: function.into(),
            position: "opening".into(),
            template: template.into(),
            example: String::new(),
            tags: vec![],
            priority,
        }
    }

    #[test]
    fn filters_by_section_and_ranks_by_overlap() {
        let ps = vec![
            p("A", "Introduction", "research_gap", "Despite progress in {field}, {problem} remains unexplored.", 3),
            p("B", "Result", "comparison", "Compared with {baseline}, our method improves {metric}.", 3),
            p("C", "Introduction", "motivation", "Recent work on retrieval has grown rapidly.", 5),
        ];
        let hits = search(&ps, &Query {
            section: Some("Introduction".into()),
            text: Some("problem remains unexplored".into()),
            ..Default::default()
        });
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].phrase.id, "A");
    }

    #[test]
    fn any_section_matches_every_filter() {
        let ps = vec![p("X", "Any", "transition", "In contrast, {x}.", 3)];
        let hits = search(&ps, &Query { section: Some("Method".into()), ..Default::default() });
        assert_eq!(hits.len(), 1);
    }
}
