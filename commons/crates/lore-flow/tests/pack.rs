//! Filling a page (`pack`): a frame of a fixed shape, blocks in order, as large as they go.
//! Small synthetic figures only.

use lore_flow::pack::{self, Air, Packed};
use lore_flow::plate::{Format, Measure};
use lore_flow::*;

fn graph(src: &str) -> Graph {
    dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"))
}

fn packed(src: &str, ratio: (u32, u32), arrangement: &str, air: &Air) -> (Graph, Packed) {
    let g = graph(src);
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let lib = Library::default();
    let m = Measure { theme: &theme, vars: &vars, library: &lib, image_size: &modules::no_images };
    let p = pack::pack(&g, &Format::new(500.0, ratio), &m, arrangement, air).expect("packed");
    (g, p)
}

fn pictures(n: usize, tag: &str) -> String {
    (0..n).map(|i| format!("  {tag}{i}: image \"P{i}\" look=photo\n")).collect()
}

fn two_areas(a: usize, b: usize) -> String {
    format!("fig t\npanel a \"First\"\n{}  m: box \"Model\"\nend\npanel b \"Second\"\n{}end\n", pictures(a, "x"), pictures(b, "y"))
}

/// Blocks of an area in reading order: by row (the picture's middle line), then across.
fn reading(p: &Packed, area: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..p.layout.nodes.len()).filter(|i| p.layout.nodes[*i].panel == area).collect();
    v.sort_by(|a, b| {
        let (x, y) = (&p.layout.nodes[*a], &p.layout.nodes[*b]);
        ((x.art.cy() * 10.0).round(), x.art.x).partial_cmp(&((y.art.cy() * 10.0).round(), y.art.x)).unwrap()
    });
    v
}

#[test]
fn nothing_lies_over_anything_and_everything_is_in_its_area() {
    for ratio in [(1, 1), (2, 1), (3, 2), (5, 2), (1, 2)] {
        let (_, p) = packed(&two_areas(6, 3), ratio, "", &Air::default());
        let l = &p.layout;
        for i in 0..l.nodes.len() {
            assert!(l.panels[l.nodes[i].panel].rect.inflate(0.5).contains(&l.nodes[i].art), "{ratio:?}: block {i} leaves its area");
            for j in i + 1..l.nodes.len() {
                assert!(!l.nodes[i].art.inflate(-0.1).overlaps(&l.nodes[j].art.inflate(-0.1)), "{ratio:?}: {i} over {j}");
            }
        }
        for a in 0..l.panels.len() {
            assert!(l.bounds.inflate(0.5).contains(&l.panels[a].rect), "{ratio:?}: area {a} leaves the page");
            for b in a + 1..l.panels.len() {
                assert!(!l.panels[a].rect.inflate(-0.1).overlaps(&l.panels[b].rect.inflate(-0.1)), "{ratio:?}: areas {a} and {b}");
            }
        }
    }
}

#[test]
fn the_page_has_the_shape_asked_for_and_is_full_one_way() {
    for ratio in [(1, 1), (2, 1), (5, 2)] {
        let (_, p) = packed(&two_areas(6, 3), ratio, "", &Air::default());
        let b = p.layout.bounds;
        assert!((b.w / b.h - ratio.0 as f64 / ratio.1 as f64).abs() < 1e-6);
        assert!(p.slack.0.min(p.slack.1) < 0.02, "{ratio:?}: slack {:?}", p.slack);
        // the areas together reach the page's edges
        let u = p.layout.panels.iter().map(|x| x.rect).reduce(|a, b| a.union(&b)).unwrap();
        assert!((u.w - b.w).abs() < 0.5 && (u.h - b.h).abs() < 0.5, "{u:?} {b:?}");
    }
}

#[test]
fn the_blocks_keep_the_order_they_are_written_in() {
    for ratio in [(1, 1), (3, 1), (1, 2)] {
        let (g, p) = packed(&two_areas(7, 4), ratio, "", &Air::default());
        for area in 0..2 {
            let written: Vec<usize> = (0..g.nodes.len()).filter(|i| p.layout.nodes[*i].panel == area).collect();
            assert_eq!(reading(&p, area), written, "{ratio:?} area {area}: {:?}", p.areas[area].rows);
        }
    }
}

