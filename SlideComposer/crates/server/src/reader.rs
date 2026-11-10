//! 論文: material from LoreReader's papers for a paper-report deck — page crops as
//! pictures, the paper's symbols and equations, the author's highlights and notes, and a
//! `Ref:` line. LoreReader is read, never written, and need not be running:
//!
//! - metadata: `private/LoreReader/papers/<sha256>/{paper,annotations,quotes,symbols}.json`
//!   (`LOREREADER_WORKSPACE`, else lore-home's `private/LoreReader`; never seeded from here);
//! - the PDF: LoreReader's library `<library>/<sha[..2]>/<sha>.pdf`, where `<library>` =
//!   `LOREREADER_LIBRARY` → `library = "…"` in `<LOREREADER_HOME or OS config dir/LoreReader>/config.toml`
//!   → `~/Documents/LoreReader/library` (the same order as `lr_core::config`).
//!
//! Pages are rendered with poppler's `pdftoppm` (the tool lore-docsrc uses), cached in this
//! workspace's `cache/lorereader/`. A figure is the **whole page** stored once as a deck
//! asset; the picture's `crop` selects the region, so the crop can be changed later (here or
//! with the canvas crop handles) and several crops of one page share one file.
//! `<deck>/lorereader.json` remembers which assets are paper pages (for re-cropping).
//! Symbols and equations are registered into the deck's own `notation.json` only.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/reader/papers | `{available, papers: [{sha, title, pages, pdf, counts…}]}` |
//! | GET | /api/reader/papers/{sha} | meta, `ref`, annotations, quotes, symbols, equations |
//! | GET | /api/reader/papers/{sha}/page/{n}?dpi= | page PNG (thumbnails / crop view) |
//! | POST | /api/decks/{name}/reader/picture | `{sha, page}` → `{asset, w, h}` page at 300 dpi as a deck asset |
//! | GET | /api/decks/{name}/reader/pictures | `{pictures: {asset: {sha, page, title}}}` |
//! | POST | /api/decks/{name}/reader/notation | `{sha, ids}` → `{added, renamed, skipped}` into the deck's notation.json |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_notation::model::{Entry, Notation};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, R, Shared, bad, session};

/// Resolution of an inserted page (a figure keeps print quality when enlarged).
pub const PICTURE_DPI: u32 = 300;
const SIDECAR: &str = "lorereader.json";

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/reader/papers", get(papers))
        .route("/api/reader/papers/{sha}", get(paper))
        .route("/api/reader/papers/{sha}/page/{n}", get(page_png))
        .route("/api/decks/{name}/reader/picture", post(picture))
        .route("/api/decks/{name}/reader/pictures", get(pictures))
        .route("/api/decks/{name}/reader/notation", post(register))
}

// ── Where LoreReader keeps things ────────────────────────────────────────────

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn user_home() -> Option<PathBuf> {
    env("HOME").or_else(|| env("USERPROFILE")).map(PathBuf::from)
}

fn expand_home(p: &str, home: Option<&Path>) -> PathBuf {
    match (p.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(p),
    }
}

/// The OS settings folder (what `dirs::config_dir` returns).
fn config_dir(home: Option<&Path>) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA").map(PathBuf::from)
    } else {
        env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))
    }
}

/// `library = "…"` at the top level of LoreReader's config.toml (before any `[table]`).
pub fn config_library(toml: &str) -> Option<String> {
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            break;
        }
        let Some(rest) = l.strip_prefix("library") else { continue };
        let Some(v) = rest.trim_start().strip_prefix('=') else { continue };
        let v = v.trim();
        let q = v.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = v[1..].find(q)?;
        return Some(v[1..1 + end].to_string()).filter(|s| !s.is_empty());
    }
    None
}

