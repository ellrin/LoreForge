//! Export an editor SVG from the command line (needs Chrome; poppler optional):
//! `cargo run -p lore-plot --example export -- figure.svg out.pdf|out.svg|out.png [dpi]`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: export <in.svg> <out.pdf|svg|png> [dpi]");
        std::process::exit(2);
    }
    let svg = std::fs::read_to_string(&a[1]).expect("read svg");
    let fmt = lore_plot::Format::parse(a[2].rsplit('.').next().unwrap_or("")).expect("pdf, svg or png");
    let dpi = a.get(3).and_then(|d| d.parse().ok()).unwrap_or(300);
    let spec = lore_plot::export::spec_from_svg(&svg);
    match lore_plot::export::export(&lore_plot::Tools::find(), &svg, spec.as_deref(), fmt, dpi) {
        Ok(b) => std::fs::write(&a[2], b).expect("write"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
