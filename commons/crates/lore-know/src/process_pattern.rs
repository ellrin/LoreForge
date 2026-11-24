//! I2: the sentence-pattern processor of the inbox (docs/brain-hub.md §16).
//!
//! One sentence the author read aloud becomes a pattern: the study-specific content cut out as
//! `{placeholders}`, placed on the writing tree by what the sentence is doing, with its tone.
//! This module owns no model call: [`PatternProcessor::request`] writes the request,
//! [`PatternProcessor::read`] checks the reply in code (it never trusts the model: the template
//! must be the sentence with holes and nothing rewritten, places must exist on the tree).
//! [`examples_for`] picks which existing patterns to show the model.

use serde_json::{json, Value};

use crate::inbox::{Context, Entry, Made, Processor, Request};
use crate::types::{facet, format, Item, Source};

/// Where a sentence can sit within its paragraph.
const POSITIONS: [&str; 4] = ["opening", "body", "closing", "transition"];
/// The tones a pattern may carry.
const TONES: [&str; 6] = ["保留說法", "量化陳述", "對比", "定義", "引用", "承接"];
/// More holes than this is a pattern that fits one paper only.
/// A sentence that names several symbols needs a hole for each; beyond this the pattern is the
/// original sentence with its words taken out, not something to reuse.
const MAX_HOLES: usize = 8;
/// At most this many places per pattern.
const MAX_PLACES: usize = 3;
/// How many existing patterns the request shows.
const MAX_EXAMPLES: usize = 30;
/// Skeletons at least this alike (normalised edit similarity) count as the same pattern.
const SIMILAR_AT: f64 = 0.82;

const SYSTEM: &str = r#"You turn ONE English sentence from a research paper into a reusable sentence pattern for the author's writing library, and file it on his writing tree.

The tree has three levels: section, subsection, occasion. A place is the path "section/subsection/occasion" and must be copied exactly from the tree you are given. Say what the sentence is doing in the paper (its move), not what the study is about.

Make the "template" from the sentence:
- Replace only what belongs to that one study (the data, the method or model names, the task, the disease, the numbers, the names) with short snake_case {placeholders}, for example {method}, {dataset}, {value}. Aim for 2 to 4 placeholders and never use more than 8.
- Keep the skeleton words verbatim: the verbs, connectives and framing words that make the move. Never rewrite, reorder, shorten or improve the sentence. Every stretch of text outside the placeholders must be copied from the sentence as it is.
- LaTeX and citations become placeholders such as {reference} or {expression}; numbers become {value}.
- A good pattern fits many papers. A sentence that is already general may have no placeholder at all.

Reply with JSON only:
- Give every field below, always.
- "usable": true, or false when the words are not a usable sentence (speech-recognition noise, a fragment). "why": one short reason when false, "" when true. When false, the other fields are "" and [].
- "template": the sentence with its holes.
- "section": the first level of the main place.
- "function": one short snake_case word for the move (for example gap, aim, contrast). Reuse a function seen in the existing patterns when one fits.
- "position": one of opening, body, closing, transition (where the sentence sits in its paragraph).
- "places": 1 to 3 paths that exist in the tree, the main one first.
- "tone": exactly one of 保留說法, 量化陳述, 對比, 定義, 引用, 承接, or "" when none fits.
- "similar": ids of the given existing patterns that make the same move with nearly the same skeleton; often none, never invent an id."#;

/// The sentence-pattern processor (format `pattern`).
pub struct PatternProcessor;

