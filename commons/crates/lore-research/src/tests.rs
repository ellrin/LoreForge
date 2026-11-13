use std::sync::Arc;

use serde_json::json;

use super::*;

const OA_SEARCH: &str = include_str!("../tests/fixtures/openalex_search.json");
const OA_WORK: &str = include_str!("../tests/fixtures/openalex_work.json");
const AX_SEARCH: &str = include_str!("../tests/fixtures/arxiv_search.xml");
const AX_ERROR: &str = include_str!("../tests/fixtures/arxiv_error.xml");
const S2_SEARCH: &str = include_str!("../tests/fixtures/s2_search.json");
const S2_MATCH: &str = include_str!("../tests/fixtures/s2_match.json");

fn cfg() -> Config {
    Config { pace: Pace::none(), ..Config::default() }
}
fn client(m: MockHttp) -> (Client, Arc<MockHttp>) {
    let m = Arc::new(m);
    (Client::with_http(cfg(), m.clone()), m)
}
fn pdf_bytes(tag: &str) -> Vec<u8> {
    format!("%PDF-1.4\n% synthetic {tag}\n1 0 obj << >> endobj\ntrailer << >>\n%%EOF\n").into_bytes()
}

#[test]
fn parses_openalex() {
    let rs = openalex::parse(&serde_json::from_str(OA_SEARCH).unwrap());
    assert_eq!(rs.len(), 3);
    let a = &rs[0];
    assert_eq!(a.title, "Sparse Widgets for Synthetic Demo Tasks");
    assert_eq!(a.authors, ["Ellrin Example", "Daidai Example"]);
    assert_eq!(a.year, Some(2024));
    assert_eq!(a.venue.as_deref(), Some("Journal of Demo Studies"));
    assert_eq!(a.doi.as_deref(), Some("10.5555/demo.2024.001"));
    assert_eq!(a.arxiv.as_deref(), Some("2401.00001"));
    assert_eq!(a.abstract_text.as_deref(), Some("We study demo widgets with demo data."));
    assert!(a.is_oa);
    assert_eq!(a.pdf_url.as_deref(), Some("https://arxiv.org/pdf/2401.00001"));
    assert_eq!(a.citations, Some(12));
    // closed: no PDF, never a download candidate
    assert!(!rs[1].is_oa && rs[1].pdf_url.is_none() && !rs[1].open_access());
    assert_eq!(rs[2].doi, None);
    assert_eq!(rs[2].pdf_url.as_deref(), Some("https://example.org/papers/gold-3.pdf"));
    // one work (by DOI)
    let w = openalex::parse(&serde_json::from_str(OA_WORK).unwrap());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].doi.as_deref(), Some("10.5555/demo.2024.001"));
}

#[test]
fn parses_arxiv_atom() {
    let rs = arxiv::parse(AX_SEARCH);
    assert_eq!(rs.len(), 2);
    let a = &rs[0];
    assert_eq!(a.arxiv.as_deref(), Some("2401.00001"));
    assert_eq!(a.title, "Sparse Widgets for Synthetic Demo Tasks");
    assert_eq!(a.abstract_text.as_deref(), Some("We study demo widgets with demo data & placeholders."));
    assert_eq!(a.authors, ["Ellrin Example", "Daidai Example"]);
    assert_eq!(a.year, Some(2024));
    assert_eq!(a.doi.as_deref(), Some("10.5555/demo.2024.001"));
    assert!(a.is_oa);
    assert_eq!(a.pdf_url.as_deref(), Some("http://arxiv.org/pdf/2401.00001v3"));
    assert_eq!(rs[1].venue.as_deref(), Some("Demo Letters 1 (2024)"));
    assert!(arxiv::parse(AX_ERROR).is_empty());
    assert!(arxiv::parse("<feed></feed>").is_empty());
}

#[test]
fn parses_s2() {
    let rs = s2::parse(&serde_json::from_str(S2_SEARCH).unwrap());
    assert_eq!(rs.len(), 2);
    assert_eq!(rs[0].doi.as_deref(), Some("10.5555/demo.2024.001"));
    assert_eq!(rs[0].arxiv.as_deref(), Some("2401.00001"));
    assert_eq!(rs[0].citations, Some(15));
    assert!(rs[0].is_oa);
    // empty openAccessPdf url = none; empty venue = none
    assert!(!rs[1].is_oa && rs[1].pdf_url.is_none() && rs[1].venue.is_none());
    let m = s2::parse(&serde_json::from_str(S2_MATCH).unwrap());
    assert_eq!(m[0].arxiv.as_deref(), Some("2402.12345"));
}

