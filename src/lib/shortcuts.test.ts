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
it("uses defaults with a visible notice for unreadable saved data", async () => {
  const { readBindings, DEFAULT_BINDINGS } = await import("./shortcuts");
  for (const raw of ["null", "[]", "bad json"]) {
    const result = readBindings(raw);
    expect(result.bindings).toEqual(DEFAULT_BINDINGS);
    expect(result.notice).toMatch(/Saved shortcuts/);
  }
  expect(readBindings(undefined)).toEqual({ bindings: DEFAULT_BINDINGS, notice: null });
});
it("preserves saved choices over newly introduced defaults", async () => {
  const { readBindings, DEFAULT_BINDINGS, bindingError } = await import("./shortcuts");
  // Simulate a saved map from before the next-terminal command existed.
  const older: Partial<typeof DEFAULT_BINDINGS> = {
    ...DEFAULT_BINDINGS,
    toggleLeft: "ArrowRight",
    toggleRight: "j",
    tour: null,
  };
  delete older.nextTerminal;
  const result = readBindings(JSON.stringify(older));
  expect(result.bindings).toEqual({ ...older, nextTerminal: null });
  expect(result.notice).toContain("Next terminal tab was left unassigned");
  expect(bindingError(result.bindings)).toBeNull();
});
it("recovers bad entries individually and keeps unrelated customisations", async () => {
  const { readBindings, bindingError } = await import("./shortcuts");
  const result = readBindings(
    JSON.stringify({
      toggleLeft: 42,
      toggleRight: "j",
      tour: null,
      focusWorkspace: "c",
      palette: "k",
      newWorkspace: "k",
    }),
  );
  expect(result.bindings).toMatchObject({
    toggleLeft: "b",
    toggleRight: "j",
    tour: null,
    focusWorkspace: "e",
    palette: "k",
    newWorkspace: null,
  });
  expect(result.notice).toContain("invalid saved binding");
  expect(result.notice).toContain("duplicates another saved shortcut");
  expect(bindingError(result.bindings)).toBeNull();
});
it("keeps explicit unassigned choices and valid swaps without a notice", async () => {
  const { readBindings } = await import("./shortcuts");
  const result = readBindings(JSON.stringify({ palette: "b", toggleLeft: "k", toggleRight: null }));
  expect(result.bindings).toMatchObject({ palette: "b", toggleLeft: "k", toggleRight: null });
  expect(result.notice).toBeNull();
});
