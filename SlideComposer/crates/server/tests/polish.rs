//! The style profile and the polish pass (排版用字微調) on synthetic decks: the author's decks
//! are made up here (footer, cover names and bullets that must never reach the profile),
//! the deck to polish is built from the public blank base, the model is a mock.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use lore_canvas::{Bullet, Deck, Editor, Frame, Geom, Kind, Look, Object, Op, Paint, Para, PicObj, Run, Slide, SlideSrc, SpObj, StyleSystem, TextBody};
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use lore_notation::Notation;
use sc_server::deckpolish::{self, Input, Report};
use sc_server::deckstyle::{self, Profile};
use serde_json::{Value, json};
use tower::ServiceExt;

const W: i64 = 12_192_000;
const H: i64 = 6_858_000;

fn look(size: f64) -> Look {
    Look { font: "Arial".into(), ea_font: "Arial".into(), size, color: "1D1D20".into(), alpha: 1.0, ..Look::default() }
}

fn styled(st: &StyleSystem, text: &str, tag: &str, em: Option<&str>) -> Run {
    let mut r = Run::text(text, tag, st.look(tag, em).unwrap());
    r.em = em.map(String::from);
    r
}

fn plain(text: &str, size: f64) -> Run {
    let mut r = Run::text(text, "body", look(size));
    r.tag = None;
    r
}

fn text_obj(id: u32, name: &str, frame: Frame, paras: Vec<Para>, ph: Option<&str>) -> Object {
    Object {
        id,
        name: name.into(),
        frame,
        hidden: false,
        ph: ph.map(String::from),
        slot: None,
        kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(TextBody { paras, ..Default::default() }), equation: false }),
        src: None,
    }
}

fn bullets(runs: Vec<Run>) -> Para {
    let mut p = Para::new(runs);
    p.bullet = Bullet::Char { ch: "•".into() };
    p
}

fn pic(id: u32, asset: &str, frame: Frame) -> Object {
    Object { id, name: "圖".into(), frame, hidden: false, ph: None, slot: None, kind: Kind::Picture(PicObj { asset: asset.into(), crop: Default::default(), alpha: 1.0, line: None, clip: "rect".into(), masks: vec![], effects: vec![] }), src: None }
}

fn title(id: u32, text: &str) -> Object {
    text_obj(id, "Title", Frame::new(W / 25, H / 20, W * 23 / 25, H / 8), vec![Para::new(vec![plain(text, 28.0)])], Some("title"))
}

// ── The author's decks (synthetic) ───────────────────────────────────────────

const FOOTER: &str = "Daidai Lab internal only";
const COVER: &str = "Presented by ellrin";
const SECRET_BULLET: &str = "zebracorn widget measurements";

fn author_deck(n: usize, imported: bool) -> Deck {
    let mut d = Deck::new(W, H, Some("blank".into()));
    let titles = ["Paper title", "Background of grading", "Why it is hard", "Overview", "Encoder", "Fusion", "Loss function", "Training details", "Results on set A", "Ablation study", "Some opinions", "Take home messages", "Thanks"];
    for (i, t) in titles.iter().enumerate() {
        let uid = d.new_uid();
        let mut s = Slide::new(uid, None);
        if imported {
            s.src = Some(SlideSrc { part: format!("ppt/slides/slide{}.xml", i + 1), k: i as u64, notes_k: 0, anims_k: 0 });
        }
        s.objects.push(title(2, t));
        if i == 0 {
            s.objects.push(text_obj(3, "sub", Frame::new(W / 10, H / 2, W / 2, H / 10), vec![Para::new(vec![plain(COVER, 18.0)])], Some("subTitle")));
        } else if i == 11 {
            let items = ["Use two stages", "Fusion matters", "Check the data"];
            let mut ps = vec![];
            for it in items {
                let mut p = Para::new(vec![plain(it, 18.0)]);
                p.bullet = Bullet::Num { scheme: "arabicPeriod".into(), start: 1 };
                ps.push(p);
            }
            s.objects.push(text_obj(3, "body", Frame::new(W / 10, H / 4, W * 8 / 10, H / 2), ps, None));
        } else if i != 12 {
            let k = 2 + (i + n) % 3;
            let mut ps: Vec<Para> = (0..k).map(|j| bullets(vec![plain(&format!("{SECRET_BULLET} {j}"), 18.0)])).collect();
            if i == 8 {
                let mut r = plain("best score", 18.0);
                r.em = Some("key".into());
                ps.push(bullets(vec![plain("We reach the ", 18.0), r]));
            }
            s.objects.push(text_obj(3, "body", Frame::new(W / 2, H / 4, W * 4 / 10, H / 2), ps, None));
            if i == 1 {
                s.objects.push(text_obj(5, "ref", Frame::new(W / 25, H * 9 / 10, W / 2, H / 20), vec![Para::new(vec![plain("Ref: Ellrin et al., 2024", 10.0)])], None));
            }
            if i == 8 {
                s.objects.push(pic(6, "fig-results", Frame::new(W / 25, H / 4, W * 4 / 10, H / 2)));
                s.objects.push(text_obj(7, "cap", Frame::new(W / 25, H * 3 / 4 + H / 100, W * 4 / 10, H / 20), vec![Para::new(vec![plain("Fig. 2 Scores on set A", 12.0)])], None));
            }
        }
        // footer and logo on every slide
        s.objects.push(text_obj(10, "footer", Frame::new(W / 25, H * 95 / 100, W / 3, H / 30), vec![Para::new(vec![plain(FOOTER, 9.0)])], None));
        s.objects.push(pic(11, "logo", Frame::new(W * 9 / 10, H * 9 / 10, W / 20, H / 20)));
        d.slides.push(s);
    }
    d
}

