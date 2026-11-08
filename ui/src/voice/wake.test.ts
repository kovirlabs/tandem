import { syllables, wakeNameWarning } from "./wake";
import { formatBytes } from "./format";

describe("wake name warning", () => {
  it("wake_warning_for_one_syllable_name", () => {
    for (const n of ["Max", "Sam", "Mike", "Joe", "Bob"]) expect(wakeNameWarning(n), n).toBe(true);
  });

  it("warns for common words even with two syllables", () => {
    for (const n of ["Buddy", "Honey", "Okay", "people"]) expect(wakeNameWarning(n), n).toBe(true);
  });

  it("is quiet for two-syllable names and names it can't judge", () => {
    for (const n of ["Timber", "Maple", "Luna", "Biscuit", "Juniper", "", "ハル", "Mary Jo"]) {
      expect(wakeNameWarning(n), n).toBe(false);
    }
  });

  it("counts syllables roughly", () => {
    expect(syllables("Timber")).toBe(2);
    expect(syllables("Mike")).toBe(1);
    expect(syllables("Bobble")).toBe(2);
    expect(syllables("ハル")).toBeNull();
  });

  it("formats sizes", () => {
    expect(formatBytes(147_951_465)).toBe("148 MB");
    expect(formatBytes(1_500_000_000)).toBe("1.5 GB");
    expect(formatBytes(0)).toBe("?");
  });
});
