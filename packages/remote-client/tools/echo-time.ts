// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * How long a keystroke takes to come back: R4's input budget, measured (35 §3, "Measuring the
 * budgets"). It takes control of a running `cat` on a computer, types one letter at a time, and
 * times each from being sent to its echo arriving, then says the median, p95 and p99.
 *
 * ```text
 * bun tools/echo-time.ts [SERVER] (--kept FILE | --code CODE [--keep FILE]) [--delay MS]
 *     [--count N] [--terminal ID] [--budget MS] [--no-browser]
 * ```
 *
 * `SERVER` is `http://127.0.0.1:8787` unless given.
 *
 * - `--kept FILE` signs in with the refresh credential in `FILE`, as a phone that is let in
 *   already, and keeps the one it is given back in its place. `--code CODE` signs in through the
 *   browser instead, asks the computer showing `CODE` to let it watch and type, and waits for its
 *   answer; with `--keep FILE` it keeps its sign-in there for `--kept` the next time.
 * - `--delay MS` puts a [delay proxy](delay-proxy.ts) between it and an `http://` server, holding
 *   everything back `MS` each way: 50 makes a round trip 100 ms longer.
 * - `--count N` keystrokes, 200 unless given. `--terminal ID` picks the terminal; otherwise the one
 *   running `cat`, or the one running terminal there is.
 * - `--budget MS` makes it exit with 1 when p95 is over `MS`.
 *
 * It types only letters, into a `cat` that prints them back, then a return and gives control
 * back. Credentials are never printed, and nothing the terminal shows is.
 */

import {
  type Channel,
  Client,
  ClientError,
  codeFromRedirect,
  normalizeCode,
  OFFER_TTL_MS,
  openChannel,
  type Scope,
  Secret,
  Session,
  type TerminalId,
} from "../src/index.ts";
import { delayProxy } from "./delay-proxy.ts";

const NAME = "Echo timing tool";
const ASKED: Scope[] = ["workspace:read", "terminal:read", "terminal:control"];

/** The value at `p` (0 to 1) of `sorted`, by the nearest rank. */
export function percentile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) return Number.NaN;
  const rank = Math.ceil(p * sorted.length);
  return sorted[Math.min(sorted.length, Math.max(1, rank)) - 1] as number;
}

/** Median, p95 and p99 of `samples`, and the extremes, in whole milliseconds. */
export function summary(samples: readonly number[]) {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (p: number) => Math.round(percentile(sorted, p));
  return {
    count: sorted.length,
    min: Math.round(sorted[0] ?? Number.NaN),
    p50: at(0.5),
    p95: at(0.95),
    p99: at(0.99),
    max: Math.round(sorted.at(-1) ?? Number.NaN),
  };
}

const args = process.argv.slice(2);
const flag = (name: string) => args.includes(name);
const value = (name: string) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
};
const valued = ["--kept", "--code", "--keep", "--delay", "--count", "--terminal", "--budget"].map(
  value,
);
const plain = args.filter((a) => !a.startsWith("--") && !valued.includes(a));
const say = (line: string) => console.log(line);

async function main(): Promise<number> {
  const server = plain[0] ?? "http://127.0.0.1:8787";
  const delayMs = Number(value("--delay") ?? 0);
  const count = Number(value("--count") ?? 200);
  const budget = value("--budget") === undefined ? null : Number(value("--budget"));
  let address = server;
  let stop = () => {};
  if (delayMs > 0) {
    const url = new URL(server);
    if (url.protocol !== "http:") {
      say("--delay is for an http:// server; a real distance needs no proxy.");
      return 2;
    }
    const proxy = delayProxy(`${url.hostname}:${url.port || 80}`, delayMs);
    address = `http://127.0.0.1:${proxy.port}${url.pathname}`;
    stop = proxy.stop;
    say(`Through a proxy adding ${delayMs} ms each way.`);
  }
  try {
    const client = new Client(address, { userAgent: `${NAME}/0.1.0` });
    const session = await signedIn(client);
    const channel = await reach(session);
    try {
      return await time(channel, count, budget);
    } finally {
      channel.close();
    }
  } finally {
    stop();
  }
}

/** A session from the kept credential, or a new one let in by the computer showing the code. */
async function signedIn(client: Client): Promise<Session> {
  const kept = value("--kept");
  if (kept) {
    const keep = async (refresh: Secret) => {
      await Bun.write(kept, refresh.expose());
    };
    return Session.resume(client, new Secret((await Bun.file(kept).text()).trim()), keep);
  }
  const code = normalizeCode(value("--code") ?? "");
  if (!code) throw new Error("Give --kept FILE, or --code CODE from the computer.");
  const keepIn = value("--keep");
  const session = await browserSignIn(client, keepIn);
  const asked = await client.pair(await session.access(), {
    offer: { kind: "code", value: new Secret(code) },
    scopes: ASKED,
  });
  say(`Asked ${asked.hostName} to be let in; waiting for its answer.`);
  const deadline = Date.now() + OFFER_TTL_MS;
  let grant = asked;
  while (grant.state === "pending" && Date.now() < deadline) {
    await Bun.sleep(2_000);
    grant = await client.grant(await session.access(), asked.id);
  }
  if (grant.state !== "approved" || !grant.scopes.includes("terminal:control")) {
    throw new Error("Not let in to type, which timing needs.");
  }
  return session;
}

