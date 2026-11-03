// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Watching a terminal: against the real server, relay and host, a real program in a real terminal,
 * and what the host cannot be made to do on demand against a channel that says it. The page has its
 * own tests, in a real browser.
 */

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { randomBytes } from "node:crypto";

import {
  type AttachmentEnd,
  type Channel,
  ClientError,
  MAX_OUTPUT_CHUNK,
  type Scope,
  type TerminalId,
  type Watcher,
} from "@yardsort/remote-client";

import { StandIn, START_TIMEOUT_MS } from "../../../../packages/remote-client/test/stand-in.ts";
import {
  fromBase64,
  linkable,
  MAX_MESSAGE,
  readToApp,
  readToPage,
  sameCell,
  toBase64,
} from "../../src/lib/bridge";
import { PageFeed, TerminalWatch } from "../../src/lib/watch";
import { TERMINAL_PAGE } from "../../src/ui/terminal-page.generated";
import { buildPage } from "../../scripts/terminal-page";
import { paired } from "./phone";

let standIn: StandIn;

beforeAll(async () => {
  standIn = await StandIn.start();
}, START_TIMEOUT_MS);

afterAll(async () => {
  await standIn?.stop();
});

async function until(what: string, done: () => boolean): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(10);
  }
}

/** What a watch asked to be drawn, in order. */
class Screen {
  said: string[] = [];
  text = "";
  reset(size: { cols: number; rows: number }): void {
    this.said.push(`reset ${size.cols}x${size.rows}`);
    this.text = "";
  }
  write(bytes: Uint8Array): void {
    this.said.push("output");
    this.text += new TextDecoder().decode(bytes);
  }
  resize(size: { cols: number; rows: number }): void {
    this.said.push(`resize ${size.cols}x${size.rows}`);
  }
  get resets(): number {
    return this.said.filter((s) => s.startsWith("reset")).length;
  }
}

async function program(channel: Channel): Promise<TerminalId> {
  const page = await channel.workspaces();
  const work = page.workspaces.find((w) => w.name === "work");
  if (!work) throw new Error("no workspace");
  const sessions = await channel.sessions({ workspace: work.id });
  const found = sessions.terminals.find((t) => t.terminal.activity !== "exited");
  if (!found) throw new Error("no running terminal");
  return found.terminal.id;
}

