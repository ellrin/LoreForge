//! lore-math — equations for SlideComposer / PaperComposer.
//!
//! * LaTeX → OMML: pandoc (offline sidecar) converts `$$…$$` to a minimal docx;
//!   the `m:oMath`/`m:oMathPara` is lifted out of `word/document.xml`.
//! * OMML → LaTeX: the OMML (Word or PowerPoint flavour) is wrapped in a minimal
//!   docx and pandoc converts it to LaTeX.
//! * Word ↔ PowerPoint flavour: PowerPoint stores run formatting as `a:rPr`
//!   (Cambria Math) inside `m:r`, and math-italic code points (𝐼, 𝑡) in `m:t`.
//! * [`equation_shape`] builds the `mc:AlternateContent` text box PowerPoint
//!   writes (a14 math choice + optional picture fallback).
//! * [`MathRenderer`]: hook for PNG/SVG fallback images (stub for now).
//! * [`check`]: what KaTeX would refuse to typeset, without a browser ([`check::check`]),
//!   and [`check::check_reading`] for LaTeX read out of a PDF (brackets must pair too).

pub mod check;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use lore_ooxml::model::*;
use lore_ooxml::package::Package;
use lore_ooxml::shapes::Rect;
use lore_ooxml::xml::{Attrs, Document, Element, Node, parse_element};

pub const NS_M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
pub const NS_A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
pub const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "pandoc not found at {0} — install pandoc (e.g. `brew install pandoc`) or set LORE_PANDOC"
    )]
    PandocMissing(String),
    #[error("pandoc failed: {0}")]
    Pandoc(String),
    #[error("no math found in pandoc output")]
    NoMath,
    #[error("ooxml: {0}")]
    Ooxml(#[from] lore_ooxml::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not supported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// pandoc sidecar configuration.
#[derive(Debug, Clone)]
pub struct Pandoc {
    pub bin: PathBuf,
}

impl Default for Pandoc {
    /// `$LORE_PANDOC`, else `/opt/homebrew/bin/pandoc` / `/usr/local/bin/pandoc` if present, else `pandoc` on PATH.
    fn default() -> Self {
        // YM_PANDOC: the name before the crates were renamed, still honoured.
        if let Ok(p) = std::env::var("LORE_PANDOC").or_else(|_| std::env::var("YM_PANDOC")) {
            return Self { bin: p.into() };
        }
        for p in ["/opt/homebrew/bin/pandoc", "/usr/local/bin/pandoc"] {
            if std::path::Path::new(p).exists() {
                return Self { bin: p.into() };
            }
        }
        Self {
            bin: "pandoc".into(),
        }
    }
}

impl Pandoc {
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// `pandoc --version` first line, or [`Error::PandocMissing`].
    pub fn version(&self) -> Result<String> {
        let out = Command::new(&self.bin)
            .arg("--version")
            .output()
            .map_err(|_| Error::PandocMissing(self.bin.display().to_string()))?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string())
    }
    pub fn available(&self) -> bool {
        self.version().is_ok()
    }

    fn run(&self, args: &[&str], stdin: &[u8]) -> Result<Vec<u8>> {
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| Error::PandocMissing(self.bin.display().to_string()))?;
        child.stdin.take().unwrap().write_all(stdin)?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(Error::Pandoc(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(out.stdout)
    }

    /// LaTeX math (without delimiters) → Word-flavour OMML element
    /// (`m:oMathPara` when `display`, else `m:oMath`), carrying `xmlns:m`.
    pub fn latex_to_omml(&self, latex: &str, display: bool) -> Result<Element> {
        let src = if display {
            format!("$${}$$\n", latex.trim())
        } else {
            format!("${}$\n", latex.trim())
        };
        let docx = self.run(&["-f", "latex", "-t", "docx", "-o", "-"], src.as_bytes())?;
        let pkg = Package::from_bytes(&docx)?;
        let doc = pkg.xml("word/document.xml")?;
        let want = if display { "m:oMathPara" } else { "m:oMath" };
        let mut found = doc.root.find_all(want).into_iter().next().cloned();
        if found.is_none() && display {
            found = doc.root.find_all("m:oMath").into_iter().next().cloned();
        }
        let mut m = found.ok_or(Error::NoMath)?;
        m.attrs.set("xmlns:m", NS_M);
        Ok(m)
    }

    /// LaTeX → PowerPoint-flavour OMML (inside `a14:m`), runs styled Cambria Math at `size_pt`.
    pub fn latex_to_pptx_math(&self, latex: &str, display: bool, size_pt: f64) -> Result<Element> {
        let m = self.latex_to_omml(latex, display)?;
        Ok(word_to_pptx(&m, size_pt))
    }

    /// OMML (Word or PowerPoint flavour; `m:oMath`, `m:oMathPara` or `a14:m`) → LaTeX.
    pub fn omml_to_latex(&self, omml: &Element) -> Result<String> {
        let word = pptx_to_word(omml);
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="{NS_M}"><w:body><w:p>{}</w:p></w:body></w:document>"#,
            word.to_xml_string()
        );
        let mut pkg = Package::default();
        pkg.set(
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec(),
        );
        pkg.set(
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec(),
        );
        pkg.set("word/document.xml", body.into_bytes());
        let docx = pkg.to_bytes()?;
        let out = self.run(&["-f", "docx", "-t", "latex", "--wrap=none"], &docx)?;
        let s = String::from_utf8_lossy(&out).trim().to_string();
        strip_delims(&s).ok_or(Error::NoMath)
    }
}

