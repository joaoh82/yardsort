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
  id,
  MAX_ID_LEN,
  MAX_INPUT_BYTES,
  MAX_TEXT,
  normalizeCode,
  parsePairingLink,
  reply,
  request,
  response,
  scopes,
  type Request,
} from "../src/index.ts";

const ask = (op: string, params?: unknown) =>
  decode(request, { requestId: "r", protocol: { major: 1, minor: 4 }, op, params });

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

  test("a platform or a purpose from the future is other; a scope from the future is refused", () => {
    const host = decode(reply, {
      kind: "host",
      host: "h",
      name: "n",
      platform: "plan9",
      appVersion: "1",
      protocol: { major: 1, minor: 9 },
      capabilities: [],
    });
    expect(host.kind === "host" && host.platform).toBe("other");
    expect(() => decode(scopes, ["workspace:read", "files:write"])).toThrow();
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
      protocol: { major: 1, minor: 4 },
      op: "activity.subscribe",
      params: {},
    } as unknown as Request);
  });

  test("an operation this build does not know is refused", () => {
    expect(() => ask("workspace.delete", {})).toThrow(/not an operation/);
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
