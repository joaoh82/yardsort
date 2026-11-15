// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The bounds a decoder holds what arrives to, which the fixtures, being well-formed, do not try.

import { describe, expect, test } from "bun:test";

import {
  cursor,
  decode,
  DecodeError,
  decodeFrame,
  displayCode,
  encodeFrame,
  event,
  FrameError,
  fromBase64,
  id,
  MAX_BRANCH,
  MAX_BRANCHES,
  MAX_CHOICES,
  MAX_FILE,
  MAX_FILE_CHUNK,
  MAX_ID_LEN,
  MAX_INPUT_BYTES,
  MAX_LABEL,
  MAX_LISTED_BYTES,
  MAX_PATH,
  MAX_PROJECT_NAME,
  MAX_REMOTE,
  MAX_TEXT,
  MAX_WORKSPACE_NAME,
  normalizeCode,
  parsePairingLink,
  reply,
  request,
  response,
  scopes,
  type Request,
} from "../src/index.ts";

const ask = (op: string, params?: unknown) =>
  decode(request, { requestId: "r", protocol: { major: 1, minor: 5 }, op, params });

const session = (title: string) => ({
  kind: "sessions",
  sessions: [
    {
      id: "s",
      workspace: "w",
      harness: "claude",
      model: null,
      effort: null,
      title,
      forkedFrom: null,
      startedAt: 1,
      endedAt: null,
      exitCode: null,
      process: "running",
      terminal: null,
    },
  ],
  terminals: [],
  next: null,
});

describe("ids", () => {
  test("uuids and server ids are ids", () => {
    for (const ok of [
      "0b6c1f0e-8f8e-4a57-9a43-5c1d2a0e9f11",
      "host_01HZX",
      "srv:example.test",
      "a",
    ]) {
      expect(decode(id(), ok) as string).toBe(ok);
    }
    expect(decode(id(), "x".repeat(MAX_ID_LEN))).toHaveLength(MAX_ID_LEN);
  });

  test("paths, whitespace and control characters are not", () => {
    for (const bad of [
      "",
      "/home/someone/project",
      "C:\\Users\\someone",
      "a b",
      "line\nbreak",
      "esc\u001b[2J",
      "ünïcode",
      "x".repeat(MAX_ID_LEN + 1),
      7,
    ]) {
      expect(() => decode(id(), bad)).toThrow(DecodeError);
    }
  });

  test("a refusal says where, never what", () => {
    try {
      decode(request, {
        requestId: "r",
        protocol: { major: 1, minor: 0 },
        op: "workspace.describe",
        params: { workspace: "/home/someone/secret" },
      });
      throw new Error("decoded");
    } catch (error) {
      expect(error).toBeInstanceOf(DecodeError);
      expect((error as Error).message).toContain("params.workspace");
      expect((error as Error).message).not.toContain("secret");
    }
  });

  test("a cursor is bounded and printable", () => {
    expect(decode(cursor, "k1.abc") as string).toBe("k1.abc");
    for (const bad of ["", "a b", "x".repeat(257), "é"])
      expect(() => decode(cursor, bad)).toThrow();
  });
});

describe("text", () => {
  test("is held to its limit in characters, as Rust counts them", () => {
    const at = "\u{1f600}".repeat(MAX_TEXT);
    // 512 UTF-16 units, 256 characters: within the limit.
    expect(decode(reply, session(at)).kind).toBe("sessions");
    expect(() => decode(reply, session(`${at}x`))).toThrow(/longer than 256/);
  });

  test("lists are held to theirs", () => {
    const page = session("t");
    page.terminals = Array.from({ length: 33 }, () => ({
      purpose: "shell",
      program: "zsh",
      harness: null,
      terminal: {
        id: "t",
        activity: "busy",
        idleMs: 0,
        size: { cols: 80, rows: 24 },
        exitCode: null,
      },
    })) as never;
    expect(() => decode(reply, page)).toThrow(/more than 32/);
  });

  test("a number past what JavaScript keeps exactly is refused, not rounded", () => {
    const resized = (seq: number) => ({
      event: "terminal.resized",
      stream: 1,
      position: { epoch: 0, seq },
      size: { cols: 80, rows: 24 },
    });
    expect(decode(event, resized(Number.MAX_SAFE_INTEGER)).event).toBe("terminal.resized");
    expect(() => decode(event, resized(2 ** 53))).toThrow(/out of range/);
    expect(() => decode(event, resized(1.5))).toThrow(/whole number/);
  });
});

