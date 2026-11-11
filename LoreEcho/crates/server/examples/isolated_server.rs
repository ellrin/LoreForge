//! The server on a throwaway folder (the example data copied in), with no model, keychain or
//! `claude` CLI, for trying the UI: `cargo run -p le-server --example isolated_server -- <empty folder> [port]`.
//! `--kb` adds the synthetic KnowledgeBase space of the tests (copied into the folder, member
//! `ellrin`, KnowledgeBase not running); `--mock "<reply>"` answers every message with that reply (several `--mock`: in turn, the last one forever).
//! `LOREECHO_VOICE=1`: with the system's voice (the Apple engines on macOS; it plays sound).
//! `LOREECHO_VOICE=paced`: a silent voice that "speaks" 5 characters a second with word events
//! (to look at 文字跟著聲音 without sound).
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
    // every `--mock "<reply>"`, in order (the last one forever): e.g. a chat reply, then the
    // JSON of 整理成筆記, then that of 做成知識卡
    let replies: Vec<String> = rest.iter().enumerate().filter(|(_, a)| *a == "--mock").filter_map(|(i, _)| rest.get(i + 1).cloned()).collect();
    let llm = match replies.is_empty() {
        false => {
            let mock = std::sync::Arc::new(lore_llm::MockProvider::new(replies).named("mock:local"));
            le_server::llm::Llm::with_factory(lore_llm::router::MockFactory::default().with("local:current", mock))
        }
        true => le_server::llm::Llm::none(),
    };
    let voice = match std::env::var("LOREECHO_VOICE").as_deref() {
        Ok("paced") => lore_voice::Voice::new(Box::new(Paced::default())),
        Ok(_) => lore_voice::Voice::system(),
        Err(_) => lore_voice::Voice::unavailable("這裡沒有語音"),
    };
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

/// A silent voice: each sentence takes 0.2 s per character, one word event per character.
#[derive(Default)]
struct Paced {
    sink: std::sync::Arc<std::sync::Mutex<Option<lore_voice::Sink>>>,
    queue: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<(u64, usize, u64)>>>>,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl lore_voice::Backend for Paced {
    fn start(&self, sink: lore_voice::Sink) {
        *self.sink.lock().unwrap() = Some(sink.clone());
        let (tx, rx) = std::sync::mpsc::channel::<(u64, usize, u64)>();
        *self.queue.lock().unwrap() = Some(tx);
        let epoch = self.epoch.clone();
        std::thread::spawn(move || {
            use lore_voice::Event;
            use std::sync::atomic::Ordering;
            for (id, n, e) in rx {
                if epoch.load(Ordering::SeqCst) != e {
                    continue; // stopped before it began
                }
                sink(Event::SpeakStarted { id });
                let mut cancelled = false;
                for i in 0..n {
                    if epoch.load(Ordering::SeqCst) != e {
                        cancelled = true;
                        break;
                    }
                    sink(Event::Word { id, start: i, len: 1 });
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                sink(Event::SpeakFinished { id, cancelled });
            }
        });
    }
    fn name(&self) -> &str {
        "paced"
    }
    fn voices(&self) -> Result<Vec<lore_voice::VoiceInfo>, String> {
        Ok(lore_voice::fake::voices())
    }
    fn listen(&self, _: &lore_voice::ListenOptions) -> Result<(), String> {
        Err("這個假聲音不能聽".into())
    }
    fn stop_listening(&self) {}
    fn speak(&self, id: u64, r: &lore_voice::SpeakRequest) -> Result<(), String> {
        let n = r.text.encode_utf16().count();
        self.queue.lock().unwrap().as_ref().ok_or("not started")?.send((id, n, self.epoch.load(std::sync::atomic::Ordering::SeqCst))).map_err(|e| e.to_string())
    }
    fn stop_speaking(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
