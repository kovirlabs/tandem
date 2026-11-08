//! Ollama adapter (`/api/chat`, NDJSON streaming, native tools and images).

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    sse, Caps, ChatChunk, ChatProvider, ChatRequest, ChatStream, Locality, ProviderError, Role,
};
use crate::conversation::tools::ToolCall;
use crate::router::locality;

pub struct OllamaProvider {
    client: reqwest::Client,
    base: String,
    model: String,
    locality: Locality,
    caps: Caps,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default()
}

fn trim_base(base: &str) -> String {
    base.trim_end_matches('/').to_string()
}

impl OllamaProvider {
    pub fn new(base: &str, model: &str, confirmed_hosts: &[String], caps: Caps) -> Self {
        Self {
            client: client(),
            base: trim_base(base),
            model: model.to_string(),
            locality: locality::classify(base, confirmed_hosts),
            caps,
        }
    }

    /// Ask Ollama what a model can do (`/api/show` → `capabilities`).
    pub async fn probe_caps(base: &str, model: &str) -> Result<Caps, ProviderError> {
        let resp = client()
            .post(format!("{}/api/show", trim_base(base)))
            .timeout(Duration::from_secs(10))
            .json(&json!({ "model": model }))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let v: Value = resp.json().await?;
        let caps: Vec<String> = v
            .get("capabilities")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            .unwrap_or_default();
        Ok(Caps {
            tools: caps.iter().any(|c| c == "tools"),
            vision: caps.iter().any(|c| c == "vision"),
            streaming: true,
        })
    }

    /// List installed models (`/api/tags`).
    pub async fn list_models(base: &str) -> Result<Vec<String>, ProviderError> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<Model>,
        }
        #[derive(Deserialize)]
        struct Model {
            name: String,
        }
        let resp = client()
            .get(format!("{}/api/tags", trim_base(base)))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let tags: Tags = resp.json().await?;
        let mut names: Vec<String> = tags.models.into_iter().map(|m| m.name).collect();
        names.sort();
        Ok(names)
    }

    pub fn build_body(&self, req: &ChatRequest) -> Value {
        let b64 = base64::engine::general_purpose::STANDARD;
        let mut messages = vec![json!({ "role": "system", "content": req.system })];
        for m in &req.messages {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let mut msg = json!({ "role": role, "content": m.content });
            if !m.images.is_empty() {
                msg["images"] = m.images.iter().map(|i| b64.encode(&i.jpeg)).collect();
            }
            if !m.tool_calls.is_empty() {
                msg["tool_calls"] = m
                    .tool_calls
                    .iter()
                    .map(|c| json!({ "function": { "name": c.name, "arguments": c.arguments } }))
                    .collect();
            }
            if let Some(name) = &m.tool_name {
                msg["tool_name"] = json!(name);
            }
            messages.push(msg);
        }
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });
        if !req.tools.is_empty() && self.caps.tools {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({ "type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.parameters
                    }})
                })
                .collect();
        }
        if req.json_mode {
            body["format"] = json!("json");
        }
        let mut options = serde_json::Map::new();
        if let Some(t) = req.temperature {
            options.insert("temperature".into(), json!(t));
        }
        if let Some(n) = req.max_tokens {
            options.insert("num_predict".into(), json!(n));
        }
        if !options.is_empty() {
            body["options"] = Value::Object(options);
        }
        body
    }
}

/// Parse one NDJSON line from `/api/chat` into zero or more chunks.
pub fn parse_line(line: &str) -> Result<Vec<ChatChunk>, ProviderError> {
    if line.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: Value =
        serde_json::from_str(line).map_err(|e| ProviderError::Protocol(e.to_string()))?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(ProviderError::Protocol(err.to_string()));
    }
    let mut out = Vec::new();
    if let Some(msg) = v.get("message") {
        if let Some(text) = msg.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                out.push(ChatChunk::Text(text.to_string()));
            }
        }
        if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let f = call.get("function").unwrap_or(call);
                let name = f.get("name").and_then(Value::as_str).unwrap_or_default();
                let arguments = match f.get("arguments") {
                    // Some models send arguments as a JSON string.
                    Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
                    Some(a) => a.clone(),
                    None => json!({}),
                };
                out.push(ChatChunk::ToolCall(ToolCall {
                    id: format!("call_{}", uuid::Uuid::new_v4().simple()),
                    name: name.to_string(),
                    arguments,
                }));
            }
        }
    }
    Ok(out)
}

