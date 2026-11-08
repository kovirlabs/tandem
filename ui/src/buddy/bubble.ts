import type { CheckinEvent } from "../ipc";
import { tDynamic } from "../i18n";

/** How long a bubble stays up before fading (ms). */
export const BUBBLE_MS = 10_000;

/** Text for a check-in bubble, or null when the companion should stay silent. */
export function bubbleText(c: CheckinEvent): string | null {
  if (c.action === "skip" || c.action === "silent_glance") return null;
  return c.text ?? tDynamic(`checkin.fallback.${c.reason}`);
}
