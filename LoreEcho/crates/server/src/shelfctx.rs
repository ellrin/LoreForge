//! The author's files in the conversation. While LoreShelf runs (ports.json `ls`), each turn asks
//! it which files the message is probably about (`GET /api/search?loose=1`, a local call). What it
//! finds is listed under the reply (SSE `files`; a click turns LoreShelf's globe to that file), and
//! the globe turns to the best one right away.
//!
//! File names never leave the machine. A turn for which LoreShelf found files is answered by the
//! local model whenever one can (even when the author's choice is Claude — `api::chat_post`), and
//! only then do the names go into the prompt. If no local model can answer, Claude is told only
//! that LoreShelf found something and listed it.

use lore_llm::LlmRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Files offered per turn.
pub const MAX_FILES: usize = 4;
/// How long LoreShelf may take before the turn goes on without it.
const WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    /// `YYYY-MM-DD` of the time it is filed under (downloaded, else the document's date, else created).
    #[serde(default)]
    pub date: String,
}

/// LoreShelf's address while it runs.
pub fn shelf_url() -> Option<String> {
    lore_home::ports::live().get("ls").map(|p| format!("http://127.0.0.1:{p}"))
}

/// Is the author looking for a file (find, open, where is…)?
pub fn wants_file(text: &str) -> bool {
    let t = text.to_lowercase();
    ["找", "哪裡", "在哪", "打開", "開啟", "檔案", "講義", "下載", "那份", "那個檔", "find", "where is", "open the", "the file"].iter().any(|k| t.contains(k))
}

fn files_of(v: &Value) -> Vec<File> {
    v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let it = &x["item"];
                    let t = &it["times"];
                    let date = ["downloaded", "doc_created", "created", "modified"].iter().find_map(|k| t[*k].as_str()).map(|s| s.chars().take(10).collect()).unwrap_or_default();
                    Some(File { id: it["id"].as_str()?.to_string(), name: it["name"].as_str()?.to_string(), kind: it["kind"].as_str().unwrap_or_default().to_string(), date })
                })
                .take(MAX_FILES)
                .collect()
        })
        .unwrap_or_default()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(WAIT).no_proxy().build().unwrap_or_default()
}

