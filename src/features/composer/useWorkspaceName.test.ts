import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SuggestedName } from "@/lib/ipc";

const core = vi.hoisted(() => ({ workspaceSuggestName: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

import { NAME_DELAY_MS, useWorkspaceName } from "./useWorkspaceName";

const words = (text: string): SuggestedName => ({
  name: text
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean)
    .slice(0, 4)
    .join("-"),
  by: null,
  problem: null,
});
const named = (name: string): SuggestedName => ({ name, by: "Claude Code", problem: null });

/** Model requests made, each resolved by hand, in the order they reached the core. */
let asked: { text: string; answer: (found: SuggestedName) => void }[] = [];

function render(message: string) {
  return renderHook(({ message }) => useWorkspaceName("p", message, "claude", true, true), {
    initialProps: { message },
  });
}

/** Let the timers and the promise callbacks behind them run. */
async function pass(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

describe("useWorkspaceName", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    asked = [];
    core.workspaceSuggestName.mockImplementation((_p: string, text: string, ask: boolean) =>
      ask
        ? new Promise<SuggestedName>((resolve) => asked.push({ text, answer: resolve }))
        : Promise.resolve(words(text)),
    );
  });
  afterEach(() => vi.useRealTimers());

  it("drops a model's name once the message it named has changed", async () => {
    const hook = render("Fix the login redirect");
    await pass(NAME_DELAY_MS);
    await act(async () => asked[0]!.answer(named("login-redirect")));
    expect(hook.result.current.shown).toBe("login-redirect");
    expect(hook.result.current.by).toBe("Claude Code");

    // Too short to ask about again: what is shown must be what Start will make.
    hook.rerender({ message: "Fix UI" });
    await pass(NAME_DELAY_MS);
    expect(hook.result.current.shown).toBe("fix-ui");
    expect(hook.result.current.by).toBeNull();
    await expect(hook.result.current.settle()).resolves.toBeUndefined();
  });

  it("asks one thing at a time when Start follows an edit, and keeps only the newer answer", async () => {
    const hook = render("Fix the login redirect");
    await pass(NAME_DELAY_MS);
    expect(asked).toHaveLength(1);

    hook.rerender({ message: "Fix the logout redirect" });
    let settled: Promise<string | undefined> = Promise.resolve(undefined);
    act(() => void (settled = hook.result.current.settle()));
    await pass(0);
    expect(asked).toHaveLength(1);

    await act(async () => asked[0]!.answer(named("login-redirect")));
    await pass(0);
    expect(asked.map((request) => request.text)).toEqual([
      "Fix the login redirect",
      "Fix the logout redirect",
    ]);
    expect(hook.result.current.shown).not.toBe("login-redirect");

    await act(async () => asked[1]!.answer(named("logout-redirect")));
    await expect(settled).resolves.toBe("logout-redirect");
    expect(hook.result.current.shown).toBe("logout-redirect");
  });
});
