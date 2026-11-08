//! OWNER: output agent. Local Piper text-to-speech (plan task O1, ADR 0004).
//!
//! Piper voices are VITS models exported to ONNX plus a JSON config. A synthesis call is:
//! text → IPA phonemes (espeak-ng, here the pure-Rust port with English data compiled into
//! the binary) → phoneme ids (from the voice's `phoneme_id_map`) → ONNX Runtime → float PCM.
//! This mirrors `piper1-gpl` (OHF-Voice), which is GPL-3.0 like this project.
//!
//! Privacy: synthesised audio stays in memory (`AudioClip`) and is never written or logged.
//! The only files touched are the voice model, its config, and espeak-ng's static language
//! data, installed once next to the voices.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use super::models::{self, ModelFile};
use super::{AudioClip, AudioError, SpeakOpts, TtsProvider};
use crate::providers::Locality;

const HF_BASE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";
/// Folder under the models dir for voices (and the espeak-ng data they need).
pub const VOICE_DIR: &str = "piper";
const ESPEAK_DATA_DIR: &str = "espeak-ng-data";
/// Written after espeak-ng data is fully installed, so a half-written copy is redone.
const ESPEAK_MARKER: &str = ".tandem-installed-espeak-ng-0.2.0";

/// One downloadable voice. Licenses come from each voice's MODEL_CARD on
/// huggingface.co/rhasspy/piper-voices (checked 2026-11-08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogVoice {
    /// e.g. `en_US-kristin-medium`.
    pub id: &'static str,
    /// Display label. Never contains gender or ethnicity words.
    pub label: &'static str,
    /// BCP-47-ish language tag, e.g. `en-US`.
    pub language: &'static str,
    /// Path of the voice folder in the piper-voices repo, e.g. `en/en_US/kristin/medium`.
    pub repo_dir: &'static str,
    /// License of the training data / model, from the MODEL_CARD.
    pub license: &'static str,
    pub onnx_sha256: &'static str,
    pub onnx_bytes: u64,
    pub json_sha256: &'static str,
    pub json_bytes: u64,
    /// Provisional default (SPEC: a listening test makes the final pick).
    pub is_default: bool,
}

/// English voices we offer. Excluded on purpose (see ADR 0004): every voice whose data is
/// non-commercial (ryan, hfc_*, semaine, l2arctic), `lessac` (research-only Blizzard license)
/// and every voice fine-tuned from lessac (amy is unclear, joe, sam, alba, aru, vctk,
/// libritts_r, reza_ibrahim), plus voices whose card only says "see URL".
static CATALOG: &[CatalogVoice] = &[
    CatalogVoice {
        id: "en_US-kristin-medium",
        label: "Kristin (US English, medium)",
        language: "en-US",
        repo_dir: "en/en_US/kristin/medium",
        license: "public domain (LibriVox)",
        onnx_sha256: "5849957f929cbf720c258f8458692d6103fff2f0e3d3b19c8259474bb06a18d4",
        onnx_bytes: 63_531_379,
        json_sha256: "5681426d4aead22195de70531eeeeddb46493cfaffc5764b2ea3db73428b651c",
        json_bytes: 4_968,
        is_default: true,
    },
    CatalogVoice {
        id: "en_US-john-medium",
        label: "John (US English, medium)",
        language: "en-US",
        repo_dir: "en/en_US/john/medium",
        license: "public domain (LibriVox)",
        onnx_sha256: "789c6c875726e627ddee93d51d8727859abe9c091c3d141591f4b83c2072e988",
        onnx_bytes: 63_531_379,
        json_sha256: "af60f177b6b550f3d7a302720c0fb89e7f94a82b5dca464775ef63b1c69ba09a",
        json_bytes: 4_965,
        is_default: false,
    },
    CatalogVoice {
        id: "en_US-norman-medium",
        label: "Norman (US English, medium)",
        language: "en-US",
        repo_dir: "en/en_US/norman/medium",
        license: "public domain (LibriVox)",
        onnx_sha256: "b9739443232a80a59c7d18810dd856899bf16a7964725f5ab81ea49b1351cb71",
        onnx_bytes: 63_531_379,
        json_sha256: "6c2db7f558a4a8deb9fe822583c1c5105f6c4e834dd0f9de8ad17a888ee9fe1d",
        json_bytes: 4_968,
        is_default: false,
    },
    CatalogVoice {
        id: "en_US-ljspeech-medium",
        label: "LJ (US English, medium)",
        language: "en-US",
        repo_dir: "en/en_US/ljspeech/medium",
        license: "public domain (LJ Speech)",
        onnx_sha256: "6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22",
        onnx_bytes: 63_531_379,
        json_sha256: "141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860",
        json_bytes: 4_972,
        is_default: false,
    },
    CatalogVoice {
        id: "en_GB-cori-medium",
        label: "Cori (UK English, medium)",
        language: "en-GB",
        repo_dir: "en/en_GB/cori/medium",
        license: "public domain (LibriVox)",
        onnx_sha256: "1899f98e5fb8310154f3c2973f4b8a929ba7245e722b3d3a85680b833d95f10d",
        onnx_bytes: 63_531_379,
        json_sha256: "e262c16d7f192f69d4edd6b4ef8a5915379e67495fcc402f1ab15eeb33da3d36",
        json_bytes: 4_966,
        is_default: false,
    },
    CatalogVoice {
        id: "en_GB-cori-high",
        label: "Cori (UK English, high quality)",
        language: "en-GB",
        repo_dir: "en/en_GB/cori/high",
        license: "public domain (LibriVox)",
        onnx_sha256: "470b4dd634c98f8a4850d7626ffc3dfc90774628eeef6605a6dd8f88f30a5903",
        onnx_bytes: 114_219_352,
        json_sha256: "9e7fb5b5671612c22f3c81cbe46c1ae87b031a4632bcb509e499dad6f1e2adec",
        json_bytes: 4_963,
        is_default: false,
    },
];

