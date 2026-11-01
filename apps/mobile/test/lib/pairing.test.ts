// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Pairing with a computer and reaching it, against the real server, relay and host through the
// stand-in (`crates/server/examples/stand-in.rs`). The person at the computer is its commands.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import {
  type Channel,
  ClientError,
  type GrantSummary,
  type HostId,
  parsePairingLink,
  Secret,
  webCrypto,
} from "@yardsort/remote-client";

import { StandIn, START_TIMEOUT_MS } from "../../../../packages/remote-client/test/stand-in.ts";
import { ASKED_SCOPES, Accounts, type Browser, type Env } from "../../src/lib/accounts";
import { redirectSystemPath } from "../../src/app/+native-intent";
import { Incoming, incoming, placeOf, waitForAnswer } from "../../src/lib/pairing";
import type { Profile } from "../../src/lib/profiles";
import { Presence, type Reach } from "../../src/lib/presence";
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

async function signedIn(name = "Ana’s phone") {
  const env: Env = {
    crypto: webCrypto() as NonNullable<ReturnType<typeof webCrypto>>,
    platform: "ios",
    userAgent: "yardsort-mobile/test (ios)",
  };
  const accounts = new Accounts(env, new MemoryVault());
  await accounts.load();
  expect(await accounts.signIn(await accounts.lookUp(standIn.server), name, browser)).toBe(true);
  return accounts;
}

const FAST = { everyMs: 20 };

/** A phone the computer has said yes to. */
async function paired(name = "Paired phone") {
  const accounts = await signedIn(name);
  const { code } = await standIn.offer();
  const asked = await accounts.pair({ kind: "code", value: new Secret(code) });
  const grant = await standIn.pending();
  expect(grant.id).toBe(asked.id);
  await standIn.approve(asked.id);
  return { accounts, grant: asked };
}

/** Presence, and a way to wait for a computer to reach a state. */
function watching(accounts: Accounts) {
  const presence = new Presence({
    open: (host) => accounts.openChannel(host),
    offlineRetryMs: 50,
    firstRetryMs: 50,
    maxRetryMs: 100,
  });
  const seen: Reach["kind"][] = [];
  presence.subscribe(() => {
    const kind = presence.getSnapshot().get(standIn.host)?.kind;
    if (kind && seen.at(-1) !== kind) seen.push(kind);
  });
  const reached = async (kind: Reach["kind"]) => {
    const deadline = Date.now() + 10_000;
    while (presence.getSnapshot().get(standIn.host)?.kind !== kind) {
      if (Date.now() > deadline) throw new Error(`never ${kind}; saw ${seen.join(", ")}`);
      await Bun.sleep(10);
    }
  };
  return { presence, seen, reached };
}