impl Processor for PatternProcessor {
    fn format(&self) -> &'static str {
        format::PATTERN
    }

    fn request(&self, entry: &Entry, cx: &Context) -> Request {
        let mut user = String::from("Tree (copy places exactly):\n");
        for line in tree_lines(&cx.tree) {
            user.push_str(&line);
            user.push('\n');
        }
        user.push_str("\nExisting patterns (id | main place | template):\n");
        let mut shown = 0;
        for it in cx.existing.iter().filter(|i| !i.field("template").is_empty()).take(MAX_EXAMPLES) {
            let main = it.tags.get(facet::PLACE).and_then(|v| v.first()).map(String::as_str).unwrap_or("-");
            user.push_str(&format!("{} | {} | {}\n", it.id, main, it.field("template")));
            shown += 1;
        }
        if shown == 0 {
            user.push_str("(none yet)\n");
        }
        user.push_str(&format!("\nSentence:\n{}\n", entry.text.trim()));
        Request { system: SYSTEM.to_string(), user, schema: schema() }
    }

    fn read(&self, entry: &Entry, cx: &Context, reply: &Value) -> Result<Made, String> {
        if reply.get("usable").and_then(Value::as_bool) == Some(false) {
            let why = reply.get("why").and_then(Value::as_str).unwrap_or("").trim();
            return Err(format!("這一段不像一個完整的句子：{}", if why.is_empty() { "原因不明" } else { why }));
        }
        let raw = reply.get("template").and_then(Value::as_str).unwrap_or("");
        let template = repair_template(raw)?;
        let holes = placeholders(&template);
        if holes.len() > MAX_HOLES {
            return Err(format!("句型有 {} 個洞，太專屬於原文；請換一個比較不具體的句型", holes.len()));
        }
        let sentence = entry.text.trim();
        if !is_faithful(&template, sentence) {
            return Err("句型和原句對不上".into());
        }
        if holes.is_empty() && has_specific_content(sentence) {
            return Err("句型沒有挖洞，但原句有數字、名稱或引用，應該挖掉".into());
        }

        let valid = leaf_paths(&cx.tree);
        let given = strings(reply.get("places"));
        let mut places: Vec<String> = vec![];
        for p in &given {
            if let Some(p) = find_place(p, &valid)
                && !places.contains(&p)
                && places.len() < MAX_PLACES
            {
                places.push(p);
            }
        }
        if places.is_empty() {
            // say what was given, so a wrong tree or a model that writes places its own way shows
            let shown = given.iter().take(3).cloned().collect::<Vec<_>>().join("、");
            return Err(if valid.is_empty() { "這一類沒有樹，沒有位置可以放".to_string() } else if shown.is_empty() { "沒有給位置".to_string() } else { format!("樹上沒有這個位置：{shown}") });
        }
        let section = places[0].split('/').next().unwrap_or("").to_string();

        let pos = reply.get("position").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        let position = if POSITIONS.contains(&pos.as_str()) { pos } else { "body".into() };
        let function = snake(reply.get("function").and_then(Value::as_str).unwrap_or(""), 30);
        let function = if function.is_empty() { "general".into() } else { function };

        let title: String = template.chars().take(80).collect();
        let mut item = Item::new(&entry.category, format::PATTERN, &title, Source::author());
        item.set("section", section);
        item.set("function", function);
        item.set("position", position);
        item.set("template", template.clone());
        item.set("example", entry.text.clone());
        item.set("priority", "3");
        for p in &places {
            item.tag(facet::PLACE, p);
        }
        let mut tones: Vec<String> = vec![];
        for t in strings(reply.get("tone")) {
            let t = t.trim().to_string();
            if TONES.contains(&t.as_str()) && !tones.contains(&t) {
                tones.push(t);
            }
        }
        for t in &tones {
            item.tag("tone", t);
        }

        let mut similar: Vec<String> = vec![];
        for id in strings(reply.get("similar")) {
            // the model may give the short id it was shown (`P167`) or the whole one
            let id = id.trim().to_string();
            let full = cx.existing.iter().find(|e| e.id == id || e.id.rsplit(':').next() == Some(id.as_str())).map(|e| e.id.clone());
            if let Some(full) = full
                && !similar.contains(&full)
            {
                similar.push(full);
            }
        }
        }
        let mine = skeleton(&template);
        for e in &cx.existing {
            let t = e.field("template");
            if !t.is_empty() && !similar.contains(&e.id) && similarity(&mine, &skeleton(t)) >= SIMILAR_AT {
                similar.push(e.id.clone());
            }
        }
        Ok(Made { item, similar })
    }
}

