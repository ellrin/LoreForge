//! Saved blocks and their words: where lines connect on a saved drawing, line breaks and
//! wrapping of a block's words, named colours for words, `size=` as a minimum, parameters no
//! kind reads. Synthetic blocks only.

use lore_canvas::{Frame, Geom, Kind, Object, Paint, SpObj, Stroke};
use lore_flow::layout::Rect;
use lore_flow::modules::{Body, Ctx, EMU_PT, ModuleTemplate, make, no_images};
use lore_flow::*;

const PT: i64 = 12_700;

fn ty() -> lore_flow::text::Type {
    lore_flow::text::Type { font: "Arial".into(), ea_font: String::new(), math_font: "Cambria Math".into(), size: 10.0, bold: false, italic: false, color: "000000".into() }
}

/// A shape at (x, y, w, h) pt with a fill and an outline.
fn drawn(name: &str, x: i64, y: i64, w: i64, h: i64) -> Object {
    Object { id: 0, name: name.into(), frame: Frame::new(x * PT, y * PT, w * PT, h * PT), hidden: false, ph: None, slot: None, kind: Kind::Shape(SpObj { geom: Geom::preset("rect"), fill: Paint::solid("role:fill"), line: Some(Stroke::new("role:line", 1.0)), text: None, equation: false }), src: None }
}

/// Words alone (no fill, no outline) at (x, y, w, h) pt.
fn words(name: &str, x: i64, y: i64, w: i64, h: i64, s: &str) -> Object {
    let (tb, _) = lore_flow::text::body(s, &ty(), &Vars::default(), &NoMath, "ctr", "ctr");
    Object { id: 0, name: name.into(), frame: Frame::new(x * PT, y * PT, w * PT, h * PT), hidden: false, ph: None, slot: None, kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(tb), equation: false }), src: None }
}

fn library(kind: &str, w: f64, h: f64, objs: Vec<Object>) -> Library {
    let v = serde_json::to_string(&objs).unwrap().replace("ROLE:FILL", "role:fill").replace("ROLE:LINE", "role:line");
    let t = ModuleTemplate { kind: kind.into(), style: "x".into(), w, h, objects: serde_json::from_str(&v).unwrap(), ..Default::default() };
    let mut lib = Library::default();
    lib.add(t).unwrap();
    lib
}

fn body(src: &str, lib: &Library) -> Body {
    let g = dsl::parse_with(src, &lib.kinds()).unwrap();
    body_of(&g.nodes[0], lib)
}

fn body_of(n: &Node, lib: &Library) -> Body {
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let cx = Ctx { theme: &theme, vars: &vars, math: &NoMath, lib, k: 1.0, symmetric: false, image_size: &no_images };
    make(n, &cx)
}

/// The paragraphs of the part named `…/name` (or `name`), as plain text.
fn paras(b: &Body, name: &str) -> Vec<String> {
    let o = b.objs.iter().find(|o| o.name == name || o.name.ends_with(&format!("/{name}"))).unwrap_or_else(|| panic!("no part {name}"));
    o.kind.sp().and_then(|s| s.text.as_ref()).map(|tb| tb.paras.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect()).unwrap_or_default()
}

fn port(b: &Body) -> (f64, f64, f64, f64) {
    let p = b.objs.iter().find(|o| o.name == "port").expect("a port");
    let f = &p.frame;
    (f.x as f64 / EMU_PT, f.y as f64 / EMU_PT, f.w as f64 / EMU_PT, f.h as f64 / EMU_PT)
}

/// A 40 × 60 pt saved block: a 40 × 30 picture on top, its caption (words alone) below.
fn captioned() -> Library {
    library("captioned", 40.0, 60.0, vec![drawn("pic", 0, 0, 40, 30), words("cap", 0, 34, 40, 20, "Caption")])
}

