# CLAUDE.md — Tandem (AI Body-Doubling Companion)

Guidance for Claude Code when working in this repository. Read `docs/SPEC.md` for the full product spec. This file covers **how to work in the codebase** and the **rules that must not be broken**.

## What this project is
Tandem is an open source (Kovir Labs) **desktop** app that acts as an AI body double for people with ADHD. An animated Rive character "works alongside" the user (default: **Timber the Busy Beaver**; a customizable **Human Companion** pack is planned for M6), talks with them by voice and text, runs focus sessions with gentle check-ins, and can optionally use **webcam snapshots** to sense presence and focus. The AI runs on **local Ollama** by default, or on a user-configured cloud provider.

## Non-negotiable rules (read before every change)
1. **Images never touch disk.** Camera frames live in memory only: capture → downscale (≤512 px) → send → drop. The only exception is the explicit `debug.save_snapshots` setting, which is off by default and clearly labeled. Never add caching, logging, or crash dumps that could contain image bytes.
2. **Image requests to a Cloud provider are blocked in `core/src/router/privacy.rs`** unless `session.cloud_vision == true`. The check must stay in the Rust router. The UI is never the only gate. Any change to this module needs tests that cover every branch.
3. **No surveillance-of-others features.** Do not add remote viewing, sharing of camera-derived data, employer or parent dashboards, or focus reports sent off-device. If a task seems to need one of these, stop and flag it.
4. **Vision prompts must not request** identity, emotions, appearance, health, or descriptions of the surroundings. Output must match the JSON contract in SPEC §6.5.
5. **No shame language.** Companion copy and prompts must never guilt, scold, score, or moralize. Praise effort. Keep it brief. Run `pnpm evals` after any prompt change.
6. **Crisis handling stays intact.** The safety block in `prompts/safety.md` is always included in the system prompt and must not be removed or weakened. Its evals must pass.
7. **Secrets only in the OS keychain** (`keyring` crate). Never in SQLite, config files, logs, or the frontend.
8. **Timers and session state are owned by the Rust core.** The UI only renders `session://*` events and sends commands.
9. **Keystroke content is never captured.** Input activity is counts and timestamps only.
10. **No telemetry by default.** Network calls go only to providers the user has configured.

## Tech stack
- **Shell:** Tauri 2 (Rust)
- **UI:** TypeScript, React, Vite, Zustand, `@rive-app/react-canvas`
- **Core:** Rust (tokio, rusqlite/sqlx, keyring, cpal for audio, ort for Silero VAD)
- **AI:** Ollama HTTP API · Anthropic · OpenAI-compatible adapters
- **Speech:** whisper.cpp (STT) and Piper (TTS) as Tauri sidecars
- **DB:** SQLite (migrations in `core/migrations/`)

## Repository layout
```
apps/desktop/            # Tauri app (src-tauri config, sidecar binaries manifest)
ui/                      # React frontend
  src/character/         # Rive wrapper + state mapping
  src/panels/            # Timer, Chat/Captions, Settings
  src/ipc/               # Typed wrappers over Tauri commands/events (generated types)
core/                    # Rust crate: all business logic
  src/session/           # FSM, timers, events
  src/checkin/           # Scheduler + signal fusion
  src/conversation/      # Orchestrator, prompt assembly, tools, memory
  src/router/            # Provider router + privacy.rs (enforcement)
  src/providers/         # ollama.rs, anthropic.rs, openai_compat.rs, mock.rs
  src/snapshot/          # Capture → downscale → route → drop
  src/audio/             # Mic/speaker I/O, VAD, STT/TTS sidecar clients
  src/store/             # SQLite repo layer
  migrations/
prompts/                 # persona.md, tones/*.md, safety.md, vision.md, tools.json
characters/beaver/       # Default Busy Beaver pack (see SPEC §3.4, §6.6)
characters/human/        # Human Companion pack (M6; customizable, see SPEC §3.4.1)
ui/src/character/customizer/  # Schema-driven customizer UI (generic, no pack-specific code)
evals/                   # YAML prompt scenarios + rubric runner
docs/                    # SPEC.md, adr/, CONTRIBUTING.md
```

