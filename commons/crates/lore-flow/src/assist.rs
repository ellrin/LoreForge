//! The flow assistant beside the editor (task `flow-assist`, class `tidy`). Like
//! lore-plot's `plot-assist`, its only output is values of the figure spec
//! ([`FlowSpec`]) — never code:
//!
//! - **adjust**: a request («block 2 改成直的», «legend 放下面») → `changes`
//!   `[{ path, value, why }]` (JSON pointers into the spec), applied to a copy on
//!   the server and checked (the text parses, [`FlowSpec::problems`], it draws);
//!   the page applies the result as one undo step.
//! - **alternatives**: 2–4 layouts of the same content (template, block
//!   directions, legend place), each with one sentence; drawn as thumbnails.
//!
//! The prompt (`prompts/assist.md`) is public and neutral; the author's habits come
//! from `private/shared/flow/assist.md` through lore-home.

use lore_llm::{LlmRequest, Message};
use serde_json::{Value, json};

use crate::figure::{Catalogue, FlowSpec, render};
use crate::llm::Llm;
use crate::spec::EdgeKind;
use crate::text::NoMath;

pub const TASK: &str = "flow-assist";
pub const PROMPT: &str = include_str!("../prompts/assist.md");

/// Paths a change may touch.
pub fn allowed(path: &str) -> bool {
    let p: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id_ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    match p.as_slice() {
        ["dsl"] | ["layout"] | ["theme"] | ["types"] | ["legend"] => true,
        ["legend", "show" | "pos" | "rows"] => true,
        ["dirs" | "blocks" | "nodes" | "lines", id] => id_ok(id),
        _ => false,
    }
}

fn layouts_text(cat: &Catalogue, n: usize) -> String {
    cat.layouts
        .iter()
        .filter(|l| l.fits(n))
        .map(|l| format!("- {} — {} — {}", l.id, l.name, if l.panels.is_empty() { "any number".to_string() } else { l.panels.len().to_string() }))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn system(cat: &Catalogue, n: usize, preferences: &str) -> String {
    let lines: Vec<&str> = EdgeKind::ALL.iter().map(|k| k.as_str()).collect();
    let mut s = PROMPT.replace("{layouts}", &layouts_text(cat, n)).replace("{lines}", &lines.join(", "));
    if !preferences.trim().is_empty() {
        s += &format!("\n\nThe author's own habits for figures (follow them unless clarity suffers):\n{}\n", preferences.trim());
    }
    s
}

pub fn adjust_schema() -> Value {
    json!({ "type": "object", "additionalProperties": false, "required": ["note", "question", "changes"], "properties": {
        "note": { "type": "string" }, "question": { "type": "string" },
        "changes": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["path", "value"], "properties": {
            "path": { "type": "string" }, "value": {}, "why": { "type": "string" } } } } } })
}

pub fn alternatives_schema(layouts: &[String]) -> Value {
    json!({ "type": "object", "additionalProperties": false, "required": ["note", "alternatives"], "properties": {
        "note": { "type": "string" }, "question": { "type": "string" },
        "alternatives": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["why", "layout"], "properties": {
            "why": { "type": "string" },
            "layout": { "type": "string", "enum": layouts },
            "dirs": { "type": "object", "additionalProperties": { "type": "string", "enum": ["right", "down"] } },
            "legend_pos": { "type": "string", "enum": ["below", "above"] } } } } } })
}

/// Set `value` at a JSON pointer, creating missing objects on the way.
pub fn set_path(root: &mut Value, path: &str, value: Value) -> Result<Value, String> {
    let parts: Vec<String> = path.trim_start_matches('/').split('/').map(|s| s.replace("~1", "/").replace("~0", "~")).collect();
    let mut cur = root;
    for (i, k) in parts.iter().enumerate() {
        if !cur.is_object() {
            *cur = json!({});
        }
        let m = cur.as_object_mut().expect("object");
        if i + 1 == parts.len() {
            return Ok(m.insert(k.clone(), value).unwrap_or(Value::Null));
        }
        cur = m.entry(k.clone()).or_insert_with(|| json!({}));
    }
    Err(format!("路徑無效：{path}"))
}

/// Apply changes to a copy of `spec`; each change gets its `before`.
pub fn apply_changes(spec: &Value, changes: &[Value]) -> Result<(Value, Vec<Value>), String> {
    let mut out = spec.clone();
    let mut done = Vec::new();
    for c in changes {
        let path = c["path"].as_str().unwrap_or("");
        if !allowed(path) {
            return Err(format!("不能改 {path}"));
        }
        let before = set_path(&mut out, path, c["value"].clone())?;
        done.push(json!({ "path": path, "before": before, "after": c["value"], "why": c["why"].as_str().unwrap_or("") }));
    }
    Ok((out, done))
}