fn write_deck(dir: &Path, name: &str, d: &Deck) {
    std::fs::create_dir_all(dir.join(name)).unwrap();
    std::fs::write(dir.join(name).join("deck.json"), serde_json::to_vec(d).unwrap()).unwrap();
}

#[test]
fn profile_from_the_authors_decks() {
    let tmp = tempfile::tempdir().unwrap();
    let home = lore_home::Home::at(tmp.path());
    let decks = home.app_dir("SlideComposer").unwrap().join("decks");
    write_deck(&decks, "talk-1", &author_deck(0, true));
    write_deck(&decks, "talk-2", &author_deck(1, true));
    write_deck(&decks, "talk-3", &author_deck(2, true));
    // a deck SlideComposer generated never feeds the profile; a short sample neither
    let mut generated = author_deck(3, false);
    generated.slides[11].objects[0] = title(2, "Generated wrap-up");
    write_deck(&decks, "generated", &generated);
    let mut short = author_deck(0, true);
    short.slides.truncate(3);
    write_deck(&decks, "sample", &short);
    let before: Vec<Vec<u8>> = ["talk-1", "talk-2", "talk-3"].iter().map(|n| std::fs::read(decks.join(n).join("deck.json")).unwrap()).collect();

    let cache = tmp.path().join("ws/cache");
    let p = deckstyle::for_home(&home, &cache).expect("a profile");
    assert_eq!(p.decks, 3);
    assert_eq!(p.pages, 39);
    let tk = &p.roles["takeaway"];
    assert_eq!(tk.fixed_title.as_deref(), Some("Take home messages"));
    assert_eq!(p.roles["thanks"].fixed_title.as_deref(), Some("Thanks"));
    assert!(p.roles["method"].share > 0.1);
    assert_eq!(p.take_home.numbered, 1.0);
    assert_eq!(p.take_home.items.median, 3.0);
    assert!(p.take_home.verb_start > 0.5, "{:?}", p.take_home);
    assert_eq!(p.refs.prefix, "Ref:");
    assert_eq!(p.captions.fig_form, "Fig.");
    assert!(p.captions.with_caption > 0.9);
    assert_eq!(p.emphasis.key_max, 1);
    assert!(p.bullet_limit("method") >= 4);
    // nothing of the content, footer, cover or the generated deck
    let json = serde_json::to_string(&p).unwrap() + &p.prompt_text();
    for leak in [FOOTER, COVER, SECRET_BULLET, "zebracorn", "Daidai", "ellrin", "Generated", "best score", "Use two stages", "logo"] {
        assert!(!json.contains(leak), "{leak} leaked: {json}");
    }
    let text = p.prompt_text();
    assert!(text.contains("Take home") && text.contains("\"Ref:\"") && text.contains("Fig."), "{text}");
    // read-only on the decks; cached; a changed deck rebuilds
    let after: Vec<Vec<u8>> = ["talk-1", "talk-2", "talk-3"].iter().map(|n| std::fs::read(decks.join(n).join("deck.json")).unwrap()).collect();
    assert_eq!(before, after);
    let file = deckstyle::cache_file(&cache);
    let cached: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(cached["key"], deckstyle::cache_key(&decks));
    let mut planted = cached.clone();
    planted["profile"]["decks"] = json!(99);
    std::fs::write(&file, serde_json::to_vec(&planted).unwrap()).unwrap();
    assert_eq!(deckstyle::load_or_build(&decks, &cache).unwrap().decks, 99, "served from the cache");
    let mut d4 = author_deck(4, true);
    d4.slides[1].objects[0] = title(2, "Background of scoring");
    write_deck(&decks, "talk-4", &d4);
    assert_eq!(deckstyle::load_or_build(&decks, &cache).unwrap().decks, 4, "rebuilt after a change");
    // no decks folder: no profile
    assert!(deckstyle::for_home(&lore_home::Home::at(tmp.path().join("nobody")), &cache).is_none());
}

