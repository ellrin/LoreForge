//! A server on a temporary folder that touches nothing real: no keychain, no `claude` CLI,
//! no private/ folder. `cargo run -p sc-server --example isolated_server -- <empty dir> <port>`

#[tokio::main]
async fn main() {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().expect("an empty folder"));
    let port: u16 = a.next().and_then(|p| p.parse().ok()).expect("a port");
    let ws = sc_core::Workspace::from_env(&dir);
    eprintln!("workspace: {}", ws.root.display());
    let home = lore_home::Home::at(dir.join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home);
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    eprintln!("http://{}", l.local_addr().unwrap());
    axum::serve(l, sc_server::router_isolated(shared)).await.unwrap();
}
