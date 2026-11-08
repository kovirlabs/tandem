import { ALL_STATES, STATE_VALUES, damLevel } from "./stateMap";

describe("stateMap", () => {
  it("maps all nine states to unique values", () => {
    expect(ALL_STATES).toHaveLength(9);
    expect(new Set(Object.values(STATE_VALUES)).size).toBe(9);
  });

  it("grows the dam stick by stick and clamps", () => {
    expect(damLevel(0)).toBe(0);
    expect(damLevel(0.1)).toBe(0);
    expect(damLevel(0.125)).toBe(13);
    expect(damLevel(1)).toBe(100);
    expect(damLevel(5)).toBe(100);
    expect(damLevel(Number.NaN)).toBe(0);
  });
});
