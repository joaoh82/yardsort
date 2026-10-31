// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The client against the real server and a real host: the server's own code with its records in
// memory, the app's connector, and a program in a real terminal (`stand-in.ts`). Everything a
// phone does in R3 that the server and the protocol can show without a phone.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import {
  type ActivityBatch,
  type AttachmentEnd,
  type Channel,
  Client,
  ClientError,
  explainRevoked,
  type GrantSummary,
  openChannel,
  parsePairingLink,
  type Scope,
  Secret,
  Session,
  type TerminalId,
} from "../src/index.ts";
import { signIn, StandIn, START_TIMEOUT_MS, until } from "./stand-in.ts";

const ASKED: Scope[] = [
  "workspace:read",
  "activity:read",
  "memory:read",
  "terminal:read",
  "terminal:control",
];
const kept = () => {};

let standIn: StandIn;
let client: Client;

beforeAll(async () => {
  standIn = await StandIn.start();
  client = new Client(standIn.server);
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

/** A phone signed in to the account, paired with the stand-in's host and let in. */
async function pairedPhone(
  name: string,
  scopes?: Scope[],
): Promise<{ session: Session; grant: GrantSummary }> {
  const session = new Session(client, await signIn(client, name), kept);
  const { code } = await standIn.offer();
  const asked = await client.pair(await session.access(), {
    offer: { kind: "code", value: new Secret(code) },
    scopes: ASKED,
  });
  expect(asked.state).toBe("pending");
  const pending = await standIn.pending();
  expect(pending.id).toBe(asked.id);
  await standIn.approve(asked.id, scopes);
  const grant = await until(
    async () => client.grant(await session.access(), asked.id),
    (g) => g.state !== "pending",
  );
  expect(grant.state).toBe("approved");
  return { session, grant };
}

async function channelOf(session: Session): Promise<Channel> {
  return openChannel(client, await session.access(), standIn.host, {
    client: "remote-client-test/1",
  });
}

/** The program the stand-in runs, as the host lists it. */
async function program(channel: Channel): Promise<TerminalId> {
  const page = await channel.workspaces();
  const work = page.workspaces.find((w) => w.name === "work");
  if (!work) throw new Error("no workspace");
  const sessions = await channel.sessions({ workspace: work.id });
  const found = sessions.terminals.find((t) => t.terminal.activity !== "exited");
  if (!found) throw new Error("no running terminal");
  return found.terminal.id;
}

describe("signing in", () => {
  test("comes back through the app's own redirect, and the device is listed as this one", async () => {
    const server = await client.describe();
    expect(server.name).toBe("Test server");
    const issued = await signIn(client, "Signing-in phone");
    const page = await client.devices(issued.access);
    const me = page.devices.find((d) => d.current);
    expect(me).toMatchObject({
      id: issued.device,
      name: "Signing-in phone",
      kind: "phone",
      platform: "ios",
    });
  });

  test("resuming spends the credential kept, and keeps the one issued in its place", async () => {
    const issued = await signIn(client, "Refreshing phone");
    const keptHere: string[] = [];
    const session = await Session.resume(client, issued.refresh, (r) => {
      keptHere.push(r.expose());
    });
    expect(keptHere).toEqual([session.refreshCredential.expose()]);
    expect((await client.devices(await session.access())).devices.length).toBeGreaterThan(0);
  });
});

describe("pairing", () => {
  test("by code: asked, approved at the computer, and listed among its grants", async () => {
    const { session, grant } = await pairedPhone("Code phone");
    expect(grant.hostName).toBe(standIn.hostName);
    expect(grant.scopes).toEqual(ASKED);
    const mine = await client.grants(await session.access());
    expect(mine.grants.map((g) => g.id)).toContain(grant.id);
    const hosts = await client.hosts(await session.access());
    expect(hosts.hosts.map((h) => h.id)).toContain(standIn.host);
  });

  test("by the QR code's link, which names this server", async () => {
    const session = new Session(client, await signIn(client, "Link phone"), kept);
    const link = parsePairingLink((await standIn.offer()).link);
    if (!link) throw new Error("not a link");
    expect(new Client(link.server).address).toBe(client.address);
    const asked = await client.pair(await session.access(), {
      offer: { kind: "secret", value: link.secret },
      scopes: ["workspace:read"],
    });
    expect(asked.requested).toEqual(["workspace:read"]);
  });

  test("a denied request says so", async () => {
    const session = new Session(client, await signIn(client, "Denied phone"), kept);
    const { code } = await standIn.offer();
    const asked = await client.pair(await session.access(), {
      offer: { kind: "code", value: new Secret(code) },
      scopes: ASKED,
    });
    await standIn.deny(asked.id);
    expect((await client.grant(await session.access(), asked.id)).state).toBe("denied");
  });

  test("another account's code finds nothing", async () => {
    await standIn.signInAs("someone-else");
    try {
      const stranger = await signIn(client, "Stranger's phone");
      const { code } = await standIn.offer();
      const refused = await client
        .pair(stranger.access, { offer: { kind: "code", value: new Secret(code) }, scopes: ASKED })
        .catch((e: unknown) => e);
      expect(refused).toBeInstanceOf(ClientError);
      expect((refused as ClientError).kind).toBe("remote");
    } finally {
      await standIn.signInAs("someone");
    }
  });
});

describe("through the relay", () => {
  test("watches the program from a snapshot, types into it, and resumes where it left off", async () => {
    const { session } = await pairedPhone("Typing phone");
    const channel = await channelOf(session);
    expect(channel.host.name).toBe(standIn.hostName);
    expect(channel.host.capabilities).toContain("terminal.attach");
    const terminal = await program(channel);

    let drawn = "";
    const seqs: number[] = [];
    const watcher = {
      output: (bytes: Uint8Array, at: { seq: number }) => {
        drawn += new TextDecoder().decode(bytes);
        if (seqs.at(-1) !== at.seq) seqs.push(at.seq);
      },
    };
    const attachment = await channel.attach(terminal, watcher);
    expect(attachment.start).toBe("snapshot");
    expect(attachment.size).toEqual({ cols: 80, rows: 24 });

    const control = await channel.acquire(terminal);
    expect(control.lease.generation).toBeGreaterThan(0);
    await control.input("hello-from-typescript\r");
    await until(
      async () => drawn,
      (d) => d.includes("got:hello-from-typescript"),
    );
    // A line as the host's own terminal drew it, which on Windows can end in escapes.
    expect((await standIn.typed()).some((l) => l.startsWith("hello-from-typescript"))).toBe(true);
    await control.release();

    // Away and back, as after a dropped connection: from the last position drawn.
    const left = attachment.drawn;
    if (!left) throw new Error("nothing drawn");
    await attachment.detach();
    const again = await channel.attach(terminal, watcher, left);
    expect(again.start).toBe("resumed");
    expect(seqs).toEqual(seqs.map((_, i) => (seqs[0] as number) + i));
    channel.close();
    expect((await channel.ended).reason).toBe("closedHere");
  });

  test("follows activity as it is recorded", async () => {
    const { session } = await pairedPhone("Following phone");
    const channel = await channelOf(session);
    const kinds: string[] = [];
    const heard = new Promise<void>((resolve) => {
      void channel.subscribe(
        {},
        {
          events: (batch: ActivityBatch) => {
            kinds.push(...batch.events.map((e) => e.kind));
            if (kinds.includes("process.exited")) resolve();
          },
        },
      );
    });
    await Bun.sleep(100);
    await standIn.run();
    await heard;
    expect(kinds).toContain("process.started");
    channel.close();
  });

  test("a viewer that may not type is refused control", async () => {
    const { session } = await pairedPhone("Watching phone", ["workspace:read", "terminal:read"]);
    const channel = await channelOf(session);
    const refused = await channel.acquire(await program(channel)).catch((e: unknown) => e);
    expect((refused as ClientError).code).toBe("forbidden");
    channel.close();
  });
});

describe("revoked", () => {
  test("a grant the computer took back: only that computer is unpaired, and the phone stays signed in", async () => {
    const { session, grant } = await pairedPhone("Unpaired phone");
    const channel = await channelOf(session);
    let ended: AttachmentEnd | undefined;
    await channel.attach(await program(channel), {
      output: () => {},
      ended: (why) => (ended = why),
    });
    await standIn.revoke(grant.id);
    const end = await channel.ended;
    expect(end).toMatchObject({ reason: "revoked", code: 4003 });
    expect(ended?.kind).toBe("closed");
    expect(await explainRevoked(session, grant.id, grant.host)).toBe("unpaired");
    expect((await client.devices(await session.access())).devices.length).toBeGreaterThan(0);
  });

  test("a phone revoked from another of the account's devices is signed out", async () => {
    const { session, grant } = await pairedPhone("Revoked phone");
    const channel = await channelOf(session);
    const other = await signIn(client, "Other device", "computer");
    const listed = await client.devices(other.access);
    expect(listed.devices.find((d) => d.id === session.device)?.revokedAt).toBeNull();
    await client.revokeDevice(other.access, session.device);
    expect((await channel.ended).reason).toBe("revoked");
    expect(await explainRevoked(session, grant.id, grant.host)).toBe("signedOut");
    const after = await client.devices(other.access);
    expect(after.devices.find((d) => d.id === session.device)?.revokedAt).not.toBeNull();
  });

  test("a server that cannot be asked deletes nothing", async () => {
    const { session, grant } = await pairedPhone("Unreachable phone");
    const elsewhere = new Session(
      new Client("http://127.0.0.1:9"),
      {
        device: session.device,
        access: await session.access(),
        accessExpiresInMs: 15 * 60_000,
        refresh: session.refreshCredential,
      },
      kept,
    );
    expect(await explainRevoked(elsewhere, grant.id, grant.host)).toBe("undetermined");
  });

  // Last: the host is gone after it.
  test("a computer taken off the account unpairs it", async () => {
    const { session, grant } = await pairedPhone("Last phone");
    const channel = await channelOf(session);
    await standIn.removeHost();
    expect((await channel.ended).reason).toBe("revoked");
    expect(await explainRevoked(session, grant.id, grant.host)).toBe("unpaired");
  });
});
