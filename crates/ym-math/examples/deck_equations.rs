//! Convert every native equation (`a14:m`) in real decks to LaTeX and back.
//!
//! ```sh
//! YM_DECKS="/path/a.pptx:/path/b.pptx" cargo run -p ym-math --example deck_equations
//! ```
//! Prints counts (OMML→LaTeX ok, LaTeX→OMML ok) and a few samples. Needs pandoc.

use ym_math::{Pandoc, slide_equations};
use ym_ooxml::Deck;

fn main() {
    let pandoc = Pandoc::default();
    match pandoc.version() {
        Ok(v) => println!("{v}"),
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    }
    let show: usize = std::env::var("YM_SHOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    for path in std::env::var("YM_DECKS")
        .expect("YM_DECKS")
        .split(':')
        .filter(|s| !s.is_empty())
    {
        let deck = Deck::open(path).expect("open deck");
        let (mut n, mut to_tex, mut back) = (0, 0, 0);
        let mut samples = Vec::new();
        let mut errors = Vec::new();
        for s in &deck.slides {
            for m in slide_equations(&s.doc) {
                n += 1;
                match pandoc.omml_to_latex(&m) {
                    Ok(t) => {
                        to_tex += 1;
                        if pandoc.latex_to_omml(&t, false).is_ok() {
                            back += 1;
                        }
                        if samples.len() < show {
                            samples.push(t);
                        }
                    }
                    Err(e) => errors.push(e.to_string()),
                }
            }
        }
        println!("{path}: equations {n}, OMML→LaTeX ok {to_tex}, LaTeX→OMML ok {back}");
        for s in samples {
            println!("    {s}");
        }
        for e in errors.iter().take(3) {
            println!("    error: {e}");
        }
    }
}
