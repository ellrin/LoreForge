//! API keys in the OS keychain: the [`ym_secrets`] crate, re-exported here so
//! `ym_llm::secrets::{KeyStore, Account, Who, KeySource, mask, KEYS_PAGE}`
//! keeps working. Apps that only store keys can depend on `ym-secrets`
//! directly (no HTTP client, no tokio).

pub use ym_secrets::*;
