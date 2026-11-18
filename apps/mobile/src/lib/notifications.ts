// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The Notifications list (48): what waits for its person on each computer, and what finished
 * while they were not looking. Each computer keeps its own list, the one its window shows. The
 * phone reads it from every computer it can reach, is told when it changes, and says back when
 * a finished row was looked at, so seen is one fact, kept on the computer.
 *
 * Nothing of it is kept on the phone beyond what is in memory while the app runs: a computer
 * that cannot be reached shows no rows, and none from before.
 *
 * What is here is the reading of the lists, and their words. The desktop's view is the model
 * for the words (`src/features/notifications/words.ts`).
 */

import {
  type Channel,
  ChannelClosed,
  type HostId,
  type NotificationPage,
  type NotificationSummary,
  type WorkspaceId,
} from "@yardsort/remote-client";

import { messageOf } from "./accounts";
import { stamp } from "./format";

/** What a computer says it answers when it keeps a list. One from before it does not. */
export const NOTIFICATION_LIST = "notification.list";
/** And when it says of itself that its list changed. */
export const NOTIFICATION_FOLLOW = "notification.follow";
/** How often a list is read again from a computer that does not say when it changes. */
export const LIST_EVERY_MS = 20_000;

/** Whether this computer keeps a Notifications list at all. */
export function keepsList(channel: Channel | null): channel is Channel {
  return !!channel?.host.capabilities.includes(NOTIFICATION_LIST);
}

/** One computer's list, as this phone last read it. */
export interface HostList {
  host: HostId;
  name: string;
  /** False for a computer on an earlier Yardsort: it keeps no list, and is not asked. */
  keeps: boolean;
  /** `null` until it has answered. */
  page: NotificationPage | null;
  /** Why it could not be read, when it could not. */
  problem: string | null;
}

export type Held = ReadonlyMap<HostId, HostList>;

/** A computer as a screen holds it: its channel while there is one, or that it is away. */
export interface Looked {
  host: HostId;
  name: string;
  channel: Channel | null;
  /** It cannot be reached: nothing of it is shown until it is back. */
  away: boolean;
}

const NONE: Held = new Map();

/**
 * Every computer's list, read through whatever channels the screen in front holds. One is read
 * when its channel opens and again when the computer says it changed, or every
 * {@link LIST_EVERY_MS} from one that does not say.
 */
export class Lists {
  #held: Held = NONE;
  #channels = new Map<HostId, Channel>();
  #reading = new Map<HostId, { again: boolean; done: Promise<void> }>();
  #timers = new Map<HostId, ReturnType<typeof setInterval>>();
  /** Rows said to be seen that their computer has not shown as seen yet, by computer. */
  #marked = new Map<HostId, Set<string>>();
  #listeners = new Set<() => void>();

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** The same object until something changes, as `useSyncExternalStore` needs. */
  getSnapshot = (): Held => this.#held;

