import { describe, expect, it } from "vitest";
import { droppedPathsText, SHIFT_ENTER, shiftEnterInput } from "./input";

const keydown = (init: KeyboardEventInit) =>
  new KeyboardEvent("keydown", { key: "Enter", ...init });

describe("shiftEnterInput", () => {
  it("encodes Shift+Enter for a program that speaks the kitty protocol", () => {
    expect(shiftEnterInput(keydown({ shiftKey: true }), true)).toBe(SHIFT_ENTER);
    expect(SHIFT_ENTER).toBe("\x1b[13;2u");
  });

  it("leaves it to xterm for one that does not — a shell would print the encoding", () => {
    expect(shiftEnterInput(keydown({ shiftKey: true }), false)).toBeNull();
  });

  it("ignores plain Enter, other modifiers, other keys and key-up", () => {
    expect(shiftEnterInput(keydown({}), true)).toBeNull();
    expect(shiftEnterInput(keydown({ shiftKey: true, ctrlKey: true }), true)).toBeNull();
    expect(shiftEnterInput(keydown({ shiftKey: true, altKey: true }), true)).toBeNull();
    expect(shiftEnterInput(keydown({ shiftKey: true, metaKey: true }), true)).toBeNull();
    expect(shiftEnterInput(keydown({ key: "a", shiftKey: true }), true)).toBeNull();
    expect(
      shiftEnterInput(new KeyboardEvent("keyup", { key: "Enter", shiftKey: true }), true),
    ).toBeNull();
  });
});

describe("droppedPathsText", () => {
  it("escapes what a POSIX shell would interpret, and ends with a space", () => {
    expect(droppedPathsText(["/home/me/notes.md"], false)).toBe("/home/me/notes.md ");
    expect(droppedPathsText(["/home/me/My Docs/plan (v2).md"], false)).toBe(
      "/home/me/My\\ Docs/plan\\ \\(v2\\).md ",
    );
    expect(droppedPathsText(["/tmp/it's $HERE"], false)).toBe("/tmp/it\\'s\\ \\$HERE ");
  });

  it("separates several files with spaces", () => {
    expect(droppedPathsText(["/a/one.rs", "/a/two.rs"], false)).toBe("/a/one.rs /a/two.rs ");
  });

  it("double-quotes on Windows only when needed, keeping backslashes", () => {
    expect(droppedPathsText(["C:\\Users\\me\\notes.md"], true)).toBe("C:\\Users\\me\\notes.md ");
    expect(droppedPathsText(["C:\\Users\\me\\My Docs\\plan.md"], true)).toBe(
      '"C:\\Users\\me\\My Docs\\plan.md" ',
    );
  });
});
