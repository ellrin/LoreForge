//! Hard-rule checks: instant, deterministic, no model.
//!
//! Supported `check` values in rules.csv (type = hard):
//! - `sentence_count`  threshold "min-max", per paragraph
//! - `max_words`       threshold "n", per sentence
//! - `banned_words`    uses vocabulary.csv, per paragraph
//! - the style checks of [`crate::stylecheck`] (`phrases`, `comma_splice`, `contractions`…),
//!   threshold = a list in the workspace's `style/checks.toml`
//!
//! Soft rules (type = soft) are listed in the report as `pending`; the
//! `softcheck` module has a model judge them.

use crate::profile::{Profile, Rule, RuleKind};
use crate::text::{paragraphs, sentences, strip_latex, word_count};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub(crate) fn parse(s: &str) -> Severity {
        match s.trim().to_lowercase().as_str() {
            "error" => Severity::Error,
            "info" => Severity::Info,
            _ => Severity::Warning,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    /// 0-based sentence index within the paragraph, when it concerns one.
    pub sentence: Option<usize>,
    /// What was found and what to write instead (style checks; `suggest` may be empty).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphReport {
    pub index: usize,
    pub sentences: usize,
    /// The sentences as split by the checker, so the UI can mark them by index.
    pub sentence_text: Vec<String>,
    pub words: usize,
    /// Worst severity found, or None when clean.
    pub status: Option<Severity>,
    pub findings: Vec<Finding>,
    /// Soft rules that apply here but need a model.
    pub pending: Vec<String>,
}

pub(crate) fn applies(rule: &Rule, scope: &str, section: Option<&str>) -> bool {
    match rule.scope.split_once(':') {
        Some(("section", name)) => {
            section.is_some_and(|s| s.eq_ignore_ascii_case(name.trim())) && scope == "paragraph"
        }
        _ => rule.scope == scope || (scope == "paragraph" && rule.scope == "sentence"),
    }
}

pub fn check_text(profile: &Profile, text: &str, section: Option<&str>) -> Vec<ParagraphReport> {
    paragraphs(text)
        .into_iter()
        .enumerate()
        .map(|(index, para)| check_paragraph(profile, index, &para, section))
        .collect()
}

fn check_paragraph(profile: &Profile, index: usize, para: &str, section: Option<&str>) -> ParagraphReport {
    let sents = sentences(para);
    let words: usize = sents.iter().map(|s| word_count(s)).sum();
    let mut findings = Vec::new();
    let mut pending = Vec::new();
    let mut prose: Option<Vec<(Option<usize>, String)>> = None;

    for rule in profile.rules.iter().filter(|r| applies(r, "paragraph", section)) {
        if rule.kind == RuleKind::Soft {
            pending.push(rule.id.clone());
            continue;
        }
        let sev = Severity::parse(&rule.severity);
        let msg = |fallback: String| {
            if rule.message.is_empty() { fallback } else { rule.message.clone() }
        };
        match rule.check.as_str() {
            "sentence_count" => {
                if let Some((lo, hi)) = parse_range(&rule.threshold) {
                    let n = sents.len();
                    if n < lo || n > hi {
                        findings.push(Finding {
                            rule: rule.id.clone(),
                            severity: sev,
                            message: format!("{} ({n})", msg(format!("{lo}-{hi} sentences expected"))),
                            sentence: None,
                            found: None,
                            suggest: None,
                        });
                    }
                }
            }
            "max_words" => {
                if let Ok(max) = rule.threshold.trim().parse::<usize>() {
                    for (i, s) in sents.iter().enumerate() {
                        let n = word_count(s);
                        if n > max {
                            findings.push(Finding {
                                rule: rule.id.clone(),
                                severity: sev,
                                message: format!("{} ({n})", msg(format!("over {max} words"))),
                                sentence: Some(i),
                                found: None,
                                suggest: None,
                            });
                        }
                    }
                }
            }
            "banned_words" => {
                for (i, s) in sents.iter().enumerate() {
                    let plain = format!(" {} ", strip_latex(s).to_lowercase());
                    for v in &profile.vocabulary {
                        let term = v.avoid.to_lowercase();
                        if contains_word(&plain, &term) {
                            let hint = if v.prefer.is_empty() {
                                format!("\u{201c}{}\u{201d}", v.avoid)
                            } else {
                                format!("\u{201c}{}\u{201d} \u{2192} \u{201c}{}\u{201d}", v.avoid, v.prefer)
                            };
                            findings.push(Finding {
                                rule: rule.id.clone(),
                                severity: sev,
                                message: hint,
                                sentence: Some(i),
                                found: Some(v.avoid.clone()),
                                suggest: Some(v.prefer.clone()).filter(|p| !p.is_empty()),
                            });
                        }
                    }
                }
            }
            kind if crate::stylecheck::is_kind(kind) => {
                let list = profile.style_checks.list(&rule.threshold);
                if crate::stylecheck::needs_list(kind) && list.is_empty() {
                    continue;
                }
                let prose = prose.get_or_insert_with(|| crate::stylecheck::prose_sentences(para, sents.len()));
                for (i, s) in prose.iter() {
                    for h in crate::stylecheck::run(kind, list, s) {
                        let hint = if h.suggest.is_empty() {
                            format!("\u{201c}{}\u{201d}", h.found)
                        } else {
                            format!("\u{201c}{}\u{201d} \u{2192} \u{201c}{}\u{201d}", h.found, h.suggest)
                        };
                        findings.push(Finding {
                            rule: rule.id.clone(),
                            severity: sev,
                            message: if rule.message.is_empty() { hint } else { format!("{} · {hint}", rule.message) },
                            sentence: *i,
                            found: Some(h.found),
                            suggest: Some(h.suggest).filter(|s| !s.is_empty()),
                        });
                    }
                }
            }
            // Unknown hard checks are ignored rather than failing the whole run.
            _ => {}
        }
    }

    let status = findings.iter().map(|f| f.severity).max();
    ParagraphReport { index, sentences: sents.len(), sentence_text: sents, words, status, findings, pending }
}

fn parse_range(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

pub(crate) fn contains_word(haystack: &str, term: &str) -> bool {
    haystack.match_indices(term).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + term.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Manifest, VocabEntry};

    fn profile() -> Profile {
        let rule = |id: &str, check: &str, threshold: &str, sev: &str| Rule {
            id: id.into(),
            kind: RuleKind::Hard,
            scope: "paragraph".into(),
            check: check.into(),
            threshold: threshold.into(),
            severity: sev.into(),
            message: String::new(),
        };
        Profile {
            root: Default::default(),
            authors: vec![],
            affiliations: vec![],
            style_checks: Default::default(),
            manifest: Manifest { name: "t".into(), schema_version: 1, language: "en".into() },
            phrases: vec![],
            expressions: vec![],
            vocabulary: vec![VocabEntry { avoid: "utilize".into(), prefer: "use".into(), note: String::new() }],
            rules: vec![
                rule("R1", "sentence_count", "2-4", "warning"),
                rule("R2", "max_words", "8", "warning"),
                rule("R3", "banned_words", "", "info"),
                Rule { kind: RuleKind::Soft, ..rule("R4", "has_topic_sentence", "", "error") },
            ],
        }
    }

    #[test]
    fn reports_hard_rules_and_lists_soft_ones() {
        let text = "We utilize a simple model.\n\nShort one. Another short one. This sentence is clearly much too long for the limit.";
        let r = check_text(&profile(), text, None);
        assert_eq!(r.len(), 2);
        // Paragraph 1: one sentence (<2) and a banned word.
        assert!(r[0].findings.iter().any(|f| f.rule == "R1"));
        assert!(r[0].findings.iter().any(|f| f.rule == "R3" && f.message.contains("use")));
        assert_eq!(r[0].pending, vec!["R4"]);
        // Paragraph 2: third sentence over 8 words.
        assert!(r[1].findings.iter().any(|f| f.rule == "R2" && f.sentence == Some(2)));
        assert_eq!(r[1].status, Some(Severity::Warning));
    }

    #[test]
    fn banned_words_match_whole_words_only() {
        assert!(contains_word(" we utilize it ", "utilize"));
        assert!(!contains_word(" utilized ", "utilize"));
    }

    #[test]
    fn style_checks_come_from_the_workspace_data() {
        let ws = std::env::temp_dir().join(format!("pc-stylecheck-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join("style")).unwrap();
        std::fs::write(ws.join("style/checks.toml"), "wordy = [{ find = \"in order to\", use = \"to\" }]\nadverbs = [\"however\"]\n").unwrap();
        let mut p = profile();
        let hard = |id: &str, check: &str, threshold: &str, sev: &str| Rule {
            id: id.into(),
            kind: RuleKind::Hard,
            scope: "sentence".into(),
            check: check.into(),
            threshold: threshold.into(),
            severity: sev.into(),
            message: String::new(),
        };
        p.rules = vec![
            hard("S1", "phrases", "wordy", "info"),
            hard("S2", "comma_splice", "adverbs", "warning"),
            hard("S3", "contractions", "", "warning"),
            hard("S4", "phrases", "jargon", "info"),
        ];
        let issues = p.load_style_checks(&ws);
        assert_eq!(issues.len(), 1, "the missing list is named: {issues:?}");
        assert!(issues[0].message.contains("jargon"));
        let text = "However, we act in order to learn. It worked, however, it's slow \\cite{in order to}.";
        let r = check_text(&p, text, None);
        let f = &r[0].findings;
        let s1 = f.iter().find(|f| f.rule == "S1").unwrap();
        assert_eq!((s1.sentence, s1.suggest.as_deref(), s1.severity), (Some(0), Some("to"), Severity::Info));
        assert!(s1.message.contains("\u{2192}"));
        assert_eq!(f.iter().filter(|f| f.rule == "S1").count(), 1, "the \\cite argument is no prose");
        let s2: Vec<_> = f.iter().filter(|f| f.rule == "S2").collect();
        assert_eq!(s2.len(), 1, "\"However,\" at the start is fine");
        assert_eq!((s2[0].sentence, s2[0].suggest.as_deref()), (Some(1), Some("; however,")));
        assert!(f.iter().any(|f| f.rule == "S3" && f.suggest.as_deref() == Some("it is")));
        // no data file: the rows find nothing and nothing breaks
        let mut q = profile();
        q.rules = p.rules.clone();
        assert!(check_text(&q, text, None)[0].findings.iter().all(|f| f.rule == "S3"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn the_example_workspace_style_checks_load_and_fire() {
        let ws = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/PaperComposer");
        let (mut p, issues) = Profile::load(&ws.join("profile")).unwrap();
        assert!(issues.iter().all(|i| !i.error), "{issues:?}");
        assert!(p.load_style_checks(&ws).is_empty(), "every list a rule names is there");
        let r = check_text(&p, "We trained it in order to learn. It converged, however, it isn't stable!", None);
        let rules: Vec<&str> = r[0].findings.iter().map(|f| f.rule.as_str()).collect();
        for want in ["R004", "R006", "R007", "R008"] {
            assert!(rules.contains(&want), "{want}: {rules:?}");
        }
    }
}
