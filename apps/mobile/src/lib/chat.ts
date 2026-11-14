// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A conversation as Chat shows it (43): its turns, read on the computer from the agent's own record
 * (`conversation.read`), and its steps and state, from the activity the computer records. Nothing
 * is read from the terminal, and nothing is kept: it lives in memory while the screen is open.
 *
 * The computer sends what was said, not when it changes, so this reads again when the
 * conversation's activity says something happened, and every few seconds while its agent works.
 */

import {
  type ActivityEvent,
  type Ask,
  type Channel,
  ChannelClosed,
  type ConversationTurn,
  type SessionRecordId,
  type WorkspaceId,
} from "@yardsort/remote-client";

import { messageOf } from "./accounts";

/** Harnesses whose conversation the computer can read, as `ys workspace context` lists them. */
export const READABLE = ["claude", "codex", "cursor", "opencode", "grok", "omp", "pi"];

/** The operation a computer must answer for Chat to be offered. */
export const CONVERSATION_READ = "conversation.read";

/** How much earlier to ask for at a time, in turns. */
const EARLIER = 10;

export interface Chat {
  /** Oldest first, numbered without gaps. */
  turns: readonly ConversationTurn[];
  /** How many the conversation has. */
  total: number;
  asks: readonly Ask[];
  answersAsks: boolean;
  /** Why the computer could not read it, when it could not. */
  unreadable: string | null;
  loaded: boolean;
  problem: string | null;
  /** Earlier turns are on the computer, not here yet. */
  hasEarlier: boolean;
}

const EMPTY: Chat = {
  turns: [],
  total: 0,
  asks: [],
  answersAsks: false,
  unreadable: null,
  loaded: false,
  problem: null,
  hasEarlier: false,
};

export class ConversationFeed {
  #chat: Chat = EMPTY;
  #listeners = new Set<() => void>();
  /** One read at a time; one more is remembered when asked for meanwhile. */
  #reading: Promise<void> | null = null;
  #again = false;

  constructor(
    readonly workspace: WorkspaceId,
    readonly session: SessionRecordId,
  ) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): Chat => this.#chat;

  /** Read what is new: the newest turns the first time, then from the last one held on. */
  async read(channel: Channel): Promise<void> {
    if (this.#reading) {
      this.#again = true;
      return this.#reading;
    }
    this.#reading = (async () => {
      do {
        this.#again = false;
        await this.#readOnce(channel);
      } while (this.#again);
    })().finally(() => {
      this.#reading = null;
    });
    return this.#reading;
  }

  /** Read the turns before the first one held. */
  async earlier(channel: Channel): Promise<void> {
    const first = this.#chat.turns[0];
    if (!first || first.n <= 1) return;
    try {
      let from = Math.max(1, first.n - EARLIER);
      let found: ConversationTurn[] = [];
      // A page may hold fewer than asked for: carry on to where the held ones start.
      for (let tries = 0; tries < EARLIER && from < first.n; tries++) {
        const page = await channel.conversation({
          workspace: this.workspace,
          session: this.session,
          from,
        });
        const before = page.turns.filter((turn) => turn.n < first.n);
        if (before.length === 0) break;
        found = [...found, ...before];
        from = (before.at(-1) as ConversationTurn).n + 1;
      }
      const turns = joined(found, this.#chat.turns);
      this.#update({ turns, hasEarlier: (turns[0]?.n ?? 1) > 1, problem: null });
    } catch (error) {
      this.#update({ problem: words(error) });
    }
  }

  /** Forget an answered question at once, rather than at the next read. */
  answered(ask: string): void {
    this.#update({ asks: this.#chat.asks.filter((a) => a.id !== ask) });
  }

  async #readOnce(channel: Channel): Promise<void> {
    const held = this.#chat.turns;
    const last = held.at(-1);
    try {
      const page = await channel.conversation({
        workspace: this.workspace,
        session: this.session,
        ...(last ? { from: last.n } : {}),
      });
      // A conversation that went back (cleared, or rewound) is read again from its newest.
      const rewound = last !== undefined && page.total < last.n;
      const fresh = rewound
        ? await channel.conversation({ workspace: this.workspace, session: this.session })
        : page;
      const turns = rewound || !last ? fresh.turns : joined(held, fresh.turns);
      this.#update({
        turns,
        total: fresh.total,
        asks: fresh.asks,
        answersAsks: fresh.answersAsks,
        unreadable: fresh.unreadable ?? null,
        loaded: true,
        problem: null,
        hasEarlier: (turns[0]?.n ?? 1) > 1,
      });
    } catch (error) {
      this.#update({ loaded: this.#chat.loaded, problem: words(error) });
    }
  }

  #update(change: Partial<Chat>): void {
    this.#chat = { ...this.#chat, ...change };
    for (const listener of this.#listeners) listener();
  }
}

