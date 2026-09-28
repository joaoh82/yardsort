import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowCheck, WorkflowItem } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  workflowList: vi.fn(),
  workflowCheck: vi.fn(),
  workflowSave: vi.fn(),
  workflowCopy: vi.fn(),
  workflowRemove: vi.fn(),
  workflowRuns: vi.fn(),
  workflowRunSteps: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
const native = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@/lib/native", () => ({ native }));
// CodeMirror and React Flow need a real layout engine; stand-ins with the same props do here.
vi.mock("./WorkflowEditor", () => ({
  WorkflowEditor: (props: {
    text: string;
    onChange?: (text: string) => void;
    readOnly?: boolean;
  }) => (
    <textarea
      aria-label="Workflow file"
      defaultValue={props.text}
      readOnly={props.readOnly}
      onChange={(event) => props.onChange?.(event.target.value)}
    />
  ),
}));
vi.mock("./WorkflowChart", () => ({
  WorkflowChart: () => <div role="figure" aria-label="Steps" />,
}));

import { useProjectsStore } from "@/stores/projects";
import { NEW_WORKFLOW, useWorkflowStore } from "@/stores/workflows";
import { WorkflowView } from "./WorkflowView";

const TEXT = "id: demo\nname: Demo\n";

function item(id: string, source: WorkflowItem["source"], extra: Partial<WorkflowItem> = {}) {
  return {
    id,
    name: id === "code-review" ? "Request code review" : "Demo",
    description: null,
    source,
    problems: [],
    workflow: {
      id,
      name: "Demo",
      description: null,
      version: 1,
      trigger: { kind: "manual", context: "workspace" },
      inputs: [],
      steps: [
        {
          id: "tell",
          needs: [],
          action: "notify",
          title: "Hi",
          body: null,
        },
      ],
    },
    text: TEXT,
    activeRuns: 0,
    ...extra,
  } satisfies WorkflowItem;
}

const valid = (text: string): WorkflowCheck => ({
  workflow: item("demo", { kind: "builtIn" }).workflow,
  id: "demo",
  problems: text.includes("oops") ? [{ line: 2, column: 1, message: "Unknown field `oops`" }] : [],
});

