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
