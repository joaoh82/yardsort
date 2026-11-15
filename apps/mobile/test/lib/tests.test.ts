// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";
import type { SessionRecordId } from "@yardsort/remote-client";

import { askedByWords, changed, diffLines } from "../../src/lib/tests";

const HERE = "session-1" as SessionRecordId;

describe("a waiting smoke test", () => {
  test("says who asked: this conversation's agent by its name, another as another", () => {
    expect(askedByWords({ askedBy: "agent", session: HERE }, HERE, "claude")).toBe("claude");
    expect(
      askedByWords({ askedBy: "agent", session: "other" as SessionRecordId }, HERE, "claude"),
    ).toBe("another agent");
    // An agent's run with no conversation recorded is not taken for this one's.
    expect(askedByWords({ askedBy: "agent" }, HERE, "claude")).toBe("another agent");
    expect(askedByWords({ askedBy: "workflow" }, HERE, "claude")).toBe("a workflow");
    expect(askedByWords({ askedBy: "app" }, HERE, "claude")).toBe("you, at the computer");
    expect(askedByWords({ askedBy: "other" }, HERE, "claude")).toBe("the computer");
  });
});

describe("what changed in a test file", () => {
  const before = ["name: Sign-in", "version: 1", "steps:", "  - id: a", "    run: one", "tail"];

  test("a changed line is one removed and one added, with the rest the same", () => {
    const after = [...before];
    after[4] = "    run: two";
    expect(diffLines(before.join("\n"), after.join("\n"))).toEqual([
      { kind: "same", text: "name: Sign-in" },
      { kind: "same", text: "version: 1" },
      { kind: "same", text: "steps:" },
      { kind: "same", text: "  - id: a" },
      { kind: "removed", text: "    run: one" },
      { kind: "added", text: "    run: two" },
      { kind: "same", text: "tail" },
    ]);
  });

  test("lines added and removed in the middle are found, whatever is alike around them", () => {
    const lines = diffLines("a\nb\nc\nd\n", "a\nc\nx\nd\n");
    expect(lines).toEqual([
      { kind: "same", text: "a" },
      { kind: "removed", text: "b" },
      { kind: "same", text: "c" },
      { kind: "added", text: "x" },
      { kind: "same", text: "d" },
    ]);
  });

  test("the same file twice has nothing changed, and an empty one is all added", () => {
    expect(diffLines("a\nb\n", "a\nb\n").every((line) => line.kind === "same")).toBe(true);
    expect(diffLines("", "a\nb\n")).toEqual([
      { kind: "added", text: "a" },
      { kind: "added", text: "b" },
    ]);
  });

  test("only the changes and the lines around them show, the rest counted", () => {
    const long = Array.from({ length: 20 }, (_, n) => `line ${n}`);
    const after = [...long];
    after[10] = "changed";
    const shown = changed(diffLines(long.join("\n"), after.join("\n")));
    expect(shown).toEqual([
      { kind: "gap", hidden: 8 },
      { kind: "same", text: "line 8" },
      { kind: "same", text: "line 9" },
      { kind: "removed", text: "line 10" },
      { kind: "added", text: "changed" },
      { kind: "same", text: "line 11" },
      { kind: "same", text: "line 12" },
      { kind: "gap", hidden: 7 },
    ]);
    // Nothing changed: nothing to show.
    expect(changed(diffLines("a\n", "a\n"))).toEqual([]);
  });
});
