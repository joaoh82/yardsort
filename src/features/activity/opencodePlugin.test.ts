import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
// The plugin Yardsort gives OpenCode, straight from the core crate: `reduce` is what runs
// inside OpenCode's process before anything leaves it.
import { YardsortActivity } from "../../../crates/core/src/activity/opencode-plugin.js";

const { reduce } = YardsortActivity;

const FIXTURES = join(__dirname, "../../../crates/core/fixtures/opencode/1.18.31");

interface Delivery {
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
