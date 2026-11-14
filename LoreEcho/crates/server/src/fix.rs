//! 修正知識卡 (author, 2026-11-14): the author says one of his KnowledgeBase cards is wrong; Claude
//! Haiku (task `le-fix`, with web search) checks it against sources and proposes a corrected
//! version; the page shows old and new side by side with the sources; only 套用 writes, through
//! KnowledgeBase's own `PUT /api/nodes/{id}` with the card's revision (a card changed meanwhile
//! is never overwritten). Nothing is written while KnowledgeBase is closed — no file fallback.
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/actions/fix/draft | `{ messages, card, request? }` → `{ card: {id, revision, title, summary, body}, fix: {title, summary, body}, changes: [{what, why, url}], sources: [{url, title}], label }` |
//! | POST | /api/actions/fix/source | `{ url }` → 204: a source page in the browser (http(s) only) |
//! | POST | /api/actions/fix/save | `{ id, revision, title, summary, body, sources }` → `{ node }`; 409 when the card changed since the draft |
//!
//! What goes to Claude: this one card (title, aliases, summary, body) and what the author said
//! about it — he asked for it; the conversation's other turns stay here.

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use lore_llm::{LlmRequest, Message};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;
use crate::chat::{Turn, clip};

pub const TASK_FIX: &str = "le-fix";
/// What the author said about the card (characters, his newest turns).
const REQUEST_CHARS: usize = 1500;
const SOURCES: usize = 6;

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/actions/fix/draft", post(draft)).route("/api/actions/fix/source", post(source)).route("/api/actions/fix/save", post(save))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).no_proxy().build().unwrap_or_default()
}

fn bad(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

const KB_CLOSED: &str = "KnowledgeBase 沒開：打開它再修正這張卡";

/// The card as KnowledgeBase has it now (`GET /api/nodes/{id}` → its `node`).
async fn node(url: &str, id: &str) -> Result<Value, Response> {
    let r = client().get(format!("{url}/api/nodes/{id}")).send().await.map_err(|e| bad(StatusCode::BAD_GATEWAY, format!("KnowledgeBase 沒有回應：{e}")))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(bad(if status == StatusCode::NOT_FOUND { StatusCode::NOT_FOUND } else { StatusCode::BAD_GATEWAY }, if status == StatusCode::NOT_FOUND { "找不到這張卡".to_string() } else { format!("KnowledgeBase：{}", v["error"].as_str().unwrap_or(status.as_str())) }));
    }
    Ok(v.get("node").cloned().unwrap_or(v))
}

/// The author's newest turns: what he says is wrong (his words only, newest last).
pub fn request_of(turns: &[Turn], extra: &str) -> String {
    let mut said: Vec<&str> = turns.iter().filter(|t| t.role == "user").map(|t| t.text.trim()).filter(|t| !t.is_empty()).collect();
    if !extra.trim().is_empty() {
        said.push(extra.trim());
    }
    let mut out: Vec<&str> = vec![];
    let mut n = 0;
    for s in said.into_iter().rev().take(3) {
        n += s.chars().count();
        out.push(s);
        if n > REQUEST_CHARS {
            break;
        }
    }
    out.reverse();
    clip(&out.join("\n"), REQUEST_CHARS)
}

const RULES: &str = "你幫一位研究者修改他知識庫裡的一張知識卡，照他說的改（他的話在下面）：可能是有錯要更正，
也可能是要補上說明、改寫得更好懂。凡是寫進卡片的事實，先用網路搜尋查證（教科書、論文、官方文件、維基百科等可靠來源）。
- 照他的要求改；他沒提到、也沒有錯的句子、寫法、LaTeX（$…$）原樣保留。要補說明時，用自然的台灣繁體中文，
  公式保留，在公式前後用一兩句話說它是什麼、每個符號代表什麼。
- title、summary、body 各自只放卡片本身的文字（沒錯的欄位照抄原文）：不要在 body 寫「標題：」「摘要：」這類標籤，
  不要加 Markdown 標題或粗體，不要加補充說明；為什麼改、根據什麼，只寫在 changes。
- 他說有錯、但查證後其實沒錯，就原樣回傳，並在 changes 說明為什麼沒有改。
- changes：每一處改動一項（what＝改了什麼，why＝為什麼，url＝根據的來源網址）。
- sources：你實際用到的來源（url、title），最多 6 個。
- 用台灣繁體中文（術語可以保留英文）。
回一個 JSON：{\"title\", \"summary\", \"body\", \"changes\": [...], \"sources\": [...]}。";

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "summary": { "type": "string" },
            "body": { "type": "string" },
            "changes": { "type": "array", "items": { "type": "object", "properties": { "what": { "type": "string" }, "why": { "type": "string" }, "url": { "type": "string" } }, "required": ["what"] } },
            "sources": { "type": "array", "items": { "type": "object", "properties": { "url": { "type": "string" }, "title": { "type": "string" } }, "required": ["url"] } }
        },
        "required": ["title", "summary", "body", "changes"]
    })
}

