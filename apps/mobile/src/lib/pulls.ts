// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Pull requests, as the computer last read them from its forge (45). The phone asks the
 * computer and never the forge, so it shows what the computer's own window shows and is as old.
 * Nothing of them is kept on the phone: they show while the computer can be reached.
 *
 * What is here is the reading of them, and their words: which colour a badge is, what a line of
 * checks says. The desktop's sidebar is the model for both.
 */

import type {
  Channel,
  Cursor,
  PullDetail,
  PullsOf,
  PullSummary,
  WorkspaceId,
} from "@yardsort/remote-client";

/** The operations a computer says it answers when it reads pull requests. */
export const PULL_LIST = "pull.list";

/** Whether this computer tells of pull requests at all. One from before them, or with no window, does not. */
export function readsPulls(channel: Channel | null): channel is Channel {
  return !!channel?.host.capabilities.includes(PULL_LIST);
}

/** Pages asked for at most in one reading: more pull requests than a phone's list is for. */
const MAX_PAGES = 10;

export interface PullsRead {
  pulls: PullSummary[];
  /** How many open ones the computer has read, over every project this phone may see. */
  open: number;
  /** The computer is reading them from the forge now: what is here is from before. */
  reading: boolean;
}

/** One pull request among a computer's: its project and number, or the following of it. */
export function keyOf(pull: Pick<PullSummary, "project" | "number" | "followed">): string {
  return pull.followed ? `${pull.project} f ${pull.followed}` : `${pull.project} n ${pull.number}`;
}

/** Every page of a computer's pull requests of one kind, in the computer's order. */
export async function readPulls(channel: Channel, of: PullsOf): Promise<PullsRead> {
  const pulls: PullSummary[] = [];
  const seen = new Set<string>();
  let cursor: Cursor | null = null;
  let open = 0;
  let reading = false;
  for (let page = 0; page < MAX_PAGES; page++) {
    const got = await channel.pulls({ of, cursor });
    for (const pull of got.pulls) {
      // The list can change between two pages; one told of twice is shown once.
      const key = keyOf(pull);
      if (seen.has(key)) continue;
      seen.add(key);
      pulls.push(pull);
    }
    open = got.open;
    reading ||= got.reading;
    if (!got.next) break;
    cursor = got.next;
  }
  return { pulls, open, reading };
}

/**
 * Each workspace's pull requests, in the computer's order: its own before what it follows in
 * other repositories, and of its own the one the computer shows first leading.
 */
export function byWorkspace(pulls: readonly PullSummary[]): Map<WorkspaceId, PullSummary[]> {
  const found = new Map<WorkspaceId, PullSummary[]>();
  for (const pull of pulls) {
    for (const workspace of pull.workspaces) {
      const held = found.get(workspace);
      if (held) held.push(pull);
      else found.set(workspace, [pull]);
    }
  }
  return found;
}

/**
 * What a pull request's colour says, as on the desktop: green for one open with nothing pending
 * or wrong, yellow for a draft or one waiting on checks or a review, red for a failed check,
 * changes asked for or conflicts, purple for merged, and closed for one closed without merging.
 */
export type PullTone = "open" | "waiting" | "failing" | "merged" | "closed";

export function toneOf(pull: PullSummary): PullTone {
  if (pull.state === "merged") return "merged";
  if (pull.state === "closed") return "closed";
  if (pull.checks.failed > 0 || pull.review === "changesRequested" || pull.conflicts) {
    return "failing";
  }
  if (pull.draft || pull.checks.running > 0 || pull.review === "required") return "waiting";
  // A state from a newer computer is not known to be fine.
  return pull.state === "open" ? "open" : "waiting";
}

/** How it is named in a line: `#42`, or `lib#7` for one in another repository. */
export function refOf(pull: Pick<PullSummary, "number" | "repo">): string {
  const repo = pull.repo?.split("/").pop();
  return repo ? `${repo}#${pull.number}` : `#${pull.number}`;
}

/**
 * What a workspace row's badge says: its first pull request, and how many more it has. A merged
 * or closed one says so in place of its number, since that is all that is left to know of it.
 */
export function badgeWords(first: PullSummary, more: number): string {
  const tone = toneOf(first);
  const name = tone === "merged" ? "merged" : tone === "closed" ? "closed" : refOf(first);
  return more > 0 ? `${name} +${more}` : name;
}

/** The state as a card's pill says it. */
export function stateWords(pull: PullSummary): string {
  if (pull.state === "merged") return "Merged";
  if (pull.state === "closed") return "Closed";
  return pull.draft ? "Draft" : "Open";
}

