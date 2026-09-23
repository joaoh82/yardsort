import { describe, expect, it } from "vitest";
import { droppedPromptPaths, insertDroppedPaths } from "./drop";

describe("droppedPromptPaths", () => {
  it("copies a path as plain text and leaves a trailing space", () => {
    expect(droppedPromptPaths(["/home/me/notes.md"])).toBe("/home/me/notes.md ");
    expect(droppedPromptPaths(["/a/one.rs", "/a/two.rs"])).toBe("/a/one.rs /a/two.rs ");
  });

  it("double-quotes a path that contains whitespace or a quote, without shell escapes", () => {
    expect(droppedPromptPaths(["/home/me/My Docs/plan.md"])).toBe('"/home/me/My Docs/plan.md" ');
    expect(droppedPromptPaths(['/tmp/say "hi".txt'])).toBe('"/tmp/say \\"hi\\".txt" ');
    expect(droppedPromptPaths(["C:\\Users\\me\\My Docs\\plan.md"])).toBe(
      '"C:\\Users\\me\\My Docs\\plan.md" ',
    );
  });
});

describe("insertDroppedPaths", () => {
  it("appends to what is already written, with a space before the path", () => {
    expect(insertDroppedPaths("Look at", 7, 7, ["/tmp/a.md"])).toEqual({
      message: "Look at /tmp/a.md ",
      caret: 18,
    });
  });

  it("inserts at the caret and replaces a selection", () => {
    expect(insertDroppedPaths("see this file", 4, 4, ["/tmp/a.md"])).toEqual({
      message: "see /tmp/a.md this file",
      caret: 14,
    });
    expect(insertDroppedPaths("see this file", 4, 8, ["/tmp/a.md"])).toEqual({
      message: "see /tmp/a.md  file",
      caret: 14,
    });
  });

  it("puts the path at the start of an empty message", () => {
    expect(insertDroppedPaths("", 0, 0, ["/tmp/a.md", "/tmp/b.md"])).toEqual({
      message: "/tmp/a.md /tmp/b.md ",
      caret: 20,
    });
  });

  it("leaves the message alone when nothing was dropped", () => {
    expect(insertDroppedPaths("keep", 4, 4, [])).toEqual({ message: "keep", caret: 4 });
  });
});
