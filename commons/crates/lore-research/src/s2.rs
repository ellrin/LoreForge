//! Semantic Scholar Graph API: search, title match, by DOI / arXiv id. An API key is optional
//! (the caller passes it from the keychain); without one S2 throttles hard, so 429 is expected.

use serde_json::Value;

use crate::{Record, Source, norm_arxiv, norm_doi, str_of};

pub const BASE: &str = "https://api.semanticscholar.org/graph/v1";
pub const FIELDS: &str = "title,authors,year,venue,externalIds,abstract,isOpenAccess,openAccessPdf,citationCount,url";

pub fn search_url(query: &str, limit: usize) -> String {
    crate::url_with(&format!("{BASE}/paper/search"), &[("query", query.to_string()), ("limit", limit.clamp(1, 100).to_string()), ("fields", FIELDS.into())])
}

pub fn match_url(title: &str) -> String {
    crate::url_with(&format!("{BASE}/paper/search/match"), &[("query", title.to_string()), ("fields", FIELDS.into())])
}

/// `DOI:<doi>` or `ARXIV:<id>`.
pub fn paper_url(id: &str) -> String {
    crate::url_with(&format!("{BASE}/paper/{id}"), &[("fields", FIELDS.into())])
}

/// `{ data: [paper…] }` or one paper.
pub fn parse(v: &Value) -> Vec<Record> {
    match v.get("data").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(paper).collect(),
        None => paper(v).into_iter().collect(),
    }
}

fn paper(p: &Value) -> Option<Record> {
    let title = str_of(p.get("title"))?;
    let pdf = str_of(p.pointer("/openAccessPdf/url"));
    let arxiv = str_of(p.pointer("/externalIds/ArXiv")).map(|a| norm_arxiv(&a));
    Some(Record {
        title,
        authors: p.get("authors").and_then(Value::as_array).into_iter().flatten().filter_map(|a| str_of(a.get("name"))).collect(),
        year: p.get("year").and_then(Value::as_i64).map(|y| y as i32),
        venue: str_of(p.get("venue")),
        doi: str_of(p.pointer("/externalIds/DOI")).map(|d| norm_doi(&d)),
        arxiv,
        abstract_text: str_of(p.get("abstract")),
        is_oa: p.get("isOpenAccess").and_then(Value::as_bool).unwrap_or(false) || pdf.is_some(),
        pdf_url: pdf,
        url: str_of(p.get("url")),
        source: Source::SemanticScholar,
        sources: vec![Source::SemanticScholar],
        citations: p.get("citationCount").and_then(Value::as_u64),
    })
}
