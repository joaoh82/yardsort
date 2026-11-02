// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The terminal's page, as built, in a real browser at a phone's size: what it draws, what it says,
 * and what its policy stops. Chromium is not a phone's WebView, so this does not show that it works
 * on one. Skipped, saying so, where no Chromium-family browser is found.
 */

import { afterAll, beforeAll, describe, expect, setDefaultTimeout, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { TERMINAL_PAGE } from "../../src/ui/terminal-page.generated";

const WAIT_MS = 30_000;
const START_MS = 120_000;
const CLIP = { x: 0, y: 0, width: 10, height: 10, scale: 1 };

// Past the default, or a slow runner's test is cut off and the browser killed with it.
setDefaultTimeout(WAIT_MS * 2);

function findBrowser(): string | null {
  const named = [
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "brave",
    "microsoft-edge",
  ];
  for (const name of named) {
    const found = Bun.which(name);
    if (found) return found;
  }
  const paths = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  ];
  for (const path of paths) {
    if (Bun.file(path).size > 0) return path;
  }
  return null;
}

async function until<T>(what: string, read: () => Promise<T>, done: (value: T) => boolean) {
  const deadline = Date.now() + WAIT_MS;
  for (;;) {
    const value = await read();
    if (done(value)) return value;
    if (Date.now() > deadline) throw new Error(`never ${what}: ${JSON.stringify(value)}`);
    await Bun.sleep(25);
  }
}

/** A tab, driven over the DevTools protocol. */
class Tab {
  #next = 1;
  #waiting = new Map<number, (result: Record<string, unknown>) => void>();

  constructor(readonly socket: WebSocket) {
    socket.onmessage = (event) => {
      const { id, result, error } = JSON.parse(String(event.data));
      if (id !== undefined) this.#waiting.get(id)?.(error ? { error } : result);
    };
  }

