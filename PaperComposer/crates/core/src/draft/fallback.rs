//! Running the draft steps on a smaller model (the local engine) when Claude
//! is not available: prompts fitted to a short context, long material read in
//! pieces and merged, replies repaired to the schema before they are parsed.
//!
//! Nothing here calls a model; the server decides which engine runs and uses
//! these helpers when it is a small one.

use super::prompts::RecordReply;
use super::session::{Experiment, ResultTable, Thought, TimelineStep};
use crate::compose::Prompt;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Added to the system prompt for a small model.
pub const STRICT: &str = "\n\nReply with exactly one JSON object in the shape given, and nothing else: no prose, no code fence. \
Every field must be present; use \"\", [] or null when there is nothing to say. Never invent facts or numbers.";

/// Short engine label for the UI, from a model ref (`claude:opus` → `Opus`,
/// `local:current` → `本機`).
pub fn engine_label(model_ref: &str) -> String {
    let (kind, name) = model_ref.split_once(':').unwrap_or(("", model_ref));
    match kind {
        "local" => "本機".into(),
        "claude" | "claude-api" | "claude-code" | "claude-subscription" => {
            let n = name.to_lowercase();
            ["opus", "sonnet", "haiku"]
                .iter()
                .find(|m| n.contains(*m))
                .map(|m| format!("{}{}", m[..1].to_uppercase(), &m[1..]))
                .unwrap_or_else(|| "Claude".into())
        }
        _ => name.to_string(),
    }
}

/// A model ref that runs on the local engine (a small model).
pub fn is_local(model_ref: &str) -> bool {
    model_ref.trim().starts_with("local:")
}

/// Prompt characters a model with `context` tokens can take, leaving about
/// half the context for the reply (mixed English and Chinese runs at ~1.5
/// characters a token).
pub fn prompt_budget(context: u32) -> usize {
    (context as usize * 3 / 4).max(6_000)
}

/// `text` within `max` characters: head and tail kept, the middle replaced by
/// a marker. Unchanged when it fits.
pub fn cut(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    if max < 200 {
        return text.chars().take(max).collect();
    }
    let tail = max / 4;
    let head = max - tail - 40;
    let a: String = text.chars().take(head).collect();
    let b: String = text.chars().skip(n - tail).collect();
    format!("{a}\n[… {} characters left out …]\n{b}", n - head - tail)
}

/// A prompt for a small model: the strict-JSON instruction added, the user
/// text cut to what fits next to the system text.
pub fn fit(p: &Prompt, budget: usize) -> Prompt {
    let system = format!("{}{STRICT}", p.system);
    let room = budget.saturating_sub(system.chars().count()).max(2_000);
    Prompt { system, user: cut(&p.user, room) }
}

