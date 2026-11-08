import type { CSSProperties } from "react";
import { commands, type VoicePhase } from "../ipc";
import { t } from "../i18n";
import { act } from "../store/actions";
import { useApp, useCompanionName } from "../store/app";
import { MicIcon } from "./MicIcon";

interface Props {
  /** Called when the mic is pressed while voice is off (e.g. open Settings → Voice). */
  onOff?: () => void;
  /** Smaller variant for the desktop buddy. */
  compact?: boolean;
  className?: string;
}

/**
 * Push-to-talk toggle: press to start listening, press again to send. Phase and mic level
 * come from the core (`voice://state`, `voice://level`); this only sends commands.
 */
export function MicButton({ onOff, compact = false, className = "" }: Props) {
  const name = useCompanionName();
  const phase: VoicePhase = useApp((s) => s.voice?.phase ?? "off");
  const micLevel = useApp((s) => s.micLevel);
  const off = phase === "off";
  const listening = phase === "listening";
  const busy = phase === "transcribing";

  const onClick = () => {
    if (off) {
      onOff?.();
      return;
    }
    if (busy) return;
    void act(listening ? commands.pttStop : commands.pttStart);
  };

  const label = off ? t("voice.mic.off") : listening ? t("voice.mic.stop") : t("voice.mic.talk", { name });
  const style = { "--mic-level": listening ? micLevel.toFixed(2) : "0" } as CSSProperties;

  return (
    <button
      type="button"
      className={`mic-button phase-${phase}${compact ? " compact" : ""} ${className}`.trim()}
      aria-label={label}
      aria-pressed={listening}
      aria-disabled={off || busy}
      aria-busy={busy || undefined}
      title={off ? t("voice.mic.off_hint") : label}
      onClick={onClick}
      style={style}
    >
      <MicIcon />
    </button>
  );
}

/** A short, polite line describing what voice is doing (or a friendly problem message). */
export function voicePhaseText(phase: VoicePhase, name: string): string {
  return t(`voice.phase.${phase}`, { name });
}
