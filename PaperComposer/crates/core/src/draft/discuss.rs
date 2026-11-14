//! 深度討論: when the author does not get a question (or wants to talk it over), a separate
//! conversation about that one question, with the record, the question and the interview so
//! far as context. It never feeds the paper: nothing in it is graded or used as material by
//! any later step. It ends in one of three ways: 整理成這題的回答 (a summary of what the author
//! said and agreed to, edited by them, then sent as their answer through the interview),
//! closed without an answer, or 這題不用寫進論文 (the item is excluded).

use super::session::{Session, Turn, Who};
use crate::compose::{json_object, json_string, Prompt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The model task (Sonnet, cloud only: a fluent conversation).
pub const TASK: &str = "pc-draft-discuss";
/// The record given to the discussion (a long record is cut).
pub const RECORD_CHARS: usize = 20_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    Open,
    /// Its summary was sent as the answer.
    Answered,
    /// Closed without an answer (or the item was excluded).
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub who: Who,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Discussion {
    /// d1, d2, …
    pub id: String,
    pub group: String,
    /// The item the question is about (may be empty: a general question).
    #[serde(default)]
    pub item: String,
    /// The interview question it is about.
    pub question: String,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub status: Status,
    /// The answer the author confirmed (Answered).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
}

/// The open discussion of a group's current question, else a new one (pushed into the session).
pub fn open<'a>(s: &'a mut Session, group: &str, item: &str, question: &str) -> &'a mut Discussion {
    let at = s.discussions.iter().position(|d| d.status == Status::Open && d.group == group && d.question == question);
    let k = match at {
        Some(k) => k,
        None => {
            let n = s.discussions.iter().filter_map(|d| d.id.strip_prefix('d')?.parse::<usize>().ok()).max().unwrap_or(0) + 1;
            s.discussions.push(Discussion { id: format!("d{n}"), group: group.into(), item: item.into(), question: question.into(), ..Default::default() });
            s.discussions.len() - 1
        }
    };
    &mut s.discussions[k]
}

pub fn reply_schema() -> Value {
    json_object(&[("reply", json_string())])
}

pub fn answer_schema() -> Value {
    json_object(&[("answer", json_string())])
}

const SYSTEM: &str = "You talk with a researcher about one question of an interview on their own study (the \
interview turns the study into a paper). They did not get the question or want to think it through. Explain what \
the question asks and why the paper needs it, show what their experiment record already says about it, answer \
what they ask, and help them find their own answer. Do not decide for them; when you suggest something, say it is \
a suggestion. A few sentences per reply, plain words, plain text (no Markdown: no **bold**, no # headings). Write in Traditional Chinese as used in Taiwan (繁體中文), \
full-width punctuation. Reply with JSON only: {\"reply\": \"…\"}.";

/// Everything the discussion starts from: key value, the record, the question with its item and
/// group, the interview so far (answers only), then the talk itself.
pub fn prompt(key_value: &str, s: &Session, record: &str, turns: &[Turn], d: &Discussion) -> Prompt {
    let mut u = format!("Key value of the paper: {}\n", key_value.trim());
    let rec: String = record.trim().chars().take(RECORD_CHARS).collect();
    if !rec.is_empty() {
        u += &format!("\n## The experiment record\n{rec}\n");
    }
    u += &question_block(s, d);
    let asked: Vec<&Turn> = turns.iter().filter(|t| !t.text.trim().is_empty()).collect();
    if !asked.is_empty() {
        u += "\n## The interview so far\n";
        for t in asked {
            u += &format!("[{}] {}: {}\n", t.group, if t.who == Who::Ai { "Q" } else { "A" }, t.text);
        }
    }
    if !d.messages.is_empty() {
        u += "\n## This discussion\n";
        for m in &d.messages {
            u += &format!("{}: {}\n", if m.who == Who::Ai { "You" } else { "Author" }, m.text);
        }
    }
    Prompt { system: SYSTEM.into(), user: u }
}

fn question_block(s: &Session, d: &Discussion) -> String {
    let g = s.group(&d.group);
    let it = s.items.iter().find(|i| i.id == d.item);
    format!(
        "\n## The question\nGroup: [{}] {} — purpose: {}\nItem: {}\nQuestion: {}\n",
        g.map(|g| g.section.as_str()).unwrap_or(""),
        g.map(|g| g.title.as_str()).unwrap_or(""),
        g.map(|g| g.purpose.as_str()).unwrap_or(""),
        it.map(|i| i.label.as_str()).unwrap_or("(general)"),
        d.question
    )
}