/// Which existing patterns to show the model: those sharing the most content words with the
/// sentence (lower-case words of four letters or more, minus a small stop list); ties by `used`
/// (more first), then id. At most `n`; deterministic.
pub fn examples_for(sentence: &str, all: &[Item], n: usize) -> Vec<Item> {
    let mine = content_words(sentence);
    let mut scored: Vec<(usize, &Item)> = all
        .iter()
        .map(|it| {
            let theirs = content_words(&format!("{} {}", strip_holes(it.field("template")), it.field("example")));
            (mine.iter().filter(|w| theirs.contains(*w)).count(), it)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.used.cmp(&a.1.used)).then(a.1.id.cmp(&b.1.id)));
    scored.into_iter().take(n).map(|(_, it)| it.clone()).collect()
}

// ── the request ──────────────────────────────────────────────────────────────

fn schema() -> Value {
    // Every field is required: a model answering under a schema leaves out whatever is optional
    // (seen with the local model: only the template came back). An unusable sentence fills them
    // with "" and [] and says why.
    let mut tones: Vec<&str> = TONES.to_vec();
    tones.push("");
    json!({
        "type": "object",
        "properties": {
            "usable": {"type": "boolean"},
            "why": {"type": "string"},
            "template": {"type": "string"},
            "places": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_PLACES},
            "section": {"type": "string"},
            "function": {"type": "string"},
            "position": {"type": "string", "enum": POSITIONS},
            "tone": {"type": "string", "enum": tones},
            "similar": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["usable", "why", "template", "places", "section", "function", "position", "tone", "similar"]
    })
}

fn label(v: &Value) -> &str {
    v.as_str().or_else(|| v["label"].as_str()).unwrap_or("")
}

