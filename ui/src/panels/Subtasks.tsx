import { useState, type FormEvent } from "react";
import { commands } from "../ipc";
import { t } from "../i18n";
import { act } from "../store/actions";
import { useApp } from "../store/app";

export function Subtasks() {
  const subtasks = useApp((s) => s.session?.subtasks ?? []);
  const [draft, setDraft] = useState("");
  const add = async (e: FormEvent) => {
    e.preventDefault();
    if (!draft.trim()) return;
    if (await act(() => commands.addSubtasks([draft.trim()]))) setDraft("");
  };
  return (
    <div className="subtasks">
      <h3>{t("subtasks.title")}</h3>
      {subtasks.length > 0 && (
        <ul>
          {subtasks.map((s) => (
            <li key={s.id} className={s.done ? "done" : ""}>
              <label>
                <input
                  type="checkbox"
                  checked={s.done}
                  disabled={s.done}
                  aria-label={t("subtasks.mark_done", { title: s.title })}
                  onChange={() => act(() => commands.completeSubtask(s.id))}
                />
                <span>{s.title}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
      <form className="row" onSubmit={add}>
        <label htmlFor="subtask-input" className="sr-only">
          {t("subtasks.add_placeholder")}
        </label>
        <input id="subtask-input" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder={t("subtasks.add_placeholder")} />
        <button type="submit" disabled={!draft.trim()}>
          {t("subtasks.add")}
        </button>
      </form>
    </div>
  );
}
