//! Claude (Anthropic Messages API).
//!
//! The API key never touches a file: it comes from `ANTHROPIC_API_KEY` or the
//! OS keychain (macOS Keychain, Windows Credential Manager). The chosen model
//! name is a preference and lives in config.toml.

use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "PaperComposer";
const ACCOUNT: &str = "anthropic-api-key";
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
pub const MODELS: &[&str] = &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    Env,
    Keychain,
}

pub struct Claude {
    http: reqwest::Client,
    base: String,
    pub model: String,
}

#[derive(Serialize)]
pub struct ClaudeStatus {
    pub configured: bool,
    pub source: Option<KeySource>,
    /// The key with all but the last four characters hidden.
    pub masked: Option<String>,
    pub model: String,
    pub models: &'static [&'static str],
}

fn keychain() -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).ok()
}

/// Read the keychain at most once per process. Every keychain read can make
/// macOS ask for the login password, and status is polled by the UI.
static CACHED: std::sync::Mutex<Option<Option<(String, KeySource)>>> = std::sync::Mutex::new(None);

/// The key and where it came from. Env wins so a shell can override.
pub(crate) fn key() -> Option<(String, KeySource)> {
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        if !k.trim().is_empty() {
            return Some((k, KeySource::Env));
        }
    }
    let mut c = CACHED.lock().unwrap_or_else(|e| e.into_inner());
    c.get_or_insert_with(|| keychain()?.get_password().ok().map(|k| (k, KeySource::Keychain))).clone()
}

fn remember(k: Option<String>) {
    *CACHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(k.map(|k| (k, KeySource::Keychain)));
}

/// Store (or with an empty string, remove) the key in the OS keychain.
pub fn save_key(k: &str) -> Result<(), String> {
    let entry = keychain().ok_or("keychain unavailable on this system")?;
    let r = if k.trim().is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        // macOS ties an item to the binary that created it; a rebuilt or
        // updated binary may not see the old item and then fails to add a
        // "duplicate". Replace it: delete, then write again.
        entry.set_password(k.trim()).or_else(|first| {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err(stale_item_help(&first)),
            }
            entry.set_password(k.trim()).map_err(|e| stale_item_help(&e))
        })
    };
    if r.is_ok() {
        remember(Some(k.trim()).filter(|k| !k.is_empty()).map(String::from));
    }
    r
}

/// `sk-ant-••••••••3f9a`: enough to tell keys apart, nothing more.
fn mask(k: &str) -> String {
    let k = k.trim();
    let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let head = if k.starts_with("sk-ant-") && k.chars().count() > 15 { "sk-ant-" } else { "" };
    format!("{head}••••••••{tail}")
}

fn stale_item_help(e: &keyring::Error) -> String {
    if cfg!(target_os = "macos") {
        format!("{e}。請打開「鑰匙圈存取」，搜尋 PaperComposer，刪除那一筆後再儲存一次")
    } else {
        e.to_string()
    }
}

/// Where API keys are created. There is no OAuth for third-party apps, so the
/// best we can do is open this page and let the user paste the key back.
pub const KEYS_PAGE: &str = "https://console.anthropic.com/settings/keys";

/// Open a URL in the system browser. Callers pass fixed URLs only (see
/// `PAGES` in lib.rs): the local API must not open arbitrary links.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = crate::proc::std_cmd("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = crate::proc::std_cmd("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = crate::proc::std_cmd("xdg-open");
    cmd.arg(url).spawn().map(|_| ()).map_err(|e| format!("無法開啟瀏覽器：{e}"))
}

#[derive(Serialize)]
struct Req<'a> {
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    system: &'a str,
    messages: [Msg<'a>; 1],
}
#[derive(Serialize)]
struct Msg<'a> {
    role: &'a str,
    content: &'a str,
}
#[derive(Deserialize)]
struct Resp {
    content: Vec<Block>,
}
#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}
#[derive(Deserialize)]
struct ApiError {
    error: ApiErrorBody,
}
#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

impl Claude {
    pub fn new(model: Option<String>) -> Claude {
        Claude {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("http client"),
            // Overridable for testing against a local stub.
            base: std::env::var("PAPERCOMPOSER_CLAUDE_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into()),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        }
    }

    pub fn status(&self) -> ClaudeStatus {
        let k = key();
        ClaudeStatus {
            configured: k.is_some(),
            masked: k.as_ref().map(|(k, _)| mask(k)),
            source: k.map(|(_, s)| s),
            model: self.model.clone(),
            models: MODELS,
        }
    }

    pub async fn complete(&self, prompt: &Prompt) -> Result<String, String> {
        let (k, _) = key().ok_or("Claude 尚未設定 API key")?;
        let body = Req {
            model: &self.model,
            max_tokens: 4096,
            temperature: 0.2,
            system: &prompt.system,
            messages: [Msg { role: "user", content: &prompt.user }],
        };
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base.trim_end_matches('/')))
            .header("x-api-key", k)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("無法連線到 Claude：{e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let msg = resp.json::<ApiError>().await.map(|e| e.error.message).unwrap_or_default();
            return Err(format!("Claude 回傳 {status}：{msg}"));
        }
        let r: Resp = resp.json().await.map_err(|e| format!("Claude 回覆無法解析：{e}"))?;
        Ok(r.content.into_iter().filter(|b| b.kind == "text").map(|b| b.text).collect::<Vec<_>>().join(""))
    }
}

#[cfg(test)]
mod tests {
    use super::mask;

    #[test]
    fn mask_shows_only_last_four() {
        assert_eq!(mask("sk-ant-api03-abcdefghijklmnop3f9a"), "sk-ant-••••••••3f9a");
        assert_eq!(mask(" short "), "••••••••hort");
        assert!(!mask("sk-ant-api03-SECRETSECRET1234").contains("SECRET"));
    }
}