/// 整理成這題的回答: the author's answer to the question from the discussion — only what they
/// said or agreed to, in their terms.
pub fn answer_prompt(key_value: &str, s: &Session, d: &Discussion) -> Prompt {
    let system = "Write the researcher's answer to the interview question from the discussion below, as they would \
give it: in their own terms, first person, short. Only what THEY said or clearly agreed to; leave out anything \
you suggested that they did not accept, and add nothing new. If they reached no answer, say so in one line. \
Traditional Chinese (繁體中文), full-width punctuation. Reply with JSON only: {\"answer\": \"…\"}."
        .to_string();
    let mut u = format!("Key value: {}\n", key_value.trim());
    u += &question_block(s, d);
    u += "\n## The discussion\n";
    for m in &d.messages {
        u += &format!("{}: {}\n", if m.who == Who::Ai { "Assistant" } else { "Researcher" }, m.text);
    }
    Prompt { system, user: u }
}

#[derive(Deserialize)]
struct Reply {
    reply: String,
}
#[derive(Deserialize)]
struct Answer {
    answer: String,
}

/// A model sometimes puts the whole JSON object inside the field (e2e: the answer box showed
/// `{"answer":"…"}`): unwrap it.
fn unwrap_field(v: String, key: &str) -> String {
    let t = v.trim();
    if t.starts_with('{') {
        if let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(t) {
            if let Some(inner) = o.get(key).and_then(|x| x.as_str()) {
                return unwrap_field(inner.to_string(), key);
            }
        }
    }
    t.to_string()
}

/// The panel shows plain text: Markdown emphasis a model adds anyway is dropped.
fn plain(t: String) -> String {
    t.replace("**", "").replace("__", "").lines().map(|l| l.trim_start_matches('#').trim_start_matches(' ').to_string()).collect::<Vec<_>>().join("\n")
}

pub fn parse_reply(r: &str) -> Result<String, String> {
    let x: Reply = super::prompts::json_in(r)?;
    Some(plain(unwrap_field(x.reply, "reply"))).filter(|x| !x.is_empty()).ok_or_else(|| "回覆是空的".into())
}

pub fn parse_answer(r: &str) -> Result<String, String> {
    let x: Answer = super::prompts::json_in(r)?;
    Ok(unwrap_field(x.answer, "answer"))
}

