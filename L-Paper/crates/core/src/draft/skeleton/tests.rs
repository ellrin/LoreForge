//! Skeleton data, ops, undo and conversion on synthetic sessions.

use super::*;
use crate::draft::session::{self as ds, DraftPara, DraftSection, Group, GroupSummary, OutlinePara, OutlineSection, Session, Turn, Who};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/skeleton_view.json")).unwrap()
}

#[test]
fn skeleton_view_fixture_round_trips() {
    let v = fixture();
    let view: View = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(serde_json::to_value(&view).unwrap(), v, "every field, null and empty list as in the contract");
    // the View made from a stored skeleton is the same shape
    let sk = Skeleton { rev: view.rev, sections: view.sections.clone(), threads: view.threads.clone(), inventory: view.inventory.clone(), edits: 1, undo: vec![Undo::default()], ..Default::default() };
    let made = sk.view(false, view.targets.clone());
    assert_eq!(serde_json::to_value(&made).unwrap(), v);
}

fn para(id: &str, say: &str) -> Para {
    Para { id: id.into(), say: say.into(), ..Default::default() }
}

/// s1 [b1: p1 p2] [b2: p3], s2 [b3: p4 p5]
fn small() -> Skeleton {
    let mut sk = Skeleton {
        sections: vec![
            Section { id: "s1".into(), title: "Results".into(), role: "results".into(), blocks: vec![
                Block { id: "b1".into(), title: "A vs B".into(), purpose: "A wins".into(), of: None, paras: vec![para("p1", "A 對 B"), para("p2", "B 對 C")] },
                Block { id: "b2".into(), title: "外部".into(), purpose: "".into(), of: None, paras: vec![para("p3", "外部結果")] },
            ] },
            Section { id: "s2".into(), title: "Discussion".into(), role: "discussion".into(), blocks: vec![Block { id: "b3".into(), title: "為什麼".into(), purpose: "".into(), of: Some("b2".into()), paras: vec![para("p4", "解釋"), para("p5", "限制")] }] },
        ],
        ..Default::default()
    };
    sk.sync_counters();
    sk
}

#[test]
fn ids_are_never_reused() {
    let mut sk = small();
    let a = sk.apply(Op::AddPara { block: "b1".into(), at: Some(1), say: "新段".into(), reason: "補".into() }).unwrap().unwrap();
    assert_eq!(a, "p6");
    sk.apply(Op::Remove { id: a.clone() }).unwrap();
    let b = sk.apply(Op::AddPara { block: "b1".into(), at: None, say: "".into(), reason: "".into() }).unwrap().unwrap();
    assert_eq!(b, "p7", "a removed id is never given again");
    sk.apply(Op::Undo).unwrap();
    sk.apply(Op::Undo).unwrap();
    assert!(sk.para("p6").is_some(), "undo brings the removed paragraph back under its id");
    let c = sk.apply(Op::AddBlock { section: "s1".into(), at: Some(0), title: "新框".into(), of: Some("nope".into()) }).unwrap().unwrap();
    assert_eq!(c, "b4");
    assert_eq!(sk.block("b4").unwrap().of, None, "an unknown `of` is dropped");
}

#[test]
fn add_para_and_block() {
    let mut sk = small();
    let id = sk.apply(Op::AddPara { block: "b1".into(), at: Some(1), say: " 新段 ".into(), reason: "想先交代".into() }).unwrap().unwrap();
    assert_eq!(sk.order(), ["p1", id.as_str(), "p2", "p3", "p4", "p5"]);
    let p = sk.para(&id).unwrap();
    assert!(p.touched && p.say == "新段" && p.reason == "想先交代" && p.state == State::Outline);
    assert!(sk.dirty() && !sk.undo.is_empty());
    let b = sk.apply(Op::AddBlock { section: "s2".into(), at: None, title: "結尾".into(), of: Some("p4".into()) }).unwrap().unwrap();
    assert_eq!(sk.sections[1].blocks.last().unwrap().id, b);
    assert_eq!(sk.block(&b).unwrap().of.as_deref(), Some("p4"));
    assert!(sk.touched_blocks.contains(&b));
    assert!(sk.apply(Op::AddPara { block: "zz".into(), at: None, say: "".into(), reason: "".into() }).is_err());
    assert!(sk.apply(Op::AddBlock { section: "zz".into(), at: None, title: "".into(), of: None }).is_err());
}

#[test]
fn remove_marks_the_seam_and_clears_of() {
    let mut sk = small();
    sk.apply(Op::Remove { id: "p2".into() }).unwrap();
    assert_eq!(sk.order(), ["p1", "p3", "p4", "p5"]);
    assert!(sk.seams.contains(&"p1".to_string()) && sk.seams.contains(&"p3".to_string()), "{:?}", sk.seams);
    sk.apply(Op::Remove { id: "b2".into() }).unwrap();
    assert_eq!(sk.order(), ["p1", "p4", "p5"]);
    assert_eq!(sk.block("b3").unwrap().of, None, "the block it expanded is gone");
    assert!(sk.apply(Op::Remove { id: "nope".into() }).is_err());
}

#[test]
fn move_paragraph_and_block() {
    let mut sk = small();
    sk.apply(Op::Move { id: "p1".into(), to: "b3".into(), at: Some(1), reason: "移到討論".into() }).unwrap();
    assert_eq!(sk.order(), ["p2", "p3", "p4", "p1", "p5"]);
    let p = sk.para("p1").unwrap();
    assert!(p.touched && p.reason == "移到討論");
    assert!(sk.seams.contains(&"p2".to_string()), "the old neighbour is a seam");
    sk.apply(Op::Move { id: "b3".into(), to: "s1".into(), at: Some(0), reason: "".into() }).unwrap();
    assert_eq!(sk.sections[0].blocks[0].id, "b3");
    assert!(sk.sections[1].blocks.is_empty());
    assert!(sk.apply(Op::Move { id: "p2".into(), to: "s1".into(), at: None, reason: "".into() }).is_err(), "a paragraph goes into a block");
    assert!(sk.apply(Op::Move { id: "b1".into(), to: "b2".into(), at: None, reason: "".into() }).is_err(), "a block goes into a section");
}

