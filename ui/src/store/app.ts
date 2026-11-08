import { create } from "zustand";
import type {
  AppState,
  AudioLevel,
  CharacterState,
  ChatDelta,
  ChatMessageDto,
  CheckinEvent,
  CoreNotice,
  DownloadProgress,
  Profile,
  ProviderStatus,
  SessionSnapshot,
  SessionTick,
  Settings,
  VoiceStatus,
} from "../ipc";

export interface ChatItem {
  id: string;
  role: "user" | "assistant";
  text: string;
  via: ChatMessageDto["via"];
  /** True while text is still streaming in. */
  draft: boolean;
}

export interface AppStore {
  ready: boolean;
  session: SessionSnapshot | null;
  character: CharacterState;
  settings: Settings | null;
  profile: Profile | null;
  provider: ProviderStatus | null;
  hasAnthropicKey: boolean;
  messages: ChatItem[];
  notice: CoreNotice | null;
  lastCheckin: (CheckinEvent & { at: number }) | null;
  pending: boolean;
  /** Voice pipeline status from `voice://state` (null until the core reports it). */
  voice: VoiceStatus | null;
  /** Mic meter, 0–1. */
  micLevel: number;
  /**
   * Companion speech loudness, 0–1, for mouth movement. `null` until a level arrives in the
   * current character state, so a `talking` state without audio keeps its idle mouth animation.
   */
  speakerLevel: number | null;
  /** Model downloads by model id (`whisper:base.en`, `piper:<voice>`). */
  downloads: Record<string, DownloadProgress>;

  hydrate: (s: AppState) => void;
  setSession: (s: SessionSnapshot) => void;
  applyTick: (t: SessionTick) => void;
  setCharacter: (c: CharacterState) => void;
  applyDelta: (d: ChatDelta) => void;
  applyMessage: (m: ChatMessageDto) => void;
  applyCheckin: (c: CheckinEvent) => void;
  setNotice: (n: CoreNotice | null) => void;
  setSettings: (s: Settings) => void;
  setProfile: (p: Profile) => void;
  setProvider: (p: ProviderStatus, hasKey?: boolean) => void;
  setPending: (p: boolean) => void;
  setVoice: (v: VoiceStatus) => void;
  applyLevel: (l: AudioLevel) => void;
  applyDownload: (d: DownloadProgress) => void;
}

function clamp01(x: number): number {
  return Number.isFinite(x) ? Math.min(1, Math.max(0, x)) : 0;
}

const MAX_MESSAGES = 200;

export const useApp = create<AppStore>((set) => ({
  ready: false,
  session: null,
  character: "idle",
  settings: null,
  profile: null,
  provider: null,
  hasAnthropicKey: false,
  messages: [],
  notice: null,
  lastCheckin: null,
  pending: false,
  voice: null,
  micLevel: 0,
  speakerLevel: null,
  downloads: {},

  hydrate: (s) =>
    set({
      ready: true,
      session: s.session,
      character: s.character,
      settings: s.settings,
      profile: s.profile,
      provider: s.provider,
      hasAnthropicKey: s.has_anthropic_key,
    }),
  setSession: (session) => set({ session }),
  applyTick: (t) =>
    set((st) =>
      st.session
        ? {
            session: {
              ...st.session,
              phase: t.phase,
              elapsed_secs: t.elapsed_secs,
              remaining_secs: t.remaining_secs,
              break_remaining_secs: t.break_remaining_secs,
              quiet_remaining_secs: t.quiet_remaining_secs,
              progress: t.progress,
            },
          }
        : {},
    ),
  setCharacter: (character) =>
    set((st) => (st.character === character ? {} : { character, speakerLevel: null })),
  applyDelta: (d) =>
    set((st) => {
      const idx = st.messages.findIndex((m) => m.id === d.message_id);
      const existing = st.messages[idx];
      if (!existing) {
        const item: ChatItem = { id: d.message_id, role: "assistant", text: d.delta, via: "text", draft: true };
        return { messages: [...st.messages, item].slice(-MAX_MESSAGES) };
      }
      const messages = st.messages.slice();
      messages[idx] = { ...existing, text: existing.text + d.delta };
      return { messages };
    }),
  applyMessage: (m) =>
    set((st) => {
      const item: ChatItem = { id: m.id, role: m.role, text: m.text, via: m.via, draft: false };
      // A finished assistant message replaces its draft in place; leftover drafts from
      // earlier tool rounds are dropped so half-finished text never lingers.
      const messages: ChatItem[] = [];
      let replaced = false;
      for (const x of st.messages) {
        if (x.id === m.id) {
          messages.push(item);
          replaced = true;
        } else if (!(m.role === "assistant" && x.draft)) {
          messages.push(x);
        }
      }
      if (!replaced) messages.push(item);
      return { messages: messages.slice(-MAX_MESSAGES) };
    }),
  applyCheckin: (c) => set({ lastCheckin: { ...c, at: Date.now() } }),
  setNotice: (notice) => set({ notice }),
  setSettings: (settings) => set({ settings }),
  setProfile: (profile) => set({ profile }),
  setProvider: (provider, hasKey) =>
    set((st) => ({ provider, hasAnthropicKey: hasKey ?? st.hasAnthropicKey })),
  setPending: (pending) => set({ pending }),
  setVoice: (voice) => set({ voice }),
  applyLevel: (l) => set(l.source === "mic" ? { micLevel: clamp01(l.level) } : { speakerLevel: clamp01(l.level) }),
  applyDownload: (d) => set((st) => ({ downloads: { ...st.downloads, [d.model]: d } })),
}));

/** The companion's display name. Never hard-code it elsewhere. */
export const useCompanionName = () => useApp((s) => s.profile?.companion.name ?? "");
