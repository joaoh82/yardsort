import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { harness, project, worktree } from "@/test/fixtures";

const core = vi.hoisted(() => ({
  ptySpawn: vi.fn(),
  ptyList: vi.fn(),
  sessionsList: vi.fn(),
  onHostEvent: vi.fn(),
  appInfo: vi.fn(),
  harnessesList: vi.fn(),
  uiStateSave: vi.fn(),
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
  core.uiStateSave.mockResolvedValue(undefined);
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

  it("a worktree still gets its harness row", () => {
    show(feature.id);

    expect(screen.getByText("Nothing running in this workspace.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Open Composer/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Open Terminal/ })).not.toBeInTheDocument();
  });
});
