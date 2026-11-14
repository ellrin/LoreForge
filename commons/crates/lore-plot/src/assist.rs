//! The figure assistant (task `plot-assist`, class `tidy`; `plot-assist-vision` with a rendering
//! of the current figure). Its only output is spec values for the existing chart functions —
//! never code:
//!
//! - **alternatives**: data (+ a sentence) → 2–4 alternative figures, each with one sentence on
//!   what it shows best; built like `plot-propose` (closed lists, columns from the headers).
//! - **adjust**: the current spec + a request («拉寬一點», «把 0.8–1.0 拉長», «legend 擋到曲線») →
//!   concrete changes `[{ path, value, why }]` (JSON pointers into the spec, never into its data),
//!   applied to a copy and checked with [`Spec::problems`]; the page applies them as one undo step.
//!
//! The prompt (`prompts/assist.md`) is public and neutral; at run time it gets the format
//! catalogue, the style preset and venue, and the author's own preferences from his private folder
//! (`private/shared/plot/assist.md`, his default style) through lore-home.

use std::collections::BTreeMap;

use lore_llm::{Image, LlmRequest, Message, TaskTable};
use serde_json::{Value, json};

use crate::propose::Llm;
use crate::spec::{Spec, TYPES, Table};

pub const TASK: &str = "plot-assist";
pub const TASK_VISION: &str = "plot-assist-vision";
pub const PROMPT: &str = include_str!("../prompts/assist.md");

/// Both lore-plot tasks (`plot-propose`, `plot-assist`, `plot-assist-vision`), for an app's task table.
pub fn task_table() -> TaskTable {
    crate::propose::task_table()
}

/// One line per format: `B1 bar · 少數類別、差異在 y · single 1.33:1 · suggest {…} · note`.
pub fn catalogue_text(formats_json: &str) -> String {
    let Ok(cat) = serde_json::from_str::<Value>(formats_json) else { return String::new() };
    let mut s = String::new();
    for f in cat["formats"].as_array().into_iter().flatten() {
        s += &format!(
            "{} {} · {} · {} {:.2}:1{}{}\n",
            f["code"].as_str().unwrap_or(""),
            f["chart"].as_str().unwrap_or(""),
            f["label"].as_str().unwrap_or(""),
            f["width"].as_str().unwrap_or(""),
            f["aspect"].as_f64().unwrap_or(1.0),
            f.get("suggest").map(|v| format!(" · typical {v}")).unwrap_or_default(),
            f.get("note").and_then(Value::as_str).map(|n| format!(" · {n}")).unwrap_or_default()
        );
    }
    if let Some(r) = cat.get("rules") {
        s += &format!("Rules (tunable defaults, not limits): {r}\n");
    }
    s
}

pub struct Context {
    pub catalogue: String,
    /// The style preset in force (resolved JSON, shortened).
    pub style_preset: String,
    pub venue: String,
    /// The author's preferences (private text + his default style), may be empty.
    pub preferences: String,
}

/// The system prompt with its placeholders filled.
pub fn system(ctx: &Context) -> String {
    let mut s = PROMPT.replace("{catalogue}", ctx.catalogue.trim()).replace("{style_preset}", ctx.style_preset.trim()).replace("{venue}", ctx.venue.trim());
    if !ctx.preferences.trim().is_empty() {
        s += &format!("\n\nThe author's own preferences (follow them unless clarity suffers):\n{}\n", ctx.preferences.trim());
    }
    s
}

fn columns(tables: &BTreeMap<String, Table>) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for t in tables.values() {
        for c in &t.columns {
            if !v.contains(c) {
                v.push(c.clone());
            }
        }
    }
    v
}

fn tables_text(tables: &BTreeMap<String, Table>) -> String {
    let mut s = String::new();
    for (id, t) in tables {
        s += &format!("table {id} ({} rows): {}\n", t.rows.len(), t.columns.join(" | "));
        for r in t.rows.iter().take(6) {
            s += &r.iter().map(|v| match v {
                Value::String(x) => x.clone(),
                Value::Null => String::new(),
                o => o.to_string(),
            }).collect::<Vec<_>>().join(" | ");
            s.push('\n');
        }
    }
    s
}