/// The request: the card and his words, web search on.
pub fn request(card: &Value, said: &str) -> LlmRequest {
    let aliases = card["aliases"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("、")).unwrap_or_default();
    let user = format!(
        "他的話：\n{said}\n\n這張卡：\n標題：{}\n別名：{}\n摘要：{}\n內容：\n{}",
        card["title"].as_str().unwrap_or(""),
        aliases,
        card["summary"].as_str().unwrap_or(""),
        card["body"].as_str().unwrap_or("")
    );
    LlmRequest::chat(RULES, vec![Message::user(user)]).json(schema()).temperature(0.2).web(true)
}

fn web_url(u: &str) -> bool {
    let u = u.trim();
    (u.starts_with("https://") || u.starts_with("http://")) && u.len() <= 500 && !u.contains(char::is_whitespace)
}

/// The model's answer, cleaned: an empty field keeps the card's own, only web links count.
pub fn read(v: &Value, card: &Value) -> Value {
    let keep = |k: &str| {
        let s = v[k].as_str().unwrap_or("").trim().to_string();
        if s.is_empty() { card[k].as_str().unwrap_or("").to_string() } else { s }
    };
    let changes: Vec<Value> = v["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let what = c["what"].as_str()?.trim();
            (!what.is_empty()).then(|| json!({ "what": clip(what, 300), "why": clip(c["why"].as_str().unwrap_or("").trim(), 400), "url": c["url"].as_str().filter(|u| web_url(u)).unwrap_or("") }))
        })
        .take(12)
        .collect();
    let mut sources: Vec<Value> = vec![];
    for s in v["sources"].as_array().into_iter().flatten() {
        let Some(url) = s["url"].as_str().filter(|u| web_url(u)) else { continue };
        if !sources.iter().any(|x| x["url"] == url) {
            sources.push(json!({ "url": url.trim(), "title": clip(s["title"].as_str().unwrap_or("").trim(), 160) }));
        }
    }
    sources.truncate(SOURCES);
    json!({ "title": clip(&keep("title"), 80), "summary": keep("summary"), "body": keep("body"), "changes": changes, "sources": sources })
}

#[derive(Deserialize)]
struct DraftBody {
    #[serde(default)]
    messages: Vec<Turn>,
    #[serde(default)]
    card: String,
    /// Words typed in the preview (when the conversation does not say what is wrong).
    #[serde(default)]
    request: String,
}

