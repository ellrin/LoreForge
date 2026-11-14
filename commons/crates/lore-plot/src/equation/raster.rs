//! Frames (SVG) → pixels, all in one go: headless Chrome prints every frame as one page of a PDF
//! and poppler rasterises the pages (`pdftoppm -r <dpi>`), so the GIF looks like the PDF; without
//! poppler, Chrome screenshots the frames stacked (a few per shot) and they are cut apart.
//! **Chrome and poppler are not part of the offline tests** (the encoder is, with synthetic frames).

use std::time::Duration;

use super::gif::{Rgba, decode_png};
use crate::export::{self, ExportError, Tools};

/// GIFs are kept at most this wide by default (slides; a GIF is not a print file).
pub const MAX_W_PX: u32 = 1200;

/// The resolution used for a figure `w_mm` wide: `dpi`, lowered so the width stays ≤ `max_w_px`.
pub fn dpi_for(w_mm: f64, dpi: u32, max_w_px: u32) -> u32 {
    let cap = (max_w_px as f64 * 25.4 / w_mm.max(1.0)).floor() as u32;
    dpi.clamp(36, 600).min(cap.max(36))
}

fn pages_html(docs: &[String], w: f64, h: f64) -> String {
    let body: String = docs
        .iter()
        .map(|d| format!("<div class=\"pg\">{}</div>", d.trim_start_matches("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")))
        .collect();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>@page{{size:{w}mm {h}mm;margin:0}}html,body{{margin:0;padding:0;background:#fff;-webkit-print-color-adjust:exact;print-color-adjust:exact}}.pg{{width:{w}mm;height:{h}mm;overflow:hidden;break-after:page;page-break-after:always}}.pg:last-child{{break-after:auto;page-break-after:auto}}.pg svg{{display:block;width:{w}mm;height:{h}mm}}</style></head><body>{body}</body></html>"
    )
}

/// Every frame at `dpi` (capped by `max_w_px`), on white.
pub fn rasterize(tools: &Tools, svgs: &[String], dpi: u32, max_w_px: u32) -> Result<Vec<Rgba>, ExportError> {
    let first = svgs.first().ok_or_else(|| ExportError::Failed("沒有任何一格".into()))?;
    let (w, h) = export::size_mm(first).ok_or_else(|| ExportError::Failed("SVG 沒有 mm 大小".into()))?;
    let dpi = dpi_for(w, dpi, max_w_px);
    let chrome = tools.chrome.as_ref().ok_or(ExportError::NoChrome)?;
    let docs: Vec<String> = svgs.iter().map(|s| export::svg_document(s, None)).collect();
    let dir = export::tmp_dir()?;
    let result = (|| {
        if let Some(pdftoppm) = tools.pdftoppm.as_ref() {
            let html = dir.join("frames.html");
            std::fs::write(&html, pages_html(&docs, w, h))?;
            let pdf = dir.join("frames.pdf");
            let mut args = export::chrome_args();
            args.extend(["--no-pdf-header-footer".to_string(), "--virtual-time-budget=15000".into(), "--run-all-compositor-stages-before-draw".into(), format!("--print-to-pdf={}", pdf.display()), export::file_url(&html)]);
            export::run(chrome, &args, Duration::from_secs(120))?;
            if pdf.is_file() {
                let prefix = dir.join("f");
                export::run(pdftoppm, &["-png".into(), "-r".into(), dpi.to_string(), "-aa".into(), "yes".into(), "-aaVector".into(), "yes".into(), pdf.display().to_string(), prefix.display().to_string()], Duration::from_secs(120))?;
                let mut pngs: Vec<_> = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("f-") && n.ends_with(".png"))).collect();
                // f-1.png … f-12.png or f-01.png …: by the number
                pngs.sort_by_key(|p| p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.rsplit('-').next()).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0));
                if pngs.len() == svgs.len() {
                    return pngs.iter().map(|p| decode_png(&std::fs::read(p)?).map_err(ExportError::Failed)).collect();
                }
            }
        }
        // stacked screenshots, a few frames per shot (Chrome caps the window height)
        let css_w = (w / 25.4 * 96.0).round() as u32;
        let css_h = (h / 25.4 * 96.0).round() as u32;
        let scale = dpi as f64 / 96.0;
        let per = ((8000.0 / (css_h as f64 * scale)).floor() as usize).clamp(1, 24);
        let mut out = Vec::new();
        for (k, chunk) in docs.chunks(per).enumerate() {
            let html = dir.join(format!("stack{k}.html"));
            let body: String = chunk.iter().map(|d| format!("<div style=\"width:{css_w}px;height:{css_h}px;overflow:hidden\">{}</div>", d.trim_start_matches("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"))).collect();
            std::fs::write(&html, format!("<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;padding:0;background:#fff}}svg{{display:block;width:{css_w}px;height:{css_h}px}}</style></head><body>{body}</body></html>"))?;
            let shot = dir.join(format!("stack{k}.png"));
            let mut args = export::chrome_args();
            args.extend([
                "--hide-scrollbars".to_string(),
                format!("--force-device-scale-factor={scale:.4}"),
                "--virtual-time-budget=10000".into(),
                format!("--window-size={css_w},{}", css_h as usize * chunk.len()),
                format!("--screenshot={}", shot.display()),
                export::file_url(&html),
            ]);
            export::run(chrome, &args, Duration::from_secs(120))?;
            let img = decode_png(&std::fs::read(&shot).map_err(|_| ExportError::Failed("Chrome 沒有產生圖片".into()))?).map_err(ExportError::Failed)?;
            let fh = img.h / chunk.len() as u32;
            for i in 0..chunk.len() as u32 {
                let a = (i * fh * img.w * 4) as usize;
                let b = ((i + 1) * fh * img.w * 4) as usize;
                out.push(Rgba { w: img.w, h: fh, px: img.px[a..b].to_vec() });
            }
        }
        Ok(out)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// SVG frames → the GIF (spec inside).
pub fn gif_from_svgs(tools: &Tools, svgs: &[String], dpi: u32, max_w_px: u32, delay_ms: u32, spec_json: Option<&str>) -> Result<Vec<u8>, ExportError> {
    let frames = rasterize(tools, svgs, dpi, max_w_px)?;
    super::gif::encode(&frames, delay_ms, spec_json).map_err(ExportError::Failed)
}

/// SVG frames → the GIF with per-frame delays (tier-1 sweeps paced by change, a hold at the end).
pub fn gif_from_svgs_with(tools: &Tools, svgs: &[String], dpi: u32, max_w_px: u32, delay_ms: u32, spec_json: Option<&str>, opts: &super::gif::Opts) -> Result<Vec<u8>, ExportError> {
    let frames = rasterize(tools, svgs, dpi, max_w_px)?;
    super::gif::encode_with(&frames, delay_ms, spec_json, opts).map_err(ExportError::Failed)
}
