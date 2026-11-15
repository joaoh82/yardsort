// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";
import type {
  Channel,
  Cursor,
  PullListParams,
  PullPage,
  PullSummary,
} from "@yardsort/remote-client";

import {
  ago,
  badgeWords,
  byWorkspace,
  changedWords,
  checksShort,
  checksWords,
  keyOf,
  openWords,
  readPulls,
  refOf,
  reviewLine,
  routeOf,
  standingWords,
  stateWords,
  toneOf,
  toneWords,
} from "../../src/lib/pulls";

const NOW = 1_800_000_000_000;

function pull(over: Partial<PullSummary> = {}): PullSummary {
  return {
    project: "project-1",
    number: 42,
    title: "Add an --imperial option",
    url: "https://github.com/orchard/orchard/pull/42",
    branch: "ys/add-imperial-option",
    base: "main",
    state: "open",
    draft: false,
    checks: { passed: 12, failed: 0, running: 0 },
    review: "approved",
    conflicts: false,
    additions: 96,
    deletions: 14,
    author: "juno-p",
    updatedAt: NOW - 12 * 60_000,
    workspaces: ["workspace-1"],
    ...over,
  } as PullSummary;
}

describe("what a badge's colour says", () => {
  test("green only for one open with nothing pending or wrong", () => {
    expect(toneOf(pull())).toBe("open");
    expect(toneOf(pull({ review: "none", checks: { passed: 0, failed: 0, running: 0 } }))).toBe(
      "open",
    );
  });

  test("yellow for a draft, a check still running or a review still required", () => {
    expect(toneOf(pull({ draft: true }))).toBe("waiting");
    expect(toneOf(pull({ checks: { passed: 9, failed: 0, running: 3 } }))).toBe("waiting");
    expect(toneOf(pull({ review: "required" }))).toBe("waiting");
  });

  test("red for a failed check, changes asked for or conflicts, whatever else is so", () => {
    expect(toneOf(pull({ checks: { passed: 4, failed: 2, running: 1 } }))).toBe("failing");
    expect(toneOf(pull({ review: "changesRequested" }))).toBe("failing");
    expect(toneOf(pull({ conflicts: true, draft: true }))).toBe("failing");
  });

  test("merged and closed outrank what its checks said when it was open", () => {
    const red = { checks: { passed: 0, failed: 3, running: 0 }, conflicts: true };
    expect(toneOf(pull({ ...red, state: "merged" }))).toBe("merged");
    expect(toneOf(pull({ ...red, state: "closed" }))).toBe("closed");
  });

  test("a state from a newer computer is not taken to be fine", () => {
    expect(toneOf(pull({ state: "other" }))).toBe("waiting");
  });

  test("a screen reader is told what the colour means", () => {
    expect(toneWords(pull())).toBe("pull request #42, open");
    expect(toneWords(pull({ draft: true }))).toBe("pull request #42, a draft");
    expect(toneWords(pull({ review: "required" }))).toBe("pull request #42, waiting");
    expect(toneWords(pull({ conflicts: true }))).toBe("pull request #42, needs attention");
    expect(toneWords(pull({ state: "merged" }))).toBe("pull request #42, merged");
    expect(toneWords(pull({ state: "closed", repo: "orchard/lib" }))).toBe(
      "pull request lib#42, closed",
    );
  });
});

describe("what a pull request is called", () => {
  test("its number, with its repository's name before it when that is another", () => {
    expect(refOf(pull())).toBe("#42");
    expect(refOf(pull({ repo: "orchard/lib", number: 7 }))).toBe("lib#7");
  });

  test("a row's badge says merged or closed in place of the number, and how many more there are", () => {
    expect(badgeWords(pull(), 0)).toBe("#42");
    expect(badgeWords(pull(), 2)).toBe("#42 +2");
    expect(badgeWords(pull({ state: "merged" }), 0)).toBe("merged");
    expect(badgeWords(pull({ state: "closed" }), 1)).toBe("closed +1");
  });

  test("a card's pill says its state, and a draft is a state of an open one", () => {
    expect(stateWords(pull())).toBe("Open");
    expect(stateWords(pull({ draft: true }))).toBe("Draft");
    expect(stateWords(pull({ state: "merged", draft: true }))).toBe("Merged");
    expect(stateWords(pull({ state: "closed" }))).toBe("Closed");
  });

  test("one followed elsewhere is another pull request than the project's own of that number", () => {
    const own = pull({ number: 7 });
    const elsewhere = pull({ number: 7, followed: "follow-1", repo: "orchard/lib" });
    expect(keyOf(own)).not.toBe(keyOf(elsewhere));
    expect(routeOf(own)).toEqual({ project: "project-1", number: "7" });
    expect(routeOf(elsewhere)).toEqual({
      project: "project-1",
      number: "7",
      followed: "follow-1",
    });
  });
});

