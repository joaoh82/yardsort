import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SchedulePreview, TestItem, WorkflowItem } from "@/lib/ipc";

const core = vi.hoisted(() => ({
  workflowList: vi.fn(),
  harnessList: vi.fn(),
  testList: vi.fn(),
  testReview: vi.fn(),
  schedulePreview: vi.fn(),
  scheduleSave: vi.fn(),
  scheduleList: vi.fn(),
  scheduleStartedRuns: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useSchedulesStore } from "@/stores/schedules";
import { useWorkflowStore } from "@/stores/workflows";
import { harness, project, schedule, testReview, worktree } from "@/test/fixtures";
import { ScheduleDialog } from "./ScheduleDialog";

const ping: WorkflowItem = {
  id: "ping",
  name: "Ping",
  description: null,
  source: { kind: "builtIn" },
  problems: [],
  text: "",
  activeRuns: 0,
  workflow: {
    id: "ping",
    name: "Ping",
    description: null,
    version: 1,
    trigger: { kind: "manual", context: "workspace" },
    inputs: [
      { id: "who", kind: "harness", label: "Who", required: true, default: null, options: [] },
      { id: "note", kind: "text", label: "A note", required: false, default: null, options: [] },
    ],
    steps: [],
  },
};

const login: TestItem = {
  id: "login",
  name: "Log in",
  description: null,
  path: "/worktrees/app/fix-login/.yardsort/tests/login.yaml",
  text: "id: login\n",
  problems: [],
  runnable: true,
  usesBrowser: false,
  browserMissing: false,
  inputs: [
    { id: "user", kind: "text", label: "User", required: true, default: "demo", options: [] },
  ],
};

const fixLogin = worktree("app", "fix-login");
const fine = (when: string): SchedulePreview => ({
  problem: null,
  when,
  next: [new Date(2026, 10, 6, 9, 0).getTime(), new Date(2026, 10, 7, 9, 0).getTime()],
});

beforeEach(() => {
  vi.clearAllMocks();
  useSchedulesStore.setState({ items: [], loaded: true, history: {}, started: [], error: null });
  useWorkflowStore.setState({ items: [ping], loaded: true });
  useHarnessStore.setState({ loaded: true, harnesses: [harness("claude"), harness("codex")] });
  useProjectsStore.setState({
    projects: [{ ...project("app"), workspaces: [...project("app").workspaces, fixLogin] }],
  });
  core.testList.mockResolvedValue([login]);
  core.testReview.mockResolvedValue(testReview({ contentHash: "hash-1" }));
  core.schedulePreview.mockResolvedValue(fine("Daily at 09:00"));
  core.scheduleList.mockResolvedValue([]);
  core.scheduleStartedRuns.mockResolvedValue([]);
});

describe("ScheduleDialog", () => {
  it("adds a workflow schedule with the answers, the time and the rules chosen", async () => {
    const saved = schedule("s-new");
    core.scheduleSave.mockResolvedValue(saved);
    const onClose = vi.fn();
    const onSaved = vi.fn();
    const user = userEvent.setup();
    render(<ScheduleDialog onClose={onClose} onSaved={onSaved} />);
    const add = screen.getByRole("button", { name: "Add schedule" });
    expect(add).toBeDisabled();

    await user.selectOptions(screen.getByRole("combobox", { name: "Workspace" }), fixLogin.id);
    await user.selectOptions(screen.getByRole("combobox", { name: "Workflow" }), "ping");
    // A required agent starts on the first one installed: what shows is what is saved.
    expect(screen.getByRole("combobox", { name: "Who" })).toHaveValue("claude");
    await user.type(screen.getByRole("textbox", { name: "A note" }), "hello");

    // It opens on a weekday morning; the core says what that comes to.
    const times = await screen.findByRole("list", { name: "Next times" });
    expect(within(times).getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByText("Daily at 09:00")).toBeInTheDocument();
    expect(core.schedulePreview).toHaveBeenLastCalledWith({
      kind: "daily",
      time: { hour: 9, minute: 0 },
    });

    await user.selectOptions(screen.getByRole("combobox", { name: "If a time is missed" }), "skip");
    await user.selectOptions(screen.getByRole("combobox", { name: "Tell me" }), "always");
    await user.type(screen.getByRole("textbox", { name: "Name" }), "Morning ping");
    await waitFor(() => expect(add).toBeEnabled());
    await user.click(add);

    expect(core.scheduleSave).toHaveBeenCalledWith({
      id: null,
      name: "Morning ping",
      target: "workflow",
      targetId: "ping",
      workspaceId: fixLogin.id,
      inputs: { who: "claude", note: "hello" },
      spec: { kind: "daily", time: { hour: 9, minute: 0 } },
      missed: "skip",
      notify: "always",
      contentHash: null,
    });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onSaved).toHaveBeenCalledWith(saved);
  });

  it("shows what is wrong with a cron expression and will not save it", async () => {
    core.schedulePreview.mockResolvedValue({
      problem: "`0 9 * *` has 4 fields; a cron expression here has five.",
      when: "Cron `0 9 * *`",
      next: [],
    });
    const user = userEvent.setup();
    render(
      <ScheduleDialog
        target="workflow"
        targetId="ping"
        workspaceId={fixLogin.id}
        onClose={vi.fn()}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox", { name: "How often" }), "cron");
    const expr = screen.getByRole("textbox", { name: "Cron expression" });
    await user.clear(expr);
    await user.type(expr, "0 9 * *");
    expect(await screen.findByRole("alert")).toHaveTextContent("has 4 fields");
    expect(core.schedulePreview).toHaveBeenLastCalledWith({ kind: "cron", expr: "0 9 * *" });
    expect(screen.queryByRole("list", { name: "Next times" })).toBeNull();
    expect(screen.getByRole("button", { name: "Add schedule" })).toBeDisabled();
  });

  it("shows a smoke test's steps, and saving approves the file it showed", async () => {
    core.scheduleSave.mockResolvedValue(schedule("s-new", { target: "test", targetId: "login" }));
    const user = userEvent.setup();
    render(
      <ScheduleDialog target="test" targetId="login" workspaceId={fixLogin.id} onClose={vi.fn()} />,
    );
    const steps = await screen.findByRole("region", { name: "What it does" });
    expect(steps).toHaveTextContent("GET http://localhost:{{ run.port }}/health");
    expect(steps).toHaveTextContent("Saving approves this file");
    expect(core.testReview).toHaveBeenCalledWith(fixLogin.id, "login");
    expect(screen.getByRole("textbox", { name: "User" })).toHaveValue("demo");

    const add = screen.getByRole("button", { name: "Add schedule" });
    await waitFor(() => expect(add).toBeEnabled());
    await user.click(add);
    expect(core.scheduleSave).toHaveBeenCalledWith(
      expect.objectContaining({
        target: "test",
        targetId: "login",
        inputs: { user: "demo" },
        contentHash: "hash-1",
      }),
    );
  });

  it("reads weekly days in week order, whatever order they were ticked in", async () => {
    core.schedulePreview.mockResolvedValue(fine("Mon, Wed at 09:00"));
    const user = userEvent.setup();
    render(
      <ScheduleDialog
        target="workflow"
        targetId="ping"
        workspaceId={fixLogin.id}
        onClose={vi.fn()}
      />,
    );
    await user.selectOptions(screen.getByRole("combobox", { name: "How often" }), "weekly");
    const days = screen.getByRole("group", { name: "Days" });
    // It starts on weekdays. Untick three, then tick Sunday.
    for (const day of ["Tue", "Thu", "Fri"]) {
      await user.click(within(days).getByRole("button", { name: day }));
    }
    await user.click(within(days).getByRole("button", { name: "Sun" }));
    expect(within(days).getByRole("button", { name: "Mon" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await waitFor(() =>
      expect(core.schedulePreview).toHaveBeenLastCalledWith({
        kind: "weekly",
        days: ["mon", "wed", "sun"],
        time: { hour: 9, minute: 0 },
      }),
    );
  });

  it("opens an existing schedule on what it holds, and saves it as the same schedule", async () => {
    const existing = schedule("s1", {
      name: "Hourly ping",
      inputs: { who: "codex", note: "hi" },
      spec: { kind: "every", minutes: 120 },
      when: "Every 2 hours",
      missed: "skip",
      notify: "never",
    });
    core.schedulePreview.mockResolvedValue(fine("Every 2 hours"));
    core.scheduleSave.mockResolvedValue(existing);
    const user = userEvent.setup();
    render(<ScheduleDialog schedule={existing} onClose={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Edit “Hourly ping”" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Workspace" })).toHaveValue(fixLogin.id);
    expect(screen.getByRole("combobox", { name: "Workflow" })).toHaveValue("ping");
    expect(screen.getByRole("combobox", { name: "Who" })).toHaveValue("codex");
    expect(screen.getByRole("combobox", { name: "How often" })).toHaveValue("every");
    expect(screen.getByRole("spinbutton", { name: "Every how many" })).toHaveValue(2);
    expect(screen.getByRole("combobox", { name: "Unit" })).toHaveValue("hours");
    expect(screen.getByRole("combobox", { name: "If a time is missed" })).toHaveValue("skip");
    expect(screen.getByRole("combobox", { name: "Tell me" })).toHaveValue("never");

    const save = screen.getByRole("button", { name: "Save" });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    expect(core.scheduleSave).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "s1",
        name: "Hourly ping",
        inputs: { who: "codex", note: "hi" },
        spec: { kind: "every", minutes: 120 },
      }),
    );
  });

  it("keeps the dialog open and says why when the core refuses", async () => {
    core.scheduleSave.mockRejectedValue({
      code: "schedule_needs_person",
      message: "`login` has a step a person does (`look`), so it cannot run on a schedule.",
    });
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(
      <ScheduleDialog target="test" targetId="login" workspaceId={fixLogin.id} onClose={onClose} />,
    );
    const add = screen.getByRole("button", { name: "Add schedule" });
    await waitFor(() => expect(add).toBeEnabled());
    await user.click(add);
    expect(await screen.findByRole("alert")).toHaveTextContent("a step a person does");
    expect(onClose).not.toHaveBeenCalled();
    expect(add).toBeEnabled();
  });
});
