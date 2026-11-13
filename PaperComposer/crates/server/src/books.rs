//! 寫書: the books PaperComposer knows, in `books.toml` beside config.toml
//! (the OS config folder, like `opened.toml`: paths are this machine's).
//! A book is a folder where the author chose (pc_core::book); it opens like a
//! paper opened where it is, so it is also listed in `opened.toml` and its
//! id there is the id the page opens.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/books | `{books: [{id, title, path, papers, modified, thumb}], folder}`: newest first; `folder` = where 新書 puts the next one |
//! | POST | /api/books | `{title, folder}` → `{id, path}`: a new book from the book template; the folder is remembered |

use super::*;

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/api/books", get(list).post(create))
}

// ── books.toml ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Book {
    pub path: PathBuf,
    #[serde(default)]
    pub title: String,
    /// LoreReader papers (sha) the book was written from.
    #[serde(default)]
    pub papers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Registry {
    /// The folder the last new book went into (新書's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_folder: Option<PathBuf>,
    #[serde(default, rename = "book")]
    pub books: Vec<Book>,
}

impl Registry {
    /// A missing or unreadable file is an empty list.
    pub fn load(file: &Path) -> Registry {
        std::fs::read_to_string(file).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        paper::write_atomic(file, &toml::to_string(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// Add the book at `path` (or update its title); the same folder is listed once.
    pub fn add(&mut self, path: &Path, title: &str) {
        let same = |p: &Path| p == path || p.canonicalize().ok().is_some_and(|x| path.canonicalize().ok().is_some_and(|y| x == y));
        match self.books.iter_mut().find(|b| same(&b.path)) {
            Some(b) => b.title = title.to_string(),
            None => self.books.push(Book { path: path.to_path_buf(), title: title.to_string(), papers: vec![] }),
        }
    }
}

/// `books.toml` beside config.toml (`PAPERCOMPOSER_BOOKS` overrides).
pub fn registry_file() -> PathBuf {
    std::env::var_os("PAPERCOMPOSER_BOOKS")
        .map(PathBuf::from)
        .or_else(|| config_path().map(|p| p.with_file_name("books.toml")))
        .unwrap_or_else(|| std::env::temp_dir().join("PaperComposer-books.toml"))
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn update<T>(f: impl FnOnce(&mut Registry) -> T) -> Result<T, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = registry_file();
    let mut r = Registry::load(&file);
    let out = f(&mut r);
    r.save(&file)?;
    Ok(out)
}

/// Where 新書 suggests: the last folder used, else Documents (or home).
fn default_folder(r: &Registry) -> Option<PathBuf> {
    r.last_folder.clone().filter(|d| d.is_dir()).or_else(dirs::document_dir).or_else(dirs::home_dir)
}

// ── Handlers ────────────────────────────────────────────

/// The book's id in opened.toml, registering it when it is not there (the
/// list was cleared, or the book came from another machine's books.toml).
async fn id_of(s: &Shared, dir: &Path) -> Option<String> {
    if let Some(e) = opened::registry().by_path(dir) {
        return Some(e.id.clone());
    }
    opened::register(s, dir).await.ok()
}

async fn list(State(s): State<Shared>) -> impl IntoResponse {
    let r = Registry::load(&registry_file());
    let mut out = Vec::new();
    for b in &r.books {
        if !pc_core::open::is_paper(&b.path) {
            continue; // moved or on a drive that is not here: kept in books.toml
        }
        let Some(id) = id_of(&s, &b.path).await else { continue };
        let c = home::card(&b.path, id.clone());
        out.push(serde_json::json!({
            "id": id,
            "title": c.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| b.title.clone()),
            "path": b.path,
            "papers": b.papers,
            "modified": c.modified,
            "thumb": c.thumb,
        }));
    }
    out.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    Json(serde_json::json!({ "books": out, "folder": default_folder(&r) }))
}

#[derive(Deserialize)]
struct NewBook {
    title: String,
    folder: PathBuf,
}

async fn create(State(s): State<Shared>, Json(b): Json<NewBook>) -> impl IntoResponse {
    if !b.folder.is_absolute() {
        return bad("請選擇存放的資料夾");
    }
    if let Err(e) = std::fs::create_dir_all(&b.folder) {
        return bad(&format!("無法建立資料夾 {}：{e}", b.folder.display()));
    }
    // the workspace's book template, else the built-in one
    let ws = s.loaded.read().await.workspace.clone();
    let scratch = std::env::temp_dir().join(format!("pc-book-template-{}", std::process::id()));
    let template = match ws.as_ref().and_then(|w| pc_core::book::workspace_template(&w.join("publishers"))) {
        Some(t) => t,
        None => {
            let t = scratch.join(pc_core::book::TEMPLATE_ID);
            if let Err(e) = pc_core::book::write_builtin(&t) {
                return bad(&e.to_string());
            }
            t
        }
    };
    let made = pc_core::book::create(&b.folder, &b.title, &template);
    let _ = std::fs::remove_dir_all(&scratch);
    let dir = match made { Ok(d) => d, Err(e) => return bad(&e) };
    let title = b.title.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Err(e) = update(|r| {
        r.add(&dir, &title);
        r.last_folder = Some(b.folder.clone());
    }) {
        return bad(&e);
    }
    match opened::register(&s, &dir).await {
        Ok(id) => Json(serde_json::json!({ "id": id, "path": dir })).into_response(),
        Err(e) => bad(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn books_toml_round_trips_and_lists_a_folder_once() {
        let d = std::env::temp_dir().join(format!("pc-books-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let file = d.join("conf/PaperComposer/books.toml");
        assert_eq!(Registry::load(&file), Registry::default(), "no file, no folder: an empty list");
        let mut r = Registry::default();
        r.add(Path::new("/books/a"), "A");
        r.add(Path::new("/books/b"), "B");
        r.add(Path::new("/books/a"), "A2");
        r.books[1].papers.push("abc123".into());
        r.last_folder = Some(PathBuf::from("/books"));
        r.save(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("[[book]]") && text.contains("last_folder = \"/books\"") && text.contains("papers = [\"abc123\"]"), "{text}");
        let back = Registry::load(&file);
        assert_eq!(back, r);
        assert_eq!(back.books.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["A2", "B"]);
        std::fs::write(&file, "not toml [[").unwrap();
        assert_eq!(Registry::load(&file), Registry::default(), "unreadable: empty, never an error");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The built-in template compiles with the engine export picks (XeLaTeX),
    /// with a term, a margin note, the boxes and Chinese in it. Skipped
    /// where XeLaTeX is not installed.
    #[test]
    fn a_new_book_compiles_with_xelatex() {
        let Some(xelatex) = crate::texmeasure::tool("xelatex") else { return };
        let root = std::env::temp_dir().join(format!("pc-books-tex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tpl = root.join("tpl");
        pc_core::book::write_builtin(&tpl).unwrap();
        let dir = pc_core::book::create(&root, "Notes 筆記", &tpl).unwrap();
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert_eq!(crate::export::engine_for(&main), "xelatex");
        let section = &pc_core::paper::section_files(&dir)[0];
        std::fs::write(
            dir.join("sections").join(section),
            "\\section{Introduction}\\label{sec:intro}\n\nA \\term{positive pair}{正樣本對} is two views of one example.\\sidenote{邊註 margin note.}\n\n\\begin{definition}[pair]\nTwo views, $\\tau > 0$.\n\\end{definition}\n\n\\begin{keyidea}\nSee Section~\\ref{sec:intro}.\n\\end{keyidea}\n\n\\begin{intuition}\n直覺 first.\n\\end{intuition}\n",
        )
        .unwrap();
        let out = std::process::Command::new(xelatex).args(["-interaction=nonstopmode", "-halt-on-error", "main.tex"]).current_dir(&dir).output().unwrap();
        let log = std::fs::read_to_string(dir.join("main.log")).unwrap_or_default();
        assert!(out.status.success() && dir.join("main.pdf").is_file(), "{}", &log[log.len().saturating_sub(3000)..]);
        if cfg!(target_os = "macos") {
            assert!(!log.contains("No CJK font found"), "macOS has a CJK font of the chain");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_default_folder_is_the_last_one_while_it_exists() {
        let d = std::env::temp_dir().join(format!("pc-books-last-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let r = Registry { last_folder: Some(d.clone()), books: vec![] };
        assert_eq!(default_folder(&r), Some(d.clone()));
        let gone = Registry { last_folder: Some(d.join("gone")), books: vec![] };
        assert_ne!(default_folder(&gone), Some(d.join("gone")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
