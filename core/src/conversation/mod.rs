//! Conversation orchestrator: builds prompts, streams replies, runs tool calls.

pub mod prompt;
pub mod safety;
pub mod tools;

use std::sync::{Arc, RwLock};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::character::CharacterState;
use crate::checkin::{CheckinAction, CheckinReason};
use crate::error::{CoreError, Result};
use crate::events::{CoreEvent, CoreNotice, EventSink};
use crate::providers::{ChatChunk, ChatMessage, ChatRequest, Role};
use crate::router::privacy::Consent;
use crate::router::Router;
use crate::session::engine::SessionEngine;
use crate::store::Store;
use prompt::PromptContext;

/// How many past messages go into each request.
const HISTORY_WINDOW: usize = 24;
/// Tool-call rounds per user turn before we stop looping.
const MAX_ROUNDS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Via {
    Text,
    Voice,
    Checkin,
}

impl Via {
    pub fn as_str(&self) -> &'static str {
        match self {
            Via::Text => "text",
            Via::Voice => "voice",
            Via::Checkin => "checkin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatRole {
    User,
    Assistant,
}

/// A finished chat message pushed on `chat://message`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ChatMessageDto {
    pub id: String,
    pub role: ChatRole,
    pub text: String,
    pub via: Via,
    pub ts: String,
}

/// A streaming text fragment pushed on `chat://delta`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ChatDelta {
    pub message_id: String,
    pub delta: String,
}

pub struct Orchestrator {
    router: Arc<Router>,
    engine: Arc<SessionEngine>,
    sink: Arc<dyn EventSink>,
    store: Option<Arc<Store>>,
    /// Async lock so turns (user messages and check-ins) never interleave.
    history: tokio::sync::Mutex<Vec<ChatMessage>>,
    ctx: RwLock<PromptContext>,
}

