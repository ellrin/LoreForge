//! The HTTP side: the OpenAI-compatible proxy and the broker's own `/api/*`.

use crate::sched::{Config, Scheduler};
use crate::upstream::Upstream;
use axum::body::{Body, Bytes};
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use lore_llm::broker::{APP_HEADER, PRIORITY_HEADER, Priority, SERVICE};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// `X-Lore-App` when a request has none.
pub const UNKNOWN_APP: &str = "unknown";

struct Inner {
    sched: Scheduler,
    /// Waiting handlers, woken when their request may start.
    wake: HashMap<u64, oneshot::Sender<()>>,
}

/// The broker: one scheduler in front of one [`Upstream`].
pub struct Broker {
    inner: Mutex<Inner>,
    upstream: Arc<dyn Upstream>,
    http: reqwest::Client,
    /// Last time a client talked to the broker or a model was in use.
    last_seen: Mutex<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A request's place in the scheduler; dropping it (finished, failed, client gone)
/// frees the place and lets the next one start.
pub struct Ticket {
    broker: Arc<Broker>,
    id: u64,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.broker.done(self.id);
    }
}

impl Broker {
    pub fn new(upstream: Arc<dyn Upstream>, config: Config) -> Arc<Broker> {
        Arc::new(Broker {
            inner: Mutex::new(Inner { sched: Scheduler::new(config), wake: HashMap::new() }),
            upstream,
            http: reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().expect("http client"),
            last_seen: Mutex::new(Instant::now()),
        })
    }

    pub fn upstream(&self) -> &Arc<dyn Upstream> {
        &self.upstream
    }

    /// A client talked to the broker, or the model is in use.
    pub fn touch(&self) {
        *lock(&self.last_seen) = Instant::now();
    }

    /// How long nobody talked to the broker and no model was in use.
    pub fn quiet_for(&self) -> Duration {
        lock(&self.last_seen).elapsed()
    }

    /// Nothing waits or runs.
    pub fn is_empty(&self) -> bool {
        let g = lock(&self.inner);
        g.sched.waiting().is_empty() && g.sched.running().is_empty()
    }

    pub fn set_focus(&self, app: Option<&str>) {
        lock(&self.inner).sched.set_focus(app);
        self.dispatch();
    }

    /// Start whatever may start now.
    fn dispatch(&self) {
        let mut g = lock(&self.inner);
        let now = Instant::now();
        while let Some(st) = g.sched.next(now) {
            if let Some(tx) = g.wake.remove(&st.id)
                && tx.send(()).is_ok()
            {
                continue;
            }
            g.sched.done(st.id); // its client is gone
        }
    }

    fn done(&self, id: u64) {
        {
            let mut g = lock(&self.inner);
            g.sched.done(id);
            g.wake.remove(&id);
        }
        self.touch();
        self.dispatch();
    }

    /// Queue a request and wait for its turn.
    pub async fn wait_turn(self: &Arc<Self>, app: &str, priority: Priority, model: &str) -> Ticket {
        // the model may have been released (idle, release button) since it was planned
        let actual = self.upstream.loaded().await;
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut g = lock(&self.inner);
            if g.sched.running().is_empty() {
                g.sched.set_loaded(actual.as_deref());
            }
            let id = g.sched.enqueue(app, priority, model, Instant::now());
            g.wake.insert(id, tx);
            id
        };
        let ticket = Ticket { broker: self.clone(), id };
        self.dispatch();
        let _ = rx.await;
        ticket
    }

    /// `GET /api/status`.
    pub async fn status(&self) -> Value {
        let loaded = self.upstream.loaded().await;
        let g = lock(&self.inner);
        let now = Instant::now();
        let item = |r: &crate::sched::Request| {
            json!({
                "app": r.app,
                "priority": r.priority,
                "effective": g.sched.effective(r, now),
                "model": r.model,
                "waiting_ms": now.saturating_duration_since(r.since).as_millis() as u64,
            })
        };
        json!({
            "service": SERVICE,
            "pid": std::process::id(),
            "loaded": loaded,
            "phase": self.upstream.phase(),
            "focused": g.sched.focused(),
            "queue": g.sched.order(now).into_iter().map(item).collect::<Vec<_>>(),
            "running": g.sched.running().len(),
            "running_apps": g.sched.running().iter().map(|r| json!({ "app": r.app, "priority": r.priority, "model": r.model })).collect::<Vec<_>>(),
            "slots": g.sched.config.slots,
        })
    }

    /// `POST /api/release`: stop the model now, unless a request runs.
    pub async fn release(&self) -> Result<(), String> {
        {
            let g = lock(&self.inner);
            if !g.sched.running().is_empty() {
                return Err("模型正在使用中，不能現在釋放".into());
            }
        }
        self.upstream.release().await;
        lock(&self.inner).sched.set_loaded(None);
        Ok(())
    }
}

