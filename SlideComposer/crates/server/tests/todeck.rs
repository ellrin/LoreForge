//! 論文報告 on a synthetic PaperComposer paper (LaTeX folder, a drawn PNG figure, a table,
//! an equation with a registered macro, a bibliography), with mock engines. Offline: the
//! equation becomes OMML only when pandoc is installed.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use lore_llm::MockProvider;
use lore_llm::router::MockFactory;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A plain RGB PNG (stored deflate blocks): white with three dark boxes side by side.
fn png(w: u32, h: u32) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend(data);
        out.extend(&c);
        out.extend(crc(&c).to_be_bytes());
    }
    let mut raw = vec![];
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let inside = y > h / 4 && y < h * 3 / 4 && (x % (w / 3)) > w / 24 && (x % (w / 3)) < w / 3 - w / 24;
            raw.extend(if inside { [40, 60, 120] } else { [255, 255, 255] });
        }
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

const MAIN: &str = r"\documentclass{article}
\input{notation}
\title{Two-Stage Grading of Synthetic Lesions}
\author{Ellrin \and Daidai}
\begin{document}
\maketitle
\begin{abstract}
We grade synthetic lesions in two stages: an encoder finds them, a fusion step weighs them and a decoder grades the image.
\end{abstract}
\section{Introduction}
Grading synthetic lesions by hand is slow \cite{ellrin2025synthetic}.
A Lesion Grading Network (LGN) needs experts; the LGN is slow.
\section{Method}
The pipeline in Fig.~\ref{fig:arch} has three modules: encoder, fusion and decoder.
\begin{figure}[t]
\centering
\includegraphics[width=\linewidth]{figures/arch.png}
\caption{Overview of the pipeline: encoder, fusion and decoder.}
\label{fig:arch}
\end{figure}
The reconstruction loss is
\begin{equation}
\mathcal{L} = \lVert \vx - \hat{\vx} \rVert^2
\label{eq:loss}
\end{equation}
We train for 30 epochs with AdamW at a learning rate of $10^{-4}$.
\section{Results}
\begin{table}[t]
\caption{Main results on the synthetic set.}
\label{tab:main}
\begin{tabular}{lcc}
Method & AUC & F1 \\
\hline
Ours & 0.91 & 0.84 \\
Baseline & 0.85 & 0.79 \\
\end{tabular}
\end{table}
Ours is better in both metrics.
\bibliographystyle{plain}
\bibliography{refs}
\end{document}
";

const BIB: &str = "@article{ellrin2025synthetic,\n  author = {Ellrin, A. and Daidai, B.},\n  title = {Synthetic Lesions for Testing},\n  journal = {Journal of Synthetic Tests},\n  year = {2025}\n}\n";

const NOTATION: &str = r#"{"version":1,"entries":[{"id":"x","kind":"symbol","latex":"\\mathbf{x}","macro":"vx","meaning":"input image"}]}"#;

fn paper(root: &Path) -> std::path::PathBuf {
    let p = root.join("papers/two-stage");
    std::fs::create_dir_all(p.join("figures")).unwrap();
    std::fs::write(p.join("main.tex"), MAIN).unwrap();
    std::fs::write(p.join("refs.bib"), BIB).unwrap();
    std::fs::write(p.join("notation.json"), NOTATION).unwrap();
    std::fs::write(p.join("notation.tex"), "\\newcommand{\\vx}{\\mathbf{x}}\n").unwrap();
    std::fs::write(p.join("figures/arch.png"), png(300, 100)).unwrap();
    p
}

fn snapshot(dir: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn walk(p: &Path, v: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            let q = e.path();
            if q.is_dir() { walk(&q, v) } else { v.push((q.clone(), std::fs::read(&q).unwrap())) }
        }
    }
    let mut v = vec![];
    walk(dir, &mut v);
    v.sort();
    v
}

