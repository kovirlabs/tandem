import { render, screen } from "@testing-library/react";
import { Character } from "./Character";
import { CHARACTER_IDS } from "./registry";
import { ALL_STATES } from "./stateMap";

describe("Character", () => {
  for (const id of CHARACTER_IDS) {
    it.each(ALL_STATES)(`renders the ${id} in state %s with an accessible label`, (state) => {
      const { container } = render(<Character characterId={id} companionName="Maple" state={state} progress={0.5} />);
      expect(screen.getByRole("img")).toHaveAccessibleName(expect.stringContaining("Maple"));
      expect(container.querySelector(`svg.${id}.state-${state}`)).not.toBeNull();
    });
  }

  it("grows the beaver's dam stick by stick", () => {
    const { container, rerender } = render(<Character characterId="beaver" companionName="M" state="working" progress={0} />);
    expect(container.querySelectorAll(".dam .stick")).toHaveLength(0);
    rerender(<Character characterId="beaver" companionName="M" state="working" progress={1} />);
    expect(container.querySelectorAll(".dam .stick")).toHaveLength(8);
  });

  it("grows the fox's den piece by piece", () => {
    const { container, rerender } = render(<Character characterId="fox" companionName="M" state="working" progress={0} />);
    expect(container.querySelectorAll(".den .den-piece")).toHaveLength(0);
    rerender(<Character characterId="fox" companionName="M" state="working" progress={0.5} />);
    expect(container.querySelectorAll(".den .den-piece")).toHaveLength(4);
    rerender(<Character characterId="fox" companionName="M" state="working" progress={1} />);
    expect(container.querySelectorAll(".den .den-piece")).toHaveLength(8);
  });

  it("falls back to the beaver for an unknown pack", () => {
    const { container } = render(<Character characterId="dragon" companionName="M" state="idle" progress={0} />);
    expect(container.querySelector("svg.beaver")).not.toBeNull();
  });
});