/// LoreReader's PDF library, resolved like `lr_core::config::Config::load`.
/// `getenv` and `home` are parameters so tests can pass their own.
pub fn library_dir(getenv: &dyn Fn(&str) -> Option<String>, home: Option<&Path>) -> PathBuf {
    if let Some(l) = getenv("LOREREADER_LIBRARY") {
        return expand_home(&l, home);
    }
    let app = getenv("LOREREADER_HOME").map(PathBuf::from).or_else(|| config_dir(home).map(|d| d.join("LoreReader")));
    if let Some(l) = app.and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok()).and_then(|t| config_library(&t)) {
        return expand_home(&l, home);
    }
    home.map(|h| h.join("Documents")).unwrap_or_else(|| PathBuf::from("Documents")).join("LoreReader/library")
}

#[cfg(test)]
static TEST_LIBRARY: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// LoreReader's folders as this server sees them (read-only).
#[derive(Debug, Clone)]
pub struct Reader {
    /// `private/LoreReader/papers`
    pub papers: PathBuf,
    /// The PDF library.
    pub library: PathBuf,
}

impl Reader {
    pub fn discover(home: &lore_home::Home) -> Option<Reader> {
        let ws = env("LOREREADER_WORKSPACE").map(PathBuf::from).or_else(|| home.app_dir("LoreReader"))?;
        #[cfg(test)]
        if let Some(l) = TEST_LIBRARY.lock().unwrap().clone() {
            return Some(Reader { papers: ws.join("papers"), library: l });
        }
        let library = library_dir(&env, user_home().as_deref());
        Some(Reader { papers: ws.join("papers"), library })
    }

    fn folder(&self, sha: &str) -> Option<PathBuf> {
        valid_sha(sha).then(|| self.papers.join(sha))
    }

    fn json(&self, sha: &str, file: &str) -> Option<Value> {
        let b = std::fs::read(self.folder(sha)?.join(file)).ok()?;
        serde_json::from_slice(&b).ok()
    }

    pub fn pdf(&self, sha: &str) -> Option<PathBuf> {
        if !valid_sha(sha) {
            return None;
        }
        Some(self.library.join(&sha[..2]).join(format!("{sha}.pdf"))).filter(|p| p.is_file())
    }

    /// paper.json of every paper, newest first.
    pub fn list(&self) -> Vec<Value> {
        let mut out: Vec<Value> = std::fs::read_dir(&self.papers)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let sha = e.file_name().to_string_lossy().to_string();
                self.json(&sha, "paper.json").filter(|m| m.get("sha256").and_then(Value::as_str) == Some(sha.as_str()))
            })
            .collect();
        out.sort_by(|a, b| b["added_at"].as_str().unwrap_or("").cmp(a["added_at"].as_str().unwrap_or("")));
        out
    }

    fn list_in(&self, sha: &str, file: &str, key: &str) -> Vec<Value> {
        self.json(sha, file).and_then(|v| v.get(key).and_then(Value::as_array).cloned()).unwrap_or_default()
    }
}

pub fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn reader(s: &Shared) -> Result<Reader, ApiError> {
    Reader::discover(&s.home).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "找不到 LoreReader 的資料夾".into()))
}

// ── What the panel shows ─────────────────────────────────────────────────────

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

/// `authors` as a list or a string; "A", "A and B", "A et al.".
fn authors_short(m: &Value) -> String {
    let list: Vec<String> = match m.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
        Some(Value::String(x)) => x.split([';', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
        _ => vec![],
    };
    match list.len() {
        0 => String::new(),
        1 => list[0].clone(),
        2 => format!("{} and {}", list[0], list[1]),
        _ => format!("{} et al.", list[0]),
    }
}

fn title_of(m: &Value) -> String {
    let t = s(m, "title");
    if t.is_empty() { s(m, "original_name").trim_end_matches(".pdf").to_string() } else { t }
}

/// `Ref: Authors, "Title," Venue, Year. doi:…` — every part only when paper.json has it.
pub fn ref_line(m: &Value) -> String {
    let mut parts = vec![];
    let a = authors_short(m);
    if !a.is_empty() {
        parts.push(a);
    }
    parts.push(title_of(m));
    let venue = s(m, "venue");
    if !venue.is_empty() {
        parts.push(venue);
    }
    let year = match m.get("year") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(x)) => x.trim().to_string(),
        _ => String::new(),
    };
    if !year.is_empty() {
        parts.push(year);
    }
    let mut line = format!("Ref: {}.", parts.join(", ").trim_end_matches('.'));
    let doi = s(m, "doi");
    if !doi.is_empty() {
        line += &format!(" doi:{doi}");
    }
    line
}

