// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What the phone records for Settings → Diagnostics: ids, states and codes with times, never words
// (35 §19).

import { describe, expect, test } from "bun:test";
import { ClientError, type Channel, type ChannelEnd, type HostId } from "@yardsort/remote-client";

import { asCode, Diagnostics, KEPT, report } from "../../src/lib/diagnostics";
import { NetworkChanges, type NetworkState } from "../../src/lib/network";
import { Presence } from "../../src/lib/presence";

const HOST = "host-1" as HostId;
const AT = Date.UTC(2026, 10, 11, 9, 30);

async function until(what: string, done: () => boolean) {
  const deadline = Date.now() + 5_000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error(`never ${what}`);
    await Bun.sleep(5);
  }
}

describe("codes", () => {
  test("a code stays as it is, and anything freer is written as other", () => {
    expect(asCode(4004)).toBe("4004");
    expect(asCode("remote:host_offline")).toBe("remote:host_offline");
    expect(asCode("The login page times out")).toBe("other");
    expect(asCode("/home/someone/work")).toBe("other");
    expect(asCode("x".repeat(49))).toBe("other");
    expect(asCode(null)).toBeUndefined();
  });

  test("a happening holds only its code, whatever it was given", () => {
    const diagnostics = new Diagnostics(() => AT);
    diagnostics.record({ what: "reach", host: HOST, reach: "offline", code: "it said: hunter2" });
    diagnostics.record({ what: "network", network: "wifi named Ana's home" });
    expect(diagnostics.getSnapshot().map((entry) => entry.happening)).toEqual([
      { what: "reach", host: HOST, reach: "offline", code: "other" },
      { what: "network", network: "other" },
    ]);
  });
});

test(`keeps the last ${KEPT}, the oldest dropped first`, () => {
  let now = AT;
  const diagnostics = new Diagnostics(() => now++);
  for (let n = 0; n < KEPT + 5; n++) diagnostics.record({ what: "network", network: `wifi:${n}` });
  const kept = diagnostics.getSnapshot();
  expect(kept).toHaveLength(KEPT);
  expect(kept[0]?.happening).toEqual({ what: "network", network: "wifi:5" });
});

test("the report is the facts, then each happening at its time", () => {
  const diagnostics = new Diagnostics(() => AT);
  diagnostics.record({ what: "signIn", standing: "signedIn" });
  diagnostics.record({ what: "reach", host: HOST, reach: "reconnecting", code: "silent" });
  const text = report(
    {
      app: "0.1.5",
      system: "ios 18.1",
      protocol: "1.7",
      server: "srv-1" as never,
      device: "dev-1" as never,
    },
    diagnostics.getSnapshot(),
    AT,
  );
  expect(text).toBe(
    [
      "Yardsort phone diagnostics",
      "As of 2026-11-11T09:30:00.000Z",
      "",
      "App 0.1.5",
      "System ios 18.1",
      "Protocol 1.7",
      "Server srv-1",
      "Device dev-1",
      "",
      "Since the app started:",
      "2026-11-11T09:30:00.000Z  sign-in signedIn",
      "2026-11-11T09:30:00.000Z  computer host-1 reconnecting (silent)",
    ].join("\n"),
  );
});

describe("what records it", () => {
  test("a computer's reach, with how its channel ended, and never the words it ended with", async () => {
    let end!: (end: ChannelEnd) => void;
    const opened: number[] = [];
    const diagnostics = new Diagnostics(() => AT);
    const presence = new Presence({
      open: async () => {
        opened.push(opened.length);
        if (opened.length > 1) {
          throw new ClientError("remote", "It is offline.", {
            code: "host_offline",
            message: "It is offline.",
            retryable: true,
          });
        }
        const ended = new Promise<ChannelEnd>((resolve) => (end = resolve));
        return { ended, close() {}, check() {} } as unknown as Channel;
      },
      explain: async () => "undetermined",
      firstRetryMs: 1,
      offlineRetryMs: 60_000,
      diagnostics,
    });
    presence.watch([HOST]);
    await until("online", () => presence.getSnapshot().get(HOST)?.kind === "online");
    end({ reason: "silent", code: 1006, text: "The login page times out" });
    await until("offline", () => presence.getSnapshot().get(HOST)?.kind === "offline");
    presence.stop();
    expect(diagnostics.getSnapshot().map((entry) => entry.happening)).toEqual([
      { what: "reach", host: HOST, reach: "connecting" },
      { what: "reach", host: HOST, reach: "online" },
      { what: "reach", host: HOST, reach: "reconnecting", code: "silent:1006" },
      { what: "reach", host: HOST, reach: "offline", code: "remote:host_offline" },
    ]);
  });

  test("a change of network, as its kind and whether it is connected", () => {
    let heard!: (state: NetworkState) => void;
    const diagnostics = new Diagnostics(() => AT);
    const network = new NetworkChanges((listener) => {
      heard = listener;
      return () => {};
    }, diagnostics);
    const stop = network.listen(() => {});
    heard({ type: "WIFI", isConnected: true, isInternetReachable: true });
    heard({ type: "CELLULAR", isConnected: true, isInternetReachable: true });
    stop();
    expect(diagnostics.getSnapshot().map((entry) => entry.happening)).toEqual([
      { what: "network", network: "CELLULAR:true:true" },
    ]);
  });
});
