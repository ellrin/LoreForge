//! The model task of a timeline (package T-llm): a description (optionally with a picture) →
//! the points of a [`TimelineSpec`] and one of the styles on offer. Signatures are frozen.
//! The model gives words only: no coordinates, no colours, no style of its own.

use lore_llm::{Image, LlmRequest, Message};
use serde_json::{Value, json};

use super::{Point, Style, TimelineSpec};
use crate::llm::Llm;

/// Task ids in lore-llm's router (`tasks.toml` of this crate).
pub const TASK: &str = "sc-flow-timeline";
pub const TASK_VISION: &str = "sc-flow-timeline-vision";
/// More points than this do not read on one slide.
pub const MAX_POINTS: usize = 10;

pub fn task_for(has_image: bool) -> &'static str {
    if has_image { TASK_VISION } else { TASK }
}

/// What the model is asked.
pub struct TimelinePrompt<'a> {
    /// The author's description, or the text the points are taken from.
    pub description: &'a str,
    /// A picture of a timeline to follow (its points and their order, never its colours).
    pub image: Option<Image>,
    /// The styles on offer (id, name, note): the model picks one id; only styles that draw as
    /// written ([`Style::cannot`] empty) are offered.
    pub styles: &'a [Style],
    /// The language of the texts (`en`, `zh-TW` …): the model keeps the description's own
    /// unless this says otherwise.
    pub language: &'a str,
}

/// Styles that draw as written: the only ones the model is offered.
fn offered(styles: &[Style]) -> Vec<&Style> {
    styles.iter().filter(|s| s.cannot.is_empty()).collect()
}

/// The JSON schema of the reply: `{ title, style, points: [{ time, title, body, icon }] }`,
/// every field required, at most [`MAX_POINTS`] points.
pub fn schema(styles: &[Style]) -> Value {
    let ids: Vec<&str> = offered(styles).iter().map(|s| s.id.as_str()).collect();
    let style = if ids.is_empty() { json!({ "type": "string" }) } else { json!({ "type": "string", "enum": ids }) };
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "style", "points"],
        "properties": {
            "title": { "type": "string" },
            "style": style,
            "points": {
                "type": "array", "minItems": 2, "maxItems": MAX_POINTS,
                "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["time", "title", "body", "icon"],
                    "properties": {
                        "time": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "icon": { "type": "string" }
                    }
                }
            }
        }
    })
}

fn system(p: &TimelinePrompt) -> String {
    let mut s = format!(
        "You extract a timeline from the author's text. A program draws it: you decide WHAT the points are, never how they look. \
Return JSON only: {{title, style, points}}.\n\n\
- points: 2 to {MAX_POINTS} points, in time order.\n\
- time: what is written as the time (a year, a month, a date, \"Phase 2\"), exactly as the source gives it.\n\
- title: at most 4 words.\n\
- body: at most 20 words, one or two plain sentences. Only facts the source states: never invent a date, a number or an event.\n\
- icon: one or two English words naming a simple picture for the point. It is a suggestion for the author and is not drawn.\n\
- title of the figure (the top-level title): at most 8 words.\n\
- style: pick one id from the list in the user text by its note (how many points, how much text).\n\
- Never give colours, positions or sizes.\n"
    );
    let lang = p.language.trim();
    if lang.is_empty() {
        s.push_str("- Write the texts in the language of the source.\n");
    } else {
        s.push_str(&format!("- Write the texts in this language: {lang}.\n"));
    }
    if p.image.is_some() {
        s.push_str("\nThe attached picture shows a timeline. Follow its points and their order; ignore its colours.\n");
    }
    s
}

