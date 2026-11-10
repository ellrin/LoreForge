//! Papers that live outside the workspace (開啟 LaTeX… / 開啟 Word…).
//!
//! A paper is a folder. The workspace's `papers/<name>/` are papers as
//! before; a folder opened anywhere else is listed in `opened.toml` beside
//! `config.toml` (the OS config folder: paths are this machine's, so the
//! list is not synced with the workspace, and clearing caches leaves it).
//! Each entry has an id that is a plain name (the folder's name, made
//! unique), used in the API exactly like a workspace paper's name.
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/shell/dialog | `{kind: latex\|word\|folder, start?}` → `{path}`; 501 without the desktop app |
//! | GET | /api/open/recent | opened folders, newest first, `exists` false when gone |
//! | POST | /api/open/forget | `{id}`: drop from the list (the folder is not touched) |
//! | POST | /api/open/inspect | `{path}` → `{kind: paper, id}` / `{kind: latex, plan}` / `{kind: archive, dest}` / `{kind: word, dest, pandoc}` |
//! | POST | /api/open/latex | `{path, stamp, restructure?}`: open as it is (only `.papercomposer/` is added); `restructure` first copies the originals to `.papercomposer/original/` |
//! | POST | /api/open/extract | `{path, dest}`: unpack an archive, then inspect the folder |
//! | POST | /api/open/word | `{path, dest}`: convert into a new folder, then open |
//! | POST | /api/open/word-upload?name= | browser mode: the .docx as the body becomes a workspace paper |

use super::*;
use pc_core::open as inplace;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/shell/dialog", post(shell_dialog))
        .route("/api/open/recent", get(recent))
        .route("/api/open/forget", post(forget))
        .route("/api/open/inspect", post(inspect))
        .route("/api/open/latex", post(open_latex))
        .route("/api/open/extract", post(extract))
        .route("/api/open/word", post(open_word))
        .route("/api/open/word-upload", post(word_upload).layer(axum::extract::DefaultBodyLimit::max(200 * 1024 * 1024)))
        .route("/api/papers/{name}/stamp", get(stamp))
}

// ── Registry ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub path: PathBuf,
    /// "latex" or "word".
    #[serde(default)]
    pub kind: String,
    /// The .docx or archive it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    /// When it was last opened (milliseconds since 1970).
    #[serde(default)]
    pub opened: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    #[serde(default, rename = "paper")]
    pub papers: Vec<Entry>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A folder name as a paper id: letters, digits, space, - _ . only.
pub fn id_from(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_start_matches('.').trim().to_string();
    if s.is_empty() { "paper".into() } else { s }
}