function words(error: unknown): string {
  return error instanceof ChannelClosed
    ? "The computer could not be reached. What is on it keeps running."
    : messageOf(error);
}

/** `held` with `newer` laid over it: a turn in both is the newer one's. */
export function joined(
  held: readonly ConversationTurn[],
  newer: readonly ConversationTurn[],
): ConversationTurn[] {
  const byNumber = new Map<number, ConversationTurn>();
  for (const turn of held) byNumber.set(turn.n, turn);
  for (const turn of newer) byNumber.set(turn.n, turn);
  return [...byNumber.values()].sort((a, b) => a.n - b.n);
}

// --- steps and state, from activity ------------------------------------------------------------

/** One thing the agent did, as its hooks reported it. */
export interface Step {
  id: string;
  at: number;
  tool: string;
  /** A file in the workspace, when the step is about one. */
  path: string | null;
  durationMs: number | null;
  state: "running" | "done" | "failed";
}

/** What the agent is doing, as its activity says. `unknown` when it reports nothing. */
export type ChatState =
  | { kind: "working"; since: number }
  | { kind: "needsYou"; since: number }
  | { kind: "done"; at: number }
  | { kind: "failed"; at: number }
  | { kind: "ended"; at: number }
  | { kind: "unknown" };

const WORKING = new Set([
  "prompt.submitted",
  "tool.started",
  "tool.completed",
  "tool.failed",
  "approval.resolved",
  "agent.subagent_started",
  "agent.subagent_stopped",
  "session.compacted",
]);

/** The conversation's state, from its newest event that says one. Open questions come first. */
export function stateOf(events: readonly ActivityEvent[], asks: readonly Ask[]): ChatState {
  const first = asks[0];
  if (first) return { kind: "needsYou", since: first.askedAt };
  let since: number | null = null;
  for (let i = events.length - 1; i >= 0; i--) {
    const event = events[i] as ActivityEvent;
    const kind = event.kind;
    if (kind === "approval.requested") return { kind: "needsYou", since: event.occurredAt };
    if (kind === "agent.notified" && event.detail.notification === "permission_prompt") {
      return { kind: "needsYou", since: event.occurredAt };
    }
    if (WORKING.has(kind)) {
      since = event.occurredAt;
      if (kind === "prompt.submitted") return { kind: "working", since };
      continue;
    }
    if (since !== null) return { kind: "working", since };
    if (kind === "turn.completed") return { kind: "done", at: event.occurredAt };
    if (kind === "turn.failed") return { kind: "failed", at: event.occurredAt };
    if (kind === "process.exited" || kind === "session.ended") {
      return { kind: "ended", at: event.occurredAt };
    }
  }
  return since !== null ? { kind: "working", since } : { kind: "unknown" };
}

/**
 * The steps of each turn, by the turn's number: the tools its agent ran between the turn's message
 * and the next one's. A tool still running is the last step of the last turn.
 */
export function stepsOf(
  events: readonly ActivityEvent[],
  turns: readonly ConversationTurn[],
): Map<number, Step[]> {
  const steps: Step[] = [];
  let running: Step | null = null;
  for (const event of events) {
    const tool = event.detail.tool;
    if (!tool) continue;
    const path = event.detail.path ?? null;
    if (event.kind === "tool.started") {
      running = {
        id: event.id,
        at: event.occurredAt,
        tool,
        path,
        durationMs: null,
        state: "running",
      };
    } else if (event.kind === "tool.completed" || event.kind === "tool.failed") {
      steps.push({
        id: event.id,
        at: running?.tool === tool ? running.at : event.occurredAt,
        tool,
        path: path ?? (running?.tool === tool ? running.path : null),
        durationMs: event.detail.durationMs ?? null,
        state: event.kind === "tool.completed" ? "done" : "failed",
      });
      running = null;
    }
  }
  if (running) steps.push(running);

  const byTurn = new Map<number, Step[]>();
  for (const step of steps) {
    // The last turn whose message came before the step.
    let owner: ConversationTurn | undefined;
    for (const turn of turns) {
      if (turn.at <= step.at) owner = turn;
      else break;
    }
    if (!owner) continue;
    const list = byTurn.get(owner.n) ?? [];
    list.push(step);
    byTurn.set(owner.n, list);
  }
  return byTurn;
}

/** "read 2 · edit 3 · bash 1": what kinds of step there were, in the order they first came. */
export function stepSummary(steps: readonly Step[]): string {
  const counts = new Map<string, number>();
  for (const step of steps) {
    const name = step.tool.toLowerCase();
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  return [...counts].map(([name, n]) => `${name} ${n}`).join(" · ");
}

/** A span of time, briefly: "9s", "1m 12s", "2h 5m". */
export function span(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
