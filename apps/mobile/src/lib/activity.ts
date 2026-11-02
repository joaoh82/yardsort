// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A workspace's recent activity, followed live while a channel is open (31 §7), and what each event
 * says in words. It lives in memory, and is never written anywhere (31 §5).
 *
 * A dropped channel loses nothing still kept: following again carries on from the last batch.
 */

import {
  type ActivityEvent,
  type Channel,
  ChannelClosed,
  ClientError,
  type Cursor,
  type EventId,
  type Subscription,
  type WorkspaceId,
} from "@yardsort/remote-client";

import { messageOf } from "./accounts";

/** The most kept in memory. Past this the oldest go, and older pages are not offered. */
const MAX_KEPT = 300;

export interface Feed {
  /** Oldest first, as the computer ordered them. */
  events: readonly ActivityEvent[];
  /** Whether the first page has come. */
  loaded: boolean;
  /** Whether new events are arriving. */
  following: boolean;
  hasOlder: boolean;
  problem: string | null;
}

export class ActivityFeed {
  #feed: Feed = { events: [], loaded: false, following: false, hasOlder: false, problem: null };
  #seen = new Set<EventId>();
  #older: Cursor | null = null;
  #newer: Cursor | null = null;
  #subscription: Subscription | null = null;
  #channel: Channel | null = null;
  /** Goes up with each start and stop: what an earlier one was waiting for no longer counts. */
  #run = 0;
  #listeners = new Set<() => void>();

  constructor(readonly workspace: WorkspaceId) {}

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** The same object until something changes, as `useSyncExternalStore` needs. */
  getSnapshot = (): Feed => this.#feed;

  /** Read and follow through `channel`, carrying on from where this got to. */
  async follow(channel: Channel): Promise<void> {
    this.#run += 1;
    const run = this.#run;
    this.#channel = channel;
    try {
      if (this.#newer) {
        await this.#subscribe(channel, this.#newer, run).catch(async (error: unknown) => {
          // Too far behind for the computer to carry on from there: start again from now.
          if (!(error instanceof ClientError) || run !== this.#run) throw error;
          this.#reset();
          await this.#first(channel, run);
        });
      } else {
        await this.#first(channel, run);
      }
    } catch (error) {
      if (run !== this.#run || error instanceof ChannelClosed) return;
      this.#update({ problem: messageOf(error), following: false, loaded: true });
    }
  }

  /** Stop following. What was read stays, to carry on from. */
  stop(): void {
    this.#run += 1;
    this.#channel = null;
    const subscription = this.#subscription;
    this.#subscription = null;
    if (subscription) void subscription.unsubscribe().catch(() => {});
    if (this.#feed.following) this.#update({ following: false });
  }

  /** Read the page before the oldest shown. */
  async older(): Promise<void> {
    const channel = this.#channel;
    const cursor = this.#older;
    const run = this.#run;
    if (!channel || !cursor) return;
    try {
      const page = await channel.activity({ workspace: this.workspace, cursor });
      if (run !== this.#run) return;
      this.#older = page.older;
      this.#add(page.events, "before");
      this.#update({ hasOlder: this.#older !== null });
    } catch (error) {
      if (run !== this.#run || error instanceof ChannelClosed) return;
      this.#update({ problem: messageOf(error) });
    }
  }

  async #first(channel: Channel, run: number): Promise<void> {
    const page = await channel.activity({ workspace: this.workspace });
    if (run !== this.#run) return;
    this.#older = page.older;
    this.#newer = page.newer;
    this.#add(page.events, "after");
    this.#update({ loaded: true, hasOlder: this.#older !== null });
    await this.#subscribe(channel, page.newer, run);
  }

  async #subscribe(channel: Channel, after: Cursor | null, run: number): Promise<void> {
    const subscription = await channel.subscribe(
      { workspace: this.workspace, ...(after ? { after } : {}) },
      {
        events: (batch) => {
          this.#newer = batch.newer;
          this.#add(batch.events, "after");
        },
        ended: (why) => {
          if (run !== this.#run) return;
          this.#subscription = null;
          this.#update({
            following: false,
            ...(why.kind === "ended" ? { problem: why.error.message } : {}),
          });
        },
      },
    );
    if (run !== this.#run) {
      void subscription.unsubscribe().catch(() => {});
      return;
    }
    this.#subscription = subscription;
    this.#newer = subscription.newer ?? this.#newer;
    this.#update({ following: true, loaded: true, problem: null });
  }

  #add(events: readonly ActivityEvent[], where: "before" | "after"): void {
    const fresh = events.filter((event) => !this.#seen.has(event.id));
    if (fresh.length === 0) return;
    for (const event of fresh) this.#seen.add(event.id);
    let all =
      where === "before" ? [...fresh, ...this.#feed.events] : [...this.#feed.events, ...fresh];
    if (all.length > MAX_KEPT) {
      const dropped = all.slice(0, all.length - MAX_KEPT);
      for (const event of dropped) this.#seen.delete(event.id);
      all = all.slice(all.length - MAX_KEPT);
      this.#older = null;
    }
    this.#update({ events: all, hasOlder: this.#older !== null });
  }

  #reset(): void {
    this.#seen.clear();
    this.#older = null;
    this.#newer = null;
    this.#feed = { ...this.#feed, events: [], hasOlder: false };
  }

  #update(change: Partial<Feed>): void {
    this.#feed = { ...this.#feed, ...change };
    for (const listener of this.#listeners) listener();
  }
}

export interface EventWords {
  title: string;
  detail: string;
  tone: "ok" | "bad" | "plain";
}

/**
 * Whether the agent started last was started without reporting what it does, so that only its start
 * and end are recorded. Reporting is off at the computer until someone turns it on (`capture`), so
 * a quiet list usually means this and not that nothing is happening.
 */
export function reportsOnlyStartAndEnd(events: readonly ActivityEvent[]): boolean {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    if (event?.kind === "process.started" && event.detail.runKind === "harness") {
      return !event.detail.capture;
    }
  }
  return false;
}