impl Registry {
    pub fn load(file: &Path) -> Registry {
        std::fs::read_to_string(file).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        paper::write_atomic(file, &toml::to_string(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.papers.iter().find(|e| e.id == id)
    }

    fn same(a: &Path, b: &Path) -> bool {
        a == b || a.canonicalize().ok().is_some_and(|x| b.canonicalize().ok().is_some_and(|y| x == y))
    }

    pub fn by_path(&self, path: &Path) -> Option<&Entry> {
        self.papers.iter().find(|e| Self::same(&e.path, path))
    }

    /// Add (or touch) the folder `path`; `taken` are names the id must not
    /// have (the workspace's papers). Returns the id.
    pub fn add(&mut self, path: &Path, kind: &str, source: Option<&Path>, taken: &[String]) -> String {
        if let Some(e) = self.papers.iter_mut().find(|e| Self::same(&e.path, path)) {
            e.opened = now();
            if source.is_some() {
                e.source = source.map(Path::to_path_buf);
            }
            return e.id.clone();
        }
        let base = id_from(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let free = |id: &str| !taken.iter().any(|t| t == id) && !self.papers.iter().any(|e| e.id == id);
        let id = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base} {k}") }).find(|i| free(i)).expect("a free id");
        self.papers.push(Entry { id: id.clone(), path: path.to_path_buf(), kind: kind.into(), source: source.map(Path::to_path_buf), opened: now() });
        id
    }

    pub fn forget(&mut self, id: &str) {
        self.papers.retain(|e| e.id != id);
    }

    /// Newest first.
    pub fn recent(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.papers.iter().collect();
        v.sort_by(|a, b| b.opened.cmp(&a.opened).then_with(|| a.id.cmp(&b.id)));
        v
    }
}

/// `opened.toml` beside config.toml.
pub fn registry_file() -> PathBuf {
    std::env::var_os("PAPERCOMPOSER_OPENED")
        .map(PathBuf::from)
        .or_else(|| config_path().map(|p| p.with_file_name("opened.toml")))
        .unwrap_or_else(|| std::env::temp_dir().join("PaperComposer-opened.toml"))
}

/// One read-modify-write of opened.toml at a time.
static REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn registry() -> Registry {
    Registry::load(&registry_file())
}

fn update<T>(f: impl FnOnce(&mut Registry) -> T) -> Result<T, String> {
    let _g = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = registry_file();
    let mut r = Registry::load(&file);
    let out = f(&mut r);
    r.save(&file)?;
    Ok(out)
}

/// An opened (external) paper's folder by id.
pub fn external_dir(id: &str) -> Option<PathBuf> {
    registry().get(id).map(|e| e.path.clone())
}

/// The folder an opened paper's PaperComposer files go into: a paper that
/// keeps them at its top level (one made in the workspace and moved out)
/// stays so; any other gets `.papercomposer/`.
fn ensure_app_dir(dir: &Path) -> Result<(), String> {
    if dir.join(pc_core::meta::META_FILE).is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(dir.join(paper::APP_DIR)).map_err(|e| e.to_string())
}

// ── Native dialogs (desktop shell) ──────────────────────

/// A native file dialog the page asks for (`/api/shell/dialog`).
#[derive(Debug, Clone, Deserialize)]
pub struct DialogReq {
    /// `latex` (a folder, or a .zip/.tar.gz/.tgz/.tex), `word` (a .docx),
    /// `folder` (where to put something).
    pub kind: String,
    /// Folder to start in.
    #[serde(default)]
    pub start: Option<PathBuf>,
    /// Suggested name (`folder`: a new folder's name).
    #[serde(default)]
    pub name: Option<String>,
}

/// Shows the dialog and returns the chosen path (blocking; run off the async runtime).
pub type DialogHook = Arc<dyn Fn(DialogReq) -> Option<PathBuf> + Send + Sync>;

/// Installed by the desktop app before the server starts; none in browser mode.
pub static DIALOG: std::sync::OnceLock<DialogHook> = std::sync::OnceLock::new();

async fn shell_dialog(Json(b): Json<DialogReq>) -> impl IntoResponse {
    let Some(hook) = DIALOG.get().cloned() else {
        return (StatusCode::NOT_IMPLEMENTED, "瀏覽器模式沒有檔案對話框").into_response();
    };
    match tokio::task::spawn_blocking(move || hook(b)).await {
        Ok(path) => Json(serde_json::json!({ "path": path })).into_response(),
        Err(e) => bad(&e.to_string()),
    }
}

// ── Handlers ────────────────────────────────────────────

async fn recent() -> impl IntoResponse {
    let r = registry();
    let list: Vec<serde_json::Value> = r
        .recent()
        .into_iter()
        .take(8)
        .map(|e| serde_json::json!({ "id": e.id, "path": e.path, "kind": e.kind, "exists": inplace::is_paper(&e.path) }))
        .collect();
    Json(list)
}

#[derive(Deserialize)]
struct IdBody {
    id: String,
}

async fn forget(Json(b): Json<IdBody>) -> impl IntoResponse {
    match update(|r| r.forget(&b.id)) {
        Ok(()) => Json(serde_json::json!({})).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct PathBody {
    path: PathBuf,
    #[serde(default)]
    stamp: String,
    #[serde(default)]
    dest: Option<PathBuf>,
    /// 開啟 LaTeX…: restructure into main.tex + sections/ (after a backup). Default: open as it is.
    #[serde(default)]
    restructure: bool,
}

fn lower_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_archive(p: &Path) -> bool {
    let n = lower_name(p);
    n.ends_with(".zip") || n.ends_with(".tar.gz") || n.ends_with(".tgz") || n.ends_with(".tar")
}

/// Where an archive unpacks by default: a folder named after it, beside it.
pub fn archive_dest(p: &Path) -> PathBuf {
    let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lower = n.to_lowercase();
    let stem = [".tar.gz", ".tgz", ".zip", ".tar"].iter().find(|x| lower.ends_with(*x)).map(|x| &n[..n.len() - x.len()]).unwrap_or(&n).to_string();
    let base = p.with_file_name(&stem);
    (1..).map(|k| if k == 1 { base.clone() } else { p.with_file_name(format!("{stem} {k}")) }).find(|d| !d.exists()).expect("a free name")
}

/// The project folder of a .tex file: its folder, or the one above when it
/// is a section file of a paper.
pub fn folder_of_tex(tex: &Path) -> PathBuf {
    let dir = tex.parent().map(Path::to_path_buf).unwrap_or_default();
    match dir.parent() {
        Some(up) if lower_name(&dir) == "sections" && up.join("main.tex").is_file() => up.to_path_buf(),
        _ => dir,
    }
}

/// An archive with one folder at the top: that folder is the project.
fn single_top(dir: &Path) -> PathBuf {
    let entries: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.file_name() != "__MACOSX").map(|e| e.path()).collect();
    match entries.as_slice() {
        [only] if only.is_dir() => only.clone(),
        _ => dir.to_path_buf(),
    }
}

/// The workspace paper a folder is, if it is one.
async fn workspace_name(s: &Shared, dir: &Path) -> Option<String> {
    let ws = s.loaded.read().await.workspace.clone()?;
    let papers = ws.join("papers").canonicalize().ok()?;
    let d = dir.canonicalize().ok()?;
    (d.parent() == Some(papers.as_path())).then(|| d.file_name().map(|n| n.to_string_lossy().to_string())).flatten()
}

async fn workspace_names(s: &Shared) -> Vec<String> {
    s.loaded.read().await.workspace.as_ref().map(|w| subdirs(&w.join("papers"))).unwrap_or_default()
}

/// Register an opened paper folder and give the page its id.
async fn opened(s: &Shared, dir: &Path, kind: &str, source: Option<&Path>) -> Result<serde_json::Value, String> {
    if let Some(name) = workspace_name(s, dir).await {
        return Ok(serde_json::json!({ "kind": "paper", "id": name }));
    }
    ensure_app_dir(dir)?;
    // what an import records: the journal its class files match, the profile's people named as authors
    let m = meta::read_meta(dir);
    if !paper::app_dir(dir).join(meta::META_FILE).is_file() {
        let (ws, people) = {
            let l = s.loaded.read().await;
            (l.workspace.clone(), l.profile.as_ref().map(|p| p.authors.clone()).unwrap_or_default())
        };
        let journal = ws.as_ref().and_then(|w| pc_core::journal::detect(&w.join("publishers"), dir));
        let main = std::fs::read_to_string(inplace::main_file(dir)).unwrap_or_default();
        let authors = if m.authors.is_empty() { pc_core::authors::match_block(&main, &people) } else { m.authors.clone() };
        meta::write_meta(dir, &PaperMeta { journal: m.journal.clone().or(journal), authors, ..m })?;
    }
    let taken = workspace_names(s).await;
    let id = update(|r| r.add(dir, kind, source, &taken))?;
    Ok(serde_json::json!({ "kind": "paper", "id": id, "path": dir }))
}

async fn inspect_folder(s: &Shared, dir: &Path) -> Result<serde_json::Value, String> {
    if let Some(name) = workspace_name(s, dir).await {
        return Ok(serde_json::json!({ "kind": "paper", "id": name }));
    }
    if let Some(e) = registry().by_path(dir).cloned() {
        if inplace::is_paper(dir) {
            return opened(s, &e.path, &e.kind, None).await;
        }
    }
    let d = dir.to_path_buf();
    match tokio::task::spawn_blocking(move || inplace::plan(&d)).await.map_err(|e| e.to_string())?? {
        None => {
            let base = paper::flatten_project(dir).map(|f| f.base).unwrap_or_else(|_| dir.to_path_buf());
            opened(s, &base, "latex", None).await
        }
        Some(plan) => Ok(serde_json::json!({ "kind": "latex", "path": dir, "plan": plan })),
    }
}

async fn inspect(State(s): State<Shared>, Json(b): Json<PathBody>) -> impl IntoResponse {
    let p = b.path;
    if !p.exists() {
        return bad(&format!("找不到 {}", p.display()));
    }
    let n = lower_name(&p);
    let res = if p.is_dir() {
        inspect_folder(&s, &p).await
    } else if n.ends_with(".tex") {
        inspect_folder(&s, &folder_of_tex(&p)).await
    } else if is_archive(&p) {
        Ok(serde_json::json!({ "kind": "archive", "path": p, "dest": archive_dest(&p) }))
    } else if n.ends_with(".docx") {
        Ok(serde_json::json!({ "kind": "word", "path": p, "dest": word::default_dest(&p), "pandoc": word::has_pandoc() }))
    } else {
        Err("請選 LaTeX 資料夾、.zip、.tar.gz、.tex 或 .docx".into())
    };
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(&e),
    }
}

async fn open_latex(State(s): State<Shared>, Json(b): Json<PathBody>) -> impl IntoResponse {
    let (p, stamp, restructure) = (b.path.clone(), b.stamp.clone(), b.restructure);
    let res = match tokio::task::spawn_blocking(move || inplace::apply(&p, &stamp, restructure)).await {
        Ok(r) => r,
        Err(e) => Err(e.to_string()),
    };
    match res {
        Ok(dir) => match opened(&s, &dir, "latex", None).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => bad(&e),
        },
        Err(e) => bad(&e),
    }
}

/// Unpack an archive into `dest` (missing or empty).
pub fn unpack_to(archive: &Path, dest: &Path) -> Result<PathBuf, String> {
    if dest.exists() && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} 已經有檔案，請選一個新的資料夾", dest.display()));
    }
    let bytes = std::fs::read(archive).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    if let Err(e) = import::unpack(&bytes, dest) {
        let _ = std::fs::remove_dir_all(dest);
        return Err(e);
    }
    Ok(single_top(dest))
}

