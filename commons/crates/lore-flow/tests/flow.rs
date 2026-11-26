//! Synthetic graphs only: parsing, layout and routing invariants, object
//! building, PPTX round trip, themes, module templates, notation checks.

use std::collections::BTreeSet;

use lore_canvas::{Editor, Kind, Object};
use lore_flow::layout::Rect;
use lore_flow::route::{Side, segment_hits};
use lore_flow::*;

const TRAIN: &str = r#"
fig demo
panel a "a. Training"
  x:    image "$\boldsymbol x$"
  aug:  box "Aug($x$)"
  net:  unet "$\theta$" depth=3
  pred: image "$\hat y$" look=mask
  loss: loss "$\mathcal L_{seg}$"
  y:    image "$y$" role=gt look=mask
panel b "b. Weights" dir=right
  q:    layer-bars "$\theta_q$" layers=[Conv,Conv,FC] taper
  v:    vector-bar "$\hat q$" n=5
  sys:  box "Fuzzy \\ system"
  w:    label "$\omega$"
end
x -> aug -> net -> pred -> loss
y -> loss : supervision
loss -> net : feedback "update"
q -> v -> sys -> w
w -> loss : cross
var omega_b = "\omega_b" "weight of a batch"
"#;

fn chain(n: usize) -> Graph {
    let mut s = String::from("fig chain\n");
    for i in 0..n {
        s.push_str(&format!("n{i}: box \"Step\"\n"));
    }
    s.push_str(&(0..n).map(|i| format!("n{i}")).collect::<Vec<_>>().join(" -> "));
    s.push('\n');
    dsl::parse(&s).unwrap()
}

fn fig(g: &Graph) -> Figure {
    draw(g, &Drawing::default()).unwrap()
}

/// The line of a connector or of a polyline.
fn stroke(o: &Object) -> &lore_canvas::Stroke {
    match &o.kind {
        Kind::Connector(c) => c.line.as_ref().unwrap(),
        Kind::Shape(sp) => sp.line.as_ref().unwrap(),
        _ => panic!("{} is not a line", o.name),
    }
}

fn walk<'a>(objs: &'a [Object], f: &mut dyn FnMut(&'a Object)) {
    for o in objs {
        f(o);
        if let Kind::Group(g) = &o.kind {
            walk(&g.children, f);
        }
    }
}

// ── L1: text form and JSON ─────────────────────────────────────────────

#[test]
fn dsl_parses_nodes_edges_panels_and_variables() {
    let g = dsl::parse(TRAIN).unwrap();
    assert_eq!(g.fig, "demo");
    assert_eq!(g.panels.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(g.panels[0].title, "a. Training");
    assert_eq!(g.nodes.len(), 10);
    let net = g.node("net").unwrap();
    assert_eq!((net.module.as_str(), net.panel.as_str(), net.num("depth")), ("unet", "a", Some(3.0)));
    let y = g.node("y").unwrap();
    assert_eq!((y.role.as_str(), y.str("look")), ("gt", Some("mask")));
    assert_eq!(g.node("q").unwrap().list("layers"), ["Conv", "Conv", "FC"]);
    assert!(g.node("q").unwrap().flag("taper"));
    // a chain is one edge per arrow; kind and label apply to each
    assert_eq!(g.edges.len(), 4 + 1 + 1 + 3 + 1);
    let fb = g.edges.iter().find(|e| e.kind == EdgeKind::Feedback).unwrap();
    assert_eq!((fb.from.as_str(), fb.to.as_str(), fb.label.as_str()), ("loss", "net", "update"));
    assert_eq!(g.vars_new, [VarNew { id: "omega_b".into(), latex: "\\omega_b".into(), meaning: "weight of a batch".into() }]);
}

#[test]
fn dsl_reports_the_line_and_the_reason() {
    let e = dsl::parse("fig a\nx: box \"X\"\nx -> y\n").unwrap_err();
    assert!(e.message.contains("\"y\""), "{e}");
    let e = dsl::parse("fig a\nx: box \"X\"\ny: box \"Y\"\nx -> y : sideways\n").unwrap_err();
    assert_eq!(e.line, 4);
    assert!(e.message.contains("sideways"), "{e}");
    let e = dsl::parse("fig a\nx: hologram \"X\"\n").unwrap_err();
    assert!(e.message.contains("hologram"), "{e}");
    let e = dsl::parse("fig a\nx: box \"unclosed\n").unwrap_err();
    assert_eq!(e.line, 2);
    assert!(dsl::parse("fig a\nx: box \"X\"\nx: box \"again\"\n").unwrap_err().message.contains("twice"));
    assert!(dsl::parse("fig a\nwhat is this\n").is_err());
}

#[test]
fn text_form_and_json_round_trip() {
    let g = dsl::parse(TRAIN).unwrap();
    let again = dsl::parse(&dsl::to_dsl(&g)).unwrap();
    assert_eq!(g, again);
    assert_eq!(Graph::from_json(&g.to_json()).unwrap(), g);
    // quotes, lists and pins survive
    let g2 = dsl::parse("fig q\narrange p / q\npanel p \"say \\\"hi\\\"\"\n a: box \"A\" col=2 lane=-1 caption=\"$512\\times512$\"\npanel q bare\n b: cuboid-seq dims=[[4,30,8],[8,20,6]]\nend\na -> b : cross \"to b\"\nlegend off\n").unwrap();
    assert_eq!(g2.panels[0].title, "say \"hi\"");
    assert_eq!(g2.arrange, [["p"], ["q"]]);
    assert_eq!(dsl::parse(&dsl::to_dsl(&g2)).unwrap(), g2);
}

#[test]
fn validation_names_what_is_wrong() {
    let mut g = chain(3);
    g.edges.push(Edge { from: "n0".into(), to: "nowhere".into(), ..Default::default() });
    assert!(matches!(g.validate(&[]), Err(SpecError::UnknownNode { .. })));
    let mut g = chain(3);
    g.nodes[1].role = "villain".into();
    assert!(matches!(g.validate(&[]), Err(SpecError::UnknownRole { .. })));
    let mut g = chain(3);
    g.nodes[1].module = "widget".into();
    assert!(matches!(g.validate(&[]), Err(SpecError::UnknownModule { .. })));
    assert!(g.validate(&["widget".to_string()]).is_ok());
    let mut g = chain(2);
    g.nodes[0].panel = "p".into();
    assert!(matches!(g.validate(&[]), Err(SpecError::UnknownPanel { .. })));
}

// ── L2: layout ─────────────────────────────────────────────────────────

fn no_overlaps(f: &Figure) {
    for (i, a) in f.layout.nodes.iter().enumerate() {
        for b in &f.layout.nodes[i + 1..] {
            assert!(!a.reserve.overlaps(&b.reserve), "{:?} overlaps {:?}", a.reserve, b.reserve);
        }
    }
}

#[test]
fn a_chain_is_centred_and_equally_spaced() {
    let f = fig(&chain(6));
    no_overlaps(&f);
    let n = &f.layout.nodes;
    // one row: the same centre line
    assert!(n.iter().all(|p| (p.port_y - n[0].port_y).abs() < 1e-6));
    assert!(n.iter().all(|p| (p.rect.cy() - n[0].rect.cy()).abs() < 1e-6));
    // left to right, equal gaps
    let gaps: Vec<f64> = n.windows(2).map(|w| w[1].rect.x - w[0].rect.right()).collect();
    assert!(gaps.iter().all(|g| *g > 0.0 && (g - gaps[0]).abs() < 1e-6), "{gaps:?}");
    // inside the frame, horizontally centred in it
    let fr = LayoutOptions::default().frame;
    assert!(fr.inflate(0.01).contains(&f.layout.bounds));
    assert!((f.layout.bounds.cx() - fr.cx()).abs() < 1e-6);
}

#[test]
fn rows_align_on_the_bodies_even_with_captions() {
    // an image with a two-line caption next to a plain box: the arrow is level
    let g = dsl::parse("fig c\na: image \"$x$\" caption=\"$512\\times512$\"\nb: box \"Model\"\nc: cuboid-seq \"$G$\" n=4\na -> b -> c\n").unwrap();
    let f = fig(&g);
    let n = &f.layout.nodes;
    assert!((n[0].port_y - n[1].port_y).abs() < 1e-6 && (n[1].port_y - n[2].port_y).abs() < 1e-6);
    // the sites lore-canvas and PowerPoint use are the frame's mid-sides: the frame is centred on the line
    assert!(n.iter().all(|p| (p.rect.cy() - p.port_y).abs() < 1e-6));
    // in a top-down panel the frame is tight instead, so arrows touch the picture and its caption
    let d = dsl::parse("fig c\npanel p \"P\" dir=down\n a: image \"$x$\" caption=\"$512\\times512$\"\n b: box \"Model\"\nend\na -> b\n").unwrap();
    let fd = fig(&d);
    assert!(fd.layout.nodes[0].rect.h / fd.layout.scale < n[0].rect.h / f.layout.scale - 5.0);
    for r in &f.routes {
        assert_eq!(r.bends(), 0, "{:?}", r.points);
    }
}

#[test]
fn branches_take_their_own_lane_and_merges_join_the_trunk() {
    let g = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\nc: op \"+\"\nd: box \"D\"\ne: box \"E\"\nside: box \"Side\"\na -> b -> c -> d -> e\nside -> c\nb -> e : skip\n").unwrap();
    let f = fig(&g);
    no_overlaps(&f);
    let n = &f.layout.nodes;
    let lane = |id: &str| n[g.node_index(id).unwrap()].lane;
    let layer = |id: &str| n[g.node_index(id).unwrap()].layer;
    assert!(["a", "b", "c", "d", "e"].iter().all(|i| lane(i) == lane("a")));
    assert_ne!(lane("side"), lane("a"));
    // an input sits right before what it feeds
    assert_eq!(layer("side") + 1, layer("c"));
    assert_eq!((0..5).map(|i| n[i].layer).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
}

#[test]
fn a_target_sits_to_the_right_of_its_loss() {
    let g = dsl::parse("fig t\nx: box \"X\"\nl: loss \"$\\mathcal L$\"\ny: image \"$y$\" role=gt\nx -> l\ny -> l : supervision\n").unwrap();
    let f = fig(&g);
    let (l, y) = (&f.layout.nodes[1], &f.layout.nodes[2]);
    assert!(y.rect.x > l.rect.right());
    assert!((y.port_y - l.port_y).abs() < 1e-6);
    let r = &f.routes[1];
    assert_eq!((r.from_side, r.to_side, r.bends()), (Side::Left, Side::Right, 0));
}

#[test]
fn panels_hold_their_nodes_and_follow_the_arrangement() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    no_overlaps(&f);
    let (a, b) = (&f.layout.panels[0], &f.layout.panels[1]);
    assert!(!a.rect.overlaps(&b.rect));
    for (n, p) in g.nodes.iter().zip(&f.layout.nodes) {
        let panel = &f.layout.panels[p.panel];
        assert_eq!(panel.id, n.panel);
        assert!(panel.rect.contains(&p.reserve), "{} outside its panel", n.id);
        assert!(!panel.title_rect.unwrap().overlaps(&p.reserve));
    }
    // explicit arrangements
    let mut rows = g.clone();
    rows.template = "rows".into();
    let f = fig(&rows);
    assert!(f.layout.panels[1].rect.y >= f.layout.panels[0].rect.bottom());
    assert!((f.layout.panels[0].rect.w - f.layout.panels[1].rect.w).abs() < 1e-6);
    let mut cols = g.clone();
    cols.arrange = vec![vec!["b".into(), "a".into()]];
    let f = fig(&cols);
    assert!(f.layout.panels[0].rect.x >= f.layout.panels[1].rect.right());
    assert!((f.layout.panels[0].rect.h - f.layout.panels[1].rect.h).abs() < 1e-6);
}

#[test]
fn a_top_down_panel_stacks_its_layers() {
    let g = dsl::parse("fig d\npanel p \"Down\" dir=down\n a: box \"A\"\n b: box \"B\"\n c: box \"C\"\nend\na -> b -> c\n").unwrap();
    let f = fig(&g);
    let n = &f.layout.nodes;
    assert!(n[1].rect.y > n[0].rect.bottom() && n[2].rect.y > n[1].rect.bottom());
    assert!((n[0].rect.cx() - n[2].rect.cx()).abs() < 1e-6);
    assert!(f.routes.iter().all(|r| r.from_side == Side::Bottom && r.to_side == Side::Top && r.bends() == 0));
}

