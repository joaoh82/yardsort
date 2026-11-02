// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Reading a computer: what is kept of it, and its activity followed, against the real server, relay
// and host through the stand-in (`crates/server/examples/stand-in.rs`).

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import {
  type ActivityEvent,
  type Channel,
  ClientError,
  type HostId,
  type Scope,
  Secret,
  type WorkspaceId,
  type WorkspacePage,
  webCrypto,
} from "@yardsort/remote-client";

import { StandIn, START_TIMEOUT_MS } from "../../../../packages/remote-client/test/stand-in.ts";
import { Accounts, type Browser, type Env } from "../../src/lib/accounts";
import { ActivityFeed, describeEvent, reportsOnlyStartAndEnd } from "../../src/lib/activity";
import { Observed } from "../../src/lib/observed";
import { MemoryVault } from "./memory-vault";

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start();
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

const browser: Browser = async (url, redirect) => {
  let next = url;
  for (let hop = 0; hop < 5; hop++) {
    const answer = await fetch(next, { redirect: "manual" });
    const location = answer.headers.get("location");
    if (!location) throw new Error(`no redirect (${answer.status})`);
    if (location.startsWith(redirect)) return location;
    next = new URL(location, next).toString();
  }
  throw new Error("the sign-in went round in circles");
};

/** A phone the computer has said yes to, with what it was allowed. */
async function paired(name: string, scopes?: Scope[]) {
  const shelf = new MemoryVault();
  const env: Env = {
    crypto: webCrypto() as NonNullable<ReturnType<typeof webCrypto>>,
    platform: "ios",
    userAgent: "yardsort-mobile/test (ios)",
    shelf,
  };
  const accounts = new Accounts(env, new MemoryVault());
  await accounts.load();
  expect(await accounts.signIn(await accounts.lookUp(standIn.server), name, browser)).toBe(true);
  const { code } = await standIn.offer();
  const asked = await accounts.pair({ kind: "code", value: new Secret(code) });
  await standIn.pending();
  await standIn.approve(asked.id, scopes);
  const list = await accounts.computers();
  expect(list.computers.map((c) => c.host)).toEqual([standIn.host]);
  return { accounts, shelf, grant: asked };
}

async function until(what: string, done: () => boolean): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(10);
  }
}

const page = (name = "fix-login"): WorkspacePage => ({
  projects: [
    { id: "p1", name: "demo", missing: false },
    { id: "p2", name: "empty", missing: false },
  ] as WorkspacePage["projects"],
  workspaces: [
    {
      id: "w1",
      project: "p1",
      name,
      kind: "worktree",
      branch: { label: "ys/fix-login", detached: false, unborn: false },
      archived: false,
      missing: false,
      tasks: [{ source: "github", key: "#1", title: "A task's title", url: "https://x.example" }],
      runningSessions: 1,
    },
  ] as WorkspacePage["workspaces"],
  next: null,
});

const HOST = "h1" as HostId;
const computer = { host: HOST, name: "Workstation", lastSeenAt: 5 };