/// The files LoreShelf thinks `text` is about (empty when it is not running or finds nothing).
pub async fn find(base: &str, text: &str) -> Vec<File> {
    let r = client().get(format!("{base}/api/search")).query(&[("loose", "1"), ("limit", "4"), ("q", text)]).send().await;
    match r {
        Ok(r) if r.status().is_success() => r.json::<Value>().await.map(|v| files_of(&v)).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Ask LoreShelf to turn to a file.
pub async fn focus(base: &str, id: &str) -> bool {
    client().post(format!("{base}/api/act")).json(&json!({ "focus": id })).send().await.is_ok_and(|r| r.status().is_success())
}

/// What the model is told. `local`: the reply is written on this machine, so the names may be said.
pub fn note(files: &[File], local: bool, turned: bool) -> String {
    if files.is_empty() {
        return String::new();
    }
    if !local {
        return format!("LoreShelf 在使用者的電腦上找到 {} 個可能相關的檔案，檔名已經列在你的回覆下方（你看不到檔名）。LoreShelf 已經轉到最相關的那一個。如果他在找檔案，請他看 LoreShelf 和下方的清單；不要猜檔名。", files.len());
    }
    let list = files.iter().enumerate().map(|(i, f)| format!("{}. 〈{}〉{}", i + 1, f.name, if f.date.is_empty() { String::new() } else { format!("（{}）", f.date) })).collect::<Vec<_>>().join("\n");
    format!(
        "使用者的電腦上（LoreShelf）有這些可能相關的檔案：\n{list}\n如果他在找檔案，就從這幾個裡面告訴他是哪一個（照檔名說，不要編造清單以外的檔案）{}，其他的列在回覆下方可以點。如果他不是在找檔案，就不用提這些。",
        if turned { "，並說 LoreShelf 已經轉到第一個" } else { "" }
    )
}

/// What LoreShelf found for a turn (asked before the engine is chosen: a turn that carries the
/// author's files is answered on this machine whenever a local model can).
pub struct Found {
    base: String,
    pub files: Vec<File>,
}

/// Ask the running LoreShelf about `text` (`None`: not running, or nothing found).
pub async fn lookup(text: &str) -> Option<Found> {
    lookup_at(&shelf_url()?, text).await
}

/// [`lookup`] at a given address (tests).
pub async fn lookup_at(base: &str, text: &str) -> Option<Found> {
    let files = find(base, text).await;
    (!files.is_empty()).then(|| Found { base: base.to_string(), files })
}

/// The shelf step of one chat turn, run by `chat::stream` after the KnowledgeBase step.
pub struct Pre {
    found: Found,
    local: bool,
}

impl Pre {
    /// `local` = the reply is written by a model on this machine (then it may know the names).
    pub fn new(found: Found, local: bool) -> Pre {
        Pre { found, local }
    }

    pub async fn run(self, mut req: LlmRequest, send: impl Fn(&str, Value)) -> LlmRequest {
        let Found { base, files } = self.found;
        // the globe turns to the best one right away (author, 2026-11-13: no click needed)
        let turned = focus(&base, &files[0].id).await;
        req.system.push_str("\n\n");
        req.system.push_str(&note(&files, self.local, turned));
        send("files", json!({ "files": files, "turned": turned, "local": self.local }));
        req
    }
}

/// `POST /api/shelf/focus { id }`: the page's click on a listed file.
pub async fn focus_post(axum::extract::State(s): axum::extract::State<crate::Shared>, axum::Json(b): axum::Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !s.shelf.load(std::sync::atomic::Ordering::Relaxed) {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({ "error": "LoreShelf 沒有開" }))).into_response();
    }
    let Some(id) = b["id"].as_str() else { return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "需要 id" }))).into_response() };
    let Some(base) = shelf_url() else { return (axum::http::StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({ "error": "LoreShelf 沒有開" }))).into_response() };
    axum::Json(json!({ "ok": focus(&base, id).await })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> File {
        File { id: format!("id-{name}"), name: name.into(), kind: "pdf".into(), date: "2026-10-05".into() }
    }

    #[test]
    fn looking_for_a_file_is_recognised() {
        assert!(wants_file("我想找找看我最近下載的一個生物力學的講義"));
        assert!(wants_file("Where is the gait analysis paper?"));
        assert!(!wants_file("這個公式為什麼成立"));
    }

    #[test]
    fn names_reach_only_a_local_model() {
        let files = vec![f("biomechanics-notes.pdf"), f("hw2.pdf")];
        let local = note(&files, true, true);
        assert!(local.contains("1. 〈biomechanics-notes.pdf〉（2026-10-05）") && local.contains("2. 〈hw2.pdf〉") && local.contains("已經轉到第一個"));
        let cloud = note(&files, false, true);
        assert!(cloud.contains("2 個") && !cloud.contains("biomechanics") && !cloud.contains("hw2"), "{cloud}");
        assert_eq!(note(&[], true, false), "");
    }

    #[test]
    fn shelf_answers_become_files() {
        let v = json!({ "items": [
            { "item": { "id": "a", "name": "x.pdf", "kind": "pdf", "times": { "created": "2026-01-02T03:04:05+08:00", "downloaded": "2026-02-03T00:00:00+08:00" } }, "via": ["name"] },
            { "item": { "name": "no id" } },
        ] });
        assert_eq!(files_of(&v), vec![File { id: "a".into(), name: "x.pdf".into(), kind: "pdf".into(), date: "2026-02-03".into() }]);
    }

    #[tokio::test]
    async fn a_turn_asks_the_shelf_lists_the_files_and_turns_to_the_first() {
        use axum::{Json, Router, routing::{get, post}};
        use std::sync::{Arc, Mutex};
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let (a1, a2) = (asked.clone(), asked.clone());
        let app = Router::new()
            .route("/api/search", get(move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let a = a1.clone();
                async move {
                    a.lock().unwrap().push(format!("search loose={} q={}", q.get("loose").cloned().unwrap_or_default(), q.get("q").cloned().unwrap_or_default()));
                    Json(json!({ "items": [{ "item": { "id": "f1", "name": "生物力學期中考筆記.pdf", "kind": "pdf", "times": { "created": "2026-10-05T00:00:00+08:00" } } }] }))
                }
            }))
            .route("/api/act", post(move |Json(b): Json<Value>| {
                let a = a2.clone();
                async move {
                    a.lock().unwrap().push(format!("act {}", b["focus"].as_str().unwrap_or_default()));
                    Json(json!({ "ok": true }))
                }
            }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

        let sent: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let s2 = sent.clone();
        let req = LlmRequest::prompt("你是夥伴。", "幫我找生物力學的講義");
        let found = lookup_at(&base, "幫我找生物力學的講義").await.expect("found");
        let out = Pre::new(found, true).run(req, move |n, d| s2.lock().unwrap().push((n.to_string(), d))).await;
        assert!(out.system.contains("〈生物力學期中考筆記.pdf〉") && out.system.contains("已經轉到第一個"), "{}", out.system);
        assert_eq!(*asked.lock().unwrap(), vec!["search loose=1 q=幫我找生物力學的講義".to_string(), "act f1".to_string()]);
        let sent = sent.lock().unwrap();
        assert_eq!(sent[0].0, "files");
        assert_eq!((sent[0].1["files"][0]["id"].as_str(), sent[0].1["turned"].as_bool()), (Some("f1"), Some(true)));

        // not asking for a file: the globe turns all the same; a cloud model never gets the name
        let out = Pre::new(lookup_at(&base, "生物力學是什麼").await.unwrap(), false).run(LlmRequest::prompt("", "生物力學是什麼"), |_, _| {}).await;
        assert!(!out.system.contains("生物力學期中考筆記") && out.system.contains("1 個"));
        assert_eq!(asked.lock().unwrap().last().unwrap(), "act f1");
    }
}