#[test]
fn a_fraction_template_places_the_panels() {
    let t = PanelTemplate::from_json(r#"{ "id": "lr", "panels": [ { "x": 0, "y": 0, "w": 0.3, "h": 1 }, { "x": 0.35, "y": 0, "w": 0.65, "h": 1 } ] }"#).unwrap();
    let mut g = dsl::parse(TRAIN).unwrap();
    g.template = "lr".into();
    let d = Drawing { options: LayoutOptions { template: Some(t), ..Default::default() }, ..Default::default() };
    let f = draw(&g, &d).unwrap();
    let (a, b) = (&f.layout.panels[0].rect, &f.layout.panels[1].rect);
    assert!((a.w / (a.w + b.w) - 0.3 / 0.95).abs() < 1e-6);
    assert!(b.x > a.right());
    no_overlaps(&f);
    assert!(PanelTemplate::from_json(r#"{ "id": "bad", "panels": [ { "x": 0, "y": 0, "w": 0.6, "h": 1 }, { "x": 0.5, "y": 0, "w": 0.5, "h": 1 } ] }"#).is_err());
}

#[test]
fn pins_override_the_automatic_position() {
    let g = dsl::parse("fig p\na: box \"A\"\nb: box \"B\"\nc: box \"C\" col=0 lane=1\na -> b\nc -> b\n").unwrap();
    let f = fig(&g);
    let n = &f.layout.nodes;
    assert!((n[2].rect.cx() - n[0].rect.cx()).abs() < 1e-6);
    assert!(n[2].rect.y > n[0].rect.bottom());
}

#[test]
fn a_panel_without_a_direction_takes_the_one_that_fits() {
    // a long chain beside a short panel: laid top-down the figure is far larger
    let mut src = String::from("fig a\npanel p \"Long\"\n");
    for i in 0..9 {
        src.push_str(&format!(" n{i}: box \"Step {i}\"\n"));
    }
    src.push_str("panel q \"Short\" dir=right\n a: box \"A\"\n b: box \"B\"\nend\n");
    src.push_str(&(0..9).map(|i| format!("n{i}")).collect::<Vec<_>>().join(" -> "));
    src.push_str("\na -> b\nn8 -> a : cross\n");
    let g = dsl::parse(&src).unwrap();
    // a tall frame: the nine steps fit much better as a column
    let d = Drawing { options: LayoutOptions { frame: Rect::new(0.0, 0.0, 300.0, 600.0), ..Default::default() }, ..Default::default() };
    let f = draw(&g, &d).unwrap();
    assert!(f.layout.panels[0].down && !f.layout.panels[1].down);
    no_overlaps(&f);
    routes_are_clean(&g, &f);
    // in a wide frame it stays left to right; a pinned panel is never turned
    let f = fig(&g);
    assert!(!f.layout.panels[0].down);
    let mut pinned = g.clone();
    pinned.nodes[0].params.insert("col".into(), serde_json::json!(0));
    assert!(!draw(&pinned, &d).unwrap().layout.panels[0].down);
}

// ── L2: routing ────────────────────────────────────────────────────────

fn routes_are_clean(g: &Graph, f: &Figure) {
    for r in &f.routes {
        let e = &g.edges[r.edge];
        assert!(!r.forced, "{} -> {} found no path", e.from, e.to);
        let (a, b) = (g.node_index(&e.from).unwrap(), g.node_index(&e.to).unwrap());
        // starts and ends on the modules' sides
        // (on one of the side's connection points: lines that share a side are spread over them)
        let on_side = |p: (f64, f64), n: &lore_flow::layout::Placed, side: lore_flow::route::Side| {
            let mid = n.port(side);
            let r = n.rect;
            let along_x = (p.1 - mid.1).abs() < 1e-6 && p.0 >= r.x - 1e-6 && p.0 <= r.right() + 1e-6;
            let along_y = (p.0 - mid.0).abs() < 1e-6 && p.1 >= r.y - 1e-6 && p.1 <= r.bottom() + 1e-6;
            along_x || along_y
        };
        assert!(on_side(r.points[0], &f.layout.nodes[a], r.from_side), "{} -> {}: does not start on its block", e.from, e.to);
        assert!(on_side(*r.points.last().unwrap(), &f.layout.nodes[b], r.to_side), "{} -> {}: does not end on its block", e.from, e.to);
        for (si, w) in r.points.windows(2).enumerate() {
            assert!((w[0].0 - w[1].0).abs() < 1e-6 || (w[0].1 - w[1].1).abs() < 1e-6, "{} -> {}: slanted segment", e.from, e.to);
            for (ni, n) in f.layout.nodes.iter().enumerate() {
                // the first and last segment touch their own module
                let own = (ni == a && si == 0) || (ni == b && si == r.points.len() - 2);
                let body = if own { n.reserve.inflate(-0.5) } else { n.reserve };
                if own && (w[0] == r.points[0] || w[1] == *r.points.last().unwrap()) {
                    continue;
                }
                assert!(!segment_hits(w[0], w[1], &body), "{} -> {} runs through {}", e.from, e.to, g.nodes[ni].id);
            }
            for p in &f.layout.panels {
                if let Some(t) = &p.title_rect {
                    assert!(!segment_hits(w[0], w[1], t), "{} -> {} runs through the title of {}", e.from, e.to, p.id);
                }
            }
        }
    }
}

#[test]
fn no_line_runs_through_a_module() {
    for src in [
        TRAIN,
        "fig s\na: box \"A\"\nb: box \"Wide middle step\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c -> d\na -> d : skip\nd -> a : feedback\na -> c : optional\n",
        "fig f\na: image \"in\"\nb: box \"B\"\nc: box \"C\"\nd: op \"+\"\ne: box \"E\"\nf: loss \"$\\mathcal L$\"\ng: image \"gt\" role=gt\na -> b -> d -> e -> f\na -> c -> d\ng -> f : supervision\nf -> b : feedback\nf -> c : feedback\nb -> e : skip\n",
        "fig g\npanel p \"P\"\n a: box \"A\"\n b: box \"B\"\npanel q \"Q\"\n c: box \"C\"\n d: box \"D\"\npanel r \"R\"\n e: box \"E\"\nend\na -> b\nc -> d\nb -> c : cross\nd -> a : cross\nb -> e : cross\ne -> c : cross\n",
    ] {
        let g = dsl::parse(src).unwrap();
        let f = fig(&g);
        no_overlaps(&f);
        routes_are_clean(&g, &f);
    }
    // a larger generated graph: a grid of chains with links between them
    let mut s = String::from("fig big\n");
    for r in 0..4 {
        for c in 0..6 {
            s.push_str(&format!("n{r}_{c}: box \"N{r}{c}\"\n"));
        }
        s.push_str(&(0..6).map(|c| format!("n{r}_{c}")).collect::<Vec<_>>().join(" -> "));
        s.push('\n');
    }
    s.push_str("n0_5 -> n1_0 : feedback\nn1_2 -> n2_4 : skip\nn3_5 -> n0_0 : feedback\nn2_1 -> n3_3\nn0_2 -> n3_4 : optional\n");
    let g = dsl::parse(&s).unwrap();
    let f = fig(&g);
    no_overlaps(&f);
    routes_are_clean(&g, &f);
}

#[test]
fn feedback_runs_below_the_row() {
    let g = dsl::parse("fig fb\na: box \"A\"\nb: box \"B\"\nc: loss \"$\\mathcal L$\"\na -> b -> c\nc -> a : feedback\n").unwrap();
    let f = fig(&g);
    let r = &f.routes[2];
    assert_eq!((r.from_side, r.to_side), (Side::Bottom, Side::Bottom));
    let lowest = r.points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    assert!(f.layout.nodes.iter().all(|n| lowest > n.rect.bottom()), "feedback must pass under every module of the row");
    assert_eq!(r.bends(), 2);
}

#[test]
fn lines_between_panels_keep_to_the_gap() {
    // b sits right under a; the line from a's last node to b's first runs back between the panels
    let g = dsl::parse("fig x\ntemplate rows\npanel a \"A\"\n a1: box \"A1\"\n a2: box \"A2\"\n a3: box \"A3\"\npanel b \"B\"\n b1: box \"B1\"\n b2: box \"B2\"\n b3: box \"B3\"\nend\na1 -> a2 -> a3\nb1 -> b2 -> b3\na3 -> b1 : cross\n").unwrap();
    let f = fig(&g);
    routes_are_clean(&g, &f);
    let r = f.routes.iter().find(|r| g.edges[r.edge].kind == EdgeKind::Cross).unwrap();
    let (pa, pb) = (&f.layout.panels[0].rect, &f.layout.panels[1].rect);
    let longest = r.points.windows(2).filter(|w| (w[0].1 - w[1].1).abs() < 1e-6).max_by(|a, b| (a[0].0 - a[1].0).abs().partial_cmp(&(b[0].0 - b[1].0).abs()).unwrap()).unwrap();
    assert!(longest[0].1 > pa.bottom() && longest[0].1 < pb.y, "the long horizontal run is in the gap: {:?}", r.points);
}

#[test]
fn unrelated_lines_do_not_share_a_track() {
    let g = dsl::parse("fig o\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c -> d\nd -> a : feedback\nc -> b : feedback\n").unwrap();
    let f = fig(&g);
    routes_are_clean(&g, &f);
    let segs = |r: &Route| r.points.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>();
    let (r1, r2) = (&f.routes[3], &f.routes[4]);
    for (a0, a1) in segs(r1) {
        for (b0, b1) in segs(r2) {
            let both_h = (a0.1 - a1.1).abs() < 1e-6 && (b0.1 - b1.1).abs() < 1e-6 && (a0.1 - b0.1).abs() < 1e-6;
            if both_h {
                let (lo, hi) = (a0.0.min(a1.0).max(b0.0.min(b1.0)), a0.0.max(a1.0).min(b0.0.max(b1.0)));
                assert!(hi - lo < 1e-6, "feedback lines overlap on y = {}", a0.1);
            }
        }
    }
}

// ── L3: objects ────────────────────────────────────────────────────────

#[test]
fn connectors_reproduce_their_routes() {
    use lore_flow::build::{connector_path, fit_connector};
    let e = 12_700.0;
    let cases: Vec<Vec<(f64, f64)>> = vec![
        vec![(0.0, 0.0), (100.0, 0.0)],
        vec![(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)],
        vec![(0.0, 0.0), (0.0, 50.0), (-80.0, 50.0)],
        vec![(0.0, 0.0), (40.0, 0.0), (40.0, 60.0), (100.0, 60.0)],
        vec![(0.0, 0.0), (-40.0, 0.0), (-40.0, -60.0), (-100.0, -60.0)],
        // U shapes: both ends on the same line, the detour outside the box
        vec![(0.0, 0.0), (0.0, 30.0), (-120.0, 30.0), (-120.0, 0.0)],
        vec![(0.0, 0.0), (0.0, -30.0), (90.0, -30.0), (90.0, 10.0)],
        vec![(0.0, 0.0), (50.0, 0.0), (50.0, 80.0), (0.0, 80.0)],
        vec![(0.0, 0.0), (30.0, 0.0), (30.0, 40.0), (90.0, 40.0), (90.0, 70.0)],
        vec![(0.0, 0.0), (0.0, 20.0), (-60.0, 20.0), (-60.0, -40.0), (-100.0, -40.0)],
        vec![(0.0, 0.0), (0.0, 20.0), (60.0, 20.0), (60.0, -40.0), (20.0, -40.0)],
    ];
    for pts in cases {
        let emu: Vec<(f64, f64)> = pts.iter().map(|p| (p.0 * e + 1e6, p.1 * e + 1e6)).collect();
        let (geom, frame, adj) = fit_connector(&emu).unwrap_or_else(|| panic!("no connector for {pts:?}"));
        let n = pts.len() - 2;
        assert_eq!(geom, if n == 0 { "straightConnector1".to_string() } else { format!("bentConnector{}", n + 1) });
        let back = connector_path(&geom, &adj, &frame);
        assert_eq!(back.len(), emu.len(), "{pts:?}");
        for (a, b) in back.iter().zip(&emu) {
            // within one point (a flat connector is given a 1 pt extent so its adjust value has a base)
            assert!((a.0 - b.0).abs() <= e && (a.1 - b.1).abs() <= e, "{pts:?}: {a:?} vs {b:?} ({geom} {frame:?} {adj:?})");
        }
    }
    assert!(fit_connector(&[(0.0, 0.0), (1e5, 0.0), (1e5, 1e5), (2e5, 1e5), (2e5, 2e5), (3e5, 2e5)]).is_none());
}

#[test]
fn objects_are_named_numbered_and_attached() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    let objs = f.objects();
    // names and unique ids, groups included
    let mut ids = BTreeSet::new();
    walk(objs, &mut |o| {
        assert!(o.name.starts_with("flow:demo/"), "{}", o.name);
        assert!(o.id >= 2 && ids.insert(o.id), "id {} reused", o.id);
    });
    assert_eq!(f.built.next_id, ids.iter().max().unwrap() + 1);
    for n in &g.nodes {
        let o = objs.iter().find(|o| o.name == format!("flow:demo/{}", n.id)).unwrap();
        assert_eq!(f.built.node_ids[&n.id], o.id);
    }
    // one shape → no group; several → a group with a port frame first
    assert!(matches!(objs.iter().find(|o| o.name == "flow:demo/aug").unwrap().kind, Kind::Shape(_)));
    let Kind::Group(net) = &objs.iter().find(|o| o.name == "flow:demo/net").unwrap().kind else { panic!("unet is a group") };
    assert_eq!(net.children[0].name, "flow:demo/net/port");
    assert!(net.children.iter().any(|c| c.name == "flow:demo/net/label"));
    // every line with up to three corners is a connector attached at both ends to top-level objects
    let top: BTreeSet<u32> = objs.iter().map(|o| o.id).collect();
    for r in &f.routes {
        let e = &g.edges[r.edge];
        let o = objs.iter().find(|o| o.name == format!("flow:demo/edge:{}-{}", e.from, e.to)).unwrap();
        match &o.kind {
            Kind::Connector(c) => {
                assert!(r.bends() <= 3);
                let (s, t) = (c.start.as_ref().unwrap(), c.end.as_ref().unwrap());
                assert_eq!((s.id, t.id), (f.built.node_ids[&e.from], f.built.node_ids[&e.to]));
                assert!(top.contains(&s.id) && top.contains(&t.id));
                assert_eq!(c.geom, if r.bends() == 0 { "straightConnector1".to_string() } else { format!("bentConnector{}", r.bends() + 1) });
                // the attached ends sit on the sites lore-canvas computes (left/right within the caption offset)
                let target = objs.iter().find(|x| x.id == t.id).unwrap();
                let sites = lore_canvas::geom::sites(target);
                let (_, end) = (r.points[0], r.points[r.points.len() - 1]);
                let site = sites[t.site as usize];
                let d = ((site.0 / 12_700.0 - end.0).abs(), (site.1 / 12_700.0 - end.1).abs());
                assert!(d.0 < 0.01 && d.1 < 0.01, "{} end {end:?} vs site {site:?}", o.name);
            }
            Kind::Shape(sp) => {
                assert!(r.bends() > 3);
                assert_eq!(sp.geom.prst, "custom");
            }
            _ => panic!("a line is a connector or a polyline"),
        }
    }
    // line styles come from the theme
    let st = stroke(objs.iter().find(|o| o.name == "flow:demo/edge:loss-net").unwrap());
    assert_eq!((st.dash.as_str(), st.tail.as_str()), ("dash", "triangle"));
    assert!(objs.iter().any(|o| o.name == "flow:demo/edge:loss-net/label"));
    assert!(objs.iter().any(|o| o.name == "flow:demo/panel:a/title"));
}

#[test]
fn every_module_kind_draws() {
    for (kind, _) in lore_flow::spec::MODULES {
        let g = dsl::parse(&format!("fig k\na: {kind} \"$F_1$\" caption=\"note\"\nb: box \"B\"\na -> b\n")).unwrap();
        let f = fig(&g);
        let p = &f.layout.nodes[0];
        assert!(p.rect.w > 4.0 && p.rect.h > 4.0, "{kind}");
        let o = f.objects().iter().find(|o| o.name == "flow:k/a").unwrap();
        let mut texts = 0;
        walk(std::slice::from_ref(o), &mut |x| {
            if let Some(sp) = x.kind.sp()
                && sp.text.as_ref().is_some_and(|t| !t.plain_text().trim().is_empty())
            {
                texts += 1;
            }
        });
        assert!(texts >= 1, "{kind} shows its label");
        assert_eq!(f.routes[0].bends(), 0, "{kind}: level arrow");
    }
}

#[test]
fn drawing_is_deterministic() {
    let g = dsl::parse(TRAIN).unwrap();
    let a = serde_json::to_string(fig(&g).objects()).unwrap();
    let b = serde_json::to_string(fig(&g).objects()).unwrap();
    assert_eq!(a, b);
    assert_eq!(fig(&g).routes, fig(&g).routes);
}

#[test]
fn the_figure_is_one_undo_step() {
    let g = dsl::parse(TRAIN).unwrap();
    let (_, deck) = pptx::blank_deck(720.0, 405.0).unwrap();
    let uid = deck.slides[0].uid.clone();
    let f = draw(&g, &Drawing { first_id: deck.slides[0].max_id() + 1, ..Default::default() }).unwrap();
    let mut ed = Editor::new(deck, None);
    ed.apply(insert_ops(&uid, 0, f.objects()), "流程圖").unwrap();
    assert_eq!(ed.deck.slide(&uid).unwrap().objects.len(), f.objects().len());
    assert_eq!(ed.can_undo(), Some("流程圖"));
    ed.undo().unwrap();
    assert!(ed.deck.slide(&uid).unwrap().objects.is_empty());
    ed.redo().unwrap();
    assert_eq!(ed.deck.slide(&uid).unwrap().objects, f.objects());
}

#[test]
fn moving_a_module_takes_its_lines_along() {
    // lore-canvas re-routes connectors attached to a moved object in the same transaction
    let g = dsl::parse("fig mv\na: box \"A\"\nb: image \"$x$\"\na -> b\n").unwrap();
    let (_, mut deck) = pptx::blank_deck(720.0, 405.0).unwrap();
    let f = fig(&g);
    deck.slides[0].objects = f.objects().to_vec();
    let b = f.built.node_ids["b"];
    let slide = &deck.slides[0];
    let moved: BTreeSet<u32> = [b].into_iter().collect();
    assert!(lore_canvas::geom::reroute(slide, &moved).is_empty() || lore_canvas::geom::reroute(slide, &moved).len() == 1);
    let mut s2 = slide.clone();
    s2.find_mut(b).unwrap().frame.y += 400_000;
    let re = lore_canvas::geom::reroute(&s2, &moved);
    assert_eq!(re.len(), 1, "the connector to the moved group is re-routed");
}

#[test]
fn pptx_round_trip_keeps_connectors_attached() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    let bytes = pptx::single_slide(f.objects(), 720.0, 405.0).unwrap();
    let deck = pptx::read(&bytes).unwrap();
    assert_eq!(deck.slides.len(), 1);
    let slide = &deck.slides[0];
    assert_eq!(slide.objects.len(), f.objects().len());
    let mut connectors = 0;
    for (o, was) in slide.objects.iter().zip(f.objects()) {
        assert_eq!(o.name, was.name);
        assert_eq!((o.frame.x, o.frame.y, o.frame.w, o.frame.h), (was.frame.x, was.frame.y, was.frame.w, was.frame.h), "{}", o.name);
        let (Kind::Connector(c), Kind::Connector(w)) = (&o.kind, &was.kind) else { continue };
        connectors += 1;
        assert_eq!((c.geom.as_str(), &c.adj), (w.geom.as_str(), &w.adj), "{}", o.name);
        assert_eq!((o.frame.rot, o.frame.flip_h, o.frame.flip_v), (was.frame.rot, was.frame.flip_h, was.frame.flip_v), "{}", o.name);
        let (s, t) = (c.start.as_ref().expect("start stays attached"), c.end.as_ref().expect("end stays attached"));
        for (a, w_att) in [(s, w.start.as_ref().unwrap()), (t, w.end.as_ref().unwrap())] {
            assert_eq!(a.site, w_att.site);
            // in the file a group's lines hold on to its port shape: a real shape with the same frame
            let target = slide.find(a.id).unwrap_or_else(|| panic!("{}: attached to a missing shape", o.name));
            assert!(matches!(target.kind, Kind::Shape(_) | Kind::Text(_)), "{} is attached to a {}", o.name, target.kind.name());
            if a.id != w_att.id {
                assert!(target.name.ends_with("/port"));
                let (parent, _) = slide.locate(a.id).unwrap();
                assert_eq!(parent, Some(w_att.id));
            }
        }
        assert_eq!(c.line.as_ref().map(|l| (&l.dash, &l.tail)), w.line.as_ref().map(|l| (&l.dash, &l.tail)));
    }
    assert_eq!(connectors, f.routes.iter().filter(|r| r.bends() <= 3).count());
    assert!(connectors >= g.edges.len() - 1);
    // native shapes: nothing was flattened into a picture
    let mut pictures = 0;
    walk(&slide.objects, &mut |o| {
        if matches!(o.kind, Kind::Picture(_) | Kind::Locked(_)) {
            pictures += 1;
        }
    });
    assert_eq!(pictures, 0);
    // an unedited re-export of what we read is byte-identical (lore-canvas's lossless path accepts our shapes)
    let od = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let again = lore_canvas::pptx_out::export_bytes(&deck, &od, None, &|_: &str| None).unwrap();
    assert_eq!(pptx::read(&again).unwrap().slides[0].objects.len(), slide.objects.len());
}

// ── text, math, variables ──────────────────────────────────────────────

#[test]
fn math_becomes_readable_text_without_a_converter() {
    use lore_flow::text::{latex_pieces, plain, width};
    assert_eq!(plain("$\\mathcal L_{seg}$"), "ℒseg");
    assert_eq!(plain("Aug($x_b$)"), "Aug(xb)");
    assert_eq!(plain("$\\frac{H}{2}\\times\\frac{W}{2}\\times 2C$"), "H/2×W/2×2C");
    assert_eq!(plain("$\\theta_{seg}$ and $\\omega$"), "θseg and ω");
    let p = latex_pieces("\\hat h_c");
    assert_eq!(p[0].text, "h\u{302}");
    assert!(p[0].italic && p[1].script == -1 && p[1].text == "c");
    let p = latex_pieces("\\mathrm{Area}(y_b)");
    assert!(!p[0].italic && p[0].text.starts_with("Area"));
    assert!(width("$\\mathcal L_{seg}$", 11.0) < width("Segmentation loss", 11.0));
    assert_eq!(lore_flow::text::lines("Fuzzy \\\\ system"), ["Fuzzy", "system"]);
    // an unclosed $ stays text
    assert_eq!(plain("costs $5"), "costs $5");
}

struct FakeMath;
impl MathSource for FakeMath {
    fn omml(&self, latex: &str, _: f64) -> Option<String> {
        Some(format!(r#"<a14:m xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>{}</m:t></m:r></m:oMath></a14:m>"#, latex.replace('\\', "/").replace('{', "(").replace('}', ")")))
    }
}

#[test]
fn math_runs_carry_office_math_when_a_converter_is_there() {
    let g = dsl::parse("fig m\na: box \"UNet $\\theta_{seg}$\"\nb: loss \"$\\mathcal L_{seg}$\"\na -> b\n").unwrap();
    let f = draw(&g, &Drawing { math: &FakeMath, ..Default::default() }).unwrap();
    let a = f.objects().iter().find(|o| o.name == "flow:m/a").unwrap();
    let sp = a.kind.sp().unwrap();
    assert!(sp.equation, "a shape with Office Math is exported as an equation box");
    let runs = &sp.text.as_ref().unwrap().paras[0].runs;
    assert_eq!(runs.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["text", "math"]);
    assert_eq!(runs[1].latex.as_deref(), Some("\\theta_{seg}"));
    assert!(runs[1].omml.as_deref().unwrap().contains("m:oMath"));
    // and it reaches the file
    let bytes = pptx::single_slide(f.objects(), 720.0, 405.0).unwrap();
    let od = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let xml = String::from_utf8_lossy(od.pkg.get(&od.slides[0].part).unwrap()).to_string();
    assert!(xml.contains("a14:m") && xml.contains("mc:AlternateContent"), "math is written as Office Math");
    // without a converter the same label is ordinary runs with a subscript
    let f = fig(&g);
    let a = f.objects().iter().find(|o| o.name == "flow:m/a").unwrap();
    let sp = a.kind.sp().unwrap();
    assert!(!sp.equation);
    let runs = &sp.text.as_ref().unwrap().paras[0].runs;
    assert!(runs.iter().all(|r| r.kind == "text"));
    assert!(runs.iter().any(|r| r.look.baseline < 0.0 && r.text == "seg"));
}

fn registry() -> lore_notation::Notation {
    lore_notation::Notation::from_json(
        r#"{ "version": 1, "entries": [
        { "id": "img", "kind": "symbol", "latex": "\\boldsymbol{x}", "macro": "img", "meaning": "input image" },
        { "id": "seg-loss", "kind": "symbol", "latex": "\\mathcal{L}_{seg}", "meaning": "segmentation loss" },
        { "id": "weight", "kind": "symbol", "latex": "\\omega", "meaning": "fuzzy weight" } ] }"#,
    )
    .unwrap()
}

#[test]
fn labels_refer_to_the_notation_table() {
    let reg = registry();
    let mut vars = Vars::from_notation(&reg);
    vars.add_new(&[VarNew { id: "qb".into(), latex: "\\tilde q_b".into(), meaning: String::new() }]);
    // an entry with a macro is referred to through the macro; others by their LaTeX
    assert_eq!(vars.resolve("image @img."), "image $\\img$.");
    assert_eq!(vars.resolve("@seg-loss"), "$\\mathcal{L}_{seg}$");
    assert_eq!(vars.resolve("$2 @weight$ and @qb"), "$2 {\\omega}$ and $\\tilde q_b$");
    assert_eq!(vars.resolve("mail@nowhere"), "mail@nowhere");
    assert_eq!(vars.unknown_refs("@img @ghost"), ["ghost"]);
    // registered macros are expanded before math is converted or measured
    assert_eq!(vars.expand("\\img_i"), "{\\boldsymbol{x}}_i");
    let g = dsl::parse("fig n\nx: image var=img\nl: loss \"@seg-loss\"\nx -> l\n").unwrap();
    let f = draw(&g, &Drawing { notation: Some(&reg), ..Default::default() }).unwrap();
    let mut seen = String::new();
    walk(f.objects(), &mut |o| {
        if let Some(t) = o.kind.sp().and_then(|s| s.text.as_ref()) {
            seen.push_str(&t.plain_text());
            seen.push('|');
        }
    });
    assert!(seen.contains("x|") && seen.contains("ℒseg"), "{seen}");
}

#[test]
fn new_variables_are_reconciled_with_the_registry() {
    use lore_flow::notation::{DRAFT_TAG, VarStatus, check_graph, register_new};
    let reg = registry();
    let mut g = dsl::parse(
        "fig v\nx: image var=x\nw: label \"@w weights\"\nq: label \"@weight\"\nl: loss \"$L_{seg}$\"\nt: label \"see Table 2\"\nu: label \"@nobody and $\\omega$\"\nx -> l\nvar x = \"\\boldsymbol{x}\" \"input image\"\nvar w = \"\\omega\" \"learning rate\"\nvar weight = \"\\tilde q_b\" \"quality score\"\n",
    )
    .unwrap();
    let rep = check_graph(&mut g, &reg);
    // x and w are already registered (under other ids): references move, nothing new
    assert_eq!(rep.vars[0].status, VarStatus::Existing { id: "img".into(), same_meaning: true });
    assert_eq!(g.node("x").unwrap().var, "img");
    assert_eq!(rep.vars[1].status, VarStatus::Existing { id: "weight".into(), same_meaning: false });
    assert_eq!(g.node("w").unwrap().label, "@weight weights");
    // "weight" is taken by another symbol: the new variable is renamed, and only its own reference follows
    assert_eq!(rep.vars[2].status, VarStatus::Renamed { from: "weight".into() });
    assert_eq!(g.vars_new.len(), 1);
    assert_eq!(g.vars_new[0].id, "weight-2");
    assert_eq!(rep.by_code("unknown_ref").len(), 1);
    assert_eq!(rep.by_code("loss_style").len(), 1);
    assert_eq!(rep.by_code("hardcoded_reference").len(), 1);
    assert!(rep.by_code("use_registered").iter().any(|f| f.message.contains("@weight")));
    // only the document's table receives the drafts
    let mut doc = lore_notation::Notation::from_json(r#"{ "version": 1, "entries": [] }"#).unwrap();
    assert_eq!(register_new(&mut doc, &g.vars_new), ["weight-2"]);
    assert!(doc.get("weight-2").unwrap().has_tag(DRAFT_TAG));
    assert!(register_new(&mut doc, &g.vars_new).is_empty());
    assert_eq!(reg.entries.len(), 3);
    // a clean figure has nothing to report
    let mut clean = dsl::parse("fig ok\nx: image var=img\nl: loss \"@seg-loss\"\nx -> l\n").unwrap();
    assert_eq!(check_graph(&mut clean, &reg).findings, []);
}

// ── themes, module templates, legend ───────────────────────────────────

#[test]
fn a_theme_file_only_lists_what_it_changes() {
    let base = Theme::classic_bw();
    assert_eq!(base.id, "classic-bw");
    let t = Theme::from_toml("id = \"warm\"\n[panel]\nfills = [\"FEF8F5\", \"FAFCFE\"]\nradius = 0.2\n[roles.model]\nfill = \"C8C1FF\"\nline = \"1D3B63\"\n[edges.feedback]\ncolor = \"C8601C\"\nwidth = 2.5\ndash = \"dash\"\ntail = \"triangle\"\n").unwrap();
    assert_eq!((t.role("model").fill.as_str(), t.role("gt").fill.as_str()), ("C8C1FF", base.role("gt").fill.as_str()));
    assert_eq!(t.edge(EdgeKind::Feedback).color, "C8601C");
    assert_eq!(t.edge(EdgeKind::Data), base.edge(EdgeKind::Data));
    assert!(Theme::from_toml("[colors]\nink = \"black\"\n").unwrap_err().contains("ink"));
    assert!(Theme::from_toml("[edges.wobbly]\ncolor = \"000000\"\nwidth = 1.0\n").is_err());
    // the theme decides the colours of what is drawn
    let g = dsl::parse(TRAIN).unwrap();
    let f = draw(&g, &Drawing { theme: t, ..Default::default() }).unwrap();
    assert_eq!(stroke(f.objects().iter().find(|o| o.name == "flow:demo/edge:loss-net").unwrap()).color, "C8601C");
    let panel = f.objects().iter().find(|o| o.name == "flow:demo/panel:b").unwrap();
    assert_eq!(panel.kind.sp().unwrap().fill, lore_canvas::Paint::solid("FAFCFE"));
    assert_eq!(panel.kind.sp().unwrap().geom.prst, "roundRect");
    let aug = f.objects().iter().find(|o| o.name == "flow:demo/aug").unwrap();
    assert_eq!(aug.kind.sp().unwrap().fill, lore_canvas::Paint::solid("C8C1FF"));
    // the public example copy is the built-in theme
    let example = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../private-example/SlideComposer/flow/themes/classic-bw.toml")).unwrap();
    assert_eq!(Theme::from_toml(&example).unwrap(), base);
}

fn example_dir() -> std::path::PathBuf {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../private-example/SlideComposer/flow")).to_path_buf()
}

fn find(f: &Figure, name: &str) -> Object {
    let mut hit = None;
    walk(f.objects(), &mut |o| {
        if o.name == name {
            hit = Some(o.clone());
        }
    });
    hit.unwrap_or_else(|| panic!("{name} missing"))
}

#[test]
fn saved_drawings_are_module_kinds_and_styles() {
    let dir = example_dir();
    // modules/<kind>/<style>.json: the style is the file name
    let lib = Library::load_dir(&dir.join("modules")).unwrap();
    assert_eq!(lib.kinds(), ["document", "store"], "kinds the library adds to the built-in ones");
    assert_eq!(lib.styles("encoder"), ["plain"]);
    assert_eq!(lib.styles("store"), ["can"]);
    let g = dsl::parse_with(
        "fig t\ne: encoder \"$E_\\phi$\" style=plain\ns: store \"Memory\"\nd: decoder \"Dec\" style=plain role=output fill=FFE5E5\ndoc: document \"Report\"\ne2: encoder \"Enc\"\ne -> s -> d -> doc -> e2\n",
        &lib.kinds(),
    )
    .unwrap();
    assert_eq!(g.node("e").unwrap().style, "plain");
    assert_eq!(dsl::parse_with(&dsl::to_dsl(&g), &lib.kinds()).unwrap(), g);
    let f = draw(&g, &Drawing { library: lib, ..Default::default() }).unwrap();
    // role colours and the label are filled in
    let body = find(&f, "flow:t/e/body");
    let sp = body.kind.sp().unwrap();
    assert_eq!(sp.fill, lore_canvas::Paint::solid(&Theme::classic_bw().role("model").fill));
    assert_eq!(sp.text.as_ref().unwrap().plain_text(), "Eϕ");
    assert_eq!(find(&f, "flow:t/d/body").kind.sp().unwrap().fill, lore_canvas::Paint::solid("FFE5E5"));
    // a drawing without a label run gets its label underneath
    assert_eq!(find(&f, "flow:t/s/label").kind.sp().unwrap().text.as_ref().unwrap().plain_text(), "Memory");
    assert_eq!(find(&f, "flow:t/s/port").frame.w, find(&f, "flow:t/s").frame.w);
    // without a style a built-in kind keeps its built-in drawing
    assert!(matches!(find(&f, "flow:t/e2").kind, Kind::Group(_)));
    assert!(f.objects().iter().all(|o| o.name != "flow:t/e2/body") || find(&f, "flow:t/e2/body").kind.sp().unwrap().geom.prst == "custom");
    assert!(f.routes.iter().all(|r| r.bends() == 0 && !r.forced));
    // the example panel template parses
    let t = PanelTemplate::from_json(&std::fs::read_to_string(dir.join("templates/one-over-two.json")).unwrap()).unwrap();
    assert_eq!(t.panels.len(), 3);
    // a file in the wrong folder is refused
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("decoder")).unwrap();
    std::fs::copy(dir.join("modules/encoder/plain.json"), tmp.path().join("decoder/plain.json")).unwrap();
    assert!(Library::load_dir(tmp.path()).unwrap_err().contains("does not match its folder"));
    assert!(Library::load_dir(&tmp.path().join("nothing-here")).unwrap().templates.is_empty());
}

/// What a drawing is made of, without its paint: per part its kind, its
/// geometry (rounded corners count as a rectangle; for a freeform only the
/// path's commands) and its place in the drawing to 1/25 of the size.
fn structure(objs: &[Object]) -> Vec<String> {
    let parts: Vec<&Object> = objs.iter().filter(|o| !o.name.ends_with("port") && o.name != "label" && !o.name.ends_with("/label")).collect();
    let b = lore_canvas::geom::bounds(parts.iter().map(|o| &o.frame));
    let (bw, bh) = (b.w.max(1) as f64, b.h.max(1) as f64);
    let q = |v: f64| (v * 25.0).round() as i64;
    let mut out = Vec::new();
    for o in parts {
        let geom = match &o.kind {
            Kind::Shape(sp) | Kind::Text(sp) => match sp.geom.prst.as_str() {
                "roundRect" => "rect".to_string(),
                "custom" => format!("custom:{}", sp.geom.path.as_deref().unwrap_or("").chars().filter(|c| c.is_ascii_alphabetic()).collect::<String>()),
                p => p.to_string(),
            },
            Kind::Connector(c) => c.geom.clone(),
            Kind::Group(g) => format!("group{:?}", structure(&g.children)),
            _ => String::new(),
        };
        let text = o.kind.sp().and_then(|s| s.text.as_ref()).is_some_and(|t| !t.plain_text().trim().is_empty());
        let f = &o.frame;
        out.push(format!("{}:{geom}:{}:{},{},{},{}", o.kind.name(), text, q((f.x - b.x) as f64 / bw), q((f.y - b.y) as f64 / bh), q(f.w as f64 / bw), q(f.h as f64 / bh)));
    }
    out.sort();
    out
}

fn drawn(kind: &str, style: &str, finish: &str) -> Vec<Object> {
    let mut node = lore_flow::sheet::sample(kind, style);
    if !finish.is_empty() {
        node.params.insert("finish".into(), finish.into());
    }
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let lib = Library::default();
    let cx = lore_flow::modules::Ctx { theme: &theme, vars: &vars, math: &NoMath, lib: &lib, k: 1.0, symmetric: false, image_size: &lore_flow::modules::no_images };
    lore_flow::modules::make(&node, &cx).objs
}

#[test]
fn styles_are_constructions_and_finish_is_paint() {
    use lore_flow::modules::generator_styles;
    for kind in ["encoder", "decoder", "autoencoder", "nn", "brace", "feature-stack", "image-stack", "loss", "op", "box", "unet"] {
        let styles = generator_styles(kind);
        if matches!(kind, "encoder" | "decoder" | "nn") {
            assert!(styles.len() >= 15, "{kind} has {} styles", styles.len());
        }
        // a finish is never a style
        for s in styles {
            assert!(!lore_flow::finish::WORDS.contains(s) && !["default", "rounded", "filled", "gradient", "outline", "dots", "squares"].contains(s), "{kind}/{s} is a finish, not a style");
        }
        // no two styles of a kind are the same drawing once the paint is fixed
        let mut seen: Vec<(&str, Vec<String>)> = Vec::new();
        for style in styles {
            let sig = structure(&drawn(kind, style, "solid mono"));
            assert!(!sig.is_empty(), "{kind}/{style} draws nothing");
            if let Some((other, _)) = seen.iter().find(|(_, s)| *s == sig) {
                panic!("{kind}/{style} is the same drawing as {kind}/{other}");
            }
            seen.push((style, sig));
        }
        // and every finish leaves the drawing as it is
        for style in styles {
            let base = structure(&drawn(kind, style, "solid mono"));
            for finish in ["outline", "gradient translucent", "flat round", "typed bold", "outline thin square"] {
                assert_eq!(structure(&drawn(kind, style, finish)), base, "{kind}/{style} with finish {finish}");
            }
        }
    }
    assert!(lore_flow::finish::Finish::of(&lore_flow::sheet::sample("box", "rect"), &Theme::classic_bw()).apply("glossy").is_err());
}

#[test]
fn every_built_in_style_draws_and_lines_meet_it() {
    use lore_flow::modules::generator_styles;
    for kind in ["encoder", "decoder", "autoencoder", "nn", "brace"] {
        for style in generator_styles(kind) {
            let g = dsl::parse(&format!("fig k\na: box \"A\"\nm: {kind} \"M\" style={style}\nb: box \"B\"\na -> m -> b\n")).unwrap();
            let f = fig(&g);
            assert!(f.routes.iter().all(|r| r.bends() == 0 && !r.forced), "{kind}/{style}");
            let o = f.objects().iter().find(|o| o.name == "flow:k/m").unwrap();
            // connection sites are where the lines end
            let sites = lore_canvas::geom::sites(o);
            let end = f.routes[0].points[1];
            assert!((sites[1].0 / 12_700.0 - end.0).abs() < 0.01 && (sites[1].1 / 12_700.0 - end.1).abs() < 0.01, "{kind}/{style}");
            // every part is a native shape, text or line
            walk(std::slice::from_ref(o), &mut |p| assert!(matches!(p.kind.name(), "shape" | "text" | "connector" | "group"), "{kind}/{style}: {}", p.name));
        }
    }
    // the theme can choose the style and the colours of a kind; a node's finish wins
    let t = Theme::from_toml("[modules.encoder]\nstyle = \"bars\"\nfill = \"C8C1FF\"\nline = \"1D3B63\"\n[modules.unet]\nfill = \"C8C1FF\"\n").unwrap();
    let g = dsl::parse("fig k\ne: encoder \"E\"\nu: unet \"U\"\no: encoder \"O\" style=trapezoid finish=outline\ne -> u -> o\n").unwrap();
    let f = draw(&g, &Drawing { theme: t, ..Default::default() }).unwrap();
    assert_eq!(find(&f, "flow:k/e/bar0").kind.sp().unwrap().fill, lore_canvas::Paint::solid("C8C1FF"));
    assert_eq!(find(&f, "flow:k/e/bar0").kind.sp().unwrap().line.as_ref().unwrap().color, "1D3B63");
    assert_eq!(find(&f, "flow:k/u/bar0").kind.sp().unwrap().fill, lore_canvas::Paint::solid("C8C1FF"));
    assert_eq!(find(&f, "flow:k/o/body").kind.sp().unwrap().fill, lore_canvas::Paint::solid("FFFFFF"), "outline = paper inside, the line carries the colour");
    // the theme's finish applies to every module
    let t = Theme::from_toml("[node]\nfinish = \"gradient round\"\n").unwrap();
    let f = draw(&dsl::parse("fig k\ne: encoder \"E\" style=bars\n").unwrap(), &Drawing { theme: t, ..Default::default() }).unwrap();
    let bar = find(&f, "flow:k/e/bar0");
    assert!(matches!(bar.kind.sp().unwrap().fill, lore_canvas::Paint::Gradient { .. }));
    assert_eq!(bar.kind.sp().unwrap().geom.prst, "roundRect");
}

#[test]
fn layer_types_are_coloured_and_named_in_one_legend() {
    let t = Theme::from_toml("[layers.conv]\ncolor = \"4472C4\"\nlabel = \"Conv + BN + ReLU\"\n[layers.pool]\ncolor = \"70AD47\"\n").unwrap();
    let g = dsl::parse("fig s\nx: image \"$x$\"\ne: encoder \"Encoder\" style=slabs\nd: decoder \"Decoder\" style=slabs\ny: image \"$y$\" look=mask\nx -> e -> d -> y\n").unwrap();
    let f = draw(&g, &Drawing { theme: t.clone(), ..Default::default() }).unwrap();
    let front = find(&f, "flow:s/e/s0-0-front");
    assert_eq!(front.kind.sp().unwrap().fill, lore_canvas::Paint::solid("4472C4"));
    // one legend for both modules, over the figure, every type once
    let keys: Vec<&str> = f.layout.swatches.iter().map(|s| s.swatch.key.as_str()).collect();
    assert_eq!(keys, ["conv", "pool", "up", "softmax"]);
    let sw = find(&f, "flow:s/colours:conv");
    assert_eq!(sw.kind.sp().unwrap().fill, lore_canvas::Paint::solid("4472C4"));
    assert!(sw.frame.y < find(&f, "flow:s/e").frame.y, "the colour legend is over the figure");
    let label = find(&f, "flow:s/colours:conv/label");
    assert_eq!(label.kind.sp().unwrap().text.as_ref().unwrap().plain_text(), "Conv + BN + ReLU");
    // mono: one colour, no legend; legend off: no legend
    let g2 = dsl::parse("fig s\ne: encoder \"E\" style=slabs finish=mono\n").unwrap();
    assert!(draw(&g2, &Drawing { theme: t.clone(), ..Default::default() }).unwrap().layout.swatches.is_empty());
    let mut g3 = g.clone();
    g3.legend = Some(false);
    assert!(draw(&g3, &Drawing { theme: t, ..Default::default() }).unwrap().layout.swatches.is_empty());
}

#[test]
fn a_matched_pair_joins_its_halves() {
    let names = |src: &str| -> Vec<String> {
        let f = fig(&dsl::parse(src).unwrap());
        let mut v = Vec::new();
        walk(f.objects(), &mut |o| v.push(o.name.clone()));
        v
    };
    // SegNet: pooling indices from every pooling layer to its upsampling layer
    let n = names("fig p\nae: autoencoder style=slabs\n");
    assert_eq!(n.iter().filter(|s| s.contains("/index")).count(), 4);
    assert!(n.iter().any(|s| s.ends_with("/enc-name")) && n.iter().any(|s| s.ends_with("/dec-name")));
    // U-Net: skips at every level but the deepest, which is one arrow
    let n = names("fig p\nae: autoencoder style=unet\n");
    assert_eq!(n.iter().filter(|s| s.contains("/skip")).count(), 3);
    // FC autoencoder: neurons in the latent space, named
    let n = names("fig p\nae: autoencoder style=fc\n");
    assert!(n.iter().any(|s| s.ends_with("/latent-name")) && n.iter().any(|s| s.ends_with("/z0")));
    // seq2seq: a context vector between the RNNs
    let n = names("fig p\nae: autoencoder style=rnn\n");
    assert!(n.iter().any(|s| s.ends_with("/c")));
    // transformer: the encoder's output enters the decoder's cross-attention
    let n = names("fig p\nae: autoencoder style=transformer\n");
    assert!(n.iter().any(|s| s.ends_with("/memory")));
    // the pair's role colours are a legend too
    let f = fig(&dsl::parse("fig p\nae: autoencoder style=fc\n").unwrap());
    let keys: Vec<&str> = f.layout.swatches.iter().map(|s| s.swatch.key.as_str()).collect();
    assert_eq!(keys, ["enc", "latent", "dec"]);
}

#[test]
fn a_selection_becomes_a_module_file() {
    // draw something, "select" it, save it as a module, use it
    let g = dsl::parse("fig s\nn: nn \"Net\" style=circles layers=[2,3,1] fill=C8C1FF\n").unwrap();
    let f = fig(&g);
    let group = f.objects().iter().find(|o| o.name == "flow:s/n").unwrap();
    let Kind::Group(gr) = &group.kind else { panic!() };
    // as the editor hands them over: top-level objects somewhere on the slide
    let mut selection: Vec<Object> = gr.children.iter().filter(|c| !c.name.ends_with("/port")).cloned().collect();
    for o in &mut selection {
        o.frame.x += group.frame.x;
        o.frame.y += group.frame.y;
    }
    let opt = SaveOptions { kind: "my-net".into(), style: "lavender".into(), description: "a small network".into(), label: Some("Net".into()), role_colours: true };
    let t = template_from_objects(&selection, &opt).unwrap();
    let text = serde_json::to_string(&t).unwrap();
    assert!(text.contains("role:fill") && text.contains("role:line") && text.contains("{{label}}"), "{text}");
    assert!(!text.contains("C8C1FF") && !text.contains("\"src\""));
    let objs: Vec<Object> = serde_json::from_value(serde_json::json!(t.objects.to_string().replace("role:fill", "000000").replace("role:line", "000000").replace("role:text", "000000"))).ok().and_then(|v: String| serde_json::from_str(&v).ok()).unwrap();
    assert!(objs.iter().all(|o| o.id == 0 && o.frame.x >= 0 && o.frame.y >= 0));
    assert!(objs.iter().any(|o| o.frame.x == 0) && objs.iter().any(|o| o.frame.y == 0), "coordinates are relative to the selection");
    // one file per style, never overwritten
    let tmp = tempfile::tempdir().unwrap();
    let p1 = Library::save(tmp.path(), &t).unwrap();
    assert!(p1.ends_with("my-net/lavender.json"));
    let p2 = Library::save(tmp.path(), &t).unwrap();
    assert!(p2.ends_with("my-net/lavender-2.json"));
    let lib = Library::load_dir(tmp.path()).unwrap();
    assert_eq!(lib.styles("my-net"), ["lavender", "lavender-2"]);
    // used like any module, in another theme's colours
    let g2 = dsl::parse_with("fig u\na: box \"A\"\nm: my-net \"Classifier\" style=lavender role=output fill=FFE5E5\na -> m\n", &lib.kinds()).unwrap();
    let f2 = draw(&g2, &Drawing { library: lib, ..Default::default() }).unwrap();
    let Kind::Group(m) = &f2.objects().iter().find(|o| o.name == "flow:u/m").unwrap().kind else { panic!() };
    assert_eq!(m.children.len(), selection.len() + 1, "every part plus the port frame");
    let neuron = m.children.iter().find(|c| c.name.ends_with("/n0-0")).unwrap();
    assert_eq!(neuron.kind.sp().unwrap().fill, lore_canvas::Paint::solid("FFE5E5"));
    assert!(m.children.iter().any(|c| c.kind.sp().and_then(|s| s.text.as_ref()).is_some_and(|t| t.plain_text() == "Classifier")));
    assert_eq!(f2.routes[0].bends(), 0);
    // a whole module group can be saved as it is
    let whole = template_from_objects(std::slice::from_ref(group), &SaveOptions { kind: "k".into(), ..Default::default() }).unwrap();
    assert_eq!(whole.objects.as_array().unwrap().len(), selection.len());
    assert!(template_from_objects(&[], &opt).is_err());
    // the built-in styles can be written out as files for a library
    let all = lore_flow::sheet::builtin_templates(&Theme::classic_bw());
    assert!(all.len() >= 15 + 15 + 15 + 15 + 5);
    let slabs = serde_json::to_string(&all.iter().find(|t| t.kind == "encoder" && t.style == "slabs").unwrap().objects).unwrap();
    assert!(slabs.contains("layer:conv") && slabs.contains("layer:pool"), "layer colours stay themeable");
    let mut lib2 = Library::default();
    for t in all {
        lib2.add(t).unwrap();
    }
    assert!(lib2.styles("encoder").contains(&"slabs".to_string()));
    // a saved drawing takes the theme's layer colours
    let t = Theme::from_toml("[layers.conv]\ncolor = \"4472C4\"\n").unwrap();
    let g3 = dsl::parse("fig v\ne: encoder \"E\" style=slabs\n").unwrap();
    let f3 = draw(&g3, &Drawing { theme: t, library: lib2, ..Default::default() }).unwrap();
    let mut blue = 0;
    walk(f3.objects(), &mut |o| {
        if o.name.starts_with("flow:v/e/") && o.kind.sp().is_some_and(|s| s.fill == lore_canvas::Paint::solid("4472C4")) {
            blue += 1;
        }
    });
    assert_eq!(blue, 10, "the ten conv layers");
    let keys: Vec<&str> = f3.layout.swatches.iter().map(|s| s.swatch.key.as_str()).collect();
    assert_eq!(keys, ["conv", "pool"], "a saved drawing keeps its colour legend");
}

#[test]
fn the_contact_sheet_shows_every_kind_and_style() {
    let lib = Library::load_dir(&example_dir().join("modules")).unwrap();
    let cat = lore_flow::sheet::catalogue(&lib);
    assert_eq!(cat.len(), lore_flow::spec::MODULES.len() + 2);
    let enc = cat.iter().find(|c| c.0 == "encoder").unwrap();
    assert!(enc.1.contains(&"slabs".to_string()) && enc.1.contains(&"plain".to_string()));
    let (objs, w, h) = lore_flow::sheet::contact_sheet(&Theme::classic_bw(), &lib, &NoMath);
    for (kind, styles) in &cat {
        for s in styles {
            assert!(objs.iter().any(|o| o.name == format!("sheet:{kind}/{s}")), "{kind}/{s}");
        }
    }
    let mut ids = BTreeSet::new();
    walk(&objs, &mut |o| {
        assert!(ids.insert(o.id));
        let f = &o.frame;
        assert!(f.x as f64 / 12_700.0 <= w && f.y as f64 / 12_700.0 <= h);
    });
    assert!(pptx::single_slide(&objs, w, h).is_ok());
    // arcs (pooling indices, dense and residual skips) are curves in the preview too
    assert!(preview::svg(&objs, w, h, 1.0).contains("<path d=\"M"));
}

#[test]
fn fonts_come_from_the_theme() {
    let t = Theme::from_toml("[font]\nfamily = \"Flow Sans\"\ntitle = \"Flow Display\"\ncaption = \"Flow Mono\"\nmath = \"Flow Math\"\nea = \"Flow CJK\"\nsize = 14.0\ntitle_size = 20.0\nsmall_size = 9.0\ntitle_bold = false\n").unwrap();
    let g = dsl::parse("fig f\npanel p \"Stage\"\n a: box \"Model\"\n b: image \"$x$\" caption=\"512\"\nend\na -> b \"feeds\"\n").unwrap();
    let f = draw(&g, &Drawing { theme: t, math: &FakeMath, options: LayoutOptions { max_scale: 1.0, frame: Rect::new(0.0, 0.0, 2000.0, 2000.0), ..Default::default() }, ..Default::default() }).unwrap();
    assert!((f.layout.scale - 1.0).abs() < 1e-9);
    let look = |name: &str| find(&f, name).kind.sp().unwrap().text.as_ref().unwrap().paras[0].runs[0].look.clone();
    let l = look("flow:f/a");
    assert_eq!((l.font.as_str(), l.ea_font.as_str(), l.size), ("Flow Sans", "Flow CJK", 14.0));
    let l = look("flow:f/panel:p/title");
    assert_eq!((l.font.as_str(), l.size, l.bold), ("Flow Display", 20.0, false));
    let l = look("flow:f/b/caption");
    assert_eq!((l.font.as_str(), l.size), ("Flow Mono", 9.0));
    assert_eq!(look("flow:f/edge:a-b/label").font, "Flow Mono");
    // variables are Office Math in the theme's math font
    let l = look("flow:f/b/label");
    assert_eq!(l.font, "Flow Math");
    assert_eq!(find(&f, "flow:f/b/label").kind.sp().unwrap().text.as_ref().unwrap().paras[0].runs[0].kind, "math");
}

/// A 2 × 1 pixel PNG header (enough for its size; the bytes are only carried along).
fn tiny_png(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    b
}

#[test]
fn image_nodes_take_real_pictures() {
    let files = |src: &str| match src {
        "wide.png" => Some(tiny_png(400, 200)),
        "tall.png" => Some(tiny_png(100, 300)),
        _ => None,
    };
    assert_eq!(pptx::image_size(&tiny_png(400, 200)), Some((400, 200)));
    let g = dsl::parse("fig i\na: image \"$x$\" src=wide.png size=60\nb: image \"crop\" src=wide.png w=40 h=40\nc: image-stack \"frames\" srcs=[tall.png,wide.png,tall.png]\nd: image-stack \"same\" src=wide.png n=2\ne: image \"lost\" src=missing.png\na -> b -> c -> d -> e\n").unwrap();
    let f = draw(&g, &Drawing { pictures: Some(&files), options: LayoutOptions { max_scale: 1.0, frame: Rect::new(0.0, 0.0, 2000.0, 2000.0), ..Default::default() }, ..Default::default() }).unwrap();
    assert_eq!(f.built.pictures, ["wide.png", "tall.png", "missing.png"]);
    // the picture's own proportions when only a size is given
    let a = find(&f, "flow:i/a/picture");
    let Kind::Picture(pa) = &a.kind else { panic!("a real picture is a picture object") };
    assert_eq!(pa.asset, "file:wide.png");
    assert_eq!((a.frame.w, a.frame.h), (60 * 12_700, 30 * 12_700));
    assert_eq!(pa.crop, lore_canvas::Crop::default());
    // cropped to fill a box whose both sides are given
    let b = find(&f, "flow:i/b/picture");
    let Kind::Picture(pb) = &b.kind else { panic!() };
    assert_eq!((b.frame.w, b.frame.h), (40 * 12_700, 40 * 12_700));
    assert!((pb.crop.l - 0.25).abs() < 1e-9 && (pb.crop.r - 0.25).abs() < 1e-9 && pb.crop.t == 0.0);
    // stacks: one picture object per file, front last
    let Kind::Group(c) = &f.objects().iter().find(|o| o.name == "flow:i/c").unwrap().kind else { panic!() };
    let pics: Vec<&str> = c.children.iter().filter_map(|x| if let Kind::Picture(p) = &x.kind { Some(p.asset.as_str()) } else { None }).collect();
    assert_eq!(pics, ["file:tall.png", "file:wide.png", "file:tall.png"]);
    let Kind::Group(d) = &f.objects().iter().find(|o| o.name == "flow:i/d").unwrap().kind else { panic!() };
    assert_eq!(d.children.iter().filter(|x| matches!(x.kind, Kind::Picture(_))).count(), 2);
    assert!(f.routes.iter().all(|r| r.bends() == 0));
    // the pictures travel into the file as media, the rest stays shapes
    let bytes = pptx::single_slide_with(f.objects(), 2000.0, 2000.0, &files).unwrap();
    let od = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    assert_eq!(od.pkg.parts.iter().filter(|p| p.name.starts_with("ppt/media/")).count(), 2);
    let back = pptx::read(&bytes).unwrap();
    let mut n = 0;
    walk(&back.slides[0].objects, &mut |o| {
        if matches!(o.kind, Kind::Picture(_)) {
            n += 1;
        }
    });
    assert_eq!(n, 7);
    let svg = preview::svg_with(f.objects(), 400.0, 200.0, 1.0, &files);
    assert!(svg.contains("data:image/png;base64,"));
}

// ── editable after export ──────────────────────────────────────────────

#[test]
fn everything_stays_an_editable_native_object() {
    let g = dsl::parse(
        "fig e\npanel p \"Stage\"\n x: image \"$x$\"\n enc: encoder \"Enc\" style=3d\n net: nn \"Net\" style=pills\n u: unet \"$\\theta$\"\n br: brace dir=right\n l: loss \"$\\mathcal L$\" style=pill\n y: image-stack \"$y$\" role=gt look=mask\nend\nx -> enc -> net -> u -> br -> l\ny -> l : supervision\nl -> u : feedback \"update\"\nl -> enc : feedback\n",
    )
    .unwrap();
    let f = draw(&g, &Drawing { math: &FakeMath, ..Default::default() }).unwrap();
    let bytes = pptx::single_slide(f.objects(), 720.0, 405.0).unwrap();
    // 1. in the file: only shapes, text boxes, connectors and groups of them
    let od = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let xml = String::from_utf8_lossy(od.pkg.get(&od.slides[0].part).unwrap()).to_string();
    assert!(!xml.contains("<p:pic>") && !xml.contains("<p:graphicFrame>") && !xml.contains("blip"), "nothing is rasterised");
    assert!(od.pkg.parts.iter().all(|p| !p.name.starts_with("ppt/media/")));
    assert!(xml.contains("<p:cxnSp>") && xml.contains("<p:grpSp>") && xml.contains("a14:m"));
    // 2. read back through lore-canvas: same kinds, same texts, lines still attached
    let deck = pptx::read(&bytes).unwrap();
    let slide = &deck.slides[0];
    let mut kinds = BTreeSet::new();
    walk(&slide.objects, &mut |o| {
        kinds.insert(o.kind.name());
    });
    assert_eq!(kinds.into_iter().collect::<Vec<_>>(), ["connector", "group", "shape", "text"]);
    let text_of = |objs: &[Object]| {
        let mut v = Vec::new();
        walk(objs, &mut |o| {
            if let Some(t) = o.kind.sp().and_then(|s| s.text.as_ref()) {
                v.push((o.name.clone(), t.paras.iter().flat_map(|p| p.runs.iter().map(|r| (r.kind.clone(), r.text.clone()))).collect::<Vec<_>>().len()));
            }
        });
        v
    };
    assert_eq!(text_of(&slide.objects), text_of(f.objects()), "every text is still text, run for run");
    let mut math_runs = 0;
    walk(&slide.objects, &mut |o| {
        if let Some(t) = o.kind.sp().and_then(|s| s.text.as_ref()) {
            math_runs += t.paras.iter().flat_map(|p| &p.runs).filter(|r| r.kind == "math" && r.omml.is_some()).count();
        }
    });
    assert!(math_runs >= 4, "variables are equation runs, not pictures of equations");
    // 3. edits go through the usual editor: retype a label, recolour a part, ungroup a module, undo
    let src = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let uid = slide.uid.clone();
    let mut ed = Editor::new(deck.clone(), None);
    let enc = slide.objects.iter().find(|o| o.name == "flow:e/enc").unwrap();
    let Kind::Group(grp) = &enc.kind else { panic!("a module is a group") };
    let n_children = grp.children.len();
    let front = grp.children.iter().find(|c| c.name.ends_with("s0-conv-front")).unwrap().id;
    ed.apply(vec![lore_canvas::Op::SetObj { slide: uid.clone(), id: front, path: "/fill".into(), value: serde_json::json!({ "kind": "solid", "color": "FFC000" }) }], "recolour").unwrap();
    let before = ed.deck.slide(&uid).unwrap().objects.len();
    ed.apply(vec![lore_canvas::Op::Ungroup { slide: uid.clone(), id: enc.id }], "ungroup").unwrap();
    let after = ed.deck.slide(&uid).unwrap();
    assert_eq!(after.objects.len(), before - 1 + n_children, "the parts are now ordinary objects on the slide");
    assert_eq!(after.find(front).unwrap().kind.sp().unwrap().fill, lore_canvas::Paint::solid("FFC000"));
    assert!(after.locate(front).unwrap().0.is_none());
    // the edited deck exports again, and the untouched modules are unchanged in it
    let again = lore_canvas::pptx_out::export_bytes(&ed.deck, &src, None, &|_: &str| None).unwrap();
    let re = pptx::read(&again).unwrap();
    let net = |d: &lore_canvas::Deck| serde_json::to_string(&d.slides[0].objects.iter().find(|o| o.name == "flow:e/net").map(|o| (&o.frame, o.kind.name()))).unwrap();
    assert_eq!(net(&re), net(&deck));
    ed.undo().unwrap();
    ed.undo().unwrap();
    assert_eq!(ed.deck.slide(&uid).unwrap().objects.len(), before);
}

#[test]
fn a_legend_appears_with_four_kinds_of_special_lines() {
    let base = "fig l\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: loss \"$\\mathcal L$\"\ne: image \"gt\" role=gt\na -> b -> c -> d\n";
    let few = dsl::parse(&format!("{base}e -> d : supervision\nd -> a : feedback\n")).unwrap();
    assert!(fig(&few).layout.legend.is_empty());
    let many = dsl::parse(&format!("{base}e -> d : supervision\nd -> a : feedback\na -> c : shared\nb -> d : repeat\nc -> a : up\n")).unwrap();
    let f = fig(&many);
    let kinds: Vec<EdgeKind> = f.layout.legend.iter().map(|l| l.kind).collect();
    assert_eq!(kinds, [EdgeKind::Data, EdgeKind::Supervision, EdgeKind::Feedback, EdgeKind::Shared, EdgeKind::Up, EdgeKind::Repeat]);
    assert!(f.objects().iter().any(|o| o.name == "flow:l/legend:feedback/label"));
    // under the figure, clear of every module, inside the frame
    let top = f.layout.legend.iter().map(|l| l.text.y).fold(f64::MAX, f64::min);
    assert!(f.layout.nodes.iter().all(|n| n.reserve.bottom() < top));
    assert!(LayoutOptions::default().frame.inflate(0.01).contains(&f.layout.bounds));
    routes_are_clean(&many, &f);
    // on and off by hand
    let mut off = many.clone();
    off.legend = Some(false);
    assert!(fig(&off).layout.legend.is_empty());
    let mut on = few.clone();
    on.legend = Some(true);
    assert_eq!(fig(&on).layout.legend.len(), 3);
    let _ = Rect::default();
}

#[test]
fn the_preview_draws_every_object() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    let svg = preview::svg(f.objects(), 720.0, 405.0, 2.0);
    assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
    assert!(svg.contains("stroke-dasharray") && svg.contains("<polygon") && svg.contains("Fuzzy"));
    assert!(svg.contains("ℒ"), "math is drawn as text");
    assert!(!svg.contains("NaN"));
}