#[test]
fn edit_reason_done() {
    let mut sk = small();
    let f = |v: serde_json::Value| v.as_object().unwrap().clone();
    sk.apply(Op::Edit { id: "p1".into(), fields: f(serde_json::json!({ "say": "A 比 B 好", "points": ["0.91 對 0.87", { "text": "五折一致", "from": "q1" }] })) }).unwrap();
    let p = sk.para("p1").unwrap();
    assert_eq!(p.say, "A 比 B 好");
    assert_eq!(p.points[1].from.as_deref(), Some("q1"));
    assert_eq!(p.state, State::Points);
    assert!(sk.apply(Op::Edit { id: "p1".into(), fields: f(serde_json::json!({ "link_next": "x" })) }).is_err(), "links are the repair's");
    assert!(sk.apply(Op::Done { id: "p1".into(), done: true }).is_err(), "no text yet");
    sk.apply(Op::Edit { id: "p1".into(), fields: f(serde_json::json!({ "text": "Method A was better." })) }).unwrap();
    assert_eq!(sk.para("p1").unwrap().state, State::Text);
    sk.apply(Op::Done { id: "p1".into(), done: true }).unwrap();
    assert_eq!(sk.para("p1").unwrap().state, State::Done);
    sk.apply(Op::Done { id: "p1".into(), done: false }).unwrap();
    assert_eq!(sk.para("p1").unwrap().state, State::Text);
    sk.apply(Op::Reason { id: "p2".into(), reason: "先講 B".into() }).unwrap();
    assert_eq!(sk.para("p2").unwrap().reason, "先講 B");
    sk.apply(Op::Edit { id: "b1".into(), fields: f(serde_json::json!({ "title": "主要比較", "purpose": "讀者知道 A 最好", "of": "p4" })) }).unwrap();
    let b = sk.block("b1").unwrap();
    assert_eq!((b.title.as_str(), b.purpose.as_str(), b.of.as_deref()), ("主要比較", "讀者知道 A 最好", Some("p4")));
    assert!(sk.apply(Op::Edit { id: "b1".into(), fields: f(serde_json::json!({ "say": "x" })) }).is_err());
    // a block's state is its lowest paragraph's
    assert_eq!(sk.block("b1").unwrap().state(), State::Outline);
}

#[test]
fn undo_restores_structure_but_keeps_what_was_written_meanwhile() {
    let mut sk = small();
    sk.apply(Op::Move { id: "p1".into(), to: "b2".into(), at: None, reason: "r".into() }).unwrap();
    // a fill lands on p1 after the move
    let p = sk.para_mut("p1").unwrap();
    p.points.push(Point { text: "來自回答".into(), from: Some("q1".into()) });
    p.settle_state();
    sk.apply(Op::Undo).unwrap();
    assert_eq!(sk.order(), ["p1", "p2", "p3", "p4", "p5"], "back where it was");
    let p = sk.para("p1").unwrap();
    assert_eq!(p.points.len(), 1, "the fill's points stay");
    assert!(!p.touched && p.reason.is_empty(), "the move's marks go with it");
    assert!(!sk.dirty());
    // an edit's undo restores only the fields it changed
    let f = |v: serde_json::Value| v.as_object().unwrap().clone();
    sk.apply(Op::Edit { id: "p2".into(), fields: f(serde_json::json!({ "say": "改過" })) }).unwrap();
    sk.para_mut("p2").unwrap().points.push(Point { text: "背景".into(), from: Some("q2".into()) });
    sk.apply(Op::Undo).unwrap();
    let p = sk.para("p2").unwrap();
    assert_eq!(p.say, "B 對 C");
    assert_eq!(p.points.len(), 1);
    assert_eq!(p.state, State::Points);
    assert!(sk.apply(Op::Undo).is_err(), "nothing left to undo");
}

#[test]
fn undo_is_bounded() {
    let mut sk = small();
    for k in 0..(UNDO_MAX + 10) {
        sk.apply(Op::Reason { id: "p1".into(), reason: format!("r{k}") }).unwrap();
    }
    assert_eq!(sk.undo.len(), UNDO_MAX);
}

#[test]
fn ops_in_order_stop_at_the_first_failure() {
    let mut sk = small();
    let rev = sk.rev;
    let r = sk.apply_all(vec![Op::AddPara { block: "b2".into(), at: None, say: "x".into(), reason: "".into() }, Op::Remove { id: "zz".into() }, Op::Remove { id: "p1".into() }]);
    assert!(r.is_err());
    assert!(sk.para("p6").is_some() && sk.para("p1").is_some(), "the first applied, the third never ran");
    assert_eq!(sk.rev, rev + 1);
    let ops: Vec<Op> = serde_json::from_value(serde_json::json!([
        { "op": "add_para", "block": "b4", "at": 2, "say": "", "reason": "" },
        { "op": "add_block", "section": "s3", "at": 1, "title": "", "of": null },
        { "op": "remove", "id": "p9" },
        { "op": "move", "id": "p9", "to": "b5", "at": 0, "reason": "" },
        { "op": "edit", "id": "p9", "fields": { "say": "…" } },
        { "op": "reason", "id": "p9", "reason": "…" },
        { "op": "done", "id": "p9", "done": true },
        { "op": "undo" }
    ]))
    .unwrap();
    assert_eq!(ops.len(), 8, "every op of the contract parses");
}

