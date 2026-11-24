//! 論文報告 on a synthetic L-Paper paper (LaTeX folder, a drawn PNG figure, a table,
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

/// The engines of a test: 論文報告's own tasks run on Opus (`deck`), every other task (the
/// term suggestions, 會動的方程式) on the shared Claude choice (`other`).
fn engines(deck: Arc<dyn lore_llm::LlmProvider>, other: Arc<dyn lore_llm::LlmProvider>) -> MockFactory {
    MockFactory::default().with("claude:opus", deck).with("claude:selected", other)
}

/// An engine that answers by what it is asked: the first rule whose words the system prompt
/// contains gives its next reply (its last one again and again), else `rest`. Call order
/// between different kinds of request does not matter; every request is recorded.
pub struct Keyed {
    rules: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    rest: String,
    pub seen: std::sync::Mutex<Vec<lore_llm::LlmRequest>>,
}

impl Keyed {
    pub fn new(rest: impl Into<String>) -> Keyed {
        Keyed { rules: Default::default(), rest: rest.into(), seen: Default::default() }
    }
    pub fn on(self, words: &str, replies: &[Value]) -> Keyed {
        self.rules.lock().unwrap().push((words.to_string(), replies.iter().map(|v| v.to_string()).collect()));
        self
    }
    /// The requests whose system prompt contains `words`.
    pub fn asked(&self, words: &str) -> Vec<lore_llm::LlmRequest> {
        self.seen.lock().unwrap().iter().filter(|r| r.system.contains(words)).cloned().collect()
    }
}

#[async_trait::async_trait]
impl lore_llm::LlmProvider for Keyed {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> lore_llm::Capabilities {
        lore_llm::Capabilities { json: true, vision: true, web: false, streaming: true, context: 1_000_000, needs_network: false }
    }
    async fn send(&self, req: &lore_llm::LlmRequest) -> Result<lore_llm::LlmResponse, lore_llm::LlmError> {
        self.seen.lock().unwrap().push(req.clone());
        let mut rules = self.rules.lock().unwrap();
        let text = match rules.iter_mut().find(|(w, _)| req.system.contains(w.as_str())) {
            Some((_, r)) if r.len() > 1 => r.remove(0),
            Some((_, r)) => r.first().cloned().unwrap_or_default(),
            None => self.rest.clone(),
        };
        Ok(lore_llm::LlmResponse { text, provider: self.id(), sources: vec![], usage: None })
    }
}

/// System-prompt words of the job's requests.
pub const ASK_PLAN: &str = "You plan a paper-report seminar deck";
pub const ASK_PARTS: &str = "main method figure";

/// What a model "saw" in the synthetic figure (three boxes side by side): three parts, the
/// cuts a little off the blank gutters at 1/3 and 2/3.
fn parts() -> Value {
    json!({ "main": "fig1", "split": true, "axis": "x", "cuts": [0.30, 0.70], "why": "three stages", "parts": [
        { "label": "Encoder", "about": "Finds the lesions." }, { "label": "Fusion", "about": "Weighs them." }, { "label": "Decoder", "about": "Grades the image." }] })
}
pub const ASK_REVIEW: &str = "You review the outline";

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
            { "role": "background", "title": "Why grading is slow", "bullets": [format!("Grade by hand {lang_text}")], "cites": ["ellrin2025synthetic", "nobody2020"], "flow": ["2019: graded by hand", "2022: one-stage models", "Now: two stages"], "takeaway": format!("Hand grading does not scale {lang_text}") },
            { "role": "overview", "title": "Big picture", "bullets": [], "figure": "fig1" },
            { "role": "method", "title": "Encoder", "bullets": ["Finds lesions as in Fig. 3"], "figure": "fig1", "module": 1 },
            { "role": "method", "title": "Fusion", "bullets": ["Weighs each lesion by $\\vx$", "Keeps the largest three", "Passes one vector on"], "figure": "fig1", "module": 2 },
            { "role": "method", "title": "Decoder.", "bullets": ["Grades the image", "Outputs four grades"], "figure": "fig1", "module": 3 },
            { "role": "loss", "title": "Reconstruction loss", "bullets": ["Penalises the distance (Eq. (1))"], "equations": ["eq1", "eq99"], "explains": [1, 1], "takeaway": "One distance term trains both stages" },
            { "role": "training", "title": "Training details", "bullets": [], "kv": [["Epochs", "30"], ["Optimiser", "AdamW"], ["Learning rate", "$10^{-4}$"]], "takeaway": "A short schedule is enough" },
            { "role": "results", "title": "Main results", "bullets": [], "table": "tab1", "key_row": "ours", "takeaway": "Ours wins on both metrics, see Table 1", "notes": "This page compares the methods.\n1. Each row is one method trained on the same split." },
            { "role": "opinion", "title": "Some opinions", "bullets": ["Test on real data"], "suggest": "a real lesion photo beside a synthetic one", "takeaway": "Synthetic results need a real test" },
            { "role": "takeaway", "title": "Take home messages", "bullets": ["1. Two stages help", "2) Fusion matters"] },
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

/// The author's own earlier decks (synthetic, "imported": slides carry `src`) in the temp home's
/// `private/SlideComposer/decks`, for the profile and the voice examples. Footers, a confidential
/// mark and a presenter line repeat on every slide; a `Ref:` line and a name on one.
pub fn author_decks(home: &lore_home::Home) {
    use lore_canvas::{Bullet, Deck, Frame, Geom, Kind, Look, Object, Paint, Para, Run, Slide, SlideSrc, SpObj, TextBody};
    const W: i64 = 12_192_000;
    const H: i64 = 6_858_000;
    let text = |id: u32, f: Frame, lines: &[&str], ph: Option<&str>| {
        let paras = lines.iter().map(|t| { let mut r = Run::text(t, "body", Look { size: 18.0, ..Look::default() }); r.tag = None; let mut p = Para::new(vec![r]); if ph.is_none() { p.bullet = Bullet::Char { ch: "•".into() }; } p }).collect();
        Object { id, name: "t".into(), frame: f, hidden: false, ph: ph.map(String::from), slot: None, kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(TextBody { paras, ..Default::default() }), equation: false }), src: None }
    };
    let pages: [(&str, &[&str]); 8] = [
        ("Grading lesions with daidai", &["ellrin · lab meeting"]),
        ("Background", &["臨床上 grading 很花時間", "Experts disagree on borderline cases"]),
        ("Overview", &["Split the task into detect and grade"]),
        ("Encoder", &["Use a small CNN to find lesion candidates", "Keep the input at full resolution"]),
        ("Results", &["Beats the baseline on every metric", "Ref: Ellrin et al., Synthetic Lesions, 2025"]),
        ("Ablation study", &["Removing fusion costs the most"]),
        ("Take home messages", &["1. Split hard tasks into small steps", "2. Check every stage by hand"]),
        ("Thanks", &["Questions?"]),
    ];
    let decks = home.app_dir("SlideComposer").unwrap().join("decks");
    for (k, n) in ["talk-a", "talk-b", "talk-c"].iter().enumerate() {
        let mut d = Deck::new(W, H, Some("blank".into()));
        for (i, (title, body)) in pages.iter().enumerate() {
            let mut sl = Slide::new(d.new_uid(), None);
            sl.src = Some(SlideSrc { part: format!("ppt/slides/slide{i}.xml"), k: i as u64, notes_k: 0, anims_k: 0 });
            sl.objects.push(text(2, Frame::new(W / 25, H / 20, W * 9 / 10, H / 8), &[title], Some("title")));
            let mine: Vec<String> = body.iter().map(|b| if k == 0 { b.to_string() } else { format!("{b} ({n})") }).collect();
            let mine: Vec<&str> = mine.iter().map(String::as_str).collect();
            sl.objects.push(text(3, Frame::new(W / 25, H / 4, W * 9 / 10, H / 2), &mine, None));
            sl.objects.push(text(4, Frame::new(W / 25, H * 9 / 10, W / 2, H / 20), &["CONFIDENTIAL · Example Lab"], Some("ftr")));
            sl.objects.push(text(5, Frame::new(W * 3 / 5, H * 9 / 10, W / 3, H / 20), &["Presented by ellrin"], None));
            d.slides.push(sl);
        }
        std::fs::create_dir_all(decks.join(n)).unwrap();
        std::fs::write(decks.join(n).join("deck.json"), serde_json::to_vec(&d).unwrap()).unwrap();
    }
}

/// A page of bullets alone reads as undigested: every content page except the take-home one
/// shows a picture, a table, an equation, or an empty picture slot that says which picture to put.
fn assert_no_text_only_page(slides: &[Value]) {
    fn has_math(v: &Value) -> bool {
        match v {
            Value::Object(m) => m.get("kind").and_then(Value::as_str) == Some("math") || m.values().any(has_math),
            Value::Array(a) => a.iter().any(has_math),
            _ => false,
        }
    }
    for (i, sl) in slides.iter().enumerate().skip(1).take(slides.len().saturating_sub(2)) {
        let objs = sl["objects"].as_array().cloned().unwrap_or_default();
        if all_text(sl).contains("Take home messages") {
            continue;
        }
        let shows = objs.iter().any(|o| {
            matches!(o["type"].as_str(), Some("picture") | Some("table") | Some("group")) || o["name"].as_str().is_some_and(|n| n.starts_with("建議放：")) && o["slot"].as_str().is_some_and(|s| s.starts_with("image"))
        }) || has_math(sl);
        assert!(shows, "slide {} is text only: {}", i + 1, all_text(sl));
    }
}

