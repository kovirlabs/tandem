import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Profile, Settings, SttModelInfo, VoiceInfo } from "../ipc";
import { SettingsPanel } from "../panels/SettingsPanel";
import { useApp } from "../store/app";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const voices: VoiceInfo[] = [
  { id: "en_US-lessac-medium", label: "Lessac (US English)", language: "en_US", license: "CC-BY-4.0", size_bytes: 63_000_000, downloaded: false, is_default: true },
  { id: "en_GB-alan-medium", label: "Alan (UK English)", language: "en_GB", license: "CC-BY-4.0", size_bytes: 63_000_000, downloaded: true, is_default: false },
];
const sttModels: SttModelInfo[] = [{ id: "base.en", size_bytes: 148_000_000, downloaded: false }];

const settings: Settings = {
  chat_provider: "ollama",
  ollama: { url: "http://localhost:11434", chat_model: "", vision_model: "", confirmed_hosts: [] },
  anthropic: { model: "claude", effort: "low" },
  default_minutes: 25,
  checkin_mode: "gentle",
  checkin_interval_min: 10,
  time_warnings: true,
  show_timer: true,
  short_break_min: 5,
  long_break_min: 15,
  message_retention_days: 30,
  debug_save_snapshots: false,
  keep_focus_signals: false,
  buddy: { enabled: false, x: null, y: null },
  voice: {
    enabled: true,
    mute: false,
    speak_replies: "voice_turns",
    speak_checkins: true,
    stt_model: "base.en",
    speech_rate: 1,
    volume: 0.9,
    ptt_shortcut: "CommandOrControl+Shift+Space",
    hands_free: true,
    wake_phrase: false,
    barge_in: true,
    input_device: "",
  },
};

function profileNamed(name: string): Profile {
  return { companion: { name, character_id: "beaver", voice_id: null, appearance: {} }, tone: "calm", about_me: "", onboarded: true };
}

function setup(name = "Maple") {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    switch (cmd) {
      case "list_voices":
        return voices;
      case "list_stt_models":
        return sttModels;
      case "list_input_devices":
        return ["Built-in Microphone"];
      case "update_profile":
        return (args as { profile: Profile }).profile;
      case "update_settings":
        return { kind: "ollama", model: "m", locality: { kind: "local" }, ready: true, tools: true, vision: false, error: null };
      default:
        return null;
    }
  });
  useApp.setState({ settings, profile: profileNamed(name), voice: null, downloads: {}, notice: null });
  return render(<SettingsPanel onClose={() => {}} />);
}

const called = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);

describe("Settings → Voice", () => {
  it("download_needs_click: nothing downloads on open, and the button names huggingface.co", async () => {
    const user = userEvent.setup();
    setup();
    const sttButton = await screen.findByRole("button", { name: /huggingface\.co.*148 MB/ });
    expect(called("list_stt_models")).toHaveLength(1);
    expect(called("download_stt_model")).toHaveLength(0);
    expect(called("download_voice")).toHaveLength(0);

    await user.click(sttButton);
    expect(called("download_stt_model")).toEqual([["download_stt_model", { id: "base.en" }]]);
    expect(called("download_voice")).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: /Download voice from huggingface\.co/ }));
    expect(called("download_voice")).toEqual([["download_voice", { id: "en_US-lessac-medium" }]]);
  });

  it("wake_warning_for_one_syllable_name: warns for a short name and not for a two-syllable one", async () => {
    const { unmount } = setup("Max");
    expect(await screen.findByText(/can trigger by accident/)).toHaveTextContent("Max");
    unmount();
    setup("Maple");
    await screen.findByRole("button", { name: /Download from huggingface/ });
    expect(screen.queryByText(/can trigger by accident/)).toBeNull();
    expect(screen.getByLabelText(/Wait for “Hey Maple” first/)).toBeInTheDocument();
  });

  it("voice_saved_to_profile: the chosen voice goes to profile.companion.voice_id, appearance untouched", async () => {
    const user = userEvent.setup();
    setup();
    await screen.findByRole("option", { name: /Alan/ });
    await user.selectOptions(screen.getByLabelText("Maple's voice"), "en_GB-alan-medium");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(called("update_profile")).toHaveLength(1));
    const [, args] = called("update_profile")[0] ?? [];
    const saved = (args as { profile: Profile }).profile;
    expect(saved.companion.voice_id).toBe("en_GB-alan-medium");
    expect(saved.companion.character_id).toBe("beaver");
    expect(saved.companion.appearance).toEqual({});
  });

  it("previews only downloaded voices, with the live companion name", async () => {
    const user = userEvent.setup();
    setup();
    const preview = await screen.findByRole("button", { name: "Preview" });
    expect(preview).toBeDisabled(); // default voice not downloaded
    await user.selectOptions(screen.getByLabelText("Maple's voice"), "en_GB-alan-medium");
    await user.click(screen.getByRole("button", { name: "Preview" }));
    expect(called("preview_voice")).toEqual([["preview_voice", { id: "en_GB-alan-medium", text: expect.stringContaining("Maple") }]]);
  });

  it("shows friendly copy for a voice problem", async () => {
    setup();
    useApp.setState({ voice: { phase: "off", hands_free: false, stt_ready: false, tts_ready: false, problem: "voice.error.stt_missing" } });
    expect(await screen.findByText(/speech model isn't downloaded yet/)).toBeInTheDocument();
  });
});
