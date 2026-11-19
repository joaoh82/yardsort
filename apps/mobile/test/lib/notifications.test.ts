// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";
import {
  type Channel,
  type ChannelEnd,
  ChannelClosed,
  ClientError,
  type HostId,
  type NotificationPage,
  type NotificationSummary,
  type ProjectId,
  type SessionRecordId,
  type TerminalId,
  type WorkspaceId,
} from "@yardsort/remote-client";

import {
  countWords,
  leadName,
  lineOf,
  type Listed,
  Lists,
  listsOf,
  marksOf,
  needing,
  opens,
  sections,
  spoken,
  toneOf,
  unseen,
  what,
  whenWords,
  workedWords,
} from "../../src/lib/notifications";

const MAC = "host-mac" as HostId;
const BOX = "host-box" as HostId;
const NOW = 1_700_000_600_000;

function row(id: string, change: Partial<NotificationSummary> = {}): NotificationSummary {
  return {
    id,
    kind: "agent.finished",
    part: "finished",
    project: "project-1" as ProjectId,
    projectName: "orchard",
    workspace: "workspace-1" as WorkspaceId,
    workspaceName: "add-imperial-option",
    session: "session-1" as SessionRecordId,
    title: "Add an imperial option",
    harness: "claude",
    terminal: "terminal-1" as TerminalId,
    at: NOW - 4 * 60_000,
    ...change,
  };
}

function page(rows: NotificationSummary[], more = false): NotificationPage {
  return {
    notifications: rows,
    needs: rows.filter((r) => r.part === "needs").length,
    finished: rows.filter((r) => r.part === "finished").length,
    more,
  };
}

/** A computer at the other end of a channel, as far as its list goes. */
class Computer {
  rows: NotificationSummary[] = [];
  asked: string[] = [];
  seen: string[][] = [];
  refuse: Error | null = null;
  /** What following is refused with, when it is. */
  refuseFollow: Error | null = null;
  changed: (() => void) | null = null;
  /** How the list stood when it was last told of, as the host keeps it for a follower. */
  #told: number | null = null;
  /** Something that happens on the computer just after it answers its first request. */
  afterFirst: (() => void) | null = null;
  #end: (end: ChannelEnd) => void = () => {};
  /** Each answer waits for this, when set. */
  after: Promise<void> | null = null;

  constructor(
    readonly capabilities = ["notification.list", "notification.seen", "notification.follow"],
  ) {}

  channel(): Channel {
    const ended = new Promise<ChannelEnd>((resolve) => {
      this.#end = resolve;
    });
    const fake = {
      host: { capabilities: this.capabilities, name: "studio-mac" },
      ended,
      notifications: async () => {
        this.asked.push("list");
        // Answered as it arrives, in the order requests were sent, as a host answers them:
        // what the answer holds is how the list stood then, however late it gets back.
        const refuse = this.refuse;
        const answer = page(this.rows.map((r) => ({ ...r })));
        this.#first();
        await this.after;
        if (refuse) throw refuse;
        return answer;
      },
      notificationsSeen: async ({ ids = [] }: { ids?: string[] }) => {
        this.asked.push("seen");
        if (this.refuse) throw this.refuse;
        this.seen.push(ids);
        this.rows = this.rows.map((r) =>
          ids.includes(r.id) ? { ...r, part: "earlier", settledAt: NOW } : r,
        );
      },
      followNotifications: async (changed: () => void) => {
        this.asked.push("follow");
        if (this.refuseFollow) throw this.refuseFollow;
        // As the host does: from here on it tells of changes, and says nothing of now.
        this.#told = this.rows.length;
        this.changed = changed;
        this.#first();
        return async () => {};
      },
    };
    return fake as unknown as Channel;
  }

  #first(): void {
    const happens = this.afterFirst;
    this.afterFirst = null;
    happens?.();
  }

  /** The host's look at its list, once a second on a real one: a follower is told of a change. */
  look(): void {
    if (this.#told === null || this.#told === this.rows.length) return;
    this.#told = this.rows.length;
    this.changed?.();
  }

  close(): void {
    this.#end({ reason: "closed", code: null } as unknown as ChannelEnd);
  }
}

/** Everything queued has run: a read is a few promises deep. */
const settled = async () => {
  for (let n = 0; n < 10; n++) await Promise.resolve();
};

const ids = (rows: { id: string }[]) => rows.map((r) => r.id);