describe("pairing", () => {
  test("by the typed code: it asks, the computer says yes, and the computer is listed", async () => {
    const accounts = await signedIn("Typing phone");
    expect((await accounts.computers()).computers).toEqual([]);

    const { code } = await standIn.offer();
    const asked = await accounts.pair({ kind: "code", value: new Secret(code) });
    expect(asked.state).toBe("pending");
    expect(asked.requested).toEqual(expect.arrayContaining(ASKED_SCOPES));
    const waiting = await accounts.computers();
    expect(waiting.computers).toEqual([]);
    expect(waiting.waiting.map((g) => g.id)).toEqual([asked.id]);

    const answer = waitForAnswer(accounts, asked, FAST);
    await standIn.approve((await standIn.pending()).id, ["workspace:read", "terminal:read"]);
    expect(await answer).toMatchObject({
      state: "approved",
      scopes: ["workspace:read", "terminal:read"],
    });

    const after = await accounts.computers();
    expect(after.waiting).toEqual([]);
    expect(after.computers).toHaveLength(1);
    expect(after.computers[0]).toMatchObject({
      host: standIn.host,
      name: standIn.hostName,
      grant: asked.id,
    });
    await accounts.unpair(asked.id);
  });

  test("by the QR code's link", async () => {
    const accounts = await signedIn("Scanning phone");
    const link = parsePairingLink((await standIn.offer()).link);
    if (!link) throw new Error("the stand-in's link did not read");
    const active = accounts.getSnapshot().active;
    expect(placeOf(link, active, accounts.getSnapshot().profiles)).toEqual({ kind: "here" });
    const asked = await accounts.pair({ kind: "secret", value: link.secret });
    const pending = await standIn.pending();
    expect(pending.id).toBe(asked.id);
    await standIn.deny(asked.id);
  });

  test("a no at the computer is told, and nothing is listed", async () => {
    const accounts = await signedIn("Refused phone");
    const { code } = await standIn.offer();
    const asked = await accounts.pair({ kind: "code", value: new Secret(code) });
    const answer = waitForAnswer(accounts, asked, FAST);
    await standIn.deny((await standIn.pending()).id);
    expect((await answer)?.state).toBe("denied");
    expect(await accounts.computers()).toEqual({ computers: [], waiting: [] });
  });

  test("a code that is wrong says so, and the phone stays signed in", async () => {
    const accounts = await signedIn("Mistyping phone");
    const refused = await accounts
      .pair({ kind: "code", value: new Secret("ZZZZ-ZZZZ") })
      .catch((e: unknown) => e);
    expect(refused).toBeInstanceOf(ClientError);
    expect((refused as ClientError).code).toBe("not_found");
    expect((refused as ClientError).message).toContain("wrong or has expired");
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
  });

  test("an offer goes once: its code does not work a second time", async () => {
    const first = await signedIn("First phone");
    const { code } = await standIn.offer();
    const asked = await first.pair({ kind: "code", value: new Secret(code) });
    const second = await signedIn("Second phone");
    await expect(second.pair({ kind: "code", value: new Secret(code) })).rejects.toThrow(
      "wrong or has expired",
    );
    await first.unpair(asked.id);
  });

  test("a request taken back here stops waiting at the computer", async () => {
    const accounts = await signedIn("Changed its mind");
    const { code } = await standIn.offer();
    const asked = await accounts.pair({ kind: "code", value: new Secret(code) });
    expect((await standIn.pending()).id).toBe(asked.id);
    await accounts.unpair(asked.id);
    expect((await accounts.grant(asked.id)).state).toBe("revoked");
    await expect(standIn.pending()).rejects.toThrow("nobody asked");
    expect((await accounts.computers()).waiting).toEqual([]);
  }, 15_000);

  test("unpairing here takes the computer off the list; the phone stays signed in", async () => {
    const { accounts, grant } = await paired("Unpairing phone");
    expect((await accounts.computers()).computers).toHaveLength(1);
    await accounts.unpair(grant.id);
    expect((await accounts.computers()).computers).toEqual([]);
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");
  });
});

describe("waiting for the answer", () => {
  const pending = (over: Partial<GrantSummary> = {}) =>
    ({ id: "g", state: "pending", expiresInMs: 60_000, ...over }) as GrantSummary;
  const unreachable = () => new ClientError("unreachable", "Could not reach the server.");

  test("a server out of reach for a moment is asked again", async () => {
    let asked = 0;
    const answer = await waitForAnswer(
      {
        grant: async () => {
          asked += 1;
          if (asked === 1) throw unreachable();
          return asked === 2 ? pending() : pending({ state: "approved" });
        },
      },
      pending(),
      { everyMs: 1 },
    );
    expect(answer?.state).toBe("approved");
    expect(asked).toBe(3);
  });

  test("out of reach until the request has run out: it says so", async () => {
    let now = 0;
    const waiting = waitForAnswer(
      {
        grant: async () => {
          now += 40_000;
          throw unreachable();
        },
      },
      pending(),
      { everyMs: 1, clock: () => now },
    );
    await expect(waiting).rejects.toThrow("Could not reach");
  });

  test("anything but being out of reach stops at once", async () => {
    const waiting = waitForAnswer(
      {
        grant: async () => {
          throw new ClientError("invalid", "Not signed in.");
        },
      },
      pending(),
      { everyMs: 1 },
    );
    await expect(waiting).rejects.toThrow("Not signed in");
  });

  test("leaving stops the asking", async () => {
    const stop = new AbortController();
    let asked = 0;
    const waiting = waitForAnswer(
      {
        grant: async () => {
          asked += 1;
          stop.abort();
          return pending();
        },
      },
      pending(),
      { everyMs: 1, signal: stop.signal },
    );
    expect(await waiting).toBeNull();
    expect(asked).toBe(1);
  });
});

