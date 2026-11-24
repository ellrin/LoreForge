//! A server on a temporary folder that touches nothing real: no keychain, no `claude` CLI,
//! no private/ folder. `cargo run -p sc-server --example isolated_server -- <empty dir> <port>`
//!
//! 論文報告 lists L-Paper papers from `PAPERCOMPOSER_WORKSPACE` (default `<dir>/PaperComposer`)
//! and `PAPERCOMPOSER_OPENED` (default `<dir>/home/opened.toml`), never the real ones. It has no
//! engine, unless `SC_TODECK_PLAN=<plan.json>` (and optionally `SC_TODECK_BOXES=<modules.json>`)
//! give the replies a mock engine sends (UI checks); `SC_TODECK_EQ=<answer.json>` the equation-plot
//! answer for 會動的方程式 (the GIF itself is drawn by the real Chrome renderer).
//! `SC_PAGE_MOCK=<dir>` (test-only) scripts 改頁's two tasks: `<dir>/edit.json` and `check.json` are JSON arrays of replies, read at every call and taken in turn (the last one stays); requests go to `<dir>/seen.jsonl`.

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
        None => match std::env::var_os("SC_PAGE_MOCK") {
            Some(d) => {
                let mut f = lore_llm::router::MockFactory::default();
                for m in ["selected", "opus", "sonnet", "haiku", "fable"] {
                    f = f.with(&format!("claude:{m}"), Arc::new(PageMock { dir: d.clone().into(), model: format!("claude:{m}") }));
                }
                sc_server::llm::Llm::with_factory(home, f)
            }
            None => sc_server::llm::Llm::none(home),
        },
    };
    let _ = shared.llm.set(llm);
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    // headless Chrome renders object.html from this server (equation fallbacks, 另存圖片)
    *shared.port.lock().unwrap() = l.local_addr().unwrap().port();
    eprintln!("http://{}", l.local_addr().unwrap());
    axum::serve(l, sc_server::router_isolated(shared)).await.unwrap();
}

/// 改頁's scripted model (`SC_PAGE_MOCK`): replies from files, by the system prompt.
struct PageMock {
    dir: std::path::PathBuf,
    model: String,
}

#[async_trait::async_trait]
impl lore_llm::LlmProvider for PageMock {
    fn id(&self) -> String {
        self.model.clone()
    }
    fn capabilities(&self) -> lore_llm::Capabilities {
        lore_llm::Capabilities { json: true, vision: true, web: false, streaming: true, context: 1_000_000, needs_network: false }
    }
    async fn send(&self, req: &lore_llm::LlmRequest) -> Result<lore_llm::LlmResponse, lore_llm::LlmError> {
        use std::io::Write;
        let task = if req.system.contains("You change one or two slides") { "edit" } else { "check" };
        let parts = || req.messages.iter().flat_map(|m| m.content.iter());
        let images = parts().filter(|p| !matches!(p, lore_llm::Part::Text { .. })).count();
        let user: String = parts().filter_map(|p| if let lore_llm::Part::Text { text } = p { Some(text.as_str()) } else { None }).collect();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("seen.jsonl")) {
            let _ = writeln!(f, "{}", serde_json::json!({ "model": self.model, "task": task, "images": images, "user": user }));
        }
        let path = self.dir.join(format!("{task}.json"));
        let mut list: Vec<serde_json::Value> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let text = list.first().map(|v| v.to_string()).unwrap_or_else(|| "{}".into());
        if list.len() > 1 {
            list.remove(0);
            let _ = std::fs::write(&path, serde_json::to_string(&list).unwrap_or_default());
        }
        Ok(lore_llm::LlmResponse { text, provider: self.id(), sources: vec![], usage: None })
    }
}
