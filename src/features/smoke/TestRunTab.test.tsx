import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { testReview, testRun, testStep } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  testRunReview: vi.fn(),
  testRunSteps: vi.fn(),
  testRuns: vi.fn(),
  testStepAnswer: vi.fn(),
  testRunCancel: vi.fn(),
  testStepOutput: vi.fn(),
  testRunSend: vi.fn(),
  onTestRunOutput: vi.fn(),
  onTestResultSent: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
const native = vi.hoisted(() => ({ revealInFileManager: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));

import { useSmokeTestsStore } from "@/stores/smokeTests";
import { useTerminalStore } from "@/stores/terminals";
import { TestRunTab } from "./TestRunTab";

const now = Date.now();
const running = testRun("r1", {
  status: "running",
  port: 43210,
  startedAt: now - 30_000,
  outputDir: "/data/test-runs/r1",
});
const steps = [
  testStep("server", {
    phase: "setup",
    action: "start",
    status: "passed",
    startedAt: now - 30_000,
    endedAt: now - 29_000,
    outputs: {
      pid: "4242",
      stdout: "/data/test-runs/r1/server.stdout",
      stderr: "/data/test-runs/r1/server.stderr",
    },
  }),
  testStep("ping", {
    action: "http",
    status: "failed",
    startedAt: now - 29_000,
    endedAt: now - 28_000,
    outputs: { status: "500", body: '{"error":"no session"}' },
    note: "status: expected 200, got 500.",
  }),
  testStep("look", {
    action: "manual",
    status: "waiting",
    startedAt: now - 28_000,
    outputs: { prompt: "Does the dashboard chart animate in?" },
  }),
  testStep("reset", { phase: "teardown", action: "run" }),
];

beforeEach(() => {
  vi.clearAllMocks();
  useSmokeTestsStore.setState({
    runs: [running],
    loaded: true,
    steps: { r1: steps },
    open: { "w-app-fix-login": ["r1"] },
    shown: { "w-app-fix-login": "r1" },
    error: null,
  });
  useTerminalStore.setState({ tabs: [], active: {}, error: null });
  core.testRunReview.mockResolvedValue(testReview());
  core.testRunSteps.mockResolvedValue(steps);
  core.testRuns.mockResolvedValue([running]);
  core.testStepAnswer.mockResolvedValue(true);
  core.testRunCancel.mockResolvedValue(true);
  core.testRunSend.mockResolvedValue({ delivered: true });
  core.testStepOutput.mockResolvedValue({
    text: "listening on 43210\n",
    from: 0,
    next: 19,
    size: 19,
  });
  core.onTestRunOutput.mockResolvedValue(() => {});
  core.onTestResultSent.mockResolvedValue(() => {});
});

describe("TestRunTab", () => {
  it("lists the steps by phase and opens on the one waiting for the person", async () => {
    render(<TestRunTab run={running} workspaceId="w-app-fix-login" />);
    const nav = screen.getByRole("navigation", { name: "Steps" });
    expect(within(nav).getByRole("heading", { name: "Setup" })).toBeInTheDocument();
    expect(within(nav).getByRole("heading", { name: "Teardown" })).toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: "ping, failed" })).toBeInTheDocument();
    const look = within(nav).getByRole("button", { name: "look, waiting for you" });
    expect(look).toHaveAttribute("aria-current", "true");
    expect(screen.getByText("Does the dashboard chart animate in?")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pass" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Send result/ })).toBeNull();
    expect(screen.getByText(/port 43210/)).toBeInTheDocument();
  });

  it("answers a manual step with a note", async () => {
    const user = userEvent.setup();
    render(<TestRunTab run={running} workspaceId="w-app-fix-login" />);
    await user.type(screen.getByRole("textbox", { name: "Note" }), "The axes flashed empty.");
    await user.click(screen.getByRole("button", { name: "Fail" }));
    expect(core.testStepAnswer).toHaveBeenCalledWith(
      "r1",
      "look",
      false,
      "The axes flashed empty.",
    );
  });

  it("shows a chosen step's note, facts, body and tailed output", async () => {
    const user = userEvent.setup();
    render(<TestRunTab run={running} workspaceId="w-app-fix-login" />);
    await user.click(screen.getByRole("button", { name: "ping, failed" }));
    expect(screen.getByLabelText("Note")).toHaveTextContent("status: expected 200, got 500.");
    expect(screen.getByRole("term")).toHaveTextContent("status");
    expect(screen.getByRole("definition")).toHaveTextContent("500");
    expect(screen.getByText(/no session/)).toBeInTheDocument();
    expect(
      await screen.findByText("GET http://localhost:{{ run.port }}/health; expect status 200"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "server, passed" }));
    await waitFor(() =>
      expect(screen.getByLabelText("stdout")).toHaveTextContent("listening on 43210"),
    );
    expect(core.testStepOutput).toHaveBeenCalledWith("r1", "server", "stdout", 0);
  });

  it("cancels a run that is going", async () => {
    const user = userEvent.setup();
    render(<TestRunTab run={running} workspaceId="w-app-fix-login" />);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(core.testRunCancel).toHaveBeenCalledWith("r1");
  });

  it("an ended run offers Run again and sends its result to the workspace's agent", async () => {
    const user = userEvent.setup();
    const ended = { ...running, status: "failed", endedAt: now };
    useTerminalStore.setState({
      tabs: [
        {
          id: "s-agent",
          workspaceId: "w-app-fix-login",
          title: "claude",
          exit: null,
          recordId: "rec-1",
          busy: false,
          attention: false,
          toldAbout: null,
          toldAt: null,
          busySince: null,
        },
        {
          id: "s-shell",
          workspaceId: "w-app-fix-login",
          title: "bash",
          exit: null,
          recordId: null,
          busy: false,
          attention: false,
          toldAbout: null,
          toldAt: null,
          busySince: null,
        },
      ],
      active: {},
      error: null,
    });
    render(<TestRunTab run={ended} workspaceId="w-app-fix-login" />);
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
    expect(screen.getByRole("button", { name: "Run again" })).toBeInTheDocument();
    // One agent, so no menu: the shell is not an agent and is not offered.
    await user.click(screen.getByRole("button", { name: "Send result to agent" }));
    expect(core.testRunSend).toHaveBeenCalledWith("r1", "s-agent");
    expect(await screen.findByRole("status")).toHaveTextContent("Sent.");
  });

  it("with no agent running, the result cannot be sent", () => {
    render(
      <TestRunTab
        run={{ ...running, status: "passed", endedAt: now }}
        workspaceId="w-app-fix-login"
      />,
    );
    expect(screen.getByRole("button", { name: "Send result to agent" })).toBeDisabled();
  });
});