fn summary(r: &Reader, m: &Value) -> Value {
    let sha = s(m, "sha256");
    let sym = r.json(&sha, "symbols.json");
    let n = |v: &Option<Value>, k: &str| v.as_ref().and_then(|v| v.get(k)).and_then(Value::as_array).map_or(0, Vec::len);
    json!({
        "sha": sha,
        "title": title_of(m),
        "pages": m.get("pages").and_then(Value::as_u64).unwrap_or(0),
        "doi": m.get("doi").cloned().unwrap_or(Value::Null),
        "added_at": s(m, "added_at"),
        "pdf": r.pdf(&sha).is_some(),
        "annotations": r.list_in(&sha, "annotations.json", "annotations").len(),
        "quotes": r.list_in(&sha, "quotes.json", "quotes").len(),
        "symbols": n(&sym, "entries"),
        "equations": n(&sym, "equations"),
    })
}

async fn papers(State(st): State<Shared>) -> R {
    let Some(r) = Reader::discover(&st.home) else {
        return Ok(Json(json!({ "available": false, "papers": [] })));
    };
    let list: Vec<Value> = r.list().iter().map(|m| summary(&r, m)).collect();
    Ok(Json(json!({ "available": r.papers.is_dir(), "papers": list })))
}

/// One paper's material, trimmed to what the panel uses.
pub fn detail(r: &Reader, sha: &str) -> Option<Value> {
    let m = r.json(sha, "paper.json")?;
    let notes: Vec<Value> = r
        .list_in(sha, "annotations.json", "annotations")
        .iter()
        .map(|a| json!({ "id": a["id"], "page": a["page"], "kind": a["kind"], "color": a["color"], "text": clean(&s(&a["quote"], "exact")), "note": s(a, "note") }))
        .filter(|a| !a["text"].as_str().unwrap_or("").is_empty() || !a["note"].as_str().unwrap_or("").is_empty())
        .collect();
    let quotes: Vec<Value> = r
        .list_in(sha, "quotes.json", "quotes")
        .iter()
        .map(|q| json!({ "id": q["id"], "page": q["page"], "text": clean(&s(q, "text")), "note": s(q, "note") }))
        .filter(|q| !q["text"].as_str().unwrap_or("").is_empty())
        .collect();
    let sym = r.json(sha, "symbols.json").unwrap_or(json!({}));
    let symbols: Vec<Value> = sym["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| !s(e, "latex").is_empty())
        .map(|e| {
            let page = e["first"]["page"].as_u64().or_else(|| e["occurrences"][0]["page"].as_u64()).or_else(|| e["evidence"][0]["page"].as_u64());
            json!({ "id": e["id"], "kind": e["kind"], "latex": e["latex"], "meaning": e["meaning"], "meaning_en": e["meaning_en"], "grade": e["grade"], "page": page })
        })
        .collect();
    let equations: Vec<Value> = sym["equations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| json!({ "id": e["id"], "latex": e["latex"], "number": e["number"], "page": e["page"], "bbox": e["bbox"] }))
        .collect();
    Some(json!({
        "paper": summary(r, &m),
        "ref": ref_line(&m),
        "notes": notes,
        "quotes": quotes,
        "symbols": symbols,
        "equations": equations,
    }))
}

/// PDF text as one line: whitespace collapsed, a word broken at a line end joined again
/// ("de- fers" → "defers"; "state-of-the-art" untouched).
pub fn clean(t: &str) -> String {
    let words: Vec<&str> = t.split_whitespace().collect();
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            let prev = words[i - 1];
            let broken = prev.len() > 2 && prev.ends_with('-') && prev[..prev.len() - 1].chars().last().is_some_and(char::is_lowercase) && w.chars().next().is_some_and(char::is_lowercase);
            if broken {
                // "state-of-the- art": a compound keeps its hyphen
                if !prev[..prev.len() - 1].contains('-') {
                    out.pop();
                }
            } else {
                out.push(' ');
            }
        }
        out.push_str(w);
    }
    out
}

