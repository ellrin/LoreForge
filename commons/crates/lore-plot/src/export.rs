//! Output (docs/plot.md §6). The editor's SVG is the master drawing; headless Chrome prints it
//! to a one-page vector PDF (`@page` = the figure's mm size, fonts embedded); SVG and PNG
//! come from that PDF through poppler (`pdftocairo -svg`, `pdftoppm -r`) when it is
//! installed, else from the SVG itself (KaTeX fonts embedded) and a Chrome screenshot.
//! The spec travels inside: PNG `iTXt` chunk `lore-plot`, SVG `<metadata>`.
//! **Chrome and poppler are not part of the offline tests.**

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;

use crate::assets;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("需要 Chrome 才能輸出 PDF／PNG（這台電腦找不到）")]
    NoChrome,
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Svg,
    Png,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "pdf" => Some(Format::Pdf),
            "svg" => Some(Format::Svg),
            "png" => Some(Format::Png),
            _ => None,
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Format::Pdf => "application/pdf",
            Format::Svg => "image/svg+xml",
            Format::Png => "image/png",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }
}

/// Where the outside programs are.
#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub chrome: Option<PathBuf>,
    pub pdftocairo: Option<PathBuf>,
    pub pdftoppm: Option<PathBuf>,
}

/// Desktop apps start with a short PATH: also look where Homebrew and distributions put them.
fn which(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

impl Tools {
    /// `LORE_PLOT_CHROME` (else `SLIDECOMPOSER_CHROME`, `CHROME`) or the usual install places.
    pub fn find() -> Tools {
        let chrome = ["LORE_PLOT_CHROME", "SLIDECOMPOSER_CHROME", "CHROME"]
            .iter()
            .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
            .or_else(|| {
                [
                    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                    "/Applications/Chromium.app/Contents/MacOS/Chromium",
                    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                    "/usr/bin/google-chrome",
                    "/usr/bin/chromium",
                    "/usr/bin/chromium-browser",
                ]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.exists())
            });
        Tools { chrome, pdftocairo: which("pdftocairo"), pdftoppm: which("pdftoppm") }
    }

    /// No outside programs at all (tests): only SVG works.
    pub fn none() -> Tools {
        Tools::default()
    }
}

// ── the SVG document ────────────────────────────────────────────────────

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// KaTeX font families the SVG uses → `@font-face` rules with the woff2 files inline.
pub fn katex_font_css(svg: &str) -> String {
    let mut css = String::new();
    for (file, bytes) in assets::KATEX_FONTS {
        // KaTeX_Main-BoldItalic.woff2 → family KaTeX_Main, weight bold, style italic
        let stem = file.trim_end_matches(".woff2");
        let (family, variant) = stem.split_once('-').unwrap_or((stem, "Regular"));
        if !svg.contains(family) {
            continue;
        }
        let weight = if variant.contains("Bold") { "bold" } else { "normal" };
        let style = if variant.contains("Italic") { "italic" } else { "normal" };
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        css.push_str(&format!("@font-face{{font-family:{family};font-weight:{weight};font-style:{style};src:url(data:font/woff2;base64,{b64}) format(\"woff2\")}}"));
    }
    css
}

/// The editor's SVG as a file: the spec in `<metadata>`, the KaTeX fonts it uses inline.
pub fn svg_document(svg: &str, spec_json: Option<&str>) -> String {
    let mut svg = strip_metadata(svg.trim());
    if svg.starts_with("<?xml")
        && let Some(e) = svg.find("?>")
    {
        svg = svg[e + 2..].trim_start().to_string();
    }
    let Some(start) = svg.find("<svg") else { return svg };
    let Some(mut end) = svg[start..].find('>').map(|e| start + e) else { return svg };
    // a viewer's zoom (`style="width:…px"`) on the root never reaches a file
    if let Some(i) = svg[start..end].find(" style=\"").map(|i| start + i)
        && let Some(j) = svg[i + 8..end].find('"').map(|j| i + 8 + j)
    {
        svg.replace_range(i..=j, "");
        end -= j + 1 - i;
    }
    let mut extra = String::new();
    if let Some(j) = spec_json {
        extra.push_str(&format!("<metadata><lore-plot xmlns=\"urn:loreforge:plot\">{}</lore-plot></metadata>", xml_text(j)));
    }
    let css = katex_font_css(&svg);
    if !css.is_empty() {
        extra.push_str(&format!("<style>{css}</style>"));
    }
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}{}{}", &svg[..=end], extra, &svg[end + 1..])
}

