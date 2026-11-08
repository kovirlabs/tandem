# 0003 — License: GPL-3.0-or-later

- **Status:** Accepted (2026-11-08)
- **Context:** SPEC §10 left the license open with Apache-2.0 as a placeholder. M2 needs a local
  text-to-speech engine. Piper is the SPEC default. The original `rhasspy/piper` (MIT) was archived on
  2025-10-06. Active development moved to `OHF-Voice/piper1-gpl`, which is **GPL-3.0-or-later**
  because it embeds espeak-ng (GPL-3.0) for phonemization.
- **Decision:** License Tandem as **GPL-3.0-or-later**. This lets us link the maintained Piper /
  espeak-ng code in-process instead of relying on an archived binary or a process boundary.
- **Consequences:**
  - All dependencies must be GPL-3-compatible. Apache-2.0, MIT, BSD, MPL-2.0, ISC and Zlib are fine;
    "GPL-2.0-only" and proprietary SDKs are not.
  - Forks and redistributions must ship source under the GPL. Running the app is unrestricted.
  - **Voice models are data with their own licenses.** Each Piper voice has a model card; some are
    non-commercial (CC-BY-NC). Bundled or default-offered voices must have a license that allows
    redistribution, and the voice picker shows each voice's license.
  - Character packs keep their own asset licenses (currently CC-BY-4.0).
  - A Mac App Store build was already ruled out by `macOSPrivateApi` (ADR 0002); the GPL adds no
    new distribution limits for direct download or Homebrew.