fn plan(lang_text: &str) -> Value {
    json!({
        "venue": "Journal of Synthetic Tests, 2025",
        "keywords": ["lesion grading", "synthetic data", "two stages"],
        "slides": [
            { "role": "cover", "title": "Two-Stage Grading of Synthetic Lesions", "bullets": [] },
            { "role": "background", "title": "Why grading is slow", "bullets": [format!("Grade by hand {lang_text}")], "cites": ["ellrin2025synthetic", "nobody2020"] },
            { "role": "overview", "title": "Big picture", "bullets": [], "figure": "fig1", "steps": ["Encode the image", "Fuse the lesions", "Decode the grade"] },
            { "role": "method", "title": "Encoder", "bullets": ["Finds lesions as in Fig. 3"], "figure": "fig1", "module": 1 },
            { "role": "method", "title": "Fusion", "bullets": ["Weighs each lesion by $\\vx$"], "figure": "fig1", "module": 2 },
            { "role": "method", "title": "Decoder", "bullets": ["Grades the image"], "figure": "fig1", "module": 3, "equations": ["eq1", "eq99"] },
            { "role": "loss", "title": "Reconstruction loss", "bullets": ["Penalises the distance (Eq. (1))"], "equations": ["eq1"] },
            { "role": "training", "title": "Training details", "bullets": [], "kv": [["Epochs", "30"], ["Optimiser", "AdamW"], ["Learning rate", "$10^{-4}$"]] },
            { "role": "results", "title": "Main results", "bullets": [], "table": "tab1", "takeaway": "Ours wins on both metrics, see Table 1" },
            { "role": "opinion", "title": "Some opinions", "bullets": ["Test on real data"] },
            { "role": "takeaway", "title": "Take home messages", "bullets": ["Two stages help", "Fusion matters"] },
            { "role": "thanks", "title": "Thanks", "bullets": [] }
        ]
    })
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>, origin: Option<&str>) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(o) = origin {
        req = req.header("origin", o);
    }
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&b).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null), headers)
}

/// A workspace with the public blank base and the public arrangements.
fn workspace(root: &Path) -> sc_core::Workspace {
    let example = sc_server::repo_dir().join("../private-example/SlideComposer");
    lore_home::copy_missing(&example, &root.join("sc")).unwrap();
    sc_core::Workspace::new(root.join("sc"), None)
}

fn texts(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(t) = m.get("text").and_then(Value::as_str) {
                out.push(t.to_string());
            }
            m.values().for_each(|x| texts(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| texts(x, out)),
        _ => {}
    }
}

