//! The Claude model list from the API (`GET /v1/models`), for new models that are not in
//! `catalog.json` yet. Needs an API key: the subscription (Claude Code) cannot list models.
//!
//! - [`ClaudeLister`] / [`HttpLister`]: one call per page, same base URL and headers as
//!   [`crate::claude_api`]; the key is a parameter and never stored or logged here.
//! - [`after`]: the last listing as [`Stored`] kept in the cache folder (`claude-models.json`,
//!   wipeable runtime data) next to the Google one. `new` = ids that were not in the previous
//!   listing (all of them on the first listing).
//! - Model ids are never hard-coded; the catalog stays the base list.
//!
//! Tested against a mock server on 127.0.0.1 only.

use crate::agy::{ModelInfo, Stored};
use crate::claude_api::{API_VERSION, DEFAULT_BASE_URL};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub const FILE: &str = "claude-models.json";
/// Pages followed at most (a safety stop, 1000 models per page).
const MAX_PAGES: usize = 10;

/// `<cache>/LoreForge/claude-models.json` (`LOREFORGE_CACHE` moves it).
pub fn default_path() -> Option<PathBuf> {
    lore_home::ports::cache_dir().map(|d| d.join(FILE))
}

/// Lists the models an API key can use (replaced by a fake in tests).
#[async_trait]
pub trait ClaudeLister: Send + Sync {
    async fn list(&self, api_key: &str) -> Result<Vec<ModelInfo>, String>;
}

/// The real endpoint.
#[derive(Debug, Clone)]
pub struct HttpLister {
    pub base_url: String,
}

impl Default for HttpLister {
    fn default() -> Self {
        HttpLister { base_url: DEFAULT_BASE_URL.into() }
    }
}

#[async_trait]
impl ClaudeLister for HttpLister {
    async fn list(&self, api_key: &str) -> Result<Vec<ModelInfo>, String> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
        let url = format!("{}/v1/models", self.base_url.trim_end_matches('/'));
        let mut out: Vec<ModelInfo> = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut q = vec![("limit", "1000".to_string())];
            if let Some(a) = &after {
                q.push(("after_id", a.clone()));
            }
            let resp = http
                .get(&url)
                .query(&q)
                .header("x-api-key", api_key)
                .header("anthropic-version", API_VERSION)
                .send()
                .await
                .map_err(|e| format!("Claude 模型清單連線失敗：{}", e.without_url()))?;
            let status = resp.status();
            let v: Value = resp.json().await.unwrap_or_default();
            if !status.is_success() {
                let m = v["error"]["message"].as_str().unwrap_or("").chars().take(200).collect::<String>();
                return Err(if m.is_empty() { format!("Claude 模型清單失敗（{}）", status.as_u16()) } else { format!("Claude 模型清單失敗（{}）：{m}", status.as_u16()) });
            }
            for m in v["data"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let Some(id) = m["id"].as_str().map(str::trim).filter(|i| !i.is_empty()) else { continue };
                if out.iter().any(|o| o.id == id) {
                    continue;
                }
                let label = m["display_name"].as_str().map(str::trim).filter(|l| !l.is_empty()).unwrap_or(id);
                out.push(ModelInfo { id: id.into(), label: label.into() });
            }
            match (v["has_more"].as_bool().unwrap_or(false), v["last_id"].as_str()) {
                (true, Some(last)) => after = Some(last.to_string()),
                _ => break,
            }
        }
        if out.is_empty() {
            return Err("Claude 沒有列出模型".into());
        }
        Ok(out)
    }
}

/// The listing after `fresh` came back: `new` = ids absent from `previous` (every id on the
/// first listing, since the catalog decides what is shown as an addition).
pub fn after(previous: Option<&Stored>, fresh: Vec<ModelInfo>, at: u64) -> Stored {
    let new = fresh.iter().filter(|m| previous.is_none_or(|p| !p.models.iter().any(|o| o.id == m.id))).map(|m| m.id.clone()).collect();
    Stored { checked: at, models: fresh, new }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Query, http::HeaderMap, http::StatusCode, routing::get};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    async fn serve(app: Router) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn pages_are_followed_with_the_api_headers() {
        let seen: Arc<Mutex<Vec<(Option<String>, String, String)>>> = Default::default();
        let s = seen.clone();
        let app = Router::new().route(
            "/v1/models",
            get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| {
                let s = s.clone();
                async move {
                    let hv = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                    s.lock().unwrap().push((q.get("after_id").cloned(), hv("x-api-key"), hv("anthropic-version")));
                    if q.get("after_id").is_none() {
                        Json(json!({ "data": [{ "id": "m-a", "display_name": "Model A" }, { "id": "m-b" }], "has_more": true, "last_id": "m-b" }))
                    } else {
                        Json(json!({ "data": [{ "id": "m-b" }, { "id": "m-c", "display_name": "Model C" }], "has_more": false, "last_id": "m-c" }))
                    }
                }
            }),
        );
        let l = HttpLister { base_url: serve(app).await };
        let got = l.list("test-key").await.unwrap();
        let ids: Vec<(&str, &str)> = got.iter().map(|m| (m.id.as_str(), m.label.as_str())).collect();
        assert_eq!(ids, [("m-a", "Model A"), ("m-b", "m-b"), ("m-c", "Model C")]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].0.as_deref(), Some("m-b"));
        assert!(seen.iter().all(|(_, k, v)| k == "test-key" && v == API_VERSION));
    }

    #[tokio::test]
    async fn a_refused_listing_is_an_error_without_the_key() {
        let app = Router::new().route("/v1/models", get(|| async { (StatusCode::UNAUTHORIZED, Json(json!({ "error": { "message": "invalid x-api-key" } }))) }));
        let l = HttpLister { base_url: serve(app).await };
        let e = l.list("sk-ant-secret-value").await.unwrap_err();
        assert!(e.contains("401") && e.contains("invalid x-api-key"), "{e}");
        assert!(!e.contains("secret"), "{e}");
    }

    #[test]
    fn new_means_absent_from_the_previous_listing() {
        let m = |id: &str| ModelInfo { id: id.into(), label: id.into() };
        let first = after(None, vec![m("a"), m("b")], 1);
        assert_eq!(first.new, ["a", "b"]);
        let second = after(Some(&first), vec![m("a"), m("b"), m("c")], 2);
        assert_eq!(second.new, ["c"]);
        assert!(after(Some(&second), vec![m("a"), m("c")], 3).new.is_empty());
    }
}
