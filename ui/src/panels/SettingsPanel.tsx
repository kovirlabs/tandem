import { useEffect, useRef, useState, type FormEvent } from "react";
import { commands, type CheckinMode, type Profile, type ProviderStatus, type Settings, type Tone } from "../ipc";
import { t, tDynamic } from "../i18n";
import { CHARACTER_IDS } from "../character/registry";
import { act } from "../store/actions";
import { useApp } from "../store/app";
import { VoiceSettings } from "../voice/VoiceSettings";

const TONES: Tone[] = ["calm", "cheerful", "dry_humor", "minimal"];
const MODES: CheckinMode[] = ["silent", "gentle", "conversational"];
const EFFORTS = ["low", "medium", "high"] as const;

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^\[|\]$/g, "").toLowerCase();
  } catch {
    return "";
  }
}

/** Mirrors core/src/router/locality.rs closely enough for a UI hint; the core decides. */
export function looksLikeLan(host: string): boolean {
  return (
    /^(10\.|192\.168\.|169\.254\.)/.test(host) ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(host) ||
    /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(host) ||
    /\.(local|lan|home\.arpa)$/.test(host) ||
    /^f[cd][0-9a-f]{2}:/.test(host) ||
    host.startsWith("fe80:")
  );
}

export function localityLabel(status: ProviderStatus | null): string {
  if (!status) return "";
  const l = status.locality;
  if (l.kind === "lan") return t(l.confirmed ? "settings.locality.lan_confirmed" : "settings.locality.lan_unconfirmed");
  return tDynamic(`settings.locality.${l.kind}`);
}

function StatusLine({ status }: { status: ProviderStatus | null }) {
  if (!status) return null;
  return (
    <p className={`status ${status.ready ? "ok" : "warn"}`} role="status">
      {status.ready ? t("settings.status.ready", { model: status.model }) : t("settings.status.error", { error: status.error ?? "" })}
      {" · "}
      {localityLabel(status)}
      {status.ready && !status.tools && <span className="detail">{t("settings.status.no_tools")}</span>}
    </p>
  );
}