async fn paper(State(st): State<Shared>, UrlPath(sha): UrlPath<String>) -> R {
    let r = reader(&st)?;
    detail(&r, &sha).map(Json).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "LoreReader 沒有這篇論文".into()))
}

// ── Page rendering (poppler) ─────────────────────────────────────────────────

/// `pdftoppm`: `LORE_POPPLER_DIR` → /opt/homebrew/bin → /usr/local/bin → /usr/bin → PATH
/// (the order lore-docsrc uses).
pub fn pdftoppm() -> Option<PathBuf> {
    let name = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
    let mut dirs: Vec<PathBuf> = env("LORE_POPPLER_DIR").map(PathBuf::from).into_iter().collect();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Width and height of a PNG (IHDR).
pub fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?)))
}

/// Page `n` (1-based) of `pdf` as PNG at `dpi`, cached as `cache/<sha>/p<n>-<dpi>.png`.
pub fn render_page(tool: &Path, pdf: &Path, sha: &str, n: u32, dpi: u32, cache: &Path) -> Result<Vec<u8>, String> {
    let dir = cache.join(sha);
    let out = dir.join(format!("p{n}-{dpi}.png"));
    if let Ok(b) = std::fs::read(&out) {
        return Ok(b);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let prefix = dir.join(format!(".p{n}-{dpi}-{}", std::process::id()));
    let o = Command::new(tool)
        .args(["-f", &n.to_string(), "-l", &n.to_string(), "-r", &dpi.to_string(), "-png", "-singlefile"])
        .arg(pdf)
        .arg(&prefix)
        .output()
        .map_err(|e| format!("pdftoppm：{e}"))?;
    let tmp = prefix.with_extension("png");
    let tmp = if tmp.is_file() { tmp } else { PathBuf::from(format!("{}.png", prefix.display())) };
    if !o.status.success() || !tmp.is_file() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("第 {n} 頁無法轉成圖片：{}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    std::fs::read(&out).map_err(|e| e.to_string())
}

fn cache_dir(st: &Shared) -> PathBuf {
    st.ws.root.join("cache/lorereader")
}

async fn render(st: &Shared, sha: &str, n: u32, dpi: u32) -> Result<Vec<u8>, ApiError> {
    let r = reader(st)?;
    let pdf = r.pdf(sha).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "LoreReader 的書庫裡沒有這篇的 PDF".into()))?;
    let tool = pdftoppm().ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "需要 poppler 的 pdftoppm 才能顯示論文頁面（brew install poppler）".into()))?;
    if n == 0 {
        return Err(bad("頁碼從 1 開始"));
    }
    let (sha, cache) = (sha.to_string(), cache_dir(st));
    tokio::task::spawn_blocking(move || render_page(&tool, &pdf, &sha, n, dpi, &cache))
        .await
        .map_err(|e| bad(e.to_string()))?
        .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e))
}

#[derive(Deserialize)]
struct DpiQ {
    #[serde(default)]
    dpi: Option<u32>,
}

async fn page_png(State(st): State<Shared>, UrlPath((sha, n)): UrlPath<(String, String)>, Query(q): Query<DpiQ>) -> Result<Response, ApiError> {
    let n: u32 = n.trim_end_matches(".png").parse().map_err(|_| bad("頁碼"))?;
    // a few fixed steps keep the cache small
    let dpi = match q.dpi.unwrap_or(72) {
        0..=40 => 36,
        41..=80 => 72,
        81..=130 => 120,
        _ => 180,
    };
    let b = render(&st, &sha, n, dpi).await?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "max-age=3600")], b).into_response())
}

