//! Write the synthetic Word paper used by the tests:
//! `cargo run -p pc-server --example synth_docx -- out.docx`.
fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "synthetic.docx".into());
    std::fs::write(&out, pc_server::word::synth::docx()).expect("write");
    println!("{out}");
}