#[test]
fn repair_scope_is_the_touched_and_their_neighbours() {
    let mut sk = small();
    assert_eq!(sk.repair_scope(), (vec![], vec![]));
    sk.apply(Op::Reason { id: "p3".into(), reason: "x".into() }).unwrap();
    assert_eq!(sk.repair_scope().0, Vec::<String>::new(), "a reason alone is not an edit to repair");
    let f = |v: serde_json::Value| v.as_object().unwrap().clone();
    sk.apply(Op::Edit { id: "p3".into(), fields: f(serde_json::json!({ "say": "新的外部結果" })) }).unwrap();
    let (paras, blocks) = sk.repair_scope();
    assert_eq!(paras, ["p2", "p3", "p4"]);
    assert_eq!(blocks, ["b2"]);
    sk.clear_marks();
    assert!(!sk.dirty() && sk.repair_scope().0.is_empty());
}

#[test]
fn decide_reverts_and_applies_changes() {
    let mut sk = small();
    sk.para_mut("p1").unwrap().text = "Method A was better, which suggests a stable gain.".into();
    sk.para_mut("p2").unwrap().link_prev = "承接 A 對 B".into();
    let e = sk.add_thread(Thread { anchor: "p2".into(), kind: "edit".into(), status: "done".into(), changes: vec![Change { id: "c1".into(), target: "p2".into(), field: "link_prev".into(), before: "".into(), after: "承接 A 對 B".into(), why: "移動".into(), state: "applied".into() }], ..Default::default() });
    let r = sk.add_thread(Thread { anchor: "p1".into(), kind: "refine".into(), status: "open".into(), changes: vec![Change { id: "c1".into(), target: "p1".into(), field: "text".into(), before: "which suggests a stable gain".into(), after: "so the gain is stable".into(), why: "用字".into(), state: "proposed".into() }], ..Default::default() });
    assert_eq!((e.as_str(), r.as_str()), ("e1", "r1"));
    sk.decide(&e, "c1", false).unwrap();
    assert_eq!(sk.para("p2").unwrap().link_prev, "");
    assert_eq!(sk.thread(&e).unwrap().changes[0].state, "reverted");
    sk.decide(&e, "c1", true).unwrap();
    assert_eq!(sk.para("p2").unwrap().link_prev, "承接 A 對 B");
    // the text moved on: the proposal no longer fits
    sk.para_mut("p1").unwrap().text = "Method A was better.".into();
    assert!(sk.decide(&r, "c1", true).unwrap_err().contains("原句不在"));
    sk.para_mut("p1").unwrap().text = "Method A was better, which suggests a stable gain.".into();
    sk.decide(&r, "c1", true).unwrap();
    assert_eq!(sk.para("p1").unwrap().text, "Method A was better, so the gain is stable.");
    assert_eq!(sk.thread(&r).unwrap().status, "done", "every proposal decided");
}

#[test]
fn next_is_the_first_open_question() {
    let mut sk = small();
    for (k, status) in ["done", "answered", "open", "open"].iter().enumerate() {
        sk.add_thread(Thread { anchor: "p1".into(), kind: "choice".into(), status: (*status).into(), question: format!("q{k}"), ..Default::default() });
    }
    assert_eq!(sk.next().as_deref(), Some("q3"));
    assert_eq!(options(vec!["a".into(), " ".into(), "b".into()]).iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["o1", "o2"]);
}

fn old_session() -> (Session, Vec<Turn>) {
    let mut s = Session::default();
    s.groups = vec![
        Group { id: "g1".into(), section: "Methods".into(), title: "損失函數".into(), purpose: "為什麼這樣設計".into(), turns: 1, summary: GroupSummary { conclusion: "加權處理模糊影格".into(), ..Default::default() }, ..Default::default() },
        Group { id: "g2".into(), section: "Results".into(), title: "主要比較".into(), purpose: "A 對 B".into(), turns: 1, ..Default::default() },
        Group { id: "g3".into(), section: "Results".into(), title: "外部".into(), turns: 0, ..Default::default() },
    ];
    s.items = vec![ds::Item { id: "i1".into(), group: "g2".into(), label: "A 對 B".into(), ..Default::default() }];
    let op = |id: &str, zh: &str, src: &[&str]| OutlinePara { id: id.into(), kind: "Results key result".into(), zh: zh.into(), en: vec!["point one".into()], sources: src.iter().map(|x| x.to_string()).collect(), floats: vec!["tab:main".into()], ..Default::default() };
    s.outline = vec![
        OutlineSection { section: "Methods".into(), subsection: None, paras: vec![op("p1", "損失", &["g1"])], confirmed: true },
        OutlineSection { section: "Results".into(), subsection: Some("Main".into()), paras: vec![op("p2", "A 對 B", &["i1"]), op("p3", "B 對 C", &["E1"]), op("p4", "外部", &["g3"])], confirmed: true },
    ];
    s.drafts = vec![DraftSection { section: "Results".into(), paras: vec![DraftPara { para: "p2".into(), text: "Method A was better.".into(), problems: vec!["x".into()], pending: vec![] }], ..Default::default() }];
    let t = |n: usize, g: &str, who: Who, text: &str| Turn { n, group: g.into(), who, text: text.into(), options: vec![], engine: String::new(), about: String::new(), picked: vec![], cands: None };
    let turns = vec![t(1, "g1", Who::Ai, "為什麼加權？"), t(2, "g1", Who::Me, "模糊影格資訊很多"), t(3, "g2", Who::Ai, "A 為什麼好？"), t(4, "g2", Who::Me, "先驗較強"), t(5, "g3", Who::Ai, "外部呢？")];
    (s, turns)
}

