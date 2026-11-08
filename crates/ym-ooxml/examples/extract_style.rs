//! Extract a style pack draft from decks.
//!
//! ```sh
//! YM_DECKS="/path/a.pptx:/path/b.pptx" YM_STYLE_DIR=styles/my-style YM_STYLE_ID=my-style \
//!   YM_STYLE_NAME="My style" cargo run -p ym-ooxml --example extract_style
//! ```
//! Writes into `YM_STYLE_DIR` (never overwriting existing files):
//! * `template.pptx` — masters/layouts/theme/media of the first deck, zero slides;
//! * `style.draft.toml` — statistical draft (tags, palette, layouts) to hand-tune
//!   into `style.toml` (also written as `style.toml` when none exists yet);
//! * `extract-stats.json` — the raw statistics (counts only, no slide text).
//! Also reports whether all decks share byte-identical masters/layouts/themes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ym_ooxml::Deck;
use ym_ooxml::style::extract_draft;
use ym_ooxml::xml::canonical_eq_bytes;

fn write_new(path: &Path, bytes: &[u8]) {
    if path.exists() {
        println!("keep existing {}", path.display());
    } else {
        std::fs::write(path, bytes).unwrap();
        println!("wrote {}", path.display());
    }
}

fn main() {
    let decks: Vec<PathBuf> = std::env::var("YM_DECKS")
        .expect("YM_DECKS")
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    let out = PathBuf::from(std::env::var("YM_STYLE_DIR").expect("YM_STYLE_DIR"));
    let id = std::env::var("YM_STYLE_ID").unwrap_or_else(|_| "style".into());
    let name = std::env::var("YM_STYLE_NAME").unwrap_or_else(|_| id.clone());
    std::fs::create_dir_all(&out).unwrap();
    let loaded: Vec<Deck> = decks
        .iter()
        .map(|p| Deck::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .collect();

    // shared template parts?
    let mut bytes_same: BTreeMap<String, usize> = BTreeMap::new();
    let mut canon_same: BTreeMap<String, usize> = BTreeMap::new();
    let first = &loaded[0];
    for p in first.pkg.parts.iter().filter(|p| {
        p.name.starts_with("ppt/slideMasters/")
            || p.name.starts_with("ppt/slideLayouts/")
            || p.name.starts_with("ppt/theme/")
    }) {
        let same = loaded
            .iter()
            .filter(|d| d.pkg.get(&p.name) == Some(p.data.as_slice()))
            .count();
        bytes_same.insert(p.name.clone(), same);
        let canon = loaded
            .iter()
            .filter(|d| {
                d.pkg.get(&p.name).is_some_and(|b| {
                    p.name.ends_with(".rels") && b == p.data.as_slice()
                        || matches!(canonical_eq_bytes(b, &p.data), Ok(Ok(())))
                })
            })
            .count();
        canon_same.insert(p.name.clone(), canon);
    }
    let n = loaded.len();
    let all_same = canon_same.values().all(|c| *c == n);
    let byte_diff: Vec<_> = bytes_same
        .iter()
        .filter(|(_, c)| **c != n)
        .map(|(k, _)| k.as_str())
        .collect();
    println!("masters/layouts/themes canonically identical across {n} decks: {all_same}");
    println!("  byte-different (attribute order only when canonical-equal): {byte_diff:?}");

    let tpl = first.template_package().unwrap();
    write_new(&out.join("template.pptx"), &tpl.to_bytes().unwrap());
    let draft = extract_draft(&loaded);
    let mut pack = draft.to_style_pack(&id, &name);
    pack.pack.source = format!(
        "{} deck(s); masters/layouts/themes canonically identical across decks: {all_same}",
        loaded.len()
    );
    let toml = pack.to_toml();
    write_new(&out.join("style.draft.toml"), toml.as_bytes());
    write_new(&out.join("style.toml"), toml.as_bytes());
    write_new(
        &out.join("extract-stats.json"),
        serde_json::to_string_pretty(&draft).unwrap().as_bytes(),
    );

    println!("\nlayouts used: {:?}", draft.layouts);
    println!("top text clusters:");
    for c in draft.clusters.iter().take(25) {
        println!(
            "  {:>6} chars {:>4} runs  {} / {}  {:>5.1}pt bold={} color={}  {:?}",
            c.chars,
            c.runs,
            c.key.latin,
            c.key.ea,
            c.key.size_10 as f64 / 10.0,
            c.key.bold,
            c.key.color,
            c.roles
        );
    }
    println!("colors:");
    for c in draft.colors.iter().take(30) {
        println!(
            "  {} text={} fill={} line={} alphas={:?}",
            c.hex, c.text, c.fill, c.line, c.alphas
        );
    }
    println!("picture alpha: {:?}", draft.picture_alpha);
    println!("circle fills: {:?}", draft.circle_fills);
    println!("trapezoid fills: {:?}", draft.trapezoid_fills);
    println!("dashed roundRects: {}", draft.dashed_round_rects);
    println!(
        "connector widths: {:?}  dashes: {:?}",
        draft.connector_widths, draft.dashes
    );
    println!("geometries: {:?}", draft.geometries);
    println!(
        "tables: {:?}  math objects: {}",
        draft.table_styles, draft.math_objects
    );
}
