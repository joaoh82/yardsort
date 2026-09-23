import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { HarnessIcon } from "./HarnessIcon";

const draw = (props: Parameters<typeof HarnessIcon>[0]) =>
  render(<HarnessIcon {...props} />).container.querySelector("svg")!;

describe("HarnessIcon", () => {
  it("gives every built-in a glyph of its own", () => {
    for (const id of ["claude", "codex", "grok", "opencode"]) {
      const svg = draw({ id });
      // Drawn, not lettered: the fallback is the only glyph made of text.
      expect(svg.querySelector("text")).toBeNull();
      expect(svg.children.length).toBeGreaterThan(0);
    }
  });

  it("falls back to the initial for a harness it has never heard of", () => {
    expect(draw({ id: "aider", label: "Aider" }).querySelector("text")).toHaveTextContent("A");
    // No label, and nothing usable in the id: still a mark, never an empty box.
    expect(draw({ id: "  " }).querySelector("text")).toHaveTextContent("?");
  });

  it("is decorative: the name beside it is what gets read out", () => {
    expect(draw({ id: "claude" })).toHaveAttribute("aria-hidden", "true");
  });
});
