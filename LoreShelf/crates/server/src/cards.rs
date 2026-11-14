//! The known cards' titles: before the terms of the folders the author knows all of become known
//! KnowledgeBase cards, Claude Haiku (`shelf-card-titles`) looks at each once — is it a clean
//! term of a field, and what is its usual written form (`Transformer Vaswani` → `Transformer`,
//! `tains domain-invariant` → dropped).
//!
//! Only terms found in a paper or a slide deck go (`Term::cloud_ok`), with sentences from those
//! files only; never a file name or path. The answers are kept beside the pool,
//! `<cache>/card-titles.json` (key → verdict), so a pool rebuilt or re-sorted meanwhile loses none.

use lore_catalog::terms::{Pool, Term};
use lore_llm::{LlmProvider, LlmRequest, Message, complete_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const BATCH: usize = 40;
const SHOWN: usize = 2;
const SENTENCE: usize = 200;

const SYSTEM: &str = "你幫一位研究者把他已經懂的專有名詞做成知識卡。每個詞來自他讀過的論文或簡報，附一兩句原文（PDF 抽出的文字，可能有斷字、黏字或作者名）。\n對每個詞：\n- keep：是否是一個乾淨、值得做卡的專有名詞（方法、模型、指標、資料集、解剖或臨床名詞等）。斷字、殘缺、只是作者名、日常英文（普通國中生就會的字，用的也是日常意思）都 false；專業用語再基本也是 true（如 ReLU、p-value、loss function、Batch Normalization）。\n- title：卡片標題，用該領域通行的寫法（英文、正確大小寫，例如 Transformer Vaswani → Transformer、distance FID → Fréchet Inception Distance (FID)）。keep 為 false 時留空。\n- field：所屬領域，繁體中文 2–6 字。\n每個詞都要回答，key 照抄。";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Titled {
    pub keep: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub field: String,
    /// The card's own words (author, 2026-11-14: quotes from papers are not a card): one sentence…
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// …and a few sentences of explanation, in Traditional Chinese, a standard formula in $…$.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// The content pass has answered (it may also have dropped the card as a field basic).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub written: bool,
}

/// `<cache>/card-titles.json`: term key → answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Titles {
    pub version: u32,
    pub titles: BTreeMap<String, Titled>,
}

impl Titles {
    pub fn file(cache_dir: &Path) -> PathBuf {
        lore_catalog::terms::Pool::file(cache_dir).with_file_name("card-titles.json")
    }
    pub fn load(cache_dir: &Path) -> Titles {
        std::fs::read(Titles::file(cache_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Titles { version: 1, ..Titles::default() })
    }
    pub fn save(&self, cache_dir: &Path) -> Result<(), String> {
        let b = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        lore_catalog::store::write_atomic(&Titles::file(cache_dir), &b).map_err(|e| e.to_string())
    }
}

/// The terms that may become known cards: from a folder he knows all of, called a term, and
/// found in a paper or a slide deck (the only ones that may go to Claude).
pub fn candidates(pool: &Pool) -> impl Iterator<Item = &Term> {
    pool.terms.iter().filter(|t| t.mastered && t.verdict.as_deref() == Some("term") && t.cloud_ok)
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "cards": { "type": "array", "items": {
            "type": "object",
            "properties": { "key": { "type": "string" }, "keep": { "type": "boolean" }, "title": { "type": "string" }, "field": { "type": "string" } },
            "required": ["key", "keep"],
            "additionalProperties": false
        } } },
        "required": ["cards"],
        "additionalProperties": false
    })
}

/// One batch's request: the term, its written form and sentences from papers / slides only.
pub fn request(batch: &[&Term]) -> LlmRequest {
    request_with(batch, false)
}

