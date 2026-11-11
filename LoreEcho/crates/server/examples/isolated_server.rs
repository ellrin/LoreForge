//! The server on a throwaway folder (the example data copied in), with no model, keychain or
//! `claude` CLI, for trying the UI: `cargo run -p le-server --example isolated_server -- <empty folder> [port]`.
//! `--kb` adds the synthetic KnowledgeBase space of the tests (copied into the folder, member
//! `ellrin`, KnowledgeBase not running); `--mock "<reply>"` answers every message with that reply.
//! `LOREECHO_VOICE=1`: with the system's voice (the Apple engines on macOS).
use std::path::{Path, PathBuf};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create folder");
    for e in std::fs::read_dir(from).expect("read folder").flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).expect("copy file");
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("an empty folder"));
    let rest: Vec<String> = args.collect();
    let port: u16 = rest.first().and_then(|p| p.parse().ok()).unwrap_or(7961);
    let flag = |name: &str| rest.iter().position(|a| a == name);
    copy_dir(&le_server::config::repo_dir().join("../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
    std::fs::create_dir_all(root.join("private")).expect("create private");
    let llm = match flag("--mock").and_then(|i| rest.get(i + 1)) {
        Some(reply) => {
            let mock = std::sync::Arc::new(lore_llm::MockProvider::new(vec![reply.clone()]).named("mock:local"));
            le_server::llm::Llm::with_factory(lore_llm::router::MockFactory::default().with("local:current", mock))
        }
        None => le_server::llm::Llm::none(),
    };
    let voice = if std::env::var_os("LOREECHO_VOICE").is_some() { lore_voice::Voice::system() } else { lore_voice::Voice::unavailable("這裡沒有語音") };
    let shared = le_server::AppState::with_voice(lore_home::Home::at(&root), le_server::config::repo_dir().join("web"), llm, None, voice);
    if flag("--kb").is_some() {
        let kb = root.join("kb");
        copy_dir(&le_server::config::repo_dir().join("crates/server/tests/fixtures/kb"), &kb);
        shared.kb.pin(le_server::kbctx::Pin { spaces: vec![kb], member: "ellrin".into(), url: None });
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind 127.0.0.1");
    println!("http://{}/  folder {}", listener.local_addr().unwrap(), root.display());
    axum::serve(listener, le_server::router_isolated(shared)).await.expect("serve");
}