const LEGEND: &[&str] = &["best", "upper right", "upper left", "lower right", "lower left", "center right", "outside right", "below", "above", "direct"];
const SCALES: &[&str] = &["linear", "log", "symlog", "stretch"];

fn axis_schema() -> Value {
    json!({ "type": "object", "properties": {
        "label": { "type": "string" }, "unit": { "type": "string" },
        "min": { "type": ["number", "null"] }, "max": { "type": ["number", "null"] },
        "scale": { "type": "string", "enum": SCALES },
        "segments": { "type": "array", "items": { "type": "array", "items": { "type": "number" } } } } })
}

/// Schema of the alternatives reply: every list closed (types, columns, tables, formats, places).
pub fn alternatives_schema(tables: &BTreeMap<String, Table>, styles: &[String], formats: &[String]) -> Value {
    let mut cols: Vec<Value> = columns(tables).into_iter().map(Value::from).collect();
    cols.push(Value::Null);
    let col = json!({ "type": ["string", "null"], "enum": cols });
    let ids: Vec<&String> = tables.keys().collect();
    let panel = json!({ "type": "object", "required": ["type", "series"], "properties": {
        "type": { "type": "string", "enum": TYPES },
        "format": { "type": "string", "enum": formats },
        "table": { "type": "string", "enum": ids },
        "x": col, "y": col, "group": col, "event": col, "z": col, "hue": col,
        "series": { "type": "array", "items": { "type": "object", "properties": {
            "y": col, "err": col, "y2": col, "score": col, "truth": col, "x": col,
            "label": { "type": "string" }, "ours": { "type": "boolean" } } } },
        "title": { "type": "string" },
        "xaxis": axis_schema(), "yaxis": axis_schema(),
        "legend": { "type": "object", "properties": { "pos": { "type": "string", "enum": LEGEND } } },
        "opts": { "type": "object" } } });
    json!({ "type": "object", "required": ["alternatives", "note"], "properties": {
        "alternatives": { "type": "array", "items": { "type": "object", "required": ["panels", "why"], "properties": {
            "why": { "type": "string" },
            "style": { "type": "string", "enum": styles },
            "grid_cols": { "type": "integer" },
            "panels": { "type": "array", "items": panel } } } },
        "note": { "type": "string" },
        "question": { "type": "string" } } })
}

/// Schema of the adjust reply: changes as JSON pointers and values.
pub fn adjust_schema() -> Value {
    json!({ "type": "object", "required": ["changes", "note"], "properties": {
        "changes": { "type": "array", "items": { "type": "object", "required": ["path"], "properties": {
            "path": { "type": "string" }, "value": {}, "why": { "type": "string" } } } },
        "note": { "type": "string" },
        "question": { "type": "string" } } })
}

pub fn request_alternatives(ctx: &Context, tables: &BTreeMap<String, Table>, sentence: &str, styles: &[String], formats: &[String]) -> LlmRequest {
    let user = format!(
        "Mode: alternatives. Propose 2–4 alternative figures for these data; each gets one sentence (why) on what it shows best, in Traditional Chinese.\n\n{}\nWhat the author wants (may be empty): {}\n",
        tables_text(tables),
        sentence.trim()
    );
    LlmRequest::chat(system(ctx), vec![Message::user(user)]).json(alternatives_schema(tables, styles, formats)).max_tokens(6000)
}

/// The spec without its data rows (columns and the first rows only), for the prompt.
pub fn spec_for_prompt(spec: &Value) -> Value {
    let mut s = spec.clone();
    if let Some(d) = s.get_mut("data").and_then(Value::as_object_mut) {
        for t in d.values_mut() {
            if let Some(rows) = t.get_mut("rows").and_then(Value::as_array_mut) {
                let n = rows.len();
                rows.truncate(5);
                t["row_count"] = n.into();
            }
        }
    }
    s
}

