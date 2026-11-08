import type { CharacterState } from "../ipc";

/**
 * Rive state-machine number input values for each companion state.
 * Packs declare the input names in character.json; values are shared by all packs.
 */
export const STATE_VALUES: Record<CharacterState, number> = {
  idle: 0,
  working: 1,
  listening: 2,
  thinking: 3,
  talking: 4,
  waiting: 5,
  celebrating: 6,
  break: 7,
  sleeping: 8,
};

export const ALL_STATES = Object.keys(STATE_VALUES) as CharacterState[];

/** Dam growth for the beaver: 0–100, quantized so it grows stick by stick. */
export function damLevel(progress: number, sticks = 8): number {
  const p = Math.min(1, Math.max(0, Number.isFinite(progress) ? progress : 0));
  return Math.round(Math.floor(p * sticks) * (100 / sticks));
}

export interface CharacterPack {
  id: string;
  name: string;
  rive_file: string;
  state_machine: string;
  inputs: { state: string; progress?: string; mouth?: string };
}

/**
 * How open the mouth is, 0–1, for the SVG fallbacks. `level` is the companion's speech
 * loudness (`voice://level`, speaker), or `null` when no audio drives the mouth.
 * Reduced motion: a fixed slightly-open mouth while talking, closed otherwise.
 */
export function mouthOpenness(level: number | null, state: CharacterState, reducedMotion: boolean): number | null {
  if (reducedMotion) return state === "talking" ? 0.3 : 0;
  if (level === null || (state !== "talking" && level <= 0.02)) return null;
  return Math.min(1, Math.max(0, Number.isFinite(level) ? level : 0));
}

/** Value for the Rive `mouthOpen` number input (0–100), or `null` to leave it alone. */
export function mouthOpenValue(level: number | null, state: CharacterState, reducedMotion: boolean): number | null {
  const open = mouthOpenness(level, state, reducedMotion);
  return open === null ? null : Math.round(open * 100);
}