// ── Into the deck ────────────────────────────────────────────────────────────

fn sidecar(dir: &Path) -> Value {
    std::fs::read(dir.join(SIDECAR)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({ "schema": "slidecomposer.lorereader/1", "pictures": {} }))
}

#[derive(Deserialize)]
struct PictureBody {
    sha: String,
    page: u32,
}

/// The whole page as a deck asset; the client inserts a picture cropped to the region.
async fn picture(State(st): State<Shared>, UrlPath(name): UrlPath<String>, Json(b): Json<PictureBody>) -> R {
    let png = render(&st, &b.sha, b.page, PICTURE_DPI).await?;
    let (w, h) = png_size(&png).ok_or_else(|| bad("頁面圖片無法讀取"))?;
    let title = Reader::discover(&st.home).and_then(|r| r.json(&b.sha, "paper.json")).map(|m| title_of(&m)).unwrap_or_default();
    let sess = session(&st, &name)?;
    let sess = sess.lock().unwrap();
    let asset = sess.add_asset(&png, "png")?;
    let mut side = sidecar(&sess.dir);
    side["pictures"][&asset] = json!({ "sha": b.sha, "page": b.page, "dpi": PICTURE_DPI, "title": title });
    lore_notation::write_atomic(&sess.dir.join(SIDECAR), &serde_json::to_string_pretty(&side).unwrap_or_default()).map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "asset": asset, "w": w, "h": h })))
}

async fn pictures(State(st): State<Shared>, UrlPath(name): UrlPath<String>) -> R {
    let sess = session(&st, &name)?;
    let sess = sess.lock().unwrap();
    Ok(Json(json!({ "pictures": sidecar(&sess.dir)["pictures"] })))
}

#[derive(Deserialize)]
struct RegisterBody {
    sha: String,
    ids: Vec<String>,
}

/// Copy symbol / equation rows of the paper into the deck's notation (never the global file).
/// A row already there (same id and LaTeX, or the same symbol under another id) is skipped;
/// an id taken by something else gets a free one. An equation brings the symbols it uses.
pub fn register_into(doc: &mut Notation, table: &Value, ids: &[String]) -> Value {
    let all: Vec<&Value> = table["entries"].as_array().into_iter().flatten().chain(table["equations"].as_array().into_iter().flatten()).collect();
    let find = |id: &str| all.iter().find(|e| e["id"].as_str() == Some(id)).copied();
    let mut want: Vec<String> = vec![];
    for id in ids {
        if let Some(e) = find(id) {
            for u in e["uses"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !want.iter().any(|w| w == u) {
                    want.push(u.to_string());
                }
            }
            if !want.contains(id) {
                want.push(id.clone());
            }
        }
    }
    let (mut added, mut skipped) = (vec![], vec![]);
    let mut renamed = BTreeMap::new();
    for id in &want {
        let Some(v) = find(id) else { continue };
        let Ok(mut e) = serde_json::from_value::<Entry>(v.clone()) else {
            skipped.push(id.clone());
            continue;
        };
        let latex = e.latex.clone().unwrap_or_default();
        if latex.trim().is_empty() {
            skipped.push(id.clone());
            continue;
        }
        if let Some(have) = doc.find_latex(&latex) {
            if have.id != e.id {
                renamed.insert(e.id.clone(), have.id.clone());
            }
            skipped.push(id.clone());
            continue;
        }
        if doc.get(&e.id).is_some() {
            let new = doc.free_id(&e.id);
            renamed.insert(e.id.clone(), new.clone());
            e.id = new;
        }
        e.uses = e.uses.iter().map(|u| renamed.get(u).cloned().unwrap_or_else(|| u.clone())).collect();
        added.push(e.id.clone());
        doc.entries.push(e);
    }
    json!({ "added": added, "renamed": renamed, "skipped": skipped })
}