async fn draft(State(s): State<Shared>, body: Option<Json<DraftBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { messages, card }") };
    if !kb_core::id::valid_node_id(&b.card) {
        return bad(StatusCode::BAD_REQUEST, "要指定哪一張卡");
    }
    let said = request_of(&b.messages, &b.request);
    if said.is_empty() {
        return bad(StatusCode::BAD_REQUEST, "說一下哪裡寫錯了");
    }
    let Some(url) = s.kb.server().await else { return bad(StatusCode::SERVICE_UNAVAILABLE, KB_CLOSED) };
    let card = match node(&url, &b.card).await {
        Ok(n) => n,
        Err(r) => return r,
    };
    let route = match s.llm.route(TASK_FIX, None).await {
        Ok(r) => r,
        Err(payload) => return (StatusCode::SERVICE_UNAVAILABLE, Json(payload)).into_response(),
    };
    let id = route.provider.id();
    let req = route.apply(request(&card, &said));
    let v = match lore_llm::complete_json(route.provider.as_ref(), &req).await {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("沒有查好：{e}");
            return bad(StatusCode::BAD_GATEWAY, msg.lines().next().unwrap_or("").trim());
        }
    };
    let fix = read(&v, &card);
    Json(json!({
        "card": { "id": b.card, "revision": card["revision"], "title": card["title"], "summary": card["summary"], "body": card["body"] },
        "fix": { "title": fix["title"], "summary": fix["summary"], "body": fix["body"] },
        "changes": fix["changes"],
        "sources": fix["sources"],
        "label": crate::llm::engine_label(&id),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SourceBody {
    url: String,
}

/// A source the fix used, in the browser (the page cannot open other sites itself). Only a plain
/// web address (the same ones a draft keeps).
async fn source(body: Option<Json<SourceBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { url }") };
    let u = b.url.trim();
    if !web_url(u) {
        return bad(StatusCode::BAD_REQUEST, "只能打開網頁網址");
    }
    match tokio::process::Command::new("/usr/bin/open").arg(u).status().await {
        Ok(s) if s.success() => StatusCode::NO_CONTENT.into_response(),
        _ => bad(StatusCode::INTERNAL_SERVER_ERROR, "打不開這個網址"),
    }
}

#[derive(Deserialize)]
struct SaveBody {
    id: String,
    revision: u64,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    sources: Vec<Value>,
}

/// The card's sources with the web pages the fix used added (`{type: "web", ref: url, title}`).
pub fn merged_sources(card: &Value, added: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = card["sources"].as_array().cloned().unwrap_or_default();
    for s in added {
        let Some(url) = s["url"].as_str().filter(|u| web_url(u)) else { continue };
        if out.iter().any(|x| x["ref"].as_str() == Some(url)) {
            continue;
        }
        let mut x = json!({ "type": "web", "ref": url, "via": "loreecho" });
        if let Some(t) = s["title"].as_str().filter(|t| !t.trim().is_empty()) {
            x["title"] = json!(clip(t.trim(), 160));
        }
        out.push(x);
    }
    out
}

async fn save(State(s): State<Shared>, body: Option<Json<SaveBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { id, revision, title, summary, body }") };
    let title = clip(&b.title.split_whitespace().collect::<Vec<_>>().join(" "), 80);
    if !kb_core::id::valid_node_id(&b.id) || title.is_empty() {
        return bad(StatusCode::BAD_REQUEST, "卡片不對或沒有標題");
    }
    let Some(url) = s.kb.server().await else { return bad(StatusCode::SERVICE_UNAVAILABLE, KB_CLOSED) };
    let card = match node(&url, &b.id).await {
        Ok(n) => n,
        Err(r) => return r,
    };
    let put = json!({
        "revision": b.revision,
        "title": title,
        "summary": b.summary.trim(),
        "body": b.body.trim(),
        "sources": merged_sources(&card, &b.sources),
    });
    match client().put(format!("{url}/api/nodes/{}", b.id)).json(&put).send().await {
        Ok(r) => {
            let status = r.status();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            if status.is_success() {
                Json(json!({ "node": v.get("node").cloned().unwrap_or(v) })).into_response()
            } else if status == StatusCode::CONFLICT {
                bad(StatusCode::CONFLICT, "這張卡在預覽之後改過了，沒有套用；請重新查一次")
            } else {
                bad(StatusCode::BAD_GATEWAY, format!("KnowledgeBase 沒有改：{}", v["error"].as_str().unwrap_or(status.as_str())))
            }
        }
        Err(e) => bad(StatusCode::BAD_GATEWAY, format!("KnowledgeBase 沒有回應：{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(role: &str, text: &str) -> Turn {
        Turn { role: role.into(), text: text.into() }
    }

    #[test]
    fn only_his_words_go_and_the_newest_last() {
        let turns = [t("user", "什麼是 Dice？"), t("assistant", "一種重疊指標。"), t("user", "你的卡把分母寫錯了")];
        assert_eq!(request_of(&turns, ""), "什麼是 Dice？\n你的卡把分母寫錯了");
        assert_eq!(request_of(&turns, "應該是 |A|+|B|"), "什麼是 Dice？\n你的卡把分母寫錯了\n應該是 |A|+|B|");
        assert_eq!(request_of(&[t("assistant", "嗨")], ""), "");
    }

    #[test]
    fn the_request_searches_the_web_and_names_no_file() {
        let card = json!({ "title": "Dice", "aliases": ["F1"], "summary": "s", "body": "$2|A\\cap B|/(|A|\\cup|B|)$" });
        let r = request(&card, "分母錯了");
        assert!(r.web_search && r.json_schema.is_some());
        let u = r.messages[0].text();
        assert!(u.contains("分母錯了") && u.contains("別名：F1") && u.contains("\\cap"));
    }

    #[test]
    fn answers_keep_the_card_where_empty_and_only_web_links() {
        let card = json!({ "title": "Dice", "summary": "舊摘要", "body": "舊" });
        let v = json!({ "title": "", "summary": "", "body": "新", "changes": [{ "what": "分母", "why": "x", "url": "javascript:alert(1)" }, { "what": " " }], "sources": [{ "url": "https://en.wikipedia.org/wiki/Dice", "title": "W" }, { "url": "https://en.wikipedia.org/wiki/Dice" }, { "url": "file:///etc/passwd" }] });
        let f = read(&v, &card);
        assert_eq!((f["title"].as_str(), f["summary"].as_str(), f["body"].as_str()), (Some("Dice"), Some("舊摘要"), Some("新")));
        assert_eq!(f["changes"].as_array().unwrap().len(), 1);
        assert_eq!(f["changes"][0]["url"], "");
        assert_eq!(f["sources"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn sources_are_added_once() {
        let card = json!({ "sources": [{ "type": "paper", "ref": "x" }, { "type": "web", "ref": "https://a.org/1" }] });
        let s = merged_sources(&card, &[json!({ "url": "https://a.org/1" }), json!({ "url": "https://b.org/2", "title": "B" }), json!({ "url": "ftp://c" })]);
        assert_eq!(s.len(), 3);
        assert_eq!(s[2], json!({ "type": "web", "ref": "https://b.org/2", "via": "loreecho", "title": "B" }));
    }
}
