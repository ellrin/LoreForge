//! 知識中樞 (docs/brain-hub.md §2 rules 3, 5 and 6, §4.2, §7): two actions that only PROPOSE changes
//! to L-Brain's knowledge. Nothing is written to the knowledge from here: a draft is answered to the
//! page, and only the author's confirmation posts it to L-Brain's proposal box, where he approves
//! each change again (`POST /api/hub/proposals`). L-Brain closed = 503, never a write anywhere else.
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/actions/know/figure | `{ image (base64 PNG/JPEG, or a data URL), note?, engine? }` → `{ proposal \| null, summary: {new_components, new_modules, linked, new_figure}, image: {name, data_url}, message?, engine, label }` (task `le-know-figure`: Claude, looks at the picture) |
//! | POST | /api/actions/know/edit | `{ id, instruction, engine? }` → `{ proposal \| null, before, after, changed, dropped, note, message?, engine, label }` (task `le-know-edit`) |
//! | POST | /api/actions/know/propose | `{ proposal }` → `{ id, open }`: the draft posted to L-Brain as `by = "le"` |
//!
//! The pieces of a picture are always *references to make* (`entity = false`), never drawable
//! entities; a library item that already is the piece is linked, not duplicated; there is never a
//! `remove`. An edit changes only what the model names, only fields the item's format has, never
//! empties a required field, and never touches a figure's drawing (`params`, `kind`, `style`, `what`,
//! `level`, `entity`).

// Err = the page's ready answer (the pattern of actions.rs); it is built once per request.
#![allow(clippy::result_large_err)]

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine as _;
use lore_know::{Change, FormatDef, Item, Proposal, decompose, facet, format};
use lore_llm::{Image, LlmRequest, Message};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Shared;
use crate::chat::clip;

pub const TASK_FIGURE: &str = "le-know-figure";
pub const TASK_EDIT: &str = "le-know-edit";
/// The hub's category of drawn flow items and of reference figures; the proposal's new pieces go to the second.
const FLOW: &str = "sc.flow";
const FLOW_REFS: &str = "sc.flowrefs";
/// This app's id as a proposal's author and as an item's source.
const BY: &str = "le";
/// A picture, decoded (the existing picture routes allow 10 MB too).
const MAX_PICTURE: usize = 10 << 20;
/// The author's instruction (characters).
const INSTRUCTION_CHARS: usize = 2000;
const CLOSED: &str = "L-Brain 沒有開著，這裡先不動";
/// The only parts of a figure an instruction may change here.
const FIGURE_EDITABLE_FIELD: &str = "description";
/// Facets an instruction may change (the hub's own three).
const FACETS: [&str; 3] = [facet::DOMAIN, facet::KIND, facet::USE];

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/actions/know/figure", post(figure).layer(DefaultBodyLimit::max(16 << 20)))
        .route("/api/actions/know/edit", post(edit))
        .route("/api/actions/know/propose", post(propose))
}

// ── answers ──

fn bad(status: StatusCode, msg: impl AsRef<str>) -> Response {
    (status, Json(json!({ "error": one_line(msg.as_ref()) }))).into_response()
}

fn one_line(s: &str) -> String {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

fn closed() -> Response {
    bad(StatusCode::SERVICE_UNAVAILABLE, CLOSED)
}

// ── L-Brain ──

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).no_proxy().build().unwrap_or_default()
}

/// L-Brain's address while it runs.
async fn brain(s: &Shared) -> Result<String, Response> {
    s.kb.server().await.ok_or_else(closed)
}

/// What L-Brain answered, as a line for the page.
fn brain_error(status: StatusCode, v: &Value) -> Response {
    let line = v["error"].as_str().map(one_line).filter(|l| !l.is_empty());
    match status {
        StatusCode::NOT_FOUND => bad(StatusCode::NOT_FOUND, line.unwrap_or_else(|| "L-Brain 裡找不到這一條".into())),
        _ => bad(StatusCode::BAD_GATEWAY, format!("L-Brain 沒有接受：{}", line.unwrap_or_else(|| status.to_string()))),
    }
}

async fn brain_get(base: &str, path: &str) -> Result<Value, Response> {
    let r = client().get(format!("{base}{path}")).send().await.map_err(|_| closed())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if status.is_success() { Ok(v) } else { Err(brain_error(status, &v)) }
}

fn items_of(v: &Value) -> Vec<Item> {
    v["items"].as_array().into_iter().flatten().filter_map(|i| serde_json::from_value::<Item>(i.clone()).ok()).collect()
}

/// The figure items the model may reuse: the drawn flow items and the reference figures.
async fn library(base: &str) -> Result<Vec<Item>, Response> {
    let mut all: Vec<Item> = vec![];
    for cat in [FLOW, FLOW_REFS] {
        let v = brain_get(base, &format!("/api/hub/items?category={cat}&limit=1000")).await?;
        for i in items_of(&v) {
            if !all.iter().any(|a| a.id == i.id) {
                all.push(i);
            }
        }
    }
    Ok(all)
}

// ── the model ──

/// Run `task` for one JSON value, with a picture when given. `Err` = the page's answer (503 with the
/// alternative engine when none can answer, 502 when the call failed).
async fn ask(s: &Shared, task: &str, engine: Option<&str>, system: &str, user: String, image: Option<Image>, schema: Value) -> Result<(Value, String), Response> {
    let route = match s.llm.route(task, engine).await {
        Ok(r) => r,
        Err(payload) => return Err((StatusCode::SERVICE_UNAVAILABLE, Json(payload)).into_response()),
    };
    let id = route.provider.id();
    let msg = match image {
        Some(img) => Message::user_with_images(user, vec![img]),
        None => Message::user(user),
    };
    let req = route.apply(LlmRequest::chat(system.to_string(), vec![msg]).json(schema).temperature(0.2));
    match lore_llm::complete_json(route.provider.as_ref(), &req).await {
        Ok(v) => Ok((v, id)),
        Err(e) => Err(bad(StatusCode::BAD_GATEWAY, format!("模型沒有整理好：{e}"))),
    }
}

