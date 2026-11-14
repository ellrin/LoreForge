//! KnowledgeBase tags (`loreshelf-cli kb-tags`): a model gives every card its domains (the first
//! = its colour in KnowledgeBase) and 2–4 two-level tags (`domain/subtopic`), chosen only from a
//! fixed vocabulary — a `tags.json` (the space's own, or one the author is about to adopt). The
//! vocabulary is the author's (his fields), so it is never in this repo; tests use a made-up one.
//!
//! Every card goes to Claude Haiku (author, 2026-11-14: knowledge cards may go to the cloud);
//! the local model only when Haiku is unavailable. Cards never come from identity documents or
//! secrets (lore-catalog's `id-doc` / `secret`), and nothing here reads a file of the catalog.
//!
//! Answers are checked against the vocabulary: anything not listed is dropped; a tag brings its
//! domain; a card left with no tag is not written (the caller lists it).

use lore_llm::{LlmProvider, LlmRequest, Message, complete_json};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const CLOUD_BATCH: usize = 20;
const LOCAL_BATCH: usize = 8;
const EXCERPT: usize = 400;
pub const MAX_TAGS: usize = 4;
pub const MAX_DOMAINS: usize = 3;

/// Domains and `domain/subtopic` tags with their zh labels (a `tags.json`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vocabulary {
    /// slug → label (alphabetical, as KnowledgeBase reads it)
    pub domains: BTreeMap<String, String>,
    pub tags: BTreeMap<String, String>,
}

impl Vocabulary {
    /// From a `tags.json` value (`{domains: {slug: {label}}, tags: {"d/s": {label}}}`).
    pub fn from_json(v: &Value) -> Vocabulary {
        let read = |k: &str| -> BTreeMap<String, String> {
            v[k].as_object().map(|m| m.iter().map(|(s, i)| (s.clone(), i["label"].as_str().unwrap_or(s).to_string())).collect()).unwrap_or_default()
        };
        let domains = read("domains");
        // a tag counts only under a listed domain
        let tags = read("tags").into_iter().filter(|(t, _)| t.split_once('/').is_some_and(|(d, _)| domains.contains_key(d))).collect();
        Vocabulary { domains, tags }
    }

    pub fn load(path: &Path) -> Result<Vocabulary, String> {
        let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value = serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Vocabulary::from_json(&v))
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// The body of `PUT /api/tags` (and of kb-core's `set_vocabulary`).
    pub fn to_json(&self) -> Value {
        let m = |x: &BTreeMap<String, String>| -> Value { x.iter().map(|(k, l)| (k.clone(), json!({ "label": l }))).collect::<serde_json::Map<_, _>>().into() };
        json!({ "domains": m(&self.domains), "tags": m(&self.tags) })
    }

    /// A domain's label (the slug itself when unlisted).
    pub fn label<'a>(&'a self, slug: &'a str) -> &'a str {
        self.domains.get(slug).map(String::as_str).unwrap_or(slug)
    }

    /// A free field name (a model's 「醫學影像」, a slug, an English name) → the one registered
    /// domain it names: the slug itself, the same label, or a label containing it / contained in
    /// it. `None` when nothing or more than one domain fits.
    pub fn map_field(&self, field: &str) -> Option<String> {
        let f = field.trim();
        if f.is_empty() {
            return None;
        }
        let slug = f.to_lowercase().replace([' ', '_'], "-");
        if self.domains.contains_key(&slug) {
            return Some(slug);
        }
        if let Some((s, _)) = self.domains.iter().find(|(_, l)| l.as_str() == f) {
            return Some(s.clone());
        }
        if f.chars().count() < 2 {
            return None;
        }
        let hits: Vec<&String> = self.domains.iter().filter(|(_, l)| l.contains(f) || (l.chars().count() >= 2 && f.contains(l.as_str()))).map(|(s, _)| s).collect();
        match hits.as_slice() {
            [one] => Some((*one).clone()),
            _ => None,
        }
    }

    /// The model's answer, kept to the vocabulary: listed domains (first = main, at most
    /// [`MAX_DOMAINS`]), listed tags (at most [`MAX_TAGS`]); a tag's domain joins the list; no
    /// listed tag → `None`.
    pub fn check(&self, domains: &[String], tags: &[String]) -> Option<(Vec<String>, Vec<String>)> {
        let mut ts: Vec<String> = Vec::new();
        for t in tags.iter().map(|t| t.trim()) {
            if self.tags.contains_key(t) && !ts.iter().any(|x| x == t) && ts.len() < MAX_TAGS {
                ts.push(t.to_string());
            }
        }
        if ts.is_empty() {
            return None;
        }
        let mut ds: Vec<String> = Vec::new();
        for d in domains.iter().map(|d| d.trim()).chain(ts.iter().map(|t| t.split('/').next().unwrap_or(""))) {
            if self.domains.contains_key(d) && !ds.iter().any(|x| x == d) {
                ds.push(d.to_string());
            }
        }
        // the tags' domains always stay; extra domains without a tag only up to the cap
        let tagged: Vec<&str> = ts.iter().map(|t| t.split('/').next().unwrap_or("")).collect();
        let mut out: Vec<String> = Vec::new();
        for d in ds {
            if tagged.contains(&d.as_str()) || out.len() < MAX_DOMAINS {
                out.push(d);
            }
        }
        Some((out, ts))
    }

