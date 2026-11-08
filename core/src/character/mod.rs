//! Companion character states. The UI maps these to Rive state-machine inputs
//! (`ui/src/character/stateMap.ts`); packs must provide every one of them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::session::SessionPhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CharacterState {
    Idle,
    Working,
    Listening,
    Thinking,
    Talking,
    Waiting,
    Celebrating,
    Break,
    Sleeping,
}

impl CharacterState {
    pub const ALL: [CharacterState; 9] = [
        CharacterState::Idle,
        CharacterState::Working,
        CharacterState::Listening,
        CharacterState::Thinking,
        CharacterState::Talking,
        CharacterState::Waiting,
        CharacterState::Celebrating,
        CharacterState::Break,
        CharacterState::Sleeping,
    ];

    /// The resting state for a session phase, used when no conversation overlay is active.
    pub fn for_phase(phase: SessionPhase) -> Self {
        match phase {
            SessionPhase::Idle => CharacterState::Idle,
            SessionPhase::Focusing => CharacterState::Working,
            SessionPhase::PausedManual => CharacterState::Idle,
            SessionPhase::PausedAway => CharacterState::Waiting,
            SessionPhase::WrappingUp => CharacterState::Celebrating,
            SessionPhase::Break => CharacterState::Break,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CharacterStateEvent {
    pub state: CharacterState,
}