// ── part A: a figure's picture ──

#[derive(Deserialize)]
struct FigureBody {
    #[serde(default)]
    image: String,
    note: Option<String>,
    engine: Option<String>,
}

/// A picture sent as base64 (or a `data:` URL): the bytes' type is read from the bytes, not trusted.
fn decode_picture(raw: &str) -> Result<(Vec<u8>, &'static str, &'static str, String), String> {
    let body = match raw.strip_prefix("data:") {
        Some(rest) => rest.split_once(',').map(|(_, b)| b).ok_or("圖片的格式不對")?,
        None => raw,
    };
    let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if b64.is_empty() {
        return Err("要送 { image }：圖片的 base64".into());
    }
    if b64.len() > MAX_PICTURE * 4 / 3 + 8 {
        return Err("圖片太大了（上限 10 MB）".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()).map_err(|_| "圖片不是有效的 base64")?;
    let (mt, ext) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", "png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ("image/jpeg", "jpg")
    } else {
        return Err("只收 PNG 或 JPEG 圖片".into());
    };
    Ok((bytes, mt, ext, b64))
}

fn summary_json(s: &decompose::Summary) -> Value {
    json!({ "new_components": s.new_components, "new_modules": s.new_modules, "linked": s.linked, "new_figure": s.new_figure })
}

/// What the plan must never contain, even when the model claims a piece "is drawable": a drawable
/// new item, or a removal. (`plan` builds none; this keeps a later change there from slipping through.)
fn plan_is_safe(p: &Proposal) -> bool {
    p.changes.iter().all(|c| c.op != "remove" && !(c.op == "add" && c.item.as_ref().is_none_or(|i| i.entity)))
}

async fn figure(State(s): State<Shared>, body: Option<Json<FigureBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { image }：圖片的 base64") };
    let (bytes, mt, ext, b64) = match decode_picture(&b.image) {
        Ok(x) => x,
        Err(e) => return bad(StatusCode::BAD_REQUEST, e),
    };
    let base = match brain(&s).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let existing = match library(&base).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (system, user) = decompose::prompt(&existing, b.note.as_deref().unwrap_or(""));
    let (v, engine) = match ask(&s, TASK_FIGURE, b.engine.as_deref(), &system, user, Some(Image::from_bytes(mt, &bytes)), decompose::schema()).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let reply = match decompose::parse(&v) {
        Ok(r) => r,
        Err(e) => return bad(StatusCode::BAD_GATEWAY, e.to_string()),
    };
    let name = format!("le-{}.{ext}", lore_know::stable_key(&b64));
    let proposal = decompose::plan(&reply, &existing, FLOW_REFS, BY, &name);
    if !plan_is_safe(&proposal) {
        return bad(StatusCode::BAD_GATEWAY, "拆出來的結果不合規則（不能是可繪製的實體或刪除），沒有提出");
    }
    let sum = decompose::summary(&proposal);
    let image = json!({ "name": name, "data_url": format!("data:{mt};base64,{b64}") });
    let label = crate::llm::engine_label(&engine);
    if proposal.changes.is_empty() {
        return Json(json!({ "proposal": null, "summary": summary_json(&sum), "image": image, "message": "這張圖的每個部分圖庫裡都已經有了，沒有要新增的", "engine": engine, "label": label })).into_response();
    }
    Json(json!({ "proposal": proposal, "summary": summary_json(&sum), "image": image, "engine": engine, "label": label })).into_response()
}

// ── part B: one item changed on the author's instruction ──

#[derive(Deserialize)]
struct EditBody {
    #[serde(default)]
    id: String,
    #[serde(default)]
    instruction: String,
    engine: Option<String>,
}

const EDIT_RULES: &str = "\
You edit one item of the author's personal knowledge library, following his instruction.
Rules:
- Change only what the instruction asks for. Keep everything else exactly as it is: leave out every \
field, tag list and trigger list that you do not change.
- Never invent facts, names, numbers or sources that neither the instruction nor the item gives. If \
the instruction asks for something the item cannot hold, change nothing and say so in `note`.
- A required field must never become empty.
- Keep the item's language and writing style (mostly Traditional Chinese or English); LaTeX stays as it is.
- `fields` uses only the field names listed in the user message, each value being the whole new text \
of that field. `tags` uses only the facets domain, kind and use, each value being the whole new list \
of that facet. `triggers` is the whole new list.
- `note` is one short sentence in Traditional Chinese saying what you changed, or why nothing changed.
Reply with one JSON value that follows the given schema and nothing else.";

fn edit_schema() -> Value {
    let strs = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "fields": { "type": "object", "additionalProperties": { "type": "string" } },
            "tags": { "type": "object", "properties": { "domain": strs, "kind": strs, "use": strs } },
            "triggers": strs,
            "note": { "type": "string" }
        },
        "required": ["note"]
    })
}

/// The fields an instruction may change for this item: all of the format's, but a figure's
/// description only.
fn editable<'a>(item: &Item, def: &'a FormatDef) -> Vec<&'a lore_know::FieldDef> {
    def.fields.iter().filter(|f| item.format != format::FIGURE || f.name == FIGURE_EDITABLE_FIELD).collect()
}