  /**
   * How these computers stand now. One with a new channel is read through it and followed; one
   * that is away loses its rows; one between two channels keeps what was read. A computer not
   * named is left as it was.
   */
  sync(computers: readonly Looked[]): void {
    for (const { host, name, channel, away } of computers) {
      if (away) this.forget(host);
      else if (channel && this.#channels.get(host) !== channel) this.#attach(host, name, channel);
    }
  }

  /** Forget every computer but these: one unpaired has no list here any more. */
  keepOnly(hosts: readonly HostId[]): void {
    for (const host of [...this.#held.keys()]) if (!hosts.includes(host)) this.forget(host);
  }

  forget(host: HostId): void {
    this.#release(host);
    this.#marked.delete(host);
    if (!this.#held.has(host)) return;
    const held = new Map(this.#held);
    held.delete(host);
    this.#held = held;
    this.#changed();
  }

  /** Read `host`'s list again, through the channel held to it. One read at a time. */
  read(host: HostId): Promise<void> {
    const reading = this.#reading.get(host);
    if (reading) {
      reading.again = true;
      return reading.done;
    }
    const state = { again: false, done: Promise.resolve() };
    state.done = (async () => {
      do {
        state.again = false;
        const channel = this.#channels.get(host);
        if (!channel || !keepsList(channel)) return;
        try {
          const page = await channel.notifications();
          // Answered through a channel since replaced: the newer one reads for itself.
          if (this.#channels.get(host) === channel) {
            this.#put(host, { page: this.#settled(host, page), problem: null });
          }
        } catch (error) {
          // The reach says what became of a channel; what was read stays.
          if (error instanceof ChannelClosed) return;
          if (this.#channels.get(host) === channel) this.#put(host, { problem: messageOf(error) });
        }
      } while (state.again);
    })().finally(() => {
      this.#reading.delete(host);
    });
    this.#reading.set(host, state);
    return state.done;
  }

  /**
   * These finished rows of `host` were looked at. They move to Earlier here at once, and the
   * computer is told; what it did not take, its next word puts right. A row that waits for an
   * answer is not marked: it leaves when it gets one.
   */
  async seen(host: HostId, ids: readonly string[]): Promise<void> {
    const held = this.#held.get(host);
    const channel = this.#channels.get(host);
    if (!held?.page || !channel) return;
    const marked = held.page.notifications
      .filter((row) => row.part === "finished" && ids.includes(row.id))
      .map((row) => row.id);
    if (marked.length === 0) return;
    const waiting = this.#marked.get(host) ?? new Set<string>();
    this.#marked.set(host, waiting);
    for (const id of marked) waiting.add(id);
    const now = Date.now();
    this.#put(host, {
      page: {
        ...held.page,
        finished: Math.max(0, held.page.finished - marked.length),
        notifications: held.page.notifications.map((row) =>
          marked.includes(row.id) ? { ...row, part: "earlier", settledAt: now } : row,
        ),
      },
    });
    try {
      await channel.notificationsSeen({ ids: marked });
    } catch (error) {
      // Not taken: the next reading shows them as the computer has them.
      for (const id of marked) waiting.delete(id);
      if (!(error instanceof ChannelClosed)) void this.read(host);
    }
  }

  /**
   * A page as it is shown: a row said to be seen stays under Earlier though this page, asked
   * for before the computer took the word, still has it as finished. Without this the row
   * would come back for a moment, and whoever is looking at it would say seen again.
   */
  #settled(host: HostId, page: NotificationPage): NotificationPage {
    const waiting = this.#marked.get(host);
    if (!waiting || waiting.size === 0) return page;
    const now = Date.now();
    const still = new Set<string>();
    const notifications = page.notifications.map((row): NotificationSummary => {
      if (!waiting.has(row.id) || row.part !== "finished") return row;
      still.add(row.id);
      return { ...row, part: "earlier", settledAt: now };
    });
    // The computer shows it as seen, or no longer lists it: nothing is waited for.
    for (const id of [...waiting]) if (!still.has(id)) waiting.delete(id);
    if (still.size === 0) return page;
    return { ...page, finished: Math.max(0, page.finished - still.size), notifications };
  }

  #attach(host: HostId, name: string, channel: Channel): void {
    this.#release(host);
    this.#channels.set(host, channel);
    void channel.ended.then(() => {
      if (this.#channels.get(host) === channel) this.#release(host);
    });
    const keeps = keepsList(channel);
    this.#put(host, { host, name, keeps, ...(keeps ? {} : { page: null, problem: null }) });
    if (!keeps) return;
    void this.read(host);
    if (channel.host.capabilities.includes(NOTIFICATION_FOLLOW)) {
      // Refused or dropped, the list still shows as it was read.
      channel.followNotifications(() => void this.read(host)).catch(() => {});
    } else {
      this.#timers.set(
        host,
        setInterval(() => void this.read(host), LIST_EVERY_MS),
      );
    }
  }

  /** Let go of the channel to `host`, which is closed or about to be. What was read stays. */
  #release(host: HostId): void {
    this.#channels.delete(host);
    clearInterval(this.#timers.get(host));
    this.#timers.delete(host);
  }

  #put(host: HostId, change: Partial<HostList>): void {
    const was = this.#held.get(host) ?? {
      host,
      name: "",
      keeps: true,
      page: null,
      problem: null,
    };
    this.#held = new Map(this.#held).set(host, { ...was, ...change });
    this.#changed();
  }

  #changed(): void {
    for (const listener of this.#listeners) listener();
  }
}

let current: { profile: string; lists: Lists } | null = null;

/**
 * The lists of one profile's computers. Another profile's are another server's: nothing read
 * for one is ever shown for the other.
 */
export function listsOf(profile: string): Lists {
  if (current?.profile !== profile) current = { profile, lists: new Lists() };
  return current.lists;
}

// --- reading what is held -----------------------------------------------------------------------

