//! OWNER: input agent. Voice turn controller: push-to-talk, hands-free (VAD), barge-in,
//! wake phrase. The public API below is a contract the commands layer (UI agent) builds
//! against: keep these names and signatures.
//!
//! Flow: mic frames (16 kHz mono) → in-memory buffer (push-to-talk) or VAD segmenter
//! (hands-free) → whisper → `clean_transcript` → `Core::send_message(text, Via::Voice)`.
//! Audio is never written to disk or logged, and neither are transcripts.
//!
//! `problem` keys emitted on `voice://state` (UI copy lives in `ui/src/i18n/en.json`):
//! see the `PROBLEM_*` constants below.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use super::capture::{rms_level, CpalMic, MicSource};
use super::models;
use super::stt::{clean_transcript, stt_model_file, stt_model_ids, WhisperStt};
use super::vad::{
    vad_model_file, BargeIn, SegmentEvent, Segmenter, SegmenterConfig, SileroVad, VadModel,
    VAD_FRAME, VAD_MODEL_ID,
};
use super::wake::{match_wake, WakeMatch};
use super::{
    AudioClip, AudioError, AudioLevel, LevelSource, Speaker, SttModelInfo, SttProvider, VoicePhase,
    VoiceStatus, STT_SAMPLE_RATE,
};
use crate::app::Core;
use crate::conversation::Via;
use crate::error::{CoreError, Result};
use crate::events::{CoreEvent, EventSink};
use crate::providers::Modality;
use crate::router::privacy::{self, Consent};
use crate::settings::{Profile, SpeakReplies, VoiceSettings};

/// Nothing (or only noise) was heard; nothing was sent.
pub const PROBLEM_NOT_HEARD: &str = "voice.not_heard";
/// No microphone found.
pub const PROBLEM_NO_MIC: &str = "voice.error.no_mic";
/// The OS denied microphone access.
pub const PROBLEM_MIC_DENIED: &str = "voice.error.mic_denied";
/// Some other microphone failure.
pub const PROBLEM_MIC: &str = "voice.error.mic";
/// The speech model isn't downloaded.
pub const PROBLEM_STT_MISSING: &str = "voice.error.stt_missing";
/// The voice-detection model (hands-free) isn't downloaded.
pub const PROBLEM_VAD_MISSING: &str = "voice.error.vad_missing";
/// Speech-to-text failed.
pub const PROBLEM_STT_FAILED: &str = "voice.error.stt_failed";
/// Hands-free listening couldn't start (no async runtime, VAD failed to load…).
pub const PROBLEM_HANDS_FREE: &str = "voice.error.hands_free";

/// Clips shorter than this are a mis-tap.
const MIN_CLIP_SECS: f32 = 0.3;
/// Push-to-talk recording cap (60 s), in samples.
const MAX_PTT_SAMPLES: usize = 60 * STT_SAMPLE_RATE as usize;
/// Mic meter: one level per ~33 ms.
const LEVEL_WINDOW: usize = STT_SAMPLE_RATE as usize / 30;
/// After "Hey {name}" alone, the next segment is accepted without the phrase for this long.
const WAKE_WINDOW: Duration = Duration::from_secs(8);
/// After the reply is in, how long to wait for speech to start before going back to Ready.
const SPEECH_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
/// Polls of silence before a finished reply counts as done (gaps between sentences).
const QUIET_POLLS: u32 = 3;

type SttLoader =
    Arc<dyn Fn(&Path) -> std::result::Result<Arc<dyn SttProvider>, AudioError> + Send + Sync>;
type VadLoader =
    Arc<dyn Fn(&Path) -> std::result::Result<Box<dyn VadModel>, AudioError> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    PushToTalk,
    HandsFree,
}

fn problem_for(e: &AudioError) -> &'static str {
    match e {
        AudioError::NoInputDevice => PROBLEM_NO_MIC,
        AudioError::PermissionDenied => PROBLEM_MIC_DENIED,
        AudioError::ModelMissing(_) => PROBLEM_STT_MISSING,
        AudioError::Stt(_) | AudioError::Privacy(_) => PROBLEM_STT_FAILED,
        _ => PROBLEM_MIC,
    }
}

struct State {
    settings: VoiceSettings,
    companion_name: String,
    /// Phase while voice is enabled and the model is present (`status()` reports `Off` otherwise).
    phase: VoicePhase,
    problem: Option<String>,
    ptt_active: bool,
    hands_free: bool,
    /// Bumped on every new utterance; stale watchers stop touching the phase.
    turn: u64,
    wake_open_until: Option<Instant>,
}

/// Shared with the mic callback on the audio thread.
struct Capture {
    sink: Arc<dyn EventSink>,
    ptt_on: AtomicBool,
    ptt: Mutex<Vec<f32>>,
    hands_free: Mutex<Option<mpsc::SyncSender<Vec<f32>>>>,
    meter: Mutex<Vec<f32>>,
}

impl Capture {
    fn on_frame(&self, frame: &[f32]) {
        if let Ok(mut m) = self.meter.lock() {
            m.extend_from_slice(frame);
            while m.len() >= LEVEL_WINDOW {
                let level = rms_level(&m[..LEVEL_WINDOW]);
                m.drain(..LEVEL_WINDOW);
                self.sink.emit(CoreEvent::Level(AudioLevel {
                    source: LevelSource::Mic,
                    level,
                }));
            }
        }
        if self.ptt_on.load(Ordering::SeqCst) {
            if let Ok(mut b) = self.ptt.lock() {
                let room = MAX_PTT_SAMPLES.saturating_sub(b.len());
                b.extend_from_slice(&frame[..frame.len().min(room)]);
            }
        } else if let Ok(g) = self.hands_free.lock() {
            if let Some(tx) = g.as_ref() {
                // Drop frames rather than block the audio thread if VAD falls behind.
                let _ = tx.try_send(frame.to_vec());
            }
        }
    }

    fn take_ptt(&self) -> Vec<f32> {
        self.ptt
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default()
    }
}

