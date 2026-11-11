//! The 論文報告 job's `profile` and `polish` stages on synthetic decks: the profile reaches the
//! outline as a summary, the polish fixes carry their exact changes, and one fix is undone
//! alone after the stage's single transaction.


use lore_canvas::{Bullet, Deck, Editor, Frame, Geom, Kind, Look, Object, Op, Paint, Para, Run, Slide, SlideSrc, SpObj, StyleSystem, TextBody};
use sc_server::deckpolish;
use sc_server::todeck::polish::{PolishInput, polish};
use sc_server::todeck::profile::{ProfileInput, profile};

const W: i64 = 12_192_000;
const H: i64 = 6_858_000;

fn text(id: u32, f: Frame, paras: Vec<Para>, ph: Option<&str>) -> Object {
    Object { id, name: "t".into(), frame: f, hidden: false, ph: ph.map(String::from), slot: None, kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(TextBody { paras, ..Default::default() }), equation: false }), src: None }
}

fn run(t: &str, st: Option<&StyleSystem>, tag: &str) -> Run {
    match st {
        Some(st) => Run::text(t, tag, st.look(tag, None).unwrap()),
        None => {
            let mut r = Run::text(t, tag, Look { size: 18.0, ..Look::default() });
            r.tag = None;
            r
        }
    }
}

fn deck(titles: &[&str], st: Option<&StyleSystem>, imported: bool) -> Deck {
    let mut d = Deck::new(W, H, Some("blank".into()));
    for (i, t) in titles.iter().enumerate() {
        let uid = d.new_uid();
        let mut s = Slide::new(uid, None);
        if imported {
            s.src = Some(SlideSrc { part: format!("ppt/slides/slide{i}.xml"), k: i as u64, notes_k: 0, anims_k: 0 });
        }
        s.objects.push(text(2, Frame::new(W / 25, H / 20, W * 9 / 10, H / 8), vec![Para::new(vec![run(t, st, "title")])], Some("title")));
        let mut p = Para::new(vec![run(&format!("point {i}"), st, "body")]);
        p.bullet = Bullet::Char { ch: "•".into() };
        s.objects.push(text(3, Frame::new(W / 25, H / 4, W * 9 / 10, H / 2), vec![p], None));
        d.slides.push(s);
    }
    d
}

const AUTHOR: [&str; 8] = ["Paper", "Background", "Overview", "Encoder", "Results", "Ablation study", "Take home messages", "Thanks"];

#[test]
fn profile_and_polish_stages() {
    let tmp = tempfile::tempdir().unwrap();
    let home = lore_home::Home::at(tmp.path());
    let decks = home.app_dir("SlideComposer").unwrap().join("decks");
    for (i, n) in ["a", "b", "c"].iter().enumerate() {
        let mut d = deck(&AUTHOR, None, true);
        d.slides[1].objects[1] = text(3, Frame::new(W / 25, H / 4, W / 2, H / 2), vec![Para::new(vec![run(&format!("deck {i}"), None, "body")])], None);
        std::fs::create_dir_all(decks.join(n)).unwrap();
        std::fs::write(decks.join(n).join("deck.json"), serde_json::to_vec(&d).unwrap()).unwrap();
    }
    let example = sc_server::repo_dir().join("../private-example/SlideComposer");
    lore_home::copy_missing(&example, &tmp.path().join("sc")).unwrap();
    let ws = sc_core::Workspace::new(tmp.path().join("sc"), None);
    let st = ws.style("blank").unwrap();

    let prof = profile(&ProfileInput { home: &home, ws: &ws, style: "blank", lang: "en" });
    assert!(prof.summary.contains("Take home messages"), "{}", prof.summary);
    assert!(prof.detail.as_ref().is_some_and(|p| p.decks == 3));
    assert!(ws.root.join("cache/deck-style/profile.json").is_file());

    // the generated deck: a period on a title, another take-home title
    let made = deck(&["Paper", "Background", "Overview", "Encoder.", "Results", "Key messages", "Thanks"], Some(&st), false);
    let mut slides = made.slides.clone();
    let rep = polish(&mut slides, &PolishInput { st: &st, profile: &prof, notation: &Default::default(), lang: "en" });
    assert_eq!(rep.checks, 6);
    assert_eq!(rep.fixes.len(), 2, "{rep:?}");
    assert!(rep.fixes.iter().all(|f| f.rule == 3 && !f.changes.is_empty()));
    assert!(rep.fixes.iter().any(|f| f.before == "Encoder." && f.after == "Encoder"));

    // the stage applies the slides as one transaction; one fix is then undone alone
    let mut ed = Editor::new(made, Some(st));
    let mut ops: Vec<Op> = ed.deck.slides.iter().map(|s| Op::DeleteSlide { slide: s.uid.clone() }).collect();
    ops.extend(slides.into_iter().enumerate().map(|(i, s)| Op::InsertSlide { index: i, slide: s }));
    ed.apply(ops, "排版用字微調").unwrap();
    let take = rep.fixes.iter().find(|f| f.after == "Take home messages").unwrap();
    let mut it: deckpolish::Item = serde_json::from_value(serde_json::json!({ "id": 0, "check": 3, "slide": take.slide, "message": take.what, "state": "applied", "changes": take.changes })).unwrap();
    deckpolish::commit(&mut ed, &mut it, false).unwrap();
    let title = |i: usize| deckpolish_title(&ed.deck, i);
    assert_eq!(title(5), "Key messages");
    assert_eq!(title(3), "Encoder");
}

fn deckpolish_title(d: &Deck, i: usize) -> String {
    sc_server::deckstyle::title_text(&d.slides[i], d.size.h)
}

