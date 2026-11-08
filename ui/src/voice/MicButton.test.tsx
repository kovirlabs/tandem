import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { VoicePhase } from "../ipc";
import { useApp } from "../store/app";
import { MicButton } from "./MicButton";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null)) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

function setPhase(phase: VoicePhase) {
  useApp.setState({ voice: { phase, hands_free: false, stt_ready: true, tts_ready: true, problem: null } });
}

describe("MicButton", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    useApp.setState({ notice: null, micLevel: 0 });
  });

  it("mic_button_toggles_ptt: press starts listening, press again stops and sends", async () => {
    const user = userEvent.setup();
    setPhase("ready");
    const { rerender } = render(<MicButton />);
    const btn = screen.getByRole("button");
    expect(btn).toHaveAttribute("aria-pressed", "false");
    await user.click(btn);
    expect(invokeMock).toHaveBeenLastCalledWith("ptt_start");

    setPhase("listening");
    rerender(<MicButton />);
    expect(screen.getByRole("button")).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button"));
    expect(invokeMock).toHaveBeenLastCalledWith("ptt_stop");
  });

  it("mic_disabled_when_off: no command, points to Settings instead", async () => {
    const user = userEvent.setup();
    const onOff = vi.fn();
    setPhase("off");
    render(<MicButton onOff={onOff} />);
    const btn = screen.getByRole("button");
    expect(btn).toHaveAttribute("aria-disabled", "true");
    expect(btn).toHaveAttribute("title", expect.stringContaining("Settings"));
    await user.click(btn);
    expect(invokeMock).not.toHaveBeenCalled();
    expect(onOff).toHaveBeenCalledOnce();
  });

  it("treats a missing voice status as off", () => {
    useApp.setState({ voice: null });
    render(<MicButton />);
    expect(screen.getByRole("button")).toHaveAttribute("aria-disabled", "true");
  });

  it("keyboard_activates_mic: Enter and Space work", async () => {
    const user = userEvent.setup();
    setPhase("ready");
    render(<MicButton />);
    screen.getByRole("button").focus();
    await user.keyboard("{Enter}");
    expect(invokeMock).toHaveBeenCalledWith("ptt_start");
    invokeMock.mockClear();
    await user.keyboard(" ");
    expect(invokeMock).toHaveBeenCalledWith("ptt_start");
  });

  it("ignores presses while transcribing", async () => {
    const user = userEvent.setup();
    setPhase("transcribing");
    render(<MicButton />);
    await user.click(screen.getByRole("button"));
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
