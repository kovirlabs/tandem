//! Provider abstraction. Every adapter must report its `locality()` honestly:
//! the router's privacy gate depends on it.

pub mod anthropic;
pub mod mock;
pub mod ollama;
pub mod sse;

use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::conversation::tools::{ToolCall, ToolSpec};

/// Where a provider runs. `Lan` counts as local only once the user confirms they control the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Locality {
    Local,
    Lan { confirmed: bool },
    Cloud,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Caps {
    pub tools: bool,
    pub vision: bool,
    pub streaming: bool,
}

/// Request modality, used by the privacy gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    Text,
    Image,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

/// An image held in memory only. Deliberately not `Serialize`, and `Debug` never prints bytes,
/// so it cannot end up in logs, the database, or crash dumps.
#[derive(Clone)]
pub struct ImageData {
    pub jpeg: Vec<u8>,
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageData(<{} bytes, redacted>)", self.jpeg.len())
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    pub images: Vec<ImageData>,
    /// Tool calls requested by the assistant in this message.
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: which call this result answers.
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    /// Opaque provider content (e.g. Anthropic thinking blocks) replayed unchanged on later turns.
    pub provider_blocks: Option<serde_json::Value>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }
    pub fn tool_result(call: &ToolCall, content: impl Into<String>) -> Self {
        let mut m = Self::new(Role::Tool, content);
        m.tool_call_id = Some(call.id.clone());
        m.tool_name = Some(call.name.clone());
        m
    }
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            images: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
            provider_blocks: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    /// Ask the provider for a JSON-only response (vision contract, tool fallback).
    pub json_mode: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    pub fn modality(&self) -> Modality {
        if self.messages.iter().any(|m| !m.images.is_empty()) {
            Modality::Image
        } else {
            Modality::Text
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatChunk {
    Text(String),
    ToolCall(ToolCall),
    /// The provider's full assistant content for this turn, emitted once at the end of a stream.
    ProviderBlocks(serde_json::Value),
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatChunk, ProviderError>> + Send>>;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("couldn't reach the provider: {0}")]
    Unreachable(String),
    #[error("provider returned an error ({status}): {body}")]
    Http { status: u16, body: String },
    #[error("unexpected provider response: {0}")]
    Protocol(String),
    #[error("missing API key for {0}")]
    MissingKey(String),
    #[error("mock script exhausted")]
    MockExhausted,
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        // reqwest errors can include the URL but never request bodies.
        ProviderError::Unreachable(e.without_url().to_string())
    }
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn id(&self) -> &str;
    fn locality(&self) -> Locality;
    fn capabilities(&self) -> Caps;
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, ProviderError>;
}