#[test]
fn layer_bars_and_blocks_join_the_colour_legend() {
    let t = Theme::from_toml("[colors]\ncycle = [\"D46A8C\", \"7B4FA0\", \"2E75B6\"]\n[layers.gated]\ncolor = \"BDD7EE\"\nlabel = \"Gated convolution\"\n[layers.dilated]\ncolor = \"F4B183\"\nlabel = \"Dilated gated convolution\"\n").unwrap();
    // a kind of layer has one colour in every module of the figure
    let g = dsl::parse("fig b\na: layer-bars layers=[Conv,MBConv,MBConv]\nb: layer-bars layers=[Dense,Conv]\na -> b\n").unwrap();
    let f = draw(&g, &Drawing { theme: t.clone(), ..Default::default() }).unwrap();
    let line = |n: &str| find(&f, n).kind.sp().unwrap().line.as_ref().unwrap().color.clone();
    assert_eq!(line("flow:b/a/bar0"), line("flow:b/b/bar1"), "Conv is the same colour in both");
    let labels: Vec<&str> = f.layout.swatches.iter().map(|s| s.swatch.label.as_str()).collect();
    assert_eq!(labels, ["Conv", "MBConv", "Dense"]);
    // in black and white the bars are all black: no colour legend
    assert!(fig(&g).layout.swatches.is_empty());
    // 3-D blocks name their types through the theme
    let g = dsl::parse("fig c\ng: cuboid-seq n=3 classes=[gated,dilated,gated]\n").unwrap();
    let f = draw(&g, &Drawing { theme: t, ..Default::default() }).unwrap();
    assert_eq!(find(&f, "flow:c/g/b1-front").kind.sp().unwrap().fill, lore_canvas::Paint::solid("F4B183"));
    let labels: Vec<&str> = f.layout.swatches.iter().map(|s| s.swatch.label.as_str()).collect();
    assert_eq!(labels, ["Gated convolution", "Dilated gated convolution"]);
}

