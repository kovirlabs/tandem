//! Speech-to-text with whisper.cpp in-process via `whisper-rs` (Metal on macOS) (M2 Part A).
//!
//! The clip is handed to whisper in memory and dropped afterwards. Transcripts are never logged.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::models::ModelFile;
use super::{AudioClip, AudioError, SttProvider, STT_SAMPLE_RATE};
use crate::providers::Locality;

/// whisper.cpp models offered in Settings: (id, size in bytes, sha256 from Hugging Face LFS).
const STT_MODELS: &[(&str, u64, &str)] = &[
    (
        "tiny.en",
        77_704_715,
        "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
    ),
    (
        "base.en",
        147_964_211,
        "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    ),
    (
        "small.en",
        487_614_201,
        "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
    ),
];

/// Ids of the offered models, smallest first, with their download size.
pub fn stt_model_ids() -> impl Iterator<Item = (&'static str, u64)> {
    STT_MODELS.iter().map(|(id, size, _)| (*id, *size))
}

/// The download for a whisper model id (`tiny.en`, `base.en`, `small.en`).
pub fn stt_model_file(id: &str) -> Option<ModelFile> {
    STT_MODELS
        .iter()
        .find(|(m, _, _)| *m == id)
        .map(|(m, _, sha)| ModelFile {
            rel_path: format!("whisper/ggml-{m}.bin"),
            url: format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{m}.bin"),
            sha256: Some((*sha).to_string()),
        })
}

/// Things whisper "hears" in silence or noise. Compared after lowercasing and trimming
/// punctuation and whitespace.
const HALLUCINATIONS: &[&str] = &[
    "blank_audio",
    "blank audio",
    "music",
    "silence",
    "no speech",
    "inaudible",
    "applause",
    "laughter",
    "noise",
    "sound",
    "thank you",
    "thanks for watching",
    "thank you for watching",
    "you",
];

fn is_hallucination(chunk: &str) -> bool {
    let core: String = chunk
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    core.is_empty() || HALLUCINATIONS.contains(&core.as_str())
}

/// Trim the transcript and return `""` if it is only noise markers (`[BLANK_AUDIO]`,
/// `(music)`, `[Music]`, `(silence)`, `*sigh*`) or a lone filler line whisper invents on
/// silence ("Thank you.", "Thanks for watching!").
pub fn clean_transcript(raw: &str) -> String {
    // Remove bracketed / parenthesised / starred annotations like [BLANK_AUDIO] or (music).
    let mut out = String::with_capacity(raw.len());
    let mut depth_close: Vec<char> = Vec::new();
    for c in raw.chars() {
        match c {
            '[' => depth_close.push(']'),
            '(' => depth_close.push(')'),
            ']' | ')' if depth_close.last() == Some(&c) => {
                depth_close.pop();
            }
            _ if depth_close.is_empty() => out.push(c),
            _ => {}
        }
    }
    let text = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if is_hallucination(&text) {
        String::new()
    } else {
        text
    }
}

/// whisper.cpp loaded from a ggml model file.
pub struct WhisperStt {
    ctx: Arc<WhisperContext>,
}

impl WhisperStt {
    /// Load the model (slow, blocking: call from `spawn_blocking`).
    pub fn load(path: &Path) -> Result<Self, AudioError> {
        if !path.is_file() {
            return Err(AudioError::ModelMissing("speech".into()));
        }
        // whisper.cpp's own logging prints decoded tokens (what the user said) at debug level.
        // With no log backend feature enabled, these hooks discard all of it.
        whisper_rs::install_logging_hooks();
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| AudioError::Stt(e.to_string()))?;
        Ok(Self { ctx: Arc::new(ctx) })
    }

    fn run(ctx: &WhisperContext, samples: &[f32]) -> Result<String, AudioError> {
        let err = |e: whisper_rs::WhisperError| AudioError::Stt(e.to_string());
        let mut state = ctx.create_state().map_err(err)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_no_timestamps(true);
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        // Never print (and so never log) what was said.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4);
        params.set_n_threads(i32::try_from(threads).unwrap_or(4));
        state.full(params, samples).map_err(err)?;
        let mut text = String::new();
        for seg in state.as_iter() {
            if let Ok(s) = seg.to_str_lossy() {
                text.push_str(&s);
            }
        }
        Ok(text.trim().to_string())
    }
}

