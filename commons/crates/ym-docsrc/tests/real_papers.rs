//! Evaluation on real papers (not in the repo). Ignored by default:
//!
//! ```sh
//! YM_DOCSRC_PAPERS='/path/a.pdf=4,5,17|/path/b.pdf=8,4,9' \
//! YM_DOCSRC_OUT=/tmp/docsrc-eval \
//!   cargo test -p ym-docsrc --test real_papers -- --ignored --nocapture
//! YM_DOCSRC_LATEX_SRC=/path/src YM_DOCSRC_LATEX_PDF=/path/main.pdf \
//!   cargo test -p ym-docsrc --test real_papers latex_vs_pdf -- --ignored --nocapture
//! ```
//! `=figures,tables,equations` are the true counts (numbered); outputs (crops,
//! catalog.json) go to `YM_DOCSRC_OUT/<n>/` for a look.

use std::path::{Path, PathBuf};
use ym_docsrc::AssetKind;
use ym_docsrc::pdf::{self, PdfOptions};

fn out_dir(n: usize) -> (Option<tempfile::TempDir>, PathBuf) {
    match std::env::var_os("YM_DOCSRC_OUT") {
        Some(d) => (None, PathBuf::from(d).join(n.to_string())),
        None => {
            let t = tempfile::tempdir().unwrap();
            let p = t.path().to_path_buf();
            (Some(t), p)
        }
    }
}

#[test]
#[ignore]
fn real_pdfs_match_true_counts() {
    let spec = std::env::var("YM_DOCSRC_PAPERS").expect("set YM_DOCSRC_PAPERS");
    for (n, item) in spec.split('|').filter(|s| !s.trim().is_empty()).enumerate() {
        let (path, want) = item
            .rsplit_once('=')
            .map(|(p, w)| (p, Some(w)))
            .unwrap_or((item, None));
        let (_keep, dir) = out_dir(n);
        let c = pdf::extract(Path::new(path.trim()), &PdfOptions::new(&dir)).unwrap();
        let placed = |k: AssetKind| {
            c.of_kind(k)
                .filter(|a| a.bbox.is_some() && a.number.is_some())
                .count()
        };
        let got = [
            placed(AssetKind::Figure),
            placed(AssetKind::Table),
            placed(AssetKind::Equation),
        ];
        println!(
            "{path}: figures/tables/equations placed = {got:?}; sections {}; references {}",
            c.sections.len(),
            c.references.len()
        );
        for a in c.assets.iter().filter(|a| a.confidence < 0.6) {
            println!("  low confidence {} {:?} {:?}", a.id, a.confidence, a.notes);
        }
        // equation numbers run 1..=N without gaps
        let mut nums: Vec<u32> = c
            .of_kind(AssetKind::Equation)
            .filter_map(|a| a.number.as_deref()?.parse().ok())
            .collect();
        nums.sort();
        assert!(
            nums.iter().enumerate().all(|(i, v)| *v == i as u32 + 1),
            "{path}: equation numbers {nums:?}"
        );
        if let Some(w) = want {
            let want: Vec<usize> = w.split(',').map(|x| x.trim().parse().unwrap()).collect();
            assert_eq!(got.to_vec(), want, "{path}");
        }
        for a in c.assets.iter().filter(|a| a.crop.is_some()) {
            assert!(dir.join(a.crop.as_ref().unwrap()).is_file());
        }
    }
}

#[test]
#[ignore]
fn latex_vs_pdf() {
    let src = std::env::var("YM_DOCSRC_LATEX_SRC").expect("set YM_DOCSRC_LATEX_SRC");
    let pdf_path = std::env::var("YM_DOCSRC_LATEX_PDF").expect("set YM_DOCSRC_LATEX_PDF");
    let (_keep, dir) = out_dir(99);
    let mut tex = ym_docsrc::latex::extract_dir(Path::new(&src)).unwrap();
    let pdf = pdf::extract(Path::new(&pdf_path), &PdfOptions::new(&dir)).unwrap();
    let numbered: Vec<String> = tex
        .assets
        .iter()
        .filter(|a| {
            matches!(
                a.kind,
                AssetKind::Figure | AssetKind::Table | AssetKind::Equation
            ) && a.number.is_some()
        })
        .map(|a| a.id.clone())
        .collect();
    let attached = tex.attach_pdf(&pdf);
    println!(
        "attached {}/{}; LaTeX sections {} vs PDF {}",
        attached.len(),
        numbered.len(),
        tex.sections.len(),
        pdf.sections.len()
    );
    let missing: Vec<&String> = numbered
        .iter()
        .filter(|id| !attached.contains(id))
        .collect();
    assert!(missing.is_empty(), "no PDF region for {missing:?}");
    assert_eq!(tex.references.len(), pdf.references.len());
}