// ── The deck to polish ───────────────────────────────────────────────────────

fn blank_style(root: &Path) -> (sc_core::Workspace, StyleSystem) {
    let example = sc_server::repo_dir().join("../private-example/SlideComposer");
    lore_home::copy_missing(&example, &root.join("sc")).unwrap();
    let ws = sc_core::Workspace::new(root.join("sc"), None);
    let st = ws.style("blank").unwrap();
    (ws, st)
}

fn styled_title(st: &StyleSystem, id: u32, text: &str) -> Object {
    let mut o = text_obj(id, "Title", Frame::new(W / 25, H / 20, W * 23 / 25, H / 8), vec![Para::new(vec![styled(st, text, "title", None)])], None);
    o.slot = Some("title-1".into());
    o
}

/// Slides that trip every check; the `uid`s are s1…s8.
fn messy_deck(st: &StyleSystem) -> Deck {
    let mut d = Deck::new(W, H, Some("blank".into()));
    let body = |id: u32, f: Frame, ps: Vec<Para>| text_obj(id, "內文", f, ps, None);
    let b = |t: &str| bullets(vec![styled(st, t, "body", None)]);
    let mut s = |objs: Vec<Object>| {
        let uid = d.new_uid();
        let mut sl = Slide::new(uid, None);
        sl.objects = objs;
        d.slides.push(sl);
    };
    // 1 cover
    s(vec![styled_title(st, 2, "Two-stage grading of synthetic lesions")]);
    // 2 background: 7 bullets (> 5), a citation without Ref:, a Reference: line
    s(vec![
        styled_title(st, 2, "Background of grading"),
        body(3, Frame::new(W / 25, H / 5, W * 9 / 10, H * 6 / 10), (0..7).map(|i| b(&format!("Point {i} about grading [{i}]"))).collect()),
        text_obj(4, "ref", Frame::new(W / 25, H * 9 / 10, W / 2, H / 20), vec![Para::new(vec![styled(st, "Reference: Ellrin et al., 2024", "caption", None)])], None),
    ]);
    // 3 method: title with a period; Figure form, lora spelling, two key spans, a tiny run
    let mut tiny = styled(st, "tiny note", "body", None);
    tiny.look.size = 10.0;
    s(vec![
        styled_title(st, 2, "Encoder."),
        body(3, Frame::new(W / 25, H / 5, W * 9 / 10, H * 4 / 10), vec![
            b("Figure 2 shows the lora adapters in the encoder"),
            bullets(vec![styled(st, "It reaches ", "body", None), styled(st, "0.91 AUC", "body", Some("key"))]),
            bullets(vec![styled(st, "and ", "body", None), styled(st, "halves", "body", Some("key")), styled(st, " ", "body", None), styled(st, "the time", "body", Some("key"))]),
            Para::new(vec![tiny]),
        ]),
    ]);
    // 4 method: overflowing box with room below
    let long = "The fusion module weighs every finding by its confidence and its position on the image, then passes the weighted map on to the decoder for the final grade";
    s(vec![styled_title(st, 2, "Fusion"), body(3, Frame::new(W / 25, H / 5, W * 9 / 10, H / 12), vec![b(long), b(long)])]);
    // 5 results: a big picture without a caption
    s(vec![styled_title(st, 2, "Results on set A"), pic(3, "fig-a", Frame::new(W / 25, H / 5, W / 2, H / 2))]);
    // 6 overflow with no room and no smaller size: manual
    let mut wall = vec![];
    for _ in 0..24 {
        wall.push(Para::new(vec![styled(st, long, "caption", None)]));
    }
    s(vec![styled_title(st, 2, "Ablation study"), body(3, Frame::new(W / 25, H / 5, W * 9 / 10, H * 3 / 4), wall)]);
    // 7 take home with another title
    s(vec![styled_title(st, 2, "Key messages"), body(3, Frame::new(W / 25, H / 5, W * 9 / 10, H / 2), vec![b("Use two stages")])]);
    // 8 thanks
    s(vec![styled_title(st, 2, "Thanks")]);
    d
}