/// 字級不一致是大忌: across a finished deck every text tag has one size, and the content
/// pages (all but the cover and the closing page) keep their title in one frame and start
/// their body on one line.
fn assert_one_size_per_role(slides: &[Value]) {
    fn runs(v: &Value, out: &mut Vec<(String, f64, String, String)>) {
        match v {
            Value::Object(m) => {
                if let (Some(tag), Some(look)) = (m.get("tag").and_then(Value::as_str), m.get("look")) {
                    if !m.get("text").and_then(Value::as_str).unwrap_or("").is_empty() || m.get("kind").and_then(Value::as_str) == Some("math") {
                        out.push((tag.to_string(), look["size"].as_f64().unwrap_or(0.0), look["font"].as_str().unwrap_or("").to_string(), look["ea_font"].as_str().unwrap_or("").to_string()));
                    }
                }
                m.values().for_each(|x| runs(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| runs(x, out)),
            _ => {}
        }
    }
    let mut all = vec![];
    slides.iter().for_each(|s| runs(s, &mut all));
    let mut sizes: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
    for (tag, size, _, _) in &all {
        sizes.entry(tag.clone()).or_default().insert(format!("{size}"));
    }
    assert!(sizes.len() >= 4, "{sizes:?}");
    for (tag, s) in &sizes {
        assert_eq!(s.len(), 1, "tag {tag} has more than one size: {s:?}");
    }
    let content = &slides[1..slides.len() - 1];
    let title = |s: &Value| s["objects"].as_array().unwrap().iter().find(|o| o["ph"] == "title" || o["slot"] == "title").map(|o| o["frame"].clone());
    let first = title(&content[0]).expect("a title");
    let mut tops = std::collections::BTreeSet::new();
    for s in content {
        assert_eq!(title(s), Some(first.clone()), "one title frame on every content page");
        let objs = s["objects"].as_array().unwrap();
        assert_eq!(objs.iter().filter(|o| o["ph"] == "title" || o["slot"] == "title" || o["name"] == "Title").count(), 1, "one title object");
        // the first thing under the title starts on the same line on every page
        let body_top = objs.iter().filter(|o| ["內文", "內文 1", "表", "圖", "圖的一段", "架構圖", "訓練細節"].contains(&o["name"].as_str().unwrap_or("")) || o["type"] == "group").map(|o| o["frame"]["y"].as_i64().unwrap()).min();
        tops.extend(body_top);
    }
    assert_eq!(tops.len(), 1, "the body starts on one line: {tops:?}");
    // a figure that is the subject of a slide gets the room: at least 80 % of the body width,
    // unless its height is what binds (then it fills most of the body's height)
    let body_w = content.iter().flat_map(|s| s["objects"].as_array().unwrap()).filter(|o| o["name"] == "內文").map(|o| o["frame"]["w"].as_i64().unwrap()).max().unwrap();
    let slide_h = body_w * 9 / 16;
    for o in content.iter().flat_map(|s| s["objects"].as_array().unwrap()).filter(|o| o["type"] == "picture" && ["圖", "架構圖", "表"].contains(&o["name"].as_str().unwrap_or(""))) {
        let (w, h) = (o["frame"]["w"].as_i64().unwrap(), o["frame"]["h"].as_i64().unwrap());
        assert!(w * 10 >= body_w * 8 || h * 10 >= slide_h * 5, "a small figure: {}", o["frame"]);
    }
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
    // the author's deck fonts and the people L-Paper keeps (synthetic)
    std::fs::create_dir_all(dir.path().join("home/private/shared")).unwrap();
    std::fs::write(dir.path().join("home/private/shared/fonts.toml"), "[deck]\nlatin = \"Brand Serif\"\neast_asian = \"Brand Sans TC\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("home/private/PaperComposer/profile")).unwrap();
    std::fs::write(dir.path().join("home/private/PaperComposer/profile/authors.csv"), "id,given,family,email,orcid,affiliations,role,note\nA001,Ellrin,Example,ellrin@example.org,,F001,,\nA002,Daidai,Example,daidai@example.org,,F001,,\n").unwrap();
    // the presenter's note patterns (private, hand-written; synthetic here)
    std::fs::create_dir_all(dir.path().join("home/private/SlideComposer/voice")).unwrap();
    std::fs::write(dir.path().join("home/private/SlideComposer/voice/notes.md"), "Open with what the page covers.\n1. Start from what is measured.").unwrap();
    let planner = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[plan("is slow")]).on(ASK_PARTS, &[parts()]));
    let f = engines(planner.clone(), planner.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());

    // who presents: ids and names only (the file also has e-mail addresses), nobody chosen yet
    let (st, who, h) = call(&app, "GET", "/api/todeck/people", None, Some("http://127.0.0.1:7878")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.get("access-control-allow-origin").is_some());
    assert_eq!(who, json!({ "people": [{ "id": "A001", "name": "Ellrin Example" }, { "id": "A002", "name": "Daidai Example" }], "presenter": null, "supervisor": null }));
    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "standard", "lang": "en", "style": "blank", "presenter": "A001", "supervisor": "A002" });
    let (st, v, h) = call(&app, "POST", "/api/todeck", Some(body.clone()), Some("http://127.0.0.1:7878")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, who, _) = call(&app, "GET", "/api/todeck/people", None, None).await;
    assert_eq!((who["presenter"].as_str(), who["supervisor"].as_str()), (Some("A001"), Some("A002")), "the choice is remembered for the next deck");
    assert_eq!(h.get("access-control-allow-origin").and_then(|x| x.to_str().ok()), Some("http://127.0.0.1:7878"));
    let name = v["name"].as_str().unwrap().to_string();
    assert_eq!(name, "Two-Stage-Grading-of-Synthetic-Lesions");
    assert_eq!((v["parts"].as_u64(), v["partsBy"].as_str(), v["planned"].as_u64()), (Some(3), Some("model"), Some(12)), "{v}");
    assert_eq!(v["notation"], 1, "the paper's symbol joins the deck's notation");

    // the model saw the paper's material by id
    let seen = planner.asked(ASK_PLAN)[0].flatten();
    assert!(seen.contains("Two-Stage Grading") && seen.contains("fig1: Overview of the pipeline") && seen.contains("tab1") && seen.contains("eq1"), "{seen}");
    assert!(seen.contains("ellrin2025synthetic"));
    // (the same engine also answered 會動的方程式's equation-plot: not a vision call)
    // a model looked at the figure before the plan was written, and the plan was told its parts
    assert_eq!(planner.asked(ASK_PARTS).len(), 1, "one vision call for the main figure");
    assert!(planner.asked(ASK_PARTS)[0].has_images());
    // the planner looks at the paper's pictures too
    let plan_req = planner.asked(ASK_PLAN)[0].clone();
    assert!(plan_req.has_images() && plan_req.flatten().contains("- image 1: figure fig1") && plan_req.system.contains("attached as images"));
    let sys = planner.asked(ASK_PLAN)[0].system.clone();
    assert!(sys.contains("exactly 3 slides, one for each part") && sys.contains("1. Encoder (Finds the lesions.); 2. Fusion"), "{sys}");
    assert!(sys.contains("How the presenter writes speaker notes") && sys.contains("1. Start from what is measured.") && sys.contains("numbered list"), "the notes rules and the private note patterns reach the planner");
    assert_eq!(planner.asked(ASK_PLAN).len(), 1, "a plan within the rules is not sent back");

    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    let slides = d["deck"]["slides"].as_array().unwrap().clone();
    // 12 planned, 12 slides: a walk-through's lines appear one by one on its own slide
    // (entrance animations), never as copies of the slide
    assert_eq!(slides.len(), 12);
    assert_eq!(slides[0]["layout"], "封面");
    assert_eq!(slides.last().unwrap()["layout"], "結尾");
    let mut all = vec![];
    texts(&d["deck"], &mut all);
    let joined = all.join("\n");
    assert!(joined.contains("Two-Stage Grading of Synthetic Lesions") && !joined.contains("Keywords:"), "the cover: title, journal, who presents");
    assert!(joined.contains("Journal of Synthetic Tests, 2025"));
    let cover = all_text(&slides[0]);
    assert!(cover.contains("Speaker: Ellrin Example") && cover.contains("Supervisor: Daidai Example") && !cover.contains("Ref:") && !cover.contains("example.org"), "{cover}");
    // one size per text role, one title frame, one body line — on the deck as polished
    assert_one_size_per_role(&slides);
    assert_no_text_only_page(&slides);
    // a page the paper has no picture for: the plan's chain is drawn as one flow picture
    // (a group that keeps its spec), or an empty picture slot says which picture to put
    let named = |title: &str| slides.iter().find(|s| all_text(s).contains(title)).unwrap()["objects"].as_array().unwrap().clone();
    let chain = named("Why grading is slow").into_iter().find(|o| o["type"] == "group").expect("the timeline is drawn");
    let mut boxes = vec![];
    texts(&chain, &mut boxes);
    assert!(boxes.join("|").contains("2019: graded by hand") && boxes.join("|").contains("Now: two stages"), "{boxes:?}");
    assert!(chain["data"].to_string().contains("n1 -> n2 -> n3"), "it reopens in the flow editor: {}", chain["data"]);
    assert!(named("Some opinions").iter().any(|o| o["name"] == "建議放：a real lesion photo beside a synthetic one" && o["slot"] == "image-1"));
    // the deck's fonts: all text in the author's deck fonts, equations in their math font
    assert_eq!(d["deck"]["fonts"], json!({ "latin": "Brand Serif", "east_asian": "Brand Sans TC" }));
    fn looks(v: &Value, out: &mut Vec<(String, String, String)>) {
        match v {
            Value::Object(m) => {
                if let (Some(k), Some(l)) = (m.get("kind").and_then(Value::as_str), m.get("look")) {
                    out.push((k.to_string(), l["font"].as_str().unwrap_or("").to_string(), l["ea_font"].as_str().unwrap_or("").to_string()));
                }
                m.values().for_each(|x| looks(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| looks(x, out)),
            _ => {}
        }
    }
    let mut ls = vec![];
    looks(&d["deck"]["slides"], &mut ls);
    assert!(ls.iter().filter(|l| l.0 == "text").count() > 20);
    for (kind, font, ea) in &ls {
        if kind == "text" {
            assert_eq!((font.as_str(), ea.as_str()), ("Brand Serif", "Brand Sans TC"));
        } else if kind == "math" {
            assert!(font.contains("Math"), "equations keep a math font: {font}");
        }
    }
    assert_eq!(d["style"]["tags"]["body"]["font"], "Brand Serif", "the editor's tags are the deck's");
    // the closing page: the title alone, centred
    let last = slides.last().unwrap()["objects"].as_array().unwrap().iter().find(|o| all_text(o).contains("Thanks")).unwrap().clone();
    assert!(all_text(slides.last().unwrap()).trim() == "Thanks" && last["frame"]["y"].as_i64().unwrap() > 1_000_000, "{last}");
    // take-home messages are numbered once (the model's own numbers are dropped)
    let home_msgs = slides.iter().find(|s| all_text(s).contains("Take home messages")).unwrap();
    assert!(all_text(home_msgs).contains("Two stages help") && !all_text(home_msgs).contains("1. Two stages"), "{}", all_text(home_msgs));
    // the takeaway: a key line under the table, never a filled box
    let res = slides.iter().find(|s| all_text(s).contains("Main results")).unwrap();
    let take = res["objects"].as_array().unwrap().iter().find(|o| o["name"] == "takeaway").unwrap();
    let table = res["objects"].as_array().unwrap().iter().find(|o| o["type"] == "table").unwrap();
    assert_eq!(take["type"], "text", "{take}");
    assert_eq!(take["text"]["paras"][0]["runs"][0]["em"], "key");
    assert!(take["frame"]["y"].as_i64().unwrap() >= table["frame"]["y"].as_i64().unwrap() + table["frame"]["h"].as_i64().unwrap());
    assert!(table["frame"]["w"].as_i64().unwrap() > 10_000_000, "the results table takes the body width: {}", table["frame"]);
    // the key row (the proposed method) stands out: bold here, the base has no key-row tint
    let rows = table["rows"].as_array().unwrap();
    let bold = |r: &Value| r["cells"][1]["text"]["paras"][0]["runs"][0]["look"]["bold"] == true;
    assert!(bold(&rows[1]) && !bold(&rows[2]), "{}", rows[1]);
    assert!(joined.contains("Ref: Ellrin and Daidai, Synthetic Lesions for Testing"), "cited key → Ref: line");
    assert!(!joined.contains("Fig. 3") && !joined.contains("Table 1") && !joined.contains("Eq. (1)"), "typed references are dropped");

    // the big picture: the figure whole, across the body width
    let content_w = slides.iter().flat_map(|s| s["objects"].as_array().unwrap()).filter(|o| o["name"] == "內文").map(|o| o["frame"]["w"].as_i64().unwrap()).max().unwrap();
    let big = slides.iter().find(|s| all_text(s).contains("Big picture")).unwrap();
    let pics: Vec<&Value> = big["objects"].as_array().unwrap().iter().filter(|o| o["type"] == "picture").collect();
    assert_eq!(pics.len(), 1);
    assert!(pics[0]["frame"]["w"].as_i64().unwrap() >= content_w * 8 / 10 && pics[0]["alpha"].as_f64().unwrap_or(1.0) > 0.99, "{}", pics[0]);
    // the walk-through of part 2: the whole figure faint, the part clear on top (cut at the
    // blank gutters, 1/3 and 2/3), never smaller than 80 % of the body width; its three lines
    // appear one by one
    let line = |o: &Value| o["name"].as_str().is_some_and(|n| n.starts_with("講解"));
    let fusion: Vec<&Value> = slides.iter().filter(|s| all_text(s).contains("Keeps the largest three")).collect();
    assert_eq!(fusion.len(), 1, "one slide, one title");
    {
        let sl = fusion[0];
        let objs = sl["objects"].as_array().unwrap();
        let pics: Vec<&Value> = objs.iter().filter(|o| o["type"] == "picture").collect();
        assert_eq!(pics.len(), 2, "{sl}");
        assert!(pics[0]["alpha"].as_f64().unwrap() < 0.5);
        assert!((pics[1]["crop"]["l"].as_f64().unwrap() - 1.0 / 3.0).abs() < 0.02 && (pics[1]["crop"]["r"].as_f64().unwrap() - 1.0 / 3.0).abs() < 0.02, "{}", pics[1]);
        assert_eq!(pics[0]["asset"], pics[1]["asset"], "one image file, two crops");
        // each line its own box, stacked under the figure across the body, each appearing on a click
        let lines: Vec<&Value> = objs.iter().filter(|o| line(o)).collect();
        assert_eq!(lines.len(), 3);
        let fig = &pics[0]["frame"];
        let mut y = fig["y"].as_i64().unwrap() + fig["h"].as_i64().unwrap();
        for l in &lines {
            assert_eq!(l["text"]["paras"].as_array().unwrap().len(), 1);
            assert!(l["frame"]["y"].as_i64().unwrap() >= y && l["frame"]["w"].as_i64().unwrap() == content_w, "{} {fig}", l["frame"]);
            y = l["frame"]["y"].as_i64().unwrap() + l["frame"]["h"].as_i64().unwrap();
        }
        let anims = sl["anims"].as_array().unwrap();
        assert_eq!(anims.iter().map(|a| (a["obj"].clone(), a["effect"].as_str().unwrap(), a["trigger"].as_str().unwrap())).collect::<Vec<_>>(), lines.iter().map(|l| (l["id"].clone(), "appear", "click")).collect::<Vec<_>>());
    }
    let uids: std::collections::BTreeSet<&str> = slides.iter().map(|s| s["uid"].as_str().unwrap()).collect();
    assert_eq!(uids.len(), slides.len());
    // the main figure is on the overview and its walk-through only; no other picture twice
    assert_eq!(slides.iter().filter(|s| s["objects"].as_array().unwrap().iter().any(|o| o["type"] == "picture")).count(), 1 + 3);

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
    // the paper's registered symbol is explained in the speaker notes of the slide whose
    // equation uses it (the last numbered item), never on the slide; the check reads it there
    let undefined_x = n["report"]["issues"].as_array().unwrap().iter().any(|i| i["code"] == "undefined_symbol" && i["entry"] == "x");
    assert!(!undefined_x, "{}", n["report"]["issues"]);
    assert!(!all_text(&d["deck"]["slides"]).contains("denotes input image"), "the symbol line is not on a slide");
    let with_sym: Vec<&Value> = slides.iter().filter(|s| s["notes"].as_str().unwrap_or("").contains("denotes input image")).collect();
    assert!(!with_sym.is_empty(), "the symbol line is in the notes");
    for s in &with_sym {
        let notes = s["notes"].as_str().unwrap();
        assert!(notes.lines().last().unwrap().contains("denotes input image"), "{notes}");
        assert!(sc_server::todeck::plan::notes_shape_ok(notes), "{notes}");
        assert!(s["objects"].to_string().contains("\\\\mathbf{x}") || !pandoc, "the notes' slide has the equation: {}", s["objects"]);
    }

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
    author_decks(&home);
    let decks_before = snapshot(&home.app_dir("SlideComposer").unwrap().join("decks"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let model = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[plan("is slow")]).on(ASK_PARTS, &[parts()]));
    let f = engines(model.clone(), model.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home.clone(), f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());

    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "standard", "lang": "en", "style": "blank", "mode": "review" });
    let (st, v, _) = call(&app, "POST", "/api/todeck", Some(body), Some("http://127.0.0.1:7878")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "review", |v| v["state"] == "review").await;
    assert_eq!(v["step"], 6);
    // 語氣: the author's own lines reach the outline prompt (never footers, marks, presenter
    // lines, references or the cover); they live in his private workspace, not in the job
    let voice_file = home.app_dir("SlideComposer").unwrap().join("voice/examples.json");
    assert!(voice_file.is_file());
    assert!(v["stages"][3]["counts"]["voice"].as_u64().unwrap() > 5, "{}", v["stages"][3]);
    let sys = model.seen.lock().unwrap().iter().find(|r| r.system.starts_with(ASK_PLAN)).unwrap().system.clone();
    assert!(sys.starts_with("You plan a paper-report") && sys.contains("Write like the presenter") && sys.contains("take-home lines \"2. Check every stage by hand\" · \"1. Split hard tasks into small steps\""), "the closest to the paper first, no near-copies: {sys}");
    assert!(sys.contains("titles \"Ablation study\"") && sys.contains("臨床上 grading 很花時間"), "{sys}");
    for never in ["CONFIDENTIAL", "Presented by", "Ref: Ellrin", "daidai", "Questions?"] {
        assert!(!sys.contains(never), "{never}");
    }
    let suggest_sys = model.seen.lock().unwrap().iter().find(|r| r.system.starts_with(ASK_REVIEW)).unwrap().system.clone();
    assert!(suggest_sys.contains("Write like the presenter"));
    let prof = std::fs::read_to_string(shared.ws.root.join(format!("cache/todeck/{id}/profile.json"))).unwrap();
    assert!(!prof.contains("Split hard tasks"), "no example text in the job's cache");
    assert_eq!(snapshot(&home.app_dir("SlideComposer").unwrap().join("decks")), decks_before, "the author's decks are read-only");
    assert_eq!(v["title"], "Two-Stage Grading of Synthetic Lesions");
    let stages = v["stages"].as_array().unwrap();
    assert_eq!(stages.iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), ["source", "notation", "terms", "profile", "outline", "review", "slides", "polish"]);
    assert_eq!(stages[0]["counts"], json!({ "fig": 1, "figTotal": 1, "table": 1, "eq": 1, "eqVision": 0, "eqPicture": 0, "sections": 4, "refs": 1, "warnings": 0, "calls": 0, "cached": 0 }));
    assert_eq!(stages[1]["counts"]["entries"], 1);
    // 不熟: no L-Brain here, so every term is unfamiliar
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
    assert_eq!(v["stages"][7]["counts"]["checks"], 6);
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    assert_eq!(d["deck"]["slides"].as_array().unwrap().len(), 11, "11 planned, 11 slides: lines appear by animation, not by copies");
    let text = all_text(&d["deck"]);
    assert!(text.contains("What the numbers say") && !text.contains("Some opinions"));
    assert_eq!(v["stages"][2]["counts"]["unknown"].as_u64().unwrap(), unknown - 1, "我懂 updated the terms");
    let saved: Value = serde_json::from_slice(&std::fs::read(shared.ws.decks.join(&name).join("terms.json")).unwrap()).unwrap();
    assert!(saved["terms"].as_array().unwrap().iter().any(|t| t["text"] == lgn["term"] && t["understood"] == true), "{saved}");
    // a second review is refused
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/review"), Some(json!({ "skip": true })), None).await;
    assert_eq!(st, StatusCode::CONFLICT);

    // 排版用字: the job's report is the deck's own report, every fix undoable through the editor
    let (_, job_rep, _) = call(&app, "GET", &format!("/api/todeck/jobs/{id}/stages/polish"), None, None).await;
    let (st, rep, _) = call(&app, "GET", &format!("/api/decks/{name}/polish"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(rep["items"], job_rep["items"], "{rep}");
    let items = rep["items"].as_array().unwrap();
    let fix = items.iter().find(|i| i["state"] == "applied").expect("the author's titles have no final period: one fix");
    {
        let (st, r, _) = call(&app, "POST", &format!("/api/decks/{name}/polish/{}", fix["id"]), Some(json!({ "action": "undo" })), None).await;
        assert_eq!(st, StatusCode::OK, "{r}");
        assert_eq!(r["item"]["state"], "undone");
        assert!(r["slides"].as_array().is_some_and(|s| !s.is_empty()) && r["order"].is_array(), "the changed slides, as an edit answers: {r}");
        let (_, again, _) = call(&app, "GET", &format!("/api/decks/{name}/polish"), None, None).await;
        assert_eq!(again["items"].as_array().unwrap().iter().find(|i| i["id"] == fix["id"]).unwrap()["state"], "undone");
    }
}

/// 微調's 方法拆幾個模組: a model looks at the figure again for that many parts and puts the
/// method slides on them; the deck walks through 2 parts instead of the 3 it was planned for.
#[tokio::test]
async fn the_authors_number_of_parts_recuts_the_main_figure() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let two = json!({ "main": "fig1", "split": true, "axis": "x", "cuts": [0.64], "parts": [{ "label": "Encoder and fusion" }, { "label": "Decoder" }], "slides": [1, 1, 2] });
    // a detail slide on part 2 of the three parts the plan was written for, and the trained parts
    let mut pl = plan("is slow");
    pl["slides"].as_array_mut().unwrap().insert(6, json!({ "role": "method", "title": "Fusion in detail", "part": 2, "bullets": ["Scores come from size"] }));
    pl["slides"][8]["trained"] = json!([1, 2]);
    pl["slides"][8]["kv"] = json!([]);
    pl["slides"][8]["bullets"] = json!(["Encoder and fusion are trained"]);
    let model = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[pl]).on(ASK_PARTS, &[parts(), two]));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, engines(model.clone(), model.clone()))).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "standard", "lang": "en", "style": "blank", "mode": "review" });
    let (_, v, _) = call(&app, "POST", "/api/todeck", Some(body), None).await;
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "review", |v| v["state"] == "review").await;
    let rows = v["review"]["outline"].clone();
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/review"), Some(json!({ "outline": rows, "answers": { "modules": 2 } })), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = job_until(&app, &id, "done", |v| v["state"] == "done").await;
    assert_eq!((v["result"]["parts"].as_u64(), v["result"]["modules"].as_u64(), v["result"]["calls"].as_u64()), (Some(2), Some(2), Some(1)), "{}", v["result"]);
    let asked = model.asked(ASK_PARTS);
    assert_eq!(asked.len(), 2);
    assert!(asked[1].flatten().contains("exactly 2 parts") && asked[1].flatten().contains("2. Fusion"), "{}", asked[1].flatten());
    let name = v["deck"].as_str().unwrap();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    let slides = d["deck"]["slides"].as_array().unwrap();
    // the figure was cut again into 2 parts: the plan's "part 2" and "parts 1 and 2" named the
    // old three, so neither page crops or marks anything, and both say so
    let warns: Vec<&str> = v["result"]["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(warns.iter().any(|w| w.contains("Fusion in detail") && w.contains("重新分段")) && warns.iter().any(|w| w.contains("Training details") && w.contains("重新分段")), "{warns:?}");
    for title in ["Fusion in detail", "Training details"] {
        let sl = slides.iter().find(|s| all_text(s).contains(title)).unwrap();
        assert!(!sl["objects"].as_array().unwrap().iter().any(|o| o["type"] == "picture"), "{title}");
    }
    // Encoder and Fusion on part 1 (the left 2/3, cut at the gutter), Decoder on part 2
    let crop = |title: &str| slides.iter().find(|s| all_text(s).contains(title) && s["objects"].as_array().unwrap().iter().any(|o| o["name"].as_str().is_some_and(|n| n.starts_with("講解")))).map(|s| s["objects"].as_array().unwrap().iter().filter(|o| o["type"] == "picture").nth(1).unwrap()["crop"].clone()).unwrap();
    assert!(crop("Encoder")["l"].as_f64().unwrap() < 0.01 && (crop("Keeps the largest")["r"].as_f64().unwrap() - 1.0 / 3.0).abs() < 0.02, "{}", crop("Encoder"));
    assert!((crop("Grades the image")["l"].as_f64().unwrap() - 2.0 / 3.0).abs() < 0.02);
    // part 1 leaves a faint side of one third: too narrow for the lines, so they go under the
    // figure across the body (the figure gives them the room but stays at least two thirds
    // wide); part 2's lines sit on its wide faint side
    let walk = |text: &str| slides.iter().rev().find(|s| all_text(s).contains(text) && s["objects"].as_array().unwrap().iter().any(|o| o["name"].as_str().is_some_and(|n| n.starts_with("講解")))).unwrap();
    let frame = |s: &Value, name: &str| s["objects"].as_array().unwrap().iter().find(|o| o["name"].as_str().is_some_and(|n| n.starts_with(name))).unwrap()["frame"].clone();
    let body_w = slides.iter().flat_map(|s| s["objects"].as_array().unwrap()).filter(|o| o["name"] == "內文").map(|o| o["frame"]["w"].as_i64().unwrap()).max().unwrap();
    let (fig, txt) = (frame(walk("Keeps the largest"), "架構圖"), frame(walk("Keeps the largest"), "講解"));
    assert!(txt["y"].as_i64().unwrap() >= fig["y"].as_i64().unwrap() + fig["h"].as_i64().unwrap() && txt["w"].as_i64().unwrap() == body_w, "{fig} {txt}");
    assert!(fig["w"].as_i64().unwrap() * 3 >= body_w * 2);
    let (fig, txt) = (frame(walk("Grades the image"), "架構圖"), frame(walk("Grades the image"), "講解"));
    assert!(txt["y"].as_i64().unwrap() < fig["y"].as_i64().unwrap() + fig["h"].as_i64().unwrap() && txt["w"].as_i64().unwrap() < body_w * 7 / 10, "{fig} {txt}");
}

