//! OpenAlex (`api.openalex.org/works`): search, by DOI, by title. The optional `mailto` is the
//! caller's setting (OpenAlex's "polite pool"); none is sent by default.

use serde_json::Value;

use crate::{Record, Source, arxiv_from_url, norm_doi, str_of};

pub const BASE: &str = "https://api.openalex.org";

fn with_mailto(mut params: Vec<(&'static str, String)>, mailto: Option<&str>) -> Vec<(&'static str, String)> {
    if let Some(m) = mailto.filter(|m| !m.trim().is_empty()) {
        params.push(("mailto", m.trim().to_string()));
    }
    params
}

pub fn search_url(query: &str, limit: usize, mailto: Option<&str>) -> String {
    crate::url_with(&format!("{BASE}/works"), &with_mailto(vec![("search", query.to_string()), ("per-page", limit.clamp(1, 50).to_string())], mailto))
}

/// `filter=title.search:` takes words: commas and colons would split the filter.
pub fn title_url(title: &str, mailto: Option<&str>) -> String {
    let words: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::url_with(&format!("{BASE}/works"), &with_mailto(vec![("filter", format!("title.search:{words}")), ("per-page", "5".into())], mailto))
}

pub fn doi_url(doi: &str, mailto: Option<&str>) -> String {
    crate::url_with(&format!("{BASE}/works/doi:{}", norm_doi(doi)), &with_mailto(Vec::new(), mailto))
}

/// `{ results: [work…] }` (a list) or one work.
pub fn parse(v: &Value) -> Vec<Record> {
    match v.get("results").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(work).collect(),
        None => work(v).into_iter().collect(),
    }
}

/// `abstract_inverted_index` `{word: [positions]}` back to text.
fn abstract_of(v: &Value) -> Option<String> {
    let idx = v.as_object()?;
    let mut words: Vec<(u64, &str)> = idx.iter().flat_map(|(w, ps)| ps.as_array().into_iter().flatten().filter_map(Value::as_u64).map(move |p| (p, w.as_str()))).collect();
    words.sort_by_key(|(p, _)| *p);
    let s = words.into_iter().map(|(_, w)| w).collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

fn work(w: &Value) -> Option<Record> {
    let title = str_of(w.get("display_name")).or_else(|| str_of(w.get("title")))?;
    let doi = str_of(w.get("doi")).or_else(|| str_of(w.pointer("/ids/doi"))).map(|d| norm_doi(&d)).filter(|d| !d.is_empty());
    let locations: Vec<&Value> = ["primary_location", "best_oa_location"].iter().filter_map(|k| w.get(*k)).chain(w.get("locations").and_then(Value::as_array).into_iter().flatten()).collect();
    let arxiv = locations
        .iter()
        .flat_map(|l| [str_of(l.get("landing_page_url")), str_of(l.get("pdf_url"))])
        .flatten()
        .find_map(|u| arxiv_from_url(&u));
    let is_oa = w.pointer("/open_access/is_oa").and_then(Value::as_bool).unwrap_or(false);
    let pdf_url = if is_oa {
        str_of(w.pointer("/best_oa_location/pdf_url")).or_else(|| locations.iter().filter(|l| l.get("is_oa").and_then(Value::as_bool) == Some(true)).find_map(|l| str_of(l.get("pdf_url"))))
    } else {
        None
    };
    Some(Record {
        title,
        authors: w
            .get("authorships")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| str_of(a.pointer("/author/display_name")))
            .collect(),
        year: w.get("publication_year").and_then(Value::as_i64).map(|y| y as i32),
        venue: str_of(w.pointer("/primary_location/source/display_name")),
        doi,
        arxiv,
        abstract_text: w.get("abstract_inverted_index").and_then(abstract_of),
        is_oa,
        pdf_url,
        url: str_of(w.pointer("/primary_location/landing_page_url")).or_else(|| str_of(w.get("id"))),
        source: Source::OpenAlex,
        sources: vec![Source::OpenAlex],
        citations: w.get("cited_by_count").and_then(Value::as_u64),
    })
}