#[test]
fn ids_normalise() {
    assert_eq!(norm_doi("https://doi.org/10.1000/ABC.1"), "10.1000/abc.1");
    assert_eq!(norm_doi("doi:10.1000/x."), "10.1000/x");
    assert_eq!(norm_arxiv("arXiv:2101.00001v12"), "2101.00001");
    assert_eq!(norm_arxiv("hep-th/9901001v2"), "hep-th/9901001");
    assert_eq!(norm_arxiv("2101.00001"), "2101.00001");
    assert_eq!(arxiv_from_url("https://arxiv.org/pdf/2101.00001v2.pdf").as_deref(), Some("2101.00001"));
    assert_eq!(arxiv_from_url("http://arxiv.org/api/errors#x"), None);
    assert_eq!(find_doi("see https://doi.org/10.5555/demo.2024.001, p. 3").as_deref(), Some("10.5555/demo.2024.001"));
    assert_eq!(find_doi("version 10.5 of the tool"), None);
    assert_eq!(find_arxiv("Example et al., arXiv:2402.12345v1 (2024)").as_deref(), Some("2402.12345"));
    assert_eq!(find_arxiv("Sparse Widgets for Synthetic Demo Tasks"), None);
}

#[test]
fn dedupe_merges_across_sources() {
    let mut all = openalex::parse(&serde_json::from_str(OA_SEARCH).unwrap());
    all.extend(arxiv::parse(AX_SEARCH));
    all.extend(s2::parse(&serde_json::from_str(S2_SEARCH).unwrap()));
    let d = dedupe(all);
    let titles: Vec<_> = d.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["Sparse Widgets for Synthetic Demo Tasks", "A Closed Placeholder Study", "Gold Access Demo Report", "Notes on Placeholder Gradients", "Closed Demo Benchmark"]);
    let a = &d[0];
    assert_eq!(a.sources, [Source::OpenAlex, Source::Arxiv, Source::SemanticScholar]);
    assert_eq!(a.citations, Some(15));
    assert!(a.pdf_url.as_deref().unwrap().contains("arxiv.org/"));
    // by normalised title alone
    let x = Record { title: "Notes on placeholder gradients!".into(), source: Source::SemanticScholar, sources: vec![Source::SemanticScholar], ..Default::default() };
    let y = Record { title: "Notes on Placeholder Gradients".into(), year: Some(2024), ..Default::default() };
    let d = dedupe(vec![x, y]);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].year, Some(2024));
}

#[test]
fn bibtex_keys_and_escaping() {
    let a = Record { title: "Gains of 50% & more: the $x_1$ {case} ~ #1".into(), authors: vec!["Ellrin Example".into(), "Example, Daidai".into()], year: Some(2024), ..Default::default() };
    assert_eq!(bibtex::key(&a), "example2024gains");
    let mut b = a.clone();
    b.title = "The Gains Again".into();
    let c = Record { doi: Some("10.5555/x(1)y".into()), venue: Some("J. Demo & Co".into()), ..a.clone() };
    let d = Record { arxiv: Some("2401.00001".into()), ..Default::default() };
    let (keys, text) = bibtex::bib(&[a.clone(), b, c, d, a]);
    assert_eq!(keys, ["example2024gains", "example2024gainsa", "doi:10.5555/x-1-y", "arXiv:2401.00001", "example2024gainsb"]);
    assert!(text.contains("title = {{Gains of 50\\% \\& more: the \\$x\\_1\\$ \\{case\\} \\textasciitilde{} \\#1}}"), "{text}");
    assert!(text.contains("author = {Ellrin Example and Example, Daidai}"));
    assert!(text.contains("@article{doi:10.5555/x-1-y,\n") && text.contains("journal = {J. Demo \\& Co}"));
    assert!(text.contains("@misc{arXiv:2401.00001,\n") && text.contains("eprint = {2401.00001}") && text.contains("archivePrefix = {arXiv}"));
    assert!(text.contains("url = {https://doi.org/10.5555/x(1)y}"));
    // a key never holds what \cite cannot take
    for k in keys {
        assert!(!k.contains([',', '{', '}', ' ', '(', ')', '%', '#', '~', '\\']), "{k}");
    }
}

