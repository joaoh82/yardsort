// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Having control of a terminal: against the real server, relay and host, a real program in a real
 * terminal, and what cannot be made to happen on demand against a channel that does it.
 */

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import {
  type Channel,
  ChannelClosed,
  ClientError,
  type ControlLoss,
  type DeviceId,
  type Scope,
  type TerminalId,
} from "@yardsort/remote-client";

import { StandIn, START_TIMEOUT_MS } from "../../../../packages/remote-client/test/stand-in.ts";
import { fitted, SUBMIT_DELAY_MS, TerminalControl } from "../../src/lib/control";
import { addedCharacter, bytesOf, controlByte } from "../../src/lib/keys";
import { readToApp } from "../../src/lib/bridge";
import type { Sight } from "../../src/lib/watch";
import { TerminalWatch } from "../../src/lib/watch";
import { paired } from "./phone";

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start();
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

async function until(what: string, done: () => boolean | Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (!(await done())) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(10);
  }
}

const NOWHERE = { reset() {}, write() {}, resize() {} };

async function program(channel: Channel): Promise<TerminalId> {
  const page = await channel.workspaces();
  const work = page.workspaces.find((w) => w.name === "work");
  if (!work) throw new Error("no workspace");
  const sessions = await channel.sessions({ workspace: work.id });
  const found = sessions.terminals.find((t) => t.terminal.activity !== "exited");
  if (!found) throw new Error("no running terminal");
  return found.terminal.id;
}

/** A phone on the terminal's screen: watching, and told of what the watch sees. */
async function onScreen(name: string, scopes?: Scope[]) {
  const { accounts } = await paired(standIn, name, scopes);
  const channel = await accounts.openChannel(standIn.host);
  const terminal = await program(channel);
  const watch = new TerminalWatch(terminal, NOWHERE);
  const me = accounts.getSnapshot().active?.device ?? null;
  const control = new TerminalControl(terminal, me);
  watch.subscribe(() => control.seen(watch.getSnapshot()));
  await watch.follow(channel);
  control.seen(watch.getSnapshot());
  return { accounts, channel, terminal, watch, control };
}

const typedSoFar = async (line: string) => (await standIn.typed()).includes(line);

