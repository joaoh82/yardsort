import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowItem } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  workflowList: vi.fn(),
  harnessList: vi.fn(),
  testList: vi.fn(),
  sessionsList: vi.fn(),
  triggerPresets: vi.fn(),
  triggerSave: vi.fn(),
  triggerTellAgents: vi.fn(),
  triggerList: vi.fn(),
  triggerStartedRuns: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { usePublishStore } from "@/stores/publish";
import { useSessionsStore } from "@/stores/sessions";
import { useTriggersStore } from "@/stores/triggers";
import { useWorkflowStore } from "@/stores/workflows";
import {
  harness,
  project,
  pullRequest,
  pullRequestsOf,
  record,
  trackedOf,
  trigger,
  triggerPresets,
  worktree,
} from "@/test/fixtures";
import { TriggerDialog } from "./TriggerDialog";

/** A built-in workflow with these inputs. */
const builtIn = (
  id: string,
  inputs: WorkflowItem["workflow"] extends infer W
    ? W extends { inputs: infer I }
      ? I
      : never
    : never = [],
): WorkflowItem => ({
  id,
  name: id,
  description: null,
  source: { kind: "builtIn" },
  problems: [],
  text: "",
  activeRuns: 0,
  workflow: {
    id,
    name: id,
    description: null,
    version: 1,
    trigger: { kind: "manual", context: "workspace" },
    inputs,
    steps: [],
  },
});

const fixLogin = worktree("app", "fix-login");

beforeEach(() => {
  vi.clearAllMocks();
  useTriggersStore.setState({
    items: [],
    loaded: true,
    history: {},
    started: [],
    presets: triggerPresets,
    error: null,
  });
  useWorkflowStore.setState({
    items: [
      builtIn("address-review"),
      builtIn("review-branch", [
        {
          id: "reviewer",
          kind: "harness",
          label: "Reviewer",
          required: true,
          default: null,
          options: [],
        },
      ]),
    ],
    loaded: true,
  });
  useHarnessStore.setState({ loaded: true, harnesses: [harness("claude"), harness("codex")] });
  useSessionsStore.setState({ byWorkspace: {} });
  usePublishStore.setState({ byProject: {} });
  useProjectsStore.setState({
    projects: [{ ...project("app"), workspaces: [...project("app").workspaces, fixLogin] }],
  });
  core.sessionsList.mockResolvedValue([]);
  core.testList.mockResolvedValue([]);
  core.triggerList.mockResolvedValue([]);
  core.triggerStartedRuns.mockResolvedValue([]);
});

