//! OWNER: output agent. Sentence-streamed text-to-speech with playback and mouth levels
//! (plan task O4). The public API is a contract the commands layer (UI agent) and `Core`
//! build against: keep these names and signatures.
//!
//! Flow: chat events (via `SpeechTap`) → `observe` decides whether this reply is spoken →
//! `SentenceSplitter` → job queue → synth task (`TtsProvider`, one sentence ahead) → play task
//! (`AudioOut`, blocking) → `CoreEvent::Level { Speaker }` on the raw sink.
//!
//! `stop()` bumps a generation counter: everything queued or in flight with an older
//! generation is dropped, playback stops within ~10 ms and the `Talking` overlay is cleared.
//! Audio stays in memory; nothing here writes or logs it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::mpsc;

use super::playback::{AudioOut, CpalOut};
use super::sentences::SentenceSplitter;
use super::tts::{self, PiperTts};
use super::{
    models, AudioClip, AudioError, AudioLevel, LevelSource, SpeakOpts, Speaker, TtsProvider,
    VoiceInfo,
};
use crate::character::CharacterState;
use crate::conversation::{ChatMessageDto, ChatRole, Via};
use crate::error::{CoreError, Result};
use crate::events::{CoreEvent, CoreNotice, EventSink};
use crate::session::engine::SessionEngine;
use crate::settings::{SpeakReplies, VoiceSettings};

/// i18n keys for notices this module emits (the UI lane adds them to `en.json`).
pub const NOTICE_TTS_FAILED: &str = "voice.error.tts_failed";
pub const NOTICE_NO_SPEAKER: &str = "voice.error.no_speaker";

/// Where synthesis comes from: real Piper voices on disk, or a fixed provider in tests.
enum TtsSource {
    Piper,
    #[cfg(test)]
    Fixed(Arc<dyn TtsProvider>),
}

#[derive(Debug, Clone)]
struct Config {
    voice: VoiceSettings,
    /// Resolved catalog id (falls back to the default voice).
    voice_id: String,
}

/// Speech state for the current user turn.
#[derive(Default)]
struct Turn {
    /// This turn's reply should be spoken.
    speak: bool,
    /// Message id of the deltas being split right now.
    msg_id: Option<String>,
    splitter: SentenceSplitter,
    /// Raw delta text received for `msg_id`.
    streamed: String,
}

struct Job {
    gen: u64,
    text: String,
    voice_id: String,
    rate: f32,
    volume: f32,
}

struct Clip {
    gen: u64,
    clip: AudioClip,
    volume: f32,
}

pub struct SpeakerPipeline {
    me: Weak<SpeakerPipeline>,
    /// The un-tapped sink (emitting through it never loops back into `observe`).
    sink: Arc<dyn EventSink>,
    models_dir: PathBuf,
    engine: Mutex<Option<Arc<SessionEngine>>>,
    config: Mutex<Config>,
    armed: AtomicBool,
    turn: Mutex<Turn>,
    gen: AtomicU64,
    /// Sentences of the current generation queued, synthesising or playing.
    pending: AtomicUsize,
    /// Serialises overlay changes against `stop()` so a late level can't re-set `Talking`.
    overlay_lock: Mutex<()>,
    queue: Mutex<Option<mpsc::UnboundedSender<Job>>>,
    source: TtsSource,
    loaded: tokio::sync::Mutex<Option<(String, Arc<dyn TtsProvider>)>>,
    out: Arc<dyn AudioOut>,
}

impl SpeakerPipeline {
    /// Must not open any audio device; devices open on first playback.
    pub fn new(sink: Arc<dyn EventSink>, models_dir: PathBuf) -> Arc<Self> {
        Self::build(sink, models_dir, TtsSource::Piper, Arc::new(CpalOut::new()))
    }

