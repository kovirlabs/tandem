//! Check-in scheduling and signal fusion (SPEC §3.2, §6.5).
//! Biased toward silence: deep focus is precious, so when in doubt we say less.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::session::CheckinMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CheckinReason {
    Interval,
    Halfway,
    FiveMinutesLeft,
    TwoMinutesLeft,
    SessionComplete,
    BreakOver,
    /// User came back after being away (presence signal).
    Returned,
    /// Focus signal suggests the user drifted off-task.
    Drift,
}

impl CheckinReason {
    pub fn describe(&self) -> &'static str {
        match self {
            CheckinReason::Interval => "a regular check-in",
            CheckinReason::Halfway => "halfway through the session",
            CheckinReason::FiveMinutesLeft => "5 minutes left in the session",
            CheckinReason::TwoMinutesLeft => "2 minutes left in the session",
            CheckinReason::SessionComplete => {
                "the focus session just finished; celebrate the effort briefly and ask what got done"
            }
            CheckinReason::BreakOver => "the break just ended; gently offer to start the next session",
            CheckinReason::Returned => "the user just came back after stepping away; welcome them back warmly",
            CheckinReason::Drift => {
                "the user may have drifted off-task; offer, without judgment, to hop back in or switch tasks"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CheckinAction {
    Skip,
    SilentGlance,
    GentleLine,
    Ask,
}

/// Vision contract focus signal (SPEC §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FocusSignal {
    Engaged,
    Uncertain,
    AwayFromTask,
    NotVisible,
}

/// What the core knows right now. `None` means "no signal available".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signals {
    /// Keyboard/mouse activity in the last minute (counts only, never content).
    pub input_active: Option<bool>,
    pub focus: Option<FocusSignal>,
}

/// Pushed on `checkin://prompt`. `text` is `None` when no line was generated; the UI then
/// uses its own fallback copy for the reason (or just animates for silent glances).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct CheckinEvent {
    pub reason: CheckinReason,
    pub action: CheckinAction,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub interval_secs: u32,
    pub time_warnings: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            interval_secs: 600,
            time_warnings: true,
        }
    }
}

/// Which scheduled check-ins already fired in the current session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScheduleState {
    pub last_checkin_elapsed: u32,
    pub halfway: bool,
    pub five_left: bool,
    pub two_left: bool,
}

impl ScheduleState {
    /// After extending, re-arm warnings that are now in the future.
    pub fn on_extend(&mut self, remaining_secs: u32) {
        if remaining_secs > 300 {
            self.five_left = false;
        }
        if remaining_secs > 120 {
            self.two_left = false;
        }
    }
}

/// Is a scheduled check-in due? At most one reason fires per call; marks it as fired.
pub fn due(
    elapsed: u32,
    planned: u32,
    sched: Schedule,
    st: &mut ScheduleState,
) -> Option<CheckinReason> {
    let remaining = planned.saturating_sub(elapsed);
    if remaining == 0 {
        return None; // completion is handled by the engine
    }
    let reason = if sched.time_warnings && planned > 240 && remaining <= 120 && !st.two_left {
        st.two_left = true;
        st.five_left = true;
        CheckinReason::TwoMinutesLeft
    } else if sched.time_warnings && planned > 600 && remaining <= 300 && !st.five_left {
        st.five_left = true;
        CheckinReason::FiveMinutesLeft
    } else if planned >= 600 && elapsed >= planned / 2 && !st.halfway {
        st.halfway = true;
        CheckinReason::Halfway
    } else if sched.interval_secs > 0
        && elapsed.saturating_sub(st.last_checkin_elapsed) >= sched.interval_secs
        && remaining > 180
    {
        CheckinReason::Interval
    } else {
        return None;
    };
    st.last_checkin_elapsed = elapsed;
    Some(reason)
}

