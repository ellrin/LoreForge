//! Provider errors, worded for the user (Traditional Chinese).

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("尚未設定：{0}")]
    NotConfigured(String),
    /// Could not reach the engine (offline, server down, timeout).
    #[error("無法連線到 {0}：{1}")]
    Network(String, String),
    #[error("{0} 回傳 {1}：{2}")]
    Api(String, u16, String),
    /// `stop_reason: refusal` (after any server-side fallback).
    #[error("{0} 拒絕了這個請求（{1}）")]
    Refused(String, String),
    /// `stop_reason: max_tokens` / `finish_reason: length`.
    #[error("{0} 的回覆被截斷（max_tokens）")]
    Truncated(String),
    #[error("{0} 的回覆無法解析：{1}")]
    Parse(String, String),
    /// The request asks for something this engine cannot do (images, web search).
    #[error("{0} 不支援{1}")]
    Unsupported(String, String),
}

/// The name JELearner used; kept so its code migrates with a `use` change.
pub type ProviderError = LlmError;

impl LlmError {
    /// Worth trying again later or on another engine (network trouble, 429, 5xx).
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::Network(..) => true,
            LlmError::Api(_, s, _) => *s == 429 || *s == 529 || *s >= 500,
            _ => false,
        }
    }
}
