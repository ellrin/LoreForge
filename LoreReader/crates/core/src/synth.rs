//! Synthetic text PDFs (tests, demos, screenshots). Never a real paper.
//!
//! One Helvetica text line per `Tj`, US Letter, 11 pt — enough for pdf.js's text layer,
//! lopdf's `extract_text`, and page-operation tests ("Page 3" is on page 3).

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

/// A PDF with one page per entry; each page is a list of lines (ASCII).
pub fn text_pdf(pages: &[Vec<String>]) -> Vec<u8> {
    titled_pdf(None, pages)
}

/// Like [`text_pdf`], with `/Title` in the document info.
pub fn titled_pdf(title: Option<&str>, pages: &[Vec<String>]) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let bold_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold", "Encoding" => "WinAnsiEncoding",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id, "F2" => bold_id } });
    let mut kids = vec![];
    for lines in pages {
        let mut ops = vec![Operation::new("BT", vec![])];
        let mut y = 740.0;
        for (i, line) in lines.iter().enumerate() {
            // A line starting with "# " is a heading (bold, larger).
            let (font, size, text) = match line.strip_prefix("# ") {
                Some(h) => ("F2", 15.0, h),
                None => ("F1", 11.0, line.as_str()),
            };
            if i > 0 {
                y -= if size > 11.0 { 24.0 } else { 15.5 };
            }
            ops.push(Operation::new("Tf", vec![font.into(), Object::Real(size)]));
            ops.push(Operation::new("Tm", vec![1.into(), 0.into(), 0.into(), 1.into(), Object::Real(72.0), Object::Real(y)]));
            ops.push(Operation::new("Tj", vec![Object::string_literal(text)]));
            if line.is_empty() {
                y += 5.0;
            }
        }
        ops.push(Operation::new("ET", vec![]));
        let content = Content { operations: ops };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().expect("content")));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        });
        kids.push(page_id.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => count,
            "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    if let Some(t) = title {
        let info = doc.add_object(dictionary! { "Title" => Object::string_literal(t), "Producer" => Object::string_literal("lr-core synth") });
        doc.trailer.set("Info", info);
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

/// Pages "Page 1" … "Page n" (page-operation tests).
pub fn numbered_pdf(n: usize, tag: &str) -> Vec<u8> {
    let pages: Vec<Vec<String>> = (1..=n).map(|i| vec![format!("{tag} Page {i}")]).collect();
    text_pdf(&pages)
}

/// The demo paper used by the UI checks: known and unknown terms, an acronym with its
/// expansion, hyphenated terms, a DOI and a references list. Invented content.
pub fn demo_paper() -> Vec<u8> {
    let p = |lines: &[&str]| lines.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    titled_pdf(
        Some("A Synthetic Study of Overlap Metrics"),
        &[
            p(&[
                "# A Synthetic Study of Overlap Metrics",
                "ellrin and daidai",
                "doi:10.5555/lorereader.demo.2026",
                "",
                "# Abstract",
                "We compare the Dice coefficient with the Jaccard index on synthetic masks.",
                "A Spectral Boundary Loss (SBL) is introduced for thin structures, and a",
                "cross-entropy baseline is trained with label-smoothing. The softmax output",
                "is calibrated with temperature scaling before thresholding.",
                "",
                "# 1. Introduction",
                "Segmentation quality is usually reported as overlap. The Dice coefficient",
                "rewards agreement between prediction and reference, while the Hausdorff",
                "distance measures the worst boundary error. Our Variational Mask Prior",
                "regularises shapes with a learned codebook and anisotropic diffusion.",
            ]),
            p(&[
                "# 2. Method",
                "The network is a U-Net with residual blocks. Each block applies batch",
                "normalization, a rectified unit and dropout. The Spectral Boundary Loss",
                "weights pixels near edges by their curvature, estimated with a Sobel filter.",
                "Training uses stochastic gradient descent with momentum and a cosine",
                "learning-rate schedule. Hyperparameters were chosen on a held-out split.",
                "",
                "# 3. Results",
                "On the synthetic benchmark the Dice coefficient improves from 0.81 to 0.86.",
                "The Hausdorff distance drops by a third. Ablations show that the SBL term",
                "matters most for thin vessels, whereas the Variational Mask Prior helps",
                "on blurry boundaries.",
            ]),
            p(&[
                "# 4. Discussion",
                "Overlap metrics hide boundary errors; reporting both kinds is advisable.",
                "Future work will study heteroscedastic uncertainty and test-time augmentation.",
                "",
                "# References",
                "[1] A. Author. Overlap Metrics Revisited. Journal of Examples, 2020.",
                "[2] B. Writer. Boundary Losses for Thin Structures. Proc. Synthetic Conf., 2021.",
            ]),
        ],
    )
}