/// The confirmed answer as the interview receives it: for the discussed item.
pub fn as_answer(d: &Discussion, text: &str) -> String {
    if d.item.is_empty() {
        text.trim().to_string()
    } else {
        format!("{}\n(answer about {})", text.trim(), d.item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::lens::Lens;
    use crate::draft::prompts::*;
    use crate::draft::session::*;

    fn session() -> Session {
        let mut s = Session::default();
        s.set_plan(vec![(
            Group { section: "Methods".into(), title: "Labels".into(), purpose: "標籤怎麼選".into(), ..Default::default() },
            vec![Item { label: "label source".into(), role: "design".into(), ..Default::default() }, Item { label: "side note".into(), role: "design".into(), ..Default::default() }],
        )]);
        s
    }

    fn turn(n: usize, who: Who, text: &str, about: &str) -> Turn {
        Turn { n, group: "g1".into(), who, text: text.into(), options: vec![], engine: String::new(), about: about.into(), picked: vec![], cands: None }
    }

    #[test]
    fn the_discussion_starts_from_the_record_the_question_and_the_interview() {
        let mut s = session();
        let turns = vec![turn(1, Who::Ai, "資料怎麼切？", "i1"), turn(2, Who::Me, "依病人切 8:2", ""), turn(3, Who::Ai, "為什麼用這種標籤？", "i1")];
        let d = open(&mut s, "g1", "i1", "為什麼用這種標籤？").clone();
        assert_eq!(d.id, "d1");
        assert_eq!(open(&mut s, "g1", "i1", "為什麼用這種標籤？").id, "d1", "the same question reopens its discussion");
        let p = prompt("kv", &s, "## 資料\n- 12,345 images", &turns, &d);
        assert!(p.user.contains("12,345 images"), "the record");
        assert!(p.user.contains("Question: 為什麼用這種標籤？") && p.user.contains("Item: label source") && p.user.contains("purpose: 標籤怎麼選"));
        assert!(p.user.contains("A: 依病人切 8:2"), "the earlier answers");
        assert!(p.system.contains("Do not decide for them"));
    }

    #[test]
    fn a_discussion_never_reaches_the_paper_unless_confirmed() {
        let l = Lens::builtin();
        let mut s = session();
        let phrase = "ZEBRAQUOKKA";
        {
            let d = open(&mut s, "g1", "i1", "為什麼用這種標籤？");
            d.messages.push(Message { who: Who::Me, text: format!("{phrase} 這題是什麼意思？") });
            d.messages.push(Message { who: Who::Ai, text: format!("這題問的是… {phrase}") });
        }
        // 這題不用寫進論文
        s.items[1].excluded = true;
        s.claims.push(Claim { id: "c1".into(), zh: "主張".into(), en: "Claim.".into(), home: "Methods".into(), ..Default::default() });
        let turns = vec![turn(1, Who::Ai, "為什麼用這種標籤？", "i1")];
        let para = OutlinePara { id: "p1".into(), kind: "k".into(), plan: "p".into(), zh: "z".into(), sources: vec!["c1".into()], ..Default::default() };
        let outline = outline_prompt("kv", &s, &l, "", None, "");
        let claims = claims_prompt("kv", &s, &l, "");
        let turnp = turn_prompt("kv", &s, &l, "g1", &turns, Some("用報告抽的標籤"), "");
        let items: Vec<&Item> = s.items.iter().collect();
        let cands = crate::draft::candidates::prompt("kv", &s, &l, &items, "record", "");
        for (name, text) in [
            ("outline", outline.user),
            ("claims", claims.user),
            ("turn", turnp.user.clone()),
            ("compose", compose_extra(&para, &s, "Methods", "", &[], &[], &record_facts(&s, "record", 100))),
            ("audit", audit_prompt("kv", &s, &l, "", "").user),
            ("review", review_prompt("kv", &s, &l).user),
            ("candidates", cands.user),
        ] {
            assert!(!text.contains(phrase), "{name} prompt carries the discussion");
        }
        assert!(!turnp.user.contains("side note") && !outline_prompt("kv", &s, &l, "", None, "").user.contains("side note"), "an excluded item is never asked or outlined");
        assert!(s.group_state(&l, "g1").items[1].missing.is_empty());
        assert!(crate::draft::candidates::targets(&s, &l, &["g1".into()]).iter().all(|i| i.id != "i2"));
        // the confirmed summary is the author's answer, for the discussed item
        let d = s.discussions[0].clone();
        let a = as_answer(&d, &format!("我用報告抽的標籤，因為 {phrase}"));
        assert!(a.ends_with("(answer about i1)"));
        assert!(turn_prompt("kv", &s, &l, "g1", &turns, Some(&a), "").user.contains(phrase), "only what the author confirmed goes in");
        // the summary prompt keeps to the author's words
        let ap = answer_prompt("kv", &s, &d);
        assert!(ap.system.contains("Only what THEY said or clearly agreed to") && ap.user.contains("Researcher: ZEBRAQUOKKA"));
        // a rewind before the interview drops it
        s.rewind(Phase::Plan);
        assert!(s.discussions.is_empty());
    }

    #[test]
    fn replies_parse() {
        assert_eq!(parse_reply(r#"{"reply":" 這題問… "}"#).unwrap(), "這題問…");
        assert!(parse_reply(r#"{"reply":" "}"#).is_err());
        assert_eq!(parse_answer(r#"{"answer":"用報告抽的"}"#).unwrap(), "用報告抽的");
        assert_eq!(parse_answer(r#"{"answer":"{\"answer\":\"用報告抽的\"}"}"#).unwrap(), "用報告抽的", "a JSON object inside the field");
        assert_eq!(parse_reply(r#"{"reply":"{\"reply\":\"這題問…\"}"}"#).unwrap(), "這題問…");
        assert_eq!(parse_reply(r###"{"reply":"## 重點\n- **標籤品質**：E1"}"###).unwrap(), "重點\n- 標籤品質：E1", "no Markdown in the panel");
        assert_eq!(serde_json::to_string(&Status::Answered).unwrap(), "\"answered\"");
    }
}
