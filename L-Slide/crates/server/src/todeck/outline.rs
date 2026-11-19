//! The plan's model call (`sc-paper-deck`, streamed so the page sees characters arrive). It
//! runs inside a stage future: when the job is stopped the runner drops the future and the
//! request with it.

use std::sync::atomic::{AtomicU64, Ordering};

use lore_llm::router::RouteError;
use lore_llm::{LlmRequest, StreamEvent};
use serde_json::Value;

use crate::jobs::StageError;
use crate::llm::Llm;

const TEMPERATURE: f32 = 0.3;

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// One line for the page, as `llm.rs` words it.
fn unavailable(e: &RouteError) -> String {
    match e {
        RouteError::Unavailable { skipped, .. } if !skipped.is_empty() => {
            let why: Vec<String> = skipped.iter().map(|k| format!("{}（{}）", k.model, k.reason)).collect();
            first_line(&format!("沒有可用的模型：{}", why.join("；")))
        }
        other => first_line(&other.reason()),
    }
}

/// A JSON request as `task`, streamed: `on_chars(n)` after every piece. One retry when the
/// reply is not JSON of the schema (like `lore_llm::complete_json`). Returns the value and
/// the provider id. A route error is 503 with lore-llm's `alternative` (改用本機／改用 Claude).
pub async fn ask_json(llm: &Llm, task: &str, req: LlmRequest, engine: Option<&str>, on_chars: &(dyn Fn(u64) + Send + Sync)) -> Result<(Value, String), StageError> {
    let route = llm.route(task, engine).await.map_err(|e| {
        let mut extra = e.payload(true);
        let msg = unavailable(&e);
        extra["error"] = msg.clone().into();
        StageError { status: 503, message: msg, extra }
    })?;
    let mut req = route.apply(req.temperature(TEMPERATURE));
    let schema = req.json_schema.clone().unwrap_or(Value::Null);
    let total = AtomicU64::new(0);
    let mut why = String::new();
    for attempt in 0..2 {
        let on = |ev: StreamEvent| {
            if let StreamEvent::Text { text } = ev {
                let n = total.fetch_add(text.chars().count() as u64, Ordering::Relaxed) + text.chars().count() as u64;
                on_chars(n);
            }
        };
        let r = route.provider.stream(&req, &on).await.map_err(|e| StageError::new(503, first_line(&format!("模型沒有回答：{e}"))))?;
        match lore_llm::json::extract_json(&r.text) {
            Some(v) => match lore_llm::json::validate(&v, &schema) {
                Ok(()) => {
                    let id = route.provider.id();
                    lore_llm::shared::record_used(task, &id);
                    return Ok((v, id));
                }
                Err(e) => why = e,
            },
            None => why = "不是 JSON".into(),
        }
        if attempt == 0 {
            req.system.push_str("\n\nReply with exactly one JSON value that matches the given schema, and nothing else.");
        }
    }
    Err(StageError::new(502, first_line(&format!("模型的大綱無法使用：{why}"))))
}
