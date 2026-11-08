//! Browser dev mode: `cargo run -p pc-server`, then open http://127.0.0.1:7878.

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7878);
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), port).await.expect("bind");
    eprintln!("PaperComposer on http://{addr}");
    serve.await.expect("server");
}
