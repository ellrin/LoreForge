//! L-Brain over its HTTP API (default http://127.0.0.1:7700). L-Reader never
//! writes to the KB on its own: a card is created only by the author's explicit
//! 「加入知識庫」, and scans never record encounters.

use std::time::Duration;

use serde_json::{Value, json};

#[derive(Clone)]
pub struct Kb {
    pub base: String,
    http: reqwest::Client,
    /// [`Kb::wake`] may start the L-Brain app (a local KB, not isolated).
    launch: bool,
}

/// Why a KB call failed, ready for the UI.
#[derive(Debug)]
pub struct KbError {
    /// The server did not answer at all (not running).
    pub down: bool,
    pub message: String,
}

impl Kb {
    pub fn new(base: &str) -> Kb {
        // No proxy: the KB is on this machine.
        let http = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(2)).build().unwrap_or_default();
        Kb { base: base.trim_end_matches('/').to_string(), http, launch: false }
    }

    /// The real L-Brain of this machine: [`Kb::wake`] starts the app when it is not running.
    pub fn launching(mut self, on: bool) -> Kb {
        let local = ["http://127.0.0.1", "http://localhost"].iter().any(|p| self.base.starts_with(p));
        self.launch = on && local;
        self
    }

    /// Before a call the author asked for (a card, a draft, opening a card): start L-Brain in
    /// the background when it does not answer and wait for it. Polls and the noun scan never
    /// call this, so quitting L-Brain keeps it closed until the author needs it.
    pub async fn wake(&self) -> Result<(), KbError> {
        if self.get_quick("/api/spaces").await {
            return Ok(());
        }
        if !self.launch {
            return Err(self.down());
        }
        let port = self.base.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(7700);
        match tokio::task::spawn_blocking(move || lore_home::ports::ensure("kb", Some(port))).await {
            Ok(Ok(p)) if p == port => Ok(()),
            Ok(Ok(p)) => Err(KbError { down: true, message: format!("L-Brain 開在 {p}，不是設定的 {}", self.base) }),
            Ok(Err(e)) => Err(KbError { down: true, message: e }),
            Err(e) => Err(KbError { down: true, message: e.to_string() }),
        }
    }

    async fn get_quick(&self, path: &str) -> bool {
        self.http.get(format!("{}{path}", self.base)).timeout(Duration::from_secs(2)).send().await.is_ok_and(|r| r.status().is_success())
    }

    fn down(&self) -> KbError {
        KbError { down: true, message: "L-Brain 沒有回應：請先開啟 L-Brain。".into() }
    }

    async fn send(&self, req: reqwest::RequestBuilder, timeout: Duration) -> Result<Value, KbError> {
        let res = req.timeout(timeout).send().await.map_err(|e| if e.is_connect() || e.is_timeout() { self.down() } else { KbError { down: false, message: e.to_string() } })?;
        let status = res.status();
        let v: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v["error"].as_str().map(str::to_string).unwrap_or_else(|| status.to_string());
            return Err(KbError { down: false, message: format!("L-Brain：{msg}") });
        }
        Ok(v)
    }

    pub async fn get(&self, path: &str) -> Result<Value, KbError> {
        self.send(self.http.get(format!("{}{path}", self.base)), Duration::from_secs(5)).await
    }

    pub async fn post(&self, path: &str, body: &Value, timeout: Duration) -> Result<Value, KbError> {
        self.send(self.http.post(format!("{}{path}", self.base)).json(body), timeout).await
    }

    /// The member's personal space (where new cards go unless one is named).
    pub async fn default_space(&self) -> Result<String, KbError> {
        let v = self.get("/api/spaces").await?;
        let spaces = v["spaces"].as_array().cloned().unwrap_or_default();
        let pick = spaces
            .iter()
            .find(|s| s["kind"] == "personal" && s["is_member"] == true)
            .or_else(|| spaces.iter().find(|s| s["is_member"] == true))
            .and_then(|s| s["id"].as_str());
        pick.map(str::to_string).ok_or(KbError { down: false, message: "L-Brain 沒有你能寫入的空間".into() })
    }

    /// Every card's title and aliases with this member's state (`GET /api/nodes`,
    /// read only): the names the noun list matches as units.
    pub async fn lexicon(&self) -> Result<Vec<lr_core::nouns::KbName>, KbError> {
        let v = self.get("/api/nodes?limit=5000").await?;
        let mut out = vec![];
        for n in v["nodes"].as_array().into_iter().flatten() {
            let (Some(id), Some(title)) = (n["id"].as_str(), n["title"].as_str()) else { continue };
            let state = n["state"].as_str().map(str::to_string);
            let aliases = n["aliases"].as_array().into_iter().flatten().filter_map(|a| a.as_str());
            for name in std::iter::once(title).chain(aliases) {
                if !name.trim().is_empty() {
                    out.push(lr_core::nouns::KbName { name: name.to_string(), node_id: id.to_string(), title: title.to_string(), state: state.clone() });
                }
            }
        }
        Ok(out)
    }
}

/// The body of `POST /api/nodes` for a card made from a PDF selection.
pub struct CardRequest<'a> {
    pub space: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub kind: Option<&'a str>,
    /// `learning` (default) or `known`.
    pub initial_state: &'a str,
    pub sha: &'a str,
    pub page: u32,
    pub paper_title: &'a str,
    /// The sentence the term was selected in (kept in the body as a quote).
    pub context: Option<&'a str>,
}

pub fn card_body(c: &CardRequest) -> Value {
    let mut v = json!({
        "space": c.space,
        "via": "lorereader",
        "kind": c.kind.filter(|k| !k.is_empty()).unwrap_or("concept"),
        "title": c.title.trim(),
        "summary": c.summary.trim(),
        "initial_state": if c.initial_state == "known" { "known" } else { "learning" },
        "sources": [{ "type": "pdf", "ref": c.sha, "where": format!("p.{}", c.page), "title": c.paper_title }],
    });
    if let Some(ctx) = c.context.map(str::trim).filter(|s| !s.is_empty()) {
        v["body"] = format!("> {}\n\n（{}，p.{}）", ctx.split_whitespace().collect::<Vec<_>>().join(" "), c.paper_title, c.page).into();
    }
    v
}

#[cfg(test)]
mod wake_tests {
    use super::*;

    /// Only this machine's L-Brain is ever started, and never when launching is off (isolated).
    #[tokio::test]
    async fn wake_starts_nothing_when_isolated_or_remote() {
        assert!(!Kb::new("http://kb.example:7700").launching(true).launch);
        assert!(Kb::new("http://127.0.0.1:7700").launching(true).launch);
        // isolated and down: the "not running" error at once, no launch
        let e = Kb::new("http://127.0.0.1:1").launching(false).wake().await.unwrap_err();
        assert!(e.down);
    }
}
