//! The handlers (routes listed in `lib.rs`).

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::characters::{self, Character};
use crate::store::{self, SettingsPatch, WindowPatch, WindowState};
use crate::{AppState, DesktopEvent, Shared, chat, llm};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/health", get(health))
        .route("/prefs.css", get(prefs_css))
        .route("/theme.css", get(theme_css))
        .route("/theme.js", get(theme_js))
        .route("/api/theme", get(theme_get).put(theme_put))
        .route("/api/window", get(window_get).put(window_put))
        .route("/api/settings", get(settings_get).put(settings_put))
        .route("/api/characters", get(characters_list).post(character_new))
        .route("/api/characters/import", post(character_import).layer(DefaultBodyLimit::max(80 << 20)))
        .route("/api/characters/{id}", get(character_get).put(character_put))
        .route("/api/characters/{id}/export", get(character_export))
        .route("/api/characters/{id}/{kind}/{state}", get(image_get).put(image_put).delete(image_delete).layer(DefaultBodyLimit::max(16 << 20)))
        .route("/api/chat", post(chat_post))
}

fn bad(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn no_store(msg: &str) -> Response {
    bad(StatusCode::SERVICE_UNAVAILABLE, format!("沒有 LoreForge 的 private 資料夾，無法{msg}"))
}

impl AppState {
    /// Change and write the window state (the page's PUT, the desktop app's moves).
    pub fn save_window(&self, p: &WindowPatch) -> Result<WindowState, String> {
        let dir = self.dir.as_ref().ok_or("沒有 private 資料夾")?;
        let _g = self.write.lock().unwrap_or_else(|e| e.into_inner());
        store::set_window(dir, p)
    }

    pub fn window(&self) -> WindowState {
        self.dir.as_deref().map(store::window).unwrap_or_default()
    }

    pub fn settings(&self) -> store::Settings {
        self.dir.as_deref().map(store::settings).unwrap_or_default()
    }

    /// The character to talk as: `wanted`, else the chosen one, else the first.
    pub fn character(&self, wanted: Option<&str>) -> Character {
        let chosen = self.settings().character;
        characters::pick(self.dir.as_deref(), wanted.or(chosen.as_deref()))
    }
}

async fn health(State(s): State<Shared>) -> Json<Value> {
    Json(json!({ "ok": true, "app": crate::config::APP, "character": s.character(None).id, "desktop": s.hook.is_some() }))
}

async fn prefs_css(State(s): State<Shared>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], s.home.prefs().fonts.css())
}

/// The LoreForge themes (commons/ui/theme.css, the same file in every app), compiled in.
async fn theme_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::CSS)
}

/// commons/ui/theme.js with the shared choice in front (first paint has the theme).
async fn theme_js(State(s): State<Shared>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::script(&s.home))
}

async fn theme_get(State(s): State<Shared>) -> Json<Value> {
    Json(json!({ "theme": s.home.theme() }))
}