#[test]
fn a_long_row_breaks_on_a_narrow_page_and_stays_whole_on_a_wide_one() {
    let src = format!("fig t\npanel a \"Only\"\n{}end\n", pictures(8, "x"));
    let (_, wide) = packed(&src, (8, 1), "", &Air::default());
    let (_, narrow) = packed(&src, (1, 1), "", &Air::default());
    assert_eq!(wide.areas[0].rows.len(), 1, "{:?}", wide.areas[0].rows);
    assert!(narrow.areas[0].rows.len() >= 2, "{:?}", narrow.areas[0].rows);
    // breaking is what keeps the text large: eight in one row on the square page would be drawn
    // no larger than on the wide one
    assert!(narrow.text_pt > 1.5 * wide.text_pt, "text {:.1} pt, in one row {:.1} pt", narrow.text_pt, wide.text_pt);
}

#[test]
fn the_air_between_blocks_is_what_was_asked() {
    for gap in [0.5, 1.5, 3.0] {
        let air = Air { gap, ..Air::default() };
        let (_, p) = packed(&two_areas(5, 5), (2, 1), "", &air);
        let fs = Theme::classic_bw().font.size;
        for area in 0..2 {
            let order = reading(&p, area);
            for w in order.windows(2) {
                let (a, b) = (&p.layout.nodes[w[0]], &p.layout.nodes[w[1]]);
                if (a.art.cy() - b.art.cy()).abs() < 0.5 {
                    let room = b.art.x.min(b.reserve.x) - a.art.right().max(a.reserve.right());
                    assert!(room >= gap * fs - 0.5, "gap {gap}: {room:.1} between {} and {}", w[0], w[1]);
                }
            }
        }
    }
}

#[test]
fn a_text_limit_gives_the_room_to_the_pictures() {
    let src = two_areas(6, 3);
    let (_, free) = packed(&src, (2, 1), "", &Air::default());
    let small = (free.text_pt * 0.7 * 10.0).round() / 10.0;
    let (_, capped) = packed(&src, (2, 1), "", &Air { text_max_pt: small, ..Air::default() });
    assert!((capped.text_pt - small).abs() < 0.05, "{} vs {small}", capped.text_pt);
    assert!(capped.cover > free.cover, "cover {} vs {}", capped.cover, free.cover);
    assert!(capped.areas.iter().all(|a| a.grow > 1.0), "{:?}", capped.areas);
    assert!(capped.slack.0.min(capped.slack.1) < 0.02);
}

#[test]
fn an_area_with_few_pictures_gets_them_larger() {
    let (_, p) = packed(&two_areas(8, 2), (3, 1), "a b", &Air { text_max_pt: 5.0, ..Air::default() });
    assert!(p.areas[1].grow >= p.areas[0].grow, "{:?}", p.areas);
}

#[test]
fn areas_of_one_row_share_its_height_and_split_the_width() {
    let src = format!("{}panel c \"Third\"\n{}end\n", two_areas(5, 2), pictures(3, "z"));
    let (_, p) = packed(&src, (2, 1), "a / b c", &Air::default());
    let r: Vec<_> = p.layout.panels.iter().map(|x| x.rect).collect();
    assert!((r[0].w - p.layout.bounds.w).abs() < 0.5);
    assert!((r[1].y - r[2].y).abs() < 0.01 && (r[1].h - r[2].h).abs() < 0.01);
    assert!(r[1].y > r[0].bottom() && r[2].x > r[1].right());
    assert!((r[2].right() - p.layout.bounds.w).abs() < 0.5);
}

