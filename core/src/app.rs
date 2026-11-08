//! `Core`: wires the store, router, session engine, and orchestrator together.
//! Every Tauri command is a thin wrapper over a method here (see `commands.rs`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Serialize;
use ts_rs::TS;

use crate::audio::controller::VoiceController;
use crate::audio::speaker::SpeakerPipeline;
use crate::audio::tap::SpeechTap;
use crate::audio::Speaker;
use crate::character::CharacterState;
use crate::checkin::{self, CheckinAction, CheckinEvent, CheckinReason, Schedule};
use crate::conversation::prompt::PromptContext;
use crate::conversation::{ChatMessageDto, ChatRole, Orchestrator, Via};
use crate::error::{CoreError, Result};
use crate::events::{CoreEvent, EventSink};
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::ollama::OllamaProvider;
use crate::providers::{Caps, ChatProvider, Locality};
use crate::router::{locality, Router};
use crate::secrets::SecretStore;
use crate::session::engine::{EngineConfig, EngineOutput, SessionEngine};
use crate::session::{CheckinMode, SessionSnapshot};
use crate::settings::{Profile, ProviderKind, Settings};
use crate::store::{HistoryEntry, Store};

const SETTINGS_KEY: &str = "app";
const PROFILE_KEY: &str = "profile";
pub const ANTHROPIC_ACCOUNT: &str = "anthropic";

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ProviderStatus {
    pub kind: ProviderKind,
    pub model: String,
    pub locality: Locality,
    pub ready: bool,
    pub tools: bool,
    pub vision: bool,
    /// Human-readable problem, if any.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AppState {
    pub session: SessionSnapshot,
    pub character: CharacterState,
    pub settings: Settings,
    pub profile: Profile,
    pub provider: Option<ProviderStatus>,
    pub has_anthropic_key: bool,
}

pub struct Core {
    store: Arc<Store>,
    router: Arc<Router>,
    engine: Arc<SessionEngine>,
    orchestrator: Arc<Orchestrator>,
    sink: Arc<dyn EventSink>,
    secrets: Arc<dyn SecretStore>,
    settings: RwLock<Settings>,
    profile: RwLock<Profile>,
    provider_status: RwLock<Option<ProviderStatus>>,
    buddy_moves: std::sync::atomic::AtomicU64,
    voice: Arc<VoiceController>,
    speaker: Arc<SpeakerPipeline>,
}

fn engine_config(s: &Settings) -> EngineConfig {
    EngineConfig {
        default_minutes: s.default_minutes,
        default_checkin_mode: s.checkin_mode,
        schedule: Schedule {
            interval_secs: s.checkin_interval_min * 60,
            time_warnings: s.time_warnings,
        },
    }
}

fn prompt_context(p: &Profile) -> PromptContext {
    PromptContext {
        companion_name: p.companion.name.clone(),
        tone: p.tone,
        about_me: p.about_me.clone(),
    }
}

