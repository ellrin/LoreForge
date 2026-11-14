//! The editor's figure spec, layout templates, legend choice, the discussion and the
//! assistant — with a scripted model (no engine is ever called) and synthetic figures.

use std::sync::{Arc, Mutex};

use lore_flow::figure::*;
use lore_flow::llm::Llm;
use lore_flow::*;
use lore_llm::LlmRequest;
use serde_json::{Value, json};

/// A synthetic two-stage segmentation pipeline (no real paper).
pub const SEG2: &str = r#"fig seg2
panel a "a. Coarse localisation"
  x: image "$\mathbf{x}$"
  det: encoder "Localiser"
  roi: box "ROI crop"
  x -> det -> roi
end
panel b "b. Fine segmentation"
  crop: image "$\mathbf{x}_{r}$"
  seg: unet "Segmenter"
  yhat: image "$\hat{\mathbf{y}}$" look=mask
  loss: loss "$\mathcal{L}_{seg}$"
  y: image "$\mathbf{y}$" role=gt look=mask
  crop -> seg -> yhat -> loss
  y -> loss : supervision
  loss -> seg : feedback
end
roi -> crop : cross
"#;

fn spec() -> FlowSpec {
    FlowSpec { lore_flow: 1, dsl: SEG2.into(), ..Default::default() }
}

/// Replies by task, records what was asked.
struct Scripted {
    replies: Mutex<Vec<Value>>,
    asked: Mutex<Vec<(String, String)>>,
}

impl Scripted {
    fn new(replies: Vec<Value>) -> Scripted {
        Scripted { replies: Mutex::new(replies), asked: Mutex::new(vec![]) }
    }
}

