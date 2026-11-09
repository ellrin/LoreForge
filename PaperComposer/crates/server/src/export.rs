//! Export a paper: compile main.tex to PDF with the journal's own class, or
//! convert it to Word with Pandoc. Intermediate files go to the user's cache
//! folder; only the finished file lands in `papers/<name>/export/`, which
//! carries its own .gitignore so exports never reach the repository.

use crate::texmeasure::tool;
use pc_core::paper;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Problem {
    /// Path relative to the paper folder, when TeX reported one.
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    /// The source text TeX stopped at (the `l.12 ...` line).
    pub context: Option<String>,
    /// What to do about it, when we know.
    pub hint: Option<String>,
    /// Which paragraph of the section file the line falls in.
    pub para: Option<usize>,
}

#[derive(Serialize, Debug, Default)]
pub struct Report {
    pub ok: bool,
    /// The finished file, relative to the paper folder.
    pub file: Option<String>,
    pub pages: Option<u32>,
    pub errors: Vec<Problem>,
    pub overfull: u32,
    pub undefined_cites: Vec<String>,
    pub undefined_refs: Vec<String>,
    pub warnings: Vec<String>,
    pub seconds: f64,
}

/// What the log of one LaTeX run says.
#[derive(Debug, Default, PartialEq)]
pub struct LogInfo {
    pub pages: Option<u32>,
    pub errors: Vec<Problem>,
    pub overfull: u32,
    pub undefined_cites: Vec<String>,
    pub undefined_refs: Vec<String>,
    pub rerun: bool,
}

/// `% !TEX program = xelatex`, or a package that needs a Unicode engine.
pub fn engine_for(main: &str) -> &'static str {
    for line in main.lines().take(20) {
        let l = line.trim().to_ascii_lowercase();
        if let Some(p) = l.strip_prefix("%").map(str::trim).and_then(|l| l.strip_prefix("!tex")).and_then(|l| l.split('=').nth(1)) {
            match p.trim() {
                "xelatex" => return "xelatex",
                "lualatex" => return "lualatex",
                "pdflatex" => return "pdflatex",
                _ => {}
            }
        }
    }
    let code: String = main.lines().map(|l| l.split('%').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    if ["{fontspec}", "{xeCJK}", "{ctex}", "{unicode-math}"].iter().any(|p| code.contains(p)) || code.contains("{ctexart}") {
        "xelatex"
    } else {
        "pdflatex"
    }
}

/// The bibliography program main.tex needs, if any.
pub fn bib_tool(main: &str) -> Option<&'static str> {
    let code: String = main.lines().map(|l| l.split('%').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    if code.contains("{biblatex}") {
        return Some(if code.contains("backend=bibtex") { "bibtex" } else { "biber" });
    }
    code.contains("\\bibliography{").then_some("bibtex")
}

fn between<'a>(s: &'a str, open: &str, close: char) -> Option<&'a str> {
    let i = s.find(open)? + open.len();
    let j = s[i..].find(close)?;
    Some(&s[i..i + j])
}

fn hint_for(msg: &str) -> Option<String> {
    if let Some(f) = between(msg, "File `", '\'').filter(|f| f.ends_with(".sty") || f.ends_with(".cls")) {
        let pkg = f.trim_end_matches(".sty").trim_end_matches(".cls");
        return Some(format!("缺少 {f}：sudo tlmgr install {pkg}"));
    }
    // "Font OT1/phv/m/n/24=phvr7t at 24.0pt not loadable: Metric (TFM) file not found."
    if msg.contains("not loadable") && msg.contains("TFM") {
        let tfm = msg.split_once('=').map(|(_, r)| r.split_whitespace().next().unwrap_or("")).unwrap_or("").to_string();
        let search = format!("tlmgr search --global --file /{tfm}.tfm");
        return Some(match font_package(&tfm) {
            Some(pkg) => format!("缺少字型 {tfm}：sudo tlmgr install {pkg}（不確定時用 {search} 找出套件）"),
            None if !tfm.is_empty() => format!("缺少字型 {tfm}：用 {search} 找出它屬於哪個套件，再 sudo tlmgr install 該套件"),
            None => "缺少字型：用 tlmgr search --global --file 找出它屬於哪個套件再安裝".into(),
        });
    }
    // fontspec (XeLaTeX / LuaLaTeX): a system font the computer does not have
    if let Some(f) = between(msg, "The font \"", '"') {
        return Some(format!("這台電腦沒有安裝字型「{f}」：安裝它，或在 main.tex 換成已安裝的字型"));
    }
    if msg.contains("Undefined control sequence") {
        return Some("指令拼錯，或少載入它的套件".into());
    }
    if msg.contains("Missing $ inserted") {
        return Some("數學符號要放在 $…$ 裡".into());
    }
    if msg.contains("Misplaced alignment tab character &") {
        return Some("表格外的 & 要寫成 \\&".into());
    }
    None
}

