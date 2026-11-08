import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  triggerList: vi.fn(),
  triggerFires: vi.fn(),
  triggerStartedRuns: vi.fn(),
  triggerSetEnabled: vi.fn(),
  triggerReset: vi.fn(),
  triggerRemove: vi.fn(),
  // The dialog, when New trigger… is pressed.
  triggerPresets: vi.fn(),
  workflowList: vi.fn(),
  harnessList: vi.fn(),
  sessionsList: vi.fn(),
  // Selecting a workspace is remembered.
  uiStateSave: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useProjectsStore } from "@/stores/projects";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { useTriggersStore } from "@/stores/triggers";
import { useWorkflowStore } from "@/stores/workflows";
import { project, trigger, triggerFire, triggerPresets, worktree } from "@/test/fixtures";
import { TriggersView } from "./TriggersView";

const reviews = trigger("t1", {
  when: "When a review comes in on #142 or someone comments on #142, run the workflow `address-review`",
});
const smoke = trigger("t2", {
  name: "Smoke when done",
  preset: null,
  events: ["workspace.done"],
  target: "test",
  targetId: "login",
  quietMs: 0,
  when: "When an agent says its work is done, run the smoke test `login`",
  workspaceId: "w-app-other",
  workspaceName: "other",
  watch: null,
  standing: "capped",
  capped: true,
  last: triggerFire("f9", { triggerId: "t2", outcome: "capped", runId: null }),
});

beforeEach(() => {
  vi.clearAllMocks();
  useTriggersStore.setState({
    items: [],
    loaded: false,
    history: {},
    started: [],
    presets: triggerPresets,
    error: null,
  });
  useSmokeTestsStore.setState({ open: {}, shown: {} });
  useWorkflowStore.setState({ items: [], loaded: true });
  useProjectsStore.setState({
    triggersOpen: true,
    triggersFocus: null,
    selectedWorkspaceId: null,
    workflowId: null,
    projects: [
      {
        ...project("app"),
        workspaces: [
          ...project("app").workspaces,
          worktree("app", "fix-login"),
          worktree("app", "other"),
        ],
      },
    ],
  });
  core.triggerList.mockResolvedValue([reviews, smoke]);
  core.triggerStartedRuns.mockResolvedValue([]);
  core.triggerFires.mockResolvedValue([]);
  core.sessionsList.mockResolvedValue([]);
  core.uiStateSave.mockResolvedValue(undefined);
});

/** The view with its list read, and `name`'s row opened. */
async function opened(name: string) {
  const user = userEvent.setup();
  render(<TriggersView />);
  const list = await screen.findByRole("list", { name: "Triggers" });
  await user.click(within(list).getByRole("button", { name: new RegExp(name) }));
  return user;
}

