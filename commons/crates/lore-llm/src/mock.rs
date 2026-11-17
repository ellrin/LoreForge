//! Canned replies for offline tests.

use crate::error::LlmError;
use crate::provider::{LlmProvider, OnEvent, check_request};
use crate::types::{Capabilities, LlmRequest, LlmResponse, StreamEvent};
use async_trait::async_trait;
use std::sync::Mutex;

/// Replies with queued answers (the last one forever) and records requests.
pub struct MockProvider {
    replies: Mutex<Vec<String>>,
    pub seen: Mutex<Vec<LlmRequest>>,
    pub caps: Capabilities,
    pub name: String,
    /// Pages "read" by a web-search request (none: the mock never searches).
    pub sources: Vec<crate::types::WebSource>,
}

impl MockProvider {
    pub fn new(replies: Vec<String>) -> MockProvider {
        MockProvider {
            replies: Mutex::new(replies),
            seen: Mutex::new(vec![]),
            caps: Capabilities { json: true, vision: true, web: true, streaming: true, context: 1_000_000, needs_network: false },
            name: "mock".into(),
            sources: vec![],
        }
    }

    /// Answer web-search requests as if these pages had been read.
    pub fn with_sources(mut self, urls: &[&str]) -> Self {
        self.sources = urls.iter().map(|u| crate::types::WebSource { url: u.to_string(), title: String::new() }).collect();
        self
    }

    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.caps = caps;
        self
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn next(&self) -> Result<String, LlmError> {
        let mut r = self.replies.lock().unwrap();
        match r.len() {
            0 => Err(LlmError::NotConfigured("mock 沒有回覆".into())),
            1 => Ok(r[0].clone()),
            _ => Ok(r.remove(0)),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn id(&self) -> String {
        self.name.clone()
    }

    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        check_request(&self.name, &self.caps, req)?;
        self.seen.lock().unwrap().push(req.clone());
        let sources = if req.web_search { self.sources.clone() } else { vec![] };
        Ok(LlmResponse { text: self.next()?, provider: self.id(), sources, usage: None })
    }

    /// Streams the reply in pieces of up to 8 characters.
    async fn stream(&self, req: &LlmRequest, on: OnEvent<'_>) -> Result<LlmResponse, LlmError> {
        let r = self.send(req).await?;
        let chars: Vec<char> = r.text.chars().collect();
        for piece in chars.chunks(8) {
            on(StreamEvent::Text { text: piece.iter().collect() });
        }
        Ok(r)
    }
}