/// Does the spec hold together and draw? (the reason when not)
pub fn check(cat: &Catalogue, v: &Value) -> Result<FlowSpec, String> {
    let spec = FlowSpec::from_value(v.clone())?;
    let g = spec.graph(&cat.library)?;
    let p = spec.problems(&g);
    if !p.is_empty() {
        return Err(p.join("；"));
    }
    render(&spec, cat, None, &NoMath, spec.frame_rect(), 2)?;
    Ok(spec)
}

fn spec_text(spec: &FlowSpec) -> String {
    let mut v = spec.to_value();
    if let Some(m) = v.as_object_mut() {
        m.remove("discussion");
    }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// A request → the changed spec and the list of changes (sent back once when the
/// first answer does not hold together).
pub async fn adjust(llm: &dyn Llm, cat: &Catalogue, spec: &FlowSpec, request: &str, preferences: &str) -> Result<Value, String> {
    let n = spec.graph(&cat.library).map(|g| g.panels.len()).unwrap_or(0);
    let user = format!("Current spec:\n{}\n\nRequest: {}\n\nMode: adjust. Reply as JSON.", spec_text(spec), request.trim());
    let req = LlmRequest::chat(system(cat, n, preferences), vec![Message::user(user)]).json(adjust_schema()).max_tokens(6000);
    let base = spec.to_value();
    let (v, engine) = llm.complete_json(TASK, req.clone()).await?;
    let attempt = |v: &Value| -> Result<(Value, Vec<Value>), String> {
        let changes = v["changes"].as_array().cloned().unwrap_or_default();
        let (out, done) = apply_changes(&base, &changes)?;
        check(cat, &out)?;
        Ok((out, done))
    };
    let (out, done, v) = match attempt(&v) {
        Ok((o, d)) => (o, d, v),
        Err(why) => {
            let mut again = req;
            again.messages.push(Message::assistant(v.to_string()));
            again.messages.push(Message::user(format!("Those changes do not work: {why}. Return the whole corrected JSON object.")));
            let (v2, _) = llm.complete_json(TASK, again).await?;
            let (o, d) = attempt(&v2).map_err(|e| format!("助手的修改用不了：{e}"))?;
            (o, d, v2)
        }
    };
    Ok(json!({ "spec": out, "changes": done, "note": v["note"].as_str().unwrap_or(""), "question": v["question"].as_str().unwrap_or(""), "engine": engine }))
}

/// 2–4 layouts of the same content, each a whole spec, drawn into `width_px` thumbnails.
pub async fn alternatives(llm: &dyn Llm, cat: &Catalogue, spec: &FlowSpec, request: &str, preferences: &str, notation: Option<&lore_notation::Notation>, width_px: f64) -> Result<Value, String> {
    let g = spec.graph(&cat.library)?;
    let n = g.panels.len();
    let ids: Vec<String> = cat.layouts.iter().filter(|l| l.fits(n)).map(|l| l.id.clone()).collect();
    let user = format!("Current spec:\n{}\n\nRequest: {}\n\nMode: alternatives. Reply as JSON.", spec_text(spec), if request.trim().is_empty() { "(none: propose layouts that suit this content)" } else { request.trim() });
    let req = LlmRequest::chat(system(cat, n, preferences), vec![Message::user(user)]).json(alternatives_schema(&ids)).max_tokens(4000);
    let (v, engine) = llm.complete_json(TASK, req).await?;
    let mut out = Vec::new();
    for a in v["alternatives"].as_array().into_iter().flatten().take(4) {
        let mut s = spec.clone();
        s.layout = a["layout"].as_str().unwrap_or("").to_string();
        if let Some(d) = a["dirs"].as_object() {
            for (k, x) in d {
                if let Some(x) = x.as_str().filter(|x| *x == "right" || *x == "down")
                    && g.panels.iter().any(|p| &p.id == k)
                {
                    s.dirs.insert(k.clone(), x.to_string());
                }
            }
        }
        if let Some(p) = a["legend_pos"].as_str() {
            s.legend.pos = p.to_string();
        }
        let Ok((fig, _)) = render(&s, cat, notation, &NoMath, s.frame_rect(), 2) else { continue };
        out.push(json!({ "why": a["why"].as_str().unwrap_or(""), "spec": s.to_value(), "svg": crate::figure::preview_svg(&fig, width_px) }));
    }
    if out.is_empty() {
        return Err("助手沒有給出畫得出來的版面".into());
    }
    Ok(json!({ "alternatives": out, "note": v["note"].as_str().unwrap_or(""), "question": v["question"].as_str().unwrap_or(""), "engine": engine }))
}
