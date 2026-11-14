//! Figures and tables as a paper (or a book: the same document) gets them: the file in
//! `figures/` under a name not taken, a `\label` from the caption that the document does not use
//! yet, the caption above or below as the journal says, the packages loaded in main.tex. One path
//! for the editor's 新增圖… and 繪圖… (the page asks `POST /api/papers/{name}/figure-block`) and
//! for 寫書's generated chapters, so labels, `\ref` targets and the notation checks are identical.
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/papers/{name}/figure-block | `{path, caption, stem?}` → `{label, block, added}`: the figure environment for a file already in `figures/`; `added` = packages put in main.tex |
//! | POST | /api/papers/{name}/figure-replace | `{label, path, caption?}` → `{label, file, added}`: the figure put where its planned stand-in is, same label (圖表規劃) |
//! | GET | /api/papers/{name}/stand-ins | the planned stand-ins in the paper `[{label, id, kind, caption, file}]` |

use super::*;

/// Where the journal puts captions (`[captions]` of its template); a book or a paper without a
/// journal: below.
fn caption_above(ws: Option<&Path>, paper: &Path, kind: &str) -> bool {
    let Some(ws) = ws else { return false };
    let Some(j) = pc_core::meta::read_meta(paper).journal else { return false };
    let Some(journal) = pc_core::journal::load(&ws.join("publishers"), &j) else { return false };
    let c = &journal.meta.captions;
    let v = if kind == "table" { &c.table } else { &c.figure };
    v.as_deref() == Some("above")
}

/// `figures/<stem>.<ext>` not used by the document or on disk yet.
pub fn free_figure_name(paper: &Path, stem: &str, ext: &str) -> String {
    let stem: String = stem.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let stem = stem.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let stem: String = stem.chars().take(40).collect();
    let stem = if stem.is_empty() { "figure".to_string() } else { stem };
    let text: String = pc_core::open::source_files(paper).iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect::<Vec<_>>().join("\n");
    (1..)
        .map(|k| if k == 1 { format!("{stem}.{ext}") } else { format!("{stem}-{k}.{ext}") })
        .find(|n| !paper.join("figures").join(n).exists() && !text.contains(&format!("figures/{n}")))
        .expect("a free name")
}

/// Write a figure file into `figures/` (新增圖…'s upload). `name` must be a plain file name.
pub fn save_figure_file(paper: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
    let figs = paper.join("figures");
    std::fs::create_dir_all(&figs).map_err(|e| e.to_string())?;
    std::fs::write(figs.join(name), bytes).map_err(|e| e.to_string())?;
    Ok(format!("figures/{name}"))
}

/// A float's label from its caption, not used in the document (nor in `also`: labels of text
/// not written yet).
pub fn new_label(paper: &Path, prefix: &str, caption: &str, also: &[String]) -> String {
    let mut taken = pc_core::paper::labels(paper);
    taken.extend(also.iter().cloned());
    let n = taken.iter().filter(|l| l.starts_with(&format!("{prefix}:"))).count();
    float_label(prefix, caption, n, &taken)
}

/// The figure environment for `path` (in `figures/`) and its label; graphicx loaded.
pub fn figure_block(ws: Option<&Path>, paper: &Path, path: &str, caption: &str, words: &str, also: &[String]) -> Result<(String, String, Vec<String>), String> {
    let label = new_label(paper, "fig", if words.trim().is_empty() { caption } else { words }, also);
    let block = lore_plot::figure::figure_block(path, caption, &label, caption_above(ws, paper, "figure"));
    let added = paper::ensure_packages(paper, &["graphicx".to_string()])?;
    Ok((label, block, added))
}

/// A table from a grid (first row = header) in the paper's table style, and its label; the
/// packages it needs loaded. The same render as the editor's 新增表….
pub fn table_block(ws: Option<&Path>, paper: &Path, grid: &[Vec<String>], caption: &str, also: &[String]) -> Result<(String, String, Vec<String>), String> {
    let main = std::fs::read_to_string(pc_core::open::main_file(paper)).unwrap_or_default();
    let journal = pc_core::meta::read_meta(paper).journal;
    let want = journal_table_style(&main, journal.as_deref());
    let styles = pc_core::textable::builtin_styles();
    let style = styles.iter().find(|s| s.id == want).or_else(|| styles.first()).ok_or("沒有表格樣式")?;
    let label = new_label(paper, "tab", caption, also);
    let mut t = pc_core::textable::from_grid(grid, caption, &label, style);
    t.caption_below = !caption_above(ws, paper, "table");
    let added = paper::ensure_packages(paper, &pc_core::textable::needed_packages(&t))?;
    Ok((label, pc_core::textable::render(&t), added))
}