    fn build(
        sink: Arc<dyn EventSink>,
        models_dir: PathBuf,
        source: TtsSource,
        out: Arc<dyn AudioOut>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            sink,
            models_dir,
            engine: Mutex::new(None),
            config: Mutex::new(Config {
                voice: VoiceSettings::default(),
                voice_id: tts::default_voice().id.to_string(),
            }),
            armed: AtomicBool::new(false),
            turn: Mutex::new(Turn::default()),
            gen: AtomicU64::new(0),
            pending: AtomicUsize::new(0),
            overlay_lock: Mutex::new(()),
            queue: Mutex::new(None),
            source,
            loaded: tokio::sync::Mutex::new(None),
            out,
        })
    }

    /// Called once by `Core`; used to set the `Talking` overlay while audio plays.
    pub fn attach_engine(&self, engine: Arc<SessionEngine>) {
        if let Ok(mut g) = self.engine.lock() {
            *g = Some(engine);
        }
    }

    /// Called at startup and whenever settings or `profile.companion.voice_id` change.
    /// `voice_id: None` = the default voice.
    pub fn apply_settings(&self, voice: &VoiceSettings, voice_id: Option<&str>) {
        let id = voice_id
            .and_then(tts::find_voice)
            .unwrap_or_else(tts::default_voice)
            .id
            .to_string();
        let silenced = !voice.enabled || voice.mute;
        if let Ok(mut g) = self.config.lock() {
            *g = Config {
                voice: voice.clone(),
                voice_id: id.clone(),
            };
        }
        if silenced && self.is_speaking() {
            self.stop();
        }
        // Warm the voice so the first reply isn't delayed by model loading (~0.5 s).
        if !silenced && self.voice_ready(&id) {
            if let (Ok(handle), Some(me)) =
                (tokio::runtime::Handle::try_current(), self.me.upgrade())
            {
                handle.spawn(async move {
                    if let Err(e) = me.provider(&id).await {
                        tracing::warn!("couldn't preload voice: {e}");
                    }
                });
            }
        }
    }

    pub fn voices(&self) -> Vec<VoiceInfo> {
        tts::voice_catalog()
            .iter()
            .map(|v| VoiceInfo {
                id: v.id.to_string(),
                label: v.label.to_string(),
                language: v.language.to_string(),
                license: v.license.to_string(),
                size_bytes: v.onnx_bytes + v.json_bytes,
                downloaded: v.is_downloaded(&self.models_dir),
                is_default: v.is_default,
            })
            .collect()
    }

    /// User-initiated download of a voice; progress goes out on `voice://download`.
    pub async fn download_voice(&self, id: &str) -> Result<()> {
        let v =
            tts::find_voice(id).ok_or_else(|| CoreError::Invalid(format!("unknown voice {id}")))?;
        let model = v.model_id();
        let sink = self.sink.clone();
        // The big model first. Its "finished" is held back so the UI sees one finish at the end.
        let s = sink.clone();
        models::ensure(&self.models_dir, &v.onnx_file(), &model, &move |p| {
            if !p.finished || p.error.is_some() {
                s.emit(CoreEvent::Download(p));
            }
        })
        .await?;
        let s = sink.clone();
        models::ensure(&self.models_dir, &v.json_file(), &model, &move |p| {
            if p.finished {
                s.emit(CoreEvent::Download(p));
            }
        })
        .await?;
        Ok(())
    }

    /// Speak `text` once with voice `id` (Settings "Preview" button). Ignores mute.
    pub async fn preview_voice(&self, id: &str, text: &str) -> Result<()> {
        let v =
            tts::find_voice(id).ok_or_else(|| CoreError::Invalid(format!("unknown voice {id}")))?;
        if !self.voice_ready(v.id) {
            return Err(AudioError::ModelMissing("voice".into()).into());
        }
        self.stop();
        let mut splitter = SentenceSplitter::new();
        let mut sentences = splitter.push(text);
        sentences.extend(splitter.finish());
        for s in sentences {
            self.enqueue(s, Some(v.id.to_string()));
        }
        Ok(())
    }

    // --- internals --------------------------------------------------------------------------

    fn config(&self) -> Config {
        match self.config.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    fn current_gen(&self) -> u64 {
        self.gen.load(Ordering::SeqCst)
    }

    fn voice_ready(&self, id: &str) -> bool {
        match &self.source {
            TtsSource::Piper => {
                tts::find_voice(id).is_some_and(|v| v.is_downloaded(&self.models_dir))
            }
            #[cfg(test)]
            TtsSource::Fixed(_) => true,
        }
    }

    fn engine(&self) -> Option<Arc<SessionEngine>> {
        self.engine.lock().ok().and_then(|g| g.clone())
    }

    /// Load (or reuse) the TTS provider for `id`.
    async fn provider(&self, id: &str) -> std::result::Result<Arc<dyn TtsProvider>, AudioError> {
        let mut loaded = self.loaded.lock().await;
        if let Some((cur, p)) = loaded.as_ref() {
            if cur == id {
                return Ok(p.clone());
            }
        }
        let provider: Arc<dyn TtsProvider> = match &self.source {
            TtsSource::Piper => {
                let v =
                    tts::find_voice(id).ok_or_else(|| AudioError::ModelMissing("voice".into()))?;
                let (onnx, json) = v.paths(&self.models_dir);
                let tts = tokio::task::spawn_blocking(move || PiperTts::load(&onnx, &json))
                    .await
                    .map_err(|e| AudioError::Tts(e.to_string()))??;
                Arc::new(tts)
            }
            #[cfg(test)]
            TtsSource::Fixed(p) => p.clone(),
        };
        *loaded = Some((id.to_string(), provider.clone()));
        Ok(provider)
    }

    /// Queue one sentence. Does nothing if the voice isn't downloaded (the controller
    /// reports `voice.error.voice_missing`) or there's no async runtime.
    fn enqueue(&self, text: String, voice_override: Option<String>) {
        if text.trim().is_empty() {
            return;
        }
        let cfg = self.config();
        let voice_id = voice_override.unwrap_or(cfg.voice_id);
        if !self.voice_ready(&voice_id) {
            return;
        }
        let Some(tx) = self.worker() else {
            return;
        };
        self.pending.fetch_add(1, Ordering::SeqCst);
        let job = Job {
            gen: self.current_gen(),
            text,
            voice_id,
            rate: cfg.voice.speech_rate,
            volume: cfg.voice.volume,
        };
        if tx.send(job).is_err() {
            self.job_done(self.current_gen());
        }
    }

    /// The job sender, starting the synth and play tasks on first use.
    fn worker(&self) -> Option<mpsc::UnboundedSender<Job>> {
        let mut q = self.queue.lock().ok()?;
        if let Some(tx) = q.as_ref() {
            if !tx.is_closed() {
                return Some(tx.clone());
            }
        }
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let (tx, mut jobs) = mpsc::unbounded_channel::<Job>();
        // Capacity 1: sentence n+1 synthesises while sentence n plays, and no further ahead.
        let (clip_tx, mut clips) = mpsc::channel::<Clip>(1);

        let me = self.me.clone();
        handle.spawn(async move {
            while let Some(job) = jobs.recv().await {
                let Some(this) = me.upgrade() else { break };
                if job.gen != this.current_gen() {
                    continue;
                }
                let result = match this.provider(&job.voice_id).await {
                    Ok(p) => {
                        let opts = SpeakOpts {
                            voice: job.voice_id.clone(),
                            rate: job.rate,
                        };
                        p.synthesize(&job.text, &opts).await
                    }
                    Err(e) => Err(e),
                };
                match result {
                    Ok(clip) if job.gen == this.current_gen() => {
                        let clip = Clip {
                            gen: job.gen,
                            clip,
                            volume: job.volume,
                        };
                        drop(this);
                        if clip_tx.send(clip).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("text-to-speech failed: {e}");
                        this.notice(NOTICE_TTS_FAILED);
                        this.job_done(job.gen);
                    }
                }
            }
        });

        let me = self.me.clone();
        handle.spawn(async move {
            while let Some(c) = clips.recv().await {
                let Some(this) = me.upgrade() else { break };
                if c.gen != this.current_gen() {
                    continue;
                }
                this.set_talking(c.gen);
                let out = this.out.clone();
                let weak = this.me.clone();
                let gen = c.gen;
                let on_level: Box<dyn Fn(f32) + Send> = Box::new(move |level| {
                    if let Some(p) = weak.upgrade() {
                        p.on_level(gen, level);
                    }
                });
                let played =
                    tokio::task::spawn_blocking(move || out.play(c.clip, c.volume, on_level)).await;
                match played {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        tracing::warn!("speaker playback failed: {e}");
                        let key = match e {
                            AudioError::NoOutputDevice => NOTICE_NO_SPEAKER,
                            _ => NOTICE_TTS_FAILED,
                        };
                        this.notice(key);
                    }
                    Err(e) => tracing::warn!("playback task failed: {e}"),
                }
                this.job_done(gen);
            }
        });

        *q = Some(tx.clone());
        Some(tx)
    }

    fn on_level(&self, gen: u64, level: f32) {
        if gen != self.current_gen() {
            // Stale clip still playing after a stop: make sure it ends now.
            self.out.stop();
            return;
        }
        if level > 0.0 {
            // The orchestrator clears its overlay when the turn ends, which can be mid-speech.
            self.set_talking(gen);
        }
        self.emit_level(level);
    }

    fn emit_level(&self, level: f32) {
        self.sink.emit(CoreEvent::Level(AudioLevel {
            source: LevelSource::Speaker,
            level,
        }));
    }

    fn notice(&self, key: &str) {
        self.sink.emit(CoreEvent::Notice(CoreNotice {
            key: key.to_string(),
            detail: None,
        }));
    }

    fn set_talking(&self, gen: u64) {
        let Ok(_g) = self.overlay_lock.lock() else {
            return;
        };
        if gen != self.current_gen() {
            return;
        }
        if let Some(engine) = self.engine() {
            engine.set_overlay(Some(CharacterState::Talking));
        }
    }

    /// Clear `Talking`, but leave other overlays (e.g. `Thinking` for a new turn) alone.
    fn clear_talking(&self) {
        if let Some(engine) = self.engine() {
            if engine.character_state() == CharacterState::Talking {
                engine.set_overlay(None);
            }
        }
    }

    /// One sentence of generation `gen` finished (played, failed or dropped).
    fn job_done(&self, gen: u64) {
        let Ok(_g) = self.overlay_lock.lock() else {
            return;
        };
        if gen != self.current_gen() {
            return;
        }
        // Saturating decrement. A CAS loop instead of `fetch_update`, which newer toolchains
        // deprecate (renamed `try_update`) while older ones lack the new name.
        let mut prev = self.pending.load(Ordering::SeqCst);
        while let Err(actual) = self.pending.compare_exchange_weak(
            prev,
            prev.saturating_sub(1),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            prev = actual;
        }
        if prev <= 1 {
            self.clear_talking();
            self.emit_level(0.0);
        }
    }

    fn speak_text(&self, text: &str) {
        let mut splitter = SentenceSplitter::new();
        for s in splitter.push(text) {
            self.enqueue(s, None);
        }
        if let Some(s) = splitter.finish() {
            self.enqueue(s, None);
        }
    }

    fn on_user_message(&self, m: &ChatMessageDto, cfg: &Config) {
        let armed = self.armed.swap(false, Ordering::SeqCst);
        let speak = match cfg.voice.speak_replies {
            SpeakReplies::Always => true,
            SpeakReplies::VoiceTurns => m.via == Via::Voice || armed,
            SpeakReplies::Never => false,
        };
        if let Ok(mut t) = self.turn.lock() {
            *t = Turn {
                speak,
                ..Turn::default()
            };
        }
    }

    fn on_delta(&self, message_id: &str, delta: &str) {
        let mut out = Vec::new();
        {
            let Ok(mut t) = self.turn.lock() else { return };
            if !t.speak {
                return;
            }
            if t.msg_id.as_deref() != Some(message_id) {
                // A new assistant message in the same turn (after a tool call): flush the old one.
                out.extend(t.splitter.finish());
                t.msg_id = Some(message_id.to_string());
                t.splitter = SentenceSplitter::new();
                t.streamed.clear();
            }
            t.streamed.push_str(delta);
            out.extend(t.splitter.push(delta));
        }
        for s in out {
            self.enqueue(s, None);
        }
    }

    fn on_assistant_message(&self, m: &ChatMessageDto) {
        let mut out = Vec::new();
        let mut speak_whole = false;
        {
            let Ok(mut t) = self.turn.lock() else { return };
            if !t.speak {
                return;
            }
            out.extend(t.splitter.finish());
            if t.msg_id.as_deref() == Some(m.id.as_str()) {
                // Streamed already. Speak anything the final text added (e.g. crisis resources).
                let streamed = t.streamed.trim();
                if let Some(extra) = m.text.trim_start().strip_prefix(streamed) {
                    if !extra.trim().is_empty() {
                        let mut s = SentenceSplitter::new();
                        out.extend(s.push(extra));
                        out.extend(s.finish());
                    }
                }
            } else {
                // No deltas for this message (non-native-tool providers): speak the final text.
                speak_whole = true;
            }
            *t = Turn::default();
        }
        for s in out {
            self.enqueue(s, None);
        }
        if speak_whole {
            self.speak_text(&m.text);
        }
    }
}