/// `local` = the local model asks: sentences from any file may go with the term.
pub fn request_with(batch: &[&Term], local: bool) -> LlmRequest {
    let list = batch
        .iter()
        .map(|t| {
            let ex = t.examples.iter().filter(|e| local || e.cloud_ok).take(SHOWN).map(|e| format!("  「{}」", e.text.chars().take(SENTENCE).collect::<String>())).collect::<Vec<_>>().join("\n");
            format!("- key: {}（寫作 {}）\n{ex}", t.key, t.label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut req = LlmRequest::chat(SYSTEM, vec![Message::user(format!("詞（{} 個）：\n{list}", batch.len()))]);
    req.json_schema = Some(schema());
    req.max_tokens = 8000;
    req
}

pub fn read_answer(v: &Value, asked: &[&Term]) -> BTreeMap<String, Titled> {
    let keys: std::collections::HashMap<String, &str> = asked.iter().flat_map(|t| [(t.key.to_lowercase(), t.key.as_str()), (t.label.to_lowercase(), t.key.as_str())]).collect();
    let mut out = BTreeMap::new();
    for x in v["cards"].as_array().into_iter().flatten() {
        let (Some(k), Some(keep)) = (x["key"].as_str(), x["keep"].as_bool()) else { continue };
        let Some(&k) = keys.get(&k.trim().to_lowercase()) else { continue };
        let title = x["title"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
        let keep = keep && !title.is_empty() && title.chars().count() <= 80;
        let field = x["field"].as_str().unwrap_or("").trim().to_string();
        out.insert(k.to_string(), Titled { keep, title: if keep { title } else { String::new() }, field: if keep { field } else { String::new() }, ..Titled::default() });
    }
    out
}

/// Ask about every candidate not asked yet; saved after each batch. Returns (asked, answered).
pub async fn run(cache_dir: &Path, p: &dyn LlmProvider, progress: impl FnMut(usize, usize)) -> Result<(usize, usize), String> {
    run_for(cache_dir, p, None, false, progress).await
}

/// [`run`] for these term keys only (`None` = the mastered candidates). Claude (`local` false)
/// only ever gets terms and sentences from papers and slide decks; the local model gets any.
pub async fn run_for(cache_dir: &Path, p: &dyn LlmProvider, keys: Option<&std::collections::HashSet<String>>, local: bool, mut progress: impl FnMut(usize, usize)) -> Result<(usize, usize), String> {
    let pool = Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf-cli terms")?;
    let mut titles = Titles::load(cache_dir);
    let todo: Vec<&Term> = match keys {
        Some(k) => pool.terms.iter().filter(|t| k.contains(&t.key) && (local || t.cloud_ok)).collect(),
        None => candidates(&pool).collect(),
    };
    let todo: Vec<&Term> = todo.into_iter().filter(|t| !titles.titles.contains_key(&t.key)).collect();
    let (total, mut answered, mut failed) = (todo.len(), 0, 0);
    for (bi, chunk) in todo.chunks(BATCH).enumerate() {
        let mut ask: Vec<&Term> = chunk.to_vec();
        for _ in 0..2 {
            if ask.is_empty() {
                break;
            }
            match complete_json(p, &request_with(&ask, local)).await {
                Ok(v) => {
                    let got = read_answer(&v, &ask);
                    answered += got.len();
                    ask.retain(|t| !got.contains_key(&t.key));
                    titles.titles.extend(got);
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("批次 {} 沒有結果：{e}", bi + 1);
                    if answered == 0 && failed >= 2 {
                        return Err(format!("Claude 沒有回答：{e}"));
                    }
                    break;
                }
            }
        }
        titles.save(cache_dir)?;
        progress(((bi + 1) * BATCH).min(total), total);
    }
    Ok((total, answered))
}

// ── the card's content (author, 2026-11-14) ──

pub const CONTENT_BATCH: usize = 15;

const CONTENT_SYSTEM: &str = "你幫一位研究者寫知識卡的內容。每張卡是一個他已經懂的專有名詞（附所屬領域，和一兩句他讀到這個詞的原文，只是線索，不要照抄）。
對每張卡：
- keep：幾乎都是 true：專業用語再基本也要做卡（例如 ReLU、p-value、Batch Normalization、Cross-Entropy、Image Classification），新概念才連得過去。只有日常英文（普通國中生就會的字，用的也是日常意思）或殘缺的詞才 false。
- summary：一句話說它是什麼（台灣繁體中文，20–60 字，術語可保留英文）。
- body：二到四句說明：用來做什麼、怎麼運作或怎麼算（有標準公式就用 $…$ 的 LaTeX 寫出）、和相近概念的差別。不要引用原文，不要 Markdown 標題或粗體。
keep 為 false 時 summary、body 留空。每張都要回答，key 照抄。";

pub fn content_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "cards": { "type": "array", "items": {
            "type": "object",
            "properties": { "key": { "type": "string" }, "keep": { "type": "boolean" }, "summary": { "type": "string" }, "body": { "type": "string" } },
            "required": ["key", "keep"],
            "additionalProperties": false
        } } },
        "required": ["cards"],
        "additionalProperties": false
    })
}