#[derive(Deserialize)]
pub(crate) struct BlockBody {
    path: String,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    stem: String,
}

pub(crate) async fn figure_block_route(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<BlockBody>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    if !b.path.starts_with("figures/") || b.path.contains("..") || !figure_name_ok(b.path.trim_start_matches("figures/")) {
        return bad("圖檔路徑不對");
    }
    let ws = s.loaded.read().await.workspace.clone();
    let words = if b.caption.trim().is_empty() { b.stem.clone() } else { b.caption.clone() };
    match figure_block(ws.as_deref(), &dir, &b.path, &b.caption, &words, &[]) {
        Ok((label, block, added)) => Json(serde_json::json!({ "label": label, "block": block, "added": added })).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
pub(crate) struct ReplaceBody {
    /// The stand-in's label (kept: the drafts' \ref stays valid).
    label: String,
    path: String,
    /// Empty: the stand-in's caption.
    #[serde(default)]
    caption: String,
}

/// The real figure (from 流程圖 or 繪圖) where its planned stand-in is, with the same label.
pub(crate) async fn replace_stand_in_route(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ReplaceBody>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    if !b.path.starts_with("figures/") || b.path.contains("..") || !figure_name_ok(b.path.trim_start_matches("figures/")) {
        return bad("圖檔路徑不對");
    }
    let Some(si) = pc_core::draft::figures::stand_ins(&dir).into_iter().find(|x| x.label == b.label && x.kind == "figure") else { return bad("論文裡沒有這個預留的圖") };
    let ws = s.loaded.read().await.workspace.clone();
    let caption = if b.caption.trim().is_empty() { si.caption.clone() } else { b.caption.trim().to_string() };
    let block = lore_plot::figure::figure_block(&b.path, &caption, &si.label, caption_above(ws.as_deref(), &dir, "figure"));
    let added = match paper::ensure_packages(&dir, &["graphicx".to_string()]) { Ok(a) => a, Err(e) => return bad(&e) };
    match pc_core::draft::figures::replace_stand_in(&dir, &si.label, &block) {
        Ok(file) => Json(serde_json::json!({ "label": si.label, "file": file, "added": added })).into_response(),
        Err(e) => bad(&e),
    }
}

/// The paper's stand-ins (planned figures and tables not made yet).
pub(crate) async fn stand_ins_route(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    Json(pc_core::draft::figures::stand_ins(&dir)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_labels_are_free_and_blocks_load_their_packages() {
        let d = std::env::temp_dir().join(format!("pc-floats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(d.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/a}\n\\end{document}\n").unwrap();
        std::fs::write(d.join("sections/a.tex"), "\\section{A}\\label{sec:a}\n\nSee \\includegraphics{figures/loss-curve.pdf}.\n\n\\label{fig:loss-curve-shape}\n").unwrap();
        assert_eq!(free_figure_name(&d, "Loss curve", "pdf"), "loss-curve-2.pdf");
        assert_eq!(save_figure_file(&d, "x.png", b"png").unwrap(), "figures/x.png");
        assert_eq!(free_figure_name(&d, "x", "png"), "x-2.png");
        let (label, block, added) = figure_block(None, &d, "figures/x.png", "Loss curve shape", "", &[]).unwrap();
        assert_eq!(label, "fig:loss-curve-shape-2");
        assert!(block.contains("\\includegraphics[width=\\linewidth]{figures/x.png}") && block.contains("\\label{fig:loss-curve-shape-2}"), "{block}");
        assert_eq!(added, ["graphicx"]);
        let (tl, tb, _) = table_block(None, &d, &[vec!["a".into(), "b".into()], vec!["1".into(), "2".into()]], "Two numbers", &["tab:two-numbers".into()]).unwrap();
        assert_eq!(tl, "tab:two-numbers-2");
        assert!(tb.contains("\\begin{table}") && tb.contains("\\label{tab:two-numbers-2}"), "{tb}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
