# 0002 — Desktop buddy as a second transparent window

- Status: accepted
- Date: 2026-11-07

## Context
We want the companion visible on the desktop while the main window is closed (concept and
design rules in `docs/ROADMAP.md` → Desktop Buddy). It extends SPEC §3.4 "mini mode".

## Decision
- A second Tauri window labelled `buddy`: frameless, transparent, always on top, no taskbar
  entry, `focusable: false` (it must never steal keyboard focus), visible on all workspaces.
  It loads the same UI bundle; `main.tsx` renders `<Buddy />` when the window label is `buddy`.
- No new backend state: core events go to every window, so the buddy just subscribes like the
  main window. It speaks only via check-in events (bubble auto-fades after 10 s).
- Position is owned by the core: `Moved` events are saved (debounced 600 ms) to
  `settings.buddy`, and `update_settings` never takes x/y from the UI. A saved position is only
  reused if it still lies on a connected display; otherwise it goes bottom-right of the primary work area.
- Tray icon (open / show-hide companion / quit). With the buddy enabled, closing the main
  window hides it instead of quitting. Tray strings are read from `ui/src/i18n/en.json`.
- macOS transparency needs `macOSPrivateApi` (no Mac App Store; direct/Homebrew distribution is unaffected).

## Consequences
- Whole-window click-through only; per-pixel click-through, full-screen auto-hide and the
  Wayland fallback are polish items on the roadmap.
- Window helpers live in `core/src/windows.rs` (behind the `tauri` feature) so commands in
  `core/src/commands.rs` can use them.
