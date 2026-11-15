// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import type { ChoiceFile } from "../../src/lib/appearance";
import { Folds, MAX_REMEMBERED } from "../../src/lib/folds";

function memoryFile(kept: string | null = null): ChoiceFile & { kept: string | null } {
  return {
    kept,
    read() {
      return this.kept;
    },
    write(text: string) {
      this.kept = text;
    },
  };
}

describe("folded projects", () => {
  test("a project is open until it is folded, and open again when pressed again", () => {
    const folds = new Folds(memoryFile());
    expect(folds.has("host-1", "p-1")).toBe(false);
    expect(folds.toggle("host-1", "p-1")).toBe(true);
    expect(folds.has("host-1", "p-1")).toBe(true);
    expect(folds.toggle("host-1", "p-1")).toBe(false);
    expect(folds.has("host-1", "p-1")).toBe(false);
  });

  test("a fold is remembered in the file, for its computer only", () => {
    const file = memoryFile();
    new Folds(file).toggle("host-1", "p-1");
    const again = new Folds(file);
    expect(again.has("host-1", "p-1")).toBe(true);
    expect(again.has("host-2", "p-1")).toBe(false);
    expect(again.has("host-1", "p-2")).toBe(false);
    // Opened again, it is gone from the file too.
    again.toggle("host-1", "p-1");
    expect(new Folds(file).has("host-1", "p-1")).toBe(false);
  });

  test("a file that cannot be read, or holds something else, is nothing folded", () => {
    expect(new Folds(memoryFile("not json")).has("host-1", "p-1")).toBe(false);
    expect(new Folds(memoryFile('{"host-1 p-1":true}')).has("host-1", "p-1")).toBe(false);
    const throws: ChoiceFile = {
      read() {
        throw new Error("no disk");
      },
      write() {
        throw new Error("no disk");
      },
    };
    const folds = new Folds(throws);
    expect(folds.has("host-1", "p-1")).toBe(false);
    // It still folds, for as long as the app is open.
    expect(folds.toggle("host-1", "p-1")).toBe(true);
    expect(folds.has("host-1", "p-1")).toBe(true);
  });

  test("the oldest folds are dropped past the most it remembers", () => {
    const file = memoryFile();
    const folds = new Folds(file);
    for (let n = 0; n <= MAX_REMEMBERED; n++) folds.toggle("host-1", `p-${n}`);
    expect(folds.has("host-1", "p-0")).toBe(false);
    expect(folds.has("host-1", "p-1")).toBe(true);
    expect(folds.has("host-1", `p-${MAX_REMEMBERED}`)).toBe(true);
    expect((JSON.parse(file.kept ?? "[]") as unknown[]).length).toBe(MAX_REMEMBERED);
  });
});
