//! A sentence about a figure's placement → commands. The model is shown the figure in words
//! (its objects by name, kind, words and cell; its lines; its ties) and the closed set of
//! [`crate::command::Command`]s, and answers with a list of them, or says what it could not
//! tell from the sentence. It never writes a coordinate: [`crate::command::apply`] works out
//! every place. Commands that cannot be done go back to the model once with their reasons.
//!
//! FROZEN SEAM: the signatures below are fixed; the bodies are to be written.

use lore_llm::{LlmRequest, Message};
use serde_json::{Value, json};

use crate::command::{Clash, Command};
use crate::llm::Llm;
use crate::spec::Graph;

/// Task id in lore-llm's router (`tasks.toml`).
pub const TASK: &str = "flow-command";

/// What a sentence came to.
#[derive(Debug, Clone)]
pub struct Said {
    /// The figure after the commands that could be done.
    pub graph: Graph,
    /// The commands the model gave (after the one correction, if there was one).
    pub commands: Vec<Command>,
    /// What changed, in words, one line per change.
    pub done: Vec<String>,
    /// The commands that still could not be done, with their reasons.
    pub cannot: Vec<Clash>,
    /// What the model could not tell from the sentence (empty when it could), in the
    /// sentence's language.
    pub unclear: String,
    /// The engine that answered.
    pub provider: String,
    /// The commands were sent back once with the reasons they could not be done.
    pub retried: bool,
}

/// One command's object schema: `do` is its constant, `fields` are (name, schema), `required`
/// the names that must be there. [`schema_commands`] is the one place a new `Command` variant
/// is added to the schema.
fn entry(name: &str, fields: &[(&str, Value)], required: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    props.insert("do".into(), json!({ "const": name }));
    for (k, v) in fields {
        props.insert((*k).into(), v.clone());
    }
    let mut req = vec![json!("do")];
    req.extend(required.iter().map(|r| json!(r)));
    json!({ "type": "object", "additionalProperties": false, "required": req, "properties": props })
}

fn schema_commands() -> Vec<Value> {
    let name = || json!({ "type": "string" });
    let way = || json!({ "type": "string", "enum": ["left", "right", "above", "below"] });
    vec![
        entry("beside", &[("a", name()), ("of", name()), ("side", way())], &["a", "of", "side"]),
        entry("align", &[("a", name()), ("with", name()), ("axis", json!({ "type": "string", "enum": ["row", "column"] }))], &["a", "with", "axis"]),
        entry("over", &[("a", name()), ("from", name()), ("to", name())], &["a", "from", "to"]),
        entry("beside-rows", &[("a", name()), ("from", name()), ("to", name()), ("fill", json!({ "type": "boolean" }))], &["a", "from", "to"]),
        entry("tie", &[("mark", name()), ("to", name()), ("side", way())], &["mark", "to"]),
        entry("swap", &[("a", name()), ("b", name())], &["a", "b"]),
        entry("cell", &[("a", name()), ("col", json!({ "type": "number" })), ("row", json!({ "type": "number" }))], &["a", "col", "row"]),
        entry("stretch", &[("a", name()), ("way", json!({ "type": "string", "enum": ["wide", "tall", "both", "none"] }))], &["a", "way"]),
        entry("break", &[("a", name()), ("before", name())], &["a", "before"]),
        entry("one-line", &[("a", name())], &["a"]),
        entry("line-side", &[("from", name()), ("to", name()), ("leave", way()), ("arrive", way())], &["from", "to"]),
        entry("brace", &[("a", name()), ("from", name()), ("to", name()), ("toward", name())], &["a", "from", "to"]),
        entry("fork", &[("a", name()), ("to", json!({ "type": "array", "items": { "type": "string" }, "minItems": 1 }))], &["a", "to"]),
        entry("free", &[("a", name())], &["a"]),
    ]
}

/// The reply's JSON schema: `{commands: [Command…], unclear: string}`, every command one of the
/// closed set with exactly its own fields.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["commands", "unclear"],
        "properties": {
            "commands": { "type": "array", "items": { "anyOf": schema_commands() } },
            "unclear": { "type": "string" }
        }
    })
}

fn side_word(s: &str) -> &str {
    match s {
        "left" => "左",
        "right" => "右",
        "top" => "上",
        "bottom" => "下",
        o => o,
    }
}

