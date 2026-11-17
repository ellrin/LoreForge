//! The whole 寫書 job in a temp folder: a synthetic L-Reader paper (the PDF read by a fake
//! extractor), a synthetic L-Brain, a mock model, MockHttp for OpenAlex / the OA PDF /
//! L-Reader (not running), tiny figure PDFs. No network, no real model, no keychain, nothing
//! under the real private folders.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lore_research::http::{MockHttp, Response};
use serde_json::{Value, json};

use super::model::{MockModel, TASK_CHAPTER, TASK_PLAN, TASK_SUGGEST};
use super::*;

const SHA: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const SHA2: &str = "6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b";
const DOI: &str = "10.5555/synth.2025.001";
const WEB_DOI: &str = "10.5555/synth.2023.777";

fn put(dir: &Path, file: &str, v: Value) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

/// L-Reader's files for the starting paper and one more read paper.
fn lorereader(root: &Path) -> lore_learn::reader::Reader {
    let lr = root.join("LoreReader");
    let p = lr.join("papers").join(SHA);
    put(&p, "paper.json", json!({ "schema": "lorereader.paper/1", "sha256": SHA, "title": "Gated Contrast for Synthetic Signals", "authors": ["Ellrin Example", "Daidai Example"], "year": 2025, "doi": format!("https://doi.org/{DOI}"), "added_at": "2026-11-10T09:00:00Z" }));
    put(&p, "reading.json", json!({ "schema": "x", "understood": ["p1"] }));
    put(&p, "chat.json", json!({ "schema": "lorereader.chat/1", "messages": [{ "role": "user", "text": "Why does the temperature scaling matter here?" }] }));
    put(&p, "quotes.json", json!({ "schema": "lorereader.quotes/1", "quotes": [{ "id": "q1", "text": "The gate keeps hard negatives.", "page": 3, "note": "" }] }));
    put(&p, "symbols.json", json!({ "schema": "lorereader.symbols/1", "entries": [{ "id": "tau", "kind": "symbol", "latex": "\\tau", "meaning_en": "temperature" }], "equations": [] }));
    let q = lr.join("papers").join(SHA2);
    put(&q, "paper.json", json!({ "schema": "lorereader.paper/1", "sha256": SHA2, "title": "A Primer on Cosine Similarity", "authors": "Daidai Example", "year": 2022, "added_at": "2026-11-01T09:00:00Z" }));
    let lib = root.join("library");
    std::fs::create_dir_all(lib.join(&SHA[..2])).unwrap();
    std::fs::write(lib.join(&SHA[..2]).join(format!("{SHA}.pdf")), b"%PDF-1.4 synthetic").unwrap();
    lore_learn::reader::Reader { papers: lr.join("papers"), library: lib }
}

/// The knowledge base: softmax known (skipped), cosine similarity learning, InfoNCE needs cosine similarity.
fn kb(root: &Path) -> PathBuf {
    let sp = root.join("kb");
    std::fs::create_dir_all(sp.join("nodes")).unwrap();
    std::fs::create_dir_all(sp.join("progress/ellrin")).unwrap();
    std::fs::write(sp.join("space.toml"), "id = \"me\"\nname = \"Me\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let node = |id: &str, title: &str, links: Value, state: Option<&str>| {
        put(&sp.join("nodes"), &format!("{id}.json"), json!({ "id": id, "kind": "concept", "title": title, "summary": format!("My note on {title}."), "links": links, "created_at": "2026-11-01T09:00:00+00:00", "updated_at": "2026-11-01T09:00:00+00:00" }));
        if let Some(st) = state {
            put(&sp.join("progress/ellrin"), &format!("{id}.json"), json!({ "state": st, "streak": 0, "next_check_at": null, "log": [{ "at": "2026-11-02T09:00:00+00:00", "event": "created" }] }));
        }
    };
    node("k1", "softmax", json!([]), Some("known"));
    node("k2", "cosine similarity", json!([]), Some("learning"));
    node("k3", "InfoNCE", json!([{ "to": "k2", "rel": "prerequisite" }]), None);
    sp
}

/// A valid 2×2 grey PNG.
fn tiny_png() -> Vec<u8> {
    use std::io::Write;
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        c.extend(kind);
        c.extend(data);
        c.extend(crc.sum().to_be_bytes());
        c
    };
    let mut ihdr = vec![];
    ihdr.extend(2u32.to_be_bytes());
    ihdr.extend(2u32.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]);
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
    z.write_all(&[0, 128, 128, 0, 128, 128]).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &z.finish().unwrap()));
    png.extend(chunk(b"IEND", &[]));
    png
}

struct FakeExtract;

