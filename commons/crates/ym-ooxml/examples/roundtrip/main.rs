//! Round-trip harness for real decks (never committed as fixtures).
//!
//! ```sh
//! YM_DECKS="/path/a.pptx:/path/b.pptx" YM_SCRATCH=/tmp/rt \
//!   cargo run -p ym-ooxml --example roundtrip
//! ```
//! Optional: `YM_RENDERER=none|quicklook` (default: PowerPoint > Keynote > QuickLook),
//! `YM_DPI` (default 48).
//!
//! For every deck: parse → per-slide inventory (objects, locked objects, raw-kept
//! features) → regenerate every slide/notes part from the typed model and write
//! the deck to scratch → reopen and compare: slide/notes parts canonically, every
//! other part byte-for-byte → optional visual comparison. Writes
//! `$YM_SCRATCH/report.json` and prints a summary table.

mod visual;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use ym_ooxml::inventory::{SlideInventory, inventory};
use ym_ooxml::xml::canonical_eq_bytes;
use ym_ooxml::{Deck, Package};

#[derive(Serialize, Default)]
struct SlideResult {
    inv: SlideInventory,
    canonical_equal: bool,
    diff: Option<String>,
    notes_equal: Option<bool>,
    ssim: Option<f64>,
    mad: Option<f64>,
}

#[derive(Serialize, Default)]
struct DeckResult {
    label: String,
    slides: usize,
    hidden: usize,
    objects: usize,
    locked: usize,
    locked_kinds: BTreeMap<String, usize>,
    raw_features: BTreeMap<String, usize>,
    canonical_equal_slides: usize,
    byte_identical_slides: usize,
    /// slide model → JSON → model → XML still canonically equal
    json_roundtrip_equal: usize,
    other_parts: usize,
    other_parts_identical: usize,
    part_list_equal: bool,
    second_save_identical: bool,
    fully_editable_pct: f64,
    parse_ms: u128,
    write_ms: u128,
    renderer: Option<String>,
    rendered_pages: Option<(usize, usize)>,
    ssim_mean: Option<f64>,
    ssim_min: Option<f64>,
    pages_ssim_ge_09: Option<usize>,
    /// Rebuild = fresh deck from the extracted style template + every slide imported.
    rebuild_canonical_equal: usize,
    rebuild_dangling_rels: usize,
    rebuild_ssim_mean: Option<f64>,
    rebuild_ssim_min: Option<f64>,
    rebuild_pages_ssim_ge_09: Option<usize>,
    slides_detail: Vec<SlideResult>,
    error: Option<String>,
}

