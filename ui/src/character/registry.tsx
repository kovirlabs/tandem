import type { CharacterState } from "../ipc";
import { BeaverFallback } from "./BeaverFallback";
import { FoxFallback } from "./FoxFallback";

export interface FallbackProps {
  state: CharacterState;
  progress: number;
  reducedMotion: boolean;
  /** Speech loudness 0–1 driving the mouth, or `null` (see `mouthOpenness`). */
  mouth: number | null;
}

/** Installed character packs (folders under `characters/`); the settings picker lists these. */
export const CHARACTER_IDS = ["beaver", "fox"] as const;
export const DEFAULT_CHARACTER = "beaver";

/** Built-in SVG drawing for a pack, used until its Rive file exists. Unknown ids get the beaver. */
export function CharacterFallback({ id, ...props }: FallbackProps & { id: string }) {
  switch (id) {
    case "fox":
      return <FoxFallback {...props} />;
    default:
      return <BeaverFallback {...props} />;
  }
}
