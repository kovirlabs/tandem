# Tandem — AI Body-Doubling Companion

**Product & Technical Specification · v0.1 (draft)**
Kovir Labs · Open source · Desktop-first
*"Tandem" is a working name. Check for name conflicts before public release.*

---

## 1. Summary

Tandem is a desktop app that acts as a **body double** for people with ADHD (and anyone who struggles to start or stay with tasks). An animated character sits on screen and "works alongside" the user. It talks with them by voice or text, helps them break tasks down, checks in at gentle intervals, and can optionally glance at webcam snapshots to tell whether the user is still present and on task.

The AI runs on a **local Ollama** model by default, or on a **cloud LLM** the user configures (bring your own key). **Camera snapshots stay local by default.** Sending images to a cloud provider takes an explicit opt-in for each session.

### 1.1 Why body doubling works (design basis)
Body doubling means doing a task while someone else is present. People with ADHD report that it helps, and the reasons shape the design:
- **Social presence and soft accountability.** Someone is "there," so drifting off is less automatic.
- **Lower activation energy.** Saying the task out loud to someone makes it easier to start.
- **External time anchoring.** Check-ins work against time blindness.
- **Co-regulation.** A calm companion who doesn't judge reduces avoidance driven by anxiety.

Tandem has to feel like a **friendly coworker in the room**. It must never feel like a supervisor, a surveillance camera, or a nagging app.

### 1.2 Goals
1. Starting a focus session should take under 10 seconds: say or type a task and go.
2. Hold a natural back-and-forth voice conversation with low latency.
3. Provide ambient presence through an animated character that visibly "works" too.
4. Offer optional, privacy-first camera awareness that adjusts check-ins and never scolds.
5. Run fully offline on reasonable hardware with Ollama, with cloud as an option.
6. Be open source, easy to extend (providers, characters, voices), and easy to self-host.

### 1.3 Non-goals
- **No monitoring of other people.** There are no employer, parental, or remote-viewer modes. The person in front of the camera is always the user who started the session, and nothing is sent to anyone else. Proposed features that break this rule are rejected.
- **Not a medical device and not therapy.** Tandem makes no claims to diagnose or treat ADHD.
- No full task-manager or calendar replacement in v1. Integrations come later.
- No mobile app in v1. The architecture should not prevent one later.

---

## 2. Users & Scenarios

**Primary persona:** an adult with ADHD who works or studies at a computer, has trouble starting tasks, loses time, and drifts into distractions.

**Secondary persona:** anyone who wants a quiet coworking presence, such as remote workers or students doing homework with a parent nearby.

### Core scenarios
| # | Scenario | What Tandem does |
|---|----------|------------------|
| S1 | "I need to do my expense report but I keep avoiding it." | Asks one short question, helps pick a tiny first step, starts a 25-min session, and the character starts "working." |
| S2 | The user goes quiet for 20 minutes and is clearly working. | Stays silent. At most, a soft visual cue at the check-in point. Never interrupts deep focus. |
| S3 | The user drifts to their phone (camera on). | At the next scheduled snapshot it notices low focus signals and offers a gentle, optional verbal nudge: "Still with the expense report, or want to switch?" |
| S4 | The user walks away. | Snapshot shows nobody present. Tandem pauses the timer, the character goes to "waiting," and it greets the user warmly on return. |
| S5 | The user says "this is too big." | Breaks the task into 3–5 concrete subtasks and lets the user pick one. |
| S6 | Session ends. | Short celebration, a one-line reflection ("What got done?"), and an offer of a break timer or the next session. |
| S7 | The user is offline or on a plane. | Everything works with local Ollama, local STT and local TTS. |

---

## 3. Functional Requirements

Priority: **P0** = MVP, **P1** = v1.0, **P2** = later.

### 3.1 Sessions & Tasks
- **P0** Start a session by voice or text with a task name and an optional duration (default 25 min, configurable 5–120).
- **P0** Visual timer with remaining time. The time display can be hidden for users who find a countdown stressful.
- **P0** Pause, resume, extend (+5/+10), and end early, all without guilt-laden copy.
- **P0** Breaks: short break (5 min default) and long break, with an optional Pomodoro-style cycle.
- **P1** Subtasks: AI-assisted breakdown and a manual checklist.
- **P1** Transition warnings: "5 minutes left" and "2 minutes left," which help with switching tasks.
- **P1** Session history with a local, private log of tasks, durations and completion notes.
- **P2** Import tasks from Todoist, Microsoft To Do, Odoo, or plain markdown files.

