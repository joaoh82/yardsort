// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A smoke test run waiting for its person, as Chat shows it: the card the computer shows, read from
 * it (`test.waiting`), and the answer (`test.decide`). A file from a repository runs only once a
 * person has seen what it does, so the card says all of it, and one the computer could not send
 * whole can only be refused from here.
 *
 * The computer says what waits, not when that changes, so Chat reads again every few seconds and
 * when the conversation's activity says something happened. Nothing is kept.
 */

import {
  type Channel,
  ChannelClosed,
  ClientError,
  type SessionRecordId,
  type TestDecision,
  type WaitingTest,
  type WorkspaceId,
} from "@yardsort/remote-client";

import { messageOf } from "./accounts";

/** The operation a computer must answer for a waiting run to show. */
export const TEST_WAITING = "test.waiting";

export interface Waiting {
  /** The oldest run waiting in the workspace, when there is one. */
  first: WaitingTest | null;
  /** How many wait behind it. */
  behind: number;
  /** Why the last answer did not go through, when it did not. */
  problem: string | null;
}

const NONE: Waiting = { first: null, behind: 0, problem: null };

export class WaitingTests {
  #waiting: Waiting = NONE;
  #listeners = new Set<() => void>();
  /** One read at a time; one more is remembered when asked for meanwhile. */
  #reading: Promise<void> | null = null;
  #again = false;

  constructor(readonly workspace: WorkspaceId) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): Waiting => this.#waiting;

  /** Ask what waits. A computer from before this answers nothing, and is not asked. */
  async read(channel: Channel): Promise<void> {
    if (!channel.host.capabilities.includes(TEST_WAITING)) return;
    if (this.#reading) {
      this.#again = true;
      return this.#reading;
    }
    this.#reading = (async () => {
      do {
        this.#again = false;
        try {
          const page = await channel.testsWaiting(this.workspace);
          this.#update({ first: page.first ?? null, behind: page.behind });
        } catch {
          // Said by the reach, and asked again in a moment: what was shown stays.
        }
      } while (this.#again);
    })().finally(() => {
      this.#reading = null;
    });
    return this.#reading;
  }

  /**
   * Let `test` run, or refuse it. The card goes once the computer took the answer, or says the
   * run waits no more; anything else may not have reached it, so the card stays to answer again.
   */
  async decide(channel: Channel, test: WaitingTest, decision: TestDecision): Promise<void> {
    this.#update({ problem: null });
    try {
      await channel.decideTest({
        workspace: this.workspace,
        run: test.run,
        contentHash: test.contentHash,
        decision,
      });
      this.#gone(test.run);
    } catch (error) {
      if (error instanceof ClientError && error.code === "not_found") {
        this.#gone(test.run);
      } else {
        this.#update({
          problem:
            error instanceof ChannelClosed
              ? "The answer did not reach the computer. Try again."
              : messageOf(error),
        });
      }
    }
    await this.read(channel);
  }

  #gone(run: string): void {
    if (this.#waiting.first?.run === run) this.#update({ first: null });
  }

  #update(change: Partial<Waiting>): void {
    this.#waiting = { ...this.#waiting, ...change };
    for (const listener of this.#listeners) listener();
  }
}

/** Who asked for the run, for the card: the agent of this conversation, another, or the person. */
export function askedByWords(
  test: Pick<WaitingTest, "askedBy" | "session">,
  conversation: SessionRecordId | null,
  harness: string,
): string {
  switch (test.askedBy) {
    case "agent":
      return test.session !== undefined && test.session === conversation
        ? harness
        : "another agent";
    case "workflow":
      return "a workflow";
    case "app":
      return "you, at the computer";
    default:
      return "the computer";
  }
}

export const PHASES = ["setup", "steps", "teardown"] as const;

export const PHASE_WORDS: Record<(typeof PHASES)[number], string> = {
  setup: "Setup",
  steps: "Steps",
  teardown: "Teardown",
};

/** One line of a two-version comparison: kept, added or removed. */
export interface DiffLine {
  kind: "same" | "added" | "removed";
  text: string;
}

/**
 * The lines of `after` against `before`, for "what changed": the same comparison the computer's
 * card draws. Lines alike at the start and the end are matched off, and the middle compared by its
 * longest common run of lines. A card carries files of at most a few hundred lines, so the middle
 * is small.
 */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  if (a.at(-1) === "") a.pop();
  if (b.at(-1) === "") b.pop();

  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail++;
  }
  const am = a.slice(head, a.length - tail);
  const bm = b.slice(head, b.length - tail);

  // lcs[i][j]: the longest common run of lines of am[i..] and bm[j..].
  const lcs: number[][] = Array.from({ length: am.length + 1 }, () =>
    new Array<number>(bm.length + 1).fill(0),
  );
  for (let i = am.length - 1; i >= 0; i--) {
    for (let j = bm.length - 1; j >= 0; j--) {
      (lcs[i] as number[])[j] =
        am[i] === bm[j]
          ? ((lcs[i + 1] as number[])[j + 1] as number) + 1
          : Math.max((lcs[i + 1] as number[])[j] as number, (lcs[i] as number[])[j + 1] as number);
    }
  }

  const out: DiffLine[] = a.slice(0, head).map((text) => ({ kind: "same", text }));
  let i = 0;
  let j = 0;
  while (i < am.length && j < bm.length) {
    if (am[i] === bm[j]) {
      out.push({ kind: "same", text: am[i] as string });
      i++;
      j++;
    } else if (((lcs[i + 1] as number[])[j] as number) >= ((lcs[i] as number[])[j + 1] as number)) {
      out.push({ kind: "removed", text: am[i++] as string });
    } else {
      out.push({ kind: "added", text: bm[j++] as string });
    }
  }
  while (i < am.length) out.push({ kind: "removed", text: am[i++] as string });
  while (j < bm.length) out.push({ kind: "added", text: bm[j++] as string });
  for (const text of b.slice(b.length - tail)) out.push({ kind: "same", text });
  return out;
}

/** How many lines either side of a change stay in view. */
const CONTEXT = 2;

/**
 * A diff with only its changes and the lines around them: a phone shows the change, not the file.
 * A gap between two kept stretches is one line that says how many were left out.
 */
export function changed(
  lines: readonly DiffLine[],
): (DiffLine | { kind: "gap"; hidden: number })[] {
  const keep = lines.map(() => false);
  lines.forEach((line, at) => {
    if (line.kind === "same") return;
    for (let k = Math.max(0, at - CONTEXT); k <= Math.min(lines.length - 1, at + CONTEXT); k++) {
      keep[k] = true;
    }
  });
  const out: (DiffLine | { kind: "gap"; hidden: number })[] = [];
  let hidden = 0;
  lines.forEach((line, at) => {
    if (keep[at]) {
      if (hidden > 0) out.push({ kind: "gap", hidden });
      hidden = 0;
      out.push(line);
    } else {
      hidden++;
    }
  });
  if (hidden > 0 && out.length > 0) out.push({ kind: "gap", hidden });
  return out;
}
