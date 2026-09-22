/** Shared reporting helpers for the benchmark runners. See docs/design/07-terminal-benchmarks.md. */

export function round(value: number, digits = 1): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

export interface Percentiles {
  p50: number;
  p95: number;
  p99: number;
  max: number;
}

/**
 * The shape of a sample set, rounded to `digits`. Sorts a copy, so the caller keeps its order.
 * Latency wants two decimals — a fast round trip is a fraction of a millisecond, and rounding
 * that to a tenth throws away the difference the benchmark exists to show.
 */
export function percentiles(samples: number[], digits = 1): Percentiles {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (quantile: number) =>
    round(sorted[Math.min(sorted.length - 1, Math.floor(quantile * sorted.length))] ?? 0, digits);
  return { p50: at(0.5), p95: at(0.95), p99: at(0.99), max: round(sorted.at(-1) ?? 0, digits) };
}