fn strip_delims(s: &str) -> Option<String> {
    let s = s.trim();
    for (a, b) in [("\\[", "\\]"), ("$$", "$$"), ("\\(", "\\)"), ("$", "$")] {
        if let (Some(i), Some(j)) = (s.find(a), s.rfind(b))
            && j > i + a.len() - 1
            && j >= i + a.len()
        {
            return Some(s[i + a.len()..j].trim().to_string());
        }
    }
    None
}

/// Map Unicode mathematical alphanumerics (𝐼 italic, 𝐌 bold, 𝑴 bold italic) to ASCII.
/// Returns (ascii, bold).
pub fn demath_char(c: char) -> (char, bool) {
    let u = c as u32;
    let blocks: [(u32, bool); 4] = [
        (0x1D400, true),
        (0x1D434, false),
        (0x1D468, true),
        (0x1D49C, false),
    ];
    for (start, bold) in blocks {
        if (start..start + 52).contains(&u) {
            let k = u - start;
            let ch = if k < 26 {
                b'A' + k as u8
            } else {
                b'a' + (k - 26) as u8
            };
            return (ch as char, bold);
        }
    }
    if c == '\u{210E}' {
        return ('h', false); // italic h (Planck constant)
    }
    if (0x1D7CE..0x1D7D8).contains(&u) {
        return (char::from_digit(u - 0x1D7CE, 10).unwrap(), true);
    }
    (c, false)
}

/// PowerPoint OMML → Word OMML: drop `a:*` formatting, unwrap `a14:m`,
/// map math-italic code points to ASCII (bold → `m:sty b`).
pub fn pptx_to_word(el: &Element) -> Element {
    let mut root = if el.name == "a14:m" {
        el.elements()
            .next()
            .cloned()
            .unwrap_or_else(|| Element::new("m:oMath"))
    } else {
        el.clone()
    };
    fn rec(e: &mut Element) {
        e.children
            .retain(|n| !matches!(n, Node::Element(c) if c.name.starts_with("a:")));
        if e.name == "m:r" {
            let mut any_bold = false;
            if let Some(t) = e.child_mut("m:t") {
                let txt = t.text();
                let mut out = String::new();
                for c in txt.chars() {
                    let (a, b) = demath_char(c);
                    any_bold |= b;
                    out.push(a);
                }
                t.children = vec![Node::Text(out)];
            }
            if any_bold && e.child("m:rPr").is_none() {
                e.children.insert(
                    0,
                    Node::Element(
                        Element::new("m:rPr")
                            .with_child(Element::new("m:sty").with_attr("m:val", "b")),
                    ),
                );
            }
        }
        for c in e.elements_mut() {
            rec(c);
        }
    }
    rec(&mut root);
    root.attrs.set("xmlns:m", NS_M);
    root
}

fn cambria_rpr(size_pt: f64, italic: bool) -> Element {
    let mut r = Element::new("a:rPr")
        .with_attr("lang", "en-US")
        .with_attr("altLang", "zh-TW")
        .with_attr("sz", ((size_pt * 100.0).round() as i64).to_string());
    if italic {
        r.attrs.set("i", "1");
    }
    r.with_child(
        Element::new("a:latin")
            .with_attr("typeface", "Cambria Math")
            .with_attr("panose", "02040503050406030204")
            .with_attr("pitchFamily", "18")
            .with_attr("charset", "0"),
    )
}

