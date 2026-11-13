//! `POST /api/export/open`: 開啟 / 在 Finder 顯示 for a file the app just exported.
//! Only files inside the workspace's decks folder (the `exports/` of a deck) with a
//! .pptx / .pdf / .png / .svg extension are accepted: the local API must not open arbitrary files.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::json;

use crate::{ApiError, R, Shared, bad};

#[derive(Deserialize)]
pub struct OpenBody {
    pub path: String,
    #[serde(default)]
    pub reveal: bool,
}

pub async fn export_open(State(s): State<Shared>, Json(b): Json<OpenBody>) -> R {
    let path = std::path::PathBuf::from(&b.path).canonicalize().map_err(|_| bad("找不到檔案"))?;
    let root = s.ws.decks.canonicalize().map_err(|_| bad("找不到簡報資料夾"))?;
    let ext_ok = path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["pptx", "pdf", "png", "svg"].iter().any(|x| e.eq_ignore_ascii_case(x)));
    if !path.starts_with(&root) || !path.is_file() || !ext_ok {
        return Err(ApiError(axum::http::StatusCode::FORBIDDEN, "只能開啟匯出的檔案".into()));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        if b.reveal {
            c.arg("-R");
        }
        c.arg(&path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        if b.reveal {
            c.arg("/select,");
        }
        c.arg(&path);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if b.reveal { path.parent().unwrap_or(&root) } else { &path });
        c
    };
    cmd.spawn().map_err(|e| bad(format!("無法開啟：{e}")))?;
    Ok(Json(json!({ "ok": true })))
}