#[async_trait]
impl ChatProvider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
    }
    fn locality(&self) -> Locality {
        self.locality
    }
    fn capabilities(&self) -> Caps {
        self.caps
    }
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, ProviderError> {
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base))
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
        let stream = sse::lines(resp.bytes_stream())
            .map(|line| line.and_then(|l| parse_line(&l)))
            .flat_map(|res| {
                let items: Vec<Result<ChatChunk, ProviderError>> = match res {
                    Ok(chunks) => chunks.into_iter().map(Ok).collect(),
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
    use crate::providers::{ChatMessage, ImageData};

    #[test]
    fn parses_text_and_tool_calls() {
        let t = parse_line(r#"{"message":{"role":"assistant","content":"Hi"},"done":false}"#);
        assert_eq!(t.ok(), Some(vec![ChatChunk::Text("Hi".into())]));

        let c = parse_line(
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"start_session","arguments":{"task":"x"}}}]},"done":false}"#,
        )
        .unwrap_or_default();
        assert!(
            matches!(&c[..], [ChatChunk::ToolCall(tc)] if tc.name == "start_session" && tc.arguments["task"] == "x")
        );

        let s = parse_line(
            r#"{"message":{"content":"","tool_calls":[{"function":{"name":"pause_session","arguments":"{}"}}]}}"#,
        )
        .unwrap_or_default();
        assert!(matches!(&s[..], [ChatChunk::ToolCall(tc)] if tc.arguments == json!({})));

        assert!(parse_line(r#"{"error":"model not found"}"#).is_err());
        assert!(parse_line("").map(|v| v.is_empty()).unwrap_or(false));
        assert!(parse_line(r#"{"done":true}"#)
            .map(|v| v.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn body_includes_images_tools_and_format() {
        let p = OllamaProvider::new(
            "http://localhost:11434/",
            "llava",
            &[],
            Caps {
                tools: true,
                vision: true,
                streaming: true,
            },
        );
        assert_eq!(p.locality(), Locality::Local);
        let mut m = ChatMessage::user("look");
        m.images.push(ImageData {
            jpeg: vec![0xff, 0xd8],
        });
        let req = ChatRequest {
            system: "sys".into(),
            messages: vec![m],
            tools: crate::conversation::tools::specs().to_vec(),
            json_mode: true,
            temperature: Some(0.2),
            max_tokens: Some(64),
        };
        let body = p.build_body(&req);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["images"][0], "/9g=");
        assert_eq!(body["format"], "json");
        assert_eq!(body["options"]["num_predict"], 64);
        assert!(body["tools"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn lan_locality_reflects_confirmation() {
        let caps = Caps::default();
        let p = OllamaProvider::new("http://192.168.1.9:11434", "m", &[], caps);
        assert_eq!(p.locality(), Locality::Lan { confirmed: false });
        let p = OllamaProvider::new(
            "http://192.168.1.9:11434",
            "m",
            &["192.168.1.9".into()],
            caps,
        );
        assert_eq!(p.locality(), Locality::Lan { confirmed: true });
    }

    /// Run with `OLLAMA_URL=http://host:11434 OLLAMA_MODEL=... cargo test -p core -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_ollama_roundtrip() {
        let base = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".into());
        let caps = OllamaProvider::probe_caps(&base, &model)
            .await
            .expect("probe");
        let p = OllamaProvider::new(&base, &model, &[], caps);
        let req = ChatRequest {
            system: "Reply with one word.".into(),
            messages: vec![ChatMessage::user("Say hello")],
            ..Default::default()
        };
        let mut s = p.chat_stream(req).await.expect("stream");
        let mut text = String::new();
        while let Some(c) = s.next().await {
            if let Ok(ChatChunk::Text(t)) = c {
                text.push_str(&t);
            }
        }
        assert!(!text.is_empty());
    }
}
