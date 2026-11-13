//! Downloaded PDFs into LoreReader's library through LoreReader's own
//! `POST /api/papers/import {path}` (it copies the file into its sha-addressed library; the
//! cache file can be wiped afterwards). LoreReader not reachable: the rest are `pending` (the
//! caller keeps them as `pending_imports` and tries again later); never an error.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Error;
use crate::http::{Http, Request};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Imported {
    pub path: PathBuf,
    /// LoreReader's sha256 of the paper.
    pub sha: String,
    /// Not in the library before.
    pub new: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportReport {
    pub imported: Vec<Imported>,
    /// Not sent: LoreReader did not answer. Try again when it runs.
    pub pending: Vec<PathBuf>,
    /// LoreReader refused the file (`{error}` of its answer).
    pub failed: Vec<(PathBuf, String)>,
}

/// Import `files` (absolute paths) into the LoreReader at `base` (`http://127.0.0.1:7890`).
/// The first "no network" failure marks that file and all after it as pending.
pub async fn import_into_reader(http: &dyn Http, base: &str, files: &[PathBuf]) -> ImportReport {
    let url = format!("{}/api/papers/import", base.trim_end_matches('/'));
    let mut rep = ImportReport::default();
    for (i, f) in files.iter().enumerate() {
        let path = f.canonicalize().unwrap_or_else(|_| f.clone());
        match http.send(Request::post_json(&url, &json!({ "path": path.to_string_lossy() }))).await {
            Err(Error::Offline(_)) => {
                rep.pending.extend(files[i..].iter().cloned());
                break;
            }
            Err(e) => rep.failed.push((f.clone(), e.to_string())),
            Ok(r) => {
                let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
                let sha = v.pointer("/paper/sha256").and_then(Value::as_str);
                match (r.status, sha) {
                    (200..=299, Some(sha)) => rep.imported.push(Imported { path: f.clone(), sha: sha.to_string(), new: v["new"].as_bool().unwrap_or(false) }),
                    (s, _) => rep.failed.push((f.clone(), v["error"].as_str().map(String::from).unwrap_or_else(|| format!("LoreReader：{s}")))),
                }
            }
        }
    }
    rep
}
