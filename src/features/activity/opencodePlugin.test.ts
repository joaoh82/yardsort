import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
// The plugin Yardsort gives OpenCode, straight from the core crate: `reduce` (OpenCode 1) and
// `reduceV2` (OpenCode 2) are what run inside OpenCode's process before anything leaves it.
import plugin from "../../../crates/core/src/activity/opencode-plugin.js";

const { reduce, reduceV2, inSharedService, start } = plugin.testing;

const FIXTURES = join(__dirname, "../../../crates/core/fixtures/opencode/1.18.31");

interface Delivery {
  api?: number;
  hook: string;
  payload: Record<string, unknown>;
  directory?: string;
}

// A pull request opened twice, with stand-ins: `gh pr create`, and a GitHub MCP server's tool.
const PULL_REQUESTS = join(
  __dirname,
  "../../../crates/core/fixtures/opencode-pull-requests/1.18.34",
);

function fixtures(dir = FIXTURES): Array<{ name: string; delivery: Delivery }> {
  return readdirSync(dir)
    .filter((name) => name.endsWith(".json"))
    .sort()
    .map((name) => ({
      name,
      delivery: JSON.parse(readFileSync(join(dir, name), "utf8")) as Delivery,
    }));
}

describe("the OpenCode plugin's reduce", () => {
  it("keeps the calls Yardsort records and drops the rest", () => {
    const kept = fixtures()
      .map(({ name, delivery }) => ({
        name,
        reduced: reduce(delivery.hook, delivery.payload, delivery.directory),
      }))
      .filter(({ reduced }) => reduced !== null)
      .map(({ name }) => name.replace(/^\d+-/, ""));
    expect(kept).toEqual([
      "event-session.created.json",
      "chat.message.json",
      "tool.execute.before-write.json",
      "event-file.edited.json",
      "tool.execute.after-write.json",
      "event-message.updated-assistant-completed.json",
      "tool.execute.before-read.json",
      "tool.execute.after-read.json",
      "event-message.updated-assistant-completed.json",
      "tool.execute.before-read.json",
      "event-message.part.updated-tool-read-error.json",
      "event-message.updated-assistant-completed.json",
      "tool.execute.before-bash.json",
      "tool.execute.after-bash.json",
      "event-message.updated-assistant-completed.json",
      "event-message.updated-assistant-completed.json",
      "event-session.idle.json",
    ]);
  });

  it("lets no content through: not the prompt, a command, an output, a file or an error", () => {
    for (const { name, delivery } of fixtures()) {
      const reduced = reduce(delivery.hook, delivery.payload, delivery.directory);
      if (!reduced) continue;
      const text = JSON.stringify(reduced);
      for (const content of [
        "Create a file",
        "hello\\n",
        "Wrote file",
        "File not found",
        '"ls"',
        // The hook's `output` object stays; a tool's `output` text does not.
        '"output":"',
        '"text"',
        '"content"',
        '"error":"',
        '"title"',
      ]) {
        expect(text, `${name} leaked ${content}`).not.toContain(content);
      }
      expect(reduced.directory).toBe("/tmp/yardsort-fixture/repo");
    }
  });

  it("keeps what the Yardsort side reads", () => {
    const by = (prefix: string) => fixtures().find(({ name }) => name.startsWith(prefix))!;
    const bash = by("63-tool.execute.after-bash");
    expect(reduce(bash.delivery.hook, bash.delivery.payload, bash.delivery.directory)).toEqual({
      hook: "tool.execute.after",
      directory: "/tmp/yardsort-fixture/repo",
      payload: {
        input: {
          tool: "bash",
          sessionID: "ses_00000000000fixture01",
          callID: expect.any(String),
          args: { workdir: "/tmp/yardsort-fixture/repo" },
        },
        output: { metadata: { exit: 0, truncated: false } },
      },
    });
    const prompt = by("04-chat.message");
    const reduced = reduce(prompt.delivery.hook, prompt.delivery.payload) as unknown as {
      payload: { output: { chars: number } };
    };
    expect(reduced.payload.output.chars).toBeGreaterThan(200);
    const failed = by("47-event-message.part.updated-tool-read-error");
    const state = (
      reduce(failed.delivery.hook, failed.delivery.payload) as unknown as {
        payload: { properties: { part: { state: Record<string, unknown> } } };
      }
    ).payload.properties.part.state;
    expect(state.status).toBe("error");
    expect(state.error).toBeUndefined();
    expect(reduce("shell.env", {})).toBeNull();
    expect(reduce("event", { type: "session.status" })).toBeNull();
  });

  it("hands on a call that may have opened a pull request with its command and output alone", () => {
    const after = fixtures(PULL_REQUESTS).filter(
      ({ delivery }) => delivery.hook === "tool.execute.after",
    );
    const reduced = after.map(({ delivery }) =>
      reduce(delivery.hook, delivery.payload, delivery.directory),
    );
    expect(reduced.map((r) => r?.payload)).toEqual([
      {
        input: {
          tool: "bash",
          sessionID: "ses_00000000000fixture01",
          callID: "chatcmpl-tool-b24da437e37ab659",
          args: { command: "gh pr create --title Bump --body Bump" },
        },
        output: {
          metadata: { exit: 0, truncated: false },
          output:
            "\nCreating pull request for bump into main in example/lib\n\nhttps://github.com/example/lib/pull/41\n",
        },
      },
      {
        input: {
          tool: "github_create_pull_request",
          sessionID: "ses_00000000000fixture01",
          callID: "chatcmpl-tool-87a6c24acc0ebde0",
          args: {},
        },
        output: {
          metadata: {},
          content: [
            {
              type: "text",
              text: '{"id": "2989102733", "url": "https://github.com/example/lib/pull/42"}',
            },
          ],
        },
      },
    ]);
  });

  it("keeps every other command's output in OpenCode's process", () => {
    const bash = (command: string) =>
      reduce(
        "tool.execute.after",
        {
          input: { tool: "bash", args: { command } },
          output: { output: "secret output", metadata: { exit: 0 } },
        },
        undefined,
      );
    for (const command of ["ls", "gh pr view 7", "gh issue create", "echo create"]) {
      expect(JSON.stringify(bash(command)), command).not.toContain("secret output");
    }
    const tool = reduce(
      "tool.execute.after",
      { input: { tool: "gitlab_create_pull_request" }, output: { content: "secret output" } },
      undefined,
    );
    expect(JSON.stringify(tool)).not.toContain("secret output");
  });
});

