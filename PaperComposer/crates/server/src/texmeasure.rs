//! Measure a table with the paper's own LaTeX class: the tabular's natural
//! width at each font size, and the page's column width, text width and
//! text height. Runs pdflatex in draft mode in a temporary folder; the
//! paper folder is only searched for its class and style files.

use pc_core::textable::{self, Measured, TexTable};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn pdflatex() -> Option<PathBuf> {
    tool("pdflatex")
}

/// A TeX (or Pandoc) program: on PATH, or where distributions install it
/// (GUI apps on macOS do not get the shell's PATH).
pub fn tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/Library/TeX/texbin", "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    for root in ["/usr/local/texlive"] {
        if let Ok(years) = std::fs::read_dir(root) {
            for y in years.flatten() {
                if let Ok(archs) = std::fs::read_dir(y.path().join("bin")) {
                    dirs.extend(archs.flatten().map(|a| a.path()));
                }
            }
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Programs/MiKTeX/miktex/bin/x64"));
    }
    dirs.push(PathBuf::from("C:/Program Files/MiKTeX/miktex/bin/x64"));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Pandoc"));
    }
    dirs.push(PathBuf::from("C:/Program Files/Pandoc"));
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// Everything before \begin{document}.
pub fn preamble(main_tex: &str) -> Option<&str> {
    main_tex.find("\\begin{document}").map(|i| &main_tex[..i])
}

/// The probe document: one box per font size, widths written to the log.
pub fn probe(preamble: &str, t: &TexTable) -> String {
    // Measure the plain tabular (no float, natural width).
    let mut bare = t.clone();
    bare.env = String::new();
    bare.width = None;
    bare.fill = false;
    bare.font = None;
    let tabular = textable::render(&bare);
    let colsep = t.colsep.as_ref().map(|c| format!("\\setlength{{\\tabcolsep}}{{{c}}}")).unwrap_or_default();
    let mut body = String::from("\\newsavebox\\pcbox\n");
    for size in textable::FONT_STEPS {
        body += &format!("\\sbox\\pcbox{{\\{size}{colsep}{tabular}}}\\typeout{{PCW {size} \\the\\wd\\pcbox}}\n");
    }
    body += "\\typeout{PCW columnwidth \\the\\columnwidth}\\typeout{PCW textwidth \\the\\textwidth}\\typeout{PCW textheight \\the\\textheight}\n";
    // Load what the table's markup needs; loading a package twice is harmless.
    let pkgs: String = textable::needed_packages(t).iter().map(|p| format!("\\usepackage{{{p}}}\n")).collect();
    format!("{preamble}\n{pkgs}\\begin{{document}}\n{body}\\end{{document}}\n")
}

pub fn parse_log(log: &str) -> Option<Measured> {
    let mut m = Measured::default();
    for line in log.lines() {
        let Some(rest) = line.strip_prefix("PCW ") else { continue };
        let (name, val) = rest.split_once(' ')?;
        let pt: f64 = val.trim().trim_end_matches("pt").parse().ok()?;
        match name {
            "columnwidth" => m.columnwidth = pt,
            "textwidth" => m.textwidth = pt,
            "textheight" => m.textheight = pt,
            size => {
                m.widths.insert(size.to_string(), pt);
            }
        }
    }
    (m.columnwidth > 0.0 && m.widths.len() == textable::FONT_STEPS.len()).then_some(m)
}