#[tokio::test]
async fn paper_becomes_a_report_deck() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let before = snapshot(&p);
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let planner = Arc::new(MockProvider::new(vec![plan("is slow").to_string()]));
    // the vision engine for the module boxes: three boxes in reading order (pixels of the 300×100 figure)
    let boxes = json!({ "modules": [
        { "label": "encoder", "box": [12, 25, 88, 75] },
        { "label": "fusion", "box": [112, 25, 188, 75] },
        { "label": "decoder", "box": [212, 25, 288, 75] } ] });
    let vision = Arc::new(MockProvider::new(vec![boxes.to_string()]).named("mock-vision"));
    let f = MockFactory::default().with("claude:selected", planner.clone()).with("claude:sonnet", vision.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());

    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "short", "lang": "en", "style": "blank" });
    let (st, v, h) = call(&app, "POST", "/api/todeck", Some(body.clone()), Some("http://127.0.0.1:7878")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(h.get("access-control-allow-origin").and_then(|x| x.to_str().ok()), Some("http://127.0.0.1:7878"));
    let name = v["name"].as_str().unwrap().to_string();
    assert_eq!(name, "Two-Stage-Grading-of-Synthetic-Lesions");
    assert_eq!(v["boxes"], 3, "{v}");
    assert_eq!(v["notation"], 1, "the paper's symbol joins the deck's notation");

    // the model saw the paper's material by id
    let seen = planner.seen.lock().unwrap()[0].flatten();
    assert!(seen.contains("Two-Stage Grading") && seen.contains("fig1: Overview of the pipeline") && seen.contains("tab1") && seen.contains("eq1"), "{seen}");
    assert!(seen.contains("ellrin2025synthetic"));
    assert_eq!(vision.seen.lock().unwrap().len(), 1, "one vision call for the main figure");

    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    let slides = d["deck"]["slides"].as_array().unwrap().clone();
    // 12 planned, the big picture expanded into one slide per step (3)
    assert_eq!(slides.len(), 14);
    assert_eq!(slides[0]["layout"], "封面");
    assert_eq!(slides.last().unwrap()["layout"], "結尾");
    let mut all = vec![];
    texts(&d["deck"], &mut all);
    let joined = all.join("\n");
    assert!(joined.contains("Two-Stage Grading of Synthetic Lesions") && joined.contains("Keywords: lesion grading"));
    assert!(joined.contains("Journal of Synthetic Tests, 2025"));
    assert!(joined.contains("Ref: Ellrin and Daidai, Synthetic Lesions for Testing"), "cited key → Ref: line");
    assert!(!joined.contains("Fig. 3") && !joined.contains("Table 1") && !joined.contains("Eq. (1)"), "typed references are dropped");

    // big picture: three slides, the current step plain, the others ghosted
    let steps: Vec<&Value> = slides.iter().filter(|s| s["objects"].as_array().unwrap().iter().any(|o| o["name"] == "step-1")).collect();
    assert_eq!(steps.len(), 3);
    let grey = |s: &Value, n: &str| s["objects"].as_array().unwrap().iter().find(|o| o["name"] == n).unwrap()["text"]["paras"][0]["runs"][1]["look"]["color"].clone();
    assert_ne!(grey(steps[0], "step-1"), json!("A6A6A6"));
    assert_eq!(grey(steps[0], "step-2"), json!("A6A6A6"));
    assert_eq!(grey(steps[1], "step-1"), json!("A6A6A6"));
    assert_ne!(grey(steps[1], "step-2"), json!("A6A6A6"));

    // method slides: the whole figure dimmed, the module cropped back on top
    let fusion = slides.iter().find(|s| all_text(s).contains("Fusion")).unwrap();
    let pics: Vec<&Value> = fusion["objects"].as_array().unwrap().iter().filter(|o| o["type"] == "picture").collect();
    assert_eq!(pics.len(), 2, "{fusion}");
    assert!(pics[0]["alpha"].as_f64().unwrap() < 0.5);
    assert!((pics[1]["crop"]["l"].as_f64().unwrap() - 112.0 / 300.0).abs() < 1e-6);
    assert_eq!(pics[0]["asset"], pics[1]["asset"], "one image file, two crops");

    // native tables: the results grid and the training key-value rows
    let tables: Vec<Value> = slides.iter().flat_map(|s| s["objects"].as_array().unwrap().clone()).filter(|o| o["type"] == "table").collect();
    assert_eq!(tables.len(), 2);
    let tt: Vec<String> = tables.iter().map(|t| { let mut v = vec![]; texts(t, &mut v); v.join("|") }).collect();
    assert!(tt.iter().any(|t| t.contains("Ours") && t.contains("0.91")));
    assert!(tt.iter().any(|t| t.contains("Optimiser") && t.contains("AdamW")));
    // every slide laid out with an author arrangement or the base's area; content slides name theirs
    assert!(slides.iter().any(|s| s["arrangement"] == "03-image-text"));

    // equations: native OMML when pandoc is here; the registry macro is expanded for display
    let pandoc = lore_math::Pandoc::default().available();
    let maths: Vec<Value> = {
        fn runs(v: &Value, out: &mut Vec<Value>) {
            match v {
                Value::Object(m) => {
                    if m.get("kind").and_then(Value::as_str) == Some("math") {
                        out.push(v.clone());
                    }
                    m.values().for_each(|x| runs(x, out));
                }
                Value::Array(a) => a.iter().for_each(|x| runs(x, out)),
                _ => {}
            }
        }
        let mut o = vec![];
        runs(&d["deck"], &mut o);
        o
    };
    if pandoc {
        assert!(maths.iter().any(|m| m["latex"].as_str().unwrap().contains("\\mathbf{x}") && m["omml"].as_str().unwrap().contains("m:oMath")), "{maths:?}");
        assert!(!maths.iter().any(|m| m["latex"].as_str().unwrap().contains("\\vx")));
    }

    // lore-notation on the generated slides: the paper's id is registered, no typed references,
    // no symbol the registry does not know
    let (_, n, _) = call(&app, "GET", &format!("/api/decks/{name}/notation"), None, None).await;
    assert!(n["notation"]["entries"].as_array().unwrap().iter().any(|e| e["id"] == "x" && e["origin"] == "document"));
    let codes: Vec<String> = n["report"]["issues"].as_array().map(|a| a.iter().filter_map(|i| i["code"].as_str().map(String::from)).collect()).unwrap_or_default();
    assert!(!codes.iter().any(|c| c == "hardcoded_reference"), "{codes:?}");
    // the paper's registered symbol is explained under the equation that uses it
    let undefined_x = n["report"]["issues"].as_array().unwrap().iter().any(|i| i["code"] == "undefined_symbol" && i["entry"] == "x");
    assert!(!undefined_x, "{}", n["report"]["issues"]);
    assert!(all_text(&d["deck"]).contains("denotes input image"));

    // the paper is untouched; the deck exports
    assert_eq!(snapshot(&p), before);
    let res = app.clone().oneshot(Request::builder().uri(format!("/api/decks/{name}/pptx")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert!(lore_ooxml::Deck::from_bytes(&bytes).is_ok());

    // again: a second deck, never over the first
    let (_, list, _) = call(&app, "GET", "/api/info", None, Some("http://localhost:7878")).await;
    assert!(list["decks"].as_array().unwrap().iter().any(|d| d["name"] == name.as_str()));
}

fn all_text(s: &Value) -> String {
    let mut v = vec![];
    texts(s, &mut v);
    v.join("\n")
}

#[tokio::test]
async fn no_engine_is_one_line_with_the_alternative_and_no_deck() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, MockFactory::default())).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let (st, v, _) = call(&app, "POST", "/api/todeck", Some(json!({ "source": { "kind": "pc", "path": p }, "style": "blank" })), None).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert!(!v["error"].as_str().unwrap().contains('\n'));
    assert!(shared.ws.deck_names().is_empty());
    // not a paper
    let (st, _, _) = call(&app, "POST", "/api/todeck", Some(json!({ "source": { "kind": "pc", "path": dir.path().join("home") }, "style": "blank" })), None).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn other_origins_get_no_cors() {
    let dir = tempfile::tempdir().unwrap();
    let ws = workspace(dir.path());
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), lore_home::Home::at(dir.path().join("home")));
    let app = sc_server::router_isolated(shared);
    let (_, _, h) = call(&app, "GET", "/api/info", None, Some("https://example.com")).await;
    assert!(h.get("access-control-allow-origin").is_none());
    let (_, _, h) = call(&app, "GET", "/api/decks/x/notation", None, Some("http://127.0.0.1:7878")).await;
    assert!(h.get("access-control-allow-origin").is_none(), "only the 論文報告 routes and /api/info");
    assert!(sc_server::local_origin("http://127.0.0.1:7878") && !sc_server::local_origin("http://127.0.0.1.evil.com:80") && !sc_server::local_origin("http://127.0.0.1:"));
}

