//! Claude through the Anthropic Messages API (API key).
//!
//! Current API conventions (JELearner decisions §3, design §9):
//! - `anthropic-version: 2026-10-21`; no `temperature` / `top_p` (rejected by
//!   the current API): `LlmRequest.temperature` / `top_p` are left out here.
//! - Structured output via `output_config.format` (JSON Schema), effort via
//!   `output_config.effort`.
//! - `fallbacks: "default"` with beta `server-side-fallback-2026-07-01`, except
//!   for Haiku (no server-side fallback there).
//! - `stop_reason` is checked (`refusal`, `max_tokens`, `pause_turn`) before
//!   the content is read.
//! - Web search is the server tool `web_search_20260209`; a `pause_turn`
//!   (server loop limit) is resumed by sending the paused assistant turn back.
//! - Images are `image` blocks with a base64 source.
//! - Streaming is SSE (`stream: true`); the events are folded back into the
//!   same message shape, so both paths share one reading of the reply.
//!
//! Tested against mock servers only.

use crate::error::LlmError;
use crate::provider::{LlmProvider, OnEvent, check_request};
use crate::sse::SseParser;
use crate::types::{Capabilities, LlmRequest, LlmResponse, Message, Part, StreamEvent, Usage, WebSource};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

pub const API_VERSION: &str = "2026-10-21";
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const WEB_SEARCH_TOOL: &str = "web_search_20260209";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const NAME: &str = "Claude";

#[derive(Debug, Clone)]
pub struct ClaudeApiConfig {
    pub base_url: String,
    /// A model id from the catalog (`cloud.models`), never written in code.
    pub model: String,
    /// `low` | `medium` | `high` | `xhigh` | `max`; `None` = model default.
    pub effort: Option<String>,
    /// Ask the API to re-run a declined request on the recommended fallback model.
    pub fallbacks: bool,
    /// Web searches allowed per request.
    pub web_max_uses: u32,
    /// Context window (from the catalog).
    pub context: u32,
    /// How many times a `pause_turn` is resumed before giving up.
    pub max_continuations: u32,
    pub timeout: Duration,
}

impl ClaudeApiConfig {
    pub fn new(model: impl Into<String>) -> ClaudeApiConfig {
        ClaudeApiConfig {
            base_url: DEFAULT_BASE_URL.into(),
            model: model.into(),
            effort: None,
            fallbacks: true,
            web_max_uses: 5,
            context: 1_000_000,
            max_continuations: 4,
            timeout: Duration::from_secs(600),
        }
    }
}

/// Server-side refusal fallbacks exist for the Opus, Sonnet and Fable lines;
/// Haiku has none (the request would be rejected).
pub fn supports_fallbacks(model: &str) -> bool {
    !model.starts_with("claude-haiku")
}

pub struct ClaudeApi {
    http: reqwest::Client,
    cfg: ClaudeApiConfig,
    api_key: String,
}

fn content_json(m: &Message) -> Value {
    let blocks: Vec<Value> = m
        .content
        .iter()
        .map(|p| match p {
            Part::Text { text } => json!({ "type": "text", "text": text }),
            Part::Image(img) => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": img.media_type, "data": img.data }
            }),
        })
        .collect();
    json!({ "role": m.role, "content": blocks })
}

impl ClaudeApi {
    pub fn new(cfg: ClaudeApiConfig, api_key: impl Into<String>) -> ClaudeApi {
        let http = reqwest::Client::builder().timeout(cfg.timeout).build().expect("http client");
        ClaudeApi { http, cfg, api_key: api_key.into() }
    }

    pub fn config(&self) -> &ClaudeApiConfig {
        &self.cfg
    }

    fn fallbacks_on(&self) -> bool {
        self.cfg.fallbacks && supports_fallbacks(&self.cfg.model)
    }

