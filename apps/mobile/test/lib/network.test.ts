// The phone's network changing is passed on once per change, and the same state again is not.

import { describe, expect, test } from "bun:test";

import { NetworkChanges, type NetworkState } from "../../src/lib/network";

describe("network changes", () => {
  test("pass on a change of kind or of being connected, once, and nothing else", () => {
    let say: (state: NetworkState) => void = () => {};
    let subscribed = 0;
    let stopped = 0;
    const changes = new NetworkChanges((heard) => {
      subscribed += 1;
      say = heard;
      return () => (stopped += 1);
    });
    let heard = 0;
    const stopA = changes.listen(() => (heard += 1));
    const stopB = changes.listen(() => (heard += 10));
    expect(subscribed).toBe(1);

    const wifi = { type: "WIFI", isConnected: true, isInternetReachable: true };
    say(wifi);
    say(wifi);
    expect(heard).toBe(0);
    say({ type: "CELLULAR", isConnected: true, isInternetReachable: true });
    expect(heard).toBe(11);
    say({ type: "CELLULAR", isConnected: true, isInternetReachable: true });
    say({ type: "NONE", isConnected: false, isInternetReachable: false });
    expect(heard).toBe(22);

    stopA();
    expect(stopped).toBe(0);
    stopB();
    expect(stopped).toBe(1);
  });
});
