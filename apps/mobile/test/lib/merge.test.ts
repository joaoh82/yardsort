// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";
import {
  type Channel,
  ClientError,
  type PublishState,
  type PullSummary,
  type WorkspaceId,
} from "@yardsort/remote-client";

import {
  checksWarning,
  mergeButtonWords,
  mergedWords,
  mergeFailure,
  mergeOffer,
  mergesPulls,
  methodWords,
} from "../../src/lib/merge";

const HEAD = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";

function pull(over: Partial<PullSummary> = {}): PullSummary {
  return {
    project: "project-1",
    number: 61,
    title: "Retry uploads with backoff",
    url: "https://github.com/orchard/orchard/pull/61",
    branch: "ys/flaky-upload-test",
    base: "main",
    state: "open",
    draft: false,
    checks: { passed: 6, failed: 0, running: 0 },
    review: "approved",
    conflicts: false,
    workspaces: ["workspace-1"],
    ...over,
  } as PullSummary;
}

function publish(over: Partial<PublishState> = {}): PublishState {
  return {
    branch: "ys/flaky-upload-test",
    base: "main",
    ahead: 0,
    gh: true,
    pullRequest: {
      number: 61,
      url: "https://github.com/orchard/orchard/pull/61",
      title: "Retry uploads with backoff",
      state: "open",
      checks: "passed",
      headOid: HEAD,
    },
    ...over,
  } as PublishState;
}

function channel(capabilities: string[]): Channel {
  return { host: { capabilities } } as unknown as Channel;
}

describe("whether the computer merges", () => {
  test("only one that answers both the merge and where a workspace stands", () => {
    expect(mergesPulls(channel(["pull.list", "workspace.publish", "pull_request.merge"]))).toBe(
      true,
    );
    expect(mergesPulls(channel(["pull.list", "pull_request.merge"]))).toBe(false);
    expect(mergesPulls(channel(["pull.list"]))).toBe(false);
    expect(mergesPulls(null)).toBe(false);
  });
});

describe("what the card offers", () => {
  test("a workspace's own, open and ready, at the commit the computer shows", () => {
    expect(mergeOffer(pull(), publish())).toEqual({
      kind: "ready",
      workspace: "workspace-1" as WorkspaceId,
      head: HEAD,
      base: "main",
    });
  });

  test("nothing before the computer said where the workspace stands", () => {
    expect(mergeOffer(pull(), null)).toEqual({ kind: "none" });
  });

  test("nothing for one merged or closed", () => {
    expect(mergeOffer(pull({ state: "merged" }), publish())).toEqual({ kind: "none" });
    expect(mergeOffer(pull({ state: "closed" }), publish())).toEqual({ kind: "none" });
  });

  test("one no workspace opened is merged on the forge", () => {
    const offer = mergeOffer(pull({ workspaces: [] }), publish());
    expect(offer).toEqual({
      kind: "note",
      why: "No workspace on this computer opened it. Merge it on the forge.",
    });
    expect(mergeOffer(pull({ followed: "f-1" }), publish()).kind).toBe("note");
  });

  test("a draft says so", () => {
    const draft = publish();
    draft.pullRequest = { ...draft.pullRequest!, draft: true };
    expect(mergeOffer(pull(), draft)).toEqual({
      kind: "note",
      why: "It is a draft. Mark it ready for review on the forge first.",
    });
  });

  test("a computer with no gh says to merge it on the forge", () => {
    expect(mergeOffer(pull(), publish({ gh: undefined }))).toEqual({
      kind: "note",
      why: "That computer has no program to merge a pull request with. Merge it on the forge.",
    });
  });

  test("conflicts show the button and say why it cannot be pressed", () => {
    const why = "Conflicts with main. Resolve them in the workspace first.";
    // The list the computer read earlier is not the last word: the forge's answer just now is.
    expect(mergeOffer(pull({ conflicts: true, draft: true }), publish()).kind).toBe("ready");
    const conflicting = publish();
    conflicting.pullRequest = { ...conflicting.pullRequest!, conflicting: true };
    expect(mergeOffer(pull(), conflicting)).toEqual({ kind: "blocked", why });
  });

  test("nothing when the workspace's pull request is another, or names no commit", () => {
    const other = publish();
    other.pullRequest = { ...other.pullRequest!, number: 62 };
    expect(mergeOffer(pull(), other)).toEqual({ kind: "none" });
    const noHead = publish();
    noHead.pullRequest = { ...noHead.pullRequest!, headOid: undefined };
    expect(mergeOffer(pull(), noHead)).toEqual({ kind: "none" });
    expect(mergeOffer(pull(), publish({ pullRequest: undefined }))).toEqual({ kind: "none" });
  });
});

describe("the sheet's words", () => {
  test("the button names the method", () => {
    expect(mergeButtonWords("squash")).toBe("Squash and merge");
    expect(mergeButtonWords("merge")).toBe("Merge");
    expect(mergeButtonWords("rebase")).toBe("Rebase and merge");
  });

  test("each method says what it leaves on the base", () => {
    expect(methodWords("squash", "main")).toBe(
      "One commit on main, with the pull request's title.",
    );
    expect(methodWords("merge", undefined)).toBe(
      "Every commit, and a merge commit on its base branch.",
    );
  });

  test("checks that did not all pass warn, and do not stop it", () => {
    expect(checksWarning({ passed: 6, failed: 0, running: 0 })).toBeNull();
    expect(checksWarning({ passed: 5, failed: 0, running: 1 })).toBe(
      "1 check is still running. The forge decides whether it may merge.",
    );
    expect(checksWarning({ passed: 3, failed: 2, running: 1 })).toBe(
      "2 checks failed, and 1 check is still running. The forge decides whether it may merge.",
    );
  });

  test("once merged, how, where and at which commit, and that the branch is kept", () => {
    expect(mergedWords("squash", "main", HEAD, "ys/flaky-upload-test")).toBe(
      "Squashed into main at a1b2c3d from this phone. The branch ys/flaky-upload-test is kept.",
    );
  });
});

describe("a merge that did not come back done", () => {
  test("a refusal is the computer's own sentence", () => {
    const said =
      "The pull request changed or is no longer ready to merge. Read it again and look before merging.";
    const error = ClientError.from({ code: "conflict", message: said } as never);
    expect(mergeFailure(error)).toEqual({ kind: "refused", words: said });
  });

  test("a merge that never left the phone says nothing was merged", () => {
    expect(mergeFailure(new ClientError("invalid", "The channel is not open yet."))).toEqual({
      kind: "unsent",
      words: "It was not sent: the connection to the computer closed. Nothing was merged.",
    });
  });

  test("an answer that never came may have been a merge", () => {
    expect(
      mergeFailure(new ClientError("unreachable", "The relay did not answer in time.")),
    ).toEqual({ kind: "lost", words: "It may have been merged. Reading it again." });
    expect(mergeFailure(new Error("gone"))).toEqual({
      kind: "lost",
      words: "It may have been merged. Reading it again.",
    });
  });
});
