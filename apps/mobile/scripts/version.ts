// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The phone app's version, the one TestFlight and the phone show beside the build number. It is
 * kept in git tags, `mobile-v1.2.3`, not in a file: the TestFlight workflow cannot push a commit
 * to `main`, whose checks only an administrator may skip, but it can push a tag. The workflow
 * asks this for the next version, builds it, and tags the commit it built once Apple has it.
 *
 * Run with `bun`: `bun scripts/version.ts [none|patch|minor|major]` prints the newest version
 * tagged in the current commit's history, raised by that part. With no tag yet, it starts from
 * `package.json`'s version, the one the first builds had.
 */

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");

/** What a run raises: nothing, for a rebuild of the same version. */
export type Part = "none" | "patch" | "minor" | "major";

/** The prefix of the phone app's tags; the desktop app's are `v1.2.3`. */
export const TAG_PREFIX = "mobile-v";

const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;

/** `version` raised by `part`, the parts after it back to 0. */
export function bump(version: string, part: Part): string {
  const found = SEMVER.exec(version);
  if (!found) throw new Error(`"${version}" is not a version such as 1.2.3.`);
  const [major, minor, patch] = found.slice(1).map(Number) as [number, number, number];
  switch (part) {
    case "none":
      return version;
    case "patch":
      return `${major}.${minor}.${patch + 1}`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    case "major":
      return `${major + 1}.0.0`;
  }
}

function compare(a: string, b: string): number {
  const [x, y] = [a, b].map((v) => SEMVER.exec(v)!.slice(1).map(Number));
  for (let i = 0; i < 3; i++) if (x![i] !== y![i]) return x![i]! - y![i]!;
  return 0;
}

/**
 * The newest version tagged in the history of `dir`'s current commit, or `null` with none. A tag
 * on another branch's commits does not count, and neither does one that is not `mobile-v1.2.3`.
 */
export function taggedVersion(dir: string): string | null {
  const tags = execFileSync("git", ["tag", "--list", `${TAG_PREFIX}*`, "--merged", "HEAD"], {
    cwd: dir,
    encoding: "utf8",
  });
  const versions = tags
    .split("\n")
    .map((tag) => tag.trim().slice(TAG_PREFIX.length))
    .filter((version) => SEMVER.test(version))
    .sort(compare);
  return versions.at(-1) ?? null;
}

/** The version to build: the tagged one, or `first` before any tag, raised by `part`. */
export function nextVersion(dir: string, part: Part, first: string): string {
  return bump(taggedVersion(dir) ?? first, part);
}

if (import.meta.main) {
  const part = (process.argv[2] ?? "none") as Part;
  if (!["none", "patch", "minor", "major"].includes(part)) {
    console.error("usage: bun scripts/version.ts [none|patch|minor|major]");
    process.exit(2);
  }
  const first = (
    JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")) as { version: string }
  ).version;
  console.log(nextVersion(ROOT, part, first));
}
