//! The voice routes every app mounts (feature `web`): listening and speaking over HTTP, the
//! same paths and bodies in LoreEcho, PaperComposer and SlideComposer.
//!
//! | method | path | what |
//! |---|---|---|
//! | GET | /api/voice | `{ available, engine \| why, listening, speaking }` |
//! | GET | /api/voice/voices | `{ voices: [{ id, name, language, quality }], language }` (the default language first) |
//! | GET | /api/voice/events | server-sent events: `hello`, then one per [`Event`](crate::Event) (event name = its `type`) |
//! | POST | /api/voice/listen | `{ continuous?, language? }` (language: the default's when missing) |
//! | POST | /api/voice/listen/stop | press-and-hold released / 常駐 off: the last words come as `final` |
//! | POST | /api/voice/speak | `{ text, voice?, rate?, pitch?, character? }` → `{ id }` |
//! | POST | /api/voice/stop | stop speaking now (the queue is dropped) |
//! | POST | /api/voice/open-settings | `{ what: "microphone" \| "speech" \| "personal-voice" \| "voices" }`: System Settings at that page (macOS, real engine only) |
//! | POST | /api/voice/warm | `{ character? }`: load that voice now, silently |
//! | GET/POST | /api/voice/personal | Personal Voice: `{ status, voices }`; POST = ask (the system's prompt only when never asked) |
//!
//! What the app decides is [`Defaults`]: how someone speaks (`character` in a body; LoreEcho's
//! characters) and the language to listen in. A [`SpeakRequest`] is itself a fixed default.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ListenOptions, PersonalVoice, SpeakRequest, Voice};

/// How `who` speaks and listens (None = the app's default): voice, rate, pitch, language.
pub trait Defaults: Send + Sync + 'static {
    fn template(&self, who: Option<&str>) -> SpeakRequest;
}

/// One way of speaking for everyone (PaperComposer, SlideComposer).
impl Defaults for SpeakRequest {
    fn template(&self, _who: Option<&str>) -> SpeakRequest {
        self.clone()
    }
}

#[derive(Clone)]
struct Web {
    voice: Voice,
    defaults: Arc<dyn Defaults>,
}

/// `/api/voice*` over `voice`, with `defaults` for the language and the speaking voice.
pub fn router<S: Clone + Send + Sync + 'static>(voice: Voice, defaults: Arc<dyn Defaults>) -> Router<S> {
    Router::new()
        .route("/api/voice", get(status))
        .route("/api/voice/voices", get(voices))
        .route("/api/voice/events", get(events))
        .route("/api/voice/listen", post(listen))
        .route("/api/voice/listen/stop", post(listen_stop))
        .route("/api/voice/speak", post(speak))
        .route("/api/voice/stop", post(stop))
        .route("/api/voice/open-settings", post(open_settings))
        .route("/api/voice/warm", post(warm))
        .route("/api/voice/personal", get(personal_get).post(personal_ask))
        .with_state(Web { voice, defaults })
}