describe("a link", () => {
  const link = (server: string) => ({ server, secret: new Secret("ys1.of.a") });
  const profile = (address: string) => ({ id: "p", address, serverName: "Elsewhere" }) as Profile;
  const here = profile("https://here.example.com/");
  const there = profile("https://there.example.com/");

  test("to the server in use, however its address is written", () => {
    expect(placeOf(link("https://HERE.example.com"), here, [here])).toEqual({ kind: "here" });
  });

  test("to another this phone is signed in to names it, and nothing switches by itself", () => {
    expect(placeOf(link("https://there.example.com"), here, [here, there])).toEqual({
      kind: "known",
      profile: there,
    });
  });

  test("to one this phone is not signed in to, with or without a profile in use", () => {
    const address = "https://new.example.com/";
    expect(placeOf(link("https://new.example.com"), here, [here])).toEqual({
      kind: "new",
      address,
    });
    expect(placeOf(link("https://new.example.com"), null, [])).toEqual({ kind: "new", address });
  });

  test("to an address the app would not talk to goes nowhere", () => {
    for (const bad of ["http://plain.example.com", "not an address", "ftp://x.example.com"]) {
      expect(placeOf(link(bad), here, [here])).toEqual({ kind: "invalid" });
    }
  });

  test("is handed out once, however many ask, and a newer one can go", () => {
    const held = new Incoming();
    expect(held.claim()).toBeNull();
    const first = link("https://a.example.com");
    held.hold(first, true);
    expect(held.claim()).toBe(first);
    expect(held.claim()).toBeNull();
    // Agreed to while being sent is the same link.
    held.agree();
    expect(held.claim()).toBeNull();
    const second = link("https://b.example.com");
    held.hold(second, true);
    expect(held.claim()).toBe(second);
  });

  test("finishing a link lets it go, and leaves a newer one that replaced it", () => {
    const held = new Incoming();
    const first = link("https://a.example.com");
    held.hold(first, true);
    held.claim();
    const second = link("https://b.example.com");
    held.hold(second, false);
    held.finish(first);
    expect(held.get()?.link).toBe(second);
    held.finish(second);
    expect(held.get()).toBeNull();
  });

  test("is held until used, and a newer one replaces it", () => {
    const held = new Incoming();
    let told = 0;
    held.subscribe(() => (told += 1));
    expect(held.get()).toBeNull();
    held.hold(link("https://a.example.com"), false);
    expect(held.get()?.agreed).toBe(false);
    const before = held.get();
    held.agree();
    expect(held.get()?.agreed).toBe(true);
    expect(held.get()).not.toBe(before);
    held.hold(link("https://b.example.com"), false);
    expect(held.get()).toMatchObject({ agreed: false, link: { server: "https://b.example.com" } });
    held.drop();
    expect(held.get()).toBeNull();
    expect(told).toBe(4);
  });
});

describe("a link the system hands the app", () => {
  test("is taken before the router reads it, so its secret is never in a route", () => {
    const secret = "ys1.of.private";
    const path = `yardsort://pair?server=https%3A%2F%2Fa.example.com&secret=${secret}`;
    const route = redirectSystemPath({ path, initial: true });
    expect(route).toBe("/pair");
    expect(route).not.toContain(secret);
    // Asked about before it is used: whoever sent it is unknown.
    expect(incoming.get()?.agreed).toBe(false);
    expect(incoming.get()?.link.secret.expose()).toBe(secret);
    incoming.drop();
  });

  test("anything else goes to the router as it came, a sign-in's return among them", () => {
    for (const path of [
      "yardsort://signed-in?code=abc",
      "yardsort://pair?server=only",
      "/account",
    ]) {
      expect(redirectSystemPath({ path, initial: false })).toBe(path);
    }
    expect(incoming.get()).toBeNull();
  });
});

