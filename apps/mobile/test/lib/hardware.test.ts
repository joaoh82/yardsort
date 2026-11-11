// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A keyboard paired with the phone: which keys are taken from the field, and what each sends
// (35 §17, slice 9).

import { describe, expect, test } from "bun:test";

import {
  ALT_CHARS,
  CTRL_CHARS,
  type HardwareKey,
  hardwareBytes,
  takes,
} from "../../src/lib/hardware";

const press = (key: string, mods: Partial<Omit<HardwareKey, "key">> = {}): HardwareKey => ({
  key,
  ctrl: false,
  alt: false,
  shift: false,
  ...mods,
});
const bytes = (key: HardwareKey, applicationCursor = false) =>
  hardwareBytes(key, applicationCursor)?.bytes ?? null;
const said = (key: HardwareKey) => hardwareBytes(key, false)?.said ?? null;

describe("what is taken from the field", () => {
  test("Esc always, and Shift-Tab", () => {
    expect(takes(press("escape"), false)).toBe(true);
    expect(takes(press("tab", { shift: true }), false)).toBe(true);
  });

  test("the arrows, Tab and Backspace on their own only while the field is empty", () => {
    for (const key of ["up", "down", "left", "right", "tab", "backspace"]) {
      expect(takes(press(key), true)).toBe(true);
      expect(takes(press(key), false)).toBe(false);
    }
  });

  test("the arrows with a modifier, and Backspace with Ctrl or Alt, whatever the field holds", () => {
    for (const mods of [{ shift: true }, { alt: true }, { ctrl: true }]) {
      expect(takes(press("left", mods), false)).toBe(true);
    }
    expect(takes(press("backspace", { alt: true }), false)).toBe(true);
    expect(takes(press("backspace", { ctrl: true }), false)).toBe(true);
    expect(takes(press("backspace", { shift: true }), false)).toBe(false);
  });

  test("Ctrl and Alt with a character, but never a character on its own or with Shift", () => {
    for (const char of CTRL_CHARS) expect(takes(press(char, { ctrl: true }), false)).toBe(true);
    for (const char of ALT_CHARS) expect(takes(press(char, { alt: true }), false)).toBe(true);
    expect(takes(press("c"), true)).toBe(false);
    expect(takes(press("c", { shift: true }), true)).toBe(false);
    expect(takes(press("1", { ctrl: true }), false)).toBe(false);
    expect(takes(press("b", { alt: true, shift: true }), false)).toBe(true);
    expect(takes(press("1", { alt: true, shift: true }), false)).toBe(false);
  });

  test("Ctrl-Tab and Alt-Tab, shifted or not, are left to the system, and Return and named keys it does not know", () => {
    expect(takes(press("tab", { ctrl: true }), true)).toBe(false);
    expect(takes(press("tab", { alt: true }), true)).toBe(false);
    expect(takes(press("tab", { ctrl: true, shift: true }), false)).toBe(false);
    expect(takes(press("tab", { alt: true, shift: true }), false)).toBe(false);
    expect(takes(press("enter"), true)).toBe(false);
    expect(takes(press("home"), true)).toBe(false);
  });
});

describe("what each key sends", () => {
  test("Esc, Tab, Shift-Tab and Backspace", () => {
    expect(bytes(press("escape"))).toBe("\x1b");
    expect(bytes(press("tab"))).toBe("\t");
    expect(bytes(press("tab", { shift: true }))).toBe("\x1b[Z");
    expect(bytes(press("backspace"))).toBe("\x7f");
    expect(bytes(press("backspace", { alt: true }))).toBe("\x1b\x7f");
    expect(bytes(press("backspace", { ctrl: true }))).toBe("\x08");
  });

  test("arrows as the program asked for them, and with modifiers as xterm sends them", () => {
    expect(bytes(press("up"))).toBe("\x1b[A");
    expect(bytes(press("up"), true)).toBe("\x1bOA");
    expect(bytes(press("right", { shift: true }))).toBe("\x1b[1;2C");
    expect(bytes(press("left", { alt: true }))).toBe("\x1b[1;3D");
    expect(bytes(press("down", { ctrl: true }), true)).toBe("\x1b[1;5B");
    expect(bytes(press("up", { ctrl: true, shift: true }))).toBe("\x1b[1;6A");
  });

  test("Ctrl with a letter is its control character, and with Alt, Esc first", () => {
    expect(bytes(press("c", { ctrl: true }))).toBe("\x03");
    expect(bytes(press("c", { ctrl: true, shift: true }))).toBe("\x03");
    expect(bytes(press("[", { ctrl: true }))).toBe("\x1b");
    expect(bytes(press(" ", { ctrl: true }))).toBe("\x00");
    expect(bytes(press("x", { ctrl: true, alt: true }))).toBe("\x1b\x18");
  });

  test("Alt sends Esc and then the character", () => {
    expect(bytes(press("b", { alt: true }))).toBe("\x1bb");
    expect(bytes(press(".", { alt: true }))).toBe("\x1b.");
    expect(bytes(press("b", { alt: true, shift: true }))).toBe("\x1bB");
  });

  test("a key the view would not take sends nothing", () => {
    expect(hardwareBytes(press("c"), false)).toBeNull();
    expect(hardwareBytes(press("1", { ctrl: true }), false)).toBeNull();
  });

  test("each is named as it was pressed", () => {
    expect(said(press("c", { ctrl: true }))).toBe("Ctrl-C");
    expect(said(press("b", { alt: true }))).toBe("Alt-B");
    expect(said(press("tab", { shift: true }))).toBe("Shift-Tab");
    expect(said(press("left", { ctrl: true }))).toBe("Ctrl-Left arrow");
    expect(said(press("escape"))).toBe("Escape");
    expect(said(press(" ", { ctrl: true }))).toBe("Ctrl-Space");
  });
});
