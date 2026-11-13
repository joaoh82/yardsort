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
  workflowContinue: vi.fn(),
  workflowRunSteps: vi.fn(),
  workflowDecide: vi.fn(),
  workflowAnswer: vi.fn(),
  workflowShowApprovalCode: vi.fn(),
  workflowWaits: vi.fn(),
  workflowWriterStatus: vi.fn(),
  workflowDescribe: vi.fn(),
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
    selection?: { id: string | null } | null;
    stepLines?: Record<string, number>;
  }) => (
    <textarea
      aria-label="Workflow file"
      data-selected-step={props.selection?.id}
      data-line={props.stepLines?.[props.selection?.id ?? ""]}
      value={props.text}
      readOnly={props.readOnly}
      onChange={(event) => props.onChange?.(event.target.value)}
    />
  ),
}));
vi.mock("./WorkflowChart", () => ({
  WorkflowChart: ({
    selectedStep,
    onSelect,
  }: {
    selectedStep: string | null;
    onSelect: (id: string) => void;
  }) => (
    <button aria-pressed={selectedStep === "tell"} onClick={() => onSelect("tell")}>
      Select step tell
    </button>
  ),
}));

// The Run dialog has tests of its own; here it only reports which workflow it started.
vi.mock("./RunWorkflowDialog", () => ({
  RunWorkflowDialog: ({
    onStarted,
  }: {
    onStarted?: (runId: string, workflowId: string) => void;
  }) => (
    <>
      <button onClick={() => onStarted?.("run-8", "code-review")}>Started it</button>
      <button onClick={() => onStarted?.("run-9", "code-review-elsewhere")}>
        Started its twin
      </button>
    </>
  ),
}));

