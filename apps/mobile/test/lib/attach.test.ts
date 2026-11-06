// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import { pastedPath } from "../../src/lib/attach";

describe("a picture's path, as it is pasted", () => {
  test("is quoted as the desktop quotes a dropped file, for the computer's own shell", () => {
    expect(pastedPath("/Users/ana/Library/Application Support/a.jpg", "macos")).toBe(
      "/Users/ana/Library/Application\\ Support/a.jpg ",
    );
    expect(pastedPath("/home/ana/data/1-it's.jpg", "linux")).toBe("/home/ana/data/1-it\\'s.jpg ");
    expect(pastedPath("C:\\Users\\Ana Lee\\a.jpg", "windows")).toBe('"C:\\Users\\Ana Lee\\a.jpg" ');
    expect(pastedPath("C:\\Users\\ana\\a.jpg", "windows")).toBe("C:\\Users\\ana\\a.jpg ");
  });
});
