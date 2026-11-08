//! ym-secrets — API keys (Anthropic) in the OS keychain, never in a repository or a URL.
//! Merged from JELearner `secrets.rs` (per-user keys, file fallback) and
//! PaperComposer `claude.rs` (single key, stale-item repair).
//!
//! Lookup order for [`KeyStore::get`]:
//! 1. an explicit key (settings, tests);
//! 2. the environment variable (`ANTHROPIC_API_KEY`): a developer override
//!    for this process. The apps' `scripts/dev.sh` keeps the key with Apple's
//!    `security` tool under `<App>-dev` and passes it this way, because
//!    unsigned debug builds would make the Keychain prompt on every rebuild;
//! 3. the OS keychain (macOS Keychain / Windows Credential Manager / Linux
//!    keyutils): service = the app name, account = `anthropic-api-key` (one
//!    key per app) or `anthropic-api-key:<workspace>:<user>` (per user);
//! 4. a file with owner-only permissions when the keychain is unavailable:
//!    `<app dir>/credentials.toml` or `<app dir>/credentials/<workspace>/<user>.toml`;
//! 5. per user, for the workspace owner only: the app's single key (legacy).
//!
//! A user never falls back to another user's key. Keychain reads can prompt
//! on macOS, so each account is read at most once per process.
//!
//! Also available as `ym_llm::secrets` (ym-llm re-exports this crate).

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    Settings,
    Env,
    Keychain,
    File,
    /// The app's single key, read for the workspace owner.
    Legacy,
}

/// Whose key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    pub workspace: String,
    pub user: String,
    /// May read the app's single (first-version) key: the workspace owner.
    pub legacy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Account {
    /// One key for the whole app (PaperComposer).
    Single,
    /// One key per workspace user (JELearner).
    User(Who),
    /// No user chosen yet: only an explicit or environment key.
    Nobody,
}

#[derive(Debug, Clone)]
pub struct KeyStore {
    /// Keychain service, e.g. `PaperComposer`, `JELearner`.
    pub service: String,
    /// Off in tests (never touch the real keychain there).
    pub use_keychain: bool,
    /// App settings folder for the file fallback.
    pub app_dir: Option<PathBuf>,
    /// Developer override; `None` = none.
    pub env_var: Option<String>,
    /// Keychain account prefix.
    pub account: String,
    /// Key name inside credential files.
    pub file_key: String,
}

/// (service, account) → key, read at most once per process.
type KeyCache = HashMap<(String, String), Option<String>>;
static CACHED: Mutex<Option<KeyCache>> = Mutex::new(None);

impl KeyStore {
    /// The Anthropic API key of `service`.
    pub fn anthropic(service: impl Into<String>, app_dir: Option<PathBuf>) -> KeyStore {
        KeyStore {
            service: service.into(),
            use_keychain: true,
            app_dir,
            env_var: Some("ANTHROPIC_API_KEY".into()),
            account: "anthropic-api-key".into(),
            file_key: "anthropic_api_key".into(),
        }
    }

    fn account_name(&self, who: &Who) -> String {
        format!("{}:{}:{}", self.account, who.workspace, who.user)
    }

    fn keychain_get(&self, account: &str) -> Option<String> {
        if !self.use_keychain {
            return None;
        }
        let mut c = CACHED.lock().unwrap_or_else(|e| e.into_inner());
        c.get_or_insert_with(HashMap::new)
            .entry((self.service.clone(), account.to_string()))
            .or_insert_with(|| keyring::Entry::new(&self.service, account).ok()?.get_password().ok())
            .clone()
            .filter(|k| !k.trim().is_empty())
    }

    fn remember(&self, account: &str, k: Option<String>) {
        let mut c = CACHED.lock().unwrap_or_else(|e| e.into_inner());
        c.get_or_insert_with(HashMap::new).insert((self.service.clone(), account.to_string()), k);
    }

    fn file_for(&self, account: &Account) -> Option<PathBuf> {
        let d = self.app_dir.as_ref()?;
        match account {
            Account::Single => Some(d.join("credentials.toml")),
            Account::User(w) => Some(d.join("credentials").join(&w.workspace).join(format!("{}.toml", w.user))),
            Account::Nobody => None,
        }
    }

    fn read_file(&self, path: &Path) -> Option<String> {
        let t: toml::Table = std::fs::read_to_string(path).ok()?.parse().ok()?;
        t.get(&self.file_key)?.as_str().map(str::to_string).filter(|k| !k.trim().is_empty())
    }

    /// The key and where it came from.
    pub fn get(&self, explicit: Option<&str>, account: &Account) -> Option<(String, KeySource)> {
        if let Some(k) = explicit.filter(|k| !k.trim().is_empty()) {
            return Some((k.to_string(), KeySource::Settings));
        }
        if let Some(var) = &self.env_var
            && let Ok(k) = std::env::var(var)
            && !k.trim().is_empty()
        {
            return Some((k, KeySource::Env));
        }
        let name = match account {
            Account::Nobody => return None,
            Account::Single => self.account.clone(),
            Account::User(w) => self.account_name(w),
        };
        if let Some(k) = self.keychain_get(&name) {
            return Some((k, KeySource::Keychain));
        }
        if let Some(k) = self.file_for(account).and_then(|p| self.read_file(&p)) {
            return Some((k, KeySource::File));
        }
        if let Account::User(w) = account
            && w.legacy
        {
            if let Some(k) = self.keychain_get(&self.account) {
                return Some((k, KeySource::Legacy));
            }
            if let Some(k) = self.file_for(&Account::Single).and_then(|p| self.read_file(&p)) {
                return Some((k, KeySource::Legacy));
            }
        }
        None
    }

