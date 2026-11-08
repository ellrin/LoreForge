//! Visual check: render decks to per-page grayscale bitmaps and compare.
//!
//! Renderers, in order of preference:
//! 1. Microsoft PowerPoint (osascript "save as PDF") — **not tested on real hardware**
//!    (PowerPoint is not installed on the build machine).
//! 2. Keynote (osascript export to PDF) — **not tested on real hardware**.
//! 3. `quicklook`: macOS Quick Look HTML preview (OfficeImport; shapes arrive as
//!    PDF snippets, rasterised with `pdftocairo`) printed to PDF by headless Google Chrome. A proxy renderer: fonts are substituted and some
//!    effects are approximated, but original and round-tripped decks go through
//!    the same pipeline, so differences are ours.
//! Then `pdftoppm -gray` → PGM → SSIM / mean-abs-diff per page.

use std::path::{Path, PathBuf};
use std::process::Command;

pub enum Renderer {
    PowerPoint,
    Keynote,
    QuickLook { chrome: PathBuf },
}

impl Renderer {
    pub fn detect() -> Option<Renderer> {
        let pick = std::env::var("YM_RENDERER").unwrap_or_default();
        let chrome = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        let ppt = Path::new("/Applications/Microsoft PowerPoint.app").exists();
        let key = Path::new("/Applications/Keynote.app").exists();
        let ql = Path::new("/usr/bin/qlmanage").exists() && chrome.exists();
        match pick.as_str() {
            "none" => None,
            "quicklook" if ql => Some(Renderer::QuickLook { chrome }),
            _ if ppt => Some(Renderer::PowerPoint),
            _ if key => Some(Renderer::Keynote),
            _ if ql => Some(Renderer::QuickLook { chrome }),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Renderer::PowerPoint => "PowerPoint",
            Renderer::Keynote => "Keynote",
            Renderer::QuickLook { .. } => "QuickLook+Chrome (proxy)",
        }
    }

    /// Render `pptx` into `out_dir/<stem>.pdf`.
    pub fn to_pdf(&self, pptx: &Path, out_dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        let stem = pptx.file_stem().unwrap().to_string_lossy().to_string();
        let pdf = out_dir.join(format!("{stem}.pdf"));
        let _ = std::fs::remove_file(&pdf);
        match self {
            Renderer::PowerPoint => {
                let script = format!(
                    r#"tell application "Microsoft PowerPoint"
                        set p to open POSIX file "{}"
                        save p in POSIX file "{}" as save as PDF
                        close p saving no
                    end tell"#,
                    pptx.display(),
                    pdf.display()
                );
                run(Command::new("osascript").arg("-e").arg(script))?;
            }
            Renderer::Keynote => {
                let script = format!(
                    r#"tell application "Keynote"
                        set d to open POSIX file "{}"
                        export d to POSIX file "{}" as PDF
                        close d saving no
                    end tell"#,
                    pptx.display(),
                    pdf.display()
                );
                run(Command::new("osascript").arg("-e").arg(script))?;
            }
            Renderer::QuickLook { chrome } => {
                let ql = out_dir.join(format!("{stem}.ql"));
                let _ = std::fs::remove_dir_all(&ql);
                std::fs::create_dir_all(&ql).map_err(|e| e.to_string())?;
                run(Command::new("qlmanage")
                    .arg("-p")
                    .arg("-o")
                    .arg(&ql)
                    .arg(pptx))?;
                let name = pptx.file_name().unwrap().to_string_lossy().to_string();
                let dir = ql.join(format!("{name}.qlpreview"));
                let html = std::fs::read_to_string(dir.join("Preview.html"))
                    .map_err(|e| format!("quicklook: {e}"))?;
                // slide size from the preview's own CSS
                let (w, h) = slide_css_size(&html).unwrap_or((720, 405));
                let css = format!(
                    "<style>@page{{size:{w}px {h}px;margin:0}}html,body{{margin:0!important;padding:0!important;background:#fff!important}}div.slide{{margin:0!important;box-shadow:none!important;-webkit-box-shadow:none!important;break-after:page;page-break-after:always}}</style>"
                );
                let mut html = html.replacen("</head>", &format!("{css}</head>"), 1);
                // Quick Look draws shapes as PDF attachments, which Chrome cannot show
                // in <img>: rasterise them (transparent) with pdftocairo and relink.
                for e in std::fs::read_dir(&dir)
                    .map_err(|e| e.to_string())?
                    .flatten()
                {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("pdf") {
                        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                        let png_base = dir.join(format!("{stem}-r"));
                        run(Command::new("pdftocairo")
                            .args(["-png", "-transp", "-r", "192", "-singlefile"])
                            .arg(&p)
                            .arg(&png_base))?;
                        html =
                            html.replace(&format!("\"{stem}.pdf\""), &format!("\"{stem}-r.png\""));
                    }
                }
                let print = dir.join("print.html");
                std::fs::write(&print, html).map_err(|e| e.to_string())?;
                run(Command::new(chrome)
                    .arg("--headless=new")
                    .arg("--disable-gpu")
                    .arg("--no-pdf-header-footer")
                    .arg(format!(
                        "--user-data-dir={}",
                        out_dir.join("chrome-profile").display()
                    ))
                    .arg(format!("--print-to-pdf={}", pdf.display()))
                    .arg(format!("file://{}", print.display())))?;
            }
        }
        if pdf.exists() {
            Ok(pdf)
        } else {
            Err(format!("{} produced no PDF", self.name()))
        }
    }
}

