import { useEffect, useRef, useState, type FormEvent } from "react";
import { commands } from "../ipc";
import { t } from "../i18n";
import { act } from "../store/actions";
import { useApp, useCompanionName } from "../store/app";
import { MicButton, voicePhaseText } from "../voice/MicButton";
import { MicIcon } from "../voice/MicIcon";
import { voiceProblemText } from "../voice/problem";

export function ChatPanel({ onOpenVoiceSettings }: { onOpenVoiceSettings?: () => void }) {
  const name = useCompanionName();
  const messages = useApp((s) => s.messages);
  const pending = useApp((s) => s.pending);
  const setPending = useApp((s) => s.setPending);
  const voice = useApp((s) => s.voice);
  const phase = voice?.phase ?? "off";
  const [open, setOpen] = useState(true);
  const [text, setText] = useState("");
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: "end" });
  }, [messages]);

  const send = async (e: FormEvent) => {
    e.preventDefault();
    const msg = text.trim();
    if (!msg || pending) return;
    setText("");
    setPending(true);
    await act(() => commands.sendMessage(msg));
    setPending(false);
  };

  return (
    <section className={`panel chat-panel${open ? "" : " collapsed"}`} aria-label={t("chat.title")}>
      <header className="chat-header">
        <h2>{t("chat.title")}</h2>
        <button className="link" aria-expanded={open} aria-controls="chat-body" onClick={() => setOpen(!open)}>
          {open ? t("chat.hide") : t("chat.show")}
        </button>
      </header>
      <div id="chat-body" hidden={!open}>
        <div className="messages" role="log" aria-live="polite" aria-label={t("chat.log_label")}>
          {messages.length === 0 && <p className="muted">{t("chat.empty", { name })}</p>}
          {messages.map((m) => (
            <p key={m.id} className={`msg ${m.role}${m.draft ? " draft" : ""}${m.via === "checkin" ? " checkin" : ""}`}>
              <span className="who">
                {m.role === "user" ? t("chat.you") : name}
                {m.via === "voice" && (
                  <span className="via-voice" role="img" aria-label={t("chat.via_voice")} title={t("chat.via_voice")}>
                    <MicIcon />
                  </span>
                )}
              </span>
              <span className="text">{m.text}</span>
            </p>
          ))}
          <div ref={endRef} />
        </div>
        {(voice?.problem || (phase !== "off" && phase !== "ready")) && (
          <p className={`voice-status${voice?.problem ? " problem" : ""}`} role="status">
            {voice?.problem ? voiceProblemText(voice.problem, name) : voicePhaseText(phase, name)}
            {phase === "speaking" && (
              <button type="button" className="link" onClick={() => void act(commands.stopSpeaking)} aria-label={t("voice.stop_speaking_label", { name })}>
                {t("voice.stop_speaking")}
              </button>
            )}
          </p>
        )}
        <form className="row chat-input" onSubmit={send}>
          <MicButton onOff={onOpenVoiceSettings} />
          <label htmlFor="chat-text" className="sr-only">
            {t("chat.placeholder", { name })}
          </label>
          <input
            id="chat-text"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder={t("chat.placeholder", { name })}
            autoComplete="off"
          />
          <button type="submit" className="primary" disabled={!text.trim() || pending}>
            {t("chat.send")}
          </button>
        </form>
      </div>
    </section>
  );
}