    /// Store a key (empty string removes it). Keychain first; the file only
    /// when the keychain is not usable on this system.
    pub fn save(&self, k: &str, account: &Account) -> Result<KeySource, String> {
        let k = k.trim();
        let name = match account {
            Account::Nobody => return Err("還沒有選擇使用者".into()),
            Account::Single => self.account.clone(),
            Account::User(w) => self.account_name(w),
        };
        let mut keychain_err = None;
        if self.use_keychain {
            match keychain_set(&self.service, &name, k) {
                Ok(()) => {
                    self.remember(&name, Some(k.to_string()).filter(|s| !s.is_empty()));
                    if k.is_empty()
                        && let Some(p) = self.file_for(account)
                    {
                        let _ = std::fs::remove_file(p);
                    }
                    return Ok(KeySource::Keychain);
                }
                Err(e) => keychain_err = Some(e),
            }
        }
        let path = self.file_for(account).ok_or_else(|| keychain_err.unwrap_or_else(|| "系統沒有設定資料夾，也無法使用金鑰圈".into()))?;
        if k.is_empty() {
            let _ = std::fs::remove_file(&path);
            return Ok(KeySource::File);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut t = toml::Table::new();
        t.insert(self.file_key.clone(), toml::Value::String(k.to_string()));
        std::fs::write(&path, toml::to_string(&t).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(KeySource::File)
    }
}

/// Write (or with an empty key, delete) a keychain item. macOS ties an item
/// to the binary that created it; a rebuilt binary may not see the old item
/// and then fails to add a "duplicate": delete it and write again.
fn keychain_set(service: &str, account: &str, k: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(service, account).map_err(|e| e.to_string())?;
    if k.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    entry.set_password(k).or_else(|first| {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err(stale_item_help(service, &first)),
        }
        entry.set_password(k).map_err(|e| stale_item_help(service, &e))
    })
}

fn stale_item_help(service: &str, e: &keyring::Error) -> String {
    if cfg!(target_os = "macos") {
        format!("{e}。請打開「鑰匙圈存取」，搜尋 {service}，刪除那一筆後再儲存一次")
    } else {
        e.to_string()
    }
}

/// `sk-ant-••••••••3f9a`: enough to tell keys apart, nothing more.
pub fn mask(k: &str) -> String {
    let k = k.trim();
    let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let head = if k.starts_with("sk-ant-") && k.chars().count() > 15 { "sk-ant-" } else { "" };
    format!("{head}••••••••{tail}")
}

/// Where API keys are created (no OAuth for third-party apps: the user
/// pastes the key back).
pub const KEYS_PAGE: &str = "https://console.anthropic.com/settings/keys";

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> KeyStore {
        let mut s = KeyStore::anthropic("ym-secrets-test", Some(dir.to_path_buf()));
        s.use_keychain = false; // never the real keychain in tests
        s.env_var = None; // independent of the developer's environment
        s
    }

    fn who(user: &str, legacy: bool) -> Account {
        Account::User(Who { workspace: "ws-1".into(), user: user.into(), legacy })
    }

    #[test]
    fn mask_shows_only_last_four() {
        assert_eq!(mask("sk-ant-api03-abcdefghijklmnop3f9a"), "sk-ant-••••••••3f9a");
        assert_eq!(mask(" short "), "••••••••hort");
        assert!(!mask("sk-ant-api03-SECRETSECRET1234").contains("SECRET"));
    }

    #[test]
    fn keys_are_per_user_with_owner_legacy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert_eq!(s.save("sk-ant-user-a-0000000000", &who("ellrin", false)).unwrap(), KeySource::File);
        assert_eq!(s.get(None, &who("ellrin", false)).unwrap().0, "sk-ant-user-a-0000000000");
        assert!(s.get(None, &who("daidai", false)).is_none(), "daidai never gets ellrin's key");
        assert!(s.get(None, &Account::Nobody).is_none());

        // The app's single key: only the owner may use it as a fallback.
        s.save("sk-ant-single-000000000", &Account::Single).unwrap();
        assert_eq!(s.get(None, &Account::Single).unwrap(), ("sk-ant-single-000000000".into(), KeySource::File));
        assert_eq!(s.get(None, &who("owner", true)).unwrap().1, KeySource::Legacy);
        assert!(s.get(None, &who("daidai", false)).is_none());

        s.save("", &who("ellrin", false)).unwrap();
        assert!(s.get(None, &who("ellrin", false)).is_none());
        assert_eq!(s.get(Some("explicit"), &Account::Nobody).unwrap().1, KeySource::Settings);
        assert!(s.save("x", &Account::Nobody).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn credential_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        store(dir.path()).save("sk-ant-x-0000000000", &Account::Single).unwrap();
        let mode = std::fs::metadata(dir.path().join("credentials.toml")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