describe("TriggersView", () => {
  it("says what a trigger is when there are none", async () => {
    core.triggerList.mockResolvedValue([]);
    render(<TriggersView />);
    expect(await screen.findByText(/No triggers yet/)).toBeInTheDocument();
  });

  it("lists every trigger with its event in words, where, what it runs and its badge", async () => {
    render(<TriggersView />);
    const list = await screen.findByRole("list", { name: "Triggers" });
    const rows = within(list).getAllByRole("button");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Address reviews");
    expect(rows[0]).toHaveTextContent("When a review comes in on #142");
    expect(rows[0]).toHaveTextContent("workflow address-review · app / fix-login");
    expect(rows[0]).toHaveTextContent("on");
    expect(rows[1]).toHaveTextContent("smoke test login · app / other");
    expect(rows[1]).toHaveTextContent("capped");
    expect(rows[1]).toHaveTextContent("Last: capped");
  });

  it("opens a trigger to its history, and a line to the run it started", async () => {
    core.triggerFires.mockResolvedValue([
      triggerFire("f1", { runId: "run-7", result: "succeeded" }),
      triggerFire("f2", {
        outcome: "skipped",
        runId: null,
        events: 1,
        event: "pr.comment",
        note: "No agent was running in `fix-login`.",
      }),
    ]);
    const user = await opened("Address reviews");
    const history = await screen.findByRole("region", { name: "History" });
    expect(core.triggerFires).toHaveBeenCalledWith("t1");
    const lines = await within(history).findAllByRole("listitem");
    expect(lines).toHaveLength(2);
    expect(lines[0]).toHaveTextContent("succeeded");
    expect(lines[0]).toHaveTextContent("pr.review ×2");
    expect(lines[0]).toHaveTextContent("1 review and 1 comment from bob");
    expect(lines[1]).toHaveTextContent("skipped");
    expect(lines[1]).toHaveTextContent("No agent was running");
    expect(within(lines[1]!).queryByRole("button")).toBeNull();
    expect(screen.getByText("Watching #142")).toBeInTheDocument();
    expect(screen.getByText("2 minutes after the last event")).toBeInTheDocument();
    expect(screen.getByText("3 runs per pull request")).toBeInTheDocument();

    // A workflow's run opens in the workflow's own view.
    await user.click(within(lines[0]!).getByRole("button", { name: "Open run" }));
    expect(useProjectsStore.getState().workflowId).toBe("address-review");
  });

  it("offers Reset on a capped trigger, and says why it stopped", async () => {
    core.triggerReset.mockResolvedValue({ ...smoke, standing: "listening", capped: false });
    const user = await opened("Smoke when done");
    expect(
      screen.getByText(/started all 3 of the runs it may for the workspace/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reset" }));
    expect(core.triggerReset).toHaveBeenCalledWith("t2");
    // Nor while the run it started before the cap is still going.
    core.triggerList.mockResolvedValue([reviews, { ...smoke, standing: "running" }]);
    await useTriggersStore.getState().load();
    expect(await screen.findByRole("button", { name: "Reset" })).toBeInTheDocument();
    // Nothing to reset on one that is not capped.
    const list = screen.getByRole("list", { name: "Triggers" });
    await user.click(within(list).getByRole("button", { name: /Address reviews/ }));
    expect(screen.queryByRole("button", { name: "Reset" })).toBeNull();
  });

  it("switches a trigger off, and shows why it could not be switched back on", async () => {
    core.triggerSetEnabled.mockResolvedValueOnce({ ...reviews, enabled: false });
    const user = await opened("Address reviews");
    const onOff = screen.getByRole("switch", { name: "Address reviews is on" });
    expect(onOff).toBeChecked();
    await user.click(onOff);
    expect(core.triggerSetEnabled).toHaveBeenCalledWith("t1", false);

    core.triggerSetEnabled.mockRejectedValueOnce({
      code: "workspace_not_found",
      message: "`fix-login` was deleted, so this has nothing to listen to.",
    });
    await user.click(onOff);
    expect(await screen.findByRole("alert")).toHaveTextContent("nothing to listen to");
  });

  it("asks before removing, keeps it on a no, and removes it on a yes", async () => {
    core.triggerRemove.mockResolvedValue(undefined);
    const user = await opened("Address reviews");
    await user.click(screen.getByRole("button", { name: "Remove…" }));
    const asked = screen.getByRole("alertdialog", { name: "Remove this trigger?" });
    expect(asked).toHaveTextContent("The runs it started stay where they are.");
    await user.click(within(asked).getByRole("button", { name: "Keep it" }));
    expect(core.triggerRemove).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Remove…" }));
    core.triggerList.mockResolvedValue([smoke]);
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", { name: "Remove" }),
    );
    expect(core.triggerRemove).toHaveBeenCalledWith("t1");
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Address reviews" })).toBeNull(),
    );
  });

  it("opened on a workspace, shows its triggers, and a new one starts there", async () => {
    useProjectsStore.setState({ triggersFocus: { workspaceId: "w-app-other" } });
    const user = userEvent.setup();
    render(<TriggersView />);
    const list = await screen.findByRole("list", { name: "Triggers" });
    expect(within(list).getAllByRole("button")).toHaveLength(1);
    expect(within(list).getByRole("button")).toHaveTextContent("Smoke when done");
    expect(screen.getByText("In other")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New trigger…" }));
    const dialog = await screen.findByRole("dialog", { name: "New trigger" });
    expect(within(dialog).getByRole("combobox", { name: "Workspace" })).toHaveValue("w-app-other");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await user.click(screen.getByRole("button", { name: "Show all" }));
    expect(within(list).getAllByRole("button")).toHaveLength(2);
  });

  it("calls a session trigger whose agent ended ended", async () => {
    core.triggerList.mockResolvedValue([
      trigger("t3", {
        name: "Settled",
        events: ["agent.settled"],
        sessionId: "rec-1",
        sessionTitle: "fix the login bug",
        sessionRunning: false,
        watch: null,
      }),
    ]);
    await opened("Settled");
    expect(screen.getAllByText("ended").length).toBeGreaterThan(0);
    expect(screen.getByText(/The agent it listens to has ended/)).toBeInTheDocument();
    expect(screen.getByText("fix the login bug's tab, in fix-login")).toBeInTheDocument();
  });

  it("closes from its own button", async () => {
    const user = userEvent.setup();
    render(<TriggersView />);
    await user.click(screen.getByRole("button", { name: "Close triggers" }));
    expect(useProjectsStore.getState().triggersOpen).toBe(false);
  });
});
