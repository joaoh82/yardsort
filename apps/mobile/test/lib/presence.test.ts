// A network change under the phone: what is waiting to try again does so at once, and what is open
// is asked whether it still answers (35 §17, slice 8).

import { describe, expect, test } from "bun:test";
import type { Channel, ChannelEnd, HostId } from "@yardsort/remote-client";

import { Presence } from "../../src/lib/presence";

const HOST = "host-1" as HostId;

/** A channel that ends when told, and notes what it was asked to check. */
function fakeChannel() {
  let end!: (end: ChannelEnd) => void;
  const ended = new Promise<ChannelEnd>((resolve) => (end = resolve));
  const checks: number[] = [];
  const channel = {
    ended,
    close: () => end({ reason: "closedHere", code: null, text: "closed here" }),
    check: (ms: number) => checks.push(ms),
  } as unknown as Channel;
  return { channel, checks, drop: () => end({ reason: "silent", code: null, text: "gone" }) };
}

async function until(what: string, done: () => boolean) {
  const deadline = Date.now() + 5_000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(5);
  }
}

describe("a network change", () => {
  test("tries a computer waiting out its backoff again at once", async () => {
    const opened: ReturnType<typeof fakeChannel>[] = [];
    const presence = new Presence({
      open: async () => {
        const made = fakeChannel();
        opened.push(made);
        return made.channel;
      },
      explain: async () => "undetermined",
      // Long enough that only the change can explain a second try.
      firstRetryMs: 60_000,
    });
    presence.watch([HOST]);
    await until("online", () => presence.getSnapshot().get(HOST)?.kind === "online");
    opened[0]?.drop();
    await until("reconnecting", () => presence.getSnapshot().get(HOST)?.kind === "reconnecting");
    presence.networkChanged();
    await until("online again", () => opened.length === 2);
    expect(presence.getSnapshot().get(HOST)?.kind).toBe("online");
    presence.stop();
  });

  test("asks an open channel whether it still answers, without closing it", async () => {
    const made = fakeChannel();
    const presence = new Presence({
      open: async () => made.channel,
      explain: async () => "undetermined",
      networkCheckMs: 1_500,
    });
    presence.watch([HOST]);
    await until("online", () => presence.getSnapshot().get(HOST)?.kind === "online");
    presence.networkChanged();
    expect(made.checks).toEqual([1_500]);
    expect(presence.getSnapshot().get(HOST)?.kind).toBe("online");
    presence.stop();
  });
});
