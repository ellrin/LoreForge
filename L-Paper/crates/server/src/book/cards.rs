//! 知識卡 for what the book taught (docs/book.md §統一入口): after a chapter is written, every
//! term it taught that has no card (no title or alias match in the L-Brain) gets one: the
//! chapter model's own one-line zh-TW summary and 2–4 sentence explanation, the book as its
//! source (its title, never a path), `via = "papercomposer-book"`, and marked 不熟 for the member
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
            Some(url) => self.create_api(url, c.term.trim(), summary, body, book).await,
            None => self.create_disk(c.term.trim(), summary, body, book),
        }
    }

    async fn create_api(&self, url: &str, title: &str, summary: &str, body: &str, book: &str) -> Result<Made, String> {
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
            "sources": [{ "type": "book", "ref": book }],
        });
        let v = get(self.http.send(Request::post_json(format!("{url}/api/nodes"), &card)).await.map_err(|e| e.to_string())?)?;
        let id = v["node"]["id"].as_str().ok_or("L-Brain 沒有回卡片 id")?.to_string();
        get(self.http.send(Request::post_json(format!("{url}/api/recall/{id}"), &json!({ "outcome": "unfamiliar", "via": VIA }))).await.map_err(|e| e.to_string())?)?;
        Ok(Made { id, title: title.to_string() })
    }

    fn create_disk(&self, title: &str, summary: &str, body: &str, book: &str) -> Result<Made, String> {
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
            sources: Some(vec![kb_core::node::Source { kind: "book".into(), reference: book.to_string(), location: String::new(), extra: Default::default() }]),
            ..Default::default()
        };
        let node = kb.create_as(&space, fields, Some(VIA), kb_core::State::Learning).map_err(|e| e.to_string())?;
        // 標為不熟: what L-Brain records for it
        kb.recall(&node.id, kb_core::progress::Outcome::Unfamiliar, Some(VIA)).map_err(|e| e.to_string())?;
        Ok(Made { id: node.id, title: title.to_string() })
    }
}

/// The chapter prompt's request for the cards of `terms` (empty: none asked).
pub fn ask(terms: &[String]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    format!(
        "\n\nAlso give `cards`: one entry for each of these terms the chapter teaches: {}. Each is {{term, summary, body}}: `summary` one \
line in Traditional Chinese (Taiwan usage) saying what the term is; `body` 2–4 sentences in Traditional Chinese explaining it as this \
chapter defines it, in your own words (never a quotation from a source); keep the English term and math as they are.",
        terms.join("; ")
    )
}

/// The card texts the reply gave for `terms` (matched by term, each once).
pub fn texts(reply: &Value, terms: &[String]) -> Vec<CardText> {
    let given: Vec<CardText> = reply["cards"].as_array().into_iter().flatten().filter_map(|c| serde_json::from_value(c.clone()).ok()).collect();
    terms
        .iter()
        .filter_map(|t| given.iter().find(|g| key(&g.term) == key(t)).map(|g| CardText { term: t.clone(), summary: g.summary.trim().to_string(), body: g.body.trim().to_string() }))
        .filter(|c| !c.summary.is_empty() && !c.body.is_empty())
        .collect()
}
