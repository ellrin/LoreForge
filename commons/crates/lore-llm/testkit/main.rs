//! Test helper for `lore_llm::shared_runtime` (never shipped, never run against real models).
//!
//! Two roles, chosen by the name it runs under:
//!
//! - **`llama-server`** (a copy of this binary under that name): a fake llama-server.
//!   Reads `--port`, `-m <weights>` and `--alias <id>`; the weights file holds
//!   `key=value` lines: `load_ms` (time before `/health` says 200, default 300) and
//!   `fail` (print it on stderr and exit at once). Answers `/health`, `/slots` (busy while
//!   a chat runs), `/v1/models` (the alias) and `/v1/chat/completions` (reply
//!   `<alias>: <last user text>`; a text `sleep:<ms>` takes that long).
//! - **anything else**: an app process. `lore-llm-testkit client <registry dir> <model home> <app>`
//!   reads commands on stdin, one per line, and answers one line each (`ok …` / `err …`):
//!   `chat <model> <interactive|background> <text>`, `acquire <model> <priority>` (holds
//!   the request open), `finish` (ends held requests), `idle <secs>`, `status`,
//!   `release`, `lock <ms>` (holds the registry lock), `quit` (the app's quit path).
//!   End of input = the process just ends (as if it crashed). Times from
//!   `LORE_TESTKIT_TIMING` = `min_residency,hold,promote_after` in seconds.

use std::io::{BufRead, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn main() {
    let argv0 = std::env::args().next().unwrap_or_default();
    let name = std::path::Path::new(&argv0).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    if name == "llama-server" {
        fake_server();
    } else {
        client();
    }
}

// ── the fake llama-server ───────────────────────────────

struct Fake {
    alias: String,
    ready_at: Instant,
    busy: AtomicUsize,
    tasks: AtomicUsize,
}

fn arg(args: &[String], k: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone())
}

fn fake_server() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = arg(&args, "--port").and_then(|p| p.parse().ok()).expect("--port");
    let weights = arg(&args, "-m").expect("-m");
    let alias = arg(&args, "--alias").unwrap_or_else(|| "model".into());
    let conf = std::fs::read_to_string(&weights).unwrap_or_default();
    let get = |k: &str| conf.lines().find_map(|l| l.trim().strip_prefix(&format!("{k}=")).map(str::to_string));
    if let Some(msg) = get("fail") {
        eprintln!("{msg}");
        std::process::exit(3);
    }
    let load = Duration::from_millis(get("load_ms").and_then(|v| v.parse().ok()).unwrap_or(300));
    eprintln!("fake llama-server {alias} on {port}");
    let fake = Arc::new(Fake { alias, ready_at: Instant::now() + load, busy: AtomicUsize::new(0), tasks: AtomicUsize::new(0) });
    let l = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
    for s in l.incoming().flatten() {
        let f = fake.clone();
        std::thread::spawn(move || serve(&f, s));
    }
}

fn serve(f: &Fake, mut s: std::net::TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head, mut body) = loop {
        let Ok(n) = s.read(&mut chunk) else { return };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break (String::from_utf8_lossy(&buf[..i]).into_owned(), buf[i + 4..].to_vec());
        }
    };
    let len: usize = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
        .unwrap_or(0);
    while body.len() < len {
        let Ok(n) = s.read(&mut chunk) else { return };
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let ready = Instant::now() >= f.ready_at;
    let (status, json) = match (method, path) {
        ("GET", "/health") if ready => (200, serde_json::json!({ "status": "ok" })),
        ("GET", "/health") => (503, serde_json::json!({ "error": { "message": "Loading model" } })),
        ("GET", "/slots") => (
            200,
            serde_json::json!([{ "id": 0, "id_task": f.tasks.load(Ordering::SeqCst), "is_processing": f.busy.load(Ordering::SeqCst) > 0 }]),
        ),
        ("GET", "/v1/models") => (200, serde_json::json!({ "data": [{ "id": f.alias }] })),
        ("POST", "/v1/chat/completions") if ready => {
            f.busy.fetch_add(1, Ordering::SeqCst);
            f.tasks.fetch_add(1, Ordering::SeqCst);
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let text = v["messages"].as_array().and_then(|m| m.iter().rev().find(|m| m["role"] == "user")).and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
            if let Some(ms) = text.strip_prefix("sleep:").and_then(|v| v.trim().parse().ok()) {
                std::thread::sleep(Duration::from_millis(ms));
            }
            f.busy.fetch_sub(1, Ordering::SeqCst);
            (200, serde_json::json!({ "choices": [{ "message": { "content": format!("{}: {text}", f.alias) }, "finish_reason": "stop" }] }))
        }
        ("POST", _) => (503, serde_json::json!({ "error": { "message": "Loading model" } })),
        _ => (404, serde_json::json!({ "error": "not found" })),
    };
    let body = json.to_string();
    let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
}