/** What a screen reader is told of a badge: the colour's meaning, in words. */
export function toneWords(pull: PullSummary): string {
  const ref = `pull request ${refOf(pull)}`;
  switch (toneOf(pull)) {
    case "merged":
      return `${ref}, merged`;
    case "closed":
      return `${ref}, closed`;
    case "failing":
      return `${ref}, needs attention`;
    case "waiting":
      return pull.draft ? `${ref}, a draft` : `${ref}, waiting`;
    case "open":
      return `${ref}, open`;
  }
}

export type ChecksVerdict = "none" | "failed" | "running" | "passed";

export function checksVerdict(checks: PullSummary["checks"]): ChecksVerdict {
  if (checks.failed > 0) return "failed";
  if (checks.running > 0) return "running";
  return checks.passed > 0 ? "passed" : "none";
}

/** A card's line of checks: _4 of 6 checks passed, 2 failed_. */
export function checksWords(checks: PullSummary["checks"]): string {
  const total = checks.passed + checks.failed + checks.running;
  if (total === 0) return "No checks reported";
  const rest = [
    checks.failed > 0 ? `${checks.failed} failed` : "",
    checks.running > 0 ? `${checks.running} still running` : "",
  ].filter(Boolean);
  if (rest.length === 0) return total === 1 ? "The one check passed" : `All ${total} checks passed`;
  return `${checks.passed} of ${total} checks passed, ${rest.join(", ")}`;
}

/** A list row's checks, short: _4/6_, or nothing when none reported. */
export function checksShort(checks: PullSummary["checks"]): string | null {
  const total = checks.passed + checks.failed + checks.running;
  return total === 0 ? null : `${checks.passed}/${total}`;
}

/**
 * A card's line about reviews, and whether it is good, bad or waiting. Nothing for a pull
 * request that is no longer open and was never reviewed: there is nothing left to wait for.
 */
export function reviewLine(
  pull: PullSummary,
  detail: Pick<PullDetail, "reviews" | "waitingOn"> | null,
): { words: string; tone: "good" | "bad" | "waiting" | "none" } | null {
  const by = (state: "approved" | "changesRequested") => {
    const names = detail?.reviews.filter((r) => r.state === state).map((r) => r.login) ?? [];
    return names.length > 0 ? ` by ${names.join(", ")}` : "";
  };
  switch (pull.review) {
    case "approved":
      return { words: `Approved${by("approved")}`, tone: "good" };
    case "changesRequested":
      return { words: `Changes requested${by("changesRequested")}`, tone: "bad" };
    case "required": {
      const asked = detail?.waitingOn ?? [];
      return {
        words:
          asked.length > 0 ? `Review required, asked of ${asked.join(", ")}` : "Review required",
        tone: "waiting",
      };
    }
    default:
      return pull.state === "open" ? { words: "No reviews yet", tone: "none" } : null;
  }
}

/** Where one is, and how it stands, in a few words under its title: _orchard/lib · checks running_. */
export function standingWords(pull: PullSummary): string {
  const stands =
    pull.state === "merged"
      ? "merged"
      : pull.state === "closed"
        ? "closed"
        : {
            failed: "checks failed",
            running: "checks running",
            passed: "checks passed",
            none: pull.draft ? "a draft" : "no checks",
          }[checksVerdict(pull.checks)];
  return [pull.repo, stands].filter(Boolean).join(" · ");
}

/** What names a pull request to a screen that shows it: the route's parameters. */
export function routeOf(pull: Pick<PullSummary, "project" | "number" | "followed">): {
  project: string;
  number: string;
  followed?: string;
} {
  return {
    project: pull.project,
    number: String(pull.number),
    ...(pull.followed ? { followed: pull.followed } : {}),
  };
}

/** How long ago, short: _5 min_, _2 h_, _3 d_. */
export function ago(ms: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - ms) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h`;
  return `${Math.floor(hours / 24)} d`;
}

/** A card's corner: _updated 5 min ago_, _merged 2 d ago_. */
export function changedWords(pull: PullSummary, now: number): string | null {
  if (pull.updatedAt === undefined) return null;
  const what = pull.state === "merged" ? "merged" : pull.state === "closed" ? "closed" : "updated";
  const when = ago(pull.updatedAt, now);
  return when === "now" ? `${what} just now` : `${what} ${when} ago`;
}

/** What the button that leaves for the forge says: its name when it is one a person knows. */
export function openWords(url: string): string {
  const host = /^https:\/\/([^/?#]+)/.exec(url)?.[1] ?? "";
  if (host === "github.com" || host.endsWith(".github.com")) return "Open on GitHub";
  return "Open in the browser";
}
