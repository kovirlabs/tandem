import { useEffect, useRef, useState, type MouseEvent } from "react";
import { Character } from "../character/Character";
import { commands, showContextMenu, startDragging } from "../ipc";
import { t } from "../i18n";
import { act, formatClock } from "../store/actions";
import { useApp, useCompanionName } from "../store/app";
import { connectCore } from "../store/bridge";
import { MicButton } from "../voice/MicButton";
import { BUBBLE_MS, bubbleText } from "./bubble";

/** Pixels the pointer must move before a press becomes a drag instead of a click. */
const DRAG_THRESHOLD = 4;

/**
 * The desktop buddy: the companion in a small transparent, always-on-top, non-focusable
 * window. It only renders core events; it never speaks on its own (docs/ROADMAP.md rules).
 */
export function Buddy() {
  const name = useCompanionName();
  const character = useApp((s) => s.character);
  const session = useApp((s) => s.session);
  const showTimer = useApp((s) => s.settings?.show_timer ?? true);
  const characterId = useApp((s) => s.profile?.companion.character_id ?? "beaver");
  const lastCheckin = useApp((s) => s.lastCheckin);
  const voicePhase = useApp((s) => s.voice?.phase ?? "off");
  // The check-in (by timestamp) whose bubble has been dismissed or timed out.
  const [hiddenAt, setHiddenAt] = useState<number | null>(null);
  const press = useRef<{ x: number; y: number; dragging: boolean } | null>(null);

  useEffect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    connectCore().then((un) => (cancelled ? un() : (cleanup = un)));
    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, []);

  const bubble = lastCheckin && lastCheckin.at !== hiddenAt ? bubbleText(lastCheckin) : null;
  useEffect(() => {
    if (!lastCheckin) return;
    const at = lastCheckin.at;
    const timer = setTimeout(() => setHiddenAt(at), BUBBLE_MS);
    return () => clearTimeout(timer);
  }, [lastCheckin]);

  const phase = session?.phase ?? "idle";
  const inSession = phase === "focusing" || phase === "paused_manual" || phase === "paused_away";
  const clock =
    phase === "break" ? session?.break_remaining_secs : inSession ? session?.remaining_secs : undefined;

  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0) return;
    press.current = { x: e.screenX, y: e.screenY, dragging: false };
  };
  const onMouseMove = (e: MouseEvent) => {
    const p = press.current;
    if (!p || p.dragging) return;
    if (Math.abs(e.screenX - p.x) + Math.abs(e.screenY - p.y) > DRAG_THRESHOLD) {
      p.dragging = true;
      void startDragging();
    }
  };
  const onMouseUp = () => {
    const p = press.current;
    press.current = null;
    if (p && !p.dragging) void act(commands.showMainWindow);
  };
  const onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    void showContextMenu([
      { text: t("buddy.menu.open"), action: () => void act(commands.showMainWindow) },
      ...(phase === "focusing" ? [{ text: t("buddy.menu.pause"), action: () => void act(commands.pauseSession) }] : []),
      ...(phase === "paused_manual" || phase === "paused_away"
        ? [{ text: t("buddy.menu.resume"), action: () => void act(commands.resumeSession) }]
        : []),
      ...(inSession ? [{ text: t("buddy.menu.quiet"), action: () => void act(() => commands.setQuiet(30)) }] : []),
      null,
      { text: t("buddy.menu.hide"), action: () => void act(() => commands.setBuddyVisible(false)) },
    ]);
  };

  return (
    <div className={`buddy voice-${voicePhase}`}>
      <div className="bubble-slot" aria-live="polite">
        {bubble && (
          <button className="bubble" onClick={() => setHiddenAt(lastCheckin?.at ?? null)} aria-label={`${bubble} (${t("buddy.dismiss")})`}>
            {bubble}
          </button>
        )}
      </div>
      <div
        className="buddy-body"
        role="button"
        tabIndex={-1}
        aria-label={t("buddy.label", { name })}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
        onContextMenu={onContextMenu}
      >
        <Character key={characterId} characterId={characterId} companionName={name} state={character} progress={session?.progress ?? 0} />
        {clock !== undefined && (
          <span className="buddy-clock">{showTimer ? formatClock(clock) : phase === "break" ? "☕" : "•"}</span>
        )}
      </div>
      {/* Push-to-talk from the buddy: shows on hover, and stays visible while listening. */}
      {voicePhase !== "off" && <MicButton compact className="buddy-mic" />}
    </div>
  );
}
