//! Find related papers on the web, open access only (`docs/book.md` §上網找論文).
//!
//! - [`Client::search`] asks OpenAlex, arXiv and Semantic Scholar in turn, politely (a pause
//!   per source, 429 retried with backoff), and returns one de-duplicated list of [`Record`]s
//!   plus the per-source errors — a source that fails does not fail the search.
//! - [`Client::resolve`] finds one reference by DOI, arXiv id or title.
//! - [`Client::download`] fetches an open-access PDF into a caller-given folder (the cache
//!   tier) as `<sha256>.pdf`, after checking it is a PDF and not too large. A record that is
//!   not open access is refused before any request: paywalled full text is never fetched.
//! - [`bibtex`]: an entry for every record, stable keys, unique within a set.
//! - [`reader::import_into_reader`]: downloaded files into LoreReader's library.
//!
//! No e-mail is sent unless the caller passes [`Config::mailto`]; the User-Agent is a generic
//! client string. HTTP goes through the [`Http`] trait: tests use [`MockHttp`], never the network.

pub mod arxiv;
pub mod bibtex;
pub mod http;
pub mod openalex;
pub mod reader;
pub mod s2;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub use http::{Http, Method, MockHttp, ReqwestHttp, Request, Response};
pub use reader::{ImportReport, Imported, import_into_reader};

pub const DEFAULT_USER_AGENT: &str = "LoreForge/0.1 (+https://github.com/ellrin/LoreForge)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    OpenAlex,
    Arxiv,
    #[serde(rename = "s2")]
    SemanticScholar,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::OpenAlex, Source::Arxiv, Source::SemanticScholar];
    pub fn label(self) -> &'static str {
        match self {
            Source::OpenAlex => "OpenAlex",
            Source::Arxiv => "arXiv",
            Source::SemanticScholar => "Semantic Scholar",
        }
    }
}

/// One paper, the same shape whichever source found it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    /// Bare, lower-case (`10.1234/abc`).
    pub doi: Option<String>,
    /// Without version (`2101.00001`).
    pub arxiv: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    /// Open access per the source (arXiv: always).
    pub is_oa: bool,
    /// An open-access PDF (OpenAlex best OA location, S2 `openAccessPdf`, arXiv PDF).
    pub pdf_url: Option<String>,
    /// Landing page.
    pub url: Option<String>,
    /// The source that found it first.
    pub source: Source,
    /// Every source that found it (after [`dedupe`]).
    pub sources: Vec<Source>,
    pub citations: Option<u64>,
    /// The open-access copy's licence as the source names it (`cc-by`, `cc-by-nc-nd`, …;
    /// OpenAlex `best_oa_location.license`, S2 `openAccessPdf.license`); None = not stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// May a figure of a paper under this licence be reproduced (with attribution) in the
/// author's own notes? Only CC BY, CC BY-SA, CC0 and public domain: NonCommercial and
/// NoDerivatives licences, publisher licences and unstated ones are redrawn or described instead.
pub fn figure_reuse_allowed(license: Option<&str>) -> bool {
    let Some(l) = license else { return false };
    let l = l.trim().to_ascii_lowercase().replace([' ', '_'], "-");
    let l = l.trim_start_matches("https://creativecommons.org/licenses/").trim_end_matches('/');
    matches!(l, "cc-by" | "cc-by-sa" | "cc0" | "public-domain" | "pd" | "publicdomain") || l.starts_with("cc-by-4") || l.starts_with("cc-by-3") || l.starts_with("cc-by-sa-") || l.starts_with("by/") || l.starts_with("by-sa/")
}

