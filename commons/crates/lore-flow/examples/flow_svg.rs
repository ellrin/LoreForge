//! Draw a flow spec (a JSON file) to an SVG file, to look at layout and routing.
//! `cargo run -p lore-flow --example flow_svg -- <spec.json> <out.svg> [flow dir]`
//! With a flow folder (one holding `modules/<kind>/<style>.json`, `themes/`, `templates/`), its
//! saved modules are drawn too: the way to look at a module while writing it.

use lore_flow::figure::{Catalogue, FlowSpec, preview_svg, render};
use lore_flow::NoMath;

fn main() {
    let mut a = std::env::args().skip(1);
    let (spec, out) = (a.next().expect("a spec file"), a.next().expect("an output file"));
    let cat = match a.next() {
        Some(dir) => Catalogue::load(Some(std::path::Path::new(&dir))),
        None => Catalogue::builtin(),
    };
    let spec = FlowSpec::from_value(serde_json::from_str(&std::fs::read_to_string(&spec).expect("read")).expect("json")).expect("spec");
    match render(&spec, &cat, None, &NoMath, spec.frame_rect(), 2) {
        Ok((fig, _)) => {
            for w in &fig.built.warnings {
                eprintln!("warning: {w}");
            }
            std::fs::write(&out, preview_svg(&fig, 1344.0)).expect("write");
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
