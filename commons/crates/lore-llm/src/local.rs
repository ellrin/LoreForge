//! Local models in the shared folder and the llama-server lifecycle
//! (JELearner `local_llm.rs` + PaperComposer `models.rs`).
//!
//! Every app on the machine keeps model weights and llama.cpp in one folder,
//! `LOCAL_LLM_HOME` (or a configured path, since an app opened from Finder
//! does not see shell variables):
//!
//! ```text
//! <home>/gguf/<owner>/<repo>/<file>     what `hf download --local-dir` writes
//! <home>/runtimes/llama.cpp/…/llama-server
//! ```
//!
//! An app may keep its own folders instead ([`Layout::new`]; PaperComposer
//! without `LOCAL_LLM_HOME` uses `<data>/models` and `<data>/llama.cpp`).
//!
//! Nothing here downloads (PaperComposer's setup or `hf download` fill the
//! folder) and nothing here starts a server: [`crate::shared_runtime`] does, one
//! llama-server per machine shared by every app, with [`server_args`]. No thinking
//! (`--reasoning-budget 0`): every task wants a fixed JSON shape quickly.
//!
//! Real llama-server runs are `#[ignore]` tests; **not tested on real
//! hardware** from this crate (JELearner measured Qwen3.5 4B on an M6 32 GB).

use crate::catalog::{Catalog, Model};
use std::path::{Path, PathBuf};

/// The shared folder: the configured one, else `LOCAL_LLM_HOME`.
pub fn home(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("LOCAL_LLM_HOME").filter(|v| !v.is_empty()).map(PathBuf::from))
}

/// [`home`], then `fallback` (`engine.toml` `[local] home`): configured, env, shared file.
pub fn home_or(configured: Option<&Path>, fallback: Option<&Path>) -> Option<PathBuf> {
    home(configured).or_else(|| fallback.filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf))
}

pub fn gguf_dir(home: &Path) -> PathBuf {
    home.join("gguf")
}

pub fn runtime_dir(home: &Path) -> PathBuf {
    home.join("runtimes").join("llama.cpp")
}

/// Files under `dir` (not in `.cache`, where hf keeps partial downloads)
/// whose path ends with `suffix`, shortest first (the plain file, not a split
/// part or an MTP head in a subfolder).
fn find_under(dir: &Path, suffix: &str) -> Option<PathBuf> {
    let want = suffix.to_lowercase();
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if e.file_name() != ".cache" {
                    stack.push(p);
                }
            } else if p.to_string_lossy().to_lowercase().ends_with(&want) {
                hits.push(p);
            }
        }
    }
    hits.sort_by_key(|p| p.as_os_str().len());
    hits.into_iter().next()
}

/// Where model weights and llama.cpp live. The shared layout is
/// [`Layout::shared`]; an app with its own folder from before
/// `LOCAL_LLM_HOME` (PaperComposer: `<data>/models`, `<data>/llama.cpp`)
/// passes its folders with [`Layout::new`]. Weights are always laid out as
/// `<gguf>/<owner>/<repo>/<file>` (what `hf download --local-dir` writes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Model weights: `<gguf>/<owner>/<repo>/<file>`.
    pub gguf: PathBuf,
    /// llama.cpp: `llama-server` anywhere below this folder.
    pub runtime: PathBuf,
}

impl Layout {
    /// The shared folder: `<home>/gguf`, `<home>/runtimes/llama.cpp`.
    pub fn shared(home: &Path) -> Layout {
        Layout { gguf: gguf_dir(home), runtime: runtime_dir(home) }
    }

    /// Any two folders (an app's own layout).
    pub fn new(gguf: impl Into<PathBuf>, runtime: impl Into<PathBuf>) -> Layout {
        Layout { gguf: gguf.into(), runtime: runtime.into() }
    }

    /// The model's weights, when downloaded.
    pub fn find_model(&self, m: &Model) -> Option<PathBuf> {
        if m.repo.is_empty() || m.file_suffix.is_empty() {
            return None;
        }
        find_under(&self.gguf.join(&m.repo), &m.file_suffix)
    }

