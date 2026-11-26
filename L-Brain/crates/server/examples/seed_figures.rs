//! Save figure specs as the made figures of their references in a hub, the way the page's 儲存
//! does: each is drawn once and its variables, connections, 指揮文字 and 口語說明 are derived.
//! `<dir>/private` holds the hub (a scratch copy, or the real one on your own risk).
//! `cargo run -p kb-server --example seed_figures -- <dir holding private/> <reference id>=<spec.json> …`
//! A reference id is as the hub lists it (`sc.flowrefs:f02`); a spec file is the flow tool's
//! `{"lore_flow":1,"dsl":"…","theme":"…","frame":[w,h]}`.

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: seed_figures <dir holding private/> <reference id>=<spec.json> ...");
        std::process::exit(2);
    }
    let dir = std::path::PathBuf::from(&args[0]);
    let mut figures = Vec::new();
    for a in &args[1..] {
        let Some((id, file)) = a.split_once('=') else {
            eprintln!("not <id>=<file>: {a}");
            std::process::exit(2);
        };
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
        let spec: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
        figures.push((id.to_string(), spec));
    }
    let home = lore_home::Home::at(&dir);
    let space = home.app_dir("KnowledgeBase").expect("private/KnowledgeBase");
    std::fs::create_dir_all(space.join("nodes")).unwrap();
    let app_dir = dir.join("app");
    let config = kb_server::Config { spaces: vec![space], member: "ellrin".into(), port: 0, web: kb_server::config::repo_dir().join("web"), intervals: Default::default(), app_dir: Some(app_dir.clone()) };
    let kb = kb_server::open(&config, Default::default()).expect("open");
    let rem = kb_server::reminders::Reminders::isolated(&app_dir, Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let shared = kb_server::AppState::with_hub(kb, config.web.clone(), rem, kb_server::llm::Llm::none(), kb_server::hub::HubState::from_home(&home));
    let app = kb_server::router_isolated(shared);
    let mut failed = 0;
    for (id, why) in kb_server::batch::seed(app, &figures).await {
        if why.is_empty() {
            println!("saved {id}");
        } else {
            failed += 1;
            println!("failed {id}: {why}");
        }
    }
    println!("{} figures, {failed} failed", figures.len());
    if failed > 0 {
        std::process::exit(1);
    }
}
