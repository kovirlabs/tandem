# 0001 — Stack and M0/M1 implementation decisions

- Status: accepted
- Date: 2026-11-07

## Context
SPEC §5.1 fixes the overall stack (Tauri 2, React/TS, Rust core, SQLite, Ollama + cloud
adapters). This ADR records the concrete choices made while building M0 (skeleton) and M1
(text MVP), and the versions in use.

## Decisions
1. **Crate layout.** The core crate's package is named `core` (so `cargo test -p core`
   works as documented), with lib name `tandem_core` so it doesn't shadow Rust's built-in
   `core` crate. Tauri commands live in `core/src/commands.rs` behind a `tauri` cargo feature,
   so core tests build without a WebView.
2. **IPC types** are generated with `ts-rs` (12.x) into `ui/src/ipc/generated/` by
   `pnpm gen:ipc`. Command wrappers in `ui/src/ipc/commands.ts` are hand-written over these
   types; ESLint forbids importing `invoke` outside `ui/src/ipc/`.
3. **Timers** use `tokio::time::Instant` in a single engine (`session/engine.rs`) ticked once
   a second, so tests run with paused virtual time. The FSM (`session::next_phase`) is a pure function.
4. **Storage:** `rusqlite` (bundled SQLite) with embedded SQL migrations tracked by
   `PRAGMA user_version`. Settings/profile are JSON values in key-value tables.
5. **Secrets:** `keyring` 3.x (pinned; 4.x has a different API) behind a `SecretStore` trait.
6. **Locality:** loopback → Local; RFC1918, link-local, CGNAT (100.64/10, e.g. Tailscale),
   IPv6 ULA/link-local and `.local`/`.lan`/`.home.arpa` → Lan; Lan is treated as local only
   after the user confirms the host. Everything else → Cloud.
7. **Anthropic adapter** is raw HTTP (no official Rust SDK): `anthropic-version: 2026-10-21`,
   default model `claude-opus-5-5` at effort `low`, server-side refusal fallbacks enabled
   (`fallbacks: "default"`), eager tool-input streaming with local validation, and the
   assistant's raw content blocks are replayed unchanged within tool loops (thinking blocks).
   No `temperature` is sent (rejected on this API version). Users can pick a faster/cheaper
   model in Settings.
8. **Tool calling:** schemas live once in `prompts/tools.json`; calls are parsed and validated
   in `conversation/tools.rs` (tolerant of local-model quirks like numeric strings, clamped
   ranges). Models without native tools use a `<tool>{json}</tool>` text protocol validated the same way.
9. **Crisis safety net:** in addition to the prompt safety block, the core detects crisis
   phrases (`prompts/crisis_patterns.txt`) and appends resources
   (`prompts/crisis_resources.md`) if the model's reply lacks them, including when no provider
   is reachable. Added after evals showed a 7B local model omitting resources for indirect phrasing.
10. **Character:** the Rive runtime WASM is bundled locally (`RuntimeLoader.setWasmUrl`) so
    the app never fetches it from a CDN (no unconfigured network calls), and is lazy-loaded
    only when a pack ships a `.riv`. Until the beaver `.riv` exists, an SVG fallback renders all states.
11. **Frontend tooling:** Vite 8, React 19, Zustand 5, Vitest 5. TypeScript is pinned to 6.x
    because typescript-eslint doesn't support TS 7 yet.
12. **License:** manifests say Apache-2.0 as a placeholder; SPEC §10 leaves this open. Confirm before release.

## Consequences
- Adding a provider = implement `ChatProvider` + honest `locality()` + router tests.
- Prompt changes are verified with `pnpm evals` against a real model; the eval runner uses
  the real orchestrator and tools, not a copy.