/// The request text: the item as JSON, the editable fields with their labels, the instruction.
fn edit_user(item: &Item, def: &FormatDef, instruction: &str) -> String {
    let shown = json!({ "title": item.title, "fields": item.fields, "tags": item.tags, "triggers": item.triggers });
    let mut out = format!("The item (JSON):\n{}\n\nFields you may change (name: label, kind, required or optional):\n", serde_json::to_string_pretty(&shown).unwrap_or_default());
    for f in editable(item, def) {
        out.push_str(&format!("- {}: {}, {}, {}\n", f.name, f.label, f.kind, if f.required { "required" } else { "optional" }));
    }
    if item.format == format::FIGURE {
        out.push_str("This item is a figure: only its title, description, tags and triggers can change here; its drawing is never touched.\n");
    }
    out.push_str("Tag facets you may change: domain, kind, use.\n");
    out.push_str(&format!("\nThe author's instruction:\n{}", clip(instruction.trim(), INSTRUCTION_CHARS)));
    out
}

fn clean_list(v: &Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    let mut out: Vec<String> = vec![];
    for s in a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    Some(out)
}

/// What the model's changes came to once checked in code.
#[derive(Debug)]
pub struct Edited {
    pub after: Item,
    /// `title`, `fields.<name>`, `tags.<facet>`, `triggers`.
    pub changed: Vec<String>,
    /// Parts of the reply that were ignored, with the reason (one line each).
    pub dropped: Vec<String>,
    pub note: String,
}

/// Apply the model's reply to `item` for its format. `Err` = one line: a required part would become empty.
pub fn apply_edit(item: &Item, def: &FormatDef, reply: &Value) -> Result<Edited, String> {
    let figure = item.format == format::FIGURE;
    let mut after = item.clone();
    let (mut changed, mut dropped): (Vec<String>, Vec<String>) = (vec![], vec![]);

    if let Some(t) = reply["title"].as_str() {
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            return Err("模型想把名稱清空，名稱不能是空的，沒有提出".into());
        }
        if t != item.title.trim() {
            after.title = t;
            changed.push("title".into());
        }
    }

    for (name, v) in reply["fields"].as_object().unwrap_or(&Map::new()) {
        let Some(f) = def.fields.iter().find(|f| f.name == *name) else {
            dropped.push(format!("fields.{name}：這一類沒有這個欄位"));
            continue;
        };
        if figure && name != FIGURE_EDITABLE_FIELD {
            dropped.push(format!("fields.{name}：圖形的「{}」不能在這裡改", f.label));
            continue;
        }
        let text = match v {
            Value::Null => continue,
            Value::String(s) => s.trim().to_string(),
            Value::Number(_) | Value::Bool(_) => v.to_string(),
            _ => {
                dropped.push(format!("fields.{name}：不是文字"));
                continue;
            }
        };
        if item.fields.get(name).is_some_and(|old| !old.is_null() && !old.is_string()) {
            dropped.push(format!("fields.{name}：這個欄位不是文字，不能在這裡改"));
            continue;
        }
        if f.required && text.is_empty() {
            return Err(format!("模型想把必填的「{}」清空，沒有提出", f.label));
        }
        if text != item.field(name).trim() {
            after.fields.insert(name.clone(), Value::String(text));
            changed.push(format!("fields.{name}"));
        }
    }

    for (fct, v) in reply["tags"].as_object().unwrap_or(&Map::new()) {
        if !FACETS.contains(&fct.as_str()) {
            dropped.push(format!("tags.{fct}：只能改 {}", FACETS.join("、")));
            continue;
        }
        let Some(mut new) = clean_list(v) else { continue };
        new.sort();
        let mut old = item.tags.get(fct).cloned().unwrap_or_default();
        old.sort();
        if new != old {
            after.tags.insert(fct.clone(), new);
            changed.push(format!("tags.{fct}"));
        }
    }

    if let Some(new) = clean_list(&reply["triggers"])
        && new != item.triggers
    {
        after.triggers = new;
        changed.push("triggers".into());
    }

    after.tidy();
    let note = reply["note"].as_str().map(one_line).filter(|n| !n.is_empty()).unwrap_or_else(|| "照你的指示修改".into());
    Ok(Edited { after, changed, dropped, note })
}

async fn edit(State(s): State<Shared>, body: Option<Json<EditBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { id, instruction }") };
    if b.id.trim().is_empty() || b.instruction.trim().is_empty() {
        return bad(StatusCode::BAD_REQUEST, "要送 { id, instruction }：改哪一條、怎麼改");
    }
    let base = match brain(&s).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let got = match brain_get(&base, &format!("/api/hub/items/{}", b.id.trim())).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Ok(item) = serde_json::from_value::<Item>(got["item"].clone()) else { return bad(StatusCode::BAD_GATEWAY, "L-Brain 回的這一條看不懂") };
    let cats = match brain_get(&base, "/api/hub/categories").await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let def = cats["formats"].as_array().into_iter().flatten().filter_map(|f| serde_json::from_value::<FormatDef>(f.clone()).ok()).find(|f| f.id == item.format);
    let Some(def) = def else { return bad(StatusCode::BAD_GATEWAY, format!("L-Brain 沒有「{}」這個格式", item.format)) };

    let (v, engine) = match ask(&s, TASK_EDIT, b.engine.as_deref(), EDIT_RULES, edit_user(&item, &def, &b.instruction), None, edit_schema()).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let done = match apply_edit(&item, &def, &v) {
        Ok(d) => d,
        Err(e) => return bad(StatusCode::BAD_GATEWAY, e),
    };
    let label = crate::llm::engine_label(&engine);
    if done.changed.is_empty() {
        let message = format!("沒有要改的：{}", done.note);
        return Json(json!({ "proposal": null, "before": item, "after": item, "changed": [], "dropped": done.dropped, "note": done.note, "message": message, "engine": engine, "label": label })).into_response();
    }
    let title = format!("改「{}」：{}", item.title.trim(), clip(&done.note, 40));
    let change = Change { op: "update".into(), item: Some(done.after.clone()), note: done.note.clone(), ..Default::default() };
    let proposal = Proposal { id: String::new(), by: BY.into(), title, changes: vec![change], state: "open".into(), created_at: String::new() };
    Json(json!({ "proposal": proposal, "before": item, "after": done.after, "changed": done.changed, "dropped": done.dropped, "note": done.note, "engine": engine, "label": label })).into_response()
}

