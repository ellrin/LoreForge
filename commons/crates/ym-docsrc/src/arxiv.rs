//! arXiv e-prints behind a trait: [`HttpArxiv`] (real network, polite) and
//! [`MockArxiv`] (tests). [`unpack`] turns an e-print (tar.gz, gzipped single
//! .tex, plain tar, or a PDF when no source exists) into a folder.

use crate::error::{Error, Result};
use crate::model::AssetCatalog;
use async_trait::async_trait;
use flate2::read::GzDecoder;
use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A normalised arXiv identifier: `2101.00001`, `2101.00001v2`, `hep-th/9901001`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArxivId(String);

impl ArxivId {
    /// Accepts bare ids, `arXiv:` prefixes and abs/pdf/e-print URLs.
    pub fn parse(s: &str) -> Result<ArxivId> {
        let re = Regex::new(r"(?i)(?:arxiv:|arxiv\.org/(?:abs|pdf|e-print|src)/)?\s*(\d{4}\.\d{4,5}(?:v\d+)?|[a-z\-]+(?:\.[A-Z]{2})?/\d{7}(?:v\d+)?)(?:\.pdf)?/?$").unwrap();
        let t = s.trim();
        re.captures(t)
            .map(|c| ArxivId(c[1].to_string()))
            .ok_or_else(|| Error::Arxiv(format!("看不懂的 arXiv 編號：{t}")))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Without the version suffix.
    pub fn base(&self) -> &str {
        let re = Regex::new(r"^(.*\d)v\d+$").unwrap();
        match re.captures(&self.0) {
            Some(c) => &self.0[..c.get(1).unwrap().end()],
            None => &self.0,
        }
    }
    /// Safe as a folder name.
    pub fn file_stem(&self) -> String {
        self.0.replace('/', "_")
    }
}

/// Metadata from the arXiv API (Atom).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArxivMeta {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub summary: String,
    pub published: String,
    pub categories: Vec<String>,
}

/// Parse the first `<entry>` of an arXiv API Atom feed.
pub fn parse_atom(xml: &str) -> Result<ArxivMeta> {
    let entry = Regex::new(r"(?s)<entry>(.*?)</entry>")
        .unwrap()
        .captures(xml)
        .ok_or_else(|| Error::Arxiv("回應沒有 entry".into()))?[1]
        .to_string();
    let tag = |name: &str| -> String {
        Regex::new(&format!(r"(?s)<{name}[^>]*>(.*?)</{name}>"))
            .unwrap()
            .captures(&entry)
            .map(|c| {
                crate::pdf::layout::unescape(
                    c[1].split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .as_str(),
                )
            })
            .unwrap_or_default()
    };
    let authors = Regex::new(r"(?s)<author>\s*<name>(.*?)</name>")
        .unwrap()
        .captures_iter(&entry)
        .map(|c| c[1].trim().to_string())
        .collect();
    let categories = Regex::new(r#"<category[^>]*term="([^"]+)""#)
        .unwrap()
        .captures_iter(&entry)
        .map(|c| c[1].to_string())
        .collect();
    let id = tag("id");
    if id.is_empty() && tag("title").is_empty() {
        return Err(Error::Arxiv("找不到這篇".into()));
    }
    Ok(ArxivMeta {
        id: id.rsplit("/abs/").next().unwrap_or(&id).to_string(),
        title: tag("title"),
        authors,
        summary: tag("summary"),
        published: tag("published"),
        categories,
    })
}

#[async_trait]
pub trait ArxivFetcher: Send + Sync {
    /// The e-print bytes (usually tar.gz; a PDF when the paper has no source).
    async fn eprint(&self, id: &ArxivId) -> Result<Vec<u8>>;
    async fn metadata(&self, id: &ArxivId) -> Result<ArxivMeta>;
}

/// Canned e-prints and Atom feeds; records requested ids.
#[derive(Debug, Default)]
pub struct MockArxiv {
    pub eprints: HashMap<String, Vec<u8>>,
    pub atoms: HashMap<String, String>,
    pub calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ArxivFetcher for MockArxiv {
    async fn eprint(&self, id: &ArxivId) -> Result<Vec<u8>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("eprint:{}", id.as_str()));
        self.eprints
            .get(id.as_str())
            .cloned()
            .ok_or_else(|| Error::Arxiv(format!("mock 沒有 {}", id.as_str())))
    }
    async fn metadata(&self, id: &ArxivId) -> Result<ArxivMeta> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("meta:{}", id.as_str()));
        parse_atom(
            self.atoms
                .get(id.as_str())
                .ok_or_else(|| Error::Arxiv(format!("mock 沒有 {}", id.as_str())))?,
        )
    }
}