describe("watching a program", () => {
  async function watching(name: string, scopes?: Scope[]) {
    const { accounts } = await paired(standIn, name, scopes);
    const channel = await accounts.openChannel(standIn.host);
    const terminal = await program(channel);
    const screen = new Screen();
    const watch = new TerminalWatch(terminal, screen);
    return { accounts, channel, terminal, screen, watch };
  }

  /** Type a line, as someone with control does. */
  async function typed(channel: Channel, terminal: TerminalId, line: string): Promise<void> {
    const control = await channel.acquire(terminal);
    await control.input(`${line}\r`);
    await control.release();
  }

  test("starts from a snapshot at the computer's size, then draws what is printed next", async () => {
    const { channel, terminal, screen, watch } = await watching("Watching phone");
    await watch.follow(channel);
    expect(watch.getSnapshot().following).toBe(true);
    expect(watch.getSnapshot().size).toEqual({ cols: 80, rows: 24 });
    expect(screen.said[0]).toBe("reset 80x24");

    await typed(channel, terminal, "first-line");
    await until("drew the answer", () => screen.text.includes("got:first-line"));
    channel.close();
    await until("told it stopped", () => !watch.getSnapshot().following);
  });

  test("a dropped channel resumes: nothing cleared, nothing lost, nothing twice, nothing sent", async () => {
    const { accounts, channel, terminal, screen, watch } = await watching("Dropped phone");
    await watch.follow(channel);
    await typed(channel, terminal, "before-the-drop");
    await until("drew it", () => screen.text.includes("got:before-the-drop"));
    const typedBefore = (await standIn.typed()).length;
    channel.close();
    await until("told it stopped", () => !watch.getSnapshot().following);

    // Printed while this phone was away.
    const other = await accounts.openChannel(standIn.host);
    await typed(other, terminal, "during-the-drop");
    await Bun.sleep(300);
    const typedDuring = (await standIn.typed()).length;
    expect(typedDuring).toBe(typedBefore + 1);

    await watch.follow(other);
    await until("drew what it missed", () => screen.text.includes("got:during-the-drop"));
    expect(screen.resets).toBe(1);
    expect(screen.text.split("got:before-the-drop").length).toBe(2);
    expect(screen.text.split("got:during-the-drop").length).toBe(2);
    // Coming back typed nothing, and neither did anything before it.
    await Bun.sleep(200);
    expect((await standIn.typed()).length).toBe(typedDuring);
    other.close();
  });

  test("stopping and following again, as the app in the background does, carries on", async () => {
    const { channel, terminal, screen, watch } = await watching("Backgrounded phone");
    await watch.follow(channel);
    await typed(channel, terminal, "before-the-lock");
    await until("drew it", () => screen.text.includes("got:before-the-lock"));
    watch.stop();
    expect(watch.getSnapshot().following).toBe(false);
    await typed(channel, terminal, "while-locked");
    await Bun.sleep(200);
    expect(screen.text).not.toContain("got:while-locked");

    await watch.follow(channel);
    await until("drew it", () => screen.text.includes("got:while-locked"));
    expect(screen.resets).toBe(1);
    channel.close();
  });

  test("a page that started anew gets a snapshot, with what was printed before on it", async () => {
    const { channel, terminal, screen, watch } = await watching("Restarted phone");
    await watch.follow(channel);
    await typed(channel, terminal, "before-the-restart");
    await until("drew it", () => screen.text.includes("got:before-the-restart"));

    watch.pageStarted();
    await watch.follow(channel);
    expect(screen.resets).toBe(2);
    await until("drew it again", () => screen.text.includes("got:before-the-restart"));
    channel.close();
  });

  test("falling behind replaces the screen through the same channel", async () => {
    const { channel, screen, watch } = await watching("Behind phone");
    await watch.follow(channel);
    await until("following", () => watch.getSnapshot().following);
    watch.startOver();
    await until("attached again", () => screen.resets === 2 && watch.getSnapshot().following);
    channel.close();
  });

  test("a program's end is told, once what it printed has come", async () => {
    const { channel, screen, watch } = await watching("Ending phone");
    const page = await channel.workspaces();
    const work = page.workspaces.find((w) => w.name === "work");
    if (!work) throw new Error("no workspace");
    await standIn.run();
    const sessions = await channel.sessions({ workspace: work.id });
    const short = sessions.terminals.find(
      (t) => t.terminal.id !== watch.terminal && t.terminal.activity !== "exited",
    );
    if (!short) throw new Error("no short program");
    const ending = new TerminalWatch(short.terminal.id, screen);
    await ending.follow(channel);
    await until("told it ended", () => ending.getSnapshot().exit !== null);
    expect(ending.getSnapshot().exit).toEqual({ code: 0, success: true });
    channel.close();
  });

  test("a terminal the computer no longer has says so", async () => {
    const { channel, screen } = await watching("Lost phone");
    const watch = new TerminalWatch("no-such-terminal" as TerminalId, screen);
    await watch.follow(channel);
    expect(watch.getSnapshot().gone).toBe(true);
    expect(watch.getSnapshot().following).toBe(false);
    expect(screen.said).toEqual([]);
    channel.close();
  });

  test("a grant that does not reach terminals says so, in the computer's words", async () => {
    const { channel, screen, watch } = await watching("Unwatching phone", ["workspace:read"]);
    await watch.follow(channel);
    expect(watch.getSnapshot().following).toBe(false);
    expect(watch.getSnapshot().problem).toBeTruthy();
    expect(screen.said).toEqual([]);
    channel.close();
  });
});

/** The number a scripted snapshot's frames carry. */
const SNAPSHOT_SEQ = 5;

