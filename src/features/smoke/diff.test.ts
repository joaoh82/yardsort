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

  it("diffs a one-line edit in a huge file without building a huge matrix", () => {
    // 50,000 identical lines with one changed in the middle: a direct LCS would allocate
    // billions of cells. Stripping the common prefix and suffix leaves a single differing line,
    // so this stays instant and shows just the edit.
    const lines = Array.from({ length: 50_000 }, (_, i) => `line ${i}`);
    const before = lines.join("\n");
    const edited = [...lines];
    edited[25_000] = "line 25000 changed";
    const diff = diffLines(before, edited.join("\n"));
    expect(diff.filter((line) => line.kind !== "same")).toEqual([
      { kind: "removed", text: "line 25000" },
      { kind: "added", text: "line 25000 changed" },
    ]);
    expect(diff).toHaveLength(50_001);
  });

  it("falls back to whole-version display when too much differs, still showing every line", () => {
    // Two files that share no lines and are both larger than the matrix cap: the diff must not
    // allocate the quadratic matrix, and must still let the reader see all of the new version.
    const before = Array.from({ length: 4000 }, (_, i) => `old ${i}`).join("\n");
    const after = Array.from({ length: 4000 }, (_, i) => `new ${i}`).join("\n");
    const diff = diffLines(before, after);
    const removed = diff.filter((line) => line.kind === "removed");
    const added = diff.filter((line) => line.kind === "added");
    expect(removed).toHaveLength(4000);
    expect(added).toHaveLength(4000);
    expect(added.at(-1)).toEqual({ kind: "added", text: "new 3999" });
  });
});
