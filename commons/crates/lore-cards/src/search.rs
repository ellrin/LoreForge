//! The web-searched card summary (知識建立一定要上網查), L-Reader's 一鍵加卡 moved here so every
//! app makes cards the same way: the model must search the web and write only from what it
//! read; no search sources or `found: false` = no card.

use lore_llm::{LlmRequest, WebSource};
use serde_json::Value;

/// The shared task (`tasks.toml`): Claude with web search only.
pub const TASK: &str = "lore-card";

/// [`TASK`] for an app's task table.
pub fn task_table() -> lore_llm::TaskTable {
    lore_llm::TaskTable::parse(include_str!("../tasks.toml")).expect("lore-cards tasks.toml parses")
}

pub const CARD_SYSTEM: &str = "你替作者把論文裡的術語做成知識卡。一定要先用網路搜尋查這個術語（教科書、論文、官方文件、可靠的百科），\
只根據查到的內容寫，不要憑記憶補。查不到可靠的說明就回 found 為 false。\
只輸出一個 JSON 物件：{\"found\": true 或 false, \"summary\": \"台灣繁體中文一到兩句：這個詞是什麼、在這個領域拿來做什麼，術語保留英文\", \"sources\": [\"你實際讀過、支持這段摘要的網址，最多三個\"]}。";

/// The pages a card cites: the ones the model named that the search really read, else the
/// first three the search read.
pub fn card_sources(answer: &Value, found: &[WebSource]) -> Vec<WebSource> {
    let named: Vec<&str> = answer["sources"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let norm = |u: &str| u.trim().trim_end_matches('/').to_string();
    let picked: Vec<WebSource> = found.iter().filter(|w| named.iter().any(|n| norm(n) == norm(&w.url))).take(3).cloned().collect();
    if picked.is_empty() { found.iter().take(3).cloned().collect() } else { picked }
}

/// The author's field (`field` in `private/shared/profile.toml`), "" when not set.
pub fn field(home: &lore_home::Home) -> String {
    let Some(p) = home.shared_file("profile.toml") else { return String::new() };
    let Ok(t) = std::fs::read_to_string(p) else { return String::new() };
    toml::from_str::<toml::Value>(&t).ok().and_then(|v| v.get("field")?.as_str().map(|x| x.trim().to_string())).unwrap_or_default()
}

/// The app's way to run [`TASK`] (its keys, its engine settings).
pub trait Search: Send + Sync + 'static {
    fn route(&self) -> impl std::future::Future<Output = Result<lore_llm::Route, String>> + Send;
    /// Where the author's field is read.
    fn home(&self) -> lore_home::Home;
}

/// A searched summary: `(summary, the pages it cites, engine id)`. Only the term and the
/// author's field are sent (never the document's text: another person's file stays here).
pub async fn summary<H: Search>(host: &H, term: &str) -> Result<(String, Vec<WebSource>, String), crate::CardError> {
    let route = host.route().await.map_err(|e| crate::CardError::new(503, e))?;
    let mut user = format!("術語：{term}");
    let f = field(&host.home());
    if !f.is_empty() {
        user += &format!("\n作者的領域：{f}");
    }
    let req = route.apply(LlmRequest::prompt(CARD_SYSTEM, user).max_tokens(2000).web(true));
    let r = route.provider.send(&req).await.map_err(|e| crate::CardError::new(502, format!("沒有查好：{e}")))?;
    if r.sources.is_empty() {
        return Err(crate::CardError::new(422, format!("{} 沒有上網查資料，沒有建立卡片", route.provider.id())));
    }
    let v = lore_llm::json::extract_json(&r.text).ok_or_else(|| crate::CardError::new(502, format!("模型（{}）沒有回 JSON", route.provider.id())))?;
    let s = v["summary"].as_str().map(str::trim).unwrap_or_default().to_string();
    if v["found"] == false || s.is_empty() {
        return Err(crate::CardError::new(422, "網路上查不到可靠的說明，沒有建立卡片"));
    }
    Ok((s, card_sources(&v, &r.sources), route.provider.id().to_string()))
}
