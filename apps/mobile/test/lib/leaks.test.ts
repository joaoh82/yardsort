// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Nothing a person typed or a program printed, and no conversation's title or memory note, reaches
 * the server's log, the host's, this phone's cache or vault, or its diagnostics (35 §19, R4's
 * gate). Against the real server, relay and host, every level of their logs kept, with several
 * phones on one terminal and many lines through it, so the logs are read after real work.
 */

import { afterAll, beforeAll, expect, test } from "bun:test";

import type { Channel, HostId, TerminalId } from "@yardsort/remote-client";

import { StandIn, START_TIMEOUT_MS } from "../../../../packages/remote-client/test/stand-in.ts";
import { TerminalControl } from "../../src/lib/control";
import { Diagnostics, report } from "../../src/lib/diagnostics";
import { Presence } from "../../src/lib/presence";
import { TerminalWatch } from "../../src/lib/watch";
import { paired } from "./phone";

/** What only a person or a program here would have written. */
const TYPED = "zz-typed-4f1c-zz";
const PRINTED = "zz-printed-9a2e-zz";
const TITLE = "zz-title-7b3d-zz";
const NOTE = "zz-note-2c8f-zz";
const MARKERS = [TYPED, PRINTED, TITLE, NOTE];

const PHONES = 3;
const LINES = 100;

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start({ logs: true });
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

async function until(what: string, done: () => boolean | Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 30_000;
  while (!(await done())) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(20);
  }
}

/** A phone as its screens use it: its channel held by a presence, and the terminal followed. */
async function onTheTerminal(name: string, diagnostics: Diagnostics) {
  const phone = await paired(standIn, name, undefined, undefined, diagnostics);
  const { accounts } = phone;
  const presence = new Presence({
    open: (host) => accounts.openChannel(host),
    explain: (host) => accounts.explain(host),
    diagnostics,
  });
  const host = standIn.host as HostId;
  presence.watch([host]);
  await until("online", () => presence.channelOf(host) !== null);
  const channel = presence.channelOf(host) as Channel;

  // What the screens read, and keep what they keep of it.
  const page = await channel.workspaces();
  await accounts.observed()?.keepWorkspaces(host, page);
  const work = page.workspaces.find((w) => w.name === "work");
  if (!work) throw new Error("no workspace");
  const sessions = await channel.sessions({ workspace: work.id });
  const memory = await channel.memory({ project: work.project });
  // The program's terminal is the conversation's, by its title.
  const terminal = sessions.sessions.find((c) => c.title.includes(TITLE))?.terminal
    ?.id as TerminalId;

  let shown = "";
  const decoder = new TextDecoder();
  const watch = new TerminalWatch(terminal, {
    reset() {},
    write(bytes: Uint8Array) {
      shown += decoder.decode(bytes, { stream: true });
    },
    resize() {},
  });
  const control = new TerminalControl(terminal, accounts.getSnapshot().active?.device ?? null);
  watch.subscribe(() => control.seen(watch.getSnapshot()));
  await watch.follow(channel);
  return {
    ...phone,
    presence,
    channel,
    control,
    watch,
    read: JSON.stringify({ sessions, memory }),
    shown: () => shown,
  };
}

test("no marker reaches a log, the phone's cache or vault, or its diagnostics", async () => {
  await standIn.conversation(`Fix the ${TITLE}`);
  await standIn.note(`Remember the ${NOTE}`);
  const diagnostics = new Diagnostics();
  const phones: Awaited<ReturnType<typeof onTheTerminal>>[] = [];
  for (let n = 0; n < PHONES; n++) phones.push(await onTheTerminal(`Phone ${n}`, diagnostics));
  const [typing] = phones;
  if (!typing) throw new Error("no phone");

  await typing.control.take(typing.channel);
  for (let line = 0; line < LINES; line++)
    expect(typing.control.send(`${TYPED} ${line}`)).toBe(true);
  for (const phone of phones) {
    await until("every line shown", () => phone.shown().includes(`got:${TYPED} ${LINES - 1}`));
  }
  // Typing at the computer takes control back, so it comes once the phone's lines are in.
  await standIn.typeHere(`${PRINTED}\n`);
  for (const phone of phones) {
    await until("the printed line shown", () => phone.shown().includes(`got:${PRINTED}`));
  }
  // The host drops off the relay and comes back, so the logs have that in them too.
  await standIn.hostOffline();
  await until("offline", () =>
    phones.every((phone) => phone.presence.getSnapshot().get(standIn.host)?.kind === "offline"),
  );
  await standIn.hostOnline();
  for (const phone of phones) phone.presence.stop();

  // What the phones did see: the markers went where they belong.
  expect(typing.read).toContain(TITLE);
  expect(typing.read).toContain(NOTE);

  const logs = standIn.logs();
  // The logs are there to be read: a line for each request at the least.
  expect(logs).toMatch(/\/v1\//);
  const said = report(
    { app: "test", system: "test", protocol: "1.7", server: null, device: null },
    diagnostics.getSnapshot(),
    Date.now(),
  );
  expect(said).toMatch(/computer \S+ offline/);
  for (const marker of MARKERS) {
    expect(logs).not.toContain(marker);
    expect(said).not.toContain(marker);
    for (const phone of phones) {
      expect([...phone.shelf.kept.values()].join("\n")).not.toContain(marker);
      expect([...phone.vault.kept.values()].join("\n")).not.toContain(marker);
    }
  }
}, 120_000);
