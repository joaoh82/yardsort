import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { harness, project, worktree } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  ptySpawn: vi.fn(),
  activityTimeline: vi.fn(),
  onActivityChanged: vi.fn(),
  projectAutomationGet: vi.fn(),
  workspaceRun: vi.fn(),
  ptyList: vi.fn(),
  sessionsList: vi.fn(),
  onHostEvent: vi.fn(),
  onSessionStarted: vi.fn(),
  appInfo: vi.fn(),
  harnessesList: vi.fn(),
  uiStateSave: vi.fn(),
  workspaceHandoff: vi.fn(),
  memoryGet: vi.fn(async () => ({ projectId: "p", shared: false, preview: null, entries: [] })),
  conversationContext: vi.fn(),
  conversationsUnasked: vi.fn(),
  conversationsShare: vi.fn(),
  sessionContext: vi.fn(async () => null),
  outcomesAgents: vi.fn(async () => []),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));
// The empty state never mounts a terminal, and xterm needs a real layout engine.
vi.mock("@/features/terminal/TerminalView", () => ({
  TerminalView: () => <div data-testid="terminal" />,
}));

import { useAppStore } from "@/stores/app";
import { useHarnessStore } from "@/stores/harnesses";
import { useProjectsStore } from "@/stores/projects";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalStore } from "@/stores/terminals";
import { WorkspacePanel } from "./WorkspacePanel";

const alpha = project("alpha");
const local = alpha.workspaces[0]!;
const feature = worktree("alpha", "feature");

beforeEach(() => {
  vi.clearAllMocks();
  core.ptyList.mockResolvedValue([]);
  core.sessionsList.mockResolvedValue([]);
  core.onHostEvent.mockResolvedValue(() => {});
  core.onSessionStarted.mockResolvedValue(() => {});
  core.uiStateSave.mockResolvedValue(undefined);
  core.conversationContext.mockResolvedValue(null);
  core.conversationsUnasked.mockResolvedValue(null);
  core.conversationsShare.mockResolvedValue({});
  core.ptySpawn.mockImplementation(async ({ workspaceId }: { workspaceId: string }) => ({
    id: `s-${workspaceId}`,
    program: "/bin/bash",
    args: [],
    cwd: null,
    pid: 1,
    size: { cols: 80, rows: 24 },
    labels: { workspace: workspaceId },
    state: { status: "running" as const },
    hasOutput: true,
    busy: false,
    idleMs: 0,
  }));
  useSessionsStore.setState({ byWorkspace: {}, error: null });
  useTerminalStore.setState({ tabs: [], active: {}, error: null });
  useHarnessStore.setState({ harnesses: [harness("claude")], loaded: true });
  useAppStore.setState({ showTimeline: false });
  core.activityTimeline.mockResolvedValue({ events: [], hasMore: false });
  core.onActivityChanged.mockResolvedValue(() => {});
});

describe("the activity timeline", () => {
  it("is offered only when the setting says so, and opens from the footer", async () => {
    const user = userEvent.setup();
    useProjectsStore.setState({
      projects: [{ ...alpha, workspaces: [local, feature] }],
      selectedWorkspaceId: feature.id,
    });
    const { rerender } = render(<WorkspacePanel />);
    expect(screen.queryByRole("button", { name: "Activity" })).toBeNull();

    useAppStore.setState({ showTimeline: true });
    rerender(<WorkspacePanel />);
    await user.click(screen.getByRole("button", { name: "Activity" }));
    const panel = await screen.findByRole("region", { name: "Activity" });
    expect(core.activityTimeline).toHaveBeenCalledWith(feature.id, null, 50);
    await user.click(within(panel).getByRole("button", { name: "Close activity" }));
    expect(screen.queryByRole("region", { name: "Activity" })).toBeNull();
  });
});

/** Put `workspace` on screen with nothing running in it. */
function show(workspaceId: string) {
  useProjectsStore.setState({
    projects: [{ ...alpha, workspaces: [local, feature] }],
    loaded: true,
    selectedWorkspaceId: workspaceId,
    composingProjectId: null,
    composingWorkspaceId: null,
    collapsed: [],
    error: null,
    notice: null,
  } as never);
  render(<WorkspacePanel />);
}

