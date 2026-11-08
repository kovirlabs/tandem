# Red Fox character pack

A friendly red fox companion (original art). Requested by a young co-designer. Until
`fox.riv` exists, the UI draws a built-in SVG fox (`ui/src/character/FoxFallback.tsx`) with
the same states.

## Rive contract
Same as the beaver (see `characters/beaver/README.md`): state machine `Companion`, number
inputs `state` (0–8), `progress` (0–100), `mouthOpen` (0–100).

Fox flavor: `working` carries pebbles and leaves to a den that **grows as the session
progresses** (mound → entrance → stones → leaves → a flower on top); `celebrating` is a tail
wag; `break` lies down with eyes closed; `sleeping` curls up. Gaze stays mostly on the den.