describe("the lists of the computers a phone can reach", () => {
  test("a computer's list is read through its channel, and followed", async () => {
    const mac = new Computer();
    mac.rows = [row("a"), row("b", { kind: "agent.asks", part: "needs" })];
    const lists = new Lists();
    let told = 0;
    lists.subscribe(() => (told += 1));
    const channel = mac.channel();

    lists.sync([{ host: MAC, name: "studio-mac", channel, away: false }]);
    await settled();
    // Followed first, then read: the computer tells of every change from the follow on.
    expect(mac.asked).toEqual(["follow", "list"]);
    const held = lists.getSnapshot();
    expect(held.get(MAC)?.name).toBe("studio-mac");
    expect(needing(held)).toBe(1);
    expect(unseen(held)).toBe(1);
    expect(told).toBeGreaterThan(0);

    // The same channel again is not read again: only what changed is.
    lists.sync([{ host: MAC, name: "studio-mac", channel, away: false }]);
    await settled();
    expect(mac.asked).toEqual(["follow", "list"]);

    // The computer says its list changed, and it is read again.
    mac.rows = [row("c", { kind: "agent.asks", part: "needs" }), ...mac.rows];
    mac.changed?.();
    await settled();
    expect(needing(lists.getSnapshot())).toBe(2);
  });

  test("something that happens while the phone is connecting is not missed", async () => {
    const mac = new Computer();
    // An agent asks just after the computer answers the phone's first request, whichever
    // that is, and before its second.
    mac.afterFirst = () => {
      mac.rows = [row("asks", { kind: "agent.asks", part: "needs" })];
    };
    const lists = new Lists();
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();
    mac.look();
    await settled();
    // Read first and followed after, it would be in neither the reading nor a later word:
    // the computer would count from a list that already had it.
    expect(needing(lists.getSnapshot())).toBe(1);
  });

  test("a computer that will not be followed, or never says, is read on a clock", async () => {
    const refusing = new Computer();
    refusing.refuseFollow = new ClientError("remote", "Not now.", {
      code: "internal",
      message: "Not now.",
      retryable: true,
    });
    const silent = new Computer(["notification.list", "notification.seen"]);
    const lists = new Lists(15);
    lists.sync([
      { host: MAC, name: "studio-mac", channel: refusing.channel(), away: false },
      { host: BOX, name: "linux-box", channel: silent.channel(), away: false },
    ]);
    await settled();
    expect(refusing.asked).toEqual(["follow", "list"]);
    expect(silent.asked).toEqual(["list"]);

    // What changes there is found at the next reading, with nobody to say so.
    refusing.rows = [row("a")];
    silent.rows = [row("b"), row("c")];
    await Bun.sleep(60);
    expect(unseen(lists.getSnapshot())).toBe(3);

    // Its channel closed, it is asked no more.
    refusing.close();
    silent.close();
    await settled();
    const asked = refusing.asked.length + silent.asked.length;
    await Bun.sleep(60);
    expect(refusing.asked.length + silent.asked.length).toBe(asked);
  });

  test("a follow that ends with its channel starts no clock", async () => {
    const mac = new Computer();
    mac.refuseFollow = new ChannelClosed({ reason: "closed", code: null } as unknown as ChannelEnd);
    const lists = new Lists(15);
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();
    const asked = mac.asked.length;
    await Bun.sleep(60);
    expect(mac.asked.length).toBe(asked);
  });

  test("a computer that is away shows no rows, and one between two channels keeps them", async () => {
    const mac = new Computer();
    mac.rows = [row("a")];
    const lists = new Lists();
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();
    expect(unseen(lists.getSnapshot())).toBe(1);

    // Dropped and trying again: what was read stays on screen.
    mac.close();
    await settled();
    lists.sync([{ host: MAC, name: "studio-mac", channel: null, away: false }]);
    expect(unseen(lists.getSnapshot())).toBe(1);

    // Offline: nothing of it is shown until it is back.
    lists.sync([{ host: MAC, name: "studio-mac", channel: null, away: true }]);
    expect(lists.getSnapshot().has(MAC)).toBe(false);

    // Back, on a new channel: read again.
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();
    expect(unseen(lists.getSnapshot())).toBe(1);
  });

  test("a computer from before the list is not asked, and says it keeps none", async () => {
    const old = new Computer([]);
    const lists = new Lists();
    lists.sync([{ host: BOX, name: "linux-box", channel: old.channel(), away: false }]);
    await settled();
    expect(old.asked).toEqual([]);
    expect(lists.getSnapshot().get(BOX)).toEqual({
      host: BOX,
      name: "linux-box",
      keeps: false,
      page: null,
      problem: null,
    });
  });

  test("a list that could not be read says why, and a closed channel says nothing", async () => {
    const mac = new Computer();
    mac.refuse = new ClientError("remote", "This phone may not watch terminals.", {
      code: "forbidden",
      message: "This phone may not watch terminals.",
      retryable: false,
    });
    const lists = new Lists();
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();
    expect(lists.getSnapshot().get(MAC)?.problem).toBe("This phone may not watch terminals.");

    const box = new Computer();
    box.refuse = new ChannelClosed({ reason: "closed", code: null } as unknown as ChannelEnd);
    lists.sync([{ host: BOX, name: "linux-box", channel: box.channel(), away: false }]);
    await settled();
    expect(lists.getSnapshot().get(BOX)?.problem).toBeNull();
  });

  test("seen moves a finished row to Earlier at once and tells the computer; a waiting row is left", async () => {
    const mac = new Computer();
    mac.rows = [row("done"), row("asks", { kind: "agent.asks", part: "needs" })];
    const lists = new Lists();
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();

    const sent = lists.seen(MAC, ["done", "asks", "not-there"]);
    // Shown before the computer has answered.
    const parts = () =>
      lists
        .getSnapshot()
        .get(MAC)
        ?.page?.notifications.map((r) => r.part);
    expect(parts()).toEqual(["earlier", "needs"]);
    expect(unseen(lists.getSnapshot())).toBe(0);
    expect(needing(lists.getSnapshot())).toBe(1);
    await sent;
    expect(mac.seen).toEqual([["done"]]);

    // Nothing finished is named: nothing is sent.
    await lists.seen(MAC, ["asks"]);
    expect(mac.seen).toEqual([["done"]]);
  });

  test("a row the computer would not mark is read back as it stands", async () => {
    const mac = new Computer();
    mac.rows = [row("done")];
    const lists = new Lists();
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();
    mac.refuse = new ClientError("remote", "No.", {
      code: "forbidden",
      message: "No.",
      retryable: false,
    });
    await lists.seen(MAC, ["done"]);
    mac.refuse = null;
    await lists.read(MAC);
    expect(lists.getSnapshot().get(MAC)?.page?.notifications[0]?.part).toBe("finished");
  });

  test("an answer through a channel since replaced is not shown over the newer one's", async () => {
    const mac = new Computer();
    mac.rows = [row("old")];
    let release = () => {};
    mac.after = new Promise<void>((resolve) => (release = resolve));
    const lists = new Lists();
    lists.sync([{ host: MAC, name: "studio-mac", channel: mac.channel(), away: false }]);
    await settled();

    const next = new Computer();
    next.rows = [row("new")];
    lists.sync([{ host: MAC, name: "studio-mac", channel: next.channel(), away: false }]);
    release();
    await settled();
    await lists.read(MAC);
    expect(ids(lists.getSnapshot().get(MAC)?.page?.notifications ?? [])).toEqual(["new"]);
  });

  test("an unpaired computer's list is forgotten, and each profile has its own lists", async () => {
    const lists = new Lists();
    const mac = new Computer();
    const box = new Computer();
    lists.sync([
      { host: MAC, name: "studio-mac", channel: mac.channel(), away: false },
      { host: BOX, name: "linux-box", channel: box.channel(), away: false },
    ]);
    await settled();
    lists.keepOnly([MAC]);
    expect([...lists.getSnapshot().keys()]).toEqual([MAC]);

    const first = listsOf("profile-a");
    expect(listsOf("profile-a")).toBe(first);
    // Another server's computers: nothing read for the first is shown for it.
    expect(listsOf("profile-b")).not.toBe(first);
    expect(listsOf("profile-b").getSnapshot().size).toBe(0);
  });
});