describe("control of a real program", () => {
  test("a line goes with its Enter, and handing back leaves nobody in control", async () => {
    const { channel, watch, control } = await onScreen("Typing phone");
    expect(watch.getSnapshot().controller).toEqual({ kind: "nobody" });
    await control.take(channel);
    expect(control.getSnapshot().holding).toBe(true);
    await until("told it is this phone", () => watch.getSnapshot().controller?.kind === "device");

    expect(control.send("a-whole-line")).toBe(true);
    await until("typed the line", () => typedSoFar("a-whole-line"));

    control.release();
    expect(control.getSnapshot().holding).toBe(false);
    await until("told nobody has it", () => watch.getSnapshot().controller?.kind === "nobody");
    expect(control.send("after-handing-back")).toBe(false);
    channel.close();
  });

  test("lines pasted arrive as lines, and keys go in their place among what is typed", async () => {
    const { channel, control } = await onScreen("Pasting phone");
    await control.take(channel);
    expect(control.send("first-pasted\nsecond-pasted\n")).toBe(true);
    await until("typed both", async () => {
      const typed = await standIn.typed();
      return typed.includes("first-pasted") && typed.includes("second-pasted");
    });

    control.press("x", "partly-");
    control.press("y", "typed");
    expect(control.send("")).toBe(true);
    await until("typed them together", () => typedSoFar("partly-typed"));
    control.release();
    channel.close();
  });

  test("typed without Enter, it waits at the program for the rest of the line", async () => {
    const { channel, control } = await onScreen("Partly typing phone");
    await control.take(channel);
    expect(control.type("begun-")).toBe(true);
    await Bun.sleep(300);
    expect(await typedSoFar("begun-")).toBe(false);
    expect(control.send("then-finished")).toBe(true);
    await until("typed the whole line", () => typedSoFar("begun-then-finished"));
    control.release();
    channel.close();
  });

  test("the size fits the phone while it has control, and the computer's comes back", async () => {
    const { channel, watch, control } = await onScreen("Fitting phone");
    expect(watch.getSnapshot().size).toEqual({ cols: 80, rows: 24 });

    // Only wanted, while watching: nothing changes for anyone else.
    control.fit({ cols: 50, rows: 20 });
    await Bun.sleep(300);
    expect(watch.getSnapshot().size).toEqual({ cols: 80, rows: 24 });

    await control.take(channel);
    await until("drew at the phone's size", () => watch.getSnapshot().size?.cols === 50);
    expect(watch.getSnapshot().size).toEqual({ cols: 50, rows: 20 });

    // A keyboard coming up.
    control.fit({ cols: 50, rows: 12 });
    await until("drew at the new size", () => watch.getSnapshot().size?.rows === 12);

    control.release();
    await until("drew at the computer's", () => watch.getSnapshot().size?.cols === 80);
    expect(watch.getSnapshot().size).toEqual({ cols: 80, rows: 24 });
    channel.close();
  });

  test("another device takes over only when asked to, and the first is told who", async () => {
    const first = await onScreen("First phone");
    await first.control.take(first.channel);
    await until("first told", () => first.watch.getSnapshot().controller?.kind === "device");

    // Not watching, it cannot have seen whose it is, so it does not take it.
    const { accounts } = await paired(standIn, "Second phone");
    const channel = await accounts.openChannel(standIn.host);
    const unseeing = new TerminalControl(first.terminal, null);
    await unseeing.take(channel);
    expect(unseeing.getSnapshot().holding).toBe(false);
    expect(unseeing.getSnapshot().notice).toBe("Another device has control of this terminal.");
    expect(first.control.getSnapshot().holding).toBe(true);
    channel.close();

    const second = await onScreen("Third phone");
    expect(second.watch.getSnapshot().controller).toMatchObject({
      kind: "device",
      name: "First phone",
    });
    await second.control.take(second.channel);
    expect(second.control.getSnapshot().holding).toBe(true);
    await until("first told it lost it", () => !first.control.getSnapshot().holding);
    // The first's channel read the names before the third was let in, and says it can't name it.
    expect(first.control.getSnapshot().notice).toBe("Another device took control.");

    expect(first.control.send("from-the-first")).toBe(false);
    expect(second.control.send("from-the-third")).toBe(true);
    await until("typed the third's", () => typedSoFar("from-the-third"));
    expect(await typedSoFar("from-the-first")).toBe(false);

    second.control.release();
    first.channel.close();
    second.channel.close();
  });

  test("a dropped channel lets go, types nothing and takes nothing back by itself", async () => {
    const { accounts, channel, terminal, watch, control } = await onScreen("Dropped phone");
    await control.take(channel);
    control.send("before-the-drop");
    await until("typed it", () => typedSoFar("before-the-drop"));
    const before = (await standIn.typed()).length;

    channel.close();
    await until("told it let go", () => !control.getSnapshot().holding);
    expect(control.getSnapshot().notice).toContain("connection dropped");

    const again = await accounts.openChannel(standIn.host);
    await watch.follow(again);
    await until("told nobody has it", () => watch.getSnapshot().controller?.kind === "nobody");
    await Bun.sleep(300);
    expect(control.getSnapshot().holding).toBe(false);
    expect((await standIn.typed()).length).toBe(before);

    // Free for anyone, at once, without taking it from a lease that outlived the channel.
    const lease = await again.acquire(terminal);
    await lease.release();
    again.close();
  });

  test("a grant that only watches is told so", async () => {
    const { channel, control } = await onScreen("Watching phone", [
      "workspace:read",
      "terminal:read",
    ]);
    await control.take(channel);
    expect(control.getSnapshot().holding).toBe(false);
    expect(control.getSnapshot().notice).toContain("watch, but not type");
    channel.close();
  });

  // Last: the person at the computer keeps control for 30 seconds after typing.
  test("the person at the computer typing takes it back at once, and keeps it", async () => {
    const { channel, watch, control } = await onScreen("Interrupted phone");
    await control.take(channel);
    control.send("before-they-typed");
    await until("typed it", () => typedSoFar("before-they-typed"));

    await standIn.typeHere("typed-at-the-computer\r");
    await until("told it lost it", () => !control.getSnapshot().holding);
    expect(control.getSnapshot().notice).toBe("The person at the computer took control.");
    expect(watch.getSnapshot().controller).toEqual({ kind: "local" });
    await until("typed theirs", () => typedSoFar("typed-at-the-computer"));

    expect(control.send("after-they-typed")).toBe(false);
    await control.take(channel);
    expect(control.getSnapshot().holding).toBe(false);
    expect(control.getSnapshot().notice).toContain("is typing here");
    await Bun.sleep(200);
    expect(await typedSoFar("after-they-typed")).toBe(false);
    channel.close();
  });
});

