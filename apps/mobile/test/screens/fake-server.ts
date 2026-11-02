// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A server, as far as these screens ask one, answering through `fetch`. Its answers are read by
 * the real protocol client, so a shape it gets wrong fails there. What the real server does is
 * tested in `test/lib/`, against the real server.
 */

import { createHash, webcrypto } from "node:crypto";

import type { Crypto, Socket } from "@yardsort/remote-client";

import type { Env } from "../../src/lib/accounts";

export const ADDRESS = "https://remote.example.com/";

interface Device {
  id: string;
  name: string;
  kind: string;
  platform: string;
  createdAt: number;
  lastSeenAt: number | null;
  revokedAt: number | null;
}

interface Grant {
  id: string;
  host: string;
  hostName: string;
  device: string;
  deviceName: string;
  state: string;
  requested: string[];
  scopes: string[];
  projects: { kind: "all" } | null;
  createdAt: number;
  decidedAt: number | null;
  expiresInMs: number | null;
}

export class FakeServer {
  name = "Example Remote";
  server = "srv-example";
  /** The refresh credential in force for each device, by device id. */
  refresh = new Map<string, string>();
  devices: Device[] = [
    {
      id: "dev-workstation",
      name: "Workstation",
      kind: "computer",
      platform: "linux",
      createdAt: 1_700_000_000_000,
      lastSeenAt: 1_700_000_100_000,
      revokedAt: null,
    },
  ];
  /** Every request, by method and path. */
  asked: string[] = [];
  /** When set, the server cannot be reached. */
  down = false;
  /** When set, the device list is answered only after this. */
  devicesAfter: Promise<void> | null = null;
  /** The computers on the account. */
  hosts = [
    {
      id: "host-work",
      name: "Workstation",
      createdAt: 1_700_000_000_000,
      lastSeenAt: 1_700_000_100_000,
      current: false,
    },
  ];
  /** Every grant and request. */
  grants: Grant[] = [];
  /** What a pairing may redeem, by what it presents. Each goes once. */
  offers = new Set<string>(["ABCD2345"]);
  /** Every offer presented to this server, as it was presented. */
  presented: string[] = [];
  /** Offers answered only after their promise. */
  pairAfter = new Map<string, Promise<void>>();
  /** Computers the relay says are offline. */
  offline = new Set<string>();
  /** What a computer answers through a channel. */
  computer = {
    projects: [{ id: "project-1", name: "demo", missing: false }],
    workspaces: [
      {
        id: "workspace-1",
        project: "project-1",
        name: "fix-login",
        kind: "worktree",
        branch: { label: "ys/fix-login", detached: false, unborn: false },
        archived: false,
        missing: false,
        tasks: [],
        runningSessions: 1,
      },
    ],
    /** A second page of workspaces, when set. */
    moreWorkspaces: null as null | Record<string, unknown>[],
    sessions: [
      {
        id: "session-1",
        workspace: "workspace-1",
        harness: "claude",
        model: "a-model",
        effort: null,
        title: "The login page times out",
        forkedFrom: null,
        startedAt: 1_700_000_000_000,
        endedAt: null,
        exitCode: null,
        process: "running",
        terminal: {
          id: "terminal-1",
          activity: "quiet",
          idleMs: 180_000,
          size: { cols: 80, rows: 24 },
          exitCode: null,
        },
      },
    ],
    endedSessions: [] as Record<string, unknown>[],
    terminals: [] as Record<string, unknown>[],
    events: [event("event-1", "process.started", { harness: "claude", model: "a-model" })],
    olderEvents: null as null | Record<string, unknown>[],
    notes: [
      {
        id: "note-1",
        project: "project-1",
        text: "Migrations are append-only.",
        author: "ana",
        harness: null,
        createdAt: 1_700_000_000_000,
        updatedAt: 1_700_000_000_000,
      },
    ],
    /** Operations refused, as a computer refuses what a grant does not reach. */
    refuses: new Map<string, string>(),
  };
  /** Every operation a computer was asked, in order. */
  operations: { op: string; params: Record<string, unknown> | undefined }[] = [];
  /** The sockets open. */
  relaySockets = new Set<Socket>();
  /** Sockets with a subscription open, to tell of what is recorded. */
  #following = new Map<Socket, number>();
  /** How many relay sockets are open, and how many ever were. */
  sockets = { open: 0, ever: 0 };
  #issued = 0;

