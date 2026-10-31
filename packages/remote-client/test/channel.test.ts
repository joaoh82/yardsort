// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The channel against a relay and a computer that misbehave on purpose, over a real WebSocket on
// this machine: going quiet, skipping a number, closing with each of the relay's codes, sending
// what no computer sends. The well-behaved path, against the real server and host, is in
// `server.test.ts`.

import { afterEach, describe, expect, test } from "bun:test";
import type { ServerWebSocket } from "bun";

import {
  type AttachmentEnd,
  ChannelClosed,
  type ChannelEnd,
  Client,
  ClientError,
  type ControlLoss,
  encodeFrame,
  type HostId,
  openChannel,
  Secret,
  type TerminalId,
} from "../src/index.ts";

type Request = { requestId: string; op: string; params?: Record<string, unknown> };
type Host = (request: Request, socket: ServerWebSocket<Stage>) => void;
type Stage = { greeted: boolean; welcomed: boolean };

const servers: { stop(force?: boolean): void }[] = [];
afterEach(() => {
  for (const server of servers.splice(0)) server.stop(true);
});

const WELCOME = {
  welcome: {
    protocol: { major: 1, minor: 4 },
    host: {
      host: "h",
      name: "Pretend",
      platform: "linux",
      appVersion: "0",
      protocol: { major: 1, minor: 4 },
      capabilities: [],
    },
  },
};

/** A relay with one computer behind it, which answers each request as `host` says. */
function relay(host: Host, greeting: unknown = { connected: { host: "h" } }) {
  const asked: Request[] = [];
  const server = Bun.serve<Stage>({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request, server) {
      const path = new URL(request.url).pathname;
      if (path === "/v1/relay/tickets")
        return Response.json({ ticket: "a-ticket", expiresInMs: 60_000 });
      if (
        path === "/v1/relay" &&
        server.upgrade(request, { data: { greeted: false, welcomed: false } })
      ) {
        return undefined;
      }
      return new Response("no", { status: 404 });
    },
    websocket: {
      message(socket, message) {
        const value = JSON.parse(String(message)) as Record<string, unknown>;
        if (!socket.data.greeted) {
          expect(value).toEqual({ ticket: "a-ticket" });
          socket.data.greeted = true;
          socket.send(JSON.stringify(greeting));
          return;
        }
        if (!socket.data.welcomed) {
          expect(value.protocols).toEqual([1]);
          socket.data.welcomed = true;
          socket.send(JSON.stringify(WELCOME));
          return;
        }
        asked.push(value as Request);
        host(value as Request, socket);
      },
    },
  });
  servers.push(server);
  return { client: new Client(`http://127.0.0.1:${server.port}`), asked };
}

const answer = (socket: ServerWebSocket<Stage>, request: Request, result: unknown) =>
  socket.send(JSON.stringify({ requestId: request.requestId, result }));

const HOST = "h" as HostId;
const TERMINAL = "t" as TerminalId;
const access = new Secret("access");

describe("opening", () => {
  test("a relay that refuses says why", async () => {
    const { client } = relay(() => {}, {
      refused: { code: "forbidden", message: "Not let in.", retryable: false },
    });
    const error = await openChannel(client, access, HOST, { client: "test/1" }).catch(
      (e: unknown) => e,
    );
    expect((error as ClientError).code).toBe("forbidden");
  });

  test("a greeting from another computer is not taken", async () => {
    const { client } = relay(() => {}, { connected: { host: "someone-else" } });
    const error = await openChannel(client, access, HOST, { client: "test/1" }).catch(
      (e: unknown) => e,
    );
    expect((error as ClientError).kind).toBe("unexpected");
  });
});

