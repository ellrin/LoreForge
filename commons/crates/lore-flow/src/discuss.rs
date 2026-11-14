//! The discussion before a figure (`docs/flow-figures.md` §3): one short exchange
//! with the model per step (task `flow-discuss`, class `create`, JSON), the author
//! decides each step. The page keeps the decisions (`answers`, step id → value) and
//! sends them with every turn; the last step turns them into the L1 text through
//! the existing `sc-flow-graph` task ([`crate::llm::propose_via`]).
//!
//! Steps: `start` (the starting point restated) → `outline` → `blocks` → `focus`
//! (model or process) → `detail` → `vars` (symbols from the notation registry) →
//! `io` (each block's inputs, outputs, what it is) → `list` (the L1 text, shown and
//! edited directly). Layout, colours and the legend follow in the editor without
//! the model.

use lore_llm::{LlmRequest, Message};
use lore_notation::Notation;
use serde_json::{Value, json};

use crate::llm::{FlowPrompt, Llm, Proposal, propose_via};
use crate::modules::Library;
use crate::spec::Graph;

pub const TASK: &str = "flow-discuss";
pub const PROMPT: &str = include_str!("../prompts/discuss.md");

/// (id, title shown to the author, what the step decides — for the model).
pub const STEPS: &[(&str, &str, &str)] = &[
    ("start", "出發點", "Restate the method from the starting material (a paper's title and method, or a description) in 2–3 sentences, so the author can confirm what the figure is about. Options: 2–3 one-sentence framings of what the figure should show."),
    ("outline", "大綱", "Propose outlines of the method: the main stages in order, each option's detail written as `stage → stage → stage`."),
    ("blocks", "幾個 block", "Propose how many blocks (panels) the figure has and which stages go in each; detail as `block 1: …; block 2: …`."),
    ("focus", "重點", "Ask whether the figure is about the model (its architecture) or the process (data and training/inference flow). Options: model, process, both (say which dominates)."),
    ("detail", "細到哪裡", "Propose the level of detail for the modules: only module names; layers; layers with tensor sizes. Detail: what one block would show at that level."),
    ("vars", "變數與方程式", "Choose which variables and equations appear in the figure (inputs, outputs, predictions, losses). Each option is one symbol: `id` (the registry id, or a new short id), `latex` (the bare symbol, e.g. `\\hat{y}`: no `$`, no `\\in …` definition, which goes into `meaning`), `meaning`, `registered`. Recommend at most 8; put registered ones first."),
    ("io", "輸入與輸出", "For every block decided earlier, say its inputs, its outputs and what the block is (one short phrase). Use the chosen symbols by LaTeX. Fill `rows`, one per block in order; options may stay empty."),
];

/// Every step id, in order (the editor adds `list`, then layout, colours, legend).
pub fn step_ids() -> Vec<&'static str> {
    STEPS.iter().map(|s| s.0).collect()
}