/** A row, with the computer it is on. */
export interface Listed extends NotificationSummary {
  host: HostId;
  hostName: string;
}

export interface Sections {
  needs: Listed[];
  finished: Listed[];
  earlier: Listed[];
}

/**
 * Every computer's rows as one list in three parts, the newest first in each. A row of a kind
 * or a part only a newer computer knows is left out: this phone has no words for it.
 */
export function sections(held: Held): Sections {
  const rows: Listed[] = [];
  for (const { host, name, page } of held.values()) {
    for (const row of page?.notifications ?? []) {
      if (row.kind !== "other" && row.part !== "other") rows.push({ ...row, host, hostName: name });
    }
  }
  const newestFirst = (a: Listed, b: Listed) => b.at - a.at;
  const part = (which: NotificationSummary["part"]) =>
    rows.filter((row) => row.part === which).sort(newestFirst);
  return { needs: part("needs"), finished: part("finished"), earlier: part("earlier") };
}

/** How many things need their person, over every computer read. What the bell counts. */
export function needing(held: Held): number {
  let needs = 0;
  for (const { page } of held.values()) needs += page?.needs ?? 0;
  return needs;
}

/** How many finished and were not seen, counted the same way. */
export function unseen(held: Held): number {
  let finished = 0;
  for (const { page } of held.values()) finished += page?.finished ?? 0;
  return finished;
}

/** Whether any computer's list was cut to what one answer carries. */
export function cut(held: Held): boolean {
  return [...held.values()].some(({ page }) => page?.more);
}

/** "2 need you, 1 finished": a computer's counts in a line, or `null` with nothing to say. */
export function countWords(needs: number, finished: number): string | null {
  const said: string[] = [];
  if (needs > 0) said.push(`${needs} ${needs === 1 ? "needs" : "need"} you`);
  if (finished > 0) said.push(`${finished} finished`);
  return said.length > 0 ? said.join(", ") : null;
}

/** What a workspace's row is marked with. Something that waits says more than something done. */
export type Mark = "needs" | "finished";

export const MARK_WORDS: Record<Mark, string> = { needs: "needs you", finished: "finished" };

/** Each workspace of a computer that has something waiting or finished, from its list. */
export function marksOf(page: NotificationPage | null | undefined): Map<WorkspaceId, Mark> {
  const marks = new Map<WorkspaceId, Mark>();
  for (const row of page?.notifications ?? []) {
    if (!row.workspace || row.kind === "other") continue;
    if (row.part === "needs") marks.set(row.workspace, "needs");
    else if (row.part === "finished" && !marks.has(row.workspace)) {
      marks.set(row.workspace, "finished");
    }
  }
  return marks;
}

// --- a row's words ------------------------------------------------------------------------------

