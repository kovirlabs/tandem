# Busy Beaver character pack

Default companion (SPEC §3.4). The Rive file `beaver.riv` has not been made yet; until it
exists, the UI draws a built-in SVG beaver (`ui/src/character/BeaverFallback.tsx`) with the
same states.

## Rive contract
- State machine: `Companion`
- Number input `state`: 0 idle, 1 working, 2 listening, 3 thinking, 4 talking, 5 waiting,
  6 celebrating, 7 break, 8 sleeping (see `ui/src/character/stateMap.ts`)
- Number input `progress`: 0–100, dam growth through the session (grows stick by stick)
- Number input `mouthOpen`: 0–100, driven by TTS amplitude (M2)

Beaver flavor: `working` gnaws or stacks sticks on a small dam that grows as the session
progresses; `celebrating` is a tail slap; `break` floats on its back; `sleeping` curls up in
the lodge. Gaze stays mostly on its own work.
