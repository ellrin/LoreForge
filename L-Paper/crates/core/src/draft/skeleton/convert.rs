//! An old session → a skeleton (改用骨架). The caller archives a copy first
//! ([`crate::draft::session::snapshot`]).
//!
//! - outline sections → sections (role from the name);
//! - outline paragraphs → paragraphs (same ids), grouped into blocks by the question group their
//!   sources come from (a group's paragraphs in a row = one block: the group's title and purpose);
//! - drafts → `text` (with their problems);
//! - each group's interview → one done thread on the group's first paragraph, the group's
//!   summary as its conclusion.
//!
//! A session with no outline yet (mid-interview) cannot be converted here: the planner lays the
//! skeleton out afresh with the answers given so far as material ([`answers_block`]), then the
//! answers are attached as threads ([`attach_answers`]).

use super::*;
use crate::draft::session::{Group, OutlinePara, Session, Turn, Who};

/// Does the session hold something `/convert` can turn into a skeleton?
pub fn can_convert(sess: &Session) -> bool {
    sess.skeleton.as_ref().is_none_or(|k| !k.exists()) && (!sess.outline.is_empty() || !sess.drafts.is_empty() || sess.groups.iter().any(|g| g.turns > 0))
}

/// The group a paragraph's sources point at (a group id, an item's group, a claim's items' group).
fn group_of<'a>(sess: &'a Session, p: &OutlinePara) -> Option<&'a Group> {
    p.sources.iter().find_map(|src| {
        if let Some(g) = sess.group(src) {
            return Some(g);
        }
        if let Some(i) = sess.items.iter().find(|i| &i.id == src) {
            return sess.group(&i.group);
        }
        let c = sess.claims.iter().find(|c| &c.id == src)?;
        sess.items.iter().find(|i| c.items.contains(&i.id)).and_then(|i| sess.group(&i.group))
    })
}

/// The groups' conversations as threads (no anchor yet): groups with at least one answer, in
/// plan order; messages in order, the first question as the question, the summary's conclusion
/// (else its explanation) as the conclusion.
pub fn group_threads(sess: &Session, turns: &[Turn]) -> Vec<(String, Thread)> {
    let mut out = vec![];
    for g in &sess.groups {
        let mine: Vec<&Turn> = turns.iter().filter(|t| t.group == g.id).collect();
        if !mine.iter().any(|t| t.who == Who::Me) {
            continue;
        }
        let messages: Vec<Msg> = mine.iter().map(|t| Msg { who: if t.who == Who::Me { "me".into() } else { "ai".into() }, text: t.text.trim().to_string() }).collect();
        let question = mine.iter().find(|t| t.who == Who::Ai).map(|t| t.text.trim().to_string()).unwrap_or_else(|| g.title.clone());
        let conclusion = [&g.summary.conclusion, &g.summary.explain].into_iter().map(|s| s.trim()).find(|s| !s.is_empty()).unwrap_or("").to_string();
        out.push((g.id.clone(), Thread { id: String::new(), anchor: String::new(), kind: "note".into(), status: "done".into(), question, options: vec![], messages, conclusion, changes: vec![] }));
    }
    out
}

/// The answers given so far, for the planner when the session has no outline yet: each group's
/// conversation under the group id (the planner names the groups a paragraph uses in `answers`).
pub fn answers_block(sess: &Session, turns: &[Turn]) -> String {
    let threads = group_threads(sess, turns);
    if threads.is_empty() {
        return String::new();
    }
    let mut s = "\n## What the author already answered in the interview (use it; never ask it again)\n".to_string();
    for (gid, t) in threads {
        let g = sess.group(&gid);
        s += &format!("### {gid} [{}] {}\n", g.map(|g| g.section.as_str()).unwrap_or(""), g.map(|g| g.title.as_str()).unwrap_or(""));
        for m in &t.messages {
            s += &format!("{}: {}\n", if m.who == "me" { "Author" } else { "Question" }, m.text);
        }
        if !t.conclusion.is_empty() {
            s += &format!("Summary: {}\n", t.conclusion);
        }
    }
    s
}