/** "12 min", "1 h 5 min": how long an agent worked, to the minute. Under a minute is not said. */
export function workedWords(ms: number): string | null {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return null;
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** The name a row leads with: its workspace, or for a schedule or a trigger without one, its own. */
export function leadName(row: NotificationSummary): string {
  return row.workspace && row.workspaceName ? row.workspaceName : row.title || row.projectName;
}

/** Where it is: its project and its computer. */
export function whereWords(row: Listed): string {
  return row.hostName ? `${row.projectName} · ${row.hostName}` : row.projectName;
}

/** What happened, in the row's words. */
export function what(row: NotificationSummary): string {
  const agent = row.harness ?? "The agent";
  const waits = row.part === "needs";
  // Named here when the row leads with its workspace; a row that leads with it does not repeat it.
  const named = row.workspace && row.workspaceName && row.title ? ` ${row.title}` : "";
  const at = row.step ? ` at ${row.step}` : "";
  switch (row.kind) {
    case "agent.asks":
      return waits ? `${agent} asks for a permission` : `${agent} asked for a permission`;
    case "agent.finished": {
      const worked = row.workedMs === undefined ? null : workedWords(row.workedMs);
      return worked ? `${agent} finished after ${worked}` : `${agent} finished`;
    }
    case "agent.failed":
      return `${agent}’s turn failed`;
    case "agent.stopped":
      return `${agent} stopped with an error`;
    case "workflow.approval":
      return waits
        ? `Workflow${named} waits for your approval${at}`
        : `Workflow${named} waited for an approval`;
    case "workflow.question":
      return waits ? `Workflow${named} asks${at}` : `Workflow${named} asked a question`;
    case "workflow.succeeded":
      return `Workflow${named} finished`;
    case "workflow.failed":
      return `Workflow${named} failed`;
    case "test.waits":
      return waits
        ? `Smoke test${named} waits for you to let it run`
        : `Smoke test${named} waited to be let run`;
    case "schedule.refused":
      return waits
        ? `Schedule${named} could not start its run`
        : `Schedule${named} could not start a run`;
    case "schedule.noWorkspace":
      return waits ? "Schedule has no workspace to run in" : "Schedule had no workspace to run in";
    case "trigger.refused":
      return waits
        ? `Trigger${named} could not start its run`
        : `Trigger${named} could not start a run`;
    case "trigger.capped":
      return waits
        ? `Trigger${named} reached its limit of runs`
        : `Trigger${named} had reached its limit of runs`;
    case "trigger.noWorkspace":
      return waits ? "Trigger has no workspace to run in" : "Trigger had no workspace to run in";
    default:
      return "";
  }
}

/** The one line of substance under the words, and how it reads. */
export interface Line {
  text: string;
  /** `code`: a command or an exit code. `quote`: someone's words. `plain`: a reason. */
  as: "code" | "quote" | "plain";
}

export function lineOf(row: NotificationSummary): Line | null {
  if (row.kind === "agent.asks") return row.command ? { text: row.command, as: "code" } : null;
  if (row.kind === "agent.stopped" && row.exitCode !== undefined) {
    return { text: `exit ${row.exitCode}`, as: "code" };
  }
  if (row.note) return { text: row.note, as: "plain" };
  if (row.said) return { text: `“${row.said}”`, as: "quote" };
  return null;
}

/** How a row is drawn: waiting, finished and unseen, gone badly, or left. */
export type Tone = "needs" | "finished" | "bad" | "earlier";

export function toneOf(row: NotificationSummary): Tone {
  if (row.part === "earlier") return "earlier";
  if (
    row.kind === "agent.failed" ||
    row.kind === "agent.stopped" ||
    row.kind === "workflow.failed"
  ) {
    return "bad";
  }
  return row.part === "needs" ? "needs" : "finished";
}

/** When, beside a row: how long it has waited, or for one that left, when it happened. */
export function whenWords(row: NotificationSummary, now = Date.now()): string {
  if (row.part === "earlier") return stamp(row.at, now);
  const minutes = Math.floor(Math.max(0, now - row.at) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

/** A screen of this app a row opens. */
export interface Opens {
  pathname: "/terminal" | "/decision" | "/workflow" | "/workspace";
  params: Record<string, string>;
  /** What that is, for a screen reader. */
  words: string;
}

/**
 * What pressing a row opens: what the phone already has for it. An agent's row opens its
 * conversation; a workflow's wait opens the decision, and its ending the run; a smoke test's
 * opens its workspace, where the card to answer is. A schedule or a trigger is changed at the
 * computer: its row goes as far as its workspace, and one without a workspace opens nothing.
 */
export function opens(row: Listed): Opens | null {
  const host = row.host;
  const about = row.kind.slice(0, row.kind.indexOf("."));
  if (about === "agent" && row.workspace && (row.session || row.terminal)) {
    return {
      pathname: "/terminal",
      params: {
        host,
        workspace: row.workspace,
        ...(row.session ? { session: row.session } : {}),
        ...(row.terminal ? { terminal: row.terminal } : {}),
        label: row.harness ?? "Terminal",
      },
      words: "the conversation",
    };
  }
  if (about === "workflow" && row.run) {
    return row.part === "needs" && row.step
      ? {
          pathname: "/decision",
          params: { host, run: row.run, step: row.step },
          words: "the decision",
        }
      : { pathname: "/workflow", params: { host, run: row.run }, words: "the run" };
  }
  if (!row.workspace) return null;
  return {
    pathname: "/workspace",
    params: { host, workspace: row.workspace },
    words:
      about === "test" && row.part === "needs" ? "its workspace, to answer it" : "its workspace",
  };
}

/** Everything a row says, for a screen reader. */
export function spoken(row: Listed, now = Date.now()): string {
  const line = lineOf(row);
  const to = opens(row);
  return [
    leadName(row),
    whereWords(row),
    what(row),
    line?.text,
    whenWords(row, now),
    to ? `opens ${to.words}` : "changed at the computer",
  ]
    .filter(Boolean)
    .join(", ");
}