/// The request for `p` (system and user text, the picture when there is one, the schema).
pub fn request(p: &TimelinePrompt) -> LlmRequest {
    let mut user = String::from("Styles on offer (id — name — note):\n");
    for st in offered(p.styles) {
        user.push_str(&format!("{} — {} — {}\n", st.id, st.name, st.note));
    }
    user.push_str(&format!("\nText:\n\n{}\n\nReturn the timeline as JSON.", p.description.trim()));
    let msg = match &p.image {
        Some(img) => Message::user_with_images(format!("{user}\n\nThe attached picture shows the timeline to follow."), vec![img.clone()]),
        None => Message::user(user),
    };
    LlmRequest::chat(system(p), vec![msg]).json(schema(p.styles)).max_tokens(4000)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The spec in a reply, checked in code and never trusted: at least two points, each with a
/// time; texts trimmed; a style that is not on offer becomes the first offered one; more than
/// [`MAX_POINTS`] points is an error. `Err` = one line the model can be sent back.
pub fn read(reply: &Value, styles: &[Style]) -> Result<TimelineSpec, String> {
    let arr = reply.get("points").and_then(Value::as_array).ok_or_else(|| format!("points: an array of 2 to {MAX_POINTS} points is needed"))?;
    if arr.len() < 2 || arr.len() > MAX_POINTS {
        return Err(format!("points: 2 to {MAX_POINTS} are needed, got {}", arr.len()));
    }
    let mut points = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        if !v.is_object() {
            return Err(format!("point {}: an object with time, title, body and icon is needed", i + 1));
        }
        let time = text(v, "time");
        if time.is_empty() {
            return Err(format!("point {}: time is missing", i + 1));
        }
        points.push(Point { time, title: text(v, "title"), body: text(v, "body"), icon: text(v, "icon"), ..Point::default() });
    }
    let on_offer = offered(styles);
    let asked = text(reply, "style");
    let style = if on_offer.iter().any(|s| s.id == asked) { asked } else { on_offer.first().map(|s| s.id.clone()).unwrap_or_default() };
    let title: String = text(reply, "title").chars().take(80).collect();
    Ok(TimelineSpec { title: title.trim_end().to_string(), style, points, ..TimelineSpec::default() })
}

/// Ask through an app's [`Llm`]; a reply that [`read`] refuses is sent back once with the
/// reason. Returns the spec and the engine that answered.
pub async fn propose_via(llm: &dyn Llm, p: &TimelinePrompt<'_>) -> Result<(TimelineSpec, String), String> {
    let task = task_for(p.image.is_some());
    let req = request(p);
    let (v, engine) = llm.complete_json(task, req.clone()).await?;
    match read(&v, p.styles) {
        Ok(spec) => Ok((spec, engine)),
        Err(why) => {
            let mut again = req;
            again.messages.push(Message::assistant(v.to_string()));
            again.messages.push(Message::user(format!("That timeline cannot be drawn: {why}. Return the whole corrected JSON object.")));
            let (v2, engine2) = llm.complete_json(task, again).await?;
            let spec = read(&v2, p.styles).map_err(|e| format!("模型給的時間軸畫不出來：{e}"))?;
            Ok((spec, engine2))
        }
    }
}

/// Task ids of the question asked before a draft: is this picture a timeline or a data flow?
pub const KIND_TASK: &str = "sc-flow-kind";
pub const KIND_TASK_VISION: &str = "sc-flow-kind-vision";

/// The request that asks which of the two a description (and its picture) shows.
pub fn kind_request(description: &str, image: Option<&Image>) -> LlmRequest {
    let system = "You are shown a figure the author wants redrawn (its description, and its picture when there is one). Say which of two kinds it is. \
\"timeline\": points placed in order along a time axis or a sequence of dated or numbered stages (years, months, phases, milestones), each with a short text; the order is the content. \
\"flow\": blocks joined by arrows that show how data or steps pass through a method (networks, modules, losses, inputs and outputs). \
A slide template whose items are years, months or numbered stages along a line, a band or a path is a timeline, whatever its title says. \
Judge by what the picture shows, not by single words in the description. Return JSON only: {kind, why}; why is one short sentence.";
    let user = format!("Description:\n{description}");
    let msg = match image {
        Some(img) => Message::user_with_images(format!("{user}\n\nThe attached picture is the figure."), vec![img.clone()]),
        None => Message::user(user),
    };
    let schema = json!({ "type": "object", "required": ["kind", "why"], "properties": { "kind": { "type": "string", "enum": ["timeline", "flow"] }, "why": { "type": "string" } } });
    LlmRequest::chat(system.to_string(), vec![msg]).json(schema).max_tokens(300)
}

