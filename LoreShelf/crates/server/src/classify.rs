//! The model pass over the term pool (`lore_catalog::terms`): which candidates are real terms of
//! a field (`term`), everyday or general academic words (`general`), names of people, places,
//! institutions or products (`name`), or broken text (`junk`) — and for a term, its field.
//!
//! Local model only (the example sentences come from all the author's files), in batches; each
//! batch is written back at once, so a run that stops (the lid closes) goes on where it was.
//! Order: terms next to cards the author knows first, then by score.

use lore_catalog::terms::{Pool, Term};
use lore_words::{Filters, phrase_key};
use lore_llm::{LlmProvider, LlmRequest, Message, complete_json};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

/// Terms per request.
pub const BATCH: usize = 20;
/// Times a batch's unanswered terms are asked again at once (the model skips some keys).
const RETRIES: usize = 1;
/// Sentences shown per term.
const SHOWN: usize = 2;
/// Characters of a shown sentence.
const SENTENCE: usize = 160;

pub const VERDICTS: [&str; 4] = ["term", "general", "name", "junk"];

const SYSTEM: &str = "你幫一位研究者整理他電腦裡檔案出現的英文詞。每個詞附有一兩句原文。\n判斷每個詞是哪一種：\n- term：具體、要查過才懂的專有名詞、方法、模型、指標、解剖或臨床名詞（值得做成知識卡）\n- general：日常英文、一般學術用語，以及這個領域的人天天在用、不必查的基本詞（例如 loss、label、mask、segmentation、encoder、loss function、training set、ground truth）\n- name：人名、地名、學校／機構、期刊、會議、產品或公司名\n- junk：斷字、黏在一起的字、編號、亂碼\n是 term 的，用繁體中文寫它所屬的領域（2–6 個字，例如「醫學影像」「生物力學」「深度學習」）。\n每個詞都要回答，key 照抄。";

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "terms": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "verdict": { "type": "string", "enum": VERDICTS },
                        "field": { "type": "string" }
                    },
                    "required": ["key", "verdict"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["terms"],
        "additionalProperties": false
    })
}