#[test]
fn a_region_is_one_item_and_frames_its_blocks() {
    let src = "fig t\npanel a \"Area\"\n  i: image \"In\" look=photo\n  region s1 \"Step one\"\n    p: image \"P\" look=photo\n    q: box \"Q\"\n  end\n  region s2 \"Step two, with a long name\"\n    r: image \"R\" look=photo\n  end\n  o: box \"Out\"\nend\n";
    for ratio in [(4, 1), (1, 1)] {
        let (g, p) = packed(src, ratio, "", &Air::default());
        let l = &p.layout;
        assert_eq!(l.regions.len(), 2);
        for rb in &l.regions {
            let reg = g.regions.iter().find(|r| r.id == rb.id).unwrap();
            for (i, n) in g.nodes.iter().enumerate() {
                let inside = rb.rect.inflate(0.5).contains(&l.nodes[i].art);
                assert_eq!(inside, reg.members.contains(&n.id), "{ratio:?}: {} and frame {}", n.id, rb.id);
            }
            let t = rb.title_rect.expect("title");
            assert!(rb.rect.inflate(0.5).contains(&t), "{ratio:?}: the title of {} leaves its frame", rb.id);
            assert!(l.panels[0].rect.contains(&rb.rect));
        }
        assert!(!l.regions[0].rect.overlaps(&l.regions[1].rect));
    }
}

#[test]
fn no_blocks_no_page() {
    let g = Graph::default();
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let lib = Library::default();
    let m = Measure { theme: &theme, vars: &vars, library: &lib, image_size: &modules::no_images };
    assert!(pack::pack(&g, &Format::new(500.0, (2, 1)), &m, "", &Air::default()).is_none());
}

// ---- alignment ----

/// A wide page with one area: three pictures of different heights and a small operator.
const ROW: &str = "fig t\npanel a \"Row\"\n  x: image \"Input picture\" look=photo\n  v: vector-bar \"v\" n=4 dir=h\n  m: box \"Model\"\n  o: op \".\"\n  x -> v -> m -> o\nend\n";

#[test]
fn the_pictures_of_a_row_stand_on_one_line_and_their_words_on_another() {
    let (g, p) = packed(ROW, (4, 1), "", &Air::default());
    let l = &p.layout;
    let at = |id: &str| g.node_index(id).unwrap();
    // one middle line
    let line = l.nodes[at("x")].art.cy();
    for id in ["v", "m", "o"] {
        assert!((l.nodes[at(id)].art.cy() - line).abs() < 0.6, "{id} is off the row's line");
    }
    // the low picture's words start as far under it as the tall picture reaches lower
    let drop = p.graph.nodes[at("v")].num("label_drop").expect("the low picture's words are lowered");
    assert!((drop - (l.nodes[at("x")].art.bottom() - l.nodes[at("v")].art.bottom())).abs() < 0.6, "{drop}");
    assert!(p.graph.nodes[at("x")].num("label_drop").is_none(), "the tallest picture's words stay");
}

#[test]
fn the_blocks_of_two_rows_stand_under_each_other() {
    let src = format!("fig t\npanel a \"Grid\"\n{}end\n", pictures(6, "x"));
    let (_, p) = packed(&src, (1, 1), "", &Air::default());
    assert!(p.areas[0].rows.len() >= 2, "{:?}", p.areas[0].rows);
    let l = &p.layout;
    let per = p.areas[0].rows[0].len();
    for (i, n) in l.nodes.iter().enumerate().skip(per) {
        assert!((n.art.cx() - l.nodes[i % per].art.cx()).abs() < 0.6, "block {i} is not under block {}", i % per);
    }
}

