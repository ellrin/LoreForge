//! End-to-end on a tiny two-column PDF made for these tests
//! (`tests/fixtures/twocol/main.tex` → `tests/fixtures/twocol.pdf`).
//! Needs poppler (`pdftotext`, `pdftoppm`, `pdfinfo`); skipped when missing.

use std::path::{Path, PathBuf};
use lore_docsrc::boxer::{MockBoxer, ModuleBox, box_modules};
use lore_docsrc::crop::{self, Renderer};
use lore_docsrc::pdf::{self, PdfOptions, poppler::Poppler};
use lore_docsrc::{AssetCatalog, AssetKind, BBox, TextOptions};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn poppler() -> Option<Poppler> {
    let p = Poppler::discover();
    if p.available() {
        Some(p)
    } else {
        eprintln!("poppler not found: skipped");
        None
    }
}

fn extract(dir: &Path, pop: Poppler) -> AssetCatalog {
    let mut o = PdfOptions::new(dir);
    o.poppler = pop;
    o.crop_dpi = 144;
    pdf::extract(&fixture("twocol.pdf"), &o).unwrap()
}

#[test]
fn pdf_mode_finds_everything() {
    let Some(pop) = poppler() else { return };
    let dir = tempfile::tempdir().unwrap();
    let c = extract(dir.path(), pop.clone());

    assert_eq!(c.meta.title, "A Toy Model for Testing Asset Extraction");
    assert_eq!(c.meta.authors, vec!["Ellrin Example", "Daidai Sample"]);
    assert!(
        c.meta
            .abstract_text
            .as_deref()
            .unwrap()
            .starts_with("We describe a toy encoder")
    );
    let ids: Vec<&str> = c.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["abstract", "s1", "s2", "s3"]);
    assert_eq!(c.sections[3].title, "Results");
    assert_eq!(c.pages[0].columns.len(), 2);

    let fig = c.asset("fig1").unwrap();
    assert_eq!(
        fig.caption.as_deref(),
        Some("The toy architecture with an encoder and a memory block.")
    );
    let fb = fig.bbox.unwrap();
    assert!(
        fb.x1 < 310.0 && fb.height() > 60.0 && fb.height() < 130.0,
        "{fb:?}"
    );
    assert!(
        fb.y1 <= fig.caption_bbox.unwrap().y0,
        "figure is above its caption"
    );

    let tab = c.asset("tab1").unwrap();
    assert_eq!(
        tab.caption.as_deref(),
        Some("Scores on the invented datasets.")
    );
    let tb = tab.bbox.unwrap();
    assert!(
        tb.x0 > 306.0 && tb.y0 >= tab.caption_bbox.unwrap().y1,
        "table below its caption: {tb:?}"
    );
    assert!(
        tab.text
            .as_deref()
            .unwrap()
            .contains("Toy model\t0.72\t0.69"),
        "{:?}",
        tab.text
    );

    let eqs: Vec<&str> = c
        .of_kind(AssetKind::Equation)
        .filter_map(|a| a.number.as_deref())
        .collect();
    assert_eq!(eqs, vec!["1", "2"]);
    assert_eq!(c.asset("eq1").unwrap().text.as_deref(), Some("h = Wx + b"));
    assert_eq!(c.asset("eq2").unwrap().section.as_deref(), Some("s2"));
    assert_eq!(tab.section.as_deref(), Some("s3"));

    assert_eq!(c.references.len(), 2);
    assert_eq!(c.references[0].key, "[1]");
    assert!(c.references[1].text.starts_with("D. Sample"));

    // crops and the catalog are on disk; sizes follow the crop DPI
    for a in c.assets.iter().filter(|a| a.bbox.is_some()) {
        let png = dir.path().join(a.crop.as_ref().unwrap());
        let img = image::open(&png).unwrap();
        let b = a.bbox.unwrap();
        assert!(
            (img.width() as f64 - b.width() * 2.0).abs() <= 3.0,
            "{} {}",
            a.id,
            img.width()
        );
    }
    let back = AssetCatalog::load(&dir.path().join("catalog.json")).unwrap();
    assert_eq!(back, c);

    // plain text for scanners: prose only, no table cells or equation pieces
    let texts = c.section_texts(TextOptions::default());
    let results = texts.iter().find(|s| s.id == "s3").unwrap();
    assert!(results.text.contains("invented datasets"));
    assert!(!results.text.contains("0.72"));
    let method = texts.iter().find(|s| s.id == "s2").unwrap();
    assert!(!method.text.contains("Wx + b"));

    // page renders are cached: a second run reuses them
    let page = dir.path().join(Renderer::page_rel(1, 100, true));
    let t0 = std::fs::metadata(&page).unwrap().modified().unwrap();
    let _ = extract(dir.path(), pop);
    assert_eq!(std::fs::metadata(&page).unwrap().modified().unwrap(), t0);
}