#[async_trait]
impl SttProvider for WhisperStt {
    fn locality(&self) -> Locality {
        Locality::Local
    }

    async fn transcribe(&self, clip: AudioClip) -> Result<String, AudioError> {
        if clip.sample_rate != STT_SAMPLE_RATE {
            return Err(AudioError::Stt(format!(
                "expected {STT_SAMPLE_RATE} Hz audio, got {} Hz",
                clip.sample_rate
            )));
        }
        let ctx = Arc::clone(&self.ctx);
        tokio::task::spawn_blocking(move || {
            // whisper needs at least ~1 s; pad short clips with silence (in memory).
            let mut samples = clip.samples;
            let min = STT_SAMPLE_RATE as usize + STT_SAMPLE_RATE as usize / 10;
            if samples.len() < min {
                samples.resize(min, 0.0);
            }
            let result = Self::run(&ctx, &samples);
            drop(samples);
            result
        })
        .await
        .map_err(|e| AudioError::Stt(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hallucination_filtered() {
        for raw in [
            "[BLANK_AUDIO]",
            " (music) ",
            "[Music]",
            "(silence)",
            "Thank you.",
            "Thanks for watching!",
            "  ",
            "[BLANK_AUDIO] [BLANK_AUDIO]",
            "(soft music) Thank you.",
            "...",
        ] {
            assert_eq!(clean_transcript(raw), "", "{raw:?} should be dropped");
        }
    }

    #[test]
    fn real_text_kept() {
        assert_eq!(
            clean_transcript("  Help me start my essay.  "),
            "Help me start my essay."
        );
        assert_eq!(
            clean_transcript("[BLANK_AUDIO] Thank you, that helps."),
            "Thank you, that helps."
        );
        assert_eq!(clean_transcript("Okay,\n  next step"), "Okay, next step");
    }

    #[test]
    fn model_file_urls() {
        for (id, _) in stt_model_ids() {
            let f = stt_model_file(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(f.rel_path, format!("whisper/ggml-{id}.bin"));
            assert_eq!(
                f.url,
                format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{id}.bin")
            );
            assert_eq!(f.sha256.as_ref().map(String::len), Some(64));
        }
        assert!(stt_model_file("base.en").is_some());
        assert!(stt_model_file("large-v3").is_none());
        assert!(stt_model_file("../../etc/passwd").is_none());
    }

    #[test]
    fn whisper_is_local() {
        assert!(WhisperStt::load(Path::new("/nonexistent/ggml-base.en.bin")).is_err());
    }

    /// Set `TANDEM_WHISPER_MODEL=/path/to/ggml-base.en.bin` to run. Uses a synthetic clip
    /// generated in memory (nothing is written), so it only checks that whisper runs and
    /// that the cleaned output for non-speech is empty or short.
    #[tokio::test]
    #[ignore = "needs a downloaded whisper model"]
    async fn whisper_transcribes_fixture() {
        let Some(path) = std::env::var_os("TANDEM_WHISPER_MODEL") else {
            return;
        };
        let stt = match WhisperStt::load(Path::new(&path)) {
            Ok(s) => s,
            Err(e) => panic!("load failed: {e}"),
        };
        assert_eq!(stt.locality(), Locality::Local);
        let clip = AudioClip {
            samples: vec![0.0; STT_SAMPLE_RATE as usize * 2],
            sample_rate: STT_SAMPLE_RATE,
        };
        let text = stt.transcribe(clip).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            clean_transcript(&text),
            "",
            "silence should clean to nothing"
        );
    }
}