describe("a line of checks", () => {
  test("says how they divide, and nothing of what there is none of", () => {
    expect(checksWords({ passed: 12, failed: 0, running: 0 })).toBe("All 12 checks passed");
    expect(checksWords({ passed: 1, failed: 0, running: 0 })).toBe("The one check passed");
    expect(checksWords({ passed: 4, failed: 2, running: 0 })).toBe(
      "4 of 6 checks passed, 2 failed",
    );
    expect(checksWords({ passed: 9, failed: 0, running: 3 })).toBe(
      "9 of 12 checks passed, 3 still running",
    );
    expect(checksWords({ passed: 1, failed: 1, running: 1 })).toBe(
      "1 of 3 checks passed, 1 failed, 1 still running",
    );
    expect(checksWords({ passed: 0, failed: 0, running: 0 })).toBe("No checks reported");
  });

  test("short, for a list", () => {
    expect(checksShort({ passed: 4, failed: 2, running: 0 })).toBe("4/6");
    expect(checksShort({ passed: 0, failed: 0, running: 0 })).toBeNull();
  });

  test("under a title: where it is and how it stands", () => {
    expect(
      standingWords(pull({ repo: "orchard/lib", checks: { passed: 2, failed: 0, running: 3 } })),
    ).toBe("orchard/lib · checks running");
    expect(standingWords(pull({ checks: { passed: 2, failed: 1, running: 3 } }))).toBe(
      "checks failed",
    );
    expect(standingWords(pull())).toBe("checks passed");
    expect(standingWords(pull({ state: "merged" }))).toBe("merged");
    expect(standingWords(pull({ draft: true, checks: { passed: 0, failed: 0, running: 0 } }))).toBe(
      "a draft",
    );
  });
});

describe("a line about reviews", () => {
  const reviews = [
    { login: "mira-k", state: "changesRequested" as const },
    { login: "tomas-r", state: "approved" as const },
    { login: "lee", state: "commented" as const },
  ];

  test("names who approved or asked for changes, once the computer has said who", () => {
    expect(reviewLine(pull(), { reviews, waitingOn: [] })).toEqual({
      words: "Approved by tomas-r",
      tone: "good",
    });
    expect(reviewLine(pull({ review: "changesRequested" }), { reviews, waitingOn: [] })).toEqual({
      words: "Changes requested by mira-k",
      tone: "bad",
    });
    // The list says only the verdict.
    expect(reviewLine(pull(), null)).toEqual({ words: "Approved", tone: "good" });
  });

  test("says who a required review waits on", () => {
    expect(reviewLine(pull({ review: "required" }), null)).toEqual({
      words: "Review required",
      tone: "waiting",
    });
    expect(
      reviewLine(pull({ review: "required" }), { reviews: [], waitingOn: ["mira-k", "core"] }),
    ).toEqual({ words: "Review required, asked of mira-k, core", tone: "waiting" });
  });

  test("an open one nobody reviewed says so, and one no longer open says nothing", () => {
    expect(reviewLine(pull({ review: "none" }), null)).toEqual({
      words: "No reviews yet",
      tone: "none",
    });
    expect(reviewLine(pull({ review: "none", state: "closed" }), null)).toBeNull();
    expect(reviewLine(pull({ review: "none", state: "merged" }), null)).toBeNull();
  });
});