fn profile() -> Profile {
    let mut p = Profile { decks: 3, ..Default::default() };
    p.roles.insert("takeaway".into(), deckstyle::RoleStyle { fixed_title: Some("Take home messages".into()), ..Default::default() });
    p.bullets = deckstyle::Stat { n: 30, median: 3.0, p90: 5.0, max: 6.0 };
    p.refs.prefix = "Ref:".into();
    p.captions.fig_form = "Fig.".into();
    p
}

fn notation() -> Notation {
    let mut n = Notation::default();
    let mut e = lore_notation::model::Entry::new("lora", lore_notation::model::Kind::Abbrev);
    e.short = Some("LoRA".into());
    e.long = Some("low-rank adaptation".into());
    n.entries.push(e);
    n
}

fn run_text(d: &Deck, slide: usize, obj: u32, para: usize) -> String {
    let o = d.slides[slide].find(obj).unwrap();
    deckstyle::body_of(o).unwrap().paras[para].runs.iter().map(|r| r.text.as_str()).collect()
}

fn items(r: &Report, check: u8, state: &str) -> usize {
    r.items.iter().filter(|i| i.check == check && i.state == state).count()
}

#[test]
fn polish_checks_fix_and_undo_item_by_item() {
    let tmp = tempfile::tempdir().unwrap();
    let (_ws, st) = blank_style(tmp.path());
    let deck = messy_deck(&st);
    let mut ed = Editor::new(deck, Some(st.clone()));
    let roles = deckpolish::roles_for(&ed.deck, &[]);
    assert_eq!(roles, ["cover", "background", "method", "method", "results", "ablation", "takeaway", "thanks"]);
    let n = notation();
    let p = profile();
    let rep = deckpolish::run_checks(&mut ed, &Input { profile: Some(&p), notation: &n, roles });
    let d = &ed.deck;
    let dump = serde_json::to_string_pretty(&rep).unwrap();

    // 1: tiny run raised to the tag minimum; overflow grown into free space; the wall is manual
    assert_eq!(items(&rep, 1, "applied"), 2, "{dump}");
    assert_eq!(items(&rep, 1, "manual"), 1, "{dump}");
    assert_eq!(deckstyle::body_of(d.slides[2].find(3).unwrap()).unwrap().paras[3].runs[0].look.size, 14.0);
    assert!(d.slides[3].find(3).unwrap().frame.h > H / 12);
    // 2: seven bullets on the background page
    assert_eq!(items(&rep, 2, "manual"), 1, "{dump}");
    // 3: period dropped, take-home title fixed
    assert_eq!(run_text(d, 2, 2, 0), "Encoder");
    assert_eq!(run_text(d, 6, 2, 0), "Take home messages");
    // 4: Reference: → Ref:, picture without caption; the cited page has a Ref: line, so no item
    assert!(run_text(d, 1, 4, 0).starts_with("Ref: Ellrin"));
    assert_eq!(items(&rep, 4, "manual"), 1, "{dump}");
    // 5: one key span left (the first), the second loses it and gets the tag's look back
    let ems: Vec<Option<String>> = deckstyle::body_of(d.slides[2].find(3).unwrap()).unwrap().paras.iter().flat_map(|p| p.runs.iter().map(|r| r.em.clone())).collect();
    assert_eq!(ems.iter().filter(|e| e.is_some()).count(), 1, "{ems:?}");
    assert_eq!(items(&rep, 5, "applied"), 1, "{dump}");
    // 6: Fig. form and the registered spelling, token only
    assert_eq!(run_text(d, 2, 3, 0), "Fig. 2 shows the LoRA adapters in the encoder");
    // nothing else of the text changed: every applied item is one transaction
    let applied = rep.items.iter().filter(|i| i.state == "applied").count();
    assert_eq!(ed.undo.len(), applied);

    // undo one item on its own (the key emphasis), later ones stay
    let mut rep = rep;
    let k = rep.items.iter().position(|i| i.check == 5).unwrap();
    deckpolish::commit(&mut ed, &mut rep.items[k], false).unwrap();
    assert_eq!(rep.items[k].state, "undone");
    let ems = deckstyle::body_of(ed.deck.slides[2].find(3).unwrap()).unwrap().paras.iter().flat_map(|p| p.runs.iter().filter(|r| r.em.is_some())).count();
    assert_eq!(ems, 3);
    assert_eq!(run_text(&ed.deck, 2, 2, 0), "Encoder");
    // and again
    deckpolish::commit(&mut ed, &mut rep.items[k], true).unwrap();
    // an item whose text was edited afterwards is not undone over the edit
    let t = rep.items.iter().position(|i| i.check == 3 && i.message.contains("句點")).unwrap();
    let path = rep.items[t].changes[0].path.clone();
    let uid = ed.deck.slides[2].uid.clone();
    ed.apply(vec![Op::SetObj { slide: uid, id: 2, path, value: json!("Encoder module") }], "edit").unwrap();
    assert!(deckpolish::commit(&mut ed, &mut rep.items[t], false).is_err());
    assert_eq!(run_text(&ed.deck, 2, 2, 0), "Encoder module");
}

