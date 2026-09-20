import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({ appQuit: vi.fn(), quitCancelled: vi.fn() }));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  hasCore: () => true,
  ipc: core,
}));

import { useProjectsStore } from "@/stores/projects";
import { useQuitStore } from "@/stores/quit";
import { useTerminalStore, type TerminalTab } from "@/stores/terminals";
import { QuitDialog } from "./QuitDialog";

const tab = (id: string, title: string): TerminalTab => ({
  id,
  workspaceId: "ws-1",
  title,
  exit: null,
  recordId: "rec",
  busy: true,
  attention: false,
});

beforeEach(() => {
  vi.clearAllMocks();
  core.appQuit.mockResolvedValue(undefined);
  core.quitCancelled.mockResolvedValue(undefined);
  useQuitStore.setState({ agents: null, deciding: false });
  useTerminalStore.setState({ tabs: [tab("pty-1", "claude")], active: {} });
  useProjectsStore.setState({
    projects: [
      {
        id: "p-1",
        name: "yardsort",
        rootPath: "/repo",
        missing: false,
        workspaces: [
          {
            id: "ws-1",
            projectId: "p-1",
            kind: "worktree",
            name: "fix-the-pty-leak",
            path: "/wt",
            branch: "ys/fix-the-pty-leak",
            baseBranch: "main",
            status: "active",
            checkedOutBranch: null,
            missing: false,
          },
        ],
      },
    ],
  } as never);
});

describe("closing the window while agents are working", () => {
  it("says nothing at all when the core has not asked", () => {
    render(<QuitDialog />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("names the agents and where they are working", () => {
    useQuitStore.getState().asked(["pty-1"]);
    render(<QuitDialog />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("1 agent is still working")).toBeInTheDocument();
    expect(screen.getByText("claude")).toBeInTheDocument();
    expect(screen.getByText("yardsort / fix-the-pty-leak")).toBeInTheDocument();
  });

  it("leaves them running by default, and does not stop anything", async () => {
    useQuitStore.getState().asked(["pty-1"]);
    render(<QuitDialog />);

    await userEvent.click(screen.getByRole("button", { name: "Leave them running" }));
    expect(core.appQuit).toHaveBeenCalledWith(false);
  });

  it("stops them only when that is what was asked for", async () => {
    useQuitStore.getState().asked(["pty-1"]);
    render(<QuitDialog />);

    await userEvent.click(screen.getByRole("button", { name: "Stop them" }));
    expect(core.appQuit).toHaveBeenCalledWith(true);
  });

  // A "no" is respected: nothing is quit, nothing is stopped, and the core is told so the next
  // close asks again rather than going straight through.
  it("cancelling quits nothing and lets the next close ask again", async () => {
    useQuitStore.getState().asked(["pty-1"]);
    render(<QuitDialog />);

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(core.appQuit).not.toHaveBeenCalled();
    expect(core.quitCancelled).toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("escape is a cancel, not a quit", async () => {
    useQuitStore.getState().asked(["pty-1"]);
    render(<QuitDialog />);

    await userEvent.keyboard("{Escape}");

    expect(core.appQuit).not.toHaveBeenCalled();
    expect(core.quitCancelled).toHaveBeenCalled();
  });
});