/// Remove `<think>…</think>` sections some local models put in their content.
pub fn strip_think(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + 8..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Keep at most `max` sentences (spoken replies stay short by default).
pub fn limit_sentences(text: &str, max: usize) -> String {
    let mut count = 0;
    for (i, c) in text.char_indices() {
        if matches!(c, '.' | '!' | '?') {
            let next = text[i + c.len_utf8()..].chars().next();
            if next.is_none_or(char::is_whitespace) {
                count += 1;
                if count == max {
                    return text[..i + c.len_utf8()].trim().to_string();
                }
            }
        }
    }
    text.trim().to_string()
}

/// The last `window` messages, never starting mid tool exchange.
fn windowed(history: &[ChatMessage], window: usize) -> Vec<ChatMessage> {
    let start = history.len().saturating_sub(window);
    let mut slice = &history[start..];
    while let Some(first) = slice.first() {
        if first.role == Role::User {
            break;
        }
        slice = &slice[1..];
    }
    slice.to_vec()
}

fn now_ts() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Orchestrator {
    pub fn new(
        router: Arc<Router>,
        engine: Arc<SessionEngine>,
        sink: Arc<dyn EventSink>,
        store: Option<Arc<Store>>,
        ctx: PromptContext,
    ) -> Self {
        Self {
            router,
            engine,
            sink,
            store,
            history: tokio::sync::Mutex::new(Vec::new()),
            ctx: RwLock::new(ctx),
        }
    }

    pub fn set_context(&self, ctx: PromptContext) {
        if let Ok(mut g) = self.ctx.write() {
            *g = ctx;
        }
    }

    fn context(&self) -> PromptContext {
        self.ctx.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn consent(&self) -> Consent {
        Consent {
            cloud_vision: self.engine.snapshot().cloud_vision,
            cloud_audio: false,
        }
    }

    fn persist(&self, role: &str, text: &str, via: Via) {
        if let Some(store) = &self.store {
            let sid = self.engine.session_id();
            if let Err(e) = store.add_message(sid.as_deref(), role, text, via.as_str()) {
                tracing::warn!("failed to save message: {e}");
            }
        }
    }

    fn notice(&self, key: &str, err: &CoreError) {
        self.sink.emit(CoreEvent::Notice(CoreNotice {
            key: key.to_string(),
            detail: Some(err.to_string()),
        }));
    }

    pub async fn clear_history(&self) {
        self.history.lock().await.clear();
    }

    /// Handle one user message end to end. Returns the final assistant message, if any text was produced.
    pub async fn send(&self, text: &str, via: Via) -> Result<Option<ChatMessageDto>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let mut history = self.history.lock().await;
        let user = ChatMessageDto {
            id: uuid::Uuid::new_v4().to_string(),
            role: ChatRole::User,
            text: text.to_string(),
            via,
            ts: now_ts(),
        };
        self.sink.emit(CoreEvent::ChatMessage(user));
        self.persist("user", text, via);
        history.push(ChatMessage::user(text));

        self.engine.set_overlay(Some(CharacterState::Thinking));
        let result = self.run_turn(&mut history, via, text).await;
        self.engine.set_overlay(None);
        match result {
            Err(e) if safety::is_crisis(text) => {
                // No model available, but crisis resources must still reach the user.
                tracing::warn!("reply failed during a crisis message: {e}");
                let reply = safety::resources().to_string();
                history.push(ChatMessage::assistant(reply.clone()));
                Ok(Some(self.finish(
                    uuid::Uuid::new_v4().to_string(),
                    reply,
                    via,
                )))
            }
            Err(e) => {
                self.notice("notice.reply_failed", &e);
                Err(e)
            }
            ok => ok,
        }
    }

    fn finish(&self, id: String, text: String, via: Via) -> ChatMessageDto {
        let dto = ChatMessageDto {
            id,
            role: ChatRole::Assistant,
            text,
            via,
            ts: now_ts(),
        };
        self.persist("assistant", &dto.text, via);
        self.sink.emit(CoreEvent::ChatMessage(dto.clone()));
        dto
    }

    async fn run_turn(
        &self,
        history: &mut Vec<ChatMessage>,
        via: Via,
        user_text: &str,
    ) -> Result<Option<ChatMessageDto>> {
        let provider = self
            .router
            .chat_provider()
            .ok_or(CoreError::NoProvider("chat"))?;
        let native_tools = provider.capabilities().tools;
        let mut final_text = String::new();
        let mut final_id = String::new();

        for _round in 0..MAX_ROUNDS {
            let system =
                prompt::system_prompt(&self.context(), &self.engine.snapshot(), !native_tools);
            let req = ChatRequest {
                system,
                messages: windowed(history, HISTORY_WINDOW),
                tools: tools::specs().to_vec(),
                ..Default::default()
            };
            let mut stream = self.router.chat_stream(req, self.consent()).await?;
            let msg_id = uuid::Uuid::new_v4().to_string();
            let mut text = String::new();
            let mut calls = Vec::new();
            let mut blocks = None;
            while let Some(chunk) = stream.next().await {
                match chunk? {
                    ChatChunk::Text(t) => {
                        if native_tools {
                            if text.is_empty() {
                                self.engine.set_overlay(Some(CharacterState::Talking));
                            }
                            self.sink.emit(CoreEvent::ChatDelta(ChatDelta {
                                message_id: msg_id.clone(),
                                delta: t.clone(),
                            }));
                        }
                        text.push_str(&t);
                    }
                    ChatChunk::ToolCall(c) => calls.push(c),
                    ChatChunk::ProviderBlocks(b) => blocks = Some(b),
                }
            }
            if !native_tools {
                let (rest, found) = tools::extract_fallback_calls(&text);
                text = rest;
                calls.extend(found);
            }
            let clean = strip_think(&text);
            let mut assistant = ChatMessage::assistant(clean.clone());
            assistant.tool_calls = calls.clone();
            assistant.provider_blocks = blocks;
            history.push(assistant);

            if !clean.is_empty() {
                final_text = clean;
                final_id = msg_id;
            }
            if calls.is_empty() {
                break;
            }
            let mut any_error = false;
            for call in &calls {
                let outcome = tools::parse(call).and_then(|t| self.engine.execute_tool(t));
                let result = match outcome {
                    Ok(r) => r,
                    Err(e) => {
                        any_error = true;
                        format!("error: {e}")
                    }
                };
                history.push(ChatMessage::tool_result(call, result));
            }
            // If the model already replied and every tool worked, we're done.
            if !final_text.is_empty() && !any_error {
                break;
            }
        }

        let checked = safety::ensure_resources(user_text, &final_text);
        if checked != final_text {
            if let Some(last) = history.iter_mut().rev().find(|m| m.role == Role::Assistant) {
                last.content = checked.clone();
            }
            final_text = checked;
            if final_id.is_empty() {
                final_id = uuid::Uuid::new_v4().to_string();
            }
        }
        if final_text.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.finish(final_id, final_text, via)))
    }

    /// Generate a short check-in line. Returns `None` on failure (the UI then uses fallback copy).
    pub async fn checkin_line(
        &self,
        reason: CheckinReason,
        action: CheckinAction,
    ) -> Option<String> {
        let mut history = self.history.lock().await;
        let cue = prompt::checkin_cue(reason, action);
        let mut messages = windowed(&history, HISTORY_WINDOW);
        messages.push(ChatMessage::user(cue.clone()));
        let req = ChatRequest {
            system: prompt::system_prompt(&self.context(), &self.engine.snapshot(), false),
            messages,
            // Tools stay declared (history may contain tool calls) but the cue says not to use them.
            tools: tools::specs().to_vec(),
            ..Default::default()
        };
        let result: Result<String> = async {
            let mut stream = self.router.chat_stream(req, self.consent()).await?;
            let mut text = String::new();
            while let Some(chunk) = stream.next().await {
                if let ChatChunk::Text(t) = chunk? {
                    text.push_str(&t);
                }
            }
            Ok(text)
        }
        .await;
        let line = match result {
            Ok(t) => limit_sentences(&strip_think(&tools::extract_fallback_calls(&t).0), 2),
            Err(e) => {
                tracing::warn!("check-in line failed: {e}");
                return None;
            }
        };
        if line.is_empty() {
            return None;
        }
        history.push(ChatMessage::user(cue));
        history.push(ChatMessage::assistant(line.clone()));
        self.persist("assistant", &line, Via::Checkin);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CollectingSink;
    use crate::providers::mock::MockProvider;
    use crate::providers::{Caps, Locality};
    use crate::session::engine::EngineConfig;
    use crate::session::SessionPhase;
    use serde_json::json;

    fn setup(
        mock: MockProvider,
    ) -> (
        Arc<MockProvider>,
        Arc<CollectingSink>,
        Arc<SessionEngine>,
        Orchestrator,
    ) {
        let sink = Arc::new(CollectingSink::default());
        let mock = Arc::new(mock);
        let router = Arc::new(Router::new());
        router.set_chat(Some(mock.clone()));
        let engine = Arc::new(SessionEngine::new(
            sink.clone(),
            None,
            EngineConfig::default(),
        ));
        let orch = Orchestrator::new(
            router,
            engine.clone(),
            sink.clone(),
            None,
            PromptContext::default(),
        );
        (mock, sink, engine, orch)
    }

    #[tokio::test]
    async fn plain_reply_streams_and_finishes() {
        let (mock, sink, _engine, orch) =
            setup(MockProvider::new(Locality::Local).with_text_reply("I'm right here."));
        let reply = orch.send("hi", Via::Text).await.ok().flatten();
        assert_eq!(reply.map(|r| r.text), Some("I'm right here.".to_string()));
        let events = sink.take();
        assert!(events.iter().any(|e| matches!(e, CoreEvent::ChatDelta(_))));
        let req = &mock.requests()[0];
        assert!(req.system.contains("You are Timber"));
        assert_eq!(req.tools.len(), tools::specs().len());
    }

    #[tokio::test]
    async fn tool_call_then_followup_reply() {
        let mock = MockProvider::new(Locality::Local)
            .with_tool_call(
                "start_session",
                json!({"task": "expense report", "minutes": 25}),
                "",
            )
            .with_text_reply("Started. Tiny first step: open the receipts folder.");
        let (mock, _sink, engine, orch) = setup(mock);
        let reply = orch
            .send("I need to do my expense report", Via::Text)
            .await
            .ok()
            .flatten();
        assert_eq!(engine.phase(), SessionPhase::Focusing);
        assert!(reply.is_some_and(|r| r.text.starts_with("Started")));
        // Second request carries the tool result back to the model.
        let second = &mock.requests()[1];
        assert!(second
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content.contains("expense report")));
        assert!(second
            .system
            .contains("focus session on \"expense report\""));
    }

    #[tokio::test]
    async fn tool_errors_are_reported_back_to_the_model() {
        let mock = MockProvider::new(Locality::Local)
            .with_tool_call("pause_session", json!({}), "Paused!")
            .with_text_reply("There's no session running yet. Want to start one?");
        let (mock, _sink, _engine, orch) = setup(mock);
        let reply = orch.send("pause", Via::Text).await.ok().flatten();
        assert!(reply.is_some_and(|r| r.text.contains("no session")));
        let second = &mock.requests()[1];
        assert!(second
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content.starts_with("error:")));
    }

    #[tokio::test]
    async fn json_fallback_for_models_without_tools() {
        let mock = MockProvider::new(Locality::Local)
            .with_caps(Caps { tools: false, vision: false, streaming: true })
            .with_text_reply("<tool>{\"name\": \"start_session\", \"arguments\": {\"task\": \"dishes\", \"minutes\": 10}}</tool> Let's do it.");
        let (mock, sink, engine, orch) = setup(mock);
        let reply = orch.send("dishes for 10", Via::Text).await.ok().flatten();
        assert_eq!(engine.snapshot().planned_secs, 600);
        assert_eq!(reply.map(|r| r.text), Some("Let's do it.".to_string()));
        assert!(mock.requests()[0].system.contains("<tool>"));
        // Fallback mode doesn't stream raw tool markup to the UI.
        assert!(!sink
            .take()
            .iter()
            .any(|e| matches!(e, CoreEvent::ChatDelta(_))));
    }

    #[tokio::test]
    async fn provider_failure_emits_notice_and_restores_character() {
        let (mock, sink, engine, orch) = setup(MockProvider::new(Locality::Local));
        mock.fail("connection refused");
        assert!(orch.send("hello", Via::Text).await.is_err());
        assert_eq!(engine.character_state(), CharacterState::Idle);
        assert!(sink
            .take()
            .iter()
            .any(|e| matches!(e, CoreEvent::Notice(n) if n.key == "notice.reply_failed")));
    }

    #[tokio::test]
    async fn checkin_line_is_trimmed_and_kept_in_history() {
        let mock = MockProvider::new(Locality::Local)
            .with_text_reply("Still cruising? I'm right here. Also, a third sentence.")
            .with_text_reply("ok");
        let (mock, _sink, _engine, orch) = setup(mock);
        let line = orch
            .checkin_line(CheckinReason::Interval, CheckinAction::GentleLine)
            .await;
        assert_eq!(line.as_deref(), Some("Still cruising? I'm right here."));
        let _ = orch.send("thanks", Via::Text).await;
        let msgs = &mock.requests()[1].messages;
        assert!(msgs
            .iter()
            .any(|m| m.role == Role::Assistant && m.content.starts_with("Still cruising")));
    }

    #[tokio::test]
    async fn crisis_resources_are_guaranteed() {
        let mock = MockProvider::new(Locality::Local).with_text_reply("I'm glad you told me.");
        let (_mock, _sink, _engine, orch) = setup(mock);
        let reply = orch
            .send("everyone would be better off without me", Via::Text)
            .await
            .ok()
            .flatten()
            .map(|r| r.text)
            .unwrap_or_default();
        assert!(reply.starts_with("I'm glad you told me."));
        assert!(reply.contains("988"));
    }

    #[tokio::test]
    async fn crisis_resources_even_when_provider_is_down() {
        let (mock, _sink, _engine, orch) = setup(MockProvider::new(Locality::Local));
        mock.fail("offline");
        let reply = orch.send("I want to die", Via::Text).await;
        assert!(reply.ok().flatten().is_some_and(|r| r.text.contains("988")));
    }

    #[test]
    fn helpers() {
        assert_eq!(strip_think("<think>hmm</think> Hello"), "Hello");
        assert_eq!(strip_think("a<think>unterminated"), "a");
        assert_eq!(limit_sentences("One. Two! Three?", 2), "One. Two!");
        assert_eq!(limit_sentences("v1.2 is out. Yes.", 1), "v1.2 is out.");
        assert_eq!(limit_sentences("No end", 2), "No end");
        let h = vec![
            ChatMessage::assistant("a"),
            ChatMessage::user("b"),
            ChatMessage::assistant("c"),
        ];
        assert_eq!(windowed(&h, 3).len(), 2);
        assert_eq!(windowed(&h, 1).len(), 0);
    }
}