/// Hang the groups' conversations on the paragraphs: `uses` = paragraph id → groups it uses (from
/// the planner); else the first paragraph of a section with the group's role; else the first one.
pub fn attach_answers(sk: &mut Skeleton, sess: &Session, turns: &[Turn], uses: &BTreeMap<String, Vec<String>>) {
    for (gid, mut t) in group_threads(sess, turns) {
        let by_plan = sk.order().into_iter().find(|p| uses.get(p).is_some_and(|g| g.contains(&gid)));
        let section = sess.group(&gid).map(|g| g.section.clone()).unwrap_or_default();
        let by_section = sk.sections.iter().find(|s| super::old_section(s) == section).and_then(|s| s.blocks.iter().flat_map(|b| b.paras.iter()).next()).map(|p| p.id.clone());
        let Some(anchor) = by_plan.or(by_section).or_else(|| sk.order().first().cloned()) else { continue };
        t.anchor = anchor;
        sk.add_thread(t);
    }
}

/// The whole conversion for a session with an outline (None without one).
pub fn from_session(sess: &Session, turns: &[Turn]) -> Option<Skeleton> {
    if sess.outline.is_empty() {
        return None;
    }
    let mut sk = Skeleton::default();
    let draft_of = |id: &str| sess.drafts.iter().flat_map(|d| d.paras.iter()).find(|d| d.para == id);
    // the outline's ids where they are p-numbers and unique; others get new ones afterwards
    let mut seen: Vec<String> = vec![];
    // group id → the paragraph its conversation hangs on
    let mut uses: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for o in &sess.outline {
        let si = match sk.sections.iter().position(|s| s.title == o.section) {
            Some(i) => i,
            None => {
                let id = sk.new_id("s");
                sk.sections.push(Section { id, title: o.section.clone(), role: role_of(&o.section), blocks: vec![] });
                sk.sections.len() - 1
            }
        };
        let mut current: Option<(String, Option<String>)> = None; // (block id, group id)
        for p in &o.paras {
            let g = group_of(sess, p);
            let gid = g.map(|g| g.id.clone());
            let new_block = match &current {
                None => true,
                Some((_, cur)) => gid.is_some() && *cur != gid,
            };
            if new_block {
                let id = format!("b{}", sk.sections.iter().map(|s| s.blocks.len()).sum::<usize>() + 1);
                let (title, purpose) = match g {
                    Some(g) => (g.title.clone(), g.purpose.clone()),
                    None => (o.subsection.clone().unwrap_or_default(), String::new()),
                };
                sk.sections[si].blocks.push(Block { id: id.clone(), title, purpose, of: None, paras: vec![] });
                current = Some((id, gid.clone()));
            }
            let pid = if p.id.strip_prefix('p').is_some_and(|n| n.parse::<u64>().is_ok()) && !seen.contains(&p.id) { p.id.clone() } else { String::new() };
            seen.push(pid.clone());
            let d = draft_of(&p.id);
            let mut para = Para {
                id: pid,
                kind: p.kind.clone(),
                say: p.zh.trim().to_string(),
                points: p.en.iter().filter(|e| !e.trim().is_empty()).map(|e| Point { text: e.trim().to_string(), from: None }).collect(),
                floats: p.floats.clone(),
                cites: p.cites.clone(),
                sources: p.sources.clone(),
                pending: p.pending.clone(),
                text: d.map(|d| d.text.clone()).unwrap_or_default(),
                problems: d.map(|d| d.problems.clone()).unwrap_or_default(),
                ..Default::default()
            };
            para.settle_state();
            if let Some(g) = &gid {
                // the group's first paragraph carries its conversation
                if !uses.values().any(|v| v.contains(g)) {
                    uses.entry(para.id.clone()).or_default().push(g.clone());
                }
            }
            let bi = sk.sections[si].blocks.len() - 1;
            sk.sections[si].blocks[bi].paras.push(para);
        }
    }
    sk.sections.iter_mut().for_each(|s| s.blocks.retain(|b| !b.paras.is_empty()));
    sk.sync_counters();
    // paragraphs without a usable id
    let missing: Vec<(usize, usize, usize)> = sk
        .sections
        .iter()
        .enumerate()
        .flat_map(|(si, s)| s.blocks.iter().enumerate().flat_map(move |(bi, b)| b.paras.iter().enumerate().filter(|(_, p)| p.id.is_empty()).map(move |(pi, _)| (si, bi, pi))))
        .collect();
    for (si, bi, pi) in missing {
        let id = sk.new_id("p");
        sk.sections[si].blocks[bi].paras[pi].id = id;
    }
    // `uses` was keyed by the outline id; a paragraph that got a new id has none (falls back by section)
    attach_answers(&mut sk, sess, turns, &uses);
    sk.rev = 1;
    Some(sk)
}
