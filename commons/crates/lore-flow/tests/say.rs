//! A sentence → commands, with scripted replies (no engine is ever called).

use std::sync::Mutex;

use lore_flow::command::{Axis, Command, Fill, Way};
use lore_flow::llm::Llm;
use lore_flow::say::{self, TASK};
use lore_flow::*;
use lore_llm::{LlmRequest, Role};
use serde_json::{Value, json};

fn graph(src: &str) -> Graph {
    dsl::parse_with(src, &["bracket".to_string()]).unwrap_or_else(|e| panic!("{e:?}"))
}

const FIG: &str = "fig t\npanel a \"Training\" none title=none\n  x: image \"In\" look=photo col=0 lane=1\n  e: box \"Encoder\" col=1 lane=1\n  n: box \"Self attention net\" col=2 lane=0\n  d: box \"Decoder\" col=2 lane=1\n  o: box \"Out\" col=3 lane=1\n  l: loss \"L\" col=3 lane=0\n  x -> e -> d -> o\n  e -> n : side\nend\n";

struct Script {
    replies: Mutex<Vec<Value>>,
    seen: Mutex<Vec<LlmRequest>>,
}

impl Script {
    fn new(replies: Vec<Value>) -> Script {
        Script { replies: Mutex::new(replies), seen: Mutex::new(Vec::new()) }
    }
}

#[async_trait::async_trait]
impl Llm for Script {
    async fn complete_json(&self, task: &str, req: LlmRequest) -> Result<(Value, String), String> {
        assert_eq!(task, TASK);
        self.seen.lock().unwrap().push(req);
        let mut r = self.replies.lock().unwrap();
        if r.is_empty() { Err("no more replies".into()) } else { Ok((r.remove(0), "script".into())) }
    }
}

fn run<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(f)
}

#[test]
fn the_schema_names_every_command_and_only_its_fields() {
    let all = vec![
        Command::Beside { a: "a".into(), of: "b".into(), side: Way::Left },
        Command::Align { a: "a".into(), with: "b".into(), axis: Axis::Row },
        Command::Over { a: "a".into(), from: "b".into(), to: "c".into() },
        Command::BesideRows { a: "a".into(), from: "b".into(), to: "c".into(), fill: true },
        Command::Tie { mark: "m".into(), to: "b".into(), side: Some(Way::Below) },
        Command::Tie { mark: "m".into(), to: "b".into(), side: None },
        Command::Swap { a: "a".into(), b: "b".into() },
        Command::Cell { a: "a".into(), col: 1.5, row: 2.0 },
        Command::Stretch { a: "a".into(), way: Fill::Both },
        Command::Break { a: "a".into(), before: "w".into() },
        Command::OneLine { a: "a".into() },
        Command::LineSide { from: "a".into(), to: "b".into(), leave: Some(Way::Right), arrive: None },
        Command::Brace { a: "a".into(), from: "b".into(), to: "c".into(), toward: Some("d".into()) },
        Command::Fork { a: "a".into(), to: vec!["b".into(), "c".into()] },
        Command::Free { a: "a".into() },
    ];
    let schema = say::schema();
    assert_eq!(schema["required"], json!(["commands", "unclear"]));
    assert_eq!(schema["additionalProperties"], json!(false));
    let entries = schema["properties"]["commands"]["items"]["anyOf"].as_array().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for c in &all {
        let v = serde_json::to_value(c).unwrap();
        let name = v["do"].as_str().unwrap().to_string();
        let e = entries.iter().find(|e| e["properties"]["do"]["const"] == json!(name)).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(e["additionalProperties"], json!(false));
        let props = e["properties"].as_object().unwrap();
        for k in v.as_object().unwrap().keys() {
            assert!(props.contains_key(k), "{name}: {k} not in schema");
        }
        for r in e["required"].as_array().unwrap() {
            assert!(v.get(r.as_str().unwrap()).is_some() || name == "tie" || name == "line-side" || name == "beside-rows", "{name}: required {r} absent");
        }
        seen.insert(name);
    }
    assert_eq!(seen.len(), entries.len(), "an entry no command has");
}

#[test]
fn the_figure_in_words_lists_objects_cells_ties_and_lines() {
    let mut g = graph(FIG);
    let i = g.node_index("l").unwrap();
    g.nodes[i].params.insert("by".into(), json!("o"));
    g.nodes[i].params.insert("side".into(), json!("right"));
    let i = g.node_index("d").unwrap();
    g.nodes[i].params.insert("span".into(), json!(2));
    g.nodes[i].params.insert("stretch".into(), json!("wide"));
    let i = g.node_index("n").unwrap();
    g.nodes[i].label = "Self\\\\attention".into();
    let w = say::figure_words(&g);
    assert!(w.contains("區塊 a「Training」"), "{w}");
    assert!(w.contains("e（box）") && w.contains("「Encoder」") && w.contains("欄 1 列 1"), "{w}");
    assert!(w.contains("Self / attention"), "{w}");
    assert!(w.contains("跨 2 欄") && w.contains("拉伸 wide"), "{w}");
    assert!(w.contains("貼著 o（右）"), "{w}");
    assert!(w.contains("線：") && w.contains("x → e（data）") && w.contains("e → n（side）"), "{w}");
    assert!(!w.contains("px") && !w.contains('#'));
}

