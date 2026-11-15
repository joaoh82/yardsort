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
const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function encode(bytes: Uint8Array, alphabet: string): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i] as number;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    out += alphabet[a >> 2];
    out += alphabet[((a & 3) << 4) | ((b ?? 0) >> 4)];
    if (b !== undefined) out += alphabet[((b & 15) << 2) | ((c ?? 0) >> 6)];
    if (c !== undefined) out += alphabet[c & 63];
  }
  return out;
}

/** Base64url without padding. */
export function base64url(bytes: Uint8Array): string {
  return encode(bytes, BASE64URL);
}

/** Standard base64, padded: as an upload's pieces go. */
export function base64(bytes: Uint8Array): string {
  const out = encode(bytes, BASE64);
  return out.padEnd(Math.ceil(out.length / 4) * 4, "=");
}

const VALUES = new Int8Array(128).fill(-1);
for (let i = 0; i < BASE64.length; i++) VALUES[BASE64.charCodeAt(i)] = i;

/**
 * The bytes of standard base64, padded: as a file's pieces come. `null` for anything else, as
 * Rust's decoder refuses it: a length that is not whole fours, a character outside the alphabet,
 * padding anywhere but the end, or bits left over after the last byte.
 */
export function fromBase64(text: string): Uint8Array | null {
  if (text.length % 4 !== 0) return null;
  const padding = text.endsWith("==") ? 2 : text.endsWith("=") ? 1 : 0;
  const bytes = new Uint8Array((text.length / 4) * 3 - padding);
  let at = 0;
  for (let i = 0; i < text.length; i += 4) {
    const digits = i + 4 === text.length ? 4 - padding : 4;
    let bits = 0;
    for (let j = 0; j < 4; j++) {
      const unit = text.charCodeAt(i + j);
      const value = j >= digits ? 0 : unit < 128 ? (VALUES[unit] as number) : -1;
      if (value < 0) return null;
      bits = (bits << 6) | value;
    }
    // Two digits carry one byte and three carry two: what they hold past that has to be nothing.
    if ((digits === 2 && bits & 0xffff) || (digits === 3 && bits & 0xff)) return null;
    bytes[at++] = bits >> 16;
    if (digits > 2) bytes[at++] = (bits >> 8) & 255;
    if (digits > 3) bytes[at++] = bits & 255;
  }
  return bytes;
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