### 3.2 Check-in Engine
- **P0** Scheduled check-ins (default: every 10 min, plus at the halfway point and the end).
- **P0** Check-in modes per session:
  - **Silent:** the character only animates a glance or wave.
  - **Gentle:** a short voice or text line with no question required.
  - **Conversational:** asks a question and waits for a reply.
- **P0** Deep-focus protection: if the user is typing or active (camera or input activity) and focus signals are high, skip or soften the check-in.
- **P1** Adaptive cadence: increase check-ins after repeated drift and decrease them after sustained focus. The user can always override.
- **P1** "Don't talk for N minutes" voice command.

### 3.3 Conversation (Voice & Text)
- **P0** Text chat panel. It is always available and has full parity with voice.
- **P0** Push-to-talk using a global hotkey (default `Ctrl/Cmd+Shift+Space`) and an on-screen button.
- **P1** Hands-free mode with voice-activity detection (VAD) and an optional wake word. The default wake phrase is **"Hey Timber"**. A two-syllable name plus "Hey" reduces false triggers. When the user renames the companion, the wake phrase follows the new name. Names of one syllable or very common words show a warning that the wake word may trigger by accident.
- **P1** Barge-in: when the user starts talking, TTS stops right away.
- **P0** Spoken replies are short by default (1–2 sentences) and longer only when asked.
- **P0** The LLM can call tools (see §6.4) to start, pause and extend timers, add subtasks, schedule check-ins and start breaks.
- **P1** Conversation memory within a session. A per-user profile stored locally holds things like preferred tone and typical session length.

### 3.4 Animated Character
- **P0** Default character: **Timber the Busy Beaver**, an original, friendly cartoon beaver (not based on any existing IP) rendered with a **Rive** state machine. Rationale: a beaver reads as "industrious" at a glance, feels like company rather than a supervisor (important for judgment-sensitive users and next to a camera), avoids the uncanny valley, and gives the project a memorable mascot.
- **P0** Name & voice: the default name is **Timber**, a single gender-neutral name that never changes with the chosen voice. The default voice is a warm, mid-range, gender-ambiguous Piper voice (chosen in M2 by a listening test). Users can **rename** the companion and pick **any** voice in Settings → Companion. The name is stored in `profile` (`companion.name`) and injected into prompts, captions and UI copy.
- **P0** States: `idle`, `working`, `listening`, `thinking`, `talking`, `waiting` (user away), `celebrating`, `break`, `sleeping`.
  - Beaver flavor: `working` = gnawing or stacking sticks on a small dam that **grows as the session progresses**. `celebrating` = tail slap. `break` = floating on its back. `sleeping` = curled in the lodge.
- **P0** Mouth movement driven by TTS output amplitude. Viseme-based lip sync is **P2**.
- **P1** Compact always-on-top "mini mode" (a small floating window in a corner).
- **P1** Character pack format so the community can contribute characters (§6.6).
- **P1** **Human Companion pack** (customizable person, see §3.4.1).
- **P2** Small expressions such as nods, thumbs-up, yawns on long sessions, and seasonal outfits.
- **P2** Beaver dam "collection": each completed session adds a stick to a persistent dam. It is cosmetic only, with no streaks and no loss.

#### 3.4.1 Human Companion pack (P1)
An optional, stylized (cartoon or flat illustration, not photoreal) human co-worker that the user can customize to look like them or like someone they'd find comforting.

