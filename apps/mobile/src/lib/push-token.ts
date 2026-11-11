// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The operating system's push token, and where a tap on a notice goes.
 *
 * Permission is asked from Settings → Notifications, not when the app opens. `expo-notifications`
 * is loaded only when one of these functions runs, so a test of the mapping can import this file
 * without starting the native module.
 */

import { noticeRoute } from "./notice-link";

export type DevicePushToken = { platform: "apns" | "fcm"; token: string };

export type PushTokenReader = () => Promise<DevicePushToken | null>;

/** What a tap handler needs of the notifications module. Tests pass a stand-in. */
export interface PushRuntime {
  getLastNotificationResponseAsync(): Promise<{ data: unknown } | null>;
  addNotificationResponseReceivedListener(handler: (data: unknown) => void): { remove(): void };
  addPushTokenListener(handler: (os: string, data: unknown) => void): { remove(): void };
}

/** iOS is APNs and Android is FCM. Anything else, or an empty token, is not registered. */
export function mapDeviceToken(os: string, data: unknown): DevicePushToken | null {
  if (typeof data !== "string" || data.length === 0 || data.length > 4096) return null;
  if ([...data].some((char) => char.charCodeAt(0) <= 0x20 || char.charCodeAt(0) > 0x7e)) {
    return null;
  }
  if (os === "ios") return { platform: "apns", token: data };
  if (os === "android") return { platform: "fcm", token: data };
  return null;
}

/** The in-app route for a notification's `deepLink`, or null when it is not a notice. */
export function routeFromNotificationData(data: unknown): string | null {
  if (!data || typeof data !== "object") return null;
  const link = (data as { deepLink?: unknown }).deepLink;
  if (typeof link !== "string") return null;
  return noticeRoute(link);
}

/**
 * Cold start reads the last tap. Later taps, and a token the system rotates, are subscribed.
 * Returns the unsubscribe. A tap whose link is not a notice is ignored.
 */
export function bindPush(
  runtime: PushRuntime,
  open: (route: string) => void,
  onToken: (token: DevicePushToken) => void,
): () => void {
  const openData = (data: unknown) => {
    const route = routeFromNotificationData(data);
    if (route) open(route);
  };
  void runtime.getLastNotificationResponseAsync().then((last) => {
    if (last) openData(last.data);
  });
  const taps = runtime.addNotificationResponseReceivedListener(openData);
  const tokens = runtime.addPushTokenListener((os, data) => {
    const token = mapDeviceToken(os, data);
    if (token) onToken(token);
  });
  return () => {
    taps.remove();
    tokens.remove();
  };
}

/** Ask once. A decision the person already made is not asked again. */
export async function requestPushPermissionOnce(): Promise<boolean> {
  try {
    const Notifications = await import("expo-notifications");
    const current = await Notifications.getPermissionsAsync();
    if (current.status !== "undetermined") return current.granted;
    const next = await Notifications.requestPermissionsAsync();
    return next.granted;
  } catch {
    return false;
  }
}

/** The token the system has now, after permission was granted. Null when there is none. */
export const readPushToken: PushTokenReader = async () => {
  try {
    const Notifications = await import("expo-notifications");
    const device = await Notifications.getDevicePushTokenAsync();
    const os = device.type === "ios" ? "ios" : device.type === "android" ? "android" : device.type;
    return mapDeviceToken(os, device.data);
  } catch {
    return null;
  }
};

/**
 * Install the tap and token listeners for the life of the app. Does not ask for permission.
 * The unsubscribe is safe to call before the native module has finished loading.
 */
export function installPushHandlers(
  open: (route: string) => void,
  onToken: (token: DevicePushToken) => void,
): () => void {
  let stop = () => {};
  let gone = false;
  void import("expo-notifications")
    .then((Notifications) => {
      if (gone) return;
      const runtime: PushRuntime = {
        getLastNotificationResponseAsync: async () => {
          const last = await Notifications.getLastNotificationResponseAsync();
          return last ? { data: last.notification.request.content.data } : null;
        },
        addNotificationResponseReceivedListener: (handler) =>
          Notifications.addNotificationResponseReceivedListener((response) => {
            handler(response.notification.request.content.data);
          }),
        addPushTokenListener: (handler) =>
          Notifications.addPushTokenListener((token) => {
            handler(token.type, token.data);
          }),
      };
      stop = bindPush(runtime, open, onToken);
    })
    .catch(() => {});
  return () => {
    gone = true;
    stop();
  };
}
