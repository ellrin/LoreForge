//! Model engines. For now: any OpenAI-compatible chat endpoint, which covers
//! llama.cpp server, Ollama and LM Studio running Gemma locally or on a home
//! machine. Without one, writing falls back to Claude (see `writing_complete`).

use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EngineConfig {
    /// Base URL ending in /v1, e.g. http://127.0.0.1:8080/v1
    pub url: Option<String>,
    pub model: Option<String>,
}

impl EngineConfig {
    /// Environment overrides the config file.
    pub fn resolve(file: Option<EngineConfig>) -> EngineConfig {
        let file = file.unwrap_or_default();
        EngineConfig {
            url: std::env::var("PAPERCOMPOSER_LLM_URL").ok().or(file.url),
            model: std::env::var("PAPERCOMPOSER_LLM_MODEL").ok().or(file.model),
        }
    }
}

#[derive(Serialize)]
pub struct EngineStatus {
    pub connected: bool,
    pub name: String,
}

pub struct Engine {
    cfg: EngineConfig,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Msg<'a>>,
    temperature: f32,
    /// A ceiling, so a model that loops cannot run for minutes.
    max_tokens: u32,
    response_format: serde_json::Value,
}

#[derive(Serialize)]
struct Msg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: ChoiceMsg,
}
#[derive(Deserialize)]
struct ChoiceMsg {
    content: String,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Engine {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .expect("http client");
        Engine { cfg, http }
    }

    pub fn configured(&self) -> bool {
        self.cfg.url.is_some()
    }

    pub async fn status(&self) -> EngineStatus {
        let Some(url) = &self.cfg.url else {
            return EngineStatus { connected: false, name: "未連接模型".into() };
        };
        let ok = self
            .http
            .get(format!("{}/models", url.trim_end_matches('/')))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        let name = self.cfg.model.clone().unwrap_or_else(|| "本機模型".into());
        EngineStatus { connected: ok, name }
    }

    /// Send a prompt; returns the raw reply text.
    pub async fn complete(&self, prompt: &Prompt, extra: Option<&str>) -> Result<String, String> {
        let url = self.cfg.url.as_deref().ok_or("no engine configured")?;
        let mut user = prompt.user.clone();
        if let Some(e) = extra {
            user += "\n";
            user += e;
        }
        let body = ChatRequest {
            model: self.cfg.model.as_deref().unwrap_or("local"),
            messages: vec![
                Msg { role: "system", content: &prompt.system },
                Msg { role: "user", content: &user },
            ],
            temperature: 0.2,
            max_tokens: 4096,
            response_format: serde_json::json!({ "type": "json_object" }),
        };
        let resp = self
            .http
            .post(format!("{}/chat/completions", url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("engine unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("engine returned {}", resp.status()));
        }
        let parsed: ChatResponse = resp.json().await.map_err(|e| format!("bad engine reply: {e}"))?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| "engine returned no choices".into())
    }
}
