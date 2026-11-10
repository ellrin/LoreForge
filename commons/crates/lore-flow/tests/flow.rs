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
        assert_eq!(r.points[0], f.layout.nodes[a].port(r.from_side));
        assert_eq!(*r.points.last().unwrap(), f.layout.nodes[b].port(r.to_side));
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

#[test]
fn every_built_in_style_draws_and_differs() {
    use lore_flow::modules::generator_styles;
    for kind in ["encoder", "decoder", "nn", "brace"] {
        let styles = generator_styles(kind);
        assert!(styles.len() >= 5, "{kind} has {} styles", styles.len());
        let mut seen = BTreeSet::new();
        for style in styles {
            let g = dsl::parse(&format!("fig k\na: box \"A\"\nm: {kind} \"M\" style={style}\nb: box \"B\"\na -> m -> b\n")).unwrap();
            let f = fig(&g);
            assert!(f.routes.iter().all(|r| r.bends() == 0 && !r.forced), "{kind}/{style}");
            let o = f.objects().iter().find(|o| o.name == "flow:k/m").unwrap();
            let mut o2 = o.clone();
            o2.frame = Default::default();
            assert!(seen.insert(serde_json::to_string(&o2).unwrap()), "{kind}/{style} looks like another style");
            // connection sites are where the lines end
            let sites = lore_canvas::geom::sites(o);
            let end = f.routes[0].points[1];
            assert!((sites[1].0 / 12_700.0 - end.0).abs() < 0.01 && (sites[1].1 / 12_700.0 - end.1).abs() < 0.01, "{kind}/{style}");
        }
    }
    // the theme can choose the style and the colours of a kind
    let t = Theme::from_toml("[modules.encoder]\nstyle = \"bars\"\nfill = \"C8C1FF\"\nline = \"1D3B63\"\n[modules.unet]\nfill = \"C8C1FF\"\n").unwrap();
    let g = dsl::parse("fig k\ne: encoder \"E\"\nu: unet \"U\"\no: encoder \"O\" style=outline\ne -> u -> o\n").unwrap();
    let f = draw(&g, &Drawing { theme: t, ..Default::default() }).unwrap();
    assert_eq!(find(&f, "flow:k/e/bar0").kind.sp().unwrap().fill, lore_canvas::Paint::solid("C8C1FF"));
    assert_eq!(find(&f, "flow:k/e/bar0").kind.sp().unwrap().line.as_ref().unwrap().color, "1D3B63");
    assert_eq!(find(&f, "flow:k/u/bar0").kind.sp().unwrap().fill, lore_canvas::Paint::solid("C8C1FF"));
    assert!(find(&f, "flow:k/o/body").kind.sp().unwrap().fill.is_none(), "a node's own style wins over the theme's");
}

#[test]
fn a_selection_becomes_a_module_file() {
    // draw something, "select" it, save it as a module, use it
    let g = dsl::parse("fig s\nn: nn \"Net\" style=filled layers=[2,3,1] fill=C8C1FF\n").unwrap();
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
    assert!(all.len() >= 7 + 7 + 5 + 5);
    let mut lib2 = Library::default();
    for t in all {
        lib2.add(t).unwrap();
    }
    assert!(lib2.styles("encoder").contains(&"gradient".to_string()));
}

#[test]
fn the_contact_sheet_shows_every_kind_and_style() {
    let lib = Library::load_dir(&example_dir().join("modules")).unwrap();
    let cat = lore_flow::sheet::catalogue(&lib);
    assert_eq!(cat.len(), lore_flow::spec::MODULES.len() + 2);
    let enc = cat.iter().find(|c| c.0 == "encoder").unwrap();
    assert!(enc.1.contains(&"3d".to_string()) && enc.1.contains(&"plain".to_string()));
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
    assert!(preview::svg(&objs, w, h, 1.0).contains("linearGradient"));
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
    let front = grp.children.iter().find(|c| c.name.ends_with("b0-front")).unwrap().id;
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