    /// The request body; public so tests (and apps) can check its shape.
    pub fn body(&self, req: &LlmRequest) -> Value {
        let mut body = json!({
            "model": self.cfg.model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": req.messages.iter().map(content_json).collect::<Vec<_>>(),
        });
        let mut output = serde_json::Map::new();
        if let Some(e) = req.effort.as_ref().or(self.cfg.effort.as_ref()) {
            output.insert("effort".into(), json!(e));
        }
        if let Some(schema) = &req.json_schema {
            output.insert("format".into(), json!({ "type": "json_schema", "schema": schema }));
        }
        if !output.is_empty() {
            body["output_config"] = Value::Object(output);
        }
        if req.web_search {
            body["tools"] = json!([{ "type": WEB_SEARCH_TOOL, "name": "web_search", "max_uses": self.cfg.web_max_uses }]);
        }
        if self.fallbacks_on() {
            body["fallbacks"] = json!("default");
        }
        body
    }

    async fn post(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let mut rb = self
            .http
            .post(format!("{}/v1/messages", self.cfg.base_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(body);
        if self.fallbacks_on() {
            rb = rb.header("anthropic-beta", FALLBACK_BETA);
        }
        let resp = rb.send().await.map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let v: Value = resp.json().await.unwrap_or_default();
        let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
        Err(LlmError::Api(NAME.into(), status.as_u16(), msg))
    }

    /// Shared loop for both paths: send, resume `pause_turn`, read the reply.
    async fn run(&self, req: &LlmRequest, on: Option<OnEvent<'_>>) -> Result<LlmResponse, LlmError> {
        check_request(NAME, &self.capabilities(), req)?;
        let mut body = self.body(req);
        let mut acc = Collected::default();
        for _ in 0..=self.cfg.max_continuations {
            let msg = match on {
                None => {
                    let resp = self.post(&body).await?;
                    resp.json::<Value>().await.map_err(|e| LlmError::Parse(NAME.into(), e.to_string()))?
                }
                Some(on) => {
                    body["stream"] = json!(true);
                    let resp = self.post(&body).await?;
                    read_stream(resp, on).await?
                }
            };
            acc.add(&msg);
            match msg["stop_reason"].as_str() {
                Some("refusal") => {
                    let why = msg["stop_details"]["explanation"]
                        .as_str()
                        .or(msg["stop_details"]["category"].as_str())
                        .unwrap_or("未說明原因")
                        .to_string();
                    return Err(LlmError::Refused(NAME.into(), why));
                }
                Some("max_tokens") => return Err(LlmError::Truncated(NAME.into())),
                Some("pause_turn") => {
                    // The server's tool loop paused: send its turn back as is and it resumes.
                    let turn = json!({ "role": "assistant", "content": msg["content"] });
                    body["messages"].as_array_mut().expect("messages").push(turn);
                    continue;
                }
                _ => {}
            }
            if acc.text.trim().is_empty() {
                return Err(LlmError::Parse(NAME.into(), "沒有文字內容".into()));
            }
            return Ok(LlmResponse { text: acc.text, provider: self.id(), sources: acc.sources, usage: Some(acc.usage) });
        }
        Err(LlmError::Api(NAME.into(), 0, "網路搜尋暫停太多次，沒有完成".into()))
    }
}

/// Text, sources and usage gathered over one or more (resumed) responses.
#[derive(Default)]
struct Collected {
    text: String,
    sources: Vec<WebSource>,
    usage: Usage,
}

impl Collected {
    fn add(&mut self, msg: &Value) {
        for b in msg["content"].as_array().into_iter().flatten() {
            match b["type"].as_str() {
                Some("text") => self.text.push_str(b["text"].as_str().unwrap_or("")),
                Some("web_search_tool_result") => {
                    // A list on success; an error object otherwise (not raised).
                    for r in b["content"].as_array().into_iter().flatten() {
                        if let Some(url) = r["url"].as_str()
                            && !self.sources.iter().any(|s| s.url == url)
                        {
                            self.sources.push(WebSource { url: url.into(), title: r["title"].as_str().unwrap_or("").into() });
                        }
                    }
                }
                _ => {}
            }
        }
        self.usage.input_tokens += msg["usage"]["input_tokens"].as_u64().unwrap_or(0);
        self.usage.output_tokens += msg["usage"]["output_tokens"].as_u64().unwrap_or(0);
    }
}

/// Fold an SSE stream back into a message (`content`, `stop_reason`,
/// `stop_details`, `usage`), reporting text and searches as they come.
async fn read_stream(resp: reqwest::Response, on: OnEvent<'_>) -> Result<Value, LlmError> {
    let mut parser = SseParser::new();
    let mut acc = StreamAcc::default();
    let mut bytes = resp.bytes_stream();
    while let Some(chunk) = bytes.next().await {
        let chunk = chunk.map_err(|e| LlmError::Network(NAME.into(), e.to_string()))?;
        for ev in parser.push(&chunk) {
            acc.event(&ev.data, on)?;
        }
    }
    for ev in parser.finish() {
        acc.event(&ev.data, on)?;
    }
    Ok(acc.message())
}

#[derive(Default)]
struct StreamAcc {
    blocks: Vec<Value>,
    partial_json: HashMap<usize, String>,
    stop_reason: Value,
    stop_details: Value,
    input_tokens: u64,
    output_tokens: u64,
}

impl StreamAcc {
    fn event(&mut self, data: &str, on: OnEvent<'_>) -> Result<(), LlmError> {
        let Ok(v) = serde_json::from_str::<Value>(data) else { return Ok(()) };
        let idx = v["index"].as_u64().unwrap_or(0) as usize;
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                self.input_tokens += v["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
            }
            "content_block_start" => {
                if self.blocks.len() <= idx {
                    self.blocks.resize(idx + 1, Value::Null);
                }
                let block = v["content_block"].clone();
                if block["type"] == "web_search_tool_result" {
                    for r in block["content"].as_array().into_iter().flatten() {
                        if let Some(url) = r["url"].as_str() {
                            on(StreamEvent::Source(WebSource { url: url.into(), title: r["title"].as_str().unwrap_or("").into() }));
                        }
                    }
                }
                self.blocks[idx] = block;
            }
            "content_block_delta" => {
                let d = &v["delta"];
                let Some(block) = self.blocks.get_mut(idx) else { return Ok(()) };
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or("");
                        append(block, "text", t);
                        if !t.is_empty() {
                            on(StreamEvent::Text { text: t.to_string() });
                        }
                    }
                    "thinking_delta" => append(block, "thinking", d["thinking"].as_str().unwrap_or("")),
                    "signature_delta" => block["signature"] = d["signature"].clone(),
                    "input_json_delta" => {
                        self.partial_json.entry(idx).or_default().push_str(d["partial_json"].as_str().unwrap_or(""));
                    }
                    "citations_delta" => {
                        if !block["citations"].is_array() {
                            block["citations"] = json!([]);
                        }
                        block["citations"].as_array_mut().expect("array").push(d["citation"].clone());
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(raw) = self.partial_json.remove(&idx)
                    && let Some(block) = self.blocks.get_mut(idx)
                {
                    block["input"] = serde_json::from_str(&raw).unwrap_or(json!({}));
                    if block["type"] == "server_tool_use" && block["name"] == "web_search" {
                        on(StreamEvent::Searching { query: block["input"]["query"].as_str().unwrap_or("").into() });
                    }
                }
            }
            "message_delta" => {
                if !v["delta"]["stop_reason"].is_null() {
                    self.stop_reason = v["delta"]["stop_reason"].clone();
                }
                if !v["delta"]["stop_details"].is_null() {
                    self.stop_details = v["delta"]["stop_details"].clone();
                }
                self.output_tokens = v["usage"]["output_tokens"].as_u64().unwrap_or(self.output_tokens);
            }
            "error" => {
                return Err(LlmError::Api(NAME.into(), 0, v["error"]["message"].as_str().unwrap_or("串流中斷").into()));
            }
            _ => {}
        }
        Ok(())
    }

    fn message(self) -> Value {
        json!({
            "content": self.blocks.into_iter().filter(|b| !b.is_null()).collect::<Vec<_>>(),
            "stop_reason": self.stop_reason,
            "stop_details": self.stop_details,
            "usage": { "input_tokens": self.input_tokens, "output_tokens": self.output_tokens },
        })
    }
}

fn append(block: &mut Value, key: &str, s: &str) {
    let cur = block[key].as_str().unwrap_or("").to_string();
    block[key] = Value::String(cur + s);
}

#[async_trait]
impl LlmProvider for ClaudeApi {
    fn id(&self) -> String {
        format!("claude:{}", self.cfg.model)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { json: true, vision: true, web: true, streaming: true, context: self.cfg.context, needs_network: true }
    }

    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        self.run(req, None).await
    }