#[async_trait::async_trait]
impl Llm for Scripted {
    async fn complete_json(&self, task: &str, req: LlmRequest) -> Result<(Value, String), String> {
        self.asked.lock().unwrap().push((task.to_string(), format!("{} {:?}", req.system.clone(), req.messages)));
        let mut r = self.replies.lock().unwrap();
        if r.is_empty() {
            return Err("no more replies".into());
        }
        Ok((r.remove(0), "scripted".into()))
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().build().unwrap()
}

fn names(fig: &Figure) -> Vec<String> {
    fig.objects().iter().map(|o| o.name.clone()).collect()
}

#[test]
fn the_spec_draws_and_lists_each_block_as_a_line() {
    let cat = Catalogue::builtin();
    let s = spec();
    let g = s.graph(&cat.library).unwrap();
    let l = linear(&g);
    assert_eq!(l[0], "block 1 (a. Coarse localisation): x -> det -> roi");
    assert_eq!(l[1], "block 2 (b. Fine segmentation): crop -> seg -> yhat -> loss ; y");
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    assert!(names(&fig).contains(&"flow:seg2/seg".to_string()));
    // a spec survives JSON, unknown keys included
    let mut v = s.to_value();
    v["later"] = json!({ "x": 1 });
    let back = FlowSpec::from_value(v.clone()).unwrap();
    assert_eq!(back.to_value()["later"], json!({ "x": 1 }));
    assert!(FlowSpec::from_value(json!({ "lore_flow": 99, "dsl": "" })).is_err());
}

#[test]
fn layout_templates_place_blocks_and_turn_them() {
    let cat = Catalogue::builtin();
    // every public template parses and the rect ones are valid panel templates
    assert!(cat.layouts.len() >= 15);
    for l in &cat.layouts {
        if !l.panels.is_empty() {
            PanelTemplate::from_json(&json!({ "id": l.id, "panels": l.panels }).to_string()).unwrap();
        }
    }
    let mut s = spec();
    // two blocks: small left, big right
    s.layout = "small-big".into();
    let (g, _, opt) = s.prepare(&cat, s.frame_rect()).unwrap();
    assert_eq!(g.template, "small-big");
    assert!(opt.template.is_some());
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    let (a, b) = (&fig.layout.panels[0].rect, &fig.layout.panels[1].rect);
    assert!(a.w < b.w && a.right() <= b.x + 0.1, "{a:?} {b:?}");
    // a three-block template does not fit two blocks: ignored
    s.layout = "two-left-big-right".into();
    let (g, _, _) = s.prepare(&cat, s.frame_rect()).unwrap();
    assert!(g.template.is_empty());
    // "block 2 vertical": the per-block direction wins over the template
    s.layout = "row".into();
    s.dirs.insert("b".into(), "down".into());
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    assert!(fig.layout.panels[1].down && !fig.layout.panels[0].down);
}

#[test]
fn block_types_colour_panels_and_nodes_and_the_legend_is_the_authors() {
    let cat = Catalogue::builtin();
    let mut s = spec();
    s.types = vec![BlockType { id: "conv".into(), label: "conv block".into(), fill: "F9D5E5".into(), line: "B03A6E".into() }];
    s.blocks.insert("b".into(), "conv".into());
    s.nodes.insert("det".into(), "conv".into());
    s.lines.insert("supervision".into(), LineLook { color: Some("2E7D32".into()), dash: Some("dash".into()), width: None, meaning: "ground truth".into() });
    let g = s.graph(&cat.library).unwrap();
    assert!(s.problems(&g).is_empty());
    let sugg = s.legend_suggestions(&g);
    assert_eq!(sugg[0].key, "type:conv");
    assert!(sugg.iter().any(|r| r.key == "line:supervision" && r.label == "ground truth"));
    s.legend = LegendSpec { show: "on".into(), pos: "below".into(), rows: sugg.clone() };
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    let objs = fig.objects();
    // the panel of block b is pink with a dark pink frame
    let pb = objs.iter().find(|o| o.name == "flow:seg2/panel:b").expect("panel b");
    let json = serde_json::to_string(pb).unwrap();
    assert!(json.contains("F9D5E5") && json.contains("B03A6E"), "{json}");
    // the legend: a pink box and a green dashed line, under the figure
    let lg: Vec<_> = objs.iter().filter(|o| o.name.contains("/legend:")).collect();
    assert!(lg.iter().any(|o| serde_json::to_string(o).unwrap().contains("2E7D32")));
    let texts: String = lg.iter().map(|o| serde_json::to_string(o).unwrap()).collect();
    assert!(texts.contains("conv block") && texts.contains("ground truth"));
    let bottom_panel = fig.layout.panels.iter().map(|p| p.rect.bottom()).fold(0.0, f64::max);
    assert!(fig.layout.legend.iter().all(|l| l.text.y >= bottom_panel - 0.5));
    // above
    s.legend.pos = "above".into();
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    let top_panel = fig.layout.panels.iter().map(|p| p.rect.y).fold(f64::MAX, f64::min);
    assert!(fig.layout.legend.iter().all(|l| l.text.bottom() <= top_panel + 0.5));
    // off: no legend at all
    s.legend.show = "off".into();
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    assert!(!names(&fig).iter().any(|n| n.contains("/legend:") || n.contains("/colours:")));
    // problems are said in the author's words
    s.blocks.insert("zz".into(), "nope".into());
    let p = s.problems(&g);
    assert!(p.iter().any(|x| x.contains("zz")) && p.iter().any(|x| x.contains("nope")));
}

#[test]
fn as_picture_is_one_locked_group_carrying_its_spec() {
    let cat = Catalogue::builtin();
    let s = spec();
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 10).unwrap();
    let max = fig.built.next_id;
    let o = as_picture(&fig, "seg2", &s, max);
    assert_eq!(o.name, "flow:seg2");
    let lore_canvas::Kind::Group(g) = &o.kind else { panic!() };
    assert!(g.lock_aspect);
    assert_eq!(FlowSpec::from_value(g.data.clone().unwrap()).unwrap(), s);
    assert_eq!(g.ch, o.frame);
}