#[test]
fn a_note_takes_no_room_and_does_not_move_the_objects() {
    let with = format!("{}", ROW.replace("end\n", "  w: label \"weights fed in from the side\" col=3 lane=1\n  w -> o : side\nend\n"));
    let (g, p) = packed(&with, (4, 1), "", &Air::default());
    let (g0, p0) = packed(ROW, (4, 1), "", &Air::default());
    assert_eq!(p.notes, ["w"]);
    assert!(p.crowded.is_empty(), "the note found room");
    assert!(p0.notes.is_empty());
    assert_eq!(p.areas[0].rows, p0.areas[0].rows, "the objects are in the same rows");
    // the objects are as far down their page with the note as without it: in the middle
    let mid = |g: &Graph, p: &Packed| {
        let ys: Vec<f64> = ["x", "v", "m", "o"].iter().map(|id| p.layout.nodes[g.node_index(id).unwrap()].art.cy()).collect();
        ys.iter().sum::<f64>() / ys.len() as f64 / p.layout.bounds.h
    };
    assert!((mid(&g, &p) - mid(&g0, &p0)).abs() < 0.01, "{} vs {}", mid(&g, &p), mid(&g0, &p0));
    // the note is drawn, inside its area and over nothing
    let l = &p.layout;
    let w = g.node_index("w").unwrap();
    assert!(l.panels[0].rect.contains(&l.nodes[w].art));
    for (i, n) in l.nodes.iter().enumerate().filter(|(i, _)| *i != w) {
        assert!(!n.rect.union(&n.reserve).inflate(-0.1).overlaps(&l.nodes[w].art), "the note lies over block {i}");
    }
    // words on a line that carries the flow are an object, and so are words on no line
    let (_, q) = packed("fig t\npanel a \"Row\"\n  x: image \"P\" look=photo\n  r: label \"result\"\n  k: label \"legend row\"\n  x -> r\nend\n", (4, 1), "", &Air::default());
    assert!(q.notes.is_empty());
}

// ---- the grid the text states: rows are the paths of the flow ----

/// Two parallel paths through two steps, as the text of a figure states them.
const PATHS: &str = "fig t\npanel a \"Two paths\"\n  x: image \"Input\" look=photo col=0 lane=0\n  y: image \"Other input\" look=photo col=0 lane=1\n  region s \"Step one\" title=above\n    e: box \"Encoder\" col=1 lane=0\n    f: box \"Encoder\" col=1 lane=1\n  end\n  u: vector-bar \"u\" n=4 dir=h col=2 lane=0\n  v: vector-bar \"v\" n=4 dir=h col=2 lane=1\n  x -> e -> u\n  y -> f -> v\n  e -> f : shared\nend\n";

#[test]
fn a_stated_grid_keeps_each_path_in_a_row_and_its_steps_in_columns() {
    let (g, p) = packed(PATHS, (2, 1), "", &Air::default());
    assert!(p.areas[0].grid);
    assert_eq!(p.areas[0].rows, [["x", "e", "u"], ["y", "f", "v"]]);
    let l = &p.layout;
    let at = |id: &str| &l.nodes[g.node_index(id).unwrap()];
    // every flow line joins two blocks on one line: it can be drawn straight
    for (a, b) in [("x", "e"), ("e", "u"), ("y", "f"), ("f", "v")] {
        assert!((at(a).port_y - at(b).port_y).abs() < 0.01, "{a} -> {b} is not level");
    }
    // the steps of the two paths stand under each other
    for (a, b) in [("x", "y"), ("e", "f"), ("u", "v")] {
        assert!((at(a).art.cx() - at(b).art.cx()).abs() < 0.01, "{a} is not over {b}");
        assert_eq!(at(a).layer, at(b).layer);
    }
    assert_eq!((at("x").lane, at("y").lane), (0, 1));
    // the region frames its two blocks and nothing else
    let f = l.regions.iter().find(|r| r.id == "s").unwrap().rect;
    assert!(f.contains(&at("e").art) && f.contains(&at("f").art));
    for id in ["x", "y", "u", "v"] {
        assert!(!f.overlaps(&at(id).art), "{id} lies in the step's frame");
    }
    assert!(l.panels[0].rect.inflate(0.5).contains(&f));
}