- **One rig, many looks.** A single Rive file with bound inputs, so there is no separate artboard per variation.
- **Customization options:**
  | Option | Control |
  |--------|---------|
  | Body presentation | Masculine · Feminine · Neutral (body shape, shoulders, default clothing cut) |
  | Skin tone | Continuous picker plus ~12 curated swatches covering a wide range of tones |
  | Face | Choice of several eye, nose and brow shapes, chosen independently |
  | Hair | Style (short, long, curly, coily, locs, braids, waves, buzz, bald, head covering) and color |
  | Facial hair | None plus several styles |
  | Glasses | None plus several frames |
  | Outfit | Hoodie, sweater, button-up, scrubs, and more, with color |
  | Age feel | Younger · Middle · Older (subtle: gray hair option, lines) |
  | Accessibility | Optional hearing aid and wheelchair or desk-chair variants |
- **No ethnicity presets or labels.** Users build the look from independent features. This avoids stereotyped bundles and covers mixed and unlisted identities. A "Randomize" button and a few unlabeled starter looks help with choice paralysis.
- **Voice is chosen separately** from appearance, with no automatic pairing by presentation.
- Appearance is saved locally in `profile` (`companion.appearance`) and can be changed any time from Settings → Companion.
- The same states as the beaver, with human equivalents: typing, sipping coffee, stretching on break, a thumbs-up or small fist pump on celebrate.
- **Art direction:** warm, simple and soft. A friendly coworker, not an avatar that watches you. Eyes glance at "their own work" most of the time and look toward the user mainly when talking or greeting.
- The art can be commissioned or contributed by the community through the pack format. A diverse review of the art (people across the represented features) is required before release.

### 3.5 Camera Awareness (Optional)
- **P0** Off by default. Turning it on requires a clear explanation screen and consent.
- **P0** Takes **single snapshots** at intervals (default every 5 min, configurable 1–15). It never records continuous video.
- **P0** Snapshots are downscaled (max 512 px long side), held **in memory only**, sent to the vision model, and discarded. They are never written to disk unless the user turns on a clearly labeled debug option.
- **P0** **Local-only by default:** images go only to providers marked `local` (§6.3). Cloud vision needs an explicit per-session opt-in with a visible "cloud vision on" badge.
- **P0** A camera-active indicator is always visible while a session uses the camera, and a one-click or hotkey control pauses it.
- **P0** The vision model returns a **structured, minimal assessment** (§6.5): present or not, a coarse focus signal, and an optional short description of activity. No identity, emotion, health or appearance inference.
- **P0** Camera signals only **inform** check-in timing and wording. They never produce shaming language, scores, or "you were distracted X times" reports, unless the user explicitly asks for focus stats (**P2**, off by default).
- **P1** Lightweight on-device presence detection (face or person detector) runs between vision calls to cut LLM load.

### 3.6 Settings & Personalization
- **P0** Provider setup: Ollama URL and model, cloud provider and key, chosen separately for chat, vision, STT and TTS.
- **P0** Voice selection, speech rate, volume, and a mute-all-voice toggle.
- **P0** Companion name (default "Timber"; 1–20 characters, any language), editable at any time.
- **P0** Tone presets: *Calm*, *Cheerful*, *Dry humor*, *Minimal*.
- **P1** "About me" notes the companion can use, such as "I do best with tiny first steps."
- **P1** Accessibility: full keyboard navigation, screen-reader labels, reduced-motion mode, high-contrast theme, captions for all speech.

### 3.7 Wellbeing & Safety
- **P0** The system prompt forbids shaming, guilt and productivity moralizing. Language focuses on effort, not output.
- **P0** If the user expresses crisis or self-harm intent, the companion responds with care, gives region-appropriate crisis resources, and does not try to act as a therapist. This behavior is hard-coded in the prompt and covered by evals (§9).
- **P1** Gentle prompts for breaks, water and movement after long unbroken stretches (90+ min). The user can turn them off.

---

## 4. Non-Functional Requirements

| Area | Requirement |
|------|-------------|
| **Latency** | Voice turn (end of speech → first audio) under **1.5 s** with cloud and under **2.5 s** with local on the reference hardware below. Text first token under 1 s with cloud. |
| **Reference HW (local)** | 8-core CPU, 16 GB RAM, GPU with 8 GB VRAM *or* Apple Silicon with 16 GB. Document smaller-model fallbacks for CPU-only machines. |
| **Resource use (idle session)** | Under 3% CPU and under 400 MB RAM for the app itself, excluding Ollama and models. |
| **Offline** | Fully functional with local providers and no network. |
| **Privacy** | No telemetry by default. Any optional crash reporting is opt-in and contains no transcripts or images. |
| **Security** | API keys are stored in the OS keychain, never in plain-text config. Every outbound endpoint can be listed in settings. |
| **Platforms** | Windows 10+, macOS 13+, Linux (AppImage/deb; Wayland and X11). |
| **Accessibility** | WCAG 2.2 AA for all UI. Captions for all speech. |
| **i18n** | All strings externalized. English ships first. |