    /// The vision projector next to the weights, when the catalog names one.
    pub fn find_mmproj(&self, m: &Model) -> Option<PathBuf> {
        find_under(&self.gguf.join(&m.repo), m.mmproj_suffix.as_deref()?)
    }

    /// `llama-server` below the runtime folder.
    pub fn find_runtime(&self) -> Option<PathBuf> {
        let exe = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
        let mut stack = vec![self.runtime.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if e.file_name() == exe {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Brains of the catalog that are on disk.
    pub fn installed<'a>(&self, cat: &'a Catalog, include_uncensored: bool) -> Vec<&'a Model> {
        cat.brains(include_uncensored).filter(|m| self.find_model(m).is_some()).collect()
    }

    /// Why a model cannot start, in the user's words.
    pub fn missing(&self, m: &Model) -> Option<String> {
        if self.find_runtime().is_none() {
            return Some(format!(
                "還沒有 llama.cpp：{} 裡找不到 llama-server。請用 PaperComposer 的「AI 與同步」安裝一次，或把 llama.cpp 解壓到這個資料夾",
                self.runtime.display()
            ));
        }
        if self.find_model(m).is_none() {
            return Some(format!(
                "還沒下載 {} {}：hf download {} --include \"*{}\" --local-dir \"{}\"",
                m.name,
                m.variant,
                m.repo,
                m.file_suffix,
                self.gguf.join(&m.repo).display()
            ));
        }
        None
    }
}

/// The model's weights in the shared folder, when downloaded.
pub fn find_model(home: &Path, m: &Model) -> Option<PathBuf> {
    Layout::shared(home).find_model(m)
}

/// The vision projector next to the weights, when the catalog names one.
pub fn find_mmproj(home: &Path, m: &Model) -> Option<PathBuf> {
    Layout::shared(home).find_mmproj(m)
}

/// `llama-server` in the shared runtimes folder.
pub fn find_runtime(home: &Path) -> Option<PathBuf> {
    Layout::shared(home).find_runtime()
}

/// Brains of the catalog that are on disk.
pub fn installed<'a>(home: &Path, cat: &'a Catalog, include_uncensored: bool) -> Vec<&'a Model> {
    Layout::shared(home).installed(cat, include_uncensored)
}

/// Why a model cannot start, in the user's words.
pub fn missing(home: &Path, m: &Model) -> Option<String> {
    Layout::shared(home).missing(m)
}

/// llama-server arguments for one model (no thinking, all layers on the GPU,
/// the catalog's context, the projector when there is one).
pub fn server_args(weights: &Path, mmproj: Option<&Path>, port: u16, context: u32) -> Vec<String> {
    let mut a: Vec<String> = vec!["-m".into(), weights.display().to_string()];
    a.extend(
        ["--host", "127.0.0.1", "--port", &port.to_string(), "-c", &context.to_string(), "-ngl", "999", "--jinja", "--reasoning-budget", "0"]
            .map(String::from),
    );
    if let Some(mm) = mmproj {
        a.extend(["--mmproj".into(), mm.display().to_string()]);
    }
    a
}

/// Minutes unused before the local model is released, unless set otherwise
/// (`[local] idle_minutes` of the shared engine file).
pub const DEFAULT_IDLE_MINUTES: u64 = 10;

#[cfg(test)]
mod tests {
    #[test]
    fn home_order_is_config_env_then_shared_file() {
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        assert_eq!(home_or(Some(a), Some(b)), Some(a.to_path_buf()));
        match std::env::var_os("LOCAL_LLM_HOME").filter(|v| !v.is_empty()) {
            Some(env) => assert_eq!(home_or(None, Some(b)), Some(PathBuf::from(env))),
            None => assert_eq!(home_or(None, Some(b)), Some(b.to_path_buf())),
        }
    }

    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn finds_files_in_the_hf_layout() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").expect("Qwen 4B in the catalog");
        assert!(find_model(home, m).is_none());
        assert!(missing(home, m).unwrap().contains("llama.cpp"));

