// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What the build has to embed for the screens to look as they should. A font it does not have is no
// error on a phone: the system's is used instead, and only someone looking would see.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";

import { describe, expect, test } from "@jest/globals";

import config from "../../app.config";
import { FONTS } from "../../src/ui/parts";

const ROOT = join(__dirname, "../..");

function plugin(name: string): Record<string, unknown> {
  const app = config({ config: {} } as never);
  const found = app.plugins?.find((entry) => Array.isArray(entry) && entry[0] === name);
  return (found as [string, Record<string, unknown>])[1];
}

describe("what the build embeds", () => {
  const embedded = plugin("expo-font").fonts as string[];

  test("every font a screen names, under that name", () => {
    expect(embedded.map((path) => basename(path, ".ttf")).sort()).toEqual(
      Object.values(FONTS).sort(),
    );
  });

  test("each one is there, with its license beside it", () => {
    for (const path of embedded) expect(existsSync(join(ROOT, path))).toBe(true);
    const files = readdirSync(join(ROOT, "assets/fonts"));
    expect(files).toEqual(expect.arrayContaining(["Inter-OFL.txt", "JetBrainsMono-OFL.txt"]));
  });

  test("the icon is whole and square, since iOS refuses one with transparency at upload", () => {
    const icon = readFileSync(join(ROOT, config({ config: {} } as never).icon as string));
    // A PNG's IHDR: width and height at 16 and 20, and its color type at 25. 2 is RGB, no alpha.
    expect([icon.readUInt32BE(16), icon.readUInt32BE(20), icon[25]]).toEqual([1024, 1024, 2]);
  });
});