#[test]
fn an_old_session_converts() {
    let (s, turns) = old_session();
    assert!(convert::can_convert(&s));
    let sk = convert::from_session(&s, &turns).unwrap();
    assert_eq!(sk.sections.iter().map(|x| (x.title.as_str(), x.role.as_str())).collect::<Vec<_>>(), [("Methods", "methods"), ("Results", "results")]);
    // Results: g2's paragraphs (p2 and p3, which follows without a group of its own), then g3's
    let res = &sk.sections[1];
    assert_eq!(res.blocks.iter().map(|b| (b.title.as_str(), b.paras.iter().map(|p| p.id.as_str()).collect::<Vec<_>>())).collect::<Vec<_>>(), [("主要比較", vec!["p2", "p3"]), ("外部", vec!["p4"])]);
    let p2 = sk.para("p2").unwrap();
    assert_eq!((p2.say.as_str(), p2.text.as_str(), p2.state), ("A 對 B", "Method A was better.", State::Text));
    assert_eq!(p2.problems, ["x"]);
    assert_eq!(sk.para("p3").unwrap().state, State::Points, "points from the outline's English lines");
    // answered groups → done threads on their paragraph, summary as conclusion; g3 had no answer
    assert_eq!(sk.threads.iter().map(|t| (t.id.as_str(), t.anchor.as_str(), t.status.as_str())).collect::<Vec<_>>(), [("n1", "p1", "done"), ("n2", "p2", "done")]);
    assert_eq!(sk.threads[0].conclusion, "加權處理模糊影格");
    assert_eq!(sk.threads[1].messages.iter().map(|m| m.who.as_str()).collect::<Vec<_>>(), ["ai", "me"]);
    // new ids continue after the converted ones
    let mut sk = sk;
    assert_eq!(sk.new_id("p"), "p5");
    assert_eq!(sk.new_id("b"), "b4");
    assert!(sk.next().is_none(), "nothing open after a conversion");
}

#[test]
fn a_session_without_an_outline_is_planned_then_the_answers_attached() {
    let (mut s, turns) = old_session();
    s.outline.clear();
    s.drafts.clear();
    assert!(convert::can_convert(&s));
    assert!(convert::from_session(&s, &turns).is_none());
    let block = convert::answers_block(&s, &turns);
    assert!(block.contains("### g1 [Methods] 損失函數") && block.contains("Author: 先驗較強") && !block.contains("### g3"), "{block}");
    let mut sk = small();
    let mut uses = BTreeMap::new();
    uses.insert("p3".to_string(), vec!["g2".to_string()]);
    convert::attach_answers(&mut sk, &s, &turns, &uses);
    // g1 (Methods) has no Methods section here: the first paragraph; g2 where the planner put it
    assert_eq!(sk.threads.iter().map(|t| t.anchor.as_str()).collect::<Vec<_>>(), ["p1", "p3"]);
    // an empty session has nothing to convert
    assert!(!convert::can_convert(&Session::default()));
}

#[test]
fn old_sessions_load_and_write_no_skeleton() {
    let s: Session = serde_json::from_str(r#"{"phase":"outline","outline":[{"section":"Methods","paras":[{"id":"p1","zh":"段落"}]}]}"#).unwrap();
    assert!(s.skeleton.is_none());
    assert!(!serde_json::to_string(&s).unwrap().contains("skeleton"));
    let mut s = s;
    s.skeleton = Some(small());
    let back: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back.skeleton, s.skeleton);
}

#[test]
fn roles_map_to_the_old_sections() {
    assert_eq!(role_of("Materials and Methods"), "methods");
    assert_eq!(role_of("Related Work"), "related");
    assert_eq!(role_of("Conclusions"), "conclusion");
    let sec = |title: &str, role: &str| Section { id: "s".into(), title: title.into(), role: role.into(), blocks: vec![] };
    assert_eq!(old_section(&sec("Related work", "related")), "Introduction");
    assert_eq!(old_section(&sec("Appendix", "other")), "Discussion");
}

#[test]
fn a_note_said_on_a_paragraph_binds_that_paragraph() {
    use crate::draft::notes;
    let mut s = Session::default();
    notes::add_in(&mut s, "p9", "Results", vec![notes::NoteUpdate { text: "不要用 novel".into(), scope: "paragraph".into(), banned: vec!["novel".into()] }], false);
    let p = |id: &str| OutlinePara { id: id.into(), ..Default::default() };
    let mine = notes::for_paragraph(&s, "Results", &p("p9"));
    assert_eq!(mine.len(), 1);
    assert_eq!(notes::banned_in(&mine, "A novel loss."), ["novel"]);
    assert!(notes::for_paragraph(&s, "Results", &p("p8")).is_empty());
    assert_eq!(s.writing_notes[0].section, "Results");
}

#[test]
fn removing_a_paragraph_drops_its_open_questions_and_keeps_the_rest_on_the_block() {
    let mut sk = small();
    let open = sk.add_thread(Thread { anchor: "p2".into(), kind: "missing".into(), status: "open".into(), question: "為什麼？".into(), ..Default::default() });
    let done = sk.add_thread(Thread { anchor: "p2".into(), kind: "choice".into(), status: "done".into(), question: "哪個？".into(), conclusion: "A".into(), ..Default::default() });
    let other = sk.add_thread(Thread { anchor: "p4".into(), kind: "choice".into(), status: "open".into(), question: "x".into(), ..Default::default() });
    sk.apply(Op::Remove { id: "p2".into() }).unwrap();
    assert!(sk.thread(&open).is_none());
    assert_eq!(sk.thread(&done).unwrap().anchor, "b1");
    assert_eq!(sk.thread(&other).unwrap().anchor, "p4");
    sk.apply(Op::Undo).unwrap();
    assert_eq!(sk.thread(&open).unwrap().anchor, "p2");
    assert_eq!(sk.thread(&done).unwrap().anchor, "p2");
    // a whole block: finished threads go to the section
    sk.apply(Op::Remove { id: "b3".into() }).unwrap();
    assert!(sk.thread(&other).is_none());
    sk.thread_mut(&done).unwrap().anchor = "p5".into();
    sk.apply(Op::Undo).unwrap();
    assert!(sk.thread(&other).is_some());
}

