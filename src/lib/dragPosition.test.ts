import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

  describe("with something on top", () => {
    const child = document.createElement("span");
    const panel = document.createElement("aside");
    element.append(child);
    let top: Element | null = null;

    beforeEach(() => {
      // jsdom does no layout, so what is at a point is said here.
      document.elementFromPoint = vi.fn(() => top);
    });
    afterEach(() => {
      delete (document as { elementFromPoint?: unknown }).elementFromPoint;
    });

    it("takes the point when it, or something inside it, is what is there", () => {
      box(110, 120);
      top = element;
      expect(isDragOver(element, { x: 50, y: 50 }, false)).toBe(true);
      top = child;
      expect(isDragOver(element, { x: 50, y: 50 }, false)).toBe(true);
    });

    it("leaves the point to whatever covers it, though its box holds it", () => {
      box(110, 120);
      top = panel;
      expect(isDragOver(element, { x: 50, y: 50 }, false)).toBe(false);
      top = null;
      expect(isDragOver(element, { x: 50, y: 50 }, false)).toBe(false);
    });

    it("asks at the logical position on Windows", () => {
      box(110, 120);
      Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
      top = element;
      isDragOver(element, { x: 180, y: 100 }, true);
      expect(document.elementFromPoint).toHaveBeenCalledWith(90, 50);
    });
  });
});
