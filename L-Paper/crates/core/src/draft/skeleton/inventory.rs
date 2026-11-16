//! 素材清單: what the paper could present, from what the earlier steps already produced (no model):
//!
//! - the figure plan ([`crate::draft::figures`]): figures and tables (a `flow` tool = a flow item);
//! - results tables built from the record that no planned table stands for;
//! - experiments with a result, and the author's own thoughts: findings;
//! - numbered equations of the notation registry (lore-notation): equations.
//!
//! Algorithms come only from the author (`add`). Evidence ids are checked against the record;
//! unknown ones are dropped. Once the author chose (POST /inventory) the list is theirs: a fresh
//! list only adds what is new, matched by where it came from.

use super::*;
use crate::draft::candidates::known_ids;
use crate::draft::figures::Status;
use crate::draft::session::Session;
use std::collections::BTreeSet;

pub const KINDS: &[&str] = &["finding", "table", "figure", "flow", "equation", "algorithm"];

/// The journal's habit: figures, tables and numbered equations to aim for.
pub fn targets(p: &crate::venueprofile::Profile) -> Targets {
    let (figure, table) = p.float_targets();
    let s = &p.equations;
    let equation = [((s.mean - 2.0).floor().max(s.min)).max(0.0) as usize, s.mean.ceil() as usize];
    Targets { figure, table, equation }
}

fn slug(s: &str) -> String {
    let t: String = s.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    t.split('-').filter(|x| !x.is_empty()).take(5).collect::<Vec<_>>().join("-")
}

fn prefix(kind: &str) -> &'static str {
    match kind {
        "table" => "tab",
        "equation" => "eq",
        "algorithm" => "alg",
        "finding" => "",
        _ => "fig",
    }
}

fn tool_for(kind: &str) -> &'static str {
    match kind {
        "table" => "table",
        "flow" => "flow",
        "equation" => "equation",
        "algorithm" => "algorithm",
        "figure" => "image",
        _ => "",
    }
}

/// A free label for `title` of `kind` (`fallback` when the title has no ASCII words).
fn label_for(kind: &str, title: &str, fallback: &str, taken: &BTreeSet<String>) -> String {
    let p = prefix(kind);
    if p.is_empty() {
        return String::new();
    }
    let s = slug(title);
    let base = format!("{p}:{}", if s.is_empty() { fallback.to_lowercase() } else { s });
    let mut label = base.clone();
    let mut k = 2;
    while taken.contains(&label) {
        label = format!("{base}-{k}");
        k += 1;
    }
    label
}

/// The list the session gives now, each with where it came from. `equations` = the registry's
/// numbered equations (label, what it is).
pub fn fresh(sess: &Session, equations: &[(String, String)]) -> Vec<(String, Inv)> {
    let known = known_ids(sess);
    let pending_ids: BTreeSet<String> = sess.pending.iter().map(|p| p.id.clone()).collect();
    let ev = |v: &[String]| -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for e in v.iter().map(|e| e.trim().to_string()) {
            if known.contains(&e) && !out.contains(&e) {
                out.push(e);
            }
        }
        out
    };
    let mut out: Vec<(String, Inv)> = vec![];
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut covered_tables: BTreeSet<String> = BTreeSet::new();
    for f in sess.figures.iter().flat_map(|p| p.items.iter()) {
        let kind = if f.kind == "table" { "table" } else if f.tool == "flow" { "flow" } else { "figure" };
        let evidence = ev(&f.evidence);
        if kind == "table" {
            covered_tables.extend(evidence.iter().filter(|e| e.starts_with('R')).cloned());
        }
        taken.insert(f.label.clone());
        let tool = if crate::draft::figures::TOOLS.contains(&f.tool.as_str()) { f.tool.clone() } else { tool_for(kind).into() };
        let pending = !f.pending.is_empty() || evidence.iter().any(|e| pending_ids.contains(e));
        out.push((
            format!("fig:{}", f.id),
            Inv { id: String::new(), kind: kind.into(), title: f.title.clone(), note: f.supports.clone(), evidence, label: f.label.clone(), keep: f.status != Status::Removed, by_author: false, pending, tool },
        ));
    }
    for t in &sess.tables {
        if t.id.is_empty() || covered_tables.contains(&t.id) {
            continue;
        }
        let label = label_for("table", &t.caption, &t.id, &taken);
        taken.insert(label.clone());
        let mut evidence = ev(&[t.id.clone()]);
        evidence.extend(ev(&t.from).into_iter().filter(|e| !evidence.contains(e)).collect::<Vec<_>>());
        let pending = sess.pending.iter().any(|p| p.table.as_deref() == Some(t.id.as_str()));
        out.push((format!("tab:{}", t.id), Inv { id: String::new(), kind: "table".into(), title: t.caption.clone(), note: t.note.clone(), evidence, label, keep: true, by_author: false, pending, tool: "table".into() }));
    }
    for e in &sess.experiments {
        if e.result.trim().is_empty() {
            continue;
        }
        let title = if e.title.trim().is_empty() { e.change.clone() } else { e.title.clone() };
        let pending = sess.pending.iter().any(|p| p.experiment.as_deref() == Some(e.id.as_str()));
        out.push((format!("exp:{}", e.id), Inv { id: String::new(), kind: "finding".into(), title, note: e.result.clone(), evidence: ev(&[e.id.clone()]), label: String::new(), keep: true, by_author: false, pending, tool: String::new() }));
    }
    for h in &sess.thoughts {
        if h.text.trim().is_empty() || h.id.is_empty() {
            continue;
        }
        let mut evidence = ev(&[h.id.clone()]);
        evidence.extend(ev(&h.about).into_iter().filter(|e| !evidence.contains(e)).collect::<Vec<_>>());
        out.push((format!("thought:{}", h.id), Inv { id: String::new(), kind: "finding".into(), title: h.text.clone(), note: String::new(), evidence, label: String::new(), keep: true, by_author: false, pending: false, tool: String::new() }));
    }
    for (label, title) in equations {
        if label.trim().is_empty() || taken.contains(label) {
            continue;
        }
        taken.insert(label.clone());
        out.push((format!("eq:{label}"), Inv { id: String::new(), kind: "equation".into(), title: if title.trim().is_empty() { label.clone() } else { title.clone() }, note: String::new(), evidence: vec![], label: label.clone(), keep: true, by_author: false, pending: false, tool: "equation".into() }));
    }
    out
}

