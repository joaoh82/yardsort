// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A pretend phone, in TypeScript: the twin of `just server-pair phone`, through this package
 * alone, as the phone app will go.
 *
 * ```text
 * bun tools/pair.ts [SERVER] CODE|LINK [--type TEXT] [--wait-for-revoke] [--no-browser]
 * bun tools/pair.ts devices [SERVER] [--revoke DEVICE] [--no-browser]
 * ```
 *
 * `SERVER` is `http://127.0.0.1:8787` unless given.
 *
 * With a code, or a `yardsort://pair?…` link, it signs in as a phone through the browser, asks the
 * computer to be let in, and waits up to five minutes for its answer, asking every two seconds.
 * Let in, it reaches the computer through the relay: it is welcomed, follows the computer's
 * activity, and watches a running terminal (a shell first, if there is one) from a snapshot,
 * checking that every item comes numbered without a gap. `--type TEXT` takes control, types
 * `TEXT` and Enter, and hands control back. Then it lets go of the stream and attaches again from
 * the last item it drew, as a phone does after losing its connection, and says whether the
 * computer resumed from there. It watches for ten seconds, or with `--wait-for-revoke` until the
 * relay closes the channel, and then asks the server what was revoked. It signs itself out at the
 * end, unless the server already did.
 *
 * `devices` signs in, lists the account's devices, and with `--revoke` revokes one of them, as a
 * phone signed out from elsewhere is. Then it signs itself out.
 *
 * `--no-browser` only prints the sign-in address, for something else to open. Credentials,
 * tickets and the link's secret are never printed. Neither is what a conversation is called: its
 * title may quote what the person typed.
 */

import {
  type ActivityBatch,
  type Channel,
  type ChannelEnd,
  Client,
  ClientError,
  type Controller,
  codeFromRedirect,
  explainRevoked,
  type GrantSummary,
  normalizeCode,
  OFFER_TTL_MS,
  openChannel,
  type PairingRequest,
  parsePairingLink,
  type Platform,
  type Scope,
  Secret,
  Session,
  type StreamPosition,
  type TerminalId,
} from "../src/index.ts";

const NAME = "TypeScript pairing tool";
const ASKED: Scope[] = [
  "workspace:read",
  "activity:read",
  "memory:read",
  "terminal:read",
  "terminal:control",
];

const args = process.argv.slice(2);
const flag = (name: string) => args.includes(name);
const value = (name: string) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
};
const valued = ["--type", "--revoke"].map(value);
const plain = args.filter((a) => !a.startsWith("--") && !valued.includes(a));

const say = (line: string) => console.log(line);

async function main(): Promise<number> {
  const devices = plain[0] === "devices";
  const rest = devices ? plain.slice(1) : plain;
  const [server, target] =
    rest[0]?.includes("://") && !rest[0].startsWith("yardsort:")
      ? [rest[0], rest[1]]
      : ["http://127.0.0.1:8787", rest[0]];
  const client = new Client(server, { userAgent: `${NAME}/0.1.0` });
  if (devices) return listDevices(client, value("--revoke"));
  if (!target) {
    say("Give the code the computer shows, or its link: pair.ts [SERVER] CODE");
    return 2;
  }
  const request = offerFrom(client, target);
  const session = await signIn(client);
  try {
    return await pairAndWatch(session, request);
  } finally {
    await signOut(session);
  }
}

/** What to redeem: the typed code, or the QR code's link, which has to name this server. */
function offerFrom(client: Client, target: string): PairingRequest {
  const link = parsePairingLink(target);
  if (link) {
    if (new Client(link.server).address !== client.address) {
      throw new ClientError("invalid", `That code is for ${link.server}; give that as SERVER.`);
    }
    return { offer: { kind: "secret", value: link.secret }, scopes: ASKED };
  }
  const code = normalizeCode(target);
  if (!code)
    throw new ClientError("invalid", "A pairing code is eight letters and digits, like ABCD-EFGH.");
  return { offer: { kind: "code", value: new Secret(code) }, scopes: ASKED };
}

/** Through the browser, back to a listener on this machine's loopback address. */
async function signIn(client: Client): Promise<Session> {
  let returned!: (url: string) => void;
  const back = new Promise<string>((resolve) => (returned = resolve));
  const listener = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
      const url = new URL(request.url);
      if (url.pathname !== "/signed-in") return new Response(null, { status: 404 });
      returned(request.url);
      const done = url.searchParams.has("code") ? "Signed in." : "Not signed in.";
      return new Response(`${done} You can close this window.`, {
        headers: { "content-type": "text/plain; charset=utf-8" },
      });
    },
  });
  try {
    const started = await client.beginSignIn(`http://127.0.0.1:${listener.port}/signed-in`);
    say(`Sign in, in a browser:\n\n  ${started.url}\n`);
    if (!flag("--no-browser")) open(started.url);
    const code = codeFromRedirect(
      await withinMs(back, 10 * 60_000, "The sign-in did not come back in time."),
    );
    const issued = await client.signIn(code, started.verifier, {
      name: NAME,
      kind: "phone",
      platform: here(),
    });
    say(`Signed in as a phone, device ${issued.device}.`);
    // Nothing is saved: the tool signs out at the end.
    return new Session(client, issued, () => {});
  } finally {
    listener.stop(true);
  }
}

