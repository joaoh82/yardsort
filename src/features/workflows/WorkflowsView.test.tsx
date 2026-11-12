import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowItem, WorkflowWait } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  workflowList: vi.fn(),
  workflowWaits: vi.fn(),
  workflowRemove: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));

import { useProjectsStore } from "@/stores/projects";
import { NEW_WORKFLOW, useWorkflowStore, WORKFLOW_LIST } from "@/stores/workflows";
import { WorkflowsRow } from "./WorkflowsRow";
import { WorkflowsView } from "./WorkflowsView";

const item = (id: string, name: string, extra: Partial<WorkflowItem> = {}): WorkflowItem => ({
  id,
  name,
  description: null,
  source: { kind: "builtIn" },
  problems: [],
  workflow: null,
  text: `id: ${id}\n`,
  activeRuns: 0,
  ...extra,
});

const mine = (id: string, name: string, replacesBuiltIn = false) =>
  item(id, name, { source: { kind: "file", path: `/wf/${id}.yaml`, replacesBuiltIn } });

const wait: WorkflowWait = {
  runId: "run-1",
  stepId: "sign-off",
  workflowId: "code-review",
  workflowName: "Request code review",
  workspaceName: "fix-login",
  kind: "approval",
  blockedOn: "your approval",
  actionNeeded: "Approve, reject, or request changes.",
  prompt: null,
  blockedLine: "Blocked: waiting for your approval.",
};

beforeEach(() => {
  vi.clearAllMocks();
  useWorkflowStore.setState({ items: [], waits: [], loaded: false, drafts: {}, error: null });
  useProjectsStore.setState({ workflowId: null, selectedWorkspaceId: "w-app" });
  core.workflowWaits.mockResolvedValue([]);
  core.workflowList.mockResolvedValue([
    item("code-review", "Request code review", { activeRuns: 2 }),
    mine("release", "Release notes"),
    mine("fix-checks", "Fix failing checks", true),
    item("broken", "Broken", {
      source: { kind: "file", path: "/wf/broken.yaml", replacesBuiltIn: false },
      problems: [{ line: 1, column: 1, message: "x" }],
    }),
  ]);
});

describe("WorkflowsRow", () => {
  it("opens the list in the center panel, stays lit while a workflow is open, and closes it", async () => {
    const user = userEvent.setup();
    render(<WorkflowsRow />);
    const row = screen.getByRole("button", { name: "Workflows" });
    expect(row).not.toHaveAttribute("aria-current");

    await user.click(row);
    expect(useProjectsStore.getState().workflowId).toBe(WORKFLOW_LIST);
    expect(row).toHaveAttribute("aria-current", "page");
    // The workspace is kept to go back to.
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-app");

    useProjectsStore.getState().openWorkflow("code-review");
    await waitFor(() => expect(row).toHaveAttribute("aria-current", "page"));
    // From a workflow, the row goes back to the list rather than closing.
    await user.click(row);
    expect(useProjectsStore.getState().workflowId).toBe(WORKFLOW_LIST);

    await user.click(row);
    expect(useProjectsStore.getState().workflowId).toBeNull();
  });

  it("counts the runs waiting on you", async () => {
    core.workflowWaits.mockResolvedValue([wait]);
    render(<WorkflowsRow />);
    expect(
      await screen.findByRole("button", { name: "Workflows, 1 run waiting on you" }),
    ).toBeInTheDocument();
  });
});

describe("WorkflowsView", () => {
  it("lists every workflow, saying which are built in and which are custom", async () => {
    render(<WorkflowsView />);
    const list = await screen.findByRole("list", { name: "Every workflow" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(4);

    const review = rows[0]!;
    expect(review).toHaveTextContent("Request code review");
    expect(review).toHaveTextContent("Built in");
    expect(within(review).getByLabelText("2 running")).toBeInTheDocument();
    // A built-in has no file to delete.
    expect(within(review).queryByRole("button", { name: /Delete|Reset/ })).toBeNull();

    expect(rows[1]).toHaveTextContent("Custom");
    expect(within(rows[1]!).getByRole("button", { name: "Delete Release notes" })).toBeEnabled();
    expect(rows[2]).toHaveTextContent("Custom, replaces the built-in");
    expect(
      within(rows[2]!).getByRole("button", { name: "Reset Fix failing checks to the built-in" }),
    ).toBeInTheDocument();
    expect(within(rows[3]!).getByLabelText("has problems")).toHaveTextContent("1 problem");
    expect(screen.getByText("1 built in · 3 custom")).toBeInTheDocument();
  });

  it("opens a workflow, or a new one, in the center panel", async () => {
    const user = userEvent.setup();
    render(<WorkflowsView />);
    await user.click(await screen.findByRole("button", { name: /Request code review/ }));
    expect(useProjectsStore.getState().workflowId).toBe("code-review");
    await user.click(screen.getByRole("button", { name: "New workflow…" }));
    expect(useProjectsStore.getState().workflowId).toBe(NEW_WORKFLOW);
  });

  it("deletes one of yours only when you say yes, and stays on the list", async () => {
    useProjectsStore.setState({ workflowId: WORKFLOW_LIST });
    const user = userEvent.setup();
    render(<WorkflowsView />);
    const remove = await screen.findByRole("button", { name: "Delete Release notes" });

    native.confirm.mockResolvedValue(false);
    await user.click(remove);
    await waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));
    expect(native.confirm.mock.calls[0]![0]).toContain("/wf/release.yaml");
    expect(core.workflowRemove).not.toHaveBeenCalled();

    native.confirm.mockResolvedValue(true);
    core.workflowRemove.mockResolvedValue(undefined);
    core.workflowList.mockResolvedValue([item("code-review", "Request code review")]);
    await user.click(remove);
    await waitFor(() => expect(core.workflowRemove).toHaveBeenCalledWith("/wf/release.yaml"));
    await waitFor(() => expect(screen.queryByText("Release notes")).toBeNull());
    expect(useProjectsStore.getState().workflowId).toBe(WORKFLOW_LIST);
  });

  it("resets a copy that replaces a built-in, saying the built-in comes back", async () => {
    native.confirm.mockResolvedValue(true);
    core.workflowRemove.mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<WorkflowsView />);
    await user.click(
      await screen.findByRole("button", { name: "Reset Fix failing checks to the built-in" }),
    );
    await waitFor(() => expect(core.workflowRemove).toHaveBeenCalledWith("/wf/fix-checks.yaml"));
    expect(native.confirm.mock.calls[0]![0]).toContain("The built-in is used again");
  });

  it("says why a delete failed", async () => {
    native.confirm.mockResolvedValue(true);
    core.workflowRemove.mockRejectedValue("permission denied");
    const user = userEvent.setup();
    render(<WorkflowsView />);
    await user.click(await screen.findByRole("button", { name: "Delete Release notes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("permission denied");
  });

  it("lists a run waiting on you and opens it in its workflow", async () => {
    core.workflowWaits.mockResolvedValue([wait]);
    const user = userEvent.setup();
    render(<WorkflowsView />);
    const waiting = await within(
      await screen.findByRole("list", { name: "Waiting on you" }),
    ).findByRole("button", { name: /Blocked: waiting for your approval/ });
    expect(waiting).toHaveTextContent("fix-login");
    await user.click(waiting);
    expect(useProjectsStore.getState().workflowId).toBe("code-review");
    expect(useWorkflowStore.getState().shownRun["code-review"]).toBe("run-1");
  });
});