describe("what the host cannot be made to do on demand", () => {
  /**
   * A channel that says what a test tells it to, recording what it is asked for. A snapshot comes as
   * the host sends it, at least one frame at the position's own number, whatever is on the screen.
   */
  function scripted(
    attached: (
      from: unknown,
      watcher: Watcher,
    ) => { start: "snapshot" | "resumed"; size?: { cols: number; rows: number } },
  ) {
    const asked: unknown[] = [];
    let last: Watcher | undefined;
    let seq = SNAPSHOT_SEQ;
    const channel = {
      attach: async (_t: TerminalId, watcher: Watcher, from?: unknown) => {
        asked.push(from ?? null);
        last = watcher;
        const { start, size } = attached(from, watcher);
        seq = SNAPSHOT_SEQ;
        if (start === "snapshot") watcher.output(new Uint8Array(0), { epoch: 1, seq });
        return {
          start,
          position: { epoch: 1, seq },
          size: size ?? { cols: 80, rows: 24 },
          controller: { kind: "nobody" },
          ended: false,
          drawn: { epoch: 1, seq: 9 },
          detach: async () => {},
        };
      },
    } as unknown as Channel;
    return {
      channel,
      asked,
      end: (why: AttachmentEnd) => last?.ended?.(why),
      /** Output past the snapshot: the stream moving on. */
      print: (text: string) => {
        seq += 1;
        last?.output(new TextEncoder().encode(text), { epoch: 1, seq });
      },
      exit: (exit: { code: number; success: boolean }) => last?.exited?.(exit),
    };
  }
  const TERMINAL = "t" as TerminalId;
  const GAP: AttachmentEnd = {
    kind: "gap",
    expected: { epoch: 1, seq: 2 },
    got: { epoch: 1, seq: 3 },
  };
  const NEEDS_SNAPSHOT = {
    kind: "detached",
    error: { code: "resnapshot_required", message: "Behind.", retryable: true },
  } as AttachmentEnd;
  const COULD_NOT = "could not be followed";
  const closed = (reason: string): AttachmentEnd =>
    ({ kind: "closed", end: { reason, code: null, text: reason } }) as AttachmentEnd;

  test("output that comes before the reply is drawn after the screen is cleared", async () => {
    const screen = new Screen();
    const { channel } = scripted((_from, watcher) => {
      watcher.output(new TextEncoder().encode("early"), { epoch: 1, seq: 1 });
      return { start: "snapshot" };
    });
    await new TerminalWatch(TERMINAL, screen).follow(channel);
    expect(screen.said).toEqual(["reset 80x24", "output", "output"]);
    expect(screen.text).toBe("early");
  });

  test("a new epoch on resuming replaces the screen", async () => {
    const screen = new Screen();
    const script = scripted(() => ({ start: "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, screen);
    await watch.follow(script.channel);
    script.end(closed("gone"));
    await watch.follow(script.channel);
    expect(script.asked[1]).toEqual({ epoch: 1, seq: 9 });
    expect(screen.resets).toBe(2);
  });

  test("a resumed stream leaves the screen alone", async () => {
    const screen = new Screen();
    const script = scripted((from) => ({ start: from ? "resumed" : "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, screen);
    await watch.follow(script.channel);
    script.end(closed("gone"));
    await watch.follow(script.channel);
    expect(screen.resets).toBe(1);
  });

  test("after falling too far behind, the next channel starts from a snapshot", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    script.end(closed("overloaded"));
    await watch.follow(script.channel);
    expect(script.asked).toEqual([null, null]);
  });

  test("the host saying it needs a new snapshot attaches again, from nothing", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const screen = new Screen();
    const watch = new TerminalWatch(TERMINAL, screen);
    await watch.follow(script.channel);
    script.end({
      kind: "detached",
      error: { code: "resnapshot_required", message: "Behind.", retryable: true },
    } as AttachmentEnd);
    await until("attached again", () => screen.resets === 2);
    expect(script.asked).toEqual([null, null]);
  });

  for (const [name, why] of [
    ["a gap", GAP],
    ["the host needing a new snapshot", NEEDS_SNAPSHOT],
  ] as const) {
    test(`${name} attaches again, and doing so every time gives up, snapshots or not`, async () => {
      const script = scripted(() => ({ start: "snapshot" }));
      const screen = new Screen();
      const watch = new TerminalWatch(TERMINAL, screen);
      await watch.follow(script.channel);
      for (let i = 0; i < 8; i += 1) {
        // The page draws each snapshot. That alone says nothing about the stream moving on.
        watch.pageCaughtUp();
        script.end(why);
        await Bun.sleep(5);
      }
      expect(watch.getSnapshot().problem).toContain(COULD_NOT);
      expect(watch.getSnapshot().following).toBe(false);
      // The first attachment and five fresh starts, and then no more.
      expect(script.asked).toHaveLength(6);
    });
  }

  test("a stream that moves on, with the page keeping up, never gives up", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    for (let i = 0; i < 12; i += 1) {
      script.print("x");
      watch.pageCaughtUp();
      script.end(GAP);
      await Bun.sleep(5);
    }
    expect(watch.getSnapshot().problem).toBeNull();
    expect(script.asked).toHaveLength(13);
  });

  test("moving on once does not excuse every snapshot after it", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    script.print("once");
    for (let i = 0; i < 8; i += 1) {
      script.end(GAP);
      await Bun.sleep(5);
      watch.pageCaughtUp();
    }
    expect(watch.getSnapshot().problem).toContain(COULD_NOT);
    expect(script.asked).toHaveLength(6);
  });

  test("a page that never catches up gives up, however much new output there is", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    for (let i = 0; i < 8; i += 1) {
      // As `PageFeed` does when too much is waiting: new output, and the page still behind.
      script.print("more than the page can draw");
      watch.startOver();
      await Bun.sleep(5);
    }
    expect(watch.getSnapshot().problem).toContain(COULD_NOT);
    expect(watch.getSnapshot().following).toBe(false);
    expect(script.asked).toHaveLength(6);
  });

  test("a page that catches up each time it falls behind goes on", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    for (let i = 0; i < 12; i += 1) {
      script.print("a burst");
      watch.pageCaughtUp();
      watch.startOver();
      await Bun.sleep(5);
    }
    expect(watch.getSnapshot().problem).toBeNull();
    expect(script.asked).toHaveLength(13);
  });

  test("output resumed after a drop counts as moving on", async () => {
    const script = scripted((from) => ({ start: from ? "resumed" : "snapshot" }));
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    for (let i = 0; i < 12; i += 1) {
      script.end(closed("gone"));
      await watch.follow(script.channel);
      script.print("x");
      watch.pageCaughtUp();
      script.end(GAP);
      await Bun.sleep(5);
    }
    expect(watch.getSnapshot().problem).toBeNull();
  });

  test("a position refused on resuming falls back to a snapshot", async () => {
    const screen = new Screen();
    const script = scripted((from) => {
      if (from) throw new ClientError("invalid", "No.");
      return { start: "snapshot" };
    });
    const watch = new TerminalWatch(TERMINAL, screen);
    await watch.follow(script.channel);
    script.end(closed("gone"));
    await watch.follow(script.channel);
    expect(script.asked).toEqual([null, { epoch: 1, seq: 9 }, null]);
    expect(watch.getSnapshot().problem).toBeNull();
  });

  test("what comes for a watch already stopped is not drawn", async () => {
    const script = scripted(() => ({ start: "snapshot" }));
    const screen = new Screen();
    const watch = new TerminalWatch(TERMINAL, screen);
    await watch.follow(script.channel);
    watch.stop();
    script.print("late");
    expect(screen.text).toBe("");
  });

  test("a size too large to draw stops watching and says so, without drawing it", async () => {
    const screen = new Screen();
    const { channel } = scripted(() => ({ start: "snapshot", size: { cols: 60_000, rows: 24 } }));
    const watch = new TerminalWatch(TERMINAL, screen);
    await watch.follow(channel);
    expect(watch.getSnapshot().problem).toContain("too large");
    expect(screen.said).toEqual([]);
  });

  test("an ended program's terminal going away keeps saying it ended", async () => {
    let refuse = false;
    const script = scripted(() => {
      if (refuse) throw ClientError.from({ code: "not_found", message: "Gone.", retryable: false });
      return { start: "snapshot" };
    });
    const watch = new TerminalWatch(TERMINAL, new Screen());
    await watch.follow(script.channel);
    script.exit({ code: 3, success: false });
    script.end(closed("gone"));
    refuse = true;
    await watch.follow(script.channel);
    expect(watch.getSnapshot().exit).toEqual({ code: 3, success: false });
    expect(watch.getSnapshot().gone).toBe(false);
  });
});