async fn theme_put(State(s): State<Shared>, Json(b): Json<Value>) -> Response {
    match s.home.set_theme(b["theme"].as_str().unwrap_or_default()) {
        Ok(t) => Json(json!({ "theme": t })).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn window_get(State(s): State<Shared>) -> Json<WindowState> {
    Json(s.window())
}

/// `{ x?, y?, expanded?, w?, h? }`: what changes. `expanded` also opens / closes the desktop panel; `w` / `h` (the open window's size) resize it.
async fn window_put(State(s): State<Shared>, body: Option<Json<WindowPatch>>) -> Response {
    let Some(Json(p)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { x, y, expanded }") };
    if s.dir.is_none() {
        return no_store("記住位置");
    }
    match s.save_window(&p) {
        Ok(w) => {
            if let Some(e) = p.expanded {
                s.tell(DesktopEvent::Expanded(e));
            } else if p.w.is_some() || p.h.is_some() {
                let (w, h) = w.size();
                s.tell(DesktopEvent::Size(w, h));
            }
            Json(w).into_response()
        }
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

async fn settings_get(State(s): State<Shared>) -> Json<store::Settings> {
    Json(s.settings())
}

/// `{ character?, shortcut? }`; the character must exist ("" = the first one).
async fn settings_put(State(s): State<Shared>, body: Option<Json<SettingsPatch>>) -> Response {
    let Some(Json(p)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { character, shortcut }") };
    let Some(dir) = s.dir.clone() else { return no_store("儲存設定") };
    if let Some(id) = p.character.as_deref().map(str::trim).filter(|c| !c.is_empty())
        && let Err(e) = characters::load(&dir, id)
    {
        return bad(StatusCode::BAD_REQUEST, format!("沒有這個人物：{id}（{}）", e.lines().next().unwrap_or("")));
    }
    let r = {
        let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
        store::set_settings(&dir, &p)
    };
    match r {
        Ok(st) => {
            if p.shortcut.is_some() {
                s.tell(DesktopEvent::Shortcut(st.shortcut.clone()));
            }
            Json(st).into_response()
        }
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

async fn characters_list(State(s): State<Shared>) -> Json<Value> {
    let mut list = s.dir.as_deref().map(characters::list).unwrap_or_default();
    if list.is_empty() {
        list.push(Character::builtin());
    }
    let chains: serde_json::Map<String, Value> = characters::STATES.iter().map(|st| (st.to_string(), json!(characters::chain(st)))).collect();
    Json(json!({ "characters": list, "current": s.character(None).id, "states": characters::STATES, "chains": chains }))
}

fn store_dir(s: &Shared) -> Result<std::path::PathBuf, Response> {
    s.dir.clone().ok_or_else(|| no_store("儲存人物"))
}

async fn character_get(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    let Some(dir) = s.dir.as_deref() else { return bad(StatusCode::NOT_FOUND, "沒有這個人物") };
    match characters::detail(dir, &id) {
        Ok(d) => Json(d).into_response(),
        Err(e) => bad(StatusCode::NOT_FOUND, e),
    }
}

/// `{ name?, persona?, voice? }`
async fn character_put(State(s): State<Shared>, Path(id): Path<String>, body: Option<Json<characters::MetaPatch>>) -> Response {
    let Some(Json(p)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { name, persona, voice }") };
    let dir = match store_dir(&s) { Ok(d) => d, Err(r) => return r };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match characters::save_meta(&dir, &id, &p) {
        Ok(d) => Json(d).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

/// `{ name }`: a new character, a copy of the public example's.
async fn character_new(State(s): State<Shared>, body: Option<Json<Value>>) -> Response {
    let name = body.as_ref().and_then(|b| b["name"].as_str()).unwrap_or_default().to_string();
    let dir = match store_dir(&s) { Ok(d) => d, Err(r) => return r };
    let template = s.home.example_dir(crate::config::APP).and_then(|d| characters::example_template(&d));
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match characters::create(&dir, template.as_deref(), &name) {
        Ok(d) => (StatusCode::CREATED, Json(d)).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

async fn character_export(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    let Some(dir) = s.dir.as_deref() else { return bad(StatusCode::NOT_FOUND, "沒有這個人物") };
    match characters::export(dir, &id) {
        Ok(zip) => ([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{id}.zip\""))], zip).into_response(),
        Err(e) => bad(StatusCode::NOT_FOUND, e),
    }
}

/// The body is a zip (see [`characters::import`]).
async fn character_import(State(s): State<Shared>, body: Bytes) -> Response {
    let dir = match store_dir(&s) { Ok(d) => d, Err(r) => return r };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match characters::import(&dir, &body) {
        Ok(d) => (StatusCode::CREATED, Json(d)).into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

async fn image_get(State(s): State<Shared>, Path((id, kind, state)): Path<(String, String, String)>) -> Response {
    let found = s.dir.as_deref().and_then(|d| match kind.as_str() {
        characters::AVATAR => characters::avatar(d, &id, &state),
        characters::FULL => characters::full_body(d, &id, &state),
        _ => None,
    });
    picture(found).await
}

/// The body is the picture itself (png, jpg, webp or svg).
async fn image_put(State(s): State<Shared>, Path((id, kind, state)): Path<(String, String, String)>, body: Bytes) -> Response {
    let dir = match store_dir(&s) { Ok(d) => d, Err(r) => return r };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match characters::save_image(&dir, &id, &kind, &state, &body) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => bad(StatusCode::BAD_REQUEST, e),
    }
}

async fn image_delete(State(s): State<Shared>, Path((id, kind, state)): Path<(String, String, String)>) -> Response {
    let dir = match store_dir(&s) { Ok(d) => d, Err(r) => return r };
    let _g = s.write.lock().unwrap_or_else(|e| e.into_inner());
    match characters::remove_image(&dir, &id, &kind, &state) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => bad(StatusCode::NOT_FOUND, e),
    }
}

async fn picture(found: Option<(std::path::PathBuf, &'static str)>) -> Response {
    let Some((path, mime)) = found else { return bad(StatusCode::NOT_FOUND, "沒有這張人物圖") };
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache"), (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'; sandbox")], bytes).into_response(),
        Err(e) => bad(StatusCode::NOT_FOUND, e.to_string()),
    }
}

async fn chat_post(State(s): State<Shared>, body: Option<Json<chat::ChatBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { messages, character }") };
    let messages = match chat::transcript(&b.messages) {
        Ok(m) => m,
        Err(e) => return bad(StatusCode::BAD_REQUEST, e),
    };
    let character = s.character(b.character.as_deref());
    let route = match s.llm.route(llm::TASK_CHAT, b.engine.as_deref()).await {
        Ok(r) => r,
        Err(payload) => return (StatusCode::SERVICE_UNAVAILABLE, Json(payload)).into_response(),
    };
    chat::stream(route, chat::request(&character, messages)).into_response()
}
