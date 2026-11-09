//! Command-line front end, mainly for evaluation on real papers (outputs go
//! wherever you point them; never into the repo).
//!
//! ```sh
//! cargo run -p lore-docsrc --example docsrc -- latex <src-dir> <out.json>
//! cargo run -p lore-docsrc --example docsrc -- pdf <paper.pdf> <out-dir> [crop-dpi]
//! cargo run -p lore-docsrc --example docsrc -- both <src-dir> <compiled.pdf> <out-dir>
//! ```

use std::path::Path;
use lore_docsrc::{AssetCatalog, AssetKind, TextOptions};

fn summary(c: &AssetCatalog) {
    println!("title: {}", c.meta.title);
    println!("authors: {:?}", c.meta.authors);
    println!("keywords: {:?}", c.meta.keywords);
    println!(
        "abstract: {}",
        c.meta
            .abstract_text
            .as_deref()
            .unwrap_or("-")
            .chars()
            .take(120)
            .collect::<String>()
    );
    println!("sections: {}", c.sections.len());
    for s in &c.sections {
        println!(
            "  {:<10} L{} {:<6} {} ({} paras)",
            s.id,
            s.level,
            s.number.as_deref().unwrap_or(""),
            s.title,
            s.paragraphs.len()
        );
    }
    for k in [
        AssetKind::Figure,
        AssetKind::Subfigure,
        AssetKind::Table,
        AssetKind::Equation,
    ] {
        let all: Vec<_> = c.of_kind(k).collect();
        let numbered = all.iter().filter(|a| a.number.is_some()).count();
        println!("{k:?}: {} ({} numbered)", all.len(), numbered);
        for a in all {
            let cap = a
                .caption
                .as_deref()
                .or(a.text.as_deref())
                .unwrap_or("")
                .chars()
                .take(70)
                .collect::<String>();
            let bb = a
                .bbox
                .map(|b| {
                    format!(
                        "p{} [{:.0},{:.0},{:.0},{:.0}]",
                        a.page.unwrap_or(0),
                        b.x0,
                        b.y0,
                        b.x1,
                        b.y1
                    )
                })
                .unwrap_or_default();
            println!(
                "  {:<8} n={:<5} conf={:.2} {} | {}",
                a.id,
                a.number.as_deref().unwrap_or("-"),
                a.confidence,
                bb,
                cap.replace('\n', " ")
            );
        }
    }
    println!("references: {}", c.references.len());
    for r in c.references.iter().take(3) {
        println!(
            "  {} :: {}",
            r.key,
            r.text.chars().take(100).collect::<String>()
        );
    }
    let words: usize = c
        .section_texts(TextOptions::default())
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum();
    println!("plain-text words: {words}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("latex") => {
            let c = lore_docsrc::latex::extract_dir(Path::new(&args[1])).expect("extract");
            summary(&c);
            if let Some(out) = args.get(2) {
                c.save(Path::new(out)).unwrap();
            }
        }
        Some("pdf") => {
            let mut o = lore_docsrc::pdf::PdfOptions::new(Path::new(&args[2]));
            if let Some(d) = args.get(3) {
                o.crop_dpi = d.parse().unwrap();
            }
            let c = lore_docsrc::pdf::extract(Path::new(&args[1]), &o).expect("extract");
            summary(&c);
        }
        Some("both") => {
            let mut tex = lore_docsrc::latex::extract_dir(Path::new(&args[1])).expect("latex");
            let o = lore_docsrc::pdf::PdfOptions::new(Path::new(&args[3]));
            let pdf = lore_docsrc::pdf::extract(Path::new(&args[2]), &o).expect("pdf");
            let got = tex.attach_pdf(&pdf);
            println!(
                "attached {} of {} LaTeX assets: {:?}",
                got.len(),
                tex.assets.len(),
                got
            );
            summary(&tex);
            tex.save(&Path::new(&args[3]).join("latex-with-pdf.json"))
                .unwrap();
        }
        _ => eprintln!(
            "usage: docsrc latex <dir> [out.json] | pdf <file.pdf> <out-dir> [dpi] | both <dir> <pdf> <out-dir>"
        ),
    }
}
