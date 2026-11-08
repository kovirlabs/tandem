# Contributing to Tandem

Thanks for helping! Please read [`CLAUDE.md`](../CLAUDE.md) first — its non-negotiable rules
(privacy, no surveillance of others, no shame language, crisis handling, secrets in the
keychain) apply to every contribution, human or AI.

## Layout
- `core/` — all business logic (Rust). Tauri commands live in `core/src/commands.rs` behind the `tauri` feature.
- `apps/desktop/src-tauri/` — the thin Tauri shell (event forwarding, setup).
- `ui/` — React + TypeScript. Talk to the core only via `ui/src/ipc/`.
- `prompts/` — prompts and tool schemas are data. `evals/scenarios/` hold prompt evals.
- `characters/` — character packs.

## Before opening a PR
```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p core --features evals
pnpm lint && pnpm typecheck && pnpm test
pnpm evals    # if you touched prompts/ or core/src/conversation/
```
If you change a Rust type that crosses IPC, run `pnpm gen:ipc` and commit the result.

## Conventions
- Conventional Commits (`feat:`, `fix:`, `docs:` …).
- Record architectural decisions as `docs/adr/NNNN-title.md`.
- User-facing strings go in `ui/src/i18n/en.json`. Never hard-code the companion's name.
- Every interactive element needs a label and keyboard access; respect reduced motion.
