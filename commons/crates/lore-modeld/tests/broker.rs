//! The broker against a fake llama-server (never a real one): proxying (streamed and
//! not), the order requests reach the server, model changes, release, single instance
//! and the idle exit.

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::extract::{Json, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use futures_util::StreamExt;
use lore_modeld::sched::Config;
use lore_modeld::server::{Broker, router};
use lore_modeld::upstream::Upstream;
use lore_modeld::{Options, Outcome, serve};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// The fake llama-server: non-streaming requests answer at once; a streaming request
/// stays open until the test feeds it chunks through its gate and drops it.
#[derive(Default)]
struct Fake {
    /// Tag (the user message) of each request, in arrival order.
    arrived: Mutex<Vec<String>>,
    gates: Mutex<Vec<(String, mpsc::UnboundedSender<String>)>>,
}

async fn fake_chat(State(f): State<Arc<Fake>>, Json(b): Json<Value>) -> axum::response::Response {
    let tag = b["messages"].as_array().and_then(|m| m.last()).and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
    f.arrived.lock().unwrap().push(tag.clone());
    if b["stream"] == true {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        f.gates.lock().unwrap().push((tag, tx));
        let s = futures_util::stream::unfold(rx, |mut rx| async move {
            let chunk = rx.recv().await?;
            Some((Ok::<_, std::io::Error>(Bytes::from(format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{chunk}\"}}}}]}}\n\n"))), rx))
        })
        .chain(futures_util::stream::once(async { Ok(Bytes::from_static(b"data: [DONE]\n\n")) }));
        let mut r = axum::response::Response::new(Body::from_stream(s));
        r.headers_mut().insert("content-type", "text/event-stream".parse().unwrap());
        return r;
    }
    Json(json!({ "choices": [{ "message": { "content": format!("re:{tag}:{}", b["model"].as_str().unwrap_or("")) }, "finish_reason": "stop" }] })).into_response()
}

