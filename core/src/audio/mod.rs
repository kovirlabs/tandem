//! Voice (M2): microphone → speech-to-text → conversation → text-to-speech → speaker.
//!
//! Plan: `docs/superpowers/plans/2026-11-08-m2-voice.md`.
//!
//! Ownership (parallel build, do not edit files you don't own):
//! - **input**  — `capture.rs`, `stt.rs`, `vad.rs`, `wake.rs`, `controller.rs`
//! - **output** — `sentences.rs`, `tts.rs`, `playback.rs`, `speaker.rs`
//! - **shared** (lead) — this file, `models.rs`, `tap.rs`
//!
//! Privacy (CLAUDE.md rules 1, 2, 9, 10): audio lives in memory only and is never written to
//! disk or logged. `AudioClip` is deliberately not `Serialize` and its `Debug` prints no samples.
//! Any STT/TTS request to a non-local provider must pass `router::privacy::check(Modality::Audio, …)`.

pub mod capture;
pub mod controller;
pub mod models;
pub mod playback;
pub mod sentences;
pub mod speaker;
pub mod stt;
pub mod tap;
pub mod tts;
pub mod vad;
pub mod wake;

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::events::CoreEvent;
use crate::providers::Locality;

/// Sample rate whisper.cpp and Silero VAD expect.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Mono PCM in memory. Never serialised, never logged, never written to disk.
#[derive(Clone, Default)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioClip {
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.samples.len() as f32 / self.sample_rate as f32
        }
    }
}

impl fmt::Debug for AudioClip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AudioClip(<{} samples @ {} Hz, redacted>)",
            self.samples.len(),
            self.sample_rate
        )
    }
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("no microphone is available")]
    NoInputDevice,
    #[error("no speaker is available")]
    NoOutputDevice,
    #[error("microphone permission was denied")]
    PermissionDenied,
    #[error("the {0} model isn't downloaded yet")]
    ModelMissing(String),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("speech-to-text failed: {0}")]
    Stt(String),
    #[error("text-to-speech failed: {0}")]
    Tts(String),
    #[error("audio device error: {0}")]
    Device(String),
    #[error("voice is turned off")]
    Disabled,
    #[error(transparent)]
    Privacy(#[from] crate::router::privacy::PrivacyError),
}

impl From<AudioError> for crate::error::CoreError {
    fn from(e: AudioError) -> Self {
        match e {
            AudioError::Privacy(p) => crate::error::CoreError::Privacy(p),
            other => crate::error::CoreError::Invalid(other.to_string()),
        }
    }
}

/// Speech-to-text engine (SPEC §6.2).
#[async_trait]
pub trait SttProvider: Send + Sync {
    fn locality(&self) -> Locality;
    /// Transcribe one utterance. Returns trimmed text (may be empty for silence/noise).
    async fn transcribe(&self, clip: AudioClip) -> Result<String, AudioError>;
}

/// Options for one synthesis call.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakOpts {
    /// Voice id, e.g. `en_US-lessac-medium`.
    pub voice: String,
    /// 0.5–2.0, 1.0 = normal.
    pub rate: f32,
}

/// Text-to-speech engine (SPEC §6.2). Called once per sentence; streaming happens at
/// sentence granularity in `speaker.rs`.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn locality(&self) -> Locality;
    async fn synthesize(&self, text: &str, opts: &SpeakOpts) -> Result<AudioClip, AudioError>;
}

/// The output pipeline as seen by the rest of the core. Implemented by
/// `speaker::SpeakerPipeline` (output owner); called by `controller.rs` (input owner) and `tap.rs`.
pub trait Speaker: Send + Sync {
    /// Called for every core event before it reaches the UI. Must be cheap and never block:
    /// filter for `ChatDelta` / `ChatMessage` and queue work.
    fn observe(&self, event: &CoreEvent);
    /// Speak the next assistant reply (the controller calls this right before a voice turn).
    fn arm_next_reply(&self);
    /// Stop now and drop everything queued (barge-in, PTT pressed, mute).
    fn stop(&self);
    fn is_speaking(&self) -> bool;
    /// The selected voice is downloaded and loadable.
    fn ready(&self) -> bool;
}

/// What the voice pipeline is doing, pushed on `voice://state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VoicePhase {
    /// Voice turned off in settings, or required models missing.
    #[default]
    Off,
    /// Ready; waiting for push-to-talk (or for speech in hands-free mode).
    Ready,
    /// Recording the user's speech.
    Listening,
    /// Turning speech into text.
    Transcribing,
    /// Waiting for the companion's reply.
    Thinking,
    /// Playing the companion's speech.
    Speaking,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, TS)]
#[ts(export)]
pub struct VoiceStatus {
    pub phase: VoicePhase,
    pub hands_free: bool,
    /// The speech-to-text model is downloaded and loaded.
    pub stt_ready: bool,
    /// The selected voice is downloaded and loaded.
    pub tts_ready: bool,
    /// i18n key for a problem to show (e.g. `voice.error.no_mic`), if any.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LevelSource {
    Mic,
    Speaker,
}

/// Loudness for mouth movement (speaker) and the mic meter (mic), ~30 per second, on `voice://level`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AudioLevel {
    pub source: LevelSource,
    /// 0.0–1.0, already smoothed.
    pub level: f32,
}

/// Model download progress, on `voice://download`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct DownloadProgress {
    /// Model id, e.g. `whisper:base.en` or `piper:en_US-lessac-medium`.
    pub model: String,
    #[ts(type = "number")]
    pub done_bytes: u64,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    pub finished: bool,
    pub error: Option<String>,
}

/// One selectable voice for the picker.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct VoiceInfo {
    /// e.g. `en_US-lessac-medium`.
    pub id: String,
    /// Display label without gender or ethnicity words, e.g. "Lessac (US English, medium)".
    pub label: String,
    pub language: String,
    /// SPDX-ish license of the voice model, from its model card.
    pub license: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub downloaded: bool,
    /// The default voice chosen by listening test.
    pub is_default: bool,
}

/// One speech-to-text model for the picker.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct SttModelInfo {
    /// whisper.cpp name, e.g. `base.en`.
    pub id: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub downloaded: bool,
}