// ── posting a confirmed draft ──

#[derive(Deserialize)]
struct ProposeBody {
    proposal: Option<Proposal>,
}

async fn propose(State(s): State<Shared>, body: Option<Json<ProposeBody>>) -> Response {
    let Some(Json(ProposeBody { proposal: Some(p) })) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { proposal }：預覽裡的那一份") };
    if p.changes.is_empty() {
        return bad(StatusCode::BAD_REQUEST, "這份提案沒有任何改動");
    }
    // the same rules as the drafts: this route is not a way around them
    if !plan_is_safe(&p) {
        return bad(StatusCode::BAD_REQUEST, "提案裡有可繪製的新實體或刪除，這裡不能提出");
    }
    let base = match brain(&s).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let title = if p.title.trim().is_empty() { "L-Echo 的提案".to_string() } else { p.title.trim().to_string() };
    let sent = client().post(format!("{base}/api/hub/proposals")).json(&json!({ "by": BY, "title": title, "changes": p.changes })).send().await;
    let r = match sent {
        Ok(r) => r,
        Err(_) => return closed(),
    };
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return brain_error(status, &v);
    }
    let Some(id) = v["proposal"]["id"].as_str().filter(|i| !i.is_empty()) else { return bad(StatusCode::BAD_GATEWAY, "L-Brain 沒有回提案的代號") };
    // the hub page lists the open proposals; a page per proposal does not exist yet
    Json(json!({ "id": id, "open": format!("{base}/#/hub/proposals") })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kbctx::Pin;
    use crate::llm::Llm;
    use crate::{AppState, Shared};
    use axum::body::Body;
    use axum::extract::{Path, Query};
    use axum::http::Request;
    use http_body_util::BodyExt;
    use lore_know::{Source, level};
    use lore_llm::MockProvider;
    use lore_llm::router::MockFactory;
    use std::collections::HashMap;
    use std::path::Path as FsPath;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    /// 1x1-ish PNG magic: only the first bytes are read.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-synthetic-test-picture-";

    fn png_b64() -> String {
        base64::engine::general_purpose::STANDARD.encode(PNG)
    }

    fn copy_dir(from: &FsPath, to: &FsPath) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            if e.path().is_dir() { copy_dir(&e.path(), &to.join(e.file_name())) } else { std::fs::copy(e.path(), to.join(e.file_name())).unwrap(); }
        }
    }

    // ── a fake L-Brain ──

    #[derive(Default)]
    struct Fake {
        /// category → items
        items: HashMap<String, Vec<Item>>,
        /// the item `GET /api/hub/items/{id}` answers with
        one: Option<Item>,
        /// `POST /api/hub/proposals` bodies
        posted: Mutex<Vec<Value>>,
        /// `GET /api/hub/items?category=` asked
        asked: Mutex<Vec<String>>,
        /// the proposal box refuses with this status
        refuse: Option<StatusCode>,
    }

    fn formats() -> Value {
        json!([
            { "id": "expression", "label": "句子", "shape": "row", "form": true, "fields": [
                { "name": "text", "label": "句子", "kind": "text", "required": true },
                { "name": "note", "label": "備註", "kind": "text", "required": false } ] },
            { "id": "figure", "label": "圖形", "shape": "figure", "form": false, "fields": [
                { "name": "what", "label": "種類", "kind": "choice", "required": true },
                { "name": "kind", "label": "節點種類", "kind": "text", "required": false },
                { "name": "style", "label": "畫法", "kind": "text", "required": false },
                { "name": "description", "label": "說明", "kind": "markdown", "required": false },
                { "name": "params", "label": "參數", "kind": "text", "required": false },
                { "name": "image", "label": "參考圖", "kind": "text", "required": false } ] }
        ])
    }

    async fn fake_brain(f: Fake) -> (String, Arc<Fake>) {
        let f = Arc::new(f);
        let (a, b, c, d) = (f.clone(), f.clone(), f.clone(), f.clone());
        let app = axum::Router::new()
            .route(
                "/api/hub/items",
                axum::routing::get(move |Query(q): Query<HashMap<String, String>>| {
                    let f = a.clone();
                    async move {
                        let cat = q.get("category").cloned().unwrap_or_default();
                        f.asked.lock().unwrap().push(format!("{cat}&limit={}", q.get("limit").cloned().unwrap_or_default()));
                        axum::Json(json!({ "items": f.items.get(&cat).cloned().unwrap_or_default(), "total": 0, "tags": {} }))
                    }
                }),
            )
            .route(
                "/api/hub/items/{id}",
                axum::routing::get(move |Path(id): Path<String>| {
                    let f = b.clone();
                    async move {
                        match f.one.clone().filter(|i| i.id == id) {
                            Some(i) => (StatusCode::OK, axum::Json(json!({ "item": i, "contains": [], "used_by": [], "problems": [] }))),
                            None => (StatusCode::NOT_FOUND, axum::Json(json!({ "error": "找不到：這一條" }))),
                        }
                    }
                }),
            )
            .route("/api/hub/categories", axum::routing::get(move || async move { axum::Json(json!({ "categories": [], "formats": formats(), "sorts": [], "facets": ["domain", "kind", "use"] })) }))
            .route(
                "/api/hub/proposals",
                axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                    let f = c.clone();
                    async move {
                        if let Some(st) = f.refuse {
                            return (st, axum::Json(json!({ "error": "沒有這個類別" })));
                        }
                        f.posted.lock().unwrap().push(body.clone());
                        (StatusCode::CREATED, axum::Json(json!({ "proposal": { "id": "p-001", "by": body["by"], "title": body["title"], "state": "open", "changes": body["changes"] } })))
                    }
                }),
            );
        let _ = d;
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (url, f)
    }

    // ── the app under test ──

    struct App {
        _tmp: tempfile::TempDir,
        shared: Shared,
        mock: Arc<MockProvider>,
    }

    /// `replies`: what the model answers (the figure task goes to `claude:selected`, the edit task to `local:current`).
    fn app(replies: &[&str], brain: Option<String>) -> App {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let manifest = FsPath::new(env!("CARGO_MANIFEST_DIR"));
        copy_dir(&manifest.join("../../../private-example/LoreEcho"), &root.join("private-example/LoreEcho"));
        std::fs::create_dir_all(root.join("private")).unwrap();
        let mock = Arc::new(MockProvider::new(replies.iter().map(|s| s.to_string()).collect()).named("mock:model"));
        let factory = MockFactory::default().with("claude:selected", mock.clone()).with("local:current", mock.clone());
        let shared = AppState::new(lore_home::Home::at(&root), root.join("web"), Llm::with_factory(factory), None);
        shared.kb.pin(Pin { spaces: vec![], member: "ellrin".into(), url: brain });
        App { _tmp: tmp, shared, mock }
    }

    async fn post(a: &App, uri: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        let res = crate::router_isolated(a.shared.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    // ── part A fixtures ──

    fn lib_item(id: &str, cat: &str, title: &str, lvl: &str, entity: bool) -> Item {
        let mut i = Item::new(cat, format::FIGURE, title, Source::author());
        i.id = id.into();
        i.level = lvl.into();
        i.entity = entity;
        i.revision = 2;
        i.set("what", if entity { "drawing" } else { "reference" });
        i
    }

    fn reply() -> String {
        json!({
            "figure": { "name": "Image classifier", "description": "A small network.", "domain": ["machine learning"], "kind": ["architecture"] },
            "modules": [
                { "name": "Encoder", "description": "Reads the image.", "components": ["Conv layer", "Pooling layer"] },
                { "name": "Head", "components": ["Linear layer"] }
            ],
            "components": [
                { "name": "Conv layer", "domain": ["machine learning"], "kind": ["architecture"] },
                { "name": "Pooling layer" },
                { "name": "Linear layer" }
            ]
        })
        .to_string()
    }

    fn figure_body() -> Value {
        json!({ "image": png_b64(), "note": "from a vision paper" })
    }

    // ── part A tests ──

    #[tokio::test]
    async fn a_picture_becomes_a_draft_proposal_of_references() {
        let (url, _) = fake_brain(Fake::default()).await;
        let a = app(&[&reply()], Some(url));
        let (s, v) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["summary"], json!({ "new_components": 3, "new_modules": 2, "linked": 0, "new_figure": true }));
        let changes = v["proposal"]["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 6);
        for c in changes {
            let it = &c["item"];
            assert_eq!((c["op"].as_str(), it["entity"].as_bool(), it["fields"]["what"].as_str()), (Some("add"), Some(false), Some("reference")));
            assert_eq!((it["category"].as_str(), it["source"]["name"].as_str()), (Some("sc.flowrefs"), Some("le")));
        }
        assert_eq!((v["proposal"]["by"].as_str(), v["proposal"]["state"].as_str()), (Some("le"), Some("open")));
        // the figure reference carries the picture's file name; the page gets the picture to show
        let fig = changes.last().unwrap();
        let name = v["image"]["name"].as_str().unwrap();
        assert!(name.starts_with("le-") && name.ends_with(".png"), "{name}");
        assert_eq!(fig["item"]["fields"]["image"], name);
        assert!(v["image"]["data_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
        // the pieces below the figure carry no picture
        assert!(changes[0]["item"]["fields"].get("image").is_none());
        assert_eq!((v["engine"].as_str(), v["label"].as_str()), (Some("mock:model"), Some("mock")));
        println!("figure draft: {}", serde_json::to_string(&json!({ "summary": v["summary"], "image.name": v["image"]["name"], "proposal.title": v["proposal"]["title"], "n_changes": changes.len() })).unwrap());
    }

    #[tokio::test]
    async fn existing_pieces_are_linked_not_added() {
        let mut conv = lib_item("sc.flow:c1", "sc.flow", "conv layers", level::COMPONENT, true);
        conv.tag(facet::DOMAIN, "machine learning").tag(facet::KIND, "architecture");
        let enc = lib_item("sc.flowrefs:m1", "sc.flowrefs", "Encoder", level::MODULE, false);
        let (url, _) = fake_brain(Fake { items: HashMap::from([("sc.flow".into(), vec![conv]), ("sc.flowrefs".into(), vec![enc])]), ..Default::default() }).await;
        let a = app(&[&reply()], Some(url));
        let (s, v) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let titles: Vec<String> = v["proposal"]["changes"].as_array().unwrap().iter().filter(|c| c["op"] == "add").map(|c| c["item"]["title"].as_str().unwrap().to_lowercase()).collect();
        assert!(!titles.iter().any(|t| t.starts_with("conv") || t == "encoder"), "{titles:?}");
        assert_eq!(v["summary"]["new_components"], 2);
        assert_eq!(v["summary"]["new_modules"], 1);
        // the new Head and the figure name the stored ids they are made of
        let head = v["proposal"]["changes"].as_array().unwrap().iter().find(|c| c["item"]["title"] == "Image classifier").unwrap();
        assert!(head["item"]["contains"].as_array().unwrap().iter().any(|c| c == "sc.flowrefs:m1"));
        assert!(v["summary"]["linked"].as_u64().unwrap() >= 1, "{v}");
    }

    #[tokio::test]
    async fn the_plan_never_draws_or_removes() {
        // the model claims every piece is drawable and an existing one should go: ignored
        let wild = json!({
            "figure": { "name": "Net", "entity": true, "drawable": true, "remove": ["Old"] },
            "modules": [{ "name": "Block", "entity": true, "components": ["Gate"] }],
            "components": [{ "name": "Gate", "entity": true, "what": "drawing", "is_drawable": true }],
            "remove": ["sc.flow:c1"]
        })
        .to_string();
        let old = lib_item("sc.flow:c1", "sc.flow", "Old", level::COMPONENT, true);
        let (url, _) = fake_brain(Fake { items: HashMap::from([("sc.flow".into(), vec![old])]), ..Default::default() }).await;
        let a = app(&[&wild], Some(url));
        let (s, v) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let changes = v["proposal"]["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 3);
        assert!(changes.iter().all(|c| c["op"] == "add" && c["item"]["entity"] == false && c["item"]["fields"]["what"] == "reference"), "{changes:?}");
        assert!(!changes.iter().any(|c| c["op"] == "remove"));
        // and the post route refuses such a draft even when the page sends one
        let bad_draft = json!({ "proposal": { "title": "x", "changes": [{ "op": "remove", "id": "sc.flow:c1" }] } });
        let (s, _) = post(&a, "/api/actions/know/propose", bad_draft).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn nothing_is_posted_until_propose() {
        let (url, brain) = fake_brain(Fake::default()).await;
        let a = app(&[&reply()], Some(url));
        let (s, _) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::OK);
        assert!(brain.posted.lock().unwrap().is_empty(), "a draft is never posted");
    }

    #[tokio::test]
    async fn propose_posts_to_lbrain() {
        let (url, brain) = fake_brain(Fake::default()).await;
        let a = app(&[&reply()], Some(url.clone()));
        let (_, draft) = post(&a, "/api/actions/know/figure", figure_body()).await;
        let sent = json!({ "proposal": draft["proposal"] });
        let (s, v) = post(&a, "/api/actions/know/propose", sent.clone()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["id"], "p-001");
        assert_eq!(v["open"], format!("{url}/#/hub/proposals"));
        let posted = brain.posted.lock().unwrap().clone();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0]["by"], "le");
        assert_eq!(posted[0]["title"], draft["proposal"]["title"]);
        assert_eq!(posted[0]["changes"].as_array().unwrap().len(), 6);
        println!("propose request to the hub: by={} title={} changes={}; answer: {v}", posted[0]["by"], posted[0]["title"], posted[0]["changes"].as_array().unwrap().len());
        // a refusal from L-Brain is shown, as one line
        let (url2, _) = fake_brain(Fake { refuse: Some(StatusCode::BAD_REQUEST), ..Default::default() }).await;
        let b = app(&[], Some(url2));
        let (s, v) = post(&b, "/api/actions/know/propose", sent).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY);
        assert!(v["error"].as_str().unwrap().contains("沒有這個類別") && !v["error"].as_str().unwrap().contains('\n'), "{v}");
        // nothing to post
        let (s, _) = post(&a, "/api/actions/know/propose", json!({})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = post(&a, "/api/actions/know/propose", json!({ "proposal": { "title": "t", "changes": [] } })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn lbrain_closed_is_a_503() {
        let a = app(&[&reply()], None);
        let (s, v) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert!(v["error"].as_str().unwrap().contains("L-Brain 沒有開著"), "{v}");
        assert!(a.mock.seen.lock().unwrap().is_empty(), "no model call is spent when the proposal could not be posted");
        let draft = json!({ "proposal": { "title": "t", "changes": [{ "op": "add", "item": { "title": "x", "entity": false } }] } });
        let (s, v) = post(&a, "/api/actions/know/propose", draft).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert!(v["error"].as_str().unwrap().contains("L-Brain 沒有開著"), "{v}");
        let (s, _) = post(&a, "/api/actions/know/edit", json!({ "id": "a:b", "instruction": "x" })).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        // an address nobody listens on is the same line
        let b = app(&[&reply()], Some("http://127.0.0.1:9".into()));
        let (s, v) = post(&b, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    }

    #[tokio::test]
    async fn a_reply_without_a_figure_is_refused() {
        let (url, _) = fake_brain(Fake::default()).await;
        for r in [json!({ "figure": { "name": "  " }, "modules": [] }), json!({ "modules": [] })] {
            let a = app(&[&r.to_string()], Some(url.clone()));
            let (s, v) = post(&a, "/api/actions/know/figure", figure_body()).await;
            assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
            let e = v["error"].as_str().unwrap();
            assert!((e.contains("沒有名稱") || e.contains("缺少 figure")) && !e.contains('\n'), "{e}");
        }
        // not a picture at all
        let a = app(&[&reply()], Some(url));
        let (s, _) = post(&a, "/api/actions/know/figure", json!({ "image": base64::engine::general_purpose::STANDARD.encode(b"just text") })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = post(&a, "/api/actions/know/figure", json!({})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_request_carries_the_picture_and_the_library_names() {
        let conv = lib_item("sc.flow:c1", "sc.flow", "Conv layer", level::COMPONENT, true);
        let enc = lib_item("sc.flowrefs:m1", "sc.flowrefs", "Transformer encoder", level::MODULE, false);
        let (url, brain) = fake_brain(Fake { items: HashMap::from([("sc.flow".into(), vec![conv]), ("sc.flowrefs".into(), vec![enc])]), ..Default::default() }).await;
        let a = app(&[&reply()], Some(url));
        let (s, _) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::OK);
        let req = a.mock.seen.lock().unwrap()[0].clone();
        let m = req.messages.last().unwrap();
        assert!(m.has_images(), "the picture is attached");
        assert!(req.has_images() && !req.web_search);
        let text = m.text();
        assert!(text.contains("Conv layer [component]") && text.contains("Transformer encoder [module]") && text.contains("from a vision paper"), "{text}");
        assert!(req.json_schema.is_some(), "the decomposition schema is attached");
        assert_eq!(*brain.asked.lock().unwrap(), vec!["sc.flow&limit=1000".to_string(), "sc.flowrefs&limit=1000".to_string()]);
        // what the model is told
        println!("figure system prompt:\n{}\n---\nuser text:\n{text}", req.system);
    }

    #[tokio::test]
    async fn everything_already_in_the_library_is_no_proposal() {
        let r = json!({ "figure": { "name": "Net", "domain": ["ml"] }, "modules": [{ "name": "Block", "components": ["Gate"] }], "components": [{ "name": "Gate", "domain": ["ml"] }] }).to_string();
        let tagged = |id: &str, cat: &str, t: &str, l: &str| {
            let mut i = lib_item(id, cat, t, l, true);
            i.tag(facet::DOMAIN, "ml");
            i
        };
        let mut fig = tagged("sc.flow:f", "sc.flow", "Net", level::FIGURE);
        let mut blk = tagged("sc.flow:b", "sc.flow", "Block", level::MODULE);
        let gate = tagged("sc.flow:g", "sc.flow", "Gate", level::COMPONENT);
        blk.contains = vec!["sc.flow:g".into()];
        fig.contains = vec!["sc.flow:b".into()];
        let (url, _) = fake_brain(Fake { items: HashMap::from([("sc.flow".into(), vec![fig, blk, gate])]), ..Default::default() }).await;
        let a = app(&[&r], Some(url));
        let (s, v) = post(&a, "/api/actions/know/figure", figure_body()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["proposal"].is_null(), "{v}");
        assert!(v["message"].as_str().unwrap().contains("都已經有了"));
        assert_eq!(v["summary"]["new_components"], 0);
    }

    // ── part B fixtures ──

    fn sentence() -> Item {
        let mut i = Item::new("pc.expressions", "expression", "Ablation study", Source::author());
        i.id = "pc.expressions:e1".into();
        i.revision = 4;
        i.set("text", "We run an ablation study.").set("note", "");
        i.tag(facet::DOMAIN, "machine learning");
        i.triggers = vec!["ablation".into()];
        i
    }

    fn figure_ref() -> Item {
        let mut i = lib_item("sc.flowrefs:f1", "sc.flowrefs", "Sketch", level::FIGURE, false);
        i.set("description", "old text").set("params", "{\"a\":1}").set("kind", "nn").set("style", "plain").set("image", "le-x.png");
        i
    }

    async fn edit_app(item: Item, replies: &[&str]) -> (App, Arc<Fake>) {
        let (url, brain) = fake_brain(Fake { one: Some(item), ..Default::default() }).await;
        (app(replies, Some(url)), brain)
    }

    // ── part B tests ──

    #[tokio::test]
    async fn an_instruction_becomes_one_update_change() {
        let r = json!({ "fields": { "note": "Used in the Method section." }, "tags": { "use": ["paper"] }, "note": "加上備註並補了用途標籤" }).to_string();
        let (a, brain) = edit_app(sentence(), &[&r]).await;
        let (s, v) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:e1", "instruction": "備註寫它用在方法那一節，標成論文用" })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let changes = v["proposal"]["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["op"], "update");
        let it = &changes[0]["item"];
        assert_eq!((it["id"].as_str(), it["revision"].as_u64()), (Some("pc.expressions:e1"), Some(4)), "the whole item with the revision read");
        assert_eq!(it["fields"]["note"], "Used in the Method section.");
        assert_eq!(it["fields"]["text"], "We run an ablation study.");
        assert_eq!(it["tags"]["use"], json!(["paper"]));
        assert_eq!(it["tags"]["domain"], json!(["machine learning"]));
        assert_eq!(changes[0]["note"], "加上備註並補了用途標籤");
        assert_eq!(v["changed"], json!(["fields.note", "tags.use"]));
        assert_eq!(v["before"]["fields"]["note"], "");
        assert_eq!(v["after"], *it);
        assert_eq!((v["proposal"]["by"].as_str(), v["proposal"]["id"].as_str()), (Some("le"), Some("")));
        assert!(brain.posted.lock().unwrap().is_empty(), "a draft is not posted");
        // the same propose route posts it
        let (s, p) = post(&a, "/api/actions/know/propose", json!({ "proposal": v["proposal"] })).await;
        assert_eq!(s, StatusCode::OK, "{p}");
        assert_eq!(brain.posted.lock().unwrap().len(), 1);
        let req = a.mock.seen.lock().unwrap()[0].clone();
        assert!(!req.web_search && !req.has_images());
        println!("edit system prompt:\n{}\n---\nuser text:\n{}", req.system, req.messages[0].text());
        println!("edit draft: changed={} dropped={} note={}", v["changed"], v["dropped"], v["note"]);
    }

    #[tokio::test]
    async fn only_named_fields_change() {
        // the model sends the other field unchanged as well, plus a null: neither counts
        let r = json!({ "title": "Ablation study", "fields": { "text": "We run an ablation study on the encoder.", "note": "", "x": null }, "note": "補上對象" }).to_string();
        let (a, _) = edit_app(sentence(), &[&r]).await;
        let (s, v) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:e1", "instruction": "句子補上 on the encoder" })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["changed"], json!(["fields.text"]));
        let it = &v["after"];
        assert_eq!(it["title"], "Ablation study");
        assert_eq!(it["fields"]["text"], "We run an ablation study on the encoder.");
        assert_eq!(it["triggers"], json!(["ablation"]));
        assert_eq!(it["tags"], v["before"]["tags"]);
        // the item as the model saw it was given as JSON
        let u = a.mock.seen.lock().unwrap()[0].messages[0].text();
        assert!(u.contains("\"We run an ablation study.\"") && u.contains("- text: 句子, text, required") && u.contains("- note: 備註, text, optional"), "{u}");
    }

    #[tokio::test]
    async fn unknown_fields_are_dropped_and_listed() {
        let r = json!({ "fields": { "colour": "red", "note": "ok" }, "tags": { "mood": ["x"], "kind": ["sentence"] }, "note": "n" }).to_string();
        let (a, _) = edit_app(sentence(), &[&r]).await;
        let (s, v) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:e1", "instruction": "x" })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["after"]["fields"].get("colour").is_none());
        assert_eq!(v["after"]["fields"]["note"], "ok");
        let dropped: Vec<&str> = v["dropped"].as_array().unwrap().iter().map(|d| d.as_str().unwrap()).collect();
        assert!(dropped.iter().any(|d| d.starts_with("fields.colour")) && dropped.iter().any(|d| d.starts_with("tags.mood")), "{dropped:?}");
        assert_eq!(v["changed"], json!(["fields.note", "tags.kind"]));
    }

    #[tokio::test]
    async fn a_required_field_cannot_be_emptied() {
        for r in [json!({ "fields": { "text": "   " }, "note": "n" }), json!({ "title": " ", "note": "n" })] {
            let (a, brain) = edit_app(sentence(), &[&r.to_string()]).await;
            let (s, v) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:e1", "instruction": "清掉它" })).await;
            assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
            let e = v["error"].as_str().unwrap();
            assert!(e.contains("不能") || e.contains("清空"), "{e}");
            assert!(!e.contains('\n') && brain.posted.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn a_figures_drawing_is_never_changed() {
        let r = json!({
            "title": "Sketch of the encoder",
            "fields": { "description": "new text", "params": "{\"a\":2}", "kind": "unet", "style": "x", "what": "drawing", "image": "evil.png" },
            "tags": { "kind": ["architecture"] },
            "triggers": ["encoder"],
            "level": "module", "entity": true,
            "note": "改名和說明"
        })
        .to_string();
        let (a, _) = edit_app(figure_ref(), &[&r]).await;
        let (s, v) = post(&a, "/api/actions/know/edit", json!({ "id": "sc.flowrefs:f1", "instruction": "改名，說明換新，順便把畫法改掉" })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["changed"], json!(["title", "fields.description", "tags.kind", "triggers"]));
        let (b, af) = (&v["before"], &v["after"]);
        for f in ["params", "kind", "style", "what", "image"] {
            assert_eq!(b["fields"][f], af["fields"][f], "{f} untouched");
        }
        assert_eq!((af["level"].as_str(), af["entity"].as_bool()), (Some("figure"), Some(false)));
        assert_eq!(af["fields"]["description"], "new text");
        assert_eq!(v["dropped"].as_array().unwrap().len(), 5, "{}", v["dropped"]);
        // the model was only offered what it may change
        let u = a.mock.seen.lock().unwrap()[0].messages[0].text();
        assert!(u.contains("- description:") && !u.contains("- params:") && !u.contains("- what:"), "{u}");
    }

    #[tokio::test]
    async fn no_change_is_no_proposal() {
        let r = json!({ "fields": { "text": "We run an ablation study." }, "tags": { "domain": ["machine learning"] }, "triggers": ["ablation"], "note": "原本就是這樣" }).to_string();
        let (a, brain) = edit_app(sentence(), &[&r]).await;
        let (s, v) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:e1", "instruction": "句子改成 We run an ablation study." })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["proposal"].is_null());
        assert_eq!(v["changed"], json!([]));
        assert!(v["message"].as_str().unwrap().contains("沒有要改的"), "{v}");
        assert!(brain.posted.lock().unwrap().is_empty());
        // a missing item, and a missing instruction
        let (s, _) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:nope", "instruction": "x" })).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = post(&a, "/api/actions/know/edit", json!({ "id": "pc.expressions:e1", "instruction": " " })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
}
