//! 流程圖 in a paper and in a book over HTTP (docs/flow-figures.md §6), offline: a synthetic
//! two-stage pipeline, PDFs made without a browser, no model. Own process (environment set
//! before the server starts).

use std::sync::Arc;

use serde_json::{Value, json};

const DSL: &str = "fig seg2\npanel a \"a. Coarse localisation\"\n  x: image \"$\\mathbf{x}$\"\n  det: encoder \"Localiser\"\n  roi: box \"ROI crop\"\n  x -> det -> roi\nend\npanel b \"b. Fine segmentation\"\n  crop: image \"@xr\"\n  seg: unet \"Segmenter\"\n  yhat: image \"$\\hat{\\mathbf{y}}$\" look=mask\n  x -> crop\n  crop -> seg -> yhat\nend\nvar xr = \"\\mathbf{x}_{r}\" \"cropped region\"\n";

/// What the page sends: the figure at print size (LoreFlow.paperSvg).
fn svg() -> String {
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 200\" width=\"141.11mm\" height=\"70.56mm\"><rect width=\"400\" height=\"200\" fill=\"#fff\"/></svg>".into()
}

async fn round_trip(http: &reqwest::Client, url: &dyn Fn(&str) -> String, name: &str, dir: &std::path::Path) {
    let figs = dir.join("figures");
    std::fs::create_dir_all(&figs).unwrap();
    // the author's own PDF of the same name is never touched
    std::fs::write(figs.join("seg2.pdf"), b"%PDF mine").unwrap();
    let spec = json!({ "lore_flow": 1, "dsl": DSL, "layout": "small-big", "discussion": { "answers": { "focus": "process" } } });
    // a figure without a print size is refused
    let r = http.post(url(&format!("/api/papers/{name}/flow"))).json(&json!({ "spec": spec, "svg": "<svg/>" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = http.post(url(&format!("/api/papers/{name}/flow"))).json(&json!({ "stem": "seg2", "spec": spec, "svg": svg() })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "{}", r.text().await.unwrap());
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["file"], "figures/seg2-2.pdf");
    assert_eq!(v["registered"], json!(["xr"]));
    // a first caption for the figure block (never an empty \caption{}), from the panel titles
    assert_eq!(v["caption"], "(a) Coarse localisation, (b) Fine segmentation.", "periods and commas only");
    assert_eq!(std::fs::read(figs.join("seg2.pdf")).unwrap(), b"%PDF mine");
    assert!(std::fs::read(figs.join("seg2-2.pdf")).unwrap().starts_with(b"%PDF-"));
    // the new variable is in the document's own registry, not the global one
    let own = std::fs::read_to_string(pc_core::paper::app_dir(dir).join("notation.json")).unwrap();
    assert!(own.contains("\"xr\"") && own.contains("flow-draft"), "{own}");
    // the spec next to the PDF re-opens the editor; the author's own PDF does not
    let v: Value = http.get(url(&format!("/api/papers/{name}/flow?file=figures/seg2-2.pdf"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["spec"]["layout"], "small-big");
    assert_eq!(http.get(url(&format!("/api/papers/{name}/flow?file=figures/seg2.pdf"))).send().await.unwrap().status().as_u16(), 404);
    // re-edit: the same file; replacing the author's PDF is refused by giving a new name
    let mut spec2 = spec.clone();
    spec2["dirs"] = json!({ "b": "down" });
    let v: Value = http.post(url(&format!("/api/papers/{name}/flow"))).json(&json!({ "replace": "figures/seg2-2.pdf", "spec": spec2, "svg": svg() })).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["file"], "figures/seg2-2.pdf");
    assert!(std::fs::read_to_string(figs.join("seg2-2.flow.json")).unwrap().contains("\"down\""));
    let v: Value = http.post(url(&format!("/api/papers/{name}/flow"))).json(&json!({ "replace": "figures/seg2.pdf", "spec": spec2, "svg": svg() })).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["file"], "figures/seg2-3.pdf");
    assert_eq!(std::fs::read(figs.join("seg2.pdf")).unwrap(), b"%PDF mine");
    // the block: the same path as 新增圖… — a registered \label, \ref by the page, no typed number
    let v: Value = http.post(url(&format!("/api/papers/{name}/figure-block"))).json(&json!({ "path": "figures/seg2-2.pdf", "caption": "The two-stage pipeline.", "stem": "seg2-2" })).send().await.unwrap().json().await.unwrap();
    let block = v["block"].as_str().unwrap();
    let label = v["label"].as_str().unwrap();
    assert!(block.contains("\\includegraphics[width=\\linewidth]{figures/seg2-2.pdf}") && block.contains(&format!("\\label{{{label}}}")), "{block}");
    // lore-notation's checks on the block with a \ref to it: nothing blocking, no typed reference
    let text = format!("{block}\n\nThe pipeline in Fig.~\\ref{{{label}}} crops $\\mathbf{{x}}_{{r}}$ first.\n");
    let doc = lore_notation::Document::from_files(&[("main.tex".to_string(), text)]);
    let n = lore_notation::Notation::load(&pc_core::paper::app_dir(dir).join("notation.json")).unwrap();
    let rep = lore_notation::check_document(doc, &n, &Default::default());
    let codes: Vec<String> = serde_json::to_value(&rep).unwrap()["issues"].as_array().map(|a| a.iter().filter_map(|i| i["code"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    assert!(!codes.iter().any(|c| c == "hardcoded_reference" || c == "undefined_ref"), "{codes:?}");
    // the starting point: title, abstract and the method section
    let v: Value = http.get(url(&format!("/api/papers/{name}/flow/source"))).send().await.unwrap().json().await.unwrap();
    let src = v["text"].as_str().unwrap();
    assert!(src.contains("coarse-to-fine") , "{src}");
}

#[tokio::test]
async fn flow_figures_in_a_paper_and_in_a_book() {
    let root = std::env::temp_dir().join(format!("pc-flow-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "ws/papers/seg", "home", "private", "books"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("ws"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("PAPERCOMPOSER_BOOKS", root.join("home/books.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    pc_server::flow::use_pdf_maker(Arc::new(|svg: &str| {
        assert!(svg.contains("mm\""));
        Ok(b"%PDF-1.4 synthetic".to_vec())
    }));
    let paper = root.join("ws/papers/seg");
    std::fs::write(paper.join("main.tex"), "\\documentclass{article}\n\\title{Two-stage segmentation}\n\\begin{document}\n\\maketitle\n\\begin{abstract}A synthetic coarse-to-fine pipeline.\\end{abstract}\n\\section{Introduction}\nWhy.\n\\section{Method}\nThe localiser crops a region; the segmenter labels it.\n\\end{document}\n").unwrap();
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = move |p: &str| format!("http://{addr}{p}");

    // the shared editor is mounted, with this app's tasks
    let r = http.get(url("/flow/")).send().await.unwrap();
    assert!(r.text().await.unwrap().contains("LoreFlow.edit"));
    let v: Value = http.get(url("/flow/api/catalogue")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["steps"][0]["id"], "start");
    assert!(http.get(url("/flow/mathsvg.js")).send().await.unwrap().text().await.unwrap().contains("LoreMathSvg"));

    round_trip(&http, &url, "seg", &paper).await;

    // a book is the same document: the same path
    let r = http.post(url("/api/books")).json(&json!({ "title": "Segmentation notes", "folder": root.join("books") })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "{}", r.text().await.unwrap());
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let book = std::path::PathBuf::from(v["path"].as_str().unwrap());
    assert!(pc_core::meta::read_meta(&book).is_book());
    // a method chapter to start from
    let main = std::fs::read_to_string(pc_core::open::main_file(&book)).unwrap();
    let main = main.replace("\\end{document}", "\\chapter{Method}\nA coarse-to-fine pipeline: localise, crop, segment.\n\\end{document}");
    std::fs::write(pc_core::open::main_file(&book), main).unwrap();
    round_trip(&http, &url, &id, &book).await;
    let _ = std::fs::remove_dir_all(&root);
}
