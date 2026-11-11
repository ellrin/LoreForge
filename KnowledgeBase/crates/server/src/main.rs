//! Browser mode: `cargo run -p kb-server`, then open http://127.0.0.1:7700.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match kb_server::Config::load(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kb-server: {e}");
            std::process::exit(2);
        }
    };
    let kb = match kb_server::open(&config, kb_core::KbOptions::default()) {
        Ok(kb) => kb,
        Err(e) => {
            eprintln!("kb-server: {e}");
            std::process::exit(1);
        }
    };
    let spaces: Vec<String> = kb.spaces().iter().map(|s| format!("{} ({})", s.id(), s.root.display())).collect();
    let (addr, serve) = kb_server::bind(config, kb).await.expect("bind 127.0.0.1");
    eprintln!("KnowledgeBase on http://{addr} · spaces: {}", spaces.join(", "));
    tokio::select! {
        r = serve => r.expect("server"),
        _ = tokio::signal::ctrl_c() => {}
    }
    kb_server::llm::quit();
}