  send(method: string, params: Record<string, unknown> = {}): Promise<Record<string, unknown>> {
    const id = this.#next;
    this.#next += 1;
    return new Promise((resolve) => {
      this.#waiting.set(id, resolve);
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async read<T>(expression: string): Promise<T> {
    const answer = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    return (answer.result as { value: T } | undefined)?.value as T;
  }
}

const browserPath = findBrowser();
if (!browserPath) console.warn("skipped: no Chromium-family browser found");

describe.skipIf(!browserPath)("the terminal's page", () => {
  let process: ReturnType<typeof Bun.spawn> | undefined;
  let tab: Tab;
  let debugPort = "";
  let lastTabId = "";
  const profile = mkdtempSync(join(tmpdir(), "ys-terminal-page-"));

  beforeAll(async () => {
    process = Bun.spawn(
      [
        browserPath as string,
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--no-first-run",
        `--user-data-dir=${profile}`,
        "--remote-debugging-port=0",
        "about:blank",
      ],
      { stdout: "ignore", stderr: "ignore" },
    );
    const deadline = Date.now() + START_MS;
    let port = "";
    while (!port) {
      if (Date.now() > deadline) throw new Error("the browser did not start");
      try {
        port = readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0] ?? "";
      } catch {
        await Bun.sleep(100);
      }
    }
    debugPort = port;
    await load();
  }, START_MS);

  afterAll(() => {
    tab?.socket.close();
    process?.kill();
    try {
      rmSync(profile, { recursive: true, force: true });
    } catch {
      // The browser may still hold it on Windows, and it is in the temporary folder.
    }
  });

  const said = () => tab.read<{ t: string; id?: number; url?: string; what?: string }[]>("said");
  /** What is on the screen. A headless tab paints when asked to, so this asks. */
  const screenText = async () => {
    await tab.send("Page.captureScreenshot", { format: "png", clip: CLIP });
    return tab.read<string>(
      "document.querySelector('.xterm-rows').innerText.replace(/\\u00a0/g, ' ')",
    );
  };

  /** Saying it drew something comes just before the screen is painted. */
  const onScreen = (text: string) =>
    until(`showed ${text}`, screenText, (shown) => shown.includes(text));

  /**
   * The page freshly opened in a tab of its own, as the app loads it. A headless browser's tab can
   * stop painting after many loads, and a phone's WebView loads the page once.
   */
  async function load(from?: string): Promise<void> {
    const before = tab?.socket;
    const opened = (await fetch(`http://127.0.0.1:${debugPort}/json/new?about:blank`, {
      method: "PUT",
    }).then((r) => r.json())) as { webSocketDebuggerUrl: string; id: string };
    const socket = new WebSocket(opened.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.onopen = resolve;
      socket.onerror = reject;
    });
    tab = new Tab(socket);
    await tab.send("Page.enable");
    await tab.send("Page.bringToFront");
    await tab.send("Runtime.enable");
    await tab.send("Emulation.setDeviceMetricsOverride", {
      width: 390,
      height: 800,
      deviceScaleFactor: 2,
      mobile: true,
    });
    await tab.send("Emulation.setTouchEmulationEnabled", { enabled: true });
    await tab.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    // As a WebView gives the page its way out: no part of what the page was built with.
    await tab.send("Page.addScriptToEvaluateOnNewDocument", {
      source:
        "window.said=[];window.ReactNativeWebView={postMessage:(m)=>window.said.push(JSON.parse(m))};",
    });
    const url = from ?? `data:text/html;base64,${Buffer.from(TERMINAL_PAGE).toString("base64")}`;
    await tab.send("Page.navigate", { url });
    await until(
      "said it was ready",
      () => tab.read<{ t: string }[] | undefined>("window.said").catch(() => undefined),
      (s) => s?.some((m) => m.t === "ready") ?? false,
    );
    // Only the tab in use is left: an old one in the background is not painted, and costs memory.
    if (before) {
      const closing = lastTabId;
      before.close();
      await fetch(`http://127.0.0.1:${debugPort}/json/close/${closing}`).catch(() => {});
    }
    lastTabId = opened.id;
  }

  async function post(message: unknown): Promise<void> {
    await tab.read(
      `window.dispatchEvent(new MessageEvent('message',{data:${JSON.stringify(JSON.stringify(message))}}));0`,
    );
  }

  /** Several messages in one go, as a busy channel delivers them: nothing is parsed in between. */
  async function postAll(messages: unknown[]): Promise<void> {
    const events = messages
      .map(
        (m) =>
          `window.dispatchEvent(new MessageEvent('message',{data:${JSON.stringify(JSON.stringify(m))}}));`,
      )
      .join("");
    await tab.read(`${events}0`);
  }

  let nextId = 1000;
  /** Send output, and wait for the page to say it is drawn. */
  async function print(text: string): Promise<void> {
    nextId += 1;
    const id = nextId;
    await post({ t: "output", id, data: Buffer.from(text).toString("base64") });
    await until("said it drew it", said, (s) => s.some((m) => m.t === "drawn" && m.id === id));
  }

  test("says it is ready, draws what it is sent, and says when", async () => {
    await load();
    expect((await said())[0]).toEqual({ t: "ready" });
    await post({ t: "reset", cols: 40, rows: 10 });
    await print("hello from the program");
    await onScreen("hello from the program");
  });

  test("draws a split multi-byte character whole, in the terminal's own bytes", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    const bytes = Buffer.from("olá ✓");
    await post({ t: "output", id: 1, data: bytes.subarray(0, 3).toString("base64") });
    await post({ t: "output", id: 2, data: bytes.subarray(3).toString("base64") });
    await until("said it drew it", said, (s) => s.some((m) => m.id === 2));
    await onScreen("olá ✓");
  });

  test("a reset replaces the screen, even right after output still being drawn", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    await postAll([
      { t: "output", id: 7, data: Buffer.from("old-screen").toString("base64") },
      { t: "reset", cols: 40, rows: 10 },
    ]);
    await print("new-screen");
    await onScreen("new-screen");
    expect(await screenText()).not.toContain("old-screen");
  });