## Common commands
```bash
pnpm install                 # install JS deps
pnpm tauri dev               # run the app in dev mode
pnpm tauri build             # production build for current OS
pnpm lint && pnpm typecheck  # UI checks
pnpm test                    # UI unit tests (Vitest)
cargo test -p core           # Rust unit + integration tests (uses mock provider)
cargo clippy --all-targets -- -D warnings
cargo fmt --all
pnpm evals                   # prompt evals (needs OLLAMA_URL or a cloud key in env)
pnpm gen:ipc                 # regenerate TS types from Rust command/event definitions
```
Before calling a task done, run `cargo fmt`, `cargo clippy`, `cargo test -p core`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`. Also run `pnpm evals` if anything in `prompts/` or `conversation/` changed.

## Conventions
- **Rust:** `thiserror` for library errors and `anyhow` only at the app boundary. No `unwrap()` outside tests. Async with tokio. Every Tauri command lives in `core/src/commands.rs` and gets a generated TS type.
- **TypeScript:** strict mode, no `any`. Components are function components. IPC only goes through `ui/src/ipc/`. Never call `invoke` directly from components.
- **Companion name is a variable.** Never hard-code "Timber" in prompts, UI copy, captions or wake-word config. Use `{{companion_name}}` in `prompts/` and the `companionName` selector in the UI, both read from `profile.companion.name` (default "Timber"). The name never changes with the selected voice.
- **Prompts are data.** Keep them in `prompts/*.md`, not inline strings. Tools are defined once in `prompts/tools.json` and validated on both sides.
- **Provider adapters** must implement `locality()` honestly and support streaming. Add a `mock.rs` scenario for any new behavior.
- **Accessibility:** every interactive element needs a label and keyboard access. Respect `prefers-reduced-motion` (the character drops to minimal idle). All speech is captioned.
- **Strings:** user-facing text goes in `ui/src/i18n/en.json`.
- **Commits:** Conventional Commits (`feat:`, `fix:`, `docs:`, ...). Record architectural decisions as ADRs in `docs/adr/NNNN-title.md`.

## How to…
**Add a provider:** implement `ChatProvider` (plus `SttProvider`/`TtsProvider` if relevant) in `core/src/providers/`, register it in `router/mod.rs`, set `locality()` correctly, add settings UI, and add router privacy tests for it.

**Add a character state:** add it to the `CharacterState` enum (Rust) → `pnpm gen:ipc` → map it in `ui/src/character/stateMap.ts` → update the required-inputs list in the pack validator → add the input to the default `.riv` file → document it in SPEC §3.4.

**Work on character packs:**
- Every pack must provide all required state inputs. The validator in `core/src/character/validate.rs` enforces this.
- Customization comes **only** from the `customization` schema in `character.json`. The customizer UI is generic. Never hard-code pack-specific controls.
- **Human Companion rules:** stylized art only (no photorealism). Features are chosen independently. **Never add ethnicity or race presets or labels**, and never bundle features into stereotyped combinations. Voice is never paired with appearance automatically. Starter looks stay unlabeled. Gaze stays mostly on the character's own work.
- Add a render test that cycles through every enum option and a sample of colors in every state (`pnpm test:characters`).
- Appearance is stored in `profile` under `companion.appearance.<character_id>`.

**Change companion behavior or tone:** edit `prompts/`, add or adjust scenarios in `evals/scenarios/`, and run `pnpm evals`. Keep spoken replies to 2 sentences or fewer by default.

**Touch the snapshot pipeline:** re-read rules 1–4. Add a test proving that no file is written (the test harness provides a temp-dir write watcher).

## Testing notes
- Use `providers::mock` for deterministic streams in Rust tests. Never hit real APIs in `cargo test`.
- Ollama integration tests are `#[ignore]` and run nightly with `OLLAMA_URL` set.
- `core/src/router/privacy.rs` needs 100% branch coverage (`cargo llvm-cov` in CI).

## Domain notes for Claude
- Body doubling works through **presence, low-friction starts, and external time cues**. When unsure, choose the option that is quieter, gentler, and needs fewer user steps.
- "Tiny first step" is the go-to move when a user is stuck: one concrete action that takes 2 minutes or less.
- Deep focus is precious. When in doubt, skip the check-in.
- The users may be sensitive to judgment. Write all copy as a friendly coworker would.

## Out of scope unless asked
Mobile apps, multi-user rooms, calendar or task-manager integrations, cloud sync, analytics. Mention these as follow-ups instead of building them.
