//! lore-flow — draw a figure from its data-flow graph, without the app.
//!
//! ```text
//! lore-flow draw  fig.flow [-o fig.pptx] [--svg fig.svg] [--png fig.png] [--json fig.json]
//!                 [--theme theme.toml] [--modules DIR] [--template t.json]
//!                 [--notation notation.json] [--size 720x405] [--no-math]
//! lore-flow graph desc.md [-o fig.flow] [--image ref.png] [--engine claude:sonnet]
//!                 [--modules DIR] [--notation notation.json] [--examples DIR]
//! lore-flow check fig.flow [--notation notation.json]
//! ```
//!
//! Input is the text form (`.flow`) or L1 JSON (`.json`). `graph` asks an LLM
//! through lore-llm (the Claude subscription or a local model; nothing is
//! configured here, and no key is read or written).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use lore_flow::*;

struct Args {
    pos: Vec<String>,
    opts: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Args {
    fn parse(raw: Vec<String>) -> Args {
        let mut a = Args { pos: vec![], opts: vec![], flags: vec![] };
        let mut i = 0;
        while i < raw.len() {
            let s = &raw[i];
            if s == "--no-math" || s == "--help" || s == "-h" {
                a.flags.push(s.clone());
            } else if s.starts_with('-') && i + 1 < raw.len() {
                a.opts.push((s.clone(), raw[i + 1].clone()));
                i += 1;
            } else {
                a.pos.push(s.clone());
            }
            i += 1;
        }
        a
    }
    fn opt(&self, k: &str) -> Option<&str> {
        self.opts.iter().find(|(o, _)| o == k).map(|(_, v)| v.as_str())
    }
    fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }
}