---

## 5. Architecture

### 5.1 Stack
| Layer | Choice | Rationale |
|-------|--------|-----------|
| App shell | **Tauri 2** (Rust) | Small binaries, native camera and mic permissions, sidecar support, cross-platform. |
| UI | **TypeScript + React** (Vite) | Large contributor pool. Rive's React runtime is mature. |
| Character | **Rive** (`@rive-app/react-canvas`) | State machines map neatly to companion states. Assets are small. |
| State | Zustand (UI), with session state owned by the Rust core | Single source of truth for timers. |
| Storage | **SQLite** (via `rusqlite` or `sqlx`) | Local, portable, inspectable. |
| Secrets | OS keychain (`keyring` crate) | Keys never touch disk in plain text. |
| Local LLM | **Ollama** HTTP API (localhost or LAN) | Chat and vision models (e.g. Llama, Qwen-VL, Gemma vision variants). |
| Cloud LLM | Provider adapters: Anthropic, OpenAI-compatible (covers OpenAI, OpenRouter, Groq, LM Studio, vLLM) | Bring your own key. One interface. |
| STT | Local **whisper.cpp** sidecar (default) · cloud STT adapter (optional) | Works offline. |
| TTS | Local **Piper** (default, GPL-3.0-or-later via OHF-Voice/piper1-gpl) · optional Kokoro or cloud TTS adapter | Fast, offline voices. Each Piper voice model carries its own license (see its model card). |
| VAD | Silero VAD (ONNX) in the Rust core | Hands-free mode and barge-in. |
| Presence detector (P1) | Small ONNX face or person detector | Cheap checks between vision calls. |

> Check the current versions and licenses of each dependency at implementation time. Record the decisions in `docs/adr/`.

### 5.2 Component diagram

```
┌──────────────────────────── Tauri App ─────────────────────────────┐
│  WebView (React/TS)                                                │
│  ┌──────────────┐ ┌──────────────┐ ┌───────────────┐ ┌───────────┐ │
│  │ Character    │ │ Timer / Task │ │ Chat & Caption│ │ Settings  │ │
│  │ (Rive SM)    │ │ Panel        │ │ Panel         │ │           │ │
│  └──────▲───────┘ └──────▲───────┘ └──────▲────────┘ └─────▲─────┘ │
│         │ events          │ commands/events │                │       │
│ ════════╪═════════════════╪═════════════════╪════════════════╪══════│
│  Rust Core                                                         │
│  ┌───────────────┐  ┌────────────────┐  ┌──────────────────────┐   │
│  │ Session Engine│◄─┤ Check-in       │◄─┤ Signal Bus           │   │
│  │ (timers, FSM) │  │ Scheduler      │  │ (input activity,     │   │
│  └──────┬────────┘  └───────┬────────┘  │  presence, vision)   │   │
│         │                   │           └──────────▲───────────┘   │
│  ┌──────▼───────────────────▼─────┐   ┌────────────┴───────────┐   │
│  │ Conversation Orchestrator      │   │ Snapshot Pipeline      │   │
│  │ (prompt build, tools, memory)  │   │ (capture→downscale→    │   │
│  └──────┬─────────────────────────┘   │  route→discard)        │   │
│         │                             └────────────┬───────────┘   │
│  ┌──────▼─────────────────────────────────────────▼────────────┐   │
│  │ Provider Router  (enforces privacy policy per modality)      │   │
│  │  ├─ OllamaAdapter (local/LAN)  ├─ AnthropicAdapter            │   │
│  │  └─ OpenAICompatAdapter        └─ (future adapters)           │   │
│  └──────────────────────────────────────────────────────────────┘   │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌────────────┐  │
│  │ Audio I/O    │ │ STT sidecar  │ │ TTS sidecar  │ │ SQLite +   │  │
│  │ (cpal, VAD)  │ │ (whisper.cpp)│ │ (Piper)      │ │ Keychain   │  │
│  └──────────────┘ └──────────────┘ └──────────────┘ └────────────┘  │
└────────────────────────────────────────────────────────────────────┘
```

