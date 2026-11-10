// A development build's timing readout: a keystroke to what comes back, and the foreground to a
// live screen, with nothing else counted.

import { describe, expect, test } from "bun:test";

import { Timings, timingWords } from "../../src/lib/timing";

describe("timings", () => {
  test("time a keystroke to the output after it, from the first one still waiting", () => {
    let now = 0;
    const timings = new Timings(() => now);
    timings.heard();
    expect(timings.getSnapshot().echoMs).toBeNull();
    timings.typed();
    now = 40;
    timings.typed();
    now = 120;
    timings.heard();
    expect(timings.getSnapshot().echoMs).toBe(120);
    now = 500;
    timings.heard();
    expect(timings.getSnapshot().echoMs).toBe(120);
  });

  test("time coming back to a live screen, and forget what was typed before", () => {
    let now = 0;
    const timings = new Timings(() => now);
    timings.drawn();
    expect(timings.getSnapshot().backMs).toBeNull();
    timings.typed();
    now = 1_000;
    timings.back();
    now = 2_300;
    timings.heard();
    expect(timings.getSnapshot().echoMs).toBeNull();
    timings.drawn();
    expect(timings.getSnapshot().backMs).toBe(1_300);
    expect(timingWords(timings.getSnapshot())).toBe("Echo not yet · back to live 1.3 s");
    expect(timingWords({ echoMs: 84.4, backMs: null })).toBe("Echo 84 ms · back to live not yet");
  });
});
