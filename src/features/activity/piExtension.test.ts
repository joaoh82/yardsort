import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
// The extension Yardsort gives pi and OMP, straight from the core crate: `reduce` is what runs
// inside the agent's process before anything leaves it.
import { reduce } from "../../../crates/core/src/activity/pi-extension";

const OMP = join(__dirname, "../../../crates/core/fixtures/omp/18.2.11");

interface Delivery {
  event: string;
  payload: Record<string, unknown>;
  session: { id?: string; cwd?: string; model?: { id?: string; provider?: string } };
}

function fixtures(): Array<{ name: string; delivery: Delivery }> {
  return readdirSync(OMP)
    .filter((name) => name.endsWith(".json"))
    .sort()
    .map((name) => ({
      name,
      delivery: JSON.parse(readFileSync(join(OMP, name), "utf8")) as Delivery,
    }));
}

describe("the pi-family extension's reduce", () => {
  it("keeps the events Yardsort records and drops the rest", () => {
    const kept = fixtures()
      .filter(({ delivery }) => reduce(delivery.event, delivery.payload, delivery.session))
      .map(({ name }) => name.replace(/^\d+-/, "").replace(/\.json$/, ""));
    expect(kept).toEqual([
      "session_start",
      "turn_start",
      "tool_execution_start-write",
      "tool_execution_end-write",
      "turn_end",
      "turn_start",
      "tool_execution_start-read",
      "tool_execution_start-read",
      "tool_execution_start-bash",
      "tool_execution_end-read",
      "tool_execution_end-read",
      "tool_execution_end-bash",
      "turn_end",
      "turn_start",
      "turn_end",
      "session_shutdown",
    ]);
  });

  it("lets no content through and keeps what the Yardsort side reads", () => {
    for (const { name, delivery } of fixtures()) {
      const reduced = reduce(delivery.event, delivery.payload, delivery.session);
      if (!reduced) continue;
      const text = JSON.stringify(reduced);
      for (const content of [
        "Create a file",
        '"content"',
        "Successfully wrote",
        "Wall time",
        '"text"',
        "thinking",
        '"command"',
      ]) {
        expect(text, `${name} leaked ${content}`).not.toContain(content);
      }
    }
    const by = (prefix: string) => fixtures().find(({ name }) => name.startsWith(prefix))!;
    const turn = by("15-turn_end");
    const reduced = reduce(turn.delivery.event, turn.delivery.payload, turn.delivery.session);
    expect(reduced?.payload).toMatchObject({
      turnIndex: 0,
      message: {
        model: "z-ai/glm-5.3-flash",
        provider: "openrouter",
        usage: { totalTokens: 19963 },
      },
    });
    const bash = by("34-tool_execution_end-bash");
    const done = reduce(bash.delivery.event, bash.delivery.payload, bash.delivery.session);
    expect(done?.payload).toMatchObject({
      toolName: "bash",
      isError: false,
      result: { details: { wallTimeMs: expect.any(Number) } },
    });
    expect(reduce("agent_end", { messages: [] }, {})).toBeNull();
  });
});
