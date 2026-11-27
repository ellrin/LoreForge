//! What the one who draws says, as commands: relations between named objects, worked out here.

use lore_flow::command::{Axis, Clash, Command, Fill, Way, apply, apply_all};
use lore_flow::settle::settle;
use lore_flow::*;

fn graph(src: &str) -> Graph {
    dsl::parse_with(src, &["bracket".to_string()]).unwrap_or_else(|e| panic!("{e:?}"))
}

const FIG: &str = "fig t\npanel a \"\" none title=none\n  x: image \"In\" look=photo col=0 lane=1\n  e: box \"Encoder\" col=1 lane=1\n  n: box \"Self attention net\" col=2 lane=0\n  d: box \"Decoder\" col=2 lane=1\n  f: box \"Fusion\" col=2 lane=2\n  o: box \"Out\" col=3 lane=1\n  h: label \"Reading the input\" col=0 lane=3\n  k: bracket \"\" style=under col=0 lane=4\n  l: loss \"L\" col=3 lane=0\n  x -> e -> d -> o\n  e -> n : side\n  e -> f : side\nend\n";

fn of<'a>(g: &'a Graph, id: &str) -> &'a spec::Node {
    &g.nodes[g.node_index(id).unwrap()]
}

/// The pattern the unaided run got wrong: a loose loss is tied to the module nearest it, which
/// is not the one it belongs to. One sentence ("the loss belongs right of the output") puts it
/// right, and the structure does not take it back.
#[test]
fn a_mark_tied_to_the_wrong_neighbour_is_put_right_by_one_command() {
    let (s, _) = settle(&graph(FIG));
    assert_eq!(of(&s, "l").str("by"), Some("n"), "nearest module by the structure alone");
    let (g, said) = apply(&s, &Command::Tie { mark: "l".into(), to: "o".into(), side: Some(Way::Right) }).unwrap();
    assert_eq!((of(&g, "l").str("by"), of(&g, "l").str("side")), (Some("o"), Some("right")));
    assert_eq!(said.len(), 1);
    let (again, _) = settle(&g);
    assert_eq!((of(&again, "l").str("by"), of(&again, "l").str("side")), (Some("o"), Some("right")));
}

/// Each placement is a relation worked out from where the named objects stand.
#[test]
fn placements_are_worked_out_from_the_objects_named() {
    let g = graph(FIG);
    // beside: the free neighbouring cell
    let (b, _) = apply(&g, &Command::Beside { a: "l".into(), of: "f".into(), side: Way::Right }).unwrap();
    assert_eq!((of(&b, "l").num("col"), of(&b, "l").num("lane")), (Some(3.0), Some(2.0)));
    // beside, the neighbouring cell taken: a column of its own between the two
    let (b, _) = apply(&g, &Command::Beside { a: "l".into(), of: "e".into(), side: Way::Right }).unwrap();
    assert_eq!((of(&b, "l").num("col"), of(&b, "l").num("lane")), (Some(1.5), Some(1.0)));
    // below the lowest object: one row further
    let (b, _) = apply(&g, &Command::Beside { a: "l".into(), of: "k".into(), side: Way::Below }).unwrap();
    assert_eq!((of(&b, "l").num("col"), of(&b, "l").num("lane")), (Some(0.0), Some(5.0)));
    // in another's row, in another's column
    let (b, _) = apply(&g, &Command::Align { a: "l".into(), with: "h".into(), axis: Axis::Row }).unwrap();
    assert_eq!(of(&b, "l").num("lane"), Some(3.0));
    let (b, _) = apply(&g, &Command::Align { a: "h".into(), with: "e".into(), axis: Axis::Column }).unwrap();
    assert_eq!(of(&b, "h").num("col"), Some(1.0));
    // over the columns from one object to another
    let (b, _) = apply(&g, &Command::Over { a: "h".into(), from: "x".into(), to: "d".into() }).unwrap();
    assert_eq!((of(&b, "h").num("col"), of(&b, "h").num("span")), (Some(0.0), Some(3.0)));
    // beside the rows from one object to another, as tall as they
    let (b, _) = apply(&g, &Command::BesideRows { a: "o".into(), from: "n".into(), to: "f".into(), fill: true }).unwrap_err_or_ok();
    assert_eq!((of(&b, "o").num("lane"), of(&b, "o").num("tall"), of(&b, "o").str("stretch")), (Some(0.0), Some(3.0), Some("tall")));
    // two objects change places
    let (b, _) = apply(&g, &Command::Swap { a: "n".into(), b: "f".into() }).unwrap();
    assert_eq!((of(&b, "n").num("lane"), of(&b, "f").num("lane")), (Some(2.0), Some(0.0)));
    // stretch, and back
    let (b, _) = apply(&g, &Command::Stretch { a: "k".into(), way: Fill::Wide }).unwrap();
    assert_eq!(of(&b, "k").str("stretch"), Some("wide"));
    let (b, _) = apply(&b, &Command::Stretch { a: "k".into(), way: Fill::None }).unwrap();
    assert!(of(&b, "k").str("stretch").is_none());
}