/// The figure in words, as the model is shown it: per area its objects (name, kind, the words
/// on it, column and row, what it spans, what it is tied to), then the lines (from → to, kind).
pub fn figure_words(g: &Graph) -> String {
    let mut s = String::new();
    let mut areas: Vec<(String, String)> = g.panels.iter().map(|p| (p.id.clone(), p.title.clone())).collect();
    if g.nodes.iter().any(|n| !areas.iter().any(|a| a.0 == n.panel)) {
        areas.push((String::new(), String::new()));
    }
    for (id, title) in &areas {
        let name = match (id.is_empty(), title.is_empty()) {
            (true, _) => "（不在任何區塊）".to_string(),
            (false, true) => id.clone(),
            (false, false) => format!("{id}「{title}」"),
        };
        s += &format!("區塊 {name}\n");
        for n in g.nodes.iter().filter(|n| &n.panel == id) {
            let mut line = format!("  {}（{}）", n.id, n.module);
            let words = n.label.replace("\\\\", " / ");
            if !words.trim().is_empty() {
                line += &format!(" 「{}」", words.trim());
            }
            if let (Some(c), Some(l)) = (n.num("col"), n.num("lane")) {
                line += &format!(" 欄 {c} 列 {l}");
            }
            if let Some(k) = n.num("span").filter(|k| *k > 1.0) {
                line += &format!(" 跨 {k} 欄");
            }
            if let Some(k) = n.num("tall").filter(|k| *k > 1.0) {
                line += &format!(" 跨 {k} 列");
            }
            if let Some(b) = n.str("by") {
                match n.str("side") {
                    Some(sd) => line += &format!(" 貼著 {b}（{}）", side_word(sd)),
                    None => line += &format!(" 貼著 {b}"),
                }
            }
            if let Some(st) = n.str("stretch") {
                line += &format!(" 拉伸 {st}");
            }
            if let Some(f) = n.str("fill") {
                line += &format!(" 顏色 {f}");
            }
            if n.flag("vertical") {
                line += " 字轉向";
            }
            s += &line;
            s.push('\n');
        }
    }
    // the frames: a row or a part of the figure is often called by its frame's title
    if !g.regions.is_empty() {
        s += "外框（標題：裡面的物件）：\n";
        for r in &g.regions {
            let title = if r.title.trim().is_empty() { "（無標題）".to_string() } else { format!("「{}」", r.title.trim()) };
            s += &format!("  {} {}：{}\n", r.id, title, r.members.join("、"));
        }
    }
    s += "線：\n";
    for e in &g.edges {
        let mut line = format!("  {} → {}（{}）", e.from, e.to, e.kind.as_str());
        if !e.from_side.is_empty() {
            line += &format!(" 從 {} 的{}出", e.from, side_word(&e.from_side));
        }
        if !e.to_side.is_empty() {
            line += &format!(" 進 {} 的{}", e.to, side_word(&e.to_side));
        }
        s += &line;
        s.push('\n');
    }
    s
}

