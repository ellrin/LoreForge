//! The `sc-flow-graph` task with canned replies (no engine is ever called).

use lore_flow::llm::{self, FlowPrompt};
use lore_flow::notation::VarStatus;
use lore_flow::*;
use lore_llm::{Image, MockProvider, Needs};
use serde_json::json;

fn registry() -> lore_notation::Notation {
    lore_notation::Notation::from_json(r#"{ "version": 1, "entries": [ { "id": "img", "kind": "symbol", "latex": "\\boldsymbol{x}", "meaning": "input image" } ] }"#).unwrap()
}

fn good() -> serde_json::Value {
    json!({
        "fig": "seg",
        "panels": [ { "id": "a", "title": "a. Training" } ],
        "nodes": [
            { "id": "x", "module": "image", "role": "input", "label": "", "var": "img", "panel": "a" },
            { "id": "net", "module": "unet", "role": "model", "label": "@theta", "panel": "a", "params": { "depth": 3 } },
            { "id": "pred", "module": "image", "role": "output", "label": "", "var": "yhat", "panel": "a", "params": { "look": "mask" } },
            { "id": "loss", "module": "loss", "label": "$\\mathcal L_{seg}$", "panel": "a" },
            { "id": "y", "module": "image", "role": "gt", "label": "$y$", "panel": "a" }
        ],
        "edges": [
            { "from": "x", "to": "net", "kind": "data" },
            { "from": "net", "to": "pred", "kind": "data" },
            { "from": "pred", "to": "loss", "kind": "data" },
            { "from": "y", "to": "loss", "kind": "supervision" },
            { "from": "loss", "to": "net", "kind": "feedback", "label": "update" }
        ],
        "vars_new": [
            { "id": "theta", "latex": "\\theta_{seg}", "meaning": "segmentation model" },
            { "id": "yhat", "latex": "\\hat y", "meaning": "predicted mask" },
            { "id": "x", "latex": "\\boldsymbol{x}", "meaning": "input image" }
        ]
    })
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().build().unwrap()
}

#[test]
fn the_task_is_defined_for_the_router() {
    let t = llm::task_table();
    t.check().unwrap();
    let spec = &t.task[llm::TASK];
    assert_eq!(spec.needs, Needs { json: true, vision: true, context: Some(32_000), ..Default::default() });
    // merges into the shared table without touching it
    let all = lore_llm::TaskTable::bundled().merge(t);
    assert!(all.task.contains_key(llm::TASK) && all.task.contains_key("sc-slide-content"));
}

#[test]
fn the_request_offers_closed_lists_and_the_notation_table() {
    let reg = registry();
    let mut lib = Library::default();
    lib.add_json(r#"{ "kind": "encoder", "description": "a block that narrows", "w": 60, "h": 40, "objects": [] }"#).unwrap();
    let example = (String::from("An image goes through a model."), dsl::parse("fig e\nx: image \"$x$\"\nf: box \"Model\"\nx -> f\n").unwrap());
    let img = Image::from_bytes("image/png", &[1, 2, 3]);
    let p = FlowPrompt { description: "x goes into a U-Net.", image: Some(img), notation: Some(&reg), library: &lib, examples: std::slice::from_ref(&example) };
    let req = llm::request(&p);
    assert!(req.has_images());
    let schema = req.json_schema.clone().unwrap();
    let modules = schema["properties"]["nodes"]["items"]["properties"]["module"]["enum"].as_array().unwrap();
    assert_eq!(modules.len(), lore_flow::spec::MODULES.len() + 1);
    assert!(modules.contains(&json!("encoder")) && modules.contains(&json!("unet")));
    let kinds = schema["properties"]["edges"]["items"]["properties"]["kind"]["enum"].as_array().unwrap();
    assert_eq!(kinds.len(), EdgeKind::ALL.len());
    // nothing about positions or colours can come back
    let node_props = schema["properties"]["nodes"]["items"]["properties"].as_object().unwrap();
    assert!(!node_props.keys().any(|k| ["x", "y", "color", "fill", "frame"].contains(&k.as_str())));
    assert_eq!(schema["properties"]["nodes"]["items"]["additionalProperties"], json!(false));
    assert!(req.system.contains("img — $\\boldsymbol{x}$ — input image"));
    assert!(req.system.contains("encoder: a block that narrows"));
    assert!(req.system.contains("Example 1") && req.system.contains("An image goes through a model."));
    // the canned good reply satisfies the schema we send
    lore_llm::json::validate(&good(), &schema).unwrap();
}

#[test]
fn a_reply_becomes_a_checked_graph() {
    let reg = registry();
    let lib = Library::default();
    let mock = MockProvider::new(vec![format!("```json\n{}\n```", good())]);
    let p = FlowPrompt { description: "…", image: None, notation: Some(&reg), library: &lib, examples: &[] };
    let prop = rt().block_on(llm::propose(&mock, &p)).unwrap();
    assert_eq!(prop.provider, "mock");
    assert!(prop.retried.is_none());
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
    // the variable the registry already has is not new: its references moved to the registry's id
    assert_eq!(prop.graph.vars_new.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["theta", "yhat"]);
    assert_eq!(prop.notation.vars[2].status, VarStatus::Existing { id: "img".into(), same_meaning: true });
    assert_eq!(prop.graph.node("x").unwrap().var, "img");
    assert!(prop.notation.findings.is_empty(), "{:?}", prop.notation.findings);
    // the author sees the text form, and it draws
    assert_eq!(dsl::parse(&prop.dsl).unwrap(), prop.graph);
    let f = draw(&prop.graph, &Drawing { notation: Some(&reg), ..Default::default() }).unwrap();
    assert!(f.routes.iter().all(|r| !r.forced));
    assert!(f.objects().iter().any(|o| o.name == "flow:seg/net"));
}

#[test]
fn a_graph_that_does_not_hold_together_is_sent_back_once() {
    let lib = Library::default();
    let mut bad = good();
    bad["edges"].as_array_mut().unwrap().push(json!({ "from": "pred", "to": "metric", "kind": "data" }));
    let mock = MockProvider::new(vec![bad.to_string(), good().to_string()]);
    let p = FlowPrompt { description: "…", image: None, notation: None, library: &lib, examples: &[] };
    let prop = rt().block_on(llm::propose(&mock, &p)).unwrap();
    assert!(prop.retried.as_deref().unwrap().contains("metric"));
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].messages.last().unwrap().text().contains("metric"));
    drop(seen);
    // twice wrong is an error, not a drawing
    let mock = MockProvider::new(vec![bad.to_string()]);
    let err = rt().block_on(llm::propose(&mock, &p)).unwrap_err();
    assert!(err.to_string().contains("metric"), "{err}");
}