#[test]
fn terms_come_from_the_model_as_suggestions() {
    let tmp = tempfile::tempdir().unwrap();
    let (_ws, st) = blank_style(tmp.path());
    let mut d = messy_deck(&st);
    let extra = text_obj(9, "內文", Frame::new(W / 25, H * 3 / 4, W / 2, H / 10), vec![Para::new(vec![styled(&st, "The mask map and the segmentation mask agree", "body", None)])], None);
    d.slides[4].objects.push(extra);
    let terms = deckpolish::candidate_terms(&d, 300);
    let names: Vec<&str> = terms.iter().map(|(t, _)| t.as_str()).collect();
    assert!(names.contains(&"mask map") && names.contains(&"segmentation mask"), "{names:?}");
    assert!(!names.iter().any(|t| t.contains(' ') && t.split(' ').count() > 3));
    let groups = vec![
        deckpolish::Group { preferred: "segmentation mask".into(), variants: vec!["mask map".into(), "not in deck".into(), "mask".into()] },
    ];
    let mut rep = Report::default();
    deckpolish::term_items(&d, &groups, &terms, &mut rep);
    assert_eq!(rep.items.len(), 1, "{rep:?}");
    let it = &mut rep.items[0];
    assert_eq!(it.state, "suggested");
    let mut ed = Editor::new(d, Some(st));
    deckpolish::commit(&mut ed, it, true).unwrap();
    assert_eq!(run_text(&ed.deck, 4, 9, 0), "The segmentation mask and the segmentation mask agree");
    deckpolish::commit(&mut ed, it, false).unwrap();
    assert_eq!(run_text(&ed.deck, 4, 9, 0), "The mask map and the segmentation mask agree");
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&b).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn polish_over_http_with_a_mock_model() {
    let tmp = tempfile::tempdir().unwrap();
    let (ws, st) = blank_style(tmp.path());
    let name = ws.new_deck("messy", "blank").unwrap();
    let mut deck = messy_deck(&st);
    deck.slides[4].objects.push(text_obj(9, "內文", Frame::new(W / 25, H * 3 / 4, W / 2, H / 10), vec![Para::new(vec![styled(&st, "The mask map and the segmentation mask agree", "body", None)])], None));
    std::fs::write(ws.decks.join(&name).join("deck.json"), serde_json::to_vec(&deck).unwrap()).unwrap();
    std::fs::write(ws.decks.join(&name).join("notation.json"), notation().to_json()).unwrap();
    let home = lore_home::Home::at(tmp.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let reply = json!({ "groups": [{ "preferred": "segmentation mask", "variants": ["mask map"] }] });
    let model = Arc::new(MockProvider::new(vec![reply.to_string()]));
    let f = MockFactory::default().with("claude:selected", model.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());

    let (s0, v0) = call(&app, "GET", &format!("/api/decks/{name}/polish"), None).await;
    assert_eq!((s0, v0["items"].as_array().map(Vec::len)), (StatusCode::OK, Some(0)));
    let (s, rep) = call(&app, "POST", &format!("/api/decks/{name}/polish"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{rep}");
    let items = rep["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["state"] == "applied"));
    let sug = items.iter().find(|i| i["state"] == "suggested").expect("a term suggestion");
    // the model saw terms, not sentences
    let seen = model.seen.lock().unwrap();
    let user = seen[0].messages.iter().map(|m| format!("{m:?}")).collect::<String>();
    assert!(user.contains("mask map") && !user.contains("The mask map and the segmentation mask agree"), "{user}");
    drop(seen);
    // 套用 the suggestion, then undo it
    let id = sug["id"].as_u64().unwrap();
    let (s, v) = call(&app, "POST", &format!("/api/decks/{name}/polish/{id}"), Some(json!({ "action": "apply" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["item"]["state"], "applied");
    assert!(v["slides"].as_array().is_some_and(|a| !a.is_empty()));
    let (s, v) = call(&app, "POST", &format!("/api/decks/{name}/polish/{id}"), Some(json!({ "action": "undo" }))).await;
    assert_eq!((s, v["item"]["state"].as_str()), (StatusCode::OK, Some("undone")));
    // a manual item cannot be applied
    let manual = items.iter().find(|i| i["state"] == "manual").unwrap()["id"].as_u64().unwrap();
    let (s, _) = call(&app, "POST", &format!("/api/decks/{name}/polish/{manual}"), Some(json!({ "action": "apply" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // the saved report remembers the states
    let (_, v) = call(&app, "GET", &format!("/api/decks/{name}/polish"), None).await;
    assert_eq!(v["items"].as_array().unwrap().iter().find(|i| i["id"].as_u64() == Some(id)).unwrap()["state"], "undone");
}

#[tokio::test]
async fn no_model_skips_only_the_term_step() {
    let tmp = tempfile::tempdir().unwrap();
    let (ws, st) = blank_style(tmp.path());
    let name = ws.new_deck("messy", "blank").unwrap();
    std::fs::write(ws.decks.join(&name).join("deck.json"), serde_json::to_vec(&messy_deck(&st)).unwrap()).unwrap();
    let home = lore_home::Home::at(tmp.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, MockFactory::default())).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let (s, rep) = call(&app, "POST", &format!("/api/decks/{name}/polish"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{rep}");
    assert!(rep["items"].as_array().unwrap().iter().any(|i| i["state"] == "applied"));
    assert_eq!(rep["notes"].as_array().map(Vec::len), Some(1), "{rep}");
}

/// Manual look at real decks, in memory only (never in CI; prints counts, never text):
/// `SC_PROFILE_DECKS=<decks dir> SC_WORKSPACE=<workspace with styles/> cargo test -p sc-server --test polish real -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_decks_counts() {
    let (Ok(d), Ok(root)) = (std::env::var("SC_PROFILE_DECKS"), std::env::var("SC_WORKSPACE")) else { return };
    let tmp = tempfile::tempdir().unwrap();
    let p = deckstyle::load_or_build(Path::new(&d), tmp.path());
    let ws = sc_core::Workspace::new(Path::new(&root), Some(tmp.path().join("decks")));
    for e in std::fs::read_dir(&d).unwrap().flatten() {
        let Ok(b) = std::fs::read(e.path().join("deck.json")) else { continue };
        let deck: Deck = serde_json::from_slice(&b).unwrap();
        let st = deck.style.as_ref().and_then(|s| ws.style(s).ok());
        let roles = deckpolish::roles_for(&deck, &[]);
        let n = deck.slides.len();
        if std::env::var("SC_RATIOS").is_ok() {
            for s in &deck.slides {
                for o in &s.objects {
                    if let (Some(r), Some(b)) = (deckpolish::fill_ratio(o), deckstyle::body_of(o))
                        && r > 1.05
                        && !b.plain_text().trim().is_empty()
                    {
                        let chars = b.plain_text().chars().count();
                        let lines = b.paras.len();
                        let sizes: Vec<f64> = b.paras.iter().flat_map(|p| p.runs.iter().map(|r| r.look.size)).collect();
                        println!("  ratio {r:.2} ph {:?} autofit {} scale {} wrap {} chars {chars} paras {lines} h {} w {} sizes {:?} ins {:?}", o.ph, b.autofit, b.font_scale, b.wrap, o.frame.h / 12700, o.frame.w / 12700, &sizes[..sizes.len().min(3)], b.insets);
                    }
                }
            }
        }
        let mut ed = Editor::new(deck, st);
        let rep = deckpolish::run_checks(&mut ed, &Input { profile: p.as_ref(), notation: &Notation::default(), roles });
        let mut c = std::collections::BTreeMap::new();
        for i in &rep.items {
            *c.entry(format!("{}-{}-{}", i.check, i.state, i.message.chars().take(8).collect::<String>())).or_insert(0) += 1;
        }
        println!("{n} slides: {c:?}");
    }
}
