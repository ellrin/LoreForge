//! Count the stretches of lines drawn on a frame, per figure spec.
//! `cargo run -p lore-flow --example flow_hug -- <flow dir> <spec.json>...`
use lore_flow::NoMath;
use lore_flow::figure::{Catalogue, FlowSpec, render};

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("a flow dir");
    let cat = Catalogue::load(Some(std::path::Path::new(&dir)));
    let mut total = 0;
    for f in a {
        let spec = FlowSpec::from_value(serde_json::from_str(&std::fs::read_to_string(&f).expect("read")).expect("json")).expect("spec");
        let (fig, _) = render(&spec, &cat, None, &NoMath, spec.frame_rect(), 2).expect("render");
        let n = lore_flow::route::hugging(&fig.layout, &fig.routes);
        let bends: usize = fig.routes.iter().map(|r| r.bends()).sum();
        total += n;
        println!("{f}: on a frame {n}, bends {bends}");
    }
    println!("total on a frame {total}");
}