const FIXTURES_2 = join(__dirname, "../../../crates/core/fixtures/opencode/2.2.6");
const PULL_REQUESTS_2 = join(
  __dirname,
  "../../../crates/core/fixtures/opencode-pull-requests/2.2.6",
);

/** Every recorded OpenCode 2 call through `reduceV2`, in order, with the plugin's memory. */
function reducedV2(dir = FIXTURES_2) {
  const seen = { started: new Map<string, number>(), models: new Map<string, unknown>() };
  return fixtures(dir).map(({ name, delivery }) => ({
    name,
    reduced: reduceV2(delivery.hook, delivery.payload, delivery.directory, seen),
  }));
}

describe("the OpenCode plugin's reduceV2, for OpenCode 2", () => {
  it("keeps the calls Yardsort records and drops the rest", () => {
    const kept = reducedV2()
      .filter(({ reduced }) => reduced !== null)
      .map(({ name }) => name.replace(/^\d+-/, ""));
    expect(kept).toEqual([
      "session.prompt.json",
      "tool.execute.before-write.json",
      "tool.execute.after-write-completed.json",
      "event-session.step.ended.json",
      "tool.execute.before-read.json",
      "tool.execute.before-read.json",
      "tool.execute.before-shell.json",
      "event-permission.asked.json",
      "tool.execute.after-read-error.json",
      "event-permission.replied.json",
      "tool.execute.after-read-completed.json",
      "tool.execute.after-shell-completed.json",
      "event-session.step.ended.json",
      "event-session.step.ended.json",
      "event-session.execution.succeeded.json",
    ]);
    for (const { reduced } of reducedV2()) {
      if (reduced) expect(reduced.api).toBe(2);
    }
  });

  it("lets no content through: not the prompt, a command, an output, a file or an error", () => {
    for (const { name, reduced } of reducedV2()) {
      if (!reduced) continue;
      const text = JSON.stringify(reduced);
      for (const content of [
        "Create a file",
        '"hello"',
        "Created file",
        "Read file",
        "File not found",
        '"ls"',
        '"resources"',
        '"content"',
        '"output":"',
        // The status may say `error`; the error itself stays.
        '"error":',
        '"text"',
      ]) {
        expect(text, `${name} leaked ${content}`).not.toContain(content);
      }
      expect(reduced.directory).toBe("/tmp/yardsort-fixture/repo");
    }
  });

  it("keeps what the Yardsort side reads, and adds a failed call's duration and a step's model", () => {
    const by = (prefix: string) =>
      reducedV2().find(({ name }) => name.startsWith(prefix))!.reduced!;
    expect(by("35-tool.execute.after-shell").payload).toEqual({
      tool: "shell",
      sessionID: "ses_00000000000fixture01",
      messageID: expect.any(String),
      id: expect.any(String),
      status: "completed",
      input: { workdir: "/tmp/yardsort-fixture/repo" },
      result: { output: { exit: 0, truncated: false }, metadata: { exit: 0, truncated: false } },
    });
    expect(by("12-tool.execute.after-write").payload.result).toEqual({
      output: { existed: false, operation: "write" },
      metadata: {},
    });
    const failed = by("27-tool.execute.after-read-error").payload;
    expect(failed.status).toBe("error");
    expect(typeof failed.durationMs).toBe("number");
    expect(failed.error).toBeUndefined();
    const step = by("14-event-session.step.ended").payload as { data: Record<string, unknown> };
    expect(step.data.model).toEqual({ id: expect.any(String), providerID: "opencode" });
    expect(step.data.tokens).toBeDefined();
    expect(step.data.files).toBeUndefined();
    const asked = by("26-event-permission.asked").payload as { data: Record<string, unknown> };
    expect(asked.data).toEqual({
      id: "per_00000000000fixture01",
      sessionID: "ses_00000000000fixture01",
      action: "shell",
      source: { type: "tool", messageID: expect.any(String), id: expect.any(String) },
    });
    const prompt = by("02-session.prompt").payload as { chars: number };
    expect(prompt.chars).toBeGreaterThan(200);
    expect(reduceV2("event", { type: "session.text.delta", data: { text: "x" } })).toBeNull();
    expect(reduceV2("permission.evaluate", {})).toBeNull();
  });

  it("times a failed call from its own start, though an MCP call inside `execute` shares its id", () => {
    const seen = { started: new Map<string, number>(), models: new Map<string, unknown>() };
    const call = (hook: string, tool: string, status?: string) =>
      reduceV2(hook, { tool, id: "call_1", status, input: {} }, undefined, seen);
    call("tool.execute.before", "execute");
    call("tool.execute.before", "github_create_pull_request");
    const inner = call("tool.execute.after", "github_create_pull_request", "error");
    const outer = call("tool.execute.after", "execute", "error");
    expect(typeof inner!.payload.durationMs).toBe("number");
    expect(typeof outer!.payload.durationMs).toBe("number");
    expect(seen.started.size).toBe(0);
  });

  it("hands on a call that may have opened a pull request with its command and output alone", () => {
    const after = reducedV2(PULL_REQUESTS_2).filter(
      ({ name, reduced }) => reduced && name.includes("tool.execute.after"),
    );
    expect(after.map(({ reduced }) => reduced!.payload)).toEqual([
      {
        tool: "shell",
        sessionID: "ses_00000000000fixture01",
        messageID: expect.any(String),
        id: expect.any(String),
        status: "completed",
        input: { command: "gh pr create --title Bump --body Bump" },
        result: {
          output: {
            exit: 0,
            truncated: false,
            output:
              "\nCreating pull request for bump into main in example/lib\n\nhttps://github.com/example/lib/pull/41\n",
          },
          metadata: { exit: 0, truncated: false },
        },
      },
      {
        tool: "github_create_pull_request",
        sessionID: "ses_00000000000fixture01",
        messageID: expect.any(String),
        id: expect.any(String),
        status: "completed",
        input: {},
        result: {
          output: {},
          metadata: {},
          content: [
            {
              type: "text",
              text: '{"id": "2989102733", "url": "https://github.com/example/lib/pull/42"}',
            },
          ],
        },
      },
      // The `execute` tool the MCP call ran inside: neither its code nor its answer.
      {
        tool: "execute",
        sessionID: "ses_00000000000fixture01",
        messageID: expect.any(String),
        id: expect.any(String),
        status: "completed",
        input: {},
        result: { output: {}, metadata: {} },
      },
    ]);
    const shell = (command: string) =>
      reduceV2("tool.execute.after", {
        tool: "shell",
        status: "completed",
        input: { command },
        result: { output: { output: "secret output", exit: 0 }, content: [{ text: "secret" }] },
      });
    for (const command of ["ls", "gh pr view 7", "gh issue create", "echo create"]) {
      expect(JSON.stringify(shell(command)), command).not.toContain("secret");
    }
    // A shell `gh pr create` hands on what it printed, not the content blocks beside it.
    expect(JSON.stringify(shell("gh pr create"))).not.toContain('"content"');
  });

  it("reports nothing from OpenCode 2's shared background service", () => {
    expect(inSharedService(["bun", "/$bunfs/root/opencode", "serve", "--service"])).toBe(true);
    expect(
      inSharedService(["bun", "/$bunfs/root/opencode", "serve", "--stdio", "--port", "0"]),
    ).toBe(false);
    const hooks: string[] = [];
    let subscribed = 0;
    const ctx = {
      location: { directory: "/w" },
      event: {
        subscribe: () => {
          subscribed += 1;
          return (async function* () {})();
        },
      },
      tool: { hook: async (name: string) => void hooks.push(`tool.${name}`) },
      session: { hook: async (name: string) => void hooks.push(`session.${name}`) },
    };
    expect(start(ctx, ["serve", "--service"])).toBeUndefined();
    expect(hooks).toEqual([]);
    expect(subscribed).toBe(0);
  });

  it("registers its hooks and its subscription in a launch's own server, and cleans up", async () => {
    const hooks: string[] = [];
    let signal: AbortSignal | undefined;
    let types: string[] = [];
    const ctx = {
      location: { directory: "/w" },
      event: {
        subscribe: (options: { types: string[]; signal: AbortSignal }) => {
          signal = options.signal;
          types = options.types;
          return (async function* () {})();
        },
      },
      tool: { hook: async (name: string) => void hooks.push(`tool.${name}`) },
      session: { hook: async (name: string) => void hooks.push(`session.${name}`) },
    };
    const cleanup = await start(ctx, ["serve", "--stdio"]);
    expect(hooks).toEqual(["tool.execute.before", "tool.execute.after", "session.prompt"]);
    expect(types).toContain("session.execution.succeeded");
    expect(types).not.toContain("session.text.delta");
    expect(signal?.aborted).toBe(false);
    cleanup!();
    expect(signal?.aborted).toBe(true);

    // A context missing a domain, or a hook that refuses, stops nothing else.
    const partial = {
      location: {},
      event: {
        subscribe: () => {
          throw new Error("no events");
        },
      },
      tool: {
        hook: async () => {
          throw new Error("refused");
        },
      },
    };
    await expect(start(partial, ["serve"])).resolves.toBeTypeOf("function");
  });
});
