import { useEffect, useState } from "react";
import { commands, type DownloadProgress, type Profile, type SpeakReplies, type SttModelInfo, type VoiceInfo, type VoiceSettings as VoiceSettingsT } from "../ipc";
import { t } from "../i18n";
import { act } from "../store/actions";
import { useApp } from "../store/app";
import { formatBytes } from "./format";
import { voiceProblemText } from "./problem";
import { wakeNameWarning } from "./wake";

const SPEAK_REPLIES: SpeakReplies[] = ["voice_turns", "always", "never"];

interface Lists {
  sttModels?: SttModelInfo[];
  voices?: VoiceInfo[];
  devices?: string[];
}

/** Local queries only: what's installed and which mics exist. Never touches the network. */
async function loadLists(): Promise<Lists> {
  const [sttModels, voices, devices] = await Promise.all([act(commands.listSttModels), act(commands.listVoices), act(commands.listInputDevices)]);
  return { sttModels, voices, devices };
}

interface Props {
  voice: VoiceSettingsT;
  profile: Profile;
  onVoice: (patch: Partial<VoiceSettingsT>) => void;
  onProfile: (profile: Profile) => void;
}

function DownloadStatus({ progress, model }: { progress: DownloadProgress | undefined; model: string }) {
  if (!progress) return null;
  if (progress.error) {
    return (
      <small className="warn" role="status">
        {t("settings.voice.download_error", { error: progress.error })}
      </small>
    );
  }
  if (progress.finished) {
    return (
      <small role="status" className="muted">
        {t("settings.voice.download_done")}
      </small>
    );
  }
  const label = t("settings.voice.downloading", { model });
  return progress.total_bytes ? (
    <progress aria-label={label} max={progress.total_bytes} value={progress.done_bytes} />
  ) : (
    <progress aria-label={label} />
  );
}

/**
 * Settings → Voice. Lists come from the core (local only); downloads start only on a click,
 * and the button names huggingface.co first (rule 10). The voice is saved to
 * `profile.companion.voice_id` with the rest of the form and is never tied to appearance.
 */