describe("what is kept", () => {
  test("is read back by a later start, each entry with when it was seen", async () => {
    const shelf = new MemoryVault();
    const first = new Observed(shelf, "k", () => 1000);
    await first.keepComputers([computer]);
    await first.keepWorkspaces(HOST, page());

    const later = new Observed(shelf, "k");
    expect(await later.computers()).toEqual([{ ...computer, observedAt: 1000 }]);
    expect(await later.workspaces(HOST)).toEqual({
      observedAt: 1000,
      projects: [{ id: "p1", name: "demo" }],
      workspaces: [
        { id: "w1", project: "p1", name: "fix-login", branch: "ys/fix-login", missing: false },
      ],
    } as never);
  });

  test("is names and branches: not a task's title, nor anything else a computer said", async () => {
    const shelf = new MemoryVault();
    const observed = new Observed(shelf, "k");
    await observed.keepComputers([{ ...computer, scopes: ["terminal:control"] } as never]);
    await observed.keepWorkspaces(HOST, page());
    const kept = [...shelf.kept.values()].join();
    expect(kept).not.toContain("A task's title");
    expect(kept).not.toContain("terminal:control");
  });

  test("of a computer no longer listed goes, and is not put back by a late answer", async () => {
    const shelf = new MemoryVault();
    const observed = new Observed(shelf, "k");
    await observed.keepComputers([computer]);
    await observed.keepWorkspaces(HOST, page());
    await observed.keepComputers([]);
    expect(await observed.workspaces(HOST)).toBeNull();
    await observed.keepWorkspaces(HOST, page());
    expect(await observed.workspaces(HOST)).toBeNull();
    expect([...shelf.kept.values()].join()).not.toContain("fix-login");
  });

  test("that does not read, or will not write, costs only what was kept", async () => {
    const shelf = new MemoryVault();
    shelf.kept.set("k", "{ not json");
    expect(await new Observed(shelf, "k").computers()).toEqual([]);
    shelf.kept.set(
      "k",
      JSON.stringify({
        version: 1,
        computers: [{ host: "h1" }, { ...computer, observedAt: 1 }],
        workspaces: { h1: { observedAt: "soon" } },
      }),
    );
    const partly = new Observed(shelf, "k");
    expect((await partly.computers()).map((c) => c.name)).toEqual(["Workstation"]);
    expect(await partly.workspaces(HOST)).toBeNull();

    const failing = new MemoryVault();
    failing.failing = /.*/;
    await new Observed(failing, "k").keepComputers([computer]);
  });

  test("is one profile's, and forgetting it leaves nothing, whatever was on its way", async () => {
    const shelf = new MemoryVault();
    const one = new Observed(shelf, "one");
    const two = new Observed(shelf, "two");
    await one.keepComputers([computer]);
    expect(await two.computers()).toEqual([]);
    const late = one.keepComputers([computer]);
    await one.forget();
    await late;
    await one.keepComputers([computer]);
    expect(shelf.kept.has("one")).toBe(false);
    expect(await one.computers()).toEqual([]);
  });

  test("goes with signing out", async () => {
    const { accounts, shelf } = await paired("Signing-out phone");
    expect(shelf.kept.size).toBe(1);
    await accounts.signOut();
    expect(shelf.kept.size).toBe(0);
  });

  test("goes with a profile that signing in again replaced, and signing out leaves nothing", async () => {
    const { accounts, shelf } = await paired("Signing-in-again phone");
    expect([...shelf.kept.values()].join()).toContain("Stand-in host");
    await accounts.signIn(await accounts.lookUp(standIn.server), "Signing-in-again phone", browser);
    expect(shelf.kept.size).toBe(0);
    await accounts.computers();
    await accounts.signOut();
    expect(shelf.kept.size).toBe(0);
  });

  test("of a computer that took this phone back goes, and the phone stays signed in", async () => {
    const { accounts, shelf, grant } = await paired("Taken-back phone");
    const channel = await accounts.openChannel(standIn.host);
    await accounts.observed()?.keepWorkspaces(standIn.host, await channel.workspaces());
    expect([...shelf.kept.values()].join()).toContain("work");
    channel.close();

    await standIn.revoke(grant.id);
    expect((await accounts.computers()).computers).toEqual([]);
    expect(await accounts.observed()?.computers()).toEqual([]);
    expect([...shelf.kept.values()].join()).not.toContain('"work"');
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
  });

  test("a server out of reach deletes nothing", async () => {
    const { accounts, shelf } = await paired("Out-of-reach phone");
    const kept = [...shelf.kept.values()].join();
    const broken = new Accounts(
      {
        ...accounts.env,
        fetch: (async () => {
          throw new TypeError("Network request failed");
        }) as never,
      },
      accounts.vault,
    );
    await broken.load();
    await broken.computers().catch(() => {});
    expect([...shelf.kept.values()].join()).toBe(kept);
  });
});