#[tokio::test]
async fn direct_skips_the_review() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let mock = Arc::new(MockProvider::new(vec![plan("with the LGN").to_string()]));
    let f = { let m: Arc<dyn lore_llm::LlmProvider> = mock.clone(); engines(m.clone(), m) };
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
    // 直接產生 asks for no 預想選項
    assert!(!mock.seen.lock().unwrap().iter().any(|r| r.system.contains(ASK_INTERVIEW)));
    let (_, o, _) = call(&app, "GET", &format!("/api/todeck/jobs/{id}/stages/outline"), None, None).await;
    assert!(o.get("interview").is_none(), "{}", o["interview"]);
}

pub const ASK_INTERVIEW: &str = "You prepare answer options for the presenter";

/// 微調's 預想選項 and 補充: the options stand on the records sent (the paper, the author's own
/// L-Paper paper and decks; a deny word, an e-mail or a path never goes); a changed 補充 redoes
/// the outline with the lines in the prompt (the same one does nothing), a line no slide carries
/// is a warning; points the author edited survive the redo on the slides still there; a failed
/// call is a warning and no options, and the job goes on; outside the review 409.
#[tokio::test]
async fn interview_options_and_the_brief() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    author_decks(&home);
    std::fs::create_dir_all(dir.path().join("home/private/shared")).unwrap();
    std::fs::write(dir.path().join("home/private/shared/deny-words.txt"), "# synthetic\nexample-company\n").unwrap();
    // the author's own L-Paper papers (synthetic): one to cite, one a deny word keeps out
    let own = |name: &str, title: &str, abs: &str| {
        let d = dir.path().join("home/private/PaperComposer/papers").join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("main.tex"), format!("\\documentclass{{article}}\n\\title{{{title}}}\n\\begin{{document}}\n\\begin{{abstract}}\n{abs}\n\\end{{abstract}}\n\\section{{Intro}}\nText.\n\\end{{document}}\n")).unwrap();
    };
    own("early-grader", "A one-stage grader for synthetic lesions", "It reached 0.80 on the synthetic set. Write to ellrin@example.org for the data.");
    own("other", "Grading at Example-Company", "Nothing to say.");
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    // the redone outline: the background retitled (same place), the loss slide gone, the
    // results slide one place earlier (same title), the brief's first line on the background
    let mut second = plan("is slow");
    second["slides"][1]["title"] = json!("Where grading started");
    second["slides"][1]["bullets"] = json!(["Our earlier one-stage grader came first"]);
    second["slides"][4]["title"] = json!("Weighting the lesions");
    second["slides"].as_array_mut().unwrap().remove(6);
    let options = json!({
        "points": [{ "id": "s9", "cands": [
            { "text": "Ours reaches 0.91 against 0.85", "from": ["paper:tab1"] },
            { "text": "Ours reaches 0.97", "from": ["paper:tab1"] },
            { "text": "Both metrics move together", "from": ["paper:tab1", "paper:sec3"] }
        ] }],
        "brief": [
            { "text": "Our earlier one-stage grader is the predecessor", "from": ["pc:early-grader"] },
            { "text": "Recall the talk from last year", "from": ["deck:never-sent"] }
        ]
    });
    let model = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[plan("is slow"), second]).on(ASK_PARTS, &[parts()]).on(ASK_INTERVIEW, &[options, json!("not an object")]));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, engines(model.clone(), model.clone()))).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "standard", "lang": "en", "style": "blank", "mode": "review" });
    let (_, v, _) = call(&app, "POST", "/api/todeck", Some(body), None).await;
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "review", |v| v["state"] == "review").await;

    // the options as kept: an invented number and an id never sent are dropped, said once
    let iv = &v["review"]["interview"];
    assert_eq!(iv["points"], json!([{ "id": "s9", "cands": [{ "text": "Ours reaches 0.91 against 0.85", "from": ["paper:tab1"] }, { "text": "Both metrics move together", "from": ["paper:tab1", "paper:sec3"] }] }]), "{iv}");
    assert_eq!(iv["brief"], json!([{ "text": "Our earlier one-stage grader is the predecessor", "from": ["pc:early-grader"] }]));
    let rec_ids: Vec<&str> = iv["records"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(rec_ids, ["paper:sec3", "paper:tab1", "pc:early-grader"]);
    assert_eq!(iv["records"][2]["label"], "A one-stage grader for synthetic lesions");
    let warns: Vec<&str> = v["review"]["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(warns.iter().filter(|w| w.starts_with("預想選項")).collect::<Vec<_>>(), [&"預想選項有 2 個沒有根據（引用了沒有送出的資料，或數字在資料裡找不到），已拿掉"]);
    // what was sent: only record kinds the flow allows, nothing a deny word, an e-mail or a path names
    let asked = model.asked(ASK_INTERVIEW);
    let user = asked[0].flatten();
    let recs = user.split("## Records").nth(1).unwrap();
    for line in recs.lines().filter(|l| l.starts_with("- ")) {
        assert!(["- paper:", "- pc:", "- deck:", "- card:"].iter().any(|k| line.starts_with(k)), "{line}");
    }
    assert!(recs.contains("- pc:early-grader (A one-stage grader for synthetic lesions): A one-stage grader for synthetic lesions. It reached 0.80 on the synthetic set.") && recs.contains("- deck:talk-a"), "{recs}");
    let all = format!("{}\n{user}", asked[0].system);
    for never in ["Example-Company", "example.org", "pc:two-stage", "pc:other", &dir.path().to_string_lossy().to_string()] {
        assert!(!all.contains(never), "{never} was sent");
    }
    // an empty brief: the plan's request has no brief block
    assert!(!model.asked(ASK_PLAN)[0].flatten().contains("What only the presenter can say"));

    // the author edits three points, then adds 補充 (with the rows the page holds)
    let mut rows = v["review"]["outline"].as_array().unwrap().clone();
    rows[1]["point"] = json!("Graders disagree on borderline cases");
    rows[6]["point"] = json!("One distance, no weights to tune");
    rows[8]["point"] = json!("Ours reaches 0.91 against 0.85");
    rows[4]["point"] = json!("Fusion keeps three lesions");
    let brief = json!(["  Our earlier one-stage grader is the predecessor ", "", "Mention the open synthetic dataset"]);
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "brief": (0..9).map(|i| format!("line {i}")).collect::<Vec<_>>() })), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "at most 8 lines");
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "brief": brief, "outline": rows })), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = job_until(&app, &id, "review again", |v| v["state"] == "review" && v["stages"][4]["state"] == "done" && v["options"]["brief"].as_array().is_some_and(|b| !b.is_empty())).await;
    assert_eq!(v["options"]["brief"], json!(["Our earlier one-stage grader is the predecessor", "Mention the open synthetic dataset"]));
    let plans = model.asked(ASK_PLAN);
    assert!(plans.len() >= 2 && plans[1].flatten().contains("## What only the presenter can say") && plans[1].flatten().contains("- Our earlier one-stage grader is the predecessor\n- Mention the open synthetic dataset"), "{}", plans[1].flatten());
    let n_plans = plans.len();
    // the line no slide carries is said; the carried one is not
    let warns: Vec<&str> = v["review"]["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(warns.contains(&"補充的「Mention the open synthetic dataset」沒有放進大綱") && !warns.iter().any(|w| w.contains("predecessor」沒有")), "{warns:?}");
    // the edited points on the slides still there (by title, else by place and kind); the loss
    // slide is gone: said, not dropped silently
    let out = v["review"]["outline"].as_array().unwrap();
    assert_eq!((out[1]["title"].as_str(), out[1]["point"].as_str()), (Some("Where grading started"), Some("Graders disagree on borderline cases")));
    let res = out.iter().find(|r| r["kind"] == "results").unwrap();
    assert_eq!((res["id"].as_str(), res["point"].as_str()), (Some("s8"), Some("Ours reaches 0.91 against 0.85")));
    assert!(warns.iter().any(|w| w.starts_with("「Reconstruction loss」改過的重點") && w.contains("One distance, no weights to tune")), "{warns:?}");
    // a retitled walk-through slide (its kind repeats) never takes a point by place
    assert!(warns.iter().any(|w| w.starts_with("「Fusion」改過的重點")) && !out.iter().any(|r| r["point"] == "Fusion keeps three lesions"), "{warns:?}");
    // the second call failed: a warning, no options, the window still works
    assert!(v["review"].get("interview").is_none(), "{}", v["review"]);
    assert!(warns.iter().any(|w| w.starts_with("預想選項沒有做")), "{warns:?}");
    // the brief is in the options request too, not to be repeated
    assert!(model.asked(ASK_INTERVIEW)[1].flatten().contains("## Lines the presenter already added"));

    // the same brief again: nothing is redone
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "brief": ["Our earlier one-stage grader is the predecessor", "Mention the open synthetic dataset"] })), None).await;
    assert_eq!((st, v["state"].as_str()), (StatusCode::OK, Some("review")));
    assert_eq!(model.asked(ASK_PLAN).len(), n_plans);

    // the page submits its rows (the carried points among them): they reach the deck
    let rows = v["review"]["outline"].clone();
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/review"), Some(json!({ "outline": rows })), None).await;
    assert_eq!(st, StatusCode::OK);
    let v = job_until(&app, &id, "done", |v| v["state"] == "done").await;
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{}", v["deck"].as_str().unwrap()), None, None).await;
    let text = all_text(&d["deck"]);
    assert!(text.contains("Ours reaches 0.91 against 0.85") && text.contains("Graders disagree on borderline cases"), "{text}");
    // not waiting any more: 補充 is refused
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "brief": ["Another line"] })), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[tokio::test]
async fn stop_aborts_the_model_call_and_a_new_process_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(workspace(dir.path()), sc_server::repo_dir().join("web"), home.clone());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = { let m: Arc<dyn lore_llm::LlmProvider> = Arc::new(Hangs { dropped: dropped.clone() }); engines(m.clone(), m) };
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
    assert!(shared2.llm.set(sc_server::llm::Llm::with_factory(home2, engines(planner.clone(), planner.clone()))).is_ok());
    let app2 = sc_server::router_isolated(shared2.clone());
    let (_, l, _) = call(&app2, "GET", "/api/todeck/jobs?unfinished=1", None, None).await;
    let j = l["jobs"].as_array().unwrap().iter().find(|j| j["id"] == id2.as_str()).cloned().unwrap();
    assert_eq!(j["state"], "interrupted");
    assert_eq!((j["doneSteps"].as_u64(), j["step"].as_u64()), (Some(4), Some(5)), "上次做到第 5 步");
    let (st, v, _) = call(&app2, "POST", &format!("/api/todeck/jobs/{id2}/resume"), Some(json!({})), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = job_until(&app2, &id2, "done", |v| v["state"] == "done").await;
    let outlines = planner.seen.lock().unwrap().iter().filter(|r| r.system.starts_with("You plan a paper-report")).count();
    assert_eq!(outlines, 1, "the outline was asked once (polish's term step may ask too)");
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
    let f = { let m: Arc<dyn lore_llm::LlmProvider> = Arc::new(MockProvider::new(vec![plan("is slow").to_string()])); engines(m.clone(), m) };
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

/// 微調: the 建議 come with the outline (cleaned, at most 5); 語言／長度 redo only the outline
/// and the job waits again; 底版 needs no redo.
#[tokio::test]
async fn review_suggestions_and_changing_language_redoes_the_outline() {
    let dir = tempfile::tempdir().unwrap();
    let p = paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let sugg = json!({ "suggestions": [
        { "op": "retitle", "id": "s9", "title": "What the numbers say", "reason": "結果頁標題太籠統" },
        { "op": "remove", "id": "s1", "reason": "封面" },
        { "op": "figure", "id": "s2", "fig": "fig1", "reason": "背景頁沒有圖" }
    ] });
    let model = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[plan("is slow"), plan("zh again")]).on(ASK_REVIEW, &[sugg, json!({ "suggestions": [] })]));
    let f = engines(model.clone(), model.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "standard", "lang": "en", "style": "blank", "mode": "review" });
    let (_, v, _) = call(&app, "POST", "/api/todeck", Some(body), None).await;
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "review", |v| v["state"] == "review").await;
    assert_eq!(v["options"], json!({ "length": "standard", "lang": "en", "style": "blank", "brief": [] }));
    let s = v["review"]["suggestions"].as_array().unwrap();
    assert_eq!(s.len(), 2, "the cover is never edited: {s:?}");
    assert_eq!((s[0]["op"].as_str(), s[0]["id"].as_str(), s[0]["title"].as_str()), (Some("retitle"), Some("s9"), Some("What the numbers say")));
    assert_eq!(v["stages"][4]["counts"]["suggestions"], 2);
    let seen = model.asked(ASK_REVIEW);
    assert!(seen[0].flatten().contains("- s9 · results · Main results"), "{}", seen[0].flatten());
    let source_before = std::fs::read(shared.ws.root.join(format!("cache/todeck/{id}/source.json"))).unwrap();

    // bad values and a job not waiting are refused
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "lang": "fr" })), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // 底版 alone: no redo
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "style": "blank" })), None).await;
    assert_eq!((st, v["state"].as_str()), (StatusCode::OK, Some("review")));
    assert_eq!((model.asked(ASK_PLAN).len(), model.asked(ASK_REVIEW).len()), (1, 1));
    // 語言: the outline again, then back to the review with the new rows
    let (st, v, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "lang": "zh" })), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = job_until(&app, &id, "review again", |v| v["state"] == "review" && v["options"]["lang"] == "zh" && v["stages"][4]["state"] == "done").await;
    assert_eq!((model.asked(ASK_PLAN).len(), model.asked(ASK_REVIEW).len()), (2, 2), "plan + suggestions again");
    assert!(model.asked(ASK_PLAN)[1].system.contains("Traditional Chinese"));
    assert_eq!(v["review"]["suggestions"], json!([]));
    assert_eq!(v["review"]["outline"][1]["point"], "Hand grading does not scale zh again", "the review shows the new outline: a page's point is its conclusion");
    assert_eq!(std::fs::read(shared.ws.root.join(format!("cache/todeck/{id}/source.json"))).unwrap(), source_before, "source not redone");
    // not waiting any more: refused
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/review"), Some(json!({ "skip": true })), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(&app, "POST", &format!("/api/todeck/jobs/{id}/params"), Some(json!({ "lang": "en" })), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
}