fn read_graph(path: &Path, lib: &Library) -> Result<Graph, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if path.extension().is_some_and(|e| e == "json") || text.trim_start().starts_with('{') {
        let g = Graph::from_json(&text)?;
        g.validate(&lib.kinds()).map_err(|e| e.to_string())?;
        Ok(g)
    } else {
        dsl::parse_with(&text, &lib.kinds()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn load_notation(a: &Args) -> Result<Option<lore_notation::Notation>, String> {
    a.opt("--notation").map(|p| lore_notation::Notation::load(Path::new(p))).transpose()
}

fn load_library(a: &Args) -> Result<Library, String> {
    match a.opt("--modules") {
        Some(d) => Library::load_dir(Path::new(d)),
        None => Ok(Library::default()),
    }
}

fn chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CHROME").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium", "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Screenshot an SVG with headless Chrome (killed after a timeout).
fn svg_to_png(svg: &Path, png: &Path, w: u32, h: u32) -> Result<(), String> {
    let bin = chrome().ok_or("no Chrome found for the PNG (set CHROME, or use --svg)")?;
    let url = format!("file://{}", svg.canonicalize().map_err(|e| e.to_string())?.display());
    let mut child = std::process::Command::new(bin)
        .args(["--headless", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1"])
        .arg(format!("--window-size={w},{h}"))
        .arg(format!("--screenshot={}", png.display()))
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if start.elapsed().as_secs() > 40 => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    if png.is_file() { Ok(()) } else { Err("Chrome did not write the PNG".into()) }
}

fn cmd_draw(a: &Args) -> Result<(), String> {
    let input = PathBuf::from(a.pos.get(1).ok_or("draw needs an input file")?);
    let library = load_library(a)?;
    let g = read_graph(&input, &library)?;
    let theme = match a.opt("--theme") {
        Some(p) => Theme::from_toml(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?)?,
        None => Theme::classic_bw(),
    };
    let (w, h) = match a.opt("--size") {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size is WxH in pt, e.g. 720x405")?;
            (w.parse::<f64>().map_err(|e| e.to_string())?, h.parse::<f64>().map_err(|e| e.to_string())?)
        }
        None => (720.0, 405.0),
    };
    let margin = 20.0;
    let mut options = LayoutOptions { frame: Rect::new(margin, margin, w - 2.0 * margin, h - 2.0 * margin), ..LayoutOptions::default() };
    if let Some(p) = a.opt("--template") {
        options.template = Some(PanelTemplate::from_json(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?)?);
    }
    let notation = load_notation(a)?;
    let pandoc = PandocMath::new(lore_math::Pandoc::new("pandoc"));
    let math: &dyn MathSource = if a.flag("--no-math") || !pandoc.available() { &NoMath } else { &pandoc };
    if !a.flag("--no-math") && !pandoc.available() {
        eprintln!("pandoc not found: math is written as plain text runs");
    }
    let fig = draw(&g, &Drawing { theme, library, notation: notation.as_ref(), math, options, first_id: 2 }).map_err(|e| e.to_string())?;
    for wng in &fig.built.warnings {
        eprintln!("warning: {wng}");
    }
    let bends: Vec<usize> = fig.routes.iter().map(|r| r.bends()).collect();
    eprintln!(
        "{}: {} nodes, {} lines ({} straight, {} bent, {} polylines), scale {:.2}",
        g.fig,
        g.nodes.len(),
        bends.len(),
        bends.iter().filter(|b| **b == 0).count(),
        bends.iter().filter(|b| (1..=3).contains(*b)).count(),
        bends.iter().filter(|b| **b > 3).count(),
        fig.layout.scale
    );
    let out = a.opt("-o").map(PathBuf::from).unwrap_or_else(|| input.with_extension("pptx"));
    std::fs::write(&out, pptx::single_slide(fig.objects(), w, h)?).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("wrote {}", out.display());
    if let Some(p) = a.opt("--json") {
        std::fs::write(p, g.to_json()).map_err(|e| format!("{p}: {e}"))?;
    }
    let png = a.opt("--png").map(PathBuf::from);
    let svg_path = a.opt("--svg").map(PathBuf::from).or_else(|| png.as_ref().map(|p| p.with_extension("svg")));
    if let Some(sp) = &svg_path {
        let zoom = 2.0;
        std::fs::write(sp, preview::svg(fig.objects(), w, h, zoom)).map_err(|e| format!("{}: {e}", sp.display()))?;
        eprintln!("wrote {}", sp.display());
        if let Some(pp) = &png {
            svg_to_png(sp, pp, (w * zoom) as u32, (h * zoom) as u32)?;
            eprintln!("wrote {}", pp.display());
        }
    }
    Ok(())
}

fn cmd_check(a: &Args) -> Result<(), String> {
    let input = PathBuf::from(a.pos.get(1).ok_or("check needs an input file")?);
    let library = load_library(a)?;
    let mut g = read_graph(&input, &library)?;
    let notation = load_notation(a)?.unwrap_or(lore_notation::Notation { version: 1, entries: vec![], conventional: vec![], ignored: vec![] });
    let rep = notation::check_graph(&mut g, &notation);
    println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
    Ok(())
}

async fn cmd_graph(a: &Args) -> Result<(), String> {
    use lore_llm::router::{LocalAccess, SubscriptionAccess};
    use lore_llm::{Catalog, RouteOptions, Router, StandardFactory, TaskTable};
    let input = PathBuf::from(a.pos.get(1).ok_or("graph needs a description file")?);
    let description = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let library = load_library(a)?;
    let notation = load_notation(a)?;
    let image = match a.opt("--image") {
        Some(p) => Some(lore_llm::Image::from_file(Path::new(p)).map_err(|e| format!("{p}: {e}"))?),
        None => None,
    };
    // worked examples: DIR/<name>.md + DIR/<name>.flow
    let mut examples = Vec::new();
    if let Some(d) = a.opt("--examples") {
        let mut files: Vec<PathBuf> = std::fs::read_dir(d).map_err(|e| format!("{d}: {e}"))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "flow")).collect();
        files.sort();
        for f in files {
            if let Ok(desc) = std::fs::read_to_string(f.with_extension("md")) {
                examples.push((desc, read_graph(&f, &library)?));
            }
        }
    }
    // engines as the apps find them: the Claude subscription (Claude Code) and local models
    let mut factory = StandardFactory::new(Catalog::load(None));
    factory.subscription = Some(SubscriptionAccess { cli: Default::default(), cache: Arc::new(Default::default()), allow_web: false, temp_prefix: "lore-flow-claude".into() });
    factory.local = Some(LocalAccess::default());
    let table = TaskTable::bundled().merge(llm::task_table());
    let router = Router::new(table, factory);
    let opts = a.opt("--engine").map(RouteOptions::only).unwrap_or_default();
    let route = router.provider_for_with(llm::TASK, &opts).await.map_err(|e| format!("no engine for {}: {e}", llm::TASK))?;
    eprintln!("engine: {}", route.provider.id());
    let prompt = llm::FlowPrompt { description: &description, image, notation: notation.as_ref(), library: &library, examples: &examples };
    let proposal = match llm::propose(route.provider.as_ref(), &prompt).await {
        Ok(p) => p,
        Err(e) => return Err(e.to_string()),
    };
    if let Some(why) = &proposal.retried {
        eprintln!("first reply corrected: {why}");
    }
    let out = a.opt("-o").map(PathBuf::from).unwrap_or_else(|| input.with_extension("flow"));
    std::fs::write(&out, &proposal.dsl).map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::write(out.with_extension("notation.json"), serde_json::to_string_pretty(&proposal.notation).unwrap_or_default()).map_err(|e| e.to_string())?;
    eprintln!("wrote {} ({} nodes, {} lines, {} new variables, {} findings)", out.display(), proposal.graph.nodes.len(), proposal.graph.edges.len(), proposal.graph.vars_new.len(), proposal.notation.findings.len());
    Ok(())
}

const USAGE: &str = "lore-flow draw  FILE [-o out.pptx] [--svg out.svg] [--png out.png] [--json out.json] [--theme t.toml] [--modules DIR] [--template t.json] [--notation n.json] [--size WxH] [--no-math]
lore-flow graph DESC.md [-o out.flow] [--image ref.png] [--engine REF] [--modules DIR] [--notation n.json] [--examples DIR]
lore-flow check FILE [--notation n.json]";

fn main() -> ExitCode {
    let a = Args::parse(std::env::args().skip(1).collect());
    let r = match a.pos.first().map(String::as_str) {
        _ if a.flag("--help") || a.flag("-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("draw") => cmd_draw(&a),
        Some("check") => cmd_check(&a),
        Some("graph") => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(cmd_graph(&a)),
            Err(e) => Err(e.to_string()),
        },
        _ => Err(USAGE.to_string()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