#[test]
fn a_mistyped_finish_is_reported() {
    let mut g = dsl::parse("fig t\ne: encoder \"E\" style=bars finish=outlin\n").unwrap();
    let rep = lore_flow::notation::check_graph(&mut g, &lore_notation::Notation::default());
    assert_eq!(rep.by_code("finish_word").len(), 1);
    // and the drawing still comes out, in the theme's finish
    assert!(fig(&g).objects().iter().any(|o| o.name == "flow:t/e"));
}

#[test]
fn the_figures_bounds_hold_every_line_after_a_block_is_moved() {
    let g = dsl::parse("fig t\nx: image \"X\"\ne: box \"Encoder\"\nd: box \"Decoder\"\nl: loss \"Loss\"\nx -> e -> d -> l\nl -> e : feedback\n").unwrap();
    let mut d = Drawing::default();
    d.options.place.insert("e".into(), [0.0, 60.0]);
    let f = draw(&g, &d).unwrap();
    let b = f.layout.bounds;
    for p in f.routes.iter().flat_map(|r| r.points.iter()) {
        assert!(p.0 >= b.x && p.0 <= b.right() && p.1 >= b.y && p.1 <= b.bottom(), "{p:?} outside {b:?}");
    }
}

/// A model wrote `"@h" var=h` on a node and the variable was drawn twice under it (the first
/// real draft of a reference, 2026-11-25). The variable is said once, however it is written.
#[test]
fn a_label_that_is_the_variable_is_drawn_once() {
    let draw = |node: &str| fig(&dsl::parse(&format!("fig v\na: image {node}\nb: box \"B\"\na -> b\nvar h = \"\\hat{{h}}_c\" \"predicted map\"\n")).unwrap());
    let tall = |node: &str| { let f = draw(node); f.layout.nodes[0].reserve.h.max(f.layout.nodes[0].rect.h) };
    let (both, label_only, var_only) = (tall("\"@h\" var=h"), tall("\"@h\""), tall("\"\" var=h"));
    assert!((both - label_only).abs() < 1e-6 && (both - var_only).abs() < 1e-6, "one line of text: {both} {label_only} {var_only}");
    // a label with other words keeps its variable line
    assert!(tall("\"image\" var=h") > label_only + 1.0);
}