/// UI check only (never in CI): a finished 論文報告 job in `SC_UI_DIR` (synthetic paper, synthetic
/// author decks, mock engines) to look at the 排版用字 report with the isolated server:
/// `SC_UI_DIR=<empty dir> cargo test -p sc-server --test todeck ui_fixture -- --ignored`, then
/// `SLIDECOMPOSER_WORKSPACE=<dir>/sc cargo run -p sc-server --example isolated_server -- <dir> <port>`.
#[tokio::test]
#[ignore]
async fn ui_fixture() {
    let Ok(d) = std::env::var("SC_UI_DIR") else { return };
    let dir = std::path::PathBuf::from(d);
    let p = paper(&dir);
    let ws = workspace(&dir);
    let home = lore_home::Home::at(dir.join("home"));
    author_decks(&home);
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let mut pl = plan("is slow");
    pl["slides"][4]["bullets"] = json!(["Weighs each lesion with a segmentation mask", "The mask map guides the fusion"]);
    pl["slides"][3]["title"] = json!("Encoder.");
    let terms = json!({ "groups": [{ "preferred": "segmentation mask", "variants": ["mask map"] }] });
    let f = { let m: Arc<dyn lore_llm::LlmProvider> = Arc::new(MockProvider::new(vec![pl.to_string(), json!({ "suggestions": [] }).to_string(), terms.to_string()])); engines(m.clone(), m) };
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "style": "blank", "lang": "en", "mode": "direct" });
    let (_, v, _) = call(&app, "POST", "/api/todeck", Some(body), None).await;
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "done", |v| v["state"] == "done").await;
    println!("deck {} {}", v["deck"], v["result"]["polish"]);
}