fn slide_css_size(html: &str) -> Option<(u32, u32)> {
    let i = html.find("div.slide, div.loading-slide { width:")?;
    let rest = &html[i..];
    let num = |key: &str| -> Option<u32> {
        let j = rest.find(key)? + key.len();
        rest[j..]
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    };
    Some((num("width:")?, num("height:")?))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr)
            .chars()
            .take(400)
            .collect());
    }
    Ok(())
}

pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// PDF → one PGM per page (pdftoppm -gray), sorted by page.
pub fn rasterize(pdf: &Path, out_prefix: &Path, dpi: u32) -> Result<Vec<Gray>, String> {
    run(Command::new("pdftoppm")
        .arg("-gray")
        .arg("-r")
        .arg(dpi.to_string())
        .arg(pdf)
        .arg(out_prefix))?;
    let dir = out_prefix.parent().unwrap();
    let stem = out_prefix
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.starts_with(&format!("{stem}-")) && n.ends_with(".pgm")
        })
        .collect();
    files.sort();
    files.iter().map(|f| read_pgm(f)).collect()
}

fn read_pgm(p: &Path) -> Result<Gray, String> {
    let b = std::fs::read(p).map_err(|e| e.to_string())?;
    // P5\n<w> <h>\n<max>\n<data>
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let s = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        fields.push(String::from_utf8_lossy(&b[s..i]).to_string());
    }
    i += 1;
    if fields[0] != "P5" {
        return Err(format!("{}: not P5", p.display()));
    }
    let w: usize = fields[1].parse().map_err(|_| "pgm w")?;
    let h: usize = fields[2].parse().map_err(|_| "pgm h")?;
    Ok(Gray {
        w,
        h,
        px: b[i..i + w * h].to_vec(),
    })
}

/// Mean SSIM over 8×8 windows (common area) and mean absolute difference (0–1).
pub fn compare(a: &Gray, b: &Gray) -> (f64, f64) {
    let w = a.w.min(b.w);
    let h = a.h.min(b.h);
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let mut ssim_sum = 0.0;
    let mut n = 0usize;
    let mut mad = 0.0;
    for by in (0..h.saturating_sub(7)).step_by(8) {
        for bx in (0..w.saturating_sub(7)).step_by(8) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for y in by..by + 8 {
                for x in bx..bx + 8 {
                    let va = a.px[y * a.w + x] as f64;
                    let vb = b.px[y * b.w + x] as f64;
                    sa += va;
                    sb += vb;
                    saa += va * va;
                    sbb += vb * vb;
                    sab += va * vb;
                    mad += (va - vb).abs();
                }
            }
            let k = 64.0;
            let (ma, mb) = (sa / k, sb / k);
            let va = saa / k - ma * ma;
            let vb = sbb / k - mb * mb;
            let cov = sab / k - ma * mb;
            ssim_sum += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1;
        }
    }
    if n == 0 {
        return (1.0, 0.0);
    }
    (ssim_sum / n as f64, mad / (n as f64 * 64.0 * 255.0))
}
