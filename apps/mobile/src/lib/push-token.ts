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

/** One tap: the request's identifier, so a cold start and the listener are the same tap once. */
export interface PushTap {
  id: string;
  data: unknown;
}

/** What a tap handler needs of the notifications module. Tests pass a stand-in. */
export interface PushRuntime {
  getLastNotificationResponseAsync(): Promise<PushTap | null>;
  /** Present when the module can forget the tap that launched the app. */
  clearLastNotificationResponseAsync?(): Promise<void> | void;
  addNotificationResponseReceivedListener(handler: (tap: PushTap) => void): { remove(): void };
  addPushTokenListener(handler: (os: string, data: unknown) => void): { remove(): void };
}

/** What showing a notice while the app is open needs. Tests pass a stand-in. */
export interface ForegroundNotices {
  setNotificationHandler(handler: {
    handleNotification: () => Promise<{
      shouldShowBanner: boolean;
      shouldShowList: boolean;
      shouldPlaySound: boolean;
      shouldSetBadge: boolean;
    }>;
  }): void;
  setNotificationChannelAsync(
    id: string,
    channel: { name: string; importance: number },
  ): Promise<unknown>;
}

/** Shown while the app is open: banner, list and sound. The badge is left alone. */
const FOREGROUND = {
  shouldShowBanner: true,
  shouldShowList: true,
  shouldPlaySound: true,
  shouldSetBadge: false,
};

/**
 * Android needs a channel before permission is asked, or the prompt has nowhere to deliver.
 * The same call installs the handler that shows a notice while the app is open.
 */
export async function enableForegroundNotices(runtime: ForegroundNotices): Promise<void> {
  runtime.setNotificationHandler({
    handleNotification: async () => FOREGROUND,
  });
  await runtime.setNotificationChannelAsync("default", { name: "Notices", importance: 5 });
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
  const seen = new Set<string>();
  const openTap = (tap: PushTap) => {
    if (tap.id !== "" && seen.has(tap.id)) return;
    if (tap.id !== "") seen.add(tap.id);
    const route = routeFromNotificationData(tap.data);
    if (route) open(route);
  };
  void runtime.getLastNotificationResponseAsync().then((last) => {
    if (last) openTap(last);
    void runtime.clearLastNotificationResponseAsync?.();
  });
  const taps = runtime.addNotificationResponseReceivedListener(openTap);
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
    await enableForegroundNotices(Notifications);
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
 * Register with the accounts already signed in, when permission was granted earlier.
 * Does not ask. Settings is the only place that asks.
 */
export async function registerGrantedPush(
  register: (readToken: PushTokenReader) => Promise<void>,
): Promise<void> {
  try {
    const Notifications = await import("expo-notifications");
    const current = await Notifications.getPermissionsAsync();
    if (!current.granted) return;
    await register(readPushToken);
  } catch {
    // A phone without the native module, or a server that cannot be reached, stays quiet.
  }
}

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
    .then(async (Notifications) => {
      if (gone) return;
      await enableForegroundNotices(Notifications);
      if (gone) return;
      const runtime: PushRuntime = {
        getLastNotificationResponseAsync: async () => {
          const last = await Notifications.getLastNotificationResponseAsync();
          return last
            ? {
                id: last.notification.request.identifier,
                data: last.notification.request.content.data,
              }
            : null;
        },
        clearLastNotificationResponseAsync: Notifications.clearLastNotificationResponseAsync,
        addNotificationResponseReceivedListener: (handler) =>
          Notifications.addNotificationResponseReceivedListener((response) => {
            handler({
              id: response.notification.request.identifier,
              data: response.notification.request.content.data,
            });
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
