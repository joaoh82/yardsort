import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  triggerList: vi.fn(),
  triggerFires: vi.fn(),
  triggerStartedRuns: vi.fn(),
  triggerPresets: vi.fn(),
  triggerSetEnabled: vi.fn(),
  triggerReset: vi.fn(),
  triggerRemove: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { trigger, triggerFire, triggerPresets } from "@/test/fixtures";
import { needingAttention, triggerOf, useTriggersStore } from "./triggers";

beforeEach(() => {
  vi.clearAllMocks();
  useTriggersStore.setState({
    items: [],
    loaded: false,
    history: {},
    started: [],
    presets: [],
    error: null,
  });
  core.triggerList.mockResolvedValue([trigger("t1")]);
  core.triggerStartedRuns.mockResolvedValue([
    { runId: "run-1", triggerId: "t1", triggerName: "Address reviews" },
  ]);
  core.triggerFires.mockResolvedValue([triggerFire("f1")]);
  core.triggerPresets.mockResolvedValue(triggerPresets);
});

describe("the triggers store", () => {
  it("loads the list and the runs triggers started", async () => {
    await useTriggersStore.getState().load();
    const state = useTriggersStore.getState();
    expect(state.loaded).toBe(true);
    expect(state.items.map((item) => item.id)).toEqual(["t1"]);
    expect(triggerOf(state.started, "run-1")?.triggerName).toBe("Address reviews");
    expect(triggerOf(state.started, "run-2")).toBeUndefined();
  });

  it("reads again, after a change, the histories already on screen and no others", async () => {
    await useTriggersStore.getState().loadHistory("t1");
    core.triggerReset.mockResolvedValue(trigger("t1"));
    expect(await useTriggersStore.getState().reset("t1")).toBe(true);
    expect(core.triggerReset).toHaveBeenCalledWith("t1");
    expect(core.triggerList).toHaveBeenCalledTimes(1);
    expect(core.triggerFires).toHaveBeenCalledTimes(2);
  });

  it("says what went wrong and reports that nothing was done", async () => {
    core.triggerSetEnabled.mockRejectedValue({
      code: "workspace_not_found",
      message: "`fix-login` was deleted, so this has nothing to listen to.",
    });
    expect(await useTriggersStore.getState().setEnabled("t1", true)).toBe(false);
    expect(useTriggersStore.getState().error).toBe(
      "`fix-login` was deleted, so this has nothing to listen to.",
    );
    expect(core.triggerList).not.toHaveBeenCalled();
  });

  it("forgets a removed trigger's history", async () => {
    await useTriggersStore.getState().loadHistory("t1");
    core.triggerRemove.mockResolvedValue(undefined);
    core.triggerList.mockResolvedValue([]);
    expect(await useTriggersStore.getState().remove("t1")).toBe(true);
    expect(useTriggersStore.getState().history).toEqual({});
    expect(useTriggersStore.getState().items).toEqual([]);
  });

  it("asks for the presets once", async () => {
    await useTriggersStore.getState().loadPresets();
    await useTriggersStore.getState().loadPresets();
    expect(core.triggerPresets).toHaveBeenCalledTimes(1);
    expect(useTriggersStore.getState().presets).toHaveLength(3);
  });

  it("counts the triggers that need the user", () => {
    expect(
      needingAttention([
        trigger("a"),
        trigger("b", { standing: "awaiting_approval" }),
        trigger("c", { standing: "capped" }),
        trigger("d", { standing: "refused" }),
        trigger("e", { standing: "no_workspace" }),
        trigger("f", { standing: "off" }),
        trigger("g", { standing: "running" }),
        trigger("h", { standing: "waiting" }),
      ]),
    ).toBe(4);
  });
});
