import { afterEach, beforeEach, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({ notify: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));

import { useRemoteStore } from "./remote";

const phone = {
  grantId: "g-1",
  deviceName: "Ana's phone",
  wantsControl: true,
  expiresInMs: 60_000,
};
const tablet = { grantId: "g-2", deviceName: "Tablet", wantsControl: false, expiresInMs: 60_000 };

beforeEach(() => {
  vi.useFakeTimers();
  native.notify.mockReset().mockResolvedValue(undefined);
  useRemoteStore.getState().settled([]);
});
afterEach(() => vi.useRealTimers());

it("says on the desktop that a device asks, once, when nobody is looking", () => {
  const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
  useRemoteStore.getState().told(phone);
  useRemoteStore.getState().told(phone);
  expect(useRemoteStore.getState().asking).toEqual([phone]);
  expect(native.notify).toHaveBeenCalledTimes(1);
  expect(native.notify).toHaveBeenCalledWith(
    "Ana's phone asks to reach this computer",
    "Answer in Settings → Remote.",
  );

  // Looking at the window: the status bar says it, not the desktop.
  focus.mockReturnValue(true);
  useRemoteStore.getState().told(tablet);
  expect(useRemoteStore.getState().asking).toEqual([phone, tablet]);
  expect(native.notify).toHaveBeenCalledTimes(1);
  focus.mockRestore();
});

it("lets a request go when it lapses, or when the server no longer has it waiting", () => {
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  useRemoteStore.getState().told(phone);
  useRemoteStore.getState().told({ ...tablet, expiresInMs: 120_000 });
  vi.advanceTimersByTime(60_000);
  expect(useRemoteStore.getState().asking.map((r) => r.grantId)).toEqual(["g-2"]);

  useRemoteStore.getState().settled([]);
  expect(useRemoteStore.getState().asking).toEqual([]);
  // Its lapse is forgotten with it: telling of it again later shows it again.
  vi.advanceTimersByTime(120_000);
  useRemoteStore.getState().told(tablet);
  expect(useRemoteStore.getState().asking).toEqual([tablet]);
});
