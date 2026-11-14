//! A journal's (or proceedings') own open-access articles, for learning how it writes
//! (PaperComposer's 期刊寫法): find the venue as an OpenAlex *source*, then list its recent
//! open-access articles with every location they are copied at (publisher, PubMed Central,
//! arXiv, …) and each copy's own licence.
//!
//! Only the venue's name (or ISSN) and its OpenAlex id are ever sent; nothing about the
//! caller's own paper. The optional `mailto` follows [`crate::Config::mailto`] (off by default).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Client, Error, Record, Source, openalex, str_of};

/// One OpenAlex source (a journal, a conference series, a repository).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceInfo {
    /// The short id (`S1234567`), without `https://openalex.org/`.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issn_l: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issn: Vec<String>,
    /// The host organisation (publisher) as OpenAlex names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// OpenAlex's `type`: `journal`, `conference`, `repository`, `book series`, …
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default)]
    pub works_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}

/// One copy of a work.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Location {
    pub is_oa: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing: Option<String>,
    /// This copy's licence (`cc-by`, …); None = not stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// `publishedVersion`, `acceptedVersion`, `submittedVersion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Where the copy is (`journal`, `repository`, …) and its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

impl Location {
    /// An arXiv copy (its licence is arXiv's to state, see [`Client::arxiv_license`]).
    pub fn is_arxiv(&self) -> bool {
        [&self.pdf_url, &self.landing].iter().any(|u| u.as_deref().is_some_and(|u| u.contains("arxiv.org/")))
    }
    /// A PubMed Central / Europe PMC copy.
    pub fn is_pmc(&self) -> bool {
        [&self.pdf_url, &self.landing].iter().any(|u| u.as_deref().is_some_and(|u| u.contains("ncbi.nlm.nih.gov/pmc") || u.contains("europepmc.org") || u.contains("pmc.ncbi.nlm.nih.gov")))
    }
}

/// A work of the venue and every copy of it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Work {
    pub record: Record,
    /// `article`, `review`, `editorial`, `letter`, …
    #[serde(default)]
    pub kind: String,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub date: String,
    /// OpenAlex's keywords of the work (generic topic words).
    #[serde(default)]
    pub keywords: Vec<String>,
    pub locations: Vec<Location>,
}

impl Work {
    /// The copies worth downloading for the venue's own structure, best first: the
    /// publisher's published version, then PubMed Central, other repositories' published
    /// versions, accepted manuscripts, and arXiv last (a preprint may differ from the
    /// journal's version). Open copies with a PDF only; repeats dropped.
    pub fn pdf_copies(&self) -> Vec<&Location> {
        let mut v: Vec<&Location> = self.locations.iter().filter(|l| l.is_oa && l.pdf_url.is_some()).collect();
        let rank = |l: &Location| -> u8 {
            let published = l.version.as_deref() == Some("publishedVersion");
            match () {
                _ if l.is_arxiv() => 5,
                _ if published && l.host_kind.as_deref() == Some("journal") => 0,
                _ if l.is_pmc() => 1,
                _ if published => 2,
                _ if l.version.as_deref() == Some("acceptedVersion") => 3,
                _ => 4,
            }
        };
        v.sort_by_key(|l| rank(l));
        let mut seen: Vec<&str> = Vec::new();
        v.retain(|l| {
            let u = l.pdf_url.as_deref().unwrap_or("");
            if seen.contains(&u) {
                return false;
            }
            seen.push(u);
            true
        });
        v
    }
}

/// What to list.
#[derive(Debug, Clone, PartialEq)]
pub struct VenueQuery {
    /// OpenAlex source id (`S1234567`).
    pub source_id: String,
    /// `YYYY-MM-DD`: published on or after.
    pub from_date: String,
    /// `article` (research articles) or `review`.
    pub kind: String,
    pub per_page: usize,
}

/// `/sources?search=` — the venue by its name (or an ISSN, which OpenAlex also matches).
pub fn sources_url(query: &str, limit: usize, mailto: Option<&str>) -> String {
    let mut p = vec![
        ("search", query.trim().to_string()),
        ("per-page", limit.clamp(1, 25).to_string()),
        ("select", "id,display_name,issn_l,issn,host_organization_name,type,works_count,homepage_url".to_string()),
    ];
    if let Some(m) = mailto.filter(|m| !m.trim().is_empty()) {
        p.push(("mailto", m.trim().to_string()));
    }
    crate::url_with(&format!("{}/sources", openalex::BASE), &p)
}