pub struct VoiceController {
    sink: Arc<dyn EventSink>,
    speaker: Arc<dyn Speaker>,
    models_dir: PathBuf,
    me: Weak<VoiceController>,
    core: Mutex<Weak<Core>>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    mic: RwLock<Arc<dyn MicSource>>,
    mic_running: Mutex<bool>,
    stt_loader: RwLock<SttLoader>,
    vad_loader: RwLock<VadLoader>,
    stt: tokio::sync::Mutex<Option<(String, Arc<dyn SttProvider>)>>,
    state: Mutex<State>,
    capture: Arc<Capture>,
    /// We set the character's `Listening` overlay and must clear it ourselves.
    listening_overlay: std::sync::atomic::AtomicBool,
}

impl VoiceController {
    /// Must not open any audio device; devices open on first use.
    pub fn new(
        sink: Arc<dyn EventSink>,
        speaker: Arc<dyn Speaker>,
        models_dir: PathBuf,
    ) -> Arc<Self> {
        let stt_loader: SttLoader =
            Arc::new(|p: &Path| Ok(Arc::new(WhisperStt::load(p)?) as Arc<dyn SttProvider>));
        let vad_loader: VadLoader =
            Arc::new(|p: &Path| Ok(Box::new(SileroVad::load(p)?) as Box<dyn VadModel>));
        Self::with_backends(
            sink,
            speaker,
            models_dir,
            Arc::new(CpalMic::new(None)),
            stt_loader,
            vad_loader,
        )
    }