describe("TriggerDialog", () => {
  it("adds a preset in a workspace, and says in words what it will do", async () => {
    const saved = trigger("t-new");
    core.triggerSave.mockResolvedValue({ trigger: saved, untold: [] });
    const onClose = vi.fn();
    const onSaved = vi.fn();
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={onClose} onSaved={onSaved} />);
    const add = screen.getByRole("button", { name: "Add trigger" });
    expect(add).toBeDisabled();

    await user.selectOptions(
      screen.getByRole("combobox", { name: "Start from" }),
      "address-reviews",
    );
    expect(screen.getByRole("checkbox", { name: "A review comes in" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Someone comments or replies" })).toBeChecked();
    expect(screen.getByRole("combobox", { name: "Workflow" })).toHaveValue("address-review");
    expect(screen.getByRole("spinbutton", { name: "Quiet time" })).toHaveValue(2);
    expect(screen.getByLabelText("What it will do")).toHaveTextContent(
      "When someone other than you reviews or comments on this workspace's pull request, 2 minutes after the last one, tell the agent to address it. At most 3 times per pull request. In app / fix-login.",
    );

    await waitFor(() => expect(add).toBeEnabled());
    await user.click(add);
    expect(core.triggerSave).toHaveBeenCalledWith({
      id: null,
      // Called what the preset is, not what its workflow is.
      name: "Address reviews",
      preset: "address-reviews",
      workspaceId: fixLogin.id,
      sessionId: null,
      events: ["pr.review", "pr.comment"],
      filters: {},
      target: "workflow",
      targetId: "address-review",
      inputs: {},
      quietMs: 120_000,
      cap: 3,
      notify: "failure",
    });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onSaved).toHaveBeenCalledWith(saved);
  });

  it("puts a trigger together from events and their filters, and sends only the ticked ones'", async () => {
    core.triggerSave.mockResolvedValue({ trigger: trigger("t-new"), untold: [] });
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={vi.fn()} />);

    await user.click(screen.getByRole("checkbox", { name: "Opens" }));
    // One trigger is about one kind of thing: an agent's events cannot join the pull request's.
    expect(screen.getByRole("checkbox", { name: "Finishes a turn" })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: /A draft counts as soon as it opens/ }));
    await user.click(screen.getByRole("checkbox", { name: "Opens" }));
    expect(screen.getByRole("checkbox", { name: "Finishes a turn" })).toBeEnabled();

    await user.click(screen.getByRole("checkbox", { name: "A review comes in" }));
    // Reviews come in bursts: two minutes, until it is changed.
    expect(screen.getByRole("spinbutton", { name: "Quiet time" })).toHaveValue(2);
    await user.selectOptions(screen.getByRole("combobox", { name: "Whose posts count" }), "anyone");
    await user.click(screen.getByRole("checkbox", { name: "Leave out bots" }));
    const states = screen.getByRole("group", { name: "Reviews that" });
    await user.click(within(states).getByRole("checkbox", { name: "Changes requested" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "address-review");
    const quiet = screen.getByRole("spinbutton", { name: "Quiet time" });
    await user.clear(quiet);
    await user.type(quiet, "45");
    await user.selectOptions(screen.getByRole("combobox", { name: "Quiet time unit" }), "seconds");
    const cap = screen.getByRole("spinbutton", { name: "At most" });
    await user.clear(cap);
    await user.type(cap, "1");
    expect(screen.getByLabelText("What it will do")).toHaveTextContent(
      "When anyone reviews this workspace's pull request (changes requested), leaving out bots, 45 seconds after the last one, tell the agent to address it. At most once per pull request.",
    );

    await user.click(screen.getByRole("button", { name: "Add trigger" }));
    expect(core.triggerSave).toHaveBeenCalledWith(
      expect.objectContaining({
        preset: null,
        events: ["pr.review"],
        // The draft filter went with its event.
        filters: { from: "anyone", states: ["changes_requested"], noBots: true },
        quietMs: 45_000,
        cap: 1,
      }),
    );
  });

  it("listens for custom events by name, offering the names the project already uses", async () => {
    core.triggerSave.mockResolvedValue({ trigger: trigger("t-new"), untold: [] });
    useTriggersStore.setState({
      items: [
        trigger("t-other", {
          events: ["custom"],
          filters: { names: ["deploy"] },
          projectId: fixLogin.projectId,
        }),
      ],
    });
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={vi.fn()} />);
    await user.click(screen.getByRole("checkbox", { name: "A custom event" }));
    // About the workspace, as `workspace.done` is: the pull request's events are another kind.
    expect(screen.getByRole("checkbox", { name: "Says its work is done" })).toBeEnabled();
    expect(screen.getByRole("checkbox", { name: "A review comes in" })).toBeDisabled();
    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "address-review");
    const add = screen.getByRole("button", { name: "Add trigger" });
    // Not without a name.
    expect(add).toBeDisabled();
    expect(screen.getByText(/Used in this project: deploy/)).toBeInTheDocument();

    const names = screen.getByRole("combobox", { name: "Custom event names" });
    await user.type(names, "Migration");
    expect(names).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText(/migration would do/)).toBeInTheDocument();
    expect(add).toBeDisabled();

    await user.clear(names);
    await user.type(names, "custom.migration-written, deploy");
    expect(screen.getByLabelText("What it will do")).toHaveTextContent(
      "When custom.migration-written or custom.deploy is raised here, tell the agent to address it. At most 3 times per workspace.",
    );
    await user.click(add);
    expect(core.triggerSave).toHaveBeenCalledWith(
      expect.objectContaining({
        events: ["custom"],
        filters: { names: ["migration-written", "deploy"] },
      }),
    );
  });

  it("offers to tell an agent already running about `ys workspace done`", async () => {
    core.sessionsList.mockResolvedValue([
      record("rec-1", { running: true, ptySessionId: "pty-1" }),
    ]);
    core.triggerSave.mockResolvedValue({
      trigger: trigger("t-new", { name: "Review when the work is done" }),
      untold: [{ sessionId: "rec-1", title: "fix the login bug" }],
    });
    core.triggerTellAgents.mockResolvedValue(1);
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={onClose} />);
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Start from" }),
      "review-when-done",
    );
    expect(screen.getByRole("combobox", { name: "Reviewer" })).toHaveValue("claude");
    await user.selectOptions(screen.getByRole("combobox", { name: "Reviewer" }), "codex");
    await user.click(screen.getByRole("button", { name: "Add trigger" }));
    expect(core.triggerSave).toHaveBeenCalledWith(
      expect.objectContaining({ targetId: "review-branch", inputs: { reviewer: "codex" } }),
    );

    const ask = await screen.findByRole("dialog", { name: "Tell the agent?" });
    expect(ask).toHaveTextContent("the agent “fix the login bug” in fix-login was already running");
    expect(onClose).not.toHaveBeenCalled();
    await user.click(within(ask).getByRole("button", { name: "Tell the agent" }));
    expect(core.triggerTellAgents).toHaveBeenCalledWith(["rec-1"]);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("offers a workflow about a pull request elsewhere for the pull request's events alone", async () => {
    const elsewhere = builtIn("code-review-elsewhere", [
      {
        id: "reviewer",
        kind: "harness",
        label: "Who reviews",
        required: true,
        default: null,
        options: [],
      },
    ]);
    elsewhere.workflow!.trigger = { kind: "manual", context: "tracked_pull_request" };
    useWorkflowStore.setState({
      items: [...useWorkflowStore.getState().items, elsewhere],
      loaded: true,
    });
    core.triggerSave.mockResolvedValue({ trigger: trigger("t-new"), untold: [] });
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={vi.fn()} />);
    const offered = () =>
      [...screen.getByRole("combobox", { name: "Workflow" }).querySelectorAll("option")].map(
        (option) => option.getAttribute("value"),
      );

    // Not for the workspace's events: such a run is always about one of those pull requests.
    await user.click(screen.getByRole("checkbox", { name: "Says its work is done" }));
    expect(offered()).not.toContain("code-review-elsewhere");
    await user.click(screen.getByRole("checkbox", { name: "Says its work is done" }));
    // Nor once it is merged: it is no longer open.
    await user.click(screen.getByRole("checkbox", { name: "It is merged" }));
    expect(offered()).not.toContain("code-review-elsewhere");
    await user.click(screen.getByRole("checkbox", { name: "It is merged" }));

    await user.click(screen.getByRole("checkbox", { name: "Opens" }));
    expect(offered()).toContain("address-review");
    expect(
      screen.getByRole("group", { name: "On a pull request it tracks elsewhere" }),
    ).toContainElement(screen.getByRole("option", { name: "code-review-elsewhere" }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Workflow" }),
      "code-review-elsewhere",
    );
    // Chosen, the workspace's events are not to be had.
    expect(screen.getByRole("checkbox", { name: "Says its work is done" })).toBeDisabled();
    expect(screen.getByRole("checkbox", { name: "A review comes in" })).toBeEnabled();
    // After these it is not open, and the core refuses them.
    expect(screen.getByRole("checkbox", { name: "It is merged" })).toBeDisabled();
    expect(screen.getByRole("checkbox", { name: "It is closed without merging" })).toBeDisabled();
    expect(
      screen.getByText(/When this workspace tracks a pull request elsewhere, or a draft of one/),
    ).toHaveTextContent("have claude review the pull request on GitHub");

    await user.click(screen.getByRole("button", { name: "Add trigger" }));
    expect(core.triggerSave).toHaveBeenCalledWith(
      expect.objectContaining({
        events: ["pr.opened"],
        targetId: "code-review-elsewhere",
        inputs: { reviewer: "claude" },
      }),
    );
  });

  it("says when the workspace's pull requests are elsewhere, and offers the workflow's twin", async () => {
    const reviewer = {
      id: "reviewer",
      kind: "harness" as const,
      label: "Who reviews",
      required: true,
      default: null,
      options: [],
    };
    const elsewhere = builtIn("code-review-elsewhere", [reviewer]);
    elsewhere.workflow!.trigger = { kind: "manual", context: "tracked_pull_request" };
    useWorkflowStore.setState({
      items: [builtIn("code-review", [reviewer]), elsewhere],
      loaded: true,
    });
    core.triggerSave.mockResolvedValue({ trigger: trigger("t-new"), untold: [] });
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={vi.fn()} />);
    await user.click(screen.getByRole("checkbox", { name: "Opens" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "code-review");
    await user.selectOptions(screen.getByRole("combobox", { name: "Who reviews" }), "codex");
    // Nothing tracked elsewhere: the workspace's own pull request is the one to hear.
    expect(screen.queryByRole("note")).toBeNull();

    // Another workspace's tracked pull request is not this one's.
    usePublishStore.setState({
      byProject: {
        [fixLogin.projectId]: pullRequestsOf([], {
          tracked: [
            trackedOf(fixLogin.id, pullRequest(41)),
            trackedOf(fixLogin.id, pullRequest(42)),
            trackedOf("w-app", pullRequest(43)),
          ],
        }),
      },
    });
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent(
      "fix-login tracks 2 pull requests in other repositories. This workflow is about the pull request from the workspace's own branch, so it never hears those.",
    );

    await user.click(within(note).getByRole("button", { name: "Use “code-review-elsewhere”" }));
    expect(screen.getByRole("combobox", { name: "Workflow" })).toHaveValue("code-review-elsewhere");
    expect(screen.queryByRole("note")).toBeNull();
    // The answer carries over: the twin asks the same question.
    expect(screen.getByRole("combobox", { name: "Who reviews" })).toHaveValue("codex");

    await user.click(screen.getByRole("button", { name: "Add trigger" }));
    expect(core.triggerSave).toHaveBeenCalledWith(
      expect.objectContaining({
        events: ["pr.opened"],
        targetId: "code-review-elsewhere",
        inputs: { reviewer: "codex" },
      }),
    );
  });

  it("says why the core refused it and stays open", async () => {
    core.triggerSave.mockRejectedValue({
      code: "trigger_variables",
      message: "`address-review` uses {{ event.summary }}, which `pr.opened` does not leave.",
    });
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<TriggerDialog workspaceId={fixLogin.id} onClose={onClose} />);
    await user.click(screen.getByRole("checkbox", { name: "Opens" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "address-review");
    await user.click(screen.getByRole("button", { name: "Add trigger" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("does not leave");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("changes a trigger, starting from what it is", async () => {
    const existing = trigger("t1", { name: "Mine", cap: 5, notify: "always" });
    core.triggerSave.mockResolvedValue({ trigger: existing, untold: [] });
    const user = userEvent.setup();
    render(<TriggerDialog trigger={existing} onClose={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Edit “Mine”" })).toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Start from" })).toBeNull();
    expect(screen.getByRole("combobox", { name: "Workspace" })).toHaveValue(fixLogin.id);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(core.triggerSave).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "t1",
        name: "Mine",
        preset: "address-reviews",
        cap: 5,
        notify: "always",
        quietMs: 120_000,
      }),
    );
  });
});
