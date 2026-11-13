//! The flow editor on its own with a scripted stand-in for the model (screenshots, UI checks;
//! never a real engine) and a synthetic notation registry:
//! `cargo run -p lore-flow --example serve -- 7991` → http://127.0.0.1:7991/flow/
use std::sync::Arc;

use axum::routing::get;
use lore_llm::LlmRequest;
use serde_json::{Value, json};

struct Scripted;

const DSL: &str = r#"fig seg2
panel a "a. Coarse localisation"
  x: image "@img"
  det: encoder "Localiser"
  roi: box "ROI crop"
  x -> det -> roi
end
panel b "b. Fine segmentation"
  crop: image "$\mathbf{x}_{r}$"
  seg: unet "Segmenter"
  yhat: image "@pred" look=mask
  loss: loss "@lseg"
  y: image "@gt" role=gt look=mask
  crop -> seg -> yhat -> loss
  y -> loss : supervision
  loss -> seg : feedback
end
roi -> crop : cross
"#;

#[async_trait::async_trait]
impl lore_flow::llm::Llm for Scripted {
    async fn complete_json(&self, task: &str, req: LlmRequest) -> Result<(Value, String), String> {
        let text = format!("{:?}", req.messages);
        let step = text.split("Step now: ").nth(1).and_then(|s| s.split('.').next()).unwrap_or("");
        if task == "flow-discuss" {
            let v = match step {
                "start" => json!({ "say": "這是一個兩階段的分割流程：先找出大概位置，再在裁切區域內做精細分割。", "options": [
                    { "label": "整條流程", "detail": "從輸入影像到最後的遮罩，兩個階段都畫" },
                    { "label": "只畫第二階段", "detail": "第一階段只當一個輸入框" } ], "pick": 0, "question": "" }),
                "outline" => json!({ "say": "方法可以分成定位、裁切、分割三步，訓練時只在分割端有損失。", "options": [
                    { "label": "三步", "detail": "localise → crop ROI → segment → loss" },
                    { "label": "兩步", "detail": "localise+crop → segment" } ], "pick": 0, "question": "" }),
                "blocks" => json!({ "say": "兩個 block 最清楚：一個粗定位、一個精細分割。", "options": [
                    { "label": "2 個 block", "detail": "block 1: localise, crop; block 2: segment, loss" },
                    { "label": "3 個 block", "detail": "block 1: localise; block 2: crop; block 3: segment, loss" } ], "pick": 0, "question": "" }),
                "focus" => json!({ "say": "兩階段的關係是重點，所以畫流程。", "options": [
                    { "label": "流程", "detail": "資料怎麼從一個階段流到下一個" }, { "label": "模型", "detail": "每個網路的結構" } ], "pick": 0, "question": "" }),
                "detail" => json!({ "say": "只到模組名稱，圖才放得進一張投影片。", "options": [
                    { "label": "模組名稱", "detail": "Localiser、Segmenter 各一個方塊" }, { "label": "到層", "detail": "U-Net 畫出編碼、解碼層" } ], "pick": 0, "question": "" }),
                "vars" => json!({ "say": "建議放輸入、預測、標註和分割損失，都在對照表裡。", "options": [
                    { "label": "x", "id": "img", "latex": "\\mathbf{x}", "meaning": "input image", "registered": true },
                    { "label": "y hat", "id": "pred", "latex": "\\hat{\\mathbf{y}}", "meaning": "predicted mask", "registered": true },
                    { "label": "y", "id": "gt", "latex": "\\mathbf{y}", "meaning": "ground-truth mask", "registered": true },
                    { "label": "L", "id": "lseg", "latex": "\\mathcal{L}_{seg}", "meaning": "segmentation loss", "registered": true },
                    { "label": "x_r", "id": "xr", "latex": "\\mathbf{x}_{r}", "meaning": "cropped region", "registered": false } ], "pick": 0, "question": "" }),
                "io" => json!({ "say": "每個 block 的輸入輸出如下，可以直接改。", "options": [], "pick": -1, "question": "", "rows": [
                    { "block": "block 1", "input": "$\\mathbf{x}$", "output": "ROI crop", "what": "coarse localiser" },
                    { "block": "block 2", "input": "$\\mathbf{x}_{r}$", "output": "$\\hat{\\mathbf{y}}$", "what": "fine segmenter (U-Net)" } ] }),
                _ => json!({ "say": "", "options": [], "pick": -1, "question": "" }),
            };
            return Ok((v, "scripted".into()));
        }
        if task.starts_with("sc-flow-graph") {
            let g = lore_flow::dsl::parse(DSL).map_err(|e| e.to_string())?;
            return Ok((serde_json::to_value(&g).unwrap(), "scripted".into()));
        }
        if task == "flow-assist" {
            if text.contains("Mode: alternatives") {
                return Ok((json!({ "note": "三種版面。", "alternatives": [
                    { "why": "左小右大：精細分割是重點。", "layout": "small-big" },
                    { "why": "上下疊：寬度不夠時。", "layout": "column" },
                    { "why": "第二個 block 直的。", "layout": "row", "dirs": { "b": "down" } } ] }), "scripted".into()));
            }
            return Ok((json!({ "note": "第二個 block 改成由上到下，圖例放在圖下方。", "question": "", "changes": [
                { "path": "/dirs/b", "value": "down", "why": "block 2 = b. Fine segmentation" },
                { "path": "/legend/show", "value": "on" },
                { "path": "/legend/pos", "value": "below", "why": "放在圖下方" } ] }), "scripted".into()));
        }
        Err(format!("no script for {task}"))
    }
}

fn notation() -> lore_notation::Notation {
    lore_notation::Notation::from_json(r#"{ "version": 1, "entries": [
        { "id": "img", "kind": "symbol", "latex": "\\mathbf{x}", "meaning": "input image" },
        { "id": "pred", "kind": "symbol", "latex": "\\hat{\\mathbf{y}}", "meaning": "predicted mask" },
        { "id": "gt", "kind": "symbol", "latex": "\\mathbf{y}", "meaning": "ground-truth mask" },
        { "id": "lseg", "kind": "symbol", "latex": "\\mathcal{L}_{seg}", "meaning": "segmentation loss" } ] }"#).unwrap()
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(7991);
    let mut app = axum::Router::new()
        .route("/theme.css", get(|| async { ([("content-type", "text/css; charset=utf-8")], include_str!("../../../ui/theme.css")) }))
        .route("/theme.js", get(|| async { ([("content-type", "text/javascript; charset=utf-8")], format!("window.LORE_THEME = null;\n{}", include_str!("../../../ui/theme.js"))) }));
    for (path, mime, body) in lore_shell::FILES {
        app = app.route(path, get(move || async move { ([("content-type", *mime)], *body) }));
    }
    let flow = lore_flow::web::Flow::bare().with_llm(Arc::new(Scripted)).with_notation(Arc::new(|_: &str| Some(notation())));
    let app = app.merge(lore_flow::web::router(flow));
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    println!("http://127.0.0.1:{port}/flow/");
    axum::serve(l, app).await.unwrap();
}