// --- what the host cannot be made to do on demand ------------------------------------------------

type Answer = { resolve: () => void; reject: (error: unknown) => void };

/** A channel whose lease does what a test says, and remembers what it was given. */
function scripted() {
  const sent: { data: string; paste: boolean }[] = [];
  const answers: Answer[] = [];
  const resized: { cols: number; rows: number }[] = [];
  const asks: boolean[] = [];
  let releases = 0;
  let lost: ((why: ControlLoss) => void) | undefined;
  let answerAcquire: ((value: unknown) => void) | null = null;
  let hold = false;
  const lease = {
    input: (data: string, options: { paste?: boolean } = {}) =>
      new Promise<void>((resolve, reject) => {
        sent.push({ data, paste: options.paste ?? false });
        answers.push({ resolve, reject });
      }),
    resize: async (size: { cols: number; rows: number }) => {
      resized.push(size);
    },
    release: async () => {
      releases += 1;
    },
  };
  const channel = {
    acquire: (_: TerminalId, options: { takeOver?: boolean; lost?: typeof lost }) => {
      asks.push(options.takeOver ?? false);
      lost = options.lost;
      if (hold) return new Promise((resolve) => (answerAcquire = resolve));
      return Promise.resolve(lease);
    },
  } as unknown as Channel;
  return {
    channel,
    sent,
    answers,
    resized,
    asks,
    lease,
    releases: () => releases,
    lose: (why: ControlLoss) => lost?.(why),
    holdAcquire: () => (hold = true),
    answerAcquire: () => answerAcquire?.(lease),
  };
}

function sight(change: Partial<Sight> = {}): Sight {
  return {
    size: null,
    controller: null,
    exit: null,
    following: true,
    gone: false,
    problem: null,
    ...change,
  };
}

const ME = "device-me" as DeviceId;
const TERMINAL = "t1" as TerminalId;
const closed = new ChannelClosed({ reason: "closedHere", code: null, text: "" });
const leaseLost = () =>
  new ClientError("remote", "lost", {
    code: "lease_lost",
    message: "lost",
    retryable: false,
  } as never);

async function holding() {
  const script = scripted();
  const control = new TerminalControl(TERMINAL, ME);
  await control.take(script.channel);
  return { script, control };
}

const settle = () => Bun.sleep(0);