/// The real arXiv. Follows the API etiquette: one request at a time, at
/// least 3 s apart, a descriptive User-Agent with a contact the app supplies,
/// `export.arxiv.org` endpoints, and an on-disk cache so the same e-print is
/// never downloaded twice. **Not tested on real network** (tests use [`MockArxiv`]).
pub struct HttpArxiv {
    client: reqwest::Client,
    pub api_base: String,
    pub eprint_base: String,
    pub min_interval: Duration,
    pub cache_dir: Option<PathBuf>,
    last: tokio::sync::Mutex<Option<Instant>>,
}

impl HttpArxiv {
    /// `contact` goes into the User-Agent (e.g. a project URL or mailto: the app's user configured).
    pub fn new(contact: &str, cache_dir: Option<PathBuf>) -> Result<HttpArxiv> {
        let ua = format!("ym-docsrc/{} ({contact})", env!("CARGO_PKG_VERSION"));
        let client = reqwest::Client::builder()
            .user_agent(ua)
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(HttpArxiv {
            client,
            api_base: "https://export.arxiv.org/api/query".into(),
            eprint_base: "https://export.arxiv.org/e-print".into(),
            min_interval: Duration::from_secs(3),
            cache_dir,
            last: tokio::sync::Mutex::new(None),
        })
    }

    async fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let wait = self.min_interval.saturating_sub(t.elapsed());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
        let resp = self.client.get(url).send().await;
        *last = Some(Instant::now());
        let resp = resp?;
        if !resp.status().is_success() {
            return Err(Error::Arxiv(format!("{url} → HTTP {}", resp.status())));
        }
        Ok(resp.bytes().await?.to_vec())
    }

    fn cached(&self, name: &str) -> Option<PathBuf> {
        self.cache_dir.as_ref().map(|d| d.join(name))
    }
}

#[async_trait]
impl ArxivFetcher for HttpArxiv {
    async fn eprint(&self, id: &ArxivId) -> Result<Vec<u8>> {
        let cache = self.cached(&format!("{}.eprint", id.file_stem()));
        if let Some(c) = &cache
            && let Ok(b) = std::fs::read(c)
        {
            return Ok(b);
        }
        let bytes = self
            .get(&format!("{}/{}", self.eprint_base, id.as_str()))
            .await?;
        if let Some(c) = &cache {
            if let Some(d) = c.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(c, &bytes)?;
        }
        Ok(bytes)
    }

    async fn metadata(&self, id: &ArxivId) -> Result<ArxivMeta> {
        let url = format!("{}?id_list={}&max_results=1", self.api_base, id.as_str());
        let xml = String::from_utf8_lossy(&self.get(&url).await?).into_owned();
        parse_atom(&xml)
    }
}

/// What an e-print turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Unpacked {
    /// A LaTeX source folder.
    Source(PathBuf),
    /// No source on arXiv: the PDF.
    Pdf(PathBuf),
}

fn safe_rel(p: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(x) => out.push(x),
            Component::CurDir => {}
            _ => return Err(Error::UnsafePath(p.display().to_string())),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::UnsafePath(p.display().to_string()));
    }
    Ok(out)
}