/// `/works?filter=…`: the venue's open-access works of one type since a date, newest first.
/// Every filter the venue profile uses is here, in one place.
pub fn works_url(q: &VenueQuery, mailto: Option<&str>) -> String {
    let id = short_id(&q.source_id);
    let filter = format!("primary_location.source.id:{id},is_oa:true,type:{},from_publication_date:{}", q.kind, q.from_date);
    let mut p = vec![
        ("filter", filter),
        ("sort", "publication_date:desc".to_string()),
        ("per-page", q.per_page.clamp(1, 100).to_string()),
        ("select", "id,doi,display_name,publication_year,publication_date,type,open_access,primary_location,best_oa_location,locations,keywords,cited_by_count".to_string()),
    ];
    if let Some(m) = mailto.filter(|m| !m.trim().is_empty()) {
        p.push(("mailto", m.trim().to_string()));
    }
    crate::url_with(&format!("{}/works", openalex::BASE), &p)
}

/// `https://openalex.org/S123` → `S123`.
pub fn short_id(id: &str) -> String {
    id.trim().rsplit('/').next().unwrap_or("").to_string()
}

pub fn parse_sources(v: &Value) -> Vec<SourceInfo> {
    v.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = short_id(&str_of(s.get("id"))?);
            let name = str_of(s.get("display_name"))?;
            Some(SourceInfo {
                id,
                name,
                issn_l: str_of(s.get("issn_l")),
                issn: s.get("issn").and_then(Value::as_array).into_iter().flatten().filter_map(|x| str_of(Some(x))).collect(),
                publisher: str_of(s.get("host_organization_name")),
                kind: str_of(s.get("type")).unwrap_or_default(),
                works_count: s.get("works_count").and_then(Value::as_u64).unwrap_or(0),
                homepage: str_of(s.get("homepage_url")),
            })
        })
        .collect()
}

fn location(l: &Value) -> Location {
    Location {
        is_oa: l.get("is_oa").and_then(Value::as_bool).unwrap_or(false),
        pdf_url: str_of(l.get("pdf_url")),
        landing: str_of(l.get("landing_page_url")),
        license: str_of(l.get("license")),
        version: str_of(l.get("version")),
        host_kind: str_of(l.pointer("/source/type")),
        host: str_of(l.pointer("/source/display_name")),
    }
}

/// `{results: [work…]}` with every location.
pub fn parse_works(v: &Value) -> Vec<Work> {
    v.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let record = openalex::parse(w).into_iter().next()?;
            let mut locations: Vec<Location> = w.get("locations").and_then(Value::as_array).into_iter().flatten().map(location).collect();
            for k in ["best_oa_location", "primary_location"] {
                if let Some(l) = w.get(k).filter(|l| l.is_object()).map(location)
                    && !locations.iter().any(|x| x.pdf_url == l.pdf_url && x.landing == l.landing)
                {
                    locations.push(l);
                }
            }
            Some(Work {
                record,
                kind: str_of(w.get("type")).unwrap_or_default(),
                date: str_of(w.get("publication_date")).unwrap_or_default(),
                keywords: w.get("keywords").and_then(Value::as_array).into_iter().flatten().filter_map(|k| str_of(k.get("display_name"))).collect(),
                locations,
            })
        })
        .collect()
}

impl Client {
    /// OpenAlex sources matching a venue name or ISSN, best first.
    pub async fn find_sources(&self, query: &str, limit: usize) -> Result<Vec<SourceInfo>, Error> {
        let url = sources_url(query, limit, self.cfg.mailto.as_deref());
        Ok(self.json(Source::OpenAlex, url).await?.map(|v| parse_sources(&v)).unwrap_or_default())
    }

    /// The venue's open-access works (see [`works_url`]).
    pub async fn venue_works(&self, q: &VenueQuery) -> Result<Vec<Work>, Error> {
        let url = works_url(q, self.cfg.mailto.as_deref());
        Ok(self.json(Source::OpenAlex, url).await?.map(|v| parse_works(&v)).unwrap_or_default())
    }

    /// One copy's PDF into `dir` (same checks as [`Client::download`]).
    pub async fn download_copy(&self, loc: &Location, dir: &std::path::Path) -> Result<crate::Downloaded, Error> {
        let url = loc.pdf_url.clone().filter(|_| loc.is_oa).ok_or(Error::NotOpenAccess)?;
        let rec = Record { is_oa: true, pdf_url: Some(url), ..Default::default() };
        self.download(&rec, dir).await
    }
}