pub async fn measure(paper: &Path, t: &TexTable) -> Result<Measured, String> {
    let tex = pdflatex().ok_or("找不到 pdflatex：請先安裝 TeX")?;
    let main = std::fs::read_to_string(pc_core::open::main_file(paper)).map_err(|_| "論文沒有主檔".to_string())?;
    let pre = preamble(&main).ok_or("主檔沒有 \\begin{document}")?;
    let dir = std::env::temp_dir().join(format!("papercomposer-measure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("probe.tex"), probe(pre, t)).map_err(|e| e.to_string())?;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let out = tokio::time::timeout(
        Duration::from_secs(40),
        crate::proc::cmd(&tex)
            .current_dir(&dir)
            // the paper folder first, then the usual search path (trailing separator)
            .env("TEXINPUTS", format!("{}{sep}", paper.display()))
            .args(["-interaction=batchmode", "-draftmode", "-no-shell-escape", "-halt-on-error", "probe.tex"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "量測逾時".to_string())?
    .map_err(|e| e.to_string())?;
    let log = std::fs::read_to_string(dir.join("probe.log")).unwrap_or_default();
    parse_log(&log).ok_or_else(|| {
        let err = log.lines().find(|l| l.starts_with('!')).unwrap_or("").to_string();
        if err.is_empty() { format!("量測失敗（pdflatex 結束碼 {:?}）", out.status.code()) } else { format!("量測失敗：{err}") }
    })
}

/// The page as the paper's class sets it, in millimetres (1 TeX pt = 25.4 / 72.27 mm).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Page {
    /// `\linewidth` inside a one-column `figure`: what `\includegraphics[width=\linewidth]` fills.
    pub column_mm: f64,
    pub text_mm: f64,
    pub height_mm: f64,
}

pub const PT_MM: f64 = 25.4 / 72.27;

/// A document that only reports the page's widths.
pub fn page_probe(preamble: &str) -> String {
    format!("{preamble}\n\\begin{{document}}\n\\typeout{{PCW columnwidth \\the\\columnwidth}}\\typeout{{PCW textwidth \\the\\textwidth}}\\typeout{{PCW textheight \\the\\textheight}}\n\\end{{document}}\n")
}

pub fn parse_page(log: &str) -> Option<Page> {
    let get = |name: &str| log.lines().find_map(|l| l.strip_prefix("PCW ")?.strip_prefix(name)?.trim().strip_suffix("pt")?.parse::<f64>().ok());
    let (c, t, h) = (get("columnwidth ")?, get("textwidth ")?, get("textheight ")?);
    let mm = |x: f64| (x * PT_MM * 10.0).round() / 10.0;
    (c > 0.0 && t > 0.0).then(|| Page { column_mm: mm(c), text_mm: mm(t), height_mm: mm(h) })
}

/// Pages measured, by preamble (a pdflatex run takes a second or two; the preamble rarely changes).
static PAGES: std::sync::Mutex<Vec<(String, Page)>> = std::sync::Mutex::new(Vec::new());

/// The paper's page widths, measured with its own class (cached per preamble).
pub async fn page(paper: &Path) -> Result<Page, String> {
    let main = std::fs::read_to_string(pc_core::open::main_file(paper)).map_err(|_| "論文沒有主檔".to_string())?;
    let pre = preamble(&main).ok_or("主檔沒有 \\begin{document}")?.to_string();
    if let Some(p) = PAGES.lock().ok().and_then(|v| v.iter().find(|(k, _)| *k == pre).map(|(_, p)| *p)) {
        return Ok(p);
    }
    let tex = pdflatex().ok_or("找不到 pdflatex：請先安裝 TeX")?;
    let dir = std::env::temp_dir().join(format!("papercomposer-page-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("page.tex"), page_probe(&pre)).map_err(|e| e.to_string())?;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let _ = tokio::time::timeout(
        Duration::from_secs(40),
        crate::proc::cmd(&tex)
            .current_dir(&dir)
            .env("TEXINPUTS", format!("{}{sep}", paper.display()))
            .args(["-interaction=batchmode", "-draftmode", "-no-shell-escape", "-halt-on-error", "page.tex"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "量測逾時".to_string())?;
    let log = std::fs::read_to_string(dir.join("page.log")).unwrap_or_default();
    let p = parse_page(&log).ok_or_else(|| log.lines().find(|l| l.starts_with('!')).map(|e| format!("量測失敗：{e}")).unwrap_or_else(|| "量測失敗".into()))?;
    if let Ok(mut v) = PAGES.lock() {
        v.push((pre, p));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_width_is_read_in_millimetres() {
        // sn-jnl: \textwidth 372pt → 130.7 mm; a two-column class gives a narrower column
        let p = parse_page("PCW columnwidth 372.0pt\nPCW textwidth 372.0pt\nPCW textheight 609.0pt\n").unwrap();
        assert_eq!((p.column_mm, p.text_mm), (130.7, 130.7));
        let two = parse_page("PCW columnwidth 252.0pt\nPCW textwidth 516.0pt\nPCW textheight 682.0pt\n").unwrap();
        assert_eq!((two.column_mm, two.text_mm), (88.6, 181.4));
        assert!(parse_page("PCW columnwidth 1pt").is_none());
        assert!(page_probe("\\documentclass{article}").contains("\\typeout{PCW textwidth \\the\\textwidth}"));
    }

    #[test]
    fn reads_widths_from_the_log() {
        let log = "x\nPCW normalsize 210.5pt\nPCW small 190.0pt\nPCW footnotesize 170.25pt\nPCW scriptsize 150.0pt\nPCW columnwidth 252.0pt\nPCW textwidth 516.0pt\nPCW textheight 682.0pt\n";
        let m = parse_log(log).unwrap();
        assert_eq!(m.widths["footnotesize"], 170.25);
        assert_eq!((m.columnwidth, m.textwidth, m.textheight), (252.0, 516.0, 682.0));
        assert!(parse_log("PCW columnwidth 1pt").is_none(), "incomplete logs are rejected");
    }
}