  fetch = (async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const url = new URL(typeof input === "string" ? input : input.toString());
    const method = init?.method ?? "GET";
    this.asked.push(`${method} ${url.pathname}`);
    if (this.down) throw new TypeError("Network request failed");
    const access = /^Bearer access-(.+)$/.exec(
      new Headers(init?.headers).get("authorization") ?? "",
    )?.[1];
    const signedIn = access ? this.devices.find((d) => d.id === access && !d.revokedAt) : undefined;

    if (method === "GET" && url.pathname === "/v1/server") {
      return Response.json({
        server: this.server,
        name: this.name,
        protocol: { major: 1, minor: 4 },
        issuer: "https://auth.example.com",
        capabilities: [],
      });
    }
    if (method === "POST" && url.pathname === "/v1/token") {
      const body = JSON.parse(String(init?.body));
      if (body.grant === "signIn") {
        const id = `dev-phone-${++this.#issued}`;
        this.devices.unshift({
          id,
          name: body.device.name,
          kind: body.device.kind,
          platform: body.device.platform,
          createdAt: 1_700_000_200_000,
          lastSeenAt: null,
          revokedAt: null,
        });
        return this.#issue(id);
      }
      const device = [...this.refresh].find(([, r]) => r === body.refresh)?.[0];
      if (!device || this.devices.find((d) => d.id === device)?.revokedAt) {
        return refused("This device is signed out.");
      }
      return this.#issue(device);
    }
    if (!signedIn) return refused("Sign in first.");
    if (method === "GET" && url.pathname === "/v1/devices") {
      await this.devicesAfter;
      return Response.json({
        devices: this.devices.map((d) => ({ ...d, current: d.id === signedIn.id })),
      });
    }
    if (method === "GET" && url.pathname === "/v1/hosts") {
      return Response.json({ hosts: this.hosts });
    }
    if (method === "GET" && url.pathname === "/v1/grants") {
      return Response.json({ grants: this.grants.filter((g) => g.device === signedIn.id) });
    }
    if (method === "POST" && url.pathname === "/v1/pairings") {
      const { offer, scopes } = JSON.parse(String(init?.body));
      this.presented.push(offer.value);
      await this.pairAfter.get(offer.value);
      if (!this.offers.delete(offer.value.replace(/-/g, ""))) {
        return Response.json(
          {
            code: "not_found",
            message: "That pairing code is wrong or has expired. Ask the host for a new one.",
            retryable: false,
          },
          { status: 404 },
        );
      }
      const host = this.hosts[0] as (typeof this.hosts)[number];
      const grant: Grant = {
        id: `grant-${this.grants.length + 1}`,
        host: host.id,
        hostName: host.name,
        device: signedIn.id,
        deviceName: signedIn.name,
        state: "pending",
        requested: scopes,
        scopes: [],
        projects: null,
        createdAt: 1_700_000_200_000,
        decidedAt: null,
        expiresInMs: 300_000,
      };
      this.grants.push(grant);
      return Response.json(grant);
    }
    const oneGrant = /^\/v1\/grants\/([^/]+)(\/revoke)?$/.exec(url.pathname);
    if (oneGrant) {
      const grant = this.grants.find((g) => g.id === oneGrant[1]);
      if (grant && oneGrant[2] && method === "POST") {
        grant.state = "revoked";
        return new Response(null, { status: 204 });
      }
      if (grant && method === "GET") return Response.json(grant);
    }
    if (method === "POST" && url.pathname === "/v1/relay/tickets") {
      const { host } = JSON.parse(String(init?.body));
      return Response.json({ ticket: `ticket-${host}`, expiresInMs: 60_000 });
    }
    const revoke = /^\/v1\/devices\/([^/]+)\/revoke$/.exec(url.pathname);
    if (method === "POST" && revoke) {
      this.revoke(revoke[1] as string);
      return new Response(null, { status: 204 });
    }
    return Response.json(
      { code: "not_found", message: "No such thing.", retryable: false },
      { status: 404 },
    );
  }) as typeof fetch;

  /** The person at the computer answering the newest request. */
  answer(state: "approved" | "denied" | "expired"): void {
    const grant = this.grants.at(-1);
    if (!grant) throw new Error("nobody asked");
    grant.state = state;
    grant.expiresInMs = null;
    if (state === "approved") {
      grant.scopes = grant.requested;
      grant.projects = { kind: "all" };
    }
  }

  /** A relay and the computers behind it, as far as opening a channel goes. */
  socket = (): Socket => {
    let stage: "greeting" | "hello" | "open" = "greeting";
    let host = "";
    const socket: Socket = {
      binaryType: "",
      readyState: 0,
      onopen: null,
      onmessage: null,
      onclose: null,
      onerror: null,
      send: (data) => {
        setTimeout(() => {
          const say = (value: unknown) => socket.onmessage?.({ data: JSON.stringify(value) });
          if (stage === "greeting") {
            host = String(JSON.parse(data).ticket).replace("ticket-", "");
            if (this.offline.has(host)) {
              say({
                refused: { code: "host_offline", message: "It is offline.", retryable: true },
              });
              return;
            }
            stage = "hello";
            say({ connected: { host } });
          } else if (stage === "hello") {
            stage = "open";
            const protocol = { major: 1, minor: 4 };
            say({
              welcome: {
                protocol,
                host: {
                  host,
                  name: "Workstation",
                  platform: "linux",
                  appVersion: "0",
                  protocol,
                  capabilities: [],
                },
              },
            });
          } else {
            const { requestId, op, params } = JSON.parse(data);
            this.operations.push({ op, params });
            const refusal = this.computer.refuses.get(op);
            say(
              refusal
                ? { requestId, error: { code: "forbidden", message: refusal, retryable: false } }
                : { requestId, result: this.#answer(op, params, socket) },
            );
          }
        }, 0);
      },
      close: (code = 1000) => {
        this.relaySockets.delete(socket);
        this.#following.delete(socket);
        this.sockets.open -= 1;
        setTimeout(() => socket.onclose?.({ code, reason: "" }), 0);
      },
    };
    this.relaySockets.add(socket);
    this.sockets.open += 1;
    this.sockets.ever += 1;
    setTimeout(() => socket.onopen?.({}), 0);
    return socket;
  };

  /** A computer's answer to one operation. */
  #answer(op: string, params: Record<string, never>, socket: Socket): unknown {
    const { computer } = this;
    switch (op) {
      case "workspace.list":
        return params.cursor
          ? { kind: "workspaces", projects: [], workspaces: computer.moreWorkspaces, next: null }
          : {
              kind: "workspaces",
              projects: computer.projects,
              workspaces: computer.workspaces,
              next: computer.moreWorkspaces ? "k1.more" : null,
            };
      case "workspace.describe":
        return {
          kind: "workspace",
          workspace: computer.workspaces[0],
          project: computer.projects[0],
        };
      case "session.list":
        return {
          kind: "sessions",
          sessions: params.includeEnded
            ? [...computer.sessions, ...computer.endedSessions]
            : computer.sessions,
          terminals: computer.terminals,
          next: null,
        };
      case "activity.list":
        return params.cursor
          ? { kind: "activity", events: computer.olderEvents, older: null, newer: "a1.n1" }
          : {
              kind: "activity",
              events: computer.events,
              older: computer.olderEvents ? "a1.older" : null,
              newer: "a1.n1",
            };
      case "activity.subscribe":
        this.#following.set(socket, this.#following.size + 1);
        return { kind: "subscribed", subscription: this.#following.get(socket), newer: "a1.n1" };
      case "activity.unsubscribe":
        this.#following.delete(socket);
        return { kind: "done" };
      case "memory.list":
        return { kind: "memory", notes: computer.notes, next: null };
      default:
        return {
          kind: "host",
          host: "host-work",
          name: "Workstation",
          platform: "linux",
          appVersion: "0",
          protocol: { major: 1, minor: 4 },
          capabilities: [],
        };
    }
  }

  /** The computer recording something, told to whoever is following. */
  record(...events: Record<string, unknown>[]): void {
    for (const [socket, subscription] of this.#following) {
      socket.onmessage?.({
        data: JSON.stringify({ event: "activity.events", subscription, events, newer: "a1.n2" }),
      });
    }
  }

  /** Revoke a device, as another of the account's devices would. */
  revoke(id: string): void {
    const device = this.devices.find((d) => d.id === id);
    if (device) device.revokedAt = 1_700_000_300_000;
    this.refresh.delete(id);
  }

  #issue(device: string): Response {
    const refresh = `refresh-${device}-${++this.#issued}`;
    this.refresh.set(device, refresh);
    return Response.json({
      device,
      access: `access-${device}`,
      accessExpiresInMs: 3_600_000,
      refresh,
    });
  }
}

/** An activity event as a computer sends it. */
export function event(id: string, kind: string, detail: Record<string, unknown> = {}) {
  return {
    id,
    workspace: "workspace-1",
    session: null,
    run: null,
    occurredAt: 1_700_000_000_000,
    kind,
    source: { producer: "app", method: "launcher", fidelity: "observed" },
    detail,
  };
}

function refused(message: string): Response {
  return Response.json({ code: "unauthenticated", message, retryable: false }, { status: 401 });
}

const crypto: Crypto = {
  random: (length) => webcrypto.getRandomValues(new Uint8Array(length)),
  // Not `subtle.digest`, which takes real time on a thread: these screens run under fake timers, so
  // `findBy` gives up after a few real milliseconds, and on a loaded runner the digest lost that race.
  sha256: async (data) => new Uint8Array(createHash("sha256").update(data).digest()),
};

export function envFor(server: FakeServer): Env {
  return {
    crypto,
    platform: "ios",
    userAgent: "yardsort-mobile/test (ios)",
    fetch: server.fetch,
    socket: server.socket,
  };
}
