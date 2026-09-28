import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowItem } from "@/lib/ipc";

const core = vi.hoisted(() => ({ workflowList: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useProjectsStore } from "@/stores/projects";
import { NEW_WORKFLOW, useWorkflowStore } from "@/stores/workflows";
import { WorkflowsSection } from "./WorkflowsSection";

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

describe("WorkflowsSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorkflowStore.setState({ items: [], loaded: false, drafts: {} });
    useProjectsStore.setState({ workflowId: null, selectedWorkspaceId: "w-app" });
    core.workflowList.mockResolvedValue([
      item("code-review", "Request code review", { activeRuns: 2 }),
      item("broken", "Broken", {
        source: { kind: "file", path: "/wf/broken.yaml", replacesBuiltIn: false },
        problems: [{ line: 1, column: 1, message: "x" }],
      }),
    ]);
  });

  it("lists every workflow with what is going on in it, and opens one in the center panel", async () => {
    const user = userEvent.setup();
    render(<WorkflowsSection />);
    const section = screen.getByRole("region", { name: "Workflows" });
    const review = await within(section).findByRole("button", { name: /Request code review/ });
    expect(within(review).getByLabelText("2 running")).toBeInTheDocument();
    const broken = within(section).getByRole("button", { name: /Broken/ });
    expect(within(broken).getByLabelText("has problems")).toBeInTheDocument();

    await user.click(review);
    expect(useProjectsStore.getState().workflowId).toBe("code-review");
    expect(review).toHaveAttribute("aria-current", "page");
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-app");
  });

  it("marks a workflow with unsaved changes, and starts a new one with +", async () => {
    useWorkflowStore.setState({ drafts: { broken: "id: broken\nname: B\n" } });
    const user = userEvent.setup();
    render(<WorkflowsSection />);
    const broken = await screen.findByRole("button", { name: /Broken/ });
    expect(within(broken).getByLabelText("unsaved changes")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New workflow" }));
    expect(useProjectsStore.getState().workflowId).toBe(NEW_WORKFLOW);
    expect(screen.getByText("New workflow", { selector: "li" })).toBeInTheDocument();
  });
});
