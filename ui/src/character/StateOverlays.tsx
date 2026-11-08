import type { CharacterState } from "../ipc";

/** Small state cues shared by the built-in SVG characters (thought dots, zzz, confetti…). */
export function StateOverlays({ state }: { state: CharacterState }) {
  switch (state) {
    case "thinking":
      return (
        <g className="thought">
          <circle cx="210" cy="60" r="5" />
          <circle cx="226" cy="44" r="7" />
          <circle cx="248" cy="28" r="10" />
        </g>
      );
    case "listening":
      return (
        <g className="listen-arcs">
          <path d="M202 70 q10 14 0 28" />
          <path d="M212 62 q16 22 0 44" />
        </g>
      );
    case "sleeping":
      return (
        <text className="zzz" x="200" y="60">
          z z z
        </text>
      );
    case "waiting":
      return (
        <text className="zzz" x="40" y="60">
          …
        </text>
      );
    case "celebrating":
      return (
        <g className="confetti">
          <rect x="60" y="30" width="8" height="8" />
          <rect x="250" y="40" width="8" height="8" />
          <rect x="200" y="20" width="8" height="8" />
          <rect x="110" y="16" width="8" height="8" />
        </g>
      );
    default:
      return null;
  }
}
