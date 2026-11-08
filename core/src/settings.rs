//! User settings (non-secret) and profile. Persisted as JSON in SQLite.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::conversation::prompt::{Tone, DEFAULT_COMPANION_NAME};
use crate::error::{CoreError, Result};
use crate::providers::anthropic;
use crate::session::engine::{MAX_MINUTES, MIN_MINUTES};
use crate::session::CheckinMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderKind {
    #[default]
    Ollama,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct OllamaSettings {
    pub url: String,
    /// Empty = pick the first installed model.
    pub chat_model: String,
    /// Empty = camera awareness unavailable (M3).
    pub vision_model: String,
    /// Hosts the user confirmed they control; treated as local for privacy rules.
    pub confirmed_hosts: Vec<String>,
}

impl Default for OllamaSettings {
    fn default() -> Self {
        Self {
            url: "http://localhost:11434".into(),
            chat_model: String::new(),
            vision_model: String::new(),
            confirmed_hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct AnthropicSettings {
    pub model: String,
    /// `output_config.effort`: low | medium | high.
    pub effort: String,
}

impl Default for AnthropicSettings {
    fn default() -> Self {
        Self {
            model: anthropic::DEFAULT_MODEL.into(),
            effort: "low".into(),
        }
    }
}

/// The desktop buddy: the companion in a small always-on-top window (docs/ROADMAP.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct BuddySettings {
    pub enabled: bool,
    /// Last position in physical pixels; `None` = default corner.
    pub x: Option<i32>,
    pub y: Option<i32>,
}

impl Default for BuddySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            x: None,
            y: None,
        }
    }
}

/// When the companion speaks its replies out loud (M2 voice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SpeakReplies {
    /// Only replies to something the user said by voice (default).
    #[default]
    VoiceTurns,
    /// Every reply, typed or spoken.
    Always,
    Never,
}

/// Voice settings (M2). The chosen voice lives in `profile.companion.voice_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct VoiceSettings {
    /// Master switch for the mic and speech. Off until the user sets voice up.
    pub enabled: bool,
    /// Mute all companion speech (captions still show).
    pub mute: bool,
    pub speak_replies: SpeakReplies,
    /// Speak gentle check-in lines out loud.
    pub speak_checkins: bool,
    /// whisper.cpp model name, e.g. `base.en`.
    pub stt_model: String,
    /// 0.5–2.0, 1.0 = normal.
    pub speech_rate: f32,
    /// 0.0–1.0.
    pub volume: f32,
    /// Global push-to-talk shortcut (Tauri accelerator syntax).
    pub ptt_shortcut: String,
    /// Part C: listen continuously and use voice-activity detection.
    pub hands_free: bool,
    /// Part C: in hands-free mode, only respond after "Hey {{companion_name}}".
    pub wake_phrase: bool,
    /// Part C: stop speaking as soon as the user starts talking.
    pub barge_in: bool,
    /// Input device name; empty = system default.
    pub input_device: String,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mute: false,
            speak_replies: SpeakReplies::VoiceTurns,
            speak_checkins: true,
            stt_model: "base.en".into(),
            speech_rate: 1.0,
            volume: 0.9,
            ptt_shortcut: "CommandOrControl+Shift+Space".into(),
            hands_free: false,
            wake_phrase: false,
            barge_in: true,
            input_device: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Settings {
    pub chat_provider: ProviderKind,
    pub ollama: OllamaSettings,
    pub anthropic: AnthropicSettings,
    pub default_minutes: u32,
    pub checkin_mode: CheckinMode,
    pub checkin_interval_min: u32,
    pub time_warnings: bool,
    pub show_timer: bool,
    pub short_break_min: u32,
    pub long_break_min: u32,
    pub message_retention_days: u32,
    /// Clearly-labelled debug option; off by default (CLAUDE.md rule 1).
    pub debug_save_snapshots: bool,
    pub keep_focus_signals: bool,
    pub buddy: BuddySettings,
    pub voice: VoiceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            chat_provider: ProviderKind::Ollama,
            ollama: OllamaSettings::default(),
            anthropic: AnthropicSettings::default(),
            default_minutes: 25,
            checkin_mode: CheckinMode::Gentle,
            checkin_interval_min: 10,
            time_warnings: true,
            show_timer: true,
            short_break_min: 5,
            long_break_min: 15,
            message_retention_days: 30,
            debug_save_snapshots: false,
            keep_focus_signals: false,
            buddy: BuddySettings::default(),
            voice: VoiceSettings::default(),
        }
    }
}