describe("what is typed", () => {
  test("text waits for its answer, then for a moment, before its Enter goes", async () => {
    const { script, control } = await holding();
    control.send("hello");
    await settle();
    expect(script.sent).toEqual([{ data: "hello", paste: false }]);

    const answered = Date.now();
    script.answers[0]?.resolve();
    await until("sent Enter", () => script.sent.length === 2);
    expect(Date.now() - answered).toBeGreaterThanOrEqual(SUBMIT_DELAY_MS - 5);
    expect(script.sent[1]).toEqual({ data: "\r", paste: false });
  });

  test("several lines are one paste, a last newline is not a second Enter, nothing is Enter", async () => {
    const { script, control } = await holding();
    control.send("one\ntwo\n\n");
    await settle();
    expect(script.sent).toEqual([{ data: "one\ntwo", paste: true }]);
    script.answers[0]?.resolve();
    await until("sent Enter", () => script.sent.length === 2);
    script.answers[1]?.resolve();
    await settle();

    control.send("");
    await settle();
    expect(script.sent[2]).toEqual({ data: "\r", paste: false });
  });

  test("typed without Enter means no Enter, however long it waits, and nothing typed is nothing", async () => {
    const { script, control } = await holding();
    expect(control.type("")).toBe(false);
    expect(control.type("1")).toBe(true);
    await settle();
    script.answers[0]?.resolve();
    await Bun.sleep(SUBMIT_DELAY_MS + 50);
    expect(script.sent).toEqual([{ data: "1", paste: false }]);

    expect(control.type("a\nb\n")).toBe(true);
    await settle();
    expect(script.sent[1]).toEqual({ data: "a\nb\n", paste: true });
  });

  test("typed without Enter, too much stays too, and without control it is not taken", async () => {
    const { script, control } = await holding();
    expect(control.type("x".repeat(32 * 1024 + 1))).toBe(false);
    expect(control.getSnapshot().notice).toContain("too long");
    control.release();
    expect(control.type("x")).toBe(false);
    expect(script.sent).toEqual([]);
  });

  test("what is typed next waits its turn behind what is on its way", async () => {
    const { script, control } = await holding();
    control.press("A", "a");
    control.press("B", "b");
    await settle();
    expect(script.sent.map((s) => s.data)).toEqual(["a"]);
    script.answers[0]?.resolve();
    await until("sent the next", () => script.sent.length === 2);
    expect(script.sent[1]?.data).toBe("b");
  });

  test("too much for one write stays with the person, and nothing goes", async () => {
    const { script, control } = await holding();
    expect(control.send("x".repeat(32 * 1024 + 1))).toBe(false);
    expect(control.getSnapshot().notice).toContain("too long");
    expect(script.sent).toEqual([]);
    expect(control.send("x".repeat(32 * 1024))).toBe(true);
  });

  test("a channel closing on an input says it may not have arrived, and sends nothing more", async () => {
    const { script, control } = await holding();
    control.send("maybe");
    control.press("Escape", "\x1b");
    await settle();
    script.answers[0]?.reject(closed);
    await until("said so", () => control.getSnapshot().uncertain !== null);
    expect(control.getSnapshot().uncertain).toEqual({ said: "maybe", text: "maybe" });
    expect(control.getSnapshot().notice).toContain("was not sent");
    await Bun.sleep(SUBMIT_DELAY_MS + 50);
    expect(script.sent).toEqual([{ data: "maybe", paste: false }]);

    expect(control.settleUncertain()).toBe("maybe");
    expect(control.getSnapshot().uncertain).toBeNull();
  });

  test("a key that may not have arrived is named, and has nothing to put back", async () => {
    const { script, control } = await holding();
    control.press("Ctrl-C", "\x03");
    await settle();
    script.answers[0]?.reject(closed);
    await until("said so", () => control.getSnapshot().uncertain !== null);
    expect(control.getSnapshot().uncertain).toEqual({ said: "Ctrl-C", text: null });
    expect(control.settleUncertain()).toBeNull();
  });

  test("a lease refused is known not to have been typed: not uncertain, and said to be unsent", async () => {
    const { script, control } = await holding();
    control.press("A", "a");
    await settle();
    script.lose({ kind: "lost", error: { code: "lease_lost" } as never });
    script.answers[0]?.reject(leaseLost());
    await until("said so", () => control.getSnapshot().notice?.includes("not sent") ?? false);
    expect(control.getSnapshot().uncertain).toBeNull();
    expect(control.getSnapshot().holding).toBe(false);
  });

  test("control lost before the Enter means no Enter", async () => {
    const { script, control } = await holding();
    control.send("no-enter");
    await settle();
    script.answers[0]?.resolve();
    await settle();
    script.lose({ kind: "lost", error: { code: "lease_lost" } as never });
    await Bun.sleep(SUBMIT_DELAY_MS + 50);
    expect(script.sent).toEqual([{ data: "no-enter", paste: false }]);
    expect(control.getSnapshot().notice).toContain("was not sent");
  });

  test("saying that something was not sent is said once", async () => {
    const { script, control } = await holding();
    control.press("A", "a");
    control.press("B", "b");
    await settle();
    script.lose({ kind: "lost", error: { code: "lease_lost" } as never });
    script.answers[0]?.reject(leaseLost());
    await settle();
    expect(control.getSnapshot().notice).toBe(
      "This phone no longer has control. What was still waiting was not sent.",
    );
  });

  test("losing control drops what was waiting, and taking it again sends none of it", async () => {
    const { script, control } = await holding();
    control.press("A", "a");
    control.press("B", "b");
    await settle();
    script.lose({ kind: "closed", end: { reason: "closedHere", code: null, text: "" } });
    expect(control.getSnapshot().notice).toContain("was not sent");
    script.answers[0]?.resolve();

    await control.take(script.channel);
    await Bun.sleep(50);
    expect(script.sent.map((s) => s.data)).toEqual(["a"]);
  });
});

