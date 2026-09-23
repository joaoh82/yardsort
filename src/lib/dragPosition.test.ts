import { afterEach, describe, expect, it, vi } from "vitest";
import { isDragOver } from "./dragPosition";

describe("isDragOver", () => {
  const element = document.createElement("div");

  afterEach(() => {
    vi.restoreAllMocks();
    Object.defineProperty(window, "devicePixelRatio", { value: 1, configurable: true });
  });

  function box(right: number, bottom: number) {
    vi.spyOn(element, "getBoundingClientRect").mockReturnValue({
      left: 10,
      top: 20,
      right,
      bottom,
      width: right - 10,
      height: bottom - 20,
      x: 10,
      y: 20,
      toJSON() {},
    });
  }

  it("uses the position as logical pixels on Linux and macOS", () => {
    box(110, 120);
    expect(isDragOver(element, { x: 10, y: 20 }, false)).toBe(true);
    expect(isDragOver(element, { x: 109, y: 119 }, false)).toBe(true);
    expect(isDragOver(element, { x: 110, y: 20 }, false)).toBe(false);
    expect(isDragOver(element, { x: 9, y: 20 }, false)).toBe(false);
  });

  it("divides a Windows position by the device pixel ratio", () => {
    box(110, 120);
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    // 180 physical pixels is 90 logical, inside the box. Unscaled, 180 is past the right edge.
    expect(isDragOver(element, { x: 180, y: 100 }, true)).toBe(true);
    expect(isDragOver(element, { x: 180, y: 100 }, false)).toBe(false);
  });
});
