// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A server, as far as these screens ask one, answering through `fetch`. Its answers are read by
 * the real protocol client, so a shape it gets wrong fails there. What the real server does is
 * tested in `test/lib/`, against the real server.
 */

import { webcrypto } from "node:crypto";

import type { Crypto } from "@yardsort/remote-client";

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

function refused(message: string): Response {
  return Response.json({ code: "unauthenticated", message, retryable: false }, { status: 401 });
}

const crypto: Crypto = {
  random: (length) => webcrypto.getRandomValues(new Uint8Array(length)),
  sha256: async (data) =>
    new Uint8Array(await webcrypto.subtle.digest("SHA-256", data as Uint8Array<ArrayBuffer>)),
};

export function envFor(server: FakeServer): Env {
  return { crypto, platform: "ios", userAgent: "yardsort-mobile/test (ios)", fetch: server.fetch };
}