/** What an event says, as the desktop says it. A kind this build does not know, by its name. */
export function describeEvent(event: ActivityEvent): EventWords {
  const d = event.detail;
  const who = d.harness ?? d.program ?? (d.runKind === "shell" ? "shell" : "process");
  const tool = () => {
    const name = d.tool ?? null;
    return name ? `${name}${d.subagentType ? ` (${d.subagentType})` : ""}` : "tool";
  };
  const where = d.path ?? (d.pathOutsideWorkspace ? "a file outside the workspace" : "");
  const parts = (...bits: (string | null | undefined)[]) => bits.filter(Boolean).join(" · ");
  const exit = d.exitCode !== undefined ? `exit ${d.exitCode}` : null;

  switch (event.kind) {
    case "process.started": {
      const how =
        d.continuation === "resumed"
          ? "resumed"
          : d.continuation === "forked"
            ? "forked"
            : "started";
      return {
        title: `${who} ${how}`,
        detail: parts(d.model, d.launchedBy === "cli" ? "from ys" : null),
        tone: "plain",
      };
    }
    case "process.exited":
      if (d.exitCode === undefined) {
        return { title: "ended without an exit status", detail: "", tone: "bad" };
      }
      return d.success
        ? { title: "exited", detail: "", tone: "ok" }
        : {
            title: `exited with ${d.exitCode}${d.signal ? ` (${d.signal})` : ""}`,
            detail: "",
            tone: "bad",
          };
    case "process.spawn_failed":
      return { title: `${who} could not start`, detail: d.reason ?? "", tone: "bad" };
    case "session.resumed":
      return { title: "conversation resumed", detail: "", tone: "plain" };
    case "session.forked":
      return { title: "conversation forked", detail: "", tone: "plain" };
    case "session.started":
      return { title: "agent session started", detail: d.model ?? "", tone: "plain" };
    case "session.ended":
      return { title: "agent session ended", detail: d.reason ?? "", tone: "plain" };
    case "session.compacted":
      return { title: "agent compacted its context", detail: d.trigger ?? "", tone: "plain" };
    case "prompt.submitted":
      return {
        title: "prompt submitted",
        detail: d.chars !== undefined ? `${d.chars} characters` : "",
        tone: "plain",
      };
    case "tool.started":
      return { title: `${tool()} started`, detail: where, tone: "plain" };
    case "tool.completed":
      return {
        title: `${tool()} done`,
        detail: parts(where, exit, d.durationMs !== undefined ? `${d.durationMs} ms` : null),
        tone: "plain",
      };
    case "tool.failed":
      return { title: `${tool()} failed`, detail: parts(where, exit), tone: "bad" };
    case "file.reported_write": {
      const what = d.change === "add" ? "added" : d.change === "delete" ? "deleted" : "changed";
      return { title: `file ${what}`, detail: where, tone: "plain" };
    }
    case "usage.reported":
      return {
        title: "tokens used",
        detail: d.totalTokens !== undefined ? `${d.totalTokens.toLocaleString()} total` : "",
        tone: "plain",
      };
    case "approval.requested":
      return { title: `permission asked for ${tool()}`, detail: where, tone: "plain" };
    case "approval.resolved":
      return {
        title: `${tool()} ${d.decision ?? "resolved"}`,
        detail: where,
        tone: d.decision === "denied" || d.decision === "deny" ? "bad" : "plain",
      };
    case "turn.started":
      return {
        title: "agent started a turn",
        detail: parts(d.turnNumber !== undefined ? `turn ${d.turnNumber}` : null, d.model),
        tone: "plain",
      };
    case "turn.completed":
      return {
        title: "agent finished its turn",
        detail: d.durationMs !== undefined ? `${(d.durationMs / 1000).toFixed(1)} s` : "",
        tone: "ok",
      };
    case "turn.failed":
      return {
        title: d.outcome === "interrupted" ? "agent’s turn was interrupted" : "agent’s turn failed",
        detail: d.errorType ?? "",
        tone: "bad",
      };
    case "agent.notified":
      return { title: "agent raised a notification", detail: d.notification ?? "", tone: "plain" };
    case "agent.subagent_started":
      return { title: "subagent started", detail: d.agentType ?? "", tone: "plain" };
    case "agent.subagent_stopped":
      return { title: "subagent stopped", detail: d.agentType ?? "", tone: "plain" };
    case "agent.model_switched":
      return { title: "agent switched model", detail: d.model ?? "", tone: "plain" };
    default:
      return { title: event.kind, detail: "", tone: "plain" };
  }
}