async function pairAndWatch(session: Session, request: PairingRequest): Promise<number> {
  const { client } = session;
  const asked = await client.pair(await session.access(), request);
  say(`Asked ${asked.hostName} to be let in; waiting for its answer.`);
  const deadline = Date.now() + OFFER_TTL_MS;
  let grant: GrantSummary = asked;
  while (grant.state === "pending" && Date.now() < deadline) {
    await Bun.sleep(2_000);
    grant = await client.grant(await session.access(), asked.id);
  }
  if (grant.state !== "approved") {
    say(`Not let in: ${grant.state}.`);
    return 1;
  }
  const projects =
    grant.projects?.kind === "all"
      ? "every project"
      : `${grant.projects?.projects.length ?? 0} project(s)`;
  const may = grant.scopes.includes("terminal:control") ? "watch and type" : "watch";
  say(`Let in by ${grant.hostName}: ${may} on ${projects}.`);

  const channel = await reach(session, grant);
  say(`Welcomed by ${channel.host.name} (${channel.host.platform}).`);
  if (channel.host.terminalsLater)
    say(`Its terminals wait: ${channel.host.terminalsLater.message}`);

  const heard: string[] = [];
  if (channel.host.capabilities.includes("activity.subscribe")) {
    await channel.subscribe(
      {},
      {
        events: (batch: ActivityBatch) => {
          for (const event of batch.events) {
            const program = event.detail.program ? `, ${event.detail.program}` : "";
            const code =
              event.detail.exitCode === undefined ? "" : `, code ${event.detail.exitCode}`;
            say(`Activity: ${event.kind}${program}${code}.`);
            heard.push(event.kind);
          }
        },
        ended: (why) =>
          why.kind === "ended" && say(`Stopped following activity: ${why.error.message}`),
      },
    );
    say("Following the computer's activity.");
  } else {
    say("This computer does not offer activity as it happens.");
  }

  const terminal = await findTerminal(channel);
  if (!terminal) {
    say("No running terminal in any workspace this phone may see.");
    channel.close();
    return 1;
  }

  const seqs: number[] = [];
  let drawn = "";
  let snapshot: number | null = null;
  const decoder = new TextDecoder();
  const watcher = {
    output: (bytes: Uint8Array, at: StreamPosition) => {
      if (seqs.at(-1) !== at.seq) seqs.push(at.seq);
      if (snapshot === null) snapshot = at.seq;
      else if (at.seq !== snapshot) drawn += decoder.decode(bytes, { stream: true });
    },
    control: (controller: Controller) => say(`Control is now ${described(controller)}.`),
    exited: (exit: { code: number }) => say(`The program ended, with code ${exit.code}.`),
  };
  let attachment = await channel.attach(terminal, watcher);
  say(
    `Attached at item ${attachment.position.seq}, ${attachment.size.cols}x${attachment.size.rows}; ` +
      `controlled by ${described(attachment.controller)}.`,
  );

  const typed = value("--type");
  if (typed !== undefined) {
    try {
      const control = await channel.acquire(terminal);
      await control.input(`${typed}\r`);
      say(
        `Typed ${JSON.stringify(typed)} and Enter, under lease generation ${control.lease.generation}.`,
      );
      await control.release();
    } catch (error) {
      say(`Could not type: ${(error as Error).message}`);
    }
  }

  // Away and back, as after a dropped connection: from the last item drawn.
  const left = attachment.drawn;
  await attachment.detach();
  attachment = await channel.attach(terminal, watcher, left ?? undefined);
  const after = left ? `after item ${left.seq}` : "with no item known whole";
  if (attachment.start === "resumed") {
    say(
      `Came back ${after}: resumed from there; controlled by ${described(attachment.controller)}.`,
    );
  } else {
    say(`Came back ${after}: sent a snapshot at item ${attachment.position.seq} instead.`);
    seqs.length = 0;
    snapshot = null;
  }

  let end: ChannelEnd | null;
  if (flag("--wait-for-revoke")) {
    say("Watching until the relay closes the channel: revoke this phone at the computer.");
    end = await channel.ended;
  } else {
    end = await Promise.race([channel.ended, Bun.sleep(10_000).then(() => null)]);
  }
  const unbroken = seqs.every((seq, i) => i === 0 || seq === (seqs[i - 1] as number) + 1);
  say(
    `Saw ${seqs.length} items from ${seqs[0]} to ${seqs.at(-1)}: ` +
      `${unbroken ? "numbered without a gap" : "NOT IN ORDER"}.`,
  );
  say(`After the snapshot the program drew ${JSON.stringify(drawn.slice(-400))}.`);
  say(`Heard of ${heard.length} events as they were recorded: ${JSON.stringify(heard)}.`);
  if (!end) {
    channel.close();
    return 0;
  }
  say(`The relay closed the channel: ${end.text}.`);
  if (end.reason !== "revoked") return 1;
  switch (await explainRevoked(session, grant.id, grant.host)) {
    case "unpaired":
      say(`${grant.hostName} took back this phone's grant. Still signed in.`);
      break;
    case "signedOut":
      say("This phone was signed out at the server.");
      signedOut = true;
      break;
    case "undetermined":
      say("The server could not say what was revoked; nothing is forgotten.");
      break;
  }
  return 0;
}