fn notation_text(n: Option<&Notation>) -> String {
    match n.filter(|n| !n.entries.is_empty()) {
        Some(n) => n
            .entries
            .iter()
            .filter(|e| e.latex.is_some())
            .take(200)
            .map(|e| format!("- {} — ${}$ — {}", e.id, e.latex.as_deref().unwrap_or(""), e.meaning.as_deref().or(e.meaning_en.as_deref()).unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n"),
        None => "(empty: every variable is new)".into(),
    }
}

/// The reply schema of a step (closed: nothing else is accepted).
pub fn schema(step: &str) -> Value {
    let option = if step == "vars" {
        json!({ "type": "object", "additionalProperties": false, "required": ["label", "id", "latex", "meaning", "registered"],
            "properties": { "label": { "type": "string" }, "detail": { "type": "string" }, "id": { "type": "string" }, "latex": { "type": "string" }, "meaning": { "type": "string" }, "registered": { "type": "boolean" } } })
    } else {
        json!({ "type": "object", "additionalProperties": false, "required": ["label", "detail"],
            "properties": { "label": { "type": "string" }, "detail": { "type": "string" } } })
    };
    let mut props = json!({
        "say": { "type": "string" },
        "options": { "type": "array", "items": option },
        "pick": { "type": "integer" },
        "question": { "type": "string" }
    });
    if step == "io" {
        props["rows"] = json!({ "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["block", "input", "output", "what"],
            "properties": { "block": { "type": "string" }, "input": { "type": "string" }, "output": { "type": "string" }, "what": { "type": "string" } } } });
    }
    json!({ "type": "object", "additionalProperties": false, "required": ["say", "options", "pick", "question"], "properties": props })
}

/// The decisions so far, as the model reads them.
pub fn decisions_text(answers: &Value) -> String {
    let mut s = String::new();
    for (id, title, _) in STEPS {
        let Some(v) = answers.get(*id).filter(|v| !v.is_null()) else { continue };
        let text = match (id, v) {
            (_, Value::String(t)) => t.trim().to_string(),
            (&"vars", Value::Array(a)) => a
                .iter()
                .map(|x| format!("{} = ${}$ ({}){}", x["id"].as_str().unwrap_or(""), x["latex"].as_str().unwrap_or(""), x["meaning"].as_str().unwrap_or(""), if x["registered"].as_bool() == Some(false) { " [new]" } else { "" }))
                .collect::<Vec<_>>()
                .join("; "),
            (&"io", Value::Array(a)) => a
                .iter()
                .map(|x| format!("{}: in {} → out {} — {}", x["block"].as_str().unwrap_or(""), x["input"].as_str().unwrap_or(""), x["output"].as_str().unwrap_or(""), x["what"].as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n  "),
            (_, other) => other.to_string(),
        };
        if !text.is_empty() {
            s += &format!("- {id} ({title}): {text}\n");
        }
    }
    s
}

/// The request of one step.
pub fn request(step: &str, source: &str, answers: &Value, notation: Option<&Notation>, preferences: &str) -> Result<LlmRequest, String> {
    let goal = STEPS.iter().find(|s| s.0 == step).map(|s| s.2).ok_or_else(|| format!("沒有這一步：{step}"))?;
    let mut system = PROMPT.replace("{step_goal}", goal).replace("{notation}", &notation_text(notation));
    if !preferences.trim().is_empty() {
        system += &format!("\n\nThe author's own habits for figures (follow them):\n{}\n", preferences.trim());
    }
    let decided = decisions_text(answers);
    let user = format!(
        "Starting material:\n{}\n\nDecided so far:\n{}\nStep now: {step}. Reply as JSON.",
        if source.trim().is_empty() { "(none given)" } else { source.trim() },
        if decided.is_empty() { "(nothing yet)\n".to_string() } else { decided }
    );
    Ok(LlmRequest::chat(system, vec![Message::user(user)]).json(schema(step)).max_tokens(3000))
}

/// One step's turn. The reply is normalised: `pick` inside the options, `options` ≤ 6.
pub async fn turn(llm: &dyn Llm, step: &str, source: &str, answers: &Value, notation: Option<&Notation>, preferences: &str) -> Result<(Value, String), String> {
    let req = request(step, source, answers, notation, preferences)?;
    let (mut v, engine) = llm.complete_json(TASK, req).await?;
    if !v.is_object() {
        return Err("模型的回答不是 JSON 物件".into());
    }
    if let Some(a) = v["options"].as_array_mut() {
        a.truncate(6);
    } else {
        v["options"] = json!([]);
    }
    let n = v["options"].as_array().map(|a| a.len()).unwrap_or(0) as i64;
    let pick = v["pick"].as_i64().unwrap_or(-1);
    v["pick"] = json!(if pick >= 0 && pick < n { pick } else { -1 });
    for k in ["say", "question"] {
        if !v[k].is_string() {
            v[k] = json!("");
        }
    }
    Ok((v, engine))
}

/// The description the graph task gets: the material and every decision, with
/// the rule that each block is one panel holding a linear chain.
pub fn description(source: &str, answers: &Value) -> String {
    format!(
        "{}\n\nThe author decided, step by step:\n{}\nDraw every block as one panel, in the order decided, titled `a. …`, `b. …`. Inside a panel the main flow is one linear chain of nodes (input → … → output); side inputs (ground truth, a loss) join it. Use exactly the variables chosen (by notation id, @id); new ones go into vars_new.",
        source.trim(),
        decisions_text(answers)
    )
}

/// The last step: the decisions → the L1 graph (and its text form).
pub async fn graph(llm: &dyn Llm, source: &str, answers: &Value, notation: Option<&Notation>, library: &Library, examples: &[(String, Graph)]) -> Result<Proposal, String> {
    let desc = description(source, answers);
    let p = FlowPrompt { description: &desc, image: None, notation, library, examples };
    propose_via(llm, &p).await
}