/// The TeX Live package of a PostScript font by its Karl Berry name prefix
/// (`phvr7t` → helvetic); BasicTeX leaves most of them out.
fn font_package(tfm: &str) -> Option<&'static str> {
    const FONTS: &[(&str, &str)] = &[
        ("phv", "helvetic"),
        ("ptm", "times"),
        ("pcr", "courier"),
        ("ppl", "palatino"),
        ("pbk", "bookman"),
        ("pag", "avantgar"),
        ("pnc", "ncntrsbk"),
        ("pzc", "zapfchan"),
        ("psy", "symbol"),
        ("pzd", "zapfding"),
        ("ec", "ec"),
        ("tc", "ec"),
        ("sf", "cm-super"),
    ];
    FONTS.iter().find(|(p, _)| tfm.starts_with(p)).map(|(_, k)| *k)
}

/// Read a log written with -file-line-error and max_print_line large enough
/// that lines are not wrapped.
pub fn parse_log(log: &str) -> LogInfo {
    let mut info = LogInfo::default();
    let lines: Vec<&str> = log.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // "./sections/02-method.tex:14: Undefined control sequence."
        let located = line.split_once(".tex:").and_then(|(f, rest)| {
            let (n, msg) = rest.split_once(": ")?;
            let n: u32 = n.parse().ok()?;
            Some((format!("{f}.tex"), n, msg))
        });
        let (file, ln, msg) = match located.filter(|(_, _, m)| !m.trim_start().starts_with("==>")) {
            Some((f, n, m)) if !f.contains(' ') => (Some(f.trim_start_matches("./").to_string()), Some(n), m.to_string()),
            _ => match line.strip_prefix("! ") {
                Some(m) => (None, None, m.to_string()),
                None => {
                    scan_warning(line, &mut info);
                    continue;
                }
            },
        };
        let context = lines[i + 1..].iter().take(12).find(|l| l.starts_with("l.")).map(|l| {
            let rest = l.trim_start_matches("l.");
            rest.split_once(' ').map(|(_, t)| t.trim().to_string()).unwrap_or_default()
        });
        let line_no = ln.or_else(|| {
            lines[i + 1..].iter().take(12).find(|l| l.starts_with("l.")).and_then(|l| l[2..].split(' ').next()?.parse().ok())
        });
        let hint = hint_for(&msg);
        info.errors.push(Problem { file, line: line_no, message: msg, context, hint, para: None });
    }
    // "Emergency stop" only repeats that an earlier error halted the run.
    if info.errors.len() > 1 {
        info.errors.retain(|e| !e.message.starts_with("Emergency stop"));
    }
    info
}

fn scan_warning(line: &str, info: &mut LogInfo) {
    if line.starts_with("Overfull \\hbox") {
        info.overfull += 1;
    } else if let Some(k) = line.contains("Warning: Citation").then(|| between(line, "Citation `", '\'')).flatten() {
        for k in k.split(',').map(str::trim) {
            if !info.undefined_cites.iter().any(|x| x == k) {
                info.undefined_cites.push(k.to_string());
            }
        }
    } else if let Some(k) = (line.contains("Warning: Reference") && line.contains("undefined")).then(|| between(line, "Reference `", '\'')).flatten() {
        if !info.undefined_refs.iter().any(|x| x == k) {
            info.undefined_refs.push(k.to_string());
        }
    } else if let Some(n) = line.contains("Output written on").then(|| between(line, " (", ' ')).flatten() {
        info.pages = n.parse().ok();
    }
    if line.contains("Rerun to get") || line.contains("Label(s) may have changed") || line.contains("undefined references") {
        info.rerun = true;
    }
}

/// Which paragraph of a section file holds `line` (1-based). Paragraphs are
/// separated by blank lines after the \section line, as `parse_section` reads them.
pub fn para_at(file: &str, text: &str, line: u32) -> Option<usize> {
    let target = text.lines().nth(line.checked_sub(1)? as usize)?.trim();
    if target.is_empty() {
        return None;
    }
    let sec = paper::parse_section(file, text);
    sec.paragraphs.iter().position(|p| p.lines().any(|l| l.trim() == target))
        .or_else(|| sec.paragraphs.iter().position(|p| p.contains(target)))
}

pub fn cache_dir(name: &str) -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer/build").join(name)
}

/// `papers/<name>/export/`, ignored by git through its own .gitignore.
pub fn export_dir(paper: &Path) -> std::io::Result<PathBuf> {
    let d = paper.join("export");
    std::fs::create_dir_all(&d)?;
    let ignore = d.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "# Exports are rebuilt from the sources.\n*\n")?;
    }
    Ok(d)
}

