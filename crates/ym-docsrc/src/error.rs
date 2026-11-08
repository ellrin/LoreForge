//! Errors.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "找不到 {tool}（{hint}）。請安裝 poppler（macOS：`brew install poppler`），或用 YM_POPPLER_DIR / Poppler::with_dir 指定位置"
    )]
    ToolMissing { tool: String, hint: String },
    #[error("{tool} 執行失敗（{status}）：{stderr}")]
    ToolFailed {
        tool: String,
        status: String,
        stderr: String,
    },
    #[error("找不到主檔：{0} 底下沒有含 \\documentclass 與 \\begin{{document}} 的 .tex")]
    NoMainTex(PathBuf),
    #[error("無法解析 {what}：{why}")]
    Parse { what: String, why: String },
    #[error("arXiv：{0}")]
    Arxiv(String),
    #[error("不安全的壓縮檔路徑：{0}")]
    UnsafePath(String),
    #[error("找不到素材 {0}")]
    NoAsset(String),
    #[error("素材 {0} 沒有頁面位置（LaTeX 模式的素材要先 attach_pdf）")]
    NoRegion(String),
    #[error("模型回覆不正確：{0}")]
    BadReply(String),
    #[error(transparent)]
    Llm(#[from] ym_llm::LlmError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn parse(what: impl Into<String>, why: impl Into<String>) -> Error {
        Error::Parse {
            what: what.into(),
            why: why.into(),
        }
    }
}