async function browserSignIn(client: Client, keepIn: string | undefined): Promise<Session> {
  let returned!: (url: string) => void;
  const back = new Promise<string>((resolve) => (returned = resolve));
  const listener = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
      returned(request.url);
      return new Response("You can close this window.");
    },
  });
  try {
    const started = await client.beginSignIn(`http://127.0.0.1:${listener.port}/signed-in`);
    say(`Sign in, in a browser:\n\n  ${started.url}\n`);
    if (!flag("--no-browser")) {
      try {
        Bun.spawn(
          process.platform === "darwin" ? ["open", started.url] : ["xdg-open", started.url],
        );
      } catch {
        // The address is printed.
      }
    }
    const issued = await client.signIn(codeFromRedirect(await back), started.verifier, {
      name: NAME,
      kind: "phone",
      platform: process.platform === "darwin" ? "macos" : "linux",
    });
    const keep = (refresh: Secret) => {
      if (keepIn) void Bun.write(keepIn, refresh.expose());
    };
    keep(issued.refresh);
    return new Session(client, issued, keep);
  } finally {
    listener.stop(true);
  }
}

async function reach(session: Session): Promise<Channel> {
  const { hosts } = await session.client.hosts(await session.access());
  const { grants } = await session.client.grants(await session.access());
  const host = grants.find((g) => g.state === "approved" && hosts.some((h) => h.id === g.host));
  if (!host) throw new Error("No computer lets this device in.");
  for (let tries = 0; ; tries++) {
    try {
      return await openChannel(session.client, await session.access(), host.host, {
        client: `${NAME}/0.1.0`,
      });
    } catch (error) {
      if (!(error instanceof ClientError) || error.code !== "host_offline" || tries >= 30)
        throw error;
      await Bun.sleep(1_000);
    }
  }
}

/** The terminal asked for, else the one running `cat`, else the one running terminal. */
async function theTerminal(channel: Channel): Promise<TerminalId> {
  const asked = value("--terminal");
  if (asked) return asked as TerminalId;
  const running: { terminal: TerminalId; cat: boolean }[] = [];
  for (const workspace of (await channel.workspaces()).workspaces) {
    const page = await channel.sessions({ workspace: workspace.id });
    for (const s of page.sessions) {
      if (s.terminal && s.terminal.activity !== "exited")
        running.push({ terminal: s.terminal.id, cat: s.harness === "cat" });
    }
    for (const t of page.terminals) {
      if (t.terminal.activity !== "exited")
        running.push({ terminal: t.terminal.id, cat: t.program === "cat" || t.harness === "cat" });
    }
  }
  const cats = running.filter((r) => r.cat);
  const found = cats.length === 1 ? cats : running;
  if (found.length !== 1) {
    throw new Error(`${found.length} terminals could be the one; name it with --terminal.`);
  }
  return (found[0] as { terminal: TerminalId }).terminal;
}

async function time(channel: Channel, count: number, budget: number | null): Promise<number> {
  const terminal = await theTerminal(channel);
  let since = "";
  let waiting: { letter: string; heard: () => void } | null = null;
  const decoder = new TextDecoder();
  const attachment = await channel.attach(terminal, {
    output(bytes) {
      since += decoder.decode(bytes, { stream: true });
      if (waiting && since.includes(waiting.letter)) waiting.heard();
    },
  });
  const control = await channel.acquire(terminal);
  const samples: number[] = [];
  try {
    // Once to begin with, to settle the line: not counted.
    for (let n = -1; n < count; n++) {
      const letter = String.fromCharCode(97 + (((n % 26) + 26) % 26));
      since = "";
      const heard = new Promise<void>((resolve) => (waiting = { letter, heard: resolve }));
      const sent = performance.now();
      await control.input(letter);
      const late = Bun.sleep(10_000).then(() => "late" as const);
      if ((await Promise.race([heard, late])) === "late") {
        throw new Error(`Keystroke ${n + 1} did not come back within 10 seconds.`);
      }
      if (n >= 0) samples.push(performance.now() - sent);
      waiting = null;
      // A line of a few dozen letters at most, so the terminal never has to wrap far.
      if (n % 40 === 39) await control.input("\r");
      await Bun.sleep(20);
    }
    await control.input("\r");
  } finally {
    await control.release().catch(() => {});
    await attachment.detach().catch(() => {});
  }
  const s = summary(samples);
  say(
    `${s.count} keystrokes to their echo: p50 ${s.p50} ms, p95 ${s.p95} ms, p99 ${s.p99} ms ` +
      `(${s.min} to ${s.max} ms).`,
  );
  if (budget !== null && s.p95 > budget) {
    say(`Over the budget: p95 is ${s.p95} ms, more than ${budget} ms.`);
    return 1;
  }
  if (budget !== null) say(`Within the budget of ${budget} ms at p95.`);
  return 0;
}

if (import.meta.main) {
  main().then(
    (code) => process.exit(code),
    (error: unknown) => {
      say(error instanceof Error ? error.message : String(error));
      process.exit(1);
    },
  );
}