#[test]
fn latex_and_pdf_modes_agree() {
    let Some(pop) = poppler() else { return };
    let dir = tempfile::tempdir().unwrap();
    let pdf = extract(dir.path(), pop);
    let mut tex = lore_docsrc::latex::extract_dir(&fixture("twocol")).unwrap();
    assert_eq!(tex.meta.title, pdf.meta.title);
    assert_eq!(tex.meta.authors, pdf.meta.authors);
    let sec_titles = |c: &AssetCatalog| {
        c.sections
            .iter()
            .map(|s| s.title.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(sec_titles(&tex), sec_titles(&pdf));
    let attached = tex.attach_pdf(&pdf);
    assert_eq!(attached, vec!["fig1", "eq1", "eq2", "tab1"]);
    let eq2 = tex.asset("eq2").unwrap();
    assert!(eq2.text.as_deref().unwrap().contains("\\sum_{i=1}^{N}"));
    assert_eq!(eq2.label.as_deref(), Some("eq:loss"));
    assert_eq!(eq2.bbox, pdf.asset("eq2").unwrap().bbox);
    assert_eq!(tex.references.len(), pdf.references.len());
}

#[test]
fn crop_operations_render() {
    let Some(pop) = poppler() else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut c = extract(dir.path(), pop.clone());
    let fb = c.asset("fig1").unwrap().bbox.unwrap();
    let left = crop::recrop(
        &mut c,
        "fig1",
        BBox::new(0.0, 0.0, fb.width() / 2.0, fb.height()),
    )
    .unwrap();
    crop::mask(&mut c, &left, &[BBox::new(0.0, 0.0, 20.0, 20.0)]).unwrap();
    let parts = crop::split(&mut c, "fig1", crop::Axis::Vertical, &[fb.width() / 2.0]).unwrap();
    assert_eq!(parts.len(), 2);
    let r = Renderer::new(pop, &fixture("twocol.pdf"), dir.path());
    let png = r.render_asset(c.asset(&left).unwrap(), 144).unwrap();
    let img = image::open(png).unwrap().to_rgba8();
    assert!((img.width() as f64 - fb.width()).abs() <= 3.0);
    // masked corner is white
    assert!(img.get_pixel(5, 5).0[..3].iter().all(|v| *v == 255));
}

#[tokio::test]
async fn semantic_module_boxes_become_regions() {
    let Some(pop) = poppler() else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut c = extract(dir.path(), pop.clone());
    let r = Renderer::new(pop, &fixture("twocol.pdf"), dir.path());
    let boxer = MockBoxer::new(vec![
        ModuleBox {
            label: "Encoder".into(),
            role: Some("model".into()),
            bbox: [0.0, 0.0, 120.0, 150.0],
        },
        ModuleBox {
            label: "Memory".into(),
            role: Some("data".into()),
            bbox: [180.0, 0.0, 330.0, 150.0],
        },
    ]);
    let ids = box_modules(&mut c, "fig1", &r, &boxer, 144).await.unwrap();
    assert_eq!(ids, vec!["fig1.m1", "fig1.m2"]);
    let fb = c.asset("fig1").unwrap().bbox.unwrap();
    let m1 = c.asset("fig1.m1").unwrap();
    assert_eq!(m1.label.as_deref(), Some("Encoder"));
    assert!(fb.contains(&m1.bbox.unwrap()));
    assert_eq!(
        boxer.seen.lock().unwrap()[0],
        "The toy architecture with an encoder and a memory block."
    );
}