describe("how long ago", () => {
  test("in the largest whole unit", () => {
    expect(ago(NOW - 20_000, NOW)).toBe("now");
    expect(ago(NOW - 5 * 60_000, NOW)).toBe("5 min");
    expect(ago(NOW - 2 * 3_600_000, NOW)).toBe("2 h");
    expect(ago(NOW - 3 * 86_400_000, NOW)).toBe("3 d");
    // A computer's clock a little ahead of the phone's is not the future.
    expect(ago(NOW + 60_000, NOW)).toBe("now");
  });

  test("a card says what happened then", () => {
    expect(changedWords(pull(), NOW)).toBe("updated 12 min ago");
    expect(changedWords(pull({ state: "merged", updatedAt: NOW - 2 * 86_400_000 }), NOW)).toBe(
      "merged 2 d ago",
    );
    expect(changedWords(pull({ state: "closed", updatedAt: NOW - 1_000 }), NOW)).toBe(
      "closed just now",
    );
    const { updatedAt: _, ...unknown } = pull();
    expect(changedWords(unknown as PullSummary, NOW)).toBeNull();
  });
});

test("the way out names the forge when it is one a person knows", () => {
  expect(openWords("https://github.com/orchard/orchard/pull/42")).toBe("Open on GitHub");
  expect(openWords("https://git.example.com/orchard/orchard/pull/42")).toBe("Open in the browser");
  // A name that only looks like it is not it.
  expect(openWords("https://github.com.example.net/x/y/pull/1")).toBe("Open in the browser");
});

test("each workspace's pull requests are its own in the computer's order, and one of two workspaces is in both", () => {
  const first = pull({ number: 88, workspaces: ["w1" as never] });
  const second = pull({ number: 80, workspaces: ["w1" as never, "w2" as never] });
  const elsewhere = pull({
    number: 7,
    followed: "f",
    repo: "orchard/lib",
    workspaces: ["w1" as never],
  });
  const nobodys = pull({ number: 90, workspaces: [] });
  const found = byWorkspace([first, second, elsewhere, nobodys]);
  expect(found.get("w1" as never)?.map((p) => refOf(p))).toEqual(["#88", "#80", "lib#7"]);
  expect(found.get("w2" as never)?.map((p) => refOf(p))).toEqual(["#80"]);
  expect(found.size).toBe(2);
});

describe("reading a computer's pull requests", () => {
  /** A channel that answers `pull.list` with these pages, in turn. */
  function answering(pages: PullPage[]): { channel: Channel; asked: PullListParams[] } {
    const asked: PullListParams[] = [];
    const channel = {
      pulls: async (params: Partial<PullListParams>) => {
        asked.push({ of: "workspaces", cursor: null, limit: null, ...params });
        const page = pages[asked.length - 1];
        if (!page) throw new Error("asked for a page there is none of");
        return page;
      },
    } as unknown as Channel;
    return { channel, asked };
  }
  const cursor = (text: string) => text as Cursor;

  test("every page is read, in order, and one told of on two pages is kept once", async () => {
    const { channel, asked } = answering([
      {
        pulls: [pull({ number: 3 }), pull({ number: 2 })],
        next: cursor("k1.a"),
        open: 3,
        reading: false,
      },
      { pulls: [pull({ number: 2 }), pull({ number: 1 })], next: null, open: 3, reading: true },
    ]);
    const read = await readPulls(channel, "open");
    expect(read.pulls.map((p) => p.number)).toEqual([3, 2, 1]);
    expect(read.open).toBe(3);
    // Any page saying so is the computer reading.
    expect(read.reading).toBe(true);
    expect(asked).toEqual([
      { of: "open", cursor: null, limit: null },
      { of: "open", cursor: cursor("k1.a"), limit: null },
    ]);
  });

  test("a list that never ends is not read for ever", async () => {
    const endless: PullPage = {
      pulls: [pull()],
      next: cursor("k1.again"),
      open: 1,
      reading: false,
    };
    const { channel, asked } = answering(Array.from({ length: 50 }, () => endless));
    const read = await readPulls(channel, "workspaces");
    expect(asked).toHaveLength(10);
    expect(read.pulls).toHaveLength(1);
  });
});
