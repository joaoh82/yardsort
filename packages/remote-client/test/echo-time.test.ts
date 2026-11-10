// The timing tool's parts: the delay proxy holds each way back as long as it says, in order, and
// the percentiles are by nearest rank (35 §3, "Measuring the budgets").

import { describe, expect, test } from "bun:test";

import { delayProxy } from "../tools/delay-proxy.ts";
import { percentile, summary } from "../tools/echo-time.ts";

describe("the delay proxy", () => {
  test("a round trip through it takes twice its delay longer, and nothing comes out of order", async () => {
    // A server that answers each piece with itself.
    const echo = Bun.listen({
      hostname: "127.0.0.1",
      port: 0,
      socket: { data: (socket, bytes) => void socket.write(bytes) },
    });
    const proxy = delayProxy(`127.0.0.1:${echo.port}`, 60);
    let heard = "";
    let done!: () => void;
    const all = new Promise<void>((resolve) => (done = resolve));
    const sent = performance.now();
    const socket = await Bun.connect({
      hostname: "127.0.0.1",
      port: proxy.port,
      socket: {
        data(_, bytes) {
          heard += new TextDecoder().decode(bytes);
          if (heard.length >= 10) done();
        },
      },
    });
    for (const piece of ["01", "23", "45", "67", "89"]) socket.write(piece);
    await all;
    const took = performance.now() - sent;
    expect(heard).toBe("0123456789");
    expect(took).toBeGreaterThanOrEqual(115);
    expect(took).toBeLessThan(2_000);
    socket.end();
    proxy.stop();
    echo.stop(true);
  });

  test("a target that is not host:port is refused", () => {
    expect(() => delayProxy("nowhere", 10)).toThrow("is not host:port");
  });
});

describe("percentiles", () => {
  test("are by nearest rank", () => {
    const hundred = Array.from({ length: 100 }, (_, n) => n + 1);
    expect(percentile(hundred, 0.5)).toBe(50);
    expect(percentile(hundred, 0.95)).toBe(95);
    expect(percentile(hundred, 0.99)).toBe(99);
    expect(percentile([7], 0.99)).toBe(7);
    expect(summary([30, 10, 20])).toEqual({
      count: 3,
      min: 10,
      p50: 20,
      p95: 30,
      p99: 30,
      max: 30,
    });
  });
});
