//! Anthropic Messages API adapter (raw HTTP; Rust has no official SDK).
//! Always `Locality::Cloud`, so image requests are gated by the router's privacy check.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use futures::StreamExt;
use serde_json::{json, Value};

use super::{
    sse, Caps, ChatChunk, ChatProvider, ChatRequest, ChatStream, Locality, ProviderError, Role,
};
use crate::conversation::tools::ToolCall;

pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub struct AnthropicProvider {
    client: reqwest::Client,
    base: String,
    model: String,
    api_key: String,
    /// `output_config.effort`; a chat companion wants short, fast answers.
    effort: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: &str, effort: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            base: DEFAULT_BASE.to_string(),
            model: model.to_string(),
            api_key,
            effort: effort.to_string(),
        }
    }

    pub fn build_body(&self, req: &ChatRequest) -> Value {
        let b64 = base64::engine::general_purpose::STANDARD;
        let mut messages: Vec<Value> = Vec::new();
        for m in &req.messages {
            match m.role {
                Role::Tool => {
                    let block = json!({
                        "type": "tool_result",
                        "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                        "content": m.content,
                    });
                    // All tool results for one assistant turn go in a single user message.
                    let merged = messages.last_mut().is_some_and(|last| {
                        let is_results = last["role"] == "user"
                            && last["content"]
                                .as_array()
                                .is_some_and(|c| c.iter().all(|b| b["type"] == "tool_result"));
                        if is_results {
                            if let Some(arr) = last["content"].as_array_mut() {
                                arr.push(block.clone());
                            }
                        }
                        is_results
                    });
                    if !merged {
                        messages.push(json!({ "role": "user", "content": [block] }));
                    }
                }
                Role::User => {
                    let mut content: Vec<Value> = m
                        .images
                        .iter()
                        .map(|i| {
                            json!({ "type": "image", "source": {
                                "type": "base64", "media_type": "image/jpeg", "data": b64.encode(&i.jpeg)
                            }})
                        })
                        .collect();
                    content.push(json!({ "type": "text", "text": m.content }));
                    messages.push(json!({ "role": "user", "content": content }));
                }
                Role::Assistant => {
                    // Replay the provider's own blocks unchanged (thinking blocks must round-trip).
                    let content = match &m.provider_blocks {
                        Some(Value::Array(blocks)) if !blocks.is_empty() => {
                            Value::Array(blocks.clone())
                        }
                        _ => {
                            let mut blocks = Vec::new();
                            if !m.content.is_empty() {
                                blocks.push(json!({ "type": "text", "text": m.content }));
                            }
                            for c in &m.tool_calls {
                                blocks.push(json!({
                                    "type": "tool_use", "id": c.id, "name": c.name, "input": c.arguments
                                }));
                            }
                            Value::Array(blocks)
                        }
                    };
                    messages.push(json!({ "role": "assistant", "content": content }));
                }
            }
        }
        let mut body = json!({
            "model": self.model,
            "max_tokens": req.max_tokens.unwrap_or(16000),
            "system": req.system,
            "messages": messages,
            "stream": true,
            "output_config": { "effort": self.effort },
            "fallbacks": "default",
        });
        if !req.tools.is_empty() {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters,
                        "eager_input_streaming": true,
                    })
                })
                .collect();
        }
        body
    }
}

/// Accumulates SSE events into chunks and the final content blocks.
#[derive(Default)]
pub struct StreamState {
    blocks: BTreeMap<u64, Value>,
    partial_json: BTreeMap<u64, String>,
}

