//! API keys in the OS keychain: the [`lore_secrets`] crate, re-exported here so
//! `lore_llm::secrets::{KeyStore, Account, Who, KeySource, mask, KEYS_PAGE}`
//! keeps working. Apps that only store keys can depend on `lore-secrets`
//! directly (no HTTP client, no tokio).
//!
//! [`SharedKeys`]: one keychain service for every LoreForge app ([`SHARED_SERVICE`]),
//! still reading the key an app stored under its own service before.

pub use lore_secrets::*;

use std::path::PathBuf;

/// The keychain service every LoreForge app stores its Anthropic key under.
pub const SHARED_SERVICE: &str = "LoreForge";

/// The services the apps used before [`SHARED_SERVICE`] (read, never written or deleted).
pub const OLD_SERVICES: [&str; 5] = ["PaperComposer", "LoreReader", "KnowledgeBase", "SlideComposer", "JELearner"];

/// The Anthropic key under [`SHARED_SERVICE`], else under an app's old service.
/// Old services are read only when the shared one has nothing (each read may ask for
/// the keychain password on macOS), the app's own first. Saving writes the shared
/// service only; old items stay where they are.
#[derive(Debug, Clone)]
pub struct SharedKeys {
    pub main: KeyStore,
    pub old: Vec<KeyStore>,
}

impl SharedKeys {
    /// `app` = the app's old service name (read first among the old ones).
    pub fn for_app(app: &str, app_dir: Option<PathBuf>) -> SharedKeys {
        let mut names: Vec<&str> = vec![app];
        names.extend(OLD_SERVICES.iter().copied().filter(|s| *s != app));
        SharedKeys {
            main: KeyStore::anthropic(SHARED_SERVICE, app_dir),
            old: names.into_iter().filter(|s| *s != SHARED_SERVICE).map(|s| KeyStore::anthropic(s, None)).collect(),
        }
    }

    /// Never the real keychain or the environment (tests).
    pub fn isolate(&mut self) {
        for k in std::iter::once(&mut self.main).chain(self.old.iter_mut()) {
            k.use_keychain = false;
            k.env_var = None;
        }
    }

    /// The key and where it came from (an old service counts as [`KeySource::Legacy`]).
    pub fn get(&self, explicit: Option<&str>, account: &Account) -> Option<(String, KeySource)> {
        if let Some(k) = self.main.get(explicit, account) {
            return Some(k);
        }
        self.old.iter().find_map(|s| s.get(None, account)).map(|(k, _)| (k, KeySource::Legacy))
    }

    /// Store (empty = remove) under the shared service.
    pub fn save(&self, k: &str, account: &Account) -> Result<KeySource, String> {
        self.main.save(k, account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_service_first_then_old_ones_and_saving_only_the_shared_one() {
        let shared = tempfile::tempdir().unwrap();
        let old = tempfile::tempdir().unwrap();
        let mut keys = SharedKeys::for_app("LoreReader", Some(shared.path().to_path_buf()));
        keys.isolate();
        assert_eq!(keys.main.service, SHARED_SERVICE);
        assert_eq!(keys.old[0].service, "LoreReader", "the app's own old service is read first");
        assert_eq!(keys.old.len(), OLD_SERVICES.len());
        assert!(keys.get(None, &Account::Single).is_none());

        // a key the app stored before (file fallback stands in for the keychain here)
        keys.old[0].app_dir = Some(old.path().to_path_buf());
        keys.old[0].save("sk-ant-old-0000000000", &Account::Single).unwrap();
        assert_eq!(keys.get(None, &Account::Single).unwrap(), ("sk-ant-old-0000000000".into(), KeySource::Legacy));

        keys.save("sk-ant-new-0000000000", &Account::Single).unwrap();
        assert_eq!(keys.get(None, &Account::Single).unwrap().0, "sk-ant-new-0000000000");
        assert!(old.path().join("credentials.toml").exists(), "the old key is never deleted");
        keys.save("", &Account::Single).unwrap();
        assert_eq!(keys.get(None, &Account::Single).unwrap().1, KeySource::Legacy);
    }
}
