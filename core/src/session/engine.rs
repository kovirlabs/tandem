//! The session engine: owns the timers, the FSM, subtasks, quiet mode, and the
//! companion's resting animation state. Never holds a lock across an `.await`.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::json;
use tokio::time::Instant;

use super::{
    next_phase, CheckinMode, SessionPhase, SessionSnapshot, SessionTick, Subtask, Transition,
};
use crate::character::{CharacterState, CharacterStateEvent};
use crate::checkin::{self, CheckinReason, Schedule, ScheduleState, Signals};
use crate::conversation::tools::Tool;
use crate::error::{CoreError, Result};
use crate::events::{CoreEvent, EventSink};
use crate::store::Store;

pub const MIN_MINUTES: u32 = 5;
pub const MAX_MINUTES: u32 = 120;

/// Something the app layer must react to after a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineOutput {
    Checkin(CheckinReason),
    SessionComplete,
    BreakOver,
}

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub default_minutes: u32,
    pub default_checkin_mode: CheckinMode,
    pub schedule: Schedule,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            default_minutes: 25,
            default_checkin_mode: CheckinMode::Gentle,
            schedule: Schedule::default(),
        }
    }
}

struct State {
    phase: SessionPhase,
    session_id: Option<String>,
    task_id: Option<String>,
    task: Option<String>,
    planned_secs: u32,
    elapsed_before: Duration,
    running_since: Option<Instant>,
    break_planned_secs: u32,
    break_started: Option<Instant>,
    checkin_mode: CheckinMode,
    subtasks: Vec<Subtask>,
    quiet_until: Option<Instant>,
    camera_enabled: bool,
    cloud_vision: bool,
    sched: ScheduleState,
    signals: Signals,
    overlay: Option<CharacterState>,
    last_character: Option<CharacterState>,
    config: EngineConfig,
}

impl State {
    fn elapsed(&self, now: Instant) -> u32 {
        let running = self.running_since.map(|s| now - s).unwrap_or_default();
        let secs = (self.elapsed_before + running).as_secs();
        u32::try_from(secs)
            .unwrap_or(u32::MAX)
            .min(self.planned_secs)
    }

    fn break_remaining(&self, now: Instant) -> u32 {
        match self.break_started {
            Some(s) => {
                let used = u32::try_from((now - s).as_secs()).unwrap_or(u32::MAX);
                self.break_planned_secs.saturating_sub(used)
            }
            None => 0,
        }
    }

    fn quiet_remaining(&self, now: Instant) -> u32 {
        self.quiet_until
            .map(|q| u32::try_from(q.saturating_duration_since(now).as_secs()).unwrap_or(0))
            .unwrap_or(0)
    }

    fn progress(&self, now: Instant) -> f32 {
        if self.planned_secs == 0 {
            0.0
        } else {
            self.elapsed(now) as f32 / self.planned_secs as f32
        }
    }

    fn snapshot(&self, now: Instant) -> SessionSnapshot {
        let elapsed = self.elapsed(now);
        SessionSnapshot {
            phase: self.phase,
            session_id: self.session_id.clone(),
            task: self.task.clone(),
            planned_secs: self.planned_secs,
            elapsed_secs: elapsed,
            remaining_secs: self.planned_secs.saturating_sub(elapsed),
            break_planned_secs: self.break_planned_secs,
            break_remaining_secs: self.break_remaining(now),
            checkin_mode: self.checkin_mode,
            subtasks: self.subtasks.clone(),
            quiet_remaining_secs: self.quiet_remaining(now),
            camera_enabled: self.camera_enabled,
            cloud_vision: self.cloud_vision,
            progress: self.progress(now),
        }
    }

    fn tick(&self, now: Instant) -> SessionTick {
        let elapsed = self.elapsed(now);
        SessionTick {
            phase: self.phase,
            elapsed_secs: elapsed,
            remaining_secs: self.planned_secs.saturating_sub(elapsed),
            break_remaining_secs: self.break_remaining(now),
            quiet_remaining_secs: self.quiet_remaining(now),
            progress: self.progress(now),
        }
    }

