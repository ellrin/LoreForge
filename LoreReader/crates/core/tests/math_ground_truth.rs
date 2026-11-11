//! Symbol detection against LaTeX ground truth.
//!
//! `tests/math/paperN.tex` are synthetic papers (invented content). They were compiled
//! with pdflatex and the pdf.js text items of the PDF were saved as
//! `tests/math/paperN.items.json` (the same JSON the reader page posts to
//! `/api/papers/{sha}/read`; see `tests/math/dump-items.js`). The PDFs are not kept.
//!
//! The ground truth is what lore-notation's own LaTeX scanner finds in the source
//! (`symbols::source_keys`); detection is compared under the same canonical keys.

use std::collections::BTreeSet;

use lr_core::symbols::{self, Glyphs};
use lr_core::text::PaperText;

fn load(name: &str) -> (PaperText, String) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/math");
    let items: PaperText = serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.items.json"))).unwrap()).unwrap();
    let tex = std::fs::read_to_string(dir.join(format!("{name}.tex"))).unwrap();
    (items.normalise(), tex)
}

struct Score {
    tp: usize,
    fp: Vec<String>,
    fn_: Vec<String>,
}

fn score(name: &str) -> (Score, symbols::Detection, PaperText) {
    let (paper, tex) = load(name);
    let det = symbols::detect(&paper, &Glyphs::default());
    let found = symbols::detected_keys(&det);
    let truth: BTreeSet<String> = symbols::source_keys(&tex);
    let tp = found.intersection(&truth).count();
    let fp = found.difference(&truth).cloned().collect();
    let fn_ = truth.difference(&found).cloned().collect();
    (Score { tp, fp, fn_ }, det, paper)
}

#[test]
fn precision_and_recall_on_synthetic_latex_papers() {
    let (mut tp, mut fp, mut fn_) = (0, 0, 0);
    for name in ["paper1", "paper2", "paper3", "paper4"] {
        let (s, _, _) = score(name);
        let p = s.tp as f64 / (s.tp + s.fp.len()).max(1) as f64;
        let r = s.tp as f64 / (s.tp + s.fn_.len()).max(1) as f64;
        eprintln!("{name}: tp {} fp {} fn {} precision {:.2} recall {:.2}\n  false positives: {:?}\n  missed: {:?}", s.tp, s.fp.len(), s.fn_.len(), p, r, s.fp, s.fn_);
        tp += s.tp;
        fp += s.fp.len();
        fn_ += s.fn_.len();
    }
    let p = tp as f64 / (tp + fp) as f64;
    let r = tp as f64 / (tp + fn_) as f64;
    eprintln!("all: tp {tp} fp {fp} fn {fn_} precision {p:.3} recall {r:.3}");
    assert!(p >= 0.85, "precision {p:.3}");
    assert!(r >= 0.85, "recall {r:.3}");
}

#[test]
fn numbered_display_equations() {
    for (name, want) in [("paper1", vec!["1", "2"]), ("paper2", vec!["1", "2", "3"]), ("paper3", vec!["1", "2", "3"]), ("paper4", vec!["1", "2", "3", "4"])] {
        let (_, det, _) = score(name);
        let nums: Vec<String> = det.equations.iter().filter_map(|q| q.number.clone()).collect();
        assert_eq!(nums, want, "{name}: {:?}", det.equations);
        for q in &det.equations {
            assert!(q.bbox[2] > q.bbox[0] && q.bbox[3] > q.bbox[1], "{q:?}");
        }
    }
}

/// Patterns from a real paper with very many symbols (sub/superscript nesting,
/// expectations over distributions, norms with subscripts), on synthetic LaTeX.
#[test]
fn nested_scripts_expectations_and_norms() {
    let (_, det, _) = score("paper2");
    let keys = symbols::detected_keys(&det);
    for k in ["\\mathbb{P}_{r}", "\\mathbb{P}_{\\theta}", "f_{w}", "g_{\\theta}", "\\mathcal{W}", "\\Pi", "\\gamma", "n_{\\mathrm{critic}}", "f", "c", "\\alpha", "m", "K"] {
        assert!(keys.contains(k), "{k} not in {keys:?}");
    }
    let latex: Vec<String> = det.spans.iter().map(|s| s.latex.replace(' ', "")).collect();
    for want in ["\\mathbb{P}_{\\theta'}", "g_{\\theta'}", "\\mathbb{P}_{\\theta_{t}}", "\\mathbb{E}_{x\\sim\\mathbb{P}_{r}}", "\\|f\\|_{L}", "\\mathbb{E}_{z\\simp(z)}", "\\Pi(\\mathbb{P}_{r},"] {
        assert!(latex.iter().any(|l| l.contains(want)), "{want} not in {latex:?}");
    }
}