/// Word OMML → PowerPoint OMML wrapped in `a14:m`: drop `w:*`, add `a:rPr`
/// (Cambria Math) to every `m:r` and `m:ctrlPr`.
pub fn word_to_pptx(el: &Element, size_pt: f64) -> Element {
    let mut m = el.clone();
    fn rec(e: &mut Element, size: f64) {
        e.children
            .retain(|n| !matches!(n, Node::Element(c) if c.name.starts_with("w:")));
        if e.name == "m:r" {
            let upright = e
                .child("m:rPr")
                .and_then(|p| p.child("m:sty"))
                .and_then(|s| s.attr("m:val"))
                .map(|v| v == "p" || v == "b")
                .unwrap_or(false);
            let pos = e
                .children
                .iter()
                .position(|n| matches!(n, Node::Element(c) if c.name == "m:t"))
                .unwrap_or(0);
            e.children
                .insert(pos, Node::Element(cambria_rpr(size, !upright)));
        } else if e.name == "m:ctrlPr" {
            e.children.push(Node::Element(cambria_rpr(size, true)));
        }
        for c in e.elements_mut() {
            rec(c, size);
        }
    }
    rec(&mut m, size_pt);
    m.attrs.set("xmlns:m", NS_M);
    Element::new("a14:m").with_child(m)
}

/// Hook for rendering equation fallback images (old PowerPoint, previews).
pub trait MathRenderer {
    /// PNG bytes of `latex` at roughly `size_pt`.
    fn render_png(&self, latex: &str, size_pt: f64) -> Result<Vec<u8>>;
    fn render_svg(&self, latex: &str, size_pt: f64) -> Result<String>;
}

/// Stub renderer. TODO: real renderer (e.g. MathJax/KaTeX via a sidecar, or
/// Typst) — until then equations are written without a picture fallback.
pub struct NoRenderer;
impl MathRenderer for NoRenderer {
    fn render_png(&self, _latex: &str, _size_pt: f64) -> Result<Vec<u8>> {
        Err(Error::Unsupported("equation PNG rendering (TODO)".into()))
    }
    fn render_svg(&self, _latex: &str, _size_pt: f64) -> Result<String> {
        Err(Error::Unsupported("equation SVG rendering (TODO)".into()))
    }
}

