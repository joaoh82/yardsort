import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BusyClock, elapsedWords } from "./BusyClock";
import { StatusDot } from "./StatusDot";

describe("elapsedWords", () => {
  it("counts seconds, then minutes, then hours and minutes", () => {
    expect(elapsedWords(-5)).toBe("0s");
    expect(elapsedWords(999)).toBe("0s");
    expect(elapsedWords(59_999)).toBe("59s");
    expect(elapsedWords(60_000)).toBe("1m");
    expect(elapsedWords(59 * 60_000 + 59_999)).toBe("59m");
    expect(elapsedWords(60 * 60_000)).toBe("1h");
    expect(elapsedWords(125 * 60_000)).toBe("2h 5m");
  });
});

describe("BusyClock", () => {
  beforeEach(() => vi.useFakeTimers({ now: 1_000_000 }));
  afterEach(() => vi.useRealTimers());

  it("counts up, every clock on one shared tick that stops when nothing shows", () => {
    const interval = vi.spyOn(globalThis, "setInterval");
    const view = render(
      <>
        <BusyClock since={1_000_000 - 58_000} />
        <BusyClock since={1_000_000 - 3_000} />
      </>,
    );
    expect(screen.getByLabelText("busy for 58s")).toBeInTheDocument();
    expect(screen.getByLabelText("busy for 3s")).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByLabelText("busy for 1m")).toBeInTheDocument();
    expect(screen.getByLabelText("busy for 5s")).toBeInTheDocument();
    // However many clocks there are, they change together: one repaint a second at most.
    expect(interval).toHaveBeenCalledTimes(1);

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    interval.mockRestore();
  });
});

describe("StatusDot", () => {
  it("does not animate when busy: an endless animation repaints the whole window every frame", () => {
    render(<StatusDot activity="busy" />);
    expect(screen.getByRole("img", { name: "busy" }).className).not.toMatch(/animate-/);
  });
});
