//! A pasted list of papers to cite, checked one by one.
//!
//! The model only splits the text into entries and copies what each one says
//! (title, authors, year, DOI, PMID, arXiv id, the user's remarks). Whether a
//! paper exists is decided by the databases: a DOI must resolve on Crossref
//! to a work with a matching title, an arXiv id must exist on arXiv, a PMID
//! on PubMed, or a search by title on Crossref and then PubMed must find it.
//! A paper none of them knows is reported, never invented; the user may still
//! file it from the bibliographic data they typed (marked as such).

use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Guess {
    #[serde(default)]
    pub title: String,
    /// As written, e.g. "Ancker JS, Edwards A, ...".
    #[serde(default)]
    pub authors: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub issue: String,
    #[serde(default)]
    pub pages: String,
    #[serde(default)]
    pub doi: String,
    #[serde(default)]
    pub pmid: String,
    #[serde(default)]
    pub arxiv: String,
    /// The user's own remarks about this paper (where it is cited, caveats).
    #[serde(default)]
    pub note: String,
}

/// The reply `extract_prompt` asks for.
pub fn extract_schema() -> serde_json::Value {
    use pc_core::compose::{json_array, json_object, json_string};
    let fields = ["title", "authors", "year", "venue", "volume", "issue", "pages", "doi", "pmid", "arxiv", "note"];
    let paper = json_object(&fields.iter().map(|f| (*f, json_string())).collect::<Vec<_>>());
    json_object(&[("papers", json_array(paper))])
}

pub fn extract_prompt(text: &str) -> Prompt {
    let system = "You split text that lists academic papers into one entry per paper. \
The text may be a numbered reference list, citations in any style, titles, DOIs, PMIDs, arXiv ids or notes, in any language. \
Headings and explanations between entries (e.g. \"B. Verified, not cited yet\") are not papers.\n\
For each paper copy what the text says into the fields: title, authors (as written), year, venue (journal or conference), volume, issue, pages, doi, pmid, arxiv (the id only, e.g. 1901.07042). \
Leave a field empty when the text does not give it. Never guess or complete a field from memory, and never add a paper that is not in the text.\n\
`note`: everything else the text says about that paper — status words, where it is cited, remarks, caveats — copied verbatim in the text's own language, the pieces joined with \" · \". Do not translate or summarise it; leave out the bare words \"PMID\", \"doi\" and \"arXiv\" when they only repeat an identifier.\n\
Reply with JSON only: {\"papers\":[{\"title\":\"\",\"authors\":\"\",\"year\":\"\",\"venue\":\"\",\"volume\":\"\",\"issue\":\"\",\"pages\":\"\",\"doi\":\"\",\"pmid\":\"\",\"arxiv\":\"\",\"note\":\"\"}]}"
        .to_string();
    Prompt { system, user: text.trim().to_string() }
}

pub fn parse_extract(reply: &str) -> Result<Vec<Guess>, String> {
    #[derive(Deserialize)]
    struct Reply {
        papers: Vec<Guess>,
    }
    let start = reply.find('{').ok_or("回覆裡沒有 JSON")?;
    let end = reply.rfind('}').ok_or("回覆裡沒有 JSON")?;
    let r: Reply = serde_json::from_str(&reply[start..=end]).map_err(|e| format!("看不懂回覆：{e}"))?;
    Ok(r.papers
        .into_iter()
        .map(clean_guess)
        .filter(|g| !g.title.trim().is_empty() || !g.doi.is_empty() || !g.arxiv.is_empty() || !g.pmid.is_empty())
        .collect())
}

/// Normalise the identifiers a guess carries.
pub fn clean_guess(g: Guess) -> Guess {
    let mut g = Guess { doi: clean_doi(&g.doi), pmid: g.pmid.trim().trim_start_matches("PMID").trim_start_matches(':').trim().to_string(), arxiv: clean_arxiv(&g.arxiv), ..g };
    // an arXiv DOI is an arXiv id
    if let Some(id) = g.doi.to_lowercase().strip_prefix("10.48550/arxiv.") {
        if g.arxiv.is_empty() {
            g.arxiv = id.to_string();
        }
    }
    if !g.pmid.chars().all(|c| c.is_ascii_digit()) {
        g.pmid.clear();
    }
    g
}

