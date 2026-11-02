//! A pasted list of papers to cite, checked one by one.
//!
//! The model only splits the text into entries and reads what each one says
//! (title, authors, year, DOI). Whether a paper exists is decided by Crossref:
//! a DOI must resolve to a work with a matching title, or a search by title
//! must find one. A paper Crossref cannot find is reported, never invented.

use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Guess {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub authors: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub doi: String,
}

pub fn extract_prompt(text: &str) -> Prompt {
    let system = "You split text that lists academic papers into one entry per paper. \
The text may be a reference list, citations in any style, titles, DOIs or notes, in any language. \
For each paper copy what the text says: title, authors, year, venue, DOI. Leave a field empty when the text does not give it. \
Never guess or complete a field from memory, never add a paper that is not in the text. \
Reply with JSON only: {\"papers\":[{\"title\":\"\",\"authors\":\"\",\"year\":\"\",\"venue\":\"\",\"doi\":\"\"}]}"
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
        .map(|g| Guess { doi: clean_doi(&g.doi), ..g })
        .filter(|g| !g.title.trim().is_empty() || !g.doi.is_empty())
        .collect())
}

pub fn clean_doi(d: &str) -> String {
    let d = d.trim();
    let d = d.trim_start_matches("https://doi.org/").trim_start_matches("http://doi.org/").trim_start_matches("https://dx.doi.org/").trim_start_matches("doi:").trim_start_matches("DOI:").trim();
    if d.starts_with("10.") { d.trim_end_matches(['.', ',', ';']).to_string() } else { String::new() }
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

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Work {
    pub doi: String,
    pub title: String,
    pub authors: String,
    pub year: String,
    pub venue: String,
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

fn work_from(item: &serde_json::Value) -> Work {
    let first = |k: &str| item[k].as_array().and_then(|a| a.first()).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let authors: Vec<String> = item["author"].as_array().map(|a| a.iter().filter_map(|x| x["family"].as_str().map(String::from)).collect()).unwrap_or_default();
    let year = ["published", "issued", "published-print", "published-online"]
        .iter()
        .find_map(|k| item[*k]["date-parts"][0][0].as_i64())
        .map(|y| y.to_string())
        .unwrap_or_default();
    Work {
        doi: item["DOI"].as_str().unwrap_or("").to_string(),
        title: first("title"),
        authors: match authors.len() {
            0 => String::new(),
            1 | 2 => authors.join(" & "),
            _ => format!("{} et al.", authors[0]),
        },
        year,
        venue: first("container-title"),
    }
}

/// Share of the guess's title words found in a work's title, 0 to 1: a
/// short guess ("the Grad-CAM paper") is covered by the full title.
fn coverage(guess: &str, title: &str) -> f32 {
    let (a, b) = (words(guess), words(title));
    if a.is_empty() {
        return 0.0;
    }
    a.iter().filter(|w| b.contains(w)).count() as f32 / a.len() as f32
}

/// The work's first author's family name appears in the guessed authors.
fn author_matches(g: &Guess, w: &Work) -> bool {
    let first = w.authors.split([' ', '&']).next().unwrap_or("").trim().to_lowercase();
    !first.is_empty() && g.authors.to_lowercase().contains(&first)
}

/// Judge the closest works for a guess.
pub fn judge(g: &Guess, works: &[Work]) -> Verdict {
    // A short guess whose words all appear in a title, with the right author
    // or year, is a candidate for the user to confirm, never a sure match.
    let partial: Vec<&Work> = works
        .iter()
        .filter(|w| title_similarity(&g.title, &w.title) < 0.4 && coverage(&g.title, &w.title) >= 0.99 && (author_matches(g, w) || (!g.year.trim().is_empty() && w.year == g.year.trim())))
        .take(3)
        .collect();
    let mut scored: Vec<(f32, &Work)> = works.iter().map(|w| (title_similarity(&g.title, &w.title), w)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let year_ok = |w: &Work| g.year.trim().is_empty() || w.year.is_empty() || {
        let (a, b) = (g.year.trim().parse::<i32>().unwrap_or(0), w.year.parse::<i32>().unwrap_or(0));
        (a - b).abs() <= 1
    };
    match scored.first() {
        Some((s, w)) if *s >= 0.8 && year_ok(w) => Verdict { status: "found".into(), work: Some((*w).clone()), candidates: vec![], note: String::new() },
        Some((s, _)) if *s >= 0.4 => Verdict {
            status: "ambiguous".into(),
            work: None,
            candidates: scored.iter().filter(|(s, _)| *s >= 0.4).take(3).map(|(_, w)| (*w).clone()).collect(),
            note: "找到相近的論文，請確認是哪一篇".into(),
        },
        _ if !partial.is_empty() => Verdict {
            status: "ambiguous".into(),
            work: None,
            candidates: partial.into_iter().cloned().collect(),
            note: "只寫了部分標題，請確認是不是這篇".into(),
        },
        _ => Verdict { status: "missing".into(), work: None, candidates: vec![], note: "Crossref 查不到這篇，可能寫錯或不存在".into() },
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("PaperComposer (https://github.com/ellrin/PaperComposer)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("http client")
}

fn base() -> String {
    std::env::var("PAPERCOMPOSER_CROSSREF_BASE").unwrap_or_else(|_| "https://api.crossref.org".into())
}

async fn crossref_doi(doi: &str) -> Result<Option<Work>, String> {
    let r = http().get(format!("{}/works/{}", base(), doi)).send().await.map_err(|e| format!("無法連線到 Crossref：{e}"))?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let v: serde_json::Value = r.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    Ok(Some(work_from(&v["message"])))
}

async fn crossref_search(g: &Guess) -> Result<Vec<Work>, String> {
    let q = [g.title.as_str(), g.authors.as_str(), g.year.as_str()].iter().filter(|s| !s.trim().is_empty()).copied().collect::<Vec<_>>().join(" ");
    let mut url = reqwest::Url::parse(&format!("{}/works", base())).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("query.bibliographic", &q).append_pair("rows", "5").append_pair("select", "DOI,title,author,issued,published,container-title");
    let v: serde_json::Value = http().get(url).send().await.map_err(|e| format!("無法連線到 Crossref：{e}"))?.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    Ok(v["message"]["items"].as_array().map(|a| a.iter().map(work_from).collect()).unwrap_or_default())
}

/// Check one paper against Crossref.
pub async fn verify(g: &Guess) -> Result<Verdict, String> {
    if !g.doi.is_empty() {
        if let Some(w) = crossref_doi(&g.doi).await? {
            // The DOI is real; it must also be the paper the text names.
            if g.title.trim().is_empty() || title_similarity(&g.title, &w.title) >= 0.6 {
                return Ok(Verdict { status: "found".into(), work: Some(w), candidates: vec![], note: String::new() });
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
    if g.title.trim().is_empty() {
        return Ok(Verdict { status: "missing".into(), work: None, candidates: vec![], note: "DOI 不存在".into() });
    }
    Ok(judge(g, &crossref_search(g).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(title: &str, year: &str) -> Work {
        Work { doi: format!("10.1/{}", title.len()), title: title.into(), year: year.into(), ..Default::default() }
    }

    #[test]
    fn parses_the_split_and_cleans_dois() {
        let r = "```json\n{\"papers\":[{\"title\":\"U-Net\",\"doi\":\"https://doi.org/10.1007/978-3-319-24574-4_28.\"},{\"title\":\" \",\"doi\":\"\"},{\"doi\":\"doi:10.1/x\"}]}\n```";
        let g = parse_extract(r).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].doi, "10.1007/978-3-319-24574-4_28");
        assert_eq!(g[1].doi, "10.1/x");
        assert_eq!(clean_doi("not a doi"), "");
    }

    #[test]
    fn similarity_ignores_case_punctuation_and_small_words() {
        assert!(title_similarity("U-Net: Convolutional Networks for Biomedical Image Segmentation", "U-net: convolutional networks for biomedical image segmentation") > 0.99);
        assert!(title_similarity("Deep residual learning", "Attention is all you need") < 0.1);
    }

    #[test]
    fn judges_found_ambiguous_and_missing() {
        let g = Guess { title: "Deep residual learning for image recognition".into(), year: "2016".into(), ..Default::default() };
        assert_eq!(judge(&g, &[w("Deep Residual Learning for Image Recognition", "2016")]).status, "found");
        // right title, wrong year by far: ask
        assert_eq!(judge(&g, &[w("Deep Residual Learning for Image Recognition", "2009")]).status, "ambiguous");
        assert_eq!(judge(&g, &[w("Deep residual learning for image classification", "2016")]).status, "ambiguous");
        assert_eq!(judge(&g, &[w("Deep residual networks for speech", "2016")]).status, "missing");
        assert_eq!(judge(&g, &[w("Attention is all you need", "2017")]).status, "missing");
        assert_eq!(judge(&g, &[]).status, "missing");
        // a short title with the right author: ask, do not file it blindly
        let short = Guess { title: "Grad-CAM".into(), authors: "Selvaraju".into(), ..Default::default() };
        let gc = Work { title: "Grad-CAM: Visual Explanations from Deep Networks via Gradient-Based Localization".into(), authors: "Selvaraju et al.".into(), year: "2017".into(), ..Default::default() };
        let v = judge(&short, &[gc.clone()]);
        assert_eq!((v.status.as_str(), v.candidates.len()), ("ambiguous", 1));
        let nobody = Guess { title: "Grad-CAM".into(), ..Default::default() };
        assert_eq!(judge(&nobody, &[gc]).status, "missing");
    }
}
