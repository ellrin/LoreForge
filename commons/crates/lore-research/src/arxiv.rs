//! arXiv's Atom API (`export.arxiv.org/api/query`). Every arXiv record is open access; its PDF
//! is `arxiv.org/pdf/<id>`. A bad query answers 200 with an empty feed.

use quick_xml::events::{BytesStart, Event};

use crate::{Record, Source, arxiv_from_url, norm_doi};

pub const BASE: &str = "https://export.arxiv.org/api/query";

pub fn search_url(query: &str, limit: usize) -> String {
    let words: Vec<String> = query.split_whitespace().map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>()).filter(|w| !w.is_empty()).collect();
    let q = words.iter().map(|w| format!("all:{w}")).collect::<Vec<_>>().join(" AND ");
    crate::url_with(BASE, &[("search_query", q), ("start", "0".into()), ("max_results", limit.clamp(1, 50).to_string())])
}

pub fn title_url(title: &str) -> String {
    let words: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let q = words.split_whitespace().map(|w| format!("ti:{w}")).collect::<Vec<_>>().join(" AND ");
    crate::url_with(BASE, &[("search_query", q), ("start", "0".into()), ("max_results", "5".into())])
}

pub fn id_url(id: &str) -> String {
    crate::url_with(BASE, &[("id_list", crate::norm_arxiv(id)), ("max_results", "1".into())])
}

/// OAI-PMH record of one paper in arXiv's own format: the only place that states the licence the
/// authors chose (OpenAlex labels arXiv copies `cc-by` whatever they are).
pub fn oai_url(id: &str) -> String {
    crate::url_with("https://export.arxiv.org/oai2", &[("verb", "GetRecord".into()), ("identifier", format!("oai:arXiv.org:{}", crate::norm_arxiv(id))), ("metadataPrefix", "arXiv".into())])
}

/// The licence of an OAI record: `cc-by`, `cc-by-sa`, `cc-by-nc-sa`, `cc0`, …, or
/// `arxiv-nonexclusive` (arXiv's default licence, also when the record names none). None = not a record.
pub fn oai_license(xml: &str) -> Option<String> {
    if !xml.contains("<arXiv") {
        return None;
    }
    let url = xml.split_once("<license>").and_then(|(_, r)| r.split_once("</license>")).map(|(l, _)| l.trim().to_string()).unwrap_or_default();
    let tail = url.split("/licenses/").nth(1).unwrap_or("").trim_matches('/');
    let mut parts = tail.split('/');
    let kind = parts.next().unwrap_or("");
    Some(match kind {
        _ if url.contains("publicdomain/zero") => "cc0".into(),
        _ if url.contains("publicdomain") => "public-domain".into(),
        "" | "nonexclusive-distrib" => "arxiv-nonexclusive".into(),
        k => format!("cc-{k}"),
    })
}

pub fn pdf_url(id: &str) -> String {
    format!("https://arxiv.org/pdf/{}", crate::norm_arxiv(id))
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).map(|a| a.value.replace("&amp;", "&"))
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        n => n.strip_prefix("#x").and_then(|h| u32::from_str_radix(h, 16).ok()).or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok())).and_then(char::from_u32),
    }
}

/// The feed's entries.
pub fn parse(xml: &str) -> Vec<Record> {
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let mut cur: Option<Record> = None;
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut author = String::new();
    let link = |rec: &mut Record, e: &BytesStart| {
        let href = attr(e, "href");
        if attr(e, "title").as_deref() == Some("pdf") {
            rec.pdf_url = href;
        } else if attr(e, "rel").as_deref() == Some("alternate") {
            rec.url = href;
        }
    };
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                if name == "entry" {
                    cur = Some(Record { source: Source::Arxiv, sources: vec![Source::Arxiv], is_oa: true, ..Default::default() });
                } else if let (Some(rec), "link") = (cur.as_mut(), name.as_str()) {
                    link(rec, &e);
                }
                path.push(name);
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                if let Some(rec) = cur.as_mut()
                    && e.name().as_ref() == "link" {
                        link(rec, &e);
                    }
            }
            Ok(Event::Text(t)) => text.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(g)) => {
                if let Some(c) = entity(&g.xml10_content()) {
                    text.push(c);
                }
            }
            Ok(Event::CData(t)) => text.push_str(&t.xml10_content()),
            Ok(Event::End(_)) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map(String::as_str).unwrap_or("");
                let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if let Some(rec) = cur.as_mut() {
                    match (parent, name.as_str()) {
                        ("entry", "id") => rec.arxiv = arxiv_from_url(&t),
                        ("entry", "title") => rec.title = t,
                        ("entry", "summary") => rec.abstract_text = Some(t).filter(|s| !s.is_empty()),
                        ("entry", "published") => rec.year = t.get(..4).and_then(|y| y.parse().ok()),
                        ("author", "name") => author = t,
                        ("entry", "author") => {
                            if !author.is_empty() {
                                rec.authors.push(std::mem::take(&mut author));
                            }
                        }
                        ("entry", "arxiv:doi") => rec.doi = Some(norm_doi(&t)).filter(|d| !d.is_empty()),
                        ("entry", "arxiv:journal_ref") => rec.venue = Some(t).filter(|s| !s.is_empty()),
                        _ => {}
                    }
                }
                if name == "entry"
                    && let Some(mut rec) = cur.take() {
                        // arXiv's error feed is an entry with an api/errors id
                        if let (Some(id), false) = (rec.arxiv.clone(), rec.title.is_empty()) {
                            if rec.pdf_url.is_none() {
                                rec.pdf_url = Some(pdf_url(&id));
                            }
                            out.push(rec);
                        }
                    }
                text.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}