/// One batch: the card's title and field, and sentences from papers / slides only.
pub fn content_request(batch: &[(&str, &Titled, Vec<&str>)]) -> LlmRequest {
    let list = batch
        .iter()
        .map(|(k, t, ex)| {
            let ex = ex.iter().take(SHOWN).map(|e| format!("  「{}」", e.chars().take(SENTENCE).collect::<String>())).collect::<Vec<_>>().join("\n");
            format!("- key: {k}\n  標題：{}（{}）\n{ex}", t.title, t.field)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut req = LlmRequest::chat(CONTENT_SYSTEM, vec![Message::user(format!("卡（{} 張）：\n{list}", batch.len()))]);
    req.json_schema = Some(content_schema());
    req.max_tokens = 12000;
    req
}

/// Writes every kept title's summary and body once (saved per batch); a field basic is dropped.
/// Returns (asked, answered).
pub async fn write_content(cache_dir: &Path, p: &dyn LlmProvider, progress: impl FnMut(usize, usize)) -> Result<(usize, usize), String> {
    write_content_for(cache_dir, p, None, false, progress).await
}

/// [`write_content`] for these keys only; `local` as in [`run_for`] (Claude: paper sentences only,
/// and only for terms found in papers or slide decks).
pub async fn write_content_for(cache_dir: &Path, p: &dyn LlmProvider, keys: Option<&std::collections::HashSet<String>>, local: bool, mut progress: impl FnMut(usize, usize)) -> Result<(usize, usize), String> {
    let pool = Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf-cli terms")?;
    let mut titles = Titles::load(cache_dir);
    let examples = |k: &str| -> Vec<String> { pool.terms.iter().find(|t| t.key == k).map(|t| t.examples.iter().filter(|e| local || e.cloud_ok).map(|e| e.text.clone()).collect()).unwrap_or_default() };
    let cloud_term = |k: &str| pool.terms.iter().find(|t| t.key == k).is_none_or(|t| t.cloud_ok);
    let todo: Vec<String> = titles.titles.iter().filter(|(k, t)| t.keep && !t.written && keys.is_none_or(|ks| ks.contains(*k)) && (local || cloud_term(k))).map(|(k, _)| k.clone()).collect();
    let (total, mut answered, mut failed) = (todo.len(), 0, 0);
    for (bi, chunk) in todo.chunks(CONTENT_BATCH).enumerate() {
        let mut ask: Vec<String> = chunk.to_vec();
        for _ in 0..2 {
            if ask.is_empty() {
                break;
            }
            let ex: Vec<Vec<String>> = ask.iter().map(|k| examples(k)).collect();
            let batch: Vec<(&str, &Titled, Vec<&str>)> = ask.iter().zip(&ex).map(|(k, e)| (k.as_str(), &titles.titles[k], e.iter().map(String::as_str).collect())).collect();
            match complete_json(p, &content_request(&batch)).await {
                Ok(v) => {
                    let mut got = 0;
                    for x in v["cards"].as_array().into_iter().flatten() {
                        let (Some(k), Some(keep)) = (x["key"].as_str().map(str::trim), x["keep"].as_bool()) else { continue };
                        let Some(t) = titles.titles.get_mut(k).filter(|_| ask.iter().any(|a| a == k)) else { continue };
                        let summary = x["summary"].as_str().unwrap_or("").trim().to_string();
                        let body = x["body"].as_str().unwrap_or("").trim().to_string();
                        if keep && summary.is_empty() {
                            continue; // no words: asked again
                        }
                        t.keep = keep;
                        t.summary = if keep { summary } else { String::new() };
                        t.body = if keep { body } else { String::new() };
                        t.written = true;
                        got += 1;
                    }
                    answered += got;
                    ask.retain(|k| !titles.titles[k].written);
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("批次 {} 沒有結果：{e}", bi + 1);
                    if answered == 0 && failed >= 2 {
                        return Err(format!("Claude 沒有回答：{e}"));
                    }
                    break;
                }
            }
        }
        titles.save(cache_dir)?;
        progress(((bi + 1) * CONTENT_BATCH).min(total), total);
    }
    Ok((total, answered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_catalog::terms::Example;
    use lore_llm::types::{Capabilities, LlmResponse};

    fn term(k: &str, cloud: bool) -> Term {
        Term {
            key: k.into(),
            label: k.into(),
            files: 1,
            count: 2,
            score: 1.0,
            cloud_ok: cloud,
            examples: vec![Example { id: "a".into(), n: 1, text: format!("from a paper: {k}"), cloud_ok: true }, Example { id: "b".into(), n: 1, text: format!("from a private note: {k}"), cloud_ok: false }],
            near: vec![],
            near_known: 0,
            domains: vec![],
            mastered: true,
            verdict: Some("term".into()),
            field: None,
        }
    }

    /// Keeps every term, titled in upper case; records what it was sent.
    struct Upper(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl LlmProvider for Upper {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { json: true, vision: false, web: false, streaming: false, context: 100_000, needs_network: false }
        }
        async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, lore_llm::LlmError> {
            let u = req.messages[0].text();
            self.0.lock().unwrap().push(u.clone());
            let cards: Vec<Value> = u.lines().filter_map(|l| l.strip_prefix("- key: ")).map(|l| l.split('（').next().unwrap().trim()).map(|k| json!({ "key": k, "keep": k != "tains", "title": k.to_uppercase(), "field": "測試" })).collect();
            Ok(LlmResponse { text: json!({ "cards": cards }).to_string(), provider: "mock".into(), sources: vec![], usage: None })
        }
    }

    #[tokio::test]
    async fn only_paper_terms_and_paper_sentences_go_and_answers_are_kept_beside_the_pool() {
        let t = tempfile::tempdir().unwrap();
        let mut terms = vec![term("transformer", true), term("tains", true), term("ellrin method", false)];
        terms.push(Term { mastered: false, ..term("daidai net", true) });
        Pool { version: 1, built: "x".into(), files: 1, terms }.save(t.path()).unwrap();
        let m = Upper(Default::default());
        assert_eq!(run(t.path(), &m, |_, _| {}).await.unwrap(), (2, 2));
        let sent = m.0.lock().unwrap().join("\n");
        assert!(sent.contains("from a paper: transformer") && !sent.contains("private note") && !sent.contains("ellrin method") && !sent.contains("daidai net"), "{sent}");
        let got = Titles::load(t.path());
        assert_eq!(got.titles["transformer"], Titled { keep: true, title: "TRANSFORMER".into(), field: "測試".into(), ..Titled::default() });
        assert!(!got.titles["tains"].keep && got.titles["tains"].title.is_empty());
        assert_eq!(run(t.path(), &m, |_, _| {}).await.unwrap(), (0, 0), "never asked twice");
    }

    /// Writes content for every card but drops `relu`; records what it was sent.
    struct Writer(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl LlmProvider for Writer {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { json: true, vision: false, web: false, streaming: false, context: 100_000, needs_network: false }
        }
        async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, lore_llm::LlmError> {
            let u = req.messages[0].text();
            self.0.lock().unwrap().push(u.clone());
            let cards: Vec<Value> = u.lines().filter_map(|l| l.strip_prefix("- key: ")).map(|k| if k == "relu" { json!({ "key": k, "keep": false }) } else { json!({ "key": k, "keep": true, "summary": format!("{k} 是一種方法。"), "body": "說明。$a=b$" }) }).collect();
            Ok(LlmResponse { text: json!({ "cards": cards }).to_string(), provider: "mock".into(), sources: vec![], usage: None })
        }
    }

    #[tokio::test]
    async fn every_kept_card_gets_its_own_words_once_and_field_basics_are_dropped() {
        let t = tempfile::tempdir().unwrap();
        Pool { version: 1, built: "x".into(), files: 1, terms: vec![term("transformer", true), term("relu", true)] }.save(t.path()).unwrap();
        let mut titles = Titles { version: 1, ..Titles::default() };
        titles.titles.insert("transformer".into(), Titled { keep: true, title: "Transformer".into(), field: "深度學習".into(), ..Titled::default() });
        titles.titles.insert("relu".into(), Titled { keep: true, title: "ReLU".into(), field: "深度學習".into(), ..Titled::default() });
        titles.titles.insert("tains".into(), Titled::default());
        titles.save(t.path()).unwrap();
        let m = Writer(Default::default());
        assert_eq!(write_content(t.path(), &m, |_, _| {}).await.unwrap(), (2, 2));
        let sent = m.0.lock().unwrap().join("\n");
        assert!(sent.contains("標題：Transformer（深度學習）") && sent.contains("from a paper: transformer") && !sent.contains("private note") && !sent.contains("tains"), "{sent}");
        let got = Titles::load(t.path());
        assert_eq!((got.titles["transformer"].summary.as_str(), got.titles["transformer"].written), ("transformer 是一種方法。", true));
        assert!(!got.titles["relu"].keep && got.titles["relu"].written && got.titles["relu"].body.is_empty());
        assert_eq!(write_content(t.path(), &m, |_, _| {}).await.unwrap(), (0, 0), "never asked twice");
    }

}