    async fn stream(&self, req: &LlmRequest, on: OnEvent<'_>) -> Result<LlmResponse, LlmError> {
        self.run(req, Some(on)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Image;
    use axum::{Json, Router, body::Body, http::HeaderMap, response::Response, routing::post};
    use std::sync::{Arc, Mutex};

    async fn serve(app: Router) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn claude(base: String, model: &str) -> ClaudeApi {
        let mut cfg = ClaudeApiConfig::new(model);
        cfg.base_url = base;
        cfg.effort = Some("medium".into());
        ClaudeApi::new(cfg, "test-key")
    }

    fn req() -> LlmRequest {
        LlmRequest::prompt("s", "u").max_tokens(100).json(json!({"type":"object"}))
    }

    #[test]
    fn body_follows_current_api() {
        let p = claude("http://x".into(), "claude-sonnet-5-5");
        let b = p.body(&req().temperature(0.2).top_p(0.9));
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["output_config"]["effort"], "medium");
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("temperature").is_none() && b.get("top_p").is_none(), "sampling params are never sent, even when the request sets them");
        assert!(b.get("tools").is_none());
        assert_eq!(b["messages"][0]["content"][0]["type"], "text");

        let r = LlmRequest::prompt("s", "what is this?").image(Image::from_bytes("image/png", b"x")).web(true).effort("low");
        let b = p.body(&r);
        assert_eq!(b["messages"][0]["content"][0]["type"], "image");
        assert_eq!(b["messages"][0]["content"][0]["source"]["type"], "base64");
        assert_eq!(b["messages"][0]["content"][0]["source"]["media_type"], "image/png");
        assert_eq!(b["tools"][0]["type"], WEB_SEARCH_TOOL);
        assert_eq!(b["output_config"]["effort"], "low", "per-request effort wins");

        let h = claude("http://x".into(), "claude-haiku-5-5").body(&req());
        assert!(h.get("fallbacks").is_none(), "no fallbacks for Haiku");
    }

