import type { CharacterState } from "../ipc";
import { damLevel, mouthOpenness } from "./stateMap";
import { StateOverlays } from "./StateOverlays";

interface Props {
  state: CharacterState;
  progress: number;
  reducedMotion: boolean;
  mouth: number | null;
}

const DEN_PIECES = 8;

/** The den grows one piece at a time through the session: mound, entrance, stones, leaves, a flower. */
function DenPiece({ i }: { i: number }) {
  switch (i) {
    case 0:
      return <ellipse className="den-piece earth" cx="0" cy="0" rx="46" ry="14" />;
    case 1:
      return <path className="den-piece earth" d="M-44 0 Q-34 -50 0 -52 Q34 -50 44 0 Z" />;
    case 2:
      return <path className="den-piece den-door" d="M-15 0 Q-15 -28 0 -28 Q15 -28 15 0 Z" />;
    case 3:
      return <circle className="den-piece pebble" cx="-32" cy="-6" r="7" />;
    case 4:
      return <circle className="den-piece pebble" cx="30" cy="-8" r="6" />;
    case 5:
      return <ellipse className="den-piece leaf" cx="-18" cy="-40" rx="10" ry="4" transform="rotate(-25 -18 -40)" />;
    case 6:
      return <ellipse className="den-piece leaf" cx="18" cy="-44" rx="10" ry="4" transform="rotate(20 18 -44)" />;
    default:
      return (
        <g className="den-piece flower">
          <path d="M2 -52 V-66" />
          <circle cx="2" cy="-70" r="5" />
        </g>
      );
  }
}

/**
 * Built-in red fox companion (original art, SVG). It works on its den while the user works;
 * the den grows with session progress. Mirrors the beaver's states.
 */
export function FoxFallback({ state, progress, reducedMotion, mouth }: Props) {
  const pieces = Math.round((damLevel(progress, DEN_PIECES) / 100) * DEN_PIECES);
  const eyesClosed = state === "sleeping" || state === "break";
  const gazeX = state === "working" || state === "thinking" ? 3 : state === "waiting" ? -4 : 0;
  const mouthOpen = state === "talking";
  // Level-driven (or static under reduced motion) when set; otherwise the CSS talking loop.
  const openness = mouthOpenness(mouth, state, reducedMotion);
  const mouthRy = openness !== null ? 1 + openness * 4.5 : mouthOpen ? 4 : 1;
  const resting = state === "break" || state === "sleeping";
  return (
    <svg
      viewBox="0 0 360 280"
      className={`companion fox state-${state}${reducedMotion ? " reduced" : ""}`}
      aria-hidden="true"
      focusable="false"
    >
      {/* grassy ground */}
      <path className="ground" d="M0 238 Q90 226 180 234 T360 230 V280 H0 Z" />
      {/* den: grows through the session */}
      <g className="den" transform="translate(278 236)">
        {Array.from({ length: pieces }, (_, i) => (
          <DenPiece key={i} i={i} />
        ))}
      </g>

      <g className="char-body" transform={resting ? "translate(64 62) scale(1 0.9)" : "translate(64 40)"}>
        {/* bushy tail with a white tip */}
        <g className="tail">
          <path className="fox-orange" d="M80 190 C 22 200, -8 156, 14 116 C 26 94, 54 98, 52 124 C 50 152, 64 170, 92 172 Z" />
          <path className="fox-cream" d="M13 118 C 18 96, 46 92, 52 116 C 50 128, 40 132, 30 128 C 22 125, 16 124, 13 118 Z" />
        </g>
        {/* body */}
        <ellipse className="fox-orange" cx="114" cy="154" rx="52" ry="56" />
        <ellipse className="fox-cream" cx="118" cy="150" rx="26" ry="38" />
        {/* legs with dark socks */}
        <rect className="fox-sock" x="90" y="188" width="18" height="24" rx="8" />
        <rect className="fox-sock" x="124" y="188" width="18" height="24" rx="8" />
        {/* head */}
        <g className="head">
          <path className="fox-orange" d="M76 64 L68 12 L106 44 Z" />
          <path className="fox-sock" d="M80 54 L76 26 L96 44 Z" />
          <path className="fox-orange" d="M150 64 L158 12 L120 44 Z" />
          <path className="fox-sock" d="M146 54 L150 26 L130 44 Z" />
          <ellipse className="fox-orange" cx="113" cy="78" rx="46" ry="38" />
          <path className="fox-cream" d="M70 82 Q76 114 113 118 Q150 114 156 82 Q136 96 113 96 Q90 96 70 82 Z" />
          {eyesClosed ? (
            <g className="eyes-closed">
              {state === "sleeping" ? (
                <>
                  <path d="M88 76 h16" />
                  <path d="M122 76 h16" />
                </>
              ) : (
                <>
                  <path d="M88 74 q8 6 16 0" />
                  <path d="M122 74 q8 6 16 0" />
                </>
              )}
            </g>
          ) : (
            <g className="eyes" transform={`translate(${gazeX} 0)`}>
              <ellipse cx="96" cy="74" rx="5" ry="6" />
              <ellipse cx="130" cy="74" rx="5" ry="6" />
              <circle className="eye-shine" cx="98" cy="72" r="1.8" />
              <circle className="eye-shine" cx="132" cy="72" r="1.8" />
            </g>
          )}
          <ellipse className="nose" cx="113" cy="100" rx="6" ry="4.5" />
          <g className={`mouth${mouthOpen ? " talking" : ""}${openness !== null && !reducedMotion ? " level-driven" : ""}`}>
            <ellipse className="mouth-inside" cx="113" cy="109" rx="5" ry={mouthRy} />
          </g>
        </g>
        {/* front paws; holding a pebble for the den while working */}
        <g className="paws">
          {state === "working" && <circle className="pebble held-pebble" cx="150" cy="134" r="9" />}
          <ellipse className="fox-sock" cx="102" cy="134" rx="10" ry="8" />
          <ellipse className="fox-sock" cx="138" cy="136" rx="10" ry="8" />
        </g>
      </g>

      <StateOverlays state={state} />
    </svg>
  );
}