#[test]
fn a_saved_blocks_art_is_its_drawing_not_its_box() {
    let lib = captioned();
    let b = body("fig f\nn: captioned \"\" style=x\n", &lib);
    assert!((b.h - 60.0).abs() < 0.5, "the block keeps its room: {}", b.h);
    let (x, y, w, h) = b.art;
    assert!(x.abs() < 0.1 && y.abs() < 0.1 && (w - 40.0).abs() < 0.1 && (h - 30.0).abs() < 0.1, "art {:?}", b.art);
    assert_eq!(port(&b), b.art, "the port child frames the picture");
    // a line leaving the block downwards leaves the picture, not the caption's room under it
    let src = "fig f\npanel p \"P\"\n  x: captioned \"\" style=x col=0 lane=0\n  a: box \"A\" col=0 lane=1\n  x -> a : data\n";
    let g = dsl::parse_with(src, &lib.kinds()).unwrap();
    let opt = layout::LayoutOptions { frame: Rect::new(0.0, 0.0, 600.0, 300.0), ..Default::default() };
    let lay = layout::layout(&g, &Theme::classic_bw(), &Vars::default(), &lib, &opt);
    let routes = route::route(&g, &lay);
    let xn = &lay.nodes[g.node_index("x").unwrap()];
    assert!(xn.art.bottom() < xn.rect.bottom() - 10.0 * lay.scale, "art {:?} in {:?}", xn.art, xn.rect);
    let start = routes[0].points[0];
    assert!((start.1 - xn.art.bottom()).abs() < 1e-6, "the line starts at the picture's bottom: {start:?} {:?}", xn.art);
    // grown, the art grows with the block
    let mut n = g.nodes[0].clone();
    n.params.insert("grow".into(), serde_json::json!(1.5));
    let big = body_of(&n, &lib);
    assert!((big.art.2 - 60.0).abs() < 0.2 && (big.art.3 - 45.0).abs() < 0.2, "grown art {:?}", big.art);
}

#[test]
fn a_block_without_spare_room_keeps_its_box() {
    let lib = library("full", 40.0, 30.0, vec![drawn("a", 0, 0, 40, 15), drawn("b", 0, 15, 40, 15)]);
    let b = body("fig f\nn: full \"\" style=x\n", &lib);
    let (x, y, w, h) = b.art;
    assert!(x.abs() < 0.1 && y.abs() < 0.1 && (w - b.w).abs() < 0.1 && (h - b.h).abs() < 0.1, "art {:?} of {} × {}", b.art, b.w, b.h);
    // an invisible part is no drawing, and a drawing with its own port keeps it
    let lib = library("hidden", 40.0, 40.0, vec![drawn("a", 0, 0, 40, 20), { let mut o = drawn("b", 0, 20, 40, 20); o.hidden = true; o }]);
    assert!((body("fig f\nn: hidden \"\" style=x\n", &lib).art.3 - 20.0).abs() < 0.1);
    let mut lib = captioned();
    let t = lib.templates.get_mut("captioned").unwrap().get_mut("x").unwrap();
    t.port = Some(lore_flow::modules::PortRect { x: 0, y: 0, w: 40 * PT, h: 60 * PT });
    let b = body("fig f\nn: captioned \"\" style=x\n", &lib);
    assert!((b.art.3 - 60.0).abs() < 0.1, "the author's port stays: {:?}", b.art);
}

/// A 200 pt wide part with the label, and a 200 pt wide part with fixed words.
fn labelled(width: i64) -> Library {
    library("labelled", width as f64, 60.0, vec![drawn("pic", 0, 0, width, 30), words("name", 0, 30, width, 14, "{{label}}"), words("cap", 0, 44, width, 14, "Fixed")])
}

#[test]
fn a_saved_blocks_label_breaks_at_double_backslash() {
    let lib = labelled(200);
    let b = body("fig f\nn: labelled \"Top \\\\ Bottom\" style=x\n", &lib);
    assert_eq!(paras(&b, "name"), ["Top", "Bottom"]);
    // a block's own words for a fixed text break the same way
    let b = body("fig f\nn: labelled \"L\" style=x text.cap=One\\\\Two\n", &lib);
    assert_eq!(paras(&b, "cap"), ["One", "Two"]);
    let b = body("fig f\nn: labelled \"L\" style=x texts=[A\\\\B]\n", &lib);
    assert_eq!(paras(&b, "cap"), ["A", "B"]);
    // each line is its own paragraph made like the first
    let o = b.objs.iter().find(|o| o.name.ends_with("cap")).unwrap();
    let ps = &o.kind.sp().unwrap().text.as_ref().unwrap().paras;
    assert_eq!(ps[0].align, ps[1].align);
    // and without a break the label is one line, as before
    assert_eq!(paras(&body("fig f\nn: labelled \"Plain\" style=x\n", &lib), "name"), ["Plain"]);
}

