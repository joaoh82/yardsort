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

/** What a phone would have been shown: fixed words and a link, nothing else. */
export interface PushAlert {
  title: string;
  body: string;
  deepLink: string;
  category: string;
}

export interface StartOptions {
  /** Keep what the server and the host log, every level of Yardsort's own crates, for {@link StandIn.logs}. */
  logs?: boolean;
}

export class StandIn {
  server!: string;
  host!: HostId;
  hostName!: string;
  #process: Bun.Subprocess<"pipe", "pipe", "pipe" | "inherit">;
  #lines: AsyncGenerator<string>;
  #logs: string[] = [];

  private constructor(options: StartOptions) {
    this.#process = Bun.spawn(
      ["cargo", "run", "-q", "-p", "yardsort-server", "--example", "stand-in"],
      {
        cwd: ROOT,
        env: options.logs
          ? { ...process.env, YARDSORT_STAND_IN_LOGS: process.env.YARDSORT_STAND_IN_LOGS ?? "1" }
          : process.env,
        stdin: "pipe",
        stdout: "pipe",
        stderr: options.logs ? "pipe" : "inherit",
      },
    );
    this.#lines = lines(this.#process.stdout);
    const stderr = this.#process.stderr;
    if (stderr instanceof ReadableStream) void this.#keep(stderr);
  }

  async #keep(stderr: ReadableStream<Uint8Array>): Promise<void> {
    for await (const line of lines(stderr)) this.#logs.push(line);
  }

  /** What the server and the host logged so far, when started with {@link StartOptions.logs}. */
  logs(): string {
    return this.#logs.join("\n");
  }

  static async start(options: StartOptions = {}): Promise<StandIn> {
    const standIn = new StandIn(options);
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
  typeHere = (text: string) => this.#ask<null>({ op: "typeHere", text });
  signInAs = (subject: string) => this.#ask<null>({ op: "signInAs", subject });
  removeHost = () => this.#ask<null>({ op: "removeHost" });
  note = (text: string) => this.#ask<null>({ op: "note", text });
  conversation = (title: string) => this.#ask<null>({ op: "conversation", title });
  hostOffline = () => this.#ask<null>({ op: "hostOffline" });
  hostOnline = () => this.#ask<null>({ op: "hostOnline" });
  /** The host says an agent on its program's terminal asked for a permission: the notice's id. */
  agentAsks = () => this.#ask<string>({ op: "agentAsks" });
  /** Each alert the server handed to the push provider, as the provider was given it. */
  pushes = () => this.#ask<PushAlert[]>({ op: "pushes" });

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
