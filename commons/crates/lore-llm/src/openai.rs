//! Any OpenAI-compatible `/v1/chat/completions` server: llama.cpp
//! `llama-server` (the local engine, design §9), LM Studio, Ollama.
//!
//! - JSON output is constrained with `response_format: json_schema`
//!   ([`crate::json::complete_json`] then validates it and retries once).
//! - Images go in as `image_url` data URLs (needs a model started with its
//!   `--mmproj`; the catalog says which have one).
//! - Streaming reads `data:` lines until `[DONE]`.
//! - `temperature` / `top_p` from the request are sent when set (Claude
//!   engines leave them out).
//! - `zh_tw`: Chinese replies are rewritten in Taiwan usage (local models
//!   write mainland vocabulary; PaperComposer `zhtw.rs`).
//!
//! Tested against mock servers only; real llama-server checks are `#[ignore]`
//! tests (**not tested on real hardware** from this crate).

use crate::error::LlmError;
use crate::provider::{LlmProvider, OnEvent, check_request};
use crate::sse::SseParser;
use crate::types::{Capabilities, LlmRequest, LlmResponse, Message, Part, StreamEvent, Usage};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    /// Base URL ending in `/v1`, e.g. `http://127.0.0.1:8080/v1`.
    pub url: String,
    /// Model name sent in the request (a catalog id for llama-server).
    pub model: String,
    /// Bearer token for hosted compatible servers; `None` for llama-server.
    pub api_key: Option<String>,
    /// The loaded model reads images (started with `--mmproj`).
    pub vision: bool,
    pub context: u32,
    /// Rewrite Chinese replies in Taiwan usage (needs the `zhtw` feature).
    pub zh_tw: bool,
    /// Shown in errors and in the provider id.
    pub label: String,
    /// The server is somewhere on the internet (not this machine / LAN).
    pub needs_network: bool,
    pub timeout: Duration,
    /// Extra request headers sent with every call (a proxy in front of the server);
    /// empty for a plain server.
    pub headers: Vec<(String, String)>,
}

impl OpenAiCompatConfig {
    /// A local llama-server.
    pub fn llama_server(url: impl Into<String>, model: impl Into<String>) -> OpenAiCompatConfig {
        OpenAiCompatConfig {
            url: url.into(),
            model: model.into(),
            api_key: None,
            vision: false,
            context: 16384,
            zh_tw: false,
            label: "llama-server".into(),
            needs_network: false,
            timeout: Duration::from_secs(600),
            headers: Vec::new(),
        }
    }
}

pub struct OpenAiCompat {
    http: reqwest::Client,
    cfg: OpenAiCompatConfig,
}

/// JELearner / PaperComposer names.
pub type LlamaServer = OpenAiCompat;
pub type LlamaServerConfig = OpenAiCompatConfig;

fn message_json(m: &Message) -> Value {
    if !m.has_images() {
        return json!({ "role": m.role, "content": m.text() });
    }
    let parts: Vec<Value> = m
        .content
        .iter()
        .map(|p| match p {
            Part::Text { text } => json!({ "type": "text", "text": text }),
            Part::Image(img) => json!({ "type": "image_url", "image_url": { "url": img.data_url() } }),
        })
        .collect();
    json!({ "role": m.role, "content": parts })
}

impl OpenAiCompat {
    pub fn new(cfg: OpenAiCompatConfig) -> OpenAiCompat {
        let http = reqwest::Client::builder().timeout(cfg.timeout).build().expect("http client");
        OpenAiCompat { http, cfg }
    }

    pub fn config(&self) -> &OpenAiCompatConfig {
        &self.cfg
    }

