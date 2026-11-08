# Tandem Roadmap

Living document. Milestones come from SPEC §8; this file tracks status and keeps feature
concepts that aren't fully specified yet so they don't get lost. Update it when scope changes.

## Where things stand (handoff, 2026-11-08)
Read this first if you're picking the project up in a new session or on another machine.

- **Repo:** `kovirlabs/tandem` (public), default branch `main`. License GPL-3.0-or-later (ADR 0003).
- **Built but not yet hand-tested:** M2 voice. Plan and integration checklist:
  `docs/superpowers/plans/2026-11-08-m2-voice.md`. Piper integration: ADR 0004.
- **Next step:** manual test on Windows (laptop) and macOS. Settings → Voice → enable, download
  `base.en` + a voice, then hold Ctrl/Cmd+Shift+Space to talk. Watch for gaps between sentences,
  barge-in echo in hands-free mode, and whether the global shortcut registers.
- **Windows install:** download the installer from Actions → "Windows installer" (artifact
  `tandem-windows-installer`), or run `install-windows.bat` from a source checkout.
- **CI is green** on macOS, Ubuntu and Windows, and the Windows installer workflow builds
  (2026-11-08). CI uses the latest stable Rust, so new lints can show up there before they do locally.
- **AI provider for testing:** Ollama on the LAN at `http://192.168.86.43:11434`, model
  `qwen3:14b` (or `hermes3:8b` for faster voice replies). Tick "I control this computer". The
  Anthropic key goes in Settings (OS keychain/Credential Manager), never in a file.
- **Not done yet:** prompt evals against `qwen3:14b`; listening test for the default voice
  (provisional: Kristin; lessac is excluded for license reasons); the follow-ups listed under M2 below.

## Milestone status

| Milestone | Status | Notes |
|-----------|--------|-------|
| **M0 Skeleton** | ✅ Done (2026-11-07) | Tauri + React + Rust core + SQLite + CI workflow (first GitHub CI run 2026-11-08) |
| **M1 Text MVP** | ✅ Done (2026-11-07) | Sessions, check-ins, Ollama + Anthropic, tools, evals, SVG beaver. See ADR 0001 |
| **Desktop Buddy (basic)** | ✅ Done (2026-11-07) | Pulled forward from M4 "mini mode". See below and ADR 0002 |
| **Red Fox character** | ✅ Done (2026-11-07) | Second built-in pack (SVG fallback) with a growing den; character picker in Settings |
| **M2 Voice** | ✅ Built, needs hands-on test | PTT + global shortcut, whisper.cpp, Piper (ADR 0004), sentence-streamed TTS, amplitude lip sync, hands-free VAD, barge-in, "Hey {name}" wake phrase. Plan: `docs/superpowers/plans/2026-11-08-m2-voice.md` |
| **M3 Camera** | Planned | Snapshot pipeline, vision contract, presence fusion |
| **M4 Cloud + Polish** | Partly done | Anthropic ✅, keychain ✅, tone presets ✅; OpenAI-compatible adapter, buddy polish, a11y audit to do |
| **M5 v1.0** | Planned | Hands-free VAD, barge-in, pack format + validator, Rive beaver, installers |
| **M6 Human Companion** | Planned | Customizable human pack |

### Open items carried from M1
- Beaver and fox `.riv` art (SVG fallbacks render all 9 states until then).
- More animal packs? (SPEC §12 open question 2.) Adding one = a `characters/<id>/` folder,
  an SVG fallback component, and one line in `ui/src/character/registry.tsx`.
- Input-activity signal (counts only; needs OS accessibility permission) to feed deep-focus protection.
- OpenAI-compatible adapter (OpenRouter, Groq, LM Studio, vLLM).
- ~~License decision~~ **Decided:** GPL-3.0-or-later (ADR 0003).
- Run CI on GitHub, including the 100% coverage gate for `router/privacy.rs`.

---

## Feature concept: Desktop Buddy

*Origin: a user idea on 2026-11-07, a desktop companion "like the old Office assistant, as a widget".*

The beaver lives directly on the desktop in a small, transparent, always-on-top window, so
the body-doubling presence stays visible while the main window is closed or behind other apps.
It extends SPEC §3.4's P1 "mini mode" (≈200×200 floating character with timer ring).

### Why it fits body doubling
Presence is the core mechanism (SPEC §1.1). A companion that's always quietly visible
"working on its dam" in the corner gives social presence and an external time cue without
the user having to keep the app window open.

### Design rules (the "don't be the paperclip" rules)
The famous desktop assistants failed mostly by **interrupting**. The buddy must:
1. **Never take keyboard focus.** The window is non-focusable; clicking it must not steal focus from the user's work mid-sentence.
2. **Speak only through the check-in engine.** It has no chatter of its own. Bubbles follow the same silent/gentle/conversational modes, quiet mode, and deep-focus protection, and fade out on their own (no reply needed).
3. **Stay small and look at its own work.** Gaze stays on its dam, glancing at the user only when speaking (CLAUDE.md character rules).
4. **Get out of the way.** Hide automatically during full-screen apps and screen sharing (polish phase). Easy to hide from the tray or right-click menu.
5. **Never shame.** No red, no "you were distracted" bubbles, no scores (CLAUDE.md rule 5).
6. **No new data.** It only renders core events it already receives; no extra sensing or network.

### Features
**Basic (built):**
- Transparent, frameless, always-on-top, non-focusable window with no taskbar entry (`buddy` window label).
- The beaver (same component as the main window, so the Rive pack slots in later) plus a small timer ring.
- Speech bubble for check-in lines, auto-fades after ~10 s, click to dismiss.
- Drag to move; the position is remembered across launches. Default position is the bottom-right of the primary monitor's work area.
- Click to open the main window; right-click for a native menu (Open, Pause/Resume, Quiet 30 min, Hide).
- Tray icon: show/hide buddy, open Tandem, quit. With the buddy enabled, closing the main window hides it instead of quitting.
- Settings toggle "Show {{name}} on the desktop".

**Polish (next):**
- Click-through on transparent pixels. Tauri only supports whole-window `set_ignore_cursor_events`, so this needs cursor-position polling that toggles click-through when over transparent regions.
- Auto-hide during full-screen apps / presentations / screen sharing.
- Multi-monitor: remember which display, recover if that display is gone.
- "Perch" mode: sit on the dock/taskbar edge.
- Size options (small / medium) and an opacity setting.
- Wayland fallback (below).
- ~~Push-to-talk from the buddy once voice (M2) lands.~~ Done (mic button on hover).
- M2 follow-ups: listening test to pick the final default voice (provisional: Kristin), echo cancellation for hands-free, mic-test button, keep the output device open between sentences, build ONNX Runtime without the build-time download (ADR 0004).

**Later ideas (unscheduled):**
- Seasonal outfits and small expressions on the buddy (SPEC P2).
- Cosmetic dam "collection" visible on the buddy (SPEC P2; no streaks, no loss).
- Human Companion pack as a buddy (M6).

### Platform caveats
| Platform | Notes |
|----------|-------|
| macOS | Transparent windows need Tauri's `macOSPrivateApi`, which rules out the **Mac App Store**. Direct download and Homebrew are fine (SPEC §10 distribution plan). |
| Windows | Works as-is. |
| Linux X11 | Works as-is. |
| Linux Wayland | The compositor controls window position and always-on-top; the buddy may not stay put. Fallback: a normal small window (polish phase). |

### Effort estimate
- Basic version: ~1 day (done).
- Polish: ~1–2 days.
