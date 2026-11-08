//! kb-core — the knowledge base (design §4).
//!
//! Files are the source of truth (one `nodes/<id>.json` per card, one
//! `progress/<member>/<id>.json` per member and card); the SQLite index ([`index`]) is
//! rebuilt from them at start and kept in step on every write.
//!
//! - [`node`]: card schema (§4.1), ULID ids ([`id`]).
//! - [`registry`]: `kinds.toml` (extensible kinds) + `apps.toml` (kinds / link types
//!   declared by apps); [`tags`]: `tags.json` (domains, tag hierarchy).
//! - [`progress`]: per-member learning state and the familiarity state machine (§4.2, §4.4).
//! - [`edges`]: `[[wikilink]]` mentions; explicit links live in the node.
//! - [`scan`]: term scanner (Aho-Corasick over titles + aliases).
//! - [`llm`]: card drafting / term extraction trait with a mock — the ym-llm
//!   integration point.
//! - [`kb`]: [`KnowledgeBase`], the façade apps use (create / edit / trash / recall /
//!   share / queries).

pub mod clock;
pub mod edges;
pub mod id;
pub mod index;
pub mod kb;
pub mod llm;
pub mod node;
pub mod progress;
pub mod registry;
pub mod scan;
pub mod tags;

pub use clock::{Clock, FixedClock, SystemClock};
pub use index::{Filter, GraphQuery, GraphView, NodeSummary, TimelineDay};
pub use kb::{KbOptions, KnowledgeBase, SpaceInfo};
pub use llm::{CardAssistant, DraftRequest, MockAssistant};
pub use node::{Node, NodeFields};
pub use progress::{Intervals, Outcome, Progress, State};
pub use registry::Registry;
pub use scan::{ScanOptions, ScanSpan};
pub use ym_store;

#[derive(Debug, thiserror::Error)]
pub enum KbError {
    #[error(transparent)]
    Store(#[from] ym_store::StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("index: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// Breaks the personal / household rules (design §3.2).
    #[error("not allowed: {0}")]
    Forbidden(String),
    /// The card changed since the caller read it.
    #[error("revision conflict: expected {expected}, file has {actual}")]
    Conflict { expected: u64, actual: u64 },
    /// The LLM assistant failed (ym-llm integration point).
    #[error("assistant: {0}")]
    Assistant(String),
}

impl KbError {
    /// Store errors keep their own category (not found / forbidden / invalid).
    pub fn category(&self) -> &'static str {
        use ym_store::StoreError as S;
        match self {
            KbError::NotFound(_) | KbError::Store(S::NotFound(_)) => "not_found",
            KbError::Forbidden(_) | KbError::Store(S::Forbidden(_)) => "forbidden",
            KbError::Invalid(_) | KbError::Store(S::Invalid(_)) | KbError::Store(S::Image(_)) | KbError::Json(_) | KbError::Toml(_) => "invalid",
            KbError::Conflict { .. } => "conflict",
            _ => "internal",
        }
    }
}

pub type Result<T> = std::result::Result<T, KbError>;