/// An icon's shape may be several straight strokes in one path (a chip's pins). The preview
/// joined all their corners into one polygon; each stroke is drawn on its own.
#[test]
fn several_straight_strokes_in_one_shape_are_not_joined() {
    let ic = lore_icons::icon("tabler:cpu").expect("the icon");
    let objs = lore_icons::objects(&ic, lore_canvas::model::Frame::new(0, 0, 914400, 914400), "112233", 1, "cpu");
    let svg = preview::svg(&objs, 72.0, 72.0, 1.0);
    assert!(!svg.contains("<polygon") && !svg.contains("<polyline"), "{svg}");
    let moves = objs.iter().filter_map(|o| o.kind.sp()).map(|sp| sp.geom.path.as_deref().unwrap_or("").matches('M').count()).sum::<usize>();
    assert_eq!(svg.matches(" M").count() + svg.matches("\"M").count(), moves, "every stroke starts on its own");
    assert!(svg.contains("stroke-linecap=\"round\""));
}

// ── icon blocks ──

fn custom_shapes(objs: &[Object]) -> usize {
    let mut n = 0;
    walk(objs, &mut |o| {
        if o.kind.sp().is_some_and(|s| s.geom.prst == "custom") {
            n += 1;
        }
    });
    n
}

fn icon_node(src: &str) -> lore_flow::spec::Node {
    dsl::parse(src).unwrap().nodes.into_iter().find(|n| n.module == "icon").unwrap()
}

fn make_icon(n: &lore_flow::spec::Node) -> lore_flow::modules::Body {
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let lib = Library::default();
    let cx = lore_flow::modules::Ctx { theme: &theme, vars: &vars, math: &NoMath, lib: &lib, k: 1.0, symmetric: true, image_size: &lore_flow::modules::no_images };
    lore_flow::modules::make(n, &cx)
}

#[test]
fn an_icon_block_is_drawn_from_the_set() {
    let g = dsl::parse("fig f\nd: icon \"Physician\" icon=tabler:server\n").unwrap();
    let f = fig(&g);
    assert!(custom_shapes(f.objects()) >= 1);
    let mut texts = Vec::new();
    walk(f.objects(), &mut |o| {
        if let Some(t) = o.kind.sp().and_then(|s| s.text.as_ref()) {
            texts.extend(t.paras.iter().flat_map(|p| p.runs.iter().map(|r| r.text.clone())));
        }
    });
    assert!(texts.iter().any(|t| t.contains("Physician")), "{texts:?}");
    assert!(f.built.warnings.is_empty(), "{:?}", f.built.warnings);
}

#[test]
fn an_icon_block_takes_lines_like_any_block() {
    let g = dsl::parse("fig f\na: box \"A\"\nd: icon \"Doc\" icon=healthicons:doctor-outline\nb: box \"B\"\na -> d -> b\n").unwrap();
    let f = fig(&g);
    assert!(f.built.warnings.is_empty(), "{:?}", f.built.warnings);
    assert_eq!(f.routes.len(), 2);
    assert!(f.routes.iter().all(|r| !r.forced));
}

#[test]
fn an_icon_block_without_an_icon_is_a_dashed_square_and_says_so() {
    for src in ["fig f\nd: icon \"Doc\"\n", "fig f\nd: icon \"Doc\" icon=tabler:no-such-thing-xyz\n"] {
        let n = icon_node(src);
        let body = make_icon(&n);
        assert_eq!(custom_shapes(&body.objs), 0);
        let sq = body.objs.iter().find(|o| o.name == "icon").expect("dashed square");
        assert_eq!(sq.kind.sp().unwrap().line.as_ref().unwrap().dash, "sysDash");
        assert_eq!(sq.frame.w, sq.frame.h);
        assert_eq!(body.warn.len(), 1, "{:?}", body.warn);
        // the figure still draws, with the label, and says what it could not draw
        let f = fig(&dsl::parse(src).unwrap());
        assert!(!f.objects().is_empty());
        assert_eq!(f.built.warnings, body.warn, "the block's line reaches the figure");
    }
    assert!(make_icon(&icon_node("fig f\nd: icon \"Doc\" icon=tabler:server\n")).warn.is_empty());
}

#[test]
fn the_icon_kind_is_in_the_kinds_the_model_is_told() {
    assert!(lore_flow::spec::MODULES.iter().any(|(k, d)| *k == "icon" && d.contains("icon=<set:name>")));
    assert_eq!(dsl::parse("fig f\nd: icon \"x\"\n").unwrap().nodes[0].role_or_default(), "data");
}

#[test]
fn an_icon_block_survives_text_and_back() {
    let g = dsl::parse("fig f\nd: icon \"Physician\" icon=healthicons:doctor-outline\n").unwrap();
    let text = dsl::to_dsl(&g);
    let back = dsl::parse(&text).unwrap();
    assert_eq!(back.nodes[0].module, "icon");
    assert_eq!(back.nodes[0].str("icon"), Some("healthicons:doctor-outline"));
    assert_eq!(back.nodes[0].label, "Physician");
}

// ── PC: colours and regions ────────────────────────────────────────────

fn make_with(src: &str, lib: &Library) -> Vec<Object> {
    let g = dsl::parse_with(src, &lib.kinds()).unwrap();
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let cx = lore_flow::modules::Ctx { theme: &theme, vars: &vars, math: &NoMath, lib, k: 1.0, symmetric: false, image_size: &lore_flow::modules::no_images };
    lore_flow::modules::make(&g.nodes[0], &cx).objs
}

fn body_of(objs: &[Object]) -> &Object {
    objs.iter().find(|o| o.kind.sp().is_some_and(|s| s.fill != lore_canvas::Paint::None)).unwrap()
}

#[test]
fn a_blocks_own_fill_wins_over_its_role() {
    let objs = make_with("fig f\nn: box \"A\" fill=FFE5E5\n", &Library::default());
    assert_eq!(body_of(&objs).kind.sp().unwrap().fill, lore_canvas::Paint::solid("FFE5E5"));
    let plain = make_with("fig f\nn: box \"A\"\n", &Library::default());
    assert_ne!(body_of(&plain).kind.sp().unwrap().fill, lore_canvas::Paint::solid("FFE5E5"));
    // text colour too
    let t = make_with("fig f\nn: box \"A\" text=C62828\n", &Library::default());
    let red = t.iter().any(|o| o.kind.sp().and_then(|s| s.text.as_ref()).is_some_and(|tb| tb.paras.iter().any(|p| p.runs.iter().any(|r| r.look.color.eq_ignore_ascii_case("C62828")))));
    assert!(red);
}

#[test]
fn a_name_alone_gives_fill_and_line() {
    let (pale, strong) = lore_flow::theme::named("blue").unwrap();
    let objs = make_with("fig f\nn: box \"A\" fill=blue\n", &Library::default());
    let sp = body_of(&objs).kind.sp().unwrap();
    assert_eq!(sp.fill, lore_canvas::Paint::solid(&pale));
    assert!(sp.line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong));
    // a line alone changes only the line
    let objs = make_with("fig f\nn: box \"A\" line=blue\n", &Library::default());
    let o = objs.iter().find(|o| o.kind.sp().is_some_and(|s| s.line.is_some())).unwrap();
    assert!(o.kind.sp().unwrap().line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong));
    assert_ne!(body_of(&objs).kind.sp().unwrap().fill, lore_canvas::Paint::solid(&pale));
    // a hex fill leaves the line alone
    let objs = make_with("fig f\nn: box \"A\" fill=FFE5E5\n", &Library::default());
    assert!(!body_of(&objs).kind.sp().unwrap().line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong));
}

fn rect_obj(name: &str, y: i64, fill: &str, line: &str) -> Object {
    use lore_canvas::{Frame, Geom, Paint, SpObj, Stroke};
    Object { id: 0, name: name.into(), frame: Frame::new(0, y, 508_000, 254_000), hidden: false, ph: None, slot: None, kind: Kind::Shape(SpObj { geom: Geom::preset("rect"), fill: Paint::solid(fill), line: Some(Stroke::new(line, 1.0)), text: None, equation: false }), src: None }
}

fn tiny_library() -> Library {
    let objs = vec![rect_obj("one", 0, "role:fill", "role:line"), rect_obj("two", 254_000, "ABCDEF", "ABCDEF")];
    let t = lore_flow::modules::ModuleTemplate { kind: "tiny".into(), style: "x".into(), w: 40.0, h: 40.0, objects: serde_json::from_str(&serde_json::to_string(&objs).unwrap().replace("ROLE:FILL", "role:fill").replace("ROLE:LINE", "role:line")).unwrap(), ..Default::default() };
    let mut lib = Library::default();
    lib.add(t).unwrap();
    lib
}

#[test]
fn a_saved_drawing_takes_the_blocks_colour_where_it_says_role() {
    let lib = tiny_library();
    let objs = make_with("fig f\nn: tiny \"A\" style=x fill=blue\n", &lib);
    let (pale, strong) = lore_flow::theme::named("blue").unwrap();
    let one = objs.iter().find(|o| o.name.ends_with("one")).unwrap().kind.sp().unwrap();
    assert_eq!(one.fill, lore_canvas::Paint::solid(&pale));
    assert!(one.line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong));
}

#[test]
fn a_fixed_colour_in_a_drawing_stays() {
    let lib = tiny_library();
    let objs = make_with("fig f\nn: tiny \"A\" style=x fill=blue\n", &lib);
    let two = objs.iter().find(|o| o.name.ends_with("two")).unwrap().kind.sp().unwrap();
    assert_eq!(two.fill, lore_canvas::Paint::solid("ABCDEF"));
}

fn laid(src: &str) -> (Graph, layout::Layout, Vec<route::Route>) {
    let g = dsl::parse(src).unwrap();
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let lib = Library::default();
    let opt = layout::LayoutOptions { frame: Rect::new(0.0, 0.0, 600.0, 300.0), ..Default::default() };
    let lay = layout::layout(&g, &theme, &vars, &lib, &opt);
    let routes = route::route(&g, &lay);
    (g, lay, routes)
}

fn built(g: &Graph, lay: &layout::Layout, routes: &[route::Route]) -> build::Built {
    let theme = Theme::classic_bw();
    build::build(g, &theme, &Vars::default(), &Library::default(), &NoMath, lay, routes, 2)
}

fn named_obj<'a>(b: &'a build::Built, name: &str) -> &'a Object {
    b.objects.iter().find(|o| o.name == name).unwrap_or_else(|| panic!("{name}"))
}

#[test]
fn a_lines_own_colour_replaces_its_kinds() {
    let (mut g, lay, routes) = laid("fig f\na: box \"A\"\nb: box \"B\"\na -> b : data \"go\"\n");
    let plain = built(&g, &lay, &routes);
    g.edges[0].color = "orange".into();
    let b = built(&g, &lay, &routes);
    let strong = lore_flow::theme::named("orange").unwrap().1;
    assert!(stroke(named_obj(&b, "flow:f/edge:a-b")).color.eq_ignore_ascii_case(&strong));
    assert!(!stroke(named_obj(&plain, "flow:f/edge:a-b")).color.eq_ignore_ascii_case(&strong));
    let l = named_obj(&b, "flow:f/edge:a-b/label");
    assert!(l.kind.sp().unwrap().text.as_ref().unwrap().paras[0].runs[0].look.color.eq_ignore_ascii_case(&strong));
    assert!(b.warnings.iter().all(|w| !w.contains("不認得")));
}

#[test]
fn dash_and_heads_stay_the_kinds() {
    let (mut g, lay, routes) = laid("fig f\na: box \"A\"\nb: box \"B\"\na -> b : optional\n");
    let before = stroke(named_obj(&built(&g, &lay, &routes), "flow:f/edge:a-b")).clone();
    g.edges[0].color = "3366CC".into();
    let b = built(&g, &lay, &routes);
    let after = stroke(named_obj(&b, "flow:f/edge:a-b"));
    assert_eq!(after.color.to_uppercase(), "3366CC");
    assert_eq!((&after.dash, &after.head, &after.tail, after.width_pt), (&before.dash, &before.head, &before.tail, before.width_pt));
}

