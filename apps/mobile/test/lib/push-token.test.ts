// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { describe, expect, test } from "bun:test";

import {
  bindPush,
  mapDeviceToken,
  routeFromNotificationData,
  type PushRuntime,
} from "../../src/lib/push-token";

const ID = "11111111-1111-4111-8111-111111111111";

describe("a device push token", () => {
  test("maps iOS to APNs and Android to FCM", () => {
    expect(mapDeviceToken("ios", "abc")).toEqual({ platform: "apns", token: "abc" });
    expect(mapDeviceToken("android", "def")).toEqual({ platform: "fcm", token: "def" });
    expect(mapDeviceToken("web", "ghi")).toBeNull();
    expect(mapDeviceToken("ios", "")).toBeNull();
    expect(mapDeviceToken("ios", "has a space")).toBeNull();
  });
});

describe("a notification tap", () => {
  test("opens the notice named by deepLink", () => {
    const link = `yardsort://notice/task/${ID}`;
    expect(routeFromNotificationData({ deepLink: link, category: "task" })).toBe(`/notice/${ID}`);
    expect(routeFromNotificationData({ deepLink: "yardsort://pair/abc" })).toBeNull();
    expect(routeFromNotificationData(null)).toBeNull();
    expect(routeFromNotificationData({ title: "A task moved forward" })).toBeNull();
  });

  test("routes the last tap on a cold start and a later tap", async () => {
    const opened: string[] = [];
    let onResponse: ((data: unknown) => void) | null = null;
    let last: { data: unknown } | null = {
      data: { deepLink: `yardsort://notice/workflow/${ID}` },
    };
    const runtime: PushRuntime = {
      getLastNotificationResponseAsync: async () => last,
      addNotificationResponseReceivedListener: (handler) => {
        onResponse = handler;
        return { remove: () => {} };
      },
      addPushTokenListener: () => ({ remove: () => {} }),
    };
    bindPush(
      runtime,
      (route) => opened.push(route),
      () => {},
    );
    // The cold-start read is a promise, and the route is opened in the reaction after it.
    await Promise.resolve();
    await Promise.resolve();
    expect(opened).toEqual([`/notice/${ID}`]);
    last = null;
    onResponse?.({ deepLink: `yardsort://notice/review/${ID}` });
    expect(opened).toEqual([`/notice/${ID}`, `/notice/${ID}`]);
    onResponse?.({ deepLink: "yardsort://signed-in?code=Q7K2M9NP" });
    expect(opened).toHaveLength(2);
  });

  test("re-registers when the system rotates the token", () => {
    const tokens: string[] = [];
    let onToken: ((os: string, data: unknown) => void) | null = null;
    const runtime: PushRuntime = {
      getLastNotificationResponseAsync: async () => null,
      addNotificationResponseReceivedListener: () => ({ remove: () => {} }),
      addPushTokenListener: (handler) => {
        onToken = handler;
        return { remove: () => {} };
      },
    };
    bindPush(
      runtime,
      () => {},
      (token) => tokens.push(`${token.platform}:${token.token}`),
    );
    onToken?.("ios", "rotated");
    onToken?.("android", "next");
    onToken?.("web", "ignored");
    expect(tokens).toEqual(["apns:rotated", "fcm:next"]);
  });
});