import { useProjectsStore } from "@/stores/projects";
import { useSchedulesStore } from "@/stores/schedules";
import { useTriggersStore } from "@/stores/triggers";
import { NEW_WORKFLOW, useWorkflowStore, WORKFLOW_LIST } from "@/stores/workflows";
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
          gate: false,
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
  stepLines: { tell: 2 },
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
    core.workflowWaits.mockResolvedValue([]);
    core.workflowWriterStatus.mockResolvedValue({
      harnessId: null,
      status: {
        enabled: true,
        available: false,
        harness: null,
        key: false,
        model: "claude-opus-5",
        problem: "No agent here can write one.",
      },
    });
    useWorkflowStore.setState({
      items: [],
      loaded: true,
      runs: {},
      steps: {},
      drafts: {},
      shownRun: {},
    });
    useProjectsStore.setState({ workflowId: null });
    useSchedulesStore.setState({ started: [] });
    useTriggersStore.setState({ started: [] });
  });

  it("shows a built-in read-only, and customizing it copies it to the user's folder", async () => {
    useWorkflowStore.setState({ items: [item("code-review", { kind: "builtIn" })] });
    core.workflowList.mockResolvedValue([
      item("code-review", { kind: "file", path: "/wf/code-review.yaml", replacesBuiltIn: true }),
    ]);
    render(<WorkflowView workflowId="code-review" />);
    expect(screen.getByText(/Built in — customize it/)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Describe it" })).toBeNull();
    expect(await screen.findByLabelText("Workflow file")).toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();

    await userEvent.setup().click(screen.getByRole("button", { name: "Customize" }));
    expect(core.workflowCopy).toHaveBeenCalledWith("code-review");
    expect(await screen.findByText(/your copy, used instead of the built-in/)).toBeInTheDocument();
  });

  it("customizing opens a box to describe the change, and the agent's answer is the copy's draft", async () => {
    const path = "/wf/code-review.yaml";
    const able = {
      enabled: true,
      available: true,
      harness: "Claude Code",
      key: false,
      model: "claude-opus-5",
      problem: null,
    };
    core.workflowWriterStatus.mockResolvedValue({ harnessId: null, status: able });
    useWorkflowStore.setState({ items: [item("code-review", { kind: "builtIn" })] });
    core.workflowList.mockResolvedValue([
      item("code-review", { kind: "file", path, replacesBuiltIn: true }),
    ]);
    const changed = `${TEXT}description: Runs the tests first.\n`;
    core.workflowDescribe.mockResolvedValue({
      text: changed,
      problems: [],
      tries: 1,
      writer: "Claude Code",
    });
    const user = userEvent.setup();
    render(<WorkflowView workflowId="code-review" />);
    expect(screen.queryByRole("region", { name: "Describe a change" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Customize" }));
    const box = await screen.findByRole("region", { name: "Describe a change" });
    expect(screen.getByRole("button", { name: "Describe a change" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    const words = within(box).getByRole("textbox", { name: "What to change" });
    await waitFor(() => expect(words).toBeEnabled());
    await user.type(words, "Run the tests first.");
    await user.click(within(box).getByRole("button", { name: "Change it" }));
    expect(core.workflowDescribe).toHaveBeenCalledWith("Run the tests first.", TEXT);
    await waitFor(() => expect(useWorkflowStore.getState().drafts["code-review"]).toBe(changed));
    expect(await within(box).findByText(/Changed by Claude Code/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Revert" })).toBeInTheDocument();
    expect(core.workflowSave).not.toHaveBeenCalled();

    // The same button closes it; a file of the user's opens it the same way.
    await user.click(screen.getByRole("button", { name: "Describe a change" }));
    expect(screen.queryByRole("region", { name: "Describe a change" })).toBeNull();
  });

  it("selects a chart node and passes its current YAML location to the editor", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    render(<WorkflowView workflowId="demo" />);
    const step = await screen.findByRole("button", { name: "Select step tell" });
    await userEvent.setup().click(step);
    expect(step).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByLabelText("Workflow file")).toHaveAttribute("data-selected-step", "tell");
    expect(screen.getByLabelText("Workflow file")).toHaveAttribute("data-line", "2");
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

  it("keeps what is typed while a save is still being written", async () => {
    const path = "/wf/demo.yaml";
    useWorkflowStore.setState({
      items: [item("demo", { kind: "file", path, replacesBuiltIn: false })],
    });
    let finish: (id: string) => void = () => {};
    core.workflowSave.mockReturnValue(new Promise<string>((resolve) => (finish = resolve)));
    core.workflowList.mockResolvedValue([
      item("demo", { kind: "file", path, replacesBuiltIn: false }, { text: `${TEXT}# one\n` }),
    ]);
    const user = userEvent.setup();
    render(<WorkflowView workflowId="demo" />);
    const file = await screen.findByLabelText("Workflow file");
    await user.type(file, "# one\n");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(core.workflowSave).toHaveBeenCalledWith(path, `${TEXT}# one\n`);
    await user.type(file, "# two\n");
    finish("demo");
    await waitFor(() => expect(core.workflowList).toHaveBeenCalled());
    expect(screen.getByLabelText("Workflow file")).toHaveValue(`${TEXT}# one\n# two\n`);
    await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeEnabled());
    expect(useWorkflowStore.getState().drafts.demo).toBe(`${TEXT}# one\n# two\n`);
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
    await user.click(screen.getByRole("button", { name: "Revert" }));
    expect(screen.getByLabelText("Workflow file")).toHaveValue(TEXT);
    expect(screen.queryByRole("button", { name: "Revert" })).toBeNull();
    expect(useWorkflowStore.getState().drafts.demo).toBeUndefined();
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
    await waitFor(() => expect(useProjectsStore.getState().workflowId).toBe(WORKFLOW_LIST));
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
    expect(screen.getByRole("region", { name: "Describe it" })).toBeInTheDocument();
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
        pullRequestsMade: [],
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
        outcome: null,
        artifacts: [
          {
            name: "spec",
            source: "file",
            detail: "SPEC.md",
            body: "Be kind.",
            truncated: false,
          },
        ],
        consumed: [],
      },
    ]);
    // A schedule started this run: the list says so, and the open run says which.
    useSchedulesStore.setState({
      started: [{ runId: "run-1", scheduleId: "s1", scheduleName: "Nightly" }],
    });
    // And a trigger's run is marked with the trigger's name.
    useTriggersStore.setState({
      started: [{ runId: "run-1", triggerId: "t1", triggerName: "Address reviews" }],
    });
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    const row = await within(runs).findByRole("button", { name: "Run in fix-login, failed" });
    expect(row).toHaveTextContent("scheduled");
    expect(row).toHaveTextContent("triggered");
    await userEvent.setup().click(row);
    expect(await within(runs).findByText("no way")).toHaveClass("whitespace-pre-wrap");
    expect(within(runs).getByText("Step `tell` failed: no way")).toHaveClass("whitespace-pre-wrap");
    // The failed run opens on the step that failed, beside the list of its steps.
    const rail = within(runs).getByRole("list", { name: "Steps of this run" });
    expect(within(rail).getByText("Notify you")).toBeInTheDocument();
    expect(within(runs).getByRole("region", { name: "Step tell" })).toBeInTheDocument();
    expect(within(runs).getByText("Artifact spec")).toBeInTheDocument();
    expect(within(runs).getByText("Be kind.")).toBeInTheDocument();
    expect(within(runs).getByText("Scheduled · Nightly")).toBeInTheDocument();
    expect(within(runs).getByText("Triggered · Address reviews")).toBeInTheDocument();
    expect(core.workflowRunSteps).toHaveBeenCalledWith("run-1");
  });

  it("continues a failed run from the step that failed, and offers it for no other run", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    const run = {
      id: "run-1",
      workflowId: "demo",
      workflowName: "Demo",
      workspaceId: "w",
      workspaceName: "fix-login",
      status: "failed",
      requestedBy: "cli",
      error: "Step `verify` failed: The test run did not end within 30m.",
      inputs: {},
      pullRequest: null,
      pullRequestsMade: [],
      createdAt: Date.now(),
      startedAt: null,
      endedAt: null,
    };
    core.workflowRuns.mockResolvedValue([run]);
    core.workflowRunSteps.mockResolvedValue([]);
    // The window reloads the list too, and the workflow is still in it.
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    core.workflowContinue.mockResolvedValue(undefined);
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    const user = userEvent.setup();
    await user.click(await within(runs).findByRole("button", { name: "Run in fix-login, failed" }));
    expect(within(runs).queryByRole("button", { name: "Cancel run" })).not.toBeInTheDocument();
    // It sits right under the error, ahead of the steps, which can run long.
    const carry = within(runs).getByRole("button", { name: "Continue from the failed step" });
    const failure = within(runs).getByText(
      "Step `verify` failed: The test run did not end within 30m.",
      { selector: "p" },
    );
    expect(failure.nextElementSibling).toBe(carry);

    // The app answers with the run going again, and the window shows what it is told.
    core.workflowRuns.mockResolvedValue([{ ...run, status: "running", error: null }]);
    await user.click(within(runs).getByRole("button", { name: "Continue from the failed step" }));
    expect(core.workflowContinue).toHaveBeenCalledWith("run-1");
    expect(
      await within(runs).findByRole("button", { name: "Run in fix-login, running" }),
    ).toBeInTheDocument();
    expect(
      within(runs).queryByRole("button", { name: "Continue from the failed step" }),
    ).not.toBeInTheDocument();
    expect(within(runs).getByRole("button", { name: "Cancel run" })).toBeInTheDocument();
  });

  it("opens on Definition unless the newest run needs looking at, and switches views", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    const run = {
      id: "run-1",
      workflowId: "demo",
      workflowName: "Demo",
      workspaceId: "w",
      workspaceName: "fix-login",
      status: "succeeded",
      requestedBy: "app",
      error: null,
      inputs: { task: "Fix the login" },
      pullRequest: null,
      pullRequestsMade: [],
      createdAt: Date.now(),
      startedAt: null,
      endedAt: null,
    };
    core.workflowRuns.mockResolvedValue([run]);
    core.workflowRunSteps.mockResolvedValue([]);
    render(<WorkflowView workflowId="demo" />);
    const views = screen.getByRole("group", { name: "View" });
    await waitFor(() =>
      expect(within(views).getByRole("button", { name: "Definition" })).toHaveAttribute(
        "aria-pressed",
        "true",
      ),
    );
    expect(screen.queryByRole("region", { name: "Runs" })).toBeNull();
    expect(await screen.findByLabelText("Workflow file")).toBeInTheDocument();
    expect(screen.getByText("Show how the run in fix-login went")).toBeInTheDocument();

    await userEvent.setup().click(within(views).getByRole("button", { name: "Runs" }));
    const runs = screen.getByRole("region", { name: "Runs" });
    // The newest run is on screen without picking it, open on its summary.
    const summary = within(runs).getByRole("region", { name: "Run summary" });
    expect(within(summary).getByText("Fix the login")).toBeInTheDocument();
    expect(within(summary).getByText("run-1")).toBeInTheDocument();
    expect(screen.queryByLabelText("Workflow file")).toBeNull();
  });

  it("lists the pull requests a run opened on its summary, and counts them on its row", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "running",
        requestedBy: "app",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [
          {
            repo: null,
            number: 12,
            url: "https://github.com/o/app/pull/12",
            title: "Analytics on the public site",
            state: "open",
            draft: false,
            openedAt: Date.now(),
          },
          {
            repo: "o/cloud",
            number: 41,
            url: "https://github.com/o/cloud/pull/41",
            title: null,
            state: null,
            draft: false,
            openedAt: Date.now(),
          },
        ],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([]);
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    expect(
      await within(runs).findByRole("button", { name: "Run in fix-login, running" }),
    ).toHaveTextContent("2 PRs");
    const made = within(runs).getByRole("region", { name: "Pull requests" });
    expect(within(made).getByText("Analytics on the public site")).toBeInTheDocument();
    expect(within(made).getByText("#12")).toBeInTheDocument();
    expect(within(made).getByText("o/cloud#41")).toBeInTheDocument();
    expect(within(made).getAllByRole("button", { name: "Open on GitHub" })).toHaveLength(2);
    // Only the project's own can be found in the Pull requests view.
    expect(within(made).getAllByRole("button", { name: "Pull requests view" })).toHaveLength(1);
  });

  it("folds a long run's succeeded steps until asked to show them", async () => {
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
        error: "Step `verify` failed: it did not end.",
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
      },
    ]);
    const step = (stepId: string, status: string) => ({
      stepId,
      action: "notify",
      status,
      startedAt: null,
      endedAt: null,
      outputs: {},
      note: null,
      outcome: null,
      artifacts: [],
      consumed: [],
    });
    core.workflowRunSteps.mockResolvedValue([
      ...["one", "two", "three", "four", "five"].map((id) => step(id, "succeeded")),
      step("verify", "failed"),
      step("after", "skipped"),
    ]);
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    const rail = within(runs).getByRole("list", { name: "Steps of this run" });
    const fold = await within(rail).findByRole("button", { name: /5 steps succeeded/ });
    expect(within(rail).queryByText("one")).toBeNull();
    expect(within(rail).getByText("verify")).toBeInTheDocument();
    await userEvent.setup().click(fold);
    expect(within(rail).getByText("one")).toBeInTheDocument();
    expect(within(rail).queryByRole("button", { name: /steps succeeded/ })).toBeNull();
  });

  it("says why a failed run could not continue", async () => {
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
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([]);
    // The window reloads the list too, and the workflow is still in it.
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    core.workflowContinue.mockRejectedValue({
      code: "workflow_already_running",
      message:
        "`demo` is already running in `fix-login` (run 3f2a9c1e). Wait for it, or cancel it.",
    });
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    const user = userEvent.setup();
    await user.click(await within(runs).findByRole("button", { name: "Run in fix-login, failed" }));
    await user.click(within(runs).getByRole("button", { name: "Continue from the failed step" }));
    expect(await within(runs).findByRole("alert")).toHaveTextContent("already running in");
  });

  it("opens the page of the workflow a run started as, when that is its twin elsewhere", async () => {
    useWorkflowStore.setState({ items: [item("code-review", { kind: "builtIn" })] });
    const user = userEvent.setup();
    render(<WorkflowView workflowId="code-review" />);
    await user.click(await screen.findByRole("button", { name: "Run…" }));
    await user.click(screen.getByRole("button", { name: "Started its twin" }));
    expect(useProjectsStore.getState().workflowId).toBe("code-review-elsewhere");
    expect(useWorkflowStore.getState().shownRun["code-review-elsewhere"]).toBe("run-9");
    expect(core.workflowRuns).toHaveBeenCalledWith("code-review-elsewhere", expect.anything());
  });

  it("stays on its own page for a run of itself", async () => {
    useWorkflowStore.setState({ items: [item("code-review", { kind: "builtIn" })] });
    const user = userEvent.setup();
    render(<WorkflowView workflowId="code-review" />);
    await user.click(await screen.findByRole("button", { name: "Run…" }));
    await user.click(screen.getByRole("button", { name: "Started it" }));
    expect(useProjectsStore.getState().workflowId).toBeNull();
    expect(core.workflowRuns).toHaveBeenCalledWith("code-review", expect.anything());
  });

  it("opens on the run asked for from elsewhere, once", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "succeeded",
        requestedBy: "trigger",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "tell",
        action: "notify",
        status: "succeeded",
        startedAt: null,
        endedAt: null,
        outputs: {},
        note: "told it",
        outcome: null,
        artifacts: [],
        consumed: [],
      },
    ]);
    // Open run, on a trigger's history.
    useWorkflowStore.getState().showRun("demo", "run-1");
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    // A run that went well opens on its summary; its steps are a click away.
    expect(await within(runs).findByRole("region", { name: "Run summary" })).toBeInTheDocument();
    const rail = within(runs).getByRole("list", { name: "Steps of this run" });
    await userEvent.setup().click(await within(rail).findByRole("button", { name: /Notify you/ }));
    expect(await within(runs).findByText("told it")).toBeInTheDocument();
    expect(core.workflowRunSteps).toHaveBeenCalledWith("run-1");
    // Taken: the view opened from the sidebar next time starts with no run picked.
    expect(useWorkflowStore.getState().shownRun.demo).toBeUndefined();
  });

  it("shows a pending approval and records the decision", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "running",
        requestedBy: "app",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
        blockedOn: "your approval",
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "sign-off",
        action: "approve",
        status: "waiting",
        startedAt: null,
        endedAt: null,
        outputs: { prompt: "Ship this plan?" },
        note: "Waiting for your approval.",
        outcome: null,
        artifacts: [],
        consumed: [
          {
            name: "plan",
            source: "output",
            detail: "brief",
            body: "Split the migration.",
            truncated: false,
          },
        ],
        approvalCode: "ABCD2345",
        approvalLocked: false,
      },
    ]);
    core.workflowDecide.mockResolvedValue(undefined);
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    await userEvent
      .setup()
      .click(await within(runs).findByRole("button", { name: "Run in fix-login, running" }));
    expect(
      (await within(runs).findAllByText("Blocked: waiting for your approval.")).length,
    ).toBeGreaterThan(0);
    expect(within(runs).getByText(/ABCD2345/)).toBeInTheDocument();
    expect(within(runs).getByText("To review: plan")).toBeInTheDocument();
    core.workflowShowApprovalCode.mockResolvedValue("ZXCV2345");
    await userEvent.setup().click(within(runs).getByRole("button", { name: "Show a new code" }));
    await waitFor(() =>
      expect(core.workflowShowApprovalCode).toHaveBeenCalledWith("run-1", "sign-off"),
    );
    const review = within(runs).getByRole("button", { name: "Review plan and decide" });
    await waitFor(() => expect(review).toBeEnabled());
    await userEvent.setup().click(review);
    const dialog = screen.getByRole("dialog", { name: "Your approval: sign-off" });
    expect(within(dialog).getByText("Split the migration.")).toBeInTheDocument();
    expect(within(dialog).getByText("Waiting for your approval.")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Request changes" })).toBeDisabled();
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Approve" }));
    expect(core.workflowDecide).toHaveBeenCalledWith("run-1", "sign-off", "approved", null);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("shows a plan formatted in the approval dialog, and keeps the note when it is closed", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "running",
        requestedBy: "app",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
        blockedOn: "your approval",
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "approve-plan",
        action: "approve",
        status: "waiting",
        startedAt: null,
        endedAt: null,
        outputs: {},
        note: "Read the plan.",
        outcome: null,
        artifacts: [],
        consumed: [
          {
            name: "plan",
            source: "file",
            detail: "factory/plan.md",
            body: "# Plan: cookieless analytics\n\nAdd it to the public site.",
            truncated: false,
          },
          {
            name: "context",
            source: "file",
            detail: "factory/context.md",
            body: "It is on main.",
            truncated: false,
          },
        ],
        approvalCode: null,
        approvalLocked: false,
        clarificationPrompt: null,
        clarificationQuestions: [],
      },
    ]);
    core.workflowDecide.mockResolvedValue(undefined);
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    const user = userEvent.setup();
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    await user.click(
      await within(runs).findByRole("button", { name: "Run in fix-login, running" }),
    );
    await user.click(within(runs).getByRole("button", { name: "Review and decide" }));
    let dialog = screen.getByRole("dialog", { name: "Your approval: approve-plan" });
    // A Markdown file is shown formatted, and the raw text is one click away.
    expect(
      within(dialog).getByRole("heading", { name: "Plan: cookieless analytics" }),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Show plain text" }));
    expect(within(dialog).getByText(/# Plan: cookieless analytics/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("tab", { name: "context" }));
    expect(within(dialog).getByText("It is on main.")).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("Feedback for approve-plan"), "Split step 2");
    await user.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(within(runs).getByText("Note written, not sent")).toBeInTheDocument();
    await user.click(within(runs).getByRole("button", { name: "Review and decide" }));
    dialog = screen.getByRole("dialog", { name: "Your approval: approve-plan" });
    await user.click(within(dialog).getByRole("button", { name: "Request changes" }));
    expect(core.workflowDecide).toHaveBeenCalledWith(
      "run-1",
      "approve-plan",
      "changes_requested",
      "Split step 2",
    );
  });

  it("shows a clarification and records the answer", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "running",
        requestedBy: "app",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
        blockedOn: "your answer",
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "discover",
        action: "wait_session",
        status: "waiting",
        startedAt: null,
        endedAt: null,
        outputs: {},
        note: null,
        outcome: null,
        artifacts: [],
        consumed: [],
        approvalCode: null,
        approvalLocked: false,
        clarificationPrompt: "1. What should the timeout be?",
        clarificationQuestions: ["What should the timeout be?"],
      },
    ]);
    core.workflowAnswer.mockResolvedValue(undefined);
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    await userEvent
      .setup()
      .click(await within(runs).findByRole("button", { name: "Run in fix-login, running" }));
    expect(within(runs).getAllByText("Blocked: waiting for your answer.").length).toBeGreaterThan(
      0,
    );
    expect(within(runs).getByText("1. What should the timeout be?")).toBeInTheDocument();
    expect(
      within(runs).getByText(
        "The code is not in this window yet. Show a new code to use a terminal.",
      ),
    ).toBeInTheDocument();
    core.workflowShowApprovalCode.mockResolvedValue(undefined);
    await userEvent.setup().click(within(runs).getByRole("button", { name: "Show a new code" }));
    expect(core.workflowShowApprovalCode).toHaveBeenCalledWith("run-1", "discover");
    await userEvent
      .setup()
      .click(within(runs).getByRole("button", { name: "Answer the question" }));
    const dialog = screen.getByRole("dialog", { name: "The agent has a question" });
    const send = within(dialog).getByRole("button", { name: "Send answer" });
    expect(send).toBeDisabled();
    await userEvent.setup().type(within(dialog).getByLabelText("Your answer"), "30 seconds");
    await userEvent.setup().click(send);
    expect(core.workflowAnswer).toHaveBeenCalledWith("run-1", "discover", "30 seconds");
  });

  it("takes several questions one at a time and sends them as one numbered answer", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "running",
        requestedBy: "app",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
        blockedOn: "your answer",
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "discover",
        action: "wait_session",
        status: "waiting",
        startedAt: null,
        endedAt: null,
        outputs: {},
        note: null,
        outcome: null,
        artifacts: [],
        consumed: [],
        approvalCode: null,
        approvalLocked: false,
        clarificationPrompt: "1. Where should it run?\n2. Which users?\n3. Any deadline?",
        clarificationQuestions: ["Where should it run?", "Which users?", "Any deadline?"],
      },
    ]);
    core.workflowAnswer.mockResolvedValue(undefined);
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    const user = userEvent.setup();
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    await user.click(
      await within(runs).findByRole("button", { name: "Run in fix-login, running" }),
    );
    await user.click(within(runs).getByRole("button", { name: "Answer 3 questions" }));
    let dialog = screen.getByRole("dialog", { name: "The agent has 3 questions" });
    expect(within(dialog).getByText("Question 1 of 3")).toBeInTheDocument();
    expect(within(dialog).getByText("Where should it run?")).toBeInTheDocument();
    expect(within(dialog).queryByText("Which users?")).not.toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("Answer to question 1"), "Both");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));

    // Closing keeps what was typed; opening again starts at the first blank question.
    await user.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(within(runs).getByText("1 of 3 answered, not sent")).toBeInTheDocument();
    await user.click(within(runs).getByRole("button", { name: "Continue answering" }));
    dialog = screen.getByRole("dialog", { name: "The agent has 3 questions" });
    expect(within(dialog).getByText("Question 2 of 3")).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("Answer to question 2"), "Agency users only");
    await user.click(within(dialog).getByRole("button", { name: "Next" }));
    // The last one is left blank.
    await user.click(within(dialog).getByRole("button", { name: "Review" }));
    const review = within(dialog).getByRole("list", { name: "Your answers" });
    expect(within(review).getByText("No answer")).toBeInTheDocument();
    expect(core.workflowAnswer).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Send answers" }));
    expect(core.workflowAnswer).toHaveBeenCalledWith(
      "run-1",
      "discover",
      "1. Both\n\n2. Agency users only\n\n3. (no answer)",
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("approves a gate that reads no documents from the panel, without the dialog", async () => {
    useWorkflowStore.setState({ items: [item("demo", { kind: "builtIn" })] });
    core.workflowRuns.mockResolvedValue([
      {
        id: "run-1",
        workflowId: "demo",
        workflowName: "Demo",
        workspaceId: "w",
        workspaceName: "fix-login",
        status: "running",
        requestedBy: "app",
        error: null,
        inputs: {},
        pullRequest: null,
        pullRequestsMade: [],
        createdAt: Date.now(),
        startedAt: null,
        endedAt: null,
        blockedOn: "your approval",
      },
    ]);
    core.workflowRunSteps.mockResolvedValue([
      {
        stepId: "release",
        action: "approve",
        status: "waiting",
        startedAt: null,
        endedAt: null,
        outputs: {},
        note: "Ready to release?",
        outcome: null,
        artifacts: [],
        consumed: [],
        approvalCode: null,
        approvalLocked: false,
        clarificationPrompt: null,
        clarificationQuestions: [],
      },
    ]);
    core.workflowDecide.mockResolvedValue(undefined);
    core.workflowList.mockResolvedValue([item("demo", { kind: "builtIn" })]);
    const user = userEvent.setup();
    render(<WorkflowView workflowId="demo" />);
    const runs = await screen.findByRole("region", { name: "Runs" });
    await user.click(
      await within(runs).findByRole("button", { name: "Run in fix-login, running" }),
    );
    expect(within(runs).queryByText(/To review/)).not.toBeInTheDocument();
    expect(within(runs).queryByRole("button", { name: /Review/ })).not.toBeInTheDocument();
    await user.click(within(runs).getByRole("button", { name: "Approve" }));
    expect(core.workflowDecide).toHaveBeenCalledWith("run-1", "release", "approved", null);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // A note, and so a request for changes, still goes through the dialog.
    await user.click(within(runs).getByRole("button", { name: "Add a note…" }));
    const dialog = screen.getByRole("dialog", { name: "Your approval: release" });
    expect(within(dialog).getByText("This approval reads no documents.")).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("Feedback for release"), "Wait for the docs");
    await user.click(within(dialog).getByRole("button", { name: "Request changes" }));
    expect(core.workflowDecide).toHaveBeenLastCalledWith(
      "run-1",
      "release",
      "changes_requested",
      "Wait for the docs",
    );
  });
});
