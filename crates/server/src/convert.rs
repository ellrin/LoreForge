//! Turn an uploaded LaTeX or Word template into a PaperComposer template,
//! with Claude Code doing the work the way a person would: read the files,
//! compile, read the log, fix, compare with the journal's PDF, repeat.
//!
//! Each conversion is a job folder in the cache:
//!   work/            Claude's working folder (its cwd); becomes the template
//!   work/_upload/    what the user uploaded, and what we prepared from it
//!   log.jsonl        Claude Code's stream-json output
//!   job.json         name, kind, status, session id
//!
//! Claude runs sandboxed: shell commands can write only inside the job and
//! cannot reach the network, only TeX tools may run, file tools are limited
//! to the job folder, and TeX itself may not open files outside it
//! (openin_any=p). The uploaded files are untrusted: comments in a template
//! can carry instructions, so nothing leaves the job folder without the user
//! saving it, and saving keeps only template file types.
//!
//! The instructions are general and public (template_convert_prompt.md); a
//! workspace may override them with its own prompts/template-convert.md.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Job {
    pub id: String,
    pub name: String,
    /// journal | conference | custom
    pub kind: String,
    /// new | running | idle | failed
    pub status: String,
    pub session: String,
    /// Has Claude been run on this job yet (follow-ups resume the session).
    #[serde(default)]
    pub started: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Running conversions, so they can be stopped and so a stale "running"
/// left by a restart reads as stopped.
static RUNNING: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

fn running(id: &str) -> bool {
    RUNNING.lock().unwrap().as_ref().is_some_and(|m| m.contains_key(id))
}

pub fn root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer/convert")
}

fn dir(id: &str) -> Option<PathBuf> {
    (id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| root().join(id))
}

pub fn work(id: &str) -> Option<PathBuf> {
    dir(id).map(|d| d.join("work"))
}

fn upload_dir(id: &str) -> Option<PathBuf> {
    work(id).map(|w| w.join("_upload"))
}