fn recorded() -> Session {
    use crate::draft::figures::{FigurePlan, PlannedFloat, Status as FS};
    use crate::draft::session::{Experiment, ResultTable, Thought};
    let mut s = Session::default();
    s.experiments = vec![
        Experiment { id: "E1".into(), title: "Method A vs Method B".into(), result: "0.91 vs 0.87".into(), ..Default::default() },
        Experiment { id: "E2".into(), title: "no result yet".into(), ..Default::default() },
    ];
    s.thoughts = vec![Thought { id: "H1".into(), text: "A 的先驗較強".into(), about: vec!["E1".into(), "E9".into()] }];
    s.tables = vec![
        ResultTable { id: "R1".into(), caption: "Main comparison".into(), from: vec!["E1".into()], ..Default::default() },
        ResultTable { id: "R2".into(), caption: "External set".into(), from: vec!["E1".into()], ..Default::default() },
    ];
    s.figures = Some(FigurePlan {
        items: vec![
            PlannedFloat { id: "T1".into(), kind: "table".into(), label: "tab:main".into(), title: "主要比較".into(), supports: "A 較好".into(), evidence: vec!["R1".into(), "X7".into()], tool: "table".into(), status: FS::Kept, ..Default::default() },
            PlannedFloat { id: "F1".into(), kind: "figure".into(), label: "fig:overview".into(), title: "整體流程".into(), tool: "flow".into(), status: FS::Proposed, ..Default::default() },
            PlannedFloat { id: "F2".into(), kind: "figure".into(), label: "fig:cut".into(), title: "刪掉的".into(), tool: "plot".into(), status: FS::Removed, ..Default::default() },
        ],
        ..Default::default()
    });
    s
}

#[test]
fn inventory_comes_from_the_record_and_the_figure_plan() {
    let s = recorded();
    let fresh = inventory::fresh(&s, &[("eq:loss".into(), "損失函數".into())]);
    let kinds: Vec<(&str, &str, &str, bool)> = fresh.iter().map(|(src, i)| (src.as_str(), i.kind.as_str(), i.label.as_str(), i.keep)).collect();
    assert_eq!(
        kinds,
        [
            ("fig:T1", "table", "tab:main", true),
            ("fig:F1", "flow", "fig:overview", true),
            ("fig:F2", "figure", "fig:cut", false),
            ("tab:R2", "table", "tab:external-set", true),
            ("exp:E1", "finding", "", true),
            ("thought:H1", "finding", "", true),
            ("eq:eq:loss", "equation", "eq:loss", true),
        ]
    );
    assert_eq!(fresh[0].1.evidence, ["R1"], "an id the record does not have is dropped");
    assert_eq!(fresh[5].1.evidence, ["H1", "E1"]);
    let mut sk = Skeleton::default();
    inventory::merge(&mut sk, fresh.clone());
    assert_eq!(sk.inventory.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["v1", "v2", "v3", "v4", "v5", "v6", "v7"]);
    // the same list again: same ids
    inventory::merge(&mut sk, fresh.clone());
    assert_eq!(sk.inventory.len(), 7);
    assert_eq!(sk.inventory[6].id, "v7");
    // the author chooses, adds an algorithm; a later record change adds one item and drops one
    inventory::choose(&mut sk, &["v1".into(), "v5".into()], vec![inventory::Added { kind: "algorithm".into(), title: "Training loop".into(), note: "".into() }, inventory::Added { kind: "figure".into(), title: "失敗案例".into(), note: "作者新增".into() }]).unwrap();
    let alg = sk.inventory.iter().find(|i| i.kind == "algorithm").unwrap();
    assert_eq!((alg.id.as_str(), alg.label.as_str(), alg.tool.as_str(), alg.by_author, alg.pending, alg.keep), ("v8", "alg:training-loop", "algorithm", true, true, true));
    assert_eq!(sk.inventory.iter().find(|i| i.id == "v9").unwrap().label, "fig:v9", "a Chinese title: the id as the label");
    assert!(inventory::choose(&mut sk, &[], vec![inventory::Added { kind: "poem".into(), title: "x".into(), note: "".into() }]).is_err());
    let mut s2 = s.clone();
    s2.thoughts.clear();
    s2.experiments.push(crate::draft::session::Experiment { id: "E3".into(), title: "Ablation".into(), result: "-0.02".into(), ..Default::default() });
    let keep_before: Vec<(String, bool)> = sk.inventory.iter().map(|i| (i.id.clone(), i.keep)).collect();
    inventory::merge(&mut sk, inventory::fresh(&s2, &[("eq:loss".into(), "損失函數".into())]));
    assert!(sk.inventory.iter().all(|i| i.id != "v6"), "the thought is gone from the record");
    let new = sk.inventory.iter().find(|i| i.title == "Ablation").unwrap();
    assert_eq!(new.id, "v10");
    for (id, keep) in keep_before.iter().filter(|(id, _)| id != "v6") {
        assert_eq!(sk.inventory.iter().find(|i| &i.id == id).unwrap().keep, *keep, "{id}: the author's choice stays");
    }
    let b = inventory::block(&sk);
    assert!(b.contains("tab:main 主要比較") && !b.contains("tab:external-set"), "{b}");
}