/// Text box holding one equation paragraph, as PowerPoint writes it:
/// `mc:AlternateContent` with an a14 choice (`a14:m`) and, when `fallback_rel`
/// is given (an image relationship id), a picture-filled fallback shape.
pub fn equation_shape(
    id: u32,
    name: &str,
    rect: Rect,
    pptx_math: Element,
    fallback_rel: Option<&str>,
) -> Shape {
    let Shape::Sp(mut sp) = lore_ooxml::shapes::text_box(id, name, rect, vec![]) else {
        unreachable!()
    };
    if let Some(tb) = sp.text.as_mut() {
        tb.body.attrs = Attrs(vec![
            ("wrap".into(), "none".into()),
            ("lIns".into(), "0".into()),
            ("tIns".into(), "0".into()),
            ("rIns".into(), "0".into()),
            ("bIns".into(), "0".into()),
            ("rtlCol".into(), "0".into()),
        ]);
        tb.paragraphs = vec![Paragraph {
            props: None,
            runs: vec![Run::Math(pptx_math)],
            end_props: None,
        }];
    }
    let choice = AltBranch {
        attrs: Attrs(vec![("Requires".into(), "a14".into())]),
        shapes: vec![Shape::Sp(sp.clone())],
    };
    let fallback = fallback_rel.map(|rid| {
        let mut fb = sp.clone();
        if let Some(tb) = fb.text.as_mut() {
            tb.paragraphs = vec![Paragraph {
                props: None,
                runs: vec![],
                end_props: Some(RunProps { tag: "a:endParaRPr".into(), attrs: Attrs(vec![("lang".into(), "en-US".into())]), ..Default::default() }),
            }];
        }
        let bf = parse_element(
            format!(r#"<a:blipFill xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>"#)
                .as_bytes(),
        )
        .map(|mut e| {
            e.attrs.remove("xmlns:r");
            e
        })
        .expect("static xml");
        fb.props.fill = Some(Fill::Blip(bf));
        AltBranch { attrs: Attrs(vec![("xmlns".into(), "".into())]), shapes: vec![Shape::Sp(fb)] }
    });
    Shape::Alt(AltContent {
        attrs: Attrs(vec![
            ("xmlns:mc".into(), NS_MC.into()),
            ("xmlns:a14".into(), NS_A14.into()),
        ]),
        choices: vec![choice],
        fallback,
    })
}

/// Pull every equation (`a14:m`) out of a slide, in reading order.
pub fn slide_equations(doc: &SlideDoc) -> Vec<Element> {
    let mut out = Vec::new();
    for s in doc.shapes() {
        for m in s.to_element().find_all("a14:m") {
            out.push(m.clone());
        }
    }
    out
}

/// Parse a standalone OMML string.
pub fn parse_omml(xml: &str) -> Result<Element> {
    Ok(Document::parse(xml.as_bytes())?.root)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPTX_SAMPLE: &str = r#"<a14:m xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:sSub><m:sSubPr><m:ctrlPr><a:rPr lang="en-US" sz="1100" i="1"><a:latin typeface="Cambria Math"/></a:rPr></m:ctrlPr></m:sSubPr><m:e><m:r><a:rPr lang="en-US" sz="1100" b="1"><a:latin typeface="Cambria Math"/></a:rPr><m:t>𝐌</m:t></m:r></m:e><m:sub><m:r><a:rPr lang="en-US" sz="1100" i="1"><a:latin typeface="Cambria Math"/></a:rPr><m:t>𝑡</m:t></m:r></m:sub></m:sSub></m:oMath></a14:m>"#;

    #[test]
    fn math_alphanumerics_map_to_ascii() {
        assert_eq!(demath_char('𝐼'), ('I', false));
        assert_eq!(demath_char('𝐌'), ('M', true));
        assert_eq!(demath_char('𝑡'), ('t', false));
        assert_eq!(demath_char('ℎ'), ('h', false));
        assert_eq!(demath_char('→'), ('→', false));
    }

    #[test]
    fn pptx_word_flavours_convert_both_ways() {
        let el = parse_omml(PPTX_SAMPLE).unwrap();
        let w = pptx_to_word(&el);
        assert_eq!(w.name, "m:oMath");
        let xml = w.to_xml_string();
        assert!(!xml.contains("a:rPr"));
        assert!(
            xml.contains("<m:t>M</m:t>")
                && xml.contains(r#"<m:sty m:val="b"/>"#)
                && xml.contains("<m:t>t</m:t>")
        );
        let p = word_to_pptx(&w, 14.0);
        assert_eq!(p.name, "a14:m");
        let px = p.to_xml_string();
        assert_eq!(px.matches("Cambria Math").count(), 3); // 2 runs + ctrlPr
        assert!(px.contains(r#"sz="1400""#));
    }

    #[test]
    fn equation_shape_has_choice_and_fallback() {
        let el = parse_omml(PPTX_SAMPLE).unwrap();
        let s = equation_shape(
            5,
            "Equation 4",
            Rect::pt(10.0, 10.0, 100.0, 20.0),
            el,
            Some("rId9"),
        );
        let x = s.to_element().to_xml_string();
        assert!(x.starts_with("<mc:AlternateContent"));
        assert!(
            x.contains(r#"<mc:Choice Requires="a14">"#) && x.contains(r#"<mc:Fallback xmlns="">"#)
        );
        assert!(x.contains(r#"r:embed="rId9""#));
        assert_eq!(s.kind_name(), "math");
    }

    #[test]
    fn missing_pandoc_is_a_clear_error() {
        let p = Pandoc::new("/nonexistent/pandoc-xyz");
        let e = p.latex_to_omml("x^2", false).unwrap_err();
        assert!(matches!(e, Error::PandocMissing(_)));
        assert!(e.to_string().contains("install pandoc"));
    }

    #[test]
    fn strip_delims_variants() {
        assert_eq!(strip_delims("\\[x^{2}\\]").as_deref(), Some("x^{2}"));
        assert_eq!(strip_delims("$a+b$").as_deref(), Some("a+b"));
    }

    /// Needs pandoc; skipped (passes) when it is not installed.
    #[test]
    fn pandoc_roundtrip_latex_omml_latex() {
        let p = Pandoc::default();
        if !p.available() {
            eprintln!("pandoc not available — skipping");
            return;
        }
        let m = p.latex_to_omml(r"\frac{a}{b} + x_i^2", true).unwrap();
        assert_eq!(m.name, "m:oMathPara");
        assert!(m.contains("m:f") && m.contains("m:sSubSup"));
        let pp = p
            .latex_to_pptx_math(r"\frac{a}{b} + x_i^2", true, 14.0)
            .unwrap();
        let back = p.omml_to_latex(&pp).unwrap();
        let compact: String = back.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains(r"\frac{a}{b}"), "{back}");
        assert!(compact.contains("x_{i}^{2}"), "{back}");
        // PowerPoint-flavour sample with math italic code points
        let l = p.omml_to_latex(&parse_omml(PPTX_SAMPLE).unwrap()).unwrap();
        let l: String = l.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(l.contains("M") && l.contains("_{t}"), "{l}");
    }
}