describe("WorkflowView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.workflowCheck.mockImplementation(async (text: string) => valid(text));
    core.workflowRuns.mockResolvedValue([]);
    core.workflowList.mockResolvedValue([]);
    useWorkflowStore.setState({ items: [], loaded: true, runs: {}, steps: {}, drafts: {} });
    useProjectsStore.setState({ workflowId: null });
  });

  it("shows a built-in read-only, and customizing it copies it to the user's folder", async () => {
    useWorkflowStore.setState({ items: [item("code-review", { kind: "builtIn" })] });
    core.workflowList.mockResolvedValue([
      item("code-review", { kind: "file", path: "/wf/code-review.yaml", replacesBuiltIn: true }),
    ]);
    render(<WorkflowView workflowId="code-review" />);
    expect(screen.getByText(/Built in — customize it/)).toBeInTheDocument();
    expect(await screen.findByLabelText("Workflow file")).toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();

    await userEvent.setup().click(screen.getByRole("button", { name: "Customize" }));
    expect(core.workflowCopy).toHaveBeenCalledWith("code-review");
    expect(await screen.findByText(/Your copy, used instead of the built-in/)).toBeInTheDocument();
  });

  it("saves an edit to the user's own file and shows the core's problems as they are typed", async () => {
    const path = "/wf/demo.yaml";
    useWorkflowStore.setState({
      items: [item("demo", { kind: "file", path, replacesBuiltIn: false })],
    });
    core.workflowSave.mockResolvedValue("demo");
    core.workflowList.mockResolvedValue([
      item("demo", { kind: "file", path, replacesBuiltIn: false }),
    ]);
    const user = userEvent.setup();
    render(<WorkflowView workflowId="demo" />);

    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    const file = await screen.findByLabelText("Workflow file");
    await user.type(file, "oops: 1\n");
    expect(await screen.findByText("2:1 Unknown field `oops`")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run…" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(core.workflowSave).toHaveBeenCalledWith(path, `${TEXT}oops: 1\n`);
    await waitFor(() => expect(useWorkflowStore.getState().drafts.demo).toBeUndefined());
  });

  it("keeps unsaved text when the workflow is left and opened again", async () => {
    const path = "/wf/demo.yaml";
    useWorkflowStore.setState({
      items: [item("demo", { kind: "file", path, replacesBuiltIn: false })],
    });
    const user = userEvent.setup();
    const { unmount } = render(<WorkflowView workflowId="demo" />);
    await user.type(await screen.findByLabelText("Workflow file"), "# mine\n");
    unmount();
    render(<WorkflowView workflowId="demo" />);
    expect(await screen.findByLabelText("Workflow file")).toHaveValue(`${TEXT}# mine\n`);
    expect(screen.getByRole("button", { name: "Revert" })).toBeInTheDocument();
  });

  it("deletes the user's file only when they say yes", async () => {
    const path = "/wf/demo.yaml";
    useWorkflowStore.setState({
      items: [item("demo", { kind: "file", path, replacesBuiltIn: false })],
    });
    useProjectsStore.setState({ workflowId: "demo" });
    const user = userEvent.setup();
    render(<WorkflowView workflowId="demo" />);

    native.confirm.mockResolvedValue(false);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(native.confirm).toHaveBeenCalledTimes(1));
    expect(core.workflowRemove).not.toHaveBeenCalled();
    expect(useProjectsStore.getState().workflowId).toBe("demo");

    native.confirm.mockResolvedValue(true);
    core.workflowRemove.mockResolvedValue(undefined);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(core.workflowRemove).toHaveBeenCalledWith(path));
    expect(useProjectsStore.getState().workflowId).toBeNull();
  });

  it("offers Reset to built-in for a copy that replaces one, and stays on it after", async () => {
    const path = "/wf/code-review.yaml";
    useWorkflowStore.setState({
      items: [item("code-review", { kind: "file", path, replacesBuiltIn: true })],
    });
    useProjectsStore.setState({ workflowId: "code-review" });
    native.confirm.mockResolvedValue(true);
    core.workflowRemove.mockResolvedValue(undefined);
    render(<WorkflowView workflowId="code-review" />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Reset to built-in" }));
    await waitFor(() => expect(core.workflowRemove).toHaveBeenCalledWith(path));
    expect(native.confirm.mock.calls[0]![0]).toContain("The built-in is used again");
    expect(useProjectsStore.getState().workflowId).toBe("code-review");
  });

  it("starts a new workflow from a template, saved under the id it is given", async () => {
    useProjectsStore.setState({ workflowId: NEW_WORKFLOW });
    core.workflowSave.mockResolvedValue("my-workflow");
    render(<WorkflowView workflowId={NEW_WORKFLOW} />);
    const file = (await screen.findByLabelText("Workflow file")) as HTMLTextAreaElement;
    expect(file.value).toContain("id: my-workflow");
    expect(screen.queryByRole("region", { name: "Runs" })).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "Save" }));
    expect(core.workflowSave).toHaveBeenCalledWith(
      null,
      expect.stringContaining("id: my-workflow"),
    );
    await waitFor(() => expect(useProjectsStore.getState().workflowId).toBe("my-workflow"));
  });

  it("lists runs, and a run opens to its steps", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "failed",
        requestedBy: "cli",
        error: "Step `tell` failed: no way",
        inputs: {},
        pullRequest: null,
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "tell",
        action: "notify",
        status: "failed",
        startedAt: null,
        endedAt: null,
        outputs: {},
        note: "no way",
      },
    ]);
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    await userEvent
      .setup()
      .click(await within(runs).findByRole("button", { name: "Run in fix-login, failed" }));
    expect(await within(runs).findByText("no way")).toBeInTheDocument();
    expect(within(runs).getByText("Notify you")).toBeInTheDocument();
    expect(core.workflowRunSteps).toHaveBeenCalledWith("run-1");
  });
});
