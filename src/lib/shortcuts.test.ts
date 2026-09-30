import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});
describe.each(["MacIntel", "Linux x86_64", "Win32"])("shortcuts on %s", (platform) => {
  it("uses the platform modifier, layout characters and shifted punctuation", async () => {
    vi.stubGlobal("navigator", { platform });
    const { eventBinding } = await import("./shortcuts");
    const mod = platform === "MacIntel" ? { metaKey: true } : { ctrlKey: true, shiftKey: true };
    expect(eventBinding(new KeyboardEvent("keydown", { key: "K", ...mod }))).toBe("k");
    expect(eventBinding(new KeyboardEvent("keydown", { key: "?", ...mod }))).toBe("/");
    expect(eventBinding(new KeyboardEvent("keydown", { key: "<", ...mod }))).toBe(",");
    expect(eventBinding(new KeyboardEvent("keydown", { key: "b", ctrlKey: true }))).toBeNull();
    expect(
      eventBinding(new KeyboardEvent("keydown", { key: "b", ...mod, isComposing: true })),
    ).toBeNull();
  });
});
it("recovers safely from malformed or conflicting persisted bindings", async () => {
  const { readBindings, DEFAULT_BINDINGS } = await import("./shortcuts");
  for (const raw of [
    "null",
    "[]",
    "bad json",
    '{"toggleLeft":42}',
    '{"toggleLeft":"k"}',
    '{"toggleLeft":"c"}',
  ])
    expect(readBindings(raw)).toEqual(DEFAULT_BINDINGS);
  expect(readBindings('{"toggleLeft":null}').toggleLeft).toBeNull();
});
