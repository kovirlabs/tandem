//! System prompt assembly (SPEC §6.7): persona + tone + session context + about-me + safety.
//! Prompts are data in `prompts/`; this module only fills in variables.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::tools;
use crate::checkin::{CheckinAction, CheckinReason};
use crate::session::{SessionPhase, SessionSnapshot};

const PERSONA: &str = include_str!("../../../prompts/persona.md");
const SAFETY: &str = include_str!("../../../prompts/safety.md");
const SESSION_CONTEXT: &str = include_str!("../../../prompts/session_context.md");
const CHECKIN: &str = include_str!("../../../prompts/checkin.md");
const TOOL_FALLBACK: &str = include_str!("../../../prompts/tool_fallback.md");
const TONE_CALM: &str = include_str!("../../../prompts/tones/calm.md");
const TONE_CHEERFUL: &str = include_str!("../../../prompts/tones/cheerful.md");
const TONE_DRY: &str = include_str!("../../../prompts/tones/dry_humor.md");
const TONE_MINIMAL: &str = include_str!("../../../prompts/tones/minimal.md");

pub const DEFAULT_COMPANION_NAME: &str = "Timber";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Tone {
    #[default]
    Calm,
    Cheerful,
    DryHumor,
    Minimal,
}

impl Tone {
    fn prompt(&self) -> &'static str {
        match self {
            Tone::Calm => TONE_CALM,
            Tone::Cheerful => TONE_CHEERFUL,
            Tone::DryHumor => TONE_DRY,
            Tone::Minimal => TONE_MINIMAL,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptContext {
    pub companion_name: String,
    pub tone: Tone,
    pub about_me: String,
}

impl Default for PromptContext {
    fn default() -> Self {
        Self {
            companion_name: DEFAULT_COMPANION_NAME.to_string(),
            tone: Tone::default(),
            about_me: String::new(),
        }
    }
}

/// Replace `{{key}}` placeholders. Values are inserted verbatim (never re-scanned).
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let key = after[..end].trim();
                match vars.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn minutes(secs: u32) -> u32 {
    secs.div_ceil(60)
}

pub fn session_summary(s: &SessionSnapshot) -> String {
    let task = s.task.as_deref().unwrap_or("(no task)");
    let mut out = match s.phase {
        SessionPhase::Idle => "No focus session is running. You're both just hanging out.".to_string(),
        SessionPhase::Focusing => format!(
            "The user is in a focus session on \"{task}\": {} of {} minutes left. Check-in style: {}.",
            minutes(s.remaining_secs),
            minutes(s.planned_secs),
            s.checkin_mode.as_str()
        ),
        SessionPhase::PausedManual => format!("The session on \"{task}\" is paused."),
        SessionPhase::PausedAway => {
            format!("The session on \"{task}\" is paused because the user stepped away.")
        }
        SessionPhase::WrappingUp => format!("The focus session on \"{task}\" just ended."),
        SessionPhase::Break => format!(
            "The user is on a break ({} minutes left).",
            minutes(s.break_remaining_secs)
        ),
    };
    if !s.subtasks.is_empty() {
        out.push_str("\nChecklist:");
        for (i, t) in s.subtasks.iter().enumerate() {
            let mark = if t.done { "x" } else { " " };
            out.push_str(&format!("\n[{mark}] {}. {} (id {})", i + 1, t.title, t.id));
        }
    }
    if s.quiet_remaining_secs > 0 {
        out.push_str(&format!(
            "\nThe user asked for quiet for {} more minutes. Only speak if they talk to you.",
            minutes(s.quiet_remaining_secs)
        ));
    }
    out
}

/// Full system prompt. The safety block is always last and always included.
pub fn system_prompt(
    ctx: &PromptContext,
    session: &SessionSnapshot,
    tool_fallback: bool,
) -> String {
    let name = ctx.companion_name.trim();
    let name = if name.is_empty() {
        DEFAULT_COMPANION_NAME
    } else {
        name
    };
    let vars = [("companion_name", name)];
    let mut parts = vec![
        render(PERSONA, &vars),
        ctx.tone.prompt().trim().to_string(),
        render(
            SESSION_CONTEXT,
            &[("session_summary", &session_summary(session))],
        ),
    ];
    let about = ctx.about_me.trim();
    if !about.is_empty() {
        parts.push(format!(
            "Notes the user wrote about themselves (use gently, don't quote back):\n{about}"
        ));
    }
    if tool_fallback {
        let tools_json = serde_json::to_string_pretty(tools::specs()).unwrap_or_default();
        parts.push(render(TOOL_FALLBACK, &[("tools_json", &tools_json)]));
    }
    parts.push(render(SAFETY, &vars));
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn checkin_cue(reason: CheckinReason, action: CheckinAction) -> String {
    let style = match action {
        CheckinAction::Ask => "ask",
        _ => "gentle",
    };
    render(
        CHECKIN,
        &[
            ("checkin_reason", reason.describe()),
            ("checkin_style", style),
        ],
    )
    .trim()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{CheckinMode, Subtask};

    fn snap(phase: SessionPhase) -> SessionSnapshot {
        SessionSnapshot {
            phase,
            session_id: None,
            task: Some("expense report".into()),
            planned_secs: 1500,
            elapsed_secs: 300,
            remaining_secs: 1200,
            break_planned_secs: 0,
            break_remaining_secs: 0,
            checkin_mode: CheckinMode::Gentle,
            subtasks: vec![Subtask {
                id: "s1".into(),
                title: "find receipts".into(),
                done: false,
            }],
            quiet_remaining_secs: 0,
            camera_enabled: false,
            cloud_vision: false,
            progress: 0.2,
        }
    }

    #[test]
    fn render_handles_missing_and_unclosed_placeholders() {
        assert_eq!(render("Hi {{name}}!", &[("name", "Ash")]), "Hi Ash!");
        assert_eq!(render("{{ name }}", &[("name", "Ash")]), "Ash");
        assert_eq!(render("{{other}}", &[]), "{{other}}");
        assert_eq!(render("open {{", &[]), "open {{");
        // Values are not re-scanned for placeholders.
        assert_eq!(render("{{a}}", &[("a", "{{a}}")]), "{{a}}");
    }

    #[test]
    fn system_prompt_uses_name_and_always_ends_with_safety() {
        let ctx = PromptContext {
            companion_name: "Maple".into(),
            tone: Tone::Minimal,
            about_me: "I like tiny steps.".into(),
        };
        let p = system_prompt(&ctx, &snap(SessionPhase::Focusing), false);
        assert!(p.starts_with("You are Maple"));
        assert!(!p.contains("Timber"));
        assert!(!p.contains("{{"));
        assert!(p.contains("expense report"));
        assert!(p.contains("tiny steps"));
        assert!(p
            .trim_end()
            .ends_with(SAFETY.lines().last().unwrap_or("").trim()));
        assert!(p.contains("988"));
    }

    #[test]
    fn blank_name_falls_back_to_default() {
        let ctx = PromptContext {
            companion_name: "  ".into(),
            ..Default::default()
        };
        assert!(system_prompt(&ctx, &snap(SessionPhase::Idle), false).starts_with("You are Timber"));
    }

    #[test]
    fn fallback_includes_tool_schemas() {
        let p = system_prompt(&PromptContext::default(), &snap(SessionPhase::Idle), true);
        assert!(p.contains("<tool>"));
        assert!(p.contains("start_session"));
    }

    #[test]
    fn summaries_for_each_phase() {
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Focusing,
            SessionPhase::PausedManual,
            SessionPhase::PausedAway,
            SessionPhase::WrappingUp,
            SessionPhase::Break,
        ] {
            assert!(!session_summary(&snap(phase)).is_empty());
        }
        let mut s = snap(SessionPhase::Focusing);
        s.quiet_remaining_secs = 600;
        assert!(session_summary(&s).contains("quiet for 10"));
    }

    #[test]
    fn checkin_cue_mentions_reason() {
        let c = checkin_cue(CheckinReason::Halfway, CheckinAction::Ask);
        assert!(c.contains("halfway"));
        assert!(c.contains("\"ask\""));
    }
}
