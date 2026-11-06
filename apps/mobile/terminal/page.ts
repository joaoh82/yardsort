// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The page that draws a terminal, inside a WebView (31 §5). It draws what it is sent and nothing
 * else: `scripts/terminal-page.ts` builds it into one document, with no way to reach the network.
 *
 * Nothing typed here goes anywhere, since typing has a native field of its own, and nothing is ever
 * said back to a program: there is no listener for what xterm.js would answer, and the host has
 * already taken a program's questions out of what a phone is sent. A clipboard write in an escape sequence goes nowhere too, since the addon
 * that would carry it out is not here.
 */

import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";

import {
  type Drawn,
  fromBase64,
  linkable,
  readToPage,
  sameCell,
  type ToApp,
} from "../src/lib/bridge";

declare global {
  interface Window {
    ReactNativeWebView?: { postMessage(message: string): void };
  }
}

function say(message: ToApp): void {
  window.ReactNativeWebView?.postMessage(JSON.stringify(message));
}

function open(url: string): void {
  if (linkable(url)) say({ t: "link", url });
}

window.addEventListener("error", () => say({ t: "trouble", what: "script" }));

const terminal = new Terminal({
  allowProposedApi: true,
  cursorBlink: false,
  disableStdin: true,
  fontFamily: "Menlo, Consolas, 'DejaVu Sans Mono', monospace",
  fontSize: 12,
  // As much as the computer keeps (`pty-host`'s `SCROLLBACK_LINES`), which a snapshot carries.
  scrollback: 10_000,
  linkHandler: { activate: (_event, url) => open(url), allowNonHttpProtocols: false },
  theme: { background: "#0b0d10", foreground: "#e8eaed", cursor: "#f5b83d" },
});
terminal.loadAddon(new Unicode11Addon());
terminal.unicode.activeVersion = "11";
terminal.loadAddon(new WebLinksAddon((_event, url) => open(url)));
terminal.open(document.getElementById("terminal") as HTMLElement);

// Not to be typed into, so a tap must not bring up the keyboard.
const field = terminal.textarea;
if (field) {
  field.readOnly = true;
  field.inputMode = "none";
  field.tabIndex = -1;
}

let cellSaid: Drawn | null = null;

/** How large a character is, for the app to work out what fits. Read from what is drawn. */
function sayCell(): void {
  const screen = terminal.element?.querySelector(".xterm-screen")?.getBoundingClientRect();
  const { cols, rows } = terminal;
  if (!screen || cols === 0 || rows === 0 || screen.width <= 0 || screen.height <= 0) return;
  const drawn = { width: screen.width, height: screen.height, cols, rows };
  if (cellSaid && sameCell(cellSaid, drawn)) return;
  cellSaid = drawn;
  say({ t: "cell", width: drawn.width / cols, height: drawn.height / rows });
}

let applicationCursor = false;

/** Arrows are sent by the app, and what a program asked for changes what they should say. */
function sayCursorKeys(): void {
  const application = terminal.modes.applicationCursorKeysMode;
  if (application === applicationCursor) return;
  applicationCursor = application;
  say({ t: "cursorKeys", application });
}

/**
 * Run `step` after everything sent before it has been drawn. xterm.js parses what it is given later,
 * so a reset or a size applied at once would land in the middle of what came before it.
 */
function inTurn(step: () => void): void {
  terminal.write(new Uint8Array(0), step);
}

function heard(message: unknown): void {
  const m = readToPage(message);
  if (!m) {
    say({ t: "trouble", what: "unreadable" });
    return;
  }
  switch (m.t) {
    case "reset":
      inTurn(() => {
        terminal.reset();
        terminal.resize(m.cols, m.rows);
        sayCell();
        sayCursorKeys();
      });
      break;
    case "resize":
      inTurn(() => terminal.resize(m.cols, m.rows));
      break;
    case "output":
      terminal.write(fromBase64(m.data), () => {
        sayCursorKeys();
        say({ t: "drawn", id: m.id });
      });
      break;
  }
}

