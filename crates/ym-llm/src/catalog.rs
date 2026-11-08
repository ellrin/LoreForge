//! The shared model catalog (`catalog.json`): cloud models (Claude) and
//! local models (GGUF brains for llama-server, plus macOS ears and mouth).
//! One schema for every app; an app may ship or load its own file of the same
//! shape (e.g. JELearner hides `uncensored` models).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const BUNDLED: &str = include_str!("../catalog.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    #[serde(default)]
    pub note: String,
    /// Memory budget for brain + ears + mouth (GB).
    #[serde(default = "default_budget")]
    pub budget_gb: f64,
    pub defaults: Defaults,
    pub cloud: Cloud,
    pub models: Vec<Model>,
}

fn default_budget() -> f64 {
    20.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    /// Default local brain (a model id).
    pub brain: String,
    #[serde(default)]
    pub ears: Option<String>,
    #[serde(default)]
    pub mouth: Option<String>,
}

/// Cloud models offered in settings; first-run defaults come from here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cloud {
    pub provider: String,
    /// Model for generation work.
    pub default: String,
    #[serde(default)]
    pub effort: Option<String>,
    /// Model for answers the user waits for; `None` = `default`.
    #[serde(default)]
    pub realtime: Option<String>,
    #[serde(default)]
    pub realtime_effort: Option<String>,
    #[serde(default)]
    pub note: String,
    /// Short names used in model refs (`claude:sonnet`) → model ids.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    pub models: Vec<CloudModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudModel {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default = "default_cloud_context")]
    pub context: u32,
}

fn default_cloud_context() -> u32 {
    200_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    /// `brain` (LLM) | `ears` (speech recognition) | `mouth` (speech).
    #[serde(default = "default_role")]
    pub role: String,
    pub name: String,
    pub variant: String,
    #[serde(default)]
    pub summary: String,
    /// Hugging Face `owner/repo`; files live in `<home>/gguf/<owner>/<repo>/`.
    pub repo: String,
    pub file_suffix: String,
    #[serde(default)]
    pub mmproj_suffix: Option<String>,
    pub approx_gb: f64,
    pub min_memory_gb: f64,
    #[serde(default)]
    pub context: Option<u32>,
    /// `llama-server` | `macos`.
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default)]
    pub license: String,
    /// Reads images (has a vision projector).
    #[serde(default)]
    pub vision: bool,
    /// Refusals removed (abliterated): tagged, never recommended.
    #[serde(default)]
    pub uncensored: bool,
}

fn default_role() -> String {
    "brain".into()
}
fn default_runtime() -> String {
    "llama-server".into()
}

impl Model {
    pub fn context_or_default(&self) -> u32 {
        self.context.unwrap_or(16384)
    }
    pub fn is_brain(&self) -> bool {
        self.role == "brain" && self.runtime == "llama-server"
    }
}

impl Catalog {
    pub fn bundled() -> Catalog {
        serde_json::from_str(BUNDLED).expect("ym-llm catalog.json is valid")
    }

    pub fn parse(json: &str) -> Result<Catalog, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// A user's own catalog replaces the bundled one when it parses.
    pub fn load(override_path: Option<&Path>) -> Catalog {
        override_path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| Catalog::parse(&s).ok())
            .unwrap_or_else(Catalog::bundled)
    }

    pub fn get(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Local LLMs, optionally without abliterated ones (household apps).
    pub fn brains(&self, include_uncensored: bool) -> impl Iterator<Item = &Model> {
        self.models.iter().filter(move |m| m.is_brain() && (include_uncensored || !m.uncensored))
    }

    /// A cloud alias (`sonnet`) or id (`claude-sonnet-5-5`) → the model id.
    pub fn cloud_model(&self, alias_or_id: &str) -> Option<&CloudModel> {
        let id = self.cloud.aliases.get(alias_or_id).map(String::as_str).unwrap_or(alias_or_id);
        self.cloud.models.iter().find(|m| m.id == id)
    }

    /// A local ref: an exact id, or an id prefix (`gemma-4-12b` →
    /// `gemma-4-12b-q8` / `-q4`). Among prefix matches: ones `installed`
    /// says are on disk first, then catalog order; abliterated models only
    /// when named exactly.
    pub fn resolve_local(&self, r: &str, installed: impl Fn(&Model) -> bool) -> Option<&Model> {
        if let Some(m) = self.get(r).filter(|m| m.is_brain()) {
            return Some(m);
        }
        let mut hits: Vec<&Model> = self.brains(false).filter(|m| m.id.starts_with(r)).collect();
        hits.sort_by_key(|m| !installed(m)); // stable: catalog order within each group
        hits.into_iter().next()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub models: Vec<String>,
    pub total_gb: f64,
    pub budget_gb: f64,
    /// About what macOS lets the GPU use by default (~2/3 of unified memory
    /// up to 36 GB, ~3/4 above). An M6 32 GB actually reports 26.8 GB, so
    /// this is conservative.
    pub gpu_usable_gb: f64,
    pub fits: bool,
}

pub fn gpu_usable(memory_gb: f64) -> f64 {
    if memory_gb > 36.0 { memory_gb * 0.75 } else { memory_gb * 2.0 / 3.0 }
}

/// Models together against the budget and the GPU share of memory.
pub fn plan(cat: &Catalog, ids: &[&str], memory_gb: f64) -> Plan {
    let total: f64 = ids.iter().filter_map(|id| cat.get(id)).map(|m| m.approx_gb).sum();
    let usable = gpu_usable(memory_gb);
    Plan {
        models: ids.iter().map(|s| s.to_string()).collect(),
        total_gb: (total * 100.0).round() / 100.0,
        budget_gb: cat.budget_gb,
        gpu_usable_gb: (usable * 10.0).round() / 10.0,
        fits: total <= cat.budget_gb && total <= usable,
    }
}

/// The memory this machine has for models (the app measures it).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Machine {
    pub memory_gb: f64,
    /// Discrete GPU memory, when there is one.
    pub gpu_memory_gb: Option<f64>,
}

