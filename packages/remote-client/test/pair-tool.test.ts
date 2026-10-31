// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The pairing tool (`tools/pair.ts`) run whole against the stand-in, with this test as the
// browser and as the person at the computer.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";

import { Client, type DeviceId } from "../src/index.ts";
import { signIn, StandIn, START_TIMEOUT_MS, until } from "./stand-in.ts";

const TOOL = join(import.meta.dir, "../tools/pair.ts");

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start();
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

/** The tool, running, with its output read line by line. */
function run(...args: string[]) {
  const tool = Bun.spawn(["bun", TOOL, ...args, "--no-browser"], {
    stdout: "pipe",
    stderr: "inherit",
  });
  const seen: string[] = [];
  const waiting: { now: { match: RegExp; resolve(line: string): void } | null } = { now: null };
  void (async () => {
    const decoder = new TextDecoder();
    let held = "";
    for await (const chunk of tool.stdout) {
      held += decoder.decode(chunk, { stream: true });
      let at: number;
      while ((at = held.indexOf("\n")) >= 0) {
        const line = held.slice(0, at);
        held = held.slice(at + 1);
        seen.push(line);
        if (waiting.now?.match.test(line)) {
          waiting.now.resolve(line);
          waiting.now = null;
        }
      }
    }
  })();
  return {
    seen,
    exited: tool.exited,
    /** The first line, from now or already seen, that matches. */
    line(match: RegExp): Promise<string> {
      const already = seen.find((l) => match.test(l));
      if (already) return Promise.resolve(already);
      return new Promise((resolve) => (waiting.now = { match, resolve }));
    },
  };
}

/** What the browser does with the address the tool prints: through to the tool's own listener. */
async function browse(tool: ReturnType<typeof run>): Promise<void> {
  const url = (await tool.line(/^\s+https?:\/\/.*\/v1\/login\?/)).trim();
  let next = url;
  for (let hop = 0; hop < 5; hop++) {
    const answer = await fetch(next, { redirect: "manual" });
    const location = answer.headers.get("location");
    if (!location) {
      expect(await answer.text()).toContain("Signed in.");
      return;
    }
    next = new URL(location, next).toString();
  }
  throw new Error("the sign-in went round in circles");
}

describe("the pairing tool", () => {
  test("signs in, is let in, watches and types, and hears that its grant was taken back", async () => {
    const { code } = await standIn.offer();
    const tool = run(standIn.server, code, "--type", "typed-by-the-tool", "--wait-for-revoke");
    await browse(tool);
    const asked = await standIn.pending();
    expect(asked.deviceName).toBe("TypeScript pairing tool");
    await standIn.approve(asked.id);
    await tool.line(/^Typed "typed-by-the-tool" and Enter/);
    // Taken in by the computer when the tool says so; printed back by the program a moment later.
    await until(standIn.typed, (lines) => lines.some((l) => l.startsWith("typed-by-the-tool")));
    await tool.line(/^Watching until the relay closes/);
    await standIn.revoke(asked.id);
    expect(await tool.exited).toBe(0);
    const out = tool.seen.join("\n");
    expect(out).toContain("Let in by Stand-in host: watch and type on every project.");
    expect(out).toMatch(/resumed from there/);
    expect(out).toMatch(/numbered without a gap/);
    expect(out).toContain("Stand-in host took back this phone's grant. Still signed in.");
    expect(out).toContain("Signed out;");
    // Nothing secret is printed: no credential, ticket or the link's secret.
    expect(out).not.toMatch(/ys1\.(at|rt|po)\./);
  }, 60_000);

  test("lists the account's devices, and revokes one", async () => {
    const client = new Client(standIn.server);
    const other = await signIn(client, "A phone to revoke");
    const tool = run("devices", standIn.server, "--revoke", other.device);
    await browse(tool);
    expect(await tool.exited).toBe(0);
    const out = tool.seen.join("\n");
    expect(out).toContain("Revoked A phone to revoke.");
    expect(out).toMatch(
      new RegExp(`${other.device as DeviceId}\\s+A phone to revoke \\(phone, ios\\), .*revoked`),
    );
    const refused = await client.devices(other.access).catch((e: unknown) => e);
    expect((refused as { code?: string }).code).toBe("unauthenticated");
  }, 60_000);
});