#[test]
fn the_request_shows_the_figure_the_commands_and_the_sentence() {
    let g = graph(FIG);
    let r = say::request(&g, "the loss belongs right of Out");
    let user = r.messages.last().unwrap().text();
    assert!(user.contains("Encoder") && user.contains("e（box）") && user.contains("the loss belongs right of Out"));
    for d in ["beside", "align", "over", "beside-rows", "tie", "swap", "cell", "stretch", "break", "one-line", "line-side", "free"] {
        assert!(r.system.contains(d), "{d}");
    }
    assert!(r.json_schema.is_some());
    assert_eq!(r.max_tokens, 1500);
}

#[test]
fn a_reply_is_read_and_a_wrong_one_says_which_command() {
    let ok = json!({ "commands": [
        { "do": "tie", "mark": "l", "to": "o", "side": "right" },
        { "do": "free", "a": "x" } ], "unclear": "哪一個？" });
    let (cs, u) = say::read(&ok).unwrap();
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0], Command::Tie { mark: "l".into(), to: "o".into(), side: Some(Way::Right) });
    assert_eq!(u, "哪一個？");
    let bad = json!({ "commands": [ { "do": "free", "a": "x" }, { "do": "teleport", "a": "x" } ], "unclear": "" });
    let e = say::read(&bad).unwrap_err();
    assert!(e.contains("command 1"), "{e}");
    assert!(say::read(&json!({ "unclear": "" })).is_err());
}

fn tie() -> Value {
    json!({ "do": "tie", "mark": "l", "to": "o", "side": "right" })
}

#[test]
fn a_sentence_becomes_commands_and_the_figure_changes() {
    let g = graph(FIG);
    let llm = Script::new(vec![json!({ "commands": [tie(), { "do": "break", "a": "n", "before": "attention" }], "unclear": "" })]);
    let s = run(say::say(&llm, &g, "loss right of Out, break Self attention net")).unwrap();
    assert!(!s.retried && s.cannot.is_empty());
    assert_eq!(s.commands.len(), 2);
    assert_eq!(s.done.len(), 2, "{:?}", s.done);
    assert_ne!(s.graph, g);
    assert_eq!(s.provider, "script");
    assert_eq!(llm.seen.lock().unwrap().len(), 1);
}

#[test]
fn what_cannot_be_done_goes_back_once_with_its_reason() {
    let g = graph(FIG);
    let llm = Script::new(vec![
        json!({ "commands": [{ "do": "tie", "mark": "zz", "to": "o" }], "unclear": "" }),
        json!({ "commands": [tie()], "unclear": "" }),
    ]);
    let s = run(say::say(&llm, &g, "the loss goes by Out")).unwrap();
    assert!(s.retried && s.cannot.is_empty());
    assert_eq!(s.commands.len(), 1);
    let seen = llm.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let last = seen[1].messages.last().unwrap();
    assert_eq!(last.role, Role::User);
    assert!(last.text().contains("沒有叫 zz 的物件"), "{}", last.text());
    assert_eq!(seen[1].messages.len(), 3);
}

#[test]
fn a_second_list_that_is_no_better_is_not_taken() {
    let g = graph(FIG);
    let first = json!({ "commands": [tie(), { "do": "tie", "mark": "zz", "to": "o" }], "unclear": "" });
    let second = json!({ "commands": [{ "do": "tie", "mark": "yy", "to": "o" }, { "do": "tie", "mark": "zz", "to": "o" }], "unclear": "" });
    let llm = Script::new(vec![first, second]);
    let s = run(say::say(&llm, &g, "x")).unwrap();
    assert!(s.retried);
    assert_eq!(s.cannot.len(), 1);
    assert_eq!(s.commands.len(), 2);
    assert_eq!(s.commands[0], Command::Tie { mark: "l".into(), to: "o".into(), side: Some(Way::Right) });
    assert_eq!(s.done.len(), 1);
}

#[test]
fn an_unclear_sentence_changes_nothing_and_says_so() {
    let g = graph(FIG);
    let llm = Script::new(vec![json!({ "commands": [], "unclear": "Which block do you mean?" })]);
    let s = run(say::say(&llm, &g, "make it nicer")).unwrap();
    assert_eq!(s.graph, g);
    assert!(s.commands.is_empty() && s.done.is_empty() && !s.retried);
    assert_eq!(s.unclear, "Which block do you mean?");
    let none = Script::new(vec![]);
    assert!(run(say::say(&none, &g, "x")).is_err());
}