impl Settings {
    /// Clamp everything into the ranges the spec allows.
    pub fn normalised(mut self) -> Self {
        self.default_minutes = self.default_minutes.clamp(MIN_MINUTES, MAX_MINUTES);
        self.checkin_interval_min = self.checkin_interval_min.clamp(1, 60);
        self.short_break_min = self.short_break_min.clamp(1, 30);
        self.long_break_min = self.long_break_min.clamp(5, 60);
        self.message_retention_days = self.message_retention_days.clamp(1, 365);
        self.ollama.url = self.ollama.url.trim().trim_end_matches('/').to_string();
        if self.ollama.url.is_empty() {
            self.ollama.url = OllamaSettings::default().url;
        }
        self.ollama.confirmed_hosts = self
            .ollama
            .confirmed_hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        if !matches!(self.anthropic.effort.as_str(), "low" | "medium" | "high") {
            self.anthropic.effort = "low".into();
        }
        if self.anthropic.model.trim().is_empty() {
            self.anthropic.model = anthropic::DEFAULT_MODEL.into();
        }
        let rate = self.voice.speech_rate;
        self.voice.speech_rate = if rate.is_finite() {
            rate.clamp(0.5, 2.0)
        } else {
            1.0
        };
        let vol = self.voice.volume;
        self.voice.volume = if vol.is_finite() {
            vol.clamp(0.0, 1.0)
        } else {
            0.9
        };
        if self.voice.stt_model.trim().is_empty() {
            self.voice.stt_model = VoiceSettings::default().stt_model;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct CompanionProfile {
    pub name: String,
    pub character_id: String,
    pub voice_id: Option<String>,
    /// `appearance.<character_id>` → customization values (SPEC §6.6).
    #[ts(type = "Record<string, unknown>")]
    pub appearance: serde_json::Value,
}

impl Default for CompanionProfile {
    fn default() -> Self {
        Self {
            name: DEFAULT_COMPANION_NAME.into(),
            character_id: "beaver".into(),
            voice_id: None,
            appearance: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Profile {
    pub companion: CompanionProfile,
    pub tone: Tone,
    pub about_me: String,
    pub onboarded: bool,
}

pub const MAX_NAME_CHARS: usize = 20;

impl Profile {
    pub fn validated(mut self) -> Result<Self> {
        let name = self.companion.name.trim().to_string();
        let n = name.chars().count();
        if n == 0 || n > MAX_NAME_CHARS {
            return Err(CoreError::Invalid(format!(
                "the companion's name needs 1 to {MAX_NAME_CHARS} characters"
            )));
        }
        self.companion.name = name;
        if self.about_me.chars().count() > 2000 {
            self.about_me = self.about_me.chars().take(2000).collect();
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_clamps() {
        let s = Settings {
            default_minutes: 500,
            checkin_interval_min: 0,
            ollama: OllamaSettings {
                url: " http://Box:11434/ ".into(),
                confirmed_hosts: vec![" GPU.lan ".into(), "".into()],
                ..Default::default()
            },
            anthropic: AnthropicSettings {
                model: "".into(),
                effort: "turbo".into(),
            },
            ..Default::default()
        }
        .normalised();
        assert_eq!(s.default_minutes, 120);
        assert_eq!(s.checkin_interval_min, 1);
        assert_eq!(s.ollama.url, "http://Box:11434");
        assert_eq!(s.ollama.confirmed_hosts, vec!["gpu.lan".to_string()]);
        assert_eq!(s.anthropic.effort, "low");
        assert_eq!(s.anthropic.model, anthropic::DEFAULT_MODEL);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let s: Settings = serde_json::from_str(r#"{"default_minutes": 30}"#).unwrap_or_default();
        assert_eq!(s.default_minutes, 30);
        assert_eq!(s.ollama.url, "http://localhost:11434");
    }

    #[test]
    fn name_validation() {
        let mut p = Profile::default();
        p.companion.name = "  Mapleleaf  ".into();
        assert_eq!(
            p.clone().validated().map(|p| p.companion.name).ok(),
            Some("Mapleleaf".into())
        );
        p.companion.name = "ビーバーさん".into(); // any language
        assert!(p.clone().validated().is_ok());
        p.companion.name = " ".into();
        assert!(p.clone().validated().is_err());
        p.companion.name = "x".repeat(21);
        assert!(p.validated().is_err());
    }
}