// ── 會動的方程式: the main loss gets its GIF ────────────────────────────

/// Draws synthetic frames instead of running Chrome (the real renderer is not in the offline tests).
struct FakeRender {
    specs: std::sync::Mutex<Vec<String>>,
}

impl sc_server::todeck::liveeq::Render for FakeRender {
    fn gif(&self, spec_json: &str, _macros: &Value) -> Result<Vec<u8>, String> {
        self.specs.lock().unwrap().push(spec_json.to_string());
        let frames: Vec<lore_plot::equation::gif::Rgba> = (0..sc_server::todeck::liveeq::FRAMES)
            .map(|k| {
                let (w, h) = (60u32, 38u32);
                let mut px = vec![255u8; (w * h * 4) as usize];
                let i = (((k as u32 % h) * w + 5) * 4) as usize;
                px[i..i + 3].copy_from_slice(&[0, 114, 178]);
                lore_plot::equation::gif::Rgba { w, h, px }
            })
            .collect();
        lore_plot::equation::gif::encode(&frames, 80, Some(spec_json))
    }
}

fn focal_paper(root: &Path) -> std::path::PathBuf {
    let p = paper(root);
    let main = MAIN
        .replace(r"\mathcal{L} = \lVert \vx - \hat{\vx} \rVert^2", r"\mathcal{L} = -\alpha_t (1 - p_t)^{\gamma} \log p_t")
        .replace("We train for 30 epochs", "We set $\\gamma = 2$ and $\\alpha_t = 0.25$; the ablation varies $\\gamma$ from 0 to 5. We train for 30 epochs");
    std::fs::write(p.join("main.tex"), main).unwrap();
    p
}

