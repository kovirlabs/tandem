import en from "./en.json";
import { t, tDynamic } from ".";

describe("i18n", () => {
  it("fills placeholders and leaves unknown ones", () => {
    expect(t("start.minutes_option", { n: 25 })).toBe("25 min");
    expect(t("chat.placeholder")).toContain("{{name}}");
    expect(tDynamic("no.such.key")).toBe("no.such.key");
  });

  it("never hard-codes the default companion name in UI copy", () => {
    for (const [key, value] of Object.entries(en)) {
      expect(value, key).not.toMatch(/timber/i);
    }
  });

  it("has fallback copy for every check-in reason", () => {
    for (const reason of ["interval", "halfway", "five_minutes_left", "two_minutes_left", "session_complete", "break_over", "returned", "drift"]) {
      expect(tDynamic(`checkin.fallback.${reason}`)).not.toBe(`checkin.fallback.${reason}`);
    }
  });

  it("contains no shame or scolding language", () => {
    const banned = /\b(should have|lazy|wasted|failed|failure|distracted \d|score|streak|disappoint|again\?!)/i;
    for (const [key, value] of Object.entries(en)) {
      expect(value, key).not.toMatch(banned);
    }
  });
});
