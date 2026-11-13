//! The job's model calls, behind one trait so the whole pipeline runs on a mock in tests.
//! [`LlmModel`] is the real one: PaperComposer's lore-llm tasks (`tasks.toml`
//! `pc-book-plan`, `pc-book-suggest`, `pc-book-chapter`), streamed so the page sees
//! characters arrive, one retry when the reply is not JSON of the schema.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lore_jobs::StageError;
use lore_llm::StreamEvent;
use pc_core::compose::Prompt;
use serde_json::Value;

use crate::llm::{EngineChoice, Llm};

pub const TASK_PLAN: &str = "pc-book-plan";
pub const TASK_SUGGEST: &str = "pc-book-suggest";
pub const TASK_CHAPTER: &str = "pc-book-chapter";

/// One JSON request. `engine`: the job's engine choice (`local`, `subscription`, `api`), none = the task's order.
pub struct Ask<'a> {
    pub task: &'a str,
    pub system: String,
    pub user: String,
    pub schema: Value,
    pub max_tokens: u32,
    pub engine: Option<&'a str>,
}

pub type OnChars<'a> = &'a (dyn Fn(u64) + Send + Sync);

#[async_trait::async_trait]
pub trait BookModel: Send + Sync {
    /// The reply as JSON of `schema`, and the provider id that wrote it.
    async fn json(&self, ask: Ask<'_>, on_chars: OnChars<'_>) -> Result<(Value, String), StageError>;
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

pub struct LlmModel(pub Arc<Llm>);

#[async_trait::async_trait]
impl BookModel for LlmModel {
    async fn json(&self, ask: Ask<'_>, on_chars: OnChars<'_>) -> Result<(Value, String), StageError> {
        let choice: Option<EngineChoice> = ask.engine.and_then(|e| serde_json::from_value(Value::String(e.to_string())).ok());
        let route = self.0.route(ask.task, choice).await.map_err(|e| {
            // route() gives lore-llm's payload as JSON when another engine could run
            let v: Value = serde_json::from_str(&e).unwrap_or(Value::Null);
            let msg = first_line(v["error"].as_str().unwrap_or(&e));
            let mut extra = if v.is_object() { v } else { serde_json::json!({}) };
            extra["error"] = msg.clone().into();
            StageError { status: 503, message: msg, extra }
        })?;
        let mut req = crate::llm::request(&route, &Prompt { system: ask.system, user: ask.user }, None, Some(ask.schema.clone())).max_tokens(ask.max_tokens);
        let total = AtomicU64::new(0);
        let mut why = String::new();
        for attempt in 0..2 {
            let on = |ev: StreamEvent| {
                if let StreamEvent::Text { text } = ev {
                    let k = text.chars().count() as u64;
                    on_chars(total.fetch_add(k, Ordering::Relaxed) + k);
                }
            };
            let r = route.provider.stream(&req, &on).await.map_err(|e| StageError::new(503, first_line(&format!("模型沒有回答：{e}"))))?;
            match lore_llm::json::extract_json(&r.text) {
                Some(v) => match lore_llm::json::validate(&v, &ask.schema) {
                    Ok(()) => {
                        let id = route.provider.id();
                        lore_llm::shared::record_used(ask.task, &id);
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
        Err(StageError::new(502, first_line(&format!("模型的回答無法使用：{why}"))))
    }
}

/// Tests: answers per task from a queue (the last answer repeats), every request kept.
#[derive(Default)]
pub struct MockModel {
    pub replies: std::sync::Mutex<std::collections::HashMap<String, Vec<Value>>>,
    pub seen: std::sync::Mutex<Vec<(String, String, String)>>,
}

impl MockModel {
    pub fn on(self, task: &str, v: Value) -> Self {
        self.replies.lock().unwrap().entry(task.to_string()).or_default().push(v);
        self
    }

    /// `(task, system, user)` of every request so far.
    pub fn asked(&self, task: &str) -> Vec<(String, String)> {
        self.seen.lock().unwrap().iter().filter(|(t, _, _)| t == task).map(|(_, s, u)| (s.clone(), u.clone())).collect()
    }
}

#[async_trait::async_trait]
impl BookModel for MockModel {
    async fn json(&self, ask: Ask<'_>, on_chars: OnChars<'_>) -> Result<(Value, String), StageError> {
        self.seen.lock().unwrap().push((ask.task.to_string(), ask.system, ask.user));
        let mut r = self.replies.lock().unwrap();
        let q = r.get_mut(ask.task).ok_or_else(|| StageError::new(503, format!("沒有可用的模型：{}", ask.task)))?;
        let v = if q.len() > 1 { q.remove(0) } else { q.first().cloned().unwrap_or(Value::Null) };
        on_chars(v.to_string().chars().count() as u64);
        Ok((v, "mock:book".into()))
    }
}