describe("following activity", () => {
  async function workspace(name: string, scopes?: Scope[]) {
    const { accounts } = await paired(name, scopes);
    const channel = await accounts.openChannel(standIn.host);
    const found = (await channel.workspaces()).workspaces.find((w) => w.name === "work");
    if (!found) throw new Error("no workspace");
    return { accounts, channel, id: found.id as WorkspaceId, project: found.project };
  }

  const exits = (feed: ActivityFeed) =>
    feed.getSnapshot().events.filter((e: ActivityEvent) => e.kind === "process.exited").length;

  test("reads what was recorded, then hears what is recorded next", async () => {
    const { channel, id } = await workspace("Following phone");
    await standIn.run();
    const feed = new ActivityFeed(id);
    let told = 0;
    feed.subscribe(() => (told += 1));
    await feed.follow(channel);
    await until("following", () => feed.getSnapshot().following);
    await until("read the first run", () => exits(feed) >= 1);
    const before = exits(feed);
    expect(told).toBeGreaterThan(0);

    await standIn.run();
    await until("heard the next run", () => exits(feed) === before + 1);
    channel.close();
    await until("told it stopped", () => !feed.getSnapshot().following);
  });

  /** Wait for the host to have recorded another exit, so what follows cannot hear it live. */
  async function recordedAnotherRun(channel: Channel, id: WorkspaceId): Promise<void> {
    const count = async () =>
      (await channel.activity({ workspace: id })).events.filter((e) => e.kind === "process.exited")
        .length;
    const before = await count();
    await standIn.run();
    const deadline = Date.now() + 10_000;
    while ((await count()) <= before) {
      if (Date.now() > deadline) throw new Error("the host never recorded the run");
      await Bun.sleep(20);
    }
  }

  /** What is asked through `channel`, by operation. */
  function asking(channel: Channel): string[] {
    const asked: string[] = [];
    const ask = channel.ask.bind(channel);
    channel.ask = (operation, onReply) => {
      asked.push(operation.op);
      return ask(operation, onReply);
    };
    return asked;
  }

  test("a channel lost loses nothing still kept, and repeats nothing", async () => {
    const { accounts, channel, id } = await workspace("Dropped phone");
    const feed = new ActivityFeed(id);
    await feed.follow(channel);
    await until("following", () => feed.getSnapshot().following);
    const before = exits(feed);

    channel.close();
    await until("told it stopped", () => !feed.getSnapshot().following);
    const again = await accounts.openChannel(standIn.host);
    await recordedAnotherRun(again, id);

    await feed.follow(again);
    await until("caught up", () => exits(feed) === before + 1);
    const ids = feed.getSnapshot().events.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    again.close();
  });

  test("stopping unsubscribes, and following again carries on", async () => {
    const { channel, id } = await workspace("Stopping phone");
    const asked = asking(channel);
    const feed = new ActivityFeed(id);
    await feed.follow(channel);
    await until("following", () => feed.getSnapshot().following);
    const before = exits(feed);
    feed.stop();
    expect(feed.getSnapshot().following).toBe(false);
    expect(asked).toContain("activity.unsubscribe");

    await recordedAnotherRun(channel, id);
    expect(exits(feed)).toBe(before);

    await feed.follow(channel);
    await until("caught up", () => exits(feed) === before + 1);
    channel.close();
  });

  test("stopping while still reading leaves one subscription, however quickly it starts again", async () => {
    const { channel, id } = await workspace("Hurrying phone");
    const asked = asking(channel);
    const feed = new ActivityFeed(id);
    const first = feed.follow(channel);
    feed.stop();
    await feed.follow(channel);
    await first;
    expect(asked.filter((op) => op === "activity.subscribe")).toHaveLength(1);
    expect(feed.getSnapshot().following).toBe(true);
    channel.close();
  });

  test("a grant that does not reach activity says so, in the computer's words", async () => {
    const { channel, id } = await workspace("No-activity phone", ["workspace:read"]);
    const feed = new ActivityFeed(id);
    await feed.follow(channel);
    expect(feed.getSnapshot().problem).toBeTruthy();
    expect(feed.getSnapshot().following).toBe(false);
    channel.close();
  });

  test("what the host records reads as words", async () => {
    const { channel, id } = await workspace("Reading phone");
    await standIn.run();
    const events = (await channel.activity({ workspace: id })).events;
    const titles = events.map((event) => describeEvent(event).title);
    expect(titles.some((title) => title.endsWith(" started"))).toBe(true);
    expect(titles).toContain("exited");
    channel.close();
  });

  test("a kind this build does not know is shown by its name", () => {
    const words = describeEvent({ kind: "something.new", detail: {} } as unknown as ActivityEvent);
    expect(words).toEqual({ title: "something.new", detail: "", tone: "plain" });
  });

  test("an agent started without reporting is told apart, by the one started last", () => {
    const started = (runKind: string, capture?: string) =>
      ({ kind: "process.started", detail: { runKind, capture } }) as unknown as ActivityEvent;
    expect(reportsOnlyStartAndEnd([])).toBe(false);
    expect(reportsOnlyStartAndEnd([started("shell")])).toBe(false);
    expect(reportsOnlyStartAndEnd([started("harness")])).toBe(true);
    expect(reportsOnlyStartAndEnd([started("harness", "hook")])).toBe(false);
    // A shell opened since says nothing about the agent.
    expect(reportsOnlyStartAndEnd([started("harness"), started("shell")])).toBe(true);
    // A new agent, after capture was turned on, settles it.
    expect(reportsOnlyStartAndEnd([started("harness"), started("harness", "hook")])).toBe(false);
    expect(reportsOnlyStartAndEnd([started("harness", "hook"), started("harness")])).toBe(true);
  });
});

describe("memory", () => {
  test("approved notes can be read, and a grant that does not reach them is refused", async () => {
    await standIn.note("Migrations are append-only.");
    const { accounts } = await paired("Reading phone", ["workspace:read", "memory:read"]);
    const channel = await accounts.openChannel(standIn.host);
    const { projects } = await channel.workspaces();
    const project = (projects[0] as (typeof projects)[number]).id;
    const { notes } = await channel.memory({ project });
    expect(notes.map((note) => note.text)).toContain("Migrations are append-only.");
    channel.close();

    const barred = await paired("Barred phone", ["workspace:read"]);
    const other = await barred.accounts.openChannel(standIn.host);
    const refused = await other.memory({ project }).catch((e: unknown) => e);
    expect((refused as ClientError).code).toBe("forbidden");
    other.close();
  });
});