async fn run(cmd: &mut Command, secs: u64) -> Result<std::process::Output, String> {
    tokio::time::timeout(Duration::from_secs(secs), cmd.kill_on_drop(true).output())
        .await
        .map_err(|_| "編譯逾時".to_string())?
        .map_err(|e| e.to_string())
}

fn search_path(paper: &Path) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    // the paper folder, then the usual search path (trailing separator)
    format!("{}{sep}", paper.display())
}

/// Mirror the paper's sub-folders in the build folder: TeX cannot create
/// folders, and \include writes an .aux next to each included file.
fn mirror_dirs(src: &Path, dst: &Path) {
    let Ok(rd) = std::fs::read_dir(src) else { return };
    for e in rd.flatten() {
        let n = e.file_name();
        let n = n.to_string_lossy();
        if e.path().is_dir() && !n.starts_with('.') && n != "export" {
            let d = dst.join(&*n);
            let _ = std::fs::create_dir_all(&d);
            mirror_dirs(&e.path(), &d);
        }
    }
}

/// Fill in which paragraph each located error points at.
fn locate(paper: &Path, errors: &mut [Problem]) {
    for e in errors {
        if let (Some(f), Some(n)) = (e.file.as_ref().filter(|f| f.starts_with("sections/")), e.line) {
            if let Ok(text) = std::fs::read_to_string(paper.join(f)) {
                let name = Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                e.para = para_at(&name, &text, n);
            }
        }
    }
}