#[test]
fn definitions_quote_their_sentence_and_conflicts_are_found() {
    let (_, det, paper) = score("paper1");
    let t = symbols::build(&paper, &det, "test");
    let get = |k: &str| t.entries.iter().find(|e| e.entry.latex.as_deref() == Some(k)).unwrap_or_else(|| panic!("{k}"));
    let w = get("w");
    assert_eq!(w.grade, "defined", "{w:?}");
    assert!(w.evidence[0].quote.contains("is the weight of pixel"), "{:?}", w.evidence);
    assert_eq!(get("\\lambda").grade, "defined");
    assert_eq!(get("\\theta").grade, "defined", "let θ be the parameters");
    for e in t.entries.iter().filter(|e| e.grade == "defined") {
        assert!(!e.evidence.is_empty() && e.evidence[0].page >= 1 && !e.evidence[0].quote.is_empty(), "🟢 only with a quote: {e:?}");
    }
    let (_, det, paper) = score("paper2");
    let t = symbols::build(&paper, &det, "test");
    let m = t.entries.iter().find(|e| e.entry.latex.as_deref() == Some("m")).unwrap();
    assert_eq!(m.grade, "conflict", "{:?}", m.evidence);
    assert_eq!(m.evidence.len(), 2);
}

/// Display equations stay inside their paragraph ("… is (1) where w_i is …"), and the
/// words of math runs are not nouns.
#[test]
fn paragraphs_keep_display_equations_and_math_is_not_a_noun() {
    let (_, det, paper) = score("paper1");
    let t = symbols::build(&paper, &det, "test");
    let eqs: Vec<(u32, usize, usize, String)> = t.equations.iter().map(|q| (q.page, q.s, q.e, q.entry.label.clone().unwrap_or_default())).collect();
    let paras = lr_core::paragraphs::split_with(&paper, &eqs);
    let with_eq = paras.iter().find(|p| p.text.contains("［式 (1)］")).unwrap_or_else(|| panic!("{:#?}", paras.iter().map(|p| &p.text).collect::<Vec<_>>()));
    assert!(with_eq.text.contains("The weighted loss is") && with_eq.text.contains("where"), "{}", with_eq.text);
    assert_eq!(with_eq.section, "1 Method");
    assert!(paras.iter().any(|p| p.section_kind == "result"));
    let mask: Vec<(u32, usize, usize)> = t.entries.iter().flat_map(|e| e.occurrences.iter().map(|o| (o.page, o.s, o.e))).chain(t.equations.iter().map(|q| (q.page, q.s, q.e))).collect();
    let inv = lr_core::nouns::inventory_masked(&paper, &[], &Default::default(), &Default::default(), &mask);
    for junk in ["lce", "nx", "rh"] {
        assert!(!inv.entries.iter().any(|e| e.key == junk), "{junk} is math, not a noun");
    }
    assert!(inv.entries.iter().any(|e| e.key == "curvature"));
}

/// A least-squares GAN loss (paper4): stacked \frac{1}{2} rebuilt, `\sim` over named
/// distributions, a tilde and a prime on the letter, three rows sharing number (1) and two
/// rows sharing (2) as `aligned`, one equation per number.
#[test]
fn fractions_rows_tildes_and_primes() {
    let (_, det, _) = score("paper4");
    let eq: Vec<(&str, &str)> = det.equations.iter().map(|q| (q.id.as_str(), q.latex.as_str())).collect();
    assert_eq!(eq.len(), 4, "{eq:#?}");
    assert_eq!(
        eq[0].1,
        "\\begin{aligned}\\mathcal{L}_{D}(D_{p}) &=\\frac{1}{2} \\mathbb{E}_{x_{p}\\sim P_{\\mathrm{real}}}[(D_{p}(x_{p}) - 1)^{2}] \\\\ \
         &\\quad +\\frac{1}{2} \\mathbb{E}_{\\tilde{x}_{p}\\sim P_{\\mathrm{syn}}}[D_{p}( \\tilde{x}_{p})^{2}] \\\\ \
         \\mathcal{L}_{G}(G_{p}) &=\\frac{1}{2} \\mathbb{E}_{\\tilde{x}_{p}\\sim P_{\\mathrm{syn}}}[(D_{p}( \\tilde{x}_{p}) - 1)^{2}]\\end{aligned}"
    );
    assert_eq!(eq[1].1, "\\begin{aligned}\\mathcal{L}_{\\mathrm{rec}} &= \\mathbb{E}_{x_{p}\\sim P_{\\mathrm{real}}}[\\|x_{p}- x'_{p}\\|_{2}] \\\\ x'_{p} &= R_{p}(D_{p}(x_{p}))\\end{aligned}");
    assert_eq!(eq[2].1, "\\mathcal{L}_{\\mathrm{dis}}= \\lambda_{a}\\mathcal{L}_{D}+ \\lambda_{b}\\mathcal{L}_{\\mathrm{rec}}");
    assert_eq!(eq[3].1, "\\mathcal{L}_{\\mathrm{gen}}= \\lambda_{a}\\mathcal{L}_{G}");
    let keys = symbols::detected_keys(&det);
    assert!(keys.contains("x_{p}^{\\prime}") && keys.contains("\\tilde{x}_{p}") && keys.contains("P_{\\mathrm{real}}"), "{keys:?}");
}