describe("what goes to the page", () => {
  function feeding() {
    const posted: { t: string; id?: number; cols?: number }[] = [];
    let behind = 0;
    const feed = new PageFeed(() => (behind += 1));
    feed.connect((text) => posted.push(JSON.parse(text)));
    return { feed, posted, behindCount: () => behind };
  }
  const chunk = (bytes: number) => new Uint8Array(bytes);

  test("goes as read, in order, until the page has too much to draw", () => {
    const { feed, posted } = feeding();
    feed.write(chunk(100 * 1024));
    feed.write(chunk(100 * 1024));
    feed.write(chunk(100 * 1024));
    expect(posted.map((m) => m.id)).toEqual([0, 1]);
    feed.resize({ cols: 90, rows: 20 });
    expect(posted.length).toBe(2);

    // The size keeps its place among the output, and goes when what came before it has.
    feed.drawn(0);
    expect(posted.map((m) => m.t)).toEqual(["output", "output", "output", "resize"]);
    expect(posted[2]?.id).toBe(2);
  });

  test("a page too far behind starts over, with what was waiting dropped", () => {
    const { feed, posted, behindCount } = feeding();
    for (let i = 0; i < 200 && behindCount() === 0; i += 1) feed.write(chunk(MAX_OUTPUT_CHUNK));
    expect(behindCount()).toBe(1);
    const sent = posted.length;
    feed.drawn(0);
    expect(posted.length).toBe(sent);
  });

  test("a reset drops what was waiting: a snapshot replaces it", () => {
    const { feed, posted } = feeding();
    feed.write(chunk(200 * 1024));
    feed.write(chunk(200 * 1024));
    feed.reset({ cols: 80, rows: 24 });
    feed.drawn(0);
    expect(posted.map((m) => m.t)).toEqual(["output", "reset"]);
  });

  test("a page started anew owes nothing, and hears nothing until it is connected", () => {
    const { feed, posted } = feeding();
    feed.write(chunk(300 * 1024));
    feed.connect(null);
    feed.write(chunk(1));
    expect(posted.length).toBe(1);

    feed.connect((text) => posted.push(JSON.parse(text)));
    feed.write(chunk(200 * 1024));
    expect(posted.length).toBe(2);
    // An answer from before it started counts for nothing: this has to wait for one of its own.
    feed.drawn(0);
    feed.write(chunk(200 * 1024));
    expect(posted.length).toBe(2);
    feed.drawn(2);
    expect(posted.length).toBe(3);
  });

  test("says when the page has drawn everything, and not before", () => {
    let caughtUp = 0;
    const posted: { id?: number }[] = [];
    const feed = new PageFeed(
      () => {},
      () => (caughtUp += 1),
    );
    feed.connect((text) => posted.push(JSON.parse(text)));
    feed.write(chunk(200 * 1024));
    feed.write(chunk(200 * 1024));
    feed.write(chunk(10));
    expect(posted.map((m) => m.id)).toEqual([0]);

    // The first is drawn, which sends the next two: more to draw, so not caught up.
    feed.drawn(0);
    expect(posted.map((m) => m.id)).toEqual([0, 1, 2]);
    feed.drawn(1);
    expect(caughtUp).toBe(0);
    feed.drawn(2);
    expect(caughtUp).toBe(1);
    // An answer it has already had changes nothing.
    feed.drawn(2);
    expect(caughtUp).toBe(1);
  });

  test("a frame of the largest size fits in a message the page will read", () => {
    const { feed } = feeding();
    let text = "";
    feed.connect((message) => (text = message));
    const bytes = new Uint8Array(randomBytes(MAX_OUTPUT_CHUNK));
    feed.write(bytes);
    expect(text.length).toBeLessThanOrEqual(MAX_MESSAGE);
    const read = readToPage(text);
    if (read?.t !== "output") throw new Error("not read back");
    expect(fromBase64(read.data)).toEqual(bytes);
  });
});

