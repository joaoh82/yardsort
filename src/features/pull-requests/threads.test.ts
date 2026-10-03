import { describe, expect, it } from "vitest";
import type { LineComment } from "@/lib/ipc";
import { placed, placeOf, threadCounts, threadsOf, unplaced } from "./threads";

const comment = (id: string, extra: Partial<LineComment> = {}): LineComment => ({
  id,
  path: "src/a.rs",
  line: 12,
  startLine: null,
  side: "right",
  originalLine: 12,
  outdated: false,
  wholeFile: false,
  author: "grace",
  at: Date.parse("2026-10-01T10:00:00Z") + Number(id) * 1000,
  body: `comment ${id}`,
  url: null,
  inReplyTo: null,
  ...extra,
});

describe("threadsOf", () => {
  it("groups replies under their first comment, oldest thread first, and only for the file", () => {
    const threads = threadsOf(
      [
        comment("3", { inReplyTo: "1" }),
        comment("2", { line: 40 }),
        comment("1"),
        comment("4", { path: "other.rs" }),
        // A reply to something not here stands on its own rather than vanish.
        comment("5", { inReplyTo: "9" }),
      ],
      "src/a.rs",
    );
    expect(threads.map((thread) => [thread.root.id, thread.replies.map((r) => r.id)])).toEqual([
      ["1", ["3"]],
      ["2", []],
      ["5", []],
    ]);
  });

  it("tells the threads with a line to sit on from the rest", () => {
    const threads = threadsOf(
      [
        comment("1"),
        comment("2", { line: null, outdated: true, originalLine: 7 }),
        comment("3", { line: null, originalLine: null, wholeFile: true }),
      ],
      "src/a.rs",
    );
    expect(placed(threads).map((t) => t.root.id)).toEqual(["1"]);
    expect(unplaced(threads).map((t) => t.root.id)).toEqual(["2", "3"]);
  });

  it("counts threads per file, not replies", () => {
    const counts = threadCounts([
      comment("1"),
      comment("2", { inReplyTo: "1" }),
      comment("3", { line: 40 }),
      comment("4", { path: "b.rs" }),
    ]);
    expect([...counts]).toEqual([
      ["src/a.rs", 2],
      ["b.rs", 1],
    ]);
  });
});

describe("placeOf", () => {
  it("says where a comment sits, in words", () => {
    expect(placeOf(comment("1"))).toBe("line 12");
    expect(placeOf(comment("1", { startLine: 10 }))).toBe("lines 10–12");
    expect(placeOf(comment("1", { side: "left" }))).toBe("line 12 of the old text");
    expect(placeOf(comment("1", { line: null, outdated: true, originalLine: 7 }))).toBe(
      "outdated — was on line 7",
    );
    expect(placeOf(comment("1", { line: null, outdated: true, originalLine: null }))).toBe(
      "outdated",
    );
    expect(placeOf(comment("1", { line: null, wholeFile: true }))).toBe("on the whole file");
  });
});