/// Split the record text (`=== file ===` blocks, see
/// `session::records_text`) into pieces of at most `max` characters: whole
/// files where they fit, else at blank lines, else at line ends. Every piece
/// starts with the name of the file it comes from. Nothing is dropped.
pub fn chunk_records(text: &str, max: usize) -> Vec<String> {
    let max = max.max(500);
    let mut files: Vec<(String, String)> = vec![];
    for block in text.split("=== ").filter(|b| !b.trim().is_empty()) {
        match block.split_once(" ===\n") {
            Some((name, body)) => files.push((name.to_string(), body.trim_end().to_string())),
            None => files.push((String::new(), block.trim_end().to_string())),
        }
    }
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.trim().is_empty() {
            out.push(std::mem::take(cur));
        }
        cur.clear();
    };
    for (name, body) in files {
        let head = if name.is_empty() { String::new() } else { format!("=== {name} ===\n") };
        let whole = format!("{head}{body}\n\n");
        if whole.chars().count() <= max {
            if cur.chars().count() + whole.chars().count() > max {
                flush(&mut cur, &mut out);
            }
            cur += &whole;
            continue;
        }
        flush(&mut cur, &mut out);
        let cont = if name.is_empty() { String::new() } else { format!("=== {name} (continued) ===\n") };
        let mut piece = head.clone();
        for para in split_keep(&body, "\n\n") {
            for unit in if para.chars().count() > max - cont.chars().count() { split_lines(&para, max - cont.chars().count()) } else { vec![para] } {
                if piece.chars().count() + unit.chars().count() > max && piece.chars().count() > cont.chars().count() {
                    out.push(std::mem::take(&mut piece));
                    piece = cont.clone();
                }
                piece += &unit;
            }
        }
        if piece.chars().count() > cont.chars().count() {
            out.push(piece);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// `s` split after each `sep` (the separator stays with the piece before it).
fn split_keep(s: &str, sep: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(i) = rest.find(sep) {
        out.push(rest[..i + sep.len()].to_string());
        rest = &rest[i + sep.len()..];
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// Lines (kept whole, a line longer than `max` cut into pieces of `max`).
fn split_lines(s: &str, max: usize) -> Vec<String> {
    let mut out = vec![];
    for line in split_keep(s, "\n") {
        let chars: Vec<char> = line.chars().collect();
        for c in chars.chunks(max.max(1)) {
            out.push(c.iter().collect());
        }
    }
    out
}

/// One record reply from replies on consecutive pieces of the record:
/// experiments and timeline steps renumbered across pieces (references to
/// them in thoughts and tables follow), duplicate experiments (same title)
/// and tables (same caption) merged, summaries joined.
pub fn merge_records(parts: Vec<RecordReply>) -> RecordReply {
    let mut out = RecordReply { summary: String::new(), experiments: vec![], timeline: vec![], thoughts: vec![], tables: vec![], conflicts: vec![] };
    let mut summaries: Vec<String> = vec![];
    for part in parts {
        let mut ids: HashMap<String, String> = HashMap::new();
        for e in part.experiments {
            let id = match out.experiments.iter().find(|x| x.title.trim().eq_ignore_ascii_case(e.title.trim())) {
                Some(x) => x.id.clone(),
                None => {
                    let id = format!("E{}", out.experiments.len() + 1);
                    out.experiments.push(Experiment { id: id.clone(), ..e.clone() });
                    id
                }
            };
            ids.insert(e.id.clone(), id);
        }
        let offset = out.timeline.len();
        for (i, t) in part.timeline.into_iter().enumerate() {
            ids.insert(format!("T{}", t.step), format!("T{}", offset + i + 1));
            out.timeline.push(TimelineStep { step: offset + i + 1, ..t });
        }
        let map = |v: Vec<String>| -> Vec<String> { v.into_iter().map(|x| ids.get(x.trim()).cloned().unwrap_or(x)).collect() };
        for h in part.thoughts {
            out.thoughts.push(Thought { id: format!("H{}", out.thoughts.len() + 1), text: h.text, about: map(h.about) });
        }
        for t in part.tables {
            if out.tables.iter().any(|x| x.caption.trim().eq_ignore_ascii_case(t.caption.trim())) {
                continue;
            }
            let from = map(t.from.clone());
            out.tables.push(ResultTable { id: format!("R{}", out.tables.len() + 1), from, ..t });
        }
        for c in part.conflicts {
            if !out.conflicts.contains(&c) {
                out.conflicts.push(c);
            }
        }
        if !part.summary.trim().is_empty() {
            summaries.push(part.summary.trim().to_string());
        }
    }
    out.summary = summaries.join("\n");
    out
}

/// Numbers in a table cell (`0.83`, `12`, `-1.5`; a `±` spread counts as
/// two numbers).
fn numbers(cell: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for ch in cell.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || (ch == '.' && !cur.is_empty()) {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(cur.trim_end_matches('.').to_string());
            cur.clear();
        }
    }
    out
}

/// Tables whose numbers are not all in the record get a note naming the
/// ones not found (the author checks them; nothing is removed). Returns how
/// many tables were flagged.
pub fn flag_unfound_numbers(tables: &mut [ResultTable], record: &str) -> usize {
    let mut flagged = 0;
    for t in tables.iter_mut() {
        let mut missing: Vec<String> = vec![];
        for n in t.rows.iter().flatten().flat_map(|c| numbers(c)) {
            if !record.contains(&n) && !missing.contains(&n) {
                missing.push(n);
            }
        }
        if !missing.is_empty() {
            flagged += 1;
            let shown: Vec<String> = missing.iter().take(6).cloned().collect();
            t.note = format!("{}（記錄裡找不到：{}{}，請核對）", t.note.trim(), shown.join("、"), if missing.len() > 6 { "…" } else { "" });
        }
    }
    flagged
}

// ── reply repair ─────────────────────────────────────────

/// A reply made to fit `schema` when it almost does: the JSON object found
/// in prose or a code fence, trailing commas removed, missing fields filled
/// with empty values, unknown fields dropped, scalars of the wrong kind
/// converted (a number where text is expected, one string where a list is).
/// `None` when there is no JSON object to work with (a cut-off reply).
pub fn repair(reply: &str, schema: &Value) -> Option<String> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    if end <= start {
        return None;
    }
    let raw = &reply[start..=end];
    let v: Value = serde_json::from_str(raw).or_else(|_| serde_json::from_str(&strip_trailing_commas(raw))).ok()?;
    serde_json::to_string(&conform(v, schema)).ok()
}

/// `,` before `}` or `]` (outside strings) removed.
fn strip_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let (mut in_str, mut esc) = (false, false);
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        }
        if c == ',' && chars[i + 1..].iter().find(|x| !x.is_whitespace()).is_some_and(|x| *x == '}' || *x == ']') {
            continue;
        }
        out.push(c);
    }
    out
}

