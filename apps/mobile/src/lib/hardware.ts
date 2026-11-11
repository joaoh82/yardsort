// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A keyboard plugged into the phone, or paired with it (35 §17, slice 9). The native view in
 * `modules/hardware-keys` hears the keys a terminal needs and the field cannot give it, and says
 * which; this turns each into what a terminal would send for it.
 *
 * Letters, digits and Return still go into the field, and Return sends it, as on the screen's own
 * keyboard. What the view takes instead, while control is held:
 *
 * - **Esc**, always, and **Shift-Tab**.
 * - **Ctrl** with a letter or `[ \ ]` or space, and **Alt** (Option) with a letter, a digit or a
 *   symbol, or with Shift and a letter: the field has no use for them.
 * - **The arrows** with Shift, Alt or Ctrl, and **Backspace** with Alt or Ctrl.
 * - **The arrows, Tab and Backspace on their own** only while the field is empty, so a draft can
 *   still be edited with them; once it is sent, they move through a program's menu again.
 *
 * **Cmd** is never taken: its shortcuts are the system's. The native code keeps the same lists as
 * {@link CTRL_CHARS} and {@link ALT_CHARS}, and takes a key only where {@link takes} says so.
 */

import { bytesOf, controlByte, KEY_NAMES } from "./keys";

/** What the native view says was pressed. */
export interface HardwareKey {
  /** A key by name, or the one character the key makes without any modifier, such as `c`. */
  key: "escape" | "tab" | "backspace" | "up" | "down" | "left" | "right" | (string & {});
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

/** The characters taken with Ctrl. */
export const CTRL_CHARS = "abcdefghijklmnopqrstuvwxyz[\\] ";
/** The characters taken with Alt. */
export const ALT_CHARS = "abcdefghijklmnopqrstuvwxyz0123456789`-=[]\\;',./";

const LETTERS = /^[a-z]$/;

const ARROWS = ["up", "down", "left", "right"] as const;
type Arrow = (typeof ARROWS)[number];
const ARROW_FINALS: Record<Arrow, string> = { up: "A", down: "B", right: "C", left: "D" };

const isArrow = (key: string): key is Arrow => (ARROWS as readonly string[]).includes(key);

/**
 * Whether the view takes `press` from the field: `plain` is whether the arrows, Tab and Backspace
 * on their own go to the terminal, which they do while the field is empty.
 */
export function takes(press: HardwareKey, plain: boolean): boolean {
  const { key, ctrl, alt, shift } = press;
  if (key === "escape") return true;
  if (isArrow(key)) return plain || ctrl || alt || shift;
  if (key === "tab") return !ctrl && !alt && (shift || plain);
  if (key === "backspace") return ctrl || alt || plain;
  if (key.length !== 1) return false;
  if (ctrl) return CTRL_CHARS.includes(key);
  // With Shift too, only a letter: Shift changes what the others are by the layout.
  if (alt) return ALT_CHARS.includes(key) && (!shift || LETTERS.test(key));
  return false;
}

/**
 * What a terminal sends for `press`, and what to call it if it may not have arrived; `null` for a
 * key the view would not take. Alt sends Esc first, as a terminal set to treat Alt as Meta does.
 */
export function hardwareBytes(
  press: HardwareKey,
  applicationCursor: boolean,
): { said: string; bytes: string } | null {
  if (!takes(press, true)) return null;
  const { key, ctrl, alt, shift } = press;
  const said = (name: string) =>
    [ctrl && "Ctrl", alt && "Alt", shift && "Shift", name].filter(Boolean).join("-");

  if (isArrow(key)) {
    const name = KEY_NAMES[key];
    if (!ctrl && !alt && !shift)
      return { said: name, bytes: bytesOf(key, { ctrl: false, applicationCursor }) };
    // xterm's modifier parameter: 1, plus 1 for Shift, 2 for Alt and 4 for Ctrl.
    const modifier = 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0);
    return { said: said(name), bytes: `\x1b[1;${modifier}${ARROW_FINALS[key]}` };
  }
  if (key === "escape") return { said: said(KEY_NAMES.escape), bytes: "\x1b" };
  if (key === "tab") return { said: said("Tab"), bytes: shift ? "\x1b[Z" : "\t" };
  if (key === "backspace") {
    const bytes = ctrl ? "\x08" : "\x7f";
    return { said: said("Backspace"), bytes: alt ? `\x1b${bytes}` : bytes };
  }
  if (ctrl) {
    const byte = controlByte(key);
    if (byte === null) return null;
    return {
      said: said(key === " " ? "Space" : key.toUpperCase()),
      bytes: alt ? `\x1b${byte}` : byte,
    };
  }
  return { said: said(key.toUpperCase()), bytes: `\x1b${shift ? key.toUpperCase() : key}` };
}
