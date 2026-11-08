import type { CharacterState } from "../ipc";
import { damLevel, mouthOpenness } from "./stateMap";
import { StateOverlays } from "./StateOverlays";

interface Props {
  state: CharacterState;
  progress: number;
  reducedMotion: boolean;
  mouth: number | null;
}

const STICKS = 8;

/**
 * Placeholder Busy Beaver drawn in SVG, used until the Rive pack (`beaver.riv`) exists.
 * Original art; states mirror the Rive state machine. Gaze stays on its own work except
 * when talking or greeting.
 */
export function BeaverFallback({ state, progress, reducedMotion, mouth }: Props) {
  const sticks = Math.round((damLevel(progress, STICKS) / 100) * STICKS);
  const eyesClosed = state === "sleeping";
  // Look at the dam (own work) while working; at the user when talking/listening/celebrating.
  const gazeX = state === "working" || state === "thinking" ? 3 : state === "waiting" ? -4 : 0;
  const mouthOpen = state === "talking";
  // Level-driven (or static under reduced motion) when set; otherwise the CSS talking loop.
  const openness = mouthOpenness(mouth, state, reducedMotion);
  const mouthRy = openness !== null ? 1.5 + openness * 5.5 : mouthOpen ? 5 : 1.5;
  return (
    <svg
      viewBox="0 0 360 280"
      className={`companion beaver state-${state}${reducedMotion ? " reduced" : ""}`}
      aria-hidden="true"
      focusable="false"
    >
      {/* water */}
      <path className="water" d="M0 236 Q45 228 90 236 T180 236 T270 236 T360 236 V280 H0 Z" />
      {/* dam: grows one stick at a time through the session */}
      <g className="dam" transform="translate(236 236)">
        {Array.from({ length: sticks }, (_, i) => {
          const row = Math.floor(i / 3);
          const col = i % 3;
          return (
            <rect
              key={i}
              className={`stick stick-${i % 3}`}
              x={-40 + col * 28 + (row % 2) * 12}
              y={-14 - row * 13}
              width={46}
              height={11}
              rx={5}
              transform={`rotate(${(i % 2 ? -1 : 1) * (4 + (i % 3) * 3)} ${-17 + col * 28} ${-8 - row * 13})`}
            />
          );
        })}
      </g>

      <g className="char-body" transform={state === "break" ? "translate(150 214) rotate(-80)" : "translate(64 40)"}>
        {/* tail */}
        <g className="tail">
          <ellipse cx="40" cy="168" rx="44" ry="20" transform="rotate(-18 40 168)" />
          <path className="tail-hatch" d="M10 168 L66 154 M14 178 L72 164 M24 186 L74 174 M26 158 L44 192 M44 152 L60 186" />
        </g>
        {/* body */}
        <ellipse className="fur" cx="112" cy="150" rx="62" ry="58" />
        <ellipse className="belly" cx="114" cy="160" rx="38" ry="40" />
        {/* feet */}
        <ellipse className="fur-dark" cx="84" cy="204" rx="18" ry="8" />
        <ellipse className="fur-dark" cx="142" cy="204" rx="18" ry="8" />
        {/* head */}
        <g className="head">
          <circle className="fur-dark" cx="78" cy="48" r="12" />
          <circle className="fur-dark" cx="146" cy="48" r="12" />
          <circle className="fur" cx="112" cy="84" r="48" />
          <ellipse className="muzzle" cx="112" cy="104" rx="26" ry="18" />
          {eyesClosed ? (
            <g className="eyes-closed">
              <path d="M88 80 q8 6 16 0" />
              <path d="M120 80 q8 6 16 0" />
            </g>
          ) : (
            <g className="eyes" transform={`translate(${gazeX} 0)`}>
              <circle cx="96" cy="78" r="6" />
              <circle cx="128" cy="78" r="6" />
              <circle className="eye-shine" cx="98" cy="76" r="2" />
              <circle className="eye-shine" cx="130" cy="76" r="2" />
            </g>
          )}
          <circle className="cheek" cx="84" cy="96" r="6" />
          <circle className="cheek" cx="140" cy="96" r="6" />
          <ellipse className="nose" cx="112" cy="96" rx="8" ry="6" />
          <g className={`mouth${mouthOpen ? " talking" : ""}${openness !== null && !reducedMotion ? " level-driven" : ""}`}>
            <ellipse className="mouth-inside" cx="112" cy="110" rx="8" ry={mouthRy} />
            <rect className="tooth" x="104.5" y="104" width="7" height="11" rx="1.5" />
            <rect className="tooth" x="112.5" y="104" width="7" height="11" rx="1.5" />
          </g>
        </g>
        {/* paws, holding a stick while working */}
        <g className="paws">
          {state === "working" && <rect className="stick stick-1 held-stick" x="118" y="118" width="58" height="10" rx="5" transform="rotate(-24 147 123)" />}
          <ellipse className="fur-dark" cx="94" cy="130" rx="11" ry="9" />
          <ellipse className="fur-dark" cx="132" cy="130" rx="11" ry="9" />
        </g>
      </g>

      <StateOverlays state={state} />
    </svg>
  );
}