#[test]
fn a_grid_is_taken_only_where_it_makes_the_lines_simpler_and_the_text_allows() {
    // six pictures on a diagonal and no line: the grid helps nothing, the rows are broken freely
    let src: String = format!("fig t\npanel a \"Stairs\"\n{}end\n", (0..6).map(|i| format!("  x{i}: image \"P{i}\" look=photo col={i} lane={i}\n")).collect::<String>());
    let (_, stairs) = packed(&src, (2, 1), "", &Air { paths: 0.01, ..Air::default() });
    assert!(!stairs.areas[0].grid);
    // two paths: freely broken rows bend their lines, the grid draws them straight
    let (_, p) = packed(PATHS, (2, 1), "", &Air::default());
    let (_, free) = packed(PATHS, (2, 1), "", &Air { paths: 0.0, ..Air::default() });
    assert!(p.areas[0].grid && !free.areas[0].grid);
    assert!(p.text_pt >= Air::default().paths * free.text_pt, "{} vs {}", p.text_pt, free.text_pt);
    // asked to cost nothing, a grid that makes the text smaller is left
    let (_, strict) = packed(PATHS, (2, 1), "", &Air { paths: 1.0, ..Air::default() });
    assert!(p.text_pt < free.text_pt && !strict.areas[0].grid);
    assert!((strict.text_pt - free.text_pt).abs() < 0.01);
}

#[test]
fn the_air_is_wider_where_a_line_runs_and_wider_again_for_its_words() {
    let fig = |edges: &str| format!("fig t\npanel a \"Row\"\n  x: image \"A\" look=photo col=0 lane=0\n  y: image \"B\" look=photo col=1 lane=0\n  z: image \"C\" look=photo col=2 lane=0\n  u: image \"D\" look=photo col=0 lane=1\n{edges}end\n");
    // one line x -> y, none between y and z; the page is wide so nothing is stretched much
    let gaps = |edges: &str| {
        let (g, p) = packed(&fig(edges), (3, 2), "", &Air { paths: 0.01, text_max_pt: 6.0, ..Air::default() });
        assert!(p.areas[0].grid);
        let at = |id: &str| p.layout.nodes[g.node_index(id).unwrap()].art;
        let fs = Theme::classic_bw().font.size;
        ((at("y").x - at("x").right()) / fs, (at("z").x - at("y").right()) / fs)
    };
    let (with, without) = gaps("  x -> y\n  x -> u\n");
    assert!(with > without + 0.5, "{with} vs {without}");
    let (worded, _) = gaps("  x -> y : data \"a long name on the line\"\n  x -> u\n");
    assert!(worded > with + 1.0, "{worded} vs {with}");
}

// ---- lines: straight, one bend, or a two-bend step; nothing else ----

fn route(points: &[(f64, f64)]) -> route::Route {
    route::Route { edge: 0, points: points.to_vec(), from_side: route::Side::Right, to_side: route::Side::Left, from_site: 0, to_site: 0, forced: false, warnings: Vec::new(), curve: false, branch: false }
}

#[test]
fn the_rule_for_a_line_is_two_bends_at_most_and_never_back() {
    use lore_flow::lines::keeps_the_rule;
    assert!(keeps_the_rule(&route(&[(0.0, 0.0), (50.0, 0.0)])), "straight");
    assert!(keeps_the_rule(&route(&[(0.0, 0.0), (50.0, 0.0), (50.0, 30.0)])), "one bend");
    assert!(keeps_the_rule(&route(&[(0.0, 0.0), (20.0, 0.0), (20.0, 30.0), (50.0, 30.0)])), "a step");
    // right, down, back left, down: the line that is not drawn
    assert!(!keeps_the_rule(&route(&[(0.0, 0.0), (50.0, 0.0), (50.0, 20.0), (10.0, 20.0), (10.0, 40.0)])));
    // two bends that come back (a U)
    assert!(!keeps_the_rule(&route(&[(0.0, 0.0), (0.0, 30.0), (50.0, 30.0), (50.0, 0.0)])));
    // three bends, even going on
    assert!(!keeps_the_rule(&route(&[(0.0, 0.0), (20.0, 0.0), (20.0, 30.0), (40.0, 30.0), (40.0, 60.0)])));
}

