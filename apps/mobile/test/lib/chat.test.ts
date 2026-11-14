// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Chat's own reading (43): the Markdown agents write, the steps and state their activity says, the
 * turns a conversation is read in, and which view each conversation was last seen in.
 */

import { describe, expect, test } from "bun:test";

import type {
  ActivityEvent,
  Channel,
  ConversationPage,
  ConversationReadParams,
  ConversationTurn,
  SessionRecordId,
  WorkspaceId,
} from "@yardsort/remote-client";

import { ConversationFeed, joined, span, stateOf, stepsOf, stepSummary } from "../../src/lib/chat";
import { inline, readMarkdown } from "../../src/lib/markdown";
import { MAX_REMEMBERED, ViewChoices } from "../../src/lib/views";

describe("Markdown", () => {
  test("reads what agents write into blocks, and the rest as written", () => {
    const blocks = readMarkdown(
      [
        "## What changed",
        "",
        "I added the `--imperial` flag, **metric** stays the *default*.",
        "It wraps here.",
        "",
        "- `src/cli.js` parses it",
        "- `src/render.js` converts",
        "  it, on a second line",
        "",
        "1. first",
        "2. second",
        "",
        "```sh",
        "$ weather --imperial Lisbon",
        "Lisbon: 71°F",
        "```",
        "",
        "> quoted",
        "",
        "---",
        "| File | Lines |",
        "| --- | ---: |",
        "| a.rs | 12 |",
        "| b.rs | 3 |",
        "",
        "See [the docs](https://example.com/docs).",
      ].join("\n"),
    );
    expect(blocks.map((b) => b.kind)).toEqual([
      "heading",
      "paragraph",
      "list",
      "list",
      "code",
      "quote",
      "rule",
      "table",
      "paragraph",
    ]);
    const paragraph = blocks[1];
    if (paragraph?.kind !== "paragraph") throw new Error("not a paragraph");
    expect(paragraph.spans).toEqual([
      { text: "I added the " },
      { text: "--imperial", code: true },
      { text: " flag, " },
      { text: "metric", bold: true },
      { text: " stays the " },
      { text: "default", italic: true },
      { text: ". It wraps here." },
    ]);
    const bullets = blocks[2];
    if (bullets?.kind !== "list") throw new Error("not a list");
    expect(bullets.items.length).toBe(2);
    expect(bullets.items[1]?.map((s) => s.text).join("")).toBe(
      "src/render.js converts it, on a second line",
    );
    const numbered = blocks[3];
    expect(numbered?.kind === "list" && numbered.ordered && numbered.start).toBe(1);
    const code = blocks[4];
    expect(code).toEqual({
      kind: "code",
      language: "sh",
      text: "$ weather --imperial Lisbon\nLisbon: 71°F",
    });
    const table = blocks[7];
    if (table?.kind !== "table") throw new Error("not a table");
    expect(table.header.map((cell) => cell[0]?.text)).toEqual(["File", "Lines"]);
    expect(table.rows.map((row) => row.map((cell) => cell[0]?.text))).toEqual([
      ["a.rs", "12"],
      ["b.rs", "3"],
    ]);
    const link = blocks[8];
    expect(link?.kind === "paragraph" && link.spans[1]).toEqual({
      text: "the docs",
      href: "https://example.com/docs",
    });
  });

  test("an unclosed fence keeps the rest as code, and snake_case is not italics", () => {
    expect(readMarkdown("```\nlet a = 1;\nstill code")).toEqual([
      { kind: "code", language: "", text: "let a = 1;\nstill code" },
    ]);
    expect(inline("a_b_c and 2 * 3 * 4")).toEqual([{ text: "a_b_c and 2 * 3 * 4" }]);
    expect(readMarkdown("")).toEqual([]);
  });
});

const T0 = 1_794_650_000_000;

function turn(n: number, at: number, said: string[] = []): ConversationTurn {
  return { n, at, user: `message ${n}`, said, cut: false };
}

let nextEvent = 0;
function event(
  kind: string,
  at: number,
  detail: ActivityEvent["detail"] = {},
  session = "s-1",
): ActivityEvent {
  nextEvent += 1;
  return {
    id: `e-${nextEvent}`,
    workspace: "w-1" as WorkspaceId,
    session: session as SessionRecordId,
    run: null,
    occurredAt: at,
    kind,
    source: { producer: "claude", method: "hook", fidelity: "reported" },
    detail,
  } as unknown as ActivityEvent;
}