describe("the list in three parts", () => {
  const held = () => {
    const lists = new Map([
      [
        MAC,
        {
          host: MAC,
          name: "studio-mac",
          keeps: true,
          problem: null,
          page: page(
            [
              row("m-asks", { kind: "agent.asks", part: "needs", at: NOW - 60_000 }),
              row("m-done", { at: NOW - 4 * 60_000 }),
              row("m-old", { part: "earlier", at: NOW - 3_600_000, settledAt: NOW - 60_000 }),
              row("m-new-kind", { kind: "other", part: "needs" }),
              row("m-new-part", { part: "other" }),
            ],
            true,
          ),
        },
      ],
      [
        BOX,
        {
          host: BOX,
          name: "linux-box",
          keeps: true,
          problem: null,
          page: page([
            row("b-test", { kind: "test.waits", part: "needs", at: NOW - 14 * 60_000 }),
            row("b-stopped", { kind: "agent.stopped", at: NOW - 22 * 60_000, exitCode: 1 }),
          ]),
        },
      ],
    ]);
    return lists;
  };

  test("every computer's rows are one list, the newest first, each naming its computer", () => {
    const parts = sections(held());
    expect(ids(parts.needs)).toEqual(["m-asks", "b-test"]);
    expect(ids(parts.finished)).toEqual(["m-done", "b-stopped"]);
    expect(ids(parts.earlier)).toEqual(["m-old"]);
    expect(parts.needs.map((r) => r.hostName)).toEqual(["studio-mac", "linux-box"]);
    // A kind or a part only a newer computer knows has no words here, and is left out.
    expect(ids([...parts.needs, ...parts.finished, ...parts.earlier])).not.toContain("m-new-kind");
  });

  test("the counts are the computers' own, of everything and not only what is listed", () => {
    const lists = held();
    // The computer counted its two kinds it could not show too: its count is what is said.
    expect(needing(lists)).toBe(3);
    expect(unseen(lists)).toBe(2);
    expect(countWords(2, 1)).toBe("2 need you, 1 finished");
    expect(countWords(1, 0)).toBe("1 needs you");
    expect(countWords(0, 3)).toBe("3 finished");
    expect(countWords(0, 0)).toBeNull();
  });

  test("a workspace is marked with what waits there, and waiting says more than finished", () => {
    const one = "workspace-1" as WorkspaceId;
    const two = "workspace-2" as WorkspaceId;
    const marks = marksOf(
      page([
        row("a", { workspace: one }),
        row("b", { workspace: one, kind: "agent.asks", part: "needs" }),
        row("c", { workspace: one }),
        row("d", { workspace: two }),
        row("e", { workspace: "workspace-3" as WorkspaceId, part: "earlier" }),
        row("f", { workspace: undefined, kind: "schedule.noWorkspace", part: "needs" }),
      ]),
    );
    expect([...marks]).toEqual([
      [one, "needs"],
      [two, "finished"],
    ]);
    expect(marksOf(null).size).toBe(0);
  });
});

