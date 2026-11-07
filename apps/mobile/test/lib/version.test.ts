// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The phone app's version for the TestFlight workflow: raised by a part, from the newest
 * `mobile-v` tag in a real repository's history.
 */

import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, test } from "bun:test";

import { bump, nextVersion, taggedVersion } from "../../scripts/version";

describe("bump", () => {
  test("raises a part and puts the ones after it back to 0", () => {
    expect(bump("1.4.7", "none")).toBe("1.4.7");
    expect(bump("1.4.7", "patch")).toBe("1.4.8");
    expect(bump("1.4.7", "minor")).toBe("1.5.0");
    expect(bump("1.4.7", "major")).toBe("2.0.0");
    expect(bump("0.9.9", "patch")).toBe("0.9.10");
  });

  test("refuses what is not a version", () => {
    expect(() => bump("1.2", "patch")).toThrow(/not a version/);
  });
});

describe("taggedVersion", () => {
  let dir = "";
  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  function repo(): (...args: string[]) => string {
    dir = mkdtempSync(join(tmpdir(), "yardsort-version-"));
    const git = (...args: string[]) =>
      execFileSync("git", args, {
        cwd: dir,
        encoding: "utf8",
        env: {
          ...process.env,
          // No signing or hooks from the person's own settings.
          GIT_CONFIG_GLOBAL: "/dev/null",
          GIT_CONFIG_NOSYSTEM: "1",
          GIT_AUTHOR_NAME: "Test",
          GIT_AUTHOR_EMAIL: "test@example.com",
          GIT_COMMITTER_NAME: "Test",
          GIT_COMMITTER_EMAIL: "test@example.com",
        },
      });
    git("init", "-q", "-b", "main");
    git("commit", "-q", "--allow-empty", "-m", "first");
    return git;
  }

  test("starts from the first version with no tag", () => {
    repo();
    expect(taggedVersion(dir)).toBeNull();
    expect(nextVersion(dir, "patch", "0.1.0")).toBe("0.1.1");
  });

  test("takes the highest version, not the last in the tags' order, and ignores the desktop's", () => {
    const git = repo();
    git("tag", "mobile-v0.9.0");
    git("commit", "-q", "--allow-empty", "-m", "second");
    git("tag", "mobile-v0.10.0");
    git("tag", "v5.0.0");
    git("tag", "mobile-vnext");
    expect(taggedVersion(dir)).toBe("0.10.0");
    expect(nextVersion(dir, "minor", "0.1.0")).toBe("0.11.0");
  });

  test("counts only tags in the current commit's history", () => {
    const git = repo();
    git("tag", "mobile-v0.2.0");
    git("switch", "-q", "-c", "other");
    git("commit", "-q", "--allow-empty", "-m", "elsewhere");
    git("tag", "-a", "mobile-v0.3.0", "-m", "Yardsort for the phone 0.3.0");
    git("switch", "-q", "main");
    expect(taggedVersion(dir)).toBe("0.2.0");
  });
});