pub fn clean_doi(d: &str) -> String {
    let d = d.trim();
    let d = d.trim_start_matches("https://doi.org/").trim_start_matches("http://doi.org/").trim_start_matches("https://dx.doi.org/").trim_start_matches("doi:").trim_start_matches("DOI:").trim();
    if d.starts_with("10.") { d.trim_end_matches(['.', ',', ';']).to_string() } else { String::new() }
}

fn clean_arxiv(a: &str) -> String {
    let a = a.trim().trim_start_matches("arXiv:").trim_start_matches("arxiv:").trim();
    let a = a.rsplit("/abs/").next().unwrap_or(a);
    // drop a version suffix: 1901.07042v2 → 1901.07042
    let a = match a.rfind('v') {
        Some(i) if i + 1 < a.len() && a[i + 1..].chars().all(|c| c.is_ascii_digit()) => &a[..i],
        _ => a,
    };
    if a.len() >= 9 && a.contains('.') && a.chars().all(|c| c.is_ascii_digit() || c == '.') { a.to_string() } else { String::new() }
}

/// Plain text of a title from a database: markup (MathML, <i>) removed,
/// whitespace collapsed.
pub fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !matches!(*w, "a" | "an" | "the" | "of" | "for" | "and" | "in" | "on" | "with" | "to" | "via" | "by"))
        .map(String::from)
        .collect()
}

/// Share of title words in common (Jaccard), 0 to 1.
pub fn title_similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.iter().filter(|w| b.contains(w)).count() as f32;
    let union = (a.len() + b.len()) as f32 - inter;
    inter / union
}

/// Share of the guess's title words found in a work's title, 0 to 1: a
/// short guess ("TRIPOD+AI statement") is covered by the full title.
fn coverage(guess: &str, title: &str) -> f32 {
    let (a, b) = (words(guess), words(title));
    if a.is_empty() {
        return 0.0;
    }
    a.iter().filter(|w| b.contains(w)).count() as f32 / a.len() as f32
}

/// A work as a database has it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Work {
    #[serde(default)]
    pub doi: String,
    #[serde(default)]
    pub title: String,
    /// "Smith et al." for display.
    #[serde(default)]
    pub authors: String,
    /// Full names, "Family, Given", for BibTeX when there is no DOI.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub issue: String,
    #[serde(default)]
    pub pages: String,
    #[serde(default)]
    pub pmid: String,
    /// "crossref" | "arxiv" | "pubmed" | "text" (the user's own text).
    #[serde(default)]
    pub source: String,
}

fn short_authors(families: &[String]) -> String {
    match families.len() {
        0 => String::new(),
        1 | 2 => families.join(" & "),
        _ => format!("{} et al.", families[0]),
    }
}

/// The work's first author's family name appears in the guessed authors.
fn author_matches(g: &Guess, w: &Work) -> bool {
    let first = w.authors.split([' ', '&']).next().unwrap_or("").trim().to_lowercase();
    !first.is_empty() && g.authors.to_lowercase().contains(&first)
}