fn nullable(schema: &Value) -> Option<&Value> {
    let any = schema["anyOf"].as_array()?;
    if any.iter().any(|s| s["type"] == "null") { any.iter().find(|s| s["type"] != "null") } else { None }
}

fn empty_of(schema: &Value) -> Value {
    if nullable(schema).is_some() {
        return Value::Null;
    }
    match schema["type"].as_str() {
        Some("object") => conform(Value::Object(Map::new()), schema),
        Some("array") => Value::Array(vec![]),
        Some("string") => Value::String(String::new()),
        Some("integer") | Some("number") => Value::from(0),
        Some("boolean") => Value::Bool(false),
        _ => Value::Null,
    }
}

fn conform(v: Value, schema: &Value) -> Value {
    if let Some(inner) = nullable(schema) {
        return if v.is_null() { v } else { conform(v, inner) };
    }
    match schema["type"].as_str() {
        Some("object") => {
            let Value::Object(mut m) = v else { return empty_of(schema) };
            let Some(props) = schema["properties"].as_object() else { return Value::Object(m) };
            let mut out = Map::new();
            for (k, sub) in props {
                let val = m.remove(k).map(|x| conform(x, sub)).unwrap_or_else(|| empty_of(sub));
                out.insert(k.clone(), val);
            }
            if schema["additionalProperties"] != Value::Bool(false) {
                out.extend(m);
            }
            Value::Object(out)
        }
        Some("array") => match v {
            Value::Array(a) => Value::Array(a.into_iter().map(|x| conform(x, &schema["items"])).collect()),
            Value::Null => Value::Array(vec![]),
            Value::String(s) if s.trim().is_empty() => Value::Array(vec![]),
            other => Value::Array(vec![conform(other, &schema["items"])]),
        },
        Some("string") => match v {
            Value::String(_) => v,
            Value::Null => Value::String(String::new()),
            Value::Array(a) => Value::String(a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join("; ")),
            other => Value::String(other.to_string()),
        },
        Some("integer") => match &v {
            Value::Number(n) if n.is_i64() || n.is_u64() => v,
            Value::Number(n) => Value::from(n.as_f64().unwrap_or(0.0).round() as i64),
            Value::String(s) => s.trim().parse::<i64>().map(Value::from).unwrap_or(Value::from(0)),
            _ => Value::from(0),
        },
        Some("boolean") => match &v {
            Value::Bool(_) => v,
            Value::String(s) => Value::Bool(s.trim().eq_ignore_ascii_case("true")),
            _ => Value::Bool(false),
        },
        _ => v,
    }
}

