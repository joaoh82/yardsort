// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What a computer refusing this phone came to (31 §5), against the real server, relay and host.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { Client, type DeviceId, type HostId } from "@yardsort/remote-client";

import {
  signIn as signInElsewhere,
  StandIn,
  START_TIMEOUT_MS,
} from "../../../../packages/remote-client/test/stand-in.ts";
import type { Accounts } from "../../src/lib/accounts";
import { Presence, type Reach } from "../../src/lib/presence";
import { paired } from "./phone";

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start();
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

/** Presence, with a way to wait for what it comes to. */
function watching(accounts: Accounts) {
  const presence = new Presence({
    open: (host: HostId) => accounts.openChannel(host),
    explain: (host: HostId) => accounts.explain(host),
    offlineRetryMs: 50,
    firstRetryMs: 50,
    maxRetryMs: 100,
  });
  const reach = () => presence.getSnapshot().get(standIn.host);
  const reached = async (kind: Reach["kind"]) => {
    const deadline = Date.now() + 10_000;
    while (reach()?.kind !== kind) {
      if (Date.now() > deadline) throw new Error(`never ${kind}; it is ${reach()?.kind}`);
      await Bun.sleep(10);
    }
  };
  presence.watch([standIn.host]);
  return { presence, reach, reached };
}

/** A way to take the server out of reach, and to have it refuse access once. */
function network() {
  const net = {
    down: false,
    lapsed: false,
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      if (net.down) throw new TypeError("Network request failed");
      if (net.lapsed && String(input).endsWith("/v1/grants")) {
        net.lapsed = false;
        return Response.json(
          { code: "unauthenticated", message: "It has lapsed.", retryable: false },
          { status: 401 },
        );
      }
      return fetch(input, init);
    }) as typeof fetch,
  };
  return net;
}