// ── the job (docs/paper-deck.md "API") ────────────────────────────────────────

/// A model call that never answers until dropped (the job's 停止 must abort it).
struct Hangs {
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

struct OnDrop(Arc<std::sync::atomic::AtomicBool>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl lore_llm::LlmProvider for Hangs {
    fn id(&self) -> String {
        "mock-hangs".into()
    }
    fn capabilities(&self) -> lore_llm::Capabilities {
        lore_llm::Capabilities { json: true, vision: true, web: false, streaming: true, context: 1_000_000, needs_network: false }
    }
    fn send<'a, 'b, 'c>(&'a self, _req: &'b lore_llm::LlmRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<lore_llm::LlmResponse, lore_llm::LlmError>> + Send + 'c>>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        let flag = self.dropped.clone();
        Box::pin(async move {
            let _guard = OnDrop(flag);
            std::future::pending::<()>().await;
            unreachable!()
        })
    }
}

async fn job_until(app: &axum::Router, id: &str, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..1000 {
        let (_, v, _) = call(app, "GET", &format!("/api/todeck/jobs/{id}"), None, None).await;
        if f(&v) {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("job never got to {what}");
}

#[tokio::test]
async fn a_job_waits_for_the_review_and_applies_it() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let f = MockFactory::default().with("claude:selected", Arc::new(MockProvider::new(vec![plan("is slow").to_string()])));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());

    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "short", "lang": "en", "style": "blank", "mode": "review" });
    let (st, v, _) = call(&app, "POST", "/api/todeck", Some(body), Some("http://127.0.0.1:7878")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "review", |v| v["state"] == "review").await;
    assert_eq!(v["step"], 6);
    assert_eq!(v["title"], "Two-Stage Grading of Synthetic Lesions");
    let stages = v["stages"].as_array().unwrap();
    assert_eq!(stages.iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), ["source", "notation", "terms", "profile", "outline", "review", "slides", "polish"]);
    assert_eq!(stages[0]["counts"], json!({ "fig": 1, "figTotal": 1, "table": 1, "eq": 1, "sections": 4, "refs": 1 }));
    assert_eq!(stages[1]["counts"]["entries"], 1);
    // 不熟: no KnowledgeBase here, so every term is unfamiliar
    let unknown = stages[2]["counts"]["unknown"].as_u64().unwrap();
    assert!(unknown >= 1 && stages[2]["counts"]["terms"].as_u64().unwrap() >= unknown, "{}", stages[2]);
    assert!(stages[4]["chars"].as_u64().unwrap() > 100, "the plan was streamed: {}", stages[4]);
    assert_eq!(stages[5]["state"], "running");
    let rv = &v["review"];
    let lgn = rv["unknown"].as_array().unwrap().iter().find(|t| t["term"].as_str().unwrap().contains("Lesion Grading Network")).cloned().unwrap_or_else(|| panic!("{}", rv["unknown"]));
    assert!(lgn["n"].as_u64().unwrap() >= 1);
    assert_eq!(rv["outline"].as_array().unwrap().len(), 12);
    assert_eq!(rv["outline"][2], json!({ "id": "s3", "kind": "overview", "title": "Big picture", "figs": ["fig1"], "point": "" }));
    let thumb = rv["figures"][0]["thumb"].as_str().unwrap().to_string();
    assert_eq!(rv["figures"][1]["label"], "表 1");
    let res = app.clone().oneshot(Request::builder().uri(&thumb).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.headers()["content-type"], "image/png");
    // the plan as written (a stage output) and nothing made yet
    let (st, o, _) = call(&app, "GET", &format!("/api/todeck/jobs/{id}/stages/outline"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(o["plan"]["slides"].as_array().unwrap().len(), 12);
    assert!(shared.ws.deck_names().is_empty());
    // listed as unfinished
    let (_, l, _) = call(&app, "GET", "/api/todeck/jobs?unfinished=1", None, None).await;
    assert_eq!(l["jobs"][0]["id"], id.as_str());

    // the author drops the opinion slide and retitles the results
    let mut rows: Vec<Value> = rv["outline"].as_array().unwrap().iter().filter(|r| r["kind"] != "opinion").cloned().collect();
    let res_row = rows.iter_mut().find(|r| r["kind"] == "results").unwrap();
    res_row["title"] = json!("What the numbers say");
    // 我懂 on the network: its marks never appear
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/review"), Some(json!({ "outline": rows, "answers": { "audience": "lab", "mainFig": null, "modules": null, "opinion": null }, "known": [lgn["term"]], "remember": false })), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = job_until(&app, &id, "done", |v| v["state"] == "done").await;
    let name = v["deck"].as_str().unwrap().to_string();
    assert_eq!(v["stages"][6]["counts"], json!({ "slide": 11, "slides": 11 }));
    assert_eq!(v["stages"][7]["counts"], json!({ "checks": 0, "fixes": 0 }));
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    assert_eq!(d["deck"]["slides"].as_array().unwrap().len(), 13, "11 planned, the big picture as 3");
    let text = all_text(&d["deck"]);
    assert!(text.contains("What the numbers say") && !text.contains("Some opinions"));
    assert_eq!(v["stages"][2]["counts"]["unknown"].as_u64().unwrap(), unknown - 1, "我懂 updated the terms");
    let saved: Value = serde_json::from_slice(&std::fs::read(shared.ws.decks.join(&name).join("terms.json")).unwrap()).unwrap();
    assert!(saved["terms"].as_array().unwrap().iter().any(|t| t["text"] == lgn["term"] && t["understood"] == true), "{saved}");
    // a second review is refused
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/review"), Some(json!({ "skip": true })), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[tokio::test]
async fn direct_skips_the_review() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let f = MockFactory::default().with("claude:selected", Arc::new(MockProvider::new(vec![plan("with the LGN").to_string()])));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "style": "blank", "lang": "en", "direct": true });
    let (st, v, _) = call(&app, "POST", "/api/todeck/jobs", Some(body), None).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "done", |v| v["state"] == "done").await;
    assert_eq!(v["stages"][5]["skipped"], true);
    assert_eq!(v["result"]["notation"], 1);
    assert!(v["result"]["unknown"].as_u64().unwrap() >= 1);
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{}", v["deck"].as_str().unwrap()), None, None).await;
    assert!(d.to_string().contains("\"em\":\"learn\""), "the unfamiliar LGN is marked");
    assert!(shared.ws.deck_names().iter().any(|d| d.name == v["deck"].as_str().unwrap()));
    let (st, r, _) = call(&app, "GET", &format!("/api/todeck/jobs/{id}/stages/review"), None, None).await;
    assert_eq!((st, r), (StatusCode::OK, json!({ "skipped": true })));
}

