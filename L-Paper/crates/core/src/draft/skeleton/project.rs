//! The skeleton in the shapes the later steps already read (主張登記, 章節校正, 摘要, 審稿,
//! 放進論文): a copy of the session whose outline, drafts and ledger come from the skeleton. Made
//! at call time and never stored, so the skeleton stays the one source; a session without a
//! skeleton is read as it is.

use super::*;
use crate::draft::session::{DraftPara, DraftSection, Group, GroupStatus, GroupSummary, Item, OutlineSection, Session, SECTIONS};

/// The session as the later steps read it: the skeleton projected when one exists.
pub fn reading(sess: Session) -> Session {
    project(&sess).unwrap_or(sess)
}

/// None without a planned skeleton.
pub fn project(sess: &Session) -> Option<Session> {
    let k = sess.skeleton.as_ref().filter(|k| k.exists())?;
    let mut s = sess.clone();
    s.outline = k
        .sections
        .iter()
        .map(|sec| {
            let old = old_section(sec);
            OutlineSection {
                section: old.to_string(),
                subsection: (!sec.title.eq_ignore_ascii_case(old)).then(|| sec.title.clone()),
                paras: sec.blocks.iter().flat_map(|b| b.paras.iter()).map(plan::outline_para).collect(),
                confirmed: true,
            }
        })
        .collect();
    s.outline.sort_by_key(|o| SECTIONS.iter().position(|x| *x == o.section));
    let mut drafts: Vec<DraftSection> = vec![];
    for o in &s.outline {
        let paras: Vec<DraftPara> = o
            .paras
            .iter()
            .filter_map(|op| k.para(&op.id))
            .filter(|p| !p.text.trim().is_empty())
            .map(|p| DraftPara { para: p.id.clone(), text: p.text.clone(), problems: p.problems.clone(), pending: lore_notation::pending::ids(&p.text).into_iter().collect() })
            .collect();
        if paras.is_empty() {
            continue;
        }
        match drafts.iter_mut().find(|d| d.section == o.section) {
            Some(d) => d.paras.extend(paras),
            None => drafts.push(DraftSection { section: o.section.clone(), paras, new_symbols: vec![], engine: String::new(), placed: false }),
        }
    }
    // the abstract is drafted by the old step, into the session's own drafts
    drafts.extend(sess.drafts.iter().filter(|d| d.section == "Abstract").cloned());
    s.drafts = drafts;
    // the ledger: a block is a group (its purpose explained), a paragraph an item
    s.groups = k
        .sections
        .iter()
        .flat_map(|sec| sec.blocks.iter().map(move |b| (sec, b)))
        .map(|(sec, b)| Group {
            id: b.id.clone(),
            section: old_section(sec).to_string(),
            title: b.title.clone(),
            purpose: b.purpose.clone(),
            status: GroupStatus::Done,
            summary: GroupSummary { explain: b.purpose.clone(), conclusion: b.paras.last().map(|p| p.say.clone()).unwrap_or_default(), ..Default::default() },
            ..Default::default()
        })
        .collect();
    s.items = k
        .sections
        .iter()
        .flat_map(|sec| sec.blocks.iter())
        .flat_map(|b| b.paras.iter().map(move |p| (b, p)))
        .map(|(b, p)| Item {
            id: p.id.clone(),
            group: b.id.clone(),
            label: p.say.clone(),
            role: if p.kind.is_empty() { "result".into() } else { p.kind.clone() },
            stance: p.points.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("；"),
            evidence: p.sources.clone(),
            refs: p.cites.clone(),
            ..Default::default()
        })
        .collect();
    Some(s)
}