fn strip_metadata(svg: &str) -> String {
    let (Some(a), Some(b)) = (svg.find("<metadata>"), svg.find("</metadata>")) else { return svg.to_string() };
    if b < a {
        return svg.to_string();
    }
    format!("{}{}", &svg[..a], &svg[b + "</metadata>".len()..])
}

/// The spec in an SVG written by [`svg_document`].
pub fn spec_from_svg(svg: &str) -> Option<String> {
    let a = svg.find("<lore-plot")?;
    let start = a + svg[a..].find('>')? + 1;
    let end = start + svg[start..].find("</lore-plot>")?;
    Some(svg[start..end].replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&"))
}

/// `width="85mm" height="60mm"` of the root element.
pub fn size_mm(svg: &str) -> Option<(f64, f64)> {
    let head = &svg[svg.find("<svg")?..];
    let head = &head[..head.find('>')?];
    let get = |k: &str| -> Option<f64> {
        let i = head.find(&format!(" {k}=\""))? + k.len() + 3;
        head[i..].split('"').next()?.trim_end_matches("mm").parse().ok()
    };
    Some((get("width")?, get("height")?))
}

/// One page of exactly the figure's size, for Chrome to print or screenshot.
pub fn print_html(svg_doc: &str, w_mm: f64, h_mm: f64) -> String {
    let body = svg_doc.trim_start_matches("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>@page{{size:{w_mm}mm {h_mm}mm;margin:0}}html,body{{margin:0;padding:0;width:{w_mm}mm;height:{h_mm}mm;overflow:hidden;background:transparent;-webkit-print-color-adjust:exact;print-color-adjust:exact}}svg{{display:block;width:{w_mm}mm;height:{h_mm}mm}}</style></head><body>{body}</body></html>"
    )
}

// ── PNG chunk ───────────────────────────────────────────────────────────

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
const KEYWORD: &str = "lore-plot";

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Chunks of a PNG: (type, data start, data end).
fn chunks(png: &[u8]) -> Option<Vec<([u8; 4], usize, usize)>> {
    if !png.starts_with(PNG_SIG) {
        return None;
    }
    let mut out = Vec::new();
    let mut at = PNG_SIG.len();
    while at + 12 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().ok()?) as usize;
        let kind: [u8; 4] = png[at + 4..at + 8].try_into().ok()?;
        let end = at + 8 + len;
        if end + 4 > png.len() {
            return None;
        }
        out.push((kind, at + 8, end));
        at = end + 4;
        if &kind == b"IEND" {
            break;
        }
    }
    Some(out)
}

/// The PNG with `spec` in an `iTXt` chunk (an older one replaced), before `IEND`.
pub fn png_with_spec(png: &[u8], spec_json: &str) -> Option<Vec<u8>> {
    let cs = chunks(png)?;
    let mut out = PNG_SIG.to_vec();
    let mut data = Vec::new();
    data.extend_from_slice(KEYWORD.as_bytes());
    data.extend_from_slice(&[0, 0, 0, 0, 0]); // NUL, no compression, method, empty language NUL, empty translated NUL
    data.extend_from_slice(spec_json.as_bytes());
    for (kind, a, b) in cs {
        if &kind == b"iTXt" && png[a..b].starts_with(format!("{KEYWORD}\0").as_bytes()) {
            continue;
        }
        if &kind == b"IEND" {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = b"iTXt".to_vec();
            body.extend_from_slice(&data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc32(&body).to_be_bytes());
        }
        out.extend_from_slice(&png[a - 8..b + 4]);
    }
    Some(out)
}

