// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What the app and the terminal's page say to each other (31 §5): typed, bounded, and checked by
 * whichever side reads it. The page is never told a credential, a ticket or a server's address, and
 * this file is all the two have in common, so it imports nothing.
 */

/** The longest message either side reads: a whole frame of output, in base64, and its wrapping. */
export const MAX_MESSAGE = 96 * 1024;
/** The most columns or rows the page draws, which is more than any real terminal has. */
export const MAX_SIDE = 1000;
/** The longest address a link may have. */
export const MAX_LINK = 2048;

export interface PageSize {
  cols: number;
  rows: number;
}

export type ToPage =
  /** Forget everything, and draw at this size: a snapshot follows. */
  | ({ t: "reset" } & PageSize)
  /** The terminal's own bytes, in base64. The page answers `drawn` with `id`. */
  | { t: "output"; id: number; data: string }
  /** The program draws at this size from here on. */
  | ({ t: "resize" } & PageSize);

export type ToApp =
  /** The page has started, with nothing drawn: on its first load, and after being reloaded. */
  | { t: "ready" }
  | { t: "drawn"; id: number }
  /** The person tapped a link. */
  | { t: "link"; url: string }
  /** Something went wrong in the page. Which, and never what the terminal said. */
  | { t: "trouble"; what: "script" | "unreadable" };

function record(message: unknown): Record<string, unknown> | null {
  if (typeof message !== "string" || message.length > MAX_MESSAGE) return null;
  try {
    const value: unknown = JSON.parse(message);
    return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

function side(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0 && value <= MAX_SIDE;
}

function counter(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/** Whether a size can be drawn. */
export function drawable(size: PageSize): boolean {
  return side(size.cols) && side(size.rows);
}

/** A message for the page, or `null` if this is not one. */
export function readToPage(message: unknown): ToPage | null {
  const m = record(message);
  if (!m) return null;
  switch (m.t) {
    case "reset":
    case "resize":
      return side(m.cols) && side(m.rows) ? { t: m.t, cols: m.cols, rows: m.rows } : null;
    case "output":
      return counter(m.id) && typeof m.data === "string" && /^[A-Za-z0-9+/]*={0,2}$/.test(m.data)
        ? { t: "output", id: m.id, data: m.data }
        : null;
    default:
      return null;
  }
}

/** A message for the app, or `null` if this is not one. */
export function readToApp(message: unknown): ToApp | null {
  const m = record(message);
  if (!m) return null;
  switch (m.t) {
    case "ready":
      return { t: "ready" };
    case "drawn":
      return counter(m.id) ? { t: "drawn", id: m.id } : null;
    case "link":
      return typeof m.url === "string" && linkable(m.url) ? { t: "link", url: m.url } : null;
    case "trouble":
      return m.what === "script" || m.what === "unreadable" ? { t: "trouble", what: m.what } : null;
    default:
      return null;
  }
}

/** Whether a link may be offered: `https`, one line, and bounded. */
export function linkable(url: string): boolean {
  return (
    url.length <= MAX_LINK &&
    /^https:\/\/[^\s/?#]/i.test(url) &&
    !/[\u0000-\u001f\u007f\s]/.test(url)
  );
}

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** The bytes as base64, which a message between the two has to be. */
export function toBase64(bytes: Uint8Array): string {
  const parts: string[] = [];
  let line = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i] as number;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    line += ALPHABET.charAt(a >> 2);
    line += ALPHABET.charAt(((a & 3) << 4) | ((b ?? 0) >> 4));
    line += b === undefined ? "=" : ALPHABET.charAt(((b & 15) << 2) | ((c ?? 0) >> 6));
    line += c === undefined ? "=" : ALPHABET.charAt(c & 63);
    if (line.length >= 4096) {
      parts.push(line);
      line = "";
    }
  }
  parts.push(line);
  return parts.join("");
}

/** Back to the bytes. Only ever given what {@link readToPage} accepted. */
export function fromBase64(text: string): Uint8Array {
  const body = text.replace(/=+$/, "");
  const bytes = new Uint8Array(Math.floor((body.length * 3) / 4));
  let bits = 0;
  let held = 0;
  let at = 0;
  for (let i = 0; i < body.length; i += 1) {
    bits = (bits << 6) | ALPHABET.indexOf(body.charAt(i));
    held += 6;
    if (held >= 8) {
      held -= 8;
      bytes[at] = (bits >> held) & 255;
      at += 1;
    }
    bits &= 255;
  }
  return bytes;
}