  test("is drawn at the computer's size, wider than the phone, and moves sideways", async () => {
    await load();
    await post({ t: "reset", cols: 200, rows: 10 });
    await print(`${"x".repeat(199)}END`);
    await until(
      "grew wider than the phone",
      () => tab.read<number>("document.documentElement.scrollWidth"),
      (wide) => wide > 390 * 2,
    );
    // By a finger, sideways, and with no empty page to wander off into below the terminal.
    await tab.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x: 300, y: 60 }],
    });
    for (let i = 1; i <= 10; i += 1) {
      await tab.send("Input.dispatchTouchEvent", {
        type: "touchMove",
        touchPoints: [{ x: 300 - 20 * i, y: 60 }],
      });
    }
    await tab.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await until(
      "moved sideways",
      () => tab.read<number>("document.documentElement.scrollLeft"),
      (left) => left > 0,
    );
    const { pageHeight, phoneHeight } = await tab.read<{ pageHeight: number; phoneHeight: number }>(
      "({pageHeight:document.documentElement.scrollHeight,phoneHeight:window.innerHeight})",
    );
    expect(pageHeight).toBeLessThanOrEqual(phoneHeight);
  });

  test("says how large a character is, and what fits by that leaves nothing to move to", async () => {
    await load();
    await post({ t: "reset", cols: 200, rows: 60 });
    await print("wide");
    const cell = (await said()).filter((m) => m.t === "cell").at(-1) as unknown as {
      width: number;
      height: number;
    };
    expect(cell.width).toBeGreaterThan(3);
    expect(cell.height).toBeGreaterThan(cell.width);
    // Wider and taller than the phone, as watching a computer's terminal is.
    expect(await tab.read<number>("document.documentElement.scrollWidth")).toBeGreaterThan(390);

    // Worked out as the app does, the terminal now fills the phone without going past it.
    const cols = Math.floor(390 / cell.width);
    const rows = Math.floor(800 / cell.height);
    await post({ t: "resize", cols, rows });
    await print(`${"x".repeat(cols - 3)}END`);
    await onScreen("END");
    const page = await tab.read<{ wide: number; tall: number; right: number; bottom: number }>(
      `(() => {
        const screen = document.querySelector('.xterm-screen').getBoundingClientRect();
        const root = document.documentElement;
        return { wide: root.scrollWidth, tall: root.scrollHeight, right: screen.right, bottom: screen.bottom };
      })()`,
    );
    expect(page.wide).toBeLessThanOrEqual(390);
    expect(page.tall).toBeLessThanOrEqual(800);
    expect(page.right).toBeLessThanOrEqual(390);
    expect(page.bottom).toBeLessThanOrEqual(800);
    // There was room for no more: one more column or row would not have fitted.
    expect(page.right + cell.width).toBeGreaterThan(390);
    expect(page.bottom + cell.height).toBeGreaterThan(800);
  });

  test("says a character's size again only if it changed", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    await print("one");
    await post({ t: "reset", cols: 60, rows: 20 });
    await print("two");
    expect((await said()).filter((m) => m.t === "cell").length).toBe(1);
  });

  test("says when a program asks for application cursor keys, and when that ends", async () => {
    await load();
    const keys = async () =>
      (await said()).filter((m) => m.t === "cursorKeys") as unknown as { application: boolean }[];
    await post({ t: "reset", cols: 40, rows: 10 });
    await print("plain");
    expect(await keys()).toEqual([]);

    await print("\x1b[?1h");
    expect((await keys()).map((m) => m.application)).toEqual([true]);
    await print("still");
    expect((await keys()).length).toBe(1);

    await print("\x1b[?1l");
    expect((await keys()).map((m) => m.application)).toEqual([true, false]);

    // A new snapshot starts from nothing, and says it again if the program still wants it.
    await print("\x1b[?1h");
    await post({ t: "reset", cols: 40, rows: 10 });
    await print("\x1b[?1h");
    await until(
      "said both",
      keys,
      (k) => k.map((m) => m.application).join() === "true,false,true,false,true",
    );
  });

  test("a new size applies in its place among the output", async () => {
    await load();
    await post({ t: "reset", cols: 100, rows: 10 });
    // Written at column 90 of 100, then narrowed, it is cut off. Narrowed first, it would have been
    // written at the new edge, and show.
    await postAll([
      { t: "output", id: 11, data: Buffer.from("\u001b[1;90HX").toString("base64") },
      { t: "resize", cols: 80, rows: 6 },
    ]);
    await print("\u001b[6;1Hafter");
    await onScreen("after");
    const rows = await tab.read<string[]>(
      "Array.from(document.querySelector('.xterm-rows').children).map((row) => row.textContent)",
    );
    expect(rows).toHaveLength(6);
    expect(rows.join("")).not.toContain("X");
  });

  test("answers no program's question, and takes no typing", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    // Device attributes, cursor position, device status, window size.
    await print("\u001b[c\u001b[6n\u001b[5n\u001b[18t\u001b]10;?\u0007");
    await tab.send("Input.insertText", { text: "typed" });
    await Bun.sleep(150);
    const kinds = (await said()).map((m) => m.t);
    expect(kinds.filter((k) => k !== "ready" && k !== "drawn" && k !== "cell")).toEqual([]);
    const field = await tab.read<{ readOnly: boolean; inputMode: string }>(
      "(()=>{const f=document.querySelector('.xterm-helper-textarea');return {readOnly:f.readOnly,inputMode:f.inputMode}})()",
    );
    expect(field).toEqual({ readOnly: true, inputMode: "none" });
  });

  test("reaches nothing: no fetch, no image, no frame, no script but its own", async () => {
    let hits = 0;
    const listening = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      fetch: (request) => {
        if (new URL(request.url).pathname === "/") {
          return new Response(TERMINAL_PAGE, { headers: { "content-type": "text/html" } });
        }
        if (new URL(request.url).pathname !== "/favicon.ico") hits += 1;
        return new Response("reached");
      },
    });
    try {
      // Served from where it would be reaching for, or the browser's own rules about a page from
      // nowhere reaching a local address would stop this, and show nothing about the page's policy.
      const address = `http://127.0.0.1:${listening.port}`;
      await load(`${address}/`);
      const ran = await tab.read<boolean>(`(async () => {
        await fetch('${address}/fetch', { mode: 'no-cors' }).catch(() => {});
        await new Promise((done) => {
          const image = new Image();
          image.onload = image.onerror = done;
          image.src = '${address}/image.png';
        });
        const frame = document.createElement('iframe');
        frame.src = '${address}/frame';
        document.body.append(frame);
        const outside = document.createElement('script');
        outside.src = '${address}/script.js';
        document.head.append(outside);
        const inline = document.createElement('script');
        inline.textContent = 'window.ran = true';
        document.head.append(inline);
        await new Promise((done) => setTimeout(done, 300));
        return window.ran === true;
      })()`);
      expect(ran).toBe(false);
      expect(hits).toBe(0);
    } finally {
      void listening.stop(true);
    }
  });

  test("keeps nothing a program tries to put on the clipboard", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    const secret = Buffer.from("from-the-program").toString("base64");
    await print(`\u001b]52;c;${secret}\u0007shown`);
    await onScreen("shown");
    const kinds = (await said()).map((m) => m.t);
    expect(kinds.filter((k) => k !== "ready" && k !== "drawn" && k !== "cell")).toEqual([]);
  });

  test("a message it cannot read draws nothing, and is said to be unreadable", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    await post({ t: "typed", data: "x" });
    await post({ t: "output", id: 1, data: "<img src=x>" });
    await until("said so", said, (s) => s.filter((m) => m.what === "unreadable").length === 2);
    expect((await screenText()).trim()).toBe("");
  });

  /** Tap a word on the screen, as a finger would. */
  async function tap(word: string): Promise<void> {
    const at = await tab.read<{ x: number; y: number } | null>(`(() => {
      for (const row of document.querySelector('.xterm-rows').children) {
        const start = row.textContent.indexOf(${JSON.stringify(word)});
        if (start < 0) continue;
        const box = row.getBoundingClientRect();
        const cell = document.querySelector('.xterm-screen').getBoundingClientRect().width / 40;
        return { x: box.left + (start + 1) * cell, y: box.top + box.height / 2 };
      }
      return null;
    })()`);
    if (!at) throw new Error(`${word} is not on the screen`);
    await tab.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...at });
    await Bun.sleep(150);
    for (const type of ["mousePressed", "mouseReleased"]) {
      await tab.send("Input.dispatchMouseEvent", { type, ...at, button: "left", clickCount: 1 });
    }
  }

  test("an https link is offered when tapped, and only then", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    await print("\u001b]8;;https://example.com/hidden\u0007tapme\u001b]8;;\u0007\r\n");
    await onScreen("tapme");
    await Bun.sleep(100);
    expect((await said()).filter((m) => m.t === "link")).toEqual([]);
    await tap("tapme");
    await until("offered it", said, (s) => s.some((m) => m.t === "link"));
    expect((await said()).filter((m) => m.t === "link")).toEqual([
      { t: "link", url: "https://example.com/hidden" },
    ]);
  });

  test("a link to anything else is not offered", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    await print("\u001b]8;;javascript:alert(1)\u0007script\u001b]8;;\u0007\r\n");
    await print("\u001b]8;;http://example.com\u0007plain\u001b]8;;\u0007\r\n");
    await onScreen("plain");
    await tap("script");
    await tap("plain");
    await Bun.sleep(200);
    expect((await said()).filter((m) => m.t === "link")).toEqual([]);
  });

  test("a finger moved down goes back through what scrolled off, and up again", async () => {
    await load();
    await post({ t: "reset", cols: 40, rows: 10 });
    await print(Array.from({ length: 200 }, (_, i) => `line-${i}`).join("\r\n"));
    await onScreen("line-199");

    const drag = async (from: number, to: number) => {
      const x = 100;
      await tab.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x, y: from }],
      });
      const steps = 12;
      for (let i = 1; i <= steps; i += 1) {
        await tab.send("Input.dispatchTouchEvent", {
          type: "touchMove",
          touchPoints: [{ x, y: from + ((to - from) * i) / steps }],
        });
      }
      await tab.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    };

    await drag(40, 300);
    const back = await until("went back", screenText, (text) => !text.includes("line-199"));
    expect(back).toContain("line-1");
    await drag(300, 40);
    await until("came forward again", screenText, (text) => text.includes("line-199"));
  });
});
