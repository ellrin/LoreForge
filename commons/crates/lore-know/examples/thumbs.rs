//! Draws every entity of the flow tool's catalogue (and two synthetic whole figures) as SVG
//! files, to look at what the hub would show: `cargo run -p lore-know --example thumbs -- <out dir> [flow dir]`.
//! Writes `<local id>.svg` and `index.json` (`[{id, title, level, line, file}]`).

use lore_flow::figure::Catalogue;
use lore_know::formats::figure::{Figure, from_catalogue, spec_item, thumbnail};
use lore_know::{Format, Source};
use serde_json::json;

const FIG_A: &str = "fig a\narrange a / b\npanel a \"a. Training\"\n  x: image \"$x$\"\n  net: unet \"Model\"\n  p: feature-stack \"$\\hat y$\"\n  l: loss \"$\\mathcal L$\"\n  y: image \"$y$\" role=gt\n  x -> net -> p -> l\n  y -> l : supervision\nend\npanel b \"b. Inference\"\n  x2: image \"$x$\"\n  net2: unet \"Model\"\n  o: image \"mask\"\n  x2 -> net2 -> o\nend\n";
const FIG_B: &str = "fig b\nx: image \"$x$\"\ne: encoder \"Encoder\"\nz: vector-bar \"$z$\"\nd: decoder \"Decoder\"\nr: image \"$\\hat x$\"\nl: loss \"$\\mathcal L_{rec}$\"\nx -> e -> z -> d -> r -> l\n";

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("out dir"));
    std::fs::create_dir_all(&out).unwrap();
    let cat = match args.next() {
        Some(d) => Catalogue::load_all(&[std::path::Path::new(&d)]),
        None => Catalogue::builtin(),
    };
    let mut items = from_catalogue(&cat, "flow", Source::lore("sc"));
    items.push(spec_item("flow", "Training row and inference row", &json!({ "lore_flow": 1, "dsl": FIG_A }), Source::author()));
    items.push(spec_item("flow", "Encoder, code, decoder, reconstruction loss", &json!({ "lore_flow": 1, "dsl": FIG_B }), Source::author()));
    let mut index = vec![];
    for (n, i) in items.iter().enumerate() {
        let local = i.id.split(':').nth(1).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("fig{n}"));
        match thumbnail(i, &cat) {
            Ok(svg) => {
                let file = format!("{local}.svg");
                std::fs::write(out.join(&file), svg).unwrap();
                index.push(json!({ "id": local, "title": i.title, "level": i.level, "line": Figure.line(i), "file": file }));
            }
            Err(e) => eprintln!("{local}: {e}"),
        }
    }
    std::fs::write(out.join("index.json"), serde_json::to_string_pretty(&index).unwrap()).unwrap();
    println!("{} drawn", index.len());
}