impl source::Extract for FakeExtract {
    fn extract(&self, _pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        std::fs::create_dir_all(out.join("crops")).unwrap();
        std::fs::write(out.join("crops/fig1.png"), tiny_png()).unwrap();
        let mut c = lore_docsrc::AssetCatalog::default();
        c.meta.title = "Gated Contrast for Synthetic Signals".into();
        c.meta.abstract_text = Some("We train an encoder with the InfoNCE loss and a learned gate over hard negatives.".into());
        let sec = |id: &str, title: &str, ps: &[&str]| lore_docsrc::Section { id: id.into(), level: 1, title: title.into(), paragraphs: ps.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        c.sections = vec![
            sec("s1", "Introduction", &["Contrastive learning trains an encoder without labels. The InfoNCE loss compares a positive pair with hard negatives using cosine similarity and a softmax with temperature scaling."]),
            sec("s2", "Method", &["Our gate selects hard negatives before the InfoNCE loss. Temperature scaling sharpens the softmax. Hard negatives matter most when the cosine similarity is high."]),
        ];
        c.assets.push(lore_docsrc::Asset { id: "f1".into(), kind: lore_docsrc::AssetKind::Figure, number: Some("1".into()), caption: Some("Overview of the gate.".into()), crop: Some("crops/fig1.png".into()), ..Default::default() });
        Ok(c)
    }
}

fn openalex_work() -> Value {
    json!({ "id": "https://openalex.org/W1", "display_name": "Gated Contrast for Synthetic Signals", "doi": format!("https://doi.org/{DOI}"), "publication_year": 2025,
        "open_access": { "is_oa": true }, "best_oa_location": { "is_oa": true, "pdf_url": "https://example.org/oa/gated.pdf", "license": "cc-by" } })
}

fn openalex_search() -> Value {
    json!({ "results": [{ "id": "https://openalex.org/W2", "display_name": "Noise-Contrastive Estimation for Synthetic Demos", "doi": format!("https://doi.org/{WEB_DOI}"), "publication_year": 2023,
        "authorships": [{ "author": { "display_name": "Daidai Example" } }], "cited_by_count": 12,
        "abstract_inverted_index": { "We": [0], "estimate": [1], "densities": [2], "by": [3], "contrast.": [4] },
        "open_access": { "is_oa": true }, "best_oa_location": { "is_oa": true, "pdf_url": "https://example.org/oa/nce.pdf", "license": "cc-by" } }] })
}

fn plan_reply() -> Value {
    json!({ "title": "Contrastive Learning from the Ground Up", "abstract": "Notes that build up to the gated InfoNCE loss.", "chapters": [
        { "title": "Similarity and temperature", "terms": ["cosine similarity", "temperature scaling"], "depth": "standard", "goal": "Compare two vectors.", "queries": ["contrastive estimation"],
          "figures": [{ "kind": "plot", "idea": "softmax for three temperatures" }] },
        { "title": "The InfoNCE loss", "terms": ["InfoNCE", "hard negatives", "Contrastive learning"], "depth": "deep", "queries": [],
          "figures": [{ "kind": "paper", "idea": "the gate", "paper_fig": "fig1" }] }
    ] })
}

fn chapter(n: usize, typed_ref: bool) -> Value {
    let paper_key = format!("doi:{DOI}");
    let web_key = format!("doi:{WEB_DOI}");
    match n {
        1 => json!({
            "body": format!("\\subsection{{Comparing vectors}}\nA \\term{{cosine similarity}}{{餘弦相似度}} compares two vectors \\cite{{{paper_key}}}.\\sidenote{{越大越像。}}\n\n\\begin{{intuition}}\nSmall temperature, sharp softmax.\n\\end{{intuition}}\n\nWith a temperature $\\tau$ the scores are divided before the softmax (Fig.~\\ref{{fig:f1}}) \\cite{{{web_key},made-up2020}}.{}\n%%FIGURE:f1\n\nTwo settings side by side (Table~\\ref{{tab:t1}}).\n%%TABLE:t1\n", if typed_ref { " As Section 2 shows, this matters." } else { "" }),
            "figures": [{ "id": "f1", "kind": "plot", "caption": "Softmax of three scores at three temperatures.", "plot": { "chart": "line", "x_label": "$\\tau$", "y_label": "largest probability", "x": [0.1, 0.5, 1.0], "series": [{ "name": "top score", "y": [0.99, 0.8, 0.6] }] } }],
            "tables": [{ "id": "t1", "caption": "Two temperatures compared.", "rows": [["tau", "sharpness"], ["0.1", "high"], ["1.0", "low"]] }]
        }),
        _ => json!({
            "body": format!("\\begin{{definition}}[InfoNCE]\nA loss over one positive and many negatives \\cite{{{paper_key}}}.\n\\end{{definition}}\n\nThe gate keeps \\term{{hard negatives}}{{困難負樣本}} (Fig.~\\ref{{fig:g}}), building on Section~\\ref{{sec:similarity-and-temperature}}.\n%%FIGURE:g\n\n\\begin{{keyidea}}\nPull positives together.\n\\end{{keyidea}}\n"),
            "figures": [{ "id": "g", "kind": "paper", "caption": "The gate over hard negatives.", "paper_fig": "fig1" }]
        }),
    }
}

struct Setup {
    root: PathBuf,
    deps: Arc<Deps>,
    model: Arc<MockModel>,
    http: Arc<MockHttp>,
}

fn setup(tag: &str, web: bool) -> Setup {
    let root = std::env::temp_dir().join(format!("pc-book-job-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let reader = lorereader(&root);
    let sp = kb(&root);
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("refs")).unwrap();
    let model = Arc::new(
        MockModel::default()
            .on(TASK_PLAN, plan_reply())
            .on(TASK_SUGGEST, json!({ "suggestions": [{ "op": "depth", "id": "c1", "depth": "brief", "reason": "你已經在學了" }, { "op": "remove", "id": "c9", "reason": "x" }] }))
            .on(TASK_CHAPTER, chapter(1, true))
            .on(TASK_CHAPTER, chapter(1, false))
            .on(TASK_CHAPTER, chapter(2, false)),
    );
    let http = Arc::new(
        MockHttp::new()
            .on("works/doi:", Response::ok(openalex_work().to_string()))
            .on("api.openalex.org/works?", Response::ok(openalex_search().to_string()))
            .on("example.org/oa/nce.pdf", Response::ok(b"%PDF-1.5\nsynthetic open-access paper\n%%EOF".to_vec())),
    );
    let mut cfg = lore_research::Config { pace: lore_research::Pace::none(), retries: 0, ..Default::default() };
    cfg.sources = vec![lore_research::Source::OpenAlex];
    let deps = Arc::new(Deps {
        cache: root.join("cache"),
        workspace: Some(ws),
        global_file: None,
        reader: Some(reader),
        reader_url: "http://127.0.0.1:7890".into(),
        kb_spaces: vec![sp],
        member: "ellrin".into(),
        filters: lore_words::Filters::default(),
        model: model.clone(),
        http: http.clone(),
        research: cfg,
        web,
        extract: Arc::new(FakeExtract),
        tools: Arc::new(tools::MockTools { compile: false, made: Default::default() }),
        plot_styles: json!([]),
        books_file: root.join("conf/books.toml"),
        apps: Some(Default::default()),
    });
    Setup { root, deps, model, http }
}

fn params(root: &Path) -> Value {
    serde_json::to_value(pipeline::Params {
        sha: SHA.into(),
        book: "new".into(),
        folder: Some(root.join("Books")),
        length: "standard".into(),
        lang: "en-zh".into(),
        page: "b5".into(),
        extras: source::Extras { terms: vec![source::ExtraTerm { term: "temperature scaling".into(), n: 3 }], stuck: vec!["p2".into()], questions: vec![] },
        ..Default::default()
    })
    .unwrap()
}

async fn until(job: &lore_jobs::Job, want: &str) {
    for _ in 0..400 {
        if job.status() == want {
            return;
        }
        if job.status() == "failed" {
            panic!("failed: {:?}", job.meta().error);
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("never {want}: {}", job.status());
}

#[tokio::test]
async fn a_book_from_a_paper_end_to_end() {
    let s = setup("e2e", true);
    // the workspace's style notes reach every chapter prompt
    let ws = s.deps.workspace.clone().unwrap();
    std::fs::create_dir_all(ws.join("style")).unwrap();
    std::fs::write(ws.join("style/10-writing.md"), "Define each abbreviation at first use.").unwrap();
    let js = lore_jobs::Jobs::open(s.deps.cache.join("jobs"), Arc::new(pipeline::BookJob { deps: s.deps.clone() }));
    let job = js.create(params(&s.root), false, "").unwrap();
    until(&job, "review").await;

    // terms: softmax is known (skipped); learning and unknown ones are taught; the card's red term counts
    let tr: pipeline::TermsOut = serde_json::from_value(job.output("terms").unwrap()).unwrap();
    let teach: Vec<&str> = tr.teach.iter().map(|t| t.term.as_str()).collect();
    assert!(!teach.iter().any(|t| t.eq_ignore_ascii_case("softmax")), "{teach:?}");
    assert!(teach.contains(&"InfoNCE") && teach.iter().any(|t| t.eq_ignore_ascii_case("cosine similarity")), "{teach:?}");
    assert!(tr.teach.iter().any(|t| t.term.eq_ignore_ascii_case("cosine similarity") && t.state == "learning"));
    assert!(tr.teach.iter().any(|t| t.term == "temperature scaling"), "the card's red term");
    assert_eq!(tr.prereq, [("InfoNCE".to_string(), "cosine similarity".to_string())]);
    let (_, plan_user) = s.model.asked(TASK_PLAN).remove(0);
    assert!(plan_user.contains("InfoNCE needs cosine similarity") && plan_user.contains("Why does the temperature scaling matter here?"), "{plan_user}");

    // the review window
    let v = js.view(&job);
    assert_eq!(v["state"], "review");
    let rv = &v["review"];
    assert_eq!(rv["chapters"].as_array().unwrap().len(), 2);
    assert!(rv["chapters"].as_array().unwrap().iter().all(|c| c["found"].is_array()), "追一層 per chapter");
    assert_eq!(rv["suggestions"].as_array().unwrap().len(), 1, "a suggestion on a row that is not there is dropped");
    assert_eq!(rv["license"], "cc-by");
    assert_eq!(v["pending_imports"], 1, "L-Reader not running: the downloaded paper waits");
    let r: research::Research = serde_json::from_value(job.output("research").unwrap()).unwrap();
    assert!(r.reuse_figures && r.sources.iter().any(|x| x.kind == "web") && r.sources.iter().any(|x| x.kind == "lr"), "{:?}", r.sources.iter().map(|x| &x.kind).collect::<Vec<_>>());
    assert!(r.cards.iter().any(|c| c.title == "InfoNCE"));
    assert!(s.http.urls().iter().any(|u| u.contains("/api/papers/import")), "the import was tried from the server");

    // the author keeps both chapters, the second one deeper and renamed
    js.submit(&job, "review", json!({ "chapters": [{ "id": "c1", "depth": "brief" }, { "id": "c2", "title": "The InfoNCE loss, step by step", "depth": "deep" }] })).unwrap();
    until(&job, "done").await;

    let w: write::WriteOut = serde_json::from_value(job.output("write").unwrap()).unwrap();
    let book = PathBuf::from(&w.book);
    assert!(w.new_book && book.starts_with(s.root.join("Books")));
    assert!(pc_core::meta::read_meta(&book).is_book());
    assert_eq!(pc_core::book::page_of(&book), "b5");
    let secs = pc_core::open::sections(&book);
    let titles: Vec<&str> = secs.iter().map(|x| x.title.as_str()).collect();
    assert_eq!(titles, ["Similarity and temperature", "The InfoNCE loss, step by step"], "the template's empty section gave way");
    let text: String = pc_core::paper::section_files(&book).iter().map(|f| std::fs::read_to_string(book.join("sections").join(f)).unwrap()).collect::<Vec<_>>().join("\n");
    // the typed reference was asked again once
    assert_eq!(s.model.asked(TASK_CHAPTER).len(), 3);
    assert!(s.model.asked(TASK_CHAPTER).iter().all(|(sys, _)| sys.contains("style rules: follow them") && sys.contains("Define each abbreviation at first use.")));
    assert!(!text.contains("Section 2") && !text.contains("made-up2020") && !text.contains("%%FIGURE"), "{text}");
    assert!(text.contains("\\term{cosine similarity}{餘弦相似度}") && text.contains("\\sidenote{"));
    // figures through the paper's path: files, labels, \ref pointed at them, the licence in the caption
    let figs: Vec<String> = std::fs::read_dir(book.join("figures")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert!(figs.iter().any(|f| f.ends_with(".plot.json")) && figs.iter().any(|f| f.ends_with(".pdf")) && figs.iter().any(|f| f.ends_with(".png")), "{figs:?}");
    for c in &w.chapters {
        for l in &c.figures {
            assert!(text.contains(&format!("\\label{{{l}}}")) && text.contains(&format!("\\ref{{{l}}}")), "{l}: {text}");
        }
    }
    assert!(text.contains("Reproduced from \\cite{") && text.contains(", CC BY."), "{text}");
    assert!(text.contains("\\begin{table}"), "the table through the paper's table path");
    // references: in the workspace library, the book's bib from what it cites
    let lib = std::fs::read_to_string(s.root.join("ws/refs/library.bib")).unwrap();
    assert!(lib.contains(DOI) && lib.contains(WEB_DOI), "{lib}");
    let main = std::fs::read_to_string(book.join("main.tex")).unwrap();
    assert!(main.contains("\\usepackage[b5]{lorenotes}") && main.contains("Notes that build up"), "{main}");
    // notation: the paper's symbol in the book's own registry; notation.tex is lore-notation's file
    let n = lore_notation::Notation::load(&pc_core::paper::app_dir(&book).join("notation.json")).unwrap();
    assert!(n.find_latex("\\tau").is_some());
    assert!(lore_notation::macros::is_generated(&std::fs::read_to_string(book.join("notation.tex")).unwrap()));
    // check: no registry, citation or figure problem
    let c: pipeline::CheckOut = serde_json::from_value(job.output("check").unwrap()).unwrap();
    let hard: Vec<&pipeline::Problem> = c.problems.iter().filter(|p| p.kind != "chapter").collect();
    assert!(hard.is_empty(), "{hard:?}");
    // books.toml lists the book with its paper
    let reg = crate::books::Registry::load(&s.deps.books_file);
    assert!(reg.books.iter().any(|b| b.path == book && b.papers == [SHA.to_string()]));
    let done = js.view(&job);
    assert_eq!(done["result"]["chapters"].as_array().unwrap().len(), 2);

    // XeLaTeX, when installed: the generated book compiles (bibtex for the [n] citations)
    if let Some(xe) = crate::texmeasure::tool("xelatex") {
        let run = |bin: &Path, args: &[&str]| std::process::Command::new(bin).args(args).current_dir(&book).output().unwrap();
        let first = run(&xe, &["-interaction=nonstopmode", "-halt-on-error", "main.tex"]);
        let log = std::fs::read_to_string(book.join("main.log")).unwrap_or_default();
        assert!(first.status.success(), "{}", &log[log.len().saturating_sub(3000)..]);
        if let Some(bt) = crate::texmeasure::tool("bibtex") {
            run(&bt, &["main"]);
            run(&xe, &["-interaction=nonstopmode", "main.tex"]);
            let out = run(&xe, &["-interaction=nonstopmode", "main.tex"]);
            let log = std::fs::read_to_string(book.join("main.log")).unwrap_or_default();
            assert!(out.status.success() && !log.contains("Citation `") && !log.contains("Reference `"), "{}", &log[log.len().saturating_sub(3000)..]);
        }
    }
    std::fs::remove_dir_all(&s.root).unwrap();
}

#[tokio::test]
async fn offline_and_direct_still_finishes_and_adds_to_a_book() {
    let s = setup("offline", false);
    let js = lore_jobs::Jobs::open(s.deps.cache.join("jobs"), Arc::new(pipeline::BookJob { deps: s.deps.clone() }));
    let job = js.create(params(&s.root), true, "").unwrap();
    until(&job, "done").await;
    assert!(s.model.asked(TASK_CHAPTER).iter().all(|(sys, _)| !sys.contains("style rules")), "no style folder: the prompt is unchanged");
    let r: research::Research = serde_json::from_value(job.output("research").unwrap()).unwrap();
    assert!(!r.reuse_figures && r.sources.iter().all(|x| x.kind != "web"), "no web: no licence known, no web papers");
    assert!(s.http.urls().is_empty(), "nothing asked of the network");
    let w: write::WriteOut = serde_json::from_value(job.output("write").unwrap()).unwrap();
    let book = PathBuf::from(&w.book);
    let text: String = pc_core::paper::section_files(&book).iter().map(|f| std::fs::read_to_string(book.join("sections").join(f)).unwrap()).collect();
    assert!(!text.contains(".png}"), "the paper's figure is not reproduced without a licence");
    // 加進這本書: the same paper again into this book → terms it already explains are not taught again
    let mut p = params(&s.root);
    p["book"] = "x".into();
    p["book_path"] = book.to_string_lossy().to_string().into();
    let again = js.create(p, true, "").unwrap();
    for _ in 0..400 {
        if again.output("terms").is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    js.stop(&again).await;
    let tr: pipeline::TermsOut = serde_json::from_value(again.output("terms").unwrap()).unwrap();
    assert!(!tr.teach.iter().any(|t| t.term.eq_ignore_ascii_case("cosine similarity") || t.term == "hard negatives"), "{:?}", tr.teach);
    std::fs::remove_dir_all(&s.root).unwrap();
}


/// A plan the model gave no abstract: the back of the title page still says what the notes are
/// (it once printed «About these notes» over nothing).
#[test]
fn the_title_pages_back_is_never_a_lone_heading() {
    let ch = |t: &str| plan::Chapter { title: t.into(), ..Default::default() };
    let mut plan = plan::Plan { title: "Notes".into(), abstract_text: String::new(), chapters: vec![ch("Learning Representations Without Labels"), ch("Contrastive Losses")] };
    let about = write::about_text(&plan, "A Simple Framework for 100% Contrastive Learning");
    assert!(about.starts_with("These notes explain") && about.contains("100\\% Contrastive") && about.contains(plan.chapters[0].title.trim()), "{about}");
    let dir = std::env::temp_dir().join(format!("pc-about-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\title{T}\n\\begin{document}\n\\maketitle\n\n\\begin{abstract}\n\\end{abstract}\n\n\\end{document}\n").unwrap();
    write::set_front(&dir, "abstract", &about);
    let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
    let body = main.split("\\begin{abstract}").nth(1).unwrap().split("\\end{abstract}").next().unwrap();
    assert!(body.contains("These notes explain"), "{main}");
    // the model's own abstract wins
    plan.abstract_text = "Notes that build up to the gated InfoNCE loss.".into();
    assert_eq!(write::about_text(&plan, "x"), "Notes that build up to the gated InfoNCE loss.");
    assert!(plan::schema()["required"].as_array().unwrap().iter().any(|r| r == "abstract"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reused_figures_always_cite_and_name_their_licence() {
    assert_eq!(write::reused_caption("The framework.", "chen2020", Some("cc-by-nc")), "The framework. Reproduced from \\cite{chen2020}, CC BY-NC.");
    assert_eq!(write::reused_caption("", "k", Some("https://creativecommons.org/licenses/by-nc-nd/4.0/")), "Reproduced from \\cite{k}, CC BY-NC-ND 4.0.");
    // NonCommercial copies may be reused in the author's own notes; unknown ones never
    assert!(lore_research::figure_reuse_allowed(Some("cc-by-nc-sa")) && !lore_research::figure_reuse_allowed(Some("arxiv-nonexclusive")) && !lore_research::figure_reuse_allowed(None));
}

/// Files: a PDF is a paper when its text says so (title and references); every call counted.
struct FileExtract(std::sync::atomic::AtomicUsize);

impl source::Extract for FileExtract {
    fn extract(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        self.text(pdf, out)
    }
    fn text(&self, pdf: &Path, _out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = std::fs::read_to_string(pdf).map_err(|e| e.to_string())?;
        let mut c = lore_docsrc::AssetCatalog::default();
        if let Some(title) = body.lines().find_map(|l| l.strip_prefix("TITLE ")) {
            c.meta.title = title.to_string();
            c.meta.abstract_text = Some(format!("{title}: we study the InfoNCE loss with temperature scaling."));
            c.references.push(lore_docsrc::Reference::default());
        }
        c.sections = vec![lore_docsrc::Section { id: "s1".into(), level: 1, title: "Introduction".into(), paragraphs: vec![body.clone()], ..Default::default() }];
        Ok(c)
    }
}

fn files_deps(root: &Path, extract: Arc<FileExtract>, http: Arc<MockHttp>, model: Arc<MockModel>, apps: &[(&str, &str)]) -> Arc<Deps> {
    Arc::new(Deps {
        cache: root.join("cache"),
        workspace: None,
        global_file: None,
        reader: None,
        reader_url: "http://127.0.0.1:9".into(),
        kb_spaces: vec![kb(root)],
        member: "ellrin".into(),
        filters: lore_words::Filters::default(),
        model,
        http,
        research: lore_research::Config { pace: lore_research::Pace::none(), retries: 0, ..Default::default() },
        web: false,
        extract,
        tools: Arc::new(tools::MockTools { compile: false, made: Default::default() }),
        plot_styles: json!([]),
        books_file: root.join("conf/books.toml"),
        apps: Some(apps.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
    })
}

/// A folder of PDFs and a typed keyword, no L-Reader paper: each file read once (a stopped job
/// resumes without reading them again), what is not a paper is never read into a prompt (L-Shelf's
/// word first, the paper rule without it), no path reaches a prompt, the source checkpoint, the
/// book or books.toml; several papers and none both plan and write.
#[tokio::test]
async fn files_folders_and_keywords_start_a_book() {
    let root = std::env::temp_dir().join(format!("pc-book-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let papers = root.join("Shelf/papers");
    std::fs::create_dir_all(papers.join("sub")).unwrap();
    std::fs::create_dir_all(papers.join(".hidden")).unwrap();
    std::fs::write(papers.join("gated.pdf"), "TITLE Gated Contrast for Synthetic Signals\nThe InfoNCE loss uses cosine similarity and hard negatives.").unwrap();
    std::fs::write(papers.join("sub/primer.pdf"), "TITLE A Primer on Synthetic Similarity\nHard negatives and cosine similarity again.").unwrap();
    std::fs::write(papers.join("receipt.pdf"), "Receipt for a synthetic purchase. Contrastive learning gift card.").unwrap();
    std::fs::write(papers.join("private.pdf"), "TITLE Private Synthetic Notes\nnever to be read").unwrap();
    std::fs::write(papers.join(".hidden/x.pdf"), "TITLE Hidden").unwrap();
    std::fs::write(papers.join("notes.txt"), "not a pdf").unwrap();
    let private_sha = source::file_sha(&papers.join("private.pdf")).unwrap();
    let extract = Arc::new(FileExtract(Default::default()));
    // L-Shelf runs and knows private.pdf: not for a model (cloud_ok false)
    let http = Arc::new(MockHttp::new().on(&format!("shelf.test/api/files/{private_sha}"), Response::ok(json!({ "id": private_sha, "cloud_ok": false, "paths": ["/x"] }).to_string())));
    let model = Arc::new(MockModel::default().on(TASK_PLAN, plan_reply()).on(TASK_SUGGEST, json!({ "suggestions": [] })).on(TASK_CHAPTER, chapter(1, false)).on(TASK_CHAPTER, chapter(2, false)));
    let deps = files_deps(&root, extract.clone(), http.clone(), model.clone(), &[("ls", "http://shelf.test")]);
    let js = lore_jobs::Jobs::open(deps.cache.join("jobs"), Arc::new(pipeline::BookJob { deps: deps.clone() }));
    let mut st = setup::Setup { sources: vec![setup::Start { kind: "folder".into(), path: Some(papers.clone()), ..Default::default() }], keywords: vec!["temperature scaling".into()], folder: Some(root.join("Books")), ..Default::default() };
    st.tidy();
    let p = pipeline::Params::from_setup(&st, None, Some(root.join("Books")));
    let job = js.create(serde_json::to_value(&p).unwrap(), false, "").unwrap();
    until(&job, "review").await;

    let src = source::Sources::from_value(job.output("source").unwrap()).unwrap();
    let titles: Vec<&str> = src.materials.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["Gated Contrast for Synthetic Signals", "A Primer on Synthetic Similarity"], "sorted, subfolders read, hidden ones not");
    assert_eq!((src.files, src.of, src.skipped), (4, 4, 2));
    assert!(src.notes.iter().any(|n| n.contains("2 份檔案不是論文")), "{:?}", src.notes);
    assert_eq!(extract.0.load(std::sync::atomic::Ordering::SeqCst), 3, "the file L-Shelf keeps local is never opened");
    let raw = std::fs::read_to_string(job.stage_file("source")).unwrap();
    assert!(!raw.contains(&root.to_string_lossy().to_string()), "no path in the source checkpoint");
    let v = js.view(&job);
    assert_eq!(v["stages"][0]["counts"]["files"], 4);
    assert_eq!(v["options"]["sources"], json!([{ "kind": "folder", "label": "papers" }]));
    // terms: the papers' and the typed keyword; the plan sees both papers, never a path, never the receipt
    let tr: pipeline::TermsOut = serde_json::from_value(job.output("terms").unwrap()).unwrap();
    assert_eq!(tr.teach[0].term, "temperature scaling", "the typed keyword first");
    assert!(tr.teach.iter().any(|t| t.term == "InfoNCE"), "{:?}", tr.teach);
    let (sys, user) = s_first(&model, TASK_PLAN);
    assert!(user.contains("Paper 1: Gated Contrast") && user.contains("Paper 2: A Primer") && sys.contains("not the subject"), "{user}");
    assert!(!user.contains("Receipt") && !user.contains("never to be read") && !user.contains(&*root.to_string_lossy()), "{user}");

    // stopped and redone from the source: no file is read again
    js.redo_from(&job, "source").unwrap();
    until(&job, "review").await;
    assert_eq!(extract.0.load(std::sync::atomic::Ordering::SeqCst), 3, "the per-file checkpoint");
    js.submit(&job, "review", json!({ "skipped": true })).unwrap();
    until(&job, "done").await;
    let w: write::WriteOut = serde_json::from_value(job.output("write").unwrap()).unwrap();
    let book = PathBuf::from(&w.book);
    let r: research::Research = serde_json::from_value(job.output("research").unwrap()).unwrap();
    let papers_in: Vec<_> = r.sources.iter().filter(|s| s.kind == "paper").collect();
    assert_eq!(papers_in.len(), 2, "each starting paper is one reference");
    assert!(papers_in.iter().all(|s| !s.chapters.is_empty()), "several papers go to the chapters that use them");
    for (_, u) in model.asked(TASK_CHAPTER) {
        assert!(!u.contains(&*root.to_string_lossy()) && !u.contains("never to be read"), "{u}");
    }
    let all: String = pc_core::open::source_files(&book).iter().chain([book.join("refs.bib")].iter()).filter_map(|f| std::fs::read_to_string(f).ok()).collect();
    assert!(!all.contains(&*papers.to_string_lossy()), "no path in the book");
    let reg = std::fs::read_to_string(&deps.books_file).unwrap();
    assert!(!reg.contains(&*papers.to_string_lossy()) && reg.contains("papers = []"), "{reg}");
    std::fs::remove_dir_all(&root).unwrap();
}

/// Only keywords: no paper at all, the plan still runs (terms only) and the book gets a title.
#[tokio::test]
async fn a_keyword_only_book_plans_without_a_paper() {
    let root = std::env::temp_dir().join(format!("pc-book-kw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let model = Arc::new(MockModel::default().on(TASK_PLAN, json!({ "title": "", "abstract": "", "chapters": [{ "title": "Temperature", "terms": ["temperature scaling"], "depth": "brief" }] })).on(TASK_SUGGEST, json!({ "suggestions": [] })).on(TASK_CHAPTER, json!({ "body": "A \\term{temperature scaling}{溫度縮放} divides scores." })));
    let deps = files_deps(&root, Arc::new(FileExtract(Default::default())), Arc::new(MockHttp::new()), model.clone(), &[]);
    let js = lore_jobs::Jobs::open(deps.cache.join("jobs"), Arc::new(pipeline::BookJob { deps: deps.clone() }));
    let st = setup::Setup { keywords: vec!["temperature scaling".into()], folder: Some(root.join("Books")), make_cards: false, ..Default::default() };
    let job = js.create(serde_json::to_value(pipeline::Params::from_setup(&st, None, Some(root.join("Books")))).unwrap(), true, "").unwrap();
    until(&job, "done").await;
    let (sys, user) = s_first(&model, TASK_PLAN);
    assert!(sys.contains("wants to learn") && !user.contains("Paper"), "{user}");
    let w: write::WriteOut = serde_json::from_value(job.output("write").unwrap()).unwrap();
    assert_eq!(w.title, "Notes on temperature scaling");
    std::fs::remove_dir_all(&root).unwrap();
}

fn s_first(m: &MockModel, task: &str) -> (String, String) {
    m.asked(task).into_iter().next().expect("asked")
}

/// A job made before several starting points: its `sha` is one L-Reader paper, its source
/// checkpoint one material.
#[test]
fn an_older_job_reads_as_one_paper() {
    let p: pipeline::Params = serde_json::from_value(json!({ "sha": SHA, "book": "new", "length": "standard", "lang": "en-zh" })).unwrap();
    assert_eq!(p.starts().len(), 1);
    assert_eq!((p.starts()[0].kind.as_str(), p.format(), p.make_cards), ("lr", "notes", false));
    let s = source::Sources::from_value(json!({ "sha": SHA, "title": "Gated Contrast for Synthetic Signals", "figures": [{ "id": "fig1", "file": "fig1.png" }] })).unwrap();
    assert_eq!((s.materials.len(), s.materials[0].kind.as_str(), s.shas().len()), (1, "lr", 1));
    assert!(s.figure("fig1").is_some());
}

/// 追一層: the found papers' own terms, against the knowledge base, under the chapter the paper
/// was found for, never what the book already teaches or the author knows.
#[test]
fn found_papers_offer_one_more_layer_of_terms() {
    let root = std::env::temp_dir().join(format!("pc-book-found-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let names = lore_learn::load_names(&[kb(&root)], "ellrin");
    let web = |title: &str, abs: &str, ch: &str| research::Src {
        kind: "web".into(),
        chapters: vec![ch.into()],
        record: lore_research::Record { title: title.into(), abstract_text: Some(abs.into()), ..Default::default() },
        ..Default::default()
    };
    let r = research::Research {
        sources: vec![
            research::Src { kind: "paper".into(), record: lore_research::Record { title: "Start".into(), abstract_text: Some("Momentum Contrast (MoCo) and more MoCo.".into()), ..Default::default() }, ..Default::default() },
            web("Noise-Contrastive Estimation for Synthetic Demos", "Noise Contrastive Estimation (NCE) fits densities. NCE compares data with noise; the softmax and InfoNCE are related to NCE.", "c2"),
        ],
        ..Default::default()
    };
    let f = research::found_terms(&r, &names, &lore_words::Filters::default(), &["InfoNCE".to_string()], 8);
    let c2 = f.get("c2").expect("under the chapter it was found for");
    assert!(c2.iter().any(|x| x.term.contains("NCE") || x.term.contains("Noise Contrastive")), "{c2:?}");
    assert!(c2.iter().all(|x| x.from == "Noise-Contrastive Estimation for Synthetic Demos"));
    assert!(!c2.iter().any(|x| x.term == "InfoNCE" || x.term.eq_ignore_ascii_case("softmax")), "taught or known: {c2:?}");
    assert!(!f.values().flatten().any(|x| x.term.contains("MoCo")), "a starting paper is not a found one");
    std::fs::remove_dir_all(&root).unwrap();
}
