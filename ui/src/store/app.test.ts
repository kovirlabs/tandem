import { useApp } from "./app";
import { formatClock } from "./actions";

const base = { via: "text" as const, ts: "" };

describe("app store", () => {
  beforeEach(() => useApp.setState({ messages: [] }));

  it("streams deltas into a draft and replaces it with the final message", () => {
    const s = useApp.getState();
    s.applyMessage({ id: "u1", role: "user", text: "hi", ...base });
    s.applyDelta({ message_id: "a1", delta: "Hel" });
    s.applyDelta({ message_id: "a1", delta: "lo" });
    expect(useApp.getState().messages.map((m) => [m.text, m.draft])).toEqual([
      ["hi", false],
      ["Hello", true],
    ]);
    s.applyMessage({ id: "a1", role: "assistant", text: "Hello.", ...base });
    expect(useApp.getState().messages.map((m) => [m.id, m.text, m.draft])).toEqual([
      ["u1", "hi", false],
      ["a1", "Hello.", false],
    ]);
  });

  it("drops stale drafts from earlier tool rounds", () => {
    const s = useApp.getState();
    s.applyDelta({ message_id: "r1", delta: "Paused!" });
    s.applyDelta({ message_id: "r2", delta: "There's no session" });
    s.applyMessage({ id: "r2", role: "assistant", text: "There's no session yet.", ...base });
    expect(useApp.getState().messages.map((m) => m.id)).toEqual(["r2"]);
  });

  it("formats clocks", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(1500)).toBe("25:00");
    expect(formatClock(3725)).toBe("1:02:05");
    expect(formatClock(-5)).toBe("0:00");
  });
});