describe("an empty workspace", () => {
  // `local` is the project's own checkout: a shell is only one of the things you might want
  // there, so the panel lists them rather than picking one.
  it("local offers what to open, with the app's mark above", async () => {
    show(local.id);

    expect(screen.getByRole("button", { name: /Open Terminal/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Open Composer/ })).toBeInTheDocument();
    // Not the harness row a worktree gets — that starts an agent without asking what about.
    expect(screen.queryByText("Nothing running in this workspace.")).not.toBeInTheDocument();
  });

  it("Open Terminal starts a shell in the checkout", async () => {
    const user = userEvent.setup();
    show(local.id);

    await user.click(screen.getByRole("button", { name: /Open Terminal/ }));
    expect(core.ptySpawn).toHaveBeenCalledWith(
      expect.objectContaining({ workspaceId: local.id, harness: null }),
    );
  });

  it("Open Composer composes a run in the checkout, creating no workspace", async () => {
    const user = userEvent.setup();
    show(local.id);

    await user.click(screen.getByRole("button", { name: /Open Composer/ }));
    expect(useProjectsStore.getState().composingWorkspaceId).toBe(local.id);
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  it("Hand off asks the core for the packet and opens the composer here with it", async () => {
    const user = userEvent.setup();
    core.workspaceHandoff.mockResolvedValue({
      text: '# Handoff from Yardsort: workspace "feature" in alpha\n',
      runs: 1,
      events: 9,
    });
    show(feature.id);

    await user.click(screen.getByRole("button", { name: "Hand off…" }));
    await waitFor(() => expect(core.workspaceHandoff).toHaveBeenCalledWith(feature.id));
    await waitFor(() => expect(useProjectsStore.getState().composingWorkspaceId).toBe(feature.id));
    expect(useProjectsStore.getState().composingPrompt).toContain("# Handoff from Yardsort");
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  it("a worktree still gets its harness row", () => {
    show(feature.id);

    expect(screen.getByText("Nothing running in this workspace.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Open Composer/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Open Terminal/ })).not.toBeInTheDocument();
  });
});

it("Run adopts a dev-server terminal in the selected workspace", async () => {
  core.projectAutomationGet.mockResolvedValue({
    copyFiles: [],
    setup: null,
    run: { program: "bun", args: ["run", "dev"] },
  });
  core.workspaceRun.mockResolvedValue({
    id: "server",
    program: "bun",
    labels: { workspace: feature.id, projectRun: feature.id },
    state: { status: "running" },
    busy: false,
  });
  show(feature.id);
  await userEvent.click(screen.getByRole("button", { name: "▶ Run" }));
  expect(core.workspaceRun).toHaveBeenCalledWith(feature.id, useTerminalStore.getState().lastSize);
  expect(useTerminalStore.getState().active[feature.id]).toBe("server");
  expect(screen.getByTestId("terminal")).toBeInTheDocument();
});
it("Run opens project settings when no command is configured", async () => {
  core.projectAutomationGet.mockResolvedValue({ copyFiles: [], setup: null, run: null });
  show(local.id);
  await userEvent.click(screen.getByRole("button", { name: "▶ Run" }));
  expect(await screen.findByRole("dialog")).toHaveAccessibleName("Project settings — alpha");
  expect(core.workspaceRun).not.toHaveBeenCalled();
});

describe("starting an agent beside other conversations", () => {
  const told = {
    about: "codex's conversation",
    note: "Other agents have worked in this workspace: codex (2 turns, last …).",
    digest: "## codex · conversation 3f2a · 2 turns",
  };
  const agentButton = () => screen.getAllByTitle(/^Start CLAUDE here/)[0]!;

  it("leaves telling it to the core when the harness takes background context", async () => {
    useHarnessStore.setState({
      harnesses: [harness("claude", { contextArgs: ["--append-system-prompt", "{context}"] })],
      loaded: true,
    });
    core.conversationContext.mockResolvedValue(told);
    show(feature.id);

    await userEvent.click(agentButton());
    expect(core.ptySpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        workspaceId: feature.id,
        harness: { id: "claude", model: null, effort: null, prompt: null },
      }),
    );
    expect(core.conversationContext).not.toHaveBeenCalled();
  });

  it("starts it bare on Alt-click", async () => {
    core.conversationContext.mockResolvedValue(told);
    const user = userEvent.setup();
    show(feature.id);

    await user.keyboard("{Alt>}");
    await user.click(agentButton());
    await user.keyboard("{/Alt}");
    expect(core.ptySpawn).toHaveBeenCalledWith(
      expect.objectContaining({ harness: expect.objectContaining({ skipContext: true }) }),
    );
  });

  it("asks first when the harness takes none, and starts nothing until answered", async () => {
    const user = userEvent.setup();
    core.conversationContext.mockResolvedValue(told);
    core.workspaceHandoff.mockResolvedValue({
      text: "# Handoff from Yardsort\n",
      runs: 1,
      events: 2,
    });
    show(feature.id);

    await user.click(agentButton());
    expect(await screen.findByRole("menuitem", { name: "Start fresh" })).toBeInTheDocument();
    expect(core.conversationContext).toHaveBeenCalledWith(feature.id, null);
    expect(core.ptySpawn).not.toHaveBeenCalled();

    await user.click(screen.getByRole("menuitem", { name: "Start with the handoff…" }));
    await waitFor(() => expect(useProjectsStore.getState().composingWorkspaceId).toBe(feature.id));
    // The composer opens on the agent whose button was pressed, not the last one used.
    expect(useProjectsStore.getState().composingHarness).toBe("claude");
    expect(core.ptySpawn).not.toHaveBeenCalled();
  });

  it("Start fresh sends nothing", async () => {
    const user = userEvent.setup();
    core.conversationContext.mockResolvedValue(told);
    show(feature.id);

    await user.click(agentButton());
    await user.click(await screen.findByRole("menuitem", { name: "Start fresh" }));
    expect(core.ptySpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        harness: { id: "claude", model: null, effort: null, prompt: null, skipContext: true },
      }),
    );
  });

  it("does not ask when there is nothing to be told", async () => {
    show(feature.id);

    await userEvent.click(agentButton());
    await waitFor(() => expect(core.ptySpawn).toHaveBeenCalled());
    expect(screen.queryByRole("menuitem")).toBeNull();
  });

  it("asks once whether the project shares conversations, before anything starts", async () => {
    const user = userEvent.setup();
    core.conversationsUnasked.mockResolvedValue("codex's conversation");
    show(feature.id);

    await user.click(agentButton());
    const question = await screen.findByRole("dialog", {
      name: "Let alpha’s agents read each other’s conversations?",
    });
    expect(question).toHaveTextContent("There is codex's conversation in this workspace.");
    expect(question).toHaveTextContent("sent to OpenAI");
    expect(core.ptySpawn).not.toHaveBeenCalled();

    await user.click(within(question).getByRole("button", { name: "Share conversations" }));
    expect(core.conversationsShare).toHaveBeenCalledWith(alpha.id, true);
    await waitFor(() => expect(core.ptySpawn).toHaveBeenCalled());
  });

  it("a no is an answer too, and closing the question starts nothing", async () => {
    const user = userEvent.setup();
    core.conversationsUnasked.mockResolvedValue("codex's conversation");
    show(feature.id);

    await user.click(agentButton());
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(core.conversationsShare).not.toHaveBeenCalled();
    expect(core.ptySpawn).not.toHaveBeenCalled();

    await user.click(agentButton());
    await user.click(await screen.findByRole("button", { name: "Not in this project" }));
    expect(core.conversationsShare).toHaveBeenCalledWith(alpha.id, false);
    await waitFor(() => expect(core.ptySpawn).toHaveBeenCalled());
  });

  it("says a new agent was told, and shows what it reads", async () => {
    const user = userEvent.setup();
    core.conversationContext.mockResolvedValue(told);
    core.ptySpawn.mockImplementation(async ({ workspaceId }: { workspaceId: string }) => ({
      id: "s-told",
      program: "claude",
      args: [],
      cwd: null,
      pid: 1,
      size: { cols: 80, rows: 24 },
      labels: {
        workspace: workspaceId,
        harness: "claude",
        record: "rec-new",
        toldAbout: told.about,
      },
      state: { status: "running" as const },
      hasOutput: false,
      busy: false,
      idleMs: 0,
    }));
    useHarnessStore.setState({
      harnesses: [harness("claude", { contextArgs: ["--append-system-prompt", "{context}"] })],
      loaded: true,
    });
    show(feature.id);

    await user.click(agentButton());
    const bar = await screen.findByRole("status");
    expect(bar).toHaveTextContent("Told about codex's conversation");
    expect(
      screen.getByRole("button", { name: "Told about codex's conversation: show" }),
    ).toBeVisible();

    await user.click(within(bar).getByRole("button", { name: "Show" }));
    const dialog = await screen.findByRole("dialog", {
      name: "This workspace’s other conversations",
    });
    expect(core.conversationContext).toHaveBeenCalledWith(feature.id, "rec-new");
    expect(await within(dialog).findByText(told.note)).toBeInTheDocument();
    expect(within(dialog).getByText(told.digest)).toBeInTheDocument();
  });
});
