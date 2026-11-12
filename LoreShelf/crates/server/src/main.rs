//! Browser mode: `cargo run -p shelf-server`, then open http://127.0.0.1:7900.
//! `LORESHELF_PORT` picks another port; `LORESHELF_HOME` another catalog.

#[tokio::main]
async fn main() {
    let catalog = lore_catalog::store::default_catalog_dir().unwrap_or_else(|| {
        eprintln!("shelf-server: 找不到目錄位置（請設定 LORESHELF_HOME）");
        std::process::exit(2);
    });
    let app = shelf_server::app_dir();
    let home = lore_home::Home::discover(&app);
    let cache = lore_catalog::store::default_cache_dir().unwrap_or_else(|| std::env::temp_dir().join("loreshelf-cache"));
    let shared = shelf_server::AppState::new(home, app.join("web"), catalog, cache, std::sync::Arc::new(shelf_server::SystemRunner));
    let port = std::env::var("LORESHELF_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(shelf_server::DEFAULT_PORT);
    let (addr, serve) = shelf_server::bind(shared, port).await.expect("bind 127.0.0.1");
    eprintln!("LoreShelf on http://{addr}");
    tokio::select! {
        r = serve => r.expect("server"),
        _ = tokio::signal::ctrl_c() => {}
    }
}
