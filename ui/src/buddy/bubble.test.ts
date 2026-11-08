import { bubbleText } from "./bubble";

describe("buddy bubble", () => {
  it("stays silent for skips and glances", () => {
    expect(bubbleText({ reason: "interval", action: "skip", text: "x" })).toBeNull();
    expect(bubbleText({ reason: "interval", action: "silent_glance", text: null })).toBeNull();
  });
  it("uses generated text, else friendly fallback copy", () => {
    expect(bubbleText({ reason: "halfway", action: "gentle_line", text: "Halfway!" })).toBe("Halfway!");
    expect(bubbleText({ reason: "drift", action: "ask", text: null })).toMatch(/hop back in/);
  });
});
