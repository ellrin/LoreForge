//! Browser mode: `cargo run -p sc-server`, then open http://127.0.0.1:7880.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7880);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).and_then(|p| p.parse().ok()).expect("--port needs a number");
                i += 2;
            }
            other => {
                eprintln!("sc-server: unknown argument `{other}`");
                std::process::exit(2);
            }
        }
    }
    let repo = sc_server::repo_dir();
    // LoreForge private/SlideComposer, created from private-example/SlideComposer the first time.
    let home = ym_home::Home::discover(&repo);
    let default = match home.seed_app("SlideComposer") {
        Ok(Some(d)) => d,
        Ok(None) => repo.join("../private/SlideComposer"),
        Err(e) => {
            eprintln!("sc-server: private/SlideComposer: {e}");
            repo.join("../private/SlideComposer")
        }
    };
    let ws = sc_core::Workspace::from_env(&default);
    let web = repo.join("web");
    eprintln!("SlideComposer workspace: {} · decks: {}", ws.root.display(), ws.decks.display());
    let shared = sc_server::state(ws, web);
    let (addr, serve) = sc_server::bind(shared, port).await.expect("bind 127.0.0.1");
    eprintln!("SlideComposer on http://{addr}");
    tokio::select! {
        r = serve => r.expect("server"),
        _ = tokio::signal::ctrl_c() => {}
    }
}