/// A random version-4 UUID (Claude Code wants one for --session-id).
fn uuid() -> String {
    let mut b = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).is_ok();
    if !filled {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b.copy_from_slice(&(t ^ (std::process::id() as u128) << 64).to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn save(job: &Job) -> Result<(), String> {
    let d = dir(&job.id).ok_or("bad job")?;
    std::fs::write(d.join("job.json"), serde_json::to_vec_pretty(job).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn load(id: &str) -> Option<Job> {
    let mut j: Job = serde_json::from_slice(&std::fs::read(dir(id)?.join("job.json")).ok()?).ok()?;
    if j.status == "running" && !running(id) {
        j.status = "idle".into();
    }
    Some(j)
}

pub fn create(name: &str, kind: &str) -> Result<Job, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("請輸入名稱".into());
    }
    let kind = if matches!(kind, "journal" | "conference" | "custom") { kind } else { "journal" };
    let id = uuid();
    let d = root().join(&id);
    for sub in ["work/_upload/source", "work/_upload/reference", "work/_upload/prep"] {
        std::fs::create_dir_all(d.join(sub)).map_err(|e| e.to_string())?;
    }
    let job = Job { id: id.clone(), name: name.into(), kind: kind.into(), status: "new".into(), session: uuid(), ..Default::default() };
    save(&job)?;
    Ok(job)
}

fn safe_file_name(n: &str) -> Option<String> {
    let n = n.rsplit(['/', '\\']).next()?.trim();
    (!n.is_empty() && !n.starts_with('.') && n.chars().all(|c| c.is_alphanumeric() || "-_. ()+".contains(c))).then(|| n.to_string())
}

/// Add an upload. Sources: archives are unpacked, single files kept as
/// named. The reference is the journal's own PDF of a typeset paper.
pub fn upload(id: &str, role: &str, filename: &str, bytes: &[u8]) -> Result<(), String> {
    let up = upload_dir(id).ok_or("bad job")?;
    if !up.is_dir() {
        return Err("找不到這次轉換".into());
    }
    let name = safe_file_name(filename).ok_or("檔名不合法")?;
    match role {
        "reference" => {
            if !bytes.starts_with(b"%PDF") {
                return Err("比對用的檔案要是 PDF".into());
            }
            std::fs::write(up.join("reference/reference.pdf"), bytes).map_err(|e| e.to_string())
        }
        "source" => {
            let lower = name.to_lowercase();
            let archive = bytes.starts_with(b"PK\x03\x04") && !lower.ends_with(".docx") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz");
            if archive {
                crate::import::unpack(bytes, &up.join("source"))?;
                lift_single_folder(&up.join("source"));
                Ok(())
            } else {
                std::fs::write(up.join("source").join(name), bytes).map_err(|e| e.to_string())
            }
        }
        _ => Err("unknown role".into()),
    }
}

/// Archives often wrap everything in one folder (`kit-main/…`): move its
/// contents up so the template's files sit at the top.
fn lift_single_folder(dir: &Path) {
    let entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).collect();
    if entries.len() != 1 || !entries[0].path().is_dir() {
        return;
    }
    let inner = entries[0].path();
    for e in std::fs::read_dir(&inner).into_iter().flatten().flatten() {
        let to = dir.join(e.file_name());
        if !to.exists() {
            let _ = std::fs::rename(e.path(), to);
        }
    }
    let _ = std::fs::remove_dir(&inner);
}

#[derive(Serialize, Debug, Default)]
pub struct Analysis {
    /// latex | word | none
    pub source: String,
    pub files: Vec<String>,
    pub main: Option<String>,
    /// Files the LaTeX source needs that are neither uploaded nor installed.
    pub missing: Vec<String>,
    pub reference: bool,
}

fn walk(d: &Path, base: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if let Ok(r) = p.strip_prefix(base) {
            out.push(r.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn kpsewhich(name: &str) -> bool {
    crate::texmeasure::tool("kpsewhich")
        .and_then(|k| crate::proc::std_cmd(k).arg(name).output().ok())
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// What a LaTeX main file needs: class, packages, inputs, figures, .bib, .bst.
pub fn needs(main: &str) -> Vec<(String, &'static [&'static str])> {
    let mut code: String = main.lines().map(|l| l.split('%').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    // examples shown in verbatim blocks are not used
    while let Some(a) = code.find("\\begin{verbatim}") {
        let b = code[a..].find("\\end{verbatim}").map(|e| a + e + "\\end{verbatim}".len()).unwrap_or(code.len());
        code.replace_range(a..b, "");
    }
    let mut v: Vec<(String, &'static [&'static str])> = Vec::new();
    let args = |cmd: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = code.as_str();
        while let Some(i) = rest.find(cmd) {
            rest = &rest[i + cmd.len()..];
            if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let mut r = rest.trim_start();
            if r.starts_with('[') {
                r = r.find(']').map(|e| &r[e + 1..]).unwrap_or(r);
            }
            if let Some(body) = r.trim_start().strip_prefix('{').and_then(|b| b.split('}').next()) {
                out.extend(body.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && !s.contains('#')));
            }
        }
        out
    };
    for c in args("\\documentclass") {
        v.push((c, &[".cls"]));
    }
    for p in [args("\\usepackage"), args("\\RequirePackage")].concat() {
        v.push((p, &[".sty"]));
    }
    for f in [args("\\input"), args("\\include")].concat() {
        v.push((f, &["", ".tex"]));
    }
    for f in args("\\includegraphics") {
        v.push((f, &["", ".pdf", ".png", ".jpg", ".jpeg", ".eps"]));
    }
    for f in args("\\bibliography") {
        v.push((f, &[".bib"]));
    }
    for f in args("\\bibliographystyle") {
        v.push((f, &[".bst"]));
    }
    v
}

pub fn analyze(id: &str) -> Option<Analysis> {
    let up = upload_dir(id)?;
    let src = up.join("source");
    let mut files = Vec::new();
    walk(&src, &src, &mut files);
    files.sort();
    let mut a = Analysis { files, reference: up.join("reference/reference.pdf").is_file(), ..Default::default() };
    if let Some(m) = pc_core::paper::find_main_tex(&src) {
        a.source = "latex".into();
        a.main = m.strip_prefix(&src).ok().map(|p| p.to_string_lossy().to_string());
        let text = std::fs::read_to_string(&m).unwrap_or_default();
        let have = |name: &str| a.files.iter().any(|f| f == name || f.ends_with(&format!("/{name}")));
        for (base, exts) in needs(&text) {
            let base_name = base.rsplit('/').next().unwrap_or(&base).to_string();
            // a name that already has its extension is looked up as written
            let exts: &[&str] = if base_name.contains('.') { &[""] } else { exts };
            let found = exts.iter().any(|e| have(&format!("{base}{e}")) || have(&format!("{base_name}{e}")))
                || (exts.contains(&".cls") || exts.contains(&".sty") || exts.contains(&".bst")) && exts.iter().any(|e| kpsewhich(&format!("{base_name}{e}")));
            if !found && !a.missing.iter().any(|m| m.starts_with(&base_name)) {
                a.missing.push(format!("{base_name}{}", exts.iter().find(|e| !e.is_empty()).unwrap_or(&"")));
            }
        }
    } else if a.files.iter().any(|f| f.to_lowercase().ends_with(".docx")) {
        a.source = "word".into();
    } else {
        a.source = "none".into();
    }
    Some(a)
}

/// What we prepare so Claude needs no extra tools: Word as Markdown plus its
/// page and style settings, the reference PDF as page images and text.
async fn prepare(id: &str) -> Result<(), String> {
    let up = upload_dir(id).ok_or("bad job")?;
    let prep = up.join("prep");
    let src = up.join("source");
    let wd = work(id).ok_or("bad job")?;
    let mut files = Vec::new();
    walk(&src, &src, &mut files);
    // a copy of the upload to work on; a Word template becomes reference.docx
    for f in &files {
        let to = if f.to_lowercase().ends_with(".docx") { wd.join("reference.docx") } else { wd.join(f) };
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src.join(f), to).map_err(|e| e.to_string())?;
    }
    if let Some(docx) = files.iter().find(|f| f.to_lowercase().ends_with(".docx")) {
        let path = src.join(docx);
        if let Some(pandoc) = crate::texmeasure::tool("pandoc") {
            let o = crate::proc::cmd(pandoc).arg(&path).args(["-t", "markdown", "--wrap=none"]).output().await.map_err(|e| e.to_string())?;
            std::fs::write(prep.join("word.md"), &o.stdout).map_err(|e| e.to_string())?;
        }
        std::fs::write(prep.join("word-format.txt"), docx_format(&path)).map_err(|e| e.to_string())?;
    }
    let reference = up.join("reference/reference.pdf");
    if reference.is_file() {
        render_pages(&reference, &prep, "reference", 3).await;
        if let Some(gs) = crate::texmeasure::tool("gs") {
            let o = crate::proc::cmd(gs)
                .args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=txtwrite", "-dFirstPage=1", "-dLastPage=3", "-sOutputFile=-"])
                .arg(&reference)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            std::fs::write(prep.join("reference.txt"), &o.stdout).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Pages 1..n of a PDF as `<stem>-<page>.png`.
pub async fn render_pages(pdf: &Path, out: &Path, stem: &str, n: u32) {
    if let Some(gs) = crate::texmeasure::tool("gs") {
        let _ = crate::proc::cmd(gs)
            .args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=png16m", "-r60", "-dFirstPage=1"])
            .arg(format!("-dLastPage={n}"))
            .arg(format!("-sOutputFile={}", out.join(format!("{stem}-%d.png")).display()))
            .arg(pdf)
            .output()
            .await;
    }
}

/// Page size, margins, columns and the main styles of a .docx, in points.
pub fn docx_format(path: &Path) -> String {
    let Ok(f) = std::fs::File::open(path) else { return String::new() };
    let Ok(mut z) = zip::ZipArchive::new(f) else { return String::new() };
    let mut read = |name: &str| -> String {
        let mut s = String::new();
        if let Ok(mut e) = z.by_name(name) {
            let _ = std::io::Read::read_to_string(&mut e, &mut s);
        }
        s
    };
    let doc = read("word/document.xml");
    let styles = read("word/styles.xml");
    let attr = |tag: &str, a: &str, src: &str| -> Option<String> {
        let i = src.find(&format!("<{tag} "))?;
        let t = &src[i..i + src[i..].find('>')?];
        let k = t.find(&format!("{a}=\""))? + a.len() + 2;
        Some(t[k..k + t[k..].find('"')?].to_string())
    };
    let pt = |twips: Option<String>| twips.and_then(|t| t.parse::<f64>().ok()).map(|t| format!("{:.1}pt", t / 20.0)).unwrap_or_else(|| "?".into());
    let mut out = String::new();
    out += &format!("page: width {} height {}\n", pt(attr("w:pgSz", "w:w", &doc)), pt(attr("w:pgSz", "w:h", &doc)));
    out += &format!(
        "margins: top {} bottom {} left {} right {}\n",
        pt(attr("w:pgMar", "w:top", &doc)), pt(attr("w:pgMar", "w:bottom", &doc)), pt(attr("w:pgMar", "w:left", &doc)), pt(attr("w:pgMar", "w:right", &doc))
    );
    out += &format!("columns: {}\n", attr("w:cols", "w:num", &doc).unwrap_or_else(|| "1".into()));
    // each style: name, font, size (half-points), bold
    for chunk in styles.split("<w:style ").skip(1) {
        let end = chunk.find("</w:style>").unwrap_or(chunk.len());
        let c = &chunk[..end];
        let Some(name) = attr("w:name", "w:val", &format!("<{c}")) else { continue };
        let keep = ["Normal", "Title", "heading 1", "heading 2", "heading 3", "caption", "Abstract", "Subtitle", "Author", "Bibliography", "Quote"];
        if !keep.iter().any(|k| name.eq_ignore_ascii_case(k)) {
            continue;
        }
        let font = attr("w:rFonts", "w:ascii", c).unwrap_or_default();
        let size = attr("w:sz", "w:val", c).and_then(|s| s.parse::<f64>().ok()).map(|s| format!("{}pt", s / 2.0)).unwrap_or_default();
        let bold = if c.contains("<w:b/>") || c.contains("<w:b ") { " bold" } else { "" };
        let align = attr("w:jc", "w:val", c).unwrap_or_default();
        out += &format!("style \"{name}\": font {font} size {size}{bold} align {align}\n");
    }
    out += &format!("default font: {}\n", attr("w:rFonts", "w:ascii", &styles).unwrap_or_default());
    out
}

/// The instructions for Claude: the public, general ones that ship with the
/// app, unless the workspace has its own `prompts/template-convert.md`.
pub fn prompt(workspace: &Path) -> String {
    std::fs::read_to_string(workspace.join("prompts/template-convert.md"))
        .ok()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string())
}

pub const DEFAULT_PROMPT: &str = include_str!("template_convert_prompt.md");

/// The task for the first run.
fn task(job: &Job, a: &Analysis) -> String {
    let kind = match job.kind.as_str() {
        "conference" => "conference",
        "custom" => "custom document (e.g. a thesis)",
        _ => "journal",
    };
    let mut t = format!("Convert the uploaded template for the {kind} \"{}\" into a PaperComposer template in the current folder.\n", job.name);
    t += &format!("Uploaded source ({}): _upload/source/ with {} files", a.source, a.files.len());
    if let Some(m) = &a.main {
        t += &format!(", main file _upload/source/{m}");
    }
    t += ".\n";
    if !a.missing.is_empty() {
        t += &format!("Not uploaded and not installed: {}. Work around them if you can and say so.\n", a.missing.join(", "));
    }
    if a.source == "word" {
        t += "Word template prepared as _upload/prep/word.md (content) and _upload/prep/word-format.txt (page, margins, columns, styles).\n";
    }
    if a.reference {
        t += "Reference PDF from the journal: _upload/prep/reference-1.png..3 (pages) and _upload/prep/reference.txt (text). Compare your output with it.\n";
    }
    t
}

/// Engine for the agent: the subscription login, or an API key.
pub enum Auth {
    Subscription,
    ApiKey(String),
}

/// Start (or continue, with `instruction`) the conversion in the background.
pub async fn run(job: &mut Job, auth: Auth, model: &str, system: &str, instruction: Option<String>) -> Result<(), String> {
    if running(&job.id) {
        return Err("正在轉換中".into());
    }
    let bin = ym_llm::claude_code::binary(None).ok_or("找不到 Claude Code：模板轉換需要先安裝並登入 Claude Code（設定 → Claude）")?;
    let d = dir(&job.id).ok_or("bad job")?;
    let wd = d.join("work");
    let a = analyze(&job.id).ok_or("bad job")?;
    if a.source == "none" {
        return Err("先上傳 LaTeX 模板（含主檔）或 Word 範本".into());
    }
    if !job.started {
        prepare(&job.id).await?;
    }
    let prompt = match (&instruction, job.started) {
        (Some(i), true) => i.clone(),
        (Some(i), false) => format!("{}\n{}", task(job, &a), i),
        (None, _) => task(job, &a),
    };
    let tex_path = crate::texmeasure::tool("pdflatex").and_then(|p| p.parent().map(Path::to_path_buf));
    let gs_path = crate::texmeasure::tool("gs").and_then(|p| p.parent().map(Path::to_path_buf));
    let path = [tex_path, gs_path].into_iter().flatten().map(|p| p.display().to_string()).chain(std::env::var("PATH").ok()).collect::<Vec<_>>().join(":");
    let mut c = crate::proc::cmd(bin);
    for v in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"] {
        c.env_remove(v);
    }
    if let Auth::ApiKey(k) = &auth {
        c.env("ANTHROPIC_API_KEY", k);
    }
    c.current_dir(&wd)
        .env("PATH", path)
        // TeX may only read and write inside the job folder (and its own trees)
        .env("openin_any", "p")
        .env("openout_any", "p")
        .arg("-p")
        .arg(&prompt)
        .args(["--output-format", "stream-json", "--verbose", "--model", model])
        .args(["--append-system-prompt", system])
        .args(["--tools", "Bash,Read,Edit,Write,Glob,Grep"])
        .arg("--allowedTools")
        .args(ALLOWED)
        .args(["--permission-mode", "dontAsk"])
        .args(["--settings", r#"{"sandbox":{"enabled":true,"autoAllowBashIfSandboxed":false,"allowUnsandboxedCommands":false}}"#]);
    if let Auth::ApiKey(_) = auth {
        c.args(["--max-budget-usd", "3"]);
    }
    if job.started {
        c.args(["--resume", &job.session]);
    } else {
        c.args(["--session-id", &job.session]);
    }
    let log = std::fs::OpenOptions::new().create(true).append(true).open(d.join("log.jsonl")).map_err(|e| e.to_string())?;
    // the user's own words go into the log too, so the conversation reads in order
    if let Some(i) = &instruction {
        let line = serde_json::json!({ "type": "pc_user", "text": i });
        std::fs::OpenOptions::new().append(true).open(d.join("log.jsonl")).and_then(|mut f| std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes())).map_err(|e| e.to_string())?;
    }
    c.stdin(Stdio::null()).stdout(Stdio::from(log)).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| format!("無法啟動 Claude Code：{e}"))?;
    let pid = child.id().unwrap_or(0);
    RUNNING.lock().unwrap().get_or_insert_with(HashMap::new).insert(job.id.clone(), pid);
    job.status = "running".into();
    job.started = true;
    job.error = None;
    save(job)?;
    let mut done = job.clone();
    tokio::spawn(async move {
        let out = child.wait_with_output().await;
        RUNNING.lock().unwrap().get_or_insert_with(HashMap::new).remove(&done.id);
        match out {
            Ok(o) if o.status.success() => done.status = "idle".into(),
            Ok(o) => {
                done.status = "failed".into();
                let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
                done.error = Some(if e.is_empty() { format!("Claude Code 結束碼 {:?}", o.status.code()) } else { e.chars().take(400).collect() });
            }
            Err(e) => {
                done.status = "failed".into();
                done.error = Some(e.to_string());
            }
        }
        let _ = save(&done);
    });
    Ok(())
}

/// Shell commands Claude may run: TeX, Ghostscript to render pages, listing.
const ALLOWED: &[&str] = &[
    "Read(./**)", "Edit(./**)", "Write(./**)", "Glob", "Grep",
    "Bash(pdflatex *)", "Bash(xelatex *)", "Bash(lualatex *)", "Bash(bibtex *)", "Bash(biber *)",
    "Bash(kpsewhich *)", "Bash(gs *)", "Bash(ls)", "Bash(ls *)",
];

pub fn stop(id: &str) {
    let pid = RUNNING.lock().unwrap().as_mut().and_then(|m| m.remove(id));
    if let Some(pid) = pid.filter(|p| *p > 0) {
        let _ = crate::proc::std_cmd("kill").arg(pid.to_string()).status();
    }
    if let Some(mut j) = load(id) {
        j.status = "idle".into();
        let _ = save(&j);
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Event {
    /// user | say | do | err | done
    pub kind: String,
    pub text: String,
}

/// A short description of a tool call.
fn describe(name: &str, input: &serde_json::Value) -> String {
    let file = |k: &str| input.get(k).and_then(|v| v.as_str()).map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).unwrap_or_default();
    match name {
        "Bash" => {
            let c = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let first = c.split_whitespace().next().unwrap_or("");
            match first {
                "pdflatex" | "xelatex" | "lualatex" => format!("編譯 {}", c.split_whitespace().last().unwrap_or("")),
                "bibtex" | "biber" => "整理參考文獻".into(),
                "gs" => "把 PDF 轉成圖片".into(),
                "kpsewhich" => format!("找套件 {}", c.split_whitespace().last().unwrap_or("")),
                _ => c.chars().take(80).collect(),
            }
        }
        "Read" => {
            let f = file("file_path");
            if f.ends_with(".png") { format!("看 {f}") } else { format!("讀 {f}") }
        }
        "Edit" => format!("修改 {}", file("file_path")),
        "Write" => format!("寫入 {}", file("file_path")),
        "Glob" | "Grep" => "搜尋檔案".into(),
        _ => name.to_string(),
    }
}

/// The log as a conversation, from line `from`.
pub fn events(id: &str, from: usize) -> (Vec<Event>, usize) {
    let Some(d) = dir(id) else { return (vec![], from) };
    let text = std::fs::read_to_string(d.join("log.jsonl")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for l in lines.iter().skip(from) {
        let Ok(e) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        match e.get("type").and_then(|t| t.as_str()) {
            Some("pc_user") => out.push(Event { kind: "user".into(), text: e["text"].as_str().unwrap_or("").into() }),
            Some("assistant") => {
                for c in e["message"]["content"].as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("text") => {
                            let t = c["text"].as_str().unwrap_or("").trim();
                            if !t.is_empty() {
                                out.push(Event { kind: "say".into(), text: t.into() });
                            }
                        }
                        Some("tool_use") => out.push(Event { kind: "do".into(), text: describe(c["name"].as_str().unwrap_or(""), &c["input"]) }),
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for c in e["message"]["content"].as_array().into_iter().flatten() {
                    if c["type"] == "tool_result" && c["is_error"].as_bool() == Some(true) {
                        let t = c["content"].as_str().map(String::from).unwrap_or_else(|| c["content"].to_string());
                        let first = t.lines().find(|l| l.starts_with('!') || l.contains("Error")).or_else(|| t.lines().next()).unwrap_or("");
                        out.push(Event { kind: "err".into(), text: first.chars().take(160).collect() });
                    }
                }
            }
            Some("result") => {
                let cost = e["total_cost_usd"].as_f64().map(|c| format!(" · US${c:.2}")).unwrap_or_default();
                let turns = e["num_turns"].as_u64().map(|n| format!("{n} 步")).unwrap_or_default();
                out.push(Event { kind: "done".into(), text: format!("{turns}{cost}") });
            }
            _ => {}
        }
    }
    (out, lines.len())
}

/// Files that may become part of a template.
const KEEP_EXT: &[&str] = &["tex", "cls", "sty", "bst", "bib", "clo", "cfg", "def", "fd", "png", "jpg", "jpeg", "pdf", "eps", "docx", "csl", "md", "toml", "txt"];

/// The files of `work/` that would be saved: template types, no build output,
/// nothing from _upload, nothing over 20 MB.
pub fn keepable(id: &str) -> Vec<String> {
    let Some(w) = work(id) else { return vec![] };
    let mut files = Vec::new();
    walk(&w, &w, &mut files);
    files.retain(|f| {
        let lower = f.to_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        !f.starts_with("_upload/")
            && !f.contains("/.") && !f.starts_with('.')
            && KEEP_EXT.contains(&ext)
            && !matches!(lower.as_str(), "sample.pdf" | "reference.pdf")
            // a PDF next to a .tex of the same name is that file's output
            && !(ext == "pdf" && w.join(format!("{}.tex", &f[..f.len() - 4])).is_file())
            && !lower.starts_with("output-") && !lower.starts_with("out-")
            && std::fs::metadata(w.join(f)).is_ok_and(|m| m.len() < 20_000_000)
    });
    files.sort();
    files
}

/// Copy the kept files into publishers/<new folder> with template.toml.
pub fn save_template(id: &str, publishers: &Path) -> Result<String, String> {
    let job = load(id).ok_or("找不到這次轉換")?;
    let w = work(id).ok_or("bad job")?;
    if !w.join("sample.tex").is_file() {
        return Err("還沒有 sample.tex：請先讓 Claude 完成轉換".into());
    }
    let files = keepable(id);
    let mut meta = pc_core::journal::Meta { name: job.name.clone(), kind: job.kind.clone(), ..Default::default() };
    if let Ok(t) = std::fs::read_to_string(w.join("template.toml")) {
        if let Ok(m) = toml::from_str::<pc_core::journal::Meta>(&t) {
            meta = pc_core::journal::Meta { name: job.name.clone(), kind: job.kind.clone(), ..m };
        }
    }
    let stem = pc_core::journal::slug(&meta);
    let mut tid = stem.clone();
    let mut n = 2;
    while publishers.join(&tid).exists() {
        tid = format!("{stem}-{n}");
        n += 1;
    }
    let dest = publishers.join(&tid);
    for f in &files {
        let to = dest.join(f);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::copy(w.join(f), to).map_err(|e| e.to_string())?;
    }
    let t = toml::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    std::fs::write(dest.join("template.toml"), t).map_err(|e| e.to_string())?;
    Ok(tid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifts_a_wrapping_folder() {
        let d = std::env::temp_dir().join(format!("pc-lift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("kit-main/sec")).unwrap();
        std::fs::write(d.join("kit-main/main.tex"), "x").unwrap();
        std::fs::write(d.join("kit-main/sec/a.tex"), "y").unwrap();
        lift_single_folder(&d);
        assert!(d.join("main.tex").is_file() && d.join("sec/a.tex").is_file() && !d.join("kit-main").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lists_what_a_main_file_needs() {
        let m = "\\begin{verbatim}\\usepackage{shown}\\end{verbatim}\n\\documentclass[journal]{ieeecolor}\n\\usepackage{tmi,amsmath}\n% \\usepackage{skip}\n\\includegraphics[width=1in]{logo}\n\\bibliographystyle{IEEEtran}\n\\bibliography{refs}\n";
        let n: Vec<String> = needs(m).into_iter().map(|(b, e)| format!("{b}{}", e.first().unwrap())).collect();
        assert_eq!(n, ["ieeecolor.cls", "tmi.sty", "amsmath.sty", "logo", "refs.bib", "IEEEtran.bst"]);
    }

    #[test]
    fn reads_claude_code_stream() {
        let id = uuid();
        let d = root().join(&id);
        std::fs::create_dir_all(&d).unwrap();
        let log = [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"I'll compile first."},{"type":"tool_use","name":"Bash","input":{"command":"pdflatex -interaction=nonstopmode sample.tex"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"This is pdfTeX\n! Undefined control sequence."}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x/_upload/prep/reference-1.png"}}]}}"#,
            r#"{"type":"pc_user","text":"use two columns"}"#,
            r#"{"type":"result","num_turns":5,"total_cost_usd":0.034}"#,
        ]
        .join("\n");
        std::fs::write(d.join("log.jsonl"), log).unwrap();
        let (ev, next) = events(&id, 0);
        let k: Vec<(&str, &str)> = ev.iter().map(|e| (e.kind.as_str(), e.text.as_str())).collect();
        assert_eq!(k, [("say", "I'll compile first."), ("do", "編譯 sample.tex"), ("err", "! Undefined control sequence."), ("do", "看 reference-1.png"), ("user", "use two columns"), ("done", "5 步 · US$0.03")]);
        assert_eq!(next, 6);
        assert_eq!(events(&id, 6).0, vec![]);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(uuid().len(), 36);
    }
}
