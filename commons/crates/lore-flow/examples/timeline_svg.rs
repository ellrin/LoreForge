//! Draw one timeline to an SVG file, to look at a style.
//! `cargo run -p lore-flow --example timeline_svg -- <style id> <out.svg> [flow folder] [points]`
//! The content is synthetic. `flow folder` holds `timelines/*.toml` (the author's styles).

use lore_flow::figure::{Catalogue, FlowSpec, preview_svg, render};
use lore_flow::timeline::{Point, TimelineSpec};
use lore_flow::{NoMath, Rect};

fn main() {
    let mut a = std::env::args().skip(1);
    let style = a.next().expect("a style id");
    let out = a.next().expect("an output file");
    let dir = a.next().filter(|d| !d.is_empty()).map(std::path::PathBuf::from);
    let n: usize = a.next().and_then(|x| x.parse().ok()).unwrap_or(5);
    let cat = Catalogue::load(dir.as_deref());
    for e in &cat.timeline_errors {
        eprintln!("not read: {e}");
    }
    for s in &cat.timelines {
        eprintln!("{} {}{}{}", s.id, s.name, if s.private { " (private)" } else { "" }, if s.cannot.is_empty() { String::new() } else { format!(" cannot: {}", s.cannot.join("; ")) });
    }
    let rows = [
        ("2021", "Pilot", "Two sites share one protocol.", "clipboard"),
        ("2022", "First cohort", "Recruitment opens and the first 200 cases are labelled.", "people"),
        ("2023", "Model v1", "A baseline is trained and checked on a held out site.", "gears"),
        ("2024", "External test", "Three more sites join. The gap to the internal test is measured.", "globe"),
        ("2025", "Release", "Code and weights are made public.", "flag"),
        ("2026", "Follow up", "One year outcomes are collected.", "calendar"),
        ("2027", "Second cohort", "The protocol is revised.", "people"),
        ("2028", "Model v2", "Trained on both cohorts.", "gears"),
        ("2029", "Audit", "An outside group repeats the test.", "shield"),
        ("2030", "Report", "The five year report is written.", "book"),
    ];
    // `TITLES_ONLY=1`: a time and a title per point, as a deck's chain of steps gives them
    let bare = std::env::var_os("TITLES_ONLY").is_some();
    let points = rows.iter().take(n).map(|(t, h, b, i)| Point { time: (*t).into(), title: (*h).into(), body: if bare { String::new() } else { (*b).into() }, icon: if bare { String::new() } else { (*i).into() }, ..Default::default() }).collect();
    let spec = FlowSpec { timeline: Some(TimelineSpec { title: if bare { String::new() } else { "Study timeline".into() }, style, points, ..Default::default() }), ..Default::default() };
    let frame = Rect::new(0.0, 0.0, 672.0, 357.0);
    match render(&spec, &cat, None, &NoMath, frame, 2) {
        Ok((fig, _)) => {
            std::fs::write(&out, preview_svg(&fig, 1344.0)).expect("write");
            eprintln!("{} objects → {out}", fig.objects().len());
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