#[test]
fn an_unknown_line_colour_is_a_warning() {
    let (mut g, lay, routes) = laid("fig f\na: box \"A\"\nb: box \"B\"\na -> b : data\n");
    let plain = built(&g, &lay, &routes);
    g.edges[0].color = "chartreuse".into();
    let b = built(&g, &lay, &routes);
    assert!(b.warnings.contains(&"a → b: 不認得的顏色 chartreuse".to_string()), "{:?}", b.warnings);
    assert_eq!(stroke(named_obj(&b, "flow:f/edge:a-b")).color, stroke(named_obj(&plain, "flow:f/edge:a-b")).color);
}

fn rbox(id: &str, rect: Rect, depth: usize) -> layout::RegionBox {
    layout::RegionBox { id: id.into(), title: id.to_uppercase(), rect, title_rect: Some(Rect::new(rect.x, rect.y - 14.0, rect.w, 12.0)), depth, line: "solid".into(), title_at: "above".into(), ..Default::default() }
}

fn index(b: &build::Built, name: &str) -> usize {
    b.objects.iter().position(|o| o.name == name).unwrap_or_else(|| panic!("{name}"))
}

#[test]
fn a_region_is_drawn_under_its_blocks() {
    let (g, mut lay, routes) = laid("fig f\na: box \"A\"\nb: box \"B\"\na -> b\n");
    let r = lay.nodes[0].rect;
    lay.regions.push(rbox("grp", r.inflate(8.0), 0));
    let b = built(&g, &lay, &routes);
    let ri = index(&b, "flow:f/region:grp");
    assert!(ri < index(&b, "flow:f/a") && ri < index(&b, "flow:f/b") && ri < index(&b, "flow:f/edge:a-b"));
    let sp = b.objects[ri].kind.sp().unwrap();
    assert_eq!(sp.fill, lore_canvas::Paint::None);
    assert_eq!(sp.line.as_ref().unwrap().dash, "solid");
}

#[test]
fn a_tinted_dashed_round_region() {
    let (g, mut lay, routes) = laid("fig f\na: box \"A\"\n");
    let mut rb = rbox("grp", lay.nodes[0].rect.inflate(8.0), 0);
    rb.tint = "green".into();
    rb.line = "dashed".into();
    rb.round = true;
    lay.regions.push(rb);
    let b = built(&g, &lay, &routes);
    let sp = b.objects[index(&b, "flow:f/region:grp")].kind.sp().unwrap();
    let (pale, strong) = lore_flow::theme::named("green").unwrap();
    assert_eq!(sp.geom.prst, "roundRect");
    assert_eq!(sp.fill, lore_canvas::Paint::solid(&pale));
    let l = sp.line.as_ref().unwrap();
    assert_eq!(l.dash, "dash");
    assert!(l.color.eq_ignore_ascii_case(&strong));
    lay.regions[0].line = "dotted".into();
    let b = built(&g, &lay, &routes);
    assert_eq!(b.objects[index(&b, "flow:f/region:grp")].kind.sp().unwrap().line.as_ref().unwrap().dash, "sysDot");
    lay.regions[0].line = "none".into();
    let b = built(&g, &lay, &routes);
    assert!(b.objects[index(&b, "flow:f/region:grp")].kind.sp().unwrap().line.is_none());
}

#[test]
fn a_region_title_sits_in_its_rect() {
    let (g, mut lay, routes) = laid("fig f\na: box \"A\"\n");
    let mut rb = rbox("grp", lay.nodes[0].rect.inflate(20.0), 0);
    rb.title = "Backbone".into();
    let t = rb.title_rect.unwrap();
    lay.regions.push(rb);
    let b = built(&g, &lay, &routes);
    let o = &b.objects[index(&b, "flow:f/region:grp/title")];
    let emu = lore_flow::modules::emu;
    assert_eq!((o.frame.x, o.frame.y, o.frame.w, o.frame.h), (emu(t.x), emu(t.y), emu(t.w), emu(t.h)));
    let tb = o.kind.sp().unwrap().text.as_ref().unwrap();
    assert_eq!(tb.plain_text(), "Backbone");
    assert_eq!(tb.paras[0].align, "ctr");
    assert!(tb.paras[0].runs[0].look.bold);
    lay.regions[0].title_at = "top-left".into();
    let b = built(&g, &lay, &routes);
    assert_eq!(b.objects[index(&b, "flow:f/region:grp/title")].kind.sp().unwrap().text.as_ref().unwrap().paras[0].align, "l");
    lay.regions[0].title_at = "left".into();
    let b = built(&g, &lay, &routes);
    assert_eq!(b.objects[index(&b, "flow:f/region:grp/title")].kind.sp().unwrap().text.as_ref().unwrap().vert.as_deref(), Some("vert270"));
    lay.regions[0].title_rect = None;
    let b = built(&g, &lay, &routes);
    assert!(!b.objects.iter().any(|o| o.name.ends_with("region:grp/title")));
}

#[test]
fn inner_regions_are_drawn_over_outer() {
    let (g, mut lay, routes) = laid("fig f\na: box \"A\"\n");
    let r = lay.nodes[0].rect;
    lay.regions.push(rbox("outer", r.inflate(20.0), 0));
    lay.regions.push(rbox("inner", r.inflate(8.0), 1));
    let b = built(&g, &lay, &routes);
    assert!(index(&b, "flow:f/region:outer") < index(&b, "flow:f/region:inner"));
    assert!(index(&b, "flow:f/region:inner") < index(&b, "flow:f/a"));
}

#[test]
fn no_regions_draws_exactly_what_it_drew() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    assert!(f.layout.regions.is_empty());
    assert!(f.objects().iter().all(|o| o.name.starts_with("flow:") && !o.name.contains("/region:")));
}

fn chart_parts(f: &Figure) -> Vec<Object> {
    let mut v = Vec::new();
    walk(f.objects(), &mut |o| {
        let part = o.name.rsplit('/').next().unwrap_or("");
        if o.name.contains("/m/") && (part == "chart" || part.starts_with("chart-")) {
            v.push(o.clone());
        }
    });
    v
}

#[test]
fn a_chart_block_is_drawn_in_a_figure() {
    let f = fig(&dsl::parse("fig c\nm: chart \"Membership\" chart=membership n=3\nb: box \"B\"\nm -> b\n").unwrap());
    let charts = chart_parts(&f);
    assert!(charts.len() >= 3, "{}", charts.len());
    assert!(charts.iter().all(|o| matches!(&o.kind, Kind::Shape(s) if s.geom.prst == "custom")));
    assert!(charts.iter().all(|o| o.kind.sp().unwrap().line.as_ref().is_none_or(|l| l.dash != "sysDash")));
}

#[test]
fn a_chart_block_without_a_chart_is_a_dashed_slot() {
    let f = fig(&dsl::parse("fig c\nm: chart \"Nothing\"\nb: box \"B\"\nm -> b\n").unwrap());
    let charts = chart_parts(&f);
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0].kind.sp().unwrap().line.as_ref().unwrap().dash, "sysDash");
}

// ── a block's own words, bars in their own colours ─────────────────────

fn text_rect(name: &str, y: i64, words: &[&str]) -> Object {
    let mut o = rect_obj(name, y, "role:fill", "role:line");
    let ty = lore_flow::text::Type { font: "Arial".into(), ea_font: String::new(), math_font: "Cambria Math".into(), size: 10.0, bold: false, italic: false, color: "000000".into() };
    let (mut tb, _) = lore_flow::text::body(words[0], &ty, &Vars::default(), &NoMath, "ctr", "ctr");
    for w in &words[1..] {
        let (rs, _) = lore_flow::text::runs(w, &ty, &Vars::default(), &NoMath);
        tb.paras[0].runs.extend(rs);
    }
    o.kind.sp_mut().unwrap().text = Some(tb);
    o
}

/// Two fixed words ("Alpha" in `top`, "Beta" in `low`, in one paragraph with the label).
fn worded_library() -> Library {
    let objs = vec![text_rect("top", 0, &["Alpha"]), text_rect("low", 254_000, &["Beta", "{{label}}"])];
    let t = lore_flow::modules::ModuleTemplate { kind: "worded".into(), style: "x".into(), w: 40.0, h: 40.0, objects: serde_json::to_value(&objs).unwrap(), ..Default::default() };
    let mut lib = Library::default();
    lib.add(t).unwrap();
    lib
}

fn all_text(objs: &[Object]) -> Vec<String> {
    objs.iter().filter_map(|o| o.kind.sp().and_then(|s| s.text.as_ref())).map(|tb| tb.paras.iter().map(|p| p.runs.iter().map(|r| r.text.clone()).collect::<String>()).collect::<Vec<_>>().join("")).collect()
}

fn make_body(src: &str, lib: &Library) -> lore_flow::modules::Body {
    let g = dsl::parse_with(src, &lib.kinds()).unwrap();
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let cx = lore_flow::modules::Ctx { theme: &theme, vars: &vars, math: &NoMath, lib, k: 1.0, symmetric: false, image_size: &lore_flow::modules::no_images };
    lore_flow::modules::make(&g.nodes[0], &cx)
}

fn texts_of(src: &str, lib: &Library) -> Vec<String> {
    all_text(&make_body(src, lib).objs)
}

#[test]
fn texts_replace_a_drawings_fixed_words_in_order() {
    let lib = worded_library();
    let t = texts_of("fig f\nn: worded \"L\" style=x texts=[One,Two]\n", &lib);
    assert!(t.contains(&"One".to_string()) && t.contains(&"TwoL".to_string()), "{t:?}");
    assert!(!t.iter().any(|s| s.contains("Alpha") || s.contains("Beta")));
}

#[test]
fn an_underscore_keeps_the_word_and_an_empty_text_blanks_it() {
    let lib = worded_library();
    let t = texts_of("fig f\nn: worded \"L\" style=x texts=[_,Two]\n", &lib);
    assert!(t.contains(&"Alpha".to_string()) && t.contains(&"TwoL".to_string()), "{t:?}");
    let t = texts_of("fig f\nn: worded \"L\" style=x texts=[\"\",_]\n", &lib);
    assert!(t.contains(&String::new()) && t.contains(&"BetaL".to_string()), "{t:?}");
}

#[test]
fn a_named_object_takes_its_text() {
    let lib = worded_library();
    let t = texts_of("fig f\nn: worded \"L\" style=x text.low=Gamma\n", &lib);
    assert!(t.contains(&"Alpha".to_string()) && t.contains(&"GammaL".to_string()), "{t:?}");
    // named comes after the list
    let t = texts_of("fig f\nn: worded \"L\" style=x texts=[One,Two] text.top=Gamma\n", &lib);
    assert!(t.contains(&"Gamma".to_string()) && t.contains(&"TwoL".to_string()), "{t:?}");
}

#[test]
fn too_many_texts_is_a_warning() {
    let lib = worded_library();
    let b = make_body("fig f\nn: worded \"L\" style=x texts=[a,b,c]\n", &lib);
    assert!(b.warn.iter().any(|w| w == "n: 這個積木只有 2 處文字"), "{:?}", b.warn);
    let b = make_body("fig f\nn: worded \"L\" style=x texts=[a,b]\n", &lib);
    assert!(b.warn.is_empty());
}

#[test]
fn a_block_without_texts_is_drawn_as_before() {
    let lib = worded_library();
    let t = texts_of("fig f\nn: worded \"L\" style=x\n", &lib);
    assert!(t.contains(&"Alpha".to_string()) && t.contains(&"BetaL".to_string()), "{t:?}");
}

fn flat(objs: &[Object]) -> Vec<Object> {
    let mut out = Vec::new();
    for o in objs {
        out.push(o.clone());
        if let Kind::Group(g) = &o.kind {
            out.extend(flat(&g.children));
        }
    }
    out
}

fn bar_objs(src: &str) -> lore_flow::modules::Body {
    make_body(src, &Library::default())
}

fn bar<'a>(b: &'a lore_flow::modules::Body, i: usize) -> &'a lore_canvas::SpObj {
    b.objs.iter().find(|o| o.name == format!("bar{i}")).unwrap().kind.sp().unwrap()
}

#[test]
fn bars_take_their_colours_in_order() {
    let b = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,MBConv1,Pool] colors=[pink,purple,blue]\n");
    for (i, c) in ["pink", "purple", "blue"].iter().enumerate() {
        let (_, strong) = lore_flow::theme::named(c).unwrap();
        assert!(bar(&b, i).line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong), "bar {i}");
    }
    // the fill stays the default
    assert_eq!(bar(&b, 0).fill, lore_canvas::Paint::solid(&Theme::classic_bw().colors.paper));
}

#[test]
fn the_last_colour_repeats() {
    let b = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,Conv,Conv,Conv] colors=[pink,_,blue]\n");
    let line = |i| bar(&b, i).line.as_ref().unwrap().color.to_uppercase();
    let (_, pink) = lore_flow::theme::named("pink").unwrap();
    let (_, blue) = lore_flow::theme::named("blue").unwrap();
    assert_eq!(line(0), pink.to_uppercase());
    assert_ne!(line(1), pink.to_uppercase());
    assert_eq!(line(2), blue.to_uppercase());
    assert_eq!(line(3), blue.to_uppercase());
}

#[test]
fn filled_bars_take_the_pale_fill() {
    let b = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,Conv] colors=[pink] filled=true\n");
    let (pale, _) = lore_flow::theme::named("pink").unwrap();
    assert_eq!(bar(&b, 1).fill, lore_canvas::Paint::solid(&pale));
}

#[test]
fn an_unknown_bar_colour_is_a_warning() {
    let b = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,Conv,Conv] colors=[mauve]\n");
    assert_eq!(b.warn.iter().filter(|w| w.as_str() == "q: 不認得的顏色 mauve").count(), 1, "{:?}", b.warn);
    let plain = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,Conv,Conv]\n");
    assert_eq!(bar(&b, 0).line.as_ref().unwrap().color, bar(&plain, 0).line.as_ref().unwrap().color);
}

#[test]
fn a_line_runs_through_the_bars() {
    let b = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,Conv,Conv] through=blue\n");
    let pos = |name: &str| b.objs.iter().position(|o| o.name == name).unwrap();
    let line = &b.objs[pos("through")];
    match &line.kind {
        Kind::Connector(c) => {
            let (_, strong) = lore_flow::theme::named("blue").unwrap();
            assert!(c.line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong));
            assert_eq!(c.line.as_ref().unwrap().dash, "dash");
        }
        _ => panic!("not a line"),
    }
    // beneath the bars, which are opaque, so it never crosses a bar's name
    assert!(pos("through") < pos("bar0") && pos("bar2") < pos("bartext0"));
    assert!((0..3).all(|i| matches!(bar(&b, i).fill, lore_canvas::Paint::Solid { .. })));
    assert!(bar(&b, 0).text.is_none());
    assert!(line.frame.w > 0 && line.frame.h == 0);
    let plain = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,Conv,Conv]\n");
    assert!(plain.objs.iter().all(|o| o.name != "through"));
    let ink = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv] through=true\n");
    assert!(ink.objs.iter().any(|o| o.name == "through"));
}

#[test]
fn bars_without_colours_are_drawn_as_before() {
    let b = bar_objs("fig f\nq: layer-bars \"L\" layers=[Conv,FC]\n");
    assert!(b.warn.is_empty());
    assert!(b.objs.iter().all(|o| o.name != "through" && !o.name.starts_with("bartext")));
    assert!(bar(&b, 0).text.is_some());
}

#[test]
fn a_library_block_shows_the_figures_own_words() {
    let lib = worded_library();
    let g = dsl::parse_with("fig f\nn: worded \"L\" style=x texts=[Enc,Dec]\n", &lib.kinds()).unwrap();
    let f = draw(&g, &Drawing { library: lib, ..Default::default() }).unwrap();
    let words = all_text(&flat(f.objects()));
    assert!(words.iter().any(|s| s == "Enc") && !words.iter().any(|s| s.contains("Alpha")), "{words:?}");
}

#[test]
fn coloured_bars_through_the_figure_text() {
    let g = dsl::parse("fig f\nq: layer-bars \"L\" layers=[Conv,MBConv1] colors=[pink,purple] through=true\n").unwrap();
    let f = fig(&g);
    let (_, pink) = lore_flow::theme::named("pink").unwrap();
    assert!(flat(f.objects()).iter().any(|o| o.name.ends_with("bar0") && o.kind.sp().is_some_and(|s| s.line.as_ref().is_some_and(|l| l.color.eq_ignore_ascii_case(&pink)))));
    assert!(flat(f.objects()).iter().any(|o| o.name.ends_with("through")));
}

// ── panel options and a line's own width ───────────────────────────────

const PANELED: &str = "fig f\npanel p \"P. Title\"\n  a: box \"A\"\n  b: box \"B\"\nend\na -> b\n";

fn panel_with(opts: &str) -> String {
    PANELED.replace("panel p \"P. Title\"", &format!("panel p \"P. Title\" {opts}"))
}

fn rect_nums(r: &Rect) -> [f64; 4] {
    [r.x, r.y, r.w, r.h].map(|v| (v * 100.0).round() / 100.0)
}

#[test]
fn a_panel_takes_a_tint_a_dashed_frame_and_round_corners() {
    let g = dsl::parse(&panel_with("dashed tint=green round")).unwrap();
    let p = &g.panels[0];
    assert_eq!((p.line.as_str(), p.tint.as_str(), p.round), ("dashed", "green", true));
    let (g, lay, routes) = laid(&panel_with("dashed tint=green round"));
    let b = built(&g, &lay, &routes);
    let sp = named_obj(&b, "flow:f/panel:p").kind.sp().unwrap();
    let (pale, strong) = lore_flow::theme::named("green").unwrap();
    assert_eq!(sp.geom.prst, "roundRect");
    assert_eq!(sp.fill, lore_canvas::Paint::solid(&pale));
    let line = sp.line.as_ref().unwrap();
    assert_eq!(line.dash, "dash");
    assert!(line.color.eq_ignore_ascii_case(&strong));
    // a hex tint is its own fill
    let (g, lay, routes) = laid(&panel_with("dotted tint=1F4E79"));
    let b = built(&g, &lay, &routes);
    let sp = named_obj(&b, "flow:f/panel:p").kind.sp().unwrap();
    assert_eq!(sp.fill, lore_canvas::Paint::solid("1F4E79"));
    assert_eq!(sp.line.as_ref().unwrap().dash, "sysDot");
    assert_eq!(sp.geom.prst, "rect");
}

#[test]
fn a_panel_without_a_frame_keeps_its_tint() {
    let (g, lay, routes) = laid(&panel_with("none tint=blue"));
    let b = built(&g, &lay, &routes);
    let sp = named_obj(&b, "flow:f/panel:p").kind.sp().unwrap();
    assert_eq!(sp.fill, lore_canvas::Paint::solid(&lore_flow::theme::named("blue").unwrap().0));
    assert!(sp.line.is_none());
    // bare wins over everything
    let (g, lay, routes) = laid(&panel_with("bare dashed tint=blue"));
    let b = built(&g, &lay, &routes);
    assert!(!b.objects.iter().any(|o| o.name.starts_with("flow:f/panel:p")));
}