#[test]
fn targets_follow_the_journal() {
    let p = crate::venueprofile::Profile { figure_target: Some([4, 7]), table_target: Some([2, 4]), equations: crate::venueprofile::Stat { mean: 5.2, min: 3.0, max: 10.0 }, ..Default::default() };
    assert_eq!(inventory::targets(&p), Targets { figure: [4, 7], table: [2, 4], equation: [3, 6] });
}

fn known() -> plan::Known {
    let set = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<std::collections::BTreeSet<String>>();
    plan::Known { labels: set(&["tab:main", "fig:size"]), cites: set(&["lee2020"]), sources: set(&["E1", "E2", "R1", "H1"]), pending: set(&["P1"]), groups: set(&["g1"]) }
}

const PLAN_REPLY: &str = r#"Here it is:
{"sections":[
 {"title":"Results","role":"results","blocks":[
   {"key":"b1","title":"解釋表 1","purpose":"A 最好","of":"","paras":[
     {"key":"x1","kind":"Results key result","say":"A 對 B","points":["A 0.91，B 0.87"],"link_prev":"","link_next":"接 B 對 C","floats":["tab:main","fig:invented"],"cites":["lee2020","made2099"],"sources":["E1","E77"],"pending":["P1","P9"],"answers":["g1","g7"]},
     {"key":"x2","kind":"","say":"B 對 C","points":[],"link_prev":"承接","link_next":"","floats":[],"cites":[],"sources":["R1"],"pending":[],"answers":[]}]},
   {"key":"b2","title":"空的","purpose":"","of":"","paras":[{"key":"x9","say":"  ","points":[]}]}]},
 {"title":"Introduction","role":"introduction","blocks":[
   {"key":"b3","title":"缺口","purpose":"讀者接受小資料是常態","of":"x1","paras":[
     {"key":"x3","kind":"Intro turn","say":"小資料是常態","points":[],"link_prev":"","link_next":"","floats":[],"cites":[],"sources":[],"pending":[],"answers":[]}]}]},
 {"title":"Nothing","role":"other","blocks":[]}],
 "questions":[
  {"para":"x2","kind":"missing","question":"turn 9 你說過 E1 的原因，B 和 C 差在哪？","options":["資料不同","標註不同","還不確定","第四個"]},
  {"para":"x1","kind":"opinion","question":"喜歡嗎？","options":[]},
  {"para":"x77","kind":"choice","question":"沒有這一段","options":[]},
  {"para":"b1","kind":"choice","question":"框不能掛問題","options":[]},
  {"para":"x1","kind":"conflict","question":"記錄寫 A 最好，但外部反轉，你要怎麼說？","options":["如實寫","放到限制"]}]}"#;

