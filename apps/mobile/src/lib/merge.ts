// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Merging a workspace's pull request from the phone (48). The computer does it, with its own `gh`
 * and sign-in, and only at the commit its person was shown: what is here is when to offer it, the
 * words of the sheet that asks, and what to say of the answer. Nothing is sent again by itself.
 */

import {
  type Channel,
  ChannelClosed,
  ClientError,
  type MergeMethod,
  type PublishState,
  type PullSummary,
  type WorkspaceId,
} from "@yardsort/remote-client";

/** What a computer that can merge says it answers. Protocol minor 14. */
export const MERGE = "pull_request.merge";
/** And where a workspace stands, which names the commit to merge at. */
export const PUBLISH = "workspace.publish";

/** Whether this computer merges a pull request for a device. */
export function mergesPulls(channel: Channel | null): channel is Channel {
  const can = channel?.host.capabilities;
  return !!can && can.includes(MERGE) && can.includes(PUBLISH);
}

/** The workspace a pull request is merged through: the one that opened it, when one did. */
export function mergeWorkspace(pull: PullSummary): WorkspaceId | null {
  if (pull.followed || pull.state !== "open") return null;
  return pull.workspaces[0] ?? null;
}

/**
 * What the card offers. `ready`: a Merge… that opens the sheet. `blocked`: one that cannot be
 * pressed, and why. `note`: no button, and a line saying where it is done instead. `none`: nothing.
 */
export type MergeOffer =
  | { kind: "ready"; workspace: WorkspaceId; head: string; base: string | undefined }
  | { kind: "blocked"; why: string }
  | { kind: "note"; why: string }
  | { kind: "none" };

export function mergeOffer(pull: PullSummary, publish: PublishState | null): MergeOffer {
  if (pull.state !== "open") return { kind: "none" };
  if (pull.followed || pull.workspaces.length === 0) {
    return { kind: "note", why: "No workspace on this computer opened it. Merge it on the forge." };
  }
  // Until the computer said where the workspace stands, there is no commit to name, and nothing
  // is said: a phone let only watch is never told, and is shown nothing.
  if (!publish) return { kind: "none" };
  if (!publish.gh) {
    return {
      kind: "note",
      why: "That computer has no program to merge a pull request with. Merge it on the forge.",
    };
  }
  const own = publish.pullRequest;
  if (!own || own.number !== pull.number || own.state !== "open") return { kind: "none" };
  // The forge's answer just now wins over the list the computer read earlier.
  if (own.draft) {
    return { kind: "note", why: "It is a draft. Mark it ready for review on the forge first." };
  }
  if (!own.headOid) return { kind: "none" };
  if (own.conflicting) {
    return {
      kind: "blocked",
      why: `Conflicts with ${pull.base ?? publish.base ?? "its base branch"}. Resolve them in the workspace first.`,
    };
  }
  const workspace = mergeWorkspace(pull);
  if (!workspace) return { kind: "none" };
  return { kind: "ready", workspace, head: own.headOid, base: pull.base ?? publish.base };
}

/** A commit as a person reads it: its first seven digits. */
export function shortOid(oid: string): string {
  return oid.slice(0, 7);
}

export const METHOD_NAMES: Record<MergeMethod, string> = {
  squash: "Squash",
  merge: "Merge",
  rebase: "Rebase",
};

/** The sheet's button: it names what it does. */
export function mergeButtonWords(method: MergeMethod): string {
  return { squash: "Squash and merge", merge: "Merge", rebase: "Rebase and merge" }[method];
}

/** The line under the method, saying what each leaves on the base branch. */
export function methodWords(method: MergeMethod, base: string | undefined): string {
  const onto = base ?? "its base branch";
  switch (method) {
    case "squash":
      return `One commit on ${onto}, with the pull request's title.`;
    case "merge":
      return `Every commit, and a merge commit on ${onto}.`;
    case "rebase":
      return `Every commit, replayed onto ${onto} without a merge commit.`;
  }
}

/** A warning when the checks did not all pass: it warns and does not stop, the forge decides. */
export function checksWarning(checks: PullSummary["checks"]): string | null {
  const parts = [
    checks.failed > 0 ? `${checks.failed} ${checks.failed === 1 ? "check" : "checks"} failed` : "",
    checks.running > 0
      ? `${checks.running} ${checks.running === 1 ? "check is" : "checks are"} still running`
      : "",
  ].filter(Boolean);
  if (parts.length === 0) return null;
  const said = parts.join(", and ");
  return `${said[0]!.toUpperCase()}${said.slice(1)}. The forge decides whether it may merge.`;
}

/** What the card says once the computer merged it. */
export function mergedWords(
  method: MergeMethod,
  base: string | undefined,
  head: string,
  branch: string,
): string {
  const how = { squash: "Squashed into", merge: "Merged into", rebase: "Rebased onto" }[method];
  return `${how} ${base ?? "its base branch"} at ${shortOid(head)} from this phone. The branch ${branch} is kept.`;
}

/**
 * What a merge that did not come back done means. A refusal is the computer's own sentence, and
 * the card is read again. An answer that never came may still have been a merge: it says so, and
 * nothing is sent again.
 */
export type MergeFailure =
  | { kind: "refused"; words: string }
  | { kind: "lost"; words: string }
  | { kind: "unsent"; words: string };

export function mergeFailure(error: unknown): MergeFailure {
  if (error instanceof ClientError && error.kind === "remote") {
    return { kind: "refused", words: error.message };
  }
  // Refused here, before anything went: the computer was never asked.
  if (
    error instanceof ChannelClosed ||
    (error instanceof ClientError && error.kind === "invalid")
  ) {
    return {
      kind: "unsent",
      words: "It was not sent: the connection to the computer closed. Nothing was merged.",
    };
  }
  return { kind: "lost", words: "It may have been merged. Reading it again." };
}