### 5.3 Session state machine

```
            start
  [Idle] ─────────► [Focusing] ◄──────── resume ───────┐
     ▲                 │  │  │                          │
     │        end/done │  │  └─ user away (presence) ─► [Paused:Away]
     │                 │  └──── pause ──────────────► [Paused:Manual]
     │                 ▼                                │
     │          [Wrapping Up] ── break ──► [Break] ─────┘ (next session)
     └──────────────── finish ─────────────┘
```
Timers live only in the Rust core. The UI subscribes to `session://tick` and `session://state` events.

### 5.4 Voice turn pipeline
1. PTT pressed or VAD detects speech → mic audio is buffered (16 kHz mono).
2. Speech ends → STT (streaming where supported) → transcript.
3. The orchestrator builds the prompt: system persona, session context, last N turns, tool schema.
4. The LLM streams back. Tool calls run immediately. Text is split into sentences.
5. The first complete sentence goes to TTS → audio playback starts, and the character switches to `talking` with amplitude-driven mouth movement.
6. Barge-in: if VAD detects user speech, TTS stops, the remaining text is dropped, and the pipeline returns to step 2.

---

## 6. Detailed Design

### 6.1 Data model (SQLite)

```sql
tasks        (id TEXT PK, title TEXT, notes TEXT, created_at, done_at NULL)
subtasks     (id TEXT PK, task_id FK, title TEXT, position INT, done_at NULL)
sessions     (id TEXT PK, task_id FK, planned_min INT, started_at, ended_at NULL,
              state TEXT, checkin_mode TEXT, camera_enabled INT, cloud_vision INT,
              reflection TEXT NULL)
session_events (id INTEGER PK, session_id FK, ts, kind TEXT, payload JSON)
               -- kinds: start, pause, resume, extend, checkin, nudge, away, back, end
messages     (id INTEGER PK, session_id FK, ts, role TEXT, content TEXT, via TEXT)
               -- via: voice|text; retention configurable (default 30 days)
profile      (key TEXT PK, value JSON)        -- tone, about-me notes, preferences
settings     (key TEXT PK, value JSON)        -- non-secret settings only
```
**Images are never stored.** `session_events` stores only the *structured* vision result (§6.5), and only when the user turns on "keep focus signals in history" (default off).

### 6.2 Provider interface

```rust
trait ChatProvider {
    fn id(&self) -> &str;
    fn locality(&self) -> Locality;              // Local | Lan | Cloud
    fn capabilities(&self) -> Caps;              // tools, vision, streaming
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream>;
}
trait SttProvider { fn locality(&self) -> Locality; async fn transcribe(...) }
trait TtsProvider { fn locality(&self) -> Locality; async fn synthesize_stream(...) }
```

### 6.3 Privacy policy enforcement (Provider Router)
- Every request is tagged with a **modality**: `text`, `image`, or `audio`.
- `Locality` comes from the endpoint: `localhost`/`127.0.0.1` → Local. RFC1918 or a user-confirmed host → Lan, which is treated as local only after the user confirms they control it. Anything else → Cloud.
- **Rule:** `image` requests to a `Cloud` provider are **rejected in the router** unless `session.cloud_vision == true`. This check lives in one function with unit tests. The UI is never the only gate.
- If no local vision model is available and cloud vision isn't opted in, camera awareness degrades to the presence detector only, or to off, with a clear message.
- Audio for cloud STT follows the same pattern with a separate opt-in.

