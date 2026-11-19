//! 加入知識庫 from any Lore app, whether L-Brain is open or not.
//!
//! L-Brain's cards are files in its spaces and its write API is a thin layer over kb-core
//! (`create_as`, `update`, `recall`), so a card does not need the app:
//!
//! * L-Brain answers (its port in ports.json, else the given addresses): its API, so its window
//!   and index see the card at once;
//! * nothing listens there: kb-core writes into the member's spaces directly (L-Brain rebuilds its
//!   index from the files when it opens). Never while L-Brain answers: one writer at a time;
//!   an L-Brain that answers wrongly or times out is an error, never a reason to write the files;
//! * no L-Brain and no spaces (tests, isolated examples): `down`, nothing written.
//!
//! Neither path syncs anything: L-Brain does not run git on its spaces either.
//!
//! [`search`] = L-Reader's 一鍵加卡 summary (web search required), shared. `web` feature:
//! [`router`] = `POST /api/lore/card` for `commons/ui/addcard.js` (`LoreAddCard.add`), the
//! selection's 加入知識庫 in L-Paper and L-Slide.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use kb_core::{KbOptions, KnowledgeBase, NodeFields, Outcome, State};
use serde_json::{Value, json};

pub mod search;
#[cfg(feature = "web")]
mod web;
#[cfg(feature = "web")]
pub use web::{NewCard, router};

/// Why a card call failed, ready for the page.
#[derive(Debug, Clone, PartialEq)]
pub struct CardError {
    /// Neither L-Brain nor its folder is reachable.
    pub down: bool,
    /// HTTP status for the page (503 down, 502 L-Brain said no, 400, 404, 409, 500).
    pub status: u16,
    pub message: String,
}

impl CardError {
    fn new(status: u16, message: impl Into<String>) -> CardError {
        CardError { down: false, status, message: message.into() }
    }
    fn down() -> CardError {
        CardError { down: true, status: 503, message: "L-Brain 沒有開，也找不到知識庫的資料夾".into() }
    }
}

impl std::fmt::Display for CardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

type Res<T> = Result<T, CardError>;

/// Where this call goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    Server(String),
    Disk,
}

/// The author's L-Brain, running or not.
#[derive(Clone)]
pub struct Brain {
    spaces: Vec<PathBuf>,
    member: String,
    urls: Vec<String>,
    ports: bool,
    http: reqwest::Client,
}

/// One disk writer per process (two clicks never write the same files at once).
static DISK: Mutex<()> = Mutex::new(());

const DEFAULT_URL: &str = "http://127.0.0.1:7700";

impl Brain {
    /// This machine's L-Brain: its port in ports.json or 127.0.0.1:7700, its spaces and member
    /// as L-Brain itself reads them (`KB_SPACES`, its config.toml, lore-home).
    pub fn system(home: &lore_home::Home) -> Brain {
        Brain::at(Some(DEFAULT_URL), lore_learn::space_dirs(home), &lore_learn::member(home)).with_ports(true)
    }