#[test]
fn a_panel_title_goes_above_or_is_left_out() {
    let (_, lay_in, _) = laid(PANELED);
    let (g, lay, routes) = laid(&panel_with("title=above"));
    let b = built(&g, &lay, &routes);
    let frame = named_obj(&b, "flow:f/panel:p");
    let title = named_obj(&b, "flow:f/panel:p/title");
    assert!(title.frame.y + title.frame.h <= frame.frame.y + 1, "title not above the frame");
    // the frame does not grow for an outside title
    assert!(lay.panels[0].rect.h <= lay_in.panels[0].rect.h + 0.01, "frame grew: {} vs {}", lay.panels[0].rect.h, lay_in.panels[0].rect.h);
    // below
    let (g, lay, routes) = laid(&panel_with("title=below"));
    let b = built(&g, &lay, &routes);
    let frame = named_obj(&b, "flow:f/panel:p");
    let title = named_obj(&b, "flow:f/panel:p/title");
    assert!(title.frame.y >= frame.frame.y + frame.frame.h - 1);
    // none: no title object, and the panel is shorter than with an inside title
    let (g, lay_none, routes) = laid(&panel_with("title=none"));
    let b = built(&g, &lay_none, &routes);
    assert!(!b.objects.iter().any(|o| o.name == "flow:f/panel:p/title"));
    assert!(lay_none.panels[0].title_rect.is_none());
    assert!(lay_none.panels[0].rect.h < lay_in.panels[0].rect.h, "{} vs {}", lay_none.panels[0].rect.h, lay_in.panels[0].rect.h);
    // left: an upright title inside the frame
    let (g, lay, routes) = laid(&panel_with("title=left"));
    let b = built(&g, &lay, &routes);
    let t = named_obj(&b, "flow:f/panel:p/title");
    assert_eq!(t.kind.sp().unwrap().text.as_ref().unwrap().vert.as_deref(), Some("vert270"));
    assert!(lay.panels[0].rect.contains(&lay.panels[0].title_rect.unwrap()));
    assert!(lay.nodes.iter().all(|n| n.rect.x >= lay.panels[0].title_rect.unwrap().right() - 0.01));
}

#[test]
fn panel_options_survive_text_and_back() {
    let g = dsl::parse(&panel_with("dir=down dashed tint=green round title=above")).unwrap();
    let text = dsl::to_dsl(&g);
    assert!(text.contains("panel p \"P. Title\" dir=down dashed tint=green round title=above\n"), "{text}");
    assert_eq!(dsl::parse(&text).unwrap(), g);
    let plain = serde_json::to_string(&dsl::parse(PANELED).unwrap().panels[0]).unwrap();
    assert_eq!(plain, r#"{"id":"p","title":"P. Title"}"#);
}

#[test]
fn a_panel_without_options_prints_and_lays_out_as_before() {
    let g = dsl::parse(PANELED).unwrap();
    assert_eq!(dsl::to_dsl(&g), "fig f\npanel p \"P. Title\"\n  a: box \"A\"\n  b: box \"B\"\nend\na -> b\n");
    let (_, lay, _) = laid(PANELED);
    assert_eq!(rect_nums(&lay.panels[0].rect), [202.5, 98.13, 195.0, 103.74]);
    assert_eq!(lay.panels[0].title_rect.map(|r| rect_nums(&r)), Some([213.3, 105.33, 67.79, 25.74]));
    assert_eq!(lay.nodes.iter().map(|n| rect_nums(&n.rect)).collect::<Vec<_>>(), vec![[220.5, 144.87, 60.0, 39.0], [319.5, 144.87, 60.0, 39.0]]);
}

#[test]
fn validation_refuses_an_unknown_panel_line() {
    let mut g = dsl::parse(PANELED).unwrap();
    g.panels[0].line = "wavy".into();
    assert!(matches!(g.validate(&[]), Err(SpecError::PanelLine { .. })));
    g.panels[0].line = "dashed".into();
    g.panels[0].title_at = "middle".into();
    assert!(matches!(g.validate(&[]), Err(SpecError::PanelTitleAt { .. })));
    assert!(dsl::parse(&panel_with("line=wavy")).is_err());
}

#[test]
fn a_line_takes_its_own_width() {
    let (mut g, lay, routes) = laid("fig f\na: box \"A\"\nb: box \"B\"\na -> b : data\n");
    let plain = built(&g, &lay, &routes);
    g.edges[0].width = 4.0;
    let b = built(&g, &lay, &routes);
    let (w0, w1) = (stroke(named_obj(&plain, "flow:f/edge:a-b")).width_pt, stroke(named_obj(&b, "flow:f/edge:a-b")).width_pt);
    assert!(w1 > w0, "{w1} vs {w0}");
    assert_eq!(dsl::parse("fig f\na: box \"A\"\nb: box \"B\"\na -> b : data width=2.5\n").unwrap().edges[0].width, 2.5);
}

#[test]
fn the_width_of_a_chain_applies_to_each_line() {
    let g = dsl::parse("fig f\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\na -> b -> c : data width=3\n").unwrap();
    assert_eq!(g.edges.len(), 2);
    assert!(g.edges.iter().all(|e| e.width == 3.0));
}

#[test]
fn a_line_width_survives_text_and_back() {
    let g = dsl::parse("fig f\na: box \"A\"\nb: box \"B\"\na -> b color=blue width=2.5\n").unwrap();
    let t = dsl::to_dsl(&g);
    assert!(t.contains("a -> b color=blue width=2.5\n"), "{t}");
    assert_eq!(dsl::parse(&t).unwrap(), g);
    let plain = dsl::parse("fig f\na: box \"A\"\nb: box \"B\"\na -> b\n").unwrap();
    assert!(!serde_json::to_string(&plain.edges[0]).unwrap().contains("width"));
}

#[test]
fn validation_refuses_a_silly_width() {
    let mut g = dsl::parse("fig f\na: box \"A\"\nb: box \"B\"\na -> b\n").unwrap();
    for w in [-1.0, 13.0, f64::NAN] {
        g.edges[0].width = w;
        assert!(matches!(g.validate(&[]), Err(SpecError::EdgeWidth { .. })), "{w}");
    }
    g.edges[0].width = 12.0;
    assert!(g.validate(&[]).is_ok());
    assert!(dsl::parse("fig f\na: box \"A\"\nb: box \"B\"\na -> b width=thick\n").is_err());
}

/// Found by a review: `fill=123456` was read as a number and the colour was dropped without a
/// word; a curved line was offered runs to slide that its curve never reads.
#[test]
fn a_six_digit_colour_is_a_colour_and_a_curve_has_no_runs() {
    let g = dsl::parse("fig t\na: box \"A\" fill=123456 line=000000\nb: box \"B\"\na -> b : data curve\n").unwrap();
    assert_eq!(g.nodes[0].str("fill"), Some("123456"));
    assert_eq!(g.nodes[0].str("line"), Some("000000"));
    let f = fig(&g);
    for r in &f.routes {
        let a = g.node_index(&g.edges[r.edge].from).unwrap();
        assert!(lore_flow::route::runs(&f.layout, a, r).is_empty());
    }
}

// ── Q: lines beneath the blocks, straight before jogged ───────────────────

fn route_of<'a>(g: &Graph, f: &'a Figure, from: &str, to: &str) -> &'a lore_flow::route::Route {
    f.routes.iter().find(|r| g.edges[r.edge].from == from && g.edges[r.edge].to == to).unwrap()
}

/// 0 = frame (panel / region), 1 = line, 2 = block, 3 = dot or label over the blocks, 4 = legend.
fn layer_of(name: &str, g: &Graph) -> u8 {
    let rest = name.split_once('/').map(|x| x.1).unwrap_or("");
    if rest.starts_with("panel:") || rest.starts_with("region:") {
        0
    } else if rest.starts_with("edge:") {
        if rest.ends_with("/label") || rest.ends_with("/junction") { 3 } else { 1 }
    } else if g.nodes.iter().any(|n| n.id == rest) {
        2
    } else {
        4
    }
}

#[test]
fn lines_are_drawn_beneath_the_blocks() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    let layers: Vec<u8> = f.objects().iter().map(|o| layer_of(&o.name, &g)).collect();
    assert!(layers.contains(&1) && layers.contains(&2) && layers.contains(&3), "{layers:?}");
    let last = |l: u8| layers.iter().rposition(|x| *x == l);
    let first = |l: u8| layers.iter().position(|x| *x == l);
    assert!(last(0).unwrap() < first(1).unwrap(), "frames, then lines: {layers:?}");
    assert!(last(1).unwrap() < first(2).unwrap(), "every line before every block: {layers:?}");
    assert!(last(2).unwrap() < first(3).unwrap(), "labels over the blocks: {layers:?}");
}

#[test]
fn connectors_still_attach_to_their_blocks() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    let ids: BTreeSet<u32> = f.built.node_ids.values().copied().collect();
    assert_eq!(f.built.node_ids.len(), g.nodes.len());
    let mut attached = 0;
    for o in f.objects() {
        if let Kind::Connector(c) = &o.kind {
            for a in c.start.iter().chain(c.end.iter()) {
                assert!(ids.contains(&a.id), "{} attaches to {}", o.name, a.id);
                attached += 1;
            }
        }
    }
    assert!(attached > 10);
    for (id, oid) in &f.built.node_ids {
        let o = f.objects().iter().find(|o| o.id == *oid).unwrap();
        assert_eq!(o.name, format!("flow:demo/{id}"));
    }
}

#[test]
fn object_ids_are_unique_with_lines_first() {
    let g = dsl::parse(TRAIN).unwrap();
    let f = fig(&g);
    let mut seen = BTreeSet::new();
    let mut top = 0;
    walk(f.objects(), &mut |o| {
        assert!(seen.insert(o.id), "id {} used twice ({})", o.id, o.name);
        top = top.max(o.id);
    });
    assert!(f.built.next_id > top, "next_id {} must be free (largest {top})", f.built.next_id);
}

/// a, b, c in a row and a curved line a → c that has to cross b.
fn curved_across() -> Graph {
    let mut g = dsl::parse("fig cv\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\na -> b -> c\na -> c\n").unwrap();
    g.edges[2].curve = true;
    g
}

#[test]
fn a_curve_through_a_block_becomes_a_routed_line_and_says_so() {
    let g = curved_across();
    let f = fig(&g);
    assert!(f.built.warnings.iter().any(|w| w == "a → c: 曲線會穿過 b，改走直角"), "{:?}", f.built.warnings);
    let o = f.objects().iter().find(|o| o.name == "flow:cv/edge:a-c").unwrap();
    // a routed line (a connector, or a polyline when it has many corners), not the smooth curve
    assert!(!matches!(&o.kind, Kind::Shape(sp) if sp.geom.prst == "custom" && sp.geom.path.as_deref().is_some_and(|p| p.contains(" C "))), "not drawn as a curve");
}

#[test]
fn a_clear_curve_stays_a_curve() {
    let mut g = dsl::parse("fig cv\na: box \"A\"\nb: box \"B\"\na -> b\n").unwrap();
    g.edges[0].curve = true;
    let f = fig(&g);
    assert!(f.built.warnings.is_empty(), "{:?}", f.built.warnings);
    let o = f.objects().iter().find(|o| o.name == "flow:cv/edge:a-b").unwrap();
    assert!(matches!(&o.kind, Kind::Shape(sp) if sp.geom.prst == "custom"));
}

#[test]
fn a_level_line_keeps_the_middle_when_it_shares_a_side() {
    let g = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c\nb -> d\n").unwrap();
    let f = fig(&g);
    for (x, y) in [("a", "b"), ("b", "c")] {
        let r = route_of(&g, &f, x, y);
        assert_eq!(r.bends(), 0, "{x} -> {y}: {:?}", r.points);
    }
    // b -> c leaves the middle; the other line leaves a quarter point
    assert_eq!(route_of(&g, &f, "b", "c").from_site, 3);
    assert_ne!(route_of(&g, &f, "b", "d").from_site, 3);
}

#[test]
fn blocks_in_one_lane_share_a_port_line() {
    let g = dsl::parse("fig l\na: box \"A\"\nb: image \"$x$\" caption=\"a long caption below\"\nc: box \"C\"\na -> b -> c\n").unwrap();
    let f = fig(&g);
    let n = &f.layout.nodes;
    assert!((n[0].port_y - n[1].port_y).abs() < 0.5 && (n[1].port_y - n[2].port_y).abs() < 0.5, "{} {} {}", n[0].port_y, n[1].port_y, n[2].port_y);
    for (x, y) in [("a", "b"), ("b", "c")] {
        assert_eq!(route_of(&g, &f, x, y).bends(), 0);
    }
}

#[test]
fn a_smaller_block_is_levelled_with_its_neighbour() {
    // a small block in one panel, a lane of tall ones in the next: the small one is moved
    // inside its frame until the line between them is straight
    let g = dsl::parse("fig v\narrange p q\npanel p \"P\" dir=right\n a: box \"A\"\nend\npanel q \"Q\" dir=right\n b: box \"B\"\n e: box \"E\" lane=1 col=0\n f: box \"F\" lane=2 col=0\nend\na -> b\n").unwrap();
    let f = fig(&g);
    let r = route_of(&g, &f, "a", "b");
    assert_eq!(r.bends(), 0, "{:?}", r.points);
    let (pa, panel) = (&f.layout.nodes[0], &f.layout.panels[f.layout.nodes[0].panel]);
    assert!(pa.reserve.y >= panel.rect.y - 1e-6 && pa.reserve.bottom() <= panel.rect.bottom() + 1e-6, "inside its frame");
    no_overlaps(&f);
}

#[test]
fn a_pinned_block_stays_in_its_cell() {
    let g = dsl::parse("fig p\na: box \"A\"\nb: box \"B\"\nc: box \"C\" col=0 lane=1\na -> b\nc -> b\n").unwrap();
    let f = fig(&g);
    let n = &f.layout.nodes;
    assert_eq!((n[2].layer, n[2].lane), (0, 1));
    assert!((n[2].rect.cx() - n[0].rect.cx()).abs() < 1e-6, "still in a's column");
    assert!(n[2].rect.y > n[0].rect.bottom());
    no_overlaps(&f);
}

#[test]
fn figures_without_these_cases_route_as_before() {
    // captured from the router before the change: no shared sides, no size mismatch
    let pts = |src: &str| -> Vec<Vec<(f64, f64)>> {
        let f = fig(&dsl::parse(src).unwrap());
        f.routes.iter().map(|r| r.points.iter().map(|p| (p.0.round(), p.1.round())).collect()).collect()
    };
    assert_eq!(
        pts("fig a\npanel p \"P\" dir=right\n x: box \"X\"\n y: box \"Y\"\n z: box \"Z\"\nend\npanel q \"Q\" dir=down\n u: box \"U\"\n v: box \"V\"\nend\nx -> y -> z\nu -> v\nz -> u\n"),
        vec![vec![(233.0, 217.0), (272.0, 217.0)], vec![(332.0, 217.0), (371.0, 217.0)], vec![(518.0, 205.0), (518.0, 229.0)], vec![(401.0, 197.0), (401.0, 185.0), (488.0, 185.0)]]
    );
    assert_eq!(
        pts("fig b\nx: image \"$x$\"\nn: box \"Net\"\nl: loss \"L\"\nt: image \"$y$\"\nx -> n -> l\nt -> l : supervision\nl -> n : feedback\n"),
        vec![vec![(262.0, 203.0), (301.0, 203.0)], vec![(361.0, 203.0), (400.0, 203.0)], vec![(458.0, 203.0), (419.0, 203.0)], vec![(410.0, 216.0), (410.0, 233.0), (331.0, 233.0), (331.0, 222.0)]]
    );
}

// ── corner schemes and layered themes ──────────────────────────────────

const GRADED: &str = "id = \"graded\"\npart = \"corners\"\n[corners]\npanel = 14\nregion = 9\ninner = 5\ndata = 0\nstep = 7\nloss = -1\n[node]\nloss = \"box\"\n";
const HARD: &str = "id = \"hard\"\npart = \"corners\"\n[corners]\n[node]\nloss = \"box\"\n";

fn built_with(theme: &Theme, g: &Graph, lay: &layout::Layout, routes: &[route::Route]) -> build::Built {
    build::build(g, theme, &Vars::default(), &Library::default(), &NoMath, lay, routes, 2)
}

fn body_with(theme: &Theme, src: &str) -> Object {
    let g = dsl::parse(&format!("fig f\n{src}\n")).unwrap();
    let (vars, lib) = (Vars::default(), Library::default());
    let cx = lore_flow::modules::Ctx { theme, vars: &vars, math: &NoMath, lib: &lib, k: 1.0, symmetric: false, image_size: &lore_flow::modules::no_images };
    lore_flow::modules::make(&g.nodes[0], &cx).objs.into_iter().find(|o| o.kind.sp().is_some_and(|s| matches!(s.geom.prst.as_str(), "rect" | "roundRect"))).expect("a rectangle")
}

fn adj(o: &Object) -> f64 {
    o.kind.sp().unwrap().geom.adj.get("adj").copied().unwrap_or(0.0)
}

#[test]
fn the_builtin_theme_has_no_corner_scheme() {
    // what classic-bw draws stays what it drew: every part keeps its own radius
    assert!(Theme::classic_bw().corners.is_none());
    assert!(Theme::from_toml("id = \"x\"\n").unwrap().corners.is_none());
}

#[test]
fn a_corner_scheme_decides_region_corners_by_depth() {
    let (g, mut lay, routes) = laid("fig f\na: box \"A\"\n");
    let rect = lay.nodes[0].rect.inflate(20.0);
    lay.regions.push(rbox("outer", rect, 0));
    lay.regions.push(rbox("inner", rect, 1));
    let graded = Theme::from_toml(GRADED).unwrap();
    let b = built_with(&graded, &g, &lay, &routes);
    let (o, i) = (&b.objects[index(&b, "flow:f/region:outer")], &b.objects[index(&b, "flow:f/region:inner")]);
    assert_eq!(o.kind.sp().unwrap().geom.prst, "roundRect", "the scheme rounds a region that did not say round");
    assert!((adj(o) / adj(i) - 9.0 / 5.0).abs() < 0.01, "outer 9 pt, inner 5 pt on the same rectangle: {} {}", adj(o), adj(i));
    // the hard scheme wins over the region's own `round`
    lay.regions[0].round = true;
    let b = built_with(&Theme::from_toml(HARD).unwrap(), &g, &lay, &routes);
    assert_eq!(b.objects[index(&b, "flow:f/region:outer")].kind.sp().unwrap().geom.prst, "rect");
    // without a scheme the word still rounds it
    let b = built(&g, &lay, &routes);
    assert_eq!(b.objects[index(&b, "flow:f/region:outer")].kind.sp().unwrap().geom.prst, "roundRect");
}