/// Is it a timeline? The model looks and says; an answer that is neither kind is an error
/// (no rule on the words stands in for it). Returns the answer and its one line on why.
pub async fn is_timeline_via(llm: &dyn Llm, description: &str, image: Option<&Image>) -> Result<(bool, String), String> {
    let task = if image.is_some() { KIND_TASK_VISION } else { KIND_TASK };
    let (v, _) = llm.complete_json(task, kind_request(description, image)).await?;
    let why = v["why"].as_str().unwrap_or("").trim().to_string();
    match v["kind"].as_str() {
        Some("timeline") => Ok((true, why)),
        Some("flow") => Ok((false, why)),
        _ => Err("模型沒有說這張是時間軸還是流程圖。".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn style(id: &str, cannot: &[&str]) -> Style {
        Style { id: id.into(), name: format!("Name {id}"), note: format!("note of {id}"), cannot: cannot.iter().map(|s| s.to_string()).collect(), ..Style::default() }
    }
    fn styles() -> Vec<Style> {
        vec![style("a", &[]), style("broken", &["axis.kind = \"path\""]), style("b", &[])]
    }
    fn prompt<'a>(st: &'a [Style], image: Option<Image>, language: &'a str) -> TimelinePrompt<'a> {
        TimelinePrompt { description: "In 2021 a pilot ran at two sites. In 2023 it grew to ten.", image, styles: st, language }
    }
    fn good() -> Value {
        json!({ "title": "Pilot", "style": "b", "points": [
            { "time": "2021", "title": "Pilot", "body": "Two sites.", "icon": "flag" },
            { "time": "2023", "title": "Growth", "body": "Ten sites.", "icon": "chart" } ] })
    }
    fn img() -> Image {
        Image::from_bytes("image/png", &[1, 2, 3])
    }

    /// Scripted replies, in order; the engine is `engine<n>` for the n-th reply.
    struct Fake {
        replies: Mutex<Vec<(Value, String)>>,
        seen: Mutex<Vec<(String, LlmRequest)>>,
    }
    impl Fake {
        fn new(r: Vec<Value>) -> Fake {
            let v: Vec<(Value, String)> = r.into_iter().enumerate().map(|(i, v)| (v, format!("engine{i}"))).rev().collect();
            Fake { replies: Mutex::new(v), seen: Mutex::new(vec![]) }
        }
    }
    #[async_trait::async_trait]
    impl Llm for Fake {
        async fn complete_json(&self, task: &str, req: LlmRequest) -> Result<(Value, String), String> {
            if let Ok(mut s) = self.seen.lock() {
                s.push((task.to_string(), req));
            }
            self.replies.lock().map_err(|e| e.to_string())?.pop().ok_or_else(|| "no more replies".to_string())
        }
    }

    // ── step 1 ──
    #[test]
    fn the_schema_offers_only_styles_that_draw() {
        let sc = schema(&styles());
        assert_eq!(sc["properties"]["style"]["enum"], json!(["a", "b"]));
        assert_eq!(sc["properties"]["points"]["minItems"], 2);
        assert_eq!(sc["properties"]["points"]["maxItems"], MAX_POINTS);
        assert_eq!(sc["additionalProperties"], false);
        assert_eq!(sc["properties"]["points"]["items"]["additionalProperties"], false);
        let none = schema(&[style("x", &["bad"])]);
        assert!(none["properties"]["style"].get("enum").is_none());
        assert_eq!(none["properties"]["style"]["type"], "string");
    }

    #[test]
    fn the_request_lists_the_offered_styles_and_the_description() {
        let st = styles();
        let r = request(&prompt(&st, None, ""));
        let user = r.messages[0].text();
        assert!(user.contains("a — Name a — note of a"));
        assert!(user.contains("b — Name b — note of b"));
        assert!(!user.contains("broken"));
        assert!(user.contains("In 2021 a pilot ran at two sites"));
        assert!(r.json_schema.is_some());
        assert!(!r.has_images());
    }

    #[test]
    fn a_picture_is_attached_and_its_colours_are_not_to_be_followed() {
        let st = styles();
        let r = request(&prompt(&st, Some(img()), ""));
        assert!(r.has_images());
        assert!(r.system.contains("ignore its colours"));
        assert!(!request(&prompt(&st, None, "")).system.contains("attached picture"));
    }

    #[test]
    fn the_request_never_asks_for_colours_or_positions() {
        let st = styles();
        let sys = request(&prompt(&st, None, "")).system;
        assert!(sys.contains("Never give colours, positions or sizes"));
        let props = schema(&st)["properties"]["points"]["items"]["properties"].clone();
        assert!(props.get("color").is_none() && props.get("x").is_none());
    }

    #[test]
    fn a_language_is_stated_only_when_given() {
        let st = styles();
        let none = request(&prompt(&st, None, "  ")).system;
        assert!(none.contains("language of the source"));
        assert!(!none.contains("this language"));
        let some = request(&prompt(&st, None, "zh-TW")).system;
        assert!(some.contains("this language: zh-TW"));
        assert!(!some.contains("language of the source"));
    }

    // ── step 2 ──
    #[test]
    fn a_good_reply_becomes_a_spec() {
        let s = read(&good(), &styles()).expect("good reply");
        assert_eq!(s.title, "Pilot");
        assert_eq!(s.style, "b");
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.points[1].time, "2023");
        assert_eq!(s.points[0].icon, "flag");
        assert!(s.id.is_empty());
    }

    #[test]
    fn a_style_that_is_not_offered_becomes_the_first_offered() {
        let mut v = good();
        v["style"] = json!("broken");
        assert_eq!(read(&v, &styles()).expect("ok").style, "a");
        v["style"] = json!("nope");
        assert_eq!(read(&v, &styles()).expect("ok").style, "a");
        assert_eq!(read(&v, &[style("x", &["bad"])]).expect("ok").style, "");
    }

    #[test]
    fn too_few_or_too_many_points_is_one_line() {
        let mut v = good();
        v["points"] = json!([{ "time": "2021" }]);
        let e = read(&v, &styles()).expect_err("one point");
        assert_eq!(e, "points: 2 to 10 are needed, got 1");
        assert!(!e.contains('\n'));
        v["points"] = Value::Array((0..11).map(|i| json!({ "time": format!("{i}") })).collect());
        assert_eq!(read(&v, &styles()).expect_err("eleven"), "points: 2 to 10 are needed, got 11");
        assert!(read(&json!({ "title": "x" }), &styles()).is_err());
    }

    #[test]
    fn a_point_without_a_time_is_named() {
        let mut v = good();
        v["points"][1]["time"] = json!("  ");
        assert!(read(&v, &styles()).expect_err("blank").starts_with("point 2:"));
        v["points"][1] = json!({ "title": "x" });
        assert!(read(&v, &styles()).expect_err("missing").starts_with("point 2:"));
        v["points"][1] = json!("2023");
        assert!(read(&v, &styles()).expect_err("not an object").starts_with("point 2:"));
    }

    #[test]
    fn texts_are_trimmed_and_unknown_keys_ignored() {
        let long = "x".repeat(100);
        let v = json!({ "title": format!("  {long}  "), "style": " a ", "extra": 1, "points": [
            { "time": " 2021 ", "title": " T ", "body": " B\n", "icon": " i ", "weird": true },
            { "time": "2022" } ] });
        let s = read(&v, &styles()).expect("ok");
        assert_eq!(s.title.chars().count(), 80);
        assert_eq!(s.style, "a");
        let p = &s.points[0];
        assert_eq!((p.time.as_str(), p.title.as_str(), p.body.as_str(), p.icon.as_str()), ("2021", "T", "B", "i"));
        assert!(s.other.is_empty());
        let wide = json!({ "title": "é".repeat(90), "points": good()["points"] });
        assert_eq!(read(&wide, &styles()).expect("ok").title.chars().count(), 80);
    }

    #[test]
    fn a_reply_never_sets_colours() {
        let mut v = good();
        v["colors"] = json!(["FF0000"]);
        v["id"] = json!("mine");
        v["points"][0]["color"] = json!("00FF00");
        let s = read(&v, &styles()).expect("ok");
        assert!(s.colors.is_empty() && s.id.is_empty());
        assert!(s.points.iter().all(|p| p.color.is_empty()));
    }

    // ── step 3 ──
    #[tokio::test]
    async fn a_refused_reply_is_sent_back_once_with_the_reason() {
        let st = styles();
        let fake = Fake::new(vec![json!({ "points": [{ "time": "2021" }] }), good()]);
        let (spec, _) = propose_via(&fake, &prompt(&st, None, "")).await.expect("second reply is good");
        assert_eq!(spec.points.len(), 2);
        let seen = fake.seen.lock().expect("lock");
        assert_eq!(seen.len(), 2);
        let m = &seen[1].1.messages;
        assert_eq!(m.len(), 3);
        assert_eq!(m[2].text(), "That timeline cannot be drawn: points: 2 to 10 are needed, got 1. Return the whole corrected JSON object.");
    }

    #[tokio::test]
    async fn two_refusals_are_an_error() {
        let st = styles();
        let bad = json!({ "points": [] });
        let fake = Fake::new(vec![bad.clone(), bad, good()]);
        let e = propose_via(&fake, &prompt(&st, None, "")).await.expect_err("two refusals");
        assert!(e.starts_with("模型給的時間軸畫不出來：points: 2 to 10"));
        assert_eq!(fake.seen.lock().expect("lock").len(), 2);
    }

    #[tokio::test]
    async fn the_vision_task_is_used_with_a_picture() {
        let st = styles();
        let fake = Fake::new(vec![good(), good()]);
        propose_via(&fake, &prompt(&st, Some(img()), "")).await.expect("ok");
        propose_via(&fake, &prompt(&st, None, "")).await.expect("ok");
        let seen = fake.seen.lock().expect("lock");
        assert_eq!(seen[0].0, TASK_VISION);
        assert_eq!(seen[1].0, TASK);
    }

    #[tokio::test]
    async fn the_engine_that_answered_is_returned() {
        let st = styles();
        let fake = Fake::new(vec![json!({ "points": [] }), good()]);
        let (_, engine) = propose_via(&fake, &prompt(&st, None, "")).await.expect("ok");
        assert_eq!(engine, "engine1");
        let fake = Fake::new(vec![good()]);
        assert_eq!(propose_via(&fake, &prompt(&st, None, "")).await.expect("ok").1, "engine0");
    }
}
