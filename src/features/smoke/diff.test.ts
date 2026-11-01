import { describe, expect, it } from "vitest";
import { diffLines } from "./diff";

describe("diffLines", () => {
  it("keeps common lines and marks the rest added or removed, in order", () => {
    const before = "id: health\nname: Health\nsteps:\n  - id: ping\n    action: http\n";
    const after =
      "id: health\nname: Health check\nsteps:\n  - id: ping\n    action: http\n  - id: look\n    action: manual\n";
    expect(diffLines(before, after)).toEqual([
      { kind: "same", text: "id: health" },
      { kind: "removed", text: "name: Health" },
      { kind: "added", text: "name: Health check" },
      { kind: "same", text: "steps:" },
      { kind: "same", text: "  - id: ping" },
      { kind: "same", text: "    action: http" },
      { kind: "added", text: "  - id: look" },
      { kind: "added", text: "    action: manual" },
    ]);
  });

  it("says nothing changed when nothing did", () => {
    expect(diffLines("a\nb\n", "a\nb\n").every((line) => line.kind === "same")).toBe(true);
  });
});