#[tokio::test]
async fn search_all_sources_with_mailto_only_when_set() {
    let m = MockHttp::new().on("api.openalex.org", Response::ok(OA_SEARCH)).on("export.arxiv.org", Response::ok(AX_SEARCH)).on("api.semanticscholar.org", Response::ok(S2_SEARCH));
    let (c, log) = client(m);
    let f = c.search("sparse demo widgets", 10).await;
    assert!(f.errors.is_empty());
    assert_eq!(f.records.len(), 5);
    let urls = log.urls();
    assert_eq!(urls.len(), 3);
    assert!(urls[0].starts_with("https://api.openalex.org/works?search=sparse+demo+widgets"), "{}", urls[0]);
    assert!(urls.iter().all(|u| !u.contains("mailto")));
    assert!(urls[1].contains("search_query=all%3Asparse+AND+all%3Ademo"), "{}", urls[1]);
    assert!(log.requests()[2].headers.iter().all(|(k, _)| k != "x-api-key"));
    // with the author's settings
    let m = Arc::new(MockHttp::new().on("api.openalex.org", Response::ok(OA_SEARCH)).on("api.semanticscholar.org", Response::ok(S2_SEARCH)));
    let c = Client::with_http(Config { mailto: Some("demo-contact".into()), s2_api_key: Some("k".into()), sources: vec![Source::OpenAlex, Source::SemanticScholar], ..cfg() }, m.clone());
    c.search("demo", 5).await;
    assert!(m.urls()[0].contains("mailto=demo-contact"));
    assert!(m.requests()[1].headers.contains(&("x-api-key".into(), "k".into())));
}

#[tokio::test]
async fn rate_limit_retries_then_gives_up() {
    let m = MockHttp::new().seq("api.openalex.org", vec![Response::status(429, "").with_header("Retry-After", "0"), Response::ok(OA_SEARCH)]);
    let (c, log) = client(m);
    let r = c.search_in(Source::OpenAlex, "demo", 5).await.unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(log.urls().len(), 2);
    let (c, log) = client(MockHttp::new().on("api.semanticscholar.org", Response::status(429, "")));
    let e = c.search_in(Source::SemanticScholar, "demo", 5).await.unwrap_err();
    assert!(matches!(e, Error::RateLimited("Semantic Scholar")), "{e:?}");
    assert_eq!(log.urls().len(), 4); // 1 + 3 retries
    assert_eq!(e.to_string(), "Semantic Scholar 要求放慢，稍後再試");
}

#[tokio::test]
async fn offline_degrades() {
    let (c, log) = client(MockHttp::new());
    let f = c.search("demo", 5).await;
    assert!(f.offline());
    assert!(f.records.is_empty());
    assert_eq!(log.urls().len(), 1, "after the first offline source the others are skipped");
    assert_eq!(f.errors[0].1.to_string(), "沒有網路");
    // one source failing (not offline) keeps the rest
    let (c, _) = client(MockHttp::new().on("api.openalex.org", Response::status(500, "")).on("export.arxiv.org", Response::ok(AX_SEARCH)).on("api.semanticscholar.org", Response::ok("{not json")));
    let f = c.search("demo", 5).await;
    assert!(!f.offline());
    assert_eq!(f.records.len(), 2);
    assert!(matches!(f.errors[0], (Source::OpenAlex, Error::Status { status: 500, .. })));
    assert!(matches!(f.errors[1], (Source::SemanticScholar, Error::Parse(_))));
    assert!(!c.resolve("10.5555/demo.2024.001").await.unwrap_err().is_offline());
    let (c, _) = client(MockHttp::new());
    assert!(c.resolve("10.5555/demo.2024.001").await.unwrap_err().is_offline());
}