async fn register(State(st): State<Shared>, UrlPath(name): UrlPath<String>, Json(b): Json<RegisterBody>) -> R {
    let r = reader(&st)?;
    let table = r.json(&b.sha, "symbols.json").ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "這篇還沒有符號表".into()))?;
    let sess = session(&st, &name)?;
    let sess = sess.lock().unwrap();
    let (_, mut doc) = crate::notation::load(&sess.dir, None);
    let out = register_into(&mut doc, &table, &b.ids);
    if out["added"].as_array().is_some_and(|a| !a.is_empty()) {
        doc.save(&sess.dir.join("notation.json")).map_err(bad)?;
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const SHA: &str = "ab00000000000000000000000000000000000000000000000000000000000001";

    /// A two-page PDF with a filled box on page 1 (no fonts needed).
    fn synthetic_pdf() -> Vec<u8> {
        let content = b"0 0 1 rg 100 400 300 200 re f\n";
        let objs: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
            format!("<< /Length {} >>\nstream\n{}endstream", content.len(), String::from_utf8_lossy(content)),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = vec![];
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    /// A LoreReader workspace (`<root>/private/LoreReader`) and library with one paper.
    fn synthetic_reader(root: &Path) -> PathBuf {
        let p = root.join("private/LoreReader/papers").join(SHA);
        std::fs::create_dir_all(&p).unwrap();
        let w = |f: &str, v: Value| std::fs::write(p.join(f), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        w("paper.json", json!({ "schema": "lorereader.paper/1", "sha256": SHA, "title": "Synthetic Triage with Uncertainty", "original_name": "triage.pdf",
            "doi": "10.0000/synthetic.1", "pages": 2, "added_at": "2026-11-10T09:00:00+08:00", "authors": ["ellrin", "daidai", "x"], "year": 2026 }));
        w("annotations.json", json!({ "schema": "lorereader.annotations/1", "annotations": [
            { "id": "a1", "page": 1, "kind": "highlight", "color": "yellow", "quote": { "exact": "the model de-\nfers  uncertain cases" }, "rects": [[0.1, 0.1, 0.5, 0.12]] },
            { "id": "a2", "page": 2, "kind": "note", "color": "blue", "quote": { "exact": "" }, "note": "比較 baseline" } ] }));
        w("quotes.json", json!({ "schema": "lorereader.quotes/1", "quotes": [ { "id": "q1", "text": "Deferral improves safety.", "page": 2 } ] }));
        w("symbols.json", json!({ "schema": "lorereader.symbols/1", "sha256": SHA,
            "entries": [
                { "id": "w", "kind": "symbol", "latex": "w_{i}", "meaning_en": "weight of sample i", "scope": SHA, "grade": "defined", "resolved_by": "program", "occurrences": [ { "page": 1, "s": 0, "e": 1, "bbox": [0,0,0,0], "latex": "w_{i}" } ] },
                { "id": "L", "kind": "symbol", "latex": "\\mathcal{L}", "scope": SHA, "grade": "unresolved", "resolved_by": "program" },
                { "id": "nolatex", "kind": "symbol", "scope": SHA, "grade": "unresolved", "resolved_by": "program" } ],
            "equations": [ { "id": "eq1", "kind": "equation", "latex": "\\mathcal{L} = \\sum_i w_{i}", "uses": ["L", "w"], "scope": SHA, "page": 1, "number": "1", "bbox": [0.2, 0.3, 0.8, 0.36], "s": 0, "e": 5 } ] }));
        let lib = root.join("library");
        std::fs::create_dir_all(lib.join(&SHA[..2])).unwrap();
        std::fs::write(lib.join(&SHA[..2]).join(format!("{SHA}.pdf")), synthetic_pdf()).unwrap();
        lib
    }

    #[test]
    fn library_follows_lorereader_config_order() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let app = d.path().join("lrhome");
        std::fs::create_dir_all(&app).unwrap();
        let none = |_: &str| None;
        assert_eq!(library_dir(&none, Some(&home)), home.join("Documents/LoreReader/library"));
        std::fs::write(app.join("config.toml"), "# x\nlibrary = \"~/papers/lib\"   # the PDFs\nexports = \"/x\"\n[engine]\nlibrary = \"/no\"\n").unwrap();
        let app_s = app.display().to_string();
        let with_home = |k: &str| (k == "LOREREADER_HOME").then(|| app_s.clone());
        assert_eq!(library_dir(&with_home, Some(&home)), home.join("papers/lib"));
        let with_env = |k: &str| match k {
            "LOREREADER_LIBRARY" => Some("/env/lib".to_string()),
            "LOREREADER_HOME" => Some(app_s.clone()),
            _ => None,
        };
        assert_eq!(library_dir(&with_env, Some(&home)), PathBuf::from("/env/lib"));
        assert_eq!(config_library("[engine]\nlibrary = \"/x\""), None);
        assert_eq!(config_library("library='/a b'"), Some("/a b".into()));
    }

    #[test]
    fn ref_line_and_text_cleanup() {
        let m = json!({ "title": "A Study", "doi": "10.1/x", "authors": ["ellrin", "daidai"], "year": "2026", "venue": "Synthetic Conf." });
        assert_eq!(ref_line(&m), "Ref: ellrin and daidai, A Study, Synthetic Conf., 2026. doi:10.1/x");
        assert_eq!(ref_line(&json!({ "title": "", "original_name": "triage.pdf" })), "Ref: triage.");
        assert_eq!(clean("the model de-\nfers  uncertain\ncases"), "the model defers uncertain cases");
        assert_eq!(clean("a state-of-the- art model, X- ray"), "a state-of-the-art model, X- ray");
    }

    #[test]
    fn register_adds_once_renames_and_brings_used_symbols() {
        let d = tempfile::tempdir().unwrap();
        synthetic_reader(d.path());
        let table: Value = serde_json::from_slice(&std::fs::read(d.path().join("private/LoreReader/papers").join(SHA).join("symbols.json")).unwrap()).unwrap();
        let mut doc = Notation::default();
        // the deck already uses `w` for something else
        let mut other = Entry::new("w", lore_notation::model::Kind::Symbol);
        other.latex = Some("\\omega".into());
        doc.entries.push(other);
        let r = register_into(&mut doc, &table, &["eq1".into(), "nolatex".into()]);
        assert_eq!(r["added"], json!(["L", "w-2", "eq1"]), "{r}");
        let eq = doc.get("eq1").unwrap();
        assert_eq!(eq.uses, ["L", "w-2"]);
        assert_eq!(doc.get("w-2").unwrap().meaning_en.as_deref(), Some("weight of sample i"));
        let again = register_into(&mut doc, &table, &["eq1".into(), "w".into()]);
        assert_eq!(again["added"], json!([]), "{again}");
    }

    async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&b).unwrap())
            }
            None => Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let st = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null), bytes)
    }

    #[tokio::test]
    async fn api_reads_lorereader_and_never_writes_it() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("forge");
        let lib = synthetic_reader(&root);
        *TEST_LIBRARY.lock().unwrap() = Some(lib.clone());
        let snapshot = |p: &Path| {
            let mut v: Vec<(PathBuf, Vec<u8>)> = vec![];
            fn walk(p: &Path, v: &mut Vec<(PathBuf, Vec<u8>)>) {
                for e in std::fs::read_dir(p).unwrap().flatten() {
                    if e.path().is_dir() { walk(&e.path(), v) } else { v.push((e.path(), std::fs::read(e.path()).unwrap())) }
                }
            }
            walk(p, &mut v);
            v.sort();
            v
        };
        let before = (snapshot(&root.join("private/LoreReader")), snapshot(&lib));

        let ws_dir = d.path().join("sc");
        // the public blank base (canvas base: no template.pptx needed)
        let blank = crate::repo_dir().join("../private-example/SlideComposer/styles/blank");
        std::fs::create_dir_all(ws_dir.join("styles/blank")).unwrap();
        for f in std::fs::read_dir(&blank).unwrap().flatten() {
            std::fs::copy(f.path(), ws_dir.join("styles/blank").join(f.file_name())).unwrap();
        }
        let ws = sc_core::Workspace::new(&ws_dir, None);
        let app = crate::router_isolated(crate::state_at(ws, crate::repo_dir().join("web"), lore_home::Home::at(&root)));

        let (st, v, _) = call(&app, "GET", "/api/reader/papers", None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["available"], true);
        assert_eq!(v["papers"][0]["title"], "Synthetic Triage with Uncertainty");
        assert_eq!(v["papers"][0]["pdf"], true);
        assert_eq!(v["papers"][0]["symbols"], 3);
        let (_, p, _) = call(&app, "GET", &format!("/api/reader/papers/{SHA}"), None).await;
        assert_eq!(p["ref"], "Ref: ellrin et al., Synthetic Triage with Uncertainty, 2026. doi:10.0000/synthetic.1");
        assert_eq!(p["notes"][0]["text"], "the model defers uncertain cases");
        assert_eq!(p["notes"][1]["note"], "比較 baseline");
        assert_eq!(p["symbols"].as_array().unwrap().len(), 2, "rows without LaTeX are left out");
        assert_eq!(p["symbols"][0]["page"], 1);
        assert_eq!(p["equations"][0]["number"], "1");
        let (st, _, _) = call(&app, "GET", "/api/reader/papers/nothex", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);

        let (st, v, _) = call(&app, "POST", "/api/decks", Some(json!({ "name": "talk", "style": "blank" }))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let (st, r, _) = call(&app, "POST", "/api/decks/talk/reader/notation", Some(json!({ "sha": SHA, "ids": ["eq1"] }))).await;
        assert_eq!(st, StatusCode::OK, "{r}");
        assert_eq!(r["added"], json!(["L", "w", "eq1"]));
        let deck_notation = Notation::load(&ws_dir.join("decks/talk/notation.json")).unwrap();
        assert_eq!(deck_notation.entries.len(), 3);
        let (_, n, _) = call(&app, "GET", "/api/decks/talk/notation", None).await;
        assert!(n["notation"]["entries"].as_array().unwrap().iter().any(|e| e["id"] == "eq1" && e["origin"] == "document"), "{n}");

        if pdftoppm().is_some() {
            let (st, _, png) = call(&app, "GET", &format!("/api/reader/papers/{SHA}/page/1?dpi=36"), None).await;
            assert_eq!(st, StatusCode::OK);
            assert_eq!(png_size(&png), Some((306, 396)));
            let (st, v, _) = call(&app, "POST", "/api/decks/talk/reader/picture", Some(json!({ "sha": SHA, "page": 1 }))).await;
            assert_eq!(st, StatusCode::OK, "{v}");
            assert_eq!((v["w"].as_u64(), v["h"].as_u64()), (Some(2550), Some(3300)));
            let asset = v["asset"].as_str().unwrap().to_string();
            assert!(ws_dir.join("decks/talk").join(asset.trim_start_matches("file:")).is_file());
            let (_, pics, _) = call(&app, "GET", "/api/decks/talk/reader/pictures", None).await;
            assert_eq!(pics["pictures"][&asset]["page"], 1);
            let (st, _, _) = call(&app, "GET", &format!("/api/reader/papers/{SHA}/page/9"), None).await;
            assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
        } else {
            eprintln!("pdftoppm not found: page rendering not tested");
        }
        assert!(ws_dir.join("cache/lorereader").exists() || pdftoppm().is_none());
        assert_eq!(before, (snapshot(&root.join("private/LoreReader")), snapshot(&lib)), "LoreReader's files are untouched");
    }
}
