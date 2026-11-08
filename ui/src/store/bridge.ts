// Connects core events to the store. Called once at startup.
import { commands, isTauri, subscribe } from "../ipc";
import { tDynamic } from "../i18n";
import { act } from "./actions";
import { useApp } from "./app";

export async function connectCore(): Promise<() => void> {
  if (!isTauri()) return () => {};
  const app = useApp.getState();
  const unsubscribe = await subscribe({
    "session://state": app.setSession,
    "session://tick": app.applyTick,
    "character://state": (e) => app.setCharacter(e.state),
    "chat://delta": app.applyDelta,
    "chat://message": app.applyMessage,
    "checkin://prompt": (c) => {
      app.applyCheckin(c);
      // No generated line (provider down): show friendly built-in copy instead.
      if (c.text === null && (c.action === "gentle_line" || c.action === "ask")) {
        app.applyMessage({
          id: `checkin-${Date.now()}`,
          role: "assistant",
          text: tDynamic(`checkin.fallback.${c.reason}`),
          via: "checkin",
          ts: new Date().toISOString(),
        });
      }
    },
    "core://notice": app.setNotice,
    "core://profile": app.setProfile,
    "core://settings": app.setSettings,
    "voice://state": app.setVoice,
    "voice://level": app.applyLevel,
    "voice://download": app.applyDownload,
  });
  app.hydrate(await commands.getState());
  const voice = await act(commands.voiceStatus);
  if (voice) app.setVoice(voice);
  return unsubscribe;
}
