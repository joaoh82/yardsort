import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  notificationList: vi.fn(),
  notificationSeen: vi.fn(),
  notificationSeenTerminal: vi.fn(),
  notificationSeenAll: vi.fn(),
  uiStateSave: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useNotificationsStore } from "@/stores/notifications";
import { useProjectsStore } from "@/stores/projects";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { useWorkflowStore } from "@/stores/workflows";
import { notification, standingNotification } from "@/test/fixtures";
import { NotificationsView } from "./NotificationsView";

const tab = (id: string): TerminalTab => ({
  id,
  workspaceId: "w-app-fix-login",
  title: "claude",
  exit: null,
  recordId: `session-${id}`,
  busy: false,
  attention: false,
  toldAbout: null,
  toldAt: null,
  busySince: null,
});

const asks = notification("asks", { command: "git push --force-with-lease" });
const finished = notification("finished", {
  kind: "agent.finished",
  part: "finished",
  workspaceId: "w-app-local",
  workspaceName: "local",
  branch: "main",
  workedMs: 12 * 60_000,
  said: "Renamed the cache keys. 14 tests pass.",
});
const stopped = notification("stopped", {
  kind: "agent.stopped",
  part: "finished",
  harnessId: "codex",
  exitCode: 1,
  terminalId: null,
  projectId: "p-site",
  projectName: "site",
  workspaceId: "w-site-cache",
  workspaceName: "cache-forecasts",
});
const seen = notification("seen", {
  kind: "agent.finished",
  part: "earlier",
  settledAt: Date.now() - 3_600_000,
  at: Date.now() - 7_200_000,
});
const all = [asks, finished, stopped, seen];

function show(items = all) {
  core.notificationList.mockResolvedValue(items);
  useNotificationsStore.setState({ items, loaded: true });
  return render(<NotificationsView />);
}

beforeEach(() => {
  vi.clearAllMocks();
  core.notificationSeen.mockResolvedValue(null);
  core.notificationSeenAll.mockResolvedValue(null);
  core.uiStateSave.mockResolvedValue(null);
  useNotificationsStore.setState({
    items: [],
    loaded: false,
    error: null,
    grouping: "kind",
    inFront: null,
  });
  useProjectsStore.setState({ notificationsOpen: true, selectedWorkspaceId: null });
  useTerminalStore.setState({ tabs: [tab("pty-asks"), tab("pty-finished")], active: {} });
});

