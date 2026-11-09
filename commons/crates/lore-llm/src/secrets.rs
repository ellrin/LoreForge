//! API keys in the OS keychain: the [`lore_secrets`] crate, re-exported here so
//! `lore_llm::secrets::{KeyStore, Account, Who, KeySource, mask, KEYS_PAGE}`
//! keeps working. Apps that only store keys can depend on `lore-secrets`
//! directly (no HTTP client, no tokio).

pub use lore_secrets::*;
