//! L-Brain with its hub on a copy of the public example data, to look at the hub's screens
//! without the author's folders: no keychain, no `claude` CLI, nothing under the real `private/`.
//! `<dir>/private` must be a copy of the repository's `private-example/`.
//! `cargo run -p kb-server --example hub_demo -- <dir> <port>`

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().expect("a folder holding private/ (a copy of private-example)"));
    let port: u16 = a.next().and_then(|p| p.parse().ok()).expect("a port");
    let home = lore_home::Home::at(&dir);
    let space = home.app_dir("KnowledgeBase").expect("private/KnowledgeBase");
    std::fs::create_dir_all(space.join("nodes")).unwrap();
    let config = kb_server::Config { spaces: vec![space], member: "ellrin".into(), port, web: kb_server::config::repo_dir().join("web"), intervals: Default::default(), app_dir: Some(dir.join("app")) };
    let kb = kb_server::open(&config, Default::default()).expect("open");
    let rem = kb_server::reminders::Reminders::isolated(&dir.join("app"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let shared = kb_server::AppState::with_hub(kb, config.web.clone(), rem, kb_server::llm::Llm::none(), kb_server::hub::HubState::from_home(&home));
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    eprintln!("http://{}", l.local_addr().unwrap());
    axum::serve(l, kb_server::router_isolated(shared)).await.unwrap();
}