#[test]
fn a_corner_scheme_tells_data_step_and_loss_apart() {
    let t = Theme::from_toml(GRADED).unwrap();
    let step = body_with(&t, "a: box \"Normalise\"");
    let s = step.kind.sp().unwrap();
    assert_eq!(s.geom.prst, "roundRect");
    // 7 pt whatever the block's size: the fraction follows the short side
    let short = step.frame.w.min(step.frame.h) as f64;
    assert!((adj(&step) / 100_000.0 * short - 7.0 * lore_flow::modules::EMU_PT as f64).abs() < short * 0.01, "{} of {short}", adj(&step));
    assert_eq!(body_with(&t, "a: box \"x\" role=input").kind.sp().unwrap().geom.prst, "rect", "data is square");
    assert_eq!(adj(&body_with(&t, "a: loss \"L\"")), 50_000.0, "a loss is a pill");
    // a block's own radius still wins, and shape=round yields to the scheme
    assert_eq!(adj(&body_with(&t, "a: box \"A\" radius=0.3")), 30_000.0);
    let hard = Theme::from_toml(HARD).unwrap();
    assert_eq!(body_with(&hard, "a: box \"A\" shape=round").kind.sp().unwrap().geom.prst, "rect");
    assert_eq!(body_with(&hard, "a: loss \"L\"").kind.sp().unwrap().geom.prst, "rect");
    assert_eq!(body_with(&Theme::classic_bw(), "a: box \"A\" shape=round").kind.sp().unwrap().geom.prst, "roundRect");
}

#[test]
fn a_style_is_layers_joined_with_plus() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("themes")).unwrap();
    let put = |name: &str, text: &str| std::fs::write(dir.path().join("themes").join(name), text).unwrap();
    put("base.toml", "id = \"base\"\n[edges.data]\ncolor = \"111111\"\nwidth = 2.2\ntail = \"triangle\"\n[colors]\nink = \"111111\"\n");
    put("pink.toml", "id = \"pink\"\npart = \"palette\"\n[colors]\nink = \"3F3F46\"\n[panel]\nline = \"D9D4E8\"\n");
    put("graded.toml", GRADED);
    // a palette that only colours a line the structure defines does not stand alone, and still stacks
    put("ink.toml", "id = \"ink\"\npart = \"palette\"\n[edges.side]\ncolor = \"3F3F46\"\n");
    put("lined.toml", "id = \"lined\"\n[edges.side]\ncolor = \"000000\"\nwidth = 1.2\ntail = \"triangle\"\n");
    put("whole.toml", "id = \"whole\"\nstack = [\"lined\", \"ink\"]\n");
    let cat = lore_flow::figure::Catalogue::load(Some(dir.path()));
    let side = cat.theme("whole").edge(EdgeKind::Side).clone();
    assert_eq!((side.color.as_str(), side.width), ("3F3F46", 1.2), "a layer that is not a theme alone is still a layer");
    let t = cat.theme("base+pink+graded");
    assert_eq!(t.id, "base+pink+graded");
    assert_eq!((t.colors.ink.as_str(), t.panel.line.as_str()), ("3F3F46", "D9D4E8"), "the palette lies over the structure");
    assert_eq!(t.edge(EdgeKind::Data).width, 2.2, "the structure's weights stay");
    assert_eq!(t.corners.unwrap().region, 9.0);
    assert_eq!(cat.theme("base+hard").corners, None, "an unknown layer gives the built-in theme");
    assert_eq!(cat.theme("base+hard").id, "classic-bw");
    // order decides
    assert_eq!(cat.theme("pink+base").colors.ink, "111111");
    // a style file that names its layers is that stack, and is what a figure picks by one id
    put("mine.toml", "id = \"mine\"\nname = \"Mine\"\nstack = [\"base\", \"pink\", \"graded\"]\n[corners]\nloss = 3\n");
    let cat = lore_flow::figure::Catalogue::load(Some(dir.path()));
    let m = cat.theme("mine");
    assert_eq!((m.id.as_str(), m.colors.ink.as_str(), m.edge(EdgeKind::Data).width), ("mine", "3F3F46", 2.2));
    assert_eq!((m.corners.unwrap().region, m.corners.unwrap().loss), (9.0, 3.0), "its own lines come last");
    assert!(m.part.is_empty());
}

#[test]
fn a_saved_block_takes_the_styles_weights_and_corners() {
    let mut a = rect_obj("thin", 0, "role:fill", "role:line");
    let mut b = rect_obj("heavy", 254_000, "role:fill", "role:line");
    let mut c = rect_obj("pill", 508_000, "role:fill", "role:line");
    for (o, w, adj) in [(&mut a, 0.75, 20_000.0), (&mut b, 3.0, 20_000.0), (&mut c, 1.0, 50_000.0)] {
        let sp = o.kind.sp_mut().unwrap();
        sp.line.as_mut().unwrap().width_pt = w;
        sp.geom.prst = "roundRect".into();
        sp.geom.adj.insert("adj".into(), adj);
    }
    let t = lore_flow::modules::ModuleTemplate { kind: "tiny".into(), style: "x".into(), w: 40.0, h: 60.0, objects: serde_json::to_value(vec![a, b, c]).unwrap(), ..Default::default() };
    let mut lib = Library::default();
    lib.add(t).unwrap();
    let draw = |theme: &Theme| {
        let g = dsl::parse_with("fig f\nn: tiny \"A\" style=x\n", &lib.kinds()).unwrap();
        let vars = Vars::default();
        let cx = lore_flow::modules::Ctx { theme, vars: &vars, math: &NoMath, lib: &lib, k: 1.0, symmetric: false, image_size: &lore_flow::modules::no_images };
        lore_flow::modules::make(&g.nodes[0], &cx).objs
    };
    let part = |objs: &[Object], name: &str| objs.iter().find(|o| o.name.ends_with(name)).unwrap().clone();
    let width = |o: &Object| o.kind.sp().unwrap().line.as_ref().unwrap().width_pt;
    // as saved without a style
    let plain = draw(&Theme::classic_bw());
    assert_eq!((width(&part(&plain, "thin")), width(&part(&plain, "heavy")), adj(&part(&plain, "thin"))), (0.75, 3.0, 20_000.0));
    // with levels and a hard scheme
    let styled = draw(&Theme::from_layers(&["[node]\nlevels = [0.6, 1.1, 1.6]\n", HARD]).unwrap());
    assert_eq!((width(&part(&styled, "thin")), width(&part(&styled, "heavy")), width(&part(&styled, "pill"))), (0.6, 1.6, 1.1));
    assert_eq!(adj(&part(&styled, "thin")), 0.0, "what the block rounded follows the scheme");
    assert_eq!(adj(&part(&styled, "pill")), 50_000.0, "a pill stays a pill");
}

#[test]
fn a_palette_names_the_tints_and_a_style_the_region_frames() {
    let soft = "[region]\nline_w = 2.2\ninner_w = 1.4\ninner_dash = \"dash\"\nsoft = true\ntitle_tint = true\n[tints.blue]\nfill = \"E6F0FB\"\nmid = \"BFD8F3\"\nline = \"6C8FD6\"\n";
    let t = Theme::from_toml(soft).unwrap();
    assert_eq!(t.tint("Blue"), Some(("E6F0FB".into(), "6C8FD6".into())));
    assert_eq!(t.tint("green"), lore_flow::theme::named("green"), "a name the palette does not list keeps its colours");
    assert_eq!(Theme::classic_bw().tint("blue"), lore_flow::theme::named("blue"));
    let (g, mut lay, routes) = laid("fig f\na: box \"A\"\n");
    let rect = lay.nodes[0].rect.inflate(20.0);
    for (id, depth) in [("outer", 0), ("inner", 1)] {
        let mut rb = rbox(id, rect, depth);
        rb.tint = "blue".into();
        lay.regions.push(rb);
    }
    let b = built_with(&t, &g, &lay, &routes);
    let sp = |name: &str| b.objects[index(&b, name)].kind.sp().unwrap().clone();
    let (o, i) = (sp("flow:f/region:outer"), sp("flow:f/region:inner"));
    // outer: pale fill, outlined a step deeper, thick; inner: the deeper fill, strong dashed outline
    assert_eq!(o.fill, lore_canvas::Paint::solid("E6F0FB"));
    assert_eq!(o.line.as_ref().unwrap().color, "BFD8F3");
    assert_eq!(i.fill, lore_canvas::Paint::solid("BFD8F3"));
    let il = i.line.as_ref().unwrap();
    assert_eq!((il.color.as_str(), il.dash.as_str()), ("6C8FD6", "dash"));
    assert!((o.line.as_ref().unwrap().width_pt / il.width_pt - 2.2 / 1.4).abs() < 0.01);
    // a region without a tint is a module's frame: the block weight, in ink
    let mut lay2 = lay.clone();
    lay2.regions[0].tint.clear();
    let b2 = built_with(&t, &g, &lay2, &routes);
    let m = b2.objects[index(&b2, "flow:f/region:outer")].kind.sp().unwrap().line.clone().unwrap();
    assert_eq!(m.color, t.colors.ink);
    assert!((m.width_pt / il.width_pt - t.node.line_w / 1.4).abs() < 0.01);
    // a block and a line that say blue take the palette's blue
    let body = body_with(&t, "a: box \"A\" fill=blue");
    assert_eq!(body.kind.sp().unwrap().fill, lore_canvas::Paint::solid("E6F0FB"));
    // the built-in theme draws a tinted region as it did
    let b = built(&g, &lay, &routes);
    let o = b.objects[index(&b, "flow:f/region:outer")].kind.sp().unwrap();
    let (pale, strong) = lore_flow::theme::named("blue").unwrap();
    assert_eq!(o.fill, lore_canvas::Paint::solid(&pale));
    assert!(o.line.as_ref().unwrap().color.eq_ignore_ascii_case(&strong));
}

#[test]
fn a_line_lies_under_a_frame_it_only_passes_through() {
    use lore_flow::build::{Layer, frame_order};
    // a panel holding two regions side by side; line 0 runs from region A to a block outside
    // both, crossing region B; line 1 stays inside A
    let frames = [Rect::new(0.0, 0.0, 300.0, 100.0), Rect::new(10.0, 10.0, 80.0, 80.0), Rect::new(120.0, 10.0, 80.0, 80.0)];
    let ends = [[(50.0, 50.0), (250.0, 50.0)], [(30.0, 50.0), (70.0, 50.0)]];
    let lines = vec![vec![(50.0, 50.0), (250.0, 50.0)], vec![(30.0, 50.0), (70.0, 50.0)]];
    let (order, forced) = frame_order(&frames, &ends, &lines);
    let at = |x: Layer| order.iter().position(|o| *o == x).unwrap();
    assert!(forced.is_empty());
    assert!(at(Layer::Frame(0)) < at(Layer::Frame(1)) && at(Layer::Frame(0)) < at(Layer::Frame(2)), "outer first");
    assert!(at(Layer::Frame(1)) < at(Layer::Line(0)), "over the frame that holds its start");
    assert!(at(Layer::Line(0)) < at(Layer::Frame(2)), "under the frame it only crosses");
    assert!(at(Layer::Frame(1)) < at(Layer::Line(1)));
    // two lines that cross each other's frames: one of them is drawn over, and said so
    let ends = [[(50.0, 50.0), (250.0, 50.0)], [(160.0, 50.0), (5.0, 50.0)]];
    let lines = vec![vec![(50.0, 50.0), (250.0, 50.0)], vec![(160.0, 50.0), (5.0, 50.0)]];
    let (order, forced) = frame_order(&frames, &ends, &lines);
    assert_eq!(order.len(), 5);
    assert_eq!(forced.len(), 1);
}

#[test]
fn soft_frames_hide_a_line_that_passes_beneath() {
    // a module frame (a region without a tint) between two blocks it does not hold
    let src = "fig f\npanel p \"P\"\n  a: box \"A\" col=0 lane=0\n  m: box \"M\" col=1 lane=0\n  b: box \"B\" col=2 lane=0\n  region grp \"\" in=p\n    m\n  end\n  a -> b : data\n";
    let g = dsl::parse(src).unwrap();
    let t = Theme::from_toml("[region]\nsoft = true\n").unwrap();
    let vars = Vars::default();
    let lib = Library::default();
    let opt = layout::LayoutOptions { frame: Rect::new(0.0, 0.0, 600.0, 300.0), ..Default::default() };
    let lay = layout::layout(&g, &t, &vars, &lib, &opt);
    let mut routes = route::route(&g, &lay);
    // the router goes round the module now; a line that cannot is drawn straight through it
    let (pa, pb) = (&lay.nodes[0].rect, &lay.nodes[2].rect);
    routes[0].points = vec![(pa.right(), pa.cy()), (pb.x, pa.cy())];
    let b = built_with(&t, &g, &lay, &routes);
    let pos = |name: &str| b.objects.iter().position(|o| o.name == name).unwrap_or_else(|| panic!("{name}"));
    let region = &b.objects[pos("flow:f/region:grp")];
    assert!(matches!(region.kind.sp().unwrap().fill, lore_canvas::Paint::Solid { .. }), "opaque");
    let crosses = routes[0].points.windows(2).any(|s| segment_hits(s[0], s[1], &lay.regions[0].rect));
    assert!(crosses, "the line passes through the module's frame");
    assert!(pos("flow:f/edge:a-b") < pos("flow:f/region:grp"), "the line is drawn before the frame it passes through");
    // the order is the rule in every style; without soft frames the frame is just not opaque
    let b = built(&g, &lay, &routes);
    let pos = |name: &str| b.objects.iter().position(|o| o.name == name).unwrap();
    assert!(pos("flow:f/edge:a-b") < pos("flow:f/region:grp"));
    assert_eq!(b.objects[pos("flow:f/region:grp")].kind.sp().unwrap().fill, lore_canvas::Paint::None);
}

#[test]
fn no_line_is_drawn_on_a_frame() {
    // two tinted rows of blocks; a line from the first block of the top row to the last of the
    // bottom row has to pass between the two frames
    let src = "fig f\npanel p \"P\"\n  a: box \"A\" col=0 lane=0\n  b: box \"B\" col=1 lane=0\n  c: box \"C\" col=2 lane=0\n  d: box \"D\" col=0 lane=1\n  e: box \"E\" col=1 lane=1\n  h: box \"H\" col=2 lane=1\n  region top \"\" tint=pink in=p\n    a b c\n  end\n  region bot \"\" tint=blue in=p\n    d e h\n  end\n  a -> b -> c : data\n  d -> e -> h : data\n  a -> h : supervision\n  c -> d : side\n";
    let (g, lay, routes) = laid(src);
    assert_eq!(g.nodes.len(), 6);
    assert_eq!(route::hugging(&lay, &routes), 0, "{:?}", routes.iter().map(|r| r.points.clone()).collect::<Vec<_>>());
}

#[test]
fn a_chart_grows_to_fill_a_panel_its_neighbour_makes_tall() {
    // panel a is three blocks high; panel b beside it holds one chart and is as high as a
    let src = "fig f\narrange a b\npanel a \"A\"\n  x: box \"X\" col=0 lane=0\n  y: box \"Y\" col=0 lane=1\n  z: box \"Z\" col=0 lane=2\n  w: box \"W\" col=0 lane=3\n  x -> y -> z -> w : data\npanel b \"B\"\n  c: chart \"C\" chart=bell\n";
    let g = dsl::parse(src).unwrap();
    let opt = layout::LayoutOptions { frame: Rect::new(0.0, 0.0, 600.0, 400.0), ..Default::default() };
    let d = Drawing { options: opt.clone(), ..Default::default() };
    let f = draw(&g, &d).unwrap();
    let plain = layout::layout(&g, &Theme::classic_bw(), &Vars::default(), &Library::default(), &opt);
    let ci = g.node_index("c").unwrap();
    let (grown, before) = (f.layout.nodes[ci].rect, plain.nodes[ci].rect);
    assert!(grown.h / f.layout.scale > before.h / plain.scale * 1.15, "{grown:?} {before:?}");
    assert!(f.layout.scale >= plain.scale * 0.99, "the figure is not drawn smaller for it");
    // the boxes keep their size: only pictures, charts and drawings grow
    let xi = g.node_index("x").unwrap();
    assert!((f.layout.nodes[xi].rect.h / f.layout.scale - plain.nodes[xi].rect.h / plain.scale).abs() < 0.5);
}

#[test]
fn a_saved_block_takes_the_size_it_is_given() {
    let lib = tiny_library();
    let plain = make_body("fig f\nn: tiny \"\" style=x\n", &lib);
    let wide = make_body("fig f\nn: tiny \"\" style=x w=80\n", &lib);
    let both = make_body("fig f\nn: tiny \"\" style=x w=80 h=20\n", &lib);
    assert!((plain.w - 40.0).abs() < 0.5 && (wide.w - 80.0).abs() < 0.5 && (wide.h - 80.0).abs() < 0.5, "{} {} {}", plain.w, wide.w, wide.h);
    assert!((both.w - 80.0).abs() < 0.5 && (both.h - 20.0).abs() < 0.5);
    // its line weights stay
    let lw = |b: &lore_flow::modules::Body| b.objs.iter().find(|o| o.name.ends_with("one")).and_then(|o| o.kind.sp()).and_then(|s| s.line.clone()).unwrap().width_pt;
    assert_eq!(lw(&plain), lw(&wide));
}

#[test]
fn a_blocks_own_wide_words_widen_the_room_it_needs() {
    // a 40 pt drawing whose part says a long sentence
    let objs = vec![text_rect("cap", 0, &["A caption much wider than the drawing itself"])];
    let t = lore_flow::modules::ModuleTemplate { kind: "capped".into(), style: "x".into(), w: 40.0, h: 20.0, objects: serde_json::to_value(&objs).unwrap(), ..Default::default() };
    let mut lib = Library::default();
    lib.add(t).unwrap();
    let b = make_body("fig f\nn: capped \"\" style=x\n", &lib);
    assert!(b.reserve_w > b.w + 60.0, "reserve {} for w {}", b.reserve_w, b.w);
}