fn openai_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": { "message": msg, "type": "lore_modeld" } }))).into_response()
}

async fn chat(State(b): State<Arc<Broker>>, headers: HeaderMap, body: Bytes) -> Response {
    b.touch();
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return openai_error(StatusCode::BAD_REQUEST, "請求不是 JSON");
    };
    let Some(model) = v["model"].as_str().map(str::trim).filter(|m| !m.is_empty()).map(str::to_string) else {
        return openai_error(StatusCode::BAD_REQUEST, "請求少了 model（模型目錄的 id）");
    };
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok());
    let app = h(APP_HEADER).map(str::trim).filter(|a| !a.is_empty()).unwrap_or(UNKNOWN_APP).to_string();
    let priority = Priority::from_header(h(PRIORITY_HEADER));

    let ticket = b.wait_turn(&app, priority, &model).await;
    let url = match b.upstream.ensure(&model).await {
        Ok(u) => u,
        Err(e) => return openai_error(StatusCode::SERVICE_UNAVAILABLE, &e),
    };
    let resp = match b
        .http
        .post(format!("{}/chat/completions", url.trim_end_matches('/')))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return openai_error(StatusCode::BAD_GATEWAY, &format!("llama-server 沒有回應：{e}")),
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/json").to_string();
    // Chunks pass through as they come (SSE included); the ticket lives as long as the
    // body, so the slot is freed when the reply ends or the client goes away.
    let stream = resp.bytes_stream().map(move |c| {
        let _held = &ticket;
        c.map_err(std::io::Error::other)
    });
    let mut out = Response::new(Body::from_stream(stream));
    *out.status_mut() = status;
    if let Ok(v) = ctype.parse() {
        out.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    if ctype.starts_with("text/event-stream") {
        out.headers_mut().insert(header::CACHE_CONTROL, "no-cache".parse().expect("header"));
    }
    out
}

async fn models(State(b): State<Arc<Broker>>) -> Json<Value> {
    b.touch();
    let loaded = b.upstream.loaded().await;
    let data: Vec<Value> = b
        .upstream
        .models()
        .into_iter()
        .map(|id| json!({ "id": id, "object": "model", "owned_by": SERVICE, "loaded": loaded.as_deref() == Some(id.as_str()) }))
        .collect();
    Json(json!({ "object": "list", "data": data }))
}

async fn status(State(b): State<Arc<Broker>>) -> Json<Value> {
    b.touch();
    Json(b.status().await)
}

async fn focus(State(b): State<Arc<Broker>>, Json(v): Json<Value>) -> Json<Value> {
    b.touch();
    b.set_focus(v["app"].as_str());
    Json(json!({ "focused": lock(&b.inner).sched.focused() }))
}

async fn release(State(b): State<Arc<Broker>>) -> Response {
    b.touch();
    match b.release().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "error": e }))).into_response(),
    }
}

/// All routes of the broker.
pub fn router(b: Arc<Broker>) -> axum::Router {
    axum::Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/v1/models", get(models))
        .route("/api/status", get(status))
        .route("/api/focus", post(focus))
        .route("/api/release", post(release))
        .with_state(b)
}