#[test]
fn every_line_drawn_keeps_the_rule_and_flow_lines_on_a_grid_are_straight() {
    let (_, p) = packed(PATHS, (2, 1), "", &Air::default());
    let rs = lines::lines(&p.graph, &p.layout);
    assert_eq!(rs.len(), p.graph.edges.len());
    for r in &rs {
        assert!(lines::keeps_the_rule(r) && !r.forced, "{:?}", r.points);
        let e = &p.graph.edges[r.edge];
        if e.kind == EdgeKind::Data {
            assert_eq!(r.bends(), 0, "{} -> {}", e.from, e.to);
        }
    }
    // the two encoders stand under each other: their line is straight down
    let shared = rs.iter().find(|r| p.graph.edges[r.edge].kind != EdgeKind::Data).unwrap();
    assert_eq!(shared.bends(), 0);
    // blocks not on a grid, anywhere on the page: still only the three shapes
    for ratio in [(1, 1), (3, 1), (1, 2)] {
        let src = format!("fig t\npanel a \"Any\"\n{}  x0 -> x3\n  x1 -> x4\n  x5 -> x0\n  x2 -> x4 : side\n  x4 -> x1 : feedback\nend\n", pictures(6, "x"));
        let (_, q) = packed(&src, ratio, "", &Air::default());
        for r in lines::lines(&q.graph, &q.layout) {
            assert!(lines::keeps_the_rule(&r), "{ratio:?}: {:?}", r.points);
        }
    }
}

#[test]
fn a_line_along_a_row_passes_beneath_the_block_between_and_stays_straight() {
    let src = "fig t\npanel a \"Row\"\n  x: image \"A\" look=photo col=0 lane=0\n  y: image \"B\" look=photo col=1 lane=0\n  z: image \"C\" look=photo col=2 lane=0\n  x -> z\nend\n";
    let (_, p) = packed(src, (4, 1), "", &Air::default());
    let rs = lines::lines(&p.graph, &p.layout);
    assert_eq!(rs[0].bends(), 0);
    assert_eq!(lines::beneath(&p.graph, &p.layout, &rs[0]), 1, "it passes the block between, beneath it");
}

#[test]
fn a_block_a_line_passes_beneath_gets_a_ground_and_no_other_block_does() {
    let src = "fig t\npanel a \"Row\"\n  x: image \"A\" look=photo col=0 lane=0\n  y: image \"B\" look=photo col=1 lane=0\n  z: image \"C\" look=photo col=2 lane=0\n  x -> z\nend\n";
    let (_, p) = packed(src, (4, 1), "", &Air::default());
    let rs = lines::lines(&p.graph, &p.layout);
    let (theme, vars, lib) = (Theme::classic_bw(), Vars::default(), Library::default());
    let built = build::build_with(&p.graph, &theme, &vars, &lib, &NoMath, &p.layout, &rs, 2, &modules::no_images);
    let grounded = |id: &str| {
        let o = built.objects.iter().find(|o| o.name == build::node_name("t", id)).unwrap();
        match &o.kind {
            lore_canvas::Kind::Group(g) => g.children.iter().any(|c| c.name.ends_with("ground")),
            _ => false,
        }
    };
    assert!(grounded("y"), "the block between has a ground under its picture");
    assert!(!grounded("x") && !grounded("z"));
    // the line lies before the block in the drawing order: beneath it
    let pos = |name: &str| built.objects.iter().position(|o| o.name.contains(name)).unwrap();
    assert!(pos("edge:x-z") < pos(&build::node_name("t", "y")));
}

#[test]
fn two_blocks_that_face_each_other_are_joined_straight_even_when_their_middles_differ() {
    let (g, mut p) = packed(PATHS, (2, 1), "", &Air::default());
    // the lower encoder a little to the right of the upper one: their widths still overlap
    let f = g.node_index("f").unwrap();
    let n = &mut p.layout.nodes[f];
    n.rect.x += 6.0;
    n.art.x += 6.0;
    n.reserve.x += 6.0;
    let rs = lines::lines(&p.graph, &p.layout);
    let shared = rs.iter().find(|r| p.graph.edges[r.edge].from == "e" && p.graph.edges[r.edge].to == "f").unwrap();
    assert_eq!(shared.bends(), 0, "{:?}", shared.points);
    assert!((shared.points[0].0 - shared.points[1].0).abs() < 1e-6, "one upright line");
    let x = shared.points[0].0;
    for id in ["e", "f"] {
        let a = p.layout.nodes[g.node_index(id).unwrap()].art;
        assert!(x > a.x && x < a.right(), "the line meets {id}'s picture");
    }
}