fn untar(bytes: &[u8], dest: &Path) -> Result<usize> {
    let mut ar = tar::Archive::new(bytes);
    let mut n = 0;
    for e in ar.entries()? {
        let mut e = e?;
        let rel = safe_rel(&e.path()?)?;
        let target = dest.join(&rel);
        match e.header().entry_type() {
            tar::EntryType::Directory => std::fs::create_dir_all(&target)?,
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if let Some(d) = target.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let mut buf = vec![];
                e.read_to_end(&mut buf)?;
                std::fs::write(&target, buf)?;
                n += 1;
            }
            _ => {} // links and specials are skipped
        }
    }
    Ok(n)
}

fn looks_like_tar(b: &[u8]) -> bool {
    b.len() >= 512
        && (&b[257..262] == b"ustar"
            || tar::Archive::new(b)
                .entries()
                .map(|mut e| e.next().is_some_and(|x| x.is_ok()))
                .unwrap_or(false))
}

/// Unpack e-print bytes into `dest` (created; must be empty or new).
pub fn unpack(bytes: &[u8], dest: &Path) -> Result<Unpacked> {
    std::fs::create_dir_all(dest)?;
    let data = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = vec![];
        GzDecoder::new(bytes).read_to_end(&mut out)?;
        out
    } else {
        bytes.to_vec()
    };
    if data.starts_with(b"%PDF") {
        let p = dest.join("paper.pdf");
        std::fs::write(&p, &data)?;
        return Ok(Unpacked::Pdf(p));
    }
    if looks_like_tar(&data) && untar(&data, dest)? > 0 {
        return Ok(Unpacked::Source(dest.to_path_buf()));
    }
    // a single gzipped .tex
    std::fs::write(dest.join("main.tex"), &data)?;
    Ok(Unpacked::Source(dest.to_path_buf()))
}