    fn transition(&mut self, t: Transition, now: Instant) -> Result<()> {
        let next = next_phase(self.phase, t)?;
        // Bank running time whenever we leave Focusing; start the clock when entering it.
        if self.phase == SessionPhase::Focusing && next != SessionPhase::Focusing {
            if let Some(s) = self.running_since.take() {
                self.elapsed_before += now - s;
            }
        }
        if next == SessionPhase::Focusing && self.running_since.is_none() {
            self.running_since = Some(now);
        }
        if self.phase == SessionPhase::Break && next != SessionPhase::Break {
            self.break_started = None;
            self.break_planned_secs = 0;
        }
        self.phase = next;
        Ok(())
    }

    fn character(&self) -> CharacterState {
        self.overlay
            .unwrap_or_else(|| CharacterState::for_phase(self.phase))
    }
}

pub struct SessionEngine {
    st: Mutex<State>,
    sink: Arc<dyn EventSink>,
    store: Option<Arc<Store>>,
}

impl SessionEngine {
    pub fn new(sink: Arc<dyn EventSink>, store: Option<Arc<Store>>, config: EngineConfig) -> Self {
        Self {
            st: Mutex::new(State {
                phase: SessionPhase::Idle,
                session_id: None,
                task_id: None,
                task: None,
                planned_secs: 0,
                elapsed_before: Duration::ZERO,
                running_since: None,
                break_planned_secs: 0,
                break_started: None,
                checkin_mode: config.default_checkin_mode,
                subtasks: Vec::new(),
                quiet_until: None,
                camera_enabled: false,
                cloud_vision: false,
                sched: ScheduleState::default(),
                signals: Signals::default(),
                overlay: None,
                last_character: None,
                config,
            }),
            sink,
            store,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned lock means a panic mid-update; the state is still the best we have.
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn log_event(&self, session_id: Option<&str>, kind: &str, payload: serde_json::Value) {
        if let (Some(store), Some(id)) = (&self.store, session_id) {
            if let Err(e) = store.add_event(id, kind, &payload) {
                tracing::warn!("failed to record session event: {e}");
            }
        }
    }

    /// Emit state + character after a mutation. Takes the guard to drop it before emitting.
    fn publish(&self, mut st: MutexGuard<'_, State>) -> SessionSnapshot {
        let now = Instant::now();
        let snap = st.snapshot(now);
        let character = st.character();
        let character_changed = st.last_character != Some(character);
        st.last_character = Some(character);
        drop(st);
        self.sink.emit(CoreEvent::SessionState(snap.clone()));
        if character_changed {
            self.sink
                .emit(CoreEvent::CharacterState(CharacterStateEvent {
                    state: character,
                }));
        }
        snap
    }

    pub fn set_config(&self, config: EngineConfig) {
        self.lock().config = config;
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        self.lock().snapshot(Instant::now())
    }

    pub fn character_state(&self) -> CharacterState {
        self.lock().character()
    }

    pub fn phase(&self) -> SessionPhase {
        self.lock().phase
    }

    pub fn quiet(&self) -> bool {
        self.lock().quiet_remaining(Instant::now()) > 0
    }

    pub fn checkin_mode(&self) -> CheckinMode {
        self.lock().checkin_mode
    }

    pub fn signals(&self) -> Signals {
        self.lock().signals
    }

    pub fn set_signals(&self, signals: Signals) {
        self.lock().signals = signals;
    }

    pub fn session_id(&self) -> Option<String> {
        self.lock().session_id.clone()
    }

    /// Temporary conversational animation (listening/thinking/talking). `None` restores the phase state.
    pub fn set_overlay(&self, overlay: Option<CharacterState>) {
        let mut st = self.lock();
        st.overlay = overlay;
        let character = st.character();
        if st.last_character == Some(character) {
            return;
        }
        st.last_character = Some(character);
        drop(st);
        self.sink
            .emit(CoreEvent::CharacterState(CharacterStateEvent {
                state: character,
            }));
    }

    pub fn start(
        &self,
        task: &str,
        minutes: Option<u32>,
        mode: Option<CheckinMode>,
    ) -> Result<SessionSnapshot> {
        let task = task.trim();
        if task.is_empty() {
            return Err(CoreError::Invalid("a session needs a task name".into()));
        }
        // Switching tasks mid-session wraps up the current one first.
        if self.phase().has_session() {
            self.end(None)?;
        }
        let now = Instant::now();
        let mut st = self.lock();
        let minutes = minutes
            .unwrap_or(st.config.default_minutes)
            .clamp(MIN_MINUTES, MAX_MINUTES);
        let mode = mode.unwrap_or(st.config.default_checkin_mode);
        let same_task = st.task.as_deref() == Some(task);
        st.transition(Transition::Start, now)?;
        // Subtasks added before any task existed (or for the same task) carry over.
        let carry_subtasks = same_task || st.task_id.is_none();
        if !carry_subtasks {
            st.subtasks.clear();
        }
        st.planned_secs = minutes * 60;
        st.elapsed_before = Duration::ZERO;
        st.running_since = Some(now);
        st.checkin_mode = mode;
        st.sched = ScheduleState::default();
        st.task = Some(task.to_string());
        if let Some(store) = &self.store {
            let task_id = match (&st.task_id, same_task) {
                (Some(id), true) => Ok(id.clone()),
                _ => store.create_task(task),
            };
            match task_id {
                Ok(task_id) => {
                    if !same_task {
                        for (i, s) in st.subtasks.iter_mut().enumerate() {
                            match store.add_subtask(&task_id, &s.title, i as i64) {
                                Ok(id) => s.id = id,
                                Err(e) => tracing::warn!("failed to save subtask: {e}"),
                            }
                        }
                    }
                    let session = store.create_session(
                        &task_id,
                        minutes,
                        mode.as_str(),
                        st.camera_enabled,
                        st.cloud_vision,
                    );
                    st.session_id = session.map_err(|e| tracing::warn!("{e}")).ok();
                    st.task_id = Some(task_id);
                }
                Err(e) => tracing::warn!("failed to save task: {e}"),
            }
        } else {
            st.session_id = Some(uuid::Uuid::new_v4().to_string());
            st.task_id = Some(uuid::Uuid::new_v4().to_string());
        }
        let sid = st.session_id.clone();
        self.log_event(sid.as_deref(), "start", json!({ "minutes": minutes }));
        Ok(self.publish(st))
    }

    fn simple(&self, t: Transition, kind: &str) -> Result<SessionSnapshot> {
        let mut st = self.lock();
        st.transition(t, Instant::now())?;
        let sid = st.session_id.clone();
        if let Some(store) = &self.store {
            if let Some(id) = &sid {
                let _ = store.set_session_state(id, st.phase.as_str());
            }
        }
        self.log_event(sid.as_deref(), kind, json!({}));
        Ok(self.publish(st))
    }

    pub fn pause(&self) -> Result<SessionSnapshot> {
        self.simple(Transition::Pause, "pause")
    }
    pub fn resume(&self) -> Result<SessionSnapshot> {
        self.simple(Transition::Resume, "resume")
    }
    pub fn away(&self) -> Result<SessionSnapshot> {
        self.simple(Transition::Away, "away")
    }
    pub fn back(&self) -> Result<SessionSnapshot> {
        self.simple(Transition::Back, "back")
    }

    pub fn extend(&self, minutes: u32) -> Result<SessionSnapshot> {
        let now = Instant::now();
        let mut st = self.lock();
        let minutes = minutes.clamp(1, 60);
        match st.phase {
            p if p.has_session() => {}
            // Extending right after time ran out picks the session back up.
            SessionPhase::WrappingUp if st.session_id.is_some() => {
                st.transition(Transition::Start, now)?;
            }
            p => {
                return Err(CoreError::InvalidTransition {
                    action: "extend".into(),
                    phase: p.as_str().into(),
                })
            }
        }
        st.planned_secs = (st.planned_secs + minutes * 60).min(MAX_MINUTES * 60 * 2);
        let remaining = st.planned_secs.saturating_sub(st.elapsed(now));
        st.sched.on_extend(remaining);
        let sid = st.session_id.clone();
        self.log_event(sid.as_deref(), "extend", json!({ "minutes": minutes }));
        Ok(self.publish(st))
    }

    /// End the session early (or record the reflection after time ran out).
    pub fn end(&self, reflection: Option<String>) -> Result<SessionSnapshot> {
        let now = Instant::now();
        let mut st = self.lock();
        let reflection = reflection.filter(|r| !r.trim().is_empty());
        if st.phase.has_session() {
            st.transition(Transition::Finish, now)?;
        } else if !(st.phase == SessionPhase::WrappingUp && reflection.is_some()) {
            return Err(CoreError::InvalidTransition {
                action: "end the session".into(),
                phase: st.phase.as_str().into(),
            });
        }
        let sid = st.session_id.clone();
        if let (Some(store), Some(id)) = (&self.store, &sid) {
            if let Err(e) = store.end_session(id, reflection.as_deref()) {
                tracing::warn!("failed to save session end: {e}");
            }
        }
        self.log_event(sid.as_deref(), "end", json!({}));
        Ok(self.publish(st))
    }

    /// Back to idle (dismiss the wrap-up screen or stop a break early).
    pub fn close(&self) -> Result<SessionSnapshot> {
        let mut st = self.lock();
        st.transition(Transition::Close, Instant::now())?;
        st.session_id = None;
        st.planned_secs = 0;
        st.elapsed_before = Duration::ZERO;
        Ok(self.publish(st))
    }

    pub fn start_break(&self, minutes: u32) -> Result<SessionSnapshot> {
        if self.phase().has_session() {
            self.end(None)?;
        }
        let now = Instant::now();
        let mut st = self.lock();
        st.transition(Transition::StartBreak, now)?;
        st.break_planned_secs = minutes.clamp(1, 60) * 60;
        st.break_started = Some(now);
        Ok(self.publish(st))
    }

    pub fn add_subtasks(&self, items: &[String]) -> Result<SessionSnapshot> {
        let items: Vec<&str> = items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if items.is_empty() {
            return Err(CoreError::Invalid("no subtasks given".into()));
        }
        let mut st = self.lock();
        let task_id = st.task_id.clone();
        for title in items {
            let pos = st.subtasks.len() as i64;
            let id = match (&self.store, &task_id) {
                (Some(store), Some(tid)) => store
                    .add_subtask(tid, title, pos)
                    .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string()),
                _ => uuid::Uuid::new_v4().to_string(),
            };
            st.subtasks.push(Subtask {
                id,
                title: title.to_string(),
                done: false,
            });
        }
        Ok(self.publish(st))
    }

    /// Accepts a subtask id, or a 1-based position / exact title (local models often use those).
    pub fn complete_subtask(&self, key: &str) -> Result<SessionSnapshot> {
        let mut st = self.lock();
        let key = key.trim();
        let idx = st
            .subtasks
            .iter()
            .position(|s| s.id == key)
            .or_else(|| {
                key.parse::<usize>()
                    .ok()
                    .filter(|n| *n >= 1 && *n <= st.subtasks.len())
                    .map(|n| n - 1)
            })
            .or_else(|| {
                st.subtasks
                    .iter()
                    .position(|s| s.title.eq_ignore_ascii_case(key))
            })
            .ok_or_else(|| CoreError::Invalid(format!("no subtask matches '{key}'")))?;
        st.subtasks[idx].done = true;
        if let Some(store) = &self.store {
            let _ = store.complete_subtask(&st.subtasks[idx].id);
        }
        Ok(self.publish(st))
    }

    pub fn set_quiet(&self, minutes: u32) -> Result<SessionSnapshot> {
        let mut st = self.lock();
        st.quiet_until = if minutes == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(u64::from(minutes.min(240)) * 60))
        };
        Ok(self.publish(st))
    }

    pub fn set_checkin_mode(&self, mode: CheckinMode) -> SessionSnapshot {
        let mut st = self.lock();
        st.checkin_mode = mode;
        self.publish(st)
    }

    /// Advance timers. Called once per second by the app's ticker.
    pub fn tick(&self) -> Vec<EngineOutput> {
        let now = Instant::now();
        let mut st = self.lock();
        let mut out = Vec::new();
        if st.quiet_until.is_some_and(|q| q <= now) {
            st.quiet_until = None;
        }
        match st.phase {
            SessionPhase::Focusing => {
                let elapsed = st.elapsed(now);
                if elapsed >= st.planned_secs {
                    if st.transition(Transition::Finish, now).is_ok() {
                        let sid = st.session_id.clone();
                        if let (Some(store), Some(id)) = (&self.store, &sid) {
                            let _ = store.end_session(id, None);
                        }
                        self.log_event(sid.as_deref(), "end", json!({ "completed": true }));
                        out.push(EngineOutput::SessionComplete);
                        self.publish(st);
                        return out;
                    }
                } else {
                    let (planned, sched) = (st.planned_secs, st.config.schedule);
                    if let Some(reason) = checkin::due(elapsed, planned, sched, &mut st.sched) {
                        out.push(EngineOutput::Checkin(reason));
                    }
                }
            }
            SessionPhase::Break if st.break_remaining(now) == 0 => {
                if st.transition(Transition::Close, now).is_ok() {
                    out.push(EngineOutput::BreakOver);
                    self.publish(st);
                    return out;
                }
            }
            SessionPhase::Idle => return out,
            _ => {}
        }
        let tick = st.tick(now);
        drop(st);
        self.sink.emit(CoreEvent::SessionTick(tick));
        out
    }

    /// Run a validated LLM tool call. Returns a short result string for the model.
    pub fn execute_tool(&self, tool: Tool) -> Result<String> {
        let snap = match tool {
            Tool::StartSession {
                task,
                minutes,
                checkin_mode,
            } => {
                let s = self.start(&task, minutes, checkin_mode)?;
                return Ok(format!(
                    "Started a {}-minute session for \"{}\".",
                    s.planned_secs / 60,
                    task
                ));
            }
            Tool::PauseSession {} => self.pause()?,
            Tool::ResumeSession {} => self.resume()?,
            Tool::ExtendSession { minutes } => self.extend(minutes)?,
            Tool::EndSession { reflection } => self.end(reflection)?,
            Tool::StartBreak { minutes } => self.start_break(minutes)?,
            Tool::AddSubtasks { items } => {
                let s = self.add_subtasks(&items)?;
                let list: Vec<String> = s
                    .subtasks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("{}. {} (id {})", i + 1, t.title, t.id))
                    .collect();
                return Ok(format!("Checklist now: {}", list.join("; ")));
            }
            Tool::CompleteSubtask { id } => self.complete_subtask(&id)?,
            Tool::SetQuiet { minutes } => self.set_quiet(minutes)?,
            Tool::SetCharacterState { state } => {
                self.set_overlay(Some(state));
                return Ok("ok".into());
            }
        };
        Ok(format!("ok; session is now {}", snap.phase.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CollectingSink;

    fn engine() -> (Arc<CollectingSink>, SessionEngine) {
        let sink = Arc::new(CollectingSink::default());
        let e = SessionEngine::new(sink.clone(), None, EngineConfig::default());
        (sink, e)
    }

    async fn advance_secs(e: &SessionEngine, secs: u32) -> Vec<EngineOutput> {
        let mut out = Vec::new();
        for _ in 0..secs {
            tokio::time::advance(Duration::from_secs(1)).await;
            out.extend(e.tick());
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn full_session_runs_to_completion() {
        let (sink, e) = engine();
        let s = e
            .start("expense report", None, None)
            .ok()
            .unwrap_or_else(|| panic!());
        assert_eq!(s.planned_secs, 1500);
        assert_eq!(e.character_state(), CharacterState::Working);
        let out = advance_secs(&e, 1500).await;
        assert_eq!(
            out,
            vec![
                EngineOutput::Checkin(CheckinReason::Interval),
                EngineOutput::Checkin(CheckinReason::Halfway),
                EngineOutput::Checkin(CheckinReason::FiveMinutesLeft),
                EngineOutput::Checkin(CheckinReason::TwoMinutesLeft),
                EngineOutput::SessionComplete,
            ]
        );
        assert_eq!(e.phase(), SessionPhase::WrappingUp);
        assert_eq!(e.character_state(), CharacterState::Celebrating);
        assert!(sink
            .take()
            .iter()
            .any(|ev| matches!(ev, CoreEvent::SessionTick(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn pause_freezes_time_and_extend_adds_time() {
        let (_sink, e) = engine();
        let _ = e.start("write", Some(10), None);
        advance_secs(&e, 60).await;
        let _ = e.pause();
        advance_secs(&e, 300).await;
        assert_eq!(e.snapshot().elapsed_secs, 60);
        let _ = e.resume();
        let s = e.extend(5).ok().unwrap_or_else(|| panic!());
        assert_eq!(s.planned_secs, 900);
        assert_eq!(s.remaining_secs, 840);
        assert!(
            e.start_break(5).is_ok(),
            "break from a running session ends it first"
        );
        assert_eq!(e.phase(), SessionPhase::Break);
        let out = advance_secs(&e, 300).await;
        assert_eq!(out, vec![EngineOutput::BreakOver]);
        assert_eq!(e.phase(), SessionPhase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn minutes_are_clamped_and_empty_task_rejected() {
        let (_sink, e) = engine();
        assert!(e.start("  ", None, None).is_err());
        assert_eq!(
            e.start("x", Some(1), None).map(|s| s.planned_secs).ok(),
            Some(300)
        );
        assert_eq!(
            e.start("y", Some(500), None).map(|s| s.planned_secs).ok(),
            Some(7200)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn quiet_mode_expires() {
        let (_sink, e) = engine();
        let _ = e.start("x", Some(30), None);
        let _ = e.set_quiet(1);
        assert!(e.quiet());
        advance_secs(&e, 61).await;
        assert!(!e.quiet());
    }

    #[tokio::test(start_paused = true)]
    async fn subtasks_carry_into_new_session_and_complete_by_position() {
        let (_sink, e) = engine();
        let _ = e.add_subtasks(&["open the file".into(), "  ".into(), "fill row 1".into()]);
        let s = e
            .start("spreadsheet", None, None)
            .ok()
            .unwrap_or_else(|| panic!());
        assert_eq!(s.subtasks.len(), 2);
        let s = e.complete_subtask("2").ok().unwrap_or_else(|| panic!());
        assert!(s.subtasks[1].done);
        assert!(e.complete_subtask("Open the file").is_ok());
        assert!(e.complete_subtask("nope").is_err());
        // A different task starts with a fresh checklist.
        let s = e
            .start("email", None, None)
            .ok()
            .unwrap_or_else(|| panic!());
        assert!(s.subtasks.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn away_and_back_pause_the_timer() {
        let (_sink, e) = engine();
        let _ = e.start("x", Some(25), None);
        let _ = e.away();
        assert_eq!(e.character_state(), CharacterState::Waiting);
        advance_secs(&e, 120).await;
        assert_eq!(e.snapshot().elapsed_secs, 0);
        let _ = e.back();
        assert_eq!(e.phase(), SessionPhase::Focusing);
    }

    #[tokio::test(start_paused = true)]
    async fn overlay_wins_until_cleared() {
        let (sink, e) = engine();
        let _ = e.start("x", None, None);
        sink.take();
        e.set_overlay(Some(CharacterState::Thinking));
        e.set_overlay(Some(CharacterState::Thinking)); // no duplicate event
        e.set_overlay(None);
        let states: Vec<CharacterState> = sink
            .take()
            .into_iter()
            .filter_map(|ev| match ev {
                CoreEvent::CharacterState(c) => Some(c.state),
                _ => None,
            })
            .collect();
        assert_eq!(
            states,
            vec![CharacterState::Thinking, CharacterState::Working]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn end_after_completion_records_reflection_only() {
        let (_sink, e) = engine();
        let _ = e.start("x", Some(5), None);
        advance_secs(&e, 300).await;
        assert_eq!(e.phase(), SessionPhase::WrappingUp);
        assert!(e.end(Some("filed 3 receipts".into())).is_ok());
        assert!(e.end(None).is_err());
        assert!(e.extend(5).is_ok(), "extend from wrap-up resumes");
        assert_eq!(e.phase(), SessionPhase::Focusing);
    }
}
