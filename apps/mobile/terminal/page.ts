// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The page that draws a terminal, inside a WebView (31 §5). It draws what it is sent and nothing
 * else: `scripts/terminal-page.ts` builds it into one document, with no way to reach the network.
 *
 * Nothing typed here goes anywhere, and nothing is ever said back to a program: there is no
 * listener for what xterm.js would answer, and the host has already taken a program's questions out
 * of what a phone is sent. A clipboard write in an escape sequence goes nowhere too, since the addon
 * that would carry it out is not here.
 */

import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";

import { fromBase64, linkable, readToPage, type ToApp } from "../src/lib/bridge";

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
  scrollback: 5000,
  linkHandler: { activate: (_event, url) => open(url), allowNonHttpProtocols: false },
  theme: { background: "#151514", foreground: "#ecece8", cursor: "#ecece8" },
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
      });
      break;
    case "resize":
      inTurn(() => terminal.resize(m.cols, m.rows));
      break;
    case "output":
      terminal.write(fromBase64(m.data), () => say({ t: "drawn", id: m.id }));
      break;
  }
}

// One finger up or down moves the page while it can move, and then through what scrolled off the
// top: xterm.js scrolls by wheel only. Sideways, and two fingers, are left to the WebView.
let finger: { y: number; x: number } | null = null;
let carried = 0;

function lineHeight(): number {
  const rows = document.querySelector(".xterm-rows");
  return rows && terminal.rows > 0 ? rows.getBoundingClientRect().height / terminal.rows : 16;
}

function pageCanMove(down: boolean): boolean {
  const { scrollTop, scrollHeight, clientHeight } = document.documentElement;
  return down ? scrollTop + clientHeight < scrollHeight - 1 : scrollTop > 0;
}

document.addEventListener(
  "touchstart",
  (event) => {
    const touch = event.touches.length === 1 ? event.touches[0] : undefined;
    finger = touch ? { y: touch.clientY, x: touch.clientX } : null;
    carried = 0;
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
    finger = { y: touch.clientY, x: touch.clientX };
    if (Math.abs(dx) > Math.abs(dy) || pageCanMove(dy > 0)) return;
    carried += dy;
    const height = lineHeight();
    const lines = Math.trunc(carried / height);
    if (lines !== 0) {
      carried -= lines * height;
      terminal.scrollLines(lines);
    }
  },
  { passive: true },
);

// Android delivers a WebView's messages to the document, and iOS to the window.
window.addEventListener("message", (event) => heard(event.data));
document.addEventListener("message", (event) => heard((event as MessageEvent).data));

say({ t: "ready" });