describe("whether a computer can be reached", () => {
  test("online is a channel held, offline is the relay saying so, and it comes back", async () => {
    const { accounts, grant } = await paired("Watching phone");
    const { presence, reached, seen } = watching(accounts);
    presence.watch([standIn.host]);
    await reached("online");
    expect(seen[0]).toBe("connecting");

    await standIn.hostOffline();
    try {
      await reached("offline");
    } finally {
      await standIn.hostOnline();
    }
    await reached("online");
    // The relay said so as the channel closed: no time spent looking as if it might come back.
    expect(seen.slice(0, 3)).toEqual(["connecting", "online", "offline"]);
    presence.stop();
    await accounts.unpair(grant.id);
  }, 30_000);

  test("a computer offline before anyone looks is offline, not an error", async () => {
    const { accounts, grant } = await paired("Late phone");
    await standIn.hostOffline();
    const { presence, reached, seen } = watching(accounts);
    try {
      presence.watch([standIn.host]);
      await reached("offline");
      expect(seen).not.toContain("gone");
    } finally {
      presence.stop();
      await standIn.hostOnline();
    }
    await accounts.unpair(grant.id);
  }, 30_000);

  test("the computer taking back its pairing is told, and not asked about again", async () => {
    const { accounts, grant } = await paired("Dropped phone");
    const { presence, reached } = watching(accounts);
    presence.watch([standIn.host]);
    await reached("online");

    await standIn.revoke(grant.id);
    await reached("gone");
    expect((await accounts.computers()).computers).toEqual([]);
    expect(accounts.getSnapshot().standing?.kind).toBe("signedIn");

    // Still on the screen's old list, it is left alone until the next start.
    let opened = 0;
    const counted = new Presence({
      open: (host: HostId) => {
        opened += 1;
        return accounts.openChannel(host);
      },
      firstRetryMs: 10,
    });
    counted.watch([standIn.host]);
    const deadline = Date.now() + 10_000;
    while (counted.getSnapshot().get(standIn.host)?.kind !== "gone") {
      if (Date.now() > deadline) throw new Error("never gone");
      await Bun.sleep(10);
    }
    counted.watch([standIn.host]);
    await Bun.sleep(50);
    expect(opened).toBe(1);
    counted.stop();
    presence.stop();
  }, 30_000);

  test("stopping closes the channel, and starting again opens a new one", async () => {
    const { accounts, grant } = await paired("Backgrounded phone");
    const channels: Channel[] = [];
    const presence = new Presence({
      open: async (host) => {
        const channel = await accounts.openChannel(host);
        channels.push(channel);
        return channel;
      },
    });
    const online = async () => {
      const deadline = Date.now() + 10_000;
      while (presence.getSnapshot().get(standIn.host)?.kind !== "online") {
        if (Date.now() > deadline) throw new Error("never online");
        await Bun.sleep(10);
      }
    };
    presence.watch([standIn.host]);
    await online();
    expect(channels[0]?.open).toBe(true);

    presence.stop();
    expect(presence.getSnapshot().size).toBe(0);
    expect(channels[0]?.open).toBe(false);
    expect((await channels[0]?.ended)?.reason).toBe("closedHere");

    presence.watch([standIn.host]);
    await online();
    expect(channels).toHaveLength(2);
    expect(channels[1]?.open).toBe(true);
    presence.stop();
    expect(channels[1]?.open).toBe(false);
    await accounts.unpair(grant.id);
  }, 30_000);

  test("a computer no longer on the list stops being watched", async () => {
    const { accounts, grant } = await paired("Tidy phone");
    const { presence, reached } = watching(accounts);
    presence.watch([standIn.host]);
    await reached("online");
    presence.watch([]);
    expect(presence.getSnapshot().has(standIn.host)).toBe(false);
    presence.stop();
    await accounts.unpair(grant.id);
  }, 30_000);
});