describe("the bridge", () => {
  test("base64 says the same as the platform's, for every length and every byte", () => {
    for (let length = 0; length < 64; length += 1) {
      const bytes = new Uint8Array(randomBytes(length));
      const text = toBase64(bytes);
      expect(text).toBe(Buffer.from(bytes).toString("base64"));
      expect(fromBase64(text)).toEqual(bytes);
    }
    const all = Uint8Array.from({ length: 256 }, (_, i) => i);
    expect(fromBase64(toBase64(all))).toEqual(all);
  });

  test("a character is the same size however many are drawn, each whole rounded to a pixel", () => {
    const drawn = (width: number, cols: number, height = 160, rows = 10) => ({
      width,
      height,
      cols,
      rows,
    });
    // As Chromium drew them, with a font 11.33 wide and the same in reverse: 453 / 40 is 11.325.
    expect(sameCell(drawn(453, 40), drawn(680, 60))).toBe(true);
    expect(sameCell(drawn(680, 60), drawn(453, 40))).toBe(true);
    // The furthest apart rounding can put them: 11.3124 wide.
    expect(sameCell(drawn(452, 40), drawn(679, 60))).toBe(true);
    expect(sameCell(drawn(288, 40), drawn(432, 60))).toBe(true);
    // And for rows, when a line is not a whole number of pixels: 16.4 high.
    expect(sameCell(drawn(288, 40, 164, 10), drawn(288, 40, 328, 20))).toBe(true);

    // A pixel more than rounding explains, either way, is another size.
    expect(sameCell(drawn(453, 40), drawn(682, 60))).toBe(false);
    expect(sameCell(drawn(453, 40), drawn(678, 60))).toBe(false);
    expect(sameCell(drawn(288, 40), drawn(320, 40))).toBe(false);
    expect(sameCell(drawn(288, 40, 160, 10), drawn(288, 40, 180, 10))).toBe(false);
  });

  test("the page reads only what it may be told", () => {
    const good = [
      { t: "reset", cols: 80, rows: 24 },
      { t: "resize", cols: 1000, rows: 1 },
      { t: "output", id: 0, data: "aGk=" },
    ];
    for (const message of good)
      expect(readToPage(JSON.stringify(message))).toEqual(message as never);
    for (const bad of [
      "",
      "not json",
      "null",
      "[]",
      "1",
      JSON.stringify({ t: "typed", data: "x" }),
      JSON.stringify({ t: "reset", cols: 0, rows: 24 }),
      JSON.stringify({ t: "reset", cols: 1001, rows: 24 }),
      JSON.stringify({ t: "resize", cols: 1.5, rows: 24 }),
      JSON.stringify({ t: "resize", cols: "80", rows: 24 }),
      JSON.stringify({ t: "output", id: -1, data: "" }),
      JSON.stringify({ t: "output", id: 1, data: "<script>" }),
      JSON.stringify({ t: "output", id: 1, data: 5 }),
      JSON.stringify({ t: "output", id: 1, data: "A".repeat(MAX_MESSAGE) }),
      42,
      null,
    ]) {
      expect(readToPage(bad)).toBeNull();
    }
  });

  test("the app reads only what it may be told, and carries nothing extra", () => {
    expect(readToApp('{"t":"ready","token":"x"}')).toEqual({ t: "ready" });
    expect(readToApp('{"t":"drawn","id":4}')).toEqual({ t: "drawn", id: 4 });
    expect(readToApp('{"t":"trouble","what":"script","detail":"x"}')).toEqual({
      t: "trouble",
      what: "script",
    });
    for (const bad of [
      '{"t":"drawn"}',
      '{"t":"trouble","what":"anything"}',
      '{"t":"size","cols":1}',
      '{"t":"link","url":"http://example.com"}',
      "{",
    ]) {
      expect(readToApp(bad)).toBeNull();
    }
  });

  test("only an https link, on one line, may be offered", () => {
    expect(linkable("https://example.com/a?b=c#d")).toBe(true);
    expect(linkable("HTTPS://example.com")).toBe(true);
    for (const bad of [
      "http://example.com",
      "javascript:alert(1)",
      "file:///etc/passwd",
      "yardsort://pair?secret=x",
      "https://",
      "https:///x",
      "https://exa mple.com",
      "https://example.com/\nsecond",
      "https://example.com/\u0007",
      `https://example.com/${"a".repeat(2048)}`,
    ]) {
      expect(linkable(bad)).toBe(false);
    }
  });
});

describe("the page in the app", () => {
  test("is what its source builds to", async () => {
    const { html } = await buildPage();
    if (html !== TERMINAL_PAGE) throw new Error("Out of date: run `just mobile-terminal-page`.");
  });

  test("names nothing it could load, and may connect to nothing", () => {
    const policy = /Content-Security-Policy content="([^"]+)"/.exec(TERMINAL_PAGE)?.[1] ?? "";
    expect(policy).toContain("default-src 'none'");
    expect(policy).not.toContain("connect-src");
    expect(policy).not.toContain("script-src 'unsafe-inline'");
    expect(TERMINAL_PAGE).not.toMatch(/<(link|iframe|img)\b[^>]*(href|src)=/);
    expect(TERMINAL_PAGE).not.toMatch(/<script\b[^>]*src=/);
  });
});