describe("enums", () => {
  test("each of who controls a terminal reads", () => {
    for (const controller of [
      { kind: "nobody" },
      { kind: "local" },
      { kind: "device", device: "d", name: "Phone" },
    ]) {
      const read = decode(event, { event: "terminal.control", stream: 1, controller });
      expect(read.event === "terminal.control" && read.controller).toEqual(controller as never);
    }
  });

  test("a platform or a purpose from the future is other; a scope from the future is dropped", () => {
    const host = decode(reply, {
      kind: "host",
      host: "h",
      name: "n",
      platform: "plan9",
      appVersion: "1",
      protocol: { major: 1, minor: 10 },
      capabilities: [],
    });
    expect(host.kind === "host" && host.platform).toBe("other");
    expect(decode(scopes, ["workspace:read", "files:write"])).toEqual(["workspace:read"]);
  });

  test("scopes read as a set, in the order Rust writes one", () => {
    expect(decode(scopes, ["terminal:control", "workspace:read", "terminal:control"])).toEqual([
      "workspace:read",
      "terminal:control",
    ]);
  });
});

describe("requests are checked as a host checks them", () => {
  test("a page of nothing, input too long or numbered from 0, and an empty size are refused", () => {
    expect(() => ask("workspace.list", { limit: 0 })).toThrow(/page of no items/);
    const input = (seq: number, data: string) =>
      ask("terminal.input", { terminal: "t", lease: "l", generation: 1, seq, data });
    expect(() => input(1, "a".repeat(MAX_INPUT_BYTES + 1))).toThrow(/more input/);
    // Counted as UTF-8, as the host counts it.
    expect(() => input(1, "é".repeat(MAX_INPUT_BYTES / 2 + 1))).toThrow(/more input/);
    expect(() => input(0, "a")).toThrow(/numbers input/);
    expect(input(1, "a".repeat(MAX_INPUT_BYTES)).op).toBe("terminal.input");
    expect(() =>
      ask("terminal.resize", {
        terminal: "t",
        lease: "l",
        generation: 1,
        size: { cols: 0, rows: 24 },
      }),
    ).toThrow(/without a row/);
  });

  test("a conversation is followed within its workspace", () => {
    expect(() => ask("activity.subscribe", { session: "s" })).toThrow(/without its workspace/);
    expect(ask("activity.subscribe", {})).toEqual({
      requestId: "r",
      protocol: { major: 1, minor: 5 },
      op: "activity.subscribe",
      params: {},
    } as unknown as Request);
  });

  test("what a start says since minor 10 is each within its bounds", () => {
    const start = (more: Record<string, unknown>) =>
      ask("workspace.start", { project: "p", prompt: "Fix it", ...more });
    expect(start({}).op).toBe("workspace.start");
    expect(start({ name: "fix-login" }).op).toBe("workspace.start");
    expect(start({ name: "n".repeat(MAX_WORKSPACE_NAME) }).op).toBe("workspace.start");
    expect(() => start({ name: "  " })).toThrow(/a name of no characters/);
    expect(() => start({ name: "n".repeat(MAX_WORKSPACE_NAME + 1) })).toThrow(/or over 80/);
    expect(() => start({ name: "two\nlines" })).toThrow(/a name of no characters/);
    expect(start({ model: "opus", effort: "high" }).op).toBe("workspace.start");
    expect(() => start({ model: "--dangerously" })).toThrow(/names no model/);
    expect(() => start({ model: " " })).toThrow(/names no model/);
    expect(() => start({ effort: "e".repeat(MAX_LABEL + 1) })).toThrow(/names no effort/);
    expect(start({ baseBranch: "develop" }).op).toBe("workspace.start");
    expect(() => start({ baseBranch: "--upload-pack=x" })).toThrow(/names no branch/);
    expect(() => start({ baseBranch: "" })).toThrow(/names no branch/);
    expect(() => start({ baseBranch: "a b" })).toThrow(/names no branch/);
    // Counted as UTF-8, as the host counts it.
    expect(start({ baseBranch: "b".repeat(MAX_BRANCH) }).op).toBe("workspace.start");
    expect(() => start({ baseBranch: "b".repeat(MAX_BRANCH + 1) })).toThrow(/names no branch/);
    expect(() => start({ baseBranch: "é".repeat(Math.ceil(MAX_BRANCH / 2)) })).toThrow(
      /names no branch/,
    );
    expect(start({ existingBranch: "ys/kept" }).op).toBe("workspace.start");
    expect(() => start({ existingBranch: "-x" })).toThrow(/names no branch/);
    // An existing branch is opened as it is: it has no base, and it names itself.
    expect(() => start({ existingBranch: "ys/kept", baseBranch: "main" })).toThrow(
      /no name and no base/,
    );
    expect(() => start({ existingBranch: "ys/kept", name: "kept" })).toThrow(/no name and no base/);
  });

  test("a clone names an address and nothing that reads as an option", () => {
    const clone = (remote: string, name?: string) => ask("project.clone", { remote, name });
    expect(clone("https://github.com/example/app.git").op).toBe("project.clone");
    expect(clone("git@github.com:example/app.git", "app").op).toBe("project.clone");
    // What is around it is the host's to trim, as it trims one typed into its own window.
    expect(clone(" https://example.com/a\n").op).toBe("project.clone");
    for (const bad of [
      "",
      "   ",
      "--upload-pack=touch /tmp/x",
      "-u",
      "https://example.com/a b",
      "https://example.com/a\tb",
      "https://example.com/a\u001bb",
      `https://example.com/${"a".repeat(MAX_REMOTE)}`,
    ]) {
      expect(() => clone(bad)).toThrow(/no repository's address/);
    }
    for (const bad of ["", " ", "../up", "a\\b", "n".repeat(MAX_PROJECT_NAME + 1)]) {
      expect(() => clone("https://example.com/a", bad)).toThrow(/a name none can have/);
    }
    expect(clone("https://example.com/a", "n".repeat(MAX_PROJECT_NAME)).op).toBe("project.clone");
  });

  test("a workspace is renamed to something, and removed by force only when it says so", () => {
    const rename = (name: string) => ask("workspace.rename", { workspace: "w", name });
    expect(rename("Login spacing").op).toBe("workspace.rename");
    expect(() => rename(" ")).toThrow(/a name of no characters/);
    expect(() => rename("n".repeat(MAX_WORKSPACE_NAME + 1))).toThrow(/or over 80/);
    for (const op of ["workspace.archive", "workspace.delete"]) {
      // A force of false is left out, as Rust writes it.
      expect(ask(op, { workspace: "w", force: false })).toEqual(ask(op, { workspace: "w" }));
      expect("force" in (ask(op, { workspace: "w" }) as { params: object }).params).toBe(false);
      expect(ask(op, { workspace: "w", force: true })).toHaveProperty("params.force", true);
      expect(() => ask(op, { workspace: "w", force: "yes" })).toThrow(/true or false/);
      expect(() => ask(op, { workspace: "w", force: null })).toThrow(/true or false/);
    }
  });

  test("a file is named by a path inside the workspace, and read from within any file's length", () => {
    const read = (path: string, oldPath?: string, offset?: number) =>
      ask("file.read", { workspace: "w", path, oldPath, of: "working", offset });
    expect(read("src/login.tsx").op).toBe("file.read");
    expect(read(".env.example", "old name.txt", MAX_FILE).op).toBe("file.read");
    // Without one, a read starts at the start, and says so.
    expect(read("a")).toHaveProperty("params.offset", 0);
    for (const outside of [
      "",
      "/etc/passwd",
      "../secrets",
      "src/../../secrets",
      "src//login.tsx",
      "src/",
      "C:\\Users\\me",
      "C:/Users/me",
      "\\\\server\\share",
      "-x",
      "a\u0000b",
      "a\u001b[2J",
    ]) {
      expect(() => read(outside)).toThrow(/no path inside the workspace/);
      expect(() => read("ok", outside)).toThrow(/no path inside the workspace/);
    }
    // Counted as UTF-8, as the host counts it.
    expect(read("p".repeat(MAX_PATH)).op).toBe("file.read");
    expect(() => read("p".repeat(MAX_PATH + 1))).toThrow(/no path inside the workspace/);
    expect(() => read("é".repeat(MAX_PATH / 2 + 1))).toThrow(/no path inside the workspace/);
    // A drive's letter is the second byte, as the host reads it: a name may hold a colon later.
    expect(read("é:x").op).toBe("file.read");
    expect(read("notes: today.md").op).toBe("file.read");
    expect(() => read("a", undefined, MAX_FILE + 1)).toThrow(/past the end of any file/);
    expect(() => read("a", undefined, -1)).toThrow(/out of range/);
    expect(() => ask("file.read", { workspace: "w", path: "a", of: "stagedOld" })).toThrow(
      /not one this build knows/,
    );

    const files = (dir?: string) => ask("workspace.files", { workspace: "w", dir });
    // The root: no folder, or an empty one, which is left out as Rust writes it.
    expect(files()).toEqual(files(""));
    expect("dir" in (files("") as { params: object }).params).toBe(false);
    expect(files("src/auth")).toHaveProperty("params.dir", "src/auth");
    for (const outside of ["../up", "/abs", "C:/x", "-x", "src/", "a\u0007"]) {
      expect(() => files(outside)).toThrow(/no path inside the workspace/);
    }
    expect(ask("workspace.changes", { workspace: "w" }).op).toBe("workspace.changes");
    expect(() => ask("workspace.changes", { workspace: "../w" })).toThrow(/not an id/);
  });

  test("an operation this build does not know is refused", () => {
    expect(() => ask("memory.edit", {})).toThrow(/not an operation/);
  });

  test("branches, choices and a project's address are held to their limits", () => {
    const branches = (more: Record<string, unknown>) =>
      decode(reply, { kind: "branches", branches: ["main"], ...more });
    expect(branches({})).toEqual({ kind: "branches", branches: ["main"] });
    // What Rust leaves out reads as left out, however it came.
    expect(branches({ checkedOut: [], more: false })).toEqual({
      kind: "branches",
      branches: ["main"],
    });
    expect(branches({ default: "main", checkedOut: ["main"], more: true })).toEqual({
      kind: "branches",
      branches: ["main"],
      default: "main",
      checkedOut: ["main"],
      more: true,
    });
    const many = Array.from({ length: MAX_BRANCHES + 1 }, (_, n) => `b${n}`);
    expect(branches({ branches: many.slice(1) }).kind).toBe("branches");
    expect(() => branches({ branches: many })).toThrow(/more than 100/);
    // A name no request could give back is not one.
    for (const bad of ["-x", "a b", "", "b".repeat(MAX_BRANCH + 1)]) {
      expect(() => branches({ branches: [bad] })).toThrow(/not a branch's name/);
      expect(() => branches({ default: bad })).toThrow(/not a branch's name/);
    }

    const harness = (models: string[]) =>
      decode(reply, { kind: "harnesses", harnesses: [{ id: "claude", name: "Claude", models }] });
    const models = Array.from({ length: MAX_CHOICES + 1 }, (_, n) => `m${n}`);
    expect(harness(models.slice(1)).kind).toBe("harnesses");
    expect(() => harness(models)).toThrow(/more than 32/);
    expect(() => harness(["m".repeat(MAX_LABEL + 1)])).toThrow(/longer than 64/);

    const project = (remote: string) =>
      decode(reply, {
        kind: "workspaces",
        projects: [{ id: "p", name: "n", missing: false, remote }],
        workspaces: [],
        next: null,
      });
    expect(project("r".repeat(MAX_REMOTE)).kind).toBe("workspaces");
    expect(() => project("r".repeat(MAX_REMOTE + 1))).toThrow(/not an address/);
    expect(() => project("https://example.com/a\u001b[2J")).toThrow(/not an address/);
  });

  test("changes, a folder's entries and a piece of a file are held to their limits", () => {
    const changes = (more: Record<string, unknown>) =>
      decode(reply, { kind: "changes", uncommitted: [], committed: [], ...more });
    expect(changes({})).toEqual({ kind: "changes", uncommitted: [], committed: [] });
    // What Rust leaves out reads as left out, however it came.
    expect(changes({ base: null, more: false })).toEqual(changes({}));
    const file = (path: string, more: Record<string, unknown> = {}) => ({
      path,
      kind: "modified",
      ...more,
    });
    expect(changes({ committed: [file("a b/c.txt", { oldPath: "a.txt" })], more: true })).toEqual({
      kind: "changes",
      uncommitted: [],
      committed: [{ path: "a b/c.txt", oldPath: "a.txt", kind: "modified" }],
      more: true,
    });
    // A path no request could give back is not one.
    for (const bad of ["", "../outside", "/etc/passwd", "C:/x", "-x", "a\u001b[2J"]) {
      expect(() => changes({ uncommitted: [file(bad)] })).toThrow(/not a path inside/);
    }
    for (const bad of ["../outside", "/etc/passwd", "p".repeat(MAX_PATH + 1)]) {
      expect(() => changes({ committed: [file("a", { oldPath: bad })] })).toThrow(
        /not a path inside/,
      );
    }
    expect(() => changes({ uncommitted: [file("a", { additions: -1 })] })).toThrow(/out of range/);
    expect(() => changes({ base: "b".repeat(MAX_BRANCH + 1) })).toThrow(/not a branch's name/);
    // A list is no longer than a computer cuts one.
    const long = (n: number) => file(`${"d".repeat(MAX_PATH - 8)}/${n}`);
    const fits = Math.floor(MAX_LISTED_BYTES / MAX_PATH);
    expect(changes({ uncommitted: Array.from({ length: fits }, (_, n) => long(n)) }).kind).toBe(
      "changes",
    );
    expect(() =>
      changes({ uncommitted: Array.from({ length: fits + 2 }, (_, n) => long(n)) }),
    ).toThrow(/more than a reply carries/);

    const folder = (entries: unknown[], more?: unknown) =>
      decode(reply, { kind: "files", entries, more });
    expect(
      folder(
        [
          { name: "src", dir: true },
          { name: "a.txt", dir: false },
        ],
        false,
      ),
    ).toEqual({
      kind: "files",
      entries: [{ name: "src", dir: true }, { name: "a.txt" }],
    });
    expect(folder([{ name: "n".repeat(255) }], true)).toHaveProperty("more", true);
    for (const bad of ["", "n".repeat(256), "a\u001b[2J", "two\nlines"]) {
      expect(() => folder([{ name: bad }])).toThrow(/not a file's name/);
    }
    const names = (count: number) =>
      Array.from({ length: count }, (_, n) => ({ name: `${"f".repeat(200)}${n}` }));
    expect(folder(names(190)).kind).toBe("files");
    expect(() => folder(names(210))).toThrow(/more than a reply carries/);

    const piece = (more: Record<string, unknown>) =>
      decode(reply, { kind: "file", state: "text", size: 3, offset: 0, done: true, ...more });
    // A piece of no bytes says nothing of them, as Rust writes it.
    expect(piece({ data: "" })).toEqual(piece({}));
    expect("data" in piece({ data: "" })).toBe(false);
    const most = "A".repeat(Math.ceil(MAX_FILE_CHUNK / 3) * 4);
    expect(piece({ data: most })).toHaveProperty("data", most);
    expect(() => piece({ data: `${most}AAAA` })).toThrow(/longer than/);
    expect(() => piece({ size: 2 ** 53 })).toThrow(/out of range/);
    expect(() => piece({ done: undefined })).toThrow(/true or false/);
  });

  test("a response with neither a result nor an error is refused", () => {
    expect(() => decode(response, { requestId: "r" })).toThrow();
    expect(
      decode(response, {
        requestId: null,
        error: { code: "invalid_request", message: "m", retryable: false },
      }),
    ).toEqual({
      requestId: null,
      error: { code: "invalid_request", message: "m", retryable: false },
    });
  });
});

describe("output frames", () => {
  test("round-trip raw bytes, escape sequences and invalid UTF-8 alike", () => {
    const bytes = Uint8Array.from([0x1b, 0x5b, 0x32, 0x4a, 0xff, 0xfe, 0x6f, 0x6b]);
    const frame = { stream: 7, position: { epoch: 3, seq: Number.MAX_SAFE_INTEGER - 1 }, bytes };
    expect(decodeFrame(encodeFrame(frame))).toEqual(frame);
  });

  test("a sequence number past what JavaScript keeps exactly is refused", () => {
    const wire = encodeFrame({
      stream: 1,
      position: { epoch: 0, seq: 0 },
      bytes: new Uint8Array(),
    });
    wire[19] = 0x01;
    expect(() => decodeFrame(wire)).toThrow(new FrameError("seqTooLarge"));
  });

  test("a frame read from a larger buffer reads only itself", () => {
    const wire = encodeFrame({
      stream: 2,
      position: { epoch: 1, seq: 9 },
      bytes: Uint8Array.from([1, 2]),
    });
    const padded = new Uint8Array(wire.length + 8);
    padded.set(wire, 4);
    expect(decodeFrame(padded.subarray(4, 4 + wire.length)).position).toEqual({ epoch: 1, seq: 9 });
  });
});

describe("base64", () => {
  test("reads back what was written, whatever its length", () => {
    for (let length = 0; length < 70; length++) {
      const bytes = Uint8Array.from({ length }, (_, n) => (n * 151 + length) % 256);
      expect(fromBase64(Buffer.from(bytes).toString("base64"))).toEqual(bytes);
    }
    expect(fromBase64("")).toEqual(new Uint8Array());
  });

  test("what is not padded standard base64 is refused, as the host's decoder refuses it", () => {
    for (const bad of [
      "QQ",
      "QUI",
      "QQ=",
      "Q===",
      "====",
      "QQ==QQ==",
      "Q=Q=",
      "QU-_",
      "QUJ\n",
      " QUJD",
      "QUJ\u00e9",
      // Bits left over after the last byte.
      "QR==",
      "QUJ=",
    ]) {
      expect(fromBase64(bad)).toBeNull();
    }
  });
});

describe("pairing codes and links", () => {
  test("a code reads back however it is typed", () => {
    expect(normalizeCode("abcd efgh")).toBe("ABCDEFGH");
    expect(normalizeCode("o1il-0000")).toBe("01110000");
    expect(displayCode("abcdefgh")).toBe("ABCD-EFGH");
    for (const bad of ["", "ABCD-EFG", "ABCD-EFGHJ", "ABCD-EFGU", "ABCD/EFGH", "ÄBCD-EFGH"]) {
      expect(normalizeCode(bad)).toBeNull();
    }
  });

  test("a link names its server and carries a secret", () => {
    const link = parsePairingLink(
      "yardsort://pair?server=https%3A%2F%2Fremote.example.com%2F&secret=ys1.po.abc",
    );
    expect(link?.server).toBe("https://remote.example.com/");
    expect(link?.secret.expose()).toBe("ys1.po.abc");
    for (const bad of [
      "yardsort://pair?server=x",
      "https://example.com/pair?server=x&secret=y",
      "yardsort://pair",
    ]) {
      expect(parsePairingLink(bad)).toBeNull();
    }
  });
});
