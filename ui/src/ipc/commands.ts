// Typed wrappers over the Rust commands in core/src/commands.rs.
// Components must call these, never `invoke` directly.
import { invoke } from "@tauri-apps/api/core";
import type {
  AppState,
  ChatMessageDto,
  CheckinMode,
  HistoryEntry,
  Profile,
  ProviderStatus,
  SessionSnapshot,
  Settings,
  SttModelInfo,
  Via,
  VoiceInfo,
  VoiceStatus,
} from "./generated";

export const getState = () => invoke<AppState>("get_state");

export const startSession = (task: string, minutes?: number, checkinMode?: CheckinMode) =>
  invoke<SessionSnapshot>("start_session", {
    task,
    minutes: minutes ?? null,
    checkin_mode: checkinMode ?? null,
  });
export const pauseSession = () => invoke<SessionSnapshot>("pause_session");
export const resumeSession = () => invoke<SessionSnapshot>("resume_session");
export const extendSession = (minutes: number) =>
  invoke<SessionSnapshot>("extend_session", { minutes });
export const endSession = (reflection?: string) =>
  invoke<SessionSnapshot>("end_session", { reflection: reflection ?? null });
export const closeSession = () => invoke<SessionSnapshot>("close_session");
export const startBreak = (minutes?: number) =>
  invoke<SessionSnapshot>("start_break", { minutes: minutes ?? null });
export const addSubtasks = (items: string[]) =>
  invoke<SessionSnapshot>("add_subtasks", { items });
export const completeSubtask = (id: string) =>
  invoke<SessionSnapshot>("complete_subtask", { id });
export const setQuiet = (minutes: number) => invoke<SessionSnapshot>("set_quiet", { minutes });
export const setCheckinMode = (mode: CheckinMode) =>
  invoke<SessionSnapshot>("set_checkin_mode", { mode });

export const sendMessage = (text: string, via: Via = "text") =>
  invoke<ChatMessageDto | null>("send_message", { text, via });

export const updateSettings = (settings: Settings) =>
  invoke<ProviderStatus>("update_settings", { settings });
export const updateProfile = (profile: Profile) =>
  invoke<Profile>("update_profile", { profile });
export const setApiKey = (provider: "anthropic", key: string) =>
  invoke<ProviderStatus>("set_api_key", { provider, key });
export const clearApiKey = (provider: "anthropic") =>
  invoke<ProviderStatus>("clear_api_key", { provider });
export const refreshProvider = () => invoke<ProviderStatus>("refresh_provider");
export const listOllamaModels = (url: string) => invoke<string[]>("list_ollama_models", { url });
export const sessionHistory = (limit?: number) =>
  invoke<HistoryEntry[]>("session_history", { limit: limit ?? null });

export const showMainWindow = () => invoke<null>("show_main_window");
export const setBuddyVisible = (visible: boolean) => invoke<Settings>("set_buddy_visible", { visible });

// Voice (M2). Downloads must only ever be called from an explicit click (rule 10).
export const voiceStatus = () => invoke<VoiceStatus>("voice_status");
export const pttStart = () => invoke<null>("ptt_start");
/** Returns the transcript, or "" if nothing was heard. */
export const pttStop = () => invoke<string>("ptt_stop");
export const pttCancel = () => invoke<null>("ptt_cancel");
export const stopSpeaking = () => invoke<null>("stop_speaking");
export const listVoices = () => invoke<VoiceInfo[]>("list_voices");
export const downloadVoice = (id: string) => invoke<null>("download_voice", { id });
export const previewVoice = (id: string, text: string) => invoke<null>("preview_voice", { id, text });
export const listSttModels = () => invoke<SttModelInfo[]>("list_stt_models");
export const downloadSttModel = (id: string) => invoke<null>("download_stt_model", { id });
export const listInputDevices = () => invoke<string[]>("list_input_devices");
