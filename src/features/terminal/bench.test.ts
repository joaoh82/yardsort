import { describe, expect, it } from "vitest";
import { percentiles, round } from "./bench";

describe("round", () => {
  it("keeps a tenth by default and more when asked", () => {
    expect(round(8.2149)).toBe(8.2);
    expect(round(0.1149, 2)).toBe(0.11);
    // A sub-millisecond sample must not round away to nothing.
    expect(round(0.06, 2)).toBe(0.06);
  });
});

describe("percentiles", () => {
  it("describes the shape of a sample set", () => {
    const samples = Array.from({ length: 100 }, (_, i) => i + 1);
    expect(percentiles(samples)).toEqual({ p50: 51, p95: 96, p99: 100, max: 100 });
  });

  it("leaves the caller's array in its original order", () => {
    const samples = [3, 1, 2];
    percentiles(samples);
    expect(samples).toEqual([3, 1, 2]);
  });

  it("reports zeroes rather than throwing on an empty run", () => {
    expect(percentiles([])).toEqual({ p50: 0, p95: 0, p99: 0, max: 0 });
  });
});
