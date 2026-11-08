import { act, render } from "@testing-library/react";
import { useApp } from "../store/app";
import { Character } from "./Character";
import { CHARACTER_IDS } from "./registry";
import { mouthOpenValue } from "./stateMap";

function mouthRy(container: HTMLElement): number {
  return Number(container.querySelector(".mouth-inside")?.getAttribute("ry"));
}

function setReducedMotion(reduced: boolean) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: reduced && query.includes("reduce"),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
}

describe("mouth movement", () => {
  afterEach(() => setReducedMotion(false));

  for (const id of CHARACTER_IDS) {
    it(`mouth_follows_level (${id})`, () => {
      setReducedMotion(false);
      const { container, rerender } = render(<Character characterId={id} companionName="M" state="talking" progress={0} mouth={0} />);
      const closed = mouthRy(container);
      rerender(<Character characterId={id} companionName="M" state="talking" progress={0} mouth={0.5} />);
      const half = mouthRy(container);
      rerender(<Character characterId={id} companionName="M" state="talking" progress={0} mouth={1} />);
      const open = mouthRy(container);
      expect(closed).toBeLessThan(half);
      expect(half).toBeLessThan(open);
      expect(container.querySelector(".mouth.level-driven")).not.toBeNull();
    });

    it(`reduced_motion_static_mouth (${id})`, () => {
      setReducedMotion(true);
      const { container, rerender } = render(<Character characterId={id} companionName="M" state="talking" progress={0} mouth={0} />);
      const a = mouthRy(container);
      rerender(<Character characterId={id} companionName="M" state="talking" progress={0} mouth={1} />);
      expect(mouthRy(container)).toBe(a);
      expect(container.querySelector(".mouth.level-driven")).toBeNull();
      expect(container.querySelector("svg.reduced")).not.toBeNull();
    });
  }

  it("keeps the built-in talking animation when no audio level has arrived", () => {
    const { container } = render(<Character characterId="beaver" companionName="M" state="talking" progress={0} mouth={null} />);
    expect(container.querySelector(".mouth.talking")).not.toBeNull();
    expect(container.querySelector(".mouth.level-driven")).toBeNull();
  });

  it("reads the speaker level from the store by default", () => {
    useApp.setState({ character: "talking", speakerLevel: null });
    const { container } = render(<Character characterId="fox" companionName="M" state="talking" progress={0} />);
    expect(container.querySelector(".mouth.level-driven")).toBeNull();
    act(() => useApp.getState().applyLevel({ source: "speaker", level: 0.8 }));
    expect(container.querySelector(".mouth.level-driven")).not.toBeNull();
  });

  it("maps levels to the Rive mouthOpen input (0–100), static under reduced motion", () => {
    expect(mouthOpenValue(0.5, "talking", false)).toBe(50);
    expect(mouthOpenValue(2, "talking", false)).toBe(100);
    expect(mouthOpenValue(null, "talking", false)).toBeNull();
    expect(mouthOpenValue(0.9, "talking", true)).toBe(mouthOpenValue(0.1, "talking", true));
    expect(mouthOpenValue(0.9, "idle", true)).toBe(0);
  });
});
