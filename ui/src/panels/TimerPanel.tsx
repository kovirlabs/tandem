import { useState, type FormEvent } from "react";
import { commands } from "../ipc";
import { t } from "../i18n";
import { act, formatClock } from "../store/actions";
import { useApp, useCompanionName } from "../store/app";
import { Subtasks } from "./Subtasks";

const LENGTHS = [15, 25, 45, 60];

function StartForm() {
  const name = useCompanionName();
  const defaultMinutes = useApp((s) => s.settings?.default_minutes ?? 25);
  const [task, setTask] = useState("");
  const [minutes, setMinutes] = useState<number>(defaultMinutes);
  const lengths = LENGTHS.includes(defaultMinutes) ? LENGTHS : [...LENGTHS, defaultMinutes].sort((a, b) => a - b);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!task.trim()) return;
    const ok = await act(() => commands.startSession(task.trim(), minutes));
    if (ok) setTask("");
  };

  return (
    <form className="start-form" onSubmit={submit}>
      <label htmlFor="task-input" className="start-prompt">
        {t("start.prompt")}
      </label>
      <input
        id="task-input"
        value={task}
        onChange={(e) => setTask(e.target.value)}
        placeholder={t("start.placeholder")}
        autoComplete="off"
        autoFocus
      />
      <div className="row">
        <label htmlFor="minutes-select">{t("start.minutes_label")}</label>
        <select id="minutes-select" value={minutes} onChange={(e) => setMinutes(Number(e.target.value))}>
          {lengths.map((n) => (
            <option key={n} value={n}>
              {t("start.minutes_option", { n })}
            </option>
          ))}
        </select>
        <button type="submit" className="primary" disabled={!task.trim()}>
          {t("start.button")}
        </button>
      </div>
      <p className="hint">{t("start.hint", { name })}</p>
    </form>
  );
}

function ProgressRing({ progress, label }: { progress: number; label: string }) {
  const r = 52;
  const c = 2 * Math.PI * r;
  const p = Math.min(1, Math.max(0, progress));
  return (
    <svg className="ring" viewBox="0 0 120 120" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(p * 100)}>
      <circle className="ring-track" cx="60" cy="60" r={r} />
      <circle className="ring-fill" cx="60" cy="60" r={r} strokeDasharray={c} strokeDashoffset={c * (1 - p)} transform="rotate(-90 60 60)" />
    </svg>
  );
}

function Focus() {
  const session = useApp((s) => s.session);
  const showTimer = useApp((s) => s.settings?.show_timer ?? true);
  if (!session) return null;
  const paused = session.phase === "paused_manual" || session.phase === "paused_away";
  return (
    <div className="focus">
      <h2 className="task-title">{session.task}</h2>
      <div className="clock-wrap">
        <ProgressRing progress={session.progress} label={t("timer.progress_label")} />
        <div className="clock" aria-live="off">
          {showTimer ? (
            <span aria-label={t("timer.label")}>{formatClock(session.remaining_secs)}</span>
          ) : (
            <span className="muted">{t("timer.hidden")}</span>
          )}
          {session.phase === "paused_manual" && <span className="badge">{t("timer.paused")}</span>}
          {session.phase === "paused_away" && <span className="badge">{t("timer.away")}</span>}
        </div>
      </div>
      {session.quiet_remaining_secs > 0 && (
        <p className="badge soft">{t("timer.quiet", { n: Math.ceil(session.quiet_remaining_secs / 60) })}</p>
      )}
      <div className="controls">
        {paused ? (
          <button className="primary" onClick={() => act(commands.resumeSession)}>
            {t("timer.resume")}
          </button>
        ) : (
          <button onClick={() => act(commands.pauseSession)}>{t("timer.pause")}</button>
        )}
        <button onClick={() => act(() => commands.extendSession(5))}>{t("timer.extend", { n: 5 })}</button>
        <button onClick={() => act(() => commands.extendSession(10))}>{t("timer.extend", { n: 10 })}</button>
        <button onClick={() => act(() => commands.endSession())}>{t("timer.end")}</button>
      </div>
      <Subtasks />
    </div>
  );
}

function WrapUp() {
  const session = useApp((s) => s.session);
  const breakMin = useApp((s) => s.settings?.short_break_min ?? 5);
  const [note, setNote] = useState("");
  const [saved, setSaved] = useState(false);
  if (!session) return null;
  const minutes = Math.max(1, Math.round(session.elapsed_secs / 60));
  const save = async (e: FormEvent) => {
    e.preventDefault();
    if (!note.trim()) return;
    if (await act(() => commands.endSession(note.trim()))) setSaved(true);
  };
  return (
    <div className="wrap">
      <h2>{t("wrap.title", { n: minutes })}</h2>
      <form onSubmit={save} className="row">
        <label htmlFor="reflection" className="sr-only">
          {t("wrap.prompt")}
        </label>
        <input id="reflection" value={note} onChange={(e) => setNote(e.target.value)} placeholder={t("wrap.prompt")} />
        <button type="submit" disabled={!note.trim() || saved}>
          {saved ? t("wrap.saved") : t("wrap.save")}
        </button>
      </form>
      <Subtasks />
      <div className="controls">
        <button className="primary" onClick={() => act(() => commands.startBreak(breakMin))}>
          {t("wrap.break", { n: breakMin })}
        </button>
        <button onClick={() => session.task && act(() => commands.startSession(session.task ?? ""))}>{t("wrap.next")}</button>
        <button onClick={() => act(commands.closeSession)}>{t("wrap.done")}</button>
      </div>
    </div>
  );
}

function Break() {
  const session = useApp((s) => s.session);
  if (!session) return null;
  return (
    <div className="break">
      <h2>{t("break.title")}</h2>
      <p>{t("break.left", { t: formatClock(session.break_remaining_secs) })}</p>
      <button onClick={() => act(commands.closeSession)}>{t("break.end")}</button>
    </div>
  );
}

export function TimerPanel() {
  const phase = useApp((s) => s.session?.phase ?? "idle");
  return (
    <section className="panel timer-panel" aria-label={t("timer.label")}>
      {phase === "idle" && <StartForm />}
      {(phase === "focusing" || phase === "paused_manual" || phase === "paused_away") && <Focus />}
      {phase === "wrapping_up" && <WrapUp />}
      {phase === "break" && <Break />}
    </section>
  );
}
