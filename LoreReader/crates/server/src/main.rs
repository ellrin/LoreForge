//! Browser mode: `cargo run -p lr-server`, then open http://127.0.0.1:7890.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(lr_core::config::DEFAULT_PORT);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).and_then(|p| p.parse().ok()).expect("--port needs a number");
                i += 2;
            }
            other => {
                eprintln!("lr-server: unknown argument `{other}`");
                std::process::exit(2);
            }
        }
    }
    let config = match lr_core::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lr-server: {e}");
            std::process::exit(2);
        }
    };
    let ws = lr_server::default_workspace();
    eprintln!("LoreReader library: {} · exports: {} · workspace: {} · KB: {}", config.library.display(), config.exports.display(), ws.display(), config.kb_url);
    let shared = lr_server::state(config, ws, lr_server::repo_dir().join("web"), false);
    let (addr, serve) = lr_server::bind(shared, port).await.expect("bind 127.0.0.1");
    eprintln!("LoreReader on http://{addr}");
    tokio::select! {
        r = serve => r.expect("server"),
        _ = tokio::signal::ctrl_c() => {}
    }
    lr_server::llm::quit();
}
