import { render, screen } from "@testing-library/react";
import { useApp } from "../store/app";
import { Buddy } from "./Buddy";

describe("Buddy", () => {
  it("shows the companion, a bubble for check-ins, and no bubble for glances", () => {
    useApp.setState({ lastCheckin: { reason: "halfway", action: "gentle_line", text: "Halfway. Still here.", at: 1 } });
    const { rerender } = render(<Buddy />);
    expect(screen.getByRole("button", { name: /on your desktop/ })).toBeInTheDocument();
    expect(screen.getByText("Halfway. Still here.")).toBeInTheDocument();
    useApp.setState({ lastCheckin: { reason: "interval", action: "silent_glance", text: null, at: 2 } });
    rerender(<Buddy />);
    expect(screen.queryByText("Halfway. Still here.")).toBeNull();
  });
});

describe("Buddy voice", () => {
  it("offers push-to-talk when voice is on and shows listening", () => {
    useApp.setState({ lastCheckin: null, voice: { phase: "off", hands_free: false, stt_ready: false, tts_ready: false, problem: null } });
    const { container, rerender } = render(<Buddy />);
    expect(screen.queryByRole("button", { name: /Talk to/ })).toBeNull();
    useApp.setState({ voice: { phase: "listening", hands_free: false, stt_ready: true, tts_ready: true, problem: null } });
    rerender(<Buddy />);
    expect(screen.getByRole("button", { name: /Done talking/ })).toHaveAttribute("aria-pressed", "true");
    expect(container.querySelector(".buddy.voice-listening")).not.toBeNull();
  });
});
