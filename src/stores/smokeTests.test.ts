import { beforeEach, describe, expect, it, vi } from "vitest";
import { testRun } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  testRuns: vi.fn(),
  testActiveRuns: vi.fn(),
  testRunsById: vi.fn(),
  testRunSteps: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useSmokeTestsStore } from "./smokeTests";

beforeEach(() => {
  vi.clearAllMocks();
  useSmokeTestsStore.setState({
    runs: [],
    loaded: false,
    steps: {},
    open: {},
    shown: {},
    error: null,
  });
  core.testRuns.mockResolvedValue([]);
  core.testActiveRuns.mockResolvedValue([]);
  core.testRunsById.mockResolvedValue([]);
  core.testRunSteps.mockResolvedValue([]);
});

describe("smoke tests store load", () => {
  it("keeps an active run that a full page of newer finished runs would otherwise crowd out", async () => {
    // A run waiting for approval in one workspace, older than the 200 newest finished runs
    // across all of them: it must survive the merge, so its approval card still shows.
    const waiting = testRun("waiting", {
      status: "awaiting_approval",
      workspaceId: "w1",
      createdAt: 1,
    });
    const history = Array.from({ length: 200 }, (_, i) =>
      testRun(`done-${i}`, { status: "passed", createdAt: 1000 + i }),
    );
    core.testRuns.mockResolvedValue(history);
    core.testActiveRuns.mockResolvedValue([waiting]);

    await useSmokeTestsStore.getState().load();

    const runs = useSmokeTestsStore.getState().runs;
    expect(runs.find((run) => run.id === "waiting")).toBeTruthy();
    // Newest first, and no run is listed twice.
    expect(runs[0]?.id).toBe("done-199");
    expect(new Set(runs.map((run) => run.id)).size).toBe(runs.length);
  });

  it("loads an open tab's run even once it has aged out of the history page", async () => {
    const ended = testRun("old-ended", { status: "passed", createdAt: 1 });
    core.testRuns.mockResolvedValue([testRun("recent", { createdAt: 1000 })]);
    core.testRunsById.mockResolvedValue([ended]);
    useSmokeTestsStore.setState({ open: { w1: ["old-ended"] } });

    await useSmokeTestsStore.getState().load();

    expect(core.testRunsById).toHaveBeenCalledWith(["old-ended"]);
    expect(useSmokeTestsStore.getState().runs.find((run) => run.id === "old-ended")).toBeTruthy();
  });

  it("does not fetch an open run that the history already carries", async () => {
    core.testRuns.mockResolvedValue([testRun("r1", { createdAt: 1 })]);
    useSmokeTestsStore.setState({ open: { w1: ["r1"] } });

    await useSmokeTestsStore.getState().load();

    expect(core.testRunsById).not.toHaveBeenCalled();
  });
});
