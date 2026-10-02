import { describe, expect, it } from "vitest";
import { project, pullRequest, pullRequestsOf, worktree } from "@/test/fixtures";
import { age, checksLabel, checksSummary, moreOpenThanListed, openIn, rowsOf } from "./rows";

const at = (iso: string) => ({ details: { updatedAt: iso } });

describe("rowsOf", () => {
  it("makes one list of every project's pull requests, most recently changed first", () => {
    const alpha = project("alpha");
    const beta = project("beta");
    const rows = rowsOf([alpha, beta], {
      [alpha.id]: pullRequestsOf([
        pullRequest(1, at("2026-09-01T00:00:00Z")),
        pullRequest(2, at("2026-09-03T00:00:00Z")),
      ]),
      [beta.id]: pullRequestsOf([pullRequest(1, at("2026-09-02T00:00:00Z"))], { viewer: "bo" }),
    });
    expect(rows.map((row) => `${row.project.name}#${row.pr.number}`)).toEqual([
      "alpha#2",
      "beta#1",
      "alpha#1",
    ]);
    // Two repositories both have a #1: the key tells them apart.
    expect(new Set(rows.map((row) => row.key)).size).toBe(3);
    // "You" is whoever gh is logged in as on that project's forge.
    expect(rows.map((row) => row.viewer)).toEqual(["ada", "bo", "ada"]);
  });

  it("says which workspace a pull request belongs to, by branch and by the core's word", () => {
    const base = project("alpha");
    const feature = worktree("alpha", "feature");
    const other = worktree("alpha", "other");
    const gone = worktree("alpha", "gone", { missing: true });
    const alpha = { ...base, workspaces: [...base.workspaces, feature, other, gone] };
    const rows = rowsOf([alpha], {
      [alpha.id]: pullRequestsOf(
        [
          pullRequest(7, { branch: "ys/feature" }),
          pullRequest(8, { branch: "someone/else" }),
          pullRequest(9, { branch: "split-off" }),
          pullRequest(10, { branch: "ys/gone" }),
        ],
        // The core matched #9 to `other` from its worktree's history: not its branch's name.
        { workspaces: { [other.id]: [9] } },
      ),
    });
    const owner = (number: number) =>
      rows.find((row) => row.pr.number === number)?.workspace?.name ?? null;
    expect(owner(7)).toBe("feature");
    expect(owner(8)).toBeNull();
    expect(owner(9)).toBe("other");
    expect(owner(10), "a folder that is gone is not somewhere to go").toBeNull();
  });

  it("leaves out a project that is missing or has not answered yet", () => {
    const alpha = project("alpha");
    const lost = project("lost", { missing: true });
    const waiting = project("waiting");
    const rows = rowsOf([alpha, lost, waiting], {
      [alpha.id]: pullRequestsOf([pullRequest(1)]),
      [lost.id]: pullRequestsOf([pullRequest(2)]),
    });
    expect(rows.map((row) => row.project.name)).toEqual(["alpha"]);
  });
});

describe("what a row says", () => {
  it("counts the open ones, and knows when the forge has more than the list", () => {
    const some = pullRequestsOf([
      pullRequest(1),
      pullRequest(2, { state: "merged" }),
      pullRequest(3, { draft: true }),
    ]);
    expect(openIn(some)).toBe(2);
    expect(moreOpenThanListed(some)).toBe(false);
    expect(moreOpenThanListed({ ...some, openTotal: 1394 })).toBe(true);
    // Before the open ones have been read there is no total to compare with.
    expect(moreOpenThanListed({ ...some, openTotal: null })).toBe(false);
    expect(openIn(undefined)).toBe(0);
  });

  it("sums the checks up as passed out of all, and in words for a screen reader", () => {
    const mixed = pullRequest(1, {
      details: { checkCounts: { passed: 9, failed: 1, running: 2 } },
    });
    expect(checksSummary(mixed)).toBe("9/12");
    expect(checksLabel(mixed)).toBe("9 of 12 checks passed, 1 failed, 2 still running");
    const none = pullRequest(2, { details: { checkCounts: { passed: 0, failed: 0, running: 0 } } });
    expect(checksSummary(none)).toBe("");
    expect(checksLabel(none)).toBe("no checks reported");
    expect(checksSummary({ ...none, details: null })).toBe("");
  });

  it("gives an age in the shortest form that is still clear", () => {
    const now = Date.parse("2026-10-02T12:00:00Z");
    const ago = (ms: number) => age(now - ms, now);
    const minute = 60_000;
    const day = 24 * 60 * minute;
    expect(ago(20_000)).toBe("now");
    expect(ago(5 * minute)).toBe("5m");
    expect(ago(3 * 60 * minute)).toBe("3h");
    expect(ago(2 * day)).toBe("2d");
    expect(ago(21 * day)).toBe("3w");
    expect(ago(90 * day)).toBe("3mo");
    expect(ago(800 * day)).toBe("2y");
    expect(age(null, now)).toBe("");
    // A clock that disagrees with the forge's must not print a negative age.
    expect(age(now + 5 * minute, now)).toBe("now");
  });
});
