//! The LLM task `plot-propose`: data + one sentence → a figure spec the author then tunes.
//! The model picks from closed lists only (chart types, column names from the tables'
//! headers, style ids, legend places) and writes labels / a caption draft; colours, sizes
//! and coordinates always come from the style. The reply is checked against the schema,
//! then structurally ([`Spec::problems`]). Worked examples come from the author's private
//! folder (`private/shared/plot/examples/*.json`: `{ "sentence", "spec" }`), never from code.

use std::collections::BTreeMap;
use std::path::Path;

use lore_llm::{LlmRequest, Message, TaskTable};
use serde_json::{Value, json};

use crate::spec::{Spec, TYPES, Table};

pub const TASK: &str = "plot-propose";

/// The task definition, to merge into an app's task table.
pub fn task_table() -> TaskTable {
    TaskTable::parse(TASKS).expect("bundled tasks.toml parses")
}
pub const TASKS: &str = include_str!("../tasks.toml");

/// How the app runs a JSON request for a task (its own routing, engines, keys).
#[async_trait::async_trait]
pub trait Llm: Send + Sync {
    /// The reply as JSON (already schema-checked) and the engine id that answered.
    async fn complete_json(&self, task: &str, req: LlmRequest) -> Result<(Value, String), String>;
}

pub struct Ask<'a> {
    pub tables: &'a BTreeMap<String, Table>,
    pub sentence: &'a str,
    /// Style ids on offer, and the one in use.
    pub styles: &'a [String],
    pub style: &'a str,
    /// "paper" | "slide" | None.
    pub target: Option<&'a str>,
    /// (sentence, spec) worked examples.
    pub examples: &'a [(String, Value)],
}

const LEGEND: &[&str] = &["best", "upper right", "upper left", "lower right", "lower left", "outside right", "below", "above"];

fn columns(a: &Ask) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for t in a.tables.values() {
        for c in &t.columns {
            if !v.contains(c) {
                v.push(c.clone());
            }
        }
    }
    v
}

/// JSON schema of the reply; every list is closed.
pub fn schema(a: &Ask) -> Value {
    let mut cols: Vec<Value> = columns(a).into_iter().map(Value::from).collect();
    cols.push(Value::Null);
    let col = json!({ "type": ["string", "null"], "enum": cols });
    let tables: Vec<&String> = a.tables.keys().collect();
    json!({
        "type": "object",
        "required": ["panels"],
        "properties": {
            "style": { "type": "string", "enum": a.styles },
            "grid_cols": { "type": "integer" },
            "caption": { "type": "string" },
            "panels": { "type": "array", "items": {
                "type": "object",
                "required": ["type", "series"],
                "properties": {
                    "type": { "type": "string", "enum": TYPES },
                    "table": { "type": "string", "enum": tables },
                    "x": col, "y": col, "group": col, "event": col,
                    "series": { "type": "array", "items": { "type": "object", "properties": {
                        "y": col, "err": col, "y2": col, "score": col, "truth": col, "x": col,
                        "label": { "type": "string" } } } },
                    "title": { "type": "string" },
                    "xlabel": { "type": "string" },
                    "ylabel": { "type": "string" },
                    "legend": { "type": "string", "enum": LEGEND },
                    "mode": { "type": "string", "enum": ["grouped", "stacked"] },
                    "values": { "type": "boolean" },
                    "ellipse": { "type": "boolean" },
                    "points": { "type": "boolean" },
                    "log_y": { "type": "boolean" }
                }
            } }
        }
    })
}

fn system() -> String {
    "You plan scientific result figures for a researcher's papers and slides. You get data tables (column names and the first rows) and one sentence saying what the figure should show. \
Answer with JSON only, matching the schema. Choose the chart type per panel: line (trends over a numeric x), bar (methods compared on metrics; err = a standard-deviation column), \
scatter (2-D embeddings / clusters; group = the class column), box or violin (distributions: one series per column), hist, heatmap (confusion matrix: first column = row names), \
roc / pr (score + truth columns, or x/y columns of a computed curve), bland (two measurement methods: y and y2), km (survival: x = time, event, group), radar. \
Use only column names that exist. Write axis labels with units in parentheses, LaTeX maths between $…$ where symbols appear (e.g. $\\tau$, $\\mathrm{HD}_{95}$). \
One panel per metric when the sentence compares several metrics with different scales; otherwise one panel. Never choose colours, sizes or positions. \
caption: one sentence describing what is shown (no numbers, no conclusions), in English."
        .into()
}

fn table_text(id: &str, t: &Table) -> String {
    let mut s = format!("table {id} ({} rows): {}\n", t.rows.len(), t.columns.join(" | "));
    for r in t.rows.iter().take(8) {
        s += &r.iter().map(|v| match v {
            Value::String(x) => x.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }).collect::<Vec<_>>().join(" | ");
        s.push('\n');
    }
    s
}