    fn with_backends(
        sink: Arc<dyn EventSink>,
        speaker: Arc<dyn Speaker>,
        models_dir: PathBuf,
        mic: Arc<dyn MicSource>,
        stt_loader: SttLoader,
        vad_loader: VadLoader,
    ) -> Arc<Self> {
        let capture = Arc::new(Capture {
            sink: sink.clone(),
            ptt_on: AtomicBool::new(false),
            ptt: Mutex::new(Vec::new()),
            hands_free: Mutex::new(None),
            meter: Mutex::new(Vec::new()),
        });
        Arc::new_cyclic(|me| Self {
            sink,
            speaker,
            models_dir,
            me: me.clone(),
            core: Mutex::new(Weak::new()),
            runtime: Mutex::new(tokio::runtime::Handle::try_current().ok()),
            mic: RwLock::new(mic),
            mic_running: Mutex::new(false),
            stt_loader: RwLock::new(stt_loader),
            vad_loader: RwLock::new(vad_loader),
            stt: tokio::sync::Mutex::new(None),
            state: Mutex::new(State {
                settings: VoiceSettings::default(),
                companion_name: String::new(),
                phase: VoicePhase::Ready,
                problem: None,
                ptt_active: false,
                hands_free: false,
                turn: 0,
                wake_open_until: None,
            }),
            capture,
            listening_overlay: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Tests: swap in a fake mic and fake engines (e.g. on the controller `Core` built).
    #[cfg(test)]
    fn set_backends(&self, mic: Arc<dyn MicSource>, stt_loader: SttLoader, vad_loader: VadLoader) {
        if let Ok(mut g) = self.mic.write() {
            *g = mic;
        }
        if let Ok(mut g) = self.stt_loader.write() {
            *g = stt_loader;
        }
        if let Ok(mut g) = self.vad_loader.write() {
            *g = vad_loader;
        }
    }

    /// Called once by `Core` after construction so the controller can send voice turns
    /// with `core.send_message(text, Via::Voice)`.
    pub fn attach_core(&self, core: Weak<Core>) {
        if let Ok(mut g) = self.core.lock() {
            *g = core;
        }
        self.remember_runtime();
    }

    /// Called at startup and whenever settings or the profile change.
    pub fn apply_settings(&self, voice: &VoiceSettings, profile: &Profile) {
        self.remember_runtime();
        let old = {
            let Ok(mut st) = self.state.lock() else {
                return;
            };
            let old = std::mem::replace(&mut st.settings, voice.clone());
            st.companion_name = profile.companion.name.clone();
            if !voice.enabled {
                st.problem = None;
            }
            old
        };
        if old.input_device != voice.input_device {
            let name = Some(voice.input_device.clone()).filter(|n| !n.trim().is_empty());
            self.mic().set_device(name);
        }
        if old.stt_model != voice.stt_model {
            // Free the old model now; the new one loads lazily on next use.
            if let Ok(mut g) = self.stt.try_lock() {
                *g = None;
            }
        }
        if !voice.enabled {
            self.ptt_cancel();
        }
        self.reconcile_hands_free();
        self.emit_status();
    }

    pub fn status(&self) -> VoiceStatus {
        let (settings, phase, problem, hands_free) = match self.state.lock() {
            Ok(st) => (
                st.settings.clone(),
                st.phase,
                st.problem.clone(),
                st.hands_free,
            ),
            Err(_) => return VoiceStatus::default(),
        };
        let stt_ready = self.stt_present(&settings.stt_model);
        let (phase, problem) = if !settings.enabled {
            (VoicePhase::Off, None)
        } else if !stt_ready {
            (VoicePhase::Off, Some(PROBLEM_STT_MISSING.to_string()))
        } else {
            (phase, problem)
        };
        VoiceStatus {
            phase,
            hands_free,
            stt_ready,
            tts_ready: self.speaker.ready(),
            problem,
        }
    }

    /// Start recording (push-to-talk pressed). Stops any companion speech first.
    pub async fn ptt_start(&self) -> Result<()> {
        self.remember_runtime();
        let (enabled, model) =
            self.with_state(|st| (st.settings.enabled, st.settings.stt_model.clone()));
        if !enabled {
            return Err(AudioError::Disabled.into());
        }
        if !self.stt_present(&model) {
            self.emit_status();
            return Err(AudioError::ModelMissing("speech".into()).into());
        }
        let already = self.with_state(|st| {
            let was = st.ptt_active;
            if !was {
                st.ptt_active = true;
                st.turn += 1;
                st.problem = None;
            }
            was
        });
        if already {
            return Ok(());
        }
        // Barge-in by button: the companion stops talking at once.
        self.speaker.stop();
        if let Ok(mut b) = self.capture.ptt.lock() {
            b.clear();
        }
        self.capture.ptt_on.store(true, Ordering::SeqCst);
        if let Err(e) = self.ensure_mic() {
            self.capture.ptt_on.store(false, Ordering::SeqCst);
            self.with_state(|st| st.ptt_active = false);
            self.set_phase(VoicePhase::Ready, Some(problem_for(&e)));
            return Err(e.into());
        }
        self.set_phase(VoicePhase::Listening, None);
        Ok(())
    }

    /// Stop recording, transcribe, and send the text as a voice turn.
    /// Returns the transcript (empty if nothing was heard).
    pub async fn ptt_stop(&self) -> Result<String> {
        let was = self.with_state(|st| std::mem::replace(&mut st.ptt_active, false));
        if !was {
            return Ok(String::new());
        }
        self.capture.ptt_on.store(false, Ordering::SeqCst);
        let samples = self.capture.take_ptt();
        self.release_mic();
        let clip = AudioClip {
            samples,
            sample_rate: STT_SAMPLE_RATE,
        };
        self.handle_utterance(clip, Origin::PushToTalk).await
    }

    /// Drop the current recording without sending anything.
    pub fn ptt_cancel(&self) {
        let was = self.with_state(|st| std::mem::replace(&mut st.ptt_active, false));
        if !was {
            return;
        }
        self.capture.ptt_on.store(false, Ordering::SeqCst);
        drop(self.capture.take_ptt());
        self.release_mic();
        self.set_phase(VoicePhase::Ready, None);
    }

    pub fn stt_models(&self) -> Vec<SttModelInfo> {
        stt_model_ids()
            .map(|(id, size_bytes)| SttModelInfo {
                id: id.to_string(),
                // The download also fetches the Silero VAD model (~2.3 MB) for hands-free.
                size_bytes: size_bytes + 2_300_000,
                downloaded: self.stt_present(id),
            })
            .collect()
    }

    /// User-initiated download of a whisper model; progress goes out on `voice://download`.
    /// Also fetches the small voice-detection model used by hands-free listening.
    pub async fn download_stt_model(&self, id: &str) -> Result<()> {
        let file = stt_model_file(id)
            .ok_or_else(|| CoreError::Invalid(format!("unknown speech model: {id}")))?;
        let sink = self.sink.clone();
        let progress = move |p| sink.emit(CoreEvent::Download(p));
        models::ensure(&self.models_dir, &file, &format!("whisper:{id}"), &progress).await?;
        models::ensure(&self.models_dir, &vad_model_file(), VAD_MODEL_ID, &progress).await?;
        self.reconcile_hands_free();
        self.emit_status();
        Ok(())
    }

    /// Names of available input devices for the Settings picker.
    pub fn input_devices(&self) -> Vec<String> {
        self.mic().devices()
    }

    // ---- internals ----------------------------------------------------------------------

    fn with_state<T>(&self, f: impl FnOnce(&mut State) -> T) -> T
    where
        T: Default,
    {
        match self.state.lock() {
            Ok(mut st) => f(&mut st),
            Err(_) => T::default(),
        }
    }

    fn remember_runtime(&self) {
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            if let Ok(mut g) = self.runtime.lock() {
                *g = Some(h);
            }
        }
    }

    fn runtime(&self) -> Option<tokio::runtime::Handle> {
        self.runtime.lock().ok().and_then(|g| g.clone())
    }

    fn mic(&self) -> Arc<dyn MicSource> {
        match self.mic.read() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    fn stt_present(&self, model: &str) -> bool {
        stt_model_file(model).is_some_and(|f| models::is_present(&self.models_dir, &f))
    }

    fn emit_status(&self) {
        let status = self.status();
        self.sync_character(status.phase);
        self.sink.emit(CoreEvent::Voice(status));
    }

    /// Show `Listening` on the character while recording or transcribing. Clear it only if we
    /// set it and the turn ended without a reply (`Thinking`/`Talking` belong to the orchestrator
    /// and speaker, which replace the overlay themselves).
    fn sync_character(&self, phase: VoicePhase) {
        use std::sync::atomic::Ordering;
        let Some(core) = self.core.lock().ok().and_then(|g| g.upgrade()) else {
            return;
        };
        match phase {
            VoicePhase::Listening | VoicePhase::Transcribing => {
                self.listening_overlay.store(true, Ordering::SeqCst);
                core.engine()
                    .set_overlay(Some(crate::character::CharacterState::Listening));
            }
            VoicePhase::Ready | VoicePhase::Off => {
                if self.listening_overlay.swap(false, Ordering::SeqCst) {
                    core.engine().set_overlay(None);
                }
            }
            VoicePhase::Thinking | VoicePhase::Speaking => {
                self.listening_overlay.store(false, Ordering::SeqCst);
            }
        }
    }

    fn set_phase(&self, phase: VoicePhase, problem: Option<&str>) {
        self.with_state(|st| {
            st.phase = phase;
            st.problem = problem.map(str::to_string);
        });
        self.emit_status();
    }

    /// Set the phase only if no newer utterance has started.
    fn set_phase_for_turn(&self, turn: u64, phase: VoicePhase) -> bool {
        let current = self.with_state(|st| {
            let same = st.turn == turn;
            if same {
                st.phase = phase;
                st.problem = None;
            }
            same
        });
        if current {
            self.emit_status();
        }
        current
    }

    fn ensure_mic(&self) -> std::result::Result<(), AudioError> {
        let mut running = self
            .mic_running
            .lock()
            .map_err(|_| AudioError::Device("microphone state".into()))?;
        if *running {
            return Ok(());
        }
        let capture = Arc::clone(&self.capture);
        self.mic()
            .start(Box::new(move |frame: &[f32]| capture.on_frame(frame)))?;
        *running = true;
        Ok(())
    }

    /// Close the mic unless push-to-talk or hands-free still needs it.
    fn release_mic(&self) {
        let needed = self.capture.ptt_on.load(Ordering::SeqCst)
            || self
                .capture
                .hands_free
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false);
        if needed {
            return;
        }
        if let Ok(mut running) = self.mic_running.lock() {
            if *running {
                self.mic().stop();
                *running = false;
                if let Ok(mut m) = self.capture.meter.lock() {
                    m.clear();
                }
                self.sink.emit(CoreEvent::Level(AudioLevel {
                    source: LevelSource::Mic,
                    level: 0.0,
                }));
            }
        }
    }

    async fn stt(&self) -> std::result::Result<Arc<dyn SttProvider>, AudioError> {
        let model = self.with_state(|st| st.settings.stt_model.clone());
        let mut cached = self.stt.lock().await;
        if let Some((id, provider)) = cached.as_ref() {
            if *id == model {
                return Ok(Arc::clone(provider));
            }
        }
        *cached = None;
        let file =
            stt_model_file(&model).ok_or_else(|| AudioError::ModelMissing("speech".into()))?;
        let path = models::path_of(&self.models_dir, &file);
        if !path.is_file() {
            return Err(AudioError::ModelMissing("speech".into()));
        }
        let loader = match self.stt_loader.read() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        };
        let provider = tokio::task::spawn_blocking(move || loader(&path))
            .await
            .map_err(|e| AudioError::Stt(e.to_string()))??;
        *cached = Some((model, Arc::clone(&provider)));
        Ok(provider)
    }

    fn expect_speech(&self) -> bool {
        let s = self.with_state(|st| st.settings.clone());
        s.enabled && !s.mute && s.speak_replies != SpeakReplies::Never && self.speaker.ready()
    }

    fn not_heard(&self, origin: Origin) -> Result<String> {
        match origin {
            Origin::PushToTalk => self.set_phase(VoicePhase::Ready, Some(PROBLEM_NOT_HEARD)),
            // Hands-free hears plenty of noise; don't nag about it.
            Origin::HandsFree => self.set_phase(VoicePhase::Ready, None),
        }
        Ok(String::new())
    }

    /// Transcribe one utterance and, if it has words in it, send it as a voice turn.
    async fn handle_utterance(&self, clip: AudioClip, origin: Origin) -> Result<String> {
        let turn = self.with_state(|st| {
            if origin == Origin::HandsFree {
                st.turn += 1;
            }
            st.turn
        });
        if clip.duration_secs() < MIN_CLIP_SECS {
            return self.not_heard(origin);
        }
        self.set_phase(VoicePhase::Transcribing, None);
        let stt = match self.stt().await {
            Ok(s) => s,
            Err(e) => {
                self.set_phase(VoicePhase::Ready, Some(problem_for(&e)));
                return Err(e.into());
            }
        };
        // M2 has no cloud speech consent: only local engines may hear the user.
        if let Err(e) = privacy::check(Modality::Audio, stt.locality(), Consent::default()) {
            self.set_phase(VoicePhase::Ready, Some(PROBLEM_STT_FAILED));
            return Err(AudioError::from(e).into());
        }
        let raw = match stt.transcribe(clip).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("speech-to-text failed");
                self.set_phase(VoicePhase::Ready, Some(PROBLEM_STT_FAILED));
                return Err(e.into());
            }
        };
        let mut text = clean_transcript(&raw);
        drop(raw);
        if text.is_empty() {
            return self.not_heard(origin);
        }
        if origin == Origin::HandsFree {
            match self.gate_wake_phrase(text) {
                Some(t) => text = t,
                None => {
                    self.set_phase(VoicePhase::Ready, None);
                    return Ok(String::new());
                }
            }
        }
        self.speaker.arm_next_reply();
        self.set_phase(VoicePhase::Thinking, None);
        self.spawn_send(text.clone(), turn);
        Ok(text)
    }

    /// Hands-free with the wake phrase on: keep only what was addressed to the companion.
    fn gate_wake_phrase(&self, text: String) -> Option<String> {
        let (wake_on, name, window_open) = self.with_state(|st| {
            let open = st.wake_open_until.is_some_and(|t| Instant::now() < t);
            (st.settings.wake_phrase, st.companion_name.clone(), open)
        });
        if !wake_on {
            return Some(text);
        }
        match match_wake(&text, &name) {
            WakeMatch::WithRest(rest) => {
                self.with_state(|st| st.wake_open_until = None);
                Some(rest)
            }
            WakeMatch::Only => {
                self.with_state(|st| st.wake_open_until = Some(Instant::now() + WAKE_WINDOW));
                None
            }
            WakeMatch::None if window_open => {
                self.with_state(|st| st.wake_open_until = None);
                Some(text)
            }
            WakeMatch::None => None,
        }
    }

    /// Send the turn and follow the reply: `Thinking` → `Speaking` → `Ready`.
    fn spawn_send(&self, text: String, turn: u64) {
        let Some(rt) = self.runtime() else {
            tracing::warn!("no async runtime to send the voice turn");
            self.set_phase(VoicePhase::Ready, None);
            return;
        };
        let me = self.me.clone();
        let core = self.core.lock().ok().and_then(|g| g.upgrade());
        rt.spawn(async move {
            let Some(this) = me.upgrade() else {
                return;
            };
            let expect_speech = this.expect_speech();
            let send = async move {
                match core {
                    Some(c) => c.send_message(&text, Via::Voice).await.map(|_| ()),
                    None => Err(CoreError::Invalid("voice isn't connected yet".into())),
                }
            };
            tokio::pin!(send);
            let mut sent_at: Option<Instant> = None;
            let mut saw_speech = false;
            let mut quiet = 0u32;
            let mut tick = tokio::time::interval(POLL);
            loop {
                tokio::select! {
                    r = &mut send, if sent_at.is_none() => {
                        sent_at = Some(Instant::now());
                        if r.is_err() && !this.speaker.is_speaking() {
                            // The orchestrator already told the UI (core://notice).
                            this.set_phase_for_turn(turn, VoicePhase::Ready);
                            return;
                        }
                    }
                    _ = tick.tick() => {}
                }
                if this.with_state(|st| st.turn) != turn {
                    return;
                }
                if this.speaker.is_speaking() {
                    quiet = 0;
                    if !saw_speech {
                        saw_speech = true;
                        this.set_phase_for_turn(turn, VoicePhase::Speaking);
                    }
                    continue;
                }
                let Some(at) = sent_at else { continue };
                quiet += 1;
                let done = if saw_speech {
                    quiet >= QUIET_POLLS
                } else {
                    !expect_speech || at.elapsed() >= SPEECH_WAIT
                };
                if done {
                    this.set_phase_for_turn(turn, VoicePhase::Ready);
                    return;
                }
            }
        });
    }

    // ---- hands-free -------------------------------------------------------------------

    fn reconcile_hands_free(&self) {
        let (want, running, model) = self.with_state(|st| {
            (
                st.settings.enabled && st.settings.hands_free,
                st.hands_free,
                st.settings.stt_model.clone(),
            )
        });
        if want && !running {
            if let Err(problem) = self.start_hands_free(&model) {
                self.with_state(|st| st.problem = Some(problem.to_string()));
            }
        } else if !want && running {
            self.stop_hands_free();
        }
    }

    fn start_hands_free(&self, model: &str) -> std::result::Result<(), &'static str> {
        if !self.stt_present(model) {
            return Err(PROBLEM_STT_MISSING);
        }
        let vad_path = models::path_of(&self.models_dir, &vad_model_file());
        if !vad_path.is_file() {
            return Err(PROBLEM_VAD_MISSING);
        }
        if self.runtime().is_none() {
            return Err(PROBLEM_HANDS_FREE);
        }
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(256);
        if let Ok(mut g) = self.capture.hands_free.lock() {
            *g = Some(tx);
        }
        if let Err(e) = self.ensure_mic() {
            if let Ok(mut g) = self.capture.hands_free.lock() {
                *g = None;
            }
            return Err(problem_for(&e));
        }
        let loader = match self.vad_loader.read() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        };
        let me = self.me.clone();
        let spawned = std::thread::Builder::new()
            .name("tandem-vad".into())
            .spawn(move || hands_free_loop(me, rx, loader, vad_path));
        if spawned.is_err() {
            self.stop_hands_free();
            return Err(PROBLEM_HANDS_FREE);
        }
        self.with_state(|st| {
            st.hands_free = true;
            st.problem = None;
        });
        Ok(())
    }

    fn stop_hands_free(&self) {
        // Dropping the sender ends the VAD thread.
        if let Ok(mut g) = self.capture.hands_free.lock() {
            *g = None;
        }
        self.with_state(|st| {
            st.hands_free = false;
            st.wake_open_until = None;
        });
        self.release_mic();
    }

    fn hands_free_failed(&self) {
        self.stop_hands_free();
        self.set_phase(VoicePhase::Ready, Some(PROBLEM_HANDS_FREE));
    }

    fn on_speech_start(&self) {
        let busy = self.with_state(|st| st.ptt_active);
        if !busy {
            self.set_phase(VoicePhase::Listening, None);
        }
    }

    fn on_segment(&self, clip: AudioClip) {
        let Some(rt) = self.runtime() else {
            return;
        };
        let me = self.me.clone();
        rt.spawn(async move {
            if let Some(this) = me.upgrade() {
                if this
                    .handle_utterance(clip, Origin::HandsFree)
                    .await
                    .is_err()
                {
                    tracing::info!("hands-free utterance wasn't sent");
                }
            }
        });
    }
}