pub fn voice_catalog() -> &'static [CatalogVoice] {
    CATALOG
}

pub fn find_voice(id: &str) -> Option<&'static CatalogVoice> {
    CATALOG.iter().find(|v| v.id == id)
}

pub fn default_voice() -> &'static CatalogVoice {
    CATALOG.iter().find(|v| v.is_default).unwrap_or(&CATALOG[0])
}

impl CatalogVoice {
    /// Model id used in `DownloadProgress`.
    pub fn model_id(&self) -> String {
        format!("piper:{}", self.id)
    }

    pub fn onnx_file(&self) -> ModelFile {
        ModelFile {
            rel_path: format!("{VOICE_DIR}/{}.onnx", self.id),
            url: format!("{HF_BASE}/{}/{}.onnx", self.repo_dir, self.id),
            sha256: Some(self.onnx_sha256.to_string()),
        }
    }

    pub fn json_file(&self) -> ModelFile {
        ModelFile {
            rel_path: format!("{VOICE_DIR}/{}.onnx.json", self.id),
            url: format!("{HF_BASE}/{}/{}.onnx.json", self.repo_dir, self.id),
            sha256: Some(self.json_sha256.to_string()),
        }
    }

    pub fn is_downloaded(&self, models_dir: &Path) -> bool {
        models::is_present(models_dir, &self.onnx_file())
            && models::is_present(models_dir, &self.json_file())
    }

    /// `(onnx, json)` paths under `models_dir`.
    pub fn paths(&self, models_dir: &Path) -> (PathBuf, PathBuf) {
        (
            models::path_of(models_dir, &self.onnx_file()),
            models::path_of(models_dir, &self.json_file()),
        )
    }
}

/// Speech rate (0.5–2.0, 1.0 = normal) → Piper `length_scale` (bigger = slower).
pub fn length_scale_for_rate(rate: f32) -> f32 {
    let rate = if rate.is_finite() { rate } else { 1.0 };
    1.0 / rate.clamp(0.5, 2.0)
}

// --- Voice config -------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct PiperConfig {
    audio: AudioCfg,
    espeak: EspeakCfg,
    #[serde(default)]
    inference: InferenceCfg,
    #[serde(default)]
    num_speakers: u32,
    phoneme_id_map: HashMap<String, Vec<i64>>,
}

