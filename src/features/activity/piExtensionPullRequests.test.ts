import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

// What the extension hands the Yardsort executable, read off a stand-in `spawn`.
interface Delivery {
  event: string;
  payload: { args?: unknown; result: { content?: unknown } };
}
const delivered: Delivery[] = [];
vi.mock("node:child_process", () => {
  const spawn = () => ({
    on: () => {},
    stdin: {
      on: () => {},
      end: (text: string) => delivered.push(JSON.parse(text)),
    },
  });
  return { spawn, default: { spawn } };
});

const { default: extension } = await import("../../../crates/core/src/activity/pi-extension");

const FIXTURES = join(__dirname, "../../../crates/core/fixtures");

/** Every recorded event of one run, played through the extension's own handlers. */
function play(dir: string) {
  const handlers = new Map<string, (event: unknown, ctx: unknown) => void>();
  extension({
    on: (name: string, handler: (event: unknown, ctx: unknown) => void) =>
      handlers.set(name, handler),
  });
  for (const name of readdirSync(join(FIXTURES, dir))
    .filter((n) => n.endsWith(".json"))
    .sort()) {
    const { event, payload } = JSON.parse(readFileSync(join(FIXTURES, dir, name), "utf8"));
    handlers.get(event)?.(payload, {});
  }
  return delivered.filter((entry) => entry.event === "tool_execution_end");
}

describe("the pi-family extension and a pull request", () => {
  beforeEach(() => {
    delivered.length = 0;
  });

  it.each(["pi-pull-requests/1.5.0", "omp-pull-requests/18.18.3"])(
    "hands on what gh pr create printed, with its command, in %s",
    (dir) => {
      const ends = play(dir);
      expect(ends).toHaveLength(1);
      const end = ends[0]!;
      expect(end.payload.args).toEqual({ command: "gh pr create --title Bump --body Bump" });
      expect(end.payload.result.content).toEqual([
        {
          type: "text",
          text: "\nCreating pull request for bump into main in example/lib\n\nhttps://github.com/example/lib/pull/41\n",
        },
      ]);
    },
  );

  it("keeps every other command's output and arguments in the agent's process", () => {
    for (const end of play("pi/0.87.1")) {
      const text = JSON.stringify(end);
      expect(text).not.toContain('"args"');
      expect(text).not.toContain('"content"');
      expect(text).not.toContain("hello.txt\\n");
    }
  });
});
