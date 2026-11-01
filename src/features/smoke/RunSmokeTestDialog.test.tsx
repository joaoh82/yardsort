import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TestItem } from "@/lib/ipc";
import { testReview, testRun } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  testList: vi.fn(),
  testReview: vi.fn(),
  testRunStart: vi.fn(),
  testRuns: vi.fn(),
  testRunSteps: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useSmokeTestsStore } from "@/stores/smokeTests";
import { RunSmokeTestDialog } from "./RunSmokeTestDialog";

const health: TestItem = {
  id: "health",
  name: "The server answers",
  description: "Starts the server and asks it.",
  path: "/worktrees/app/fix-login/.yardsort/tests/health.yaml",
  text: "id: health\n",
  problems: [],
  runnable: true,
  usesBrowser: false,
  inputs: [
    {
      id: "user",
      kind: "text",
      label: "Who to log in as",
      required: true,
      default: "demo@example.test",
      options: [],
    },
    {
      id: "mode",
      kind: "choice",
      label: "Mode",
      required: false,
      default: null,
      options: ["fast", "slow"],
    },
  ],
};
const broken: TestItem = {
  id: "login",
  name: "Log in",
  description: null,
  path: "/worktrees/app/fix-login/.yardsort/tests/login.yaml",
  text: "id: login\n",
  problems: [{ line: 4, column: 3, message: "A `run` step needs `program`." }],
  runnable: false,
  usesBrowser: false,
  inputs: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  useSmokeTestsStore.setState({
    runs: [],
    loaded: true,
    steps: {},
    open: {},
    shown: {},
    error: null,
  });
  core.testList.mockResolvedValue([health, broken]);
  core.testReview.mockResolvedValue(testReview());
  core.testRunStart.mockResolvedValue({ runId: "r9", awaitingApproval: false });
  core.testRuns.mockResolvedValue([]);
  core.testRunSteps.mockResolvedValue([]);
});

describe("RunSmokeTestDialog", () => {
  it("lists the tests with their readiness, picks the one that can run, and runs it with its inputs", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<RunSmokeTestDialog workspaceId="w-app-fix-login" onClose={onClose} />);
    const choose = await screen.findByRole("combobox", { name: "Test" });
    expect(
      within(choose).getByRole("option", { name: "The server answers — ready" }),
    ).toBeInTheDocument();
    expect(within(choose).getByRole("option", { name: "Log in — 1 problem" })).toBeInTheDocument();
    // The one runnable test is chosen already, its inputs on their defaults.
    expect(choose).toHaveValue("health");
    expect(screen.getByRole("textbox", { name: "Who to log in as" })).toHaveValue(
      "demo@example.test",
    );
    const steps = await screen.findByRole("list", { name: "Setup steps" });
    expect(steps).toHaveTextContent("start `bun run dev --port {{ run.port }}`");
    expect(screen.getByText(/Pressing Run approves this file/)).toBeInTheDocument();

    await user.selectOptions(screen.getByRole("combobox", { name: "Mode" }), "slow");
    await user.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() =>
      expect(core.testRunStart).toHaveBeenCalledWith(
        "health",
        "w-app-fix-login",
        { user: "demo@example.test", mode: "slow" },
        "abc",
      ),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(useSmokeTestsStore.getState().shown["w-app-fix-login"]).toBe("r9");
  });

  it("a test with problems shows them and cannot run", async () => {
    const user = userEvent.setup();
    render(<RunSmokeTestDialog workspaceId="w-app-fix-login" onClose={() => {}} />);
    await user.selectOptions(await screen.findByRole("combobox", { name: "Test" }), "login");
    expect(screen.getByRole("list", { name: "Problems" })).toHaveTextContent(
      "4:3: A `run` step needs `program`.",
    );
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
  });

  it("reopens a recent run from here", async () => {
    const user = userEvent.setup();
    useSmokeTestsStore.setState({ runs: [testRun("r1", { status: "passed" })] });
    const onClose = vi.fn();
    render(<RunSmokeTestDialog workspaceId="w-app-fix-login" onClose={onClose} />);
    const recent = await screen.findByRole("region", { name: "Recent runs" });
    await user.click(within(recent).getByRole("button", { name: /The server answers/ }));
    expect(useSmokeTestsStore.getState().shown["w-app-fix-login"]).toBe("r1");
    expect(onClose).toHaveBeenCalled();
  });

  it("says when the workspace has no tests", async () => {
    core.testList.mockResolvedValue([]);
    render(<RunSmokeTestDialog workspaceId="w-app-fix-login" onClose={() => {}} />);
    expect(await screen.findByText(/has no smoke tests/)).toBeInTheDocument();
  });
});
