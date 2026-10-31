// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The HTTP client and the session, against small real servers on this machine.

import { afterEach, describe, expect, test } from "bun:test";

import {
  challengeFor,
  Client,
  ClientError,
  codeFromRedirect,
  type DeviceId,
  NotKept,
  Secret,
  Session,
  webCrypto,
} from "../src/index.ts";

const servers: { stop(force?: boolean): void }[] = [];
afterEach(() => {
  for (const server of servers.splice(0)) server.stop(true);
});

function serve(fetch: (request: Request) => Response | Promise<Response>): string {
  const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch });
  servers.push(server);
  return `http://127.0.0.1:${server.port}`;
}

describe("a server's address", () => {
  test("is https, or plain http on a loopback address written as one", () => {
    for (const ok of [
      "https://remote.example.com",
      "https://example.com/yardsort/",
      "http://127.0.0.1:8787",
      "http://[::1]:8787",
      " https://Remote.Example.com ",
    ]) {
      expect(() => new Client(ok)).not.toThrow();
    }
    for (const bad of [
      "http://remote.example.com",
      "http://localhost:8787",
      "ftp://example.com",
      "https://example.com/?next=x",
      "https://example.com/#x",
      "https://user@example.com",
      "https://example.com:99999",
      "not an address",
    ]) {
      expect(() => new Client(bad)).toThrow(ClientError);
    }
  });

  test("ends in a slash, so paths join onto it, and names the relay", () => {
    const client = new Client("https://Example.com/yardsort");
    expect(client.address).toBe("https://example.com/yardsort/");
    expect(client.relayAddress()).toBe("wss://example.com/yardsort/v1/relay");
    expect(client.relayAddress("i-1")).toBe("wss://example.com/yardsort/v1/relay?route=i-1");
    expect(new Client("http://127.0.0.1:8787").relayAddress()).toBe("ws://127.0.0.1:8787/v1/relay");
  });
});

