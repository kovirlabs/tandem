//! Voice-activity detection for hands-free mode and barge-in (M2 Part C).
//!
//! [`SileroVad`] runs the Silero VAD v5 ONNX model (MIT) through `ort`. [`Segmenter`] turns
//! per-frame speech probabilities into utterances; [`BargeIn`] decides when the user is
//! talking over the companion. Clips stay in memory only.

use std::path::Path;

use super::models::ModelFile;
use super::{AudioClip, AudioError, STT_SAMPLE_RATE};

/// Samples per VAD frame at 16 kHz (32 ms), as Silero v5 expects.
pub const VAD_FRAME: usize = 512;
const FRAME_MS: u32 = (VAD_FRAME as u32 * 1000) / STT_SAMPLE_RATE;
/// Silero v5 prepends the last 64 samples of the previous frame.
const CONTEXT: usize = 64;
const STATE_LEN: usize = 2 * 128;

/// Model id used in `DownloadProgress`.
pub const VAD_MODEL_ID: &str = "vad:silero";

/// Silero VAD v5.1.2 from the snakers4/silero-vad GitHub repository (MIT license).
pub fn vad_model_file() -> ModelFile {
    ModelFile {
        rel_path: "vad/silero_vad.onnx".into(),
        url:
            "https://github.com/snakers4/silero-vad/raw/v5.1.2/src/silero_vad/data/silero_vad.onnx"
                .into(),
        sha256: Some("2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f".into()),
    }
}

/// A speech-probability model fed 512-sample frames at 16 kHz.
pub trait VadModel: Send {
    /// Probability (0–1) that `frame` contains speech.
    fn process(&mut self, frame: &[f32]) -> Result<f32, AudioError>;
    /// Forget the recurrent state (between listening sessions).
    fn reset(&mut self);
}

/// Silero VAD v5 via ONNX Runtime.
pub struct SileroVad {
    _env: ort::environment::Environment,
    session: ort::session::Session,
    state: Vec<f32>,
    context: Vec<f32>,
}

impl SileroVad {
    /// Load the model (blocking; small, ~2 MB). Telemetry is turned off (CLAUDE.md rule 10).
    pub fn load(path: &Path) -> Result<Self, AudioError> {
        if !path.is_file() {
            return Err(AudioError::ModelMissing("voice detection".into()));
        }
        let e = |e: &dyn std::fmt::Display| AudioError::Device(format!("voice detection: {e}"));
        let env = ort::init()
            .with_name("tandem-vad")
            .with_telemetry(false)
            .build()
            .map_err(|x| e(&x))?;
        // CPU only: ort's default policy registers CoreML on macOS, which crashed on Piper's
        // graph (see tts.rs); Silero is tiny, so CPU costs nothing.
        let session = ort::session::Session::builder(&env)
            .map_err(|x| e(&x))?
            .with_auto_device(ort::session::builder::AutoDevicePolicy::PreferCPU)
            .map_err(|x| e(&x))?
            .with_intra_threads(1)
            .map_err(|x| e(&x))?
            .with_inter_threads(1)
            .map_err(|x| e(&x))?
            .commit_from_file(path)
            .map_err(|x| e(&x))?;
        Ok(Self {
            _env: env,
            session,
            state: vec![0.0; STATE_LEN],
            context: vec![0.0; CONTEXT],
        })
    }
}

impl VadModel for SileroVad {
    fn process(&mut self, frame: &[f32]) -> Result<f32, AudioError> {
        use ort::value::Tensor;
        let e = |e: &dyn std::fmt::Display| AudioError::Device(format!("voice detection: {e}"));
        let mut input = Vec::with_capacity(CONTEXT + VAD_FRAME);
        input.extend_from_slice(&self.context);
        input.extend(
            frame
                .iter()
                .copied()
                .chain(std::iter::repeat(0.0))
                .take(VAD_FRAME),
        );
        self.context
            .copy_from_slice(&input[input.len() - CONTEXT..]);
        let x = Tensor::from_array(([1usize, CONTEXT + VAD_FRAME], input)).map_err(|x| e(&x))?;
        let state =
            Tensor::from_array(([2usize, 1, 128], self.state.clone())).map_err(|x| e(&x))?;
        let sr = Tensor::from_array(((), vec![i64::from(STT_SAMPLE_RATE)])).map_err(|x| e(&x))?;
        let outputs = self
            .session
            .run(ort::inputs!["input" => x, "state" => state, "sr" => sr])
            .map_err(|x| e(&x))?;
        let prob = outputs
            .get("output")
            .ok_or_else(|| e(&"no output"))?
            .try_extract_tensor::<f32>()
            .map_err(|x| e(&x))?
            .1
            .first()
            .copied()
            .unwrap_or(0.0);
        if let Some(st) = outputs.get("stateN") {
            if let Ok((_, s)) = st.try_extract_tensor::<f32>() {
                if s.len() == STATE_LEN {
                    self.state.copy_from_slice(s);
                }
            }
        }
        Ok(prob.clamp(0.0, 1.0))
    }