export function SettingsPanel({ onClose, focusSection }: { onClose: () => void; focusSection?: "voice" }) {
  const app = useApp();
  const [settings, setSettings] = useState<Settings | null>(app.settings);
  const [profile, setProfile] = useState<Profile | null>(app.profile);
  const [models, setModels] = useState<string[]>([]);
  const [apiKey, setApiKey] = useState("");
  const [saved, setSaved] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);

  // Keep the latest onClose without re-running the focus effect: the parent re-renders on
  // every character/voice change, and re-focusing would pull focus out of the form.
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    dialogRef.current?.focus();
    if (focusSection) document.getElementById(`settings-${focusSection}`)?.scrollIntoView?.({ block: "start" });
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onCloseRef.current();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [focusSection]);

  if (!settings || !profile) return null;
  const host = hostOf(settings.ollama.url);
  const hostConfirmed = settings.ollama.confirmed_hosts.includes(host);
  const set = (patch: Partial<Settings>) => {
    setSaved(false);
    setSettings({ ...settings, ...patch });
  };
  const setOllama = (patch: Partial<Settings["ollama"]>) => set({ ollama: { ...settings.ollama, ...patch } });
  const setVoice = (patch: Partial<Settings["voice"]>) => set({ voice: { ...settings.voice, ...patch } });
  const setAnthropic = (patch: Partial<Settings["anthropic"]>) => set({ anthropic: { ...settings.anthropic, ...patch } });
  const setCompanionName = (name: string) => {
    setSaved(false);
    setProfile({ ...profile, companion: { ...profile.companion, name } });
  };

  const findModels = async () => {
    const list = await act(() => commands.listOllamaModels(settings.ollama.url));
    if (list) setModels(list);
  };

  const toggleHost = (checked: boolean) => {
    const others = settings.ollama.confirmed_hosts.filter((h) => h !== host);
    setOllama({ confirmed_hosts: checked ? [...others, host] : others });
  };

  const save = async (e: FormEvent) => {
    e.preventDefault();
    const p = await act(() => commands.updateProfile(profile));
    if (!p) return;
    app.setProfile(p);
    if (apiKey.trim()) {
      const st = await act(() => commands.setApiKey("anthropic", apiKey.trim()));
      if (st) app.setProvider(st, true);
      setApiKey("");
    }
    const status = await act(() => commands.updateSettings(settings));
    if (!status) return;
    app.setSettings(settings);
    app.setProvider(status);
    setSaved(true);
  };

  const clearKey = async () => {
    const st = await act(() => commands.clearApiKey("anthropic"));
    if (st) app.setProvider(st, false);
  };

  return (
    <div className="dialog-backdrop">
      <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title" tabIndex={-1} ref={dialogRef}>
        <header className="dialog-header">
          <h2 id="settings-title">{t("settings.title")}</h2>
          <button className="link" onClick={onClose} aria-label={t("settings.close")}>
            ✕
          </button>
        </header>
        <form onSubmit={save} className="settings-form">
          <fieldset>
            <legend>{t("settings.section.companion")}</legend>
            <label htmlFor="companion-name">{t("settings.name")}</label>
            <input id="companion-name" value={profile.companion.name} maxLength={20} onChange={(e) => setCompanionName(e.target.value)} aria-describedby="name-help" />
            <small id="name-help">{t("settings.name_help")}</small>
            <label htmlFor="character">{t("settings.character")}</label>
            <select
              id="character"
              value={profile.companion.character_id}
              onChange={(e) => {
                setSaved(false);
                setProfile({ ...profile, companion: { ...profile.companion, character_id: e.target.value } });
              }}
              aria-describedby="character-help"
            >
              {CHARACTER_IDS.map((id) => (
                <option key={id} value={id}>
                  {tDynamic(`character.pack.${id}`)}
                </option>
              ))}
            </select>
            <small id="character-help">{t("settings.character_help")}</small>
            <label htmlFor="tone">{t("settings.tone")}</label>
            <select id="tone" value={profile.tone} onChange={(e) => setProfile({ ...profile, tone: e.target.value as Tone })}>
              {TONES.map((tone) => (
                <option key={tone} value={tone}>
                  {t(`settings.tone.${tone}`)}
                </option>
              ))}
            </select>
            <label htmlFor="about-me">{t("settings.about_me")}</label>
            <textarea id="about-me" rows={3} value={profile.about_me} onChange={(e) => setProfile({ ...profile, about_me: e.target.value })} aria-describedby="about-help" />
            <small id="about-help">{t("settings.about_me_help")}</small>
          </fieldset>

          <fieldset>
            <legend>{t("settings.section.ai")}</legend>
            <StatusLine status={app.provider} />
            <div role="radiogroup" aria-label={t("settings.provider")}>
              {(["ollama", "anthropic"] as const).map((kind) => (
                <label key={kind} className="radio">
                  <input type="radio" name="provider" value={kind} checked={settings.chat_provider === kind} onChange={() => set({ chat_provider: kind })} />
                  {t(`settings.provider.${kind}`)}
                </label>
              ))}
            </div>
            {settings.chat_provider === "ollama" ? (
              <>
                <label htmlFor="ollama-url">{t("settings.ollama_url")}</label>
                <div className="row">
                  <input id="ollama-url" value={settings.ollama.url} onChange={(e) => setOllama({ url: e.target.value })} />
                  <button type="button" onClick={findModels}>
                    {t("settings.ollama_refresh")}
                  </button>
                </div>
                {looksLikeLan(host) && (
                  <>
                    <label className="checkbox">
                      <input type="checkbox" checked={hostConfirmed} onChange={(e) => toggleHost(e.target.checked)} />
                      {t("settings.lan_confirm", { host })}
                    </label>
                    <small>{t("settings.lan_help")}</small>
                  </>
                )}
                <label htmlFor="ollama-model">{t("settings.ollama_model")}</label>
                <select id="ollama-model" value={settings.ollama.chat_model} onChange={(e) => setOllama({ chat_model: e.target.value })}>
                  <option value="">{t("settings.ollama_model_auto")}</option>
                  {[...new Set([settings.ollama.chat_model, ...models].filter(Boolean))].map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
                <label htmlFor="ollama-vision">{t("settings.ollama_vision_model")}</label>
                <select id="ollama-vision" value={settings.ollama.vision_model} onChange={(e) => setOllama({ vision_model: e.target.value })}>
                  <option value="">{t("settings.ollama_vision_none")}</option>
                  {[...new Set([settings.ollama.vision_model, ...models].filter(Boolean))].map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              </>
            ) : (
              <>
                <label htmlFor="api-key">{t("settings.anthropic_key")}</label>
                {app.hasAnthropicKey && (
                  <p className="row">
                    <small>{t("settings.anthropic_key_saved")}</small>
                    <button type="button" className="link" onClick={clearKey}>
                      {t("settings.anthropic_key_clear")}
                    </button>
                  </p>
                )}
                <input id="api-key" type="password" autoComplete="off" value={apiKey} placeholder={t("settings.anthropic_key_placeholder")} onChange={(e) => setApiKey(e.target.value)} />
                <label htmlFor="anthropic-model">{t("settings.anthropic_model")}</label>
                <input id="anthropic-model" value={settings.anthropic.model} onChange={(e) => setAnthropic({ model: e.target.value })} />
                <label htmlFor="anthropic-effort">{t("settings.anthropic_effort")}</label>
                <select id="anthropic-effort" value={settings.anthropic.effort} onChange={(e) => setAnthropic({ effort: e.target.value })}>
                  {EFFORTS.map((x) => (
                    <option key={x} value={x}>
                      {t(`settings.effort.${x}`)}
                    </option>
                  ))}
                </select>
                <small>{t("settings.cloud_note")}</small>
              </>
            )}
          </fieldset>

          <fieldset>
            <legend>{t("settings.section.sessions")}</legend>
            <label htmlFor="default-minutes">{t("settings.default_minutes")}</label>
            <input id="default-minutes" type="number" min={5} max={120} value={settings.default_minutes} onChange={(e) => set({ default_minutes: Number(e.target.value) })} />
            <label htmlFor="checkin-mode">{t("settings.checkin_mode")}</label>
            <select id="checkin-mode" value={settings.checkin_mode} onChange={(e) => set({ checkin_mode: e.target.value as CheckinMode })}>
              {MODES.map((m) => (
                <option key={m} value={m}>
                  {t(`settings.checkin.${m}`)}
                </option>
              ))}
            </select>
            <label htmlFor="checkin-interval">{t("settings.checkin_interval")}</label>
            <input id="checkin-interval" type="number" min={1} max={60} value={settings.checkin_interval_min} onChange={(e) => set({ checkin_interval_min: Number(e.target.value) })} />
            <label className="checkbox">
              <input type="checkbox" checked={settings.time_warnings} onChange={(e) => set({ time_warnings: e.target.checked })} />
              {t("settings.time_warnings")}
            </label>
            <label className="checkbox">
              <input type="checkbox" checked={settings.show_timer} onChange={(e) => set({ show_timer: e.target.checked })} />
              {t("settings.show_timer")}
            </label>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={settings.buddy.enabled}
                onChange={(e) => set({ buddy: { ...settings.buddy, enabled: e.target.checked } })}
                aria-describedby="buddy-help"
              />
              {t("settings.buddy", { name: profile.companion.name })}
            </label>
            <small id="buddy-help">{t("settings.buddy_help")}</small>
          </fieldset>

          <VoiceSettings
            voice={settings.voice}
            profile={profile}
            onVoice={setVoice}
            onProfile={(p) => {
              setSaved(false);
              setProfile(p);
            }}
          />

          <fieldset>
            <legend>{t("settings.section.privacy")}</legend>
            <label htmlFor="retention">{t("settings.retention")}</label>
            <input id="retention" type="number" min={1} max={365} value={settings.message_retention_days} onChange={(e) => set({ message_retention_days: Number(e.target.value) })} />
          </fieldset>

          <div className="dialog-actions">
            {saved && (
              <span role="status" className="muted">
                {t("settings.saved")}
              </span>
            )}
            <button type="submit" className="primary">
              {t("settings.save")}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
