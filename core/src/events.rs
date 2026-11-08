//! Events the core pushes to the UI. Each variant maps to one Tauri event channel.

use std::sync::Mutex;

use serde::Serialize;
use ts_rs::TS;

use crate::audio::{AudioLevel, DownloadProgress, VoiceStatus};
use crate::character::CharacterStateEvent;
use crate::checkin::CheckinEvent;
use crate::conversation::{ChatDelta, ChatMessageDto};
use crate::session::{SessionSnapshot, SessionTick};
use crate::settings::{Profile, Settings};

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CoreNotice {
    /// i18n key for the UI (e.g. `notice.provider_unreachable`).
    pub key: String,
    /// Optional technical detail; never contains image data or secrets.
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CoreEvent {
    SessionState(SessionSnapshot),
    SessionTick(SessionTick),
    Checkin(CheckinEvent),
    ChatDelta(ChatDelta),
    ChatMessage(ChatMessageDto),
    CharacterState(CharacterStateEvent),
    Notice(CoreNotice),
    /// Profile changed (name, character, tone…) so every window can re-render.
    Profile(Profile),
    Settings(Settings),
    /// Voice pipeline phase and readiness (M2).
    Voice(VoiceStatus),
    /// Mic meter / mouth movement level (M2), ~30 Hz.
    Level(AudioLevel),
    /// Model download progress (M2).
    Download(DownloadProgress),
}

impl CoreEvent {
    pub const SESSION_STATE: &'static str = "session://state";
    pub const SESSION_TICK: &'static str = "session://tick";
    pub const CHECKIN: &'static str = "checkin://prompt";
    pub const CHAT_DELTA: &'static str = "chat://delta";
    pub const CHAT_MESSAGE: &'static str = "chat://message";
    pub const CHARACTER_STATE: &'static str = "character://state";
    pub const NOTICE: &'static str = "core://notice";
    pub const PROFILE: &'static str = "core://profile";
    pub const SETTINGS: &'static str = "core://settings";
    pub const VOICE: &'static str = "voice://state";
    pub const LEVEL: &'static str = "voice://level";
    pub const DOWNLOAD: &'static str = "voice://download";

    pub fn channel(&self) -> &'static str {
        match self {
            CoreEvent::SessionState(_) => Self::SESSION_STATE,
            CoreEvent::SessionTick(_) => Self::SESSION_TICK,
            CoreEvent::Checkin(_) => Self::CHECKIN,
            CoreEvent::ChatDelta(_) => Self::CHAT_DELTA,
            CoreEvent::ChatMessage(_) => Self::CHAT_MESSAGE,
            CoreEvent::CharacterState(_) => Self::CHARACTER_STATE,
            CoreEvent::Notice(_) => Self::NOTICE,
            CoreEvent::Profile(_) => Self::PROFILE,
            CoreEvent::Settings(_) => Self::SETTINGS,
            CoreEvent::Voice(_) => Self::VOICE,
            CoreEvent::Level(_) => Self::LEVEL,
            CoreEvent::Download(_) => Self::DOWNLOAD,
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        let v = match self {
            CoreEvent::SessionState(p) => serde_json::to_value(p),
            CoreEvent::SessionTick(p) => serde_json::to_value(p),
            CoreEvent::Checkin(p) => serde_json::to_value(p),
            CoreEvent::ChatDelta(p) => serde_json::to_value(p),
            CoreEvent::ChatMessage(p) => serde_json::to_value(p),
            CoreEvent::CharacterState(p) => serde_json::to_value(p),
            CoreEvent::Notice(p) => serde_json::to_value(p),
            CoreEvent::Profile(p) => serde_json::to_value(p),
            CoreEvent::Settings(p) => serde_json::to_value(p),
            CoreEvent::Voice(p) => serde_json::to_value(p),
            CoreEvent::Level(p) => serde_json::to_value(p),
            CoreEvent::Download(p) => serde_json::to_value(p),
        };
        v.unwrap_or(serde_json::Value::Null)
    }
}

/// Where core events go. The desktop app forwards them to the WebView; tests collect them.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);
}

/// Sink that drops everything (headless tools such as the eval runner).
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _event: CoreEvent) {}
}

/// Sink that records events, for tests.
#[derive(Default)]
pub struct CollectingSink {
    events: Mutex<Vec<CoreEvent>>,
}

impl CollectingSink {
    pub fn take(&self) -> Vec<CoreEvent> {
        match self.events.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => Vec::new(),
        }
    }
}

impl EventSink for CollectingSink {
    fn emit(&self, event: CoreEvent) {
        if let Ok(mut g) = self.events.lock() {
            g.push(event);
        }
    }
}
