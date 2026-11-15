//! A server on a temporary folder that touches nothing real: no keychain, no `claude` CLI,
//! no private/ folder. `cargo run -p kb-server --example isolated_server -- <empty dir> <port>`

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().expect("an empty folder"));
    let port: u16 = a.next().and_then(|p| p.parse().ok()).expect("a port");
    let space = dir.join("personal");
    std::fs::create_dir_all(space.join("nodes")).unwrap();
    std::fs::write(space.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let config = kb_server::Config { spaces: vec![space], member: "ellrin".into(), port, web: kb_server::config::repo_dir().join("web"), intervals: Default::default(), app_dir: Some(dir.join("app")) };
    let kb = kb_server::open(&config, Default::default()).expect("open");
    let rem = kb_server::reminders::Reminders::isolated(&dir.join("app"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let shared = kb_server::AppState::with_llm(kb, config.web.clone(), rem, kb_server::llm::Llm::none());
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    eprintln!("http://{}", l.local_addr().unwrap());
    axum::serve(l, kb_server::router_isolated(shared)).await.unwrap();
}