impl Core {
    /// Open (or create) `<data_dir>/tandem.db` and build the core.
    pub async fn init(
        data_dir: &Path,
        sink: Arc<dyn EventSink>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| CoreError::Invalid(format!("can't create data dir: {e}")))?;
        let store = Store::open(&data_dir.join("tandem.db"))?;
        Self::with_store_in(Arc::new(store), sink, secrets, data_dir.join("models")).await
    }

    /// Tests and evals: models (never downloaded there) resolve under the temp dir.
    pub async fn with_store(
        store: Arc<Store>,
        sink: Arc<dyn EventSink>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Arc<Self>> {
        let models_dir = std::env::temp_dir().join("tandem-models");
        Self::with_store_in(store, sink, secrets, models_dir).await
    }

    pub async fn with_store_in(
        store: Arc<Store>,
        sink: Arc<dyn EventSink>,
        secrets: Arc<dyn SecretStore>,
        models_dir: PathBuf,
    ) -> Result<Arc<Self>> {
        // Voice output sees chat events on their way to the UI (audio/tap.rs).
        let raw_sink = sink;
        let speaker = SpeakerPipeline::new(raw_sink.clone(), models_dir.clone());
        let sink: Arc<dyn EventSink> = Arc::new(SpeechTap::new(
            raw_sink.clone(),
            speaker.clone() as Arc<dyn Speaker>,
        ));
        let settings = store
            .get_setting::<Settings>(SETTINGS_KEY)
            .unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable settings: {e}");
                None
            })
            .unwrap_or_default()
            .normalised();
        let profile = store
            .get_profile::<Profile>(PROFILE_KEY)
            .unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable profile: {e}");
                None
            })
            .unwrap_or_default();
        if let Err(e) = store.purge_messages_older_than(settings.message_retention_days) {
            tracing::warn!("message retention purge failed: {e}");
        }
        let router = Arc::new(Router::new());
        let engine = Arc::new(SessionEngine::new(
            sink.clone(),
            Some(store.clone()),
            engine_config(&settings),
        ));
        let orchestrator = Arc::new(Orchestrator::new(
            router.clone(),
            engine.clone(),
            sink.clone(),
            Some(store.clone()),
            prompt_context(&profile),
        ));
        speaker.attach_engine(engine.clone());
        let voice = VoiceController::new(
            raw_sink.clone(),
            speaker.clone() as Arc<dyn Speaker>,
            models_dir,
        );
        let core = Arc::new(Self {
            store,
            router,
            engine,
            orchestrator,
            sink,
            secrets,
            settings: RwLock::new(settings),
            profile: RwLock::new(profile),
            provider_status: RwLock::new(None),
            buddy_moves: std::sync::atomic::AtomicU64::new(0),
            voice,
            speaker,
        });
        core.voice.attach_core(Arc::downgrade(&core));
        core.apply_voice_settings();
        core.rebuild_providers().await;
        Ok(core)
    }

    /// Test/eval hook: use a specific chat provider instead of the configured one.
    pub fn set_chat_provider(&self, provider: Arc<dyn ChatProvider>) {
        self.router.set_chat(Some(provider));
    }

    /// Voice input: push-to-talk, hands-free, barge-in (M2).
    pub fn voice(&self) -> &Arc<VoiceController> {
        &self.voice
    }

    /// Voice output: text-to-speech pipeline and voices (M2).
    pub fn speaker(&self) -> &Arc<SpeakerPipeline> {
        &self.speaker
    }

    fn apply_voice_settings(&self) {
        let settings = self.settings();
        let profile = self.profile();
        self.speaker
            .apply_settings(&settings.voice, profile.companion.voice_id.as_deref());
        self.voice.apply_settings(&settings.voice, &profile);
    }

    pub fn engine(&self) -> &Arc<SessionEngine> {
        &self.engine
    }

    pub fn orchestrator(&self) -> &Arc<Orchestrator> {
        &self.orchestrator
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn profile(&self) -> Profile {
        self.profile.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn provider_status(&self) -> Option<ProviderStatus> {
        self.provider_status.read().ok().and_then(|g| g.clone())
    }

    /// Start the once-per-second timer loop.
    pub fn spawn_ticker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let core = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                for out in core.engine.tick() {
                    let reason = match out {
                        EngineOutput::Checkin(r) => r,
                        EngineOutput::SessionComplete => CheckinReason::SessionComplete,
                        EngineOutput::BreakOver => CheckinReason::BreakOver,
                    };
                    let c = Arc::clone(&core);
                    tokio::spawn(async move { c.run_checkin(reason).await });
                }
            }
        })
    }

    /// Decide and deliver one check-in.
    pub async fn run_checkin(&self, reason: CheckinReason) {
        let action = checkin::decide(
            reason,
            self.engine.checkin_mode(),
            self.engine.signals(),
            self.engine.quiet(),
        );
        if let Some(sid) = self.engine.session_id() {
            let payload = serde_json::json!({ "reason": reason, "action": action });
            let _ = self.store.add_event(&sid, "checkin", &payload);
        }
        let text = match action {
            CheckinAction::Skip => return,
            CheckinAction::SilentGlance => None,
            CheckinAction::GentleLine | CheckinAction::Ask => {
                let line = self.orchestrator.checkin_line(reason, action).await;
                if let Some(t) = &line {
                    self.sink.emit(CoreEvent::ChatMessage(ChatMessageDto {
                        id: uuid::Uuid::new_v4().to_string(),
                        role: ChatRole::Assistant,
                        text: t.clone(),
                        via: Via::Checkin,
                        ts: chrono::Utc::now().to_rfc3339(),
                    }));
                }
                line
            }
        };
        self.sink.emit(CoreEvent::Checkin(CheckinEvent {
            reason,
            action,
            text,
        }));
    }

    /// (Re)create providers from settings. Never fails; problems land in `ProviderStatus.error`.
    pub async fn rebuild_providers(&self) -> ProviderStatus {
        let settings = self.settings();
        let status = match settings.chat_provider {
            ProviderKind::Ollama => self.build_ollama(&settings).await,
            ProviderKind::Anthropic => self.build_anthropic(&settings),
        };
        if let Ok(mut g) = self.provider_status.write() {
            *g = Some(status.clone());
        }
        status
    }

    async fn build_ollama(&self, s: &Settings) -> ProviderStatus {
        let o = &s.ollama;
        let loc = locality::classify(&o.url, &o.confirmed_hosts);
        let mut status = ProviderStatus {
            kind: ProviderKind::Ollama,
            model: o.chat_model.clone(),
            locality: loc,
            ready: false,
            tools: false,
            vision: false,
            error: None,
        };
        let mut model = o.chat_model.clone();
        if model.is_empty() {
            match OllamaProvider::list_models(&o.url).await {
                Ok(models) if !models.is_empty() => model = models[0].clone(),
                Ok(_) => {
                    status.error = Some("Ollama has no models installed yet.".into());
                    self.router.set_chat(None);
                    return status;
                }
                Err(e) => {
                    status.error = Some(e.to_string());
                    self.router.set_chat(None);
                    return status;
                }
            }
        }
        status.model = model.clone();
        let caps = match OllamaProvider::probe_caps(&o.url, &model).await {
            Ok(c) => {
                status.ready = true;
                c
            }
            Err(e) => {
                status.error = Some(e.to_string());
                // Keep a provider so it works once the server comes back.
                Caps {
                    tools: true,
                    vision: false,
                    streaming: true,
                }
            }
        };
        status.tools = caps.tools;
        status.vision = caps.vision;
        self.router.set_chat(Some(Arc::new(OllamaProvider::new(
            &o.url,
            &model,
            &o.confirmed_hosts,
            caps,
        ))));
        if !o.vision_model.is_empty() {
            let vcaps = Caps {
                tools: false,
                vision: true,
                streaming: true,
            };
            self.router.set_vision(Some(Arc::new(OllamaProvider::new(
                &o.url,
                &o.vision_model,
                &o.confirmed_hosts,
                vcaps,
            ))));
        } else {
            self.router.set_vision(None);
        }
        status
    }

    fn build_anthropic(&self, s: &Settings) -> ProviderStatus {
        let mut status = ProviderStatus {
            kind: ProviderKind::Anthropic,
            model: s.anthropic.model.clone(),
            locality: Locality::Cloud,
            ready: false,
            tools: true,
            vision: true,
            error: None,
        };
        match self.secrets.get(ANTHROPIC_ACCOUNT) {
            Ok(Some(key)) if !key.is_empty() => {
                self.router.set_chat(Some(Arc::new(AnthropicProvider::new(
                    key,
                    &s.anthropic.model,
                    &s.anthropic.effort,
                ))));
                status.ready = true;
            }
            Ok(_) => {
                self.router.set_chat(None);
                status.error = Some("Add an Anthropic API key in Settings.".into());
            }
            Err(e) => {
                self.router.set_chat(None);
                status.error = Some(e.to_string());
            }
        }
        // Camera snapshots never default to the cloud: vision stays on the local Ollama model if any.
        status
    }

    // ---- commands -------------------------------------------------------------------------

    pub fn state(&self) -> AppState {
        AppState {
            session: self.engine.snapshot(),
            character: self.engine.character_state(),
            settings: self.settings(),
            profile: self.profile(),
            provider: self.provider_status(),
            has_anthropic_key: self.has_api_key(ANTHROPIC_ACCOUNT),
        }
    }

    pub fn start_session(
        &self,
        task: &str,
        minutes: Option<u32>,
        checkin_mode: Option<CheckinMode>,
    ) -> Result<SessionSnapshot> {
        self.engine.start(task, minutes, checkin_mode)
    }

    pub fn pause_session(&self) -> Result<SessionSnapshot> {
        self.engine.pause()
    }

    pub fn resume_session(&self) -> Result<SessionSnapshot> {
        self.engine.resume()
    }

    pub fn extend_session(&self, minutes: u32) -> Result<SessionSnapshot> {
        self.engine.extend(minutes)
    }

    pub fn end_session(&self, reflection: Option<String>) -> Result<SessionSnapshot> {
        self.engine.end(reflection)
    }

    pub fn close_session(&self) -> Result<SessionSnapshot> {
        self.engine.close()
    }

    pub fn start_break(&self, minutes: Option<u32>) -> Result<SessionSnapshot> {
        let m = minutes.unwrap_or_else(|| self.settings().short_break_min);
        self.engine.start_break(m)
    }

    pub fn add_subtasks(&self, items: &[String]) -> Result<SessionSnapshot> {
        self.engine.add_subtasks(items)
    }

    pub fn complete_subtask(&self, id: &str) -> Result<SessionSnapshot> {
        self.engine.complete_subtask(id)
    }

    pub fn set_quiet(&self, minutes: u32) -> Result<SessionSnapshot> {
        self.engine.set_quiet(minutes)
    }

    pub fn set_checkin_mode(&self, mode: CheckinMode) -> SessionSnapshot {
        self.engine.set_checkin_mode(mode)
    }

    pub async fn send_message(&self, text: &str, via: Via) -> Result<Option<ChatMessageDto>> {
        self.orchestrator.send(text, via).await
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<ProviderStatus> {
        let mut settings = settings.normalised();
        // The buddy's position is owned by the core (window moves); never take it from the UI.
        let current = self.settings().buddy;
        settings.buddy.x = current.x;
        settings.buddy.y = current.y;
        self.store.set_setting(SETTINGS_KEY, &settings)?;
        self.engine.set_config(engine_config(&settings));
        if let Ok(mut g) = self.settings.write() {
            *g = settings.clone();
        }
        self.sink.emit(CoreEvent::Settings(settings));
        self.apply_voice_settings();
        Ok(self.rebuild_providers().await)
    }

    pub fn update_profile(&self, profile: Profile) -> Result<Profile> {
        let profile = profile.validated()?;
        self.store.set_profile(PROFILE_KEY, &profile)?;
        self.orchestrator.set_context(prompt_context(&profile));
        if let Ok(mut g) = self.profile.write() {
            *g = profile.clone();
        }
        self.sink.emit(CoreEvent::Profile(profile.clone()));
        self.apply_voice_settings();
        Ok(profile)
    }

    pub async fn set_api_key(&self, account: &str, key: &str) -> Result<ProviderStatus> {
        let key = key.trim();
        if key.is_empty() {
            return Err(CoreError::Invalid("the key is empty".into()));
        }
        self.secrets.set(account, key)?;
        Ok(self.rebuild_providers().await)
    }

    pub async fn clear_api_key(&self, account: &str) -> Result<ProviderStatus> {
        self.secrets.delete(account)?;
        Ok(self.rebuild_providers().await)
    }

    pub fn has_api_key(&self, account: &str) -> bool {
        matches!(self.secrets.get(account), Ok(Some(k)) if !k.is_empty())
    }

    pub async fn list_ollama_models(&self, url: &str) -> Result<Vec<String>> {
        Ok(OllamaProvider::list_models(url).await?)
    }

    /// Show or hide the desktop buddy (persisted).
    pub fn set_buddy_enabled(&self, enabled: bool) -> Result<Settings> {
        let mut settings = self.settings();
        settings.buddy.enabled = enabled;
        self.store.set_setting(SETTINGS_KEY, &settings)?;
        if let Ok(mut g) = self.settings.write() {
            *g = settings.clone();
        }
        self.sink.emit(CoreEvent::Settings(settings.clone()));
        Ok(settings)
    }

    /// Remember where the user dragged the buddy. Window move events arrive in bursts,
    /// so writes are debounced: only the last position in a 600 ms quiet window is saved.
    pub fn save_buddy_position(self: &Arc<Self>, x: i32, y: i32) {
        let generation = {
            let Ok(mut g) = self.settings.write() else {
                return;
            };
            g.buddy.x = Some(x);
            g.buddy.y = Some(y);
            self.buddy_moves
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1
        };
        let core = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            if core.buddy_moves.load(std::sync::atomic::Ordering::SeqCst) == generation {
                if let Err(e) = core.store.set_setting(SETTINGS_KEY, &core.settings()) {
                    tracing::warn!("failed to save buddy position: {e}");
                }
            }
        });
    }

    pub fn session_history(&self, limit: u32) -> Result<Vec<HistoryEntry>> {
        self.store.recent_sessions(limit.min(200))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CollectingSink;
    use crate::providers::mock::MockProvider;
    use crate::secrets::MemorySecretStore;
    use crate::session::SessionPhase;

    async fn core() -> (Arc<CollectingSink>, Arc<Core>) {
        let sink = Arc::new(CollectingSink::default());
        let store = Arc::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")));
        let mut s = Settings::default();
        // Point at a closed port so tests never touch a real server.
        s.ollama.url = "http://127.0.0.1:9".into();
        s.ollama.chat_model = "test".into();
        store
            .set_setting(SETTINGS_KEY, &s)
            .unwrap_or_else(|e| panic!("{e}"));
        let core = Core::with_store(store, sink.clone(), Arc::new(MemorySecretStore::default()))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        (sink, core)
    }

    #[tokio::test]
    async fn unreachable_ollama_reports_error_but_keeps_provider() {
        let (_sink, core) = core().await;
        let status = core.provider_status().unwrap_or_else(|| panic!());
        assert!(!status.ready);
        assert!(status.error.is_some());
        assert_eq!(status.locality, Locality::Local);
    }

    #[tokio::test]
    async fn anthropic_needs_key_in_secret_store() {
        let (_sink, core) = core().await;
        let mut s = core.settings();
        s.chat_provider = ProviderKind::Anthropic;
        let status = core
            .update_settings(s)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!status.ready);
        let status = core
            .set_api_key(ANTHROPIC_ACCOUNT, " sk-test ")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(status.ready);
        assert!(core.has_api_key(ANTHROPIC_ACCOUNT));
        // The key never appears in persisted settings or app state.
        let state = serde_json::to_string(&core.state()).unwrap_or_default();
        assert!(!state.contains("sk-test"));
        let settings = serde_json::to_string(&core.settings()).unwrap_or_default();
        assert!(!settings.contains("sk-test"));
        assert!(core.clear_api_key(ANTHROPIC_ACCOUNT).await.is_ok());
        assert!(!core.has_api_key(ANTHROPIC_ACCOUNT));
    }

    #[tokio::test]
    async fn profile_changes_reach_the_prompt() {
        let (_sink, core) = core().await;
        let mock = Arc::new(MockProvider::new(Locality::Local).with_text_reply("Hi."));
        core.set_chat_provider(mock.clone());
        let mut p = core.profile();
        p.companion.name = "Maple".into();
        assert!(core.update_profile(p).is_ok());
        let _ = core.send_message("hello", Via::Text).await;
        assert!(mock.requests()[0].system.starts_with("You are Maple"));
        let mut bad = core.profile();
        bad.companion.name = String::new();
        assert!(core.update_profile(bad).is_err());
        // Every window hears about profile changes (e.g. the desktop buddy).
        assert!(_sink
            .take()
            .iter()
            .any(|e| matches!(e, CoreEvent::Profile(p) if p.companion.name == "Maple")));
    }

    #[tokio::test]
    async fn checkin_flow_emits_line_or_glance() {
        let (sink, core) = core().await;
        let mock = Arc::new(
            MockProvider::new(Locality::Local).with_text_reply("Still with it? I'm here."),
        );
        core.set_chat_provider(mock);
        let _ = core.start_session("report", None, Some(CheckinMode::Gentle));
        sink.take();
        core.run_checkin(CheckinReason::Interval).await;
        let ev: Vec<CheckinEvent> = sink
            .take()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::Checkin(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].action, CheckinAction::GentleLine);
        assert_eq!(ev[0].text.as_deref(), Some("Still with it? I'm here."));

        let _ = core.set_quiet(30);
        core.run_checkin(CheckinReason::Interval).await;
        let glance = sink.take().into_iter().find_map(|e| match e {
            CoreEvent::Checkin(c) => Some(c),
            _ => None,
        });
        assert_eq!(
            glance.map(|c| (c.action, c.text)),
            Some((CheckinAction::SilentGlance, None))
        );
    }

    #[tokio::test]
    async fn checkin_without_provider_falls_back_to_ui_copy() {
        let (sink, core) = core().await;
        let mock = Arc::new(MockProvider::new(Locality::Local));
        mock.fail("down");
        core.set_chat_provider(mock);
        let _ = core.start_session("report", None, None);
        core.run_checkin(CheckinReason::Halfway).await;
        let ev = sink.take().into_iter().find_map(|e| match e {
            CoreEvent::Checkin(c) => Some(c),
            _ => None,
        });
        assert_eq!(ev.map(|c| c.text), Some(None));
    }

    #[tokio::test(start_paused = true)]
    async fn buddy_toggle_and_debounced_position() {
        let (_sink, core) = core().await;
        assert!(core.settings().buddy.enabled);
        assert!(core.set_buddy_enabled(false).is_ok());
        core.save_buddy_position(10, 20);
        core.save_buddy_position(300, 400);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let saved: Settings = core
            .store
            .get_setting(SETTINGS_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();
        assert!(!saved.buddy.enabled);
        assert_eq!((saved.buddy.x, saved.buddy.y), (Some(300), Some(400)));
    }

    #[tokio::test]
    async fn session_history_is_recorded() {
        let (_sink, core) = core().await;
        let _ = core.start_session("taxes", Some(5), None);
        let _ = core.end_session(Some("did one form".into()));
        assert_eq!(core.engine().phase(), SessionPhase::WrappingUp);
        let h = core.session_history(10).unwrap_or_default();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].reflection.as_deref(), Some("did one form"));
    }
}
