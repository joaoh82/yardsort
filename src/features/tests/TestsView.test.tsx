import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TestListing } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  testCatalog: vi.fn(),
  testRuns: vi.fn(),
  testActiveRuns: vi.fn(),
  testRunsById: vi.fn(),
  testRunSteps: vi.fn(),
  uiStateSave: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useProjectsStore } from "@/stores/projects";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { project, testRun, worktree } from "@/test/fixtures";
import { TestsView } from "./TestsView";

const listing = (id: string, over: Partial<TestListing> = {}): TestListing => ({
  item: {
    id,
    name: id === "health" ? "The server answers" : id,
    description: null,
    path: `/worktrees/app/fix-login/.yardsort/tests/${id}.yaml`,
    text: `id: ${id}\n`,
    problems: [],
    runnable: true,
    usesBrowser: false,
    browserMissing: false,
    inputs: [],
    ...over.item,
  },
  projectId: "p-app",
  projectName: "app",
  workspaceId: "w-app-fix-login",
  workspaceName: "fix-login",
  envMissing: [],
  lastRun: null,
  ...over,
});

describe("TestsView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.testCatalog.mockResolvedValue([
      listing("health", { lastRun: { runId: "r1", status: "passed", createdAt: 1, endedAt: 2 } }),
      listing("login", {
        projectId: "p-app",
        envMissing: ["PASSWORD"],
      }),
    ]);
    core.testRuns.mockResolvedValue([testRun("r1", { status: "passed", requestedBy: "app" })]);
    core.testActiveRuns.mockResolvedValue([]);
    core.testRunsById.mockResolvedValue([]);
    useProjectsStore.setState({
      projects: [{ ...project("app"), workspaces: [worktree("app", "fix-login")] }],
      ui: {},
    });
    useSmokeTestsStore.setState({
      catalog: [],
      catalogLoaded: false,
      runs: [],
      loaded: false,
      open: {},
      shown: {},
      steps: {},
    });
  });

  it("lists every workspace's tests with their last verdict and what they need", async () => {
    render(<TestsView />);
    const list = await screen.findByRole("list", { name: "Tests" });
    expect(within(list).getByText("The server answers")).toBeInTheDocument();
    expect(within(list).getByText("passed")).toBeInTheDocument();
    // The second test has never run and needs an unset environment value.
    expect(within(list).getByText("login")).toBeInTheDocument();
    expect(within(list).getByText(/unset PASSWORD/)).toBeInTheDocument();
    expect(within(list).getByText("never run")).toBeInTheDocument();
  });

  it("filters to the tests that need attention", async () => {
    const user = userEvent.setup();
    render(<TestsView />);
    await screen.findByRole("list", { name: "Tests" });
    await user.click(screen.getByRole("checkbox", { name: "Needs attention" }));
    const list = screen.getByRole("list", { name: "Tests" });
    expect(within(list).queryByText("The server answers")).toBeNull();
    expect(within(list).getByText("login")).toBeInTheDocument();
  });

  it("switches to the Runs tab and lists runs", async () => {
    const user = userEvent.setup();
    render(<TestsView />);
    await screen.findByRole("list", { name: "Tests" });
    await user.click(screen.getByRole("tab", { name: "Runs" }));
    const runs = await screen.findByRole("list", { name: "Runs" });
    expect(within(runs).getByText("The server answers")).toBeInTheDocument();
  });

  it("opens a test to its file, history and Run in…", async () => {
    const user = userEvent.setup();
    render(<TestsView />);
    const list = await screen.findByRole("list", { name: "Tests" });
    await user.click(within(list).getByText("The server answers"));
    expect(await screen.findByLabelText("Run in workspace")).toBeInTheDocument();
    // The read-only file is shown.
    expect(screen.getByText(/^id: health/)).toBeInTheDocument();
  });

  it("resets the run destination when a different workspace's copy is opened", async () => {
    // The same test id lives in two worktrees of the project. Opening one then the other must not
    // leave the run destination pointing at the first workspace.
    core.testCatalog.mockResolvedValue([
      listing("health", { workspaceId: "w-app-a", workspaceName: "a" }),
      listing("health", { workspaceId: "w-app-b", workspaceName: "b" }),
    ]);
    const user = userEvent.setup();
    render(<TestsView />);
    await screen.findByRole("list", { name: "Tests" });
    await user.click(screen.getByText("app / a").closest("button")!);
    await user.click(screen.getByText("app / b").closest("button")!);
    expect(await screen.findByLabelText("Run in workspace")).toHaveValue("w-app-b");
  });

  it("loads a filtered workspace's own history, past the global cap", async () => {
    const user = userEvent.setup();
    render(<TestsView />);
    await screen.findByRole("list", { name: "Tests" });
    core.testRuns.mockClear();
    await user.selectOptions(screen.getByLabelText("Workspace"), "w-app-fix-login");
    // The workspace's full history is fetched for the filter, not just the global cache.
    expect(core.testRuns).toHaveBeenCalledWith("w-app-fix-login", expect.any(Number));
  });
});