async fn extract(State(s): State<Shared>, Json(b): Json<PathBody>) -> impl IntoResponse {
    let dest = b.dest.clone().unwrap_or_else(|| archive_dest(&b.path));
    let res = match unpack_to(&b.path, &dest) {
        Ok(dir) => inspect_folder(&s, &dir).await,
        Err(e) => Err(e),
    };
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(&e),
    }
}

async fn open_word(State(s): State<Shared>, Json(b): Json<PathBody>) -> impl IntoResponse {
    let dest = b.dest.clone().unwrap_or_else(|| word::default_dest(&b.path));
    let people = s.loaded.read().await.profile.as_ref().map(|p| p.authors.clone()).unwrap_or_default();
    let mut c = match word::convert(&b.path, &dest, &people).await {
        Ok(c) => c,
        Err(e) => return bad(&e),
    };
    // the reference checks again, with the author's notation registry
    if let Some(ws) = s.loaded.read().await.workspace.clone() {
        c.notation = word::reference_findings(&c.dir, &crate::notation::load(&ws, &c.dir));
    }
    // typed numbers ("Table 1") become review comments on the spot, so they are not lost with a toast
    if !c.notation.is_empty() {
        c.comments += typed_number_comments(&c.dir);
    }
    match opened(&s, &c.dir, "word", Some(&b.path)).await {
        Ok(mut v) => {
            v["converted"] = serde_json::to_value(&c).unwrap_or_default();
            Json(v).into_response()
        }
        Err(e) => bad(&e),
    }
}