pub fn request_adjust(ctx: &Context, spec: &Value, ask: &str, image: Option<Image>) -> LlmRequest {
    let user = format!(
        "Mode: adjust. The current figure spec (data rows shortened):\n{}\n\nThe author's request: {}\n\nAnswer with `changes`: JSON pointers into the spec (\"/panels/0/yaxis/scale\", \"/size/w\", \"/set/legend.size\") and their new values; never change /data. List every change with why (Traditional Chinese).\n\n{CONVENTIONS}",
        serde_json::to_string(&spec_for_prompt(spec)).unwrap_or_default(),
        ask.trim()
    );
    let msg = match image {
        Some(img) => Message::user_with_images(format!("{user}\nThe attached picture is the current rendering."), vec![img]),
        None => Message::user(user),
    };
    LlmRequest::chat(system(ctx), vec![msg]).json(adjust_schema()).max_tokens(4000)
}

/// One alternative as a full spec (the request's data, the style's look).
fn alt_spec(a: &Value, tables: &BTreeMap<String, Table>, styles: &[String], style: &str) -> Result<Spec, String> {
    let first = tables.keys().next().cloned().ok_or("沒有資料")?;
    let panels: Vec<Value> = a["panels"]
        .as_array()
        .ok_or("沒有 panels")?
        .iter()
        .map(|p| {
            let mut q = p.clone();
            let o = q.as_object_mut().expect("an object");
            let table = o.remove("table").and_then(|t| t.as_str().map(str::to_string)).filter(|t| tables.contains_key(t)).unwrap_or_else(|| first.clone());
            o.insert("data".into(), table.into());
            for k in ["x", "y", "group", "event", "z", "hue"] {
                if o.get(k).is_some_and(Value::is_null) {
                    o.remove(k);
                }
            }
            if let Some(series) = o.get_mut("series").and_then(Value::as_array_mut) {
                for s in series.iter_mut().filter_map(Value::as_object_mut) {
                    s.retain(|_, v| !v.is_null());
                }
            }
            for k in ["xaxis", "yaxis"] {
                if let Some(ax) = o.get_mut(k).and_then(Value::as_object_mut) {
                    ax.retain(|_, v| !v.is_null());
                }
            }
            o.entry("notes").or_insert_with(|| json!([]));
            o.entry("opts").or_insert_with(|| json!({}));
            q
        })
        .collect();
    if panels.is_empty() {
        return Err("沒有任何一格".into());
    }
    let n = panels.len();
    let cols = a["grid_cols"].as_u64().map(|c| c.clamp(1, 4) as usize).unwrap_or(n.min(3)).min(n).max(1);
    let st = a["style"].as_str().filter(|s| styles.iter().any(|x| x == s)).unwrap_or(style);
    let data: serde_json::Map<String, Value> = tables.iter().map(|(k, t)| (k.clone(), serde_json::to_value(t).unwrap_or_default())).collect();
    let spec = Spec::from_value(json!({
        "lore_plot": crate::spec::VERSION, "style": st, "set": {}, "size": null,
        "grid": { "cols": cols, "rows": n.div_ceil(cols), "gap": 5, "labels": if n > 1 { "a" } else { "none" } },
        "data": data, "panels": panels, "caption": "",
    }))
    .map_err(|e| e.to_string())?;
    spec.validate().map_err(|e| e.to_string())?;
    Ok(spec)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Alternative {
    pub spec: Spec,
    pub why: String,
}

/// The usable alternatives of a reply (unusable ones are left out, with their reasons).
pub fn parse_alternatives(reply: &Value, tables: &BTreeMap<String, Table>, styles: &[String], style: &str) -> (Vec<Alternative>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for (i, a) in reply["alternatives"].as_array().into_iter().flatten().enumerate() {
        match alt_spec(a, tables, styles, style) {
            Ok(spec) => ok.push(Alternative { spec, why: a["why"].as_str().unwrap_or("").to_string() }),
            Err(e) => bad.push(format!("第 {} 個：{e}", i + 1)),
        }
    }
    (ok, bad)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Change {
    pub path: String,
    pub before: Value,
    pub after: Value,
    pub why: String,
}

/// Set `value` at JSON pointer `path` (objects created on the way; `-` appends to an array).
fn set_pointer(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let parts: Vec<String> = path.trim_start_matches('/').split('/').map(|p| p.replace("~1", "/").replace("~0", "~")).collect();
    if parts.is_empty() || parts[0].is_empty() {
        return Err("空的路徑".into());
    }
    let mut cur = root;
    for (k, p) in parts.iter().enumerate() {
        let last = k == parts.len() - 1;
        if cur.is_null() {
            *cur = json!({});
        }
        match cur {
            Value::Object(o) => {
                if last {
                    if value.is_null() { o.remove(p); } else { o.insert(p.clone(), value); }
                    return Ok(());
                }
                cur = o.entry(p.clone()).or_insert_with(|| json!({}));
            }
            Value::Array(a) => {
                if p == "-" && last {
                    a.push(value);
                    return Ok(());
                }
                let i: usize = p.parse().map_err(|_| format!("{path}：{p} 不是位置"))?;
                if i >= a.len() {
                    return Err(format!("{path}：沒有第 {i} 項"));
                }
                if last {
                    a[i] = value;
                    return Ok(());
                }
                cur = &mut a[i];
            }
            _ => return Err(format!("{path}：{p} 前面不是物件")),
        }
    }
    Ok(())
}

/// What an axis (`xaxis`, `yaxis`, `zaxis`) takes; anything else is silently ignored by the charts.
pub const AXIS_KEYS: &[&str] = &["label", "unit", "min", "max", "log", "ticks", "fmt", "scale", "segments", "minor", "reverse", "rotate", "grid", "hidden", "linthresh"];

/// A change into an axis that names a key the charts do not read (a model's `lim`, `range`): that key.
fn unknown_axis_key(path: &str, value: &Value) -> Option<String> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let at = parts.iter().position(|p| matches!(*p, "xaxis" | "yaxis" | "zaxis"))?;
    match parts.get(at + 1) {
        Some(k) => (!AXIS_KEYS.contains(k)).then(|| k.to_string()),
        None => value.as_object().and_then(|o| o.keys().find(|k| !AXIS_KEYS.contains(&k.as_str())).cloned()),
    }
}

/// How the spec reads, for the adjust request: the names the charts know.
const CONVENTIONS: &str = "Spec conventions: axes are physical — `xaxis` runs across, `yaxis` up. A dumbbell (`type: dumbbell`) and a horizontal bar chart (`opts.horizontal: true`) have the categories down the y axis and the values across the x axis, so a value range or a value label goes into `xaxis` there. An axis takes only: label, unit, min, max, log, ticks, fmt, scale, segments, minor, reverse, rotate, grid, hidden, linthresh (a range is `min` and `max`). Bars and dumbbells sort with `opts.sort` = \"asc\" | \"desc\" (no other sort keys). Remove a series by replacing the whole `series` array.";

/// Apply the reply's changes to a copy of `spec`: never into /data or /lore_plot; the result must
/// pass [`Spec::problems`]. Returns the new spec and what changed (before → after).
pub fn apply_changes(spec: &Value, reply: &Value) -> Result<(Spec, Vec<Change>), String> {
    let mut s = spec.clone();
    let mut out = Vec::new();
    for c in reply["changes"].as_array().into_iter().flatten() {
        let path = c["path"].as_str().unwrap_or("").to_string();
        if path.starts_with("/data") || path.starts_with("/lore_plot") || path.is_empty() {
            return Err(format!("不能改 {path}（資料不由助手改）"));
        }
        let before = s.pointer(&path).cloned().unwrap_or(Value::Null);
        let after = c.get("value").cloned().unwrap_or(Value::Null);
        if let Some(bad) = unknown_axis_key(&path, &after) {
            return Err(format!("座標軸沒有 `{bad}` 這個鍵（只有 {}）", AXIS_KEYS.join("、")));
        }
        set_pointer(&mut s, &path, after.clone())?;
        out.push(Change { path, before, after, why: c["why"].as_str().unwrap_or("").to_string() });
    }
    let spec = Spec::from_value(s).map_err(|e| e.to_string())?;
    spec.validate().map_err(|e| format!("改完的圖不能用：{e}"))?;
    Ok((spec, out))
}

/// What the page gets back.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Reply {
    pub alternatives: Vec<Alternative>,
    pub spec: Option<Spec>,
    pub changes: Vec<Change>,
    pub note: String,
    pub question: Option<String>,
    pub skipped: Vec<String>,
    pub engine: String,
}