/// Fetch, unpack into `dest/<id>/`, and extract the LaTeX source (title and
/// authors from the arXiv API when the source lacks them). `Ok(None)` when
/// the e-print is only a PDF (use [`crate::pdf::extract`] on it).
pub async fn fetch_and_extract(
    f: &dyn ArxivFetcher,
    id: &ArxivId,
    dest: &Path,
) -> Result<(Unpacked, Option<AssetCatalog>)> {
    let bytes = f.eprint(id).await?;
    let dir = dest.join(id.file_stem());
    let un = unpack(&bytes, &dir)?;
    let Unpacked::Source(root) = &un else {
        return Ok((un, None));
    };
    let mut cat = crate::latex::extract_dir(root)?;
    cat.meta.arxiv_id = Some(id.as_str().to_string());
    if let Ok(m) = f.metadata(id).await {
        if cat.meta.title.trim().is_empty() {
            cat.meta.title = m.title;
        }
        if cat.meta.authors.is_empty() {
            cat.meta.authors = m.authors;
        }
        if cat.meta.abstract_text.is_none() && !m.summary.is_empty() {
            cat.meta.abstract_text = Some(m.summary);
        }
    }
    Ok((un, Some(cat)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    #[test]
    fn ids() {
        assert_eq!(ArxivId::parse("2101.00001").unwrap().as_str(), "2101.00001");
        assert_eq!(
            ArxivId::parse("arXiv:2101.00001v3").unwrap().base(),
            "2101.00001"
        );
        assert_eq!(
            ArxivId::parse("https://arxiv.org/abs/2310.12345v1")
                .unwrap()
                .as_str(),
            "2310.12345v1"
        );
        assert_eq!(
            ArxivId::parse("https://arxiv.org/pdf/2310.12345.pdf")
                .unwrap()
                .as_str(),
            "2310.12345"
        );
        assert_eq!(
            ArxivId::parse("hep-th/9901001").unwrap().file_stem(),
            "hep-th_9901001"
        );
        assert!(ArxivId::parse("not an id").is_err());
    }

    pub fn tarball(files: &[(&str, &str)]) -> Vec<u8> {
        let mut b = tar::Builder::new(vec![]);
        for (name, body) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, body.as_bytes()).unwrap();
        }
        let raw = b.into_inner().unwrap();
        let mut gz = GzEncoder::new(vec![], Compression::default());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    const ATOM: &str = r#"<?xml version="1.0"?><feed><entry><id>http://arxiv.org/abs/2101.00001v1</id><published>2021-01-01T00:00:00Z</published>
<title>A Mock
  Paper &amp; More</title><summary> The abstract. </summary><author><name>Ellrin Example</name></author><author><name>Daidai Sample</name></author>
<category term="cs.CV" scheme="x"/></entry></feed>"#;

    #[tokio::test]
    async fn mock_fetch_unpack_extract() {
        let tex = "\\documentclass{article}\n\\begin{document}\n\\section{Intro}\nHello \\cite{k}.\n\\begin{figure}\\includegraphics{figs/a}\\caption{Arch.}\\label{f}\\end{figure}\n\\bibliography{refs}\n\\end{document}\n";
        let mut m = MockArxiv::default();
        m.eprints.insert(
            "2101.00001".into(),
            tarball(&[
                ("ms.tex", tex),
                ("figs/a.png", "png"),
                ("refs.bib", "@misc{k, title={K}}"),
            ]),
        );
        m.atoms.insert("2101.00001".into(), ATOM.into());
        let dir = tempfile::tempdir().unwrap();
        let id = ArxivId::parse("2101.00001").unwrap();
        let (un, cat) = fetch_and_extract(&m, &id, dir.path()).await.unwrap();
        assert_eq!(un, Unpacked::Source(dir.path().join("2101.00001")));
        let cat = cat.unwrap();
        assert_eq!(cat.meta.title, "A Mock Paper & More");
        assert_eq!(cat.meta.authors, vec!["Ellrin Example", "Daidai Sample"]);
        assert_eq!(cat.meta.arxiv_id.as_deref(), Some("2101.00001"));
        assert_eq!(cat.asset("fig1").unwrap().files, vec!["figs/a.png"]);
        assert_eq!(cat.references.len(), 1);
        assert_eq!(
            *m.calls.lock().unwrap(),
            vec!["eprint:2101.00001", "meta:2101.00001"]
        );
    }

    #[test]
    fn unpack_variants_and_safety() {
        let dir = tempfile::tempdir().unwrap();
        // gzipped single file
        let mut gz = GzEncoder::new(vec![], Compression::default());
        gz.write_all(b"\\documentclass{article}").unwrap();
        let un = unpack(&gz.finish().unwrap(), &dir.path().join("a")).unwrap();
        assert_eq!(un, Unpacked::Source(dir.path().join("a")));
        assert!(dir.path().join("a/main.tex").is_file());
        // PDF
        assert!(matches!(
            unpack(b"%PDF-1.5 ...", &dir.path().join("b")).unwrap(),
            Unpacked::Pdf(_)
        ));
        // path traversal is refused
        let mut raw = vec![0u8; 1024];
        let name = b"../evil.tex";
        raw[..name.len()].copy_from_slice(name);
        let mut h = tar::Header::new_old();
        h.as_mut_bytes()[..name.len()].copy_from_slice(name);
        h.set_size(1);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        let mut evil = h.as_bytes().to_vec();
        evil.extend_from_slice(b"x");
        evil.resize(512 * 2, 0);
        evil.extend_from_slice(&[0u8; 1024]);
        assert!(matches!(
            unpack(&evil, &dir.path().join("c")),
            Err(Error::UnsafePath(_))
        ));
        assert!(!dir.path().join("evil.tex").exists());
    }

    #[test]
    fn atom() {
        let m = parse_atom(ATOM).unwrap();
        assert_eq!(m.id, "2101.00001v1");
        assert_eq!(m.summary, "The abstract.");
        assert_eq!(m.categories, vec!["cs.CV"]);
        assert!(parse_atom("<feed></feed>").is_err());
    }
}