describe("NotificationsView", () => {
  it("lists what needs you, what finished unseen, and what left, each row in its words", () => {
    show();
    const needs = screen.getByRole("region", { name: "Needs you" });
    expect(within(needs).getByRole("listitem")).toHaveTextContent(
      "fix-loginappys/fix-login· Fix the login formclaude asks for a permissiongit push --force-with-lease",
    );
    expect(within(needs).getByRole("img", { name: "needs you" })).toBeInTheDocument();

    const done = screen.getByRole("region", { name: "Finished, not seen" });
    const rows = within(done).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(within(done).getByText("claude finished after 12 min")).toBeInTheDocument();
    expect(within(done).getByText("“Renamed the cache keys. 14 tests pass.”")).toBeInTheDocument();
    expect(within(done).getByText("codex stopped with an error")).toBeInTheDocument();
    expect(within(done).getByText("exit 1")).toBeInTheDocument();
    expect(within(done).getByRole("img", { name: "went badly, not seen yet" })).toBeInTheDocument();

    const earlier = screen.getByRole("region", { name: "Earlier" });
    expect(within(earlier).getByRole("img", { name: "seen" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "All, 3" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Needs you, 1" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Finished, 2" })).toBeInTheDocument();
  });

  it("opens the agent's terminal from a row, and leaves marking it seen to looking at it", async () => {
    const user = userEvent.setup();
    show();
    await user.click(
      screen.getByRole("button", { name: "local in app: claude finished after 12 min" }),
    );
    const projects = useProjectsStore.getState();
    expect(projects.selectedWorkspaceId).toBe("w-app-local");
    expect(projects.notificationsOpen).toBe(false);
    expect(useTerminalStore.getState().active["w-app-fix-login"]).toBe("pty-finished");
    expect(core.notificationSeen).not.toHaveBeenCalled();
  });

  it("opens the workspace of an agent whose process is gone", async () => {
    const user = userEvent.setup();
    show();
    await user.click(
      screen.getByRole("button", {
        name: "cache-forecasts in site: codex stopped with an error",
      }),
    );
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-site-cache");
    expect(useTerminalStore.getState().active).toEqual({});
  });

  it("marks one finished row seen, or all of them, without opening anything", async () => {
    const user = userEvent.setup();
    show();
    const done = screen.getByRole("region", { name: "Finished, not seen" });
    expect(within(done).getAllByRole("button", { name: "Mark seen" })).toHaveLength(2);
    // A question has no such button: it leaves when it is answered.
    const needs = screen.getByRole("region", { name: "Needs you" });
    expect(within(needs).queryByRole("button", { name: "Mark seen" })).toBeNull();

    await user.click(within(done).getAllByRole("button", { name: "Mark seen" })[0]!);
    expect(core.notificationSeen).toHaveBeenCalledTimes(1);
    expect(useProjectsStore.getState().notificationsOpen).toBe(true);
    expect(screen.getByRole("tab", { name: "Finished, 1" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Mark all seen" }));
    expect(core.notificationSeenAll).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("region", { name: "Finished, not seen" })).toBeNull();
    expect(screen.getByRole("button", { name: "Mark all seen" })).toBeDisabled();
  });

  it("groups by project when asked, and remembers the choice", async () => {
    const user = userEvent.setup();
    show();
    await user.click(screen.getByRole("radio", { name: "Project" }));
    expect(core.uiStateSave).toHaveBeenCalledWith("notifications.grouping", "project");
    const app = screen.getByRole("region", { name: "app" });
    expect(within(app).getAllByRole("listitem")).toHaveLength(2);
    expect(within(app).getByText("1 needs you · 1 finished")).toBeInTheDocument();
    expect(
      within(screen.getByRole("region", { name: "site" })).getAllByRole("listitem"),
    ).toHaveLength(1);
    expect(screen.getByRole("region", { name: "Earlier" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Project" })).toBeChecked();
  });

  it("shows one part of the list when its tab is chosen", async () => {
    const user = userEvent.setup();
    show();
    await user.click(screen.getByRole("tab", { name: "Earlier" }));
    expect(screen.getAllByRole("region").map((r) => r.getAttribute("aria-label"))).toEqual([
      "Notifications",
      "Earlier",
    ]);
    await user.click(screen.getByRole("tab", { name: "Needs you, 1" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("says so when nothing waits, and still shows what left", () => {
    show([seen]);
    expect(
      screen.getByText("Nothing needs you, and nothing finished while you were away."),
    ).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Earlier" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "All" })).toBeInTheDocument();
  });

  it("lists a workflow, a smoke test, a schedule and a trigger beside the agents", () => {
    show([
      standingNotification("question", {
        kind: "workflow.question",
        said: "Which zones should the fix cover?",
        // The newer of the two that wait, so the first of them.
        at: Date.now() - 30_000,
      }),
      standingNotification("failed", {
        kind: "workflow.failed",
        part: "finished",
        note: "The agent's terminal closed.",
      }),
      standingNotification("lost", {
        kind: "schedule.noWorkspace",
        title: "Nightly review",
        workspaceId: null,
        branch: null,
        at: Date.now() - 120_000,
      }),
    ]);
    const needs = screen.getByRole("region", { name: "Needs you" });
    const [question, lost] = within(needs).getAllByRole("listitem");
    expect(question).toHaveTextContent(
      "fix-loginappys/fix-login· ReviewWorkflow asks you a question“Which zones should the fix cover?”",
    );
    // With no workspace, the schedule's own name leads, and the one it lost follows.
    expect(lost).toHaveTextContent(
      "Nightly reviewapp· was in fix-loginSchedule has no workspace to run in",
    );
    // Nothing that waits can be marked seen: it leaves when it is dealt with.
    expect(within(needs).queryByRole("button", { name: "Mark seen" })).toBeNull();

    const done = screen.getByRole("region", { name: "Finished, not seen" });
    expect(within(done).getByRole("listitem")).toHaveTextContent(
      "fix-loginappys/fix-login· ReviewWorkflow failedThe agent's terminal closed.",
    );
    expect(within(done).getByRole("img", { name: "went badly, not seen yet" })).toBeInTheDocument();
    expect(within(done).getByRole("button", { name: "Mark seen" })).toBeInTheDocument();
  });

  it("opens a workflow's run from its row", async () => {
    const user = userEvent.setup();
    show([standingNotification("approval")]);
    await user.click(
      screen.getByRole("button", { name: "fix-login in app: Workflow waits for your approval" }),
    );
    expect(useProjectsStore.getState().workflowId).toBe("review");
    expect(useProjectsStore.getState().notificationsOpen).toBe(false);
    expect(useWorkflowStore.getState().shownRun.review).toBe("run-approval");
  });

  it("opens the workspace of a smoke test that waits, where its file is approved", async () => {
    const user = userEvent.setup();
    show([standingNotification("test", { kind: "test.waits", title: "Half-hour zones" })]);
    await user.click(
      screen.getByRole("button", {
        name: "fix-login in app: Smoke test waits for you to approve its file",
      }),
    );
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-app-fix-login");
    expect(useProjectsStore.getState().notificationsOpen).toBe(false);
  });

  it("opens Schedules and Triggers from their rows", async () => {
    const user = userEvent.setup();
    show([
      standingNotification("refused", { kind: "schedule.refused", title: "Nightly review" }),
      standingNotification("capped", { kind: "trigger.capped", title: "Review when done" }),
    ]);
    await user.click(
      screen.getByRole("button", { name: "fix-login in app: Schedule could not start its run" }),
    );
    expect(useProjectsStore.getState().schedulesOpen).toBe(true);
    expect(useProjectsStore.getState().notificationsOpen).toBe(false);

    useProjectsStore.getState().openNotifications(true);
    await user.click(
      await screen.findByRole("button", {
        name: "fix-login in app: Trigger reached its limit of runs",
      }),
    );
    const projects = useProjectsStore.getState();
    expect(projects.triggersOpen).toBe(true);
    // On its workspace's triggers, so the one that is held is easy to find.
    expect(projects.triggersFocus).toEqual({ workspaceId: "w-app-fix-login" });
  });

  it("closes from its header", async () => {
    const user = userEvent.setup();
    show();
    await user.click(screen.getByRole("button", { name: "Close notifications" }));
    expect(useProjectsStore.getState().notificationsOpen).toBe(false);
  });
});
