//! 知識卡 for what the book taught (docs/book.md §統一入口): after a chapter is written, every
//! term it taught that has no card (no title or alias match in the L-Brain) gets one. Its words
//! come from a web search of its own (知識建立一定要上網查, task `pc-book-card`): a one-line
//! zh-TW summary and a 2–4 sentence explanation written only from what the search read; no
//! search, or nothing reliable found, means no card. Its sources are the book (its title, never
//! a path) and the pages the search read; `via = "papercomposer-book"`, and marked 不熟 for the member
//! (created, then `Outcome::Unfamiliar`, exactly what L-Brain's 標為不熟 records), so lore-learn
//! reads it back as `Familiar::Unknown`. While L-Brain runs its API is used (`POST /api/nodes`,
//! `POST /api/recall/{id}`); otherwise kb-core writes into the member's personal space.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const VIA: &str = "papercomposer-book";

/// A card's words, from the chapter reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CardText {
    pub term: String,
    pub summary: String,
    pub body: String,
    /// The pages the web search read that the card rests on.
    pub sources: Vec<lore_llm::WebSource>,
}

/// A card the job made.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Made {
    pub id: String,
    pub title: String,
}

/// Where cards go.
pub struct Sink {
    pub spaces: Vec<PathBuf>,
    pub member: String,
    /// L-Brain's address while it runs.
    pub kb_url: Option<String>,
    pub http: Arc<dyn lore_research::Http>,
}

fn key(s: &str) -> String {
    lore_words::phrase_key(s)
}

impl Sink {
    /// Terms of `terms` that no card names (title or alias), each once.
    pub fn without_card(&self, terms: &[String]) -> Vec<String> {
        let names: Vec<String> = lore_learn::load_names(&self.spaces, &self.member).iter().map(|n| key(&n.name)).collect();
        let mut out: Vec<String> = vec![];
        for t in terms {
            let k = key(t);
            if !k.is_empty() && !names.contains(&k) && !out.iter().any(|o| key(o) == k) {
                out.push(t.clone());
            }
        }
        out
    }

    /// One card, marked 不熟. `book` = the book's title (its source).
    pub async fn create(&self, c: &CardText, book: &str) -> Result<Made, String> {
        let summary = c.summary.trim();
        let body = c.body.trim();
        if summary.is_empty() || body.is_empty() {
            return Err("模型沒有給這張卡的摘要與說明".into());
        }
        match &self.kb_url {
            Some(url) => self.create_api(url, c.term.trim(), summary, body, book, &c.sources).await,
            None => self.create_disk(c.term.trim(), summary, body, book, &c.sources),
        }
    }

    async fn create_api(&self, url: &str, title: &str, summary: &str, body: &str, book: &str, web: &[lore_llm::WebSource]) -> Result<Made, String> {
        use lore_research::http::Request;
        let get = |r: lore_research::http::Response| -> Result<Value, String> {
            if !(200..300).contains(&r.status) {
                return Err(format!("L-Brain 回答 {}：{}", r.status, r.text().lines().next().unwrap_or("")));
            }
            serde_json::from_slice(&r.body).map_err(|e| e.to_string())
        };
        let v = get(self.http.send(Request::get(format!("{url}/api/spaces"))).await.map_err(|e| e.to_string())?)?;
        let spaces = v["spaces"].as_array().cloned().unwrap_or_default();
        let space = spaces
            .iter()
            .find(|s| s["kind"] == "personal" && s["is_member"] == true)
            .or_else(|| spaces.iter().find(|s| s["is_member"] == true))
            .and_then(|s| s["id"].as_str())
            .ok_or("知識庫沒有你的空間")?
            .to_string();
        let card = json!({
            "space": space, "via": VIA, "initial_state": "learning",
            "title": title, "summary": summary, "body": body,
            "sources": std::iter::once(json!({ "type": "book", "ref": book })).chain(web.iter().map(|w| json!({ "type": "url", "ref": w.url, "title": w.title }))).collect::<Vec<_>>(),
        });
        let v = get(self.http.send(Request::post_json(format!("{url}/api/nodes"), &card)).await.map_err(|e| e.to_string())?)?;
        let id = v["node"]["id"].as_str().ok_or("L-Brain 沒有回卡片 id")?.to_string();
        get(self.http.send(Request::post_json(format!("{url}/api/recall/{id}"), &json!({ "outcome": "unfamiliar", "via": VIA }))).await.map_err(|e| e.to_string())?)?;
        Ok(Made { id, title: title.to_string() })
    }