describe("who has control", () => {
  test("a report of the person at the computer lets go at once, and what was waiting with it", async () => {
    const { script, control } = await holding();
    control.seen(sight({ controller: { kind: "device", device: ME, name: "Me" } }));
    control.press("A", "a");
    control.press("B", "b");
    await settle();

    control.seen(sight({ controller: { kind: "local" } }));
    expect(control.getSnapshot().holding).toBe(false);
    expect(control.getSnapshot().notice).toBe(
      "The person at the computer took control. What was still waiting was not sent.",
    );
    expect(script.releases()).toBe(1);
    script.answers[0]?.resolve();
    await Bun.sleep(20);
    expect(script.sent.map((s) => s.data)).toEqual(["a"]);
  });

  test("this phone's own device, or nobody, is no loss", async () => {
    const { control } = await holding();
    control.seen(sight({ controller: { kind: "device", device: ME, name: "Me" } }));
    expect(control.getSnapshot().holding).toBe(true);
    control.seen(sight({ controller: { kind: "nobody" } }));
    expect(control.getSnapshot().holding).toBe(true);
  });

  test("what was last heard before taking over is no loss: only a new report is", async () => {
    const script = scripted();
    const control = new TerminalControl(TERMINAL, ME);
    const other = { kind: "device", device: "other" as DeviceId, name: "iPad" } as const;
    control.seen(sight({ controller: other }));
    await control.take(script.channel);
    expect(script.asks).toEqual([true]);
    // Something else changes, with the controller not yet told anew.
    control.seen(sight({ controller: other, size: { cols: 50, rows: 20 } }));
    expect(control.getSnapshot().holding).toBe(true);

    control.seen(sight({ controller: { ...other } }));
    expect(control.getSnapshot().holding).toBe(false);
    expect(control.getSnapshot().notice).toBe("iPad took control.");
  });

  test("not held by another, asking does not take over", async () => {
    for (const controller of [null, { kind: "nobody" }, { kind: "local" }] as const) {
      const script = scripted();
      const control = new TerminalControl(TERMINAL, ME);
      control.seen(sight({ controller }));
      await control.take(script.channel);
      expect(script.asks).toEqual([false]);
    }
    const script = scripted();
    const control = new TerminalControl(TERMINAL, ME);
    control.seen(sight({ controller: { kind: "device", device: ME, name: "Me" } }));
    await control.take(script.channel);
    expect(script.asks).toEqual([false]);
  });

  test("a program's end lets go, with nothing to tell", async () => {
    const { script, control } = await holding();
    control.seen(sight({ exit: { code: 0, success: true } }));
    expect(control.getSnapshot()).toMatchObject({ holding: false, notice: null });
    expect(script.releases()).toBe(1);
  });

  test("leaving while control is being asked for lets go of it when it comes", async () => {
    const script = scripted();
    script.holdAcquire();
    const control = new TerminalControl(TERMINAL, ME);
    const taking = control.take(script.channel);
    expect(control.getSnapshot().asking).toBe(true);
    void control.take(script.channel);
    expect(script.asks.length).toBe(1);

    control.release();
    expect(control.getSnapshot().asking).toBe(false);
    script.answerAcquire();
    await taking;
    expect(control.getSnapshot().holding).toBe(false);
    expect(script.releases()).toBe(1);
    expect(control.send("x")).toBe(false);
  });
});

describe("the size", () => {
  test("what fits, or nothing when too little would", () => {
    const cell = { width: 7.5, height: 15 };
    expect(fitted({ width: 390, height: 600 }, cell)).toEqual({ cols: 52, rows: 40 });
    expect(fitted({ width: 149, height: 600 }, cell)).toBeNull();
    expect(fitted({ width: 390, height: 74 }, cell)).toBeNull();
    expect(fitted({ width: 150, height: 75 }, cell)).toEqual({ cols: 20, rows: 5 });
  });

  test("is asked for once it settles, not again unchanged, and not without control", async () => {
    const script = scripted();
    const control = new TerminalControl(TERMINAL, ME);
    control.fit({ cols: 40, rows: 30 });
    await Bun.sleep(200);
    expect(script.resized).toEqual([]);

    await control.take(script.channel);
    expect(script.resized).toEqual([{ cols: 40, rows: 30 }]);

    control.fit({ cols: 40, rows: 20 });
    control.fit({ cols: 40, rows: 15 });
    control.fit({ cols: 40, rows: 12 });
    await Bun.sleep(250);
    expect(script.resized).toEqual([
      { cols: 40, rows: 30 },
      { cols: 40, rows: 12 },
    ]);

    control.fit({ cols: 40, rows: 12 });
    await Bun.sleep(250);
    expect(script.resized.length).toBe(2);

    // On the way out, a size asked for goes nowhere.
    control.fit({ cols: 60, rows: 12 });
    control.release();
    await Bun.sleep(250);
    expect(script.resized.length).toBe(2);
  });

  test("waits for it to settle: a keyboard coming up in steps is one redraw", async () => {
    const { script, control } = await holding();
    for (const rows of [30, 24, 18, 12]) {
      control.fit({ cols: 40, rows });
      await Bun.sleep(50);
    }
    expect(script.resized).toEqual([]);
    await Bun.sleep(250);
    expect(script.resized).toEqual([{ cols: 40, rows: 12 }]);
  });

  test("taking control again asks for the size again", async () => {
    const { script, control } = await holding();
    control.fit({ cols: 40, rows: 30 });
    await Bun.sleep(200);
    control.release();
    await control.take(script.channel);
    expect(script.resized).toEqual([
      { cols: 40, rows: 30 },
      { cols: 40, rows: 30 },
    ]);
  });
});

