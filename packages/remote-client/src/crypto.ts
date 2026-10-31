// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * PKCE (RFC 7636): a random verifier the device keeps, and its SHA-256 (the challenge), which is
 * all the browser carries. A code read from a redirect is worth nothing without the verifier.
 *
 * Randomness and SHA-256 come from the platform: Web Crypto where there is one (Bun, browsers),
 * and whatever the app passes where there is not (React Native has no `crypto.subtle`).
 */

export interface Crypto {
  /** `length` bytes from the operating system's random source. */
  random(length: number): Uint8Array;
  sha256(data: Uint8Array): Promise<Uint8Array>;
}

/** Web Crypto's, or `null` on a platform without it. */
export function webCrypto(): Crypto | null {
  const crypto = (globalThis as { crypto?: globalThis.Crypto }).crypto;
  if (!crypto?.getRandomValues || !crypto.subtle) return null;
  return {
    random: (length) => crypto.getRandomValues(new Uint8Array(length)),
    sha256: async (data) =>
      new Uint8Array(await crypto.subtle.digest("SHA-256", data as Uint8Array<ArrayBuffer>)),
  };
}

const BASE64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/** Base64url without padding. */
export function base64url(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i] as number;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    out += BASE64URL[a >> 2];
    out += BASE64URL[((a & 3) << 4) | ((b ?? 0) >> 4)];
    if (b !== undefined) out += BASE64URL[((b & 15) << 2) | ((c ?? 0) >> 6)];
    if (c !== undefined) out += BASE64URL[c & 63];
  }
  return out;
}

/** 256 random bits as base64url: a verifier of 43 characters. */
export function newVerifier(crypto: Crypto): string {
  return base64url(crypto.random(32));
}

/** The S256 challenge for `verifier`. */
export async function challengeFor(crypto: Crypto, verifier: string): Promise<string> {
  const ascii = new Uint8Array(verifier.length);
  for (let i = 0; i < verifier.length; i++) ascii[i] = verifier.charCodeAt(i);
  return base64url(await crypto.sha256(ascii));
}