fn children(v: &Value) -> &[Value] {
    v["children"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// The tree as an indented list; the third level shows the whole path (what a place is written as).
fn tree_lines(tree: &Value) -> Vec<String> {
    let mut out = vec![];
    for a in tree["tree"].as_array().into_iter().flatten() {
        out.push(label(a).to_string());
        for b in children(a) {
            out.push(format!("  {}", label(b)));
            for c in children(b) {
                out.push(format!("    {}/{}/{}", label(a), label(b), label(c)));
            }
        }
    }
    out
}

/// The tree's path a model meant by `given`: the path itself, or the same with another
/// separator (`›`, `>`, `／`, spaces around `/`), or, when the model left levels out, the one
/// path that ends with what it gave (the last level alone, or the last two). `None` when
/// nothing or more than one path fits.
fn find_place(given: &str, valid: &[String]) -> Option<String> {
    let norm = |s: &str| s.replace(['›', '>', '／', '→', '\\'], "/").split('/').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join("/");
    let g = norm(given);
    if g.is_empty() {
        return None;
    }
    if let Some(p) = valid.iter().find(|p| **p == g) {
        return Some(p.clone());
    }
    let fold = |s: &str| s.to_lowercase().replace(' ', "");
    let gf = fold(&g);
    if let Some(p) = valid.iter().find(|p| fold(p) == gf) {
        return Some(p.clone());
    }
    let tail: Vec<&String> = valid.iter().filter(|p| fold(p).ends_with(&format!("/{gf}"))).collect();
    if tail.len() == 1 {
        return Some(tail[0].clone());
    }
    // a slip of a character or two (a model writing 超引數 for 超參數): the one path that close
    let gc: Vec<char> = gf.chars().collect();
    let mut near: Vec<(usize, &String)> = valid.iter().map(|p| (levenshtein(&gc, &fold(p).chars().collect::<Vec<_>>()), p)).filter(|(d, _)| *d <= 2).collect();
    near.sort();
    if near.len() == 1 || (near.len() > 1 && near[0].0 < near[1].0) {
        return Some(near[0].1.clone());
    }
    None
}

/// Every three-level path of the tree.
fn leaf_paths(tree: &Value) -> Vec<String> {
    tree_lines(tree).into_iter().filter(|l| l.starts_with("    ")).map(|l| l.trim().to_string()).collect()
}

// ── reading the reply ────────────────────────────────────────────────────────

/// A string or an array of strings, as a list.
fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

/// snake_case: lower-case letters, digits and single underscores; starts with a letter.
fn snake(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.starts_with(|c: char| c.is_ascii_digit()) { format!("x_{out}") } else { out };
    out.chars().take(max).collect::<String>().trim_matches('_').to_string()
}

/// The template with every `{name}` rewritten to a safe name. Unbalanced braces cannot be read.
fn repair_template(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("模型沒有給句型".into());
    }
    let mut out = String::new();
    let mut inside: Option<String> = None;
    for c in raw.chars() {
        match (c, inside.as_mut()) {
            ('{', None) => inside = Some(String::new()),
            ('{', Some(_)) => return Err("句型的大括號不成對".into()),
            ('}', Some(name)) => {
                let n = snake(name, 30);
                out.push('{');
                out.push_str(if n.is_empty() { "part" } else { &n });
                out.push('}');
                inside = None;
            }
            ('}', None) => return Err("句型的大括號不成對".into()),
            (c, Some(name)) => name.push(c),
            (c, None) => out.push(c),
        }
    }
    if inside.is_some() {
        return Err("句型的大括號不成對".into());
    }
    Ok(out)
}

/// Split a (repaired) template into the text outside placeholders and the placeholder names.
fn split_template(t: &str) -> (Vec<String>, Vec<String>) {
    let mut texts = vec![String::new()];
    let mut names = vec![];
    let mut name: Option<String> = None;
    for c in t.chars() {
        match (c, name.as_mut()) {
            ('{', None) => name = Some(String::new()),
            ('}', Some(_)) => {
                names.push(name.take().unwrap_or_default());
                texts.push(String::new());
            }
            (c, Some(n)) => n.push(c),
            (c, None) => texts.last_mut().unwrap().push(c),
        }
    }
    (texts, names)
}

fn placeholders(t: &str) -> Vec<String> {
    split_template(t).1
}

/// Lower-case, whitespace collapsed, punctuation at both ends dropped.
fn norm(s: &str) -> String {
    let lower = s.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'").replace(['\u{201c}', '\u{201d}'], "\"");
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// Every stretch of three words or more outside the placeholders appears in the sentence.
fn is_faithful(template: &str, sentence: &str) -> bool {
    let s = norm(sentence);
    split_template(template).0.iter().all(|seg| {
        let n = norm(seg);
        n.split_whitespace().count() < 3 || s.contains(&n)
    })
}

/// Digits, a capitalised word inside the sentence (a name or an acronym), a bracketed citation or "et al".
fn has_specific_content(sentence: &str) -> bool {
    if sentence.chars().any(|c| c.is_ascii_digit()) || sentence.contains('[') || sentence.contains('\\') || sentence.contains("et al") {
        return true;
    }
    sentence.split_whitespace().skip(1).any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        w.chars().next().is_some_and(char::is_uppercase) && w != "I"
    })
}

// ── similarity ───────────────────────────────────────────────────────────────

/// The template with every placeholder as `{}`, lower-cased, whitespace collapsed.
fn skeleton(t: &str) -> String {
    let (texts, _) = split_template(t);
    texts.join("{}").to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 1 − edit distance / longer length; two empty skeletons are not alike.
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / longest as f64
}

// ── examples ─────────────────────────────────────────────────────────────────

const STOP: [&str; 24] = [
    "that", "this", "with", "from", "which", "their", "there", "these", "those", "have", "been", "were", "than", "then", "also", "such", "into", "will", "would", "about", "when", "while", "where", "using",
];