impl StreamState {
    /// Handle one SSE `data:` payload.
    pub fn handle(&mut self, data: &str) -> Result<Vec<ChatChunk>, ProviderError> {
        let v: Value =
            serde_json::from_str(data).map_err(|e| ProviderError::Protocol(e.to_string()))?;
        let idx = v["index"].as_u64().unwrap_or(0);
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or_default() {
            "content_block_start" => {
                self.blocks.insert(idx, v["content_block"].clone());
            }
            "content_block_delta" => {
                let d = &v["delta"];
                let block = self.blocks.entry(idx).or_insert_with(|| json!({}));
                match d["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or_default();
                        let prev = block["text"].as_str().unwrap_or_default().to_string();
                        block["text"] = json!(prev + t);
                        if !t.is_empty() {
                            out.push(ChatChunk::Text(t.to_string()));
                        }
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        let prev = block["thinking"].as_str().unwrap_or_default().to_string();
                        block["thinking"] = json!(prev + t);
                    }
                    "signature_delta" => {
                        block["signature"] = d["signature"].clone();
                    }
                    "input_json_delta" => {
                        self.partial_json
                            .entry(idx)
                            .or_default()
                            .push_str(d["partial_json"].as_str().unwrap_or_default());
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(block) = self.blocks.get_mut(&idx) {
                    if block["type"] == "tool_use" {
                        let raw = self.partial_json.remove(&idx).unwrap_or_default();
                        // Eager streaming means the API no longer validates input; guard the parse.
                        // Invalid JSON becomes `{}`, which fails schema validation downstream.
                        let input = if raw.trim().is_empty() {
                            json!({})
                        } else {
                            serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
                        };
                        block["input"] = input.clone();
                        out.push(ChatChunk::ToolCall(ToolCall {
                            id: block["id"].as_str().unwrap_or_default().to_string(),
                            name: block["name"].as_str().unwrap_or_default().to_string(),
                            arguments: input,
                        }));
                    }
                }
            }
            "message_delta" => {
                let stop = v["delta"]["stop_reason"].as_str().unwrap_or_default();
                if stop == "refusal" {
                    return Err(ProviderError::Protocol(
                        "the model declined this request".into(),
                    ));
                }
            }
            "message_stop" => {
                out.push(ChatChunk::ProviderBlocks(Value::Array(
                    self.blocks.values().cloned().collect(),
                )));
            }
            "error" => {
                let msg = v["error"]["message"].as_str().unwrap_or("stream error");
                return Err(ProviderError::Protocol(msg.to_string()));
            }
            _ => {}
        }
        Ok(out)
    }
}

#[async_trait]
impl ChatProvider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
    fn capabilities(&self) -> Caps {
        Caps {
            tools: true,
            vision: true,
            streaming: true,
        }
    }
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, ProviderError> {
        if self.api_key.is_empty() {
            return Err(ProviderError::MissingKey("Anthropic".into()));
        }
        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .json(&self.build_body(&req))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let mut state = StreamState::default();
        let stream = sse::lines(resp.bytes_stream()).flat_map(move |line| {
            let items: Vec<Result<ChatChunk, ProviderError>> = match line {
                Ok(l) => match l.strip_prefix("data:") {
                    Some(data) => match state.handle(data.trim()) {
                        Ok(chunks) => chunks.into_iter().map(Ok).collect(),
                        Err(e) => vec![Err(e)],
                    },
                    None => Vec::new(),
                },
                Err(e) => vec![Err(e)],
            };
            futures::stream::iter(items)
        });
        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ChatMessage;

    #[test]
    fn assembles_text_tool_use_and_blocks() {
        let mut s = StreamState::default();
        let events = [
            r#"{"type":"message_start","message":{}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"On it."}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"tu_1","name":"start_session","input":{}}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"task\":"}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"taxes\"}"}}"#,
            r#"{"type":"content_block_stop","index":2}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let chunks: Vec<ChatChunk> = events
            .iter()
            .flat_map(|e| s.handle(e).unwrap_or_default())
            .collect();
        assert_eq!(chunks[0], ChatChunk::Text("On it.".into()));
        assert!(
            matches!(&chunks[1], ChatChunk::ToolCall(c) if c.id == "tu_1" && c.arguments["task"] == "taxes")
        );
        match &chunks[2] {
            ChatChunk::ProviderBlocks(Value::Array(b)) => {
                assert_eq!(b.len(), 3);
                assert_eq!(b[0]["signature"], "sig");
                assert_eq!(b[2]["input"]["task"], "taxes");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn refusal_and_error_events_fail() {
        let mut s = StreamState::default();
        assert!(s
            .handle(r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#)
            .is_err());
        assert!(s
            .handle(r#"{"type":"error","error":{"message":"overloaded"}}"#)
            .is_err());
    }

    #[test]
    fn body_merges_tool_results_and_omits_sampling_params() {
        let p = AnthropicProvider::new("k".into(), DEFAULT_MODEL, "low");
        let call_a = ToolCall {
            id: "a".into(),
            name: "pause_session".into(),
            arguments: json!({}),
        };
        let call_b = ToolCall {
            id: "b".into(),
            name: "set_quiet".into(),
            arguments: json!({"minutes": 10}),
        };
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = vec![call_a.clone(), call_b.clone()];
        let req = ChatRequest {
            system: "sys".into(),
            messages: vec![
                ChatMessage::user("pause and be quiet"),
                assistant,
                ChatMessage::tool_result(&call_a, "ok"),
                ChatMessage::tool_result(&call_b, "ok"),
            ],
            temperature: Some(0.3),
            ..Default::default()
        };
        let body = p.build_body(&req);
        let msgs = body["messages"].as_array().cloned().unwrap_or_default();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["content"].as_array().map(|a| a.len()), Some(2));
        assert_eq!(msgs[1]["content"][1]["type"], "tool_use");
        assert!(body.get("temperature").is_none());
        assert_eq!(body["output_config"]["effort"], "low");
    }
}