#[test]
fn a_plan_is_checked_by_code() {
    let mut sk = Skeleton::default();
    sk.new_id("v");
    let p = plan::parse(PLAN_REPLY, &known(), &mut sk).unwrap();
    // paper order whatever the model returned; empty blocks and sections gone
    assert_eq!(p.sections.iter().map(|s| (s.id.as_str(), s.title.as_str())).collect::<Vec<_>>(), [("s2", "Introduction"), ("s1", "Results")]);
    assert_eq!(p.sections[1].blocks.len(), 1);
    let x1 = &p.sections[1].blocks[0].paras[0];
    assert_eq!(x1.id, "p1");
    assert_eq!((x1.floats.clone(), x1.cites.clone(), x1.sources.clone(), x1.pending.clone()), (vec!["tab:main".to_string()], vec!["lee2020".to_string()], vec!["E1".to_string()], vec!["P1".to_string()]));
    assert_eq!(x1.points, [Point { text: "A 0.91，B 0.87".into(), from: None }]);
    assert_eq!(x1.state, State::Points, "a paragraph with points starts in points");
    assert_eq!(p.sections[1].blocks[0].paras[1].state, State::Outline);
    assert_eq!(p.uses.get("p1").unwrap(), &vec!["g1".to_string()]);
    assert_eq!(p.sections[0].blocks[0].of.as_deref(), Some("p1"), "`of` by the model's key → the real id");
    // questions: known kinds on paragraphs only, ids scrubbed, at most three options
    assert_eq!(p.questions.len(), 2);
    assert_eq!(p.questions[0].question, "你說過 的原因，B 和 C 差在哪？");
    assert!(!p.questions[0].question.contains("turn") && !p.questions[0].question.contains("E1"));
    assert_eq!(p.questions[0].options.len(), 3);
    assert_eq!((p.questions[0].anchor.as_str(), p.questions[1].anchor.as_str()), ("p2", "p1"));
    plan::install(&mut sk, p);
    assert_eq!(sk.threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["q1", "q2"]);
    assert_eq!(sk.next().as_deref(), Some("q1"));
    assert_eq!(sk.threads[0].messages[0].who, "ai");
    assert!(plan::parse(r#"{"sections":[],"questions":[]}"#, &known(), &mut sk).is_err());
    assert_eq!(plan::scrub("第 3 輪你提到（turn 9）的事", &Default::default()), "你提到的事");
}

#[test]
fn at_most_fifteen_questions() {
    let qs: Vec<String> = (0..30).map(|k| format!(r#"{{"para":"x1","kind":"missing","question":"問題 {k}？","options":["a","b"]}}"#)).collect();
    let reply = format!(r#"{{"sections":[{{"title":"Results","role":"results","blocks":[{{"key":"b1","title":"t","purpose":"","of":"","paras":[{{"key":"x1","say":"s"}}]}}]}}],"questions":[{}]}}"#, qs.join(","));
    let p = plan::parse(&reply, &known(), &mut Skeleton::default()).unwrap();
    assert_eq!(p.questions.len(), plan::MAX_QUESTIONS);
    assert_eq!(p.questions[0].question, "問題 0？", "most important first: the model's order");
}

#[test]
fn the_repair_changes_only_its_scope() {
    let mut sk = small();
    for (k, p) in sk.paras_mut().enumerate() {
        p.link_prev = format!("prev{k}");
        p.link_next = format!("next{k}");
        p.text = format!("Text {k}.");
    }
    let f = |v: serde_json::Value| v.as_object().unwrap().clone();
    sk.apply(Op::Move { id: "p5".into(), to: "b1".into(), at: Some(0), reason: "限制先講".into() }).unwrap();
    sk.apply(Op::Edit { id: "p3".into(), fields: f(serde_json::json!({ "say": "外部排名反轉" })) }).unwrap();
    let before: Vec<Para> = sk.paras().cloned().collect();
    let (paras, blocks) = sk.repair_scope();
    assert_eq!(paras, ["p5", "p1", "p2", "p3", "p4"]);
    let prompt = plan::repair_prompt("kv", &sk, &paras, &blocks);
    assert!(prompt.user.contains("p5 CHANGED") && prompt.user.contains("the author's reason: 限制先講"));
    // the model also "fixes" a paragraph outside the scope, and a block it did not see
    let reply = r#"{"paras":[{"id":"p5","say":"方法的限制","link_prev":"","link_next":"接著看 A 對 B","why":"移到最前面"},{"id":"p1","say":"A 對 B","link_prev":"承接限制","link_next":"next0","why":"前面多了一段"},{"id":"p9","say":"x","link_prev":"","link_next":"","why":""}],
      "blocks":[{"id":"b1","purpose":"先講限制再比較","why":"多了限制"},{"id":"b3","purpose":"不該改","why":""}],
      "jumps":[{"id":"p3","why":"從比較跳到外部","fix":"加一句過渡"}]}"#;
    let mut sk2 = sk.clone();
    // pretend only p5..p3 were in scope (p4 out) to check byte identity outside it
    let scope: Vec<String> = paras.iter().filter(|x| *x != "p4").cloned().collect();
    assert!(blocks.contains(&"b3".to_string()), "p4 is a seam: its block is in the scope");
    let blocks: Vec<String> = blocks.into_iter().filter(|b| b != "b3").collect();
    let n = plan::apply_repair(&mut sk2, plan::parse_repair(reply).unwrap(), &scope, &blocks);
    assert_eq!(n, 5, "p5 say+link_prev+link_next, p1 link_prev, b1 purpose");
    for p in &before {
        if !scope.contains(&p.id) {
            assert_eq!(serde_json::to_string(sk2.para(&p.id).unwrap()).unwrap(), serde_json::to_string(p).unwrap(), "{} outside the scope is byte-identical", p.id);
        }
    }
    assert_eq!(sk2.block("b3").unwrap().purpose, "", "a block outside the scope keeps its purpose");
    assert_eq!(sk2.block("b1").unwrap().purpose, "先講限制再比較");
    assert_eq!(sk2.para("p3").unwrap().jump.as_ref().unwrap().fix, "加一句過渡");
    let edits: Vec<&Thread> = sk2.threads.iter().filter(|t| t.kind == "edit").collect();
    assert_eq!(edits.iter().map(|t| t.anchor.as_str()).collect::<Vec<_>>(), ["p5", "p1", "b1"]);
    assert_eq!(edits[0].changes.iter().map(|c| (c.id.as_str(), c.field.as_str(), c.state.as_str())).collect::<Vec<_>>(), [("c1", "say", "applied"), ("c2", "link_prev", "applied"), ("c3", "link_next", "applied")]);
    assert!(!sk2.para("p5").unwrap().touched && sk2.para("p5").unwrap().reason.is_empty());
    // 還原 one change
    let e1 = edits[0].id.clone();
    sk2.decide(&e1, "c1", false).unwrap();
    assert_eq!(sk2.para("p5").unwrap().say, "限制");
}

fn answered(sk: &mut Skeleton, anchor: &str, answer: &str) -> String {
    sk.add_thread(Thread { anchor: anchor.into(), kind: "missing".into(), status: "answered".into(), question: "原因是什麼？".into(), messages: vec![Msg { who: "ai".into(), text: "原因是什麼？".into() }, Msg { who: "me".into(), text: answer.into() }], ..Default::default() })
}

fn fill(points: &[&str], conclusion: &str, follow: Option<&str>) -> plan::FillReply {
    plan::FillReply { points: points.iter().map(|x| x.to_string()).collect(), conclusion: conclusion.into(), gap: follow.is_some(), follow_up: follow.map(|q| plan::FollowUp { kind: "missing".into(), question: q.into(), options: vec!["a".into(), "b".into()] }), writing_notes: vec![] }
}

#[test]
fn a_fill_lands_where_the_paragraph_is_now() {
    let mut sk = small();
    sk.para_mut("p3").unwrap().points.push(Point { text: "planner".into(), from: None });
    let q = answered(&mut sk, "p3", "兩邊資料來源不同");
    sk.para_mut("p3").unwrap().busy = Some("fill".into());
    // the author moves the paragraph while the fill runs
    sk.apply(Op::Move { id: "p3".into(), to: "b3".into(), at: Some(0), reason: "".into() }).unwrap();
    let ids = Default::default();
    assert!(plan::apply_fill(&mut sk, &q, fill(&["來源不同"], "資料來源不同", None), &ids).is_none());
    let p = sk.para("p3").unwrap();
    assert_eq!(p.points.iter().map(|x| (x.text.as_str(), x.from.as_deref())).collect::<Vec<_>>(), [("planner", None), ("來源不同", Some(q.as_str()))]);
    assert_eq!((p.busy.clone(), p.state), (None, State::Points));
    assert_eq!(sk.thread(&q).unwrap().status, "done");
    assert_eq!(sk.thread(&q).unwrap().conclusion, "資料來源不同");
    assert_eq!(sk.block_of("p3").unwrap().id, "b3");
    // again (an answer folded in twice): its own points are replaced, never doubled
    plan::apply_fill(&mut sk, &q, fill(&["來源不同，標註相同"], "", None), &ids);
    assert_eq!(sk.para("p3").unwrap().points.len(), 2);
}

#[test]
fn a_fill_for_a_removed_paragraph_is_dropped() {
    let mut sk = small();
    let q = answered(&mut sk, "p2", "是");
    sk.apply(Op::Remove { id: "p2".into() }).unwrap();
    // the answered thread hangs on the block now, but its fill-in is dropped (never into another paragraph)
    assert_eq!(sk.thread(&q).unwrap().anchor, "b1");
    plan::apply_fill(&mut sk, &q, fill(&["x"], "y", Some("再問？")), &Default::default());
    assert_eq!(sk.thread(&q).unwrap().status, "done");
    assert!(sk.paras().all(|p| p.points.is_empty()));
    assert_eq!(sk.threads.len(), 1, "no follow-up");
    // removed, then brought back by undo before the fill landed: it lands there
    let mut sk = small();
    let q = answered(&mut sk, "p2", "是");
    sk.apply(Op::Remove { id: "p2".into() }).unwrap();
    sk.apply(Op::Undo).unwrap();
    plan::apply_fill(&mut sk, &q, fill(&["x"], "y", None), &Default::default());
    assert_eq!(sk.para("p2").unwrap().points[0].text, "x");
    // a note on a block goes into its first paragraph
    let mut sk = small();
    let n = sk.add_thread(Thread { anchor: "b2".into(), kind: "note".into(), status: "answered".into(), messages: vec![Msg { who: "me".into(), text: "外部要先講".into() }], ..Default::default() });
    plan::apply_fill(&mut sk, &n, fill(&["外部先講"], "", None), &Default::default());
    assert_eq!(sk.para("p3").unwrap().points[0].from.as_deref(), Some(n.as_str()));
    // a thread whose paragraph and block are both gone: nothing to write into
    let mut sk = small();
    let q = answered(&mut sk, "p9", "是");
    let before: Vec<Para> = sk.paras().cloned().collect();
    plan::apply_fill(&mut sk, &q, fill(&["x"], "y", Some("再問？")), &Default::default());
    assert_eq!(sk.paras().cloned().collect::<Vec<_>>(), before);
    assert_eq!(sk.threads.len(), 1, "no follow-up on nothing");
}

#[test]
fn a_follow_up_only_for_a_real_gap_and_never_after_no() {
    let mut sk = small();
    let ids: std::collections::BTreeSet<String> = ["E1".to_string()].into();
    let other = sk.add_thread(Thread { anchor: "p4".into(), kind: "choice".into(), status: "open".into(), question: "後面的".into(), ..Default::default() });
    let q = answered(&mut sk, "p1", "A 在小資料比較好，因為先驗");
    let f = plan::apply_fill(&mut sk, &q, fill(&["A 小資料較好"], "先驗", Some("E1 的先驗是什麼？")), &ids).unwrap();
    assert_eq!(sk.thread(&f).unwrap().question, "的先驗是什麼？");
    assert_eq!(sk.threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [other.as_str(), q.as_str(), f.as_str()]);
    // with an open question after it (the one the page shows now), the follow-up comes after that one
    let mut sk2 = small();
    let q2 = answered(&mut sk2, "p1", "因為先驗");
    let shown = sk2.add_thread(Thread { anchor: "p4".into(), kind: "choice".into(), status: "open".into(), question: "現在這題".into(), ..Default::default() });
    let later = sk2.add_thread(Thread { anchor: "p5".into(), kind: "choice".into(), status: "open".into(), question: "再後面".into(), ..Default::default() });
    let f2 = plan::apply_fill(&mut sk2, &q2, fill(&["先驗"], "先驗", Some("哪種先驗？")), &ids).unwrap();
    assert_eq!(sk2.threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [q2.as_str(), shown.as_str(), f2.as_str(), later.as_str()]);
    assert_eq!(sk2.next().as_deref(), Some(shown.as_str()), "the question shown stays next");
    // answering the follow-up never makes another one
    sk.thread_mut(&f).unwrap().status = "answered".into();
    sk.thread_mut(&f).unwrap().messages.push(Msg { who: "me".into(), text: "形狀先驗".into() });
    assert!(plan::apply_fill(&mut sk, &f, fill(&["形狀先驗"], "形狀", Some("還有呢？")), &ids).is_none());
    // "no / not yet": recorded as said, no follow-up even when the model wants one
    let n = answered(&mut sk, "p2", "還沒有做外部驗證");
    assert!(plan::apply_fill(&mut sk, &n, fill(&[], "", Some("什麼時候會做？")), &ids).is_none());
    let p = sk.para("p2").unwrap();
    assert_eq!(p.points[0].text, "還沒有做外部驗證");
    assert_eq!(sk.thread(&n).unwrap().conclusion, "還沒有做外部驗證");
    assert!(plan::is_negative("沒有") && plan::is_negative("No, not yet") && !plan::is_negative("有，兩次"));
    // a gap the model did not flag is no follow-up
    let g = answered(&mut sk, "p3", "有");
    let mut r = fill(&["有"], "有", Some("x？"));
    r.gap = false;
    assert!(plan::apply_fill(&mut sk, &g, r, &ids).is_none());
}