async fn serve_on(app: axum::Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

async fn fake_server() -> (Arc<Fake>, String) {
    let f = Arc::new(Fake::default());
    let app = axum::Router::new()
        .route("/v1/chat/completions", post(fake_chat))
        .route("/v1/models", get(|| async { Json(json!({ "data": [] })) }))
        .route("/slots", get(|| async { Json(json!([{ "id": 0, "id_task": 1, "is_processing": false }])) }))
        .with_state(f.clone());
    let url = serve_on(app).await;
    (f, format!("{url}/v1"))
}

/// The injected upstream: the fake server, with a record of model loads.
struct FakeUpstream {
    url: String,
    loaded: Mutex<Option<String>>,
    loads: Mutex<Vec<String>>,
}

#[async_trait]
impl Upstream for FakeUpstream {
    async fn ensure(&self, model: &str) -> Result<String, String> {
        if model == "missing" {
            return Err("還沒下載 missing".into());
        }
        let mut l = self.loaded.lock().unwrap();
        if l.as_deref() != Some(model) {
            *l = Some(model.to_string());
            self.loads.lock().unwrap().push(model.to_string());
        }
        Ok(self.url.clone())
    }
    async fn loaded(&self) -> Option<String> {
        self.loaded.lock().unwrap().clone()
    }
    async fn release(&self) {
        *self.loaded.lock().unwrap() = None;
    }
    fn phase(&self) -> Value {
        match &*self.loaded.lock().unwrap() {
            Some(m) => json!({ "state": "ready", "model": m }),
            None => json!({ "state": "idle" }),
        }
    }
    fn models(&self) -> Vec<String> {
        vec!["a".into(), "b".into()]
    }
}

struct Rig {
    fake: Arc<Fake>,
    up: Arc<FakeUpstream>,
    base: String,
    http: reqwest::Client,
}

async fn rig(config: Config) -> Rig {
    let (fake, url) = fake_server().await;
    let up = Arc::new(FakeUpstream { url, loaded: Mutex::new(None), loads: Mutex::default() });
    let broker = Broker::new(up.clone(), config);
    let base = serve_on(router(broker)).await;
    Rig { fake, up, base, http: reqwest::Client::new() }
}

impl Rig {
    fn chat(&self, app: Option<&str>, prio: Option<&str>, model: &str, tag: &str, stream: bool) -> reqwest::RequestBuilder {
        let mut rb = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .json(&json!({ "model": model, "stream": stream, "messages": [{ "role": "user", "content": tag }] }));
        if let Some(a) = app {
            rb = rb.header("X-Lore-App", a);
        }
        if let Some(p) = prio {
            rb = rb.header("X-Lore-Priority", p);
        }
        rb
    }

    /// Start a streaming request in the background; its whole text when it ends.
    fn spawn_stream(&self, app: &str, prio: &str, model: &str, tag: &str) -> tokio::task::JoinHandle<String> {
        let rb = self.chat(Some(app), Some(prio), model, tag, true);
        tokio::spawn(async move { rb.send().await.unwrap().text().await.unwrap() })
    }

    async fn status(&self) -> Value {
        self.http.get(format!("{}/api/status", self.base)).send().await.unwrap().json().await.unwrap()
    }

    async fn until(&self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..500 {
            let s = self.status().await;
            if f(&s) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out: {what}: {}", self.status().await);
    }

    /// Feed and close the open stream with this tag.
    async fn finish(&self, tag: &str) {
        for _ in 0..500 {
            let g = {
                let mut gates = self.fake.gates.lock().unwrap();
                gates.iter().position(|(t, _)| t == tag).map(|i| gates.remove(i))
            };
            if let Some((_, tx)) = g {
                tx.send(format!("{tag}-done")).unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{tag} never reached the server");
    }

    fn arrived(&self) -> Vec<String> {
        self.fake.arrived.lock().unwrap().clone()
    }
}

fn queue_apps(s: &Value) -> Vec<String> {
    s["queue"].as_array().unwrap().iter().map(|q| q["app"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn proxies_plain_replies_and_lists_models() {
    let r = rig(Config::default()).await;
    let s = r.status().await;
    assert_eq!(s["service"], "lore-modeld");
    assert!(s["loaded"].is_null() && s["running"] == 0 && s["focused"].is_null());
    let v: Value = r.chat(Some("pc"), Some("interactive"), "a", "hi", false).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "re:hi:a");
    // no headers at all: background from "unknown", still served
    let v: Value = r.chat(None, None, "a", "x", false).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "re:x:a");
    let s = r.status().await;
    assert_eq!((s["loaded"].as_str(), s["phase"]["state"].as_str()), (Some("a"), Some("ready")));
    let m: Value = r.http.get(format!("{}/v1/models", r.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(m["data"].as_array().unwrap().iter().map(|d| (d["id"].as_str().unwrap(), d["loaded"] == true)).collect::<Vec<_>>(), vec![("a", true), ("b", false)]);
    // errors in OpenAI shape
    let e = r.chat(Some("pc"), None, "missing", "x", false).send().await.unwrap();
    assert_eq!(e.status(), 503);
    assert!(e.json::<Value>().await.unwrap()["error"]["message"].as_str().unwrap().contains("還沒下載"));
    let e = r.http.post(format!("{}/v1/chat/completions", r.base)).json(&json!({ "messages": [] })).send().await.unwrap();
    assert_eq!(e.status(), 400);
    assert_eq!(r.status().await["running"], 0, "failed requests free their slot");
}

/// Streamed replies pass through chunk by chunk (the first arrives while the server
/// still holds the rest), with the SSE content type, and work with lore-llm's client.
#[tokio::test]
async fn streams_pass_through_chunk_by_chunk() {
    let r = rig(Config::default()).await;
    let resp = r.chat(Some("lr"), Some("interactive"), "a", "s1", true).send().await.unwrap();
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = resp.bytes_stream();
    let tx = loop {
        if let Some((_, tx)) = r.fake.gates.lock().unwrap().pop() {
            break tx;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    tx.send("one".into()).unwrap();
    let first = String::from_utf8(body.next().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(first.contains("\"one\""), "{first}");
    assert_eq!(r.status().await["running"], 1, "still running while the stream is open");
    tx.send("two".into()).unwrap();
    drop(tx);
    let mut rest = String::new();
    while let Some(c) = body.next().await {
        rest.push_str(std::str::from_utf8(&c.unwrap()).unwrap());
    }
    assert!(rest.contains("\"two\"") && rest.ends_with("data: [DONE]\n\n"), "{rest}");
    r.until("slot freed", |s| s["running"] == 0).await;

    // The same through lore-llm's broker client.
    use lore_llm::LlmProvider;
    let b = lore_llm::broker::BrokerAccess::new("lr", lore_llm::broker::Priority::Interactive);
    let p = b.provider(&r.base, "a");
    let feeder = async {
        r.finish("via-client").await;
    };
    let req = lore_llm::LlmRequest::prompt("s", "via-client");
    let (got, _) = tokio::join!(p.stream(&req, &|_| {}), feeder);
    assert_eq!(got.unwrap().text, "via-client-done");
}

/// The order requests reach the server: the focused app, then interactive, then
/// background, first come first served within a level; a running background request is
/// not cut.
#[tokio::test]
async fn requests_reach_the_server_in_priority_order() {
    let r = rig(Config { slots: 1, promote_after: None }).await;
    let first = r.spawn_stream("kb", "background", "a", "kb-1");
    r.until("kb-1 runs", |s| s["running"] == 1).await;
    let kb2 = r.spawn_stream("kb", "background", "a", "kb-2");
    r.until("queued", |s| s["queue"].as_array().unwrap().len() == 1).await;
    let lr = r.spawn_stream("lr", "interactive", "a", "lr-1");
    r.until("queued", |s| s["queue"].as_array().unwrap().len() == 2).await;
    let pc = r.spawn_stream("pc", "background", "a", "pc-1");
    let s = r.until("queued", |s| s["queue"].as_array().unwrap().len() == 3).await;
    assert_eq!(queue_apps(&s), vec!["lr", "kb", "pc"], "interactive first, then first come");
    let f: Value = r.http.post(format!("{}/api/focus", r.base)).json(&json!({ "app": "pc" })).send().await.unwrap().json().await.unwrap();
    assert_eq!(f["focused"], "pc");
    let s = r.status().await;
    assert_eq!(queue_apps(&s), vec!["pc", "lr", "kb"], "the focused app first");
    assert_eq!(s["focused"], "pc");
    assert_eq!(s["queue"][0]["priority"], "background");
    assert!(s["queue"][2]["waiting_ms"].as_u64().is_some());
    assert_eq!(r.arrived(), vec!["kb-1"], "kb-1 keeps running; nobody else reached the server");

    for tag in ["kb-1", "pc-1", "lr-1", "kb-2"] {
        r.finish(tag).await;
    }
    for h in [first, kb2, lr, pc] {
        h.await.unwrap();
    }
    assert_eq!(r.arrived(), vec!["kb-1", "pc-1", "lr-1", "kb-2"]);
}

/// A model change waits for the running request; the model is loaded once per change.
#[tokio::test]
async fn a_model_change_waits_for_the_running_request() {
    let r = rig(Config { slots: 2, promote_after: None }).await;
    let a1 = r.spawn_stream("kb", "background", "a", "a-1");
    r.until("a-1 runs", |s| s["running"] == 1).await;
    let b1 = r.spawn_stream("pc", "interactive", "b", "b-1");
    r.until("b-1 waits", |s| s["queue"].as_array().unwrap().len() == 1).await;
    let a2 = r.spawn_stream("lr", "interactive", "a", "a-2");
    let s = r.until("a-2 waits too", |s| s["queue"].as_array().unwrap().len() == 2).await;
    assert_eq!(s["running"], 1, "a free slot, but b needs a change and a-2 does not overtake it");
    assert_eq!(*r.up.loads.lock().unwrap(), vec!["a"]);
    r.finish("a-1").await;
    a1.await.unwrap();
    r.finish("b-1").await;
    b1.await.unwrap();
    r.finish("a-2").await;
    a2.await.unwrap();
    assert_eq!(r.arrived(), vec!["a-1", "b-1", "a-2"]);
    assert_eq!(*r.up.loads.lock().unwrap(), vec!["a", "b", "a"]);
}

/// Release: refused while a request runs, then the model goes.
#[tokio::test]
async fn release_is_refused_while_running() {
    let r = rig(Config::default()).await;
    let h = r.spawn_stream("pc", "interactive", "a", "busy");
    r.until("running", |s| s["running"] == 1).await;
    let resp = r.http.post(format!("{}/api/release", r.base)).send().await.unwrap();
    assert_eq!(resp.status(), 409);
    assert!(resp.json::<Value>().await.unwrap()["error"].as_str().unwrap().contains("使用中"));
    r.finish("busy").await;
    h.await.unwrap();
    r.until("idle", |s| s["running"] == 0).await;
    assert_eq!(r.http.post(format!("{}/api/release", r.base)).send().await.unwrap().status(), 204);
    let s = r.status().await;
    assert!(s["loaded"].is_null());
    assert_eq!(s["phase"]["state"], "idle");
    // the next request loads it again
    r.chat(Some("pc"), None, "a", "again", false).send().await.unwrap();
    assert_eq!(*r.up.loads.lock().unwrap(), vec!["a", "a"]);
}

/// A client that goes away while waiting leaves the queue.
#[tokio::test]
async fn a_client_that_gives_up_leaves_the_queue() {
    let r = rig(Config::default()).await;
    let h = r.spawn_stream("kb", "background", "a", "long");
    r.until("running", |s| s["running"] == 1).await;
    let waiting = r.spawn_stream("lr", "interactive", "a", "gives-up");
    r.until("queued", |s| s["queue"].as_array().unwrap().len() == 1).await;
    waiting.abort();
    r.until("withdrawn", |s| s["queue"].as_array().unwrap().is_empty()).await;
    r.finish("long").await;
    h.await.unwrap();
    assert_eq!(r.arrived(), vec!["long"]);
}

fn upstream(url: String) -> Arc<FakeUpstream> {
    Arc::new(FakeUpstream { url, loaded: Mutex::new(None), loads: Mutex::default() })
}

/// One broker per state file; it records itself, and leaves after the linger once the
/// model is released (and forgets its state file).
#[tokio::test]
async fn single_instance_and_idle_exit() {
    let d = tempfile::tempdir().unwrap();
    let state = d.path().join("modeld.json");
    let (_fake, url) = fake_server().await;
    let up = upstream(url.clone());
    let mut opts = Options::new(state.clone());
    opts.linger = Duration::from_millis(400);
    let first = tokio::spawn(serve(opts.clone(), up.clone(), std::future::pending()));
    let base = loop {
        if let Some(b) = lore_llm::broker::find_at(&state).await {
            break b;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(lore_llm::broker::read_state(&state).unwrap().1, std::process::id());
    let second = serve(opts.clone(), upstream(url.clone()), std::future::pending()).await.unwrap();
    assert_eq!(second, Outcome::AlreadyRunning(base.clone()));

    // A loaded model keeps it alive past the linger.
    let http = reqwest::Client::new();
    http.post(format!("{base}/v1/chat/completions")).json(&json!({ "model": "a", "messages": [] })).send().await.unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(!first.is_finished(), "model loaded: stays");
    up.release().await;
    let out = tokio::time::timeout(Duration::from_secs(5), first).await.expect("exits after the linger").unwrap().unwrap();
    assert_eq!(out, Outcome::Stopped);
    assert!(!state.exists(), "state file removed");
}

/// The real binary: a second start exits at once (state under `LOREFORGE_CACHE`,
/// nothing read from a real private folder or model folder).
#[cfg(unix)]
#[tokio::test]
async fn the_binary_runs_once() {
    let d = tempfile::tempdir().unwrap();
    let cache = d.path().join("cache");
    let bin = env!("CARGO_BIN_EXE_lore-modeld");
    let cmd = || {
        let mut c = tokio::process::Command::new(bin);
        c.env("LOREFORGE_CACHE", &cache)
            .env("LOREFORGE_HOME", d.path().join("home"))
            .env("LOCAL_LLM_HOME", d.path().join("models"))
            .args(["--linger", "30"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        c
    };
    let mut first = cmd().spawn().unwrap();
    let state = cache.join("modeld.json");
    let base = {
        let mut found = None;
        for _ in 0..500 {
            if let Some(b) = lore_llm::broker::find_at(&state).await {
                found = Some(b);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        found.expect("the broker started")
    };
    let st = lore_llm::broker::status(&base).await.unwrap();
    assert_eq!(st["pid"].as_u64(), first.id().map(u64::from));
    assert!(st["loaded"].is_null());
    let out = tokio::time::timeout(Duration::from_secs(10), cmd().output()).await.expect("second start returns").unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(&base), "says where the running one is");
    assert!(first.try_wait().unwrap().is_none(), "the first one still runs");
    // A model that is not downloaded: a clear 503, no llama-server started.
    let r = reqwest::Client::new().post(format!("{base}/v1/chat/completions")).json(&json!({ "model": "qwen3.5-4b-q8", "messages": [] })).send().await.unwrap();
    assert_eq!(r.status(), 503);
    let _ = std::process::Command::new("kill").arg(first.id().unwrap().to_string()).status();
    let _ = tokio::time::timeout(Duration::from_secs(5), first.wait()).await;
    assert!(!state.exists(), "SIGTERM: a clean exit forgets the state file");
}
