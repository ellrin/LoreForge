//! Section files of a paper: add, delete (and bring back), reorder, and the
//! split that turns a typed `\section{…}` into a file of its own. The work
//! on main.tex and sections/ is pc_core::paper; here review.json follows:
//! a deleted section's tracked changes and comments are put aside with it
//! (and come back with it), a split moves them to the file their paragraph
//! went to. Renaming is a title change through PUT sections/{file}.
//!
//! Every reply carries the paper's new `stamp`, so the page does not take
//! its own change for one made elsewhere (open.js).

use super::*;
use pc_core::review::{self, Review};

/// review.json key holding the review of deleted sections, by parked file name.
const PARKED: &str = "parked_sections";

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/papers/{name}/section-ops/add", post(add))
        .route("/api/papers/{name}/section-ops/delete", post(delete))
        .route("/api/papers/{name}/section-ops/restore", post(restore))
        .route("/api/papers/{name}/section-ops/order", post(order))
        .route("/api/papers/{name}/section-ops/split", post(split))
}

fn done(dir: &Path, mut v: serde_json::Value) -> axum::response::Response {
    v["stamp"] = serde_json::Value::String(paper::stamp(dir));
    Json(v).into_response()
}

fn edit_review(dir: &Path, f: impl FnOnce(&mut Review)) {
    let mut r = review::load(dir);
    let before = r.clone();
    f(&mut r);
    if r != before {
        if let Err(e) = review::save(dir, &r) {
            eprintln!("review.json: {e}");
        }
    }
}

#[derive(Deserialize)]
struct Add {
    title: String,
    /// The section it follows; none = first.
    #[serde(default)]
    after: Option<String>,
}