    /// The list the model chooses from, one domain per line.
    fn prompt_list(&self) -> String {
        self.domains
            .iter()
            .map(|(d, l)| {
                let subs: Vec<String> = self.tags.iter().filter(|(t, _)| t.split('/').next() == Some(d.as_str())).map(|(t, tl)| format!("{t}（{tl}）")).collect();
                format!("- {d}（{l}）：{}", subs.join("、"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One card to tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    /// What the card has now (a hint for the model; free-form ones are dropped on write).
    pub domains: Vec<String>,
    pub tags: Vec<String>,
    /// Only the local model may read it.
    pub local_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tagged {
    pub domains: Vec<String>,
    pub tags: Vec<String>,
    /// `cloud` or `local`
    pub by: &'static str,
}

const SYSTEM: &str = "你幫一位研究者整理知識庫的卡片標籤。每張卡附標題、摘要與內文開頭。\n只能從下面的清單選，照抄 slug，不可自創：\n- domains：1–2 個領域 slug，第一個是這張卡最主要的領域。\n- tags：2–4 個「領域/子題」標籤（至少 2 個，真的只有一個合適才給 1 個），選最能找到這張卡的。\n每張卡都要回答，key 照抄。\n\n清單：\n";

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "cards": { "type": "array", "items": {
            "type": "object",
            "properties": { "key": { "type": "string" }, "domains": { "type": "array", "items": { "type": "string" } }, "tags": { "type": "array", "items": { "type": "string" } } },
            "required": ["key", "domains", "tags"],
            "additionalProperties": false
        } } },
        "required": ["cards"],
        "additionalProperties": false
    })
}

/// One batch: keys `c1…` (no card id), title, summary, the body's start, today's labels.
pub fn request(batch: &[&Card], vocab: &Vocabulary) -> LlmRequest {
    let list = batch
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let body: String = c.body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(EXCERPT).collect();
            let now: Vec<&str> = c.domains.iter().chain(c.tags.iter()).map(String::as_str).collect();
            format!("- key: c{}\n  標題：{}\n  摘要：{}\n  內文：{}{}", i + 1, c.title, c.summary.trim(), body, if now.is_empty() { String::new() } else { format!("\n  現有：{}", now.join("、")) })
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut req = LlmRequest::chat(format!("{SYSTEM}{}", vocab.prompt_list()), vec![Message::user(format!("卡（{} 張）：\n{list}", batch.len()))]);
    req.json_schema = Some(schema());
    req.max_tokens = 6000;
    req
}

/// card id → checked answer (cards with no listed tag are left out).
pub fn read_answer(v: &Value, asked: &[&Card], vocab: &Vocabulary, by: &'static str) -> BTreeMap<String, Tagged> {
    let strs = |x: &Value| -> Vec<String> { x.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let mut out = BTreeMap::new();
    for x in v["cards"].as_array().into_iter().flatten() {
        let Some(i) = x["key"].as_str().and_then(|k| k.trim().strip_prefix('c')).and_then(|n| n.parse::<usize>().ok()) else { continue };
        let Some(c) = i.checked_sub(1).and_then(|i| asked.get(i)) else { continue };
        if let Some((domains, tags)) = vocab.check(&strs(&x["domains"]), &strs(&x["tags"])) {
            out.insert(c.id.clone(), Tagged { domains, tags, by });
        }
    }
    out
}

/// Tag every card: local-only ones on `local` (when there is one), the rest on `cloud` (when
/// there is one, else `local`). A local-only card never reaches `cloud`. Returns id → answer;
/// cards missing from it got no usable answer.
pub async fn run(cards: &[Card], vocab: &Vocabulary, cloud: Option<&dyn LlmProvider>, local: Option<&dyn LlmProvider>, mut progress: impl FnMut(usize, usize)) -> Result<BTreeMap<String, Tagged>, String> {
    let (to_local, to_cloud): (Vec<&Card>, Vec<&Card>) = cards.iter().partition(|c| c.local_only || cloud.is_none());
    let mut out = BTreeMap::new();
    let total = cards.len();
    let mut done = 0;
    for (list, p, size, by) in [(to_cloud, cloud, CLOUD_BATCH, "cloud"), (to_local, local, LOCAL_BATCH, "local")] {
        let Some(p) = p else { continue };
        let mut failed = 0;
        for chunk in list.chunks(size) {
            let mut ask: Vec<&Card> = chunk.to_vec();
            for _ in 0..2 {
                if ask.is_empty() {
                    break;
                }
                match complete_json(p, &request(&ask, vocab)).await {
                    Ok(v) => {
                        let got = read_answer(&v, &ask, vocab, by);
                        ask.retain(|c| !got.contains_key(&c.id));
                        out.extend(got);
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("一批沒有結果：{e}");
                        if out.is_empty() && failed >= 2 {
                            return Err(format!("模型沒有回答：{e}"));
                        }
                        break;
                    }
                }
            }
            done += chunk.len();
            progress(done, total);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_llm::types::{Capabilities, LlmResponse};

    fn vocab() -> Vocabulary {
        Vocabulary::from_json(&json!({
            "domains": { "alpha": { "label": "甲學" }, "beta": { "label": "乙學分析" }, "gamma": { "label": "丙學" } },
            "tags": { "alpha/one": { "label": "一" }, "alpha/two": { "label": "二" }, "beta/three": { "label": "三" }, "zeta/off": { "label": "外" } }
        }))
    }

    /// Answers every card with a fixed choice (one listed, one invented); records what it saw.
    struct Mock(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl LlmProvider for Mock {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { json: true, vision: false, web: false, streaming: false, context: 100_000, needs_network: false }
        }
        async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, lore_llm::LlmError> {
            let u = req.messages[0].text();
            self.0.lock().unwrap().push(format!("{}\n{u}", req.system));
            let cards: Vec<Value> = u
                .lines()
                .filter_map(|l| l.trim().strip_prefix("- key: "))
                .map(|k| json!({ "key": k, "domains": ["甲學", "beta", "made-up"], "tags": ["alpha/one", "beta/three", "alpha/invented", "deep/x", "alpha/one"] }))
                .collect();
            Ok(LlmResponse { text: json!({ "cards": cards }).to_string(), provider: "mock".into(), sources: vec![], usage: None })
        }
    }

    fn card(id: &str, title: &str, local_only: bool) -> Card {
        Card { id: id.into(), title: title.into(), summary: format!("summary of {title}"), body: format!("body of {title}"), local_only, ..Card::default() }
    }

    /// A card marked `local_only` stays on the local model (the field is kept for callers that need it;
    /// kb-tags sends every card to Haiku); unlisted tags are dropped.
    #[tokio::test]
    async fn routing_and_unlisted_tags_are_dropped() {
        let (c, l) = (Mock(Default::default()), Mock(Default::default()));
        let cards = vec![card("n1", "Paper Term", false), card("n2", "Homework Term ellrin", true), card("n3", "Other Paper Term", false)];
        let got = run(&cards, &vocab(), Some(&c), Some(&l), |_, _| {}).await.unwrap();
        let sent_cloud = c.0.lock().unwrap().join("\n");
        let sent_local = l.0.lock().unwrap().join("\n");
        assert!(sent_cloud.contains("Paper Term") && sent_cloud.contains("Other Paper Term"));
        assert!(!sent_cloud.contains("Homework") && !sent_cloud.contains("ellrin"), "{sent_cloud}");
        assert!(sent_local.contains("Homework Term ellrin") && !sent_local.contains("Other Paper"));
        assert!(!sent_cloud.contains("n1"), "card ids stay home: {sent_cloud}");
        // only listed tags, deduped; the free「甲學」is not a slug; the tags' domains come along
        let want = Tagged { domains: vec!["beta".into(), "alpha".into()], tags: vec!["alpha/one".into(), "beta/three".into()], by: "cloud" };
        assert_eq!(got["n1"], want);
        assert_eq!(got["n2"].by, "local");
        // the vocabulary went to the model; the tag under an unlisted domain did not
        assert!(sent_cloud.contains("alpha/two（二）") && !sent_cloud.contains("zeta/off"));
    }

    #[tokio::test]
    async fn no_local_model_means_local_cards_wait_and_no_cloud_means_all_local() {
        let c = Mock(Default::default());
        let cards = vec![card("n1", "Paper Term", false), card("n2", "Homework Term", true)];
        let got = run(&cards, &vocab(), Some(&c), None, |_, _| {}).await.unwrap();
        assert!(got.contains_key("n1") && !got.contains_key("n2"));
        assert!(!c.0.lock().unwrap().join("\n").contains("Homework"));
        let l = Mock(Default::default());
        let got = run(&cards, &vocab(), None, Some(&l), |_, _| {}).await.unwrap();
        assert_eq!(got.values().filter(|t| t.by == "local").count(), 2);
    }

    #[test]
    fn checking_answers_and_mapping_fields() {
        let v = vocab();
        assert_eq!(v.check(&["gamma".into()], &["nope/x".into()]), None, "no listed tag, nothing to write");
        let (d, t) = v.check(&["gamma".into(), "beta".into(), "alpha".into()], &["alpha/one".into(), "alpha/two".into(), "beta/three".into(), "alpha/one".into()]).unwrap();
        assert_eq!(d, vec!["gamma", "beta", "alpha"]);
        assert_eq!(t.len(), 3);
        assert_eq!(v.map_field("甲學").as_deref(), Some("alpha"));
        assert_eq!(v.map_field("乙學").as_deref(), Some("beta"), "a label containing it");
        assert_eq!(v.map_field("Beta").as_deref(), Some("beta"));
        assert_eq!(v.map_field("學"), None, "too short");
        assert_eq!(v.map_field("影像評估"), None);
        assert_eq!(v.label("alpha"), "甲學");
        assert_eq!(v.to_json()["tags"]["alpha/one"]["label"], "一");
        assert!(v.to_json()["tags"].get("zeta/off").is_none());
    }
}
