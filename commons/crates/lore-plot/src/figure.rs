//! Figures in documents: a paper's `figures/<stem>.pdf` with `<stem>.plot.json` next to it
//! (PaperComposer), a slide picture as a PNG that carries its spec (SlideComposer).

use std::path::{Path, PathBuf};

use crate::export::{self, ExportError, Format, Tools};
use crate::spec::Spec;

/// The spec file next to a figure file (`a/fig.pdf` → `a/fig.plot.json`).
pub fn spec_path(figure: &Path) -> PathBuf {
    let stem = figure.file_stem().and_then(|s| s.to_str()).unwrap_or("figure");
    figure.with_file_name(format!("{stem}.plot.json"))
}

/// The spec saved next to `figure`, if this tool made it.
pub fn spec_next_to(figure: &Path) -> Option<Spec> {
    Spec::from_json(&std::fs::read_to_string(spec_path(figure)).ok()?).ok()
}

/// A file-name stem from free text: lower-case ASCII words joined by `-`, ≤ 40 chars.
pub fn stem_of(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "figure".into() } else { out }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("f"), std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// A stem in `dir` that is free (no `<stem>.pdf`, `<stem>.plot.json`), or `stem` itself when
/// `replace` and the PDF there was made by this tool (its spec is next to it).
pub fn free_stem(dir: &Path, stem: &str, replace: bool) -> String {
    let ours = |s: &str| dir.join(format!("{s}.plot.json")).is_file();
    // a flow figure's PDF (`.flow.json` next to it) belongs to the flow editor, not to this tool
    let taken = |s: &str| dir.join(format!("{s}.pdf")).exists() || dir.join(format!("{s}.plot.json")).exists() || dir.join(format!("{s}.flow.json")).exists();
    if replace && ours(stem) || !taken(stem) {
        return stem.to_string();
    }
    (2..).map(|k| format!("{stem}-{k}")).find(|s| !taken(s)).expect("a free name")
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Saved {
    pub stem: String,
    /// `<stem>.pdf`
    pub file: String,
    /// `<stem>.plot.json`
    pub spec_file: String,
}

/// Write `<dir>/<stem>.pdf` and `<dir>/<stem>.plot.json`. A user's PDF (no spec next to it)
/// is never overwritten: the figure gets a free name instead.
pub fn save_paper_figure(tools: &Tools, dir: &Path, stem: &str, replace: bool, spec: &Spec, svg: &str) -> Result<Saved, ExportError> {
    let stem = free_stem(dir, &stem_of(stem), replace);
    let json = spec.to_json();
    let pdf = export::export(tools, svg, Some(&json), Format::Pdf, 0)?;
    save_files(dir, &stem, &pdf, &json)
}

/// The writing half of [`save_paper_figure`] (PDF bytes already made).
pub fn save_files(dir: &Path, stem: &str, pdf: &[u8], spec_json: &str) -> Result<Saved, ExportError> {
    let file = format!("{stem}.pdf");
    let spec_file = format!("{stem}.plot.json");
    write_atomic(&dir.join(&file), pdf)?;
    write_atomic(&dir.join(&spec_file), spec_json.as_bytes())?;
    Ok(Saved { stem: stem.to_string(), file, spec_file })
}

/// A slide picture: PNG at `dpi` with the spec inside.
pub fn slide_png(tools: &Tools, spec: &Spec, svg: &str, dpi: u32) -> Result<Vec<u8>, ExportError> {
    export::export(tools, svg, Some(&spec.to_json()), Format::Png, dpi)
}

/// The spec a slide picture carries.
pub fn spec_in_png(png: &[u8]) -> Option<Spec> {
    Spec::from_json(&export::spec_from_png(png)?).ok()
}

/// The spec an equation GIF made here carries (application extension `LOREPLOT1.0`).
pub fn spec_in_gif(gif: &[u8]) -> Option<Spec> {
    Spec::from_json(&crate::equation::gif::spec_from_gif(gif)?).ok()
}

/// The spec a slide picture carries, PNG or GIF.
pub fn spec_in_picture(bytes: &[u8]) -> Option<Spec> {
    spec_in_png(bytes).or_else(|| spec_in_gif(bytes))
}

/// The `figure` environment PaperComposer inserts (same shape as its 新增圖…):
/// registered `\label`, caption above or below, `\linewidth` wide.
pub fn figure_block(file: &str, caption: &str, label: &str, caption_above: bool) -> String {
    let cap = [format!("  \\caption{{{}}}", tex_text(caption)), format!("  \\label{{{label}}}")];
    let img = format!("  \\includegraphics[width=\\linewidth]{{{file}}}");
    let mut lines = vec!["\\begin{figure}[t]".to_string(), "  \\centering".into()];
    if caption_above {
        lines.extend(cap);
        lines.push(img);
    } else {
        lines.push(img);
        lines.extend(cap);
    }
    lines.push("\\end{figure}".into());
    lines.join("\n")
}

/// Caption text for LaTeX: `% & #` escaped unless already escaped; maths left alone.
pub fn tex_text(s: &str) -> String {
    let mut out = String::new();
    let mut prev = ' ';
    for c in s.trim().chars() {
        if matches!(c, '%' | '&' | '#') && prev != '\\' {
            out.push('\\');
        }
        out.push(c);
        prev = c;
    }
    out
}
