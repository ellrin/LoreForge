//! The editor on its own with a scripted stand-in for the model (screenshots, UI checks; never
//! a real engine): `cargo run -p lore-plot --example serve -- 7990` → http://127.0.0.1:7990/plot/
use std::sync::Arc;

use axum::routing::get;
use serde_json::{Value, json};

struct Scripted;

#[async_trait::async_trait]
impl lore_plot::propose::Llm for Scripted {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        let text = format!("{:?}", req.messages);
        if task == "equation-plot" && !text.contains("focal") && !text.contains("gamma") {
            // a loss that is not a template (synthetic margin softmax): the general path; one expectation is false on purpose
            return Ok((json!({ "match": "custom", "expr": "let(t, s * (c - m), logsumexp(vec(j, 1, K, if(j == 1, t, s * cneg))) - t)",
                "x": { "name": "c", "label": "$\\cos\\theta_y$", "min": -1, "max": 1 },
                "vars": [{ "name": "s", "label": "$s$", "role": "param", "value": 30, "min": 1, "max": 64, "step": 1 }, { "name": "m", "label": "$m$", "role": "param", "value": 0.35, "min": 0, "max": 0.8, "step": 0.05 },
                         { "name": "K", "label": "$K$", "role": "param", "value": 10, "min": 2, "max": 1000, "step": 1, "integer": true, "log": true }, { "name": "cneg", "label": "$\\cos\\theta_j$", "role": "param", "value": 0, "min": -1, "max": 1, "step": 0.05 }],
                "reduction": ["$K = 10$ 類；另外 $K - 1$ 類的 $\\cos\\theta_j$ 都相同", "$s$、$m$ 用論文的值（30、0.35）"],
                "properties": [{ "kind": "nonneg", "text": "≥ 0" }, { "kind": "decreasing", "text": "隨 $\\cos\\theta_y$ 遞減" }, { "kind": "increasing", "var": "m", "text": "$m$ 越大越大" },
                               { "kind": "decreasing", "var": "s", "text": "$s$ 越大越小" }],
                "y_label": "$\\mathcal{L}$", "animate": "m" }), "scripted".into()));
        }
        if task == "equation-plot" {
            // a focal-loss equation → the template with the paper's values (synthetic)
            return Ok((json!({ "match": "template", "template": "focal", "paper_values": [{ "name": "gamma", "value": 2 }, { "name": "alpha", "value": 0.25 }],
                "reduction": [], "properties": [{ "kind": "decreasing", "var": "alpha", "text": "deliberately wrong: ↓ in $\\alpha_t$" }], "animate": "gamma" }), "scripted".into()));
        }
        if task.starts_with("plot-assist") && text.contains("Mode: adjust") {
            return Ok((json!({ "note": "y 軸上段拉長三倍、圖例移到圖外上方、寬改成雙欄。", "changes": [
                { "path": "/panels/0/yaxis/scale", "value": "stretch", "why": "差異集中在 0.8 以上" },
                { "path": "/panels/0/yaxis/segments", "value": [[0.8, 1.0, 3]] },
                { "path": "/panels/0/legend/pos", "value": "above", "why": "原本擋到長條" },
                { "path": "/size/w", "value": 175 } ] }), "scripted".into()));
        }
        // alternatives over the first table: grouped bars, small multiples, a heatmap
        let cols: Vec<String> = req.json_schema.as_ref().and_then(|s| s.pointer("/properties/alternatives/items/properties/panels/items/properties/x/enum")).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let x = cols.first().cloned().unwrap_or_default();
        let ys: Vec<Value> = cols.iter().skip(1).filter(|c| !c.ends_with("_sd")).map(|c| json!({ "y": c, "err": format!("{c}_sd"), "label": c })).collect();
        let ys_ok: Vec<Value> = ys.iter().map(|s| { let mut s = s.clone(); if !cols.contains(&s["err"].as_str().unwrap_or("").to_string()) { s.as_object_mut().unwrap().remove("err"); } s }).collect();
        Ok((json!({ "note": "三種畫法：比較方法用分組長條最清楚。", "alternatives": [
            { "why": "分組長條：每個指標並排比較，我們的方法用強調色。", "panels": [{ "type": "bar", "format": "B3", "x": x, "series": ys_ok, "legend": { "pos": "above" } }] },
            { "why": "點線：看各方法在指標間的走勢。", "panels": [{ "type": "line", "x": x, "series": ys_ok }] },
            { "why": "雷達：一眼看整體輪廓（不適合精確比較）。", "panels": [{ "type": "radar", "x": x, "series": ys_ok }] } ] }), "scripted".into()))
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(7990);
    let home = lore_home::Home::discover(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
    let mut app = axum::Router::new()
        .route("/theme.css", get(|| async { ([("content-type", "text/css; charset=utf-8")], include_str!("../../../ui/theme.css")) }))
        .route("/theme.js", get(|| async { ([("content-type", "text/javascript; charset=utf-8")], format!("window.LORE_THEME = null;\n{}", include_str!("../../../ui/theme.js"))) }));
    for (path, mime, body) in lore_shell::FILES {
        app = app.route(path, get(move || async move { ([("content-type", *mime)], *body) }));
    }
    let app = app.merge(lore_plot::router(lore_plot::Plot::new(home).with_llm(Arc::new(Scripted))));
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    println!("http://127.0.0.1:{port}/plot/");
    axum::serve(l, app).await.unwrap();
}
