import { useEffect } from "react";
import { useRive, useStateMachineInput } from "@rive-app/react-canvas";
import { RuntimeLoader } from "@rive-app/canvas";
import riveWasmUrl from "@rive-app/canvas/rive.wasm?url";

// Serve the Rive runtime from the app bundle; never fetch it from a CDN (no unconfigured network calls).
RuntimeLoader.setWasmUrl(riveWasmUrl);

/** Rive inputs are mutable handles by design; keep the mutation in one place. */
function setInput(input: { value: number | boolean } | null, value: number) {
  if (input) input.value = value;
}
import type { CharacterState } from "../ipc";
import { STATE_VALUES, damLevel, mouthOpenValue, type CharacterPack } from "./stateMap";

interface Props {
  pack: CharacterPack;
  baseUrl: string;
  state: CharacterState;
  progress: number;
  reducedMotion: boolean;
  mouth: number | null;
}

export function RiveCharacter({ pack, baseUrl, state, progress, reducedMotion, mouth }: Props) {
  const { rive, RiveComponent } = useRive({
    src: `${baseUrl}/${pack.rive_file}`,
    stateMachines: pack.state_machine,
    autoplay: true,
  });
  const stateInput = useStateMachineInput(rive, pack.state_machine, pack.inputs.state);
  const progressInput = useStateMachineInput(rive, pack.state_machine, pack.inputs.progress ?? "progress");
  // Optional: packs without a mouth input simply get null here.
  const mouthInput = useStateMachineInput(rive, pack.state_machine, pack.inputs.mouth ?? "mouthOpen");

  useEffect(() => {
    // Reduced motion: hold a minimal idle instead of the full animation set.
    setInput(stateInput, reducedMotion ? STATE_VALUES.idle : STATE_VALUES[state]);
  }, [stateInput, state, reducedMotion]);

  useEffect(() => {
    setInput(progressInput, damLevel(progress));
  }, [progressInput, progress]);

  useEffect(() => {
    const value = mouthOpenValue(mouth, state, reducedMotion);
    setInput(mouthInput, value ?? 0);
  }, [mouthInput, mouth, state, reducedMotion]);

  return <RiveComponent className="rive-canvas" aria-hidden="true" />;
}