    fn reset(&mut self) {
        self.state.iter_mut().for_each(|v| *v = 0.0);
        self.context.iter_mut().for_each(|v| *v = 0.0);
    }
}

/// Segmenter thresholds (SPEC §5.4; plan task I4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmenterConfig {
    pub start_prob: f32,
    pub start_ms: u32,
    pub end_prob: f32,
    pub end_ms: u32,
    pub max_ms: u32,
    /// Audio kept from before the confirmed start so the first syllable isn't cut.
    pub preroll_ms: u32,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            start_prob: 0.5,
            start_ms: 250,
            end_prob: 0.35,
            end_ms: 700,
            max_ms: 30_000,
            preroll_ms: 200,
        }
    }
}

#[derive(Debug)]
pub enum SegmentEvent {
    SpeechStart,
    /// One utterance (pre-roll + speech + trailing silence), in memory only.
    SpeechEnd(AudioClip),
}

/// Turns per-frame speech probabilities into utterances.
pub struct Segmenter {
    cfg: SegmenterConfig,
    in_speech: bool,
    /// Frames of the current (pending or confirmed) segment.
    buf: Vec<f32>,
    /// Recent non-speech audio, kept as pre-roll.
    preroll: Vec<f32>,
    high_ms: u32,
    low_ms: u32,
    speech_ms: u32,
}

impl Segmenter {
    pub fn new(cfg: SegmenterConfig) -> Self {
        Self {
            cfg,
            in_speech: false,
            buf: Vec::new(),
            preroll: Vec::new(),
            high_ms: 0,
            low_ms: 0,
            speech_ms: 0,
        }
    }

    pub fn in_speech(&self) -> bool {
        self.in_speech
    }

    /// Drop everything (e.g. when hands-free stops or PTT takes over).
    pub fn reset(&mut self) {
        self.in_speech = false;
        self.buf.clear();
        self.preroll.clear();
        self.high_ms = 0;
        self.low_ms = 0;
        self.speech_ms = 0;
    }

    fn clip(samples: Vec<f32>) -> AudioClip {
        AudioClip {
            samples,
            sample_rate: STT_SAMPLE_RATE,
        }
    }

    /// Feed one frame (`VAD_FRAME` samples) and its speech probability.
    pub fn push(&mut self, prob: f32, frame: &[f32]) -> Option<SegmentEvent> {
        if !self.in_speech {
            if prob >= self.cfg.start_prob {
                self.high_ms += FRAME_MS;
                self.buf.extend_from_slice(frame);
                if self.high_ms >= self.cfg.start_ms {
                    self.in_speech = true;
                    self.low_ms = 0;
                    self.speech_ms = self.high_ms;
                    let mut seg = std::mem::take(&mut self.preroll);
                    seg.append(&mut self.buf);
                    self.buf = seg;
                    return Some(SegmentEvent::SpeechStart);
                }
            } else {
                // A blip: move the pending frames back into pre-roll and start over.
                self.high_ms = 0;
                let pending = std::mem::take(&mut self.buf);
                self.preroll.extend(pending);
                self.preroll.extend_from_slice(frame);
                let keep = (STT_SAMPLE_RATE * self.cfg.preroll_ms / 1000) as usize;
                if self.preroll.len() > keep {
                    let cut = self.preroll.len() - keep;
                    self.preroll.drain(..cut);
                }
            }
            return None;
        }
        self.buf.extend_from_slice(frame);
        self.speech_ms += FRAME_MS;
        if prob < self.cfg.end_prob {
            self.low_ms += FRAME_MS;
        } else {
            self.low_ms = 0;
        }
        if self.low_ms >= self.cfg.end_ms {
            let samples = std::mem::take(&mut self.buf);
            self.reset();
            return Some(SegmentEvent::SpeechEnd(Self::clip(samples)));
        }
        if self.speech_ms >= self.cfg.max_ms {
            // Too long: hand off what we have and keep listening as a new segment.
            let samples = std::mem::take(&mut self.buf);
            self.speech_ms = 0;
            self.low_ms = 0;
            return Some(SegmentEvent::SpeechEnd(Self::clip(samples)));
        }
        None
    }
}

/// Barge-in: the user talking over the companion. Higher thresholds than the segmenter
/// so the companion's own voice leaking into the mic doesn't stop it.
pub struct BargeIn {
    pub prob: f32,
    pub min_ms: u32,
    high_ms: u32,
}

impl Default for BargeIn {
    fn default() -> Self {
        Self {
            prob: 0.85,
            min_ms: 300,
            high_ms: 0,
        }
    }
}

