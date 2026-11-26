//! Draft and draw every 待製作 reference of a hub into a folder, to look at. Saves nothing.
//! `<dir>/private` holds the hub (a copy of `private-example/`, or the real one on your own risk).
//! `cargo run -p kb-server --example make_batch -- <dir holding private/> <out dir> [--real] [--no-keychain] [--page=<width_pt>,<w>:<h>] [id …]`
//! Without `--real` no model answers (every row says why); `--real` uses the shared model choice
//! of `<dir>/private/shared` and the model settings in `<dir>/app`.

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut real = false;
    let mut keychain = true;
    let mut page: Option<(f64, (u32, u32))> = None;
    let mut pos = vec![];
    for a in std::env::args().skip(1) {
        if a == "--real" {
            real = true;
        } else if a == "--no-keychain" {
            keychain = false;
        } else if let Some(v) = a.strip_prefix("--page=") {
            // --page=500,3:2 : every draft is made for a page 500 pt wide of ratio 3:2
            page = v.split_once(',').and_then(|(w, r)| {
                let (x, y) = r.split_once(':')?;
                Some((w.trim().parse().ok()?, (x.trim().parse().ok()?, y.trim().parse().ok()?)))
            });
            if page.is_none() {
                eprintln!("--page=<width_pt>,<w>:<h>");
                std::process::exit(2);
            }
        } else {
            pos.push(a);
        }
    }
    if pos.len() < 2 {
        eprintln!("usage: make_batch <dir holding private/> <out dir> [--real] [id ...]");
        std::process::exit(2);
    }
    let dir = std::path::PathBuf::from(&pos[0]);
    let out = std::path::PathBuf::from(&pos[1]);
    let only: Vec<String> = pos[2..].to_vec();
    let home = lore_home::Home::at(&dir);
    let space = home.app_dir("KnowledgeBase").expect("private/KnowledgeBase");
    std::fs::create_dir_all(space.join("nodes")).unwrap();
    let app_dir = dir.join("app");
    let config = kb_server::Config { spaces: vec![space], member: "ellrin".into(), port: 0, web: kb_server::config::repo_dir().join("web"), intervals: Default::default(), app_dir: Some(app_dir.clone()) };
    let kb = kb_server::open(&config, Default::default()).expect("open");
    let rem = kb_server::reminders::Reminders::isolated(&app_dir, Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let llm = if real {
        // as the real server builds it (`AppState::with_home_hub`), the folders being this one's
        let mut l = kb_server::llm::Llm::new(Some(&app_dir));
        l.share_engine(home.clone());
        if keychain { l } else { l.without_keychain() }
    } else {
        kb_server::llm::Llm::none()
    };
    let shared = kb_server::AppState::with_hub(kb, config.web.clone(), rem, llm, kb_server::hub::HubState::from_home(&home));
    let app = kb_server::router_isolated(shared);
    let made = kb_server::batch::run_on_page(app, &out, &only, page, &mut |n, total, m| {
        if m.error.is_empty() {
            println!("{n}/{total} ok {} ({})", m.title, m.engine);
        } else {
            println!("{n}/{total} failed {}: {}", m.title, m.error);
        }
    })
    .await
    .expect("write the folder");
    println!("{} references, {} failed", made.len(), made.iter().filter(|m| !m.error.is_empty()).count());
    println!("{}", out.join("index.html").display());
}
