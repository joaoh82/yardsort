// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What the phone itself provides: the credential store, the system browser, randomness and
 * SHA-256, and what to call this app and this phone. Everything else in `lib/` takes these as
 * arguments, so it runs in tests without a phone.
 */

import type { Crypto, Platform } from "@yardsort/remote-client";
import Constants from "expo-constants";
import * as ExpoCrypto from "expo-crypto";
import * as Device from "expo-device";
import { File, Paths } from "expo-file-system";
import { manipulateAsync, SaveFormat } from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import * as SecureStore from "expo-secure-store";
import * as WebBrowser from "expo-web-browser";
import { Platform as ReactPlatform } from "react-native";

import type { Browser, Env } from "./accounts";
import type { ChoiceFile } from "./appearance";
import { LONGEST_SIDE, type PickImage } from "./attach";
import type { Shelf } from "./observed";
import type { Vault } from "./profiles";

/**
 * The Keychain or the Keystore. Readable only while the phone is unlocked, and never carried to a
 * backup or another phone (31 §3); a phone restored from a backup signs in again.
 */
const KEPT: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
};

export const secureVault: Vault = {
  get: (key) => SecureStore.getItemAsync(key, KEPT),
  set: (key, value) => SecureStore.setItemAsync(key, value, KEPT),
  delete: (key) => SecureStore.deleteItemAsync(key, KEPT),
};

/**
 * The phone's cache folder: never in a backup, and the system may clear it, which costs only what
 * was last seen. A key is this app's own, never anything a server said.
 */
export const cacheShelf: Shelf = {
  get: async (key) => {
    const file = new File(Paths.cache, `${key}.json`);
    return file.exists ? file.text() : null;
  },
  set: async (key, value) => {
    const file = new File(Paths.cache, `${key}.json`);
    if (!file.exists) file.create();
    await file.write(value);
  },
  delete: async (key) => {
    const file = new File(Paths.cache, `${key}.json`);
    if (file.exists) file.delete();
  },
};

/** In the documents folder, as the system never clears it. It says nothing about anyone. */
export const appearanceFile: ChoiceFile = {
  read: () => {
    const file = new File(Paths.document, "appearance.txt");
    return file.exists ? file.textSync() : null;
  },
  write: (choice) => {
    const file = new File(Paths.document, "appearance.txt");
    if (!file.exists) file.create();
    file.writeSync(choice);
  },
};

/**
 * `ASWebAuthenticationSession` on iOS, a Custom Tab on Android: the system's browser, never a view
 * inside the app (RFC 8252). Only an address at `redirect` comes back to the app.
 */
export const systemBrowser: Browser = async (url, redirect) => {
  const result = await WebBrowser.openAuthSessionAsync(url, redirect);
  return result.type === "success" ? result.url : null;
};

/** React Native has `crypto.getRandomValues` from Expo, but no `crypto.subtle`. */
export const nativeCrypto: Crypto = {
  random: (length) => ExpoCrypto.getRandomBytes(length),
  sha256: async (data) =>
    new Uint8Array(
      await ExpoCrypto.digest(
        ExpoCrypto.CryptoDigestAlgorithm.SHA256,
        data as Uint8Array<ArrayBuffer>,
      ),
    ),
};

const platform: Platform =
  ReactPlatform.OS === "ios" ? "ios" : ReactPlatform.OS === "android" ? "android" : "other";

export const appVersion: string = Constants.expoConfig?.version ?? "0.0.0";

export const nativeEnv: Env = {
  crypto: nativeCrypto,
  platform,
  userAgent: `yardsort-mobile/${appVersion} (${platform})`,
  shelf: cacheShelf,
};

/**
 * What to call this phone at first. iOS gives apps only "iPhone" unless they hold an entitlement,
 * so the person is asked to change it.
 */
export function suggestedDeviceName(): string {
  return Device.deviceName ?? Device.modelName ?? "Phone";
}

/**
 * One picture from the phone's library, through the system's own picker, which needs no access to
 * the rest of the library. Sent as a JPEG no larger than {@link LONGEST_SIDE}: a photo straight off
 * the camera is several megabytes, and HEIC, which some agents cannot read.
 */
export const pickImage: PickImage = async () => {
  const chosen = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ["images"],
    allowsMultipleSelection: false,
    quality: 1,
  });
  const asset = chosen.canceled ? undefined : chosen.assets[0];
  if (!asset) return null;
  const longest = Math.max(asset.width, asset.height);
  const resize =
    longest > LONGEST_SIDE
      ? [
          {
            resize:
              asset.width >= asset.height ? { width: LONGEST_SIDE } : { height: LONGEST_SIDE },
          },
        ]
      : [];
  const made = await manipulateAsync(asset.uri, resize, {
    compress: 0.8,
    format: SaveFormat.JPEG,
  });
  const file = new File(made.uri);
  const bytes = await file.bytes();
  if (file.exists) file.delete();
  const stem = (asset.fileName ?? "photo").replace(/\.[^.]*$/, "");
  return { name: `${stem}.jpg`, bytes };
};