#[test]
fn long_words_in_a_saved_block_wrap_to_its_part() {
    let lib = labelled(60);
    let n = Node { id: "n".into(), module: "labelled".into(), style: "x".into(), label: "A long label that does not fit in one line".into(), ..Default::default() };
    let b = body_of(&n, &lib);
    let lines = paras(&b, "name");
    assert!(lines.len() >= 3, "{lines:?}");
    assert_eq!(lines.join(" "), "A long label that does not fit in one line", "no word is lost or cut");
    for l in &lines {
        assert!(lore_flow::text::width(l, 10.0) <= 60.5, "{l:?} is wider than its part");
    }
    // the part keeps its size
    let part = b.objs.iter().find(|o| o.name.ends_with("name")).unwrap();
    assert_eq!(part.frame.w, 60 * PT);
    // one word wider than the part stays whole
    let one = Node { label: "Supercalifragilisticexpialidocious".into(), ..n.clone() };
    assert_eq!(paras(&body_of(&one, &lib), "name"), ["Supercalifragilisticexpialidocious"]);
    // math is never broken inside
    let m = Node { label: "Loss $a + b + c + d + e$ here".into(), ..n.clone() };
    // (NoMath writes the math as plain runs without its spaces)
    let ml = paras(&body_of(&m, &lib), "name");
    assert!(ml.iter().any(|l| l.contains("a+b+c+d+e")) && !ml.iter().any(|l| l.ends_with('+') || l.starts_with('+')), "{ml:?}");
    // replaced words wrap too
    let mut r = n.clone();
    r.label = "L".into();
    r.params.insert("text.cap".into(), "Words given by the block for the fixed text".into());
    let caps = paras(&body_of(&r, &lib), "cap");
    assert!(caps.len() >= 3 && caps.iter().all(|l| lore_flow::text::width(l, 10.0) <= 60.5), "{caps:?}");
    // fixed words of the drawing are left as they were saved
    let mut wide = labelled(60);
    wide.templates.get_mut("labelled").unwrap().get_mut("x").unwrap().objects = serde_json::to_value(vec![words("cap", 0, 0, 60, 14, "Fixed words much wider than the part")]).unwrap();
    assert_eq!(paras(&body_of(&Node { label: String::new(), ..n.clone() }, &wide), "cap"), ["Fixed words much wider than the part"]);
}

fn colours(b: &Body) -> Vec<String> {
    b.objs.iter().filter_map(|o| o.kind.sp().and_then(|s| s.text.as_ref())).flat_map(|tb| tb.paras.iter().flat_map(|p| p.runs.iter().map(|r| r.look.color.to_uppercase()))).collect()
}

#[test]
fn white_text_works() {
    let lib = Library::default();
    let b = body("fig f\nn: box \"A\" fill=black text=white\n", &lib);
    assert!(!colours(&b).is_empty() && colours(&b).iter().all(|c| c == "FFFFFF"), "{:?}", colours(&b));
    assert!(b.warn.is_empty(), "{:?}", b.warn);
    let b = body("fig f\nn: box \"A\" text=black\n", &lib);
    assert!(colours(&b).iter().all(|c| c == "000000"), "{:?}", colours(&b));
    // white is a colour for fills and lines too
    let b = body("fig f\nn: box \"A\" fill=white line=black\n", &lib);
    let sp = b.objs[0].kind.sp().unwrap();
    assert_eq!(sp.fill, Paint::solid("FFFFFF"));
    // and in a saved drawing whose words take the role's colour (as saved blocks' labels do)
    let mut lib = labelled(200);
    let t = lib.templates.get_mut("labelled").unwrap().get_mut("x").unwrap();
    t.objects = serde_json::from_str(&t.objects.to_string().replace("\"000000\"", "\"role:text\"")).unwrap();
    let run_colour = |b: &Body| {
        let o = b.objs.iter().find(|o| o.name.ends_with("name")).unwrap();
        o.kind.sp().unwrap().text.as_ref().unwrap().paras[0].runs[0].look.color.to_uppercase()
    };
    assert_eq!(run_colour(&body("fig f\nn: labelled \"Name\" style=x text=white\n", &lib)), "FFFFFF");
    // a colour the drawing fixed for its words stays
    let fixed = labelled(200);
    assert_eq!(run_colour(&body("fig f\nn: labelled \"Name\" style=x text=white\n", &fixed)), "000000");
}

#[test]
fn an_unknown_text_colour_is_a_warning() {
    let lib = Library::default();
    let b = body("fig f\nn: box \"A\" text=whitish\n", &lib);
    assert_eq!(b.warn, ["n: 不認得的顏色 whitish"]);
    // fill= and line= say so too; a known name or a hex says nothing
    let b = body("fig f\nn: box \"A\" fill=bluu line=C62828 text=white\n", &lib);
    assert_eq!(b.warn, ["n: 不認得的顏色 bluu"]);
    // in the drawn figure's warnings, once
    let g = dsl::parse("fig f\nn: box \"A\" text=whitish\n").unwrap();
    let f = draw(&g, &Drawing::default()).unwrap();
    assert_eq!(f.built.warnings.iter().filter(|w| w.contains("whitish")).count(), 1, "{:?}", f.built.warnings);
}

