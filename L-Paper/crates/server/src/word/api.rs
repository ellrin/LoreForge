//! HTTP handlers for what the Word round trip leaves to the author
//! (review.js): the references read from Word's typed list, and checking
//! them on Crossref / PubMed when the author asks (never on import).
//!
//! | GET  | /api/papers/{name}/word-refs         | `{refs: [{key, kind, title, authors, year, venue, checked}]}` |
//! | POST | /api/papers/{name}/word-refs/confirm | check the unchecked ones; `{updated, ambiguous, missing, refs}` |

use crate::citecheck::{self, Guess};
use crate::{bad, paper_dir, Shared};
use axum::extract::{Path as AxPath, State};
use axum::response::IntoResponse;
use axum::Json;
use pc_core::bib::{self, Entry};
use std::path::Path;

/// Entries of the paper's bibliography that came from a Word list (they keep the text in `annote`).
fn word_entries(dir: &Path) -> (std::path::PathBuf, Vec<Entry>) {
    let path = dir.join("refs.bib");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    (path, bib::parse(&text).0)
}

fn from_word(e: &Entry) -> bool {
    e.get("annote").is_some() && e.key.starts_with("ref")
}

fn view(entries: &[Entry]) -> serde_json::Value {
    let refs: Vec<serde_json::Value> = entries
        .iter()
        .filter(|e| from_word(e))
        .map(|e| {
            serde_json::json!({
                "key": e.key, "kind": e.kind, "title": e.title(), "authors": e.authors_short(), "year": e.year(), "venue": e.venue(),
                "checked": e.get("x-checked").is_some(),
            })
        })
        .collect();
    serde_json::json!({ "refs": refs })
}

pub(crate) async fn list(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    Json(view(&word_entries(&dir).1)).into_response()
}

/// What we read from the typed reference, as the databases are asked.
pub fn guess(e: &Entry) -> Guess {
    Guess {
        title: e.title(),
        authors: bib::clean(e.get("author").unwrap_or("")).replace(" and ", ", "),
        year: e.year(),
        venue: e.venue(),
        volume: e.get("volume").unwrap_or("").into(),
        issue: e.get("number").unwrap_or("").into(),
        pages: e.get("pages").unwrap_or("").replace("--", "-"),
        doi: e.get("doi").unwrap_or("").into(),
        pmid: String::new(),
        arxiv: e.get("eprint").unwrap_or("").into(),
        note: String::new(),
    }
}

/// A found work's fields over the entry's (key, original text kept).
pub fn apply(e: &mut Entry, w: &citecheck::Work) {
    let annote = e.get("annote").map(String::from);
    let mut f = citecheck::entry_fields(w);
    f.remove("note");
    if e.kind == "inproceedings" {
        if let Some(j) = f.remove("journal") {
            f.insert("booktitle".into(), j);
        }
    } else if f.contains_key("journal") {
        e.kind = "article".into();
    }
    for k in ["title", "journal", "booktitle"] {
        if let Some(v) = f.get_mut(k) {
            *v = format!("{{{v}}}");
        }
    }
    e.fields = f;
    if let Some(a) = annote {
        e.fields.insert("annote".into(), a);
    }
    e.fields.insert("x-checked".into(), w.source.clone());
}

pub(crate) async fn confirm(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let (path, mut entries) = word_entries(&dir);
    let (mut updated, mut ambiguous, mut missing) = (vec![], vec![], vec![]);
    for e in entries.iter_mut().filter(|e| from_word(e) && e.get("x-checked").is_none()) {
        match citecheck::verify(&guess(e)).await {
            Ok(v) if v.status == "found" => {
                if let Some(w) = v.work {
                    apply(e, &w);
                    updated.push(e.key.clone());
                }
            }
            Ok(v) if v.status == "ambiguous" => ambiguous.push(e.key.clone()),
            Ok(_) => missing.push(e.key.clone()),
            Err(err) => return bad(&err),
        }
    }
    if !updated.is_empty() {
        if let Err(e) = pc_core::paper::write_atomic(&path, &bib::write(&entries)) {
            return bad(&e.to_string());
        }
    }
    let mut v = view(&entries);
    v["updated"] = serde_json::json!(updated);
    v["ambiguous"] = serde_json::json!(ambiguous);
    v["missing"] = serde_json::json!(missing);
    Json(v).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_found_work_replaces_the_typed_fields() {
        let text = crate::word::refs::bibtex(&["A. Writer, Folded widgets, Proceedings of Widget Day (2020) 1-10.".into()]);
        let mut e = bib::parse(&text).0.remove(0);
        assert!(from_word(&e));
        let g = guess(&e);
        assert_eq!((g.title.as_str(), g.authors.as_str(), g.year.as_str(), g.pages.as_str()), ("Folded widgets", "Writer, A.", "2020", "1-10"));
        let w = citecheck::Work { doi: "10.1/x".into(), title: "Folded Widgets".into(), names: vec!["Writer, Ann".into()], year: "2020".into(), venue: "Proc. Widget Day".into(), pages: "1-10".into(), source: "crossref".into(), ..Default::default() };
        apply(&mut e, &w);
        assert_eq!(e.kind, "inproceedings");
        assert_eq!((e.get("booktitle"), e.get("doi"), e.get("author"), e.get("x-checked")), (Some("{Proc. Widget Day}"), Some("10.1/x"), Some("Writer, Ann"), Some("crossref")));
        assert!(e.get("annote").is_some_and(|a| a.starts_with("A. Writer")));
    }
}