trait Either {
    fn unwrap_err_or_ok(self) -> (Graph, Vec<String>);
}
impl Either for Result<(Graph, Vec<String>), Clash> {
    /// `o` beside three rows takes the cell of the loss beside it: the loss is tied first.
    fn unwrap_err_or_ok(self) -> (Graph, Vec<String>) {
        match self {
            Ok(r) => r,
            Err(c) => {
                assert_eq!(c.who, vec!["l".to_string()], "{}", c.why);
                let g = graph(FIG);
                let (g, _) = apply(&g, &Command::Tie { mark: "l".into(), to: "o".into(), side: None }).unwrap();
                apply(&g, &Command::BesideRows { a: "o".into(), from: "n".into(), to: "f".into(), fill: true }).unwrap()
            }
        }
    }
}

/// A command that cannot be done says why and who is in the way; an unknown name comes back
/// with the names nearest to it; the figure is left as it was.
#[test]
fn what_cannot_be_done_says_why_and_who() {
    let g = graph(FIG);
    let taken = apply(&g, &Command::Cell { a: "l".into(), col: 2.0, row: 1.0 }).unwrap_err();
    assert_eq!(taken.who, vec!["d".to_string()]);
    assert!(taken.why.contains('d') && taken.why.contains('l'), "{}", taken.why);
    let unknown = apply(&g, &Command::Tie { mark: "los".into(), to: "n".into(), side: None }).unwrap_err();
    assert_eq!(unknown.who.first().map(String::as_str), Some("l"), "{unknown:?}");
    let by_words = apply(&g, &Command::Tie { mark: "l".into(), to: "fusion".into(), side: None }).unwrap_err();
    assert!(by_words.who.contains(&"f".to_string()), "{by_words:?}");
    let main = apply(&g, &Command::Tie { mark: "d".into(), to: "n".into(), side: None }).unwrap_err();
    assert!(main.why.contains("主線"), "{}", main.why);
    let no_line = apply(&g, &Command::LineSide { from: "x".into(), to: "o".into(), leave: Some(Way::Above), arrive: None }).unwrap_err();
    assert!(no_line.who.iter().any(|w| w == "x → e"), "{no_line:?}");
    let no_word = apply(&g, &Command::Break { a: "h".into(), before: "nothing".into() }).unwrap_err();
    assert!(no_word.why.contains("Reading the input"), "{}", no_word.why);
    // several at once: the one that cannot be done is left out, the others apply
    let (after, said, not) = apply_all(&g, &[Command::Cell { a: "l".into(), col: 2.0, row: 1.0 }, Command::Break { a: "h".into(), before: "the".into() }]);
    assert_eq!((said.len(), not.len()), (1, 1));
    assert_eq!(of(&after, "h").label, "Reading\\\\the input");
    assert_eq!(of(&after, "l").num("col"), Some(3.0));
}