/// Every typed "Table 1" / "Figure 2" / "Fig. 3" / "Eq. 4" in the paper's text, as (paragraph, the words).
pub fn typed_numbers(paras: &[String]) -> Vec<(usize, String)> {
    const WORDS: [&str; 6] = ["Table", "Figure", "Fig.", "Equation", "Eq.", "Algorithm"];
    let mut out = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        for w in WORDS {
            let mut from = 0;
            while let Some(at) = p[from..].find(w).map(|x| x + from) {
                from = at + w.len();
                let before_ok = p[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
                let rest = p[from..].trim_start_matches([' ', '\u{a0}']);
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if before_ok && !digits.is_empty() && rest.len() < p[from..].len() {
                    let end = p.len() - rest.len() + digits.len();
                    out.push((i, p[at..end].to_string()));
                }
            }
        }
    }
    out
}

/// Review comments (source "word") for the typed numbers left by a conversion.
fn typed_number_comments(dir: &Path) -> usize {
    let mut r = pc_core::review::load(dir);
    let (mut n, at) = (0, pc_core::review::now());
    for sec in paper::load_paper(dir) {
        for (index, exact) in typed_numbers(&sec.paragraphs) {
            r.comments.push(pc_core::review::Comment {
                id: pc_core::review::new_id("m"),
                author: String::new(),
                at: at.clone(),
                file: sec.file.clone(),
                index,
                exact,
                prefix: String::new(),
                suffix: String::new(),
                text: "手打的編號：改成 \\ref{…}，編號才會跟著走".into(),
                replies: vec![],
                resolved: false,
                orphan: false,
                source: Some("word".into()),
                extra: Default::default(),
            });
            n += 1;
        }
    }
    if n > 0 && pc_core::review::save(dir, &r).is_err() {
        return 0;
    }
    n
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}

