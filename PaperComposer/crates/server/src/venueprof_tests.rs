//! The 期刊寫法 job on mocks: OpenAlex and the PDFs from `MockHttp`, a fake extractor, a fake
//! model; temp folders for the cache and the workspace. No network, no model, no poppler.

use std::sync::Mutex;

use lore_research::{MockHttp, Response};

use super::*;

fn pdf(tag: &str) -> Vec<u8> {
    format!("%PDF-1.4\n% synthetic {tag}\n%%EOF\n").into_bytes()
}

fn filler(n: usize, tag: &str) -> String {
    (0..n).map(|i| format!("{tag}{}", i % 5)).collect::<Vec<_>>().join(" ")
}

/// Sections for the synthetic PDF (by its tag); `short` = a PDF without enough prose.
struct FakeExtract;

impl Extract for FakeExtract {
    fn extract(&self, pdf: &Path, _out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        let text = String::from_utf8_lossy(&std::fs::read(pdf).map_err(|e| e.to_string())?).to_string();
        if text.contains("short") {
            return Ok(lore_docsrc::AssetCatalog { sections: vec![lore_docsrc::Section { id: "s1".into(), level: 1, title: "Note".into(), paragraphs: vec!["Too short.".into()], ..Default::default() }], ..Default::default() });
        }
        let sec = |id: &str, level: u8, title: &str, paras: Vec<String>| lore_docsrc::Section { id: id.into(), level, title: title.into(), paragraphs: paras, ..Default::default() };
        let mut eq = lore_docsrc::Asset::new("eq1", lore_docsrc::AssetKind::Equation);
        eq.number = Some("1".into());
        eq.section = Some("s2".into());
        let mut fig = lore_docsrc::Asset::new("fig1", lore_docsrc::AssetKind::Figure);
        fig.number = Some("1".into());
        fig.caption = Some("Overview of the proposed pipeline.".into());
        Ok(lore_docsrc::AssetCatalog {
            sections: vec![
                sec("s1", 1, "1. Introduction", vec![format!("Clinical screening matters for patients [1]. {}", filler(300, "intro"))]),
                sec("s2", 1, "2. Methods", vec![format!("We propose a widget network. {}", filler(500, "meth"))]),
                sec("s3", 1, "3. Results", vec![format!("The proposed method achieves good scores. {}", filler(400, "res"))]),
                sec("s4", 1, "4. Discussion", vec![format!("A limitation of this study is its size. {}", filler(300, "dis"))]),
            ],
            assets: vec![eq, fig],
            ..Default::default()
        })
    }
}

