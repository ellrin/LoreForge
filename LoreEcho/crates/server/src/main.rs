//! Browser mode: `cargo run -p le-server`, then open http://127.0.0.1:7860.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match le_server::Config::load(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("le-server: {e}");
            std::process::exit(2);
        }
    };
    let (addr, shared, serve) = le_server::bind(config, None).await.expect("bind 127.0.0.1");
    let data = shared.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "(none)".into());
    eprintln!("LoreEcho on http://{addr} · data: {data}");
    tokio::select! {
        r = serve => r.expect("server"),
        _ = tokio::signal::ctrl_c() => {}
    }
    le_server::llm::quit();
}
