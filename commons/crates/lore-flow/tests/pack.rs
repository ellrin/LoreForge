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
fn a_grid_that_costs_too_much_text_is_left_for_free_rows() {
    // six pictures on a diagonal: the grid is six columns by six rows, mostly empty
    let src: String = format!("fig t\npanel a \"Stairs\"\n{}end\n", (0..6).map(|i| format!("  x{i}: image \"P{i}\" look=photo col={i} lane={i}\n")).collect::<String>());
    let (_, free) = packed(&src, (2, 1), "", &Air { paths: 0.0, ..Air::default() });
    let (_, grid) = packed(&src, (2, 1), "", &Air { paths: 0.01, ..Air::default() });
    let (_, chosen) = packed(&src, (2, 1), "", &Air::default());
    assert!(!free.areas[0].grid && grid.areas[0].grid);
    assert!(grid.text_pt < Air::default().paths * free.text_pt, "{} vs {}", grid.text_pt, free.text_pt);
    assert!(!chosen.areas[0].grid, "the default leaves a grid this costly");
    assert!((chosen.text_pt - free.text_pt).abs() < 0.01);
    // a grid that costs little is kept
    let (_, p) = packed(PATHS, (2, 1), "", &Air::default());
    let (_, q) = packed(PATHS, (2, 1), "", &Air { paths: 0.0, ..Air::default() });
    assert!(p.areas[0].grid && p.text_pt >= Air::default().paths * q.text_pt);
}