/// Browser mode (no file paths): an uploaded .docx becomes a workspace paper.
async fn word_upload(State(s): State<Shared>, AxQuery(q): AxQuery<UploadQuery>, body: axum::body::Bytes) -> impl IntoResponse {
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    let base = id_from(q.name.trim_end_matches(".docx"));
    let taken: Vec<String> = workspace_names(&s).await.into_iter().chain(registry().papers.into_iter().map(|e| e.id)).collect();
    let name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base} {k}") }).find(|n| !taken.contains(n)).expect("a free name");
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("papercomposer-upload-{}-{n}.docx", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, &body) {
        return bad(&e.to_string());
    }
    let people = s.loaded.read().await.profile.as_ref().map(|p| p.authors.clone()).unwrap_or_default();
    let res = word::convert(&tmp, &ws.join("papers").join(&name), &people).await;
    let _ = std::fs::remove_file(&tmp);
    match res {
        Ok(c) => Json(serde_json::json!({ "kind": "paper", "id": name, "converted": c })).into_response(),
        Err(e) => bad(&e),
    }
}

/// The paper's sources as they are on disk (see `paper::stamp`): the page
/// compares it with what it loaded to notice edits made elsewhere.
async fn stamp(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    Json(serde_json::json!({ "stamp": inplace::stamp(&dir) })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-opened-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn finds_typed_numbers() {
        let p = vec!["See Table 1 and Figure 2, Fig. 3; not Tables or MyTable 4.".to_string(), "Eq. 12 holds".to_string()];
        let got: Vec<String> = typed_numbers(&p).into_iter().map(|(i, t)| format!("{i}:{t}")).collect();
        assert_eq!(got, ["0:Table 1", "0:Figure 2", "0:Fig. 3", "1:Eq. 12"]);
    }

    #[test]
    fn ids_are_plain_unique_names() {
        assert_eq!(id_from("Widgets (LaTeX)"), "Widgets LaTeX");
        assert_eq!(id_from(".hidden"), "hidden");
        assert_eq!(id_from("///"), "paper");
        let root = tmp("ids");
        let file = root.join("conf/opened.toml");
        let mut r = Registry::default();
        let a = root.join("a/Paper");
        let b = root.join("b/Paper");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(r.add(&a, "latex", None, &["Paper".into()]), "Paper 2", "a workspace paper has the name");
        assert_eq!(r.add(&b, "word", Some(&root.join("x.docx")), &[]), "Paper");
        assert_eq!(r.add(&a, "latex", None, &[]), "Paper 2", "the same folder keeps its id");
        r.save(&file).unwrap();
        let back = Registry::load(&file);
        assert_eq!(back, r);
        assert_eq!(back.get("Paper").unwrap().source.as_deref(), Some(root.join("x.docx").as_path()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_list_is_not_a_cache() {
        // clearing the caches (build output, thumbnails) leaves the opened folders listed
        let file = registry_file();
        if std::env::var_os("PAPERCOMPOSER_OPENED").is_none() {
            if let (Some(cache), Some(cfg)) = (dirs::cache_dir(), config_path()) {
                assert!(!file.starts_with(&cache), "{}", file.display());
                assert_eq!(file.parent(), cfg.parent());
            }
        }
        let root = tmp("cache");
        let file = root.join("config/PaperComposer/opened.toml");
        let cache = root.join("cache/PaperComposer/build/P");
        std::fs::create_dir_all(&cache).unwrap();
        let mut r = Registry::default();
        r.add(&root.join("P"), "latex", None, &[]);
        r.save(&file).unwrap();
        std::fs::remove_dir_all(root.join("cache")).unwrap();
        assert_eq!(Registry::load(&file).papers.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn archives_unpack_beside_themselves() {
        let root = tmp("zip");
        let zip_path = root.join("Draft v2.zip");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("Draft v2/main.tex", o).unwrap();
            z.write_all(b"\\documentclass{article}\n\\begin{document}\n\\section{A}\nx\n\\end{document}\n").unwrap();
            z.finish().unwrap();
        }
        assert!(is_archive(&zip_path) && is_archive(Path::new("a.TAR.GZ")) && !is_archive(Path::new("a.docx")));
        let dest = archive_dest(&zip_path);
        assert_eq!(dest, root.join("Draft v2"));
        let project = unpack_to(&zip_path, &dest).unwrap();
        assert_eq!(project, dest.join("Draft v2"), "one folder at the top is the project");
        assert!(project.join("main.tex").is_file());
        // the default never reuses a folder; a chosen one must be empty
        assert_eq!(archive_dest(&zip_path), root.join("Draft v2 2"));
        assert!(unpack_to(&zip_path, &dest).is_err());
        // a .tex in sections/ opens its paper
        std::fs::create_dir_all(project.join("sections")).unwrap();
        assert_eq!(folder_of_tex(&project.join("sections/01-a.tex")), project);
        assert_eq!(folder_of_tex(&project.join("main.tex")), project);
        let _ = std::fs::remove_dir_all(&root);
    }
}
