//! HTTP behind a small trait: [`ReqwestHttp`] for real use, [`MockHttp`] for tests (canned
//! responses by URL, a call log, scripted sequences such as 429 → 200, "no network").

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Stop reading (and fail with [`Error::TooLarge`]) past this many bytes.
    pub max_body: Option<u64>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self { method: Method::Get, url: url.into(), headers: Vec::new(), body: None, max_body: None }
    }
    pub fn post_json(url: impl Into<String>, v: &serde_json::Value) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: Some(serde_json::to_vec(v).unwrap_or_default()),
            max_body: None,
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub status: u16,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self { status: 200, headers: Vec::new(), body: body.into() }
    }
    pub fn status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self { status, headers: Vec::new(), body: body.into() }
    }
    pub fn with_header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_ascii_lowercase(), v.to_string()));
        self
    }
    pub fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// One request, one response. Transport failures that mean "no network" (connect, DNS,
/// timeout) are [`Error::Offline`]; any HTTP status is a `Response`, not an error.
#[async_trait]
pub trait Http: Send + Sync {
    async fn send(&self, req: Request) -> Result<Response, Error>;
}

/// reqwest with a fixed User-Agent and timeout.
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    pub fn new(user_agent: &str, timeout: Duration) -> Self {
        let client = reqwest::Client::builder().user_agent(user_agent).timeout(timeout).connect_timeout(Duration::from_secs(10)).build().unwrap_or_default();
        Self { client }
    }
}

#[async_trait]
impl Http for ReqwestHttp {
    async fn send(&self, req: Request) -> Result<Response, Error> {
        let mut b = match req.method {
            Method::Get => self.client.get(&req.url),
            Method::Post => self.client.post(&req.url),
        };
        for (k, v) in &req.headers {
            b = b.header(k, v);
        }
        if let Some(body) = req.body {
            b = b.body(body);
        }
        let net = |e: reqwest::Error| if e.is_connect() || e.is_timeout() { Error::Offline(e.to_string()) } else { Error::Network(e.to_string()) };
        let mut r = b.send().await.map_err(net)?;
        let status = r.status().as_u16();
        let headers = r.headers().iter().map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string())).collect();
        if let (Some(max), Some(len)) = (req.max_body, r.content_length())
            && len > max {
                return Err(Error::TooLarge { limit: max });
            }
        let mut body = Vec::new();
        while let Some(chunk) = r.chunk().await.map_err(net)? {
            body.extend_from_slice(&chunk);
            if req.max_body.is_some_and(|m| body.len() as u64 > m) {
                return Err(Error::TooLarge { limit: req.max_body.unwrap_or(0) });
            }
        }
        Ok(Response { status, headers, body })
    }
}

/// A URL pattern and its answers (`Err` = no network).
type Route = (String, VecDeque<Result<Response, String>>);

/// Canned responses for tests: the first route whose pattern is contained in the URL answers;
/// a route with several responses gives them in order and then repeats the last one.
/// No route = "no network" ([`Error::Offline`]). Every request is logged.
#[derive(Default)]
pub struct MockHttp {
    routes: Mutex<Vec<Route>>,
    log: Mutex<Vec<Request>>,
}

impl MockHttp {
    pub fn new() -> Self {
        Self::default()
    }
    /// Answer URLs containing `pattern` with `r`.
    pub fn on(self, pattern: &str, r: Response) -> Self {
        self.seq(pattern, vec![r])
    }
    /// Answer URLs containing `pattern` with these responses in turn.
    pub fn seq(self, pattern: &str, rs: Vec<Response>) -> Self {
        self.routes.lock().unwrap().push((pattern.to_string(), rs.into_iter().map(Ok).collect()));
        self
    }
    /// URLs containing `pattern` fail as "no network".
    pub fn offline(self, pattern: &str) -> Self {
        self.routes.lock().unwrap().push((pattern.to_string(), VecDeque::from([Err("connection refused".to_string())])));
        self
    }
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
    pub fn urls(&self) -> Vec<String> {
        self.requests().into_iter().map(|r| r.url).collect()
    }
}

#[async_trait]
impl Http for MockHttp {
    async fn send(&self, req: Request) -> Result<Response, Error> {
        self.log.lock().unwrap().push(req.clone());
        let mut routes = self.routes.lock().unwrap();
        let Some((_, q)) = routes.iter_mut().find(|(p, _)| req.url.contains(p.as_str())) else {
            return Err(Error::Offline(format!("no route for {}", req.url)));
        };
        let r = if q.len() > 1 { q.pop_front().unwrap() } else { q.front().cloned().unwrap() };
        let r = r.map_err(Error::Offline)?;
        if req.max_body.is_some_and(|m| r.body.len() as u64 > m) {
            return Err(Error::TooLarge { limit: req.max_body.unwrap_or(0) });
        }
        Ok(r)
    }
}
