//! The server on a throwaway folder (the example data copied in), with no model, keychain or
//! `claude` CLI, for trying the UI: `cargo run -p le-server --example isolated_server -- <empty folder> [port]`.
use std::path::{Path, PathBuf};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create folder");
    for e in std::fs::read_dir(from).expect("read folder").flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).expect("copy file");
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("an empty folder"));
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(7961);
    copy_dir(&le_server::config::repo_dir().join("../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
    std::fs::create_dir_all(root.join("private")).expect("create private");
    let shared = le_server::AppState::new(lore_home::Home::at(&root), le_server::config::repo_dir().join("web"), le_server::llm::Llm::none(), None);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind 127.0.0.1");
    println!("http://{}/  folder {}", listener.local_addr().unwrap(), root.display());
    axum::serve(listener, le_server::router_isolated(shared)).await.expect("serve");
}