/// The request for one batch. `fields` = the reader's fields (the domains of his cards).
pub fn request(batch: &[&Term], fields: &[String]) -> LlmRequest {
    let reader = if fields.is_empty() { String::new() } else { format!("讀者的領域：{}\n\n", fields.join("、")) };
    let list = batch
        .iter()
        .map(|t| {
            let ex = t.examples.iter().take(SHOWN).map(|e| format!("  「{}」", e.text.chars().take(SENTENCE).collect::<String>())).collect::<Vec<_>>().join("\n");
            format!("- key: {}（寫作 {}）\n{ex}", t.key, t.label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut req = LlmRequest::chat(SYSTEM, vec![Message::user(format!("{reader}詞（{} 個）：\n{list}", batch.len()))]);
    req.json_schema = Some(schema());
    req.max_tokens = 6000;
    req
}

/// The answer's verdicts for the asked keys only (anything else is ignored).
pub fn read_answer(v: &Value, asked: &[&Term]) -> HashMap<String, (String, Option<String>)> {
    // the model sometimes answers with the written form (`IEEE`) or another case: both lead back to the key
    let keys: HashMap<String, &str> = asked.iter().flat_map(|t| [(t.key.to_lowercase(), t.key.as_str()), (t.label.to_lowercase(), t.key.as_str())]).collect();
    let mut out = HashMap::new();
    for x in v["terms"].as_array().into_iter().flatten() {
        let (Some(k), Some(verdict)) = (x["key"].as_str(), x["verdict"].as_str()) else { continue };
        let Some(&k) = keys.get(&k.trim().to_lowercase()) else { continue };
        if !VERDICTS.contains(&verdict) {
            continue;
        }
        let field = (verdict == "term").then(|| x["field"].as_str().map(str::trim).filter(|f| !f.is_empty()).map(str::to_string)).flatten();
        out.insert(k.to_string(), (verdict.to_string(), field));
    }
    out
}

/// The rule after the model: a single everyday word (`Filters::is_everyday`: `loss`, `mask`,
/// `segmentation`) is never a card, whatever the model said — the author reads it without
/// stopping. Returns how many verdicts changed.
pub fn settle(pool: &mut Pool, filters: &Filters) -> usize {
    let mut n = 0;
    for t in pool.terms.iter_mut().filter(|t| t.verdict.as_deref() == Some("term") && !t.label.contains(' ') && filters.is_everyday(&t.label)) {
        t.verdict = Some("general".into());
        t.field = None;
        n += 1;
    }
    n
}

/// Tag on the lines LoreShelf adds to the ignore list (`grep -v '# LoreShelf'` takes them out).
pub const IGNORE_TAG: &str = "# LoreShelf";

/// Lines to add to the shared ignore list (`private/LoreReader/ignore-words.txt`, read by every
/// reader of papers): the words the model or the rule call general or a name, so a new paper
/// never asks about them again. Not the ones the built-in lists already drop, nor any already
/// in `current` (the file's text).
pub fn ignore_lines(pool: &Pool, filters: &Filters, current: &str) -> Vec<String> {
    let mut have: std::collections::HashSet<String> = current.lines().map(|l| phrase_key(l.split('#').next().unwrap_or(""))).filter(|k| !k.is_empty()).collect();
    let mut out = Vec::new();
    for t in pool.terms.iter().filter(|t| matches!(t.verdict.as_deref(), Some("general" | "name"))) {
        let label = t.label.split_whitespace().collect::<Vec<_>>().join(" ");
        if label.is_empty() || label.contains('#') || (!label.contains(' ') && filters.is_general(&label)) {
            continue;
        }
        if have.insert(phrase_key(&label)) {
            out.push(format!("{label}  {IGNORE_TAG}"));
        }
    }
    out
}

/// What a run did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Run {
    pub asked: usize,
    pub answered: usize,
    pub failed_batches: usize,
    pub left: usize,
}

/// Sort what is not sorted yet, `max` terms at most (`None` = all), saving after every batch.
/// `fields` = the reader's fields. `progress(done, total)` after each batch.
pub async fn run(cache_dir: &Path, p: &dyn LlmProvider, fields: &[String], filters: &Filters, max: Option<usize>, mut progress: impl FnMut(usize, usize)) -> Result<Run, String> {
    let mut pool = Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf terms")?;
    let mut order: Vec<usize> = (0..pool.terms.len()).filter(|&i| pool.terms[i].verdict.is_none()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&pool.terms[a], &pool.terms[b]);
        // his own seminar material first (it becomes known cards), then terms next to known cards
        y.mastered.cmp(&x.mastered).then((y.near_known > 0).cmp(&(x.near_known > 0))).then(y.score.partial_cmp(&x.score).unwrap_or(std::cmp::Ordering::Equal))
    });
    if let Some(m) = max {
        order.truncate(m);
    }
    let total = order.len();
    let mut r = Run::default();
    // by key: a pool rebuilt meanwhile may order its terms differently
    let keys: Vec<String> = order.iter().map(|&i| pool.terms[i].key.clone()).collect();
    for (bi, chunk) in keys.chunks(BATCH).enumerate() {
        let mut ask: Vec<String> = chunk.to_vec();
        for attempt in 0..=RETRIES {
            let batch: Vec<&Term> = ask.iter().filter_map(|k| pool.terms.iter().find(|t| &t.key == k && t.verdict.is_none())).collect();
            if batch.is_empty() {
                break;
            }
            if attempt == 0 {
                r.asked += batch.len();
            }
            let got = match complete_json(p, &request(&batch, fields)).await {
                Ok(v) => read_answer(&v, &batch),
                Err(e) => {
                    r.failed_batches += 1;
                    eprintln!("批次 {} 沒有結果：{e}", bi + 1);
                    // the engine itself is gone (no model, no server): stop instead of failing every batch
                    if r.answered == 0 && r.failed_batches >= 2 {
                        return Err(format!("模型沒有回答：{e}"));
                    }
                    break;
                }
            };
            for t in pool.terms.iter_mut().filter(|t| t.verdict.is_none()) {
                if let Some((v, f)) = got.get(&t.key) {
                    t.verdict = Some(v.clone());
                    t.field.clone_from(f);
                    r.answered += 1;
                }
            }
            ask.retain(|k| !got.contains_key(k));
        }
        // written after every batch: a run that stops keeps what it did (merged with a pool
        // rebuilt meanwhile, whose verdicts win where both have one)
        let latest = Pool::load(cache_dir).unwrap_or_default();
        if latest.built != pool.built {
            let mut latest = latest;
            lore_catalog::terms::carry_verdicts(&mut latest, &pool);
            pool = latest;
        }
        settle(&mut pool, filters);
        pool.save(cache_dir).map_err(|e| e.to_string())?;
        progress(((bi + 1) * BATCH).min(total), total);
    }
    r.left = pool.terms.iter().filter(|t| t.verdict.is_none()).count();
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_catalog::terms::Example;
    use lore_llm::types::{Capabilities, LlmResponse};

    /// Answers `term` for every key it is asked about.
    struct Echoing;

    #[async_trait::async_trait]
    impl LlmProvider for Echoing {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { json: true, vision: false, web: false, streaming: false, context: 100_000, needs_network: false }
        }
        async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, lore_llm::LlmError> {
            let keys: Vec<Value> = req.messages[0].text().lines().filter_map(|l| l.strip_prefix("- key: ")).map(|l| l.split('（').next().unwrap().trim()).map(|k| json!({ "key": k, "verdict": "term", "field": "測試" })).collect();
            Ok(LlmResponse { text: json!({ "terms": keys }).to_string(), provider: "mock".into(), sources: vec![], usage: None })
        }
    }

    /// Answers only the first half of what it is asked.
    struct Half;

    #[async_trait::async_trait]
    impl LlmProvider for Half {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
            Echoing.capabilities()
        }
        async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, lore_llm::LlmError> {
            let mut r = Echoing.send(req).await?;
            let v: Value = serde_json::from_str(&r.text).unwrap();
            let a = v["terms"].as_array().unwrap();
            r.text = json!({ "terms": a[..a.len().div_ceil(2)] }).to_string();
            Ok(r)
        }
    }

    #[tokio::test]
    async fn keys_the_model_skips_are_asked_again_at_once() {
        let t = tempfile::tempdir().unwrap();
        Pool { version: 1, built: "x".into(), files: 1, terms: (0..8).map(|i| term(&format!("t{i}"), 0, 1.0)).collect() }.save(t.path()).unwrap();
        let r = run(t.path(), &Half, &[], &Filters::default(), None, |_, _| {}).await.unwrap();
        assert_eq!((r.asked, r.answered, r.left), (8, 6, 2), "half, then half of the rest");
    }

    fn term(k: &str, near: usize, score: f64) -> Term {
        Term { key: k.into(), label: k.into(), files: 1, count: 2, score, cloud_ok: false, examples: vec![Example { id: "a".into(), n: 3, text: format!("The {k} was measured."), cloud_ok: false }], near: vec![], near_known: near, domains: vec![], mastered: false, verdict: None, field: None }
    }

    #[test]
    fn the_request_lists_keys_with_sentences_and_asks_for_json() {
        let t = term("talocrural joint", 1, 2.0);
        let r = request(&[&t], &["biomechanics".into()]);
        assert!(r.system.contains("term") && r.json_schema.is_some());
        let u = r.messages[0].text();
        assert!(u.contains("讀者的領域：biomechanics") && u.contains("- key: talocrural joint") && u.contains("「The talocrural joint was measured.」"), "{u}");
    }

    #[test]
    fn only_asked_keys_and_known_verdicts_are_taken() {
        let (a, mut b) = (term("a", 0, 1.0), term("b", 0, 1.0));
        b.label = "B-Net".into();
        let v = json!({ "terms": [
            { "key": "a", "verdict": "term", "field": " 生物力學 " },
            { "key": "b-net", "verdict": "general", "field": "ignored" },
            { "key": "zzz", "verdict": "term" },
            { "key": "a2", "verdict": "maybe" }
        ] });
        let got = read_answer(&v, &[&a, &b]);
        assert_eq!(got.get("a"), Some(&("term".into(), Some("生物力學".into()))));
        assert_eq!(got.get("b"), Some(&("general".into(), None)));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn everyday_words_are_settled_general_and_go_to_the_ignore_list() {
        let mut terms: Vec<Term> = ["loss", "discriminator", "Masks", "ellrin model", "daidai lab", "results"].iter().map(|k| term(k, 0, 1.0)).collect();
        for t in &mut terms[..3] {
            t.verdict = Some("term".into());
        }
        terms[3].verdict = Some("general".into());
        terms[4].verdict = Some("name".into());
        terms[5].verdict = Some("general".into());
        let mut pool = Pool { version: 1, built: "x".into(), files: 1, terms };
        let f = Filters::default();
        assert_eq!(settle(&mut pool, &f), 2);
        assert_eq!(pool.terms[1].verdict.as_deref(), Some("term"));
        let lines = ignore_lines(&pool, &f, "# header\nDaidai Lab\n");
        assert_eq!(lines, vec!["loss  # LoreShelf", "Masks  # LoreShelf", "ellrin model  # LoreShelf"], "not already listed, not a word the built-in lists drop");
        assert!(Filters::new(&lines).is_ignored("mask"));
    }

    #[tokio::test]
    async fn a_run_saves_each_batch_and_goes_on_from_where_it_stopped() {
        let t = tempfile::tempdir().unwrap();
        let mut terms: Vec<Term> = (0..90).map(|i| term(&format!("t{i:02}"), usize::from(i >= 80), i as f64)).collect();
        terms[5].verdict = Some("junk".into());
        Pool { version: 1, built: "x".into(), files: 1, terms }.save(t.path()).unwrap();
        let mock = Echoing;
        let r = run(t.path(), &mock, &[], &Filters::default(), Some(45), |_, _| {}).await.unwrap();
        assert_eq!((r.asked, r.answered, r.failed_batches, r.left), (45, 45, 0, 44));
        let pool = Pool::load(t.path()).unwrap();
        assert!((80..90).all(|i| pool.terms[i].verdict.is_some()), "terms next to known cards come first");
        assert_eq!(pool.terms[5].verdict.as_deref(), Some("junk"), "a verdict is never asked again");
        let r = run(t.path(), &mock, &[], &Filters::default(), None, |_, _| {}).await.unwrap();
        assert_eq!((r.asked, r.left), (44, 0));
    }
}
