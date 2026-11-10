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

/// A long synthetic paper (≈8,000 words, invented content) as page lines: the
/// target scenario of the noun list. Deterministic for a given `seed`.
pub fn long_paper_lines(seed: u64) -> Vec<Vec<String>> {
    let mut rng = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut pick = |n: usize| -> usize {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng >> 33) as usize) % n.max(1)
    };
    const METHODS: &[&str] = &["Spectral Boundary Loss", "variational autoencoder", "U-Net", "attention gate", "anisotropic diffusion", "graph convolution", "label smoothing", "test-time augmentation", "deep supervision", "contrastive pretraining", "focal loss", "Monte Carlo dropout", "conditional random field", "residual block", "transformer encoder", "Wasserstein critic", "curriculum schedule", "knowledge distillation", "Gumbel softmax", "stochastic weight averaging"];
    const METRICS: &[&str] = &["Dice coefficient", "Hausdorff distance", "average surface distance", "intersection over union", "AUROC", "Brier score", "expected calibration error", "volumetric similarity", "mean absolute error", "Cohen kappa"];
    const DATASETS: &[&str] = &["SynthVessel", "MaskBench", "OrganGrid", "ThinStructures-2K", "the PolypSim archive", "RetinaToy"];
    const STATS: &[&str] = &["Wilcoxon signed-rank test", "bootstrap confidence interval", "Bonferroni correction", "paired t-test", "mixed-effects model", "Bland-Altman analysis", "intraclass correlation"];
    const ANATOMY: &[&str] = &["retinal vasculature", "coronary artery", "pulmonary nodule", "hepatic lesion", "optic disc", "left ventricle", "renal cortex", "colonic polyp"];
    const NAMES: &[&str] = &["Hausdorff", "Sobel", "Kullback", "Gaussian", "Laplacian"];
    const OPEN: &[&str] = &["In this work", "Following prior studies", "To address this limitation", "As a consequence", "In practice", "For completeness", "In contrast", "Notably", "Overall", "Moreover", "In addition", "Specifically", "Similarly", "Importantly"];
    const VERBS: &[&str] = &["improves", "stabilises", "regularises", "outperforms", "complements", "constrains", "calibrates", "accelerates", "simplifies", "generalises"];
    const GENERAL: &[&str] = &[
        "the proposed approach", "the baseline model", "the training procedure", "our implementation", "the evaluation protocol", "the overall framework",
        "the main experiment", "the reported results", "the second stage", "the previous method", "a simple variant", "the final configuration",
    ];
    const TAILS: &[&str] = &[
        "on every split of the data", "without additional annotation", "at a modest computational cost", "across all reported settings",
        "when the images are noisy", "for thin and tortuous structures", "under strong class imbalance", "in the low-data regime",
        "with a fixed random seed", "after early stopping on the validation set", "compared with the original formulation",
    ];
    // Specialist single words: invented clinical words (root × suffix) and a list of
    // technical words; general vocabulary from the built-in word lists.
    let roots = ["vascul", "angio", "osteo", "cardio", "neuro", "hepato", "nephro", "pulmo", "dermo", "myo", "arterio", "chondro"];
    let sufs = ["graphy", "pathy", "genesis", "metry", "plasty", "sclerosis", "lysis", "trophy"];
    let clinical: Vec<String> = roots.iter().flat_map(|r| sufs.iter().map(move |s| format!("{r}{s}"))).collect();
    const TECH: &[&str] = &["tortuosity", "skeletonization", "homotopy", "persistence", "eigendecomposition", "isotropy", "heteroscedasticity", "quantile", "hyperparameter", "backpropagation", "logit", "softmax", "sigmoid", "tensor", "manifold", "geodesic", "wavelet", "spectrogram", "voxel", "superpixel", "morphology", "dilation", "erosion", "centerline", "bifurcation", "stenosis", "perfusion", "contrastive", "adversarial", "variational", "bayesian", "posterior", "likelihood", "marginalization", "ensembling", "bagging", "boosting", "kurtosis", "skewness", "autocorrelation", "multicollinearity", "imputation", "stratification", "propensity", "bootstrapping", "permutation", "heatmap", "saliency", "occlusion", "registration", "deformation", "diffeomorphism", "jacobian", "hessian", "laplacian", "curvature", "anisotropy", "denoising", "inpainting", "upsampling", "downsampling", "quantization", "pruning", "distillation", "transformer", "tokenizer", "embedding", "encoder", "decoder", "bottleneck", "skip-connection", "batchnorm", "layernorm", "warmup", "annealing", "overfitting", "underfitting", "calibration", "reliability-diagram", "conformal", "ensemble", "uncertainty", "aleatoric", "epistemic", "entropy", "divergence", "regularizer", "sparsity", "lasso", "ridge", "elastic-net", "kriging", "spline", "isosurface", "mesh", "tetrahedral", "octree", "pointcloud", "graphlet", "hypergraph"];
    let general: Vec<&str> = include_str!("common_words.txt").lines().chain(include_str!("academic_words.txt").lines()).filter(|l| !l.starts_with('#')).flat_map(str::split_whitespace).filter(|w| w.len() >= 4).collect();
    let mut sent = |kind: usize| -> String {
        let cw = clinical[pick(clinical.len())].clone();
        let tw = TECH[pick(TECH.len())];
        let tw2 = TECH[pick(TECH.len())];
        let (g1, g2, g3) = (general[pick(general.len())], general[pick(general.len())], general[pick(general.len())]);
        let extra = match pick(4) {
            0 => format!(" The {tw} term relates to {cw} and to the {g1} {g2} of the {g3}."),
            1 => format!(" We note the {g1}, the {g2} and the {g3} of the {tw}."),
            2 => format!(" Here {tw2} and {tw} matter for {cw}."),
            _ => String::new(),
        };
        let o = OPEN[pick(OPEN.len())];
        let g = GENERAL[pick(GENERAL.len())];
        let t = TAILS[pick(TAILS.len())];
        let m = METHODS[pick(METHODS.len())];
        let m2 = METHODS[pick(METHODS.len())];
        let me = METRICS[pick(METRICS.len())];
        let d = DATASETS[pick(DATASETS.len())];
        let s = STATS[pick(STATS.len())];
        let a = ANATOMY[pick(ANATOMY.len())];
        let n = NAMES[pick(NAMES.len())];
        let v = VERBS[pick(VERBS.len())];
        let x = 60 + pick(35);
        let y = pick(10);
        let base = match (kind, pick(6)) {
            (0, 0) => format!("{o}, segmentation of the {a} remains difficult because boundaries are thin and contrast is low."),
            (0, 1) => format!("{o}, earlier work relied on the {m} and reported the {me} on {d}."),
            (0, 2) => format!("{o}, few studies examine how the {m} interacts with the {m2} {t}."),
            (0, 3) => format!("Researchers such as {n} described related ideas, but the {a} was not considered."),
            (0, 4) => format!("{o}, clinicians need reliable estimates of the {a} {t}."),
            (0, _) => format!("{o}, {g} {v} the {m} {t}."),
            (1, 0) => format!("{o}, the {m} is applied to every feature map before the {m2}."),
            (1, 1) => format!("The loss combines the {m} with a {n} kernel whose width is chosen on {d}."),
            (1, 2) => format!("{o}, {g} uses the {m2} so that the {a} keeps its topology."),
            (1, 3) => format!("Each step of the {m} is motivated by the need to preserve the {a} {t}."),
            (1, 4) => format!("We optimise the weights with stochastic gradient descent and a cosine learning-rate schedule {t}."),
            (1, _) => format!("{o}, {g} {v} the {m} {t}."),
            (2, 0) => format!("On {d}, the {me} improves from 0.{x} to 0.{} for the {a}.", x + 3),
            (2, 1) => format!("The {s} shows that the difference is significant (p = 0.00{}).", y + 1),
            (2, 2) => format!("{o}, the {me} of the {m} is within the {s} of the baseline {t}."),
            (2, 3) => format!("Ablations show that removing the {m} lowers the {me} by {y}.{x} points on {d}."),
            (2, 4) => format!("We report the {me} and the {me} with a {s} across five folds."),
            (2, _) => format!("{o}, {g} {v} the {m} {t}."),
            (_, 0) => format!("{o}, these findings suggest that the {m} is useful for the {a}."),
            (_, 1) => format!("A limitation is that {d} contains few examples of the {a}."),
            (_, 2) => format!("Future work will combine the {m} with the {m2} and study the {me} {t}."),
            _ => format!("{o}, {g} {v} the {m} {t}."),
        };
        base + &extra
    };
    let sections: &[(&str, usize, usize)] = &[
        ("Abstract", 0, 10),
        ("1. Introduction", 0, 62),
        ("2. Related Work", 0, 55),
        ("3. Methods", 1, 82),
        ("4. Results", 2, 70),
        ("5. Discussion", 3, 44),
        ("6. Conclusion", 3, 11),
    ];
    let mut lines: Vec<String> = vec!["# A Long Synthetic Study of Thin-Structure Segmentation".into(), "ellrin and daidai".into(), String::new()];
    for (head, kind, n) in sections {
        lines.push(format!("# {head}"));
        let mut para = String::new();
        for k in 0..*n {
            para += &sent(*kind);
            para.push(' ');
            if k % 6 == 5 || k + 1 == *n {
                // wrap at ~88 characters
                let mut line = String::new();
                for w in para.split_whitespace() {
                    if line.len() + w.len() + 1 > 88 {
                        lines.push(std::mem::take(&mut line));
                    }
                    if !line.is_empty() {
                        line.push(' ');
                    }
                    line += w;
                }
                if !line.is_empty() {
                    lines.push(line);
                }
                lines.push(String::new());
                para.clear();
            }
        }
    }
    lines.push("# References".into());
    for i in 1..=12 {
        lines.push(format!("[{i}] A. Writer. Quaternion Wavelet Methods, Part {i}. Journal of Examples, 20{:02}.", i + 8));
    }
    // pages of 44 lines
    lines.chunks(44).map(|c| c.to_vec()).collect()
}

/// [`long_paper_lines`] as a PDF.
pub fn long_paper(seed: u64) -> Vec<u8> {
    titled_pdf(Some("A Long Synthetic Study of Thin-Structure Segmentation"), &long_paper_lines(seed))
}