#[test]
fn a_module_outside_the_list_is_rejected_by_the_schema() {
    let lib = Library::default();
    let mut odd = good();
    odd["nodes"][1]["module"] = json!("transformer-3000");
    let mock = MockProvider::new(vec![odd.to_string()]);
    let p = FlowPrompt { description: "…", image: None, notation: None, library: &lib, examples: &[] };
    assert!(rt().block_on(llm::propose(&mock, &p)).is_err());
    // (complete_json asked twice)
    assert_eq!(mock.seen.lock().unwrap().len(), 2);
}

#[test]
fn model_ids_are_tidied() {
    let lib = Library::default();
    let mut v = good();
    v["nodes"][3]["id"] = json!("L seg");
    v["edges"][2]["to"] = json!("L seg");
    v["edges"][3]["to"] = json!("L seg");
    v["edges"][4]["from"] = json!("L seg");
    v["arrange"] = json!([["a", "ghost"]]);
    v["nodes"][0]["var"] = json!("@x");
    let mock = MockProvider::new(vec![v.to_string()]);
    let p = FlowPrompt { description: "…", image: None, notation: None, library: &lib, examples: &[] };
    let prop = rt().block_on(llm::propose(&mock, &p)).unwrap();
    assert!(prop.graph.node("L_seg").is_some());
    assert!(prop.graph.arrange.is_empty());
    assert_eq!(prop.graph.node("x").unwrap().var, "x");
}
