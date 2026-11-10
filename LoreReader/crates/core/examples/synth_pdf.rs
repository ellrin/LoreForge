//! Write synthetic PDFs (never a real paper) for demos and UI checks.
//!
//! cargo run -p lr-core --example synth_pdf -- demo out.pdf      # the demo paper (3 pages)
//! cargo run -p lr-core --example synth_pdf -- long out.pdf      # ≈8,000 words (noun-list scenario)
//! cargo run -p lr-core --example synth_pdf -- numbered 5 out.pdf # "Page 1" … "Page 5"

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (bytes, out) = match a.first().map(String::as_str) {
        Some("demo") => (lr_core::synth::demo_paper(), a.get(1)),
        Some("long") => (lr_core::synth::long_paper(7), a.get(1)),
        Some("numbered") => (lr_core::synth::numbered_pdf(a.get(1).and_then(|n| n.parse().ok()).unwrap_or(3), "Extra"), a.get(2)),
        _ => {
            eprintln!("usage: synth_pdf demo <out.pdf> | long <out.pdf> | numbered <n> <out.pdf>");
            std::process::exit(2);
        }
    };
    let out = out.expect("output path");
    std::fs::write(out, bytes).expect("write");
    eprintln!("wrote {out}");
}
