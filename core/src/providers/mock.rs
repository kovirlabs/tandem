//! Deterministic provider for tests: replays scripted turns and records requests.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;

use super::{Caps, ChatChunk, ChatProvider, ChatRequest, ChatStream, Locality, ProviderError};
use crate::conversation::tools::ToolCall;

pub struct MockProvider {
    locality: Locality,
    caps: Caps,
    script: Mutex<VecDeque<Vec<ChatChunk>>>,
    requests: Mutex<Vec<ChatRequest>>,
    fail_with: Mutex<Option<String>>,
}

impl MockProvider {
    pub fn new(locality: Locality) -> Self {
        Self {
            locality,
            caps: Caps {
                tools: true,
                vision: true,
                streaming: true,
            },
            script: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            fail_with: Mutex::new(None),
        }
    }

    pub fn with_caps(mut self, caps: Caps) -> Self {
        self.caps = caps;
        self
    }

    /// Queue one assistant turn that streams `text` word by word.
    pub fn with_text_reply(self, text: &str) -> Self {
        self.push_turn(split_words(text));
        self
    }

    /// Queue one assistant turn made of arbitrary chunks.
    pub fn with_turn(self, chunks: Vec<ChatChunk>) -> Self {
        self.push_turn(chunks);
        self
    }

    /// Queue a turn that calls one tool, optionally followed by text.
    pub fn with_tool_call(self, name: &str, arguments: serde_json::Value, text: &str) -> Self {
        let mut chunks = vec![ChatChunk::ToolCall(ToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            arguments,
        })];
        chunks.extend(split_words(text));
        self.push_turn(chunks);
        self
    }

    /// Make every subsequent request fail as if the provider were unreachable.
    pub fn fail(&self, reason: &str) {
        if let Ok(mut g) = self.fail_with.lock() {
            *g = Some(reason.to_string());
        }
    }

    pub fn push_turn(&self, chunks: Vec<ChatChunk>) {
        if let Ok(mut g) = self.script.lock() {
            g.push_back(chunks);
        }
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

fn split_words(text: &str) -> Vec<ChatChunk> {
    text.split_inclusive(' ')
        .map(|w| ChatChunk::Text(w.to_string()))
        .collect()
}

#[async_trait]
impl ChatProvider for MockProvider {
    fn id(&self) -> &str {
        "mock"
    }
    fn locality(&self) -> Locality {
        self.locality
    }
    fn capabilities(&self) -> Caps {
        self.caps
    }
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, ProviderError> {
        if let Some(reason) = self.fail_with.lock().ok().and_then(|g| g.clone()) {
            return Err(ProviderError::Unreachable(reason));
        }
        if let Ok(mut g) = self.requests.lock() {
            g.push(req);
        }
        let turn = self
            .script
            .lock()
            .ok()
            .and_then(|mut g| g.pop_front())
            .ok_or(ProviderError::MockExhausted)?;
        Ok(Box::pin(stream::iter(turn.into_iter().map(Ok))))
    }
}