describe("signing in", () => {
  test("the challenge is the RFC 7636 one", async () => {
    const crypto = webCrypto();
    if (!crypto) throw new Error("no Web Crypto");
    expect(await challengeFor(crypto, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")).toBe(
      "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
    );
  });

  test("the browser is sent with the challenge, never the verifier", async () => {
    const started = await new Client("https://example.com").beginSignIn("yardsort://signed-in");
    const verifier = started.verifier.expose();
    expect(verifier).toHaveLength(43);
    expect(started.url).not.toContain(verifier);
    const crypto = webCrypto();
    if (!crypto) throw new Error("no Web Crypto");
    expect(started.url).toBe(
      `https://example.com/v1/login?challenge=${await challengeFor(crypto, verifier)}` +
        "&challengeMethod=S256&redirect=yardsort%3A%2F%2Fsigned-in",
    );
  });

  test("the code comes back on the redirect; a refusal says so without echoing the page", () => {
    expect(codeFromRedirect("yardsort://signed-in?code=the-code&x=1").expose()).toBe("the-code");
    try {
      codeFromRedirect("yardsort://signed-in?error=%3Cscript%3Eunauthenticated");
      throw new Error("read a code");
    } catch (error) {
      expect(error).toBeInstanceOf(ClientError);
      expect((error as ClientError).code).toBe("unauthenticated");
      expect((error as Error).message).not.toContain("<");
    }
  });
});

describe("answers", () => {
  test("a refusal is the server's own reason; anything else is unexpected", async () => {
    const base = serve((request) => {
      if (new URL(request.url).pathname === "/v1/devices") {
        return Response.json(
          { code: "unauthenticated", message: "Sign in again.", retryable: false },
          { status: 401 },
        );
      }
      return new Response("<html>a proxy's page</html>", { status: 502 });
    });
    const client = new Client(base);
    const refused = await client.devices(new Secret("a")).catch((e: unknown) => e);
    expect(refused).toBeInstanceOf(ClientError);
    expect((refused as ClientError).code).toBe("unauthenticated");
    const odd = await client.hosts(new Secret("a")).catch((e: unknown) => e);
    expect((odd as ClientError).kind).toBe("unexpected");
    expect((odd as Error).message).not.toContain("proxy");
  });

  test("an answer that does not read is unexpected, and a redirect is not followed", async () => {
    let followed = false;
    const base = serve((request) => {
      const path = new URL(request.url).pathname;
      if (path === "/elsewhere") followed = true;
      if (path === "/v1/server") return Response.json({ server: "../etc", name: "x" });
      return new Response(null, { status: 302, headers: { location: "/elsewhere" } });
    });
    const client = new Client(base);
    expect(((await client.describe().catch((e: unknown) => e)) as ClientError).kind).toBe(
      "unexpected",
    );
    expect(
      ((await client.hosts(new Secret("a")).catch((e: unknown) => e)) as ClientError).kind,
    ).toBe("unexpected");
    expect(followed).toBe(false);
  });

  test("a server that does not answer in time says so", async () => {
    const base = serve(() => new Promise<Response>(() => {}));
    const client = new Client(base, { timeoutMs: 100 });
    expect(((await client.describe().catch((e: unknown) => e)) as ClientError).kind).toBe(
      "timedOut",
    );
  });

  test("the access credential goes in the header, nowhere else", async () => {
    let seen: { authorization: string | null; url: string } | undefined;
    const base = serve((request) => {
      seen = { authorization: request.headers.get("authorization"), url: request.url };
      return Response.json({ hosts: [] });
    });
    await new Client(base).hosts(new Secret("ys1.at.the-access"));
    expect(seen?.authorization).toBe("Bearer ys1.at.the-access");
    expect(seen?.url).not.toContain("the-access");
  });
});

/** A stand-in for `POST /v1/token` that answers each refresh, slowly, with the next credentials. */
function tokenServer(expiresInMs: number) {
  const presented: string[] = [];
  let issued = 0;
  const base = serve(async (request) => {
    const body = (await request.json()) as { refresh: string };
    presented.push(body.refresh);
    issued += 1;
    await Bun.sleep(50);
    return Response.json({
      device: "4f1c2a3b-5d6e-4f70-8192-a3b4c5d6e7f8",
      access: `ys1.at.${issued}`,
      accessExpiresInMs: expiresInMs,
      refresh: `ys1.rt.${issued}`,
    });
  });
  return { client: new Client(base), presented };
}

const first = {
  device: "4f1c2a3b-5d6e-4f70-8192-a3b4c5d6e7f8" as DeviceId,
  access: new Secret("ys1.at.0"),
  accessExpiresInMs: 0,
  refresh: new Secret("ys1.rt.0"),
};

describe("a session", () => {
  test("refreshes once for every caller waiting, and keeps the new credential before any uses it", async () => {
    const { client, presented } = tokenServer(15 * 60_000);
    const order: string[] = [];
    const session = new Session(client, first, async (refresh) => {
      await Bun.sleep(20);
      order.push(`kept ${refresh.expose()}`);
    });
    const accesses = await Promise.all(
      Array.from({ length: 5 }, () =>
        session.access().then((a) => (order.push("used"), a.expose())),
      ),
    );
    expect(presented).toEqual(["ys1.rt.0"]);
    expect(accesses).toEqual(Array(5).fill("ys1.at.1"));
    expect(order[0]).toBe("kept ys1.rt.1");
    // Not due again for another 14 minutes: no second refresh.
    expect((await session.access()).expose()).toBe("ys1.at.1");
    expect(presented).toHaveLength(1);
  });

  test("a refresh credential that could not be kept is still the one used next", async () => {
    const { client, presented } = tokenServer(0);
    let failing = true;
    const kept: string[] = [];
    const keep = (refresh: Secret) => {
      if (failing) throw new Error("the credential store is locked");
      kept.push(refresh.expose());
    };
    // Resuming spends the one given: when its replacement cannot be kept, the session comes with
    // the failure, holding the only copy.
    const failed = await Session.resume(client, new Secret("ys1.rt.0"), keep).catch(
      (e: unknown) => e,
    );
    expect(failed).toBeInstanceOf(NotKept);
    const session = (failed as NotKept).session;
    expect(session.refreshCredential.expose()).toBe("ys1.rt.1");

    expect(await session.access().catch((e: unknown) => e)).toBeInstanceOf(NotKept);
    failing = false;
    const access = await session.access();
    // Each the one issued with the failed keep, never a spent one.
    expect(presented).toEqual(["ys1.rt.0", "ys1.rt.1", "ys1.rt.2"]);
    expect(access.expose()).toBe("ys1.at.3");
    expect(kept).toEqual(["ys1.rt.3"]);
  });
});
