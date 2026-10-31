// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Signing in, staying signed in and signing out, against the real server and its sign-in through
// the stand-in identity provider (`crates/server/examples/stand-in.rs`, which the protocol
// client's tests run too). Only the phone is pretend: a map for its credential store, and a
// browser that follows the server's redirects by hand.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { Client, type DeviceId, webCrypto } from "@yardsort/remote-client";

import {
  signIn as signInElsewhere,
  StandIn,
  START_TIMEOUT_MS,
} from "../../../../packages/remote-client/test/stand-in.ts";
import { Accounts, type Browser, type Env } from "../../src/lib/accounts";
import { MemoryVault } from "./memory-vault";

let standIn: StandIn;
let second: StandIn;

beforeAll(async () => {
  [standIn, second] = await Promise.all([StandIn.start(), StandIn.start()]);
}, START_TIMEOUT_MS);

afterAll(async () => {
  await Promise.all([standIn?.stop(), second?.stop()]);
});

/** The system browser, as far as sign-in needs one: the provider signs whoever comes in at once. */
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

/** The phone's network: every request it makes, by method and path, and a switch to lose it. */
function network() {
  const asked: string[] = [];
  const net = {
    asked,
    down: false,
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      const url = new URL(typeof input === "string" ? input : input.toString());
      asked.push(`${init?.method ?? "GET"} ${url.pathname}`);
      if (net.down) throw new TypeError("Network request failed");
      return fetch(input, init);
    }) as typeof fetch,
  };
  return net;
}

function phone(vault = new MemoryVault()) {
  const net = network();
  const env: Env = {
    crypto: webCrypto() as NonNullable<ReturnType<typeof webCrypto>>,
    platform: "ios",
    userAgent: "yardsort-mobile/test (ios)",
    fetch: net.fetch,
  };
  return { accounts: new Accounts(env, vault), vault, net };
}

async function signedIn(server = standIn, name = "Ana’s phone") {
  const it = phone();
  await it.accounts.load();
  const found = await it.accounts.lookUp(server.server);
  expect(await it.accounts.signIn(found, name, browser)).toBe(true);
  return it;
}

/** Another of the account's devices, a computer, signed in at `server`. */
async function elsewhere(server = standIn) {
  const client = new Client(server.server);
  const credentials = await signInElsewhere(client, "Workstation", "computer");
  return { client, access: credentials.access };
}

describe("signing in", () => {
  test("goes through the browser, names this phone, and keeps the credential", async () => {
    const { accounts, vault } = phone();
    await accounts.load();
    expect(accounts.getSnapshot()).toMatchObject({ loaded: true, active: null, standing: null });

    const found = await accounts.lookUp(standIn.server);
    expect(found.description.name.length).toBeGreaterThan(0);
    expect(await accounts.signIn(found, "  Ana’s phone ", browser)).toBe(true);

    const snapshot = accounts.getSnapshot();
    expect(snapshot.standing?.kind).toBe("signedIn");
    expect(snapshot.active).toMatchObject({
      address: standIn.server,
      server: found.description.server,
      deviceName: "Ana’s phone",
    });
    expect(vault.credentials()).toHaveLength(1);

    const me = (await accounts.devices()).devices.find((d) => d.current);
    expect(me).toMatchObject({ name: "Ana’s phone", kind: "phone", platform: "ios" });
    expect(me?.id).toBe(snapshot.active?.device as DeviceId);
  });

  test("a browser closed before the end keeps nothing", async () => {
    const { accounts, vault } = phone();
    await accounts.load();
    const found = await accounts.lookUp(standIn.server);
    expect(await accounts.signIn(found, "Ana’s phone", async () => null)).toBe(false);
    expect(accounts.getSnapshot().active).toBeNull();
    expect(vault.kept.size).toBe(0);
  });

  test("a sign-in that cannot be kept is undone at the server", async () => {
    const { accounts, vault } = phone();
    await accounts.load();
    const found = await accounts.lookUp(standIn.server);
    vault.failing = /^yardsort\.refresh\./;
    await expect(accounts.signIn(found, "Unkept phone", browser)).rejects.toThrow();
    expect(accounts.getSnapshot().active).toBeNull();
    const other = await elsewhere();
    const unkept = (await other.client.devices(other.access)).devices.filter(
      (d) => d.name === "Unkept phone",
    );
    expect(unkept).toHaveLength(1);
    expect(unkept[0]?.revokedAt).not.toBeNull();
  });
});