/** Through the relay. A computer a moment from connecting is offline for that moment. */
async function reach(session: Session, grant: GrantSummary): Promise<Channel> {
  for (let tries = 0; ; tries++) {
    try {
      const channel = await openChannel(session.client, await session.access(), grant.host, {
        client: `${NAME}/0.1.0 (${here()})`,
      });
      say(`Connected to ${grant.hostName} through the relay.`);
      return channel;
    } catch (error) {
      if (!(error instanceof ClientError) || error.code !== "host_offline" || tries >= 10)
        throw error;
      await Bun.sleep(1_000);
    }
  }
}

/** A running terminal in a workspace this phone may see: a shell first. */
async function findTerminal(channel: Channel): Promise<TerminalId | null> {
  const found: { what: string; where: string; terminal: TerminalId; shell: boolean }[] = [];
  const page = await channel.workspaces();
  for (const workspace of page.workspaces) {
    const sessions = await channel.sessions({ workspace: workspace.id });
    for (const conversation of sessions.sessions) {
      if (conversation.terminal && conversation.terminal.activity !== "exited") {
        found.push({
          what: `a ${conversation.harness} conversation`,
          where: workspace.name,
          terminal: conversation.terminal.id,
          shell: false,
        });
      }
    }
    for (const other of sessions.terminals) {
      if (other.terminal.activity !== "exited") {
        found.push({
          what: other.program,
          where: workspace.name,
          terminal: other.terminal.id,
          shell: other.purpose === "shell",
        });
      }
    }
  }
  const chosen = found.find((f) => f.shell) ?? found[0];
  if (!chosen) return null;
  say(`Watching ${JSON.stringify(chosen.what)} in ${chosen.where}.`);
  return chosen.terminal;
}

async function listDevices(client: Client, revoke: string | undefined): Promise<number> {
  const session = await signIn(client);
  try {
    if (revoke !== undefined) {
      const page = await client.devices(await session.access());
      const device = page.devices.find((d) => d.id === revoke);
      if (!device) {
        say(`The account has no device ${revoke}.`);
        return 1;
      }
      await client.revokeDevice(await session.access(), device.id);
      say(`Revoked ${device.name}.`);
    }
    const page = await client.devices(await session.access());
    say(`${page.devices.length} device(s) on this account, newest first:`);
    for (const d of page.devices) {
      const state =
        d.revokedAt === null
          ? d.current
            ? "this one"
            : "signed in"
          : `revoked ${when(d.revokedAt)}`;
      const seen = d.lastSeenAt === null ? "never seen" : `last seen ${when(d.lastSeenAt)}`;
      say(`  ${d.id}  ${d.name} (${d.kind}, ${d.platform}), ${seen}, ${state}`);
    }
    return 0;
  } finally {
    await signOut(session);
  }
}

let signedOut = false;

async function signOut(session: Session): Promise<void> {
  if (signedOut) return;
  try {
    await session.signOut();
    say("Signed out; the computers it was paired with no longer list it.");
  } catch (error) {
    say(`Could not sign out: ${(error as Error).message}`);
  }
}

function described(controller: Controller): string {
  return controller.kind === "device" ? `a device (${controller.name})` : controller.kind;
}

function when(ms: number): string {
  return new Date(ms).toISOString().replace("T", " ").slice(0, 19);
}

function here(): Platform {
  return process.platform === "darwin"
    ? "macos"
    : process.platform === "win32"
      ? "windows"
      : "linux";
}

/** The system browser. Not `cmd /c start` on Windows: cmd would read the query's `&`s. */
function open(url: string): void {
  const argv =
    process.platform === "darwin"
      ? ["open", url]
      : process.platform === "win32"
        ? ["rundll32", "url.dll,FileProtocolHandler", url]
        : ["xdg-open", url];
  try {
    Bun.spawn(argv, { stdout: "ignore", stderr: "ignore" });
  } catch {
    // The address is printed; a person can open it.
  }
}

function withinMs<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  return Promise.race([
    promise,
    Bun.sleep(ms).then(() => {
      throw new ClientError("timedOut", message);
    }),
  ]);
}

main().then(
  (code) => process.exit(code),
  (error: unknown) => {
    say(error instanceof Error ? error.message : String(error));
    process.exit(1);
  },
);