describe("steps and state", () => {
  const turns = [turn(1, T0), turn(2, T0 + 60_000)];
  const events = [
    event("prompt.submitted", T0),
    event("tool.started", T0 + 1_000, { tool: "Read", path: "src/cli.js" }),
    event("tool.completed", T0 + 1_500, { tool: "Read", path: "src/cli.js", durationMs: 500 }),
    event("tool.started", T0 + 2_000, { tool: "Edit", path: "src/render.js" }),
    event("tool.completed", T0 + 3_000, { tool: "Edit", durationMs: 1_000 }),
    event("turn.completed", T0 + 4_000),
    event("prompt.submitted", T0 + 60_000),
    event("tool.started", T0 + 61_000, { tool: "Bash" }),
    event("tool.failed", T0 + 62_000, { tool: "Bash" }),
    event("tool.started", T0 + 63_000, { tool: "Edit", path: "src/cli.js" }),
  ];

  test("each turn has the tools its agent ran after its message, the one running last", () => {
    const steps = stepsOf(events, turns);
    expect(steps.get(1)?.map((s) => [s.tool, s.path, s.state])).toEqual([
      ["Read", "src/cli.js", "done"],
      ["Edit", "src/render.js", "done"],
    ]);
    expect(steps.get(2)?.map((s) => [s.tool, s.state])).toEqual([
      ["Bash", "failed"],
      ["Edit", "running"],
    ]);
    expect(stepSummary(steps.get(1) ?? [])).toBe("read 1 · edit 1");
  });

  test("the state is the newest that says one, and a question asked comes first", () => {
    expect(stateOf(events, [])).toEqual({ kind: "working", since: T0 + 60_000 });
    expect(stateOf(events.slice(0, 6), [])).toEqual({ kind: "done", at: T0 + 4_000 });
    expect(stateOf([...events, event("approval.requested", T0 + 64_000)], [])).toEqual({
      kind: "needsYou",
      since: T0 + 64_000,
    });
    expect(
      stateOf(events, [{ id: "a", askedAt: T0 + 70_000, tool: "Bash", command: "npm test" }]),
    ).toEqual({ kind: "needsYou", since: T0 + 70_000 });
    expect(stateOf([event("turn.failed", T0)], [])).toEqual({ kind: "failed", at: T0 });
    expect(stateOf([], [])).toEqual({ kind: "unknown" });
  });

  test("a span of time is said briefly", () => {
    expect(span(9_400)).toBe("9s");
    expect(span(72_000)).toBe("1m 12s");
    expect(span(7_500_000)).toBe("2h 5m");
  });
});

/** A channel that answers `conversation.read` from a list of turns, at most `fit` a page. */
function reading(all: ConversationTurn[], fit: number) {
  const asked: (number | undefined)[] = [];
  const channel = {
    conversation: async (params: ConversationReadParams): Promise<ConversationPage> => {
      asked.push(params.from);
      const page =
        params.from === undefined
          ? all.slice(-fit)
          : all.filter((t) => t.n >= (params.from as number)).slice(0, fit);
      return {
        session: params.session,
        total: all.length,
        turns: page,
        asks: [],
        answersAsks: true,
      };
    },
  } as unknown as Channel;
  return { channel, asked };
}

describe("reading a conversation", () => {
  const feed = () => new ConversationFeed("w-1" as WorkspaceId, "s-1" as SessionRecordId);
  const all = Array.from({ length: 30 }, (_, i) => turn(i + 1, T0 + i * 1_000, [`reply ${i + 1}`]));

  test("the newest first, then from the last one held, which may have grown", async () => {
    const chat = feed();
    const { channel, asked } = reading(all, 5);
    await chat.read(channel);
    expect(chat.getSnapshot().turns.map((t) => t.n)).toEqual([26, 27, 28, 29, 30]);
    expect(chat.getSnapshot().hasEarlier).toBe(true);
    all[29] = turn(30, all[29]?.at ?? 0, ["reply 30", "and more"]);
    all.push(turn(31, T0 + 31_000));
    await chat.read(channel);
    expect(asked).toEqual([undefined, 30]);
    const turns = chat.getSnapshot().turns;
    expect(turns.map((t) => t.n)).toEqual([26, 27, 28, 29, 30, 31]);
    expect(turns.at(-2)?.said).toEqual(["reply 30", "and more"]);
  });

  test("earlier turns are read until they meet the ones held, however few a page holds", async () => {
    const chat = feed();
    const { channel } = reading(all, 3);
    await chat.read(channel);
    const first = chat.getSnapshot().turns[0]?.n as number;
    await chat.earlier(channel);
    const turns = chat.getSnapshot().turns.map((t) => t.n);
    expect(turns[0]).toBe(first - 10);
    expect(turns).toEqual(Array.from({ length: turns.length }, (_, i) => first - 10 + i));
  });

  test("a conversation that went back is read again from its newest", async () => {
    const chat = feed();
    const short = all.slice(0, 31);
    const { channel } = reading(short, 50);
    await chat.read(channel);
    short.length = 2;
    await chat.read(channel);
    expect(chat.getSnapshot().turns.map((t) => t.n)).toEqual([1, 2]);
  });

  test("turns laid over held ones replace them by number", () => {
    expect(
      joined([turn(1, 0), turn(2, 0, ["old"])], [turn(2, 0, ["new"]), turn(3, 0)]).map((t) => [
        t.n,
        t.said,
      ]),
    ).toEqual([
      [1, []],
      [2, ["new"]],
      [3, []],
    ]);
  });
});

describe("Chat or Terminal", () => {
  test("Chat unless the person chose otherwise, remembered across a restart", () => {
    let kept: string | null = null;
    const file = { read: () => kept, write: (text: string) => void (kept = text) };
    const views = new ViewChoices(file);
    expect(views.get("s-1")).toBe("chat");
    views.choose("s-1", "terminal");
    expect(new ViewChoices(file).get("s-1")).toBe("terminal");
    expect(new ViewChoices(file).get("s-2")).toBe("chat");
  });

  test("only the newest choices are kept, and a damaged file is no choices", () => {
    let kept: string | null = null;
    const file = { read: () => kept, write: (text: string) => void (kept = text) };
    const views = new ViewChoices(file);
    for (let i = 0; i <= MAX_REMEMBERED; i++) views.choose(`s-${i}`, "terminal");
    const again = new ViewChoices(file);
    expect(again.get("s-0")).toBe("chat");
    expect(again.get(`s-${MAX_REMEMBERED}`)).toBe("terminal");
    kept = "{not json";
    expect(new ViewChoices(file).get("s-5")).toBe("chat");
  });
});