/// The default brain when this machine can hold it, else the most demanding
/// brain that fits (`min_memory_gb` tracks the tier better than file size).
/// Abliterated models are never recommended.
pub fn recommend(cat: &Catalog, m: &Machine) -> Option<String> {
    let pool = m.memory_gb.max(m.gpu_memory_gb.unwrap_or(0.0));
    let fits: Vec<&Model> = cat.brains(false).filter(|x| pool + 0.5 >= x.min_memory_gb).collect();
    fits.iter()
        .find(|x| x.id == cat.defaults.brain)
        .or_else(|| fits.iter().max_by(|a, b| a.min_memory_gb.total_cmp(&b.min_memory_gb).then(a.approx_gb.total_cmp(&b.approx_gb))))
        .map(|x| x.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_is_consistent() {
        let c = Catalog::bundled();
        assert!(c.get(&c.defaults.brain).is_some_and(|m| m.is_brain()));
        for id in [c.defaults.ears.as_ref().unwrap(), c.defaults.mouth.as_ref().unwrap()] {
            assert!(c.get(id).is_some(), "{id}");
        }
        assert!(c.cloud.models.iter().any(|m| m.id == c.cloud.default));
        for id in c.cloud.aliases.values() {
            assert!(c.cloud.models.iter().any(|m| &m.id == id), "{id}");
        }
        assert!(c.brains(true).all(|m| !m.repo.is_empty() && !m.file_suffix.is_empty()));
        assert!(c.brains(true).any(|m| m.uncensored) && c.brains(false).all(|m| !m.uncensored));
        assert!(c.brains(false).all(|m| m.vision == m.mmproj_suffix.is_some()));
        let mut ids: Vec<&str> = c.models.iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), c.models.len(), "unique ids");
    }

    #[test]
    fn plan_fits_a_32gb_mac() {
        let c = Catalog::bundled();
        let d = &c.defaults;
        let p = plan(&c, &[&d.brain, d.ears.as_deref().unwrap(), d.mouth.as_deref().unwrap()], 32.0);
        assert!(p.fits, "{p:?}");
        assert!((p.gpu_usable_gb - 21.3).abs() < 0.1);
        assert!(!plan(&c, &["gemma-4-12b-q8"], 16.0).fits);
    }

    #[test]
    fn recommends_by_memory_never_uncensored() {
        let c = Catalog::bundled();
        let mac = |g: f64| Machine { memory_gb: g, gpu_memory_gb: None };
        assert_eq!(recommend(&c, &mac(32.0)).as_deref(), Some("gemma-4-12b-q8"));
        assert_eq!(recommend(&c, &mac(16.0)).as_deref(), Some("gemma-4-12b-q4"));
        assert_eq!(recommend(&c, &mac(12.0)).as_deref(), Some("qwen3.5-4b-q8"));
        assert_eq!(recommend(&c, &mac(8.0)), None);
        let mut only = c.clone();
        only.models.retain(|m| m.uncensored);
        assert_eq!(recommend(&only, &mac(128.0)), None);
    }

    #[test]
    fn resolves_aliases_and_prefixes() {
        let c = Catalog::bundled();
        assert_eq!(c.cloud_model("sonnet").unwrap().id, "claude-sonnet-5-5");
        assert_eq!(c.cloud_model("claude-haiku-5-5").unwrap().id, "claude-haiku-5-5");
        assert!(c.cloud_model("gpt").is_none());
        assert_eq!(c.resolve_local("qwen3.5-9b-q8", |_| false).unwrap().id, "qwen3.5-9b-q8");
        assert_eq!(c.resolve_local("qwen3.5-9b", |_| false).unwrap().id, "qwen3.5-9b-q8");
        // Prefix: catalog order, but a downloaded one wins.
        assert_eq!(c.resolve_local("gemma-4-12b", |_| false).unwrap().id, "gemma-4-12b-q4");
        assert_eq!(c.resolve_local("gemma-4-12b", |m| m.id.ends_with("q8")).unwrap().id, "gemma-4-12b-q8");
        assert!(c.resolve_local("gemma-4-12b-a", |_| true).is_none(), "abliterated only by exact id");
        assert!(c.resolve_local("apple-speech", |_| true).is_none(), "not a brain");
    }
}
