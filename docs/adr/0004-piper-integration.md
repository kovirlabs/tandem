# 0004 — Piper text-to-speech: in-process with `ort` and pure-Rust espeak-ng

- **Status:** Accepted (2026-11-08)
- **Context:** M2 needs local Piper voices (SPEC §6.2), in process if possible. It has to build on
  macOS arm64, Windows and Linux with no Python and no build-time downloads beyond crates.io and git
  dependencies. The input lane already pins `ort = "=2.0.0-rc.15"` for Silero VAD, and `ort-sys`
  links ONNX Runtime natively (`links = "onnxruntime"`), so only one `ort` version can be in the
  dependency graph. ADR 0003 made the project GPL-3.0-or-later so it can use GPL-3 espeak-ng code.
- **Options tried** (plan task O1, in order of preference):
  1. **A maintained Piper crate.**
     - `piper-rs` 0.2.0 (MIT, May 2026) pins `ort = "=2.0.0-rc.12"`, which conflicts with the input
       lane. Its `espeak-rs-sys` builds C espeak-ng with CMake. On this Mac (arm64, Darwin 27,
       current clang) the espeak-ng tool it runs at build time crashes with SIGTRAP while compiling
       `intonations`. Without that feature it builds, but it ships no compiled espeak-ng data, so
       we would have to bundle `espeak-ng-data` ourselves and point an env var at it.
     - `dengjen-piper-rs` 0.2.12 (GPL-3, Oct 2026) pins `ort = "=2.0.0-rc.14"`, which also
       conflicts. It is young, with about 200 downloads.
     - `piper-phonemize` 1.5.9 (GPL-3, k2-fsa) downloads prebuilt native libraries and espeak data
       from GitHub releases in `build.rs`. That is a build-time download we don't want.
  2. **Build `libpiper` from OHF-Voice/piper1-gpl with CMake.** Its CMake fetches espeak-ng and
     ONNX Runtime at configure time. That is a build-time download, and it would add a second ONNX
     Runtime next to `ort`'s.
  3. **`ort` + an espeak-ng phonemizer, with our own Piper inference.** ✅
- **Decision:** Use option 3:
  - Use **`ort` 2.0.0-rc.15**, the same dependency the input lane added.
  - For phonemization, use **`espeak-ng` 0.2.0** (GPL-3.0-or-later). It is a pure-Rust port of
    eSpeak NG that is checked bit-for-bit against the C library on IPA output. Its English data
    (phoneme tables + `en_dict`, about 2.4 MB) is compiled into the binary with the
    `bundled-data-en` feature. There is no C build, no CMake, and no data to ship separately.
  - For text normalization, use **`unicode-normalization`** (MIT/Apache).
  - The inference code is about 100 lines in `core/src/audio/tts.rs` and follows piper1-gpl:
    1. Get IPA from espeak with punctuation kept (`text_to_phonemes_phonemizer`, using the voice's
       `espeak.voice`).
    2. Apply NFD normalization.
    3. Build the ids as `^ _ (phoneme _)* $` using the voice's `phoneme_id_map`.
    4. Run ONNX with inputs `input`, `input_lengths` and `scales = [noise_scale,
       length_scale / rate, noise_w]`, plus `sid` for multi-speaker voices.
    5. Peak-normalize the output.
  - **ONNX Runtime runs on the CPU only** (`AutoDevicePolicy::PreferCPU`). `ort`'s default policy
    registers CoreML on macOS, which crashed with SIGSEGV inside BNNS on the Piper VITS graph.
    On the CPU, a debug build synthesizes 3 s of speech in about 70–100 ms, and loading a voice
    takes about 0.5 s. Telemetry is off (rule 10).
  - At the first voice load, espeak-ng's English data is installed from the binary into
    `<data_dir>/models/piper/espeak-ng-data/`. This is static engine data, not user data. Audio
    never touches disk.
  - **No sidecar** is needed, and `apps/` doesn't change.
- **Voice catalog** (`tts::voice_catalog()`, from each voice's MODEL_CARD on
  huggingface.co/rhasspy/piper-voices, checked 2026-11-08; SHA-256 values are pinned):

  | id | label | license | notes |
  |---|---|---|---|
  | `en_US-kristin-medium` | Kristin (US English, medium) | public domain (LibriVox) | **provisional default** |
  | `en_US-john-medium` | John (US English, medium) | public domain (LibriVox) | fine-tuned from kristin |
  | `en_US-norman-medium` | Norman (US English, medium) | public domain (LibriVox) | |
  | `en_US-ljspeech-medium` | LJ (US English, medium) | public domain (LJ Speech) | |
  | `en_GB-cori-medium` | Cori (UK English, medium) | public domain (LibriVox) | |
  | `en_GB-cori-high` | Cori (UK English, high quality) | public domain (LibriVox) | 114 MB |

  - **Excluded:**
    - Non-commercial data: ryan (CC BY-NC-SA), hfc_female and hfc_male, semaine, l2arctic.
    - **lessac**: its Blizzard 2013 license allows research only and explicitly excludes speech
      products.
    - **Every voice fine-tuned from lessac**: joe, sam, alba, aru, vctk, libritts_r and
      reza_ibrahim. Their data licenses are permissive, but the weights start from a checkpoint
      trained on the research-only data.
    - Voices whose card says only "see URL" with no clear license: amy, kusal, alan, jenny_dioco.
    - bryce: fine-tuned from an unreleased base model.
  - Labels use the voice's name plus accent and quality, with no gender or ethnicity words (a test
    enforces this).
  - The **default is `en_US-kristin-medium`**. It is a single-speaker voice trained from scratch on
    about 11.5 h of LibriVox audio, chosen provisionally for a warm, mid-range read. SPEC asks for
    a listening test to make the final pick. To change the default, flip `is_default` in the
    catalog.
- **Consequences:**
  - A lessac-family voice can't be the default. This rules out the most common Piper default
    (`en_US-lessac-medium`). The example in `SpeakOpts`'s doc comment is only an example.
  - The espeak-ng port is young (v0.2.0, Sep 2026), and its upstream oracle tests cover IPA. If
    phonemes ever drift, the fallback is `espeak-rs-sys` (C espeak-ng) with prebuilt
    `espeak-ng-data`. That only changes `PiperTts::load` and the phonemize call.
  - `ort`'s default `download-binaries` feature (chosen by the input lane) fetches a prebuilt ONNX
    Runtime from pyke's CDN at build time, and caches it. That is the one build-time download
    left. To avoid it, build with `ORT_LIB_LOCATION` pointing at a local ONNX Runtime, or switch
    `ort` to `load-dynamic`. This is a lead-level decision because both lanes share it.
  - Voices download only when the user presses **Download** (`download_voice`), with SHA-256
    checked by `models::ensure`.