/// Compile `main.tex` to `export/<name>.pdf`: engine, bibliography, then
/// engine again until cross-references settle (at most four runs).
pub async fn build_pdf(paper: &Path, name: &str, prebuilt_bbl: bool) -> Result<Report, String> {
    let start = Instant::now();
    let main = std::fs::read_to_string(paper.join("main.tex")).map_err(|_| "論文沒有 main.tex".to_string())?;
    let engine = engine_for(&main);
    let tex = tool(engine).ok_or_else(|| format!("找不到 {engine}：請先安裝 TeX"))?;
    let out = cache_dir(name);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    mirror_dirs(paper, &out);
    let _ = std::fs::remove_file(out.join("main.pdf"));
    if prebuilt_bbl {
        std::fs::copy(paper.join("main.bbl"), out.join("main.bbl")).map_err(|e| e.to_string())?;
    }
    let mut report = Report::default();
    // fontspec fonts (XeLaTeX / LuaLaTeX): substitutes in copies, or stop here
    let fonts = if engine == "pdflatex" { None } else { Some(check_fonts(paper, &out).await?) };
    if let Some(f) = &fonts {
        report.warnings.extend(f.prep.notes.iter().cloned());
        if !f.prep.missing.is_empty() {
            report.errors = f
                .prep
                .missing
                .iter()
                .map(|(file, line, msg)| Problem {
                    file: Some(file.clone()),
                    line: Some(*line),
                    message: msg.clone(),
                    hint: Some("安裝其中一個字型、把字型檔放進 private/shared/fonts/，或在 private/shared/fonts.toml 的 [fallback] 為它加一條規則".into()),
                    ..Default::default()
                })
                .collect();
            report.seconds = (start.elapsed().as_secs_f64() * 10.0).round() / 10.0;
            return Ok(report);
        }
    }
    let sep = if cfg!(windows) { ";" } else { ":" };
    let main_arg = fonts.as_ref().and_then(|f| f.prep.main.clone()).unwrap_or_else(|| PathBuf::from("main.tex"));
    let latex = |out: &Path| {
        let mut c = crate::proc::cmd(&tex);
        if let Some(f) = &fonts {
            if let Some(dir) = &f.private_dir {
                // trailing separator: then the usual font path
                c.env("OSFONTDIR", format!("{}{sep}", dir.display()));
            }
            if let Some(copies) = &f.prep.copies {
                // the rewritten copies first, then the paper folder and TeX's own path
                c.env("TEXINPUTS", format!("{}{sep}", copies.display()));
            }
        }
        c.current_dir(paper)
            .env("max_print_line", "100000")
            .env("error_line", "254")
            .env("half_error_line", "238")
            // TeX's default restricted shell escape: lets epstopdf convert
            // .eps figures, nothing else
            .args(["-interaction=nonstopmode", "-halt-on-error", "-file-line-error"])
            .arg(format!("-output-directory={}", out.display()))
            .arg(&main_arg);
        c
    };
    let mut info = LogInfo::default();
    for pass in 0..4 {
        let o = run(&mut latex(&out), 120).await?;
        let log = std::fs::read_to_string(out.join("main.log")).unwrap_or_default();
        info = parse_log(&log);
        if !o.status.success() || !info.errors.is_empty() {
            if info.errors.is_empty() {
                info.errors.push(Problem { message: format!("{engine} 失敗（結束碼 {:?}）", o.status.code()), ..Default::default() });
            }
            break;
        }
        if pass == 0 {
            if let Some(b) = bib_tool(&main).filter(|_| !prebuilt_bbl) {
                report.warnings.extend(run_bib(b, paper, &out).await?);
                continue;
            }
        }
        if !info.rerun {
            break;
        }
    }
    report.seconds = (start.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    report.pages = info.pages;
    report.overfull = info.overfull;
    report.undefined_cites = info.undefined_cites;
    report.undefined_refs = info.undefined_refs;
    report.errors = info.errors;
    // errors in a rewritten copy point at the author's file (same lines)
    if let Some(copies) = fonts.as_ref().and_then(|f| f.prep.copies.as_ref()) {
        let prefix = format!("{}/", copies.display());
        for e in &mut report.errors {
            if let Some(rel) = e.file.as_deref().and_then(|f| f.strip_prefix(&prefix)) {
                e.file = Some(rel.to_string());
            }
        }
    }
    locate(paper, &mut report.errors);
    if report.errors.is_empty() {
        let dest = export_dir(paper).map_err(|e| e.to_string())?.join(format!("{name}.pdf"));
        std::fs::copy(out.join("main.pdf"), &dest).map_err(|_| "編譯完成但沒有產生 PDF".to_string())?;
        report.ok = true;
        report.file = Some(format!("export/{name}.pdf"));
    }
    Ok(report)
}

struct FontCheck {
    prep: crate::texfonts::Prepared,
    private_dir: Option<PathBuf>,
}

/// Resolve the fonts the paper names (`lore-fonts`, the author's rules in
/// private/shared/fonts.toml) and write copies with substitutes when needed.
async fn check_fonts(paper: &Path, out: &Path) -> Result<FontCheck, String> {
    let (paper, out) = (paper.to_path_buf(), out.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let home = crate::loreforge_home();
        let private_dir = home.shared_file("fonts").filter(|p| p.is_dir());
        let table = lore_fonts::Table::with_user(&home.prefs().fonts.fallback);
        let installed = crate::texfonts::installed(private_dir.as_deref());
        let prep = crate::texfonts::prepare(&paper, &out, &table, installed, private_dir.as_deref()).map_err(|e| e.to_string())?;
        Ok(FontCheck { prep, private_dir })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run BibTeX or Biber in the build folder; returns its warnings.
async fn run_bib(prog: &str, paper: &Path, out: &Path) -> Result<Vec<String>, String> {
    let exe = tool(prog).ok_or_else(|| format!("找不到 {prog}：sudo tlmgr install {prog}"))?;
    let mut c = crate::proc::cmd(exe);
    c.current_dir(out).env("BIBINPUTS", search_path(paper)).env("BSTINPUTS", search_path(paper));
    if prog == "biber" {
        c.arg(format!("--input-directory={}", paper.display()));
    }
    let o = run(c.arg("main"), 60).await?;
    let log = std::fs::read_to_string(out.join("main.blg")).unwrap_or_default();
    let mut warn = bib_warnings(&log);
    if !o.status.success() && warn.is_empty() {
        warn.push(format!("{prog} 失敗"));
    }
    Ok(warn)
}

/// The useful lines of a .blg file, with where the problem is.
pub fn bib_warnings(blg: &str) -> Vec<String> {
    let mut v = Vec::new();
    let lines: Vec<&str> = blg.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        // "Repeated entry---line 494 of file references.bib", then " : @article{Key"
        let at = l.split_once("---line ").and_then(|(what, rest)| {
            let (n, file) = rest.split_once(" of file ")?;
            Some((what, n.trim(), file.trim()))
        });
        let msg = if let Some(f) = l.strip_prefix("I couldn't open style file ") {
            format!("缺少 {f}：tlmgr search --global --file /{f}")
        } else if let Some((what, n, file)) = at {
            let key = lines.get(i + 1).and_then(|x| x.split_once('{')).map(|(_, k)| k.trim().trim_end_matches(',').to_string()).filter(|k| !k.is_empty());
            let what = if what.starts_with("Repeated entry") {
                format!("重複的條目{}", key.map(|k| format!(" {k}")).unwrap_or_default())
            } else if what.starts_with("I was expecting") {
                "格式錯誤（可能少了逗號或括號）".to_string()
            } else {
                what.to_string()
            };
            format!("{file} 第 {n} 行：{what}")
        } else if l.contains("I couldn't open") || l.contains("I found no") || l.starts_with("ERROR") {
            l.trim().to_string()
        } else {
            continue;
        };
        if !v.contains(&msg) {
            v.push(msg);
        }
    }
    v
}

/// Convert to `export/<name>.docx` with Pandoc. A `reference.docx` in the
/// paper folder (copied from the publisher template) sets the Word styles,
/// and a `.csl` file there sets the citation style.
/// Math from packages Pandoc does not know, as what it does (only for
/// commands the paper does not define itself; Pandoc keeps the first definition).
const PANDOC_COMPAT: &[(&str, &str)] = &[
    ("\\Tilde", "[1]{\\tilde{#1}}"),
    ("\\mathbbmss", "[1]{\\mathbb{#1}}"),
    ("\\mathbbm", "[1]{\\mathbb{#1}}"),
    ("\\mathds", "[1]{\\mathbb{#1}}"),
    ("\\bm", "[1]{\\boldsymbol{#1}}"),
    ("\\Checkmark", "{\\checkmark}"),
    ("\\XSolid", "{\\times}"),
    ("\\degree", "{^\\circ}"),
    // subfigure / subfig: Pandoc drops these and the images inside them
    ("\\subfigure", "[2][]{#2}"),
    ("\\subfloat", "[2][]{#2}"),
];

pub async fn build_docx(paper: &Path, name: &str, bibs: &[String]) -> Result<Report, String> {
    let start = Instant::now();
    let pandoc = tool("pandoc").ok_or("pandoc-missing")?;
    let dest = export_dir(paper).map_err(|e| e.to_string())?.join(format!("{name}.docx"));
    let work = cache_dir(name).join("docx");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let defined = paper::paper_macros(paper);
    let compat: String = PANDOC_COMPAT
        .iter()
        .filter(|(m, _)| !defined.contains_key(*m))
        .map(|(m, d)| format!("\\providecommand{{{m}}}{d}\n"))
        .collect();
    let compat_file = work.join("compat.tex");
    std::fs::write(&compat_file, compat).map_err(|e| e.to_string())?;
    let filter = work.join("figures.lua");
    std::fs::write(&filter, figure_filter(paper).await).map_err(|e| e.to_string())?;
    let bibs: Vec<PathBuf> = bibs.iter().map(|b| paper.join(b)).filter(|b| b.is_file()).collect();
    let pandoc_run = |bibs: &[PathBuf]| {
        let mut c = crate::proc::cmd(&pandoc);
        c.current_dir(paper).arg(&compat_file).args(["main.tex", "--from=latex", "--to=docx", "--resource-path=."]).arg("-o").arg(&dest);
        c.arg(format!("--lua-filter={}", filter.display()));
        if !bibs.is_empty() {
            c.arg("--citeproc");
            for b in bibs {
                c.arg(format!("--bibliography={}", b.display()));
            }
            if let Some(csl) = first_with_ext(paper, "csl") {
                c.arg(format!("--csl={csl}"));
            }
        }
        if paper.join("reference.docx").is_file() {
            c.arg("--reference-doc=reference.docx");
        }
        c
    };
    let mut o = run(&mut pandoc_run(&bibs), 120).await?;
    // BibTeX shrugs off a missing comma; Pandoc refuses the whole file. Retry
    // with copies read by our own parser (the author's files stay as they are).
    if !o.status.success() && String::from_utf8_lossy(&o.stderr).contains("Error reading bibliography") {
        let mut clean = Vec::new();
        for b in &bibs {
            let src = String::from_utf8_lossy(&std::fs::read(b).map_err(|e| e.to_string())?).to_string();
            let out = work.join(b.file_name().unwrap_or_default());
            std::fs::write(&out, pc_core::bib::write(&pc_core::bib::parse(&src).0)).map_err(|e| e.to_string())?;
            clean.push(out);
        }
        o = run(&mut pandoc_run(&clean), 120).await?;
    }
    let stderr = String::from_utf8_lossy(&o.stderr);
    let mut report = Report { seconds: (start.elapsed().as_secs_f64() * 10.0).round() / 10.0, ..Default::default() };
    report.warnings = pandoc_warnings(&stderr, &mut report.undefined_cites);
    if o.status.success() {
        report.ok = true;
        report.file = Some(format!("export/{name}.docx"));
    } else {
        report.errors.push(Problem { message: report.warnings.first().cloned().unwrap_or_else(|| "pandoc 失敗".into()), ..Default::default() });
    }
    Ok(report)
}

/// Files of the paper folder that are the app's, not the paper's.
const APP_FILES: &[&str] = &["citations.csv", "main.bbl", "main.tex", "00README.json"];

/// `export/<name>-latex.zip`: a source package that compiles on its own,
/// as journals and arXiv want it: main.tex with the sections merged back
/// in, the compiled .bbl, and the class, style, .bib and figure files.
/// Everything sits at the top level, because many submission systems
/// (Editorial Manager among them) do not accept subfolders: paths in
/// main.tex are rewritten to match. Run after a successful build_pdf, so
/// the .bbl is current.
pub fn build_latex_zip(paper: &Path, name: &str) -> Result<String, String> {
    use std::io::Write;
    let main = std::fs::read_to_string(paper.join("main.tex")).map_err(|e| e.to_string())?;
    let merged = paper::flatten(paper, paper, &main, &mut Vec::new(), 0);

    // Files to ship, top folder first, each with a name at the top level.
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut flat_of: std::collections::HashMap<PathBuf, String> = Default::default();
    let mut queue = std::collections::VecDeque::from([(paper.to_path_buf(), String::new())]);
    while let Some((dir, prefix)) = queue.pop_front() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().collect();
        entries.sort_by_key(|e| (e.path().is_dir(), e.file_name()));
        for e in entries {
            let n = e.file_name().to_string_lossy().to_string();
            let top = prefix.is_empty();
            if n.starts_with('.') || (top && (n == "export" || n == "sections" || APP_FILES.contains(&n.as_str())))
                || paper::BUILD_EXT.iter().any(|x| n.ends_with(&format!(".{x}")))
            {
                continue;
            }
            let rel = format!("{prefix}{n}");
            if e.path().is_dir() {
                queue.push_back((e.path(), format!("{rel}/")));
                continue;
            }
            let bytes = std::fs::read(e.path()).map_err(|e| e.to_string())?;
            // the same name at the top: an identical copy is dropped, another file is renamed
            let flat = match files.iter().find(|(_, f)| *f == n) {
                None => n.clone(),
                Some((p, f)) if std::fs::read(p).is_ok_and(|b| b == bytes) => {
                    flat_of.insert(e.path(), f.clone());
                    continue;
                }
                Some(_) => rel.replace('/', "-"),
            };
            flat_of.insert(e.path(), flat.clone());
            files.push((e.path(), flat));
        }
    }
    let tex = flat_paths(&merged, paper, &flat_of);

    let rel = format!("export/{name}-latex.zip");
    let dest = export_dir(paper).map_err(|e| e.to_string())?.join(format!("{name}-latex.zip"));
    let file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let err = |e: zip::result::ZipError| e.to_string();
    zip.start_file("main.tex", opts).map_err(err)?;
    zip.write_all(tex.as_bytes()).map_err(|e| e.to_string())?;
    let bbl = cache_dir(name).join("main.bbl");
    let bbl = if bbl.is_file() { bbl } else { paper.join("main.bbl") };
    if let Ok(b) = std::fs::read(&bbl) {
        zip.start_file("main.bbl", opts).map_err(err)?;
        zip.write_all(&b).map_err(|e| e.to_string())?;
    }
    for (path, flat) in files {
        zip.start_file(flat, opts).map_err(err)?;
        zip.write_all(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(err)?;
    Ok(rel)
}

/// Point every file reference in `tex` at its top-level name: figures are
/// found the way LaTeX finds them; other paths keep their last part.
/// `\graphicspath` is dropped, since nothing is in a folder any more.
pub fn flat_paths(tex: &str, paper: &Path, flat_of: &std::collections::HashMap<PathBuf, String>) -> String {
    const CMDS: &[&str] = &["\\includegraphics", "\\bibliography", "\\bibliographystyle", "\\addbibresource", "\\usepackage", "\\input", "\\include", "\\graphicspath"];
    let mut out = String::with_capacity(tex.len());
    let mut i = 0;
    while i < tex.len() {
        let Some(at) = tex[i..].find('\\').map(|k| i + k) else { break };
        out.push_str(&tex[i..at]);
        let rest = &tex[at..];
        let Some(cmd) = CMDS.iter().find(|c| rest.starts_with(**c) && !rest[c.len()..].starts_with(|ch: char| ch.is_ascii_alphabetic())) else {
            out.push('\\');
            i = at + 1;
            continue;
        };
        // \cmd*[opts]{args}
        let mut j = at + cmd.len();
        let b = tex.as_bytes();
        while j < b.len() && (b[j] == b'*' || b[j] == b' ') {
            j += 1;
        }
        if j < b.len() && b[j] == b'[' {
            if let Some(e) = tex[j..].find(']') {
                j += e + 1;
            }
        }
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let Some(end) = (j < b.len() && b[j] == b'{').then(|| group_close(tex, j)).flatten() else {
            out.push_str(&tex[at..j]);
            i = j;
            continue;
        };
        if *cmd == "\\graphicspath" {
            i = end + 1;
            continue;
        }
        let arg = &tex[j + 1..end];
        let new: Vec<String> = arg
            .split(',')
            .map(|a| {
                let t = a.trim();
                if *cmd == "\\includegraphics" {
                    if let Some(f) = crate::preview::resolve_figure(paper, t).and_then(|p| flat_of.get(&p)) {
                        return f.clone();
                    }
                }
                if t.contains('/') { t.rsplit('/').next().unwrap_or(t).to_string() } else { a.to_string() }
            })
            .collect();
        out.push_str(&tex[at..=j]);
        out.push_str(&new.join(","));
        out.push('}');
        i = end + 1;
    }
    if i < tex.len() {
        out.push_str(&tex[i..]);
    }
    out
}

/// Index of the `}` closing the group that opens at `open`.
fn group_close(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0, open);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Pandoc's stderr in a few readable lines. Its multi-line messages (the
/// source excerpt with a caret) are folded into one count per kind.
pub fn pandoc_warnings(stderr: &str, cites: &mut Vec<String>) -> Vec<String> {
    let (mut math, mut v) = (0, Vec::new());
    for l in stderr.lines() {
        // continuation lines of the previous message
        if l.starts_with(' ') || l.trim().is_empty() {
            continue;
        }
        let l = l.trim_start_matches("[WARNING] ");
        if let Some(k) = l.strip_prefix("Citeproc: citation ").and_then(|r| r.split(' ').next()) {
            if !cites.iter().any(|c| c == k) {
                cites.push(k.to_string());
            }
        } else if l.starts_with("Could not convert TeX math") {
            math += 1;
        } else if let Some(r) = l.strip_prefix("Could not fetch resource ") {
            v.push(format!("找不到圖片 {}", r.split(':').next().unwrap_or(r).trim()));
        } else if !l.contains("rendering as TeX") && !v.iter().any(|x| x == l) && v.len() < 12 {
            v.push(l.to_string());
        }
    }
    if math > 0 {
        v.insert(0, format!("{math} 個方程式無法轉成 Word 公式，保留 LaTeX 原文"));
    }
    v
}

/// A Pandoc filter that points every figure at an image Word can show:
/// found the way LaTeX finds it (graphicspath, missing extension), PDF and
/// EPS figures converted to PNG.
async fn figure_filter(paper: &Path) -> String {
    let mut tex = std::fs::read_to_string(paper.join("main.tex")).unwrap_or_default();
    for f in paper::section_files(paper) {
        tex += &std::fs::read_to_string(paper.join("sections").join(f)).unwrap_or_default();
    }
    let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer/figures");
    let mut map = String::new();
    let mut seen = Vec::new();
    for part in tex.split("\\includegraphics").skip(1) {
        let rest = part.trim_start();
        let rest = if rest.starts_with('[') { rest.find(']').map(|i| &rest[i + 1..]).unwrap_or(rest) } else { rest };
        let Some(arg) = rest.trim_start().strip_prefix('{').and_then(|r| r.split('}').next()) else { continue };
        let arg = arg.trim();
        if seen.iter().any(|x: &String| x == arg) {
            continue;
        }
        seen.push(arg.to_string());
        let Some(file) = crate::preview::resolve_figure(paper, arg) else { continue };
        let Some((img, _)) = crate::preview::figure_image(&file, &cache).await else { continue };
        let lua = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        map += &format!("  [\"{}\"] = \"{}\",\n", lua(arg), lua(&img.to_string_lossy()));
    }
    format!("local map = {{\n{map}}}\nfunction Image(el)\n  local to = map[el.src]\n  if to then el.src = to end\n  return el\nend\n")
}

fn first_with_ext(dir: &Path, ext: &str) -> Option<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).ok()?.flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(&format!(".{ext}")))
        .collect();
    v.sort();
    v.into_iter().next()
}