export function VoiceSettings({ voice, profile, onVoice, onProfile }: Props) {
  const name = profile.companion.name;
  const status = useApp((s) => s.voice);
  const micLevel = useApp((s) => s.micLevel);
  const downloads = useApp((s) => s.downloads);
  const [sttModels, setSttModels] = useState<SttModelInfo[]>([]);
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const [devices, setDevices] = useState<string[]>([]);
  const [busy, setBusy] = useState<Record<string, boolean>>({});

  const applyLists = (l: Lists) => {
    if (l.sttModels) setSttModels(l.sttModels);
    if (l.voices) setVoices(l.voices);
    if (l.devices) setDevices(l.devices);
  };

  useEffect(() => {
    let cancelled = false;
    void loadLists().then((l) => {
      if (cancelled) return;
      if (l.sttModels) setSttModels(l.sttModels);
      if (l.voices) setVoices(l.voices);
      if (l.devices) setDevices(l.devices);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const download = async (key: string, fn: () => Promise<null>) => {
    setBusy((b) => ({ ...b, [key]: true }));
    await act(fn);
    setBusy((b) => ({ ...b, [key]: false }));
    applyLists(await loadLists());
  };

  const stt = sttModels.find((m) => m.id === voice.stt_model);
  const sttKey = `whisper:${voice.stt_model}`;
  const selectedVoiceId = profile.companion.voice_id ?? "";
  const activeVoice = voices.find((v) => v.id === selectedVoiceId) ?? voices.find((v) => v.is_default);
  const voiceKey = activeVoice ? `piper:${activeVoice.id}` : "";
  const setVoiceId = (id: string) => onProfile({ ...profile, companion: { ...profile.companion, voice_id: id || null } });
  const warnWake = wakeNameWarning(name);

  return (
    <fieldset id="settings-voice">
      <legend>{t("settings.section.voice")}</legend>
      <label className="checkbox">
        <input type="checkbox" checked={voice.enabled} onChange={(e) => onVoice({ enabled: e.target.checked })} aria-describedby="voice-enabled-help" />
        {t("settings.voice.enabled", { name })}
      </label>
      <small id="voice-enabled-help">{t("settings.voice.enabled_help")}</small>
      {status?.problem && (
        <p className="status warn" role="status">
          {voiceProblemText(status.problem, name)}
        </p>
      )}

      <p className="muted" id="download-note">
        <small>{t("settings.voice.download_note")}</small>
      </p>

      <label htmlFor="stt-model">{t("settings.voice.stt_model")}</label>
      <select id="stt-model" value={voice.stt_model} onChange={(e) => onVoice({ stt_model: e.target.value })}>
        {[...new Set([voice.stt_model, ...sttModels.map((m) => m.id)])].map((id) => {
          const m = sttModels.find((x) => x.id === id);
          const size = m ? formatBytes(m.size_bytes) : "?";
          return (
            <option key={id} value={id}>
              {t("settings.voice.stt_option", { id, size })}
              {m?.downloaded ? ` · ${t("settings.voice.downloaded")}` : ""}
            </option>
          );
        })}
      </select>
      {stt && !stt.downloaded && (
        <button type="button" onClick={() => void download(sttKey, () => commands.downloadSttModel(stt.id))} disabled={busy[sttKey]} aria-describedby="download-note">
          {t("settings.voice.download_stt", { size: formatBytes(stt.size_bytes) })}
        </button>
      )}
      <DownloadStatus progress={downloads[sttKey]} model={voice.stt_model} />

      <label htmlFor="voice-id">{t("settings.voice.voice", { name })}</label>
      <select id="voice-id" value={selectedVoiceId} onChange={(e) => setVoiceId(e.target.value)} aria-describedby="voice-id-help">
        <option value="">{t("settings.voice.voice_default")}</option>
        {voices.map((v) => (
          <option key={v.id} value={v.id}>
            {t("settings.voice.voice_option", { label: v.label, license: v.license, size: formatBytes(v.size_bytes) })}
            {v.downloaded ? ` · ${t("settings.voice.downloaded")}` : ""}
          </option>
        ))}
      </select>
      <small id="voice-id-help">{t("settings.voice.voice_help")}</small>
      {activeVoice && (
        <div className="row">
          {!activeVoice.downloaded && (
            <button type="button" onClick={() => void download(voiceKey, () => commands.downloadVoice(activeVoice.id))} disabled={busy[voiceKey]} aria-describedby="download-note">
              {t("settings.voice.download_voice", { size: formatBytes(activeVoice.size_bytes) })}
            </button>
          )}
          <button
            type="button"
            disabled={!activeVoice.downloaded}
            onClick={() => void act(() => commands.previewVoice(activeVoice.id, t("settings.voice.preview_text", { name })))}
          >
            {t("settings.voice.preview")}
          </button>
        </div>
      )}
      {activeVoice && <DownloadStatus progress={downloads[voiceKey]} model={activeVoice.label} />}

      <label htmlFor="speech-rate">{t("settings.voice.speech_rate")}</label>
      <input
        id="speech-rate"
        type="range"
        min={0.5}
        max={2}
        step={0.1}
        value={voice.speech_rate}
        aria-valuetext={`${voice.speech_rate.toFixed(1)}×`}
        onChange={(e) => onVoice({ speech_rate: Number(e.target.value) })}
      />
      <label htmlFor="volume">{t("settings.voice.volume")}</label>
      <input
        id="volume"
        type="range"
        min={0}
        max={1}
        step={0.05}
        value={voice.volume}
        aria-valuetext={`${Math.round(voice.volume * 100)}%`}
        onChange={(e) => onVoice({ volume: Number(e.target.value) })}
      />
      <label className="checkbox">
        <input type="checkbox" checked={voice.mute} onChange={(e) => onVoice({ mute: e.target.checked })} />
        {t("settings.voice.mute", { name })}
      </label>
      <label htmlFor="speak-replies">{t("settings.voice.speak_replies")}</label>
      <select id="speak-replies" value={voice.speak_replies} onChange={(e) => onVoice({ speak_replies: e.target.value as SpeakReplies })}>
        {SPEAK_REPLIES.map((x) => (
          <option key={x} value={x}>
            {t(`settings.voice.speak.${x}`)}
          </option>
        ))}
      </select>
      <label className="checkbox">
        <input type="checkbox" checked={voice.speak_checkins} onChange={(e) => onVoice({ speak_checkins: e.target.checked })} />
        {t("settings.voice.speak_checkins")}
      </label>

      <label htmlFor="ptt-shortcut">{t("settings.voice.shortcut")}</label>
      <input id="ptt-shortcut" value={voice.ptt_shortcut} onChange={(e) => onVoice({ ptt_shortcut: e.target.value })} aria-describedby="ptt-help" autoComplete="off" spellCheck={false} />
      <small id="ptt-help">{t("settings.voice.shortcut_help")}</small>

      <label htmlFor="input-device">{t("settings.voice.input_device")}</label>
      <select id="input-device" value={voice.input_device} onChange={(e) => onVoice({ input_device: e.target.value })}>
        <option value="">{t("settings.voice.input_default")}</option>
        {[...new Set([voice.input_device, ...devices].filter(Boolean))].map((d) => (
          <option key={d} value={d}>
            {d}
          </option>
        ))}
      </select>
      <label htmlFor="mic-meter">{t("settings.voice.mic_meter")}</label>
      <meter id="mic-meter" min={0} max={1} value={micLevel} aria-describedby="mic-meter-help" />
      <small id="mic-meter-help">{t("settings.voice.mic_meter_help")}</small>

      <fieldset className="nested">
        <legend>{t("settings.voice.hands_free_group")}</legend>
        <label className="checkbox">
          <input type="checkbox" checked={voice.hands_free} onChange={(e) => onVoice({ hands_free: e.target.checked })} aria-describedby="hands-free-help" />
          {t("settings.voice.hands_free")}
        </label>
        <small id="hands-free-help">{t("settings.voice.hands_free_help", { name })}</small>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={voice.wake_phrase}
            disabled={!voice.hands_free}
            onChange={(e) => onVoice({ wake_phrase: e.target.checked })}
            aria-describedby={warnWake ? "wake-warning" : undefined}
          />
          {t("settings.voice.wake_phrase", { name })}
        </label>
        {warnWake && (
          <small id="wake-warning" className="warn" role="note">
            {t("settings.voice.wake_warning", { name })}
          </small>
        )}
        <label className="checkbox">
          <input type="checkbox" checked={voice.barge_in} onChange={(e) => onVoice({ barge_in: e.target.checked })} />
          {t("settings.voice.barge_in")}
        </label>
        <small>{t("settings.voice.headphones_tip", { name })}</small>
      </fieldset>
    </fieldset>
  );
}