/// Runs on the `tandem-vad` thread until hands-free stops (the sender is dropped).
fn hands_free_loop(
    me: Weak<VoiceController>,
    rx: mpsc::Receiver<Vec<f32>>,
    loader: VadLoader,
    path: PathBuf,
) {
    let mut vad = match loader(&path) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("voice detection model failed to load");
            if let Some(this) = me.upgrade() {
                this.hands_free_failed();
            }
            return;
        }
    };
    let mut seg = Segmenter::new(SegmenterConfig::default());
    let mut barge = BargeIn::default();
    let mut pending: Vec<f32> = Vec::with_capacity(VAD_FRAME * 4);
    let mut started_while_speaking = false;
    let mut barged = false;
    while let Ok(frame) = rx.recv() {
        pending.extend_from_slice(&frame);
        while pending.len() >= VAD_FRAME {
            let chunk: Vec<f32> = pending.drain(..VAD_FRAME).collect();
            let Some(this) = me.upgrade() else {
                return;
            };
            if this.capture.ptt_on.load(Ordering::SeqCst) {
                seg.reset();
                continue;
            }
            let prob = vad.process(&chunk).unwrap_or(0.0);
            let speaking = this.speaker.is_speaking();
            let barge_on = this.with_state(|st| st.settings.barge_in);
            if barge_on && barge.push(prob, speaking) {
                this.speaker.stop();
                barged = true;
                this.on_speech_start();
            }
            match seg.push(prob, &chunk) {
                Some(SegmentEvent::SpeechStart) => {
                    started_while_speaking = speaking;
                    if !speaking {
                        this.on_speech_start();
                    }
                }
                Some(SegmentEvent::SpeechEnd(clip)) => {
                    // Speech that began while the companion talked is likely its own echo,
                    // unless the user clearly talked over it (barge-in).
                    let keep = !started_while_speaking || barged;
                    started_while_speaking = false;
                    barged = false;
                    if keep {
                        this.on_segment(clip);
                    } else {
                        drop(clip);
                    }
                }
                None => {}
            }
        }
    }
    vad.reset();
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;

    use super::*;
    use crate::audio::capture::fake::{FakeFailure, FakeMic};
    use crate::conversation::ChatRole;
    use crate::events::CollectingSink;
    use crate::providers::mock::MockProvider;
    use crate::providers::Locality;
    use crate::secrets::MemorySecretStore;
    use crate::settings::Settings;
    use crate::store::Store;

    #[derive(Default)]
    struct FakeSpeaker {
        stops: AtomicUsize,
        arms: AtomicUsize,
        speaking: AtomicBool,
    }
    impl Speaker for FakeSpeaker {
        fn observe(&self, _e: &CoreEvent) {}
        fn arm_next_reply(&self) {
            self.arms.fetch_add(1, Ordering::SeqCst);
        }
        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.speaking.store(false, Ordering::SeqCst);
        }
        fn is_speaking(&self) -> bool {
            self.speaking.load(Ordering::SeqCst)
        }
        fn ready(&self) -> bool {
            false
        }
    }

    /// Returns scripted transcripts in order, then the last one forever.
    struct FakeStt {
        replies: Mutex<VecDeque<String>>,
        calls: AtomicUsize,
        locality: Locality,
    }
    impl FakeStt {
        fn new(replies: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.iter().map(|s| s.to_string()).collect()),
                calls: AtomicUsize::new(0),
                locality: Locality::Local,
            })
        }
    }
    #[async_trait]
    impl SttProvider for FakeStt {
        fn locality(&self) -> Locality {
            self.locality
        }
        async fn transcribe(&self, _clip: AudioClip) -> std::result::Result<String, AudioError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut q = self
                .replies
                .lock()
                .map_err(|_| AudioError::Stt("lock".into()))?;
            let next = if q.len() > 1 {
                q.pop_front()
            } else {
                q.front().cloned()
            };
            Ok(next.unwrap_or_default())
        }
    }

    /// Speech probability from loudness, so tests can script speech with amplitude.
    struct LoudnessVad;
    impl VadModel for LoudnessVad {
        fn process(&mut self, frame: &[f32]) -> std::result::Result<f32, AudioError> {
            Ok(if rms_level(frame) > 0.05 { 0.95 } else { 0.0 })
        }
        fn reset(&mut self) {}
    }

    fn stt_loader(stt: Arc<FakeStt>) -> SttLoader {
        Arc::new(move |_p: &Path| Ok(stt.clone() as Arc<dyn SttProvider>))
    }

    fn vad_loader() -> VadLoader {
        Arc::new(|_p: &Path| Ok(Box::new(LoudnessVad) as Box<dyn VadModel>))
    }

    /// A models dir with (fake) speech and VAD model files in place.
    fn models_dir_with_models() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        for f in [
            stt_model_file("base.en").unwrap_or_else(|| panic!()),
            vad_model_file(),
        ] {
            let p = models::path_of(dir.path(), &f);
            std::fs::create_dir_all(p.parent().unwrap_or_else(|| panic!()))
                .unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(&p, b"fake").unwrap_or_else(|e| panic!("{e}"));
        }
        dir
    }

    fn enabled() -> VoiceSettings {
        VoiceSettings {
            enabled: true,
            ..VoiceSettings::default()
        }
    }

    struct Rig {
        sink: Arc<CollectingSink>,
        speaker: Arc<FakeSpeaker>,
        mic: Arc<FakeMic>,
        stt: Arc<FakeStt>,
        ctl: Arc<VoiceController>,
        _dir: tempfile::TempDir,
    }

    fn rig_with(
        mic: FakeMic,
        stt: Arc<FakeStt>,
        voice: VoiceSettings,
        dir: tempfile::TempDir,
    ) -> Rig {
        let sink = Arc::new(CollectingSink::default());
        let speaker = Arc::new(FakeSpeaker::default());
        let mic = Arc::new(mic);
        let ctl = VoiceController::with_backends(
            sink.clone(),
            speaker.clone(),
            dir.path().to_path_buf(),
            mic.clone(),
            stt_loader(stt.clone()),
            vad_loader(),
        );
        ctl.apply_settings(&voice, &Profile::default());
        Rig {
            sink,
            speaker,
            mic,
            stt,
            ctl,
            _dir: dir,
        }
    }

    fn rig(stt: &[&str]) -> Rig {
        rig_with(
            FakeMic::default(),
            FakeStt::new(stt),
            enabled(),
            models_dir_with_models(),
        )
    }

    fn speech(secs: f32) -> Vec<f32> {
        (0..(secs * STT_SAMPLE_RATE as f32) as usize)
            .map(|i| 0.3 * (i as f32 * 0.07).sin())
            .collect()
    }

    fn last_status(sink: &CollectingSink) -> Option<VoiceStatus> {
        sink.take().into_iter().rev().find_map(|e| match e {
            CoreEvent::Voice(s) => Some(s),
            _ => None,
        })
    }

    #[tokio::test]
    async fn ptt_start_stops_speaker() {
        let r = rig(&["hello"]);
        r.speaker.speaking.store(true, Ordering::SeqCst);
        assert!(r.ctl.ptt_start().await.is_ok());
        assert_eq!(r.speaker.stops.load(Ordering::SeqCst), 1);
        assert!(!r.speaker.is_speaking());
        assert!(r.mic.is_running());
        assert_eq!(r.ctl.status().phase, VoicePhase::Listening);
    }

    #[tokio::test]
    async fn short_clip_sends_nothing() {
        let r = rig(&["hello there"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(0.2));
        let text = r.ctl.ptt_stop().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, "");
        assert_eq!(
            r.stt.calls.load(Ordering::SeqCst),
            0,
            "too short to transcribe"
        );
        assert_eq!(r.speaker.arms.load(Ordering::SeqCst), 0);
        assert!(!r.mic.is_running());
        let s = last_status(&r.sink).unwrap_or_else(|| panic!("no status"));
        assert_eq!(s.phase, VoicePhase::Ready);
        assert_eq!(s.problem.as_deref(), Some(PROBLEM_NOT_HEARD));
    }

    #[tokio::test]
    async fn hallucinated_transcript_sends_nothing() {
        let r = rig(&["[BLANK_AUDIO]"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(1.0));
        assert_eq!(r.ctl.ptt_stop().await.ok().as_deref(), Some(""));
        assert_eq!(r.stt.calls.load(Ordering::SeqCst), 1);
        assert_eq!(r.speaker.arms.load(Ordering::SeqCst), 0);
        let s = r.ctl.status();
        assert_eq!(
            (s.phase, s.problem.as_deref()),
            (VoicePhase::Ready, Some(PROBLEM_NOT_HEARD))
        );
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        let r = rig(&["hello"]);
        assert_eq!(r.ctl.ptt_stop().await.ok().as_deref(), Some(""));
        r.ctl.ptt_cancel();
        assert_eq!(r.ctl.status().phase, VoicePhase::Ready);
        assert_eq!(r.mic.starts.load(Ordering::SeqCst), 0);
        // And the normal flow still works afterwards.
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(0.6));
        assert_eq!(r.ctl.ptt_stop().await.ok().as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn double_start_is_ok() {
        let r = rig(&["one long thought"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(0.5));
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(0.5));
        assert_eq!(r.mic.starts.load(Ordering::SeqCst), 1, "mic opened once");
        assert_eq!(r.speaker.stops.load(Ordering::SeqCst), 1);
        let text = r.ctl.ptt_stop().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, "one long thought");
        assert_eq!(r.speaker.arms.load(Ordering::SeqCst), 1);
        // A second stop is a no-op.
        assert_eq!(r.ctl.ptt_stop().await.ok().as_deref(), Some(""));
        assert_eq!(r.speaker.arms.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn disabled_returns_error() {
        let r = rig_with(
            FakeMic::default(),
            FakeStt::new(&["hi"]),
            VoiceSettings::default(),
            models_dir_with_models(),
        );
        let err = r.ctl.ptt_start().await;
        assert!(
            matches!(err, Err(CoreError::Invalid(ref m)) if m == &AudioError::Disabled.to_string())
        );
        assert_eq!(r.mic.starts.load(Ordering::SeqCst), 0);
        let s = r.ctl.status();
        assert_eq!((s.phase, s.problem), (VoicePhase::Off, None));
    }

    #[tokio::test]
    async fn missing_model_reports_problem() {
        let empty = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let models = empty.path().to_path_buf();
        let r = rig_with(FakeMic::default(), FakeStt::new(&["hi"]), enabled(), empty);
        let s = r.ctl.status();
        assert_eq!(s.phase, VoicePhase::Off);
        assert!(!s.stt_ready);
        assert_eq!(s.problem.as_deref(), Some(PROBLEM_STT_MISSING));
        assert!(r.ctl.ptt_start().await.is_err());
        assert_eq!(r.mic.starts.load(Ordering::SeqCst), 0);
        assert!(r.ctl.stt_models().iter().all(|m| !m.downloaded));
        // Nothing was downloaded on its own.
        assert!(!r
            .sink
            .take()
            .iter()
            .any(|e| matches!(e, CoreEvent::Download(_))));
        assert_eq!(
            std::fs::read_dir(&models).map(|d| d.count()).unwrap_or(0),
            0
        );
        // Hands-free can't start either, and says why.
        let hf = VoiceSettings {
            hands_free: true,
            ..enabled()
        };
        r.ctl.apply_settings(&hf, &Profile::default());
        assert!(!r.ctl.status().hands_free);
    }

    #[tokio::test]
    async fn device_error_maps_to_problem() {
        for (failure, problem) in [
            (FakeFailure::NoDevice, PROBLEM_NO_MIC),
            (FakeFailure::Denied, PROBLEM_MIC_DENIED),
        ] {
            let r = rig_with(
                FakeMic::failing(failure),
                FakeStt::new(&["hi"]),
                enabled(),
                models_dir_with_models(),
            );
            assert!(r.ctl.ptt_start().await.is_err());
            let s = r.ctl.status();
            assert_eq!(s.phase, VoicePhase::Ready);
            assert_eq!(s.problem.as_deref(), Some(problem));
            // Not stuck: stop is a harmless no-op.
            assert_eq!(r.ctl.ptt_stop().await.ok().as_deref(), Some(""));
        }
    }

    #[tokio::test]
    async fn cancel_drops_recording() {
        let r = rig(&["should not be sent"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(1.0));
        r.ctl.ptt_cancel();
        assert!(!r.mic.is_running());
        assert_eq!(r.stt.calls.load(Ordering::SeqCst), 0);
        assert_eq!(r.ctl.status().phase, VoicePhase::Ready);
    }

    #[tokio::test]
    async fn disabling_mid_utterance_cancels() {
        let r = rig(&["hello"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.ctl
            .apply_settings(&VoiceSettings::default(), &Profile::default());
        assert!(!r.mic.is_running());
        assert_eq!(r.ctl.status().phase, VoicePhase::Off);
        assert_eq!(r.ctl.ptt_stop().await.ok().as_deref(), Some(""));
    }

    #[tokio::test]
    async fn cloud_stt_is_blocked_by_privacy_gate() {
        let stt = Arc::new(FakeStt {
            replies: Mutex::new(VecDeque::from(vec!["secret".to_string()])),
            calls: AtomicUsize::new(0),
            locality: Locality::Cloud,
        });
        let r = rig_with(FakeMic::default(), stt, enabled(), models_dir_with_models());
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(1.0));
        assert!(matches!(r.ctl.ptt_stop().await, Err(CoreError::Privacy(_))));
        assert_eq!(r.stt.calls.load(Ordering::SeqCst), 0, "audio never sent");
    }

    #[tokio::test]
    async fn mic_level_is_streamed() {
        let r = rig(&["hi"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(0.5));
        let levels: Vec<f32> = r
            .sink
            .take()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::Level(AudioLevel {
                    source: LevelSource::Mic,
                    level,
                }) => Some(level),
                _ => None,
            })
            .collect();
        assert!(levels.len() >= 14, "~30 Hz, got {}", levels.len());
        assert!(levels.iter().all(|l| *l > 0.1 && *l <= 1.0));
    }

    async fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
        for _ in 0..100 {
            if let Some(v) = f() {
                return Some(v);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        None
    }

    async fn voice_core(models: &Path) -> (Arc<CollectingSink>, Arc<Core>) {
        let sink = Arc::new(CollectingSink::default());
        let store = Arc::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")));
        let mut s = Settings::default();
        s.ollama.url = "http://127.0.0.1:9".into();
        s.ollama.chat_model = "test".into();
        s.voice.enabled = true;
        store
            .set_setting("app", &s)
            .unwrap_or_else(|e| panic!("{e}"));
        let core = Core::with_store_in(
            store,
            sink.clone(),
            Arc::new(MemorySecretStore::default()),
            models.to_path_buf(),
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        core.set_chat_provider(Arc::new(
            MockProvider::new(Locality::Local).with_text_reply("Nice. What's the first line?"),
        ));
        (sink, core)
    }

    #[tokio::test]
    async fn transcript_is_sent_as_voice_turn() {
        let dir = models_dir_with_models();
        let (sink, core) = voice_core(dir.path()).await;
        let mic = Arc::new(FakeMic::default());
        core.voice().set_backends(
            mic.clone(),
            stt_loader(FakeStt::new(&["Help me start my essay."])),
            vad_loader(),
        );
        assert!(core.voice().ptt_start().await.is_ok());
        mic.feed(&speech(1.2));
        let text = core
            .voice()
            .ptt_stop()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, "Help me start my essay.");
        let mut seen = Vec::new();
        let user = wait_for(|| {
            seen.extend(sink.take());
            seen.iter().find_map(|e| match e {
                CoreEvent::ChatMessage(m) if m.role == ChatRole::User => Some(m.clone()),
                _ => None,
            })
        })
        .await
        .unwrap_or_else(|| panic!("no user message"));
        assert_eq!(user.via, Via::Voice);
        assert_eq!(user.text, "Help me start my essay.");
        // The reply arrives and, with no voice ready to speak it, the phase returns to Ready.
        let ready = wait_for(|| {
            seen.extend(sink.take());
            let replied = seen
                .iter()
                .any(|e| matches!(e, CoreEvent::ChatMessage(m) if m.role == ChatRole::Assistant));
            (replied && core.voice().status().phase == VoicePhase::Ready).then_some(())
        })
        .await;
        assert!(
            ready.is_some(),
            "phase stuck at {:?}",
            core.voice().status().phase
        );
    }

    #[tokio::test]
    async fn speaking_phase_follows_speaker() {
        let r = rig(&["hello"]);
        assert!(r.ctl.ptt_start().await.is_ok());
        r.mic.feed(&speech(0.6));
        // The companion is already talking when the turn goes out (no core attached, so the
        // send fails fast): the phase follows the speaker instead of jumping to Ready.
        r.speaker.speaking.store(true, Ordering::SeqCst);
        assert!(r.ctl.ptt_stop().await.is_ok());
        let speaking =
            wait_for(|| (r.ctl.status().phase == VoicePhase::Speaking).then_some(())).await;
        assert!(speaking.is_some(), "phase {:?}", r.ctl.status().phase);
        r.speaker.speaking.store(false, Ordering::SeqCst);
        let ready = wait_for(|| (r.ctl.status().phase == VoicePhase::Ready).then_some(())).await;
        assert!(ready.is_some(), "phase {:?}", r.ctl.status().phase);
    }

    /// Top-level entries in `dir` (names only).
    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    fn all_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    #[tokio::test]
    async fn audio_never_written_to_disk() {
        let models = models_dir_with_models();
        let before = all_files(models.path());
        let scratch = tempfile::Builder::new()
            .prefix("tandem-tmpdir-")
            .tempdir()
            .unwrap_or_else(|e| panic!("{e}"));
        let old_tmp = std::env::var_os("TMPDIR");
        std::env::set_var("TMPDIR", scratch.path());

        let (sink, core) = voice_core(models.path()).await;
        let mic = Arc::new(FakeMic::default());
        core.voice().set_backends(
            mic.clone(),
            stt_loader(FakeStt::new(&["Let's do the dishes."])),
            vad_loader(),
        );
        let ok = core.voice().ptt_start().await.is_ok();
        mic.feed(&speech(1.5));
        let text = core.voice().ptt_stop().await;
        let mut seen = Vec::new();
        let replied = wait_for(|| {
            seen.extend(sink.take());
            seen.iter()
                .any(|e| matches!(e, CoreEvent::ChatMessage(m) if m.role == ChatRole::Assistant))
                .then_some(())
        })
        .await;

        match old_tmp {
            Some(v) => std::env::set_var("TMPDIR", v),
            None => std::env::remove_var("TMPDIR"),
        }
        assert!(ok);
        assert_eq!(text.ok().as_deref(), Some("Let's do the dishes."));
        assert!(replied.is_some());
        assert_eq!(all_files(models.path()), before, "models dir unchanged");
        // Other tests running in parallel may create their own `.tmp*` dirs while TMPDIR
        // points here; anything else would be ours.
        let ours: Vec<String> = entries(scratch.path())
            .into_iter()
            .filter(|n| !n.starts_with(".tmp"))
            .collect();
        assert!(ours.is_empty(), "unexpected files: {ours:?}");
        // Leave the dir in place if other tests are still using it.
        if !entries(scratch.path()).is_empty() {
            let _ = scratch.keep();
        }
    }

    #[tokio::test]
    async fn hands_free_wake_phrase_gates_segments() {
        let voice = VoiceSettings {
            hands_free: true,
            wake_phrase: true,
            ..enabled()
        };
        let r = rig_with(
            FakeMic::default(),
            FakeStt::new(&[
                "Hey Timber, start the essay.",
                "I wonder what's for lunch.",
                "Hey Timber.",
                "Do the dishes first.",
            ]),
            voice,
            models_dir_with_models(),
        );
        let mut profile = Profile::default();
        profile.companion.name = "Timber".into();
        r.ctl.apply_settings(
            &VoiceSettings {
                hands_free: true,
                wake_phrase: true,
                ..enabled()
            },
            &profile,
        );
        assert!(r.ctl.status().hands_free);
        assert!(r.mic.is_running());
        let silence = vec![0.0f32; STT_SAMPLE_RATE as usize];
        for i in 1..=4 {
            r.mic.feed(&speech(1.0));
            r.mic.feed(&silence);
            let done = wait_for(|| (r.stt.calls.load(Ordering::SeqCst) >= i).then_some(())).await;
            assert!(done.is_some(), "segment {i} not transcribed");
            // Let the transcript be handled before the next segment.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // 1: wake + request (sent). 2: no wake (dropped). 3: wake only (opens window).
        // 4: inside the window (sent).
        assert_eq!(r.speaker.arms.load(Ordering::SeqCst), 2);

        // Turning hands-free off closes the mic.
        r.ctl.apply_settings(&enabled(), &profile);
        assert!(!r.ctl.status().hands_free);
        assert!(!r.mic.is_running());
    }

    #[tokio::test]
    async fn hands_free_barge_in_stops_speaker() {
        let voice = VoiceSettings {
            hands_free: true,
            barge_in: true,
            ..enabled()
        };
        let r = rig_with(
            FakeMic::default(),
            FakeStt::new(&["wait, stop"]),
            voice,
            models_dir_with_models(),
        );
        assert!(r.ctl.status().hands_free);
        r.speaker.speaking.store(true, Ordering::SeqCst);
        r.mic.feed(&speech(0.6));
        let stopped = wait_for(|| (r.speaker.stops.load(Ordering::SeqCst) > 0).then_some(())).await;
        assert!(stopped.is_some(), "barge-in should stop the speaker");
    }

    #[tokio::test]
    async fn hands_free_needs_vad_model() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let f = stt_model_file("base.en").unwrap_or_else(|| panic!());
        let p = models::path_of(dir.path(), &f);
        std::fs::create_dir_all(p.parent().unwrap_or_else(|| panic!()))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&p, b"fake").unwrap_or_else(|e| panic!("{e}"));
        let voice = VoiceSettings {
            hands_free: true,
            ..enabled()
        };
        let r = rig_with(FakeMic::default(), FakeStt::new(&["hi"]), voice, dir);
        let s = r.ctl.status();
        assert!(!s.hands_free);
        assert_eq!(s.problem.as_deref(), Some(PROBLEM_VAD_MISSING));
        assert!(!r.mic.is_running());
        // Push-to-talk still works.
        assert!(r.ctl.ptt_start().await.is_ok());
    }

    #[tokio::test]
    async fn unknown_model_download_is_rejected() {
        let r = rig(&["hi"]);
        assert!(r.ctl.download_stt_model("../evil").await.is_err());
        assert!(r
            .sink
            .take()
            .iter()
            .all(|e| !matches!(e, CoreEvent::Download(_))));
    }
}
