//! Structured (JSON) replies: extract, validate against the schema, retry once.
//!
//! Every engine is asked for constrained output (Claude `output_config.format`,
//! llama-server `response_format: json_schema`, Claude Code `--json-schema`),
//! but the reply is still checked here: local models occasionally wrap JSON
//! in a code fence or miss a required field. A failed check is retried once
//! (design §9).

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::LlmRequest;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The JSON value in a reply: the whole text, or inside a ```json fence, or
/// the outermost `{…}` / `[…]`.
pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Some(v);
    }
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```")
            && let Ok(v) = serde_json::from_str(after[..end].trim())
        {
            return Some(v);
        }
    }
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(a), Some(b)) = (t.find(open), t.rfind(close))
            && a < b
            && let Ok(v) = serde_json::from_str(&t[a..=b])
        {
            return Some(v);
        }
    }
    None
}

/// A small JSON Schema check: `type`, `properties`, `required`, `items`,
/// `enum`, `additionalProperties: false`. Enough for the schemas the apps
/// write; anything else in the schema is accepted as is.
pub fn validate(v: &Value, schema: &Value) -> Result<(), String> {
    validate_at(v, schema, "$")
}

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn validate_at(v: &Value, schema: &Value, at: &str) -> Result<(), String> {
    match &schema["type"] {
        Value::String(t) if !type_ok(v, t) => return Err(format!("{at} 應該是 {t}")),
        Value::Array(ts) if !ts.iter().filter_map(Value::as_str).any(|t| type_ok(v, t)) => {
            return Err(format!("{at} 的型別不符"));
        }
        _ => {}
    }
    if let Some(choices) = schema["enum"].as_array()
        && !choices.contains(v)
    {
        return Err(format!("{at} 不在允許的值裡"));
    }
    if let Some(obj) = v.as_object() {
        for k in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !obj.contains_key(k) {
                return Err(format!("{at} 缺少 {k}"));
            }
        }
        let props = schema["properties"].as_object();
        for (k, sub) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(s) => validate_at(sub, s, &format!("{at}.{k}"))?,
                None if schema["additionalProperties"] == Value::Bool(false) => {
                    return Err(format!("{at} 多了 {k}"));
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(s)) = (v.as_array(), schema.get("items")) {
        for (i, it) in items.iter().enumerate() {
            validate_at(it, s, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

/// Send a request that has a `json_schema`, return the checked JSON value.
/// One retry when the reply does not parse or does not match the schema.
pub async fn complete_json(p: &dyn LlmProvider, req: &LlmRequest) -> Result<Value, LlmError> {
    let schema = req.json_schema.clone().ok_or_else(|| LlmError::NotConfigured("complete_json 需要 json_schema".into()))?;
    let first = p.send(req).await?;
    let why = match extract_json(&first.text) {
        Some(v) => match validate(&v, &schema) {
            Ok(()) => return Ok(v),
            Err(e) => e,
        },
        None => "不是 JSON".to_string(),
    };
    let mut again = req.clone();
    again.system.push_str("\n\nReply with exactly one JSON value that matches the given schema, and nothing else.");
    let second = p.send(&again).await?;
    let v = extract_json(&second.text).ok_or_else(|| LlmError::Parse(p.id(), format!("兩次都不是 JSON（第一次：{why}）")))?;
    validate(&v, &schema).map_err(|e| LlmError::Parse(p.id(), e))?;
    Ok(v)
}

/// [`complete_json`] into a Rust type.
pub async fn complete_as<T: DeserializeOwned>(p: &dyn LlmProvider, req: &LlmRequest) -> Result<T, LlmError> {
    let v = complete_json(p, req).await?;
    serde_json::from_value(v).map_err(|e| LlmError::Parse(p.id(), e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProvider;
    use serde_json::json;

    fn schema() -> Value {
        json!({"type":"object","properties":{"word":{"type":"string"},"n":{"type":"integer"}},"required":["word"],"additionalProperties":false})
    }

    #[test]
    fn extracts_from_fences_and_prose() {
        assert_eq!(extract_json(r#"{"a":1}"#), Some(json!({"a":1})));
        assert_eq!(extract_json("Sure:\n```json\n{\"a\":1}\n```"), Some(json!({"a":1})));
        assert_eq!(extract_json("Here it is {\"a\": [1]} done"), Some(json!({"a":[1]})));
        assert_eq!(extract_json("no json"), None);
    }

    #[test]
    fn validates_the_common_keywords() {
        assert!(validate(&json!({"word":"x","n":2}), &schema()).is_ok());
        assert!(validate(&json!({"n":2}), &schema()).unwrap_err().contains("word"));
        assert!(validate(&json!({"word":1}), &schema()).is_err());
        assert!(validate(&json!({"word":"x","extra":1}), &schema()).unwrap_err().contains("extra"));
        let arr = json!({"type":"array","items":{"type":"string","enum":["a","b"]}});
        assert!(validate(&json!(["a","b"]), &arr).is_ok());
        assert!(validate(&json!(["c"]), &arr).is_err());
    }

    #[tokio::test]
    async fn retries_once_then_gives_up() {
        let p = MockProvider::new(vec!["oops".into(), "```json\n{\"word\":\"happy\"}\n```".into()]);
        let req = LlmRequest::prompt("s", "u").json(schema());
        assert_eq!(complete_json(&p, &req).await.unwrap(), json!({"word":"happy"}));
        {
            let seen = p.seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            assert!(seen[1].system.contains("exactly one JSON value"));
        }

        let bad = MockProvider::new(vec!["{\"n\":1}".into()]);
        assert!(matches!(complete_json(&bad, &req).await, Err(LlmError::Parse(..))));

        #[derive(serde::Deserialize)]
        struct W {
            word: String,
        }
        let ok = MockProvider::new(vec!["{\"word\":\"glad\"}".into()]);
        assert_eq!(complete_as::<W>(&ok, &req).await.unwrap().word, "glad");
    }
}
