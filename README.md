# Tandem

An open source AI **body double** for people with ADHD (Kovir Labs). A friendly animated
companion — Timber the Busy Beaver by default — works alongside you, helps you start, and
checks in gently. Runs on local [Ollama](https://ollama.com) by default, or on a cloud
provider you configure. See [`docs/SPEC.md`](docs/SPEC.md) for the product spec and
[`CLAUDE.md`](CLAUDE.md) for the rules every change must follow.

## Status: M0 + M1 (text MVP)
Working now:
- Focus sessions with timers owned by the Rust core: pause/resume, extend, end, breaks, subtasks, quiet mode
- Check-in engine (interval, halfway, 5/2-minute heads-up, session end) with silent/gentle/conversational modes and deep-focus protection hooks
- Chat with streaming replies and LLM tool calling (native tools, with a JSON fallback for models without them)
- Providers: Ollama (local or LAN) and Anthropic; API keys only in the OS keychain
- Privacy router: image/audio requests to non-local providers are blocked in `core/src/router/privacy.rs`
- Crisis safety: prompt safety block plus a deterministic resource safety net
- SQLite history, settings, and profile (companion rename, tone, about-me)
- Placeholder SVG beaver with all nine states (the Rive pack isn't made yet)
- Prompt evals (`pnpm evals`)

Not yet: voice (M2), camera (M3), mini mode, OpenAI-compatible adapter, the `.riv` art, input-activity signals.

## Quick start
Prerequisites: Rust (stable), Node 24 + pnpm, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
pnpm install
pnpm tauri dev
```
Then open **Settings → AI provider**. For an Ollama server elsewhere on your network, enter its
address (e.g. `http://192.168.1.20:11434`), press **Find models**, and tick "I control this
computer" so it's treated as local for privacy rules. For Anthropic, paste an API key; it's
stored in your system keychain.

## Development
```bash
cargo test -p core            # Rust unit + integration tests (mock provider, no network)
cargo clippy --workspace --all-targets --all-features -- -D warnings
pnpm lint && pnpm typecheck && pnpm test
pnpm gen:ipc                  # regenerate ui/src/ipc/generated from Rust types
OLLAMA_URL=http://localhost:11434 OLLAMA_MODEL=qwen2.5:7b pnpm evals
EVAL_PROVIDER=anthropic ANTHROPIC_API_KEY=... pnpm evals
```
See [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) and the ADRs in [`docs/adr/`](docs/adr/).

## License
Tandem is licensed under the **GNU General Public License v3.0 or later** (see [`LICENSE`](LICENSE)).
We chose GPL-3.0 so the app can use the maintained [Piper](https://github.com/OHF-Voice/piper1-gpl)
text-to-speech engine (ADR 0003). Character packs and voice models carry their own licenses
inside each pack or model card.