impl BargeIn {
    /// Feed one frame's probability. Returns `true` once when the companion should stop.
    pub fn push(&mut self, prob: f32, companion_speaking: bool) -> bool {
        if !companion_speaking || prob < self.prob {
            self.high_ms = 0;
            return false;
        }
        let before = self.high_ms;
        self.high_ms += FRAME_MS;
        before < self.min_ms && self.high_ms >= self.min_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: [f32; VAD_FRAME] = [0.1; VAD_FRAME];

    fn feed(seg: &mut Segmenter, prob: f32, ms: u32) -> Vec<SegmentEvent> {
        (0..ms.div_ceil(FRAME_MS))
            .filter_map(|_| seg.push(prob, &FRAME))
            .collect()
    }

    fn frames_for(ms: u32) -> u32 {
        ms.div_ceil(FRAME_MS)
    }

    #[test]
    fn segment_starts_after_250ms() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        // 224 ms (7 frames) of speech: not yet.
        assert!(feed(&mut seg, 0.9, 7 * FRAME_MS).is_empty());
        assert!(!seg.in_speech());
        // The 8th frame crosses 250 ms.
        let ev = feed(&mut seg, 0.9, FRAME_MS);
        assert!(matches!(ev.as_slice(), [SegmentEvent::SpeechStart]));
        assert!(seg.in_speech());
    }

    #[test]
    fn segment_ends_after_700ms_silence() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        feed(&mut seg, 0.2, 400); // pre-roll
        assert_eq!(feed(&mut seg, 0.9, 1_000).len(), 1);
        // A short pause (and a mid-level probability) doesn't end it.
        assert!(feed(&mut seg, 0.1, 500).is_empty());
        assert!(feed(&mut seg, 0.4, 100).is_empty());
        assert!(feed(&mut seg, 0.1, 650).is_empty());
        let ev = feed(&mut seg, 0.1, 64);
        let clip = match ev.as_slice() {
            [SegmentEvent::SpeechEnd(c)] => c.clone(),
            other => panic!("expected end, got {other:?}"),
        };
        assert!(!seg.in_speech());
        assert_eq!(clip.sample_rate, STT_SAMPLE_RATE);
        // Pre-roll 200 ms + speech 1024 ms + pauses and trailing silence 1344 ms.
        let secs = clip.duration_secs();
        assert!(secs > 2.5 && secs < 2.65, "{secs}");
    }

    #[test]
    fn blip_is_ignored() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        for _ in 0..20 {
            assert!(feed(&mut seg, 0.95, 192).is_empty());
            assert!(feed(&mut seg, 0.05, 64).is_empty());
        }
        assert!(!seg.in_speech());
    }

    #[test]
    fn max_length_splits() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        let ev = feed(&mut seg, 0.9, 31_000);
        assert!(matches!(ev.first(), Some(SegmentEvent::SpeechStart)));
        let ends: Vec<f32> = ev
            .iter()
            .filter_map(|e| match e {
                SegmentEvent::SpeechEnd(c) => Some(c.duration_secs()),
                _ => None,
            })
            .collect();
        assert_eq!(ends.len(), 1);
        assert!((ends[0] - 30.0).abs() < 0.1, "{}", ends[0]);
        // Still listening: the rest becomes the next segment.
        assert!(seg.in_speech());
        let tail = feed(&mut seg, 0.0, 800);
        assert!(matches!(tail.as_slice(), [SegmentEvent::SpeechEnd(_)]));
    }

    #[test]
    fn barge_in_needs_300ms_high_prob() {
        let mut b = BargeIn::default();
        // Not speaking: never triggers.
        assert!(!(0..30).any(|_| b.push(0.99, false)));
        // Medium probability while speaking: never triggers (likely echo).
        assert!(!(0..30).any(|_| b.push(0.7, true)));
        // 288 ms of high probability: not yet; then interrupted resets it.
        assert!(!(0..frames_for(288)).any(|_| b.push(0.9, true)));
        assert!(!b.push(0.2, true));
        let fired: Vec<bool> = (0..frames_for(400)).map(|_| b.push(0.9, true)).collect();
        assert_eq!(
            fired.iter().filter(|f| **f).count(),
            1,
            "fires exactly once"
        );
        let at = fired.iter().position(|f| *f).unwrap_or(usize::MAX) as u32 + 1;
        assert!(at * FRAME_MS >= 300);
    }

    #[test]
    fn vad_model_is_pinned() {
        let f = vad_model_file();
        assert_eq!(f.rel_path, "vad/silero_vad.onnx");
        assert!(f.url.starts_with("https://github.com/snakers4/silero-vad/"));
        assert!(f.sha256.is_some());
    }

    /// Set `TANDEM_SILERO_MODEL=/path/to/silero_vad.onnx` to run.
    #[test]
    #[ignore = "needs the Silero VAD model"]
    fn silero_runs_on_silence_and_tone() {
        let Some(path) = std::env::var_os("TANDEM_SILERO_MODEL") else {
            return;
        };
        let mut vad = SileroVad::load(Path::new(&path)).unwrap_or_else(|e| panic!("{e}"));
        let silence = [0.0f32; VAD_FRAME];
        let mut p = 0.0;
        for _ in 0..10 {
            p = vad.process(&silence).unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(p < 0.2, "silence prob {p}");
        vad.reset();
    }
}