#[derive(Debug, Clone, Deserialize)]
struct AudioCfg {
    sample_rate: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct EspeakCfg {
    voice: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct InferenceCfg {
    noise_scale: f32,
    length_scale: f32,
    noise_w: f32,
}

impl Default for InferenceCfg {
    fn default() -> Self {
        Self {
            noise_scale: 0.667,
            length_scale: 1.0,
            noise_w: 0.8,
        }
    }
}

const PAD: &str = "_";
const BOS: &str = "^";
const EOS: &str = "$";

/// IPA phoneme string → model input ids, as piper1-gpl does it: BOS, PAD, then each
/// (NFD-decomposed) phoneme followed by PAD, then EOS. Unknown phonemes are skipped.
fn phonemes_to_ids(phonemes: &str, map: &HashMap<String, Vec<i64>>) -> Vec<i64> {
    let get = |k: &str| map.get(k).cloned().unwrap_or_default();
    let pad = get(PAD);
    let mut ids = get(BOS);
    ids.extend(&pad);
    let mut buf = [0u8; 4];
    for c in phonemes.nfd() {
        if let Some(id) = map.get(c.encode_utf8(&mut buf) as &str) {
            ids.extend(id);
            ids.extend(&pad);
        }
    }
    ids.extend(get(EOS));
    ids
}

// --- espeak-ng data -----------------------------------------------------------------------

static ESPEAK_INSTALL: Mutex<()> = Mutex::new(());

/// Install espeak-ng's English data (compiled into the binary) into `dir` once.
fn ensure_espeak_data(dir: &Path) -> Result<(), AudioError> {
    let _guard = ESPEAK_INSTALL.lock().map_err(|_| tts_err("espeak lock"))?;
    if dir.join(ESPEAK_MARKER).is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| tts_err(format!("espeak data dir: {e}")))?;
    espeak_ng::install_bundled_languages(dir, &["en"])
        .map_err(|e| tts_err(format!("espeak data install: {e}")))?;
    std::fs::write(dir.join(ESPEAK_MARKER), b"ok")
        .map_err(|e| tts_err(format!("espeak data marker: {e}")))?;
    Ok(())
}

fn tts_err(msg: impl std::fmt::Display) -> AudioError {
    AudioError::Tts(msg.to_string())
}

// --- Engine -------------------------------------------------------------------------------

struct Engine {
    _env: ort::environment::Environment,
    session: ort::session::Session,
    espeak: espeak_ng::EspeakNg,
}

pub struct PiperTts {
    config: PiperConfig,
    engine: Arc<Mutex<Engine>>,
}

impl PiperTts {
    /// Load a voice. Heavy (creates an ONNX Runtime session): call from `spawn_blocking`.
    /// espeak-ng data is installed into `<voice dir>/espeak-ng-data` on first use.
    pub fn load(onnx_path: &Path, json_path: &Path) -> Result<Self, AudioError> {
        if !onnx_path.is_file() || !json_path.is_file() {
            return Err(AudioError::ModelMissing("voice".into()));
        }
        let raw = std::fs::read(json_path).map_err(|e| tts_err(format!("voice config: {e}")))?;
        let config: PiperConfig =
            serde_json::from_slice(&raw).map_err(|e| tts_err(format!("voice config: {e}")))?;
        let data_dir = onnx_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(ESPEAK_DATA_DIR);
        ensure_espeak_data(&data_dir)?;
        let espeak = espeak_ng::EspeakNg::with_data_dir(&config.espeak.voice, &data_dir)
            .map_err(|e| tts_err(format!("espeak: {e}")))?;
        let err = |e: ort::Error| tts_err(format!("onnx: {e}"));
        // Telemetry off (CLAUDE.md rule 10).
        let env = ort::init()
            .with_name("tandem-tts")
            .with_telemetry(false)
            .build()
            .map_err(err)?;
        // CPU only: ort's default device policy registers CoreML on macOS, which crashes
        // (SIGSEGV inside BNNS) on Piper's VITS graph. CPU is fast enough (~10x real time).
        let session = ort::session::Session::builder(&env)
            .map_err(err)?
            .with_auto_device(ort::session::builder::AutoDevicePolicy::PreferCPU)
            .map_err(|e| tts_err(format!("onnx: {e}")))?
            .commit_from_file(onnx_path)
            .map_err(|e| tts_err(format!("onnx: {e}")))?;
        Ok(Self {
            config,
            engine: Arc::new(Mutex::new(Engine {
                _env: env,
                session,
                espeak,
            })),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.audio.sample_rate
    }

    /// Blocking synthesis of one chunk of text.
    fn synthesize_blocking(
        engine: &Mutex<Engine>,
        config: &PiperConfig,
        text: &str,
        rate: f32,
    ) -> Result<AudioClip, AudioError> {
        let mut eng = engine.lock().map_err(|_| tts_err("engine lock"))?;
        let phonemes = eng
            .espeak
            .text_to_phonemes_phonemizer(text)
            .map_err(|e| tts_err(format!("phonemize: {e}")))?;
        let ids = phonemes_to_ids(&phonemes, &config.phoneme_id_map);
        if ids.len() <= 3 {
            return Ok(AudioClip {
                samples: Vec::new(),
                sample_rate: config.audio.sample_rate,
            });
        }
        let inf = &config.inference;
        let scales = vec![
            inf.noise_scale,
            inf.length_scale * length_scale_for_rate(rate),
            inf.noise_w,
        ];
        let n = ids.len();
        let err = |e: ort::Error| tts_err(format!("onnx: {e}"));
        let input = ort::value::Tensor::from_array(([1usize, n], ids)).map_err(err)?;
        let lengths = ort::value::Tensor::from_array(([1usize], vec![n as i64])).map_err(err)?;
        let scales = ort::value::Tensor::from_array(([3usize], scales)).map_err(err)?;
        let mut inputs = ort::inputs![
            "input" => input,
            "input_lengths" => lengths,
            "scales" => scales,
        ];
        if config.num_speakers > 1 {
            let sid = ort::value::Tensor::from_array(([1usize], vec![0i64])).map_err(err)?;
            inputs.push(("sid".into(), sid.into()));
        }
        let outputs = eng.session.run(inputs).map_err(err)?;
        let (_, data) = outputs[0].try_extract_tensor::<f32>().map_err(err)?;
        Ok(AudioClip {
            samples: peak_normalize(data),
            sample_rate: config.audio.sample_rate,
        })
    }
}

/// Scale so the loudest sample sits at 0.95, as Piper does (it normalises each utterance),
/// which keeps sentences at a steady loudness.
fn peak_normalize(data: &[f32]) -> Vec<f32> {
    let peak = data.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(0.01);
    let gain = 0.95 / peak;
    data.iter().map(|s| (s * gain).clamp(-1.0, 1.0)).collect()
}

#[async_trait]
impl TtsProvider for PiperTts {
    fn locality(&self) -> Locality {
        Locality::Local
    }

    async fn synthesize(&self, text: &str, opts: &SpeakOpts) -> Result<AudioClip, AudioError> {
        let engine = self.engine.clone();
        let config = self.config.clone();
        let text = text.to_string();
        let rate = opts.rate;
        tokio::task::spawn_blocking(move || {
            Self::synthesize_blocking(&engine, &config, &text, rate)
        })
        .await
        .map_err(|e| tts_err(format!("synthesis task: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_default() {
        let defaults: Vec<_> = voice_catalog().iter().filter(|v| v.is_default).collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(default_voice().id, defaults[0].id);
        assert!(voice_catalog().len() >= 5);
    }

    #[test]
    fn catalog_labels_have_no_gender_words() {
        const DENY: &[&str] = &[
            "male",
            "female",
            "man",
            "woman",
            "men",
            "women",
            "boy",
            "girl",
            "masculine",
            "feminine",
            "he",
            "she",
        ];
        for v in voice_catalog() {
            for word in v
                .label
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
            {
                assert!(!DENY.contains(&word), "{} has '{word}'", v.label);
            }
            // The id is shown in some places too.
            let id = v.id.to_lowercase();
            assert!(!id.contains("male"), "{}", v.id);
        }
    }

    #[test]
    fn catalog_excludes_nc() {
        for v in voice_catalog() {
            let l = v.license.to_lowercase();
            assert!(!l.contains("nc"), "{} is {}", v.id, v.license);
            assert!(!l.contains("non-commercial") && !l.contains("noncommercial"));
            // Research-only lessac data and its fine-tunes are excluded too.
            for banned in [
                "lessac", "ryan", "hfc_", "semaine", "l2arctic", "joe", "sam", "alba",
            ] {
                assert!(!v.id.contains(banned), "{}", v.id);
            }
        }
    }

    #[test]
    fn rate_maps_to_length_scale() {
        assert!((length_scale_for_rate(1.0) - 1.0).abs() < 1e-6);
        assert!((length_scale_for_rate(2.0) - 0.5).abs() < 1e-6);
        assert!((length_scale_for_rate(0.5) - 2.0).abs() < 1e-6);
        // Clamped to 0.5–2.0, and junk falls back to normal speed.
        assert!((length_scale_for_rate(10.0) - 0.5).abs() < 1e-6);
        assert!((length_scale_for_rate(0.0) - 2.0).abs() < 1e-6);
        assert!((length_scale_for_rate(f32::NAN) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn model_files_point_at_huggingface() {
        let v = default_voice();
        assert_eq!(
            v.onnx_file().url,
            format!("{HF_BASE}/{}/{}.onnx", v.repo_dir, v.id)
        );
        assert!(v.json_file().url.ends_with(".onnx.json"));
        assert_eq!(v.onnx_file().rel_path, format!("piper/{}.onnx", v.id));
        assert_eq!(v.model_id(), format!("piper:{}", v.id));
    }

    #[test]
    fn ids_follow_piper_layout() {
        let map: HashMap<String, Vec<i64>> = [("_", 0), ("^", 1), ("$", 2), ("h", 20), ("i", 21)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), vec![v]))
            .collect();
        // "?" is unknown and skipped.
        assert_eq!(phonemes_to_ids("hi?", &map), vec![1, 0, 20, 0, 21, 0, 2]);
    }

    #[test]
    fn peak_normalize_keeps_range() {
        let out = peak_normalize(&[0.1, -0.5, 0.25]);
        assert!((out[1] + 0.95).abs() < 1e-6);
        assert!(out.iter().all(|s| s.abs() <= 1.0));
        assert!(peak_normalize(&[]).is_empty());
    }

    #[test]
    fn missing_files_are_model_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (onnx, json) = default_voice().paths(dir.path());
        assert!(matches!(
            PiperTts::load(&onnx, &json),
            Err(AudioError::ModelMissing(_))
        ));
    }

    /// Real synthesis. Needs a downloaded voice: set `TANDEM_PIPER_MODELS` to a models dir
    /// that contains `piper/<default voice>.onnx(.json)`. Skips (passes) if it's missing.
    /// Never writes audio anywhere.
    #[tokio::test]
    #[ignore]
    async fn piper_synthesizes_hello() {
        let Some(dir) = std::env::var_os("TANDEM_PIPER_MODELS").map(PathBuf::from) else {
            eprintln!("TANDEM_PIPER_MODELS not set; skipping");
            return;
        };
        let v = default_voice();
        let (onnx, json) = v.paths(&dir);
        if !onnx.is_file() {
            eprintln!("{} not downloaded; skipping", v.id);
            return;
        }
        let t0 = std::time::Instant::now();
        let tts = tokio::task::spawn_blocking(move || PiperTts::load(&onnx, &json))
            .await
            .expect("join")
            .expect("load");
        let load = t0.elapsed();
        let opts = SpeakOpts {
            voice: v.id.into(),
            rate: 1.0,
        };
        let t1 = std::time::Instant::now();
        let clip = tts
            .synthesize("Hello! Let's take one tiny step together.", &opts)
            .await
            .expect("synthesize");
        let synth = t1.elapsed();
        let secs = clip.duration_secs();
        let rms = (clip.samples.iter().map(|s| s * s).sum::<f32>()
            / clip.samples.len().max(1) as f32)
            .sqrt();
        eprintln!(
            "load {load:?}, synth {synth:?}, audio {secs:.2}s @ {} Hz, rms {rms:.3}",
            clip.sample_rate
        );
        assert_eq!(clip.sample_rate, 22_050);
        assert!(secs > 1.0 && secs < 6.0, "duration {secs}");
        assert!(rms > 0.02, "silent output");
        // Faster speech is shorter.
        let fast = tts
            .synthesize(
                "Hello! Let's take one tiny step together.",
                &SpeakOpts {
                    rate: 1.5,
                    ..opts.clone()
                },
            )
            .await
            .expect("synthesize fast");
        assert!(fast.duration_secs() < secs);
    }
}
