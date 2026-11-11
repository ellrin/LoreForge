//! lore-llm — the single LLM layer for every app (design §2.2, §9).
//!
//! - [`LlmProvider`]: one trait for single-shot structured JSON, streaming
//!   chat, multi-turn transcripts and image inputs.
//! - Engines: [`ClaudeApi`] (API key; streaming, web search, vision),
//!   [`ClaudeCode`] (the user's subscription through the Claude Code CLI),
//!   [`OpenAiCompat`] / [`LlamaServer`] (llama-server and other
//!   OpenAI-compatible servers), [`MockProvider`].
//! - [`catalog`]: the shared model catalog; [`local`]: the shared
//!   `LOCAL_LLM_HOME` folder and llama-server's arguments.
//! - [`shared_runtime`]: one llama-server per machine shared by every app in-process
//!   (registry in the cache folder under `flock`, one model at a time, priorities,
//!   idle release, the status words every app shows; docs/local-models.md).
//! - [`shared`]: the engine choices all apps share (task classes, routes, the Claude
//!   connection) and the one-click alternative after a fixed route fails.
//! - [`router`]: task routing from `tasks.toml` (needs → model, fallback,
//!   offline), with per-task user overrides.
//! - [`json`]: extract / validate / retry structured replies.
//! - [`secrets`]: API keys in the OS keychain (the `lore-secrets` crate, re-exported).
//! - [`zhtw`] (feature `zhtw`): Taiwan usage for local-model replies.
//!
//! Upper layers never name an engine or a model: they ask the router for a
//! task's provider. Tests never call real Claude, the real CLI or a real
//! llama-server (mock HTTP servers and a fake CLI instead).

pub mod catalog;
pub mod claude_api;
pub mod claude_code;
pub mod error;
pub mod json;
pub mod local;
pub mod mock;
pub mod openai;
pub mod provider;
pub mod router;
pub mod secrets;
pub mod shared;
pub mod shared_runtime;
pub mod sse;
pub mod types;
#[cfg(feature = "zhtw")]
pub mod zhtw;

pub use catalog::Catalog;
pub use claude_api::{ClaudeApi, ClaudeApiConfig};
pub use claude_code::ClaudeCode;
pub use error::{LlmError, ProviderError};
pub use json::{complete_as, complete_json};
pub use mock::MockProvider;
pub use openai::{LlamaServer, LlamaServerConfig, OpenAiCompat, OpenAiCompatConfig};
pub use provider::{LlmProvider, OnEvent};
pub use router::{ModelRef, Plan, ProviderFactory, Route, RouteError, RouteOptions, Router, StandardFactory, TaskSpec, TaskTable};
pub use shared::{Alternative, EngineKind, EngineSettings, RouteMode, TaskClass};
pub use types::{Capabilities, Image, LlmRequest, LlmResponse, Message, Needs, Part, Role, StreamEvent, Usage, WebSource};