// ── an app process ──────────────────────────────────────

fn client() {
    use lore_llm::shared_runtime::{Priority, SharedLocal, SharedRuntime, Timing};
    use lore_llm::{Catalog, LlmProvider, LlmRequest};
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 || args[1] != "client" {
        eprintln!("usage: lore-llm-testkit client <registry dir> <model home> <app>");
        std::process::exit(2);
    }
    let (dir, home, app) = (args[2].clone(), std::path::PathBuf::from(&args[3]), args[4].clone());
    let t: Vec<f64> = std::env::var("LORE_TESTKIT_TIMING").unwrap_or_default().split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let d = |i: usize, def: f64| Duration::from_secs_f64(t.get(i).copied().unwrap_or(def));
    let timing = Timing {
        min_residency: d(0, 0.0),
        hold: d(1, 0.0),
        promote_after: d(2, 120.0),
        idle_tick: Duration::from_secs(3600),
        start_wait: Duration::from_secs(20),
        stop_grace: Duration::from_secs(2),
        poll_min: Duration::from_millis(100),
        poll_max: Duration::from_millis(300),
        probe_timeout: Duration::from_millis(800),
    };
    let rt = SharedRuntime::new(app, &dir, timing);
    let layout = lore_llm::local::Layout::shared(&home);
    let catalog = Catalog::bundled();
    let tokio = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio");
    let mut held = Vec::new();
    let stdout = std::io::stdout();
    let say = |line: String| {
        let mut o = stdout.lock();
        let _ = writeln!(o, "{line}");
        let _ = o.flush();
    };
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let w: Vec<&str> = line.split_whitespace().collect();
        let model = |i: usize| w.get(i).and_then(|id| catalog.get(id)).cloned();
        let prio = |i: usize| w.get(i).and_then(|p| Priority::parse(p)).unwrap_or_default();
        match w.first().copied() {
            Some("chat") => {
                let Some(m) = model(1) else { say("err unknown model".into()); continue };
                let p = SharedLocal { rt: rt.clone(), layout: layout.clone(), model: m, priority: prio(2), zh_tw: false, vision: false };
                let text = w[3..].join(" ");
                match tokio.block_on(p.complete(&LlmRequest::prompt("s", text))) {
                    Ok(r) => say(format!("ok {r}")),
                    Err(e) => say(format!("err {e}")),
                }
            }
            Some("acquire") => {
                let Some(m) = model(1) else { say("err unknown model".into()); continue };
                match tokio.block_on(rt.acquire(&layout, &m, prio(2))) {
                    Ok(tk) => {
                        say(format!("ok {}", tk.url));
                        held.push(tk);
                    }
                    Err(e) => say(format!("err {e}")),
                }
            }
            Some("finish") => {
                held.clear();
                say("ok".into());
            }
            Some("idle") => {
                let s: f64 = w.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                say(format!("ok {}", rt.check_idle_blocking(Some(Duration::from_secs_f64(s)))));
            }
            Some("status") => say(format!("ok {}", rt.snapshot_blocking().status.text(&catalog))),
            Some("release") => match rt.release_blocking() {
                Ok(()) => say("ok".into()),
                Err(e) => say(format!("err {e}")),
            },
            Some("lock") => {
                let ms: u64 = w.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                std::fs::create_dir_all(&dir).expect("registry folder");
                let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(std::path::Path::new(&dir).join(lore_llm::shared_runtime::LOCK)).expect("lock file");
                f.lock().expect("lock");
                say("ok locked".into());
                std::thread::sleep(Duration::from_millis(ms));
                drop(f);
                say("ok unlocked".into());
            }
            Some("quit") => {
                drop(held);
                rt.shutdown();
                say("ok bye".into());
                return;
            }
            _ => say(format!("err unknown command: {line}")),
        }
    }
    // end of input: gone without the quit path (a crash)
    std::process::exit(0);
}