pub fn request(a: &Ask) -> LlmRequest {
    let mut user = String::new();
    for (sentence, spec) in a.examples.iter().take(3) {
        user += &format!("Example request: {sentence}\nExample figure (the author's own): {}\n\n", serde_json::to_string(spec).unwrap_or_default());
    }
    for (id, t) in a.tables {
        user += &table_text(id, t);
    }
    user += &format!("\nStyles: {} (current: {}). Target: {}.\nRequest: {}\n", a.styles.join(", "), a.style, a.target.unwrap_or("paper"), a.sentence.trim());
    LlmRequest::chat(system(), vec![Message::user(user)]).json(schema(a)).max_tokens(4000)
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string)
}

/// The model's reply → a spec (data from the request, everything visual from the style).
pub fn to_spec(reply: &Value, a: &Ask) -> Result<Spec, String> {
    let first = a.tables.keys().next().cloned().ok_or("沒有資料")?;
    let panels: Vec<Value> = reply["panels"]
        .as_array()
        .ok_or("回答沒有 panels")?
        .iter()
        .map(|p| {
            let table = s(p, "table").filter(|t| a.tables.contains_key(t)).unwrap_or_else(|| first.clone());
            let series: Vec<Value> = p["series"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .map(|q| {
                            let mut o = serde_json::Map::new();
                            for k in ["y", "err", "y2", "score", "truth", "x", "label"] {
                                if let Some(x) = s(q, k) {
                                    o.insert(k.into(), x.into());
                                }
                            }
                            Value::Object(o)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut opts = serde_json::Map::new();
            if let Some(m) = s(p, "mode") {
                opts.insert("mode".into(), m.into());
            }
            for k in ["values", "ellipse", "points"] {
                if p[k].as_bool() == Some(true) {
                    opts.insert(k.into(), true.into());
                }
            }
            let mut yaxis = json!({ "label": s(p, "ylabel").unwrap_or_default() });
            if p["log_y"].as_bool() == Some(true) {
                yaxis["log"] = true.into();
            }
            let mut out = json!({
                "type": s(p, "type").unwrap_or_else(|| "bar".into()),
                "data": table,
                "series": series,
                "title": s(p, "title").unwrap_or_default(),
                "xaxis": { "label": s(p, "xlabel").unwrap_or_default() },
                "yaxis": yaxis,
                "legend": { "pos": s(p, "legend").unwrap_or_else(|| "best".into()) },
                "opts": opts,
                "notes": [],
            });
            for k in ["x", "y", "group", "event"] {
                if let Some(c) = s(p, k) {
                    out[k] = c.into();
                }
            }
            out
        })
        .collect();
    if panels.is_empty() {
        return Err("模型沒有給任何一格".into());
    }
    let n = panels.len();
    let cols = reply["grid_cols"].as_u64().map(|c| c.clamp(1, 4) as usize).unwrap_or(n.min(3)).min(n).max(1);
    let style = s(reply, "style").filter(|x| a.styles.contains(x)).unwrap_or_else(|| a.style.to_string());
    let data: serde_json::Map<String, Value> = a.tables.iter().map(|(k, t)| (k.clone(), serde_json::to_value(t).unwrap_or_default())).collect();
    let v = json!({
        "lore_plot": crate::spec::VERSION,
        "style": style,
        "set": {},
        "size": null,
        "grid": { "cols": cols, "rows": n.div_ceil(cols), "gap": 5, "labels": if n > 1 { "a" } else { "none" } },
        "data": data,
        "panels": panels,
        "caption": s(reply, "caption").unwrap_or_default(),
    });
    let spec = Spec::from_value(v).map_err(|e| e.to_string())?;
    spec.validate().map_err(|e| format!("模型給的圖不能用：{e}"))?;
    Ok(spec)
}

/// Ask the model; returns the spec and the engine that answered. One retry when the first
/// reply names columns that are not there.
pub async fn propose(llm: &dyn Llm, a: &Ask<'_>) -> Result<(Spec, String), String> {
    let req = request(a);
    let (v, engine) = llm.complete_json(TASK, req.clone()).await?;
    match to_spec(&v, a) {
        Ok(s) => Ok((s, engine)),
        Err(why) => {
            let mut again = req;
            again.system.push_str(&format!("\n\nYour previous answer was not usable: {why}. Use only the listed column names."));
            let (v, engine) = llm.complete_json(TASK, again).await?;
            to_spec(&v, a).map(|s| (s, engine))
        }
    }
}

/// The author's worked examples (`<dir>/*.json` with `sentence` and `spec`), at most 12.
pub fn examples(dir: &Path) -> Vec<(String, Value)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json")).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|f| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
            let mut spec = v.get("spec")?.clone();
            // the example's own data is not needed, only its shape
            if let Some(o) = spec.as_object_mut() {
                o.remove("data");
            }
            Some((v.get("sentence")?.as_str()?.to_string(), spec))
        })
        .take(12)
        .collect()
}