/// Words and lines: a break before a word (the only one), back to one line, the side a line
/// leaves and arrives by, and both handed back.
#[test]
fn words_break_where_said_and_lines_take_the_sides_said() {
    let g = graph(FIG);
    let (b, _) = apply(&g, &Command::Break { a: "n".into(), before: "attention".into() }).unwrap();
    assert_eq!(of(&b, "n").label, "Self\\\\attention net");
    let (b, _) = apply(&b, &Command::Break { a: "n".into(), before: "net".into() }).unwrap();
    assert_eq!(of(&b, "n").label, "Self attention\\\\net");
    let (b, _) = apply(&b, &Command::OneLine { a: "n".into() }).unwrap();
    assert_eq!(of(&b, "n").label, "Self attention net");
    let (b, said) = apply(&g, &Command::LineSide { from: "e".into(), to: "n".into(), leave: Some(Way::Above), arrive: Some(Way::Left) }).unwrap();
    let e = b.edges.iter().find(|e| e.from == "e" && e.to == "n").unwrap();
    assert_eq!((e.from_side.as_str(), e.to_side.as_str(), said.len()), ("top", "left", 2));
    let (b, _) = apply(&b, &Command::LineSide { from: "e".into(), to: "n".into(), leave: None, arrive: None }).unwrap();
    let e = b.edges.iter().find(|e| e.from == "e" && e.to == "n").unwrap();
    assert!(e.from_side.is_empty() && e.to_side.is_empty());
}

/// What a command placed stays through `settle`, for every command that places; and taking it
/// back hands the object to the structure again.
#[test]
fn what_is_said_stays_when_the_structure_settles_again() {
    let (s, _) = settle(&graph(FIG));
    // by the structure alone: the encoder stands beside the three rows it feeds, the heading
    // breaks, the bracket is not stretched (it lies over one column)
    assert_eq!(of(&s, "e").num("tall"), Some(3.0));
    let said: Vec<Command> = vec![
        Command::Align { a: "e".into(), with: "f".into(), axis: Axis::Row },
        Command::Over { a: "k".into(), from: "x".into(), to: "e".into() },
        Command::Stretch { a: "k".into(), way: Fill::None },
        Command::OneLine { a: "n".into() },
        Command::Beside { a: "h".into(), of: "k".into(), side: Way::Below },
    ];
    let (g, done, not) = apply_all(&s, &said);
    assert!(not.is_empty(), "{not:?} after {done:?}");
    let (again, settled) = settle(&g);
    for id in ["e", "k", "n", "h"] {
        assert_eq!(of(&again, id), of(&g, id), "{id} was changed: {settled:?}");
    }
    assert_eq!((of(&again, "e").num("lane"), of(&again, "e").num("tall")), (Some(2.0), None));
    // taken back: the structure settles the encoder beside its rows again
    let (freed, _) = apply(&again, &Command::Free { a: "e".into() }).unwrap();
    let (back, _) = settle(&freed);
    assert!(!of(&back, "e").flag("stated"));
}