async fn run_with_eq_answer(answer: Value) -> (tempfile::TempDir, axum::Router, Value, Arc<FakeRender>, Arc<MockProvider>) {
    let dir = tempfile::tempdir().unwrap();
    let p = focal_paper(dir.path());
    let ws = workspace(dir.path());
    let home = lore_home::Home::at(dir.path().join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let planner = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[plan("is slow")]).on(ASK_PARTS, &[parts()]));
    // claude:sonnet answers the equation
    let sonnet = Arc::new(MockProvider::new(vec![answer.to_string()]).named("mock-sonnet"));
    let f = engines(planner.clone(), planner).with("claude:sonnet", sonnet.clone());
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, f)).is_ok());
    let fake = Arc::new(FakeRender { specs: Default::default() });
    assert!(shared.liveeq.set(fake.clone()).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "length": "standard", "lang": "en", "style": "blank" });
    let (st, v, _) = call(&app, "POST", "/api/todeck", Some(body), Some("http://127.0.0.1:7878")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    (dir, app, v, fake, sonnet)
}

#[tokio::test]
async fn the_main_loss_gets_an_animated_gif_that_reopens() {
    let answer = json!({ "match": "template", "template": "focal", "paper_values": [{ "name": "gamma", "value": 2 }, { "name": "alpha", "value": 0.25 }],
        "reduction": [], "properties": [{ "kind": "nonneg", "text": "≥ 0" }], "animate": "gamma" });
    let (_d, app, v, fake, sonnet) = run_with_eq_answer(answer).await;
    assert_eq!(v["live_eq"]["equation"], "eq1", "{v}");
    // the model read the loss, the paper's symbols and the tuning text
    let seen = sonnet.seen.lock().unwrap().last().unwrap().flatten();
    assert!(seen.contains("\\alpha_t (1 - p_t)^{\\gamma}") && seen.contains("ablation varies") && seen.contains("input image"), "{seen}");
    // one GIF, on the loss slide, sweeping γ with the paper's value marked; slide style
    let specs = fake.specs.lock().unwrap().clone();
    assert_eq!(specs.len(), 1);
    let spec: Value = serde_json::from_str(&specs[0]).unwrap();
    assert_eq!(spec["kind"], "equation");
    assert_eq!(spec["panels"][0]["eq"]["animate"]["var"], "gamma");
    assert_eq!(spec["panels"][0]["eq"]["vars"][0]["paper"], 2.0);
    // colour roles fixed in the file: the input 1, γ and α distinct (a re-edit keeps them)
    let roles: Vec<u64> = [&spec["panels"][0]["eq"]["x"]["color"], &spec["panels"][0]["eq"]["vars"][0]["color"], &spec["panels"][0]["eq"]["vars"][1]["color"]].iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(roles[0], 1);
    assert!(roles[1] != roles[2] && roles[1] >= 2 && roles[2] >= 2, "{roles:?}");
    assert!(spec["style"].as_str().unwrap().contains("slide"));
    let name = v["name"].as_str().unwrap();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    let slides = d["deck"]["slides"].as_array().unwrap();
    let gifs: Vec<(&Value, &Value)> = slides.iter().flat_map(|s| s["objects"].as_array().unwrap().iter().map(move |o| (s, o))).filter(|(_, o)| o["type"] == "picture" && o["asset"].as_str().unwrap().ends_with(".gif")).collect();
    assert_eq!(gifs.len(), 1, "one per deck");
    let (slide, pic) = gifs[0];
    assert!(all_text(slide).contains("Reconstruction loss"), "beside the equation on the loss slide");
    // under the text box, inside the slide, the figure's aspect
    let text = slide["objects"].as_array().unwrap().iter().find(|o| o["name"] == "內文").unwrap();
    let (tf, pf) = (&text["frame"], &pic["frame"]);
    assert!(pf["y"].as_i64().unwrap() >= tf["y"].as_i64().unwrap() + tf["h"].as_i64().unwrap(), "{tf} {pf}");
    assert!(pf["w"].as_f64().unwrap() >= tf["w"].as_f64().unwrap() * 0.6, "the GIF uses most of the slot's width: {tf} {pf}");
    // the slide's own equation is coloured like the GIF (deck palette): KaTeX in the editor, OMML in the .pptx
    let maths: Vec<&Value> = text["text"]["paras"].as_array().unwrap().iter().flat_map(|p| p["runs"].as_array().unwrap().iter()).filter(|r| r["kind"] == "math").collect();
    if let Some(m) = maths.first() {
        let l = m["latex"].as_str().unwrap();
        assert!(l.contains("\\textcolor{") && l.contains("{\\gamma}"), "{l}");
        if let Some(o) = m["omml"].as_str() {
            assert!(o.contains("a:srgbClr"), "{o}");
        }
    }
    let (_, nt, _) = call(&app, "GET", &format!("/api/decks/{name}/notation"), None, None).await;
    // lore-notation reads the coloured equation as the same symbols (no symbol named after the colouring)
    let subjects: Vec<String> = nt["report"]["issues"].as_array().unwrap().iter().filter_map(|i| i["subject"].as_str().map(String::from)).collect();
    assert!(!subjects.iter().any(|s| s.contains("textcolor") || s.contains('#')), "{subjects:?}");
    let keys: Vec<&str> = nt["report"]["symbols"].as_array().unwrap().iter().filter_map(|k| k["key"].as_str()).collect();
    assert!(keys.contains(&"\\gamma") && keys.contains(&"p") && !keys.iter().any(|k| k.contains("textcolor")), "{keys:?}");
    let ratio = pf["h"].as_f64().unwrap() / pf["w"].as_f64().unwrap();
    let (sw, sh) = sc_server::todeck::liveeq::DECK_MM;
    assert!((ratio - sh / sw).abs() < 0.01, "{ratio}");
    // 編輯 re-opens it: the GIF carries the spec
    let asset = pic["asset"].as_str().unwrap();
    let (st, sp, _) = call(&app, "GET", &format!("/api/decks/{name}/plot?asset={}", asset), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(sp["spec"]["panels"][0]["type"], "equation");
    // the .pptx keeps the animated GIF as a picture (PowerPoint plays it), spec still inside
    let res = app.clone().oneshot(Request::builder().uri(format!("/api/decks/{name}/pptx")).body(Body::empty()).unwrap()).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let gif_names: Vec<String> = zip.file_names().filter(|n| n.starts_with("ppt/media/") && n.ends_with(".gif")).map(String::from).collect();
    assert_eq!(gif_names.len(), 1, "{gif_names:?}");
    let mut buf = vec![];
    std::io::Read::read_to_end(&mut zip.by_name(&gif_names[0]).unwrap(), &mut buf).unwrap();
    let info = lore_plot::equation::gif::info(&buf).unwrap();
    assert_eq!(info.frames, sc_server::todeck::liveeq::FRAMES);
    assert_eq!(info.loops, Some(0));
    assert!(info.spec.is_some());
    let mut ct = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("[Content_Types].xml").unwrap(), &mut ct).unwrap();
    assert!(ct.contains("image/gif"));
}