/// Open a finished export, or show it in the file manager.
pub fn open(path: &Path, reveal: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = crate::proc::std_cmd("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = crate::proc::std_cmd("explorer");
        if reveal { c.arg(format!("/select,{}", path.display())); } else { c.arg(path); }
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = crate::proc::std_cmd("xdg-open");
        c.arg(if reveal { path.parent().unwrap_or(path) } else { path });
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("無法開啟：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_engine_and_bibliography_program() {
        assert_eq!(engine_for("\\documentclass{IEEEtran}\n\\begin{document}"), "pdflatex");
        assert_eq!(engine_for("% !TEX program = XeLaTeX\n\\documentclass{article}"), "xelatex");
        assert_eq!(engine_for("\\usepackage{xeCJK}"), "xelatex");
        assert_eq!(engine_for("% \\usepackage{fontspec}"), "pdflatex", "commented out");
        assert_eq!(bib_tool("\\bibliographystyle{IEEEtran}\n\\bibliography{refs}"), Some("bibtex"));
        assert_eq!(bib_tool("\\usepackage[backend=biber]{biblatex}"), Some("biber"));
        assert_eq!(bib_tool("\\usepackage[backend=bibtex]{biblatex}"), Some("bibtex"));
        assert_eq!(bib_tool("% \\bibliography{refs}"), None);
    }

    #[test]
    fn reads_errors_and_warnings_from_the_log() {
        let log = "\
(./main.tex
./sections/02-method.tex:14: Undefined control sequence.
l.14 We use \\foo
                 to measure.
./main.tex:3: LaTeX Error: File `nicetable.sty' not found.
./main.tex:3:  ==> Fatal error occurred, no output PDF file produced!
Overfull \\hbox (3.2pt too wide) in paragraph at lines 5--9
Overfull \\hbox (1.0pt too wide) in paragraph at lines 12--13
LaTeX Warning: Citation `smith2020' on page 1 undefined on input line 7.
Package natbib Warning: Citation `smith2020' on page 2 undefined on input line 9.
LaTeX Warning: Reference `fig:x' on page 1 undefined on input line 4.
LaTeX Warning: There were undefined references.
Output written on /tmp/b/main.pdf (5 pages, 12345 bytes).
";
        let i = parse_log(log);
        assert_eq!(i.errors.len(), 2);
        assert_eq!(i.errors[0].file.as_deref(), Some("sections/02-method.tex"));
        assert_eq!(i.errors[0].line, Some(14));
        assert_eq!(i.errors[0].context.as_deref(), Some("We use \\foo"));
        assert!(i.errors[1].hint.as_deref().unwrap().contains("tlmgr install nicetable"));
        assert_eq!((i.overfull, i.pages, i.rerun), (2, Some(5), true));
        assert_eq!(i.undefined_cites, ["smith2020"]);
        assert_eq!(i.undefined_refs, ["fig:x"]);
        let font = parse_log("! Font OT1/phv/m/n/24=phvr7t at 24.0pt not loadable: Metric (TFM) file not found.\n<to be read again>\nl.74 \\maketitle\n");
        let h = font.errors[0].hint.as_deref().unwrap();
        assert!(h.contains("sudo tlmgr install helvetic") && h.contains("tlmgr search --global --file /phvr7t.tfm"), "{h}");
        let bare = parse_log("! Emergency stop.\n<*> main.tex\n");
        assert_eq!(bare.errors[0].message, "Emergency stop.");
        assert_eq!(bare.errors[0].file, None);
    }

    #[test]
    fn finds_the_paragraph_of_a_line() {
        let text = "\\section{Method}\n\nFirst paragraph\nstill first.\n\nSecond with \\foo here.\n";
        assert_eq!(para_at("02-method.tex", text, 6), Some(1));
        assert_eq!(para_at("02-method.tex", text, 4), Some(0));
        assert_eq!(para_at("02-method.tex", text, 2), None);
    }

    #[test]
    fn rewrites_paths_for_a_flat_package() {
        let d = std::env::temp_dir().join(format!("pc-flat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("figs")).unwrap();
        std::fs::write(d.join("main.tex"), "\\graphicspath{{figs/}}\n").unwrap();
        std::fs::write(d.join("figs/a.png"), "x").unwrap();
        let mut map = std::collections::HashMap::new();
        map.insert(d.join("figs/a.png"), "figs-a.png".to_string());
        let tex = "\\graphicspath{{figs/}}\n\\includegraphics[width=3cm]{a}\n\\bibliography{bib/refs,other}\n\\usepackage{sty/my}\n\\includegraphicsx{keep/me}\n";
        assert_eq!(
            flat_paths(tex, &d, &map),
            "\n\\includegraphics[width=3cm]{figs-a.png}\n\\bibliography{refs,other}\n\\usepackage{my}\n\\includegraphicsx{keep/me}\n"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn folds_pandoc_warnings() {
        let err = "[WARNING] Could not convert TeX math \\underset{x}\\mathop{y}, rendering as TeX:\n  ^{x}\n       ^\n[WARNING] Could not convert TeX math a, rendering as TeX:\n[WARNING] Could not fetch resource sim_9: replacing image with description\n[WARNING] Citeproc: citation smith2020 not found\n";
        let mut cites = vec![];
        assert_eq!(pandoc_warnings(err, &mut cites), ["2 個方程式無法轉成 Word 公式，保留 LaTeX 原文", "找不到圖片 sim_9"]);
        assert_eq!(cites, ["smith2020"]);
    }

    #[test]
    fn reads_bibtex_warnings() {
        let blg = "Warning--I didn't find a database entry for \"nokey\"\nWarning--I didn't find a database entry for \"nokey\"\nI couldn't open database file refs.bib\n";
        assert_eq!(bib_warnings(blg), ["I couldn't open database file refs.bib"]);
        assert_eq!(bib_warnings("I couldn't open style file IEEEtran.bst\n"), ["缺少 IEEEtran.bst：tlmgr search --global --file /IEEEtran.bst"]);
        let blg = "Repeated entry---line 494 of file references.bib\n : @article{Tropp2007Signal\n :                         ,\nI was expecting a `,' or a `}'---line 512 of file references.bib\n";
        assert_eq!(bib_warnings(blg), ["references.bib 第 494 行：重複的條目 Tropp2007Signal", "references.bib 第 512 行：格式錯誤（可能少了逗號或括號）"]);
    }
}