fn strip_holes(t: &str) -> String {
    split_template(t).0.join(" ")
}

fn content_words(s: &str) -> std::collections::BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| w.len() >= 4 && !STOP.contains(w))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_place_written_another_way_is_still_found() {
        let valid: Vec<String> = ["Method/訓練/超參數設定", "Method/訓練/訓練環境與硬體", "Method/資料/資料數量", "Result/主要結果/數字陳述", "Discussion/界線/限制", "Method/評估/限制"].iter().map(|s| s.to_string()).collect();
        let f = |g: &str| super::find_place(g, &valid);
        assert_eq!(f("Method/訓練/超參數設定").as_deref(), Some("Method/訓練/超參數設定"));
        assert_eq!(f("Method › 訓練 › 超參數設定").as_deref(), Some("Method/訓練/超參數設定"));
        assert_eq!(f("method / 訓練 / 訓練環境與硬體").as_deref(), Some("Method/訓練/訓練環境與硬體"));
        assert_eq!(f("訓練/超參數設定").as_deref(), Some("Method/訓練/超參數設定"), "the section left out");
        assert_eq!(f("資料數量").as_deref(), Some("Method/資料/資料數量"), "only the last level");
        assert_eq!(f("限制"), None, "two paths could be meant: neither is guessed");
        assert_eq!(f("Method/訓練/不存在"), None);
        assert_eq!(f("Method/訓練/超引數設定").as_deref(), Some("Method/訓練/超參數設定"), "one character off");
        assert_eq!(f(""), None);
    }

    use super::*;

    fn tree() -> Value {
        json!({"family": "pc/寫作準則", "levels": ["段落", "小節", "使用時機"], "tree": [
            {"label": "Introduction", "children": [
                {"label": "問題", "children": ["現有做法的不足", "為什麼重要"]},
                {"label": "本文", "children": ["目標", "貢獻"]}]},
            {"label": "Result", "children": [
                {"label": "主要結果", "children": ["數值比較"]}]}]})
    }

    fn pattern(id: &str, template: &str, place: &str, used: u64) -> Item {
        let mut i = Item::new("pc.patterns", format::PATTERN, template, Source::author());
        i.id = id.into();
        i.used = used;
        i.set("template", template);
        i.tag(facet::PLACE, place);
        i
    }

    fn cx(existing: Vec<Item>) -> Context {
        Context { tree: tree(), existing }
    }

    fn entry(text: &str) -> Entry {
        Entry { id: "i1".into(), category: "pc.patterns".into(), text: text.into(), ..Default::default() }
    }

    const S: &str = "However, existing methods for chest x-ray screening rely on large labelled datasets.";

    fn good() -> Value {
        json!({"usable": true, "template": "However, existing methods for {task} rely on large labelled datasets.",
            "section": "Result", "function": "Research Gap", "position": "opening",
            "places": ["Introduction/問題/現有做法的不足", "Introduction/本文/目標"],
            "tone": "對比", "similar": []})
    }

    #[test]
    fn the_request_carries_the_tree_the_examples_and_the_sentence() {
        let c = cx(vec![pattern("pc.patterns:a1", "We propose {method} for {task}.", "Introduction/本文/貢獻", 0)]);
        let r = PatternProcessor.request(&entry(S), &c);
        assert!(r.system.contains("snake_case"));
        assert!(r.user.contains("Introduction\n  問題\n    Introduction/問題/現有做法的不足"));
        assert!(r.user.contains("pc.patterns:a1 | Introduction/本文/貢獻 | We propose {method} for {task}."));
        assert!(r.user.ends_with(&format!("Sentence:\n{S}\n")));
        assert_eq!(r.schema["required"][0], "usable");
        assert_eq!(PatternProcessor.format(), "pattern");
    }

    #[test]
    fn the_request_is_stable() {
        let c = cx(vec![pattern("x:1", "A {b} c d.", "Result/主要結果/數值比較", 2)]);
        let e = entry(S);
        assert_eq!(PatternProcessor.request(&e, &c), PatternProcessor.request(&e, &c));
    }

    #[test]
    fn a_good_reply_becomes_a_placed_pattern() {
        let m = PatternProcessor.read(&entry(S), &cx(vec![]), &good()).unwrap();
        let i = &m.item;
        assert_eq!(i.format, "pattern");
        assert_eq!(i.category, "pc.patterns");
        assert_eq!(i.field("template"), "However, existing methods for {task} rely on large labelled datasets.");
        assert_eq!(i.field("section"), "Introduction");
        assert_eq!(i.field("function"), "research_gap");
        assert_eq!(i.field("position"), "opening");
        assert_eq!(i.field("example"), S);
        assert_eq!(i.field("priority"), "3");
        assert_eq!(i.tags[facet::PLACE], ["Introduction/問題/現有做法的不足", "Introduction/本文/目標"]);
        assert_eq!(i.tags["tone"], ["對比"]);
        assert!(i.title.starts_with("However, existing"));
        assert!(m.similar.is_empty());
    }

    #[test]
    fn a_rewritten_sentence_is_refused() {
        let mut r = good();
        r["template"] = json!("Prior approaches to {task} depend heavily on annotated data.");
        assert_eq!(PatternProcessor.read(&entry(S), &cx(vec![]), &r).unwrap_err(), "句型和原句對不上");
        // a plain copy of a general sentence is fine, one with names and numbers is not
        let general = "Little is known about the effect of image size.";
        let mut r = good();
        r["template"] = json!(general);
        assert!(PatternProcessor.read(&entry(general), &cx(vec![]), &r).is_ok());
        let specific = "The model reached 91.2 percent accuracy on the test set.";
        r["template"] = json!(specific);
        assert!(PatternProcessor.read(&entry(specific), &cx(vec![]), &r).is_err());
    }

    #[test]
    fn too_many_holes_are_refused() {
        let s = "We trained alpha on beta with gamma using delta and epsilon over zeta for eta in theta at iota.";
        let mut r = good();
        r["template"] = json!("We trained {a} on {b} with {c} using {d} and {e} over {f} for {g} in {h} at {i}.");
        let e = PatternProcessor.read(&entry(s), &cx(vec![]), &r).unwrap_err();
        assert!(e.contains('9') && e.contains("不具體"));
        // eight is the most: a sentence naming several symbols needs a hole for each
        r["template"] = json!("We trained {a} on {b} with {c} using {d} and {e} over {f} for {g} in {h} at iota.");
        assert!(PatternProcessor.read(&entry(s), &cx(vec![]), &r).is_ok());
    }

    #[test]
    fn unknown_places_are_dropped_and_none_left_is_refused() {
        let mut r = good();
        r["places"] = json!(["Nope/x/y", "Introduction/問題", "Result/主要結果/數值比較", "Result/主要結果/數值比較", "Introduction/本文/貢獻", "Introduction/本文/目標", "Introduction/問題/為什麼重要"]);
        let m = PatternProcessor.read(&entry(S), &cx(vec![]), &r).unwrap();
        assert_eq!(m.item.tags[facet::PLACE], ["Result/主要結果/數值比較", "Introduction/本文/貢獻", "Introduction/本文/目標"]);
        r["places"] = json!(["Nope/x/y", "Introduction"]);
        assert_eq!(PatternProcessor.read(&entry(S), &cx(vec![]), &r).unwrap_err(), "樹上沒有這個位置：Nope/x/y、Introduction", "it says what was given");
        r["places"] = json!(null);
        assert!(PatternProcessor.read(&entry(S), &cx(vec![]), &r).is_err());
    }

    #[test]
    fn the_section_follows_the_main_place() {
        let mut r = good();
        r["section"] = json!("Introduction");
        r["places"] = json!(["Result/主要結果/數值比較", "Introduction/本文/目標"]);
        r["position"] = json!("middle");
        r["function"] = json!("  ");
        let m = PatternProcessor.read(&entry(S), &cx(vec![]), &r).unwrap();
        assert_eq!(m.item.field("section"), "Result");
        assert_eq!(m.item.field("position"), "body");
        assert_eq!(m.item.field("function"), "general");
    }

    #[test]
    fn bad_placeholders_are_repaired_or_refused() {
        let mut r = good();
        r["template"] = json!("However, existing methods for {Chest X-ray Task} rely on large labelled datasets.");
        let m = PatternProcessor.read(&entry(S), &cx(vec![]), &r).unwrap();
        assert!(m.item.field("template").contains("{chest_x_ray_task}"));
        r["template"] = json!("However, existing methods for {} rely on large labelled datasets.");
        assert!(PatternProcessor.read(&entry(S), &cx(vec![]), &r).unwrap().item.field("template").contains("{part}"));
        for bad in ["However, existing methods for {task rely on large labelled datasets.", "However, existing methods for task} rely on large labelled datasets.", "However {a {b}} rely.", ""] {
            r["template"] = json!(bad);
            assert!(PatternProcessor.read(&entry(S), &cx(vec![]), &r).is_err(), "{bad}");
        }
    }

    #[test]
    fn noise_is_refused_with_its_reason() {
        let r = json!({"usable": false, "why": "只有幾個字"});
        assert_eq!(PatternProcessor.read(&entry("uh the the"), &cx(vec![]), &r).unwrap_err(), "這一段不像一個完整的句子：只有幾個字");
    }

    #[test]
    fn a_near_copy_of_an_existing_pattern_is_named() {
        let c = cx(vec![
            pattern("pc.patterns:a", "However,  existing methods for {x} rely on large labelled datasets", "Introduction/問題/現有做法的不足", 0),
            pattern("pc.patterns:b", "We propose {method} for {task}.", "Introduction/本文/貢獻", 0),
        ]);
        let m = PatternProcessor.read(&entry(S), &c, &good()).unwrap();
        assert_eq!(m.similar, ["pc.patterns:a"]);
    }

    #[test]
    fn tone_and_similar_keep_only_what_is_allowed() {
        let c = cx(vec![pattern("pc.patterns:b", "We propose {method} for {task}.", "Introduction/本文/貢獻", 0)]);
        let mut r = good();
        r["tone"] = json!("sarcastic");
        r["similar"] = json!(["pc.patterns:b", "pc.patterns:ghost", "pc.patterns:b"]);
        let m = PatternProcessor.read(&entry(S), &c, &r).unwrap();
        assert!(!m.item.tags.contains_key("tone"));
        assert_eq!(m.similar, ["pc.patterns:b"]);
        r["tone"] = json!(["量化陳述", "bogus"]);
        assert_eq!(PatternProcessor.read(&entry(S), &c, &r).unwrap().item.tags["tone"], ["量化陳述"]);
    }

    #[test]
    fn examples_are_the_patterns_closest_to_the_sentence() {
        let all = vec![
            pattern("c", "We propose {method} for {task}.", "Introduction/本文/貢獻", 9),
            pattern("b", "However, existing methods for {task} rely on {resource}.", "Introduction/問題/現有做法的不足", 1),
            pattern("a", "However, existing methods for {task} struggle with noisy labelled data.", "Introduction/問題/現有做法的不足", 1),
            pattern("d", "Results are shown in {reference}.", "Result/主要結果/數值比較", 5),
        ];
        let got: Vec<String> = examples_for(S, &all, 3).into_iter().map(|i| i.id).collect();
        assert_eq!(got, ["a", "b", "c"]);
        assert_eq!(examples_for(S, &all, 3), examples_for(S, &all, 3));
        assert!(examples_for(S, &all, 0).is_empty());
    }
}