#[test]
fn size_is_a_minimum_growth_may_exceed_it() {
    // panel a is four blocks high; panel b beside it holds one picture of a given size
    let src = |p: &str| format!("fig f\narrange a b\npanel a \"A\"\n  x: box \"X\" col=0 lane=0\n  y: box \"Y\" col=0 lane=1\n  z: box \"Z\" col=0 lane=2\n  w: box \"W\" col=0 lane=3\n  x -> y -> z -> w : data\npanel b \"B\"\n  c: image \"\" {p}\n");
    let opt = layout::LayoutOptions { frame: Rect::new(0.0, 0.0, 600.0, 400.0), ..Default::default() };
    let d = Drawing { options: opt.clone(), ..Default::default() };
    let g = dsl::parse(&src("size=30")).unwrap();
    let f = draw(&g, &d).unwrap();
    let ci = g.node_index("c").unwrap();
    let grown = f.layout.nodes[ci].art.h / f.layout.scale;
    assert!(grown > 30.0 * 1.15, "a size= picture grows past its size: {grown}");
    // the size is the smallest it is drawn
    assert!((body("fig f\nc: image \"\" size=30\n", &Library::default()).w - 30.0).abs() < 0.1);
    // w= and h= stay exact
    let g = dsl::parse(&src("w=30 h=30")).unwrap();
    let f = draw(&g, &d).unwrap();
    let exact = f.layout.nodes[ci].art.h / f.layout.scale;
    assert!((exact - 30.0).abs() < 0.5, "w= h= keep their size: {exact}");
    assert!(!lore_flow::modules::growable(&g.nodes[ci]));
}

#[test]
fn a_tiny_size_is_a_warning() {
    let lib = Library::default();
    let b = body("fig f\nc: image \"\" size=1.5\n", &lib);
    assert_eq!(b.warn, ["c: size 是點數（pt），不是倍數"]);
    assert!(body("fig f\nc: image \"\" size=30\n", &lib).warn.is_empty());
    // an operator's size is its own business
    assert!(body("fig f\no: op \"+\" size=6\n", &lib).warn.is_empty());
    // a saved block too, and in the figure's warnings once
    let lib = captioned();
    assert!(body("fig f\nn: captioned \"\" style=x size=2\n", &lib).warn.iter().any(|w| w == "n: size 是點數（pt），不是倍數"));
    let g = dsl::parse("fig f\nc: image \"\" size=2\n").unwrap();
    let f = draw(&g, &Drawing::default()).unwrap();
    assert_eq!(f.built.warnings.iter().filter(|w| w.contains("size 是點數")).count(), 1, "{:?}", f.built.warnings);
}

#[test]
fn an_unknown_parameter_is_a_warning() {
    let mut g = dsl::parse("fig f\nx: image \"X\" colour=red sizee=30 look=mask size=30 col=0 lane=0\nn: nn \"N\" layers=[3,4,2] finish=outline\nb: box \"B\" text.cap=Hi texts=[a] fill=blue bold\nx -> n -> b\n").unwrap();
    let rep = lore_flow::notation::check_graph(&mut g, &lore_notation::Notation::default());
    let found = rep.by_code("unknown_param");
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|f| f.node.as_deref() == Some("x")));
    assert!(found.iter().any(|f| f.message.contains("x") && f.message.contains("\"colour\"")), "{found:?}");
    assert!(found.iter().any(|f| f.message.contains("\"sizee\"")), "{found:?}");
    // the crate's own parameters are not the author's mistakes
    let mut h = dsl::parse("fig f\nl: layer-bars \"L\" layers=[Conv,Pool]\n").unwrap();
    h.nodes[0].params.insert("_classes".into(), serde_json::json!(["Conv"]));
    h.nodes[0].params.insert("grow".into(), serde_json::json!(1.2));
    assert!(lore_flow::notation::check_graph(&mut h, &lore_notation::Notation::default()).by_code("unknown_param").is_empty());
}