    /// A fixed address (or none) and fixed spaces; ports.json is not read (tests, `KB_URL`).
    pub fn at(url: Option<&str>, spaces: Vec<PathBuf>, member: &str) -> Brain {
        let http = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(2)).build().unwrap_or_default();
        Brain { spaces, member: member.to_string(), urls: url.map(|u| u.trim_end_matches('/').to_string()).into_iter().collect(), ports: false, http }
    }

    /// No L-Brain at all (isolated examples): every call is `down`.
    pub fn none() -> Brain {
        Brain::at(None, vec![], "me")
    }

    /// Also look for L-Brain's port in ports.json (first).
    pub fn with_ports(mut self, on: bool) -> Brain {
        self.ports = on;
        self
    }

    pub fn spaces(&self) -> &[PathBuf] {
        &self.spaces
    }

    pub fn member(&self) -> &str {
        &self.member
    }

    /// L-Brain's API when it answers, else the folders when there are any.
    pub async fn place(&self) -> Res<Place> {
        let mut cands: Vec<String> = vec![];
        if self.ports
            && let Some(p) = lore_home::ports::live().get("kb")
        {
            cands.push(format!("http://127.0.0.1:{p}"));
        }
        for u in &self.urls {
            if !cands.contains(u) {
                cands.push(u.clone());
            }
        }
        for u in cands {
            match self.http.get(format!("{u}/api/spaces")).timeout(Duration::from_secs(5)).send().await {
                Ok(r) if r.status().is_success() => return Ok(Place::Server(u)),
                Ok(r) => return Err(CardError::new(502, format!("L-Brain 回答 {}", r.status()))),
                Err(e) if e.is_connect() => continue,
                Err(e) => return Err(CardError::new(502, format!("L-Brain 沒有回應：{e}"))),
            }
        }
        if self.spaces.is_empty() { Err(CardError::down()) } else { Ok(Place::Disk) }
    }

    async fn api(&self, req: reqwest::RequestBuilder) -> Res<Value> {
        let res = req.timeout(Duration::from_secs(15)).send().await.map_err(|e| CardError::new(502, format!("L-Brain 沒有回應：{e}")))?;
        let status = res.status();
        let v: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v["error"].as_str().map(str::to_string).unwrap_or_else(|| status.to_string());
            return Err(CardError::new(if matches!(status.as_u16(), 404 | 409) { status.as_u16() } else { 502 }, format!("L-Brain：{msg}")));
        }
        Ok(v)
    }

    /// Run `f` on a kb-core opened over the spaces (blocking, one at a time).
    async fn disk<T: Send + 'static>(&self, f: impl FnOnce(&mut KnowledgeBase) -> Res<T> + Send + 'static) -> Res<T> {
        let (spaces, member) = (self.spaces.clone(), self.member.clone());
        tokio::task::spawn_blocking(move || {
            let _g = DISK.lock().unwrap_or_else(|e| e.into_inner());
            let mut mounted = kb_core::lore_store::Spaces::new();
            for d in &spaces {
                mounted.mount(d).map_err(|e| CardError::new(500, e.to_string()))?;
            }
            let mut kb = KnowledgeBase::open(mounted, &member, KbOptions::default()).map_err(|e| CardError::new(500, e.to_string()))?;
            f(&mut kb)
        })
        .await
        .map_err(|e| CardError::new(500, e.to_string()))?
    }

    /// A new card from L-Brain's own `POST /api/nodes` body (`space?`, `via`, `initial_state`,
    /// the card's fields). No `space`: the member's personal space. → `{ node, via: kb | file }`.
    pub async fn create(&self, mut body: Value) -> Res<Value> {
        match self.place().await? {
            Place::Server(url) => {
                if body.get("space").and_then(Value::as_str).is_none_or(str::is_empty) {
                    let v = self.api(self.http.get(format!("{url}/api/spaces"))).await?;
                    body["space"] = json!(personal(&v["spaces"]).ok_or(CardError::new(409, "知識庫沒有你能寫入的空間"))?);
                }
                let v = self.api(self.http.post(format!("{url}/api/nodes")).json(&body)).await?;
                Ok(json!({ "node": v["node"], "via": "kb" }))
            }
            Place::Disk => {
                let via = body["via"].as_str().unwrap_or("kb").to_string();
                let state: State = serde_json::from_value(body["initial_state"].clone()).unwrap_or(State::Learning);
                let space = body["space"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                let fields: NodeFields = serde_json::from_value(body).map_err(|e| CardError::new(400, e.to_string()))?;
                let node = self
                    .disk(move |kb| {
                        let space = match space {
                            Some(s) => s,
                            None => {
                                let info = kb.spaces_info().map_err(|e| CardError::new(500, e.to_string()))?;
                                personal(&serde_json::to_value(&info).unwrap_or(Value::Null)).ok_or(CardError::new(409, "知識庫沒有你能寫入的空間"))?
                            }
                        };
                        kb.create_as(&space, fields, Some(&via), state).map_err(|e| CardError::new(500, e.to_string()))
                    })
                    .await?;
                Ok(json!({ "node": node, "via": "file" }))
            }
        }
    }

    /// A review answer (`remembered | forgot | unfamiliar`) → `{ progress, … }`.
    pub async fn recall(&self, id: &str, outcome: &str, via: &str) -> Res<Value> {
        let o: Outcome = serde_json::from_value(json!(outcome)).map_err(|_| CardError::new(400, "不認得的結果"))?;
        match self.place().await? {
            Place::Server(url) => self.api(self.http.post(format!("{url}/api/recall/{id}")).json(&json!({ "outcome": outcome, "via": via }))).await,
            Place::Disk => {
                let (id, via) = (id.to_string(), via.to_string());
                self.disk(move |kb| kb.recall(&id, o, Some(&via)).map_err(not_found)).await.map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
            }
        }
    }

    /// A card as L-Brain's `GET /api/nodes/{id}` gives it (`{ space, node, progress, … }`).
    pub async fn view(&self, id: &str) -> Res<Value> {
        match self.place().await? {
            Place::Server(url) => self.api(self.http.get(format!("{url}/api/nodes/{id}"))).await,
            Place::Disk => {
                let id = id.to_string();
                self.disk(move |kb| kb.view(&id).map_err(not_found)).await.map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
            }
        }
    }

    /// An edit as L-Brain's `PUT /api/nodes/{id}` takes it (`revision?` + fields) → `{ node }`.
    pub async fn update(&self, id: &str, body: Value) -> Res<Value> {
        match self.place().await? {
            Place::Server(url) => self.api(self.http.put(format!("{url}/api/nodes/{id}")).json(&body)).await,
            Place::Disk => {
                let revision = body["revision"].as_u64();
                let fields: NodeFields = serde_json::from_value(body).map_err(|e| CardError::new(400, e.to_string()))?;
                let id = id.to_string();
                self.disk(move |kb| kb.update(&id, fields, revision).map_err(not_found)).await.map(|n| json!({ "node": n }))
            }
        }
    }
}

fn not_found(e: kb_core::KbError) -> CardError {
    match e {
        kb_core::KbError::NotFound(id) => CardError::new(404, format!("知識庫沒有這張卡（{id}）")),
        kb_core::KbError::Invalid(m) => CardError::new(400, m),
        kb_core::KbError::Conflict { .. } => CardError::new(409, "這張卡在這之間改過了"),
        e => CardError::new(500, e.to_string()),
    }
}

/// The member's personal space in a `spaces` list (L-Brain's `/api/spaces` or `spaces_info`).
fn personal(spaces: &Value) -> Option<String> {
    let list = spaces.as_array()?;
    list.iter()
        .find(|s| s["kind"] == "personal" && s["is_member"] == true)
        .or_else(|| list.iter().find(|s| s["is_member"] == true))
        .and_then(|s| s["id"].as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests;
