//! Bringing papers into the reference library.
//!
//! The bibliographic record never comes from a model: the DOI is read from
//! the PDF (or typed), BibTeX comes from Crossref (doi.org content
//! negotiation), abstract, work type and venue from OpenAlex. The model only
//! writes the summary and one-liner and picks tags from the user's list.

use pc_core::bib::{self, Entry, Tag};
use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};
use std::path::Path;

const UA: &str = "PaperComposer (https://github.com/ellrin/PaperComposer)";

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("http client")
}

/// Bases are overridable so tests can use local stubs.
fn base(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}

/// The first DOI in a text: `10.<registrant>/<suffix>`.
pub fn doi_in_text(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("10.") {
        let start = from + i;
        from = start + 3;
        // must not be part of a longer number, e.g. "110.5"
        if start > 0 && lower.as_bytes()[start - 1].is_ascii_digit() {
            continue;
        }
        let rest = &text[start..];
        let reg: String = rest[3..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if reg.len() < 4 || !rest[3 + reg.len()..].starts_with('/') {
            continue;
        }
        // A DOI wrapped at "-", "/" or "." continues on the next line.
        let mut suffix = String::new();
        let mut chars = rest[4 + reg.len()..].chars().peekable();
        while let Some(c) = chars.next() {
            if c.is_whitespace() {
                let wrapped = c == '\n' && suffix.ends_with(['-', '/', '.']) && chars.peek().is_some_and(|n| n.is_ascii_alphanumeric());
                if wrapped { continue } else { break }
            }
            if "\"<>{}".contains(c) {
                break;
            }
            suffix.push(c);
        }
        let suffix = suffix.trim_end_matches(['.', ',', ';', ')', ']', ':']);
        if !suffix.is_empty() {
            return Some(format!("10.{reg}/{suffix}"));
        }
    }
    None
}

/// Text of a PDF's first two pages ("" when it cannot be read).
pub fn pdf_head(path: &Path) -> String {
    std::panic::catch_unwind(|| {
        let bytes = std::fs::read(path).ok()?;
        let pages = pdf_extract::extract_text_from_mem_by_pages(&bytes).ok()?;
        Some(pages.iter().take(2).cloned().collect::<Vec<_>>().join("\n"))
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// DOI from a PDF's first pages, or from its file name.
pub fn pdf_doi(path: &Path, head: &str) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().replace('_', "/");
    doi_in_text(head).or_else(|| doi_in_text(&name.replace(".pdf", "")))
}

/// Does the fetched title appear in the PDF? Most of its content words must
/// be on the first pages; a DOI cut short (e.g. the proceedings volume
/// instead of the chapter) fails this.
pub fn title_matches(title: &str, head: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>();
    let head = norm(head);
    let words: Vec<String> = norm(&bib::clean(title)).split_whitespace().filter(|w| w.len() > 3).map(String::from).collect();
    if words.is_empty() || head.trim().is_empty() {
        return true; // nothing to compare against
    }
    let hits = words.iter().filter(|w| head.contains(w.as_str())).count();
    hits * 10 >= words.len() * 7
}

/// Crossref's BibTeX for a DOI.
pub async fn crossref_bibtex(doi: &str) -> Result<String, String> {
    let url = format!("{}/{}", base("PAPERCOMPOSER_DOI_BASE", "https://doi.org"), doi);
    let r = http()
        .get(&url)
        .header("Accept", "application/x-bibtex; charset=utf-8")
        .send()
        .await
        .map_err(|e| format!("無法連線到 doi.org：{e}"))?;
    if !r.status().is_success() {
        return Err(format!("doi.org 找不到這個 DOI（{}）", r.status()));
    }
    let text = r.text().await.map_err(|e| e.to_string())?;
    if !text.trim_start().starts_with('@') {
        return Err("doi.org 沒有回傳 BibTeX".into());
    }
    Ok(text)
}

#[derive(Debug, Default, Serialize)]
pub struct Work {
    pub abstract_text: String,
    /// OpenAlex work type: article, review, preprint, ...
    pub kind: String,
    pub venue: String,
    /// journal, conference, repository, ...
    pub venue_type: String,
}

#[derive(Deserialize)]
struct OaWork {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    abstract_inverted_index: Option<std::collections::HashMap<String, Vec<usize>>>,
    #[serde(default)]
    primary_location: Option<OaLocation>,
}
#[derive(Deserialize)]
struct OaLocation {
    #[serde(default)]
    source: Option<OaSource>,
}
#[derive(Deserialize)]
struct OaSource {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// Abstract, type and venue from OpenAlex; empty when it does not know the DOI.
pub async fn openalex(doi: &str) -> Work {
    let url = format!("{}/works/doi:{}", base("PAPERCOMPOSER_OPENALEX_BASE", "https://api.openalex.org"), doi);
    let Ok(r) = http().get(&url).send().await else { return Work::default() };
    let Ok(w) = r.json::<OaWork>().await else { return Work::default() };
    let abstract_text = w
        .abstract_inverted_index
        .map(|idx| {
            let mut words: Vec<(usize, String)> = idx.into_iter().flat_map(|(w, pos)| pos.into_iter().map(move |p| (p, w.clone()))).collect();
            words.sort();
            words.into_iter().map(|(_, w)| w).collect::<Vec<_>>().join(" ")
        })
        .unwrap_or_default();
    let src = w.primary_location.and_then(|l| l.source);
    Work {
        abstract_text,
        kind: w.kind.unwrap_or_default(),
        venue: src.as_ref().and_then(|s| s.display_name.clone()).unwrap_or_default(),
        venue_type: src.and_then(|s| s.kind).unwrap_or_default(),
    }
}

#[derive(Deserialize)]
struct S2Paper {
    #[serde(default, rename = "abstract")]
    abstract_text: Option<String>,
}

/// Abstract from Semantic Scholar, for DOIs OpenAlex has none for.
pub async fn semantic_scholar_abstract(doi: &str) -> String {
    let url = format!("{}/graph/v1/paper/DOI:{}?fields=abstract", base("PAPERCOMPOSER_S2_BASE", "https://api.semanticscholar.org"), doi);
    let Ok(r) = http().get(&url).send().await else { return String::new() };
    r.json::<S2Paper>().await.ok().and_then(|p| p.abstract_text).unwrap_or_default()
}

/// The "Abstract" paragraph from a PDF's first page, when it has one.
pub fn pdf_abstract(head: &str) -> String {
    let lower = head.to_lowercase();
    let Some(i) = lower.find("abstract") else { return String::new() };
    let rest = &head[i + "abstract".len()..];
    let rest = rest.trim_start_matches([':', '.', '—', '-', ' ', '\n']);
    // stop at the next heading-like marker
    let end = ["\nkeywords", "\nkey words", "\nindex terms", "\n1 introduction", "\n1. introduction", "\nintroduction"]
        .iter()
        .filter_map(|m| rest.to_lowercase().find(m))
        .min()
        .unwrap_or(rest.len().min(2000));
    rest[..end.min(rest.len())].split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── search: Claude plans and ranks, OpenAlex supplies the papers ──

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub doi: String,
    pub title: String,
    pub authors: String,
    pub year: Option<u32>,
    pub venue: String,
    pub venue_type: String,
    pub cited_by: u64,
    pub kind: String,
    pub oa_url: Option<String>,
    pub abstract_text: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Deserialize)]
struct OaList {
    #[serde(default)]
    results: Vec<OaHit>,
}
#[derive(Deserialize)]
struct OaHit {
    #[serde(default)]
    doi: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    publication_year: Option<u32>,
    #[serde(default)]
    cited_by_count: u64,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    primary_location: Option<OaLocation>,
    #[serde(default)]
    authorships: Vec<OaAuthorship>,
    #[serde(default)]
    open_access: Option<OaAccess>,
    #[serde(default)]
    abstract_inverted_index: Option<std::collections::HashMap<String, Vec<usize>>>,
}
#[derive(Deserialize)]
struct OaAuthorship {
    #[serde(default)]
    author: Option<OaAuthor>,
}
#[derive(Deserialize)]
struct OaAuthor {
    #[serde(default)]
    display_name: Option<String>,
}
#[derive(Deserialize)]
struct OaAccess {
    #[serde(default)]
    oa_url: Option<String>,
}

fn unindex(idx: Option<std::collections::HashMap<String, Vec<usize>>>) -> String {
    idx.map(|idx| {
        let mut words: Vec<(usize, String)> = idx.into_iter().flat_map(|(w, pos)| pos.into_iter().map(move |p| (p, w.clone()))).collect();
        words.sort();
        words.into_iter().map(|(_, w)| w).collect::<Vec<_>>().join(" ")
    })
    .unwrap_or_default()
}

/// How the tags the user picked steer the database query.
pub struct Steer {
    pub review_only: bool,
    pub recent_years: Option<u32>,
    pub most_cited: bool,
}

impl Steer {
    pub fn from_tags(tags: &[String]) -> Steer {
        let has = |t: &str| tags.iter().any(|x| x == t);
        Steer {
            review_only: has("review"),
            recent_years: (has("new_algorithm") || has("new_application") || has("sota")).then_some(3),
            most_cited: has("classical_algorithm") || has("traditional_application"),
        }
    }
}

/// One OpenAlex search. Preprint-only works (repository sources) are dropped.
pub async fn openalex_search(query: &str, steer: &Steer, this_year: u32) -> Result<Vec<Candidate>, String> {
    let mut filters = vec!["has_doi:true".to_string()];
    if steer.review_only {
        filters.push("type:review".into());
    }
    if let Some(y) = steer.recent_years {
        filters.push(format!("from_publication_date:{}-01-01", this_year.saturating_sub(y)));
    }
    let mut url = reqwest::Url::parse(&format!("{}/works", base("PAPERCOMPOSER_OPENALEX_BASE", "https://api.openalex.org"))).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("search", query)
        .append_pair("filter", &filters.join(","))
        .append_pair("per-page", "12")
        .append_pair("select", "doi,title,publication_year,cited_by_count,type,primary_location,authorships,open_access,abstract_inverted_index");
    if steer.most_cited {
        url.query_pairs_mut().append_pair("sort", "cited_by_count:desc");
    }
    let r = http().get(url).send().await.map_err(|e| format!("無法連線到 OpenAlex：{e}"))?;
    if !r.status().is_success() {
        return Err(format!("OpenAlex 回傳 {}", r.status()));
    }
    let list: OaList = r.json().await.map_err(|e| format!("OpenAlex 回覆無法讀取：{e}"))?;
    Ok(list
        .results
        .into_iter()
        .filter_map(|h| {
            let doi = h.doi?.trim_start_matches("https://doi.org/").to_string();
            let src = h.primary_location.and_then(|l| l.source);
            let venue_type = src.as_ref().and_then(|s| s.kind.clone()).unwrap_or_default();
            if venue_type == "repository" {
                return None; // only on arXiv and the like
            }
            let names: Vec<String> = h.authorships.iter().filter_map(|a| a.author.as_ref()?.display_name.clone()).collect();
            let authors = match names.len() {
                0 => String::new(),
                1 => names[0].clone(),
                _ => format!("{} et al.", names[0].split_whitespace().last().unwrap_or(&names[0])),
            };
            Some(Candidate {
                doi,
                title: h.title.unwrap_or_default(),
                authors,
                year: h.publication_year,
                venue: src.and_then(|s| s.display_name).unwrap_or_default(),
                venue_type,
                cited_by: h.cited_by_count,
                kind: h.kind.unwrap_or_default(),
                oa_url: h.open_access.and_then(|o| o.oa_url),
                abstract_text: unindex(h.abstract_inverted_index),
                reason: String::new(),
            })
        })
        .collect())
}

/// The reply `queries_prompt` asks for.
pub fn queries_schema() -> serde_json::Value {
    use pc_core::compose::{json_array, json_object, json_string};
    json_object(&[("queries", json_array(json_string()))])
}

/// The reply `rank_prompt` asks for.
pub fn rank_schema() -> serde_json::Value {
    use pc_core::compose::{json_array, json_object, json_string};
    json_object(&[("picks", json_array(json_object(&[("id", serde_json::json!({ "type": "integer" })), ("reason", json_string())])))])
}

/// The reply `notes_prompt` asks for.
pub fn notes_schema() -> serde_json::Value {
    use pc_core::compose::{json_array, json_object, json_string};
    json_object(&[("summary", json_string()), ("one_liner", json_string()), ("tags", json_array(json_string()))])
}

pub fn queries_prompt(request: &str, steer_labels: &[String]) -> Prompt {
    Prompt {
        system: "You turn a researcher's request into searches for a scholarly database (OpenAlex full-text search). \
Reply with JSON only: {\"queries\": [\"...\"]} with 2 to 4 short English keyword queries of 3 to 8 words each, \
covering different phrasings of the topic. No boolean operators, no quotes."
            .into(),
        user: format!("Request:\n{request}\n\nWanted: {}\n", if steer_labels.is_empty() { "any".to_string() } else { steer_labels.join(", ") }),
    }
}

pub fn parse_queries(reply: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Q {
        queries: Vec<String>,
    }
    let (Some(a), Some(b)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    serde_json::from_str::<Q>(&reply[a..=b]).map(|q| q.queries.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(4).collect()).unwrap_or_default()
}

pub fn rank_prompt(request: &str, cands: &[Candidate]) -> Prompt {
    let mut user = format!("Request:\n{request}\n\nCandidates:\n");
    for (i, c) in cands.iter().enumerate() {
        let abs: String = c.abstract_text.chars().take(320).collect();
        user += &format!("[{i}] {} ({}; {}; cited {}; {})\n    {}\n", c.title, c.year.map(|y| y.to_string()).unwrap_or_default(), c.venue, c.cited_by, c.kind, abs);
    }
    Prompt {
        system: "You pick the papers that best fit a researcher's request. Choose only from the numbered candidates; never add, rename or invent papers. \
Reply with JSON only: {\"picks\": [{\"id\": 0, \"reason\": \"one sentence in Traditional Chinese (繁體中文) on why it fits\"}]}, best first, at most 8."
            .into(),
        user,
    }
}

/// The ranked candidates; ids outside the list are ignored.
pub fn apply_ranking(reply: &str, cands: &[Candidate]) -> Option<Vec<Candidate>> {
    #[derive(Deserialize)]
    struct Pick {
        id: usize,
        #[serde(default)]
        reason: String,
    }
    #[derive(Deserialize)]
    struct R {
        picks: Vec<Pick>,
    }
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    let r: R = serde_json::from_str(&reply[a..=b]).ok()?;
    let mut seen = std::collections::HashSet::new();
    Some(
        r.picks
            .into_iter()
            .filter(|p| p.id < cands.len() && seen.insert(p.id))
            .take(8)
            .map(|p| Candidate { reason: p.reason, ..cands[p.id].clone() })
            .collect(),
    )
}

/// `ronneberger2015unet`: first author's surname, year, first content word.
pub fn make_key(e: &Entry, taken: &[String]) -> String {
    const SKIP: &[&str] = &["a", "an", "the", "on", "of", "for", "in", "to", "and", "with", "towards", "toward", "via"];
    let ascii = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    let author = e.get("author").unwrap_or("").split(" and ").next().unwrap_or("").to_string();
    let surname = match author.split_once(',') {
        Some((last, _)) => last.to_string(),
        None => author.split_whitespace().last().unwrap_or("").to_string(),
    };
    // words split on spaces only, so "U-Net" gives "unet"
    let word = bib::clean(e.get("title").unwrap_or(""))
        .split_whitespace()
        .map(ascii)
        .find(|w| w.len() > 1 && !SKIP.contains(&w.as_str()))
        .unwrap_or_default();
    let base = format!("{}{}{}", ascii(&surname), e.year(), word);
    let base = if base.is_empty() { "ref".to_string() } else { base };
    let mut key = base.clone();
    let mut n = 1;
    while taken.contains(&key) {
        n += 1;
        key = format!("{base}{}", (b'a' + (n - 2) as u8) as char);
    }
    key
}

/// Venue patterns that earn the top-venue tags, from profile/venues.csv
/// (`pattern,tag`), or a small default list. Matching is case-insensitive
/// on the venue name.
pub fn venue_tags(profile: &Path, venue: &str) -> Vec<String> {
    let rows: Vec<(String, String)> = std::fs::read_to_string(profile.join("venues.csv"))
        .ok()
        .map(|s| s.lines().skip(1).filter_map(|l| l.split_once(',')).map(|(p, t)| (p.trim().to_lowercase(), t.trim().to_string())).collect())
        .filter(|v: &Vec<(String, String)>| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_VENUES.iter().map(|(p, t)| (p.to_lowercase(), t.to_string())).collect());
    // The venue must start with the pattern ("Nature Medicine" for "nature"),
    // not merely contain it ("Lecture Notes in Computer Science" ≠ "science").
    let v = venue.to_lowercase();
    let v = v.trim();
    let mut out = Vec::new();
    for (p, t) in rows {
        // Long names are distinctive anywhere in the venue; short ones
        // ("nature", "cvpr") must start it or appear as "(CVPR)".
        let hit = !p.is_empty()
            && (if p.len() >= 15 { v.contains(p.as_str()) } else {
                v == p || v.strip_prefix(p.as_str()).is_some_and(|r| r.starts_with([' ', ':', ',', '–', '-'])) || v.contains(&format!("({p})"))
            });
        if hit && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

pub const DEFAULT_VENUES: &[(&str, &str)] = &[
    ("computer vision and pattern recognition", "top_conference"),
    ("international conference on computer vision", "top_conference"),
    ("european conference on computer vision", "top_conference"),
    ("neural information processing systems", "top_conference"),
    ("international conference on machine learning", "top_conference"),
    ("international conference on learning representations", "top_conference"),
    ("medical image computing and computer-assisted intervention", "top_conference"),
    ("miccai", "top_conference"),
    ("aaai conference", "top_conference"),
    ("cvpr", "top_conference"),
    ("iccv", "top_conference"),
    ("eccv", "top_conference"),
    ("neurips", "top_conference"),
    ("ieee transactions on medical imaging", "top_journal"),
    ("medical image analysis", "top_journal"),
    ("ieee transactions on pattern analysis and machine intelligence", "top_journal"),
    ("nature", "top_journal"),
    ("science", "top_journal"),
    ("radiology", "top_journal"),
    ("the lancet", "top_journal"),
];

/// Prompt for the summary, one-liner and tags. Facts come only from the
/// given record; tags only from the user's vocabulary.
pub fn notes_prompt(title: &str, venue: &str, year: &str, kind: &str, abstract_text: &str, tags: &[Tag]) -> Prompt {
    let system = "You write short notes for a researcher's reference library. Use only the title, venue and abstract given; \
do not add facts, numbers or claims that are not there. Reply with JSON only: \
{\"summary\": \"2-3 sentences in Traditional Chinese (繁體中文)\", \"one_liner\": \"one sentence in Traditional Chinese that starts with 這篇\", \"tags\": [\"tag ids from the list that clearly apply\"]}. \
Choose tags only from the list; leave out any you are unsure of."
        .to_string();
    let mut user = format!("Title: {title}\nVenue: {venue}\nYear: {year}\nType: {kind}\n\nAbstract:\n{}\n\nTags:\n", if abstract_text.is_empty() { "(not available)" } else { abstract_text });
    for t in tags {
        user += &format!("- {}: {}\n", t.id, t.label);
    }
    Prompt { system, user }
}

#[derive(Debug, Default, Deserialize)]
pub struct Notes {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub one_liner: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub fn parse_notes(reply: &str, tags: &[Tag]) -> Option<Notes> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let mut n: Notes = serde_json::from_str(&reply[start..=end]).ok()?;
    n.tags.retain(|t| tags.iter().any(|x| &x.id == t));
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_dois() {
        assert_eq!(doi_in_text("https://doi.org/10.1007/978-3-319-24574-4_28.").as_deref(), Some("10.1007/978-3-319-24574-4_28"));
        assert_eq!(doi_in_text("DOI: 10.1109/TMI.2019.2959609, pp. 1-10").as_deref(), Some("10.1109/TMI.2019.2959609"));
        assert_eq!(doi_in_text("value 110.5/2 and 10.12/x").as_deref(), None, "registrant needs 4+ digits");
        assert_eq!(doi_in_text("no doi here"), None);
    }

    #[test]
    fn doi_wrapped_across_lines() {
        assert_eq!(doi_in_text("DOI: 10.1007/978-\n3-319-24574-4_28 Springer").as_deref(), Some("10.1007/978-3-319-24574-4_28"));
        assert_eq!(doi_in_text("doi 10.1109/TMI.2019.1\nNext line").as_deref(), Some("10.1109/TMI.2019.1"));
    }

    #[test]
    fn abstract_from_pdf_text() {
        let head = "Title\nAuthors\nAbstract. There is large consent that deep networks\nneed many samples.\nKeywords: segmentation\n1 Introduction";
        assert_eq!(pdf_abstract(head), "There is large consent that deep networks need many samples.");
    }

    #[test]
    fn ranking_only_uses_candidates() {
        let c = |t: &str| Candidate { doi: format!("10.1/{t}"), title: t.into(), authors: String::new(), year: Some(2020), venue: String::new(), venue_type: String::new(), cited_by: 0, kind: String::new(), oa_url: None, abstract_text: String::new(), reason: String::new() };
        let cands = vec![c("A"), c("B"), c("C")];
        let out = apply_ranking("{\"picks\":[{\"id\":2,\"reason\":\"r2\"},{\"id\":9,\"reason\":\"made up\"},{\"id\":2,\"reason\":\"dup\"},{\"id\":0,\"reason\":\"r0\"}]}", &cands).unwrap();
        assert_eq!(out.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["C", "A"]);
        assert_eq!(out[0].reason, "r2");
        assert_eq!(parse_queries("```json\n{\"queries\":[\"a b c\",\" \",\"d e f\"]}\n```"), vec!["a b c", "d e f"]);
        let st = Steer::from_tags(&["review".into(), "classical_algorithm".into()]);
        assert!(st.review_only && st.most_cited && st.recent_years.is_none());
    }

    #[test]
    fn title_check() {
        let head = "U-Net: Convolutional Networks for Biomedical\nImage Segmentation\nOlaf Ronneberger";
        assert!(title_matches("{U-Net}: Convolutional Networks for Biomedical Image Segmentation", head));
        assert!(!title_matches("Medical Image Computing and Computer-Assisted Intervention – MICCAI 2015: Proceedings, Part III", head));
    }

    #[test]
    fn makes_keys() {
        let (e, _) = bib::parse("@article{X, author={Ronneberger, Olaf and Fischer, P.}, title={{U-Net}: Convolutional Networks}, year={2015}}");
        assert_eq!(make_key(&e[0], &[]), "ronneberger2015unet");
        assert_eq!(make_key(&e[0], &["ronneberger2015unet".into()]), "ronneberger2015uneta");
        let (e, _) = bib::parse("@article{Y, author={Kaiming He}, title={On the Deep Residual Learning}, year={2016}}");
        assert_eq!(make_key(&e[0], &[]), "he2016deep");
    }

    #[test]
    fn tags_from_venues_and_model() {
        let dir = std::env::temp_dir();
        assert_eq!(venue_tags(&dir.join("nope"), "IEEE Transactions on Medical Imaging"), vec!["top_journal"]);
        assert!(venue_tags(&dir.join("nope"), "Some Workshop").is_empty());
        assert!(venue_tags(&dir.join("nope"), "Lecture Notes in Computer Science").is_empty(), "contains 'science' but is not Science");
        assert_eq!(venue_tags(&dir.join("nope"), "Nature Medicine"), vec!["top_journal"]);
        assert_eq!(venue_tags(&dir.join("nope"), "2016 IEEE Conference on Computer Vision and Pattern Recognition (CVPR)"), vec!["top_conference"]);
        assert_eq!(venue_tags(&dir.join("nope"), "Medical Image Computing and Computer-Assisted Intervention – MICCAI 2015"), vec!["top_conference"]);
        let tags = vec![Tag { id: "medical".into(), label: "醫學".into() }];
        let n = parse_notes("```json\n{\"summary\":\"s\",\"one_liner\":\"這篇…\",\"tags\":[\"medical\",\"invented\"]}\n```", &tags).unwrap();
        assert_eq!(n.tags, vec!["medical"], "only tags from the list");
    }
}