impl Record {
    /// Where the open-access PDF can be fetched, best first; empty when not open access.
    pub fn pdf_candidates(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(a) = &self.arxiv {
            v.push(arxiv::pdf_url(a));
        }
        if self.is_oa
            && let Some(u) = &self.pdf_url
                && !v.contains(u) {
                    v.push(u.clone());
                }
        v
    }
    pub fn open_access(&self) -> bool {
        !self.pdf_candidates().is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Connect / DNS / timeout: the job degrades ("沒有網路") instead of failing.
    #[error("沒有網路")]
    Offline(String),
    #[error("網路錯誤：{0}")]
    Network(String),
    #[error("{0} 要求放慢，稍後再試")]
    RateLimited(&'static str),
    #[error("{source_name} 回應 {status}")]
    Status { source_name: String, status: u16 },
    #[error("看不懂 {0} 的回應")]
    Parse(String),
    #[error("不是開放取用，不下載")]
    NotOpenAccess,
    #[error("下載的不是 PDF")]
    NotPdf,
    #[error("檔案太大（上限 {limit} bytes）")]
    TooLarge { limit: u64 },
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn is_offline(&self) -> bool {
        matches!(self, Error::Offline(_))
    }
}

/// Pauses between requests to one source, and the 429 backoff.
#[derive(Debug, Clone)]
pub struct Pace {
    pub openalex: Duration,
    pub arxiv: Duration,
    pub s2: Duration,
    /// First wait after a 429 (doubled each retry) when the answer has no `Retry-After`.
    pub backoff: Duration,
    /// Longest wait honoured from `Retry-After`.
    pub max_wait: Duration,
}

impl Default for Pace {
    /// arXiv asks for 3 s between calls; S2 without a key allows about one per second.
    fn default() -> Self {
        Self { openalex: Duration::from_millis(200), arxiv: Duration::from_secs(3), s2: Duration::from_millis(1100), backoff: Duration::from_secs(2), max_wait: Duration::from_secs(30) }
    }
}

impl Pace {
    /// No waiting (tests).
    pub fn none() -> Self {
        Self { openalex: Duration::ZERO, arxiv: Duration::ZERO, s2: Duration::ZERO, backoff: Duration::ZERO, max_wait: Duration::ZERO }
    }
    fn of(&self, s: Source) -> Duration {
        match s {
            Source::OpenAlex => self.openalex,
            Source::Arxiv => self.arxiv,
            Source::SemanticScholar => self.s2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub user_agent: String,
    /// OpenAlex's optional `mailto` (its "polite pool"). None by default: the author decides.
    pub mailto: Option<String>,
    /// Semantic Scholar's optional `x-api-key` (from the keychain, never from a file here).
    pub s2_api_key: Option<String>,
    pub sources: Vec<Source>,
    pub pace: Pace,
    /// Retries after a 429 / 503.
    pub retries: u32,
    pub max_pdf_bytes: u64,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.into(),
            mailto: None,
            s2_api_key: None,
            sources: Source::ALL.to_vec(),
            pace: Pace::default(),
            retries: 3,
            max_pdf_bytes: 80 * 1024 * 1024,
            timeout: Duration::from_secs(60),
        }
    }
}

/// What a search found, and which sources failed (with why).
#[derive(Debug, Default)]
pub struct Found {
    pub records: Vec<Record>,
    pub errors: Vec<(Source, Error)>,
}

impl Found {
    /// Every source failed for lack of network.
    pub fn offline(&self) -> bool {
        self.records.is_empty() && !self.errors.is_empty() && self.errors.iter().all(|(_, e)| e.is_offline())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Downloaded {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub url: String,
}

pub struct Client {
    http: Arc<dyn Http>,
    cfg: Config,
    last: tokio::sync::Mutex<HashMap<Source, Instant>>,
}

impl Client {
    /// Real HTTP (reqwest) with the config's User-Agent.
    pub fn new(cfg: Config) -> Self {
        let http = Arc::new(ReqwestHttp::new(&cfg.user_agent, cfg.timeout));
        Self::with_http(cfg, http)
    }
    pub fn with_http(cfg: Config, http: Arc<dyn Http>) -> Self {
        Self { http, cfg, last: tokio::sync::Mutex::new(HashMap::new()) }
    }
    pub fn config(&self) -> &Config {
        &self.cfg
    }
    pub fn http(&self) -> Arc<dyn Http> {
        self.http.clone()
    }

    /// Wait for the source's pause, send, retry 429 / 503 with backoff.
    async fn call(&self, src: Source, mut req: Request) -> Result<Option<Response>, Error> {
        if src == Source::SemanticScholar
            && let Some(k) = self.cfg.s2_api_key.as_deref().filter(|k| !k.is_empty()) {
                req = req.header("x-api-key", k);
            }
        let mut wait = self.cfg.pace.backoff;
        for attempt in 0..=self.cfg.retries {
            {
                let mut last = self.last.lock().await;
                if let Some(t) = last.get(&src) {
                    let gap = self.cfg.pace.of(src);
                    let since = t.elapsed();
                    if since < gap {
                        tokio::time::sleep(gap - since).await;
                    }
                }
                last.insert(src, Instant::now());
            }
            let r = self.http.send(req.clone()).await?;
            match r.status {
                200..=299 => return Ok(Some(r)),
                404 => return Ok(None),
                429 | 503 if attempt < self.cfg.retries => {
                    let after = r.header("retry-after").and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
                    tokio::time::sleep(after.unwrap_or(wait).min(self.cfg.pace.max_wait)).await;
                    wait *= 2;
                }
                429 => return Err(Error::RateLimited(src.label())),
                s => return Err(Error::Status { source_name: src.label().into(), status: s }),
            }
        }
        Err(Error::RateLimited(src.label()))
    }

    async fn json(&self, src: Source, url: String) -> Result<Option<Value>, Error> {
        match self.call(src, Request::get(url).header("accept", "application/json")).await? {
            None => Ok(None),
            Some(r) => serde_json::from_slice(&r.body).map(Some).map_err(|_| Error::Parse(src.label().into())),
        }
    }

    async fn atom(&self, url: String) -> Result<Vec<Record>, Error> {
        Ok(match self.call(Source::Arxiv, Request::get(url).header("accept", "application/atom+xml")).await? {
            None => Vec::new(),
            Some(r) => arxiv::parse(&r.text()),
        })
    }

    /// The licence arXiv states for paper `id` (see [`arxiv::oai_license`]); None when arXiv
    /// does not answer with a record. Use it over a source's `license` for anything on arXiv.
    pub async fn arxiv_license(&self, id: &str) -> Result<Option<String>, Error> {
        Ok(match self.call(Source::Arxiv, Request::get(arxiv::oai_url(id))).await? {
            None => None,
            Some(r) => arxiv::oai_license(&r.text()),
        })
    }

    /// One source's search.
    pub async fn search_in(&self, src: Source, query: &str, limit: usize) -> Result<Vec<Record>, Error> {
        let mailto = self.cfg.mailto.as_deref();
        Ok(match src {
            Source::OpenAlex => self.json(src, openalex::search_url(query, limit, mailto)).await?.map(|v| openalex::parse(&v)).unwrap_or_default(),
            Source::Arxiv => self.atom(arxiv::search_url(query, limit)).await?,
            Source::SemanticScholar => self.json(src, s2::search_url(query, limit)).await?.map(|v| s2::parse(&v)).unwrap_or_default(),
        })
    }

    /// Every configured source, de-duplicated. A source that fails is listed in `errors`; once
    /// one source is offline the others are not tried (no network is no network).
    pub async fn search(&self, query: &str, limit: usize) -> Found {
        let mut found = Found::default();
        let mut all = Vec::new();
        for &src in &self.cfg.sources {
            if found.errors.iter().any(|(_, e)| e.is_offline()) {
                found.errors.push((src, Error::Offline("skipped".into())));
                continue;
            }
            match self.search_in(src, query, limit).await {
                Ok(rs) => all.extend(rs),
                Err(e) => found.errors.push((src, e)),
            }
        }
        found.records = dedupe(all);
        found
    }

    /// One reference by DOI, arXiv id or title (whichever `reference` looks like); the sources'
    /// answers merged. `Ok(None)` = nobody knows it.
    pub async fn resolve(&self, reference: &str) -> Result<Option<Record>, Error> {
        let mut got = Vec::new();
        let mut last_err: Option<Error> = None;
        let mailto = self.cfg.mailto.as_deref();
        let has = |s| self.cfg.sources.contains(&s);
        let reference = reference.trim();
        let offline = |e: &Option<Error>| e.as_ref().is_some_and(Error::is_offline);
        let mut asks: Vec<(Source, Ask)> = Vec::new();
        if let Some(doi) = find_doi(reference) {
            asks.push((Source::OpenAlex, Ask::Json(openalex::doi_url(&doi, mailto))));
            asks.push((Source::SemanticScholar, Ask::Json(s2::paper_url(&format!("DOI:{doi}")))));
        } else if let Some(id) = find_arxiv(reference) {
            asks.push((Source::Arxiv, Ask::Atom(arxiv::id_url(&id))));
            asks.push((Source::SemanticScholar, Ask::Json(s2::paper_url(&format!("ARXIV:{id}")))));
        } else {
            for &src in &self.cfg.sources {
                asks.push((src, match src {
                    Source::OpenAlex => Ask::Json(openalex::title_url(reference, mailto)),
                    Source::Arxiv => Ask::Atom(arxiv::title_url(reference)),
                    Source::SemanticScholar => Ask::Json(s2::match_url(reference)),
                }));
            }
        }
        let by_title = find_doi(reference).is_none() && find_arxiv(reference).is_none();
        let want = norm_title(reference);
        for (src, ask) in asks {
            if !has(src) || offline(&last_err) {
                continue;
            }
            let r = match ask {
                Ask::Json(u) => self.json(src, u).await.map(|v| match (src, v) {
                    (_, None) => Vec::new(),
                    (Source::SemanticScholar, Some(v)) => s2::parse(&v),
                    (_, Some(v)) => openalex::parse(&v),
                }),
                Ask::Atom(u) => self.atom(u).await,
            };
            match r {
                Ok(v) => got.extend(v.into_iter().filter(|r| !by_title || title_close(&norm_title(&r.title), &want))),
                Err(e) => last_err = Some(e),
            }
        }
        let merged = dedupe(got);
        match (merged.into_iter().next(), last_err) {
            (Some(r), _) => Ok(Some(r)),
            (None, Some(e)) => Err(e),
            (None, None) => Ok(None),
        }
    }

    /// The record's open-access PDF into `dir` as `<sha256>.pdf` (kept if already there).
    /// Not open access → [`Error::NotOpenAccess`] without any request. Tries arXiv first, then
    /// the source's OA location; an HTML landing page instead of a PDF is [`Error::NotPdf`].
    pub async fn download(&self, rec: &Record, dir: &Path) -> Result<Downloaded, Error> {
        let urls = rec.pdf_candidates();
        if urls.is_empty() {
            return Err(Error::NotOpenAccess);
        }
        let mut err = Error::NotPdf;
        for url in urls {
            let src = if url.contains("arxiv.org/") { Source::Arxiv } else { Source::OpenAlex };
            let mut req = Request::get(&url).header("accept", "application/pdf");
            req.max_body = Some(self.cfg.max_pdf_bytes);
            match self.call(src, req).await {
                Ok(Some(r)) => match save_pdf(&r.body, dir, self.cfg.max_pdf_bytes) {
                    Ok((path, sha256)) => return Ok(Downloaded { path, sha256, bytes: r.body.len() as u64, url }),
                    Err(e) => err = e,
                },
                Ok(None) => err = Error::Status { source_name: url.clone(), status: 404 },
                Err(e) if e.is_offline() => return Err(e),
                Err(e) => err = e,
            }
        }
        Err(err)
    }
}

enum Ask {
    Json(String),
    Atom(String),
}

/// Check `%PDF-` and the size, write `<dir>/<sha256>.pdf` atomically.
pub fn save_pdf(bytes: &[u8], dir: &Path, max: u64) -> Result<(PathBuf, String), Error> {
    if bytes.len() as u64 > max {
        return Err(Error::TooLarge { limit: max });
    }
    // a few bytes of junk before the header are allowed by readers; HTML is not
    if !bytes.iter().take(1024).collect::<Vec<_>>().windows(5).any(|w| w.iter().map(|b| **b).eq(*b"%PDF-")) {
        return Err(Error::NotPdf);
    }
    let sha = hex(&Sha256::digest(bytes));
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{sha}.pdf"));
    if !path.exists() {
        let tmp = dir.join(format!(".{sha}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
    }
    Ok((path, sha))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Merge records that are the same paper (same DOI, arXiv id or normalised title): sources
/// joined, the first non-empty field kept, the highest citation count, an arXiv PDF preferred.
pub fn dedupe(records: Vec<Record>) -> Vec<Record> {
    let mut out: Vec<Record> = Vec::new();
    for r in records {
        let nt = norm_title(&r.title);
        let same = out.iter().position(|o| {
            (r.doi.is_some() && o.doi == r.doi) || (r.arxiv.is_some() && o.arxiv == r.arxiv) || (nt.len() >= 12 && norm_title(&o.title) == nt)
        });
        match same {
            None => out.push(r),
            Some(i) => {
                let o = &mut out[i];
                for s in r.sources {
                    if !o.sources.contains(&s) {
                        o.sources.push(s);
                    }
                }
                if o.authors.is_empty() {
                    o.authors = r.authors;
                }
                o.year = o.year.or(r.year);
                o.venue = o.venue.take().or(r.venue);
                o.license = o.license.take().or(r.license);
                o.doi = o.doi.take().or(r.doi);
                o.arxiv = o.arxiv.take().or(r.arxiv);
                o.abstract_text = o.abstract_text.take().or(r.abstract_text);
                o.url = o.url.take().or(r.url);
                o.citations = o.citations.max(r.citations);
                let arxiv_pdf = |u: &Option<String>| u.as_deref().is_some_and(|u| u.contains("arxiv.org/"));
                if r.is_oa && r.pdf_url.is_some() && (!o.is_oa || o.pdf_url.is_none() || (arxiv_pdf(&r.pdf_url) && !arxiv_pdf(&o.pdf_url))) {
                    o.pdf_url = r.pdf_url;
                }
                o.is_oa |= r.is_oa;
            }
        }
    }
    out
}

/// Lower-case letters and digits only.
pub fn norm_title(t: &str) -> String {
    t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn title_close(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a == b || (a.len().min(b.len()) >= 20 && (a.starts_with(b) || b.starts_with(a))))
}

/// `https://doi.org/10.1/X` / `doi:10.1/X` → `10.1/x`.
pub fn norm_doi(d: &str) -> String {
    let d = d.trim();
    let low = d.to_ascii_lowercase();
    let start = low.find("10.").unwrap_or(0);
    low[start..].trim_end_matches(['.', ',', ';']).to_string()
}

/// `arXiv:2101.00001v2` → `2101.00001`.
pub fn norm_arxiv(a: &str) -> String {
    let a = a.trim();
    let a = a.strip_prefix("arXiv:").or_else(|| a.strip_prefix("arxiv:")).unwrap_or(a);
    match a.rfind('v') {
        Some(i) if i > 0 && a[i + 1..].chars().all(|c| c.is_ascii_digit()) && !a[i + 1..].is_empty() && a[..i].ends_with(|c: char| c.is_ascii_digit()) => a[..i].to_string(),
        _ => a.to_string(),
    }
}

/// The arXiv id in an `arxiv.org/abs|pdf/<id>` URL.
pub fn arxiv_from_url(u: &str) -> Option<String> {
    let i = u.find("arxiv.org/")?;
    let rest = &u[i + "arxiv.org/".len()..];
    let id = rest.strip_prefix("abs/").or_else(|| rest.strip_prefix("pdf/"))?;
    let id = id.split(['?', '#']).next()?.trim_end_matches(".pdf").trim_end_matches('/');
    (!id.is_empty()).then(|| norm_arxiv(id))
}

/// A DOI in free text (`10.<digits>/<non-space>`).
pub fn find_doi(s: &str) -> Option<String> {
    let low = s.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = low[at..].find("10.") {
        let st = at + i;
        let rest = &low[st..];
        let reg: String = rest[3..].chars().take_while(char::is_ascii_digit).collect();
        if reg.len() >= 4 && rest[3 + reg.len()..].starts_with('/') {
            let tail: String = rest.chars().take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '<' | '>')).collect();
            let d = norm_doi(&tail);
            if d.len() > 3 + reg.len() + 1 {
                return Some(d);
            }
        }
        at = st + 3;
    }
    None
}

/// A new-style arXiv id (`2101.00001`, `arXiv:2101.00001v2`) or an `arxiv.org` link.
pub fn find_arxiv(s: &str) -> Option<String> {
    if let Some(a) = arxiv_from_url(s) {
        return Some(a);
    }
    for w in s.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '[' | ']')) {
        let w = w.strip_prefix("arXiv:").or_else(|| w.strip_prefix("arxiv:")).unwrap_or(w);
        let id = norm_arxiv(w);
        if let Some((a, b)) = id.split_once('.')
            && a.len() == 4 && a.chars().all(|c| c.is_ascii_digit()) && (4..=5).contains(&b.len()) && b.chars().all(|c| c.is_ascii_digit()) {
                return Some(id);
            }
    }
    None
}

/// `base?k=v&…` with the values form-encoded.
pub(crate) fn url_with(base: &str, params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return base.to_string();
    }
    match reqwest::Url::parse_with_params(base, params.iter().map(|(k, v)| (*k, v.as_str()))) {
        Ok(u) => u.to_string(),
        Err(_) => base.to_string(),
    }
}

pub(crate) fn str_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests;