describe("opening again", () => {
  test("after a restart: the server is asked who it is before the credential goes to it", async () => {
    const { vault, accounts: first } = await signedIn();
    const device = first.getSnapshot().active?.device;

    const { accounts, net } = phone(vault);
    await accounts.load();
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
    expect(net.asked).toEqual(["GET /v1/server", "POST /v1/token"]);
    expect((await accounts.devices()).devices.find((d) => d.current)?.id).toBe(device as DeviceId);
  });

  test("twice at once spends the credential once", async () => {
    const { vault } = await signedIn();
    const { accounts, net } = phone(vault);
    await Promise.all([accounts.load(), accounts.load(), accounts.load()]);
    await Promise.all([accounts.open(), accounts.open()]);
    expect(net.asked.filter((r) => r === "POST /v1/token")).toHaveLength(1);
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
  });

  test("an address that now answers as another server is sent nothing", async () => {
    const { vault } = await signedIn();
    // The profile was signed in to a server this address no longer is.
    const index = JSON.parse(vault.kept.get("yardsort.profiles") as string);
    index.profiles[0].server = "some-other-server";
    vault.kept.set("yardsort.profiles", JSON.stringify(index));

    const { accounts, net } = phone(vault);
    await accounts.load();
    expect(accounts.getSnapshot().standing).toMatchObject({ kind: "otherServer" });
    expect(net.asked).toEqual(["GET /v1/server"]);
    // Nothing is deleted by itself; the person removes it.
    expect(vault.credentials()).toHaveLength(1);
    await accounts.forgetHere(accounts.getSnapshot().active?.id as string);
    expect(vault.kept.get("yardsort.profiles")).not.toContain("some-other-server");
    expect(vault.credentials()).toEqual([]);
  });

  test("a server that cannot be reached deletes nothing, and is tried again", async () => {
    const { vault } = await signedIn();
    const { accounts, net } = phone(vault);
    net.down = true;
    await accounts.load();
    expect(accounts.getSnapshot().standing).toMatchObject({ kind: "unreachable" });
    expect(vault.credentials()).toHaveLength(1);

    net.down = false;
    await accounts.open();
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
  });
});

describe("signed out", () => {
  test("from another device: the profile is deleted on opening, and the phone says so", async () => {
    const { vault, accounts: first } = await signedIn();
    const device = first.getSnapshot().active?.device as DeviceId;
    const other = await elsewhere();
    await other.client.revokeDevice(other.access, device);

    const { accounts } = phone(vault);
    await accounts.load();
    const snapshot = accounts.getSnapshot();
    expect(snapshot.active).toBeNull();
    expect(snapshot.profiles).toEqual([]);
    expect(snapshot.notice).toContain("signed out");
    expect(vault.credentials()).toEqual([]);
  });

  test("from another device while open: the next request finds out", async () => {
    const { accounts, vault } = await signedIn();
    const device = accounts.getSnapshot().active?.device as DeviceId;
    const other = await elsewhere();
    await other.client.revokeDevice(other.access, device);

    await expect(accounts.devices()).rejects.toThrow();
    expect(accounts.getSnapshot().active).toBeNull();
    expect(accounts.getSnapshot().notice).toContain("signed out");
    expect(vault.credentials()).toEqual([]);
  });

  test("here: the phone is revoked at the server and deleted from the phone", async () => {
    const { accounts, vault } = await signedIn();
    const device = accounts.getSnapshot().active?.device as DeviceId;
    await accounts.signOut();
    expect(accounts.getSnapshot()).toMatchObject({ active: null, profiles: [], notice: null });
    expect(vault.kept.get("yardsort.profiles")).not.toContain(device);
    expect(vault.credentials()).toEqual([]);

    const other = await elsewhere();
    const gone = (await other.client.devices(other.access)).devices.find((d) => d.id === device);
    expect(gone?.revokedAt).not.toBeNull();
  });

  test("here, with the server out of reach: nothing is deleted until the person says", async () => {
    const { accounts, vault, net } = await signedIn();
    net.down = true;
    await expect(accounts.signOut()).rejects.toThrow();
    expect(accounts.getSnapshot().active).not.toBeNull();
    expect(vault.credentials()).toHaveLength(1);
  });
});

describe("two servers", () => {
  test("each keeps its own sign-in, and switching back needs no new one", async () => {
    const { accounts, vault, net } = await signedIn(standIn, "Ana’s phone");
    const firstId = accounts.getSnapshot().active?.id as string;
    expect(
      await accounts.signIn(await accounts.lookUp(second.server), "Ana’s phone", browser),
    ).toBe(true);
    const snapshot = accounts.getSnapshot();
    expect(snapshot.profiles.map((p) => p.address)).toEqual([standIn.server, second.server]);
    expect(snapshot.active?.address).toBe(second.server);
    expect(vault.credentials()).toHaveLength(2);

    // Each server knows only its own sign-in of this phone.
    const devicesHere = (await accounts.devices()).devices.filter((d) => d.current);
    expect(devicesHere).toHaveLength(1);
    expect(devicesHere[0]?.id).toBe(snapshot.active?.device as DeviceId);

    const before = net.asked.length;
    await accounts.activate(firstId);
    expect(accounts.getSnapshot().active?.address).toBe(standIn.server);
    expect(net.asked.slice(before)).toEqual([]);
    expect((await accounts.devices()).devices.find((d) => d.current)?.id).toBe(
      accounts.getSnapshot().active?.device as DeviceId,
    );
  });
});