#[cfg(test)]
mod tests {
    use super::super::prompts::{self, record_schema};
    use super::*;

    #[test]
    fn labels_are_a_few_characters() {
        assert_eq!(engine_label("claude:opus"), "Opus");
        assert_eq!(engine_label("claude-code:sonnet"), "Sonnet");
        assert_eq!(engine_label("claude:claude-haiku-5-5"), "Haiku");
        assert_eq!(engine_label("claude:selected"), "Claude");
        assert_eq!(engine_label("local:current"), "本機");
        assert_eq!(engine_label("local:qwen3.5-9b-q8"), "本機");
        assert!(is_local("local:current") && !is_local("claude:opus"));
    }

    #[test]
    fn cut_keeps_head_and_tail() {
        let t = "a".repeat(5000) + &"z".repeat(5000);
        let c = cut(&t, 2000);
        assert!(c.chars().count() <= 2100);
        assert!(c.starts_with("aaa") && c.ends_with("zzz") && c.contains("left out"));
        assert_eq!(cut("short", 100), "short");
        let p = fit(&Prompt { system: "S".into(), user: t.clone() }, 6000);
        assert!(p.system.contains("Every field must be present"));
        assert!(p.user.chars().count() < 6000);
    }

    #[test]
    fn records_are_chunked_without_losing_a_line() {
        let mut text = String::new();
        for f in 0..3 {
            text += &format!("=== run{f}.md ===\n");
            for i in 0..60 {
                text += &format!("line {f}-{i}: Dice 0.{i:02}\n");
                if i % 7 == 0 {
                    text += "\n";
                }
            }
            text += "\n\n";
        }
        text += "=== tiny.txt ===\nshort\n\n";
        let parts = chunk_records(&text, 900);
        assert!(parts.len() > 3);
        for p in &parts {
            assert!(p.chars().count() <= 900, "{}", p.chars().count());
            assert!(p.starts_with("=== "), "every piece says where it comes from: {p}");
        }
        let all = parts.join("");
        for f in 0..3 {
            for i in 0..60 {
                assert!(all.contains(&format!("line {f}-{i}: Dice 0.{i:02}")), "line {f}-{i}");
            }
        }
        assert!(all.contains("short"));
        // A single line longer than the piece is split, not dropped.
        let long = format!("=== x ===\n{}\n", "y".repeat(2500));
        let p = chunk_records(&long, 1000);
        assert_eq!(p.iter().map(|x| x.matches('y').count()).sum::<usize>(), 2500);
    }

