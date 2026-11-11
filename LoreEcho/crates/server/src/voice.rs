//! Voice routes (design §7): listening and speaking through lore-voice.
//!
//! | method | path | what |
//! |---|---|---|
//! | GET | /api/voice | `{ available, engine \| why, listening, speaking }` |
//! | GET | /api/voice/voices | `{ voices: [{ id, name, language, quality }] }` (the character's language first) |
//! | GET | /api/voice/events | server-sent events, one per lore-voice `Event` (event name = its `type`) |
//! | POST | /api/voice/listen | `{ continuous?, language? }` (language: the character's when missing) |
//! | POST | /api/voice/listen/stop | press-and-hold released / 常駐 off: the last words come as `final` |
//! | POST | /api/voice/speak | `{ text, voice?, rate?, character? }` → `{ id }` (試聽; the character's voice when missing) |
//! | POST | /api/voice/stop | stop speaking now (the queue is dropped) |
//! | POST | /api/voice/open-settings | `{ what: "microphone" \| "speech" }`: System Settings at that privacy page (macOS, real engine only) |
//!
//! A reply read aloud as it streams goes through `POST /api/chat { speak: true }` (`chat.rs`).

use std::convert::Infallible;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/voice", get(status))
        .route("/api/voice/voices", get(voices))
        .route("/api/voice/events", get(events))
        .route("/api/voice/listen", post(listen))
        .route("/api/voice/listen/stop", post(listen_stop))
        .route("/api/voice/speak", post(speak))
        .route("/api/voice/stop", post(stop))
        .route("/api/voice/open-settings", post(open_settings))
}

fn bad(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

async fn status(State(s): State<Shared>) -> Json<Value> {
    let (ok, what) = s.voice.status();
    let mut v = json!({ "available": ok, "listening": s.voice.listening(), "speaking": s.voice.speaking() });
    v[if ok { "engine" } else { "why" }] = json!(what);
    Json(v)
}

async fn voices(State(s): State<Shared>) -> Response {
    let language = s.character(None).language;
    let v = s.voice.clone();
    match tokio::task::spawn_blocking(move || v.voices()).await {
        Ok(Ok(mut list)) => {
            lore_voice::sort_voices(&mut list, &language);
            Json(json!({ "voices": list, "language": language })).into_response()
        }
        Ok(Err(e)) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Every voice event as it happens (a page that falls behind skips the oldest).
async fn events(State(s): State<Shared>) -> Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = s.voice.subscribe();
    let first = SseEvent::default().event("hello").data(json!({ "listening": s.voice.listening(), "speaking": s.voice.speaking() }).to_string());
    let rest = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(e) => {
                    let v = serde_json::to_value(&e).unwrap_or(Value::Null);
                    let name = v["type"].as_str().unwrap_or("voice").to_string();
                    return Some((Ok(SseEvent::default().event(name).data(v.to_string())), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
    });
    let s = futures_util::StreamExt::chain(futures_util::stream::once(async move { Ok(first) }), rest);
    Sse::new(s).keep_alive(KeepAlive::default())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ListenBody {
    continuous: bool,
    language: Option<String>,
}

async fn listen(State(s): State<Shared>, body: Option<Json<ListenBody>>) -> Response {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let language = b.language.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| s.character(None).language);
    match s.voice.listen(&lore_voice::ListenOptions { language, continuous: b.continuous }) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn listen_stop(State(s): State<Shared>) -> StatusCode {
    s.voice.stop_listening();
    StatusCode::NO_CONTENT
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SpeakBody {
    text: String,
    voice: Option<String>,
    rate: Option<f64>,
    character: Option<String>,
}

async fn speak(State(s): State<Shared>, body: Option<Json<SpeakBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { text }") };
    let Some(text) = lore_voice::split::speakable(&b.text) else { return bad(StatusCode::BAD_REQUEST, "沒有要說的話") };
    if text.chars().count() > 2000 {
        return bad(StatusCode::BAD_REQUEST, "太長了");
    }
    let c = s.character(b.character.as_deref());
    let req = lore_voice::SpeakRequest { text, voice: b.voice.unwrap_or(c.voice), rate: b.rate.unwrap_or(c.voice_rate), language: c.language };
    match s.voice.speak(&req) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(e) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn stop(State(s): State<Shared>) -> StatusCode {
    s.voice.stop_speaking();
    StatusCode::NO_CONTENT
}

/// The two privacy pages of System Settings (nothing else can be opened from here).
pub fn settings_url(what: &str) -> Option<&'static str> {
    match what {
        "microphone" => Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"),
        "speech" => Some("x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition"),
        _ => None,
    }
}

async fn open_settings(State(s): State<Shared>, body: Option<Json<Value>>) -> Response {
    let what = body.as_ref().and_then(|Json(b)| b["what"].as_str()).unwrap_or("").to_string();
    let Some(url) = settings_url(&what) else { return bad(StatusCode::BAD_REQUEST, "what = microphone | speech") };
    // only with the real engine on macOS (tests and other systems open nothing)
    if s.voice.status().1 != "apple" || !cfg!(target_os = "macos") {
        return Json(json!({ "opened": false, "url": url })).into_response();
    }
    match std::process::Command::new("/usr/bin/open").arg(url).status() {
        Ok(st) if st.success() => Json(json!({ "opened": true, "url": url })).into_response(),
        Ok(st) => bad(StatusCode::INTERNAL_SERVER_ERROR, format!("open: {st}")),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