describe("liveness", () => {
  test("a computer that stops answering is taken to be gone, and what was asked fails", async () => {
    const { client, asked } = relay(() => {});
    const channel = await openChannel(client, access, HOST, {
      client: "test/1",
      probeAfterMs: 100,
      probeTimeoutMs: 100,
    });
    const waiting = channel.workspaces().catch((e: unknown) => e);
    const end = await channel.ended;
    expect(end.reason).toBe("silent");
    expect(await waiting).toBeInstanceOf(ChannelClosed);
    expect(asked.map((r) => r.op)).toEqual(["workspace.list", "host.describe"]);
  });

  test("a computer that answers is asked again after each quiet spell, and kept", async () => {
    const { client, asked } = relay((request, socket) =>
      answer(socket, request, { kind: "host", ...WELCOME.welcome.host }),
    );
    const channel = await openChannel(client, access, HOST, {
      client: "test/1",
      probeAfterMs: 60,
      probeTimeoutMs: 60,
    });
    await Bun.sleep(400);
    expect(channel.open).toBe(true);
    expect(asked.filter((r) => r.op === "host.describe").length).toBeGreaterThanOrEqual(3);
    channel.close();
    expect((await channel.ended).reason).toBe("closedHere");
  });
});

describe("how a channel ends", () => {
  for (const [code, reason] of [
    [4003, "revoked"],
    [4004, "hostOffline"],
    [4008, "overloaded"],
    [4503, "restarting"],
    [4321, "unknown"],
    [1000, "gone"],
  ] as const) {
    test(`close code ${code} is ${reason}`, async () => {
      const { client } = relay((request, socket) => {
        if (request.op === "workspace.list") socket.close(code, "bye");
      });
      const channel = await openChannel(client, access, HOST, { client: "test/1" });
      const asked = channel.workspaces().catch((e: unknown) => e);
      const end: ChannelEnd = await channel.ended;
      expect(end.reason).toBe(reason);
      expect(end.code).toBe(code);
      if (reason === "unknown") expect(end.text).toContain(String(code));
      expect(((await asked) as ChannelClosed).end).toEqual(end);
    });
  }

  test("a message larger than any may be closes it as broken", async () => {
    const { client } = relay((_, socket) =>
      socket.send(JSON.stringify({ event: "x", pad: "a".repeat(70_000) })),
    );
    const channel = await openChannel(client, access, HOST, { client: "test/1" });
    void channel.workspaces().catch(() => {});
    expect((await channel.ended).reason).toBe("invalid");
  });

  test("an event or a frame from a newer computer is passed over, not taken as broken", async () => {
    const { client } = relay((request, socket) => {
      socket.send(JSON.stringify({ event: "terminal.someday", stream: 1 }));
      const future = encodeFrame({
        stream: 1,
        position: { epoch: 0, seq: 0 },
        bytes: new Uint8Array(),
      });
      future[1] = 2;
      socket.send(future);
      answer(socket, request, { kind: "workspaces", projects: [], workspaces: [], next: null });
    });
    const channel = await openChannel(client, access, HOST, { client: "test/1" });
    expect((await channel.workspaces()).workspaces).toEqual([]);
    expect(channel.open).toBe(true);
    channel.close();
  });
});

