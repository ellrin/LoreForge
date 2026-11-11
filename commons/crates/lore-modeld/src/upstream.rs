//! What the broker forwards to: the one llama-server of the machine ([`LocalUpstream`],
//! a [`LocalRuntime`]), or anything else that serves `/v1` (tests use a fake server).

use async_trait::async_trait;
use lore_llm::catalog::Catalog;
use lore_llm::local::{Layout, LocalRuntime};
use serde_json::Value;
use std::sync::Arc;

/// The model server behind the broker.
#[async_trait]
pub trait Upstream: Send + Sync {
    /// The `/v1` base URL of a server running `model` (a catalog id), loading it (and
    /// unloading another one) when needed. The scheduler calls it only when no request
    /// runs on another model.
    async fn ensure(&self, model: &str) -> Result<String, String>;
    /// The model loaded now, if any.
    async fn loaded(&self) -> Option<String>;
    /// Stop the model now.
    async fn release(&self);
    /// For `/api/status`: `{ "state": "idle" | "starting" | "ready" | "error", … }`.
    fn phase(&self) -> Value;
    /// Model ids this upstream can serve (`GET /v1/models`).
    fn models(&self) -> Vec<String>;
}

/// The real thing: one [`LocalRuntime`] over a model folder. **Not tested on real
/// hardware** from this crate (the runtime itself is; see lore-llm).
pub struct LocalUpstream {
    pub runtime: Arc<LocalRuntime>,
    /// `None`: no model folder configured (`LOCAL_LLM_HOME` unset and no `--gguf`).
    pub layout: Option<Layout>,
    pub catalog: Catalog,
}

#[async_trait]
impl Upstream for LocalUpstream {
    async fn ensure(&self, model: &str) -> Result<String, String> {
        let layout = self.layout.as_ref().ok_or("沒有設定共用模型資料夾（LOCAL_LLM_HOME）")?;
        let m = self
            .catalog
            .resolve_local(model, |m| layout.find_model(m).is_some())
            .ok_or_else(|| format!("模型目錄裡沒有本機模型「{model}」"))?;
        Ok(self.runtime.ensure_in(layout, m).await?.url)
    }

    async fn loaded(&self) -> Option<String> {
        self.runtime.current_info().await.map(|i| i.model_id)
    }

    async fn release(&self) {
        self.runtime.release().await;
    }

    fn phase(&self) -> Value {
        serde_json::to_value(self.runtime.phase()).unwrap_or(Value::Null)
    }

    fn models(&self) -> Vec<String> {
        match &self.layout {
            Some(l) => l.installed(&self.catalog, true).into_iter().map(|m| m.id.clone()).collect(),
            None => Vec::new(),
        }
    }
}