#[test]
fn a_block_says_what_it_replaces_and_what_it_prints() {
    use lore_flow::modules::{Library, ModuleTemplate};
    let mut lib = Library::default();
    lib.add_json(r#"{ "kind": "u-ladder", "style": "bars", "description": "an encoder-decoder as upright bars", "names": ["U-Net", "U 形"], "replaces": ["unet", "chart=membership"], "w": 60, "h": 40,
        "objects": [ { "id": 0, "name": "t", "type": "shape", "frame": { "x": 0, "y": 0, "w": 508000, "h": 254000 }, "geom": { "prst": "rect" }, "fill": { "kind": "none" },
          "text": { "insets": [0,0,0,0], "anchor": "ctr", "wrap": true, "paras": [ { "align": "ctr", "runs": [ { "kind": "text", "text": "× N", "look": { "font": "Arial", "size": 10, "color": "role:text" } } ] } ] } } ] }"#)
        .unwrap();
    let t: &ModuleTemplate = lib.get("u-ladder", "bars").unwrap();
    assert_eq!(t.fixed_texts(), vec!["× N".to_string()]);
    let g = lore_flow::dsl::parse_with("fig f\na: unet \"Seg\"\nb: chart \"M\" chart=membership\nc: chart \"S\" chart=bars\nd: u-ladder \"Encoder × N\" style=bars\n", &lib.kinds()).unwrap();
    assert!(t.stands_in_for(&g.nodes[0]) && t.stands_in_for(&g.nodes[1]) && !t.stands_in_for(&g.nodes[2]));
    let advice = lore_flow::llm::block_advice(&g, &lib);
    assert_eq!(advice.len(), 3, "{advice:?}");
    assert!(advice[0].contains("block a uses the built-in unet") && advice[0].contains("u-ladder style=bars"));
    assert!(advice[2].contains("already prints '× N'"));
}

#[test]
fn a_block_says_what_it_already_draws() {
    use lore_flow::modules::Library;
    let mut lib = Library::default();
    lib.add_json(r#"{ "kind": "head", "style": "scores", "description": "a backbone, its scores and their brace", "includes": ["layer-bars", "brace"], "w": 60, "h": 40, "objects": [] }"#).unwrap();
    // layer-bars → head → brace → label: the bars and the brace are drawn twice; the label is not
    let g = lore_flow::dsl::parse_with(
        "fig f\nx: image \"$x$\"\nl: layer-bars \"\" layers=[Conv,FC]\nh: head \"Q\" style=scores\nb: brace \"\"\nq: label \"q\"\nfar: layer-bars \"\" layers=[Conv]\nx -> l -> h -> b -> q\nx -> far\n",
        &lib.kinds(),
    )
    .unwrap();
    let advice = lore_flow::llm::block_advice(&g, &lib);
    assert_eq!(advice.len(), 2, "{advice:?}");
    assert!(advice.iter().any(|a| a.contains("block h (head) already draws its layer-bars") && a.contains("remove l")), "{advice:?}");
    assert!(advice.iter().any(|a| a.contains("already draws its brace") && a.contains("remove b")), "{advice:?}");
    // a block of the same kind that is not joined to the head is another thing
    assert!(!advice.iter().any(|a| a.contains("far")), "{advice:?}");
}

#[test]
fn a_dropped_word_of_a_slot_the_description_uses_is_named() {
    use lore_flow::modules::Library;
    let mut lib = Library::default();
    let run = |t: &str| format!(r#"{{ "id": 0, "name": "t", "type": "shape", "frame": {{ "x": 0, "y": 0, "w": 508000, "h": 127000 }}, "geom": {{ "prst": "rect" }}, "fill": {{ "kind": "none" }}, "text": {{ "insets": [0,0,0,0], "anchor": "ctr", "wrap": true, "paras": [ {{ "align": "l", "runs": [ {{ "kind": "text", "text": "{t}", "look": {{ "font": "Arial", "size": 10, "color": "role:text" }} }} ] }} ] }} }}"#);
    lib.add_json(&format!(r#"{{ "kind": "card", "style": "two", "description": "a rule card", "w": 60, "h": 40, "objects": [ {}, {} ] }}"#, run("Antecedent:"), run("Consequent:"))).unwrap();
    let g = lore_flow::dsl::parse_with("fig f\nr: card \"\" style=two texts=[\"Product rule\",\"Consequent: Choquet integral\"]\n", &lib.kinds()).unwrap();
    // the description writes the heading: dropping it is named; the slot written in full is not
    let advice = lore_flow::llm::block_advice_for(&g, &lib, "A box: Antecedent: Product rule, then Consequent: Choquet integral.");
    assert_eq!(advice.len(), 1, "{advice:?}");
    assert!(advice[0].contains("block r") && advice[0].contains("'Antecedent'") && advice[0].contains("'Product rule'"), "{advice:?}");
    // a description that does not use the block's word: the slot is a placeholder, free to replace
    assert!(lore_flow::llm::block_advice_for(&g, &lib, "A box with the product rule and the Choquet integral.").iter().all(|a| !a.contains("Antecedent")));
    assert!(lore_flow::llm::block_advice(&g, &lib).is_empty());
}
