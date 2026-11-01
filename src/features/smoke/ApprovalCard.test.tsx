import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { testReview, testRun } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  testRunReview: vi.fn(),
  testRunApprove: vi.fn(),
  testRunRefuse: vi.fn(),
  testRuns: vi.fn(),
  testRunSteps: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useSmokeTestsStore } from "@/stores/smokeTests";
import { ApprovalCard } from "./ApprovalCard";

const waiting = testRun("r1", { inputs: { user: "demo@example.test" } });
const other = testRun("r2", { workspaceId: "w-other", workspaceName: "other" });

beforeEach(() => {
  vi.clearAllMocks();
  useSmokeTestsStore.setState({
    runs: [waiting, other],
    loaded: true,
    steps: {},
    open: {},
    shown: {},
    error: null,
  });
  core.testRunReview.mockResolvedValue(testReview());
  core.testRunApprove.mockResolvedValue(true);
  core.testRunRefuse.mockResolvedValue(true);
  core.testRuns.mockResolvedValue([other]);
  core.testRunSteps.mockResolvedValue([]);
});

describe("ApprovalCard", () => {
  it("shows the waiting run's steps in words, who asked, its inputs and the names it uses", async () => {
    render(<ApprovalCard workspaceId="w-app-fix-login" />);
    const card = await screen.findByRole("region", { name: "Run “The server answers”?" });
    expect(card).toHaveTextContent("Asked by an agent, with ys test run");
    expect(card).toHaveTextContent("user = demo@example.test");
    const setup = await within(card).findByRole("list", { name: "Setup steps" });
    expect(setup).toHaveTextContent("start `bun run dev --port {{ run.port }}`");
    expect(within(card).getByRole("list", { name: "Steps steps" })).toHaveTextContent(
      "GET http://localhost:{{ run.port }}/health; expect status 200",
    );
    expect(card).toHaveTextContent("DATABASE_URL (not set)");
    expect(card).toHaveTextContent("one is not set and would be empty");
    expect(core.testRunReview).toHaveBeenCalledWith("r1");
    // Another workspace's run is not this workspace's business.
    expect(screen.queryByText(/other/)).toBeNull();
  });

  it("shows what changed since the version approved before", async () => {
    core.testRunReview.mockResolvedValue(
      testReview({ previous: "id: health\nname: The server answers\nversion: 1\n" }),
    );
    render(<ApprovalCard workspaceId="w-app-fix-login" />);
    const changed = await screen.findByLabelText("What changed");
    expect(changed).toHaveTextContent("+ - id: ping");
    expect(changed).not.toHaveTextContent("- id: health");
  });

  it("Run approves and opens the run's tab; Don't run refuses", async () => {
    const user = userEvent.setup();
    render(<ApprovalCard workspaceId="w-app-fix-login" />);
    await screen.findByRole("list", { name: "Setup steps" });
    await user.click(screen.getByRole("button", { name: "Run" }));
    expect(core.testRunApprove).toHaveBeenCalledWith("r1");
    expect(useSmokeTestsStore.getState().shown["w-app-fix-login"]).toBe("r1");
    expect(useSmokeTestsStore.getState().open["w-app-fix-login"]).toEqual(["r1"]);

    useSmokeTestsStore.setState({ runs: [waiting] });
    await user.click(await screen.findByRole("button", { name: "Don’t run" }));
    expect(core.testRunRefuse).toHaveBeenCalledWith("r1");
  });

  it("answers to Mod+Enter and Escape only while it has the keyboard", async () => {
    const user = userEvent.setup();
    render(<ApprovalCard workspaceId="w-app-fix-login" />);
    const card = await screen.findByRole("region", { name: "Run “The server answers”?" });
    await screen.findByRole("list", { name: "Setup steps" });
    // Keys pressed elsewhere do nothing to it.
    await user.keyboard("{Escape}");
    expect(core.testRunRefuse).not.toHaveBeenCalled();

    card.focus();
    await user.keyboard("{Control>}{Shift>}{Enter}{/Shift}{/Control}");
    expect(core.testRunApprove).toHaveBeenCalledWith("r1");
    useSmokeTestsStore.setState({ runs: [waiting] });
    (await screen.findByRole("region", { name: "Run “The server answers”?" })).focus();
    await user.keyboard("{Escape}");
    expect(core.testRunRefuse).toHaveBeenCalledWith("r1");
  });

  it("is not there when nothing waits", () => {
    useSmokeTestsStore.setState({ runs: [testRun("r3", { status: "running" })] });
    render(<ApprovalCard workspaceId="w-app-fix-login" />);
    expect(screen.queryByRole("region")).toBeNull();
  });
});