async fn add(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Add>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _lock = REVIEW_LOCK.lock().await;
    match paper::add_section(&dir, &b.title, b.after.as_deref()) {
        Ok(file) => done(&dir, serde_json::json!({ "file": file })),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct Delete {
    file: String,
}

async fn delete(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Delete>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _lock = REVIEW_LOCK.lock().await;
    let (parked, after) = match paper::delete_section(&dir, &b.file) { Ok(x) => x, Err(e) => return bad(&e) };
    edit_review(&dir, |r| park(r, &b.file, &parked));
    done(&dir, serde_json::json!({ "parked": parked, "file": b.file, "after": after }))
}

/// Move the changes and comments of `file` aside, under the parked name.
fn park(r: &mut Review, file: &str, parked: &str) {
    let changes: Vec<_> = r.changes.iter().filter(|c| c.file == file).cloned().collect();
    let comments: Vec<_> = r.comments.iter().filter(|c| c.file == file).cloned().collect();
    if changes.is_empty() && comments.is_empty() {
        return;
    }
    r.changes.retain(|c| c.file != file);
    r.comments.retain(|c| c.file != file);
    let store = r.extra.entry(PARKED).or_insert_with(|| serde_json::json!({}));
    if !store.is_object() {
        *store = serde_json::json!({});
    }
    store[parked] = serde_json::json!({ "changes": changes, "comments": comments });
}

/// Bring the review of a parked section back, as `file`.
fn unpark(r: &mut Review, parked: &str, file: &str) {
    let Some(store) = r.extra.get_mut(PARKED).and_then(|v| v.as_object_mut()) else { return };
    let Some(v) = store.remove(parked) else { return };
    if store.is_empty() {
        r.extra.remove(PARKED);
    }
    let changes: Vec<review::Change> = serde_json::from_value(v["changes"].clone()).unwrap_or_default();
    let comments: Vec<review::Comment> = serde_json::from_value(v["comments"].clone()).unwrap_or_default();
    r.changes.extend(changes.into_iter().map(|mut c| { c.file = file.to_string(); c }));
    r.comments.extend(comments.into_iter().map(|mut c| { c.file = file.to_string(); c }));
}

#[derive(Deserialize)]
struct Restore {
    parked: String,
    file: String,
    #[serde(default)]
    after: Option<String>,
}

async fn restore(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Restore>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _lock = REVIEW_LOCK.lock().await;
    let file = match paper::restore_section(&dir, &b.parked, &b.file, b.after.as_deref()) { Ok(f) => f, Err(e) => return bad(&e) };
    edit_review(&dir, |r| unpark(r, &b.parked, &file));
    done(&dir, serde_json::json!({ "file": file }))
}

#[derive(Deserialize)]
struct Order {
    files: Vec<String>,
}

async fn order(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Order>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _lock = REVIEW_LOCK.lock().await;
    match paper::reorder_sections(&dir, &b.files) {
        Ok(()) => done(&dir, serde_json::json!({})),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct Split {
    file: String,
}

async fn split(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Split>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    if !paper::safe_name(&b.file) || !b.file.ends_with(".tex") {
        return bad("invalid section file");
    }
    let _lock = REVIEW_LOCK.lock().await;
    let Ok(text) = std::fs::read_to_string(dir.join("sections").join(&b.file)) else { return bad("找不到這個章節") };
    let cuts: Vec<usize> = paper::parse_section(&b.file, &text).paragraphs.iter().enumerate().filter(|(_, p)| paper::section_heading(p).is_some()).map(|(i, _)| i).collect();
    let made = match paper::split_section(&dir, &b.file) { Ok(m) => m, Err(e) => return bad(&e) };
    if !made.is_empty() {
        edit_review(&dir, |r| moved(r, &dir, &b.file, &cuts, &made));
    }
    done(&dir, serde_json::json!({ "files": made }))
}

/// After a split: paragraphs from `cuts[k]` on went to `made[k]` (the
/// heading paragraph itself became that file's title).
fn moved(r: &mut Review, dir: &Path, file: &str, cuts: &[usize], made: &[String]) {
    let place = |i: usize| -> Option<(String, usize)> {
        let k = cuts.iter().rposition(|&c| c <= i)?;
        Some((made[k].clone(), i.saturating_sub(cuts[k] + 1)))
    };
    for c in r.changes.iter_mut().filter(|c| c.file == file) {
        if let Some((f, i)) = place(c.index) {
            (c.file, c.index) = (f, i);
        }
    }
    for c in r.comments.iter_mut().filter(|c| c.file == file) {
        if let Some((f, i)) = place(c.index) {
            (c.file, c.index) = (f, i);
        }
    }
    for f in std::iter::once(file.to_string()).chain(made.iter().cloned()) {
        if let Ok(t) = std::fs::read_to_string(dir.join("sections").join(&f)) {
            review::reanchor(r, &f, &paper::parse_section(&f, &t).paragraphs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(file: &str, index: usize) -> review::Change {
        serde_json::from_value(serde_json::json!({ "id": format!("c{index}"), "author": "ellrin", "at": "2026-11-10T00:00:00Z", "file": file, "index": index, "before": null, "after": "x" })).unwrap()
    }

    #[test]
    fn a_deleted_sections_review_comes_back_with_it() {
        let mut r = Review { changes: vec![change("a.tex", 0), change("b.tex", 1)], ..Default::default() };
        park(&mut r, "a.tex", "a-2.tex");
        assert_eq!(r.changes.len(), 1);
        assert!(r.extra[PARKED]["a-2.tex"]["changes"].is_array());
        unpark(&mut r, "a-2.tex", "a.tex");
        assert_eq!(r.changes.iter().map(|c| c.file.as_str()).collect::<Vec<_>>(), ["b.tex", "a.tex"]);
        assert!(!r.extra.contains_key(PARKED), "nothing left aside");
    }

    #[test]
    fn a_split_moves_review_entries_with_their_paragraphs() {
        let dir = std::env::temp_dir().join(format!("pc-split-review-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Review { changes: vec![change("a.tex", 0), change("a.tex", 3), change("a.tex", 5)], ..Default::default() };
        // paragraphs 0 1 | 2 = \section{M}, 3 4 | 5 = \section{N}
        moved(&mut r, &dir, "a.tex", &[2, 5], &["m.tex".into(), "n.tex".into()]);
        let at: Vec<_> = r.changes.iter().map(|c| (c.file.as_str(), c.index)).collect();
        assert_eq!(at, [("a.tex", 0), ("m.tex", 0), ("n.tex", 0)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