fn bad(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

async fn status(State(s): State<Web>) -> Json<Value> {
    let (ok, what) = s.voice.status();
    let mut v = json!({ "available": ok, "listening": s.voice.listening(), "speaking": s.voice.speaking() });
    v[if ok { "engine" } else { "why" }] = json!(what);
    Json(v)
}

async fn voices(State(s): State<Web>) -> Response {
    let language = s.defaults.template(None).language;
    let v = s.voice.clone();
    match tokio::task::spawn_blocking(move || v.voices()).await {
        Ok(Ok(mut list)) => {
            crate::sort_voices(&mut list, &language);
            Json(json!({ "voices": list, "language": language })).into_response()
        }
        Ok(Err(e)) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Every voice event as it happens (a page that falls behind skips the oldest).
async fn events(State(s): State<Web>) -> Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>> {
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

async fn listen(State(s): State<Web>, body: Option<Json<ListenBody>>) -> Response {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let language = b.language.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| s.defaults.template(None).language);
    match s.voice.listen(&ListenOptions { language, continuous: b.continuous }) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn listen_stop(State(s): State<Web>) -> StatusCode {
    s.voice.stop_listening();
    StatusCode::NO_CONTENT
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SpeakBody {
    text: String,
    voice: Option<String>,
    rate: Option<f64>,
    pitch: Option<f64>,
    character: Option<String>,
}

async fn speak(State(s): State<Web>, body: Option<Json<SpeakBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { text }") };
    let Some(text) = crate::split::speakable(&b.text) else { return bad(StatusCode::BAD_REQUEST, "沒有要說的話") };
    if text.chars().count() > 2000 {
        return bad(StatusCode::BAD_REQUEST, "太長了");
    }
    // the default's voice, rate and pitch unless the body asks for others (試聽, not saved yet)
    let mut req = s.defaults.template(b.character.as_deref());
    req.text = text;
    if let Some(v) = b.voice {
        req.voice = v;
    }
    if let Some(r) = b.rate {
        req.rate = r;
    }
    if let Some(p) = b.pitch {
        req.pitch = p;
    }
    match s.voice.speak(&req) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(e) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn stop(State(s): State<Web>) -> StatusCode {
    s.voice.stop_speaking();
    StatusCode::NO_CONTENT
}

/// The System Settings pages a page may open (nothing else can be opened from here):
/// the two privacy pages, Accessibility → Personal Voice and Accessibility → Spoken Content.
/// The privacy pages are the shared ones (`lore_shell::permissions::Permission::settings_url`).
pub fn settings_url(what: &str) -> Option<&'static str> {
    match what {
        "microphone" | "speech" | "personal-voice" => crate::Permission::parse(what).map(|p| p.settings_url()),
        "voices" => Some("x-apple.systempreferences:com.apple.Accessibility-Settings.extension?SpokenContent"),
        _ => None,
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WarmBody {
    character: Option<String>,
}

/// An open window: get the voice ready (no sound) so the first sentence starts sooner.
async fn warm(State(s): State<Web>, body: Option<Json<WarmBody>>) -> StatusCode {
    let t = s.defaults.template(body.and_then(|Json(b)| b.character).as_deref());
    s.voice.warm(&t);
    StatusCode::NO_CONTENT
}

fn personal_json(r: Result<PersonalVoice, String>, voice: &Voice) -> Response {
    match r {
        Ok(st) => {
            // how many the author has made (listed only once allowed)
            let made = if st == PersonalVoice::Authorized { voice.voices().map(|v| v.iter().filter(|x| x.personal).count()).unwrap_or(0) } else { 0 };
            Json(json!({ "status": st, "voices": made })).into_response()
        }
        Err(e) => bad(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

/// `{ status: not_determined | denied | unsupported | authorized, voices }` (asks nothing).
async fn personal_get(State(s): State<Web>) -> Response {
    let v = s.voice.clone();
    tokio::task::spawn_blocking(move || personal_json(v.personal_voice(false), &v)).await.unwrap_or_else(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Ask for Personal Voice: the system's prompt (only when never asked), then the same as GET.
async fn personal_ask(State(s): State<Web>) -> Response {
    let v = s.voice.clone();
    tokio::task::spawn_blocking(move || personal_json(v.personal_voice(true), &v)).await.unwrap_or_else(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn open_settings(State(s): State<Web>, body: Option<Json<Value>>) -> Response {
    let what = body.as_ref().and_then(|Json(b)| b["what"].as_str()).unwrap_or("").to_string();
    let Some(url) = settings_url(&what) else { return bad(StatusCode::BAD_REQUEST, "what = microphone | speech | personal-voice | voices") };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Call, FakeBackend};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn status_listen_and_speak_with_a_fixed_default() {
        let (backend, fake) = FakeBackend::instant();
        let def = SpeakRequest { language: "en-US".into(), ..Default::default() };
        let app: Router = router(Voice::new(Box::new(backend)), Arc::new(def));
        let (st, v) = call(&app, "GET", "/api/voice", None).await;
        assert_eq!((st, v["available"].as_bool()), (StatusCode::OK, Some(true)));
        let (st, v) = call(&app, "GET", "/api/voice/voices", None).await;
        assert_eq!((st, v["language"].as_str()), (StatusCode::OK, Some("en-US")));
        // press and hold: the default's language when the page names none
        assert_eq!(call(&app, "POST", "/api/voice/listen", Some(json!({}))).await.0, StatusCode::NO_CONTENT);
        assert_eq!(call(&app, "POST", "/api/voice/listen/stop", None).await.0, StatusCode::NO_CONTENT);
        assert_eq!(call(&app, "POST", "/api/voice/listen", Some(json!({ "language": "zh-TW" }))).await.0, StatusCode::NO_CONTENT);
        let langs: Vec<String> = fake.calls().into_iter().filter_map(|c| if let Call::Listen(o) = c { Some(o.language) } else { None }).collect();
        assert_eq!(langs, ["en-US", "zh-TW"]);
        let (st, v) = call(&app, "POST", "/api/voice/speak", Some(json!({ "text": "Hello." }))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(call(&app, "POST", "/api/voice/speak", Some(json!({ "text": "" }))).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call(&app, "POST", "/api/voice/open-settings", Some(json!({ "what": "microphone" }))).await.1["opened"], json!(false));
    }

    #[tokio::test]
    async fn without_voice_the_status_says_why() {
        let app: Router = router(Voice::unavailable("沒有語音"), Arc::new(SpeakRequest::default()));
        let (_, v) = call(&app, "GET", "/api/voice", None).await;
        assert_eq!((v["available"].as_bool(), v["why"].as_str()), (Some(false), Some("沒有語音")));
        assert_eq!(call(&app, "POST", "/api/voice/listen", Some(json!({}))).await.0, StatusCode::SERVICE_UNAVAILABLE);
    }
}