fn year_close(g: &Guess, w: &Work) -> bool {
    g.year.trim().is_empty() || w.year.is_empty() || {
        let (a, b) = (g.year.trim().parse::<i32>().unwrap_or(0), w.year.parse::<i32>().unwrap_or(0));
        (a - b).abs() <= 1
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Verdict {
    /// "found" | "ambiguous" | "missing"
    pub status: String,
    pub work: Option<Work>,
    /// For "ambiguous": the closest works, for the user to pick from.
    pub candidates: Vec<Work>,
    pub note: String,
}

fn found(w: Work) -> Verdict {
    Verdict { status: "found".into(), work: Some(w), candidates: vec![], note: String::new() }
}

/// A database work is this paper when the titles agree: closely, or the
/// written title is a shorter form of it with the right author or year.
fn same_paper(g: &Guess, w: &Work) -> bool {
    if g.title.trim().is_empty() {
        return true;
    }
    let sim = title_similarity(&g.title, &w.title);
    // either title may be the short form of the other
    let cov = coverage(&g.title, &w.title).max(coverage(&w.title, &g.title));
    sim >= 0.6 || (cov >= 0.9 && (author_matches(g, w) || year_close(g, w))) || (sim >= 0.45 && author_matches(g, w) && year_close(g, w))
}

/// Judge the closest works from a title search.
pub fn judge(g: &Guess, works: &[Work]) -> Verdict {
    let mut scored: Vec<(f32, &Work)> = works.iter().map(|w| (title_similarity(&g.title, &w.title), w)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    if let Some((s, w)) = scored.first() {
        if *s >= 0.8 && year_close(g, w) {
            return found((*w).clone());
        }
    }
    if let Some(w) = works.iter().find(|w| title_similarity(&g.title, &w.title) >= 0.6 && author_matches(g, w) && year_close(g, w)) {
        return found(w.clone());
    }
    // a short title whose words all appear in a title, with the right author
    // or year: a candidate for the user to confirm
    let partial: Vec<Work> = works
        .iter()
        .filter(|w| coverage(&g.title, &w.title) >= 0.99 && (author_matches(g, w) || (!g.year.trim().is_empty() && w.year == g.year.trim())))
        .take(3)
        .cloned()
        .collect();
    let close: Vec<Work> = scored.iter().filter(|(s, _)| *s >= 0.4).take(3).map(|(_, w)| (*w).clone()).collect();
    if !close.is_empty() {
        return Verdict { status: "ambiguous".into(), work: None, candidates: close, note: "找到相近的論文，請確認是哪一篇".into() };
    }
    if !partial.is_empty() {
        return Verdict { status: "ambiguous".into(), work: None, candidates: partial, note: "只寫了部分標題，請確認是不是這篇".into() };
    }
    Verdict { status: "missing".into(), work: None, candidates: vec![], note: "Crossref、PubMed 都查不到這篇".into() }
}

// ── the databases ───────────────────────────────────────

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("PaperComposer (https://github.com/ellrin/PaperComposer)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("http client")
}

fn base(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}

fn crossref_work(item: &serde_json::Value) -> Work {
    let first = |k: &str| item[k].as_array().and_then(|a| a.first()).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let people = item["author"].as_array().cloned().unwrap_or_default();
    let families: Vec<String> = people.iter().filter_map(|x| x["family"].as_str().map(String::from)).collect();
    let names = people
        .iter()
        .filter_map(|x| {
            let f = x["family"].as_str()?;
            Some(match x["given"].as_str() { Some(g) => format!("{f}, {g}"), None => f.to_string() })
        })
        .collect();
    let year = ["published", "issued", "published-print", "published-online"]
        .iter()
        .find_map(|k| item[*k]["date-parts"][0][0].as_i64())
        .map(|y| y.to_string())
        .unwrap_or_default();
    // Crossref keeps "False-Positive Psychology" and its subtitle apart
    let title = match first("subtitle") {
        s if s.trim().is_empty() => plain(&first("title")),
        s => format!("{}: {}", plain(&first("title")).trim_end_matches(':'), plain(&s)),
    };
    Work {
        doi: item["DOI"].as_str().unwrap_or("").to_string(),
        title,
        authors: short_authors(&families),
        names,
        year,
        venue: plain(&first("container-title")),
        volume: item["volume"].as_str().unwrap_or("").into(),
        issue: item["issue"].as_str().unwrap_or("").into(),
        pages: item["page"].as_str().unwrap_or("").into(),
        pmid: String::new(),
        source: "crossref".into(),
    }
}

async fn crossref_doi(doi: &str) -> Result<Option<Work>, String> {
    let r = http().get(format!("{}/works/{}", base("PAPERCOMPOSER_CROSSREF_BASE", "https://api.crossref.org"), doi)).send().await.map_err(|e| format!("無法連線到 Crossref：{e}"))?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let v: serde_json::Value = r.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    Ok(Some(crossref_work(&v["message"])))
}

async fn crossref_search(g: &Guess) -> Result<Vec<Work>, String> {
    let q = [g.title.as_str(), g.authors.as_str(), g.year.as_str()].iter().filter(|s| !s.trim().is_empty()).copied().collect::<Vec<_>>().join(" ");
    let mut url = reqwest::Url::parse(&format!("{}/works", base("PAPERCOMPOSER_CROSSREF_BASE", "https://api.crossref.org"))).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("query.bibliographic", &q).append_pair("rows", "5").append_pair("select", "DOI,title,author,issued,published,container-title,volume,issue,page");
    let v: serde_json::Value = http().get(url).send().await.map_err(|e| format!("無法連線到 Crossref：{e}"))?.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    Ok(v["message"]["items"].as_array().map(|a| a.iter().map(crossref_work).collect()).unwrap_or_default())
}

/// Text inside the first `<tag ...>...</tag>` after `from`.
fn xml_text<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = s.find(&format!("<{tag}"))?;
    let start = open + s[open..].find('>')? + 1;
    let end = start + s[start..].find(&format!("</{tag}>"))?;
    Some(&s[start..end])
}

async fn arxiv_work(id: &str) -> Result<Option<Work>, String> {
    let url = format!("{}/api/query?id_list={id}", base("PAPERCOMPOSER_ARXIV_BASE", "https://export.arxiv.org"));
    let body = http().get(url).send().await.map_err(|e| format!("無法連線到 arXiv：{e}"))?.text().await.map_err(|e| e.to_string())?;
    let Some(entry) = xml_text(&body, "entry") else { return Ok(None) };
    let title = plain(xml_text(entry, "title").unwrap_or(""));
    if title.is_empty() || title.eq_ignore_ascii_case("error") {
        return Ok(None);
    }
    let mut names = Vec::new();
    let mut families = Vec::new();
    let mut rest = entry;
    while let Some(i) = rest.find("<author>") {
        let a = &rest[i..];
        if let Some(n) = xml_text(a, "name") {
            let n = plain(n);
            let (given, family) = n.rsplit_once(' ').unwrap_or(("", n.as_str()));
            families.push(family.to_string());
            names.push(if given.is_empty() { family.to_string() } else { format!("{family}, {given}") });
        }
        rest = &a[8..];
    }
    let year = xml_text(entry, "published").map(|p| p.chars().take(4).collect()).unwrap_or_default();
    Ok(Some(Work { doi: format!("10.48550/arXiv.{id}"), title, authors: short_authors(&families), names, year, venue: "arXiv".into(), source: "arxiv".into(), ..Default::default() }))
}

fn pubmed_work(v: &serde_json::Value) -> Work {
    let names_raw: Vec<String> = v["authors"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    // "Ball CG" → family "Ball", given "C. G."
    let names: Vec<String> = names_raw
        .iter()
        .map(|n| match n.rsplit_once(' ') {
            Some((f, ini)) if ini.chars().all(|c| c.is_ascii_uppercase()) => format!("{f}, {}", ini.chars().map(|c| format!("{c}.")).collect::<Vec<_>>().join(" ")),
            _ => n.clone(),
        })
        .collect();
    let families: Vec<String> = names.iter().map(|n| n.split(',').next().unwrap_or("").to_string()).collect();
    let doi = v["articleids"].as_array().and_then(|a| a.iter().find(|x| x["idtype"] == "doi")).and_then(|x| x["value"].as_str()).unwrap_or("").to_string();
    Work {
        doi,
        title: plain(v["title"].as_str().unwrap_or("")).trim_end_matches('.').to_string(),
        authors: short_authors(&families),
        names,
        year: v["pubdate"].as_str().unwrap_or("").chars().take(4).collect(),
        venue: v["fulljournalname"].as_str().or(v["source"].as_str()).unwrap_or("").to_string(),
        volume: v["volume"].as_str().unwrap_or("").into(),
        issue: v["issue"].as_str().unwrap_or("").into(),
        pages: v["pages"].as_str().unwrap_or("").into(),
        pmid: v["uid"].as_str().unwrap_or("").into(),
        source: "pubmed".into(),
    }
}

async fn pubmed_summaries(ids: &[String]) -> Result<Vec<Work>, String> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let url = format!("{}/entrez/eutils/esummary.fcgi?db=pubmed&retmode=json&id={}", base("PAPERCOMPOSER_PUBMED_BASE", "https://eutils.ncbi.nlm.nih.gov"), ids.join(","));
    let v: serde_json::Value = http().get(url).send().await.map_err(|e| format!("無法連線到 PubMed：{e}"))?.json().await.map_err(|e| e.to_string())?;
    Ok(ids.iter().filter_map(|id| v["result"].get(id)).filter(|x| x["title"].is_string()).map(pubmed_work).collect())
}

async fn pubmed_search(g: &Guess) -> Result<Vec<Work>, String> {
    let mut url = reqwest::Url::parse(&format!("{}/entrez/eutils/esearch.fcgi", base("PAPERCOMPOSER_PUBMED_BASE", "https://eutils.ncbi.nlm.nih.gov"))).map_err(|e| e.to_string())?;
    let title: String = g.title.chars().map(|c| if c.is_alphanumeric() || c.is_whitespace() || c == '-' { c } else { ' ' }).collect();
    url.query_pairs_mut().append_pair("db", "pubmed").append_pair("retmode", "json").append_pair("retmax", "5").append_pair("term", &format!("{}[Title]", title.split_whitespace().collect::<Vec<_>>().join(" ")));
    let v: serde_json::Value = http().get(url).send().await.map_err(|e| format!("無法連線到 PubMed：{e}"))?.json().await.map_err(|e| e.to_string())?;
    let ids: Vec<String> = v["esearchresult"]["idlist"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    pubmed_summaries(&ids).await
}

/// Check one paper against the databases.
pub async fn verify(g: &Guess) -> Result<Verdict, String> {
    // 1. a DOI that resolves to this paper
    if !g.doi.is_empty() && g.arxiv.is_empty() {
        if let Some(w) = crossref_doi(&g.doi).await? {
            if same_paper(g, &w) {
                return Ok(found(w));
            }
            let mut v = judge(g, &crossref_search(g).await?);
            if v.status != "found" {
                v.candidates.insert(0, w);
                v.status = "ambiguous".into();
                v.note = "DOI 指向的論文標題不同，請確認".into();
            }
            return Ok(v);
        }
    }
    // 2. an arXiv id
    if !g.arxiv.is_empty() {
        if let Some(w) = arxiv_work(&g.arxiv).await? {
            if same_paper(g, &w) {
                return Ok(found(w));
            }
            return Ok(Verdict { status: "ambiguous".into(), work: None, candidates: vec![w], note: "arXiv 編號指向的論文標題不同，請確認".into() });
        }
    }
    // 3. a PMID
    if !g.pmid.is_empty() {
        if let Some(w) = pubmed_summaries(&[g.pmid.clone()]).await?.into_iter().next() {
            if same_paper(g, &w) {
                return Ok(found(w));
            }
        }
    }
    if g.title.trim().is_empty() {
        return Ok(Verdict { status: "missing".into(), work: None, candidates: vec![], note: "編號查不到".into() });
    }
    // 4. a title search: Crossref, then PubMed (clinical papers without a DOI)
    let v = judge(g, &crossref_search(g).await?);
    if v.status == "found" {
        return Ok(v);
    }
    let p = judge(g, &pubmed_search(g).await.unwrap_or_default());
    if p.status == "found" {
        return Ok(p);
    }
    Ok(if v.status == "ambiguous" { v } else { p })
}

/// The user's own bibliographic data as a work, for a paper no database
/// has (e.g. Holm 1979 in Scand J Stat: no DOI, not in PubMed).
pub fn from_text(g: &Guess) -> Work {
    let names: Vec<String> = g
        .authors
        .split([',', ';'])
        .map(str::trim)
        .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("et al.") && !n.eq_ignore_ascii_case("et al"))
        .map(|n| match n.rsplit_once(' ') {
            Some((f, ini)) if ini.chars().all(|c| c.is_ascii_uppercase()) => format!("{f}, {}", ini.chars().map(|c| format!("{c}.")).collect::<Vec<_>>().join(" ")),
            _ => n.to_string(),
        })
        .collect();
    let families: Vec<String> = names.iter().map(|n| n.split(',').next().unwrap_or("").to_string()).collect();
    Work {
        doi: g.doi.clone(),
        title: g.title.trim().to_string(),
        authors: short_authors(&families),
        names,
        year: g.year.trim().to_string(),
        venue: g.venue.trim().to_string(),
        volume: g.volume.trim().to_string(),
        issue: g.issue.trim().to_string(),
        pages: g.pages.trim().to_string(),
        pmid: g.pmid.clone(),
        source: "text".into(),
    }
}

/// BibTeX fields for a work without a DOI record to fetch.
pub fn entry_fields(w: &Work) -> std::collections::BTreeMap<String, String> {
    let mut f = std::collections::BTreeMap::new();
    let mut put = |k: &str, v: &str| {
        if !v.trim().is_empty() {
            f.insert(k.to_string(), v.trim().replace('&', "\\&").replace('%', "\\%"));
        }
    };
    put("title", &w.title);
    put("author", &w.names.join(" and "));
    put("journal", &w.venue);
    put("year", &w.year);
    put("volume", &w.volume);
    put("number", &w.issue);
    put("pages", &w.pages.replace('-', "--").replace("----", "--"));
    put("doi", &w.doi);
    put("pmid", &w.pmid);
    if w.source == "text" {
        put("note", "Bibliographic data as entered by the author");
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(title: &str, year: &str) -> Work {
        Work { doi: format!("10.1/{}", title.len()), title: title.into(), year: year.into(), ..Default::default() }
    }

    #[test]
    fn parses_the_split_and_cleans_ids() {
        let r = "```json\n{\"papers\":[{\"title\":\"U-Net\",\"doi\":\"https://doi.org/10.1007/978-3-319-24574-4_28.\"},{\"title\":\" \",\"doi\":\"\"},{\"doi\":\"doi:10.1/x\"},{\"title\":\"JPG\",\"arxiv\":\"arXiv:1901.07042v2\",\"pmid\":\"PMID 123\"}]}\n```";
        let g = parse_extract(r).unwrap();
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].doi, "10.1007/978-3-319-24574-4_28");
        assert_eq!(g[1].doi, "10.1/x");
        assert_eq!((g[2].arxiv.as_str(), g[2].pmid.as_str()), ("1901.07042", "123"));
        assert_eq!(clean_doi("not a doi"), "");
        assert_eq!(clean_guess(Guess { doi: "10.48550/arXiv.1906.06423".into(), ..Default::default() }).arxiv, "1906.06423");
    }

    #[test]
    fn plain_drops_markup() {
        assert_eq!(plain("Ark\n  <mml:math xmlns:mml=\"x\"><mml:mo>+</mml:mo></mml:math>: Supervised"), "Ark +: Supervised");
        assert_eq!(plain("<i>In vivo</i> study"), "In vivo study");
    }

    #[test]
    fn similarity_ignores_case_punctuation_and_small_words() {
        assert!(title_similarity("U-Net: Convolutional Networks for Biomedical Image Segmentation", "U-net: convolutional networks for biomedical image segmentation") > 0.99);
        assert!(title_similarity("Deep residual learning", "Attention is all you need") < 0.1);
    }

    #[test]
    fn a_short_title_with_its_doi_is_the_paper() {
        let g = Guess { title: "TRIPOD+AI statement".into(), authors: "Collins GS, Moons KGM".into(), year: "2024".into(), ..Default::default() };
        let full = Work { title: "TRIPOD+AI statement: updated guidance for reporting clinical prediction models that use regression or machine learning methods".into(), authors: "Collins et al.".into(), year: "2024".into(), ..Default::default() };
        assert!(same_paper(&g, &full));
        let other = Work { title: "A survey of statements".into(), authors: "Lee et al.".into(), year: "2001".into(), ..Default::default() };
        assert!(!same_paper(&g, &other));
        // the database title is the short one (subtitle kept apart)
        let long = Guess { title: "False-positive psychology: undisclosed flexibility in data collection".into(), authors: "Simmons JP, Nelson LD".into(), year: "2011".into(), ..Default::default() };
        let short = Work { title: "False-Positive Psychology".into(), authors: "Simmons et al.".into(), year: "2011".into(), ..Default::default() };
        assert!(same_paper(&long, &short));
        let item = serde_json::json!({"DOI":"10.1177/x","title":["False-Positive Psychology"],"subtitle":["Undisclosed Flexibility"],"author":[{"family":"Simmons","given":"Joseph P."}]});
        assert_eq!(crossref_work(&item).title, "False-Positive Psychology: Undisclosed Flexibility");
    }

    #[test]
    fn judges_found_ambiguous_and_missing() {
        let g = Guess { title: "Deep residual learning for image recognition".into(), year: "2016".into(), ..Default::default() };
        assert_eq!(judge(&g, &[w("Deep Residual Learning for Image Recognition", "2016")]).status, "found");
        // right title, wrong year by far: ask
        assert_eq!(judge(&g, &[w("Deep Residual Learning for Image Recognition", "2009")]).status, "ambiguous");
        assert_eq!(judge(&g, &[w("Deep residual learning for image classification", "2016")]).status, "ambiguous");
        assert_eq!(judge(&g, &[w("Deep residual networks for speech", "2016")]).status, "missing");
        assert_eq!(judge(&g, &[]).status, "missing");
        let short = Guess { title: "Grad-CAM".into(), authors: "Selvaraju".into(), ..Default::default() };
        let gc = Work { title: "Grad-CAM: Visual Explanations from Deep Networks via Gradient-Based Localization".into(), authors: "Selvaraju et al.".into(), year: "2017".into(), ..Default::default() };
        let v = judge(&short, &[gc.clone()]);
        assert_eq!((v.status.as_str(), v.candidates.len()), ("ambiguous", 1));
        assert_eq!(judge(&Guess { title: "Grad-CAM".into(), ..Default::default() }, &[gc]).status, "missing");
    }

    #[test]
    fn pubmed_and_text_records_become_bibtex_fields() {
        let v = serde_json::json!({"uid":"19865564","title":"The occult pneumothorax: what have we learned?","authors":[{"name":"Ball CG"},{"name":"Kirkpatrick AW"}],"pubdate":"2009 Oct","fulljournalname":"Canadian journal of surgery","volume":"52","issue":"5","pages":"E173-9","articleids":[{"idtype":"pubmed","value":"19865564"}]});
        let p = pubmed_work(&v);
        assert_eq!((p.year.as_str(), p.authors.as_str(), p.doi.as_str()), ("2009", "Ball & Kirkpatrick", ""));
        let f = entry_fields(&p);
        assert_eq!(f["author"], "Ball, C. G. and Kirkpatrick, A. W.");
        assert_eq!(f["pages"], "E173--9");
        assert_eq!(f["pmid"], "19865564");
        let holm = from_text(&Guess { title: "A simple sequentially rejective multiple test procedure".into(), authors: "Holm S".into(), year: "1979".into(), venue: "Scand J Stat".into(), volume: "6".into(), issue: "2".into(), pages: "65-70".into(), ..Default::default() });
        let f = entry_fields(&holm);
        assert_eq!((f["author"].as_str(), f["pages"].as_str()), ("Holm, S.", "65--70"));
        assert!(f["note"].contains("entered by the author"));
    }
}
