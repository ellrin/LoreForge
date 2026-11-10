//! Start screen: the workspace's papers, most recently edited first, each with
//! its title, the 討論草稿 phase it is at, and a picture of its first page
//! when the paper has been compiled.

use super::*;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/home/papers", get(list))
        .route("/api/home/papers/{name}/thumb", get(thumbnail))
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Card {
    pub name: String,
    /// Seconds since 1970 of the newest file in the paper folder.
    pub modified: u64,
    /// `\title{…}` from main.tex, as plain text.
    pub title: Option<String>,
    /// draft/session.json `phase`; none when there is no session.
    pub phase: Option<String>,
    /// A compiled PDF exists, so `/thumb` can draw the first page.
    pub thumb: bool,
    /// Where it is, for a paper opened outside the workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn card(dir: &Path, name: String) -> Card {
    Card {
        modified: newest(dir, 3),
        title: std::fs::read_to_string(dir.join("main.tex")).ok().and_then(|t| title(&t)),
        phase: phase(dir),
        thumb: compiled_pdf(dir, &name).is_some(),
        name,
        path: None,
    }
}

/// Every paper in `papers/`, newest first.
#[cfg(test)]
pub fn cards(papers: &Path) -> Vec<Card> {
    cards_with(papers, &[])
}

/// The workspace's papers and the folders opened elsewhere that are still
/// there, newest first.
pub fn cards_with(papers: &Path, opened: &[(String, PathBuf)]) -> Vec<Card> {
    let names = if papers.as_os_str().is_empty() { vec![] } else { subdirs(papers) };
    let mut out: Vec<Card> = names.iter().map(|name| card(&papers.join(name), name.clone())).collect();
    for (id, dir) in opened {
        if dir.join("main.tex").is_file() && !names.contains(id) {
            out.push(Card { path: Some(dir.display().to_string()), ..card(dir, id.clone()) });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The newest modification time under `dir` (hidden entries and `export/`
/// skipped: exports are rebuilt, not edited), `depth` folders deep.
fn newest(dir: &Path, depth: u32) -> u64 {
    let secs = |m: &std::fs::Metadata| m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let mut best = std::fs::metadata(dir).map(|m| secs(&m)).unwrap_or(0);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name();
        let n = n.to_string_lossy();
        if n.starts_with('.') || n == "export" {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        let t = if m.is_dir() { if depth > 0 { newest(&e.path(), depth - 1) } else { 0 } } else { secs(&m) };
        best = best.max(t);
    }
    best
}

/// The session's phase as written (`interview`, `draft`, …). Read loosely so
/// a session from an older or newer version still shows its phase.
fn phase(paper: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pc_core::draft::session::draft_dir(paper).join("session.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(v.get("phase").and_then(|p| p.as_str()).unwrap_or("topic").to_string())
}

/// `\title[short]{Long title}` → "Long title", commands and braces dropped.
pub fn title(main: &str) -> Option<String> {
    let at = main.lines().filter(|l| !l.trim_start().starts_with('%')).map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    let start = at.find("\\title")? + "\\title".len();
    let mut rest = at[start..].trim_start();
    if rest.starts_with('[') {
        rest = rest[rest.find(']')? + 1..].trim_start();
    }
    rest = rest.strip_prefix('{')?;
    let mut depth = 1;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let raw = &rest[..end?];
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // `\\` is a line break; `\cmd` is dropped, its argument kept
                if chars.peek() == Some(&'\\') {
                    chars.next();
                    out.push(' ');
                } else {
                    while chars.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                        chars.next();
                    }
                    if chars.peek().is_some_and(|c| !c.is_whitespace() && *c != '{') {
                        if let Some(c) = chars.next() {
                            out.push(c); // \& \% …
                        }
                    }
                }
            }
            '{' | '}' | '~' => out.push(if c == '~' { ' ' } else { '\0' }),
            _ => out.push(c),
        }
    }
    let t = out.replace('\0', "").split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

/// The newest PDF compiled from this paper: the build folder's or an export.
fn compiled_pdf(dir: &Path, name: &str) -> Option<PathBuf> {
    [export::cache_dir(name).join("main.pdf"), dir.join("export").join(format!("{name}.pdf"))]
        .into_iter()
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

async fn list(State(s): State<Shared>) -> impl IntoResponse {
    let ws = s.loaded.read().await.workspace.clone();
    let opened: Vec<(String, PathBuf)> = super::opened::registry().papers.into_iter().map(|e| (e.id, e.path)).collect();
    let papers = ws.map(|w| w.join("papers")).unwrap_or_default();
    Json(cards_with(&papers, &opened))
}

/// The first page of the paper's newest compiled PDF, about 384 px wide.
async fn thumbnail(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let none = || (StatusCode::NOT_FOUND, "no compiled PDF").into_response();
    let Some(pdf) = compiled_pdf(&dir, &name) else { return none() };
    let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer/home");
    let Some((img, mime)) = preview::figure_image(&pdf, &cache).await else { return none() };
    let (img, mime) = thumb::thumbnail(&img, mime, &cache.join("thumbs"), 384).await;
    match std::fs::read(img) {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")], bytes).into_response(),
        Err(_) => none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_titles() {
        assert_eq!(title("\\title{A Study of \\emph{Things}}").as_deref(), Some("A Study of Things"));
        assert_eq!(title("% \\title{old}\n\\title[Short]{Long\\\\ Title \\& More}").as_deref(), Some("Long Title & More"));
        assert_eq!(title("\\title{}"), None);
        assert_eq!(title("no title here"), None);
    }

    #[test]
    fn lists_papers_newest_first_with_phase() {
        let root = std::env::temp_dir().join(format!("pc-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let papers = root.join("papers");
        let set = |p: &Path, secs: u64| {
            let f = std::fs::File::options().write(true).open(p).unwrap();
            f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
        };
        // ellrin's paper: older, with a draft session; daidai's: newer, none
        for (name, secs) in [("ellrin-notes", 1_000_000), ("daidai-study", 2_000_000)] {
            let d = papers.join(name);
            std::fs::create_dir_all(d.join("sections")).unwrap();
            std::fs::write(d.join("main.tex"), format!("\\documentclass{{article}}\n\\title{{Paper by {name}}}\n")).unwrap();
            std::fs::write(d.join("sections/intro.tex"), "x").unwrap();
            set(&d.join("main.tex"), secs);
            set(&d.join("sections/intro.tex"), secs);
        }
        let draft = papers.join("ellrin-notes/draft");
        std::fs::create_dir_all(&draft).unwrap();
        std::fs::write(draft.join("session.json"), r#"{"phase":"interview","experiments":[]}"#).unwrap();
        set(&draft.join("session.json"), 1_000_500);
        // an export does not count as an edit; hidden folders are skipped
        std::fs::create_dir_all(papers.join("ellrin-notes/export")).unwrap();
        std::fs::create_dir_all(papers.join(".hidden")).unwrap();
        let now_dirs = |d: &Path| {
            // folder mtimes change as files are added; pin them too
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() {
                    std::fs::File::open(e.path()).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10)).unwrap();
                }
            }
        };
        for name in ["ellrin-notes", "daidai-study"] {
            let d = papers.join(name);
            now_dirs(&d);
            std::fs::File::open(&d).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10)).unwrap();
        }

        let c = cards(&papers);
        assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["daidai-study", "ellrin-notes"]);
        assert_eq!(c[0].phase, None);
        assert_eq!(c[1].phase.as_deref(), Some("interview"));
        assert_eq!(c[1].modified, 1_000_500, "the session counts as an edit");
        assert_eq!(c[0].title.as_deref(), Some("Paper by daidai-study"));
        assert!(!c[0].thumb);

        // an edit moves a paper to the front
        set(&papers.join("ellrin-notes/sections/intro.tex"), 3_000_000);
        assert_eq!(cards(&papers)[0].name, "ellrin-notes");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn lists_papers_opened_elsewhere_with_their_place() {
        let root = std::env::temp_dir().join(format!("pc-home-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ext = root.join("Projects/daidai draft");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("main.tex"), "\\title{Outside}\n").unwrap();
        let gone = root.join("Projects/moved away");
        let c = cards_with(&root.join("ws/papers"), &[("daidai draft".into(), ext.clone()), ("moved away".into(), gone)]);
        assert_eq!(c.len(), 1, "a folder that is gone is not a card");
        assert_eq!((c[0].name.as_str(), c[0].title.as_deref(), c[0].path.as_deref()), ("daidai draft", Some("Outside"), Some(ext.to_str().unwrap())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn session_without_phase_is_at_the_start() {
        let root = std::env::temp_dir().join(format!("pc-home-phase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("draft")).unwrap();
        assert_eq!(phase(&root), None);
        std::fs::write(root.join("draft/session.json"), "{}").unwrap();
        assert_eq!(phase(&root).as_deref(), Some("topic"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
