import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  scheduleList: vi.fn(),
  scheduleRuns: vi.fn(),
  scheduleStartedRuns: vi.fn(),
  scheduleSetEnabled: vi.fn(),
  scheduleRemove: vi.fn(),
  scheduleRunNow: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { schedule, scheduleRun } from "@/test/fixtures";
import { needingAttention, scheduleOf, useSchedulesStore } from "./schedules";

beforeEach(() => {
  vi.clearAllMocks();
  useSchedulesStore.setState({ items: [], loaded: false, history: {}, started: [], error: null });
  core.scheduleList.mockResolvedValue([schedule("s1")]);
  core.scheduleStartedRuns.mockResolvedValue([
    { runId: "run-1", scheduleId: "s1", scheduleName: "Morning ping" },
  ]);
  core.scheduleRuns.mockResolvedValue([scheduleRun("l1")]);
});

describe("the schedules store", () => {
  it("loads the list and the runs schedules started", async () => {
    await useSchedulesStore.getState().load();
    const state = useSchedulesStore.getState();
    expect(state.loaded).toBe(true);
    expect(state.items.map((item) => item.id)).toEqual(["s1"]);
    expect(scheduleOf(state.started, "run-1")?.scheduleName).toBe("Morning ping");
    expect(scheduleOf(state.started, "run-2")).toBeUndefined();
  });

  it("reads again, after a change, the histories already on screen and no others", async () => {
    await useSchedulesStore.getState().loadHistory("s1");
    core.scheduleSetEnabled.mockResolvedValue(schedule("s1", { enabled: false }));
    expect(await useSchedulesStore.getState().setEnabled("s1", false)).toBe(true);
    expect(core.scheduleSetEnabled).toHaveBeenCalledWith("s1", false);
    expect(core.scheduleList).toHaveBeenCalledTimes(1);
    expect(core.scheduleRuns).toHaveBeenCalledTimes(2);
    expect(core.scheduleRuns).toHaveBeenLastCalledWith("s1");
  });

  it("says what went wrong and reports that nothing was done", async () => {
    core.scheduleRunNow.mockRejectedValue({
      code: "workflow_already_running",
      message: "`ping` is already running in `fix-login`.",
    });
    expect(await useSchedulesStore.getState().runNow("s1")).toBe(false);
    expect(useSchedulesStore.getState().error).toBe("`ping` is already running in `fix-login`.");
    expect(core.scheduleList).not.toHaveBeenCalled();
  });

  it("forgets a removed schedule's history", async () => {
    await useSchedulesStore.getState().loadHistory("s1");
    core.scheduleRemove.mockResolvedValue(undefined);
    core.scheduleList.mockResolvedValue([]);
    expect(await useSchedulesStore.getState().remove("s1")).toBe(true);
    expect(useSchedulesStore.getState().history).toEqual({});
    expect(useSchedulesStore.getState().items).toEqual([]);
  });

  it("counts the schedules that need the user", () => {
    expect(
      needingAttention([
        schedule("a"),
        schedule("b", { standing: "awaiting_approval" }),
        schedule("c", { standing: "refused" }),
        schedule("d", { standing: "no_workspace" }),
        schedule("e", { standing: "off" }),
        schedule("f", { standing: "running" }),
        schedule("g", { standing: "done" }),
      ]),
    ).toBe(3);
  });
});