const SYSTEM: &str = "你把作者的一句話，變成流程圖版面的指令。作者說的是物件之間的關係，你從下列封閉的指令裡挑，程式會自己算出位置；你不寫任何座標。\n\
規則：\n\
- 只能用使用者給的清單裡的名字，一字不改；不可以自己發明物件。\n\
- 作者要求幾件事，就給幾個指令，一件事一個指令，不多給。\n\
- 句子沒有說清楚，或提到的東西不在清單裡，那一部分不要給指令，並把缺什麼寫在 unclear（用句子的語言）；全部都清楚時 unclear 留空字串。\n\
- 作者說「A 是 B 的」「A 屬於 B」，而 A 是損失（loss）、回傳箭頭（arrow）或註記時，用 tie，不用 beside：這種標記貼著 B，不佔格子。beside 只用在一般物件。\n\
- 作者用外框的標題稱呼一排或一部分時，取那個外框裡的物件。\n\
- 只回一個 JSON 物件 {commands, unclear}。\n\n\
指令（do 的值）：\n\
- beside {a, of, side}：a 放在 of 的某一邊（left/right/above/below）。\n\
- align {a, with, axis}：a 和 with 同一列（axis=row）或同一欄（axis=column）。\n\
- over {a, from, to}：標題或括弧 a 蓋在 from 到 to 的幾欄之上。\n\
- beside-rows {a, from, to, fill?}：a 在 from 到 to 幾列的旁邊、置中；fill=true 時和那幾列一樣高。\n\
- tie {mark, to, side?}：損失、回傳箭頭、註記 mark 屬於區塊 to，寫在它旁邊，不佔格子。\n\
- swap {a, b}：兩個物件對調位置。\n\
- cell {a, col, row}：a 放在指定的欄和列（照清單上寫的算法）。\n\
- stretch {a, way}：a 拉到所佔欄的寬度（wide）、列的高度（tall）、兩者（both），或恢復原大小（none）。\n\
- break {a, before}：a 的字在 before 這個字前面換行。\n\
- one-line {a}：a 的字排成一行。\n\
- line-side {from, to, leave?, arrive?}：from 到 to 的線從哪一邊出、從哪一邊進。\n\
- brace {a, from, to, toward?}：大括號或尺寸標記 a 括住 from 到 to 這幾個物件，跟它們一樣長，尖端朝向 toward。\n\
- fork {a, to}：標題或說明方框 a 講的是 to 裡的每個物件（to 是名字的清單），畫成從 a 分叉到每一個的線。\n\
- free {a}：收回之前對 a 說過的（綁定、跨欄、拉伸），讓圖自己決定。";

/// The request for one sentence.
pub fn request(g: &Graph, sentence: &str) -> LlmRequest {
    let user = format!("圖現在的樣子：\n{}\n作者的話：\n{}", figure_words(g), sentence.trim());
    LlmRequest::chat(SYSTEM.to_string(), vec![Message::user(user)]).json(schema()).max_tokens(1500)
}

/// The commands and the `unclear` words of a reply; `Err` when it is not of the schema.
pub fn read(reply: &Value) -> Result<(Vec<Command>, String), String> {
    let list = reply.get("commands").and_then(|c| c.as_array()).ok_or("the reply has no commands list")?;
    let mut out = Vec::new();
    for (i, c) in list.iter().enumerate() {
        out.push(serde_json::from_value::<Command>(c.clone()).map_err(|e| format!("command {i}: {e}"))?);
    }
    let unclear = reply.get("unclear").and_then(|u| u.as_str()).unwrap_or("").to_string();
    Ok((out, unclear))
}

/// One sentence through the model (task [`TASK`]) and the commands through
/// [`crate::command::apply_all`]. When some cannot be done, the model is asked once more with
/// the reasons (and the names nearest to an unknown one) and its second list is taken when it
/// leaves fewer undone.
pub async fn say(llm: &dyn Llm, g: &Graph, sentence: &str) -> Result<Said, String> {
    let req = request(g, sentence);
    let (v, provider) = llm.complete_json(TASK, req.clone()).await?;
    let (commands, unclear) = read(&v)?;
    let (graph, done, cannot) = crate::command::apply_all(g, &commands);
    let first = Said { graph, commands, done, cannot, unclear, provider, retried: false };
    if first.cannot.is_empty() {
        return Ok(first);
    }
    let reasons: Vec<String> = first.cannot.iter().map(|c| if c.who.is_empty() { format!("- {}", c.why) } else { format!("- {}（相關：{}）", c.why, c.who.join("、")) }).collect();
    let mut again = req;
    again.messages.push(Message::assistant(v.to_string()));
    again.messages.push(Message::user(format!("這些指令做不到：\n{}\n請改正，回完整的新清單（做得到的也要再列一次）。", reasons.join("\n"))));
    // a failed or unreadable second call leaves the first result, marked as retried
    let Ok((v2, provider2)) = llm.complete_json(TASK, again).await else { return Ok(Said { retried: true, ..first }) };
    let Ok((commands2, unclear2)) = read(&v2) else { return Ok(Said { retried: true, ..first }) };
    let (graph2, done2, cannot2) = crate::command::apply_all(g, &commands2);
    if cannot2.len() < first.cannot.len() {
        Ok(Said { graph: graph2, commands: commands2, done: done2, cannot: cannot2, unclear: unclear2, provider: provider2, retried: true })
    } else {
        Ok(Said { retried: true, ..first })
    }
}