describe("a computer refusing this phone", () => {
  test("that took back its grant: only it goes, and the phone stays signed in", async () => {
    const { accounts, vault, grant } = await paired(standIn, "Taken back");
    await accounts.observed()?.keepWorkspaces(standIn.host, {
      projects: [],
      workspaces: [],
      next: null,
    });
    expect(await accounts.observed()?.workspaces(standIn.host)).not.toBeNull();
    const { presence, reach, reached } = watching(accounts);
    await reached("online");

    await standIn.revoke(grant.id);
    await reached("gone");
    expect(reach()).toEqual({ kind: "gone", why: "tookBack" });
    expect(accounts.getSnapshot().lost).toEqual([
      { host: standIn.host, name: standIn.hostName, why: "tookBack" },
    ]);
    expect(await accounts.observed()?.computers()).toEqual([]);
    expect(await accounts.observed()?.workspaces(standIn.host)).toBeNull();
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
    expect(vault.credentials()).toHaveLength(1);

    accounts.dismissLost(standIn.host);
    expect(accounts.getSnapshot().lost).toEqual([]);
    presence.stop();
  }, 30_000);

  test("that did it while nobody was looking: the next list says so", async () => {
    const { accounts, grant } = await paired(standIn, "Away");
    await standIn.revoke(grant.id);
    expect(accounts.getSnapshot().lost).toEqual([]);
    await accounts.computers();
    expect(accounts.getSnapshot().lost.map((lost) => lost.why)).toEqual(["tookBack"]);
    // Told once: a later list has nothing to add.
    await accounts.computers();
    expect(accounts.getSnapshot().lost).toHaveLength(1);
  }, 30_000);

  test("unpaired here: nobody is told what they just did", async () => {
    const { accounts, grant } = await paired(standIn, "Unpairing");
    const { presence, reached } = watching(accounts);
    await reached("online");

    await accounts.unpair(grant.id, standIn.host);
    await reached("gone");
    await accounts.computers();
    expect(accounts.getSnapshot().lost).toEqual([]);
    presence.stop();
  }, 30_000);

  test("unpairing that fails leaves the news to come", async () => {
    const net = network();
    const { accounts, grant } = await paired(standIn, "Unpair failed", undefined, net.fetch);
    net.down = true;
    await expect(accounts.unpair(grant.id, standIn.host)).rejects.toThrow();
    net.down = false;
    await standIn.revoke(grant.id);
    await accounts.computers();
    expect(accounts.getSnapshot().lost.map((lost) => lost.why)).toEqual(["tookBack"]);
  }, 30_000);

  test("with the phone itself revoked: signed out, and nothing is kept", async () => {
    const { accounts, vault, shelf } = await paired(standIn, "Revoked");
    const device = accounts.getSnapshot().active?.device;
    const { presence, reach, reached } = watching(accounts);
    await reached("online");
    expect(shelf.kept.size).toBeGreaterThan(0);

    const client = new Client(standIn.server);
    const other = await signInElsewhere(client, "Other device", "computer");
    await client.revokeDevice(other.access, device as DeviceId);

    const deadline = Date.now() + 10_000;
    while (accounts.getSnapshot().active) {
      if (Date.now() > deadline) throw new Error("never signed out");
      await Bun.sleep(10);
    }
    expect(accounts.getSnapshot().notice).toMatch(/This phone was signed out of/);
    expect(vault.credentials()).toEqual([]);
    expect(reach()?.kind).not.toBe("gone");
    await Bun.sleep(100);
    expect(shelf.kept.size).toBe(0);
    presence.stop();
  }, 30_000);

  test("with the phone itself revoked: that is the answer, not that nobody knows", async () => {
    const { accounts } = await paired(standIn, "Revoked, asked");
    const client = new Client(standIn.server);
    const other = await signInElsewhere(client, "Other device", "computer");
    const device = accounts.getSnapshot().active?.device as DeviceId;
    await client.revokeDevice(other.access, device);
    expect(await accounts.explain(standIn.host)).toBe("signedOut");
    expect(accounts.getSnapshot().active).toBeNull();
  }, 30_000);

  test("with the server out of reach: nothing goes, and it is asked again", async () => {
    const net = network();
    const { accounts, vault, grant } = await paired(standIn, "Cut off", undefined, net.fetch);
    const { presence, reach, reached } = watching(accounts);
    await reached("online");

    net.down = true;
    await standIn.revoke(grant.id);
    await reached("unreachable");
    expect(reach()).toEqual({ kind: "unreachable" });
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
    expect(accounts.getSnapshot().lost).toEqual([]);
    expect(await accounts.observed()?.computers()).toHaveLength(1);
    expect(vault.credentials()).toHaveLength(1);

    net.down = false;
    await reached("gone");
    expect(reach()).toEqual({ kind: "gone", why: "tookBack" });
    expect(await accounts.observed()?.computers()).toEqual([]);
    presence.stop();
  }, 30_000);

  test("left while the server is being asked: nothing more is said", async () => {
    const { accounts, grant } = await paired(standIn, "Left");
    let answer = () => {};
    let asking = false;
    const presence = new Presence({
      open: (host: HostId) => accounts.openChannel(host),
      explain: async (host: HostId) => {
        asking = true;
        await new Promise<void>((resolve) => {
          answer = resolve;
        });
        return accounts.explain(host);
      },
    });
    presence.watch([standIn.host]);
    while (presence.getSnapshot().get(standIn.host)?.kind !== "online") await Bun.sleep(10);

    await standIn.revoke(grant.id);
    while (!asking) await Bun.sleep(10);
    presence.stop();
    answer();
    await Bun.sleep(100);
    expect(presence.getSnapshot().size).toBe(0);
  }, 30_000);

  test("the server is asked with access refreshed: lapsed is not signed out", async () => {
    const net = network();
    const { accounts, grant } = await paired(standIn, "Lapsed", undefined, net.fetch);
    await standIn.revoke(grant.id);
    net.lapsed = true;
    expect(await accounts.explain(standIn.host)).toBe("tookBack");
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
  }, 30_000);

  test("while the server still counts it as paired: nothing goes", async () => {
    const { accounts } = await paired(standIn, "Still paired");
    expect(await accounts.explain(standIn.host)).toBe("undetermined");
    expect(await accounts.observed()?.computers()).toHaveLength(1);
    expect(accounts.getSnapshot().lost).toEqual([]);
  }, 30_000);

  // Last: the host is gone after it.
  test("taken off the account: told apart from taking the phone back", async () => {
    const { accounts } = await paired(standIn, "Host removed");
    const { presence, reach, reached } = watching(accounts);
    await reached("online");

    await standIn.removeHost();
    await reached("gone");
    expect(reach()).toEqual({ kind: "gone", why: "removed" });
    expect(accounts.getSnapshot().lost.map((lost) => lost.why)).toEqual(["removed"]);
    expect(await accounts.observed()?.computers()).toEqual([]);
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
    presence.stop();
  }, 30_000);
});
