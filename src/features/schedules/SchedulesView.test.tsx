import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  scheduleList: vi.fn(),
  scheduleRuns: vi.fn(),
  scheduleStartedRuns: vi.fn(),
  scheduleSetEnabled: vi.fn(),
  scheduleRemove: vi.fn(),
  scheduleRunNow: vi.fn(),
  // Selecting a workspace is remembered.
  uiStateSave: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useProjectsStore } from "@/stores/projects";
import { useSchedulesStore } from "@/stores/schedules";
import { useSmokeTestsStore } from "@/stores/smokeTests";
import { project, schedule, scheduleRun, worktree } from "@/test/fixtures";
import { SchedulesView } from "./SchedulesView";

const morning = schedule("s1");
const smoke = schedule("s2", {
  name: "Nightly smoke",
  target: "test",
  targetId: "login",
  when: "Cron `0 3 * * *`",
  standing: "awaiting_approval",
  last: scheduleRun("l9", { scheduleId: "s2", awaitingApproval: true }),
});

beforeEach(() => {
  vi.clearAllMocks();
  useSchedulesStore.setState({ items: [], loaded: false, history: {}, started: [], error: null });
  useSmokeTestsStore.setState({ open: {}, shown: {} });
  useProjectsStore.setState({
    schedulesOpen: true,
    selectedWorkspaceId: null,
    workflowId: null,
    projects: [
      {
        ...project("app"),
        workspaces: [...project("app").workspaces, worktree("app", "fix-login")],
      },
    ],
  });
  core.scheduleList.mockResolvedValue([morning, smoke]);
  core.scheduleStartedRuns.mockResolvedValue([]);
  core.scheduleRuns.mockResolvedValue([]);
  core.uiStateSave.mockResolvedValue(undefined);
});

/** The view with its list read, and `name`'s row opened. */
async function opened(name: string) {
  const user = userEvent.setup();
  render(<SchedulesView />);
  const list = await screen.findByRole("list", { name: "Schedules" });
  await user.click(within(list).getByRole("button", { name: new RegExp(name) }));
  return user;
}

describe("SchedulesView", () => {
  it("says what a schedule is when there are none", async () => {
    core.scheduleList.mockResolvedValue([]);
    render(<SchedulesView />);
    expect(await screen.findByText(/No schedules yet/)).toBeInTheDocument();
  });

  it("lists every schedule with what it runs, where, when and where it stands", async () => {
    render(<SchedulesView />);
    const list = await screen.findByRole("list", { name: "Schedules" });
    const rows = within(list).getAllByRole("button");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Morning ping");
    expect(rows[0]).toHaveTextContent("workflow ping · app / fix-login");
    expect(rows[0]).toHaveTextContent("Daily at 09:00");
    expect(rows[0]).toHaveTextContent("next ");
    expect(rows[0]).toHaveTextContent("on");
    expect(rows[1]).toHaveTextContent("smoke test login");
    expect(rows[1]).toHaveTextContent("waiting for approval");
    expect(rows[1]).toHaveTextContent("Last: waiting for approval");
  });

  it("opens a schedule to its history, and a line to the run it started", async () => {
    core.scheduleRuns.mockResolvedValue([
      scheduleRun("l1", { scheduleId: "s2", runId: "run-7", result: "failed" }),
      scheduleRun("l2", {
        scheduleId: "s2",
        outcome: "skipped",
        runId: null,
        note: "The previous run was still going.",
      }),
      scheduleRun("l3", { scheduleId: "s2", manual: true, result: "passed" }),
    ]);
    const user = await opened("Nightly smoke");
    const history = await screen.findByRole("region", { name: "History" });
    expect(core.scheduleRuns).toHaveBeenCalledWith("s2");
    const lines = await within(history).findAllByRole("listitem");
    expect(lines).toHaveLength(3);
    expect(lines[0]).toHaveTextContent("failed");
    expect(lines[1]).toHaveTextContent("skipped");
    expect(lines[1]).toHaveTextContent("The previous run was still going.");
    expect(within(lines[1]!).queryByRole("button")).toBeNull();
    expect(lines[2]).toHaveTextContent("Run now");
    expect(screen.getByText(/waiting for you to approve the test file/)).toBeInTheDocument();

    // A smoke test's run opens as a tab in its workspace.
    await user.click(within(lines[0]!).getByRole("button", { name: "Open run" }));
    expect(useProjectsStore.getState().selectedWorkspaceId).toBe("w-app-fix-login");
    expect(useSmokeTestsStore.getState().shown["w-app-fix-login"]).toBe("run-7");
  });

  it("switches a schedule off and on, and starts its run now", async () => {
    core.scheduleSetEnabled.mockResolvedValue({ ...morning, enabled: false });
    core.scheduleRunNow.mockResolvedValue(scheduleRun("l1"));
    const user = await opened("Morning ping");
    const onOff = screen.getByRole("switch", { name: "Morning ping is on" });
    expect(onOff).toBeChecked();
    await user.click(onOff);
    expect(core.scheduleSetEnabled).toHaveBeenCalledWith("s1", false);

    await user.click(screen.getByRole("button", { name: "Run now" }));
    expect(core.scheduleRunNow).toHaveBeenCalledWith("s1");
  });

  it("shows why a run could not be started now", async () => {
    core.scheduleRunNow.mockRejectedValue({
      code: "workspace_archived",
      message: "`fix-login` is archived. Restore it before running a workflow in it.",
    });
    const user = await opened("Morning ping");
    await user.click(screen.getByRole("button", { name: "Run now" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("`fix-login` is archived");
  });

  it("asks before removing, and keeps the schedule when the answer is no", async () => {
    const user = await opened("Morning ping");
    await user.click(screen.getByRole("button", { name: "Remove…" }));
    const asked = screen.getByRole("alertdialog", { name: "Remove this schedule?" });
    expect(asked).toHaveTextContent("The runs it started stay where they are.");
    await user.click(within(asked).getByRole("button", { name: "Keep it" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(core.scheduleRemove).not.toHaveBeenCalled();
    expect(screen.getByRole("heading", { name: "Morning ping" })).toBeInTheDocument();
  });

  it("removes the schedule when the answer is yes, and closes it", async () => {
    core.scheduleRemove.mockResolvedValue(undefined);
    const user = await opened("Morning ping");
    await user.click(screen.getByRole("button", { name: "Remove…" }));
    core.scheduleList.mockResolvedValue([smoke]);
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", { name: "Remove" }),
    );
    expect(core.scheduleRemove).toHaveBeenCalledWith("s1");
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Morning ping" })).toBeNull());
  });

  it("says a schedule whose workspace is gone has nowhere to run", async () => {
    core.scheduleList.mockResolvedValue([
      schedule("s3", {
        name: "Orphan",
        workspaceId: null,
        standing: "no_workspace",
        enabled: false,
      }),
    ]);
    await opened("Orphan");
    expect(screen.getByText(/was deleted, so it has nowhere to run/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run now" })).toBeDisabled();
  });

  it("closes from its own button", async () => {
    const user = userEvent.setup();
    render(<SchedulesView />);
    await user.click(screen.getByRole("button", { name: "Close schedules" }));
    expect(useProjectsStore.getState().schedulesOpen).toBe(false);
  });
});