    fn create_disk(&self, title: &str, summary: &str, body: &str, book: &str, web: &[lore_llm::WebSource]) -> Result<Made, String> {
        let mut spaces = lore_store::Spaces::new();
        for d in &self.spaces {
            spaces.mount(d).map_err(|e| e.to_string())?;
        }
        let mut kb = kb_core::KnowledgeBase::open(spaces, &self.member, kb_core::KbOptions::default()).map_err(|e| e.to_string())?;
        let info = kb.spaces_info().map_err(|e| e.to_string())?;
        let space = info
            .iter()
            .find(|s| s.kind == lore_store::SpaceKind::Personal && s.is_member)
            .or_else(|| info.iter().find(|s| s.is_member))
            .map(|s| s.id.clone())
            .ok_or("知識庫沒有你的空間")?;
        let fields = kb_core::NodeFields {
            title: Some(title.to_string()),
            summary: Some(summary.to_string()),
            body: Some(body.to_string()),
            sources: Some(
                std::iter::once(kb_core::node::Source { kind: "book".into(), reference: book.to_string(), location: String::new(), extra: Default::default() })
                    .chain(web.iter().map(|w| {
                        let mut extra: std::collections::BTreeMap<String, Value> = Default::default();
                        if !w.title.is_empty() {
                            extra.insert("title".into(), w.title.clone().into());
                        }
                        kb_core::node::Source { kind: "url".into(), reference: w.url.clone(), location: String::new(), extra: extra.into_iter().collect() }
                    }))
                    .collect(),
            ),
            ..Default::default()
        };
        let node = kb.create_as(&space, fields, Some(VIA), kb_core::State::Learning).map_err(|e| e.to_string())?;
        // 標為不熟: what L-Brain records for it
        kb.recall(&node.id, kb_core::progress::Outcome::Unfamiliar, Some(VIA)).map_err(|e| e.to_string())?;
        Ok(Made { id: node.id, title: title.to_string() })
    }
}

/// The card search's instructions (lore-cards' `CARD_SYSTEM` with the explanation a book card
/// also needs).
pub const SYSTEM: &str = "你替作者把一本自學書裡教到的術語做成知識卡，作者要靠這些卡補齊背景知識。\
一定要先用網路搜尋查這個術語（教科書、論文、官方文件、可靠的百科），只根據查到的內容寫，不要憑記憶補；查不到可靠的說明就回 found 為 false。\
用台灣繁體中文；專有名詞第一次出現寫成「中文（English）」，沒有通用中文譯名的直接用英文（例如 ReLU、BERT）。\
只輸出一個 JSON 物件：{\"found\": true 或 false, \"summary\": \"一句話：這個詞是什麼\", \
\"body\": \"2 到 4 句說明：它解決什麼、怎麼運作、和相近概念的差別；用自己的話，不是引句；數學用 $…$\", \
\"sources\": [\"你實際讀過、支持這張卡的網址，最多三個\"]}。";

pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["found"],
        "properties": {
            "found": { "type": "boolean" },
            "summary": { "type": "string" },
            "body": { "type": "string" },
            "sources": { "type": "array", "items": { "type": "string" } }
        }
    })
}

/// The search request for `term`: the book's and the chapter's titles and the chapter's own
/// sentences about it (the book's text, never a source document or a path), so the search
/// looks for the sense the book uses.
pub fn ask(term: &str, book: &str, chapter: &str, chapter_text: &str) -> String {
    let mut user = format!("術語：{term}\n書名：{book}\n章名：{chapter}");
    let ex = excerpt(chapter_text, term, 600);
    if !ex.is_empty() {
        user += &format!("\n這一章裡的句子：{ex}");
    }
    user
}

/// Up to `max` characters of `text` around the first mention of `term` (ASCII case ignored).
fn excerpt(text: &str, term: &str, max: usize) -> String {
    let n = term.len();
    let Some(pos) = text.char_indices().position(|(i, _)| text.get(i..i + n).is_some_and(|w| w.eq_ignore_ascii_case(term))) else { return String::new() };
    let chars: Vec<char> = text.chars().collect();
    let start = pos.saturating_sub(max / 3);
    chars[start..(start + max).min(chars.len())].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The card from the search's reply: refused when the search found nothing reliable or the
/// reply has no words.
pub fn from_reply(term: &str, v: &Value, found: &[lore_llm::WebSource]) -> Result<CardText, String> {
    let summary = v["summary"].as_str().map(str::trim).unwrap_or_default();
    let body = v["body"].as_str().map(str::trim).unwrap_or_default();
    if v["found"] == false || summary.is_empty() || body.is_empty() {
        return Err("網路上查不到可靠的說明".into());
    }
    Ok(CardText { term: term.to_string(), summary: summary.to_string(), body: body.to_string(), sources: lore_cards::search::card_sources(v, found) })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_excerpt_is_around_the_term_on_any_text() {
        let t = "溫度 İİ ".repeat(200) + "A Temperature Scaling divides scores." + &" 後面".repeat(300);
        let ex = super::excerpt(&t, "temperature scaling", 600);
        assert!(ex.contains("Temperature Scaling divides") && ex.chars().count() <= 600, "{ex}");
        assert_eq!(super::excerpt("nothing here", "softmax", 600), "");
    }
}