#[test]
fn the_shared_folders_font_choice_sets_a_figures_words() {
    use lore_flow::figure::Catalogue;
    let root = std::env::temp_dir().join(format!("lore-flow-font-{}", std::process::id()));
    let flow = root.join("shared").join("flow");
    std::fs::create_dir_all(&flow).unwrap();
    // no choice: the theme's own font
    let own = Catalogue::load(Some(&flow)).theme("").font.clone();
    assert!(Catalogue::load(Some(&flow)).figure_font.is_empty());
    std::fs::write(root.join("shared").join("fonts.toml"), "[figure]\nlatin = \"Example Serif\"\n").unwrap();
    let cat = Catalogue::load(Some(&flow));
    let f = cat.theme("").font.clone();
    assert_eq!((f.family.as_str(), f.title.as_str(), f.caption.as_str()), ("Example Serif", "Example Serif", "Example Serif"));
    assert_eq!(f.math, own.math, "equations keep the math font");
    assert_eq!(f.size, own.size);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn quoted_words_in_a_list_keep_their_line_break_and_their_latex() {
    let g = graph("fig t\na: box \"A\" texts=[\"One:\\\\Two\",\"$\\tilde q$\"]\n");
    let v = g.nodes[0].list("texts");
    assert_eq!(v, ["One:\\\\Two", "$\\tilde q$"]);
    assert_eq!(text::lines(&v[0]), ["One:", "Two"]);
}

/// What the one who draws states over the choices made here: the grid kept (`Air.stated`), a
/// block over several columns (`span=`), a block as wide as its columns (`stretch`), a mark
/// tied to a block (`by=`, `side=`).
#[test]
fn stated_placement_spans_stretches_and_ties_marks() {
    let src = "fig t\npanel a \"\" none title=none\n  a: box \"A\" col=0 lane=0\n  b: box \"B\" col=1 lane=0\n  c: box \"C\" col=2 lane=0\n  d: box \"D\" col=3 lane=0\n  h: box \"A heading much longer than any one of the blocks\" col=0 span=4 lane=1\n  k: box \"\" col=0 span=4 lane=2 stretch=true\n  m: box \"m\" by=b side=below\n  a -> b -> c -> d\nend\n";
    let (g, p) = packed(src, (1, 1), "", &Air { stated: true, ..Air::default() });
    assert!(p.areas[0].grid, "the stated grid is kept");
    let at = |id: &str| p.layout.nodes[g.node_index(id).unwrap()].art;
    // the heading lies over the four columns and widens none of them past its own share
    assert!((at("h").cx() - (at("a").x + at("d").right()) / 2.0).abs() < 0.5);
    assert!(at("b").cx() - at("a").cx() < at("h").w / 2.0);
    // the stretched block is as wide as the columns it lies over
    assert!((at("k").x - at("a").x.min(at("h").x)).abs() < 1.0 && (at("k").right() - at("d").right().max(at("h").right())).abs() < 1.0, "{:?} {:?} {:?}", at("k"), at("a"), at("d"));
    // the mark takes no cell and is written under its block
    assert!(p.notes.contains(&"m".to_string()));
    assert_eq!(p.areas[0].rows[0], ["a", "b", "c", "d"]);
    assert!(at("m").y >= at("b").bottom() - 0.01 && (at("m").cx() - at("b").cx()).abs() < at("b").w);
    // without the statement the same text may still be broken freely; with it, never
    let (_, free) = packed(src, (2, 1), "", &Air { paths: 0.0, ..Air::default() });
    assert!(!free.areas[0].grid);
}

/// A block over several rows (`tall=`) stands in their middle and makes none of them taller
/// alone; with `stretch=tall` it is as tall as those rows together.
#[test]
fn a_tall_block_stands_beside_its_rows_and_can_take_their_height() {
    let src = |extra: &str| format!("fig t\npanel a \"\" none title=none\n  a: box \"A\" col=0 lane=0\n  b: box \"B\" col=0 lane=1\n  c: box \"C\" col=0 lane=2\n  t: box \"Tall\" vertical=true col=1 lane=0 tall=3{extra}\n  a -> t\n  b -> t\n  c -> t\nend\n");
    let (g, p) = packed(&src(""), (1, 1), "", &Air { stated: true, ..Air::default() });
    assert!(p.areas[0].grid);
    let at = |p: &Packed, id: &str| p.layout.nodes[g.node_index(id).unwrap()].art;
    assert!((at(&p, "t").cy() - at(&p, "b").cy()).abs() < 0.5, "in the middle of its three rows");
    let (_, q) = packed(&src(" stretch=tall"), (1, 1), "", &Air { stated: true, ..Air::default() });
    assert!(at(&q, "t").y <= at(&q, "a").y + 0.5 && at(&q, "t").bottom() >= at(&q, "c").bottom() - 0.5, "{:?}", at(&q, "t"));
    // so every row's line into it is straight
    let routes = lore_flow::lines::lines(&q.graph, &q.layout);
    assert!(routes.iter().all(|r| r.bends() == 0));
}

/// Two lines that cross are told apart: the side line lies behind the flow line (drawn first
/// and lighter); lines that do not cross, or that share a block, are left as they are.
#[test]
fn of_two_crossing_lines_the_side_line_is_behind() {
    let src = "fig t\npanel a \"\" none title=none\n  a: box \"A\" col=0 lane=1\n  b: box \"B\" col=2 lane=1\n  c: box \"C\" col=1 lane=0\n  d: box \"D\" col=1 lane=2\n  e: box \"E\" col=3 lane=1\n  c -> d : side\n  a -> b\n  b -> e\nend\n";
    let (_, p) = packed(src, (1, 1), "", &Air { stated: true, ..Air::default() });
    let routes = lore_flow::lines::lines(&p.graph, &p.layout);
    let behind = lore_flow::build::lines_behind(&p.graph, &routes);
    let of = |from: &str| routes.iter().position(|r| p.graph.edges[r.edge].from == from).unwrap();
    assert!(behind[of("c")], "the side line crosses the flow line and lies behind it");
    assert!(!behind[of("a")] && !behind[of("b")]);
}

/// The ground under a block passed beneath has the colour the block stands on: inside a tinted
/// frame the frame's fill, never a white patch.
#[test]
fn the_ground_of_a_passed_block_takes_its_frames_colour() {
    let src = "fig t\npanel a \"Row\"\n  x: image \"A\" look=photo col=0 lane=0\n  region r \"\" tint=grey\n    y: image \"B\" look=photo col=1 lane=0\n  end\n  z: image \"C\" look=photo col=2 lane=0\n  x -> z\nend\n";
    let (_, p) = packed(src, (4, 1), "", &Air { stated: true, ..Air::default() });
    let rs = lines::lines(&p.graph, &p.layout);
    let (theme, vars, lib) = (Theme::classic_bw(), Vars::default(), Library::default());
    let built = build::build_with(&p.graph, &theme, &vars, &lib, &NoMath, &p.layout, &rs, 2, &modules::no_images);
    let fill = |o: &lore_canvas::Object| match &o.kind {
        lore_canvas::Kind::Shape(s) => match &s.fill {
            lore_canvas::Paint::Solid { color, .. } => Some(color.clone()),
            _ => None,
        },
        _ => None,
    };
    let frame = built.objects.iter().find(|o| o.name.contains("region:r")).and_then(fill).expect("the frame is filled");
    let y = built.objects.iter().find(|o| o.name == build::node_name("t", "y")).unwrap();
    let lore_canvas::Kind::Group(g) = &y.kind else { panic!("a group") };
    let ground = g.children.iter().find(|c| c.name.ends_with("ground")).and_then(fill).expect("a ground");
    assert_eq!(ground, frame);
    assert_ne!(ground.trim_start_matches('#').to_uppercase(), theme.colors.paper.trim_start_matches('#').to_uppercase());
}