    #[tokio::test]
    async fn reads_text_and_reports_refusal_and_truncation() {
        let app = Router::new().route(
            "/v1/messages",
            post(|headers: HeaderMap, Json(b): Json<Value>| async move {
                assert_eq!(headers["x-api-key"], "test-key");
                assert_eq!(headers["anthropic-version"], API_VERSION);
                assert_eq!(headers["anthropic-beta"], FALLBACK_BETA);
                match b["system"].as_str().unwrap() {
                    "refuse" => Json(json!({"content": [], "stop_reason": "refusal", "stop_details": {"type":"refusal","category":"cyber","explanation":"nope"}})),
                    "long" => Json(json!({"content": [{"type":"text","text":"{\"a\":"}], "stop_reason": "max_tokens"})),
                    _ => Json(json!({"content": [{"type":"thinking","thinking":""},{"type":"text","text":"{\"ok\":"},{"type":"text","text":"true}"}], "stop_reason": "end_turn", "usage": {"input_tokens": 5, "output_tokens": 3}})),
                }
            }),
        );
        let p = claude(serve(app).await, "claude-sonnet-5-5");
        let r = p.send(&req()).await.unwrap();
        assert_eq!(r.text, "{\"ok\":true}");
        assert_eq!(r.usage.unwrap().output_tokens, 3);
        let mut q = req();
        q.system = "refuse".into();
        assert!(matches!(p.send(&q).await, Err(LlmError::Refused(_, w)) if w == "nope"));
        q.system = "long".into();
        assert!(matches!(p.send(&q).await, Err(LlmError::Truncated(_))));
    }