describe("a terminal's stream", () => {
  test("is drawn in order, a split piece included, and ends at the first gap", async () => {
    const frame = (seq: number, text: string) =>
      encodeFrame({
        stream: 3,
        position: { epoch: 0, seq },
        bytes: new TextEncoder().encode(text),
      });
    const { client, asked } = relay((request, socket) => {
      if (request.op !== "terminal.attach") return answer(socket, request, { kind: "done" });
      answer(socket, request, {
        kind: "attached",
        stream: 3,
        start: "snapshot",
        position: { epoch: 0, seq: 5 },
        size: { cols: 80, rows: 24 },
        controller: { kind: "local" },
      });
      for (const [seq, text] of [
        [5, "snap"],
        [5, "shot"],
        [6, "!"],
        [8, "lost"],
      ] as const) {
        socket.send(frame(seq, text));
      }
      socket.send(
        JSON.stringify({
          event: "terminal.resized",
          stream: 3,
          position: { epoch: 0, seq: 9 },
          size: { cols: 1, rows: 1 },
        }),
      );
    });
    const channel = await openChannel(client, access, HOST, { client: "test/1" });
    const drawn: string[] = [];
    const ended = new Promise<AttachmentEnd>((resolve) => {
      void channel.attach(TERMINAL, {
        output: (bytes) => drawn.push(new TextDecoder().decode(bytes)),
        ended: resolve,
      });
    });
    expect(await ended).toEqual({
      kind: "gap",
      expected: { epoch: 0, seq: 7 },
      got: { epoch: 0, seq: 8 },
    });
    expect(drawn).toEqual(["snap", "shot", "!"]);
    await Bun.sleep(50);
    // The computer is told to stop sending it.
    expect(asked.at(-1)).toMatchObject({
      op: "terminal.detach",
      params: { terminal: "t", stream: 3 },
    });
    channel.close();
  });

  test("a resumed stream goes on after the position it was given", async () => {
    const { client } = relay((request, socket) => {
      answer(socket, request, {
        kind: "attached",
        stream: 4,
        start: "resumed",
        position: { epoch: 2, seq: 41 },
        size: { cols: 80, rows: 24 },
        controller: { kind: "nobody" },
      });
      socket.send(
        encodeFrame({ stream: 4, position: { epoch: 2, seq: 42 }, bytes: Uint8Array.from([0x61]) }),
      );
    });
    const channel = await openChannel(client, access, HOST, { client: "test/1" });
    let seen = 0;
    const attachment = await channel.attach(
      TERMINAL,
      { output: () => (seen += 1) },
      { epoch: 2, seq: 41 },
    );
    expect(attachment.start).toBe("resumed");
    expect(attachment.drawn).toEqual({ epoch: 2, seq: 41 });
    await Bun.sleep(50);
    expect(seen).toBe(1);
    expect(attachment.drawn).toEqual({ epoch: 2, seq: 42 });
    channel.close();
  });
});

describe("control", () => {
  test("input is numbered within the lease; what was in flight when the channel closed is uncertain", async () => {
    const { client, asked } = relay((request, socket) => {
      if (request.op === "terminal.acquire") {
        return answer(socket, request, {
          kind: "lease",
          id: "l",
          generation: 7,
          ttlMs: 30_000,
          renewEveryMs: 10_000,
        });
      }
      if (request.op === "terminal.input" && request.params?.seq === 1)
        return answer(socket, request, { kind: "done" });
      socket.close(4004, "gone");
    });
    const channel = await openChannel(client, access, HOST, { client: "test/1" });
    const losses: ControlLoss[] = [];
    const control = await channel.acquire(TERMINAL, { lost: (why) => losses.push(why) });
    await control.input("ls\r");
    const uncertain = await control.input("rm\r").catch((e: unknown) => e);
    expect(uncertain).toBeInstanceOf(ChannelClosed);
    expect(asked.filter((r) => r.op === "terminal.input").map((r) => r.params)).toEqual([
      { terminal: "t", lease: "l", generation: 7, seq: 1, data: "ls\r", paste: false },
      { terminal: "t", lease: "l", generation: 7, seq: 2, data: "rm\r", paste: false },
    ]);
    expect(losses.map((l) => l.kind)).toEqual(["closed"]);
    // Nothing is sent again by itself: there is no channel to send it on, and no lease.
    expect(control.held).toBe(false);
  });

  test("a lease is renewed while held, and lost when the computer says so", async () => {
    let renewals = 0;
    const { client } = relay((request, socket) => {
      if (request.op === "terminal.acquire") {
        return answer(socket, request, {
          kind: "lease",
          id: "l",
          generation: 1,
          ttlMs: 300,
          renewEveryMs: 40,
        });
      }
      if (request.op === "terminal.renew" && ++renewals < 3) {
        return answer(socket, request, {
          kind: "lease",
          id: "l",
          generation: 1,
          ttlMs: 300,
          renewEveryMs: 40,
        });
      }
      socket.send(
        JSON.stringify({
          requestId: request.requestId,
          error: { code: "lease_lost", message: "Someone else has control.", retryable: false },
        }),
      );
    });
    const channel = await openChannel(client, access, HOST, { client: "test/1" });
    const lost = new Promise<ControlLoss>((resolve) => {
      void channel.acquire(TERMINAL, { lost: resolve });
    });
    const why = await lost;
    expect(why.kind === "lost" && why.error.code).toBe("lease_lost");
    expect(renewals).toBe(3);
    channel.close();
  });
});