pub struct AlternativesAsk<'a> {
    pub tables: &'a BTreeMap<String, Table>,
    pub sentence: &'a str,
    pub styles: &'a [String],
    pub style: &'a str,
    pub formats: &'a [String],
}

pub async fn alternatives(llm: &dyn Llm, ctx: &Context, a: &AlternativesAsk<'_>) -> Result<Reply, String> {
    let req = request_alternatives(ctx, a.tables, a.sentence, a.styles, a.formats);
    let (v, engine) = llm.complete_json(TASK, req.clone()).await?;
    let (mut alts, mut skipped) = parse_alternatives(&v, a.tables, a.styles, a.style);
    let mut v = v;
    if alts.is_empty() {
        let mut again = req;
        again.system.push_str(&format!("\n\nNone of your alternatives was usable: {}. Use only the listed column names.", skipped.join("; ")));
        let (v2, _) = llm.complete_json(TASK, again).await?;
        (alts, skipped) = parse_alternatives(&v2, a.tables, a.styles, a.style);
        v = v2;
    }
    if alts.is_empty() {
        return Err(format!("助手給的圖都不能用：{}", skipped.join("；")));
    }
    Ok(Reply { alternatives: alts, spec: None, changes: vec![], note: v["note"].as_str().unwrap_or("").into(), question: v["question"].as_str().filter(|q| !q.trim().is_empty()).map(str::to_string), skipped, engine })
}

