// Typed subscriptions to core events. Channel names match core/src/events.rs.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AudioLevel,
  CharacterStateEvent,
  ChatDelta,
  ChatMessageDto,
  CheckinEvent,
  CoreNotice,
  DownloadProgress,
  Profile,
  SessionSnapshot,
  SessionTick,
  Settings,
  VoiceStatus,
} from "./generated";

export interface CoreEvents {
  "session://state": SessionSnapshot;
  "session://tick": SessionTick;
  "checkin://prompt": CheckinEvent;
  "chat://delta": ChatDelta;
  "chat://message": ChatMessageDto;
  "character://state": CharacterStateEvent;
  "core://notice": CoreNotice;
  "core://profile": Profile;
  "core://settings": Settings;
  "voice://state": VoiceStatus;
  "voice://level": AudioLevel;
  "voice://download": DownloadProgress;
}

export type CoreHandlers = { [K in keyof CoreEvents]: (payload: CoreEvents[K]) => void };

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Subscribe to every core event; returns a single unsubscribe function. */
export async function subscribe(handlers: CoreHandlers): Promise<() => void> {
  const channels = Object.keys(handlers) as (keyof CoreEvents)[];
  const unlisteners: UnlistenFn[] = await Promise.all(
    channels.map((channel) =>
      listen<CoreEvents[typeof channel]>(channel, (e) => {
        (handlers[channel] as (p: CoreEvents[typeof channel]) => void)(e.payload);
      }),
    ),
  );
  return () => unlisteners.forEach((u) => u());
}
