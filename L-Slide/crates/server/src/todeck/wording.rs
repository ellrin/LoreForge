//! The author's wording rules (`style/wording.toml` of the shared folder, lore-notation
//! `punct`) for the English a 論文報告 plan writes: the slide lines, the takeaway line and the
//! speaker notes. Wording only (preferred spellings, hyphen compounds): no punctuation marks and
//! no sentence limits, a slide line is not prose. Titles, `Ref:` lines, citations, equations
//! and any text with Chinese are never touched. What the fix cannot settle is a job warning.

use lore_notation::punct::{self, Guard};

use super::plan::Plan;
use super::source::Material;

/// The shared wording rule as a wording-only guard (None: nothing to check).
pub fn guard_of(home: &lore_home::Home) -> Option<Guard> {
    let raw = std::fs::read_to_string(home.shared_file("style/wording.toml")?).ok()?;
    let rule = punct::Rule::from_toml(&raw)?.wording_only();
    rule.enabled().then(|| Guard { rule, exempt: vec![] })
}

fn has_han(s: &str) -> bool {
    s.chars().any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
}

/// Fix the plan's English lines in place; returns the warnings (繁中) for what is left.
pub fn apply(plan: &mut Plan, m: &Material, guard: &Guard) -> Vec<String> {
    if !guard.active() {
        return vec![];
    }
    // the paper's own hyphenated terms stay: its title, keywords, references and notation
    let own = format!("{} {} {} {}", m.title, m.keywords.join(" "), m.refs.values().cloned().collect::<Vec<_>>().join(" "), serde_json::to_string(&m.notation).unwrap_or_default());
    let g = Guard { rule: guard.rule.clone(), exempt: punct::hyphenated_terms(&own) };
    let mut warnings = vec![];
    let mut one = |s: &mut String, at: &str| {
        if s.trim().is_empty() || has_han(s) {
            return;
        }
        let (fixed, left) = g.finish(s);
        *s = fixed;
        for v in left {
            warnings.push(format!("{at}：用字規則還有一處沒有改掉（{}）", v.describe()));
        }
    };
    for (k, sl) in plan.slides.iter_mut().enumerate() {
        let at = format!("第 {} 頁", k + 1);
        for t in sl.bullets.iter_mut().chain(sl.steps.iter_mut()).chain(sl.flow.iter_mut()) {
            one(t, &at);
        }
        one(&mut sl.takeaway, &at);
        one(&mut sl.notes, &format!("{at}的講者備註"));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todeck::plan::PlanSlide;

    fn guard() -> Guard {
        let rule = punct::Rule::from_toml("[punctuation]\nspellings = [\"softmax\", \"cross entropy\"]\nhyphen_whitelist = [\"X-ray\"]\n").unwrap().wording_only();
        Guard { rule, exempt: vec![] }
    }

    #[test]
    fn spellings_are_fixed_in_lines_and_notes_but_not_titles_citations_or_chinese() {
        let mut m = Material { title: "A Self-Supervised Study".into(), ..Default::default() };
        m.refs.insert("a".into(), "Lee et al., Data-Driven Models".into());
        let mut plan = Plan {
            slides: vec![PlanSlide {
                title: "The soft-max head".into(),
                bullets: vec!["A soft-max with cross-entropy: 0.70–0.75, self-supervised, data-driven and task-specific.".into(), "中文的 soft-max 不改".into(), "See \\cite{soft-max} and $soft-max$.".into()],
                takeaway: "Soft max wins".into(),
                notes: "We use cross-entropy; a long long sentence is not checked here.".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let w = apply(&mut plan, &m, &guard());
        let s = &plan.slides[0];
        assert_eq!(s.title, "The soft-max head");
        assert_eq!(s.bullets[0], "A softmax with cross entropy: 0.70–0.75, self-supervised, data-driven and task specific.");
        assert_eq!(s.bullets[1], "中文的 soft-max 不改");
        assert_eq!(s.bullets[2], "See \\cite{soft-max} and $soft-max$.");
        assert_eq!(s.takeaway, "Softmax wins");
        assert_eq!(s.notes, "We use cross entropy; a long long sentence is not checked here.");
        assert!(w.is_empty(), "{w:?}");
        assert!(apply(&mut plan, &m, &Guard::default()).is_empty());
    }

    #[test]
    fn numbered_notes_keep_their_lines() {
        let mut plan = Plan { slides: vec![PlanSlide { notes: "This page covers the head.\n1. A soft-max turns scores into shares.\n2. The cross-entropy compares them.".into(), ..Default::default() }], ..Default::default() };
        apply(&mut plan, &Material::default(), &guard());
        assert_eq!(plan.slides[0].notes, "This page covers the head.\n1. A softmax turns scores into shares.\n2. The cross entropy compares them.");
        assert!(crate::todeck::plan::notes_shape_ok(&plan.slides[0].notes));
    }
}