#[tokio::test]
async fn stop_aborts_the_model_call_and_a_new_process_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(workspace(dir.path()), sc_server::repo_dir().join("web"), home.clone());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = MockFactory::default().with("claude:selected", Arc::new(Hangs { dropped: dropped.clone() }));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home.clone(), f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "style": "blank", "lang": "en", "mode": "direct" });
    let (_, v, _) = call(&app, "POST", "/api/todeck", Some(body.clone()), None).await;
    let id = v["job"].as_str().unwrap().to_string();
    job_until(&app, &id, "outline", |v| v["stages"][4]["state"] == "running").await;

    // a second job hangs too: a fresh process finds it interrupted at the outline
    let (_, v2, _) = call(&app, "POST", "/api/todeck", Some(body), None).await;
    let id2 = v2["job"].as_str().unwrap().to_string();
    job_until(&app, &id2, "outline", |v| v["stages"][4]["state"] == "running").await;

    // 停止: the request is dropped, the worker released, the job resumable
    let t = std::time::Instant::now();
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/stop"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["state"], "stopped", "{v}");
    assert!(t.elapsed() < std::time::Duration::from_secs(3));
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst), "the in-flight model call was aborted");
    assert_eq!(v["resumable"], true);
    assert_eq!(v["stages"][4]["state"], "pending");

    // the app ends (the old state is simply abandoned) and starts again on the same workspace
    let source_before = std::fs::read(shared.ws.root.join(format!("cache/todeck/{id2}/source.json"))).unwrap();
    let home2 = lore_home::Home::at(dir.path().join("home"));
    let shared2 = sc_server::state_at(sc_core::Workspace::new(dir.path().join("sc"), None), sc_server::repo_dir().join("web"), home2.clone());
    let planner = Arc::new(MockProvider::new(vec![plan("is slow").to_string()]));
    assert!(shared2.llm.set(sc_server::llm::Llm::with_factory(home2, MockFactory::default().with("claude:selected", planner.clone()))).is_ok());
    let app2 = sc_server::router_isolated(shared2.clone());
    let (_, l, _) = call(&app2, "GET", "/api/todeck/jobs?unfinished=1", None, None).await;
    let j = l["jobs"].as_array().unwrap().iter().find(|j| j["id"] == id2.as_str()).cloned().unwrap();
    assert_eq!(j["state"], "interrupted");
    assert_eq!((j["doneSteps"].as_u64(), j["step"].as_u64()), (Some(4), Some(5)), "上次做到第 5 步");
    let (st, v, _) = call(&app2, "POST", &format!("/api/todeck/jobs/{id2}/resume"), Some(json!({})), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = job_until(&app2, &id2, "done", |v| v["state"] == "done").await;
    assert_eq!(planner.seen.lock().unwrap().len(), 1, "only the outline asked the model");
    assert_eq!(std::fs::read(shared2.ws.root.join(format!("cache/todeck/{id2}/source.json"))).unwrap(), source_before, "source was not redone");
    assert!(shared2.ws.deck_names().iter().any(|d| d.name == v["deck"].as_str().unwrap()));
    // a finished job can be deleted; ids are folder names only
    let (st, _, _) = call(&app2, "DELETE", &format!("/api/todeck/jobs/{id2}"), None, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = call(&app2, "GET", &format!("/api/todeck/jobs/{id2}"), None, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = call(&app2, "GET", "/api/todeck/jobs/..%2Fx", None, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn events_long_poll_answers_on_change() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(workspace(dir.path()), sc_server::repo_dir().join("web"), home.clone());
    let f = MockFactory::default().with("claude:selected", Arc::new(MockProvider::new(vec![plan("is slow").to_string()])));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let (_, v, _) = call(&app, "POST", "/api/todeck/jobs", Some(json!({ "source": { "kind": "pc", "path": p }, "style": "blank", "mode": "review" })), None).await;
    let id = v["id"].as_str().unwrap().to_string();
    let mut seq = 0;
    for _ in 0..200 {
        let (_, v, _) = call(&app, "GET", &format!("/api/todeck/jobs/{id}/events?after={seq}"), None, None).await;
        assert!(v["seq"].as_u64().unwrap() > seq);
        seq = v["seq"].as_u64().unwrap();
        if v["state"] == "review" {
            return;
        }
    }
    panic!("never reached the review");
}
