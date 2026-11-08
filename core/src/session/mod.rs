//! Session model and finite-state machine (SPEC §5.3). Timers live only here in the core.

pub mod engine;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionPhase {
    Idle,
    Focusing,
    PausedManual,
    PausedAway,
    WrappingUp,
    Break,
}

impl SessionPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionPhase::Idle => "idle",
            SessionPhase::Focusing => "focusing",
            SessionPhase::PausedManual => "paused",
            SessionPhase::PausedAway => "paused (away)",
            SessionPhase::WrappingUp => "wrapping up",
            SessionPhase::Break => "on a break",
        }
    }

    /// A focus session exists (running, paused, or just finished).
    pub fn has_session(&self) -> bool {
        matches!(
            self,
            SessionPhase::Focusing | SessionPhase::PausedManual | SessionPhase::PausedAway
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CheckinMode {
    Silent,
    #[default]
    Gentle,
    Conversational,
}

impl CheckinMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckinMode::Silent => "silent",
            CheckinMode::Gentle => "gentle",
            CheckinMode::Conversational => "conversational",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub done: bool,
}

/// Full session state pushed on `session://state`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct SessionSnapshot {
    pub phase: SessionPhase,
    pub session_id: Option<String>,
    pub task: Option<String>,
    pub planned_secs: u32,
    pub elapsed_secs: u32,
    pub remaining_secs: u32,
    pub break_planned_secs: u32,
    pub break_remaining_secs: u32,
    pub checkin_mode: CheckinMode,
    pub subtasks: Vec<Subtask>,
    pub quiet_remaining_secs: u32,
    pub camera_enabled: bool,
    pub cloud_vision: bool,
    /// 0.0–1.0 through the focus block (drives the beaver's dam growth).
    pub progress: f32,
}

/// Lightweight once-per-second update pushed on `session://tick`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct SessionTick {
    pub phase: SessionPhase,
    pub elapsed_secs: u32,
    pub remaining_secs: u32,
    pub break_remaining_secs: u32,
    pub quiet_remaining_secs: u32,
    pub progress: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Start,
    Pause,
    Resume,
    Away,
    Back,
    Finish,
    StartBreak,
    Close,
}

impl Transition {
    fn verb(&self) -> &'static str {
        match self {
            Transition::Start => "start a session",
            Transition::Pause => "pause",
            Transition::Resume => "resume",
            Transition::Away => "mark away",
            Transition::Back => "mark back",
            Transition::Finish => "end the session",
            Transition::StartBreak => "start a break",
            Transition::Close => "close",
        }
    }
}

/// Pure FSM: the next phase for a transition, or an error if it isn't allowed.
pub fn next_phase(phase: SessionPhase, t: Transition) -> Result<SessionPhase, CoreError> {
    use SessionPhase as P;
    use Transition as T;
    let next = match (phase, t) {
        (P::Idle | P::WrappingUp | P::Break, T::Start) => P::Focusing,
        (P::Focusing, T::Pause) => P::PausedManual,
        (P::PausedManual | P::PausedAway, T::Resume) => P::Focusing,
        (P::Focusing, T::Away) => P::PausedAway,
        // Walking away while manually paused keeps the manual pause.
        (P::PausedManual, T::Away) => P::PausedManual,
        (P::PausedAway, T::Back) => P::Focusing,
        (P::PausedManual, T::Back) => P::PausedManual,
        (P::Focusing | P::PausedManual | P::PausedAway, T::Finish) => P::WrappingUp,
        (P::Idle | P::WrappingUp, T::StartBreak) => P::Break,
        (P::WrappingUp | P::Break, T::Close) => P::Idle,
        _ => {
            return Err(CoreError::InvalidTransition {
                action: t.verb().to_string(),
                phase: phase.as_str().to_string(),
            })
        }
    };
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionPhase as P;
    use Transition as T;

    #[test]
    fn happy_path() {
        let mut p = P::Idle;
        for (t, want) in [
            (T::Start, P::Focusing),
            (T::Pause, P::PausedManual),
            (T::Resume, P::Focusing),
            (T::Away, P::PausedAway),
            (T::Back, P::Focusing),
            (T::Finish, P::WrappingUp),
            (T::StartBreak, P::Break),
            (T::Start, P::Focusing),
            (T::Finish, P::WrappingUp),
            (T::Close, P::Idle),
        ] {
            p = next_phase(p, t).unwrap_or(P::Idle);
            assert_eq!(p, want, "{t:?}");
        }
    }

    #[test]
    fn manual_pause_survives_away_and_back() {
        assert_eq!(
            next_phase(P::PausedManual, T::Away).ok(),
            Some(P::PausedManual)
        );
        assert_eq!(
            next_phase(P::PausedManual, T::Back).ok(),
            Some(P::PausedManual)
        );
        assert_eq!(next_phase(P::PausedAway, T::Resume).ok(), Some(P::Focusing));
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        for (p, t) in [
            (P::Focusing, T::Start),
            (P::Idle, T::Pause),
            (P::Idle, T::Finish),
            (P::Focusing, T::StartBreak),
            (P::Break, T::Pause),
            (P::Idle, T::Close),
            (P::Focusing, T::Close),
            (P::Idle, T::Back),
        ] {
            assert!(next_phase(p, t).is_err(), "{p:?} {t:?}");
        }
    }

    #[test]
    fn has_session() {
        assert!(P::Focusing.has_session());
        assert!(P::PausedAway.has_session());
        assert!(!P::WrappingUp.has_session());
        assert!(!P::Idle.has_session());
    }
}