        touch(&home.join("runtimes/llama.cpp/llama-b1/llama-server"));
        let repo = home.join("gguf").join(&m.repo);
        // A partial download in hf's .cache does not count; the plain file wins over an MTP head.
        touch(&repo.join(".cache/huggingface/download/Qwen3.5-4B-Q8_0.gguf"));
        assert!(missing(home, m).unwrap().contains("hf download"));
        touch(&repo.join("MTP/mtp-Qwen3.5-4B-Q8_0.gguf"));
        touch(&repo.join("Qwen3.5-4B-Q8_0.gguf"));
        touch(&repo.join("mmproj-BF16.gguf"));
        assert_eq!(find_model(home, m).unwrap(), repo.join("Qwen3.5-4B-Q8_0.gguf"));
        assert_eq!(find_mmproj(home, m).unwrap(), repo.join("mmproj-BF16.gguf"));
        assert_eq!(find_runtime(home).unwrap(), home.join("runtimes/llama.cpp/llama-b1/llama-server"));
        assert!(missing(home, m).is_none());
        assert_eq!(installed(home, &cat, false).iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["qwen3.5-4b-q8"]);
    }

    #[test]
    fn an_app_layout_finds_files_in_its_own_folders() {
        // PaperComposer before LOCAL_LLM_HOME: <data>/models/<owner>/<repo>, <data>/llama.cpp.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let l = Layout::new(data.join("models"), data.join("llama.cpp"));
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").unwrap();
        let e = l.missing(m).unwrap();
        assert!(e.contains("llama.cpp"), "{e}");
        touch(&data.join("llama.cpp/build/bin/llama-server"));
        touch(&data.join("models").join(&m.repo).join("Qwen3.5-4B-Q8_0.gguf"));
        assert!(l.missing(m).is_none());
        assert_eq!(l.find_runtime().unwrap(), data.join("llama.cpp/build/bin/llama-server"));
        assert_eq!(l.installed(&cat, true).len(), 1);
        // The shared layout over the same folder sees none of it.
        assert!(find_model(data, m).is_none() && find_runtime(data).is_none());
        assert_eq!(Layout::shared(data), Layout::new(data.join("gguf"), data.join("runtimes/llama.cpp")));
    }

    #[test]
    fn server_args_turn_thinking_off() {
        let a = server_args(Path::new("/m.gguf"), Some(Path::new("/mm.gguf")), 9000, 32768);
        let has = |k: &str, v: &str| a.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(has("--reasoning-budget", "0") && has("-c", "32768") && has("--port", "9000") && has("--mmproj", "/mm.gguf"));
        assert!(has("--host", "127.0.0.1"), "never listens beyond this machine");
    }

    /// Real hardware: llama-server and Qwen3.5 4B from the shared folder, one
    /// JSON answer. Run: `cargo test -p lore-llm real_local -- --ignored`.
    #[tokio::test]
    #[ignore = "needs LOCAL_LLM_HOME with llama.cpp and Qwen3.5 4B"]
    async fn real_local_model_answers() {
        use crate::types::LlmRequest;
        let home = home(None).expect("LOCAL_LLM_HOME");
        let cat = Catalog::bundled();
        let m = cat.get("qwen3.5-4b-q8").unwrap();
        let reg = tempfile::tempdir().unwrap();
        let rt = crate::shared_runtime::SharedRuntime::new("test", reg.path(), Default::default());
        let p = crate::shared_runtime::SharedLocal {
            rt: rt.clone(),
            layout: Some(Layout::shared(&home)),
            model: m.clone(),
            priority: crate::shared_runtime::Priority::Interactive,
            zh_tw: false,
            vision: false,
        };
        let schema = serde_json::json!({"type":"object","properties":{"word":{"type":"string"}},"required":["word"]});
        let req = LlmRequest::prompt("Reply with JSON only.", "Give one English word that means 'happy'.").max_tokens(200).json(schema);
        let v = crate::json::complete_json(&p, &req).await.unwrap();
        assert!(v["word"].as_str().is_some_and(|w| !w.is_empty()));
        rt.shutdown();
    }
}