#[tokio::test]
async fn an_unsure_equation_is_skipped_not_guessed() {
    // a custom expression (needs the author's eye) and no parameter the paper tunes: nothing inserted
    let answer = json!({ "match": "custom", "expr": "x^2", "x": { "name": "x", "min": -1, "max": 1 }, "vars": [], "reduction": [], "properties": [{ "kind": "nonneg", "text": "≥ 0" }] });
    let (_d, app, v, fake, _) = run_with_eq_answer(answer).await;
    assert!(v["live_eq"].is_null(), "{v}");
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("會動的方程式沒有放")), "{v}");
    assert!(fake.specs.lock().unwrap().is_empty());
    let name = v["name"].as_str().unwrap();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    assert!(!d.to_string().contains(".gif"));
    // a template without the paper's tuned parameter is skipped too
    let answer = json!({ "match": "template", "template": "focal", "paper_values": [], "reduction": [], "properties": [] });
    let (_d, _app, v, _, _) = run_with_eq_answer(answer).await;
    assert!(v["live_eq"].is_null(), "{v}");
}

// ── layout: a cropped part beside the text, the trained parts, paired equations ──

/// A synthetic pipeline figure, 900 × 300: three blocks that look different (blue with
/// rows, orange with a disc, green with bars), blank gutters at 1/3 and 2/3 crossed by a thin arrow line.
fn pipeline_png() -> Vec<u8> {
    let mut img = image::RgbaImage::from_pixel(900, 300, image::Rgba([255, 255, 255, 255]));
    let blocks = [(30u32, [52u8, 101, 164]), (330, [230, 145, 30]), (630, [60, 150, 90])];
    for (k, (x0, c)) in blocks.iter().enumerate() {
        for y in 40..260u32 {
            for x in *x0..*x0 + 240 {
                let (dx, dy) = (x as i32 - (*x0 as i32 + 120), y as i32 - 150);
                let light = match k {
                    0 => (y / 22) % 2 == 0,
                    1 => dx * dx + dy * dy < 70 * 70,
                    _ => (x / 30) % 2 == 0,
                };
                let edge = x < x0 + 6 || x >= x0 + 234 || !(46..254).contains(&y);
                let p = if edge { [c[0] / 2, c[1] / 2, c[2] / 2] } else if light { [c[0] / 2 + 127, c[1] / 2 + 127, c[2] / 2 + 127] } else { *c };
                img.put_pixel(x, y, image::Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
    for x in (270..330u32).chain(570..630) {
        for y in 149..152 {
            img.put_pixel(x, y, image::Rgba([60, 60, 60, 255]));
        }
    }
    let mut buf = std::io::Cursor::new(vec![]);
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

/// The synthetic paper with the pipeline figure and five display equations (two of them long).
fn layout_paper(root: &Path) -> std::path::PathBuf {
    let p = paper(root);
    let eqs = r"The total loss is
\begin{equation}
\mathcal{L} = \mathcal{L}_{rec} + \lambda \mathcal{L}_{cls}
\end{equation}
with the reconstruction term
\begin{equation}
\mathcal{L}_{rec} = \lVert \vx - \hat{\vx} \rVert^2
\end{equation}
and the grading term
\begin{equation}
\mathcal{L}_{cls} = -\sum_{k=1}^{4} y_k \log p_k
\end{equation}
The fusion weight of a lesion is
\begin{equation}
w_i = \frac{\exp(s_i / \tau)}{\sum_{j=1}^{N} \exp(s_j / \tau)} \cdot \frac{a_i}{\max_{j} a_j} + \beta \, \frac{1}{N} \sum_{j=1}^{N} s_j
\end{equation}
and the fused vector is
\begin{equation}
\mathbf{z} = \sum_{i=1}^{N} w_i \, \mathbf{h}_i + \gamma \, \frac{1}{N} \sum_{i=1}^{N} \mathbf{h}_i + \delta \, \max_{i} \mathbf{h}_i - \epsilon \, \min_{i} \mathbf{h}_i + \eta \, \frac{1}{N} \sum_{i=1}^{N} \lVert \mathbf{h}_i - \mathbf{z} \rVert^2
\end{equation}";
    let main = MAIN.replace("The reconstruction loss is\n\\begin{equation}\n\\mathcal{L} = \\lVert \\vx - \\hat{\\vx} \\rVert^2\n\\label{eq:loss}\n\\end{equation}", eqs);
    assert!(main.contains("mathcal{L}_{cls}"));
    std::fs::write(p.join("main.tex"), main).unwrap();
    std::fs::write(p.join("figures/arch.png"), pipeline_png()).unwrap();
    p
}

/// A scripted plan with every new arrangement and each fallback, one per slide.
fn layout_plan() -> Value {
    let notes = "This page gives the detail.\n1. The first point comes from what is measured.";
    json!({
        "venue": "Journal of Synthetic Tests, 2025",
        "keywords": ["lesion grading", "synthetic data"],
        "slides": [
            { "role": "cover", "title": "Two-Stage Grading of Synthetic Lesions", "bullets": [] },
            { "role": "background", "title": "Why grading is slow", "bullets": ["Experts grade every image by hand"], "flow": ["2019: graded by hand", "2022: one-stage models", "Now: two stages"] },
            { "role": "overview", "title": "Big picture", "bullets": [], "figure": "fig1" },
            { "role": "method", "title": "Encoder finds the lesions", "bullets": ["Finds lesion candidates", "Keeps full resolution"], "figure": "fig1", "module": 1 },
            { "role": "method", "title": "Fusion weighs the lesions", "bullets": ["Weighs each lesion", "Keeps the largest three"], "figure": "fig1", "module": 2 },
            { "role": "method", "title": "Decoder grades the image", "bullets": ["Grades the image", "Outputs four grades"], "figure": "fig1", "module": 3 },
            { "role": "method", "title": "How fusion picks three lesions", "part": 2, "bullets": ["Each lesion gets a score from its size and contrast", "The three highest scores are kept, the rest dropped", "One vector leaves the block, whatever the lesion count"], "notes": notes },
            { "role": "method", "title": "Fusion turns scores into weights", "part": 2, "bullets": ["A sharper temperature favours the largest lesion", "The mean score keeps small lesions from vanishing"], "equations": ["eq4"], "explains": [1], "notes": notes },
            { "role": "loss", "title": "One loss trains both stages", "bullets": ["Both stages share one objective", "The weight $\\lambda$ balances grading against reconstruction", "A larger weight favours the grade"], "equations": ["eq1"], "explains": [2], "notes": notes },
            { "role": "loss", "title": "What each loss term compares", "bullets": ["Reconstruction compares the image with its copy", "Grading compares the four grade scores with the label", "Both are averaged over the batch"], "equations": ["eq2", "eq3"], "explains": [1, 2], "notes": notes },
            { "role": "training", "title": "Only encoder and fusion are trained", "trained": [1, 2], "bullets": ["Encoder and fusion are trained, the decoder stays frozen", "The total loss with $\\lambda = 0.5$ trains them"], "notes": notes },
            { "role": "training", "title": "Training settings", "bullets": [], "kv": [["Epochs", "30"], ["Optimiser", "AdamW"], ["Learning rate", "$10^{-4}$"]], "notes": notes },
            { "role": "method", "title": "A part the figure does not have", "part": 7, "bullets": ["This slide names a part that was never confirmed", "It falls back to the usual page"], "notes": notes },
            { "role": "loss", "title": "An equation nobody explains", "bullets": ["This line is about something else", "So is this one"], "equations": ["eq2"], "notes": notes },
            { "role": "method", "title": "Two long equations stay paired", "bullets": ["The weight mixes a softmax over scores with the mean score", "The fused vector adds mean, maximum and minimum terms"], "equations": ["eq4", "eq5"], "explains": [1, 2], "notes": notes },
            { "role": "results", "title": "Main results", "bullets": ["Each row is one method on the same split"], "table": "tab1", "key_row": "ours", "takeaway": "Ours wins on both metrics", "notes": "This page compares the methods.\n1. Each row is one method trained on the same split." },
            { "role": "takeaway", "title": "Take home messages", "bullets": ["Two stages help", "Fusion matters"] },
            { "role": "thanks", "title": "Thanks", "bullets": [] }
        ]
    })
}

/// The deck of [`layout_plan`] in `dir` (direct mode, mock engines): the router, the deck's
/// name and the finished job.
async fn layout_deck(dir: &Path, plan: Value, parts: Value) -> (axum::Router, String, Value) {
    let p = layout_paper(dir);
    let ws = workspace(dir);
    let home = lore_home::Home::at(dir.join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let planner = Arc::new(Keyed::new("{}").on(ASK_PLAN, &[plan]).on(ASK_PARTS, &[parts]).on(ASK_REVIEW, &[json!({ "suggestions": [] })]));
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, engines(planner.clone(), planner))).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let body = json!({ "source": { "kind": "pc", "path": p }, "style": "blank", "lang": "en", "length": "long", "mode": "direct" });
    let (_, v, _) = call(&app, "POST", "/api/todeck", Some(body), None).await;
    let id = v["job"].as_str().unwrap().to_string();
    let v = job_until(&app, &id, "done", |v| v["state"] == "done" || v["state"] == "failed").await;
    assert_eq!(v["state"], "done", "{v}");
    (app, v["deck"].as_str().unwrap().to_string(), v)
}

/// Visual check only (never in CI): the layout deck in `SC_UI_DIR`, to print with the isolated
/// server (`SLIDECOMPOSER_WORKSPACE=<dir>/sc … isolated_server -- <dir> <port>`, then
/// `POST /api/decks/<name>/pdf`); its warnings go to `<dir>/warnings.json`.
#[tokio::test]
#[ignore]
async fn layout_fixture() {
    let Ok(d) = std::env::var("SC_UI_DIR") else { return };
    let dir = std::path::PathBuf::from(d);
    let mut plan = layout_plan();
    // `SC_LAYOUT_OLD=1`: the same plan without the pairing and part fields (the pages as they were)
    if std::env::var_os("SC_LAYOUT_OLD").is_some() {
        for s in plan["slides"].as_array_mut().unwrap() {
            for k in ["part", "trained", "explains"] {
                s.as_object_mut().unwrap().remove(k);
            }
        }
    }
    let (_, name, v) = layout_deck(&dir, plan, parts()).await;
    std::fs::write(dir.join("warnings.json"), serde_json::to_vec_pretty(&json!({ "deck": name, "warnings": v["result"]["warnings"] })).unwrap()).unwrap();
    println!("deck {name}");
}

/// The author's pages after a rehearsal, from a plan: the part a slide is about cropped from
/// the main figure beside its lines, the trained parts clear on the training slide, each
/// equation with the line that explains it; and each fallback with its warning.
#[tokio::test]
async fn a_part_beside_its_lines_the_trained_parts_and_paired_equations() {
    let dir = tempfile::tempdir().unwrap();
    let (app, name, v) = layout_deck(dir.path(), layout_plan(), parts()).await;
    let warns: Vec<&str> = v["result"]["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    let slides = d["deck"]["slides"].as_array().unwrap().clone();
    assert_eq!(slides.len(), 18, "{warns:?}");
    assert_one_size_per_role(&slides);
    assert_no_text_only_page(&slides);
    let objs = |title: &str| slides.iter().find(|s| all_text(s).contains(title)).unwrap_or_else(|| panic!("no slide {title}"))["objects"].as_array().unwrap().clone();
    let named = |o: &[Value], n: &str| o.iter().find(|x| x["name"] == n).cloned().unwrap_or_else(|| panic!("no {n}"));
    let fr = |o: &Value, k: &str| o["frame"][k].as_i64().unwrap();
    let pics = |o: &[Value]| o.iter().filter(|x| x["type"] == "picture").cloned().collect::<Vec<_>>();
    let kinds = |o: &Value| o["text"]["paras"].as_array().unwrap().iter().map(|p| p["runs"][0]["kind"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let pandoc = lore_math::Pandoc::default().available();
    let whole = named(&objs("Big picture"), "圖");
    let (content_x, content_w) = (fr(&whole, "x"), fr(&whole, "w"));
    let third = |x: &Value| (x.as_f64().unwrap() - 1.0 / 3.0).abs() < 0.01;

    // 1. the part: cropped at the confirmed cuts (the gutters at 1/3 and 2/3), not the whole
    // figure, readable, the lines beside it
    let o = objs("How fusion picks three lesions");
    assert_eq!(pics(&o).len(), 1, "the part alone, never the whole figure beside it");
    let part = named(&o, "圖的一段");
    assert!(third(&part["crop"]["l"]) && third(&part["crop"]["r"]) && part["crop"]["t"] == 0.0 && part["crop"]["b"] == 0.0, "the crop is the confirmed part 2: {}", part["crop"]);
    assert_eq!((part["asset"].clone(), part["alpha"].as_f64()), (whole["asset"].clone(), Some(1.0)), "the figure's own file, cropped");
    let text = named(&o, "內文");
    assert!(fr(&part, "w") * 10 >= content_w * 3 && fr(&part, "x") == content_x, "a readable size: {}", part["frame"]);
    assert!(fr(&text, "x") >= fr(&part, "x") + fr(&part, "w") && fr(&text, "y") == fr(&part, "y"), "the lines beside the part: {} {}", text["frame"], part["frame"]);
    assert_eq!(text["text"]["paras"].as_array().unwrap().len(), 3);
    // the same part with an equation: still beside, the equation under its line
    let o = objs("Fusion turns scores into weights");
    let (part, text) = (named(&o, "圖的一段"), named(&o, "內文"));
    assert!(fr(&text, "x") >= fr(&part, "x") + fr(&part, "w"));
    if pandoc {
        assert_eq!(kinds(&text), ["text", "math", "text"]);
    }

    // 2. the training slide: the whole figure faint, parts 1 and 2 clear as one span, the lines under it
    let o = objs("Only encoder and fusion are trained");
    let p = pics(&o);
    assert_eq!(p.len(), 2, "{p:?}");
    // (the walk-through's faint figure is drawn at the style's ghost alpha)
    let ghost = named(&objs("Fusion weighs the lesions"), "架構圖")["alpha"].as_f64().unwrap();
    assert!((p[0]["alpha"].as_f64().unwrap() - ghost).abs() < 1e-6 && ghost < 0.5 && p[0]["name"] == "架構圖", "the untrained rest at the style's ghost alpha: {}", p[0]);
    assert!(p[1]["crop"]["l"] == 0.0 && third(&p[1]["crop"]["r"]) && p[1]["alpha"] == 1.0, "{}", p[1]);
    assert_eq!((fr(&p[1], "x"), fr(&p[1], "y"), fr(&p[1], "h")), (fr(&p[0], "x"), fr(&p[0], "y"), fr(&p[0], "h")));
    assert!((fr(&p[1], "w") * 3 - fr(&p[0], "w") * 2).abs() < 100 && fr(&p[0], "w") * 10 >= content_w * 8, "{} {}", p[1]["frame"], p[0]["frame"]);
    let text = named(&o, "內文");
    assert!(fr(&text, "y") >= fr(&p[0], "y") + fr(&p[0], "h") && fr(&text, "w") == content_w);
    assert!(!o.iter().any(|x| x["type"] == "table") && objs("Training settings").iter().any(|x| x["type"] == "table"));

    if pandoc {
        // 3. one equation: right under the line that explains it (the second), in the page's one text box
        let o = objs("One loss trains both stages");
        let text = named(&o, "內文");
        assert_eq!(kinds(&text), ["text", "text", "math", "text"]);
        assert!(text["text"]["paras"][2]["runs"][0]["latex"].as_str().unwrap().contains("lambda"));
        // two equations: two columns, each line with its equation right under it, the two
        // equations on one line, the line that explains neither below
        let o = objs("What each loss term compares");
        let (l1, e1, l2, e2, rest) = (named(&o, "內文 1"), named(&o, "配對方程式 1"), named(&o, "內文 2"), named(&o, "配對方程式 2"), named(&o, "內文"));
        for (l, e) in [(&l1, &e1), (&l2, &e2)] {
            assert_eq!((fr(e, "x"), fr(e, "w"), fr(e, "y")), (fr(l, "x"), fr(l, "w"), fr(l, "y") + fr(l, "h")), "an equation's frame joins its line's: {} {}", l["frame"], e["frame"]);
            assert_eq!(kinds(e), ["math"]);
        }
        assert!(all_text(&l1).contains("Reconstruction") && e1["text"]["paras"][0]["runs"][0]["latex"].as_str().unwrap().contains("rec"));
        assert!(all_text(&l2).contains("Grading") && e2["text"]["paras"][0]["runs"][0]["latex"].as_str().unwrap().contains("cls"));
        assert!(fr(&l1, "y") == fr(&l2, "y") && fr(&e1, "y") == fr(&e2, "y") && fr(&l2, "x") >= fr(&l1, "x") + fr(&l1, "w") && fr(&l2, "x") + fr(&l2, "w") <= content_x + content_w);
        assert!(fr(&rest, "y") >= fr(&e1, "y") + fr(&e1, "h") && all_text(&rest).contains("Both are averaged") && kinds(&rest) == ["text"]);
        // two equations too wide for columns: stacked, each still under its own line
        let o = objs("Two long equations stay paired");
        assert!(!o.iter().any(|x| x["name"] == "內文 1"));
        assert_eq!(kinds(&named(&o, "內文")), ["text", "math", "text", "math"]);
        // fallback: an equation no line explains goes after the lines, as before, and is reported
        assert_eq!(kinds(&named(&objs("An equation nobody explains"), "內文")), ["text", "text", "math"]);
        assert!(warns.iter().any(|w| w.contains("An equation nobody") && w.contains("沒有配到說明它的那一行")), "{warns:?}");
        assert_eq!(warns.iter().filter(|w| w.contains("沒有配到說明")).count(), 1, "{warns:?}");
    }
    // fallback: a part the figure does not have: no crop, the usual page (an empty picture slot), reported
    let o = objs("A part the figure does not have");
    assert!(pics(&o).is_empty() && o.iter().any(|x| x["name"].as_str().unwrap().starts_with("建議放：")));
    assert!(warns.iter().any(|w| w.contains("A part the figure") && w.contains("架構圖沒有第 7 段")), "{warns:?}");
    assert!(!warns.iter().any(|w| w.contains("訓練的") || w.contains("How fusion")), "what worked is not reported: {warns:?}");

    // the export keeps the crop and the faint figure
    let res = app.clone().oneshot(Request::builder().uri(format!("/api/decks/{name}/pptx")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert!(lore_ooxml::Deck::from_bytes(&bytes).is_ok());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let xml = |zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, n: usize| {
        let mut x = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(&format!("ppt/slides/slide{n}.xml")).unwrap(), &mut x).unwrap();
        x
    };
    let at = |title: &str| slides.iter().position(|s| all_text(s).contains(title)).unwrap() + 1;
    let x = xml(&mut zip, at("How fusion picks three lesions"));
    assert!(x.contains(r#"<a:srcRect l="33333" r="33333"/>"#), "{x}");
    let x = xml(&mut zip, at("Only encoder and fusion are trained"));
    assert!(x.contains("<a:alphaModFix amt=") && x.contains(r#"<a:srcRect r="33333"/>"#), "{x}");
    if pandoc {
        let x = xml(&mut zip, at("What each loss term compares"));
        assert_eq!(x.matches("<m:oMath>").count(), 2, "{x}");
    }

    // lore-notation on the generated slides: the equations went the registered way (macros
    // expanded, no typed reference), wherever the layout put them
    let (_, n, _) = call(&app, "GET", &format!("/api/decks/{name}/notation"), None, None).await;
    let codes: Vec<String> = n["report"]["issues"].as_array().map(|a| a.iter().filter_map(|i| i["code"].as_str().map(String::from)).collect()).unwrap_or_default();
    assert!(!codes.iter().any(|c| c == "hardcoded_reference") && !d["deck"].to_string().contains("\\\\vx"), "{codes:?}");
    assert!(n["notation"]["entries"].as_array().unwrap().iter().any(|e| e["id"] == "x"));

    // 排版用字 has nothing to say about the new arrangements (no overflow, no missing caption)
    let (_, rep, _) = call(&app, "GET", &format!("/api/decks/{name}/polish"), None, None).await;
    let msgs = rep.to_string();
    assert!(!msgs.contains("文字超出框") && !msgs.contains("圖沒有圖說"), "{msgs}");
}