impl Speaker for SpeakerPipeline {
    fn observe(&self, event: &CoreEvent) {
        let cfg = self.config();
        let v = &cfg.voice;
        if !v.enabled || v.mute || v.speak_replies == SpeakReplies::Never {
            self.armed.store(false, Ordering::SeqCst);
            return;
        }
        match event {
            CoreEvent::ChatDelta(d) => self.on_delta(&d.message_id, &d.delta),
            CoreEvent::ChatMessage(m) => match (m.role, m.via) {
                (ChatRole::User, _) => self.on_user_message(m, &cfg),
                (ChatRole::Assistant, Via::Checkin) => {
                    if v.speak_checkins {
                        self.speak_text(&m.text);
                    }
                }
                (ChatRole::Assistant, _) => self.on_assistant_message(m),
            },
            _ => {}
        }
    }

    fn arm_next_reply(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn stop(&self) {
        {
            let _g = self.overlay_lock.lock();
            self.gen.fetch_add(1, Ordering::SeqCst);
            self.pending.store(0, Ordering::SeqCst);
            self.armed.store(false, Ordering::SeqCst);
            if let Ok(mut t) = self.turn.lock() {
                // Silence the rest of the reply that was being spoken.
                *t = Turn::default();
            }
            self.out.stop();
            self.clear_talking();
        }
        self.emit_level(0.0);
    }

    fn is_speaking(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }

    fn ready(&self) -> bool {
        self.voice_ready(&self.config().voice_id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use async_trait::async_trait;

    use super::*;
    use crate::audio::playback::{tone, FakeOut};
    use crate::conversation::ChatDelta;
    use crate::events::CollectingSink;
    use crate::providers::Locality;
    use crate::session::engine::EngineConfig;

    struct FakeTts {
        texts: Mutex<Vec<String>>,
        secs: f32,
    }

    #[async_trait]
    impl TtsProvider for FakeTts {
        fn locality(&self) -> Locality {
            Locality::Local
        }
        async fn synthesize(
            &self,
            text: &str,
            _opts: &SpeakOpts,
        ) -> std::result::Result<AudioClip, AudioError> {
            if let Ok(mut g) = self.texts.lock() {
                g.push(text.to_string());
            }
            Ok(tone(self.secs, 0.3, 22_050))
        }
    }

    struct Rig {
        speaker: Arc<SpeakerPipeline>,
        sink: Arc<CollectingSink>,
        out: Arc<FakeOut>,
        tts: Arc<FakeTts>,
        engine: Arc<SessionEngine>,
    }

    impl Rig {
        fn texts(&self) -> Vec<String> {
            self.tts.texts.lock().map(|g| g.clone()).unwrap_or_default()
        }
        async fn idle(&self) {
            let t0 = Instant::now();
            while self.speaker.is_speaking() && t0.elapsed() < Duration::from_secs(5) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(!self.speaker.is_speaking(), "still speaking after 5 s");
        }
    }

    fn voice(f: impl FnOnce(&mut VoiceSettings)) -> VoiceSettings {
        let mut v = VoiceSettings {
            enabled: true,
            ..VoiceSettings::default()
        };
        f(&mut v);
        v
    }

    fn rig_with(v: VoiceSettings, clip_secs: f32) -> Rig {
        let sink = Arc::new(CollectingSink::default());
        let out = Arc::new(FakeOut::new(1.0));
        let tts = Arc::new(FakeTts {
            texts: Mutex::new(Vec::new()),
            secs: clip_secs,
        });
        let speaker = SpeakerPipeline::build(
            sink.clone(),
            PathBuf::from("/nonexistent"),
            TtsSource::Fixed(tts.clone()),
            out.clone(),
        );
        let engine = Arc::new(SessionEngine::new(
            Arc::new(CollectingSink::default()),
            None,
            EngineConfig::default(),
        ));
        speaker.attach_engine(engine.clone());
        speaker.apply_settings(&v, None);
        Rig {
            speaker,
            sink,
            out,
            tts,
            engine,
        }
    }

    fn rig(v: VoiceSettings) -> Rig {
        rig_with(v, 0.05)
    }

    fn msg(id: &str, role: ChatRole, via: Via, text: &str) -> CoreEvent {
        CoreEvent::ChatMessage(ChatMessageDto {
            id: id.into(),
            role,
            text: text.into(),
            via,
            ts: String::new(),
        })
    }

    fn delta(id: &str, d: &str) -> CoreEvent {
        CoreEvent::ChatDelta(ChatDelta {
            message_id: id.into(),
            delta: d.into(),
        })
    }

    const REPLY: &str = "Let's open the essay draft together. Then write one rough sentence.";

    /// A streamed reply: user message, deltas, then the final assistant message.
    fn streamed_turn(s: &SpeakerPipeline, via: Via) {
        s.observe(&msg("u1", ChatRole::User, via, "help me start my essay"));
        s.observe(&delta("a1", "Let's open the essay "));
        s.observe(&delta("a1", "draft together. Then write one"));
        s.observe(&delta("a1", " rough sentence."));
        s.observe(&msg("a1", ChatRole::Assistant, via, REPLY));
    }

    fn expected() -> Vec<String> {
        vec![
            "Let's open the essay draft together.".to_string(),
            "Then write one rough sentence.".to_string(),
        ]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn voice_turn_reply_is_spoken() {
        let r = rig(voice(|_| {}));
        r.speaker.arm_next_reply();
        streamed_turn(&r.speaker, Via::Voice);
        assert!(r.speaker.is_speaking());
        r.idle().await;
        assert_eq!(r.texts(), expected());
        assert_eq!(r.out.played_count(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn typed_reply_not_spoken_by_default() {
        let r = rig(voice(|_| {}));
        streamed_turn(&r.speaker, Via::Text);
        assert!(!r.speaker.is_speaking());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(r.texts().is_empty());
        assert_eq!(r.out.played_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn always_speaks_typed() {
        let r = rig(voice(|v| v.speak_replies = SpeakReplies::Always));
        streamed_turn(&r.speaker, Via::Text);
        r.idle().await;
        assert_eq!(r.texts(), expected());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mute_speaks_nothing() {
        for v in [
            voice(|v| {
                v.speak_replies = SpeakReplies::Always;
                v.mute = true;
            }),
            voice(|v| v.speak_replies = SpeakReplies::Never),
            voice(|v| {
                v.speak_replies = SpeakReplies::Always;
                v.enabled = false;
            }),
        ] {
            let r = rig(v);
            r.speaker.arm_next_reply();
            streamed_turn(&r.speaker, Via::Voice);
            r.speaker.observe(&msg(
                "c1",
                ChatRole::Assistant,
                Via::Checkin,
                "Still with you.",
            ));
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(r.texts().is_empty());
            assert_eq!(r.out.played_count(), 0);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn checkin_spoken_when_enabled() {
        let line = "How's the outline going? I'm right here.";
        let r = rig(voice(|_| {}));
        r.speaker
            .observe(&msg("c1", ChatRole::Assistant, Via::Checkin, line));
        r.idle().await;
        assert_eq!(
            r.texts(),
            vec!["How's the outline going?", "I'm right here."]
        );

        let r = rig(voice(|v| v.speak_checkins = false));
        r.speaker
            .observe(&msg("c1", ChatRole::Assistant, Via::Checkin, line));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(r.texts().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_clears_queue_and_overlay() {
        let r = rig_with(voice(|_| {}), 0.5);
        r.speaker
            .observe(&msg("u1", ChatRole::User, Via::Voice, "hi"));
        r.speaker.observe(&msg(
            "a1",
            ChatRole::Assistant,
            Via::Voice,
            "Sentence number one is here. Sentence number two is here. Sentence three. And four.",
        ));
        let t0 = Instant::now();
        while r.out.played_count() == 0 && t0.elapsed() < Duration::from_secs(2) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(r.engine.character_state(), CharacterState::Talking);

        let t_stop = Instant::now();
        r.speaker.stop();
        assert!(!r.speaker.is_speaking());
        assert_ne!(r.engine.character_state(), CharacterState::Talking);
        // The fake player notices within its 5 ms poll.
        while r.out.interrupted.load(Ordering::SeqCst) == 0
            && t_stop.elapsed() < Duration::from_millis(100)
        {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(
            r.out.interrupted.load(Ordering::SeqCst),
            1,
            "not stopped within 100 ms"
        );

        // Nothing else plays: the queue was dropped.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(r.out.played_count(), 1);
        assert_ne!(r.engine.character_state(), CharacterState::Talking);
        // Late deltas of the stopped reply stay silent.
        r.speaker
            .observe(&delta("a2", "More words for you. And even more words."));
        assert!(!r.speaker.is_speaking());
        // The mouth closes.
        let levels: Vec<f32> = r
            .sink
            .take()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::Level(l) => Some(l.level),
                _ => None,
            })
            .collect();
        assert_eq!(levels.last().copied(), Some(0.0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn final_message_without_deltas_is_spoken() {
        let r = rig(voice(|_| {}));
        r.speaker.arm_next_reply();
        r.speaker
            .observe(&msg("u1", ChatRole::User, Via::Voice, "help me start"));
        r.speaker
            .observe(&msg("a9", ChatRole::Assistant, Via::Voice, REPLY));
        r.idle().await;
        assert_eq!(r.texts(), expected());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tool_round_messages_flush_in_order() {
        let r = rig(voice(|_| {}));
        r.speaker
            .observe(&msg("u1", ChatRole::User, Via::Voice, "start a session"));
        // Round 1 streams text without a sentence end, then a tool call happens.
        r.speaker
            .observe(&delta("r1", "Starting a timer for your essay now"));
        // Round 2 is a new message id.
        r.speaker
            .observe(&delta("r2", "You've got twenty-five minutes. "));
        r.speaker.observe(&msg(
            "r2",
            ChatRole::Assistant,
            Via::Voice,
            "You've got twenty-five minutes.",
        ));
        r.idle().await;
        assert_eq!(
            r.texts(),
            vec![
                "Starting a timer for your essay now",
                "You've got twenty-five minutes."
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_model_reports_problem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sink = Arc::new(CollectingSink::default());
        let out = Arc::new(FakeOut::new(1.0));
        let speaker = SpeakerPipeline::build(
            sink,
            dir.path().to_path_buf(),
            TtsSource::Piper,
            out.clone(),
        );
        speaker.apply_settings(&voice(|_| {}), None);
        assert!(!speaker.ready());
        assert!(speaker.voices().iter().all(|v| !v.downloaded));
        speaker.arm_next_reply();
        streamed_turn(&speaker, Via::Voice);
        assert!(!speaker.is_speaking());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(out.played_count(), 0);
        let err = speaker
            .preview_voice(tts::default_voice().id, "Hi there.")
            .await;
        assert!(err.is_err());
        // Nothing was written into the models dir.
        assert_eq!(
            std::fs::read_dir(dir.path())
                .map(|d| d.count())
                .unwrap_or(0),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn levels_emitted_while_playing() {
        let r = rig_with(voice(|_| {}), 0.3);
        r.speaker
            .observe(&msg("u1", ChatRole::User, Via::Voice, "hi"));
        r.speaker.observe(&msg(
            "a1",
            ChatRole::Assistant,
            Via::Voice,
            "Here we go, one tiny step at a time.",
        ));
        r.idle().await;
        let levels: Vec<AudioLevel> = r
            .sink
            .take()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::Level(l) => Some(l),
                _ => None,
            })
            .collect();
        assert!(levels.iter().all(|l| l.source == LevelSource::Speaker));
        assert!(levels.len() >= 5, "{} levels", levels.len());
        assert!(levels.iter().any(|l| l.level > 0.3));
        assert_eq!(levels.last().map(|l| l.level), Some(0.0));
        // Done speaking: the overlay is gone.
        assert_ne!(r.engine.character_state(), CharacterState::Talking);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn preview_ignores_mute_and_unknown_voice_errors() {
        let r = rig(voice(|v| v.mute = true));
        assert!(r.speaker.preview_voice("nope", "Hello.").await.is_err());
        let ok = r
            .speaker
            .preview_voice(tts::default_voice().id, "Hi! This is how I sound.")
            .await;
        assert!(ok.is_ok());
        r.idle().await;
        assert_eq!(r.texts(), vec!["Hi! This is how I sound."]);
    }

    /// Real Piper voice through the whole pipeline into the fake speaker (no sound, no files).
    /// Set `TANDEM_PIPER_MODELS` to a models dir with the default voice downloaded.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn piper_pipeline_end_to_end() {
        let Some(dir) = std::env::var_os("TANDEM_PIPER_MODELS").map(PathBuf::from) else {
            eprintln!("TANDEM_PIPER_MODELS not set; skipping");
            return;
        };
        let out = Arc::new(FakeOut::new(0.05));
        let speaker = SpeakerPipeline::build(
            Arc::new(CollectingSink::default()),
            dir,
            TtsSource::Piper,
            out.clone(),
        );
        speaker.apply_settings(&voice(|_| {}), None);
        assert!(speaker.ready());
        let t0 = Instant::now();
        streamed_turn(&speaker, Via::Voice);
        while out.played_count() == 0 && t0.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        eprintln!("first audio after {:?}", t0.elapsed());
        while speaker.is_speaking() && t0.elapsed() < Duration::from_secs(20) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let clips = out.played.lock().map(|g| g.clone()).unwrap_or_default();
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().all(|(c, _)| c.duration_secs() > 0.8));
    }

    #[test]
    fn constructor_opens_nothing_and_unknown_voice_falls_back() {
        let s = SpeakerPipeline::new(
            Arc::new(CollectingSink::default()),
            PathBuf::from("/nonexistent"),
        );
        s.apply_settings(&voice(|_| {}), Some("not-a-voice"));
        assert_eq!(s.config().voice_id, tts::default_voice().id);
        assert!(!s.ready());
        assert!(!s.is_speaking());
        assert_eq!(s.voices().len(), tts::voice_catalog().len());
        assert_eq!(s.voices().iter().filter(|v| v.is_default).count(), 1);
    }
}