/// Bring a fresh list into the stored one. Before the author chose, the fresh list replaces it
/// (ids kept for items from the same source); afterwards only new items come in (their keep as
/// proposed), the author's items and choices stay, and items whose source is gone leave.
pub fn merge(sk: &mut Skeleton, fresh: Vec<(String, Inv)>) {
    let by_src: BTreeMap<String, String> = sk.inv_src.iter().map(|(id, src)| (src.clone(), id.clone())).collect();
    let srcs: BTreeSet<String> = fresh.iter().map(|(s, _)| s.clone()).collect();
    let mut kept: Vec<Inv> = vec![];
    let mut src_of: BTreeMap<String, String> = BTreeMap::new();
    for it in &sk.inventory {
        match sk.inv_src.get(&it.id) {
            Some(src) if srcs.contains(src) => {}
            Some(_) => continue,
            None => {}
        }
        if it.by_author || sk.inventory_set {
            if let Some(src) = sk.inv_src.get(&it.id) {
                src_of.insert(it.id.clone(), src.clone());
            }
            kept.push(it.clone());
        }
    }
    for (src, mut it) in fresh {
        match by_src.get(&src) {
            Some(id) if kept.iter().any(|k| &k.id == id) => {
                // the author's choice stays; what the record says is refreshed
                if let Some(k) = kept.iter_mut().find(|k| &k.id == id) {
                    let keep = k.keep;
                    *k = Inv { id: id.clone(), keep, ..it };
                }
            }
            Some(id) => {
                it.id = id.clone();
                src_of.insert(id.clone(), src);
                kept.push(it);
            }
            None => {
                it.id = sk.new_id("v");
                src_of.insert(it.id.clone(), src);
                kept.push(it);
            }
        }
    }
    sk.inventory = kept;
    sk.inv_src = src_of;
}

/// One thing the author adds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Added {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
}

/// POST /inventory: `keep` = every kept id; `add` = the author's own items (kept; a figure, table,
/// flow, equation or algorithm of the author's waits for them to make it: `pending`).
pub fn choose(sk: &mut Skeleton, keep: &[String], add: Vec<Added>) -> Result<(), String> {
    if let Some(a) = add.iter().find(|a| !KINDS.contains(&a.kind.trim().to_lowercase().as_str())) {
        return Err(format!("沒有這種素材：{}", a.kind));
    }
    for it in sk.inventory.iter_mut() {
        it.keep = keep.contains(&it.id);
    }
    let mut taken: BTreeSet<String> = sk.inventory.iter().map(|i| i.label.clone()).collect();
    for a in add {
        let kind = a.kind.trim().to_lowercase();
        if a.title.trim().is_empty() {
            continue;
        }
        let id = sk.new_id("v");
        let label = label_for(&kind, &a.title, &id, &taken);
        taken.insert(label.clone());
        sk.inventory.push(Inv { id, kind: kind.clone(), title: a.title.trim().to_string(), note: a.note.trim().to_string(), evidence: vec![], label, keep: true, by_author: true, pending: kind != "finding", tool: tool_for(&kind).into() });
    }
    sk.inventory_set = true;
    sk.rev += 1;
    Ok(())
}

/// The kept items as prompt text for the planner.
pub fn block(sk: &Skeleton) -> String {
    let kept: Vec<&Inv> = sk.inventory.iter().filter(|i| i.keep).collect();
    if kept.is_empty() {
        return String::new();
    }
    let mut s = "\n## What the paper presents (the author's list; a paragraph's `floats` may only name these labels)\n".to_string();
    for i in kept {
        s += &format!("- [{}] {}{}", i.kind, if i.label.is_empty() { String::new() } else { format!("{} ", i.label) }, i.title);
        if !i.note.is_empty() {
            s += &format!(" — {}", i.note);
        }
        if !i.evidence.is_empty() {
            s += &format!(" (record: {})", i.evidence.join(", "));
        }
        if i.pending {
            s += " (not made yet)";
        }
        s += "\n";
    }
    s
}
