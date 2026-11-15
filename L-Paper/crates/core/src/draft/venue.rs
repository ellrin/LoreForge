//! ★ 版型: which journal templates to suggest, from what the session knows.
//!
//! A plain, deterministic score, no model call:
//! - size: many claims, long claims, many question groups or experiments
//!   point to a journal; a few short claims to a conference;
//! - sections: templates whose sample has the sections the session plans
//!   (Methods, Results, Discussion, …);
//! - habit: templates the author's other papers already use.

use super::session::Session;

/// A template as the suggester sees it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Candidate {
    pub id: String,
    /// "journal" or "conference" (anything else counts as a journal).
    pub kind: String,
    /// `\section` titles of its sample.
    pub sections: Vec<String>,
    /// Other papers in the workspace on this template.
    pub uses: usize,
}

/// Does the session point to a journal (Some(true)), a conference
/// (Some(false)), or is there too little to say (None)?
pub fn wants_journal(sess: &Session) -> Option<bool> {
    let claims = sess.claims.len();
    let claim_chars: usize = sess.claims.iter().map(|c| c.zh.chars().count() + c.en.split_whitespace().count() * 2).sum();
    let groups = sess.groups.iter().filter(|g| !g.id.starts_with(['a', 'v'])).count();
    let experiments = sess.experiments.len();
    if claims == 0 && groups == 0 && experiments == 0 {
        return None;
    }
    Some(claims >= 5 || claim_chars >= 400 || groups >= 8 || experiments >= 6)
}

/// The sections the session plans to write (groups, outline, claims' homes).
pub fn planned_sections(sess: &Session) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let names = sess.groups.iter().map(|g| &g.section).chain(sess.outline.iter().map(|o| &o.section)).chain(sess.claims.iter().map(|c| &c.home));
    for s in names {
        if !s.trim().is_empty() && !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

/// "Methods" matches "Method", "Materials and methods", "2 Methodology".
fn has_section(sample: &[String], want: &str) -> bool {
    let stem: String = want.to_lowercase().chars().take(6).collect();
    sample.iter().any(|s| s.to_lowercase().contains(&stem))
}

/// Up to `n` template ids, best first; only templates with some reason.
pub fn suggest(sess: &Session, cands: &[Candidate], n: usize) -> Vec<String> {
    let journal = wants_journal(sess);
    let planned = planned_sections(sess);
    let mut scored: Vec<(i64, &Candidate)> = cands
        .iter()
        .map(|c| {
            let is_journal = c.kind != "conference";
            let kind = match journal {
                Some(j) if j == is_journal => 30,
                _ => 0,
            };
            let fit = if planned.is_empty() { 0 } else { 20 * planned.iter().filter(|p| has_section(&c.sections, p)).count() as i64 / planned.len() as i64 };
            let habit = 10 * c.uses.min(3) as i64;
            (kind + fit + habit, c)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    scored.into_iter().take(n).map(|(_, c)| c.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::session::{Claim, Group};

    fn cand(id: &str, kind: &str, sections: &[&str], uses: usize) -> Candidate {
        Candidate { id: id.into(), kind: kind.into(), sections: sections.iter().map(|s| s.to_string()).collect(), uses }
    }

    fn session(claims: usize) -> Session {
        let mut s = Session::default();
        for (i, sec) in ["Introduction", "Methods", "Results", "Discussion"].iter().enumerate() {
            s.groups.push(Group { id: format!("g{}", i + 1), section: (*sec).into(), title: "t".into(), ..Default::default() });
        }
        for i in 0..claims {
            s.claims.push(Claim { id: format!("c{i}"), zh: "主張".into(), home: "Results".into(), ..Default::default() });
        }
        s
    }

    #[test]
    fn nothing_known_suggests_only_by_habit() {
        let cands = [cand("conf-a", "conference", &["Introduction"], 0), cand("jour-b", "journal", &[], 2)];
        assert_eq!(wants_journal(&Session::default()), None);
        assert_eq!(suggest(&Session::default(), &cands, 3), ["jour-b"]);
    }

    #[test]
    fn size_picks_the_kind_and_sections_break_ties() {
        let cands = [
            cand("conf-imrad", "conference", &["Introduction", "Method", "Results", "Discussion"], 0),
            cand("conf-short", "conference", &["Introduction", "Approach"], 0),
            cand("jour-imrad", "journal", &["Introduction", "Materials and methods", "Results", "Discussion", "Conclusion"], 0),
            cand("jour-other", "journal", &["Background"], 0),
        ];
        // a few claims: conferences first, the one with the planned sections on top
        assert_eq!(suggest(&session(2), &cands, 3), ["conf-imrad", "conf-short", "jour-imrad"]);
        // many claims: journals first
        assert_eq!(suggest(&session(6), &cands, 3), ["jour-imrad", "jour-other", "conf-imrad"]);
        // the author's habit counts too
        let mut c2 = cands.to_vec();
        c2[3].uses = 3;
        assert_eq!(suggest(&session(6), &c2, 2), ["jour-other", "jour-imrad"]);
        assert_eq!(suggest(&session(6), &cands, 1).len(), 1);
    }
}