#[test]
fn private_templates_and_colours_join_the_public_ones() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("layouts")).unwrap();
    std::fs::write(dir.path().join("layouts/mine.json"), r#"[{ "id": "mine-2", "name": "我的兩格", "panels": [{"x":0,"y":0,"w":0.5,"h":1},{"x":0.5,"y":0,"w":0.5,"h":1}] }, { "id": "bad", "panels": [{"x":0,"y":0,"w":2,"h":1}] }]"#).unwrap();
    std::fs::write(dir.path().join("palette.json"), r#"[["mine", "112233", "445566"]]"#).unwrap();
    // an app's own flow/ folder (SlideComposer's templates/ and themes/) is read after the shared one
    let app = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(app.path().join("templates")).unwrap();
    std::fs::write(app.path().join("templates/one-over-two.json"), r#"{ "id": "one-over-two", "name": "一上二下", "panels": [{"x":0,"y":0,"w":1,"h":0.48},{"x":0,"y":0.52,"w":0.49,"h":0.48},{"x":0.51,"y":0.52,"w":0.49,"h":0.48}] }"#).unwrap();
    std::fs::create_dir_all(app.path().join("themes")).unwrap();
    std::fs::write(app.path().join("themes/soft.toml"), "id = \"soft\"\nname = \"Soft\"\n[panel]\nfill = \"EEF4FB\"\n").unwrap();
    let flow = web::Flow::bare().with_dir(dir.path().to_path_buf()).with_app_dir(app.path().to_path_buf());
    let c2 = flow.catalogue();
    assert!(c2.layout("one-over-two").is_some() && c2.themes.iter().any(|t| t.0 == "soft"));
    let cat = flow.catalogue();
    assert!(cat.layout("mine-2").is_some_and(|l| l.private));
    assert!(cat.layout("bad").is_none());
    assert_eq!(flow.palette()[0][1], "112233");
    // thumbnails: the figure in every template that fits two blocks
    let v = web::layouts(&flow, &spec(), "", 120.0);
    let ids: Vec<&str> = v["layouts"].as_array().unwrap().iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"mine-2") && ids.contains(&"small-big") && !ids.contains(&"two-left-big-right"));
    assert!(v["layouts"][0]["svg"].as_str().unwrap().starts_with("<svg"));
    // the preview answer
    let p = web::preview(&flow, &spec(), "", 600.0);
    assert!(p["svg"].as_str().unwrap().contains("<svg"));
    assert_eq!(p["blocks"].as_array().unwrap().len(), 2);
    assert!(p["lines"].as_array().unwrap().iter().any(|l| l["id"] == "feedback"));
    let bad = web::preview(&flow, &FlowSpec { dsl: "x -> ".into(), ..Default::default() }, "", 600.0);
    assert!(bad["error"].is_string());
    // the editor's files are served
    assert!(web::asset("editor.js").is_some() && web::asset("katex/fonts/KaTeX_Main-Regular.woff2").is_some());
}

