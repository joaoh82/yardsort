// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The operating system's push token, when this build has one.
 *
 * A development build and the tests leave this returning null. The phone still tells the server
 * it can be reached, and the server keeps notices until a token is registered. Wiring
 * `expo-notifications` means assigning {@link readPushToken} to a function that returns
 * `getDevicePushTokenAsync`'s token with platform `apns` or `fcm`.
 */

export type DevicePushToken = { platform: "apns" | "fcm"; token: string };

export type PushTokenReader = () => Promise<DevicePushToken | null>;

export const readPushToken: PushTokenReader = async () => null;