pub async fn adjust(llm: &dyn Llm, ctx: &Context, spec: &Value, ask: &str, image: Option<Image>) -> Result<Reply, String> {
    let task = if image.is_some() { TASK_VISION } else { TASK };
    let req = request_adjust(ctx, spec, ask, image);
    let (v, engine) = llm.complete_json(task, req.clone()).await?;
    let (new, changes, v) = match apply_changes(spec, &v) {
        Ok((n, c)) => (n, c, v),
        Err(why) => {
            let mut again = req;
            again.system.push_str(&format!("\n\nYour changes could not be applied: {why}. Change only existing parts of the spec, never /data."));
            let (v2, _) = llm.complete_json(task, again).await?;
            let (n, c) = apply_changes(spec, &v2)?;
            (n, c, v2)
        }
    };
    Ok(Reply { alternatives: vec![], spec: Some(new), changes, note: v["note"].as_str().unwrap_or("").into(), question: v["question"].as_str().filter(|q| !q.trim().is_empty()).map(str::to_string), skipped: vec![], engine })
}

/// The author's preferences for the prompt: `private/shared/plot/assist.md` and his default style.
pub fn preferences(home: &lore_home::Home) -> String {
    let dir = crate::style::private_dir(home);
    let mut s = dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("assist.md")).ok()).unwrap_or_default();
    let settings = crate::style::settings(home);
    if let Some(d) = settings.default_style {
        s += &format!("\nDefault style for papers: {d}.");
    }
    if let Some(d) = settings.slide_style {
        s += &format!("\nDefault style for slides: {d}.");
    }
    s
}