// One finger up or down moves through everything there is: the page while it can move, and past
// its top, what scrolled off the screen. Both are moved here, not by the WebView: a native scroll
// takes the gesture for itself, and the page then hears nothing more of it, so the history could
// never be reached. Sideways, and two fingers, are left to the WebView.
let finger: { y: number; x: number; at: number } | null = null;
let upright: boolean | null = null;
let carried = 0;
let speed = 0;
let gliding = 0;

/** The fastest a glide starts, in pixels a millisecond: a hard flick, and no more. */
const FASTEST = 6;

function lineHeight(): number {
  const rows = document.querySelector(".xterm-rows");
  return rows && terminal.rows > 0 ? rows.getBoundingClientRect().height / terminal.rows : 16;
}

/** Move the page by up to `by` pixels, down for more than 0. What it could not move is left. */
function movePage(by: number): number {
  const before = window.scrollY;
  window.scrollTo(window.scrollX, before + by);
  return by - (window.scrollY - before);
}

/** Move through what scrolled off by up to `by` pixels. What it could not move is left. */
function moveHistory(by: number): number {
  const buffer = terminal.buffer.active;
  const room = by < 0 ? buffer.viewportY : buffer.baseY - buffer.viewportY;
  if (room <= 0) {
    carried = 0;
    return by;
  }
  carried += by;
  const height = lineHeight();
  const wanted = Math.trunc(carried / height);
  const lines = Math.sign(wanted) * Math.min(Math.abs(wanted), room);
  if (lines !== 0) {
    carried -= lines * height;
    terminal.scrollLines(lines);
  }
  return 0;
}

/**
 * Up, the page goes to its top before the history moves; down, the history comes back to the
 * screen before the page moves. Returns whether anything could move.
 */
function move(by: number): boolean {
  let left = by;
  if (left < 0) left = movePage(left);
  if (left !== 0) left = moveHistory(left);
  if (left > 0) left = movePage(left);
  return left !== by;
}

function stopGliding(): void {
  cancelAnimationFrame(gliding);
  gliding = 0;
}

/** Carry on after the finger lifts, slowing, as a native scroll would. */
function glide(): void {
  let last = performance.now();
  const step = (now: number) => {
    const elapsed = Math.min(now - last, 50);
    last = now;
    speed *= Math.pow(0.995, elapsed);
    if (Math.abs(speed) < 0.02 || !move(speed * elapsed)) {
      gliding = 0;
      return;
    }
    gliding = requestAnimationFrame(step);
  };
  gliding = requestAnimationFrame(step);
}

document.addEventListener(
  "touchstart",
  (event) => {
    stopGliding();
    const touch = event.touches.length === 1 ? event.touches[0] : undefined;
    finger = touch ? { y: touch.clientY, x: touch.clientX, at: event.timeStamp } : null;
    upright = null;
    carried = 0;
    speed = 0;
  },
  { passive: true },
);

document.addEventListener(
  "touchmove",
  (event) => {
    const touch = event.touches.length === 1 ? event.touches[0] : undefined;
    if (!touch || !finger) {
      finger = null;
      return;
    }
    const dy = finger.y - touch.clientY;
    const dx = finger.x - touch.clientX;
    if (dx === 0 && dy === 0) return;
    // The first move decides: a native scroll cannot be stopped once it has started.
    if (upright === null) upright = Math.abs(dy) >= Math.abs(dx);
    if (!upright) {
      finger = null;
      return;
    }
    if (event.cancelable) event.preventDefault();
    const elapsed = Math.max(event.timeStamp - finger.at, 1);
    finger = { y: touch.clientY, x: touch.clientX, at: event.timeStamp };
    speed = Math.max(-FASTEST, Math.min(FASTEST, 0.8 * (dy / elapsed) + 0.2 * speed));
    move(dy);
  },
  { passive: false },
);

document.addEventListener(
  "touchend",
  (event) => {
    // A finger held still before lifting means stop there.
    const held = finger ? event.timeStamp - finger.at : Infinity;
    if (upright && held < 100) glide();
    finger = null;
  },
  { passive: true },
);

document.addEventListener("touchcancel", () => {
  finger = null;
});

// Android delivers a WebView's messages to the document, and iOS to the window.
window.addEventListener("message", (event) => heard(event.data));
document.addEventListener("message", (event) => heard((event as MessageEvent).data));

say({ t: "ready" });
