//! What the shared Lore shell (`docs/shell.md`) needs from this server.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /icons.js, /base.css, /shell.js | `lore_shell::FILES` (commons/ui, compiled in) |
//! | GET | /api/papers/{name}/export/file?kind=pdf | the last compiled PDF (匯出 ▾ 列印 prints it) |
//! | POST | /api/papers/{name}/save-as | `{name}` → `{id}`: 另存新檔, a copy of the paper under a new name |
//!
//! 另存新檔 puts the copy beside the original: a workspace paper into the
//! workspace's `papers/`, a paper opened where it is into the same parent folder
//! (and into opened.toml). It never overwrites: a name that is taken is refused.
//! The copy leaves out `export/`, `.git/` and `.papercomposer/original/`.

use super::*;

pub(crate) fn routes() -> Router<Shared> {
    let mut r = Router::new()
        .route("/api/papers/{name}/export/file", get(export_file))
        .route("/api/papers/{name}/save-as", post(save_as));
    for (path, mime, body) in lore_shell::FILES {
        r = r.route(path, get(move || async move { ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body) }));
    }
    r
}

#[derive(Deserialize)]
struct KindQuery {
    #[serde(default)]
    kind: String,
}

async fn export_file(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<KindQuery>) -> axum::response::Response {
    if !matches!(q.kind.as_str(), "" | "pdf") {
        return bad("only the PDF prints");
    }
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    match std::fs::read(dir.join("export").join(format!("{name}.pdf"))) {
        Ok(b) => ([(header::CONTENT_TYPE, "application/pdf"), (header::CACHE_CONTROL, "no-cache")], b).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "還沒有匯出").into_response(),
    }
}

#[derive(Deserialize)]
struct SaveAs {
    name: String,
}

async fn save_as(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<SaveAs>) -> axum::response::Response {
    let from = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let new = b.name.trim().to_string();
    if !paper::safe_name(&new) {
        return bad("名稱只能有字母、數字、空格與 - _ .");
    }
    if let Some(r) = name_taken_outside(&new) {
        return r;
    }
    let ws = s.loaded.read().await.workspace.clone();
    let in_ws = ws.as_ref().is_some_and(|w| from.parent().is_some_and(|p| p == w.join("papers")));
    let to = if in_ws {
        ws.as_ref().map(|w| w.join("papers").join(&new))
    } else {
        from.parent().map(|p| p.join(&new))
    };
    let Some(to) = to else { return bad("no workspace") };
    if to.exists() {
        return bad(&format!("已經有 {}", to.display()));
    }
    if ws.as_ref().is_some_and(|w| w.join("papers").join(&new).exists()) {
        return bad(&format!("已經有一篇叫 {new} 的論文"));
    }
    let (f, t) = (from.clone(), to.clone());
    let copied = tokio::task::spawn_blocking(move || copy_paper(&f, &t)).await.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| e.to_string()));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&to);
        return bad(&e);
    }
    if in_ws {
        return Json(serde_json::json!({ "id": new, "path": null })).into_response();
    }
    match opened::register(&s, &to).await {
        Ok(id) => Json(serde_json::json!({ "id": id, "path": to })).into_response(),
        Err(e) => bad(&e),
    }
}

/// Copy a paper folder for 另存新檔: everything but `export/`, `.git/` and
/// `.papercomposer/original/` (the restructuring's backup of the first version).
pub fn copy_paper(from: &Path, to: &Path) -> std::io::Result<()> {
    fn walk(from: &Path, to: &Path, depth: usize, in_app: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let n = entry.file_name().to_string_lossy().to_string();
            if (depth == 0 && (n == "export" || n == ".git")) || (in_app && depth == 1 && n == "original") {
                continue;
            }
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), &target, depth + 1, depth == 0 && n == ".papercomposer")?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    walk(from, to, 0, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_as_copies_the_paper_without_exports_git_or_the_original_backup() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("pc-saveas-{}-{nanos}", std::process::id()));
        let from = root.join("ellrin-paper");
        for d in ["sections", "export", ".git", ".papercomposer/original", "figures"] {
            std::fs::create_dir_all(from.join(d)).unwrap();
        }
        std::fs::write(from.join("main.tex"), "\\documentclass{article}").unwrap();
        std::fs::write(from.join("sections/intro.tex"), "\\section{Intro}").unwrap();
        std::fs::write(from.join("export/ellrin-paper.pdf"), "%PDF").unwrap();
        std::fs::write(from.join(".git/HEAD"), "ref").unwrap();
        std::fs::write(from.join(".papercomposer/layout.toml"), "main = \"main.tex\"").unwrap();
        std::fs::write(from.join(".papercomposer/original/main.tex"), "old").unwrap();
        std::fs::write(from.join("figures/a.png"), "png").unwrap();
        let to = root.join("daidai-copy");
        copy_paper(&from, &to).unwrap();
        assert!(to.join("main.tex").is_file() && to.join("sections/intro.tex").is_file() && to.join("figures/a.png").is_file());
        assert!(to.join(".papercomposer/layout.toml").is_file());
        assert!(!to.join("export").exists() && !to.join(".git").exists() && !to.join(".papercomposer/original").exists());
        assert!(from.join("export/ellrin-paper.pdf").is_file(), "the original is untouched");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_shell_files_are_served_and_the_page_loads_them_in_order() {
        assert_eq!(lore_shell::FILES.iter().map(|f| f.0).collect::<Vec<_>>(), ["/icons.js", "/base.css", "/shell.js", "/makedeck.js", "/makebook.js", "/polish.js", "/findfile.js"]);
        let ui = Paths::from_repo().ui;
        assert!(!ui.join("icons.js").exists(), "the app's own icons.js is gone (commons/ui/icons.js is served)");
        let index = std::fs::read_to_string(ui.join("index.html")).unwrap();
        let at = |s: &str| index.find(s).unwrap_or_else(|| panic!("{s}"));
        assert!(at("href=\"theme.css\"") < at("href=\"base.css\"") && at("href=\"base.css\"") < at("href=\"styles.css\""));
        assert!(at("src=\"theme.js\"") < at("src=\"icons.js\"") && at("src=\"icons.js\"") < at("src=\"shell.js\"") && at("src=\"shell.js\"") < at("src=\"makedeck.js\"") && at("src=\"makedeck.js\"") < at("src=\"app.js\""));
        assert!(at("src=\"shell.js\"") < at("src=\"findfile.js\"") && at("src=\"findfile.js\"") < at("src=\"draft.js\""), "搜尋檔案 (LoreFindFile) before the 討論草稿");
        // no native confirm() anywhere: LoreShell.ask
        for f in std::fs::read_dir(&ui).unwrap().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "js") {
                let src = std::fs::read_to_string(&p).unwrap();
                assert!(!src.contains("confirm("), "{}: native confirm()", p.display());
            }
        }
    }
}