#[tokio::test]
async fn resolves_by_doi_arxiv_and_title() {
    let m = MockHttp::new()
        .on("api.openalex.org/works/doi:10.5555/demo.2024.001", Response::ok(OA_WORK))
        .on("graph/v1/paper/DOI:", Response::status(404, r#"{"error":"Paper not found"}"#))
        .on("id_list=2402.12345", Response::ok(AX_SEARCH))
        .on("graph/v1/paper/ARXIV:2402.12345", Response::ok(json!({ "title": "Notes on Placeholder Gradients", "externalIds": { "ArXiv": "2402.12345" }, "isOpenAccess": true, "citationCount": 1 }).to_string()))
        .on("filter=title.search", Response::ok(r#"{"results": []}"#))
        .on("export.arxiv.org/api/query?search_query=ti", Response::ok(AX_SEARCH))
        .on("paper/search/match", Response::status(404, r#"{"error":"Title match not found"}"#));
    let (c, log) = client(m);
    let r = c.resolve("https://doi.org/10.5555/DEMO.2024.001").await.unwrap().unwrap();
    assert_eq!(r.title, "Sparse Widgets for Synthetic Demo Tasks");
    let r = c.resolve("arXiv:2402.12345v1").await.unwrap().unwrap();
    assert!(r.sources.contains(&Source::Arxiv));
    // by title: only a close title counts (the feed also has another paper)
    let r = c.resolve("Notes on placeholder gradients").await.unwrap().unwrap();
    assert_eq!(r.arxiv.as_deref(), Some("2402.12345"));
    assert!(log.urls().iter().any(|u| u.contains("filter=title.search%3ANotes+on+placeholder+gradients")));
    assert!(c.resolve("Nothing Like This Exists Anywhere").await.unwrap().is_none());
}

#[tokio::test]
async fn downloads_open_access_only() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = pdf_bytes("a");
    let m = MockHttp::new()
        .on("arxiv.org/pdf/2401.00001", Response::status(503, "busy"))
        .on("example.org/papers/gold-3.pdf", Response::ok(pdf.clone()))
        .on("example.org/papers/landing.pdf", Response::ok("<!doctype html><html>Sign in</html>"));
    let m = Arc::new(m);
    let c = Client::with_http(Config { retries: 0, ..cfg() }, m.clone());
    let rs = openalex::parse(&serde_json::from_str(OA_SEARCH).unwrap());
    // paywalled: refused, no request
    let n = m.urls().len();
    assert!(matches!(c.download(&rs[1], dir.path()).await, Err(Error::NotOpenAccess)));
    assert_eq!(m.urls().len(), n);
    // gold OA PDF: saved as <sha>.pdf
    let d = c.download(&rs[2], dir.path()).await.unwrap();
    assert_eq!(d.path, dir.path().join(format!("{}.pdf", d.sha256)));
    assert_eq!(std::fs::read(&d.path).unwrap(), pdf);
    assert_eq!(d.sha256.len(), 64);
    // again: same file, no duplicate
    let again = c.download(&rs[2], dir.path()).await.unwrap();
    assert_eq!(again.path, d.path);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    // a landing page instead of a PDF
    let html = Record { is_oa: true, pdf_url: Some("https://example.org/papers/landing.pdf".into()), ..Default::default() };
    assert!(matches!(c.download(&html, dir.path()).await, Err(Error::NotPdf)));
    // arXiv busy, nothing else
    assert!(matches!(c.download(&rs[0], dir.path()).await, Err(Error::Status { status: 503, .. })));
    // too large
    let small = Client::with_http(Config { max_pdf_bytes: 10, ..cfg() }, m.clone());
    assert!(matches!(small.download(&rs[2], dir.path()).await, Err(Error::TooLarge { limit: 10 })));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    // offline
    let off = Client::with_http(cfg(), Arc::new(MockHttp::new()));
    assert!(off.download(&rs[2], dir.path()).await.unwrap_err().is_offline());
}

#[tokio::test]
async fn imports_into_lorereader_or_keeps_pending() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = ["a", "b", "c"]
        .iter()
        .map(|t| {
            let (p, _) = save_pdf(&pdf_bytes(t), dir.path(), 1 << 20).unwrap();
            p
        })
        .collect();
    let m = MockHttp::new().seq(
        "127.0.0.1:7890/api/papers/import",
        vec![
            Response::ok(json!({ "paper": { "sha256": "aa" }, "new": true }).to_string()),
            Response::status(400, json!({ "error": "讀不了這份 PDF" }).to_string()),
            Response::ok(json!({ "paper": { "sha256": "cc" }, "new": false }).to_string()),
        ],
    );
    let rep = import_into_reader(&m, "http://127.0.0.1:7890/", &files).await;
    assert_eq!(rep.imported.len(), 2);
    assert_eq!(rep.imported[0], Imported { path: files[0].clone(), sha: "aa".into(), new: true });
    assert_eq!(rep.failed, vec![(files[1].clone(), "讀不了這份 PDF".to_string())]);
    assert!(rep.pending.is_empty());
    let body: serde_json::Value = serde_json::from_slice(m.requests()[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(body["path"].as_str().map(PathBuf::from), Some(files[0].canonicalize().unwrap()));
    // LoreReader not running: everything pending, nothing lost
    let rep = import_into_reader(&MockHttp::new(), "http://127.0.0.1:7890", &files).await;
    assert_eq!(rep.pending, files);
    assert!(rep.imported.is_empty() && rep.failed.is_empty());
    let back: ImportReport = serde_json::from_str(&serde_json::to_string(&rep).unwrap()).unwrap();
    assert_eq!(back, rep);
}

#[test]
fn record_json_shape() {
    let r = arxiv::parse(AX_SEARCH).remove(0);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["source"], "arxiv");
    assert!(v.get("abstract").is_some());
    let s = serde_json::to_value(Source::SemanticScholar).unwrap();
    assert_eq!(s, "s2");
}

#[test]
fn licence_from_openalex_and_reuse_rule() {
    let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/openalex_search.json")).unwrap();
    let recs = crate::openalex::parse(&v);
    let gold = recs.iter().find(|r| r.title.starts_with("Gold")).unwrap();
    assert_eq!(gold.license.as_deref(), Some("cc-by"));
    assert!(recs.iter().find(|r| r.title.starts_with("Sparse")).unwrap().license.is_none());
    // the author's own study notes: every CC licence, CC0, public domain — with a citation and the licence in the caption
    for ok in ["cc-by", "CC BY", "cc-by-sa", "cc0", "public-domain", "https://creativecommons.org/licenses/by/4.0/", "cc-by-nc", "cc-by-nc-sa", "cc-by-nc-nd", "https://creativecommons.org/licenses/by-nc-nd/4.0/", "cc-by-nd"] {
        assert!(crate::figure_reuse_allowed(Some(ok)), "{ok}");
    }
    // unknown, publisher, all rights reserved: never
    for no in ["publisher-specific-oa", "other-oa", "arxiv-nonexclusive", "all-rights-reserved", "implied-oa", ""] {
        assert!(!crate::figure_reuse_allowed(Some(no)), "{no}");
    }
    assert!(!crate::figure_reuse_allowed(None));
    // a publication keeps the strict rule
    assert!(crate::figure_reuse_allowed_published(Some("cc-by")) && !crate::figure_reuse_allowed_published(Some("cc-by-nc")));
    // how a caption names them
    assert_eq!(crate::licence_label("cc-by-nc").as_deref(), Some("CC BY-NC"));
    assert_eq!(crate::licence_label("https://creativecommons.org/licenses/by-nc-sa/4.0/").as_deref(), Some("CC BY-NC-SA 4.0"));
    assert_eq!(crate::licence_label("cc0").as_deref(), Some("CC0"));
    assert_eq!(crate::licence_label("public-domain").as_deref(), Some("public domain"));
    assert_eq!(crate::licence_label("other-oa"), None);
}

#[test]
fn arxiv_states_its_own_licence() {
    let rec = |lic: &str| format!("<OAI-PMH><GetRecord><record><metadata><arXiv xmlns=\"http://arxiv.org/OAI/arXiv/\"><id>2401.00001</id>{lic}</arXiv></metadata></record></GetRecord></OAI-PMH>");
    assert_eq!(crate::arxiv::oai_license(&rec("<license>http://arxiv.org/licenses/nonexclusive-distrib/1.0/</license>")).as_deref(), Some("arxiv-nonexclusive"));
    assert_eq!(crate::arxiv::oai_license(&rec("")).as_deref(), Some("arxiv-nonexclusive"), "no licence named: arXiv's default");
    assert_eq!(crate::arxiv::oai_license(&rec("<license>http://creativecommons.org/licenses/by/4.0/</license>")).as_deref(), Some("cc-by"));
    assert_eq!(crate::arxiv::oai_license(&rec("<license>http://creativecommons.org/licenses/by-nc-sa/4.0/</license>")).as_deref(), Some("cc-by-nc-sa"));
    assert_eq!(crate::arxiv::oai_license(&rec("<license>http://creativecommons.org/publicdomain/zero/1.0/</license>")).as_deref(), Some("cc0"));
    assert_eq!(crate::arxiv::oai_license("<error code=\"idDoesNotExist\"/>"), None);
    assert!(crate::figure_reuse_allowed(Some("cc-by")) && !crate::figure_reuse_allowed(Some("arxiv-nonexclusive")) && crate::figure_reuse_allowed(Some("cc-by-nc-sa")));
    assert!(crate::arxiv::oai_url("arXiv:2401.00001v2").contains("oai%3AarXiv.org%3A2401.00001") || crate::arxiv::oai_url("2401.00001").contains("2401.00001"));
}