/// The spec a PNG carries ([`png_with_spec`]).
pub fn spec_from_png(png: &[u8]) -> Option<String> {
    for (kind, a, b) in chunks(png)? {
        let d = &png[a..b];
        if &kind == b"iTXt" && d.starts_with(format!("{KEYWORD}\0").as_bytes()) {
            let rest = &d[KEYWORD.len() + 1..];
            // compression flag, method, language\0, translated\0
            let rest = rest.get(2..)?;
            let l = rest.iter().position(|&c| c == 0)?;
            let rest = &rest[l + 1..];
            let t = rest.iter().position(|&c| c == 0)?;
            return String::from_utf8(rest[t + 1..].to_vec()).ok();
        }
    }
    None
}

/// Pixel size from the PNG header.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if !png.starts_with(PNG_SIG) || png.len() < 24 {
        return None;
    }
    Some((u32::from_be_bytes(png[16..20].try_into().ok()?), u32::from_be_bytes(png[20..24].try_into().ok()?)))
}

// ── running Chrome / poppler ────────────────────────────────────────────

fn tmp_dir() -> std::io::Result<PathBuf> {
    let d = std::env::temp_dir().join(format!(
        "lore-plot-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    ));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Run a program and wait at most `limit` (its process group killed after).
fn run(bin: &Path, args: &[String], limit: Duration) -> Result<(), ExportError> {
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| ExportError::Failed(format!("{}：{e}", bin.display())))?;
    let start = Instant::now();
    let done = loop {
        match child.try_wait()? {
            Some(st) => break Some(st),
            None if start.elapsed() >= limit => break None,
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    };
    #[cfg(unix)]
    // SAFETY: kill(2) on the process group made for this child.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    match done {
        None => Err(ExportError::Failed(format!("{} 超過 {} 秒沒有完成", bin.file_name().and_then(|n| n.to_str()).unwrap_or(""), limit.as_secs()))),
        Some(_) => Ok(()),
    }
}

fn chrome_args() -> Vec<String> {
    let mut a: Vec<String> = ["--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--use-mock-keychain", "--password-store=basic", "--allow-file-access-from-files"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Ok(extra) = std::env::var("SLIDECOMPOSER_CHROME_ARGS") {
        a.extend(extra.split_whitespace().map(str::to_string));
    }
    a
}

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/").replace(' ', "%20").replace('#', "%23");
    if s.starts_with('/') { format!("file://{s}") } else { format!("file:///{s}") }
}

/// The figure as a one-page vector PDF.
pub fn pdf(tools: &Tools, svg_doc: &str, w_mm: f64, h_mm: f64) -> Result<Vec<u8>, ExportError> {
    let chrome = tools.chrome.as_ref().ok_or(ExportError::NoChrome)?;
    let dir = tmp_dir()?;
    let html = dir.join("figure.html");
    std::fs::write(&html, print_html(svg_doc, w_mm, h_mm))?;
    let out = dir.join("figure.pdf");
    let mut args = chrome_args();
    args.extend(["--no-pdf-header-footer".to_string(), "--virtual-time-budget=8000".into(), "--run-all-compositor-stages-before-draw".into(), format!("--print-to-pdf={}", out.display()), file_url(&html)]);
    let r = run(chrome, &args, Duration::from_secs(60));
    let bytes = std::fs::read(&out);
    let _ = std::fs::remove_dir_all(&dir);
    r?;
    bytes.map_err(|_| ExportError::Failed("Chrome 沒有產生 PDF".into()))
}

/// The figure's SVG without a page open: headless Chrome runs `LorePlot.render` (the same
/// `plot.js` the editor uses, from [`assets::FILES`]) on `spec_json` with `styles` (what
/// `GET /plot/api/styles` answers as `styles`) and the DOM is read back. For jobs on the server
/// (PaperComposer's 寫書) that make figures with no editor open.
pub fn render_spec(tools: &Tools, spec_json: &str, styles: &serde_json::Value) -> Result<String, ExportError> {
    let chrome = tools.chrome.as_ref().ok_or(ExportError::NoChrome)?;
    serde_json::from_str::<serde_json::Value>(spec_json).map_err(|e| ExportError::Failed(format!("規格不是 JSON：{e}")))?;
    let dir = tmp_dir()?;
    let result = (|| {
        std::fs::create_dir_all(dir.join("katex/fonts"))?;
        for (path, _, bytes) in assets::FILES {
            if !path.ends_with(".html") {
                std::fs::write(dir.join(path), bytes)?;
            }
        }
        for (file, bytes) in assets::KATEX_FONTS {
            std::fs::write(dir.join("katex/fonts").join(file), bytes)?;
        }
        let json_of = |name: &str| assets::get(name).map(|(_, b)| String::from_utf8_lossy(b).to_string()).unwrap_or_else(|| "{}".into());
        // `</` inside a script would end it
        let safe = |s: &str| s.replace("</", "<\\/");
        let page = format!(
            "<!doctype html><meta charset=\"utf-8\"><body><pre id=\"out\"></pre>\n\
<script>window.__F={{\"palettes.json\":{pal},\"formats.json\":{fmt}}};const __f=window.fetch;window.fetch=(u,o)=>{{const n=String(u).split('/').pop();return n in window.__F?Promise.resolve({{ok:true,json:async()=>window.__F[n]}}):__f(u,o);}};</script>\n\
<script src=\"plot.js\"></script><script src=\"charts.js\"></script><script src=\"charts3d.js\"></script><script src=\"formats.js\"></script>\n\
<script>LorePlot.render({spec},{{styles:{styles}}}).then(r=>{{document.getElementById('out').textContent=r.svg;}}).catch(e=>{{document.getElementById('out').textContent='ERROR '+e;}});</script>\n",
            pal = safe(&json_of("palettes.json")),
            fmt = safe(&json_of("formats.json")),
            spec = safe(spec_json),
            styles = safe(&styles.to_string()),
        );
        let html = dir.join("render.html");
        std::fs::write(&html, page)?;
        let mut args = chrome_args();
        args.extend(["--virtual-time-budget=15000".to_string(), "--dump-dom".into(), file_url(&html)]);
        let out = run_capture(chrome, &args, Duration::from_secs(60))?;
        let text = String::from_utf8_lossy(&out);
        let inner = text.split_once("<pre id=\"out\">").and_then(|(_, r)| r.split_once("</pre>")).map(|(a, _)| a).unwrap_or("");
        let svg = inner.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", "\u{a0}").replace("&amp;", "&");
        if let Some(e) = svg.strip_prefix("ERROR ") {
            return Err(ExportError::Failed(format!("畫圖失敗：{}", e.lines().next().unwrap_or(""))));
        }
        if !svg.trim_start().starts_with("<svg") {
            return Err(ExportError::Failed("Chrome 沒有畫出圖".into()));
        }
        Ok(svg)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// [`run`] keeping what the program writes to stdout.
fn run_capture(bin: &Path, args: &[String], limit: Duration) -> Result<Vec<u8>, ExportError> {
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| ExportError::Failed(format!("{}：{e}", bin.display())))?;
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = vec![];
        if let Some(o) = stdout.as_mut() {
            let _ = std::io::Read::read_to_end(o, &mut buf);
        }
        buf
    });
    let start = Instant::now();
    let done = loop {
        match child.try_wait()? {
            Some(st) => break Some(st),
            None if start.elapsed() >= limit => break None,
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    };
    #[cfg(unix)]
    // SAFETY: kill(2) on the process group made for this child.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    let out = reader.join().unwrap_or_default();
    match done {
        None => Err(ExportError::Failed(format!("{} 超過 {} 秒沒有完成", bin.file_name().and_then(|n| n.to_str()).unwrap_or(""), limit.as_secs()))),
        Some(_) => Ok(out),
    }
}

/// PDF → SVG with glyphs as paths (poppler), or None without pdftocairo.
pub fn svg_from_pdf(tools: &Tools, pdf: &[u8]) -> Option<String> {
    let bin = tools.pdftocairo.as_ref()?;
    let dir = tmp_dir().ok()?;
    std::fs::write(dir.join("f.pdf"), pdf).ok()?;
    let out = dir.join("f.svg");
    let ok = run(bin, &["-svg".into(), dir.join("f.pdf").display().to_string(), out.display().to_string()], Duration::from_secs(30)).is_ok();
    let s = std::fs::read_to_string(&out).ok();
    let _ = std::fs::remove_dir_all(&dir);
    if ok { s } else { None }
}

/// PNG at `dpi`: from the PDF through pdftoppm, else a Chrome screenshot of the SVG.
pub fn png(tools: &Tools, svg_doc: &str, pdf: Option<&[u8]>, w_mm: f64, h_mm: f64, dpi: u32) -> Result<Vec<u8>, ExportError> {
    let dpi = dpi.clamp(72, 1200);
    let dir = tmp_dir()?;
    let result = (|| {
        if let (Some(bin), Some(pdf)) = (tools.pdftoppm.as_ref(), pdf) {
            std::fs::write(dir.join("f.pdf"), pdf)?;
            let prefix = dir.join("f");
            run(bin, &["-png".into(), "-r".into(), dpi.to_string(), "-singlefile".into(), "-aa".into(), "yes".into(), "-aaVector".into(), "yes".into(), dir.join("f.pdf").display().to_string(), prefix.display().to_string()], Duration::from_secs(60))?;
            if let Ok(b) = std::fs::read(dir.join("f.png")) {
                return Ok(b);
            }
        }
        let chrome = tools.chrome.as_ref().ok_or(ExportError::NoChrome)?;
        let html = dir.join("figure.html");
        std::fs::write(&html, print_html(svg_doc, w_mm, h_mm))?;
        let css_w = (w_mm / 25.4 * 96.0).round() as u32;
        let css_h = (h_mm / 25.4 * 96.0).round() as u32;
        let out = dir.join("shot.png");
        let mut args = chrome_args();
        args.extend([
            "--hide-scrollbars".to_string(),
            format!("--force-device-scale-factor={:.4}", dpi as f64 / 96.0),
            "--default-background-color=00000000".into(),
            "--virtual-time-budget=8000".into(),
            format!("--window-size={css_w},{css_h}"),
            format!("--screenshot={}", out.display()),
            file_url(&html),
        ]);
        run(chrome, &args, Duration::from_secs(60))?;
        std::fs::read(&out).map_err(|_| ExportError::Failed("Chrome 沒有產生圖片".into()))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// One output file, the spec inside where the format allows.
pub fn export(tools: &Tools, svg: &str, spec_json: Option<&str>, format: Format, dpi: u32) -> Result<Vec<u8>, ExportError> {
    let (w, h) = size_mm(svg).ok_or_else(|| ExportError::Failed("SVG 沒有 mm 大小".into()))?;
    let doc = svg_document(svg, spec_json);
    match format {
        Format::Svg => {
            if tools.chrome.is_some()
                && tools.pdftocairo.is_some()
                && let Ok(p) = pdf(tools, &doc, w, h)
                && let Some(s) = svg_from_pdf(tools, &p)
            {
                // poppler's SVG: glyphs as paths; the spec rides along as metadata
                return Ok(svg_document(&s, spec_json).into_bytes());
            }
            Ok(doc.into_bytes())
        }
        Format::Pdf => pdf(tools, &doc, w, h),
        Format::Png => {
            let p = if tools.pdftoppm.is_some() { pdf(tools, &doc, w, h).ok() } else { None };
            let bytes = png(tools, &doc, p.as_deref(), w, h, dpi)?;
            Ok(match spec_json {
                Some(j) => png_with_spec(&bytes, j).unwrap_or(bytes),
                None => bytes,
            })
        }
    }
}