    #[tokio::test]
    async fn api_errors_carry_status_and_message() {
        let app = Router::new().route(
            "/v1/messages",
            post(|| async { (axum::http::StatusCode::TOO_MANY_REQUESTS, Json(json!({"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}))) }),
        );
        let p = claude(serve(app).await, "claude-sonnet-5-5");
        let e = p.send(&req()).await.unwrap_err();
        assert!(matches!(&e, LlmError::Api(_, 429, m) if m == "slow down"), "{e}");
        assert!(e.is_transient());
    }

    #[tokio::test]
    async fn web_search_resumes_pause_turn_and_collects_sources() {
        let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
        let seen = calls.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move |Json(b): Json<Value>| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(b.clone());
                    if b["messages"].as_array().unwrap().len() == 1 {
                        Json(json!({"content": [
                            {"type":"server_tool_use","id":"srv_1","name":"web_search","input":{"query":"llama.cpp"}},
                            {"type":"web_search_tool_result","tool_use_id":"srv_1","content":[{"type":"web_search_result","url":"https://example.com/a","title":"A"}]},
                            {"type":"text","text":"Part one. "}
                        ], "stop_reason": "pause_turn"}))
                    } else {
                        Json(json!({"content": [{"type":"text","text":"Part two."}], "stop_reason": "end_turn"}))
                    }
                }
            }),
        );
        let p = claude(serve(app).await, "claude-sonnet-5-5");
        let r = p.send(&LlmRequest::prompt("s", "find it").web(true)).await.unwrap();
        assert_eq!(r.text, "Part one. Part two.");
        assert_eq!(r.sources, vec![WebSource { url: "https://example.com/a".into(), title: "A".into() }]);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1]["messages"][1]["role"], "assistant", "paused turn sent back");
        assert_eq!(calls[1]["messages"].as_array().unwrap().len(), 2, "no extra 'continue' user turn");
    }

    #[tokio::test]
    async fn streaming_reports_deltas_and_reads_the_same_reply() {
        let sse = [
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"usage\":{\"input_tokens\":7}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"s\",\"name\":\"web_search\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"rust\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"web_search_tool_result\",\"tool_use_id\":\"s\",\"content\":[{\"type\":\"web_search_result\",\"url\":\"https://example.com/r\",\"title\":\"R\"}]}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"text_delta\",\"text\":\"，世界\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":12}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ]
        .concat();
        let app = Router::new().route(
            "/v1/messages",
            post(move |Json(b): Json<Value>| {
                let sse = sse.clone();
                async move {
                    assert_eq!(b["stream"], true);
                    // Deliver in awkward 7-byte chunks.
                    let chunks: Vec<Result<Vec<u8>, std::io::Error>> = sse.as_bytes().chunks(7).map(|c| Ok(c.to_vec())).collect();
                    Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from_stream(futures_util::stream::iter(chunks)))
                        .unwrap()
                }
            }),
        );
        let p = claude(serve(app).await, "claude-sonnet-5-5");
        let events = Mutex::new(Vec::new());
        let on = |e: StreamEvent| events.lock().unwrap().push(e);
        let r = p.stream(&LlmRequest::prompt("s", "hi").web(true), &on).await.unwrap();
        assert_eq!(r.text, "你好，世界");
        assert_eq!(r.usage, Some(Usage { input_tokens: 7, output_tokens: 12 }));
        let ev = events.into_inner().unwrap();
        assert_eq!(ev[0], StreamEvent::Searching { query: "rust".into() });
        assert!(matches!(&ev[1], StreamEvent::Source(s) if s.url == "https://example.com/r"));
        assert_eq!(ev[2], StreamEvent::Text { text: "你好".into() });
        assert_eq!(ev[3], StreamEvent::Text { text: "，世界".into() });
    }

    #[tokio::test]
    async fn streamed_refusal_is_reported() {
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"category\":\"bio\"}}}\n\n";
        let app = Router::new().route("/v1/messages", post(move || async move { sse }));
        let p = claude(serve(app).await, "claude-sonnet-5-5");
        let r = p.stream(&LlmRequest::prompt("s", "x"), &|_| {}).await;
        assert!(matches!(r, Err(LlmError::Refused(_, w)) if w == "bio"));
    }
}