fn label_for(p: &Path) -> String {
    // parent dir names, e.g. 2024-spring-talk
    let comps: Vec<String> = p
        .parent()
        .map(|d| {
            d.components()
                .rev()
                .take(2)
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut c = comps;
    c.reverse();
    c.join("-")
}

fn main() {
    let decks: Vec<PathBuf> = std::env::var("YM_DECKS")
        .expect("set YM_DECKS (':'-separated .pptx paths)")
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    let scratch = PathBuf::from(std::env::var("YM_SCRATCH").expect("set YM_SCRATCH (output dir)"));
    std::fs::create_dir_all(&scratch).unwrap();
    let dpi: u32 = std::env::var("YM_DPI")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    let renderer = visual::Renderer::detect();
    println!(
        "renderer: {}",
        renderer
            .as_ref()
            .map(|r| r.name())
            .unwrap_or("none (structural only)")
    );

    let mut results = Vec::new();
    for (i, d) in decks.iter().enumerate() {
        let label = format!("{}-{}", i + 1, label_for(d));
        let mut r = DeckResult {
            label: label.clone(),
            ..Default::default()
        };
        if let Err(e) = run_deck(d, &scratch, &label, renderer.as_ref(), dpi, &mut r) {
            r.error = Some(e);
        }
        results.push(r);
    }

    println!();
    println!(
        "| deck | slides (hidden) | objects | locked | locked kinds | canonical-equal (byte-identical) slides | other parts identical | fully editable | SSIM mean/min (pages ≥0.9) |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for r in &results {
        println!(
            "| {} | {} ({}) | {} | {} | {} | {}/{} ({}) | {}/{} | {:.0}% | {} |",
            r.label,
            r.slides,
            r.hidden,
            r.objects,
            r.locked,
            if r.locked_kinds.is_empty() {
                "-".into()
            } else {
                format!("{:?}", r.locked_kinds)
            },
            r.canonical_equal_slides,
            r.slides,
            r.byte_identical_slides,
            r.other_parts_identical,
            r.other_parts,
            r.fully_editable_pct,
            match (r.ssim_mean, r.ssim_min) {
                (Some(m), Some(n)) => format!(
                    "{m:.4}/{n:.4} ({}/{})",
                    r.pages_ssim_ge_09.unwrap_or(0),
                    r.rendered_pages.map(|p| p.0).unwrap_or(0)
                ),
                _ => "n/a".into(),
            }
        );
        if let Some(e) = &r.error {
            println!("  ERROR: {e}");
        }
    }
    let rep = scratch.join("report.json");
    std::fs::write(&rep, serde_json::to_string_pretty(&results).unwrap()).unwrap();
    for r in &results {
        println!(
            "{}: json-roundtrip-equal {}/{}, second save identical {}, parse {} ms, write {} ms, raw-kept features {:?}",
            r.label,
            r.json_roundtrip_equal,
            r.slides,
            r.second_save_identical,
            r.parse_ms,
            r.write_ms,
            r.raw_features
        );
        println!(
            "   rebuild from template: canonical-equal {}/{}, dangling rels {}, SSIM mean/min {:?}/{:?} (pages ≥0.9: {:?})",
            r.rebuild_canonical_equal,
            r.slides,
            r.rebuild_dangling_rels,
            r.rebuild_ssim_mean,
            r.rebuild_ssim_min,
            r.rebuild_pages_ssim_ge_09
        );
    }
    println!("\nreport: {}", rep.display());
}

fn run_deck(
    path: &Path,
    scratch: &Path,
    label: &str,
    renderer: Option<&visual::Renderer>,
    dpi: u32,
    r: &mut DeckResult,
) -> Result<(), String> {
    let t0 = Instant::now();
    let deck = Deck::open(path).map_err(|e| e.to_string())?;
    r.parse_ms = t0.elapsed().as_millis();
    let inv = inventory(&deck);

    let t1 = Instant::now();
    let out_pkg = deck.to_package();
    let bytes = out_pkg.to_bytes().map_err(|e| e.to_string())?;
    r.write_ms = t1.elapsed().as_millis();
    let src_copy = scratch.join(format!("{label}.orig.pptx"));
    let rt = scratch.join(format!("{label}.rt.pptx"));
    for p in [&src_copy, &rt] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&rt, &bytes).map_err(|e| e.to_string())?;

    // compare
    let orig = Package::open(path).map_err(|e| e.to_string())?;
    let back = Package::from_bytes(&bytes).map_err(|e| e.to_string())?;
    r.part_list_equal = orig.parts.iter().map(|p| &p.name).collect::<Vec<_>>()
        == back.parts.iter().map(|p| &p.name).collect::<Vec<_>>();
    let regenerated: Vec<&str> = deck
        .slides
        .iter()
        .flat_map(|s| {
            std::iter::once(s.part.as_str()).chain(s.notes.as_ref().map(|n| n.part.as_str()))
        })
        .collect();
    for p in &orig.parts {
        if regenerated.contains(&p.name.as_str()) {
            continue;
        }
        r.other_parts += 1;
        if back.get(&p.name) == Some(p.data.as_slice()) {
            r.other_parts_identical += 1;
        }
    }
    for (s, iv) in deck.slides.iter().zip(inv) {
        let mut sr = SlideResult {
            inv: iv,
            ..Default::default()
        };
        match canonical_eq_bytes(orig.get(&s.part).unwrap(), back.get(&s.part).unwrap()) {
            Ok(Ok(())) => sr.canonical_equal = true,
            Ok(Err(d)) => sr.diff = Some(d),
            Err(e) => sr.diff = Some(e.to_string()),
        }
        if let Some(n) = &s.notes {
            sr.notes_equal = Some(matches!(
                canonical_eq_bytes(orig.get(&n.part).unwrap(), back.get(&n.part).unwrap()),
                Ok(Ok(()))
            ));
        }
        r.byte_identical_slides += (orig.get(&s.part) == back.get(&s.part)) as usize;
        let json = serde_json::to_string(&s.doc).map_err(|e| e.to_string())?;
        let doc2: ym_ooxml::SlideDoc = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        r.json_roundtrip_equal += matches!(
            canonical_eq_bytes(orig.get(&s.part).unwrap(), &doc2.to_bytes()),
            Ok(Ok(()))
        ) as usize;
        r.slides += 1;
        r.hidden += sr.inv.hidden as usize;
        r.objects += sr.inv.objects;
        r.locked += sr.inv.locked.len();
        for k in &sr.inv.locked {
            *r.locked_kinds.entry(k.clone()).or_default() += 1;
        }
        for (k, v) in &sr.inv.raw_features {
            *r.raw_features.entry(k.clone()).or_default() += v;
        }
        r.canonical_equal_slides += sr.canonical_equal as usize;
        r.slides_detail.push(sr);
    }
    let editable = r
        .slides_detail
        .iter()
        .filter(|s| s.inv.fully_editable())
        .count();
    r.fully_editable_pct = if r.slides == 0 {
        100.0
    } else {
        editable as f64 * 100.0 / r.slides as f64
    };
    // idempotence: parse the written deck and write again
    let again = Deck::from_bytes(&bytes)
        .and_then(|d| d.to_bytes())
        .map_err(|e| e.to_string())?;
    let p2 = Package::from_bytes(&again).map_err(|e| e.to_string())?;
    r.second_save_identical = p2
        .parts
        .iter()
        .zip(&back.parts)
        .all(|(a, b)| a.name == b.name && a.data == b.data)
        && p2.parts.len() == back.parts.len();

    // rebuild from the style template
    let tpl = deck.template_package().map_err(|e| e.to_string())?;
    let mut nd = Deck::from_package(tpl).map_err(|e| e.to_string())?;
    for i in 0..deck.slides.len() {
        nd.import_slide(&deck, i).map_err(|e| e.to_string())?;
    }
    let rb_bytes = nd.to_bytes().map_err(|e| e.to_string())?;
    let rb = scratch.join(format!("{label}.rebuild.pptx"));
    let _ = std::fs::remove_file(&rb);
    std::fs::write(&rb, &rb_bytes).map_err(|e| e.to_string())?;
    let rbp = Package::from_bytes(&rb_bytes).map_err(|e| e.to_string())?;
    let rbd = Deck::from_bytes(&rb_bytes).map_err(|e| e.to_string())?;
    for (a, b) in deck.slides.iter().zip(&rbd.slides) {
        r.rebuild_canonical_equal += matches!(
            canonical_eq_bytes(orig.get(&a.part).unwrap(), rbp.get(&b.part).unwrap()),
            Ok(Ok(()))
        ) as usize;
        for rel in b.rels.items.iter().filter(|x| !x.external) {
            if !rbp.has(&ym_ooxml::package::resolve_target(&b.part, &rel.target)) {
                r.rebuild_dangling_rels += 1;
            }
        }
    }

    for s in r.slides_detail.iter().filter(|s| !s.canonical_equal) {
        println!(
            "[{label}] slide {} differs: {}",
            s.inv.index,
            s.diff.as_deref().unwrap_or("?")
        );
    }

    // visual
    if let Some(ren) = renderer {
        r.renderer = Some(ren.name().into());
        std::fs::copy(path, &src_copy).map_err(|e| e.to_string())?;
        let vdir = scratch.join(format!("{label}.render"));
        let _ = std::fs::remove_dir_all(&vdir);
        let pa = ren.to_pdf(&src_copy, &vdir)?;
        let pb = ren.to_pdf(&rt, &vdir)?;
        let ga = visual::rasterize(&pa, &vdir.join("orig"), dpi)?;
        let gb = visual::rasterize(&pb, &vdir.join("rt"), dpi)?;
        r.rendered_pages = Some((ga.len(), gb.len()));
        let scores: Vec<(f64, f64)> = ga
            .iter()
            .zip(&gb)
            .map(|(a, b)| visual::compare(a, b))
            .collect();
        if !scores.is_empty() {
            r.ssim_mean = Some(scores.iter().map(|s| s.0).sum::<f64>() / scores.len() as f64);
            r.ssim_min = Some(scores.iter().map(|s| s.0).fold(1.0, f64::min));
            r.pages_ssim_ge_09 = Some(scores.iter().filter(|s| s.0 >= 0.9).count());
        }
        // pages map to visible slides in order (renderers skip hidden slides)
        let visible: Vec<usize> = r
            .slides_detail
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.inv.hidden)
            .map(|(i, _)| i)
            .collect();
        let pages_cover_all = ga.len() == r.slides;
        for (pi, (ssim, mad)) in scores.iter().enumerate() {
            let si = if pages_cover_all {
                Some(pi)
            } else {
                visible.get(pi).copied()
            };
            if let Some(si) = si {
                r.slides_detail[si].ssim = Some(*ssim);
                r.slides_detail[si].mad = Some(*mad);
            }
        }
        let pc = ren.to_pdf(&rb, &vdir)?;
        let gc = visual::rasterize(&pc, &vdir.join("rebuild"), dpi)?;
        let sc: Vec<f64> = ga
            .iter()
            .zip(&gc)
            .map(|(a, b)| visual::compare(a, b).0)
            .collect();
        if !sc.is_empty() && ga.len() == gc.len() {
            r.rebuild_ssim_mean = Some(sc.iter().sum::<f64>() / sc.len() as f64);
            r.rebuild_ssim_min = Some(sc.iter().copied().fold(1.0, f64::min));
            r.rebuild_pages_ssim_ge_09 = Some(sc.iter().filter(|s| **s >= 0.9).count());
        }
        let _ = std::fs::remove_file(&src_copy);
    }
    Ok(())
}