#[test]
fn the_discussion_asks_one_step_and_keeps_the_decisions() {
    let notation = lore_notation::Notation::from_json(r#"{ "version": 1, "entries": [ { "id": "img", "kind": "symbol", "latex": "\\mathbf{x}", "meaning": "input image" } ] }"#).unwrap();
    let llm = Scripted::new(vec![json!({ "say": "兩個階段。", "options": [{ "label": "兩階段", "detail": "localise → crop → segment" }, { "label": "x", "detail": "y" }], "pick": 7, "question": "" })]);
    let answers = json!({ "start": "A two-stage segmentation pipeline.", "focus": "process" });
    let (v, engine) = rt().block_on(discuss::turn(&llm, "outline", "Title: Two-stage segmentation", &answers, Some(&notation), "")).unwrap();
    assert_eq!(engine, "scripted");
    assert_eq!(v["pick"], -1, "a pick outside the options is dropped");
    let asked = llm.asked.lock().unwrap();
    assert_eq!(asked[0].0, discuss::TASK);
    assert!(asked[0].1.contains("A two-stage segmentation pipeline.") && asked[0].1.contains("img — $\\mathf{x}$".replace("mathf", "mathbf").as_str()));
    // the vars step offers symbols; io returns rows
    assert!(discuss::schema("vars")["properties"]["options"]["items"]["required"].as_array().unwrap().iter().any(|x| x == "latex"));
    assert!(discuss::schema("io")["properties"]["rows"].is_object());
    assert!(discuss::request("nope", "", &json!({}), None, "").is_err());
    // the tasks are defined for the router
    let t = llm::task_table();
    t.check().unwrap();
    assert!(t.task.contains_key(discuss::TASK) && t.task.contains_key(assist::TASK));
}

#[test]
fn the_last_step_gives_the_l1_text() {
    let llm = Scripted::new(vec![json!({
        "fig": "seg2", "panels": [{ "id": "a", "title": "a. Coarse" }, { "id": "b", "title": "b. Fine" }],
        "nodes": [
            { "id": "x", "module": "image", "label": "", "var": "img", "panel": "a" },
            { "id": "det", "module": "box", "label": "Localiser", "panel": "a" },
            { "id": "seg", "module": "unet", "label": "Segmenter", "panel": "b" },
            { "id": "yhat", "module": "image", "label": "", "var": "yhat", "panel": "b" }
        ],
        "edges": [{ "from": "x", "to": "det", "kind": "data" }, { "from": "det", "to": "seg", "kind": "cross" }, { "from": "seg", "to": "yhat", "kind": "data" }],
        "vars_new": [{ "id": "yhat", "latex": "\\hat{\\mathbf{y}}", "meaning": "predicted mask" }]
    })]);
    let answers = json!({ "start": "Two stages.", "blocks": "block 1: localise; block 2: segment", "vars": [{ "id": "img", "latex": "\\mathbf{x}", "meaning": "image", "registered": true }] });
    let p = rt().block_on(discuss::graph(&llm, "Two-stage segmentation", &answers, None, &Library::default(), &[])).unwrap();
    assert!(p.dsl.contains("panel a") && p.dsl.contains("x -> det"));
    let asked = llm.asked.lock().unwrap();
    assert_eq!(asked[0].0, llm::TASK);
    assert!(asked[0].1.contains("block 1: localise"));
}

#[test]
fn the_assistant_changes_only_spec_values() {
    let cat = Catalogue::builtin();
    // "block 2 改成直的, legend 放下面"
    let llm = Scripted::new(vec![json!({ "note": "第二個 block 改成由上到下，圖例放在下面。", "question": "", "changes": [
        { "path": "/dirs/b", "value": "down", "why": "第二個 block" },
        { "path": "/legend/pos", "value": "below" } ] })]);
    let v = rt().block_on(assist::adjust(&llm, &cat, &spec(), "block 2 改成直的，legend 放下面", "")).unwrap();
    assert_eq!(v["spec"]["dirs"]["b"], "down");
    assert_eq!(v["spec"]["legend"]["pos"], "below");
    assert_eq!(v["changes"][0]["before"], Value::Null);
    // a change outside the spec's values is refused, the model is asked once more
    let llm = Scripted::new(vec![
        json!({ "note": "", "question": "", "changes": [{ "path": "/frame", "value": [1, 1] }] }),
        json!({ "note": "", "question": "", "changes": [{ "path": "/dirs/zz", "value": "down" }] }),
    ]);
    let e = rt().block_on(assist::adjust(&llm, &cat, &spec(), "x", "")).unwrap_err();
    assert!(e.contains("zz"), "{e}");
    assert!(!assist::allowed("/discussion") && assist::allowed("/lines/skip") && !assist::allowed("/lines/a/b"));
    // alternatives: layouts of the same content, drawn
    let llm = Scripted::new(vec![json!({ "note": "兩種。", "alternatives": [
        { "why": "左小右大：第二階段是重點。", "layout": "small-big" },
        { "why": "上下疊。", "layout": "column", "dirs": { "a": "down", "zz": "down" }, "legend_pos": "above" } ] })]);
    let v = rt().block_on(assist::alternatives(&llm, &cat, &spec(), "", "", None, 200.0)).unwrap();
    let alts = v["alternatives"].as_array().unwrap();
    assert_eq!(alts.len(), 2);
    assert_eq!(alts[1]["spec"]["dirs"], json!({ "a": "down" }));
    assert_eq!(alts[0]["spec"]["dsl"], SEG2);
    assert!(alts[0]["svg"].as_str().unwrap().starts_with("<svg"));
}

#[test]
fn a_flow_picture_scales_as_one_and_exports() {
    // insert as one locked group, scale it, export a PPTX with the attached connectors
    let cat = Catalogue::builtin();
    let s = spec();
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    let pic = as_picture(&fig, "seg2", &s, fig.built.next_id);
    let (od, deck) = lore_flow::pptx::blank_deck(720.0, 405.0).unwrap();
    let mut ed = lore_canvas::Editor::new(deck, None);
    let uid = ed.deck.slides[0].uid.clone();
    ed.apply(vec![lore_canvas::Op::InsertObj { slide: uid.clone(), parent: None, index: 0, obj: pic.clone() }], "流程圖").unwrap();
    assert_eq!(ed.undo.len(), 1);
    ed.apply(vec![lore_canvas::Op::Scale { slide: uid.clone(), id: pic.id, x: 0, y: 0, w: pic.frame.w / 2 }], "縮放").unwrap();
    let g = ed.deck.slides[0].find(pic.id).unwrap();
    assert_eq!(g.frame.w, pic.frame.w / 2);
    let bytes = lore_canvas::pptx_out::export_bytes(&ed.deck, &od, None, &|_: &str| None).unwrap();
    let back = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let cd = lore_canvas::pptx_in::import(&back, None, None);
    let g = cd.slides[0].objects.iter().find(|o| o.name == "flow:seg2").expect("the figure");
    // every attached connector end points at a shape that exists in the figure
    fn ids(o: &lore_canvas::Object, out: &mut Vec<u32>) {
        out.push(o.id);
        if let lore_canvas::Kind::Group(g) = &o.kind {
            g.children.iter().for_each(|c| ids(c, out));
        }
    }
    let mut all = vec![];
    ids(g, &mut all);
    fn ends(o: &lore_canvas::Object, out: &mut Vec<u32>) {
        match &o.kind {
            lore_canvas::Kind::Connector(c) => out.extend(c.start.iter().chain(c.end.iter()).map(|a| a.id)),
            lore_canvas::Kind::Group(g) => g.children.iter().for_each(|c| ends(c, out)),
            _ => {}
        }
    }
    let mut e = vec![];
    ends(g, &mut e);
    assert!(!e.is_empty());
    assert!(e.iter().all(|id| all.contains(id)), "{e:?}");
}

#[allow(dead_code)]
fn _unused(_: Arc<()>) {}

#[test]
fn a_paper_figure_keeps_its_latex_for_tex_glyphs_and_has_a_print_size() {
    let cat = Catalogue::builtin();
    let s = FlowSpec { frame: Some([496.0, 300.0]), ..spec() };
    let (fig, _) = render(&s, &cat, None, &TexMath, s.frame_rect(), 2).unwrap();
    let svg = svg_mm(&fig);
    // print size in mm on the root (lore-plot's PDF export reads it)
    let head = &svg[svg.find("<svg").unwrap()..];
    let head = &head[..head.find('>').unwrap()];
    assert!(head.contains("mm\"") && head.contains("width=\"") && head.contains("height=\""), "{head}");
    // each line with maths carries its LaTeX source for KaTeX
    assert!(svg.contains("data-tex=\"$\\mathbf{x}$\""), "{}", &svg[..svg.len().min(400)]);
    assert!(svg.contains("data-tex=\"$\\mathcal{L}_{seg}$\""));
    // without TexMath (previews) nothing is marked
    let (fig, _) = render(&s, &cat, None, &NoMath, s.frame_rect(), 2).unwrap();
    assert!(!preview_svg(&fig, 400.0).contains("data-tex"));
    // a line that starts with maths keeps its words in the label font; a line of maths only is TeX
    let dsl = "fig t\npanel a \"a. A\"\n  m: box \"Model\"\n  r: label \"@w and text\" role=output\n  o: label \"@w\" role=output\nend\nm -> r\nm -> o\nvar w = \"\\omega\" \"weight\"\n";
    let s = FlowSpec { lore_flow: 1, dsl: dsl.into(), frame: Some([496.0, 300.0]), ..Default::default() };
    let (fig, _) = render(&s, &cat, None, &TexMath, s.frame_rect(), 2).unwrap();
    let svg = svg_mm(&fig);
    let font_of = |tex: &str| { let at = svg.find(&format!("data-tex=\"{tex}\"")).unwrap_or_else(|| panic!("{tex}: {svg}")); svg[at..].split("data-font=\"").nth(1).unwrap().split('"').next().unwrap().to_string() };
    assert_ne!(font_of("$\\omega$ and text"), "KaTeX_Main");
    assert_eq!(font_of("$\\omega$"), "KaTeX_Main");
}

/// A model often writes a new variable as `$\mathbf{x}$`: the figure then showed the TeX source
/// ("\mathbf{x}") and the registry got the dollars. The latex is kept bare.
#[test]
fn new_variables_lose_the_maths_delimiters_a_model_adds() {
    let g = lore_flow::dsl::parse("fig t\npanel a \"a. A\"\n  x: image \"Input\" role=input var=x\n  m: box \"Model\"\nend\nx -> m\nvar x = \"$\\mathbf{x}$\" \"input\"\nvar yb = \"\\(y^{\\text{bert}}\\)\"\nvar two = \"$a$ and $b$\"\n").unwrap();
    let l: Vec<&str> = g.vars_new.iter().map(|v| v.latex.as_str()).collect();
    assert_eq!(l, vec!["\\mathbf{x}", "y^{\\text{bert}}", "$a$ and $b$"]);
    assert_eq!(lore_flow::spec::bare_latex("$$\\hat{y}$$"), "\\hat{y}");
    assert_eq!(lore_flow::spec::bare_latex("\\hat{y}"), "\\hat{y}");
}