describe("a row's words", () => {
  const listed = (change: Partial<NotificationSummary> = {}): Listed => ({
    ...row("r", change),
    host: MAC,
    hostName: "studio-mac",
  });

  test("an agent: asking, finished with how long it worked, failed and stopped", () => {
    expect(what(row("r", { kind: "agent.asks", part: "needs" }))).toBe(
      "claude asks for a permission",
    );
    // Once answered the list does not know how, and does not say.
    expect(what(row("r", { kind: "agent.asks", part: "earlier" }))).toBe(
      "claude asked for a permission",
    );
    expect(what(row("r", { workedMs: 12 * 60_000 }))).toBe("claude finished after 12 min");
    expect(what(row("r", { workedMs: 20_000 }))).toBe("claude finished");
    expect(what(row("r", { harness: undefined }))).toBe("The agent finished");
    expect(what(row("r", { kind: "agent.stopped", harness: "codex" }))).toBe(
      "codex stopped with an error",
    );
    expect(what(row("r", { kind: "agent.failed" }))).toBe("claude’s turn failed");
    expect(workedWords(65 * 60_000)).toBe("1 h 5 min");
    expect(workedWords(2 * 3_600_000)).toBe("2 h");
  });

  test("a workflow, a smoke test, a schedule and a trigger are named in the words", () => {
    const flow = { title: "release-check", step: "pick-version", run: "run-1" };
    expect(what(row("r", { kind: "workflow.question", part: "needs", ...flow }))).toBe(
      "Workflow release-check asks at pick-version",
    );
    expect(what(row("r", { kind: "workflow.approval", part: "needs", ...flow }))).toBe(
      "Workflow release-check waits for your approval at pick-version",
    );
    expect(what(row("r", { kind: "workflow.failed", ...flow }))).toBe(
      "Workflow release-check failed",
    );
    expect(what(row("r", { kind: "test.waits", part: "needs", title: "sign-in" }))).toBe(
      "Smoke test sign-in waits for you to let it run",
    );
    expect(what(row("r", { kind: "trigger.capped", part: "needs", title: "on-review" }))).toBe(
      "Trigger on-review reached its limit of runs",
    );
    // With no workspace the row leads with its own name, and the words do not say it twice.
    const lost = row("r", {
      kind: "schedule.noWorkspace",
      part: "needs",
      title: "nightly",
      workspace: undefined,
      workspaceName: "old-spike",
    });
    expect(leadName(lost)).toBe("nightly");
    expect(what(lost)).toBe("Schedule has no workspace to run in");
    expect(leadName(row("r"))).toBe("add-imperial-option");
  });

  test("the line under them: a command, an exit code, a reason, or what was said", () => {
    expect(lineOf(row("r", { kind: "agent.asks", part: "needs", command: "git push" }))).toEqual({
      text: "git push",
      as: "code",
    });
    expect(lineOf(row("r", { kind: "agent.asks", part: "earlier" }))).toBeNull();
    expect(lineOf(row("r", { kind: "agent.stopped", exitCode: 1 }))).toEqual({
      text: "exit 1",
      as: "code",
    });
    expect(lineOf(row("r", { said: "Added the flag." }))).toEqual({
      text: "“Added the flag.”",
      as: "quote",
    });
    expect(lineOf(row("r", { kind: "workflow.failed", note: "No agent is set up." }))).toEqual({
      text: "No agent is set up.",
      as: "plain",
    });
    expect(lineOf(row("r"))).toBeNull();
  });

  test("how it is drawn, and when it says it happened", () => {
    expect(toneOf(row("r", { kind: "agent.asks", part: "needs" }))).toBe("needs");
    expect(toneOf(row("r"))).toBe("finished");
    expect(toneOf(row("r", { kind: "agent.stopped" }))).toBe("bad");
    expect(toneOf(row("r", { kind: "workflow.failed" }))).toBe("bad");
    expect(toneOf(row("r", { kind: "agent.stopped", part: "earlier" }))).toBe("earlier");

    expect(whenWords(row("r", { at: NOW - 20_000 }), NOW)).toBe("now");
    expect(whenWords(row("r", { at: NOW - 14 * 60_000 }), NOW)).toBe("14m");
    expect(whenWords(row("r", { at: NOW - 3 * 3_600_000 }), NOW)).toBe("3h");
    expect(whenWords(row("r", { at: NOW - 50 * 3_600_000 }), NOW)).toBe("2d");
    // One that left says when it happened, as a time and not as how long it has waited.
    expect(whenWords(row("r", { part: "earlier", at: NOW - 60_000 }), NOW)).not.toMatch(/m$/);
  });

  test("what a row opens: the conversation, the decision, the run, or its workspace", () => {
    expect(opens(listed())).toEqual({
      pathname: "/terminal",
      params: {
        host: MAC,
        workspace: "workspace-1",
        session: "session-1",
        terminal: "terminal-1",
        label: "claude",
      },
      words: "the conversation",
    });
    // An agent whose process ended has no terminal: its conversation still opens.
    expect(opens(listed({ terminal: undefined }))?.params).toEqual({
      host: MAC,
      workspace: "workspace-1",
      session: "session-1",
      label: "claude",
    });

    const flow = { session: undefined, terminal: undefined, run: "run-1", step: "sign-off" };
    expect(opens(listed({ kind: "workflow.approval", part: "needs", ...flow }))).toEqual({
      pathname: "/decision",
      params: { host: MAC, run: "run-1", step: "sign-off" },
      words: "the decision",
    });
    // Answered, there is nothing left to decide: the run is what there is to see.
    expect(opens(listed({ kind: "workflow.approval", part: "earlier", ...flow }))?.pathname).toBe(
      "/workflow",
    );
    expect(opens(listed({ kind: "workflow.succeeded", ...flow, step: undefined }))).toEqual({
      pathname: "/workflow",
      params: { host: MAC, run: "run-1" },
      words: "the run",
    });

    expect(opens(listed({ kind: "test.waits", part: "needs", session: undefined }))).toEqual({
      pathname: "/workspace",
      params: { host: MAC, workspace: "workspace-1" },
      words: "its workspace, to answer it",
    });
    expect(opens(listed({ kind: "trigger.refused", part: "needs" }))?.pathname).toBe("/workspace");
    // A schedule with no workspace is changed at the computer: there is nothing here to open.
    const lost = listed({ kind: "schedule.noWorkspace", part: "needs", workspace: undefined });
    expect(opens(lost)).toBeNull();
    expect(spoken(lost, NOW)).toContain("changed at the computer");
    expect(spoken(listed({ said: "Done." }), NOW)).toBe(
      "add-imperial-option, orchard · studio-mac, claude finished, “Done.”, 4m, opens the conversation",
    );
  });
});