/// Decide how (or whether) to check in.
pub fn decide(
    reason: CheckinReason,
    mode: CheckinMode,
    signals: Signals,
    quiet: bool,
) -> CheckinAction {
    let talkative = match mode {
        CheckinMode::Conversational => CheckinAction::Ask,
        CheckinMode::Gentle => CheckinAction::GentleLine,
        CheckinMode::Silent => CheckinAction::SilentGlance,
    };
    match reason {
        // Session/break boundaries always get a line; silence would feel like being ignored.
        CheckinReason::SessionComplete | CheckinReason::BreakOver => {
            if quiet || mode == CheckinMode::Silent {
                CheckinAction::GentleLine
            } else {
                CheckinAction::Ask
            }
        }
        _ if quiet || mode == CheckinMode::Silent => CheckinAction::SilentGlance,
        CheckinReason::Returned => CheckinAction::GentleLine,
        _ => {
            let away = matches!(
                signals.focus,
                Some(FocusSignal::AwayFromTask) | Some(FocusSignal::NotVisible)
            );
            let deep_focus = signals.input_active == Some(true) && !away;
            if deep_focus || signals.focus == Some(FocusSignal::Engaged) {
                return match reason {
                    CheckinReason::Interval => CheckinAction::Skip,
                    _ => CheckinAction::SilentGlance,
                };
            }
            match reason {
                CheckinReason::FiveMinutesLeft | CheckinReason::TwoMinutesLeft => {
                    CheckinAction::GentleLine
                }
                _ => talkative,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CheckinAction as A;
    use CheckinReason as R;

    fn run(planned: u32, sched: Schedule) -> Vec<(u32, R)> {
        let mut st = ScheduleState::default();
        (1..=planned)
            .filter_map(|e| due(e, planned, sched, &mut st).map(|r| (e, r)))
            .collect()
    }

    #[test]
    fn default_25_minute_schedule() {
        let fired = run(1500, Schedule::default());
        assert_eq!(
            fired,
            vec![
                (600, R::Interval),
                (750, R::Halfway),
                (1200, R::FiveMinutesLeft),
                (1380, R::TwoMinutesLeft),
            ]
        );
    }

    #[test]
    fn short_sessions_skip_halfway_and_five_minute_warning() {
        let fired = run(300, Schedule::default());
        assert_eq!(fired, vec![(180, R::TwoMinutesLeft)]);
        assert!(run(
            300,
            Schedule {
                interval_secs: 0,
                time_warnings: false
            }
        )
        .is_empty());
    }

    #[test]
    fn extend_rearms_warnings() {
        let mut st = ScheduleState {
            five_left: true,
            two_left: true,
            ..Default::default()
        };
        st.on_extend(600);
        assert!(!st.five_left && !st.two_left);
        let mut st2 = ScheduleState {
            five_left: true,
            two_left: true,
            ..Default::default()
        };
        st2.on_extend(200);
        assert!(st2.five_left && !st2.two_left);
    }

    #[test]
    fn quiet_and_silent_only_glance() {
        let s = Signals::default();
        for r in [
            R::Interval,
            R::Halfway,
            R::FiveMinutesLeft,
            R::Drift,
            R::Returned,
        ] {
            assert_eq!(decide(r, CheckinMode::Gentle, s, true), A::SilentGlance);
            assert_eq!(decide(r, CheckinMode::Silent, s, false), A::SilentGlance);
        }
        assert_eq!(
            decide(R::SessionComplete, CheckinMode::Silent, s, false),
            A::GentleLine
        );
        assert_eq!(
            decide(R::BreakOver, CheckinMode::Gentle, s, true),
            A::GentleLine
        );
        assert_eq!(
            decide(R::SessionComplete, CheckinMode::Gentle, s, false),
            A::Ask
        );
    }

    #[test]
    fn deep_focus_is_protected() {
        let typing = Signals {
            input_active: Some(true),
            focus: None,
        };
        assert_eq!(
            decide(R::Interval, CheckinMode::Conversational, typing, false),
            A::Skip
        );
        assert_eq!(
            decide(R::Halfway, CheckinMode::Gentle, typing, false),
            A::SilentGlance
        );
        let engaged = Signals {
            input_active: None,
            focus: Some(FocusSignal::Engaged),
        };
        assert_eq!(
            decide(R::Interval, CheckinMode::Gentle, engaged, false),
            A::Skip
        );
        // Typing but camera says away from task: not treated as deep focus.
        let drifting = Signals {
            input_active: Some(true),
            focus: Some(FocusSignal::AwayFromTask),
        };
        assert_eq!(
            decide(R::Interval, CheckinMode::Gentle, drifting, false),
            A::GentleLine
        );
    }

    #[test]
    fn modes_without_signals() {
        let s = Signals::default();
        assert_eq!(
            decide(R::Interval, CheckinMode::Gentle, s, false),
            A::GentleLine
        );
        assert_eq!(
            decide(R::Interval, CheckinMode::Conversational, s, false),
            A::Ask
        );
        assert_eq!(
            decide(R::TwoMinutesLeft, CheckinMode::Conversational, s, false),
            A::GentleLine
        );
        assert_eq!(
            decide(R::Returned, CheckinMode::Conversational, s, false),
            A::GentleLine
        );
        assert_eq!(
            decide(R::Drift, CheckinMode::Conversational, s, false),
            A::Ask
        );
    }
}