describe("the keys a phone lacks", () => {
  const plain = { ctrl: false, applicationCursor: false };

  test("say what a terminal says", () => {
    expect(bytesOf("escape", plain)).toBe("\x1b");
    expect(bytesOf("tab", plain)).toBe("\t");
    expect(bytesOf("enter", plain)).toBe("\r");
    expect(bytesOf("backspace", plain)).toBe("\x7f");
    expect(["up", "down", "right", "left"].map((k) => bytesOf(k as "up", plain))).toEqual([
      "\x1b[A",
      "\x1b[B",
      "\x1b[C",
      "\x1b[D",
    ]);
  });

  test("arrows follow what the program asked for, and Ctrl moves them by a word", () => {
    expect(bytesOf("up", { ctrl: false, applicationCursor: true })).toBe("\x1bOA");
    expect(bytesOf("left", { ctrl: false, applicationCursor: true })).toBe("\x1bOD");
    expect(bytesOf("left", { ctrl: true, applicationCursor: false })).toBe("\x1b[1;5D");
    expect(bytesOf("left", { ctrl: true, applicationCursor: true })).toBe("\x1b[1;5D");
    expect(bytesOf("escape", { ctrl: true, applicationCursor: false })).toBe("\x1b");
  });

  test("Ctrl and a character", () => {
    expect(controlByte("c")).toBe("\x03");
    expect(controlByte("C")).toBe("\x03");
    expect(controlByte("a")).toBe("\x01");
    expect(controlByte("z")).toBe("\x1a");
    expect(controlByte("[")).toBe("\x1b");
    expect(controlByte("@")).toBe("\x00");
    expect(controlByte("_")).toBe("\x1f");
    expect(controlByte(" ")).toBe("\x00");
    expect(controlByte("?")).toBe("\x7f");
    expect(controlByte("1")).toBeNull();
    expect(controlByte("é")).toBeNull();
    expect(controlByte("ab")).toBeNull();
  });

  test("what one change to the field added, if that is all it did", () => {
    expect(addedCharacter("", "c")).toBe("c");
    expect(addedCharacter("hello", "helloc")).toBe("c");
    expect(addedCharacter("hello", "hcello")).toBe("c");
    expect(addedCharacter("aa", "aaa")).toBe("a");
    expect(addedCharacter("hello", "hell")).toBeNull();
    expect(addedCharacter("hello", "jello")).toBeNull();
    expect(addedCharacter("hello", "helloab")).toBeNull();
    expect(addedCharacter("hello", "xellob")).toBeNull();
  });
});

describe("what the page says about drawing", () => {
  test("a character's size, within bounds", () => {
    const said = (m: unknown) => readToApp(JSON.stringify(m));
    expect(said({ t: "cell", width: 7.2, height: 15 })).toEqual({
      t: "cell",
      width: 7.2,
      height: 15,
    });
    expect(said({ t: "cell", width: 0, height: 15 })).toBeNull();
    expect(said({ t: "cell", width: 7, height: 201 })).toBeNull();
    expect(said({ t: "cell", width: "7", height: 15 })).toBeNull();
    expect(said({ t: "cell", width: 7 })).toBeNull();
  });

  test("whether arrows changed, nothing else", () => {
    const said = (m: unknown) => readToApp(JSON.stringify(m));
    expect(said({ t: "cursorKeys", application: true })).toEqual({
      t: "cursorKeys",
      application: true,
    });
    expect(said({ t: "cursorKeys", application: 1 })).toBeNull();
  });
});
