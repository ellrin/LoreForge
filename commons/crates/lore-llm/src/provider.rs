//! The one trait every engine implements.

use crate::error::LlmError;
use crate::types::{Capabilities, LlmRequest, LlmResponse, StreamEvent};
use async_trait::async_trait;

/// Callback for streaming events. Apps forward these to SSE / a channel.
pub type OnEvent<'a> = &'a (dyn Fn(StreamEvent) + Send + Sync);

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Short engine id shown in the UI and stored with generated content,
    /// e.g. `claude:claude-sonnet-5-5`, `claude-subscription:…`, `llama-server:qwen3.5-9b-q8`.
    fn id(&self) -> String;

    fn capabilities(&self) -> Capabilities;

    /// One call (single prompt or whole transcript), the reply in one piece.
    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError>;

    /// The same call, reporting text as it arrives. The default answers in
    /// one piece (engines without real streaming).
    async fn stream(&self, req: &LlmRequest, on: OnEvent<'_>) -> Result<LlmResponse, LlmError> {
        let r = self.send(req).await?;
        for s in &r.sources {
            on(StreamEvent::Source(s.clone()));
        }
        on(StreamEvent::Text { text: r.text.clone() });
        Ok(r)
    }

    /// Just the reply text.
    async fn complete(&self, req: &LlmRequest) -> Result<String, LlmError> {
        Ok(self.send(req).await?.text)
    }
}

/// Reject requests the engine cannot serve before spending a call on them.
pub fn check_request(name: &str, caps: &Capabilities, req: &LlmRequest) -> Result<(), LlmError> {
    if req.has_images() && !caps.vision {
        return Err(LlmError::Unsupported(name.into(), "看圖".into()));
    }
    if req.web_search && !caps.web {
        return Err(LlmError::Unsupported(name.into(), "網路搜尋".into()));
    }
    Ok(())
}

#[async_trait]
impl<T: LlmProvider + ?Sized> LlmProvider for std::sync::Arc<T> {
    fn id(&self) -> String {
        (**self).id()
    }
    fn capabilities(&self) -> Capabilities {
        (**self).capabilities()
    }
    async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        (**self).send(req).await
    }
    async fn stream(&self, req: &LlmRequest, on: OnEvent<'_>) -> Result<LlmResponse, LlmError> {
        (**self).stream(req, on).await
    }
}
