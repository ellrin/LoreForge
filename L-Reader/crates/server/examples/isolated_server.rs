//! The server on a throwaway folder, for trying the UI without touching the
//! real library, notes or settings, and without any model, keychain or
//! `claude` CLI (`state(…, isolated = true)`):
//! `cargo run -p lr-server --example isolated_server -- <empty folder> [port] [kb url]`.
//! The folder gets library/, exports/ and ws/ (papers, shared settings).
use lr_core::config::{Config, EngineConfig};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("a folder"));
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(7990);
    let kb_url = args.next().unwrap_or_else(|| "http://127.0.0.1:9".into());
    let config = Config {
        library: root.join("library"),
        exports: root.join("exports"),
        kb_url,
        claude_model: None,
        engine: EngineConfig { url: None, model: None },
        app_dir: None,
    };
    let shared = lr_server::state(config, root.join("ws"), lr_server::repo_dir().join("web"), true);
    let (addr, serve) = lr_server::bind(shared, port).await.expect("bind 127.0.0.1");
    println!("http://{addr}/  folder {}", root.display());
    serve.await.expect("serve");
}