/// A command is written as the model will return it, and read back the same.
#[test]
fn a_command_is_written_and_read_as_json() {
    let c: Vec<Command> = serde_json::from_str(r#"[{"do":"tie","mark":"l","to":"n","side":"above"},{"do":"beside-rows","a":"o","from":"n","to":"f"},{"do":"line-side","from":"e","to":"n","leave":"above"}]"#).unwrap();
    assert_eq!(c[0], Command::Tie { mark: "l".into(), to: "n".into(), side: Some(Way::Above) });
    assert_eq!(c[1], Command::BesideRows { a: "o".into(), from: "n".into(), to: "f".into(), fill: false });
    assert_eq!(serde_json::to_string(&c[2]).unwrap(), r#"{"do":"line-side","from":"e","to":"n","leave":"above"}"#);
}

/// The figure a command answers with is placed and drawn like any other: the tied mark takes
/// no cell and the page still packs.
#[test]
fn a_commanded_figure_packs() {
    use lore_flow::pack::{self, Air};
    use lore_flow::plate::{Format, Measure};
    let (s, _) = settle(&graph(FIG));
    let (g, _) = apply(&s, &Command::Tie { mark: "l".into(), to: "o".into(), side: Some(Way::Right) }).unwrap();
    let (theme, vars, lib) = (Theme::classic_bw(), Vars::default(), Library::default());
    let m = Measure { theme: &theme, vars: &vars, library: &lib, image_size: &modules::no_images };
    let p = pack::pack(&g, &Format::new(500.0, (2, 1)), &m, "", &Air { stated: true, ..Air::default() }).expect("packed");
    assert!(p.notes.contains(&"l".to_string()), "{:?}", p.notes);
    // it is written in free room by the output (the page ends right of the output here, so
    // the side said is kept as nearly as the room allows), over nothing
    let l = p.layout.nodes[g.node_index("l").unwrap()].art;
    let o = p.layout.nodes[g.node_index("o").unwrap()].art;
    assert!(p.crowded.is_empty(), "{:?}", p.crowded);
    assert!(p.layout.nodes.iter().enumerate().all(|(i, n)| i == g.node_index("l").unwrap() || !n.art.inflate(-0.5).overlaps(&l)), "the loss {l:?} lies over a block");
    assert!(l.cx() > o.cx() && (l.cy() - o.cy()).abs() < 2.0 * o.h, "the loss {l:?} by the output {o:?}");
}

/// A mark tied to another mark is written beside where that mark was written, on the side said.
#[test]
fn a_mark_tied_to_a_mark_is_written_beside_it() {
    use lore_flow::pack::{self, Air};
    use lore_flow::plate::{Format, Measure};
    let src = "fig t\npanel a \"\" none title=none\n  x: box \"X\" col=0 lane=0\n  y: box \"Y\" col=1 lane=0\n  z: box \"Z\" col=2 lane=0\n  w: box \"W\" col=1 lane=2\n  l: loss \"L\" col=1 lane=1\n  r: loss \"R\" col=2 lane=2\n  x -> y -> z\n  y -> w\nend\n";
    let g = graph(src);
    let (g, _, not) = apply_all(&g, &[Command::Tie { mark: "r".into(), to: "l".into(), side: Some(Way::Left) }, Command::Tie { mark: "l".into(), to: "y".into(), side: Some(Way::Below) }]);
    assert!(not.is_empty(), "{not:?}");
    let (theme, vars, lib) = (Theme::classic_bw(), Vars::default(), Library::default());
    let m = Measure { theme: &theme, vars: &vars, library: &lib, image_size: &modules::no_images };
    let p = pack::pack(&g, &Format::new(500.0, (2, 1)), &m, "", &Air { stated: true, ..Air::default() }).expect("packed");
    let at = |id: &str| p.layout.nodes[g.node_index(id).unwrap()].art;
    let (l, r, y) = (at("l"), at("r"), at("y"));
    assert!(l.y >= y.bottom() - 0.5 && l.cx() > y.x && l.cx() < y.right() + l.w, "L {l:?} under Y {y:?}");
    assert!(r.right() <= l.x + 0.5 && (r.cy() - l.cy()).abs() < l.h, "R {r:?} left of L {l:?}");
}

/// A brace and a fork are relations, not drawings. The brace lies along what it gathers, as
/// long as they are together, on the side of the block it points at, and takes no cell; the
/// fork is plain lines from a heading to each thing it is about, out of one side.
#[test]
fn a_brace_lies_along_what_it_gathers_and_a_fork_reaches_each_thing() {
    use lore_flow::pack::{self, Air};
    use lore_flow::plate::{Format, Measure};
    let src = "fig t\npanel a \"\" none title=none\n  p: box \"P\" col=0 lane=0\n  q: box \"Q\" col=0 lane=1\n  r: box \"R\" col=0 lane=2\n  b: brace \"\" col=1 lane=1\n  h: box \"About these\" col=2 lane=1\n  c: box \"Caption\" col=4 lane=0\n  u: box \"U\" col=3 lane=1\n  v: box \"V\" col=4 lane=1\n  w: box \"W\" col=5 lane=1\n  p -> q -> r\n  u -> v -> w\nend\n";
    let g = graph(src);
    let (g, said, not) = apply_all(&g, &[
        Command::Brace { a: "b".into(), from: "p".into(), to: "r".into(), toward: Some("h".into()) },
        Command::Fork { a: "c".into(), to: vec!["u".into(), "v".into(), "w".into()] },
    ]);
    assert!(not.is_empty() && said.len() == 2, "{not:?} {said:?}");
    assert_eq!(g.edges.iter().filter(|e| e.kind == EdgeKind::Gather).count(), 3);
    let (theme, vars, lib) = (Theme::classic_bw(), Vars::default(), Library::default());
    let m = Measure { theme: &theme, vars: &vars, library: &lib, image_size: &modules::no_images };
    let p = pack::pack(&g, &Format::new(500.0, (2, 1)), &m, "", &Air { stated: true, ..Air::default() }).expect("packed");
    let at = |id: &str| p.layout.nodes[g.node_index(id).unwrap()].art;
    // the brace: no cell, from the top of the first to the bottom of the last, between them
    // and the heading
    assert!(p.notes.contains(&"b".to_string()));
    let (b, top, low, h) = (at("b"), at("p"), at("r"), at("h"));
    assert!((b.y - top.y).abs() < 1.5 && (b.bottom() - low.bottom()).abs() < 1.5, "brace {b:?} along {top:?} … {low:?}");
    assert!(b.x >= top.right() && b.right() <= h.x + 0.5, "brace {b:?} between {top:?} and {h:?}");
    // the fork: three lines out of the caption's bottom into the top of each, without heads
    let rs = lines::lines(&p.graph, &p.layout);
    let fork: Vec<&route::Route> = rs.iter().filter(|r| g.edges[r.edge].kind == EdgeKind::Gather).collect();
    assert_eq!(fork.len(), 3);
    for r in &fork {
        let to = at(&g.edges[r.edge].to);
        let end = r.points[r.points.len() - 1];
        assert_eq!((r.from_side, r.to_side), (route::Side::Bottom, route::Side::Top));
        assert!((end.0 - to.cx()).abs() < 0.5 && (end.1 - to.y).abs() < 0.5, "ends at {end:?}, the top middle of {to:?}");
        assert!(lines::keeps_the_rule(r));
    }
    assert_eq!(theme.edge(EdgeKind::Gather).tail, "none");
    // thin, close to what it gathers, and its surroundings are what it needs
    assert!(b.w < 8.0 * p.layout.scale && b.x - top.right() < 6.0 * p.layout.scale, "brace {b:?} beside {top:?} (scale {})", p.layout.scale);
    assert!(p.unmet.is_empty(), "{:?}", p.unmet);
    // a brace is defined with its surroundings: a tip that points at nothing, or an open side
    // of one plain object, is refused with the reason
    let no_tip = apply(&g, &Command::Brace { a: "b".into(), from: "p".into(), to: "r".into(), toward: None }).unwrap_err();
    assert!(no_tip.why.contains("尖端"), "{}", no_tip.why);
    let short = apply(&g, &Command::Brace { a: "b".into(), from: "q".into(), to: "q".into(), toward: Some("h".into()) }).unwrap_err();
    assert!(short.why.contains("開口端"), "{}", short.why);
    // taken back: the brace is an object in its cell again
    let (freed, _) = apply(&g, &Command::Free { a: "b".into() }).unwrap();
    assert!(!pack::is_note(&freed, freed.node_index("b").unwrap()));
}