    /// The request body; public so tests can check its shape.
    pub fn body(&self, req: &LlmRequest) -> Value {
        let mut messages = vec![json!({ "role": "system", "content": req.system })];
        messages.extend(req.messages.iter().map(message_json));
        let mut body = json!({ "model": self.cfg.model, "max_tokens": req.max_tokens, "messages": messages });
        if let Some(schema) = &req.json_schema {
            body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "reply", "schema": schema } });
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(p) = req.top_p {
            body["top_p"] = json!(p);
        }
        body
    }

    /// The server answers `/models` (used to wait for a starting llama-server
    /// and for the status light).
    pub async fn ping(&self) -> bool {
        self.http
            .get(format!("{}/models", self.cfg.url.trim_end_matches('/')))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    async fn post(&self, body: &Value) -> Result<reqwest::Response, LlmError> {
        let mut rb = self.http.post(format!("{}/chat/completions", self.cfg.url.trim_end_matches('/'))).json(body);
        if let Some(k) = &self.cfg.api_key {
            rb = rb.bearer_auth(k);
        }
        for (k, v) in &self.cfg.headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        let resp = rb.send().await.map_err(|e| LlmError::Network(self.cfg.label.clone(), e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let v: Value = resp.json().await.unwrap_or_default();
        let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
        Err(LlmError::Api(self.cfg.label.clone(), status.as_u16(), msg))
    }

    fn polish(&self, text: String) -> String {
        #[cfg(feature = "zhtw")]
        if self.cfg.zh_tw {
            return crate::zhtw::to_taiwan(&text);
        }
        text
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompat {
    fn id(&self) -> String {
        format!("{}:{}", self.cfg.label, self.cfg.model)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            json: true,
            vision: self.cfg.vision,
            web: false,
            streaming: true,
            context: self.cfg.context,
            needs_network: self.cfg.needs_network,
        }
    }

    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let name = self.cfg.label.clone();
        check_request(&name, &self.capabilities(), req)?;
        let v: Value = self.post(&self.body(req)).await?.json().await.map_err(|e| LlmError::Parse(name.clone(), e.to_string()))?;
        if v["choices"][0]["finish_reason"] == "length" {
            return Err(LlmError::Truncated(name));
        }
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| LlmError::Parse(name.clone(), "沒有 choices[0].message.content".into()))?;
        let usage = v["usage"].is_object().then(|| Usage {
            input_tokens: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output_tokens: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        });
        Ok(LlmResponse { text: self.polish(text), provider: self.id(), sources: vec![], usage })
    }

    async fn stream(&self, req: &LlmRequest, on: OnEvent<'_>) -> Result<LlmResponse, LlmError> {
        let name = self.cfg.label.clone();
        check_request(&name, &self.capabilities(), req)?;
        let mut body = self.body(req);
        body["stream"] = json!(true);
        let resp = self.post(&body).await?;
        let mut parser = SseParser::new();
        let mut text = String::new();
        let mut finish = Value::Null;
        let mut handle = |data: &str, text: &mut String| {
            if data == "[DONE]" {
                return;
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            if let Some(d) = v["choices"][0]["delta"]["content"].as_str()
                && !d.is_empty()
            {
                text.push_str(d);
                // Chunk-wise conversion is approximate; the final text is converted whole.
                on(StreamEvent::Text { text: self.polish(d.to_string()) });
            }
            if !v["choices"][0]["finish_reason"].is_null() {
                finish = v["choices"][0]["finish_reason"].clone();
            }
        };
        let mut bytes = resp.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|e| LlmError::Network(name.clone(), e.to_string()))?;
            for ev in parser.push(&chunk) {
                handle(&ev.data, &mut text);
            }
        }
        for ev in parser.finish() {
            handle(&ev.data, &mut text);
        }
        if finish == "length" {
            return Err(LlmError::Truncated(name));
        }
        Ok(LlmResponse { text: self.polish(text), provider: self.id(), sources: vec![], usage: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Image;
    use axum::{Json, Router, routing::post};

    async fn serve(app: Router) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn chat_completions_with_json_schema() {
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                post(|Json(b): Json<Value>| async move {
                    assert_eq!(b["response_format"]["type"], "json_schema");
                    assert_eq!(b["messages"][0]["role"], "system");
                    assert!(b.get("temperature").is_none());
                    if b["max_tokens"] == 1 {
                        return Json(json!({"choices": [{"message": {"content": "{"}, "finish_reason": "length"}]}));
                    }
                    Json(json!({"choices": [{"message": {"content": "{\"a\":1}"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}))
                }),
            )
            .route("/v1/models", axum::routing::get(|| async { Json(json!({"data": []})) }));
        let base = serve(app).await;
        let p = OpenAiCompat::new(OpenAiCompatConfig::llama_server(format!("{base}/v1"), "gemma-4-12b-q8"));
        assert!(p.ping().await);
        let req = LlmRequest::prompt("s", "u").json(json!({"type":"object"}));
        let r = p.send(&req).await.unwrap();
        assert_eq!(r.text, "{\"a\":1}");
        assert_eq!(r.usage.unwrap().output_tokens, 2);
        assert_eq!(p.id(), "llama-server:gemma-4-12b-q8");
        assert!(matches!(p.send(&req.clone().max_tokens(1)).await, Err(LlmError::Truncated(_))));
    }

    #[tokio::test]
    async fn extra_headers_are_sent() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|h: axum::http::HeaderMap| async move {
                let v = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                Json(json!({"choices": [{"message": {"content": format!("{}|{}", v("x-lore-app"), v("x-lore-priority"))}, "finish_reason": "stop"}]}))
            }),
        );
        let base = serve(app).await;
        let mut cfg = OpenAiCompatConfig::llama_server(format!("{base}/v1"), "m");
        assert_eq!(OpenAiCompat::new(cfg.clone()).complete(&LlmRequest::prompt("s", "u")).await.unwrap(), "|", "none by default");
        cfg.headers = vec![("X-Lore-App".into(), "pc".into()), ("X-Lore-Priority".into(), "interactive".into())];
        assert_eq!(OpenAiCompat::new(cfg).complete(&LlmRequest::prompt("s", "u")).await.unwrap(), "pc|interactive");
    }

    #[test]
    fn images_become_data_urls_only_with_vision() {
        let mut cfg = OpenAiCompatConfig::llama_server("http://x/v1", "m");
        let req = LlmRequest::prompt("s", "what?").image(Image::from_bytes("image/png", b"hi"));
        let b = OpenAiCompat::new(cfg.clone()).body(&req);
        assert_eq!(b["messages"][1]["content"][0]["type"], "image_url");
        assert_eq!(b["messages"][1]["content"][0]["image_url"]["url"], "data:image/png;base64,aGk=");
        assert_eq!(b["messages"][1]["content"][1]["text"], "what?");
        // Text-only turns stay plain strings (widest server support).
        assert_eq!(OpenAiCompat::new(cfg.clone()).body(&LlmRequest::prompt("s", "u"))["messages"][1]["content"], "u");
        // Sampling params go to local engines when the request sets them.
        let b = OpenAiCompat::new(cfg.clone()).body(&LlmRequest::prompt("s", "u").temperature(0.2).top_p(0.9));
        assert!((b["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!((b["top_p"].as_f64().unwrap() - 0.9).abs() < 1e-6);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert!(matches!(rt.block_on(OpenAiCompat::new(cfg.clone()).send(&req)), Err(LlmError::Unsupported(..))));
        cfg.vision = true;
        assert!(OpenAiCompat::new(cfg).capabilities().vision);
    }

    #[tokio::test]
    async fn streams_deltas_until_done() {
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let app = Router::new().route("/v1/chat/completions", post(move || async move { sse }));
        let base = serve(app).await;
        let p = OpenAiCompat::new(OpenAiCompatConfig::llama_server(format!("{base}/v1"), "m"));
        let got = std::sync::Mutex::new(Vec::new());
        let r = p.stream(&LlmRequest::prompt("s", "u"), &|e| got.lock().unwrap().push(e)).await.unwrap();
        assert_eq!(r.text, "Hello");
        assert_eq!(got.into_inner().unwrap(), vec![StreamEvent::Text { text: "Hel".into() }, StreamEvent::Text { text: "lo".into() }]);
    }

    #[cfg(feature = "zhtw")]
    #[tokio::test]
    async fn zh_tw_rewrites_mainland_terms() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async { Json(json!({"choices": [{"message": {"content": "{\"reason\": \"引用的數據不足\"}"}, "finish_reason": "stop"}]})) }),
        );
        let base = serve(app).await;
        let mut cfg = OpenAiCompatConfig::llama_server(format!("{base}/v1"), "m");
        cfg.zh_tw = true;
        assert_eq!(OpenAiCompat::new(cfg).complete(&LlmRequest::prompt("s", "u")).await.unwrap(), "{\"reason\": \"引用的資料不足\"}");
    }
}
