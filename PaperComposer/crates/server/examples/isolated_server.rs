//! The server on a throwaway copy of everything it reads and writes, for
//! trying the UI without touching real papers or settings:
//! `cargo run -p pc-server --example isolated_server -- <empty folder> [port]`.
//! The folder gets a copy of private-example/ (the workspace is its
//! PaperComposer/), a home/ for config and caches, and opened.toml.
fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("a folder"));
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(7990);
    std::fs::create_dir_all(root.join("home")).expect("home");
    // SAFETY: before any thread starts.
    unsafe {
        std::env::set_var("LOREFORGE_PRIVATE", &root);
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("PaperComposer"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("HOME", root.join("home"));
    }
    tokio::runtime::Runtime::new().expect("runtime").block_on(async {
        let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), port).await.expect("bind");
        println!("http://{addr}/  workspace {}", root.join("PaperComposer").display());
        serve.await.expect("serve");
    });
}
