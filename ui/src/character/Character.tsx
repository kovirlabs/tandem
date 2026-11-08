import { lazy, Suspense, useEffect, useState } from "react";
import type { CharacterState } from "../ipc";
import { t } from "../i18n";
import { useApp } from "../store/app";
import { CharacterFallback } from "./registry";
import type { CharacterPack } from "./stateMap";
import { usePrefersReducedMotion } from "./usePrefersReducedMotion";

// The Rive runtime is only loaded when a pack actually ships a .riv file.
const RiveCharacter = lazy(() => import("./RiveCharacter").then((m) => ({ default: m.RiveCharacter })));

interface Props {
  characterId: string;
  companionName: string;
  state: CharacterState;
  progress: number;
  /** Speech loudness 0–1 for the mouth; defaults to the live speaker level from the store. */
  mouth?: number | null;
}

/** Loads the pack's Rive file if present; otherwise shows the built-in SVG beaver. */
export function Character({ characterId, companionName, state, progress, mouth }: Props) {
  const reducedMotion = usePrefersReducedMotion();
  // Subscribing here (not in App/Buddy) keeps ~30 Hz level updates from re-rendering the page.
  const storeLevel = useApp((s) => s.speakerLevel);
  const mouthLevel = mouth === undefined ? storeLevel : mouth;
  const [pack, setPack] = useState<CharacterPack | null>(null);
  const baseUrl = `/${characterId}`;

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const res = await fetch(`${baseUrl}/character.json`);
        if (!res.ok) return;
        const manifest = (await res.json()) as CharacterPack;
        const riv = await fetch(`${baseUrl}/${manifest.rive_file}`, { method: "HEAD" });
        const isRive = riv.ok && !(riv.headers.get("content-type") ?? "").includes("text/html");
        if (!cancelled && isRive) setPack(manifest);
      } catch {
        // No pack available (tests, missing file): keep the fallback.
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [baseUrl]);

  const label = t("character.label", {
    name: companionName,
    state: t(`character.state.${state}`),
  });

  return (
    <figure className="character" role="img" aria-label={label}>
      {pack ? (
        <Suspense fallback={<CharacterFallback id={characterId} state={state} progress={progress} reducedMotion={reducedMotion} mouth={mouthLevel} />}>
          <RiveCharacter pack={pack} baseUrl={baseUrl} state={state} progress={progress} reducedMotion={reducedMotion} mouth={mouthLevel} />
        </Suspense>
      ) : (
        <CharacterFallback id={characterId} state={state} progress={progress} reducedMotion={reducedMotion} mouth={mouthLevel} />
      )}
    </figure>
  );
}