/// Labels every paragraph by its first word; logs every prompt it was given.
#[derive(Default)]
struct FakeModel {
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl BookModel for FakeModel {
    async fn json(&self, ask: Ask<'_>, on: crate::book::model::OnChars<'_>) -> Result<(Value, String), StageError> {
        assert_eq!(ask.task, TASK);
        self.prompts.lock().unwrap().push(format!("{}\n{}", ask.system, ask.user));
        on(10);
        let mut paras = vec![];
        for (i, line) in ask.user.lines().filter(|l| l.starts_with("[P")).enumerate() {
            let body = line.split_once("] ").map(|x| x.1).unwrap_or("");
            let c = if body.starts_with("Clinical") { "value" } else if body.starts_with("We propose") { "method" } else if body.starts_with("The proposed") { "results" } else { "limitations" };
            paras.push(json!({ "p": i, "c": c }));
        }
        Ok((json!({ "sections": [], "paragraphs": paras, "points": ["pros_cons"] }), "fake:model".into()))
    }
}

fn work(n: usize, kind: &str, locs: Value) -> Value {
    json!({
        "id": format!("https://openalex.org/W{n}"), "doi": format!("https://doi.org/10.5555/jdi.{n}"),
        "display_name": format!("Demo widget article {n}"), "publication_year": 2026, "publication_date": format!("2026-0{}-01", (n % 9) + 1),
        "type": kind, "open_access": { "is_oa": true }, "locations": locs, "keywords": [{ "display_name": if n == 6 { "Retina" } else { "Widgets" } }]
    })
}

fn loc(url: &str, licence: Option<&str>, host: &str) -> Value {
    json!({ "is_oa": true, "pdf_url": url, "landing_page_url": url, "license": licence, "version": "publishedVersion", "source": { "display_name": host, "type": if host == "arXiv" { "repository" } else { "journal" } } })
}

fn works() -> String {
    json!({ "results": [
        work(1, "article", json!([loc("https://example.org/a1.pdf", Some("cc-by"), "Journal of Demo Imaging")])),
        work(2, "article", json!([loc("https://example.org/a2.pdf", None, "Journal of Demo Imaging")])),           // no licence
        work(3, "editorial", json!([loc("https://example.org/a3.pdf", Some("cc-by"), "Journal of Demo Imaging")])),  // not research
        work(4, "article", json!([loc("https://example.org/a4.pdf", Some("cc-by-nc"), "Journal of Demo Imaging"), loc("https://pmc.ncbi.nlm.nih.gov/articles/PMC4/pdf", Some("cc-by-nc"), "PubMed Central")])), // landing page, then PMC
        work(5, "article", json!([loc("https://arxiv.org/pdf/2601.00005", Some("cc-by"), "arXiv")])),              // arXiv: its own licence
        work(6, "article", json!([loc("https://example.org/a6.pdf", Some("cc-by"), "Journal of Demo Imaging")])),
        work(7, "article", json!([loc("https://example.org/a7.pdf", Some("cc-by"), "Journal of Demo Imaging")])),   // too short to read
    ]})
    .to_string()
}

fn mock() -> Arc<MockHttp> {
    Arc::new(
        MockHttp::new()
            .on("api.openalex.org/works", Response::ok(works()))
            .on("example.org/a1.pdf", Response::ok(pdf("one")))
            .on("example.org/a4.pdf", Response::ok("<html>landing page</html>"))
            .on("PMC4/pdf", Response::ok(pdf("four")))
            .on("export.arxiv.org/oai2", Response::ok("<record><arXiv><license>http://creativecommons.org/licenses/by/4.0/</license></arXiv></record>"))
            .on("arxiv.org/pdf/2601.00005", Response::ok(pdf("five")))
            .on("example.org/a6.pdf", Response::ok(pdf("six")))
            .on("example.org/a7.pdf", Response::ok(pdf("short"))),
    )
}

fn ident() -> Identity {
    Identity { openalex: "S900001".into(), name: "Journal of Demo Imaging".into(), issn_l: "1234-5678".into(), ..Default::default() }
}

#[tokio::test]
async fn the_job_reads_open_articles_and_saves_a_profile() {
    let cache = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let http = mock();
    let model = Arc::new(FakeModel::default());
    let mut research = lore_research::Config { pace: lore_research::Pace::none(), ..Default::default() };
    research.mailto = None;
    let deps = Arc::new(Deps { cache: cache.path().to_path_buf(), http: http.clone(), research, model: model.clone(), extract: Arc::new(FakeExtract) });
    let js = Jobs::open(cache.path().join("jobs"), Arc::new(VenueJob { deps }));
    let p = Params { template: "jdi".into(), workspace: ws.path().to_path_buf(), identity: ident(), kind: "article".into(), topic: vec!["private retina keyword".into()], today: "2026-11-14".into(), engine: None, local: vec![] };
    let job = js.create(serde_json::to_value(&p).unwrap(), true, "Journal of Demo Imaging").unwrap();
    job.settled().await;
    assert_eq!(job.status(), status::DONE, "{:?}", job.meta().error);

    // the profile: four articles read (1, 4 via PMC, 5 via arXiv, 6); 7 too short, 2 and 3 skipped
    let prof = vp::load(ws.path(), "jdi").unwrap();
    assert_eq!(prof.articles.len(), 4);
    assert_eq!(prof.made, "2026-11-14");
    assert_eq!(prof.engine, "fake:model");
    // the topic ranked work 6 (keyword Retina) first, locally
    assert_eq!(prof.articles[0].doi, "10.5555/jdi.6");
    let copies: Vec<(&str, &str)> = prof.articles.iter().map(|a| (a.doi.as_str(), a.copy.as_str())).collect();
    assert!(copies.contains(&("10.5555/jdi.4", "pmc")) && copies.contains(&("10.5555/jdi.5", "arxiv")), "{copies:?}");
    assert_eq!(prof.articles.iter().find(|a| a.doi == "10.5555/jdi.4").unwrap().licence, "cc-by-nc");
    assert_eq!(prof.skipped.get("沒有標明授權"), Some(&1));
    assert_eq!(prof.skipped.get("不是研究文章"), Some(&1));
    assert_eq!(prof.skipped.get("讀不出章節"), Some(&1));
    // proportions computed in code: methods 501 of 1505 body words in every article
    let m = prof.sections.iter().find(|s| s.role == "methods").unwrap();
    assert!((m.share.mean - 506.0 / 1525.0).abs() < 0.01, "{:?}", m.share);
    assert!((m.equation_share.mean - 1.0).abs() < 1e-9);
    assert_eq!(prof.habits.iter().find(|h| h.id == "fig1_overview").map(|h| h.count), Some(4));
    assert!(ws.path().join("venues/jdi/profile.md").is_file());

    // downloads in the cache, never in the workspace; one model call per article
    assert!(std::fs::read_dir(cache.path().join("pdf")).unwrap().count() >= 4);
    assert!(!ws.path().join("pdf").exists());
    assert_eq!(model.prompts.lock().unwrap().len(), 4);

    // privacy: only the venue went out — no topic, no e-mail
    for r in http.requests() {
        assert!(!r.url.contains("retina") && !r.url.contains("private") && !r.url.contains("mailto"), "{}", r.url);
        assert!(r.body.is_none());
    }
    let works_url = http.urls().into_iter().find(|u| u.contains("/works?")).unwrap();
    assert!(works_url.contains("S900001") && works_url.contains("type%3Aarticle") && works_url.contains("from_publication_date%3A2024-11-14"), "{works_url}");
    // closed / unlicensed copies are never requested
    assert!(!http.urls().iter().any(|u| u.contains("a2.pdf") || u.contains("a3.pdf")));
    // nothing the model saw came from the author
    assert!(!model.prompts.lock().unwrap().iter().any(|p| p.contains("private retina")));

    // made again from the labels: the labelled articles are not sent again
    js.redo_from(&job, "label").unwrap();
    job.settled().await;
    assert_eq!(job.status(), status::DONE);
    assert_eq!(model.prompts.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn offline_fails_the_job_with_one_line() {
    let cache = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let deps = Arc::new(Deps { cache: cache.path().to_path_buf(), http: Arc::new(MockHttp::new()), research: lore_research::Config { pace: lore_research::Pace::none(), ..Default::default() }, model: Arc::new(FakeModel::default()), extract: Arc::new(FakeExtract) });
    let js = Jobs::open(cache.path().join("jobs"), Arc::new(VenueJob { deps }));
    let p = Params { template: "jdi".into(), workspace: ws.path().to_path_buf(), identity: ident(), kind: "article".into(), today: "2026-11-14".into(), ..Default::default() };
    let job = js.create(serde_json::to_value(&p).unwrap(), true, "x").unwrap();
    job.settled().await;
    assert_eq!(job.status(), status::FAILED);
    assert_eq!(job.meta().error.unwrap()["error"].as_str().map(|e| e.contains("沒有網路")), Some(true));
    assert!(vp::load(ws.path(), "jdi").is_none());
}

#[test]
fn choosing_keeps_research_articles_with_a_licence() {
    let ws: Vec<Work> = lore_research::venue::parse_works(&serde_json::from_str(&works()).unwrap());
    let (keep, skipped) = choose(ws, "article", &[]);
    let dois: Vec<String> = keep.iter().map(|w| w.record.doi.clone().unwrap()).collect();
    assert_eq!(dois.len(), 5);
    assert!(!dois.iter().any(|d| d.ends_with(".2") || d.ends_with(".3")));
    assert_eq!(skipped["沒有標明授權"], 1);
    assert_eq!(skipped["不是研究文章"], 1);
}

fn base(id: &str, name: &str, publisher: &str, kind: &str) -> pc_core::journal::Journal {
    pc_core::journal::Journal {
        id: id.into(),
        meta: pc_core::journal::Meta { name: name.into(), publisher: publisher.into(), kind: kind.into(), ..Default::default() },
        class: String::new(),
        class_options: String::new(),
        bib_style: String::new(),
        bib_styles: vec![],
        has_sample: true,
        cleveref: false,
        hidden: true,
    }
}

#[test]
fn a_new_journal_finds_its_base_and_requirements() {
    let bases = [base("demo-press", "Demo Press article（通用）", "Demo Press", "journal"), base("ieee-like", "Institute Trans（通用）", "IEEE", "journal"), base("ieee-conf", "Institute Conference（通用）", "IEEE", "conference")];
    assert_eq!(base_for(&bases, "Demo Press BV", false).as_deref(), Some("demo-press"));
    assert_eq!(base_for(&bases, "Institute of Electrical and Electronics Engineers", false).as_deref(), Some("ieee-like"));
    assert_eq!(base_for(&bases, "Institute of Electrical and Electronics Engineers", true).as_deref(), Some("ieee-conf"));
    assert_eq!(base_for(&bases, "Unknown House", false), None);
    assert_eq!(base_for(&bases, "", false), None);

    let ws = tempfile::tempdir().unwrap();
    assert!(requirements_for(ws.path(), None, Some(&ident())).is_none());
    std::fs::create_dir_all(ws.path().join("venues")).unwrap();
    std::fs::write(ws.path().join("venues/requirements.toml"), "[journal.\"1234-5678\"]\nbase = \"demo-press\"\n[journal.\"1234-5678\".submission]\nabstract_words = 200\nrequired_statements = [\"Data availability\"]\nchecked = \"2026-11-10\"\n").unwrap();
    let (b, sub) = requirements_for(ws.path(), None, Some(&ident())).unwrap();
    assert_eq!(b.as_deref(), Some("demo-press"));
    assert_eq!(Submission::from_table(sub.as_ref()).unwrap().abstract_words, Some(200));

    // by template id: written into the existing journal templates it lists
    let pubs = ws.path().join("publishers");
    for (id, toml) in [("jdi", "name = \"Journal of Demo Imaging\"\n"), ("other", "name = \"Other Demo Journal\"\n[venue]\nopenalex = \"S9\"\nname = \"Other Demo Journal\"\nissn_l = \"1234-5678\"\n")] {
        std::fs::create_dir_all(pubs.join(id)).unwrap();
        std::fs::write(pubs.join(id).join("template.toml"), toml).unwrap();
    }
    let mut t = std::fs::read_to_string(ws.path().join("venues/requirements.toml")).unwrap();
    t.push_str("[template.jdi.submission]\nwords = 6000\nchecked = \"2026-11-12\"\n");
    std::fs::write(ws.path().join("venues/requirements.toml"), t).unwrap();
    let mut up = apply_requirements(ws.path());
    up.sort();
    assert_eq!(up, ["jdi", "other"]);
    let jdi = pc_core::journal::load(&pubs, "jdi").unwrap();
    assert_eq!(Submission::from_table(jdi.meta.submission.as_ref()).unwrap().words, Some(6000));
    assert_eq!(jdi.meta.name, "Journal of Demo Imaging");
    assert!(apply_requirements(ws.path()).is_empty(), "nothing changes twice");
}

#[test]
fn keywords_of_the_paper_stay_local_input() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\begin{keywords}\nretina \\sep widgets; demo\n\\end{keywords}\n\\end{document}\n").unwrap();
    assert_eq!(paper_topic(d.path()), ["retina", "widgets", "demo"]);
}

/// The real extractor (crops and catalog off) on lore-docsrc's synthetic two-column PDF,
/// through what the job keeps. Needs poppler; skipped without it.
#[test]
fn the_real_extractor_gives_sections_captions_and_placed_equations() {
    if !lore_docsrc::pdf::poppler::Poppler::discover().available() {
        eprintln!("poppler not found: skipped");
        return;
    }
    let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../commons/crates/lore-docsrc/tests/fixtures/twocol.pdf");
    let out = tempfile::tempdir().unwrap();
    let cat = TextExtract.extract(&pdf, out.path()).unwrap();
    assert!(!out.path().join("crops").exists() && !out.path().join("catalog.json").exists());
    let mut a = ArticleText { sections: article_of(&cat), ..Default::default() };
    fill_assets(&mut a, &cat);
    let titles: Vec<&str> = a.sections.iter().map(|s| s.title.as_str()).collect();
    assert!(titles.contains(&"Introduction") && titles.contains(&"Method") && titles.contains(&"Results"), "{titles:?}");
    let method = a.sections.iter().position(|s| s.title == "Method").unwrap();
    assert!(a.equations.len() >= 2 && a.equations.iter().all(|e| *e == Some(method)), "{:?}", a.equations);
    assert!(a.captions.iter().any(|c| c.kind == "figure" && c.number == "1") && a.captions.iter().any(|c| c.kind == "table"));
    assert_eq!(vp::canonical_role(&a.sections[method].title), "methods");
}
