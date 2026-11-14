//! A server on a temporary folder that touches nothing real: no keychain, no `claude` CLI,
//! no private/ folder. `cargo run -p sc-server --example isolated_server -- <empty dir> <port>`
//!
//! 論文報告 lists PaperComposer papers from `PAPERCOMPOSER_WORKSPACE` (default `<dir>/PaperComposer`)
//! and `PAPERCOMPOSER_OPENED` (default `<dir>/home/opened.toml`), never the real ones. It has no
//! engine, unless `SC_TODECK_PLAN=<plan.json>` (and optionally `SC_TODECK_BOXES=<modules.json>`)
//! give the replies a mock engine sends (UI checks); `SC_TODECK_EQ=<answer.json>` the equation-plot
//! answer for 會動的方程式 (the GIF itself is drawn by the real Chrome renderer).

use std::sync::Arc;

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().expect("an empty folder"));
    let port: u16 = a.next().and_then(|p| p.parse().ok()).expect("a port");
    for (k, v) in [("PAPERCOMPOSER_WORKSPACE", dir.join("PaperComposer")), ("PAPERCOMPOSER_OPENED", dir.join("home/opened.toml"))] {
        if std::env::var_os(k).is_none() {
            // SAFETY: before any thread starts.
            unsafe { std::env::set_var(k, v) };
        }
    }
    tokio::runtime::Runtime::new().expect("runtime").block_on(serve(dir, port));
}

async fn serve(dir: std::path::PathBuf, port: u16) {
    let ws = sc_core::Workspace::from_env(&dir);
    eprintln!("workspace: {}", ws.root.display());
    let home = lore_home::Home::at(dir.join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let file = |k: &str| std::env::var(k).ok().and_then(|p| std::fs::read_to_string(p).ok());
    let llm = match file("SC_TODECK_PLAN") {
        Some(plan) => {
            let mut f = lore_llm::router::MockFactory::default().with("claude:selected", Arc::new(lore_llm::MockProvider::new(vec![plan])));
            // claude:sonnet: the module boxes, then 會動的方程式's equation-plot answer (SC_TODECK_EQ)
            let sonnet: Vec<String> = [file("SC_TODECK_BOXES"), file("SC_TODECK_EQ")].into_iter().flatten().collect();
            if !sonnet.is_empty() {
                f = f.with("claude:sonnet", Arc::new(lore_llm::MockProvider::new(sonnet)));
            }
            sc_server::llm::Llm::with_factory(home, f)
        }
        None => sc_server::llm::Llm::none(home),
    };
    let _ = shared.llm.set(llm);
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    // headless Chrome renders object.html from this server (equation fallbacks, 另存圖片)
    *shared.port.lock().unwrap() = l.local_addr().unwrap().port();
    eprintln!("http://{}", l.local_addr().unwrap());
    axum::serve(l, sc_server::router_isolated(shared)).await.unwrap();
}
