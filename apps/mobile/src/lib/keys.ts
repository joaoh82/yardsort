// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The keys a phone lacks, as the bytes a terminal would send for them (31 §5). */

export type Key = "escape" | "tab" | "up" | "down" | "left" | "right" | "backspace" | "enter";

const ARROWS: Partial<Record<Key, string>> = { up: "A", down: "B", right: "C", left: "D" };

const OTHERS: Partial<Record<Key, string>> = {
  escape: "\x1b",
  tab: "\t",
  backspace: "\x7f",
  enter: "\r",
};

/** What a key says to a screen reader, and to the person when it may not have arrived. */
export const KEY_NAMES: Record<Key, string> = {
  escape: "Escape",
  tab: "Tab",
  up: "Up arrow",
  down: "Down arrow",
  left: "Left arrow",
  right: "Right arrow",
  backspace: "Backspace",
  enter: "Enter",
};

/**
 * What pressing `key` sends. A program that asked for application cursor keys, as a full-screen one
 * does, reads arrows differently. Ctrl changes only the arrows here: a letter is typed to go with it.
 */
export function bytesOf(key: Key, how: { ctrl: boolean; applicationCursor: boolean }): string {
  const arrow = ARROWS[key];
  if (!arrow) return OTHERS[key] as string;
  if (how.ctrl) return `\x1b[1;5${arrow}`;
  return `${how.applicationCursor ? "\x1bO" : "\x1b["}${arrow}`;
}

/** The byte for Ctrl and `char`, or `null` when there is no such thing, as for a digit. */
export function controlByte(char: string): string | null {
  if (char === " ") return "\x00";
  if (char === "?") return "\x7f";
  const code = char.toUpperCase().charCodeAt(0);
  return char.length === 1 && code >= 0x40 && code <= 0x5f
    ? String.fromCharCode(code - 0x40)
    : null;
}

/** The one character a change added to a field, or `null` when it did anything else. */
export function addedCharacter(before: string, after: string): string | null {
  if (after.length !== before.length + 1) return null;
  for (let at = 0; at < after.length; at += 1) {
    if (after.charAt(at) !== before.charAt(at)) {
      return after.slice(0, at) + after.slice(at + 1) === before ? after.charAt(at) : null;
    }
  }
  return null;
}
