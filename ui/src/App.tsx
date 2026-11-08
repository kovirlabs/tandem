import { useEffect, useState } from "react";
import { Character } from "./character/Character";
import { isTauri } from "./ipc";
import { t } from "./i18n";
import { ChatPanel } from "./panels/ChatPanel";
import { NoticeBar } from "./panels/NoticeBar";
import { SettingsPanel } from "./panels/SettingsPanel";
import { TimerPanel } from "./panels/TimerPanel";
import { useApp, useCompanionName } from "./store/app";
import { connectCore } from "./store/bridge";

export function App() {
  const ready = useApp((s) => s.ready);
  const character = useApp((s) => s.character);
  const progress = useApp((s) => s.session?.progress ?? 0);
  const characterId = useApp((s) => s.profile?.companion.character_id ?? "beaver");
  const name = useCompanionName();
  // null = closed; "voice" opens Settings scrolled to the Voice section.
  const [settingsOpen, setSettingsOpen] = useState<null | "top" | "voice">(null);

  useEffect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    connectCore().then((un) => {
      if (cancelled) un();
      else cleanup = un;
    });
    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, []);

  if (!isTauri()) {
    return <main className="app not-tauri">{t("app.not_in_tauri")}</main>;
  }

  return (
    <div className="app">
      <header className="topbar">
        <h1>{name || t("app.title")}</h1>
        <button onClick={() => setSettingsOpen("top")} disabled={!ready}>
          {t("settings.open")}
        </button>
      </header>
      <NoticeBar />
      <main className="layout">
        <div className="stage">
          <Character key={characterId} characterId={characterId} companionName={name} state={character} progress={progress} />
        </div>
        <aside className="side">
          <TimerPanel />
        </aside>
        <div className="bottom">
          <ChatPanel onOpenVoiceSettings={() => ready && setSettingsOpen("voice")} />
        </div>
      </main>
      {settingsOpen && <SettingsPanel focusSection={settingsOpen === "voice" ? "voice" : undefined} onClose={() => setSettingsOpen(null)} />}
    </div>
  );
}