### 6.4 LLM tools (function calling)
| Tool | Args | Effect |
|------|------|--------|
| `start_session` | `task`, `minutes?`, `checkin_mode?` | Creates the task if needed and starts the timer |
| `pause_session` / `resume_session` | — | Changes state |
| `extend_session` | `minutes` | Adds time |
| `end_session` | `reflection?` | Wraps up |
| `start_break` | `minutes` | Starts the break timer |
| `add_subtasks` | `items[]` | Adds subtasks to the checklist |
| `complete_subtask` | `id` | Marks done → celebration animation |
| `set_quiet` | `minutes` | Suppresses verbal check-ins |
| `set_character_state` | `state` | Explicit animation cue (rarely needed) |

For models without native tool calling (some local models), fall back to a JSON-mode protocol that is validated against the same schemas.

### 6.5 Vision assessment contract
Prompt the vision model to return **only** this JSON (validated; on parse failure, treat as `unknown`):

```json
{
  "present": true,
  "focus_signal": "engaged | uncertain | away_from_task | not_visible",
  "activity": "typing at computer",       // ≤ 8 words, optional
  "confidence": 0.0
}
```
The prompt explicitly forbids describing the person's appearance or emotions, guessing identity, or commenting on the surroundings. The check-in scheduler combines this result with keyboard and mouse activity (counts only, never keystroke content) and the session state to choose one of: *skip*, *silent glance*, *gentle line*, or *ask*.

### 6.6 Character pack format
```
characters/<id>/
  character.json   # name, author, license, rive file, state-machine name,
                   # input names mapped to Tandem states, default voice id,
                   # optional "customization" schema
  <id>.riv
  preview.png
  LICENSE
```
The core validates that a pack provides every required state input before loading it.

**Optional customization schema.** Packs like the Human Companion declare their adjustable features in `character.json`. The Settings UI builds the customizer from this schema automatically, so no pack-specific UI code is needed.

```json
"customization": {
  "skin_tone":    { "type": "color",  "rive_input": "skinColor", "swatches": ["#F9DCC4", "...", "#3B2219"] },
  "presentation": { "type": "enum",   "rive_input": "bodyType",  "options": ["masculine", "feminine", "neutral"] },
  "hair_style":   { "type": "enum",   "rive_input": "hairStyle", "options": ["short", "curly", "coily", "locs", "braids", "..."] },
  "hair_color":   { "type": "color",  "rive_input": "hairColor" },
  "glasses":      { "type": "enum",   "rive_input": "glasses",   "options": ["none", "round", "square"] }
}
```
Supported types: `color`, `enum`, `bool`, `number` (range). Values are saved per character id in `profile`.

### 6.7 Persona & prompting guidelines
The system prompt is composed from: **base persona** + **tone preset** + **session context** + **user "about me"** + **safety block**. Key rules:
- Act as a coworker in the room, not a boss. Short, warm, concrete.
- Default to one sentence when speaking. Ask at most one question at a time.
- Praise effort and starting, not output.
- Never use guilt, shame, comparisons, or "you should."
- When the user is stuck, offer a **tiny first step** of 2 minutes or less.
- Respect silence. When in doubt, say less.

---

## 7. UX Notes
- **First run:** meet Timber the Busy Beaver (default; "Want to call me something else?" lets the user rename it right away), or pick another installed character. If the Human Companion pack is installed, a quick customizer appears with a "Randomize" button and a "Skip, I'll do this later" option. Then choose a voice → test the mic → choose a provider (detect Ollama automatically if it is running) → camera explanation (skippable; off by default) → "What do you want to work on?"
- **Main window:** character is dominant (≈60%), timer and task to the side, chat and captions collapsible below.
- **Mini mode:** about 200×200 px floating character with a timer ring. Click to expand.
- **Copy tone examples:**
  - Check-in (gentle): "Still cruising on the report? I'm right here."
  - Drift (camera): "Hey — want to hop back in, or switch to something else?"
  - Return from away: "Welcome back. Ready to pick up where you left off?"
  - End: "That's 25 minutes done. Nice. What got finished?"
- **Never:** red warning colors for distraction, focus "scores" on screen by default, or streak-loss penalties.

---

## 8. Milestones

