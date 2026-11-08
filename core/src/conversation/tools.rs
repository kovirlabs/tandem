//! LLM tools. Schemas are defined once in `prompts/tools.json`; calls are validated here
//! (native tool calls and the JSON fallback protocol go through the same parser).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::character::CharacterState;
use crate::error::{CoreError, Result};
use crate::session::CheckinMode;

const TOOLS_JSON: &str = include_str!("../../../prompts/tools.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// All tool specs from `prompts/tools.json`.
pub fn specs() -> &'static [ToolSpec] {
    static SPECS: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    SPECS.get_or_init(|| match serde_json::from_str(TOOLS_JSON) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("prompts/tools.json is invalid: {e}");
            Vec::new()
        }
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "name", content = "arguments", rename_all = "snake_case")]
pub enum Tool {
    StartSession {
        task: String,
        #[serde(default)]
        minutes: Option<u32>,
        #[serde(default)]
        checkin_mode: Option<CheckinMode>,
    },
    PauseSession {},
    ResumeSession {},
    ExtendSession {
        minutes: u32,
    },
    EndSession {
        #[serde(default)]
        reflection: Option<String>,
    },
    StartBreak {
        minutes: u32,
    },
    AddSubtasks {
        items: Vec<String>,
    },
    CompleteSubtask {
        id: String,
    },
    SetQuiet {
        minutes: u32,
    },
    SetCharacterState {
        state: CharacterState,
    },
}

/// Local models often send numbers as strings or `null` for omitted args; normalise first.
fn normalise(mut args: Value) -> Value {
    if let Value::Object(map) = &mut args {
        map.retain(|_, v| !v.is_null());
        let key = "minutes";
        if let Some(Value::String(s)) = map.get(key) {
            if let Ok(n) = s.trim().parse::<u32>() {
                map.insert(key.to_string(), json!(n));
            }
        }
        if let Some(Value::Number(n)) = map.get(key) {
            if let Some(f) = n.as_f64() {
                if f.fract() != 0.0 || n.is_f64() {
                    map.insert(key.to_string(), json!(f.round().max(0.0) as u32));
                }
            }
        }
        if let Some(Value::Number(n)) = map.get("id") {
            let s = n.to_string();
            map.insert("id".into(), json!(s));
        }
    }
    if args.is_null() {
        json!({})
    } else {
        args
    }
}

/// Validate a raw tool call against the known tools.
pub fn parse(call: &ToolCall) -> Result<Tool> {
    if !specs().iter().any(|s| s.name == call.name) {
        return Err(CoreError::Tool(format!("unknown tool '{}'", call.name)));
    }
    let v = json!({ "name": call.name, "arguments": normalise(call.arguments.clone()) });
    serde_json::from_value(v).map_err(|e| CoreError::Tool(format!("{}: {e}", call.name)))
}

/// JSON fallback for models without native tool calling: extract `<tool>{...}</tool>` blocks.
/// Returns the remaining text and the calls found.
pub fn extract_fallback_calls(text: &str) -> (String, Vec<ToolCall>) {
    let mut calls = Vec::new();
    let mut rest = String::new();
    let mut cursor = text;
    while let Some(start) = cursor.find("<tool>") {
        rest.push_str(&cursor[..start]);
        let after = &cursor[start + 6..];
        let (inner, next) = match after.find("</tool>") {
            Some(end) => (&after[..end], &after[end + 7..]),
            None => (after, ""),
        };
        if let Ok(v) = serde_json::from_str::<Value>(inner.trim()) {
            let name = v["name"].as_str().unwrap_or_default().to_string();
            if !name.is_empty() {
                calls.push(ToolCall {
                    id: format!("call_{}", uuid::Uuid::new_v4().simple()),
                    name,
                    arguments: v.get("arguments").cloned().unwrap_or(json!({})),
                });
            }
        }
        cursor = next;
    }
    rest.push_str(cursor);
    (rest.trim().to_string(), calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: args,
        }
    }

    #[test]
    fn every_spec_parses_into_a_tool() {
        let samples = [
            json!({"task": "t", "minutes": 25, "checkin_mode": "gentle"}),
            json!({}),
            json!({}),
            json!({"minutes": 5}),
            json!({"reflection": "done"}),
            json!({"minutes": 5}),
            json!({"items": ["a"]}),
            json!({"id": "x"}),
            json!({"minutes": 10}),
            json!({"state": "working"}),
        ];
        assert_eq!(specs().len(), samples.len());
        for (spec, args) in specs().iter().zip(samples) {
            assert!(parse(&call(&spec.name, args)).is_ok(), "{}", spec.name);
        }
    }

    #[test]
    fn tolerant_of_local_model_quirks() {
        assert_eq!(
            parse(&call(
                "start_session",
                json!({"task": "x", "minutes": "30", "checkin_mode": null})
            ))
            .ok(),
            Some(Tool::StartSession {
                task: "x".into(),
                minutes: Some(30),
                checkin_mode: None
            })
        );
        assert_eq!(
            parse(&call("extend_session", json!({"minutes": 10.0}))).ok(),
            Some(Tool::ExtendSession { minutes: 10 })
        );
        assert_eq!(
            parse(&call("complete_subtask", json!({"id": 2}))).ok(),
            Some(Tool::CompleteSubtask { id: "2".into() })
        );
        assert!(parse(&call("pause_session", Value::Null)).is_ok());
    }

    #[test]
    fn rejects_invalid_calls() {
        assert!(parse(&call("launch_rockets", json!({}))).is_err());
        assert!(parse(&call("start_session", json!({}))).is_err());
        assert!(parse(&call("set_character_state", json!({"state": "angry"}))).is_err());
        assert!(parse(&call(
            "start_session",
            json!({"task": "x", "checkin_mode": "loud"})
        ))
        .is_err());
    }

    #[test]
    fn fallback_extraction() {
        let (text, calls) = extract_fallback_calls(
            "<tool>{\"name\": \"start_session\", \"arguments\": {\"task\": \"dishes\"}}</tool>\nLet's go.",
        );
        assert_eq!(text, "Let's go.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["task"], "dishes");
        let (text, calls) = extract_fallback_calls("No tools here.");
        assert_eq!((text.as_str(), calls.len()), ("No tools here.", 0));
        let (_, calls) = extract_fallback_calls("<tool>not json</tool>ok");
        assert!(calls.is_empty());
    }
}