    #[test]
    fn record_parts_merge_with_ids_followed() {
        let lens = super::super::lens::Lens::builtin();
        let a = prompts::parse_record(
            r#"{"summary":"前半","experiments":[{"id":"E1","title":"Baseline","change":"","metrics":"Dice","result":"0.80","purpose":"","purpose_sure":false,"mode":null}],
            "timeline":[{"step":1,"what":"a","observed":"","changed":""}],"thoughts":[{"text":"想法一","about":["E1","T1"]}],
            "tables":[{"caption":"Main","columns":["m","Dice"],"rows":[["A","0.80"]],"note":"","from":["E1"]}],"conflicts":["x"]}"#,
            &lens,
        )
        .unwrap();
        let b = prompts::parse_record(
            r#"{"summary":"後半","experiments":[{"id":"E1","title":"Ablation","change":"","metrics":"","result":"0.75","purpose":"","purpose_sure":true,"mode":null},{"id":"E2","title":"baseline","change":"","metrics":"","result":"","purpose":"","purpose_sure":false,"mode":null}],
            "timeline":[{"step":1,"what":"b","observed":"","changed":""}],"thoughts":[{"text":"想法二","about":["E1","E2","T1"]}],
            "tables":[{"caption":"main","columns":["m"],"rows":[["B"]],"note":"","from":[]},{"caption":"Ablation","columns":["v","Dice"],"rows":[["-w","0.75"]],"note":"","from":["E1"]}],"conflicts":["x","y"]}"#,
            &lens,
        )
        .unwrap();
        let m = merge_records(vec![a, b]);
        assert_eq!(m.summary, "前半\n後半");
        assert_eq!(m.experiments.iter().map(|e| (e.id.as_str(), e.title.as_str())).collect::<Vec<_>>(), [("E1", "Baseline"), ("E2", "Ablation")]);
        assert_eq!(m.timeline.iter().map(|t| t.step).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(m.thoughts[1].id, "H2");
        assert_eq!(m.thoughts[1].about, ["E2", "E1", "T2"], "the second piece's E1 is the merged E2, its E2 the same title as E1");
        assert_eq!(m.tables.iter().map(|t| (t.id.as_str(), t.caption.as_str())).collect::<Vec<_>>(), [("R1", "Main"), ("R2", "Ablation")]);
        assert_eq!(m.tables[1].from, ["E2"]);
        assert_eq!(m.conflicts, ["x", "y"]);
    }

    #[test]
    fn unfound_numbers_are_flagged_not_dropped() {
        let mut t = vec![
            ResultTable { caption: "a".into(), columns: vec!["m".into(), "Dice".into()], rows: vec![vec!["A".into(), "0.83 ± 0.02".into()]], ..Default::default() },
            ResultTable { caption: "b".into(), columns: vec!["m".into(), "Dice".into()], rows: vec![vec!["B".into(), "0.91".into()]], note: "主表".into(), ..Default::default() },
        ];
        assert_eq!(flag_unfound_numbers(&mut t, "run 3: Dice 0.83 (sd 0.02)"), 1);
        assert_eq!(t[0].note, "");
        assert!(t[1].note.starts_with("主表") && t[1].note.contains("0.91"), "{}", t[1].note);
        assert_eq!(t[1].rows[0][1], "0.91");
    }

    #[test]
    fn repair_fills_the_shape_a_small_model_missed() {
        let reply = "好的，結果如下：\n```json\n{\"summary\": 3, \"experiments\": [{\"id\": \"E1\", \"title\": \"Base\", \"purpose_sure\": \"true\", \"extra\": 1,}],\n \"tables\": {\"caption\": \"T\", \"columns\": \"m\", \"rows\": [[\"a\"]]},}\n```";
        assert!(prompts::parse_record(reply, &super::super::lens::Lens::builtin()).is_err(), "as sent it does not parse");
        let fixed = repair(reply, &record_schema()).unwrap();
        let r = prompts::parse_record(&fixed, &super::super::lens::Lens::builtin()).unwrap();
        assert_eq!(r.summary, "3");
        assert_eq!(r.experiments[0].title, "Base");
        assert!(r.experiments[0].purpose_sure);
        assert_eq!(r.experiments[0].mode, None);
        assert_eq!(r.tables.len(), 1, "one object where a list is expected becomes a list");
        assert_eq!(r.tables[0].columns, ["m"]);
        assert!(r.timeline.is_empty() && r.conflicts.is_empty());
        let v: Value = serde_json::from_str(&fixed).unwrap();
        assert!(v["experiments"][0].get("extra").is_none());
        assert_eq!(strip_trailing_commas(r#"{"a":"x,}","b":[1,2,],}"#), r#"{"a":"x,}","b":[1,2]}"#);
        assert!(repair("{\"summary\": \"cut off", &record_schema()).is_none());
    }
}