| Milestone | Scope | Exit criteria |
|-----------|-------|---------------|
| **M0 Skeleton** | Tauri + React scaffold, Rust core, SQLite, CI (lint, test, build on 3 OSes) | App launches. CI is green. |
| **M1 Text MVP** | Sessions, timer, check-in engine (text), Ollama chat adapter, tools, first-pass Busy Beaver (idle, working, talking) | Can run a full 25-min session by text with check-ins. |
| **M2 Voice** | PTT, whisper.cpp, Piper, streaming sentence TTS, amplitude lip sync | Voice turn latency target met with local models. |
| **M3 Camera** | Snapshot pipeline, router privacy enforcement, vision contract, scheduler fusion | Away/back pause works. Privacy unit tests pass. No image ever reaches disk. |
| **M4 Cloud + Polish** | Anthropic and OpenAI-compatible adapters, keychain, mini mode, tone presets, accessibility pass | WCAG AA audit passes. |
| **M5 v1.0 Release** | Hands-free VAD, barge-in, character pack format (including the customization schema), all Busy Beaver states, docs, installers | Signed builds for all 3 OSes. Contributor docs complete. |
| **M6 Human Companion** | Human Companion pack: one rig, customization inputs, auto-generated customizer UI, starter looks, diverse art review | Every customization combination renders correctly in every state. Art review is signed off. |

---

## 9. Testing & Evaluation
- **Unit:** session FSM transitions, scheduler decisions, router privacy rules (100% branch coverage on the privacy module), vision JSON parsing.
- **Integration:** a mock provider that replays scripted streams. An Ollama integration suite runs on a nightly CI job with a small model.
- **Prompt evals** (`evals/`): a YAML scenario suite checked against a rubric, covering:
  - Tone: no shame or guilt language (red-team inputs like "I wasted the whole day").
  - Brevity: spoken replies of 2 sentences or fewer by default.
  - Correct tool use for task-start phrases.
  - **Crisis handling:** a caring reply plus resources every time.
  - Vision output: no appearance or emotion descriptions.
- **Manual:** latency measurements on reference hardware, and a screen-reader walkthrough.
- **User testing:** a small cohort of ADHD volunteers testing the felt experience ("did it feel supportive or surveilling?").

---

## 10. Open Source & Project
- **License:** **GPL-3.0-or-later** (decided 2026-11-08, ADR 0003) so Tandem can use the maintained Piper engine, which is GPL-3.0 because it embeds espeak-ng. Character and voice assets carry their own licenses inside each pack.
- **Repo:** Kovir Labs. A mirror on Forgejo or GitHub is optional.
- **Governance:** `CONTRIBUTING.md`, Code of Conduct, ADRs for major decisions.
- **Distribution:** GitHub Releases (signed), with Flathub/Homebrew/winget later.

---

## 11. Risks & Mitigations
| Risk | Mitigation |
|------|------------|
| The camera feels creepy | Off by default, snapshots only, local-only by default, visible indicator, never shame, never store images |
| Misuse for surveilling others | No remote viewing or reporting features at all. Stated as a hard non-goal. |
| Local voice latency is too slow | Sentence-level streaming, small STT models, filler animations (`thinking`) to cover the gap |
| Local models have weak tool calling | JSON-mode fallback with schema validation, plus recommended tested models in the docs |
| Interruptions break focus | Deep-focus protection, quiet mode, adaptive cadence biased toward silence |
| Human figures feel uncanny, stereotyped, or watchful | Stylized art only, features chosen independently with no ethnicity presets, diverse art review, gaze mostly on "own work" |
| Customization overwhelms users | Beaver works with zero setup, "Randomize" and starter looks, customizer is skippable |
| Over-reliance or wellbeing concerns | Break prompts, no streak pressure, crisis handling covered by evals |

---

## 12. Open Questions
1. Should there be a "coworking room" for multiple *humans* (P2)? It would need a careful privacy design.
2. ~~Default character type~~ **Decided:** the Busy Beaver is the default, and the customizable Human Companion pack is planned for M6. The default name is **Timber** (one gender-neutral name, renameable, independent of voice). Still open: should more animal packs ship in the box?
3. Should calendar integration auto-suggest sessions, or is that too pushy?
4. Is a wake word worth the CPU and battery cost, or should push-to-talk plus VAD stay the default?
5. What message retention defaults best balance useful context against privacy (currently 30 days)?
