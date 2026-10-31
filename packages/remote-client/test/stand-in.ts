// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The real server, a stand-in identity provider and a real host, on this machine:
// `crates/server/examples/stand-in.rs`, run through Cargo. Commands to it are what the person at
// the host would do; see that file.

import { join } from "node:path";

import {
  type Client,
  codeFromRedirect,
  type Credentials,
  type DeviceKind,
  type GrantId,
  type GrantSummary,
  type HostId,
  type Scope,
} from "../src/index.ts";

const ROOT = join(import.meta.dir, "../../..");

/** Long enough for Cargo to build the example on a cold cache. */
export const START_TIMEOUT_MS = 10 * 60_000;

export interface Offered {
  code: string;
  secret: string;
  link: string;
}

export class StandIn {
  server!: string;
  host!: HostId;
  hostName!: string;
  #process: ReturnType<typeof Bun.spawn<"pipe", "pipe", "inherit">>;
  #lines: AsyncGenerator<string>;

  private constructor() {
    this.#process = Bun.spawn(
      ["cargo", "run", "-q", "-p", "yardsort-server", "--example", "stand-in"],
      {
        cwd: ROOT,
        stdin: "pipe",
        stdout: "pipe",
        stderr: "inherit",
      },
    );
    this.#lines = lines(this.#process.stdout);
  }

  static async start(): Promise<StandIn> {
    const standIn = new StandIn();
    const first = await standIn.#next();
    const ready = (
      JSON.parse(first) as { ready?: { server: string; host: HostId; hostName: string } }
    ).ready;
    if (!ready) throw new Error(`the stand-in said ${first}`);
    Object.assign(standIn, ready);
    return standIn;
  }

  async #next(): Promise<string> {
    const { value, done } = await this.#lines.next();
    if (done) throw new Error(`the stand-in stopped (${await this.#process.exited})`);
    return value;
  }

  async #ask<T>(command: Record<string, unknown>): Promise<T> {
    this.#process.stdin.write(`${JSON.stringify(command)}\n`);
    await this.#process.stdin.flush();
    const answer = JSON.parse(await this.#next()) as { ok?: T; error?: string };
    if (answer.error !== undefined) throw new Error(`the stand-in: ${answer.error}`);
    return answer.ok as T;
  }

  offer = () => this.#ask<Offered>({ op: "offer" });
  pending = () => this.#ask<GrantSummary>({ op: "pending" });
  approve = (grant: GrantId, scopes?: Scope[]) =>
    this.#ask<GrantSummary>({ op: "approve", grant, ...(scopes ? { scopes } : {}) });
  deny = (grant: GrantId) => this.#ask<GrantSummary>({ op: "deny", grant });
  revoke = (grant: GrantId) => this.#ask<null>({ op: "revoke", grant });
  run = () => this.#ask<null>({ op: "run" });
  typed = () => this.#ask<string[]>({ op: "typed" });
  signInAs = (subject: string) => this.#ask<null>({ op: "signInAs", subject });
  removeHost = () => this.#ask<null>({ op: "removeHost" });

  async stop(): Promise<void> {
    this.#process.stdin.end();
    const stopped = await Promise.race([this.#process.exited, Bun.sleep(10_000).then(() => null)]);
    if (stopped === null) this.#process.kill();
  }
}

async function* lines(stream: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const decoder = new TextDecoder();
  let held = "";
  for await (const chunk of stream) {
    held += decoder.decode(chunk, { stream: true });
    let at: number;
    while ((at = held.indexOf("\n")) >= 0) {
      yield held.slice(0, at);
      held = held.slice(at + 1);
    }
  }
}

/**
 * Sign in as the app does, through `yardsort://signed-in`, with the stand-in's provider signing
 * whoever comes in at once. A browser would follow the redirects; this follows them by hand and
 * reads the code off the last one.
 */
export async function signIn(
  client: Client,
  name: string,
  kind: DeviceKind = "phone",
): Promise<Credentials> {
  const { url, verifier } = await client.beginSignIn("yardsort://signed-in");
  let next = url;
  for (let hop = 0; hop < 5; hop++) {
    const answer = await fetch(next, { redirect: "manual" });
    const location = answer.headers.get("location");
    if (!location) throw new Error(`no redirect from ${new URL(next).pathname} (${answer.status})`);
    if (location.startsWith("yardsort://")) {
      return client.signIn(codeFromRedirect(location), verifier, { name, kind, platform: "ios" });
    }
    next = new URL(location, next).toString();
  }
  throw new Error("the sign-in went round in circles");
}

/** Ask until `done`, or give up. */
export async function until<T>(
  ask: () => Promise<T>,
  done: (value: T) => boolean,
  withinMs = 10_000,
): Promise<T> {
  const deadline = Date.now() + withinMs;
  for (;;) {
    const value = await ask();
    if (done(value)) return value;
    if (Date.now() > deadline) throw new Error("it did not happen in time");
    await Bun.sleep(50);
  }
}
